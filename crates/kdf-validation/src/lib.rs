//! Shared KDataForge validation engine for the desktop, language server and CLI.

fn runtime_ignored_directory(name: &str) -> bool {
    name.starts_with('.')
        || name.eq_ignore_ascii_case("Docs")
        || name.eq_ignore_ascii_case("Scripts")
}

/// Whether a document is outside development-only directories below the selected root.
pub fn is_runtime_authored_path(path: &std::path::Path, root: &std::path::Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    relative.parent().is_none_or(|parent| {
        parent.components().all(|component| match component {
            std::path::Component::Normal(name) => {
                !runtime_ignored_directory(&name.to_string_lossy())
            }
            _ => false,
        })
    })
}

/// Excludes development-only descendants from runtime-authored pack walks.
pub(crate) fn runtime_authored_entry(entry: &walkdir::DirEntry) -> bool {
    entry.depth() == 0
        || !entry.file_type().is_dir()
        || !entry
            .file_name()
            .to_str()
            .is_some_and(runtime_ignored_directory)
}

mod affected;
mod assist;
pub mod authoring;
pub mod cold;
pub mod conditions;
mod conditions_check;
pub mod config;
mod config_build;
mod document;
pub mod evidence;
pub mod expression;
pub mod field_docs;
mod operation;
mod pack_rules;
mod preview;
mod project;
mod reflection;
mod reflection_archive;
mod reflection_diff;
mod reflection_query;
mod reflection_validation;
mod refs;
mod rules;
mod runtime_features;
mod schema;
mod setting_index;
mod settings;
pub mod slots;
mod smm_profile;
mod sound;
mod source;
pub mod template;
pub mod text_encoding;
mod workspace_rules;

pub use affected::{affected_elements, AffectedElement, AffectedElements};
pub use assist::{AssistContext, AssistDocumentHit, AssistOverview, AssistPack};
pub use cold::{builtin_catalog, ColdCatalog, ColdEntry, Reload};
pub use field_docs::FieldDoc;
pub use operation::{
    operation_spec, tag_container_operation, tag_container_selector_property, OperationSpec,
    OPERATIONS, TAG_CONTAINER_SELECTOR, TAG_CONTAINER_SELECTOR_DOC,
};
pub use preview::{preview_binding, BindingPreview, DiagnosticChange, LintSummary};
pub use project::{
    bound_datapack_path, canonical_file_path, datapack_selection, effective_datapack_override,
    lint_path, lint_workspace, validate_document, ColdPropertyLevel, FileBuffer, LintOptions,
    Profile,
};
pub use reflection::{ClassSummary, ReflectionIndex, ReflectionStatus};
pub use reflection_diff::{
    diff_reflection_archives, ReflectionChange, ReflectionChangeKind, ReflectionDiff,
    ReflectionDiffStatus, ReflectionPropertyChange,
};
pub use reflection_query::{
    display_type, explicit_params, AssetEntry, Callable, FunctionInfo, Implicit, MemberInfo,
    MemberKind, ParamInfo, PropertyInfo, PropertyKind, ReloadClassifier, ResolvedClass, TargetKind,
};
pub use rules::{generated_identity, property_path_identity, GeneratedNamespace};
pub use schema::{infer_type, SchemaRegistry, CANONICAL_TYPES, ROOT_TYPES};
pub use setting_index::{
    SettingCategory, SettingPlacement, SettingProvider, SettingRecord, SettingValueType,
    SettingsIndex,
};
pub use settings::{
    DatapackValidation, LintSettings, LintTrigger, OutputFormat, UnknownClassSeverity,
};
pub use smm_profile::{
    compare_required_mods, parse_profiles_json, SmmModMatch, SmmModState, SmmProfile,
    SmmProfileComparison, SmmProfileMod, SmmProfiles,
};
pub use source::{byte_offset_at_utf16, clamped_byte_offset_at_utf16, Position, Range, SourceMap};

use serde::Serialize;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Information,
}

/// Presentation level of an informational diagnostic.
///
/// Informational diagnostics (`property.cold`) keep `severity: Warning` so existing consumers
/// that only know errors and warnings still display them, but they are excluded from
/// `LintResult::warnings` (so `--warnings-as-errors` ignores them). Consumers that understand
/// the field should show them as information or hint instead of a warning.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Advisory {
    Information,
    Hint,
}

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub file: Option<PathBuf>,
    pub document: Option<usize>,
    #[serde(rename = "yamlPath")]
    pub yaml_path: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub schema: Option<String>,
    pub source: String,
    pub range: Range,
    /// `Some` for informational diagnostics (see [`Advisory`]); absent in JSON otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advisory: Option<Advisory>,
    /// Provenance of a proposed correction; absent means no verified replacement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<evidence::Evidence>,
}

impl Diagnostic {
    #[expect(
        clippy::too_many_arguments,
        reason = "constructor names each distinct diagnostic field at call sites"
    )]
    pub fn new(
        severity: Severity,
        code: impl Into<String>,
        message: impl Into<String>,
        file: Option<PathBuf>,
        document: Option<usize>,
        yaml_path: impl Into<String>,
        range: Range,
        source: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            code: code.into(),
            message: message.into(),
            file,
            document,
            yaml_path: yaml_path.into(),
            line: Some(range.start.line as usize + 1),
            column: Some(range.start.character as usize + 1),
            schema: None,
            source: source.into(),
            range,
            advisory: None,
            evidence: None,
        }
    }

    /// Marks the diagnostic as informational: severity stays `Warning`, the advisory level tells
    /// aware consumers to downgrade the presentation.
    pub fn with_advisory(mut self, advisory: Advisory) -> Self {
        self.severity = Severity::Warning;
        self.advisory = Some(advisory);
        self
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct LintResult {
    pub ok: bool,
    pub packs: usize,
    pub documents: usize,
    pub errors: usize,
    pub warnings: usize,
    pub diagnostics: Vec<Diagnostic>,
}

impl LintResult {
    pub fn finish(&mut self) {
        self.diagnostics.sort_by(|a, b| {
            (
                a.file.as_ref(),
                a.document,
                a.range.start.line,
                a.range.start.character,
                &a.code,
                &a.message,
            )
                .cmp(&(
                    b.file.as_ref(),
                    b.document,
                    b.range.start.line,
                    b.range.start.character,
                    &b.code,
                    &b.message,
                ))
        });
        self.errors = self
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        self.warnings = self
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning && d.advisory.is_none())
            .count();
        self.ok = self.errors == 0;
    }

    /// Merge known schema/semantic reports of one failure while preserving distinct rules.
    pub fn merge_redundant(&mut self) {
        let semantic = self
            .diagnostics
            .iter()
            .filter(|item| item.source == "semantic")
            .map(|item| {
                (
                    item.file.clone(),
                    item.document,
                    item.yaml_path.clone(),
                    item.range.start,
                    item.code.clone(),
                )
            })
            .collect::<Vec<_>>();
        self.diagnostics.retain(|item| {
            if item.source != "schema" {
                return true;
            }
            !semantic
                .iter()
                .any(|(file, document, path, position, code)| {
                    file == &item.file
                        && document == &item.document
                        && path == &item.yaml_path
                        && position == &item.range.start
                        && same_failure(&item.code, code)
                })
        });
        self.finish();
    }
}

fn same_failure(schema: &str, semantic: &str) -> bool {
    matches!(
        (schema, semantic),
        ("schema.enum", "semantic.invalid-op")
            | ("schema.anyOf", "semantic.scope-target")
            | ("schema.oneOf", "semantic.where-operator")
            | ("schema.oneOf", "semantic.scope-filter")
            | ("schema.oneOf", "config.condition-invalid")
            | ("schema.pattern", "semantic.property-path")
            | ("schema.required", "semantic.op-required")
    )
}
