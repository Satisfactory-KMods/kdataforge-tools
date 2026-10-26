use crate::conditions::{strip_element_conditions, ConditionSite};
use crate::conditions_check::{check_conditions, check_sites, ConditionPlace};
use crate::config::{is_config_file, ConfigIndex};
use crate::config_build;
use crate::document::{
    emit, parse_file, runtime_bool, validate_one, Reporter, SchemaOutcome, MAX_YAML_BYTES,
};
use crate::refs::{self, Env};
use crate::schema::{infer_type, path_within};
use crate::workspace_rules::{validate_dependencies, validate_redirects};
use crate::{
    Diagnostic, LintResult, Range, ReflectionIndex, ReflectionStatus, SchemaRegistry, Severity,
    SourceMap,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use walkdir::WalkDir;

/// Set by the desktop after it has checked the installed plugin and its enabled state.
/// An empty value explicitly selects schema-only validation.
const EFFECTIVE_DATAPACK_ENV: &str = "KDF_EFFECTIVE_DATAPACK_PATH";

pub fn effective_datapack_override() -> Option<Option<PathBuf>> {
    std::env::var_os(EFFECTIVE_DATAPACK_ENV)
        .map(|value| datapack_selection(&value.to_string_lossy()))
}

/// Datapack selected by a host (an environment variable, the language server's
/// `initializationOptions.kdf.datapackPath` or a `workspace/didChangeConfiguration` setting): the
/// path, or `None` for an empty text, which explicitly selects schema-only validation.
pub fn datapack_selection(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    (!value.is_empty()).then(|| PathBuf::from(value))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Profile {
    Runtime,
    Legacy,
}

/// Presentation of `property.cold` (`.kdf-editor.toml [lint] coldProperties`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ColdPropertyLevel {
    #[default]
    Info,
    Hint,
    Off,
}

#[derive(Clone, Debug)]
pub struct LintOptions {
    pub datapack_validation: crate::DatapackValidation,
    pub schema_dirs: Vec<PathBuf>,
    pub allow_schema_override: bool,
    pub allow_unknown_types: bool,
    pub merge_duplicate_diagnostics: bool,
    pub profile: Profile,
    pub datapack_path: Option<PathBuf>,
    /// Caller-owned snapshot avoids re-reading large archives on every edit.
    pub datapack_index: Option<Arc<ReflectionIndex>>,
    /// Schema-only validation: no datapack is loaded, whatever `datapack_path`,
    /// `KDF_EFFECTIVE_DATAPACK_PATH` or the project binding say (a host chose it explicitly).
    pub no_datapack: bool,
    /// How `property.cold` is reported (information, hint or not at all).
    pub cold_properties: ColdPropertyLevel,
}

impl Default for LintOptions {
    fn default() -> Self {
        Self {
            datapack_validation: crate::DatapackValidation::default(),
            schema_dirs: Vec::new(),
            allow_schema_override: false,
            allow_unknown_types: false,
            merge_duplicate_diagnostics: false,
            profile: Profile::Runtime,
            datapack_path: None,
            datapack_index: None,
            no_datapack: false,
            cold_properties: ColdPropertyLevel::Info,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FileBuffer {
    pub path: PathBuf,
    pub text: String,
}

/// Resolves aliases through the nearest existing ancestor of a new document.
pub fn canonical_file_path(path: &Path) -> std::io::Result<PathBuf> {
    dunce::canonicalize(path).or_else(|error| {
        if error.kind() != std::io::ErrorKind::NotFound || fs::symlink_metadata(path).is_ok() {
            return Err(error);
        }
        path.file_name().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "document has no filename")
        })?;
        let mut ancestor = path;
        let mut missing = Vec::new();
        loop {
            let Some(component) = ancestor.components().next_back() else {
                return Err(error);
            };
            missing.push(component);
            let Some(parent) = ancestor.parent() else {
                return Err(error);
            };
            ancestor = parent;
            let existing = if ancestor.as_os_str().is_empty() {
                Path::new(".")
            } else {
                ancestor
            };
            match dunce::canonicalize(existing) {
                Ok(mut canonical) => {
                    if !canonical.is_dir() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::NotADirectory,
                            "document ancestor is not a directory",
                        ));
                    }
                    for component in missing.into_iter().rev() {
                        match component {
                            std::path::Component::ParentDir => {
                                canonical.pop();
                            }
                            std::path::Component::CurDir => {}
                            component => canonical.push(component.as_os_str()),
                        }
                    }
                    return Ok(canonical);
                }
                Err(cause)
                    if cause.kind() == std::io::ErrorKind::NotFound
                        && fs::symlink_metadata(existing).is_err() => {}
                Err(cause) => return Err(cause),
            }
        }
    })
}

#[cfg(test)]
mod canonical_file_path_tests {
    use super::canonical_file_path;
    use std::fs;

    #[test]
    fn resolves_existing_file_and_new_siblings_without_creating_them() {
        let root = tempfile::tempdir().unwrap();
        let existing = root.path().join("existing.yml");
        fs::write(&existing, "type: cdo\n").unwrap();
        assert_eq!(
            canonical_file_path(&existing).unwrap(),
            dunce::canonicalize(&existing).unwrap()
        );
        let canonical_root = dunce::canonicalize(root.path()).unwrap();
        for suffix in ["new.yml", "missing/deeper/new.yml"] {
            let document = root.path().join(suffix);
            assert_eq!(
                canonical_file_path(&document).unwrap(),
                canonical_root.join(suffix)
            );
            assert!(!document.exists());
        }
        assert!(!root.path().join("missing").exists());
    }

    #[test]
    fn normalizes_missing_suffix_only_after_resolving_existing_ancestor() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("existing")).unwrap();
        let expected = dunce::canonicalize(root.path())
            .unwrap()
            .join("other/deep/new.yml");
        for suffix in [
            "existing/../other/./deep/new.yml",
            "missing/../other/deep/new.yml",
        ] {
            assert_eq!(
                canonical_file_path(&root.path().join(suffix)).unwrap(),
                expected
            );
        }
        assert!(!root.path().join("other").exists());
    }

    #[test]
    fn rejects_file_as_document_parent() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file");
        fs::write(&file, "contents").unwrap();
        assert!(canonical_file_path(&file.join("missing/new.yml")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn resolves_symlink_before_parent_segments_and_missing_directories() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target/inner");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("existing.yml"), "type: cdo\n").unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        let canonical_target = dunce::canonicalize(&target).unwrap();
        assert_eq!(
            canonical_file_path(&alias.join("existing.yml")).unwrap(),
            canonical_target.join("existing.yml")
        );
        assert_eq!(
            canonical_file_path(&alias.join("missing/deep/new.yml")).unwrap(),
            canonical_target.join("missing/deep/new.yml")
        );
        assert_eq!(
            canonical_file_path(&alias.join("../missing/new.yml")).unwrap(),
            canonical_target.parent().unwrap().join("missing/new.yml")
        );
        assert!(!target.join("missing").exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_dangling_symlink_ancestor() {
        let root = tempfile::tempdir().unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(root.path().join("absent"), &alias).unwrap();
        assert!(canonical_file_path(&alias.join("missing/new.yml")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn resolves_case_alias_before_missing_directories() {
        let root = tempfile::tempdir().unwrap();
        let alias = std::path::PathBuf::from(root.path().to_str().unwrap().to_ascii_uppercase());
        assert_eq!(
            canonical_file_path(&alias.join("missing/deep/new.yml")).unwrap(),
            dunce::canonicalize(root.path())
                .unwrap()
                .join("missing/deep/new.yml")
        );
    }
}

static DEFAULT_REGISTRY: OnceLock<Result<Arc<SchemaRegistry>, String>> = OnceLock::new();

fn registry_for(options: &LintOptions) -> Result<Arc<SchemaRegistry>, String> {
    if options.schema_dirs.is_empty() {
        DEFAULT_REGISTRY
            .get_or_init(|| SchemaRegistry::new(&LintOptions::default()).map(Arc::new))
            .clone()
    } else {
        SchemaRegistry::new(options).map(Arc::new)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ParsedFile {
    pub(crate) path: PathBuf,
    pub(crate) text: String,
    pub(crate) documents: Vec<Value>,
}

pub fn lint_path(root: &Path, options: &LintOptions) -> Result<LintResult, String> {
    if root.is_file() {
        if !is_yaml(root) {
            return Err("Selected lint file must be YAML".into());
        }
        let file = dunce::canonicalize(root).map_err(|error| error.to_string())?;
        let parent = file.parent().ok_or("Selected lint file has no parent")?;
        let scope = parent
            .ancestors()
            .find(|directory| directory.join("pack.yml").is_file())
            .unwrap_or(parent);
        return lint_workspace_scoped(scope, &[], options, Some(&file));
    }
    lint_workspace(root, &[], options)
}

pub fn lint_workspace(
    root: &Path,
    buffers: &[FileBuffer],
    options: &LintOptions,
) -> Result<LintResult, String> {
    lint_workspace_scoped(root, buffers, options, None)
}

fn lint_workspace_scoped(
    root: &Path,
    buffers: &[FileBuffer],
    options: &LintOptions,
    selected: Option<&Path>,
) -> Result<LintResult, String> {
    let registry = registry_for(options)?;
    let mut result = LintResult::default();
    if !root.is_dir() {
        result.diagnostics.push(Diagnostic::new(
            Severity::Error,
            "root.missing",
            "DataForge root does not exist",
            Some(root.to_path_buf()),
            None,
            "$",
            Range::default(),
            "semantic",
        ));
        result.finish();
        return Ok(result);
    }
    let root =
        dunce::canonicalize(root).map_err(|e| format!("cannot resolve {}: {e}", root.display()))?;
    if let Some(file) = selected.filter(|file| {
        let scope = file
            .ancestors()
            .find(|parent| {
                parent.file_name().is_some_and(|name| {
                    name.eq_ignore_ascii_case("DataForge")
                        || name.eq_ignore_ascii_case("data-editor")
                })
            })
            .unwrap_or(&root);
        !crate::is_runtime_authored_path(file, scope)
    }) {
        result.diagnostics.push(ignored_directory_diagnostic(file));
        result.finish();
        return Ok(result);
    }
    let owned_datapack = if options.datapack_index.is_none() && !options.no_datapack {
        let selected = if let Some(path) = &options.datapack_path {
            Some(path.clone())
        } else if let Some(override_path) = effective_datapack_override() {
            override_path
        } else {
            bound_datapack_path(&root)
                .or_else(|| selected.and_then(|_| root.ancestors().find_map(bound_datapack_path)))
        };
        selected.map(|path| ReflectionIndex::load(&path))
    } else {
        None
    };
    let datapack = options
        .datapack_index
        .as_deref()
        .or(owned_datapack.as_ref());
    if let Some(index) = datapack {
        if matches!(
            index.status,
            ReflectionStatus::Invalid
                | ReflectionStatus::UnsupportedVersion
                | ReflectionStatus::MissingData
        ) {
            result.diagnostics.push(Diagnostic::new(
                Severity::Warning,
                "datapack.unavailable",
                index
                    .reason
                    .clone()
                    .unwrap_or_else(|| "reflection data unavailable".into()),
                Some(root.clone()),
                None,
                "$",
                Range::default(),
                "datapack",
            ));
        }
    }
    let mut files = HashMap::<PathBuf, String>::new();
    for entry in WalkDir::new(&root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            crate::runtime_authored_entry(entry)
                && selected.is_none_or(|file| {
                    let path = entry.path();
                    file == path
                        || file.starts_with(path)
                        || (is_pack_manifest(path) && path.parent() == Some(root.as_path()))
                })
        })
    {
        let entry = entry.map_err(|e| format!("cannot walk {}: {e}", root.display()))?;
        let path = entry.path();
        if entry.file_type().is_symlink() {
            result.diagnostics.push(Diagnostic::new(
                Severity::Error,
                "root.symlink",
                "symlinks are not allowed in DataForge packs",
                Some(path.to_path_buf()),
                None,
                "$",
                Range::default(),
                "semantic",
            ));
            continue;
        }
        if entry.file_type().is_file() && is_yaml(path) {
            if entry
                .metadata()
                .ok()
                .is_some_and(|meta| meta.len() > MAX_YAML_BYTES as u64)
            {
                result.diagnostics.push(Diagnostic::new(
                    Severity::Error,
                    "yaml.too-large",
                    "YAML file exceeds 8 MiB safety limit",
                    Some(path.to_path_buf()),
                    None,
                    "$",
                    Range::default(),
                    "yaml",
                ));
                continue;
            }
            match crate::text_encoding::read_text_file(path, MAX_YAML_BYTES as u64) {
                Ok(text) => {
                    files.insert(path.to_path_buf(), text);
                }
                Err(error) => result.diagnostics.push(Diagnostic::new(
                    Severity::Error,
                    "root.read",
                    format!("cannot read YAML file: {error}"),
                    Some(path.to_path_buf()),
                    None,
                    "$",
                    Range::default(),
                    "semantic",
                )),
            }
        }
    }
    for buffer in buffers {
        let path = if buffer.path.is_absolute() {
            buffer.path.clone()
        } else {
            root.join(&buffer.path)
        };
        let path = canonical_file_path(&path).unwrap_or(path);
        if path_within(&path, &root) && !crate::is_runtime_authored_path(&path, &root) {
            result.diagnostics.push(ignored_directory_diagnostic(&path));
            continue;
        }
        if path_within(&path, &root) && is_yaml(&path) {
            files.insert(path, buffer.text.clone());
        }
    }
    let mut files = files.into_iter().collect::<Vec<_>>();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let parsed = files
        .into_iter()
        .filter_map(|(path, text)| parse_file(path, text, &mut result))
        .collect::<Vec<_>>();
    let mut include_documents = parsed
        .iter()
        .filter(|file| file.documents.len() > 1)
        .map(|file| {
            let indices = file
                .documents
                .iter()
                .enumerate()
                .filter(|(_, document)| {
                    document.is_object() && infer_type(&file.path, document, true).is_some()
                })
                .map(|(index, _)| index)
                .collect::<HashSet<_>>();
            (file.path.clone(), indices)
        })
        .collect::<HashMap<_, _>>();
    if selected.is_some() {
        // Inspect only explicitly referenced multi-document include files. Their
        // content supplies index existence, never sibling validation results.
        for relative in parsed
            .iter()
            .flat_map(|file| &file.documents)
            .flat_map(|document| {
                let include = document.get("include");
                include.and_then(Value::as_str).into_iter().chain(
                    include
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str),
                )
            })
        {
            let Some((relative_file, suffix)) = relative
                .strip_suffix(']')
                .and_then(|value| value.rsplit_once('['))
            else {
                continue;
            };
            if suffix
                .parse::<usize>()
                .ok()
                .is_none_or(|index| index.to_string() != suffix)
            {
                continue;
            }
            let target = root.join(relative_file);
            if include_documents.contains_key(&target) || !path_within(&target, &root) {
                continue;
            }
            let Ok(resolved) = dunce::canonicalize(&target) else {
                continue;
            };
            if !path_within(&resolved, &root)
                || !fs::metadata(&resolved).is_ok_and(|meta| meta.len() <= MAX_YAML_BYTES as u64)
            {
                continue;
            }
            let Ok(text) = crate::text_encoding::read_text_file(&resolved, MAX_YAML_BYTES as u64)
            else {
                continue;
            };
            let Some(file) = parse_file(target.clone(), text, &mut LintResult::default()) else {
                continue;
            };
            if file.documents.len() > 1 {
                let indices = file
                    .documents
                    .iter()
                    .enumerate()
                    .filter(|(_, document)| {
                        document.is_object() && infer_type(&file.path, document, true).is_some()
                    })
                    .map(|(index, _)| index)
                    .collect();
                include_documents.insert(target, indices);
            }
        }
    }
    let manifests = parsed
        .iter()
        .filter(|file| is_pack_manifest(&file.path))
        .collect::<Vec<_>>();
    result.packs = manifests.len();
    if manifests.is_empty() {
        result.diagnostics.push(Diagnostic::new(
            Severity::Error,
            "root.no-packs",
            "no pack.yml found",
            Some(root.clone()),
            None,
            "$",
            Range::default(),
            "semantic",
        ));
    }
    let config_index = config_build::build(&parsed, datapack);
    emit_definition_issues(&config_index, &parsed, &mut result);
    let mut refs = BTreeMap::<String, PathBuf>::new();
    let mut dependencies = BTreeMap::<String, Vec<String>>::new();
    for file in &manifests {
        if options.profile == Profile::Legacy && file.path.parent() == Some(root.as_path()) {
            emit(
                file,
                0,
                "",
                Severity::Error,
                "policy.legacy-layout",
                "expected DataForge/**/<pack>/pack.yml layout",
                &mut result,
            );
        }
        if let Some(document) = file.documents.first() {
            let source = SourceMap::parse(&file.text).ok();
            validate_one(
                file,
                0,
                document,
                "pack",
                &registry,
                options,
                source.as_ref(),
                &mut result,
            );
            crate::rules::validate_condition_mod_specs(file, 0, document, &mut result);
            check_manifest(
                file,
                document,
                source.as_ref(),
                options,
                datapack,
                Some(&config_index),
                None,
                file.path.parent(),
                &mut result,
            );
            // The runtime scanner drops disabled packs before resolving pack refs and dependencies.
            if document.get("enabled").and_then(runtime_bool) == Some(false) {
                continue;
            }
            if let Some(reference) = document.get("ref").and_then(Value::as_str) {
                if let Some(other) = refs.get(reference) {
                    emit(
                        file,
                        0,
                        "/ref",
                        Severity::Error,
                        "semantic.duplicate-pack-ref",
                        format!(
                            "duplicate pack ref {reference:?}; already used by {}",
                            other.display()
                        ),
                        &mut result,
                    );
                } else {
                    refs.insert(reference.to_owned(), file.path.clone());
                    let deps = document
                        .get("dependencies")
                        .and_then(Value::as_array)
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default();
                    dependencies.insert(reference.to_owned(), deps);
                }
            }
        }
    }
    // Cross-pack availability requires a workspace scan; file lint cannot infer it
    // from a single manifest snapshot.
    if selected.is_none() {
        validate_dependencies(&refs, &dependencies, &parsed, &mut result);
    }

    let pack_roots = manifests
        .iter()
        .filter_map(|file| file.path.parent().map(Path::to_path_buf))
        .collect::<HashSet<_>>();
    let active_pack_refs = refs
        .iter()
        .filter_map(|(reference, path)| {
            path.parent()
                .map(|pack_root| (pack_root.to_path_buf(), reference.clone()))
        })
        .collect::<HashMap<_, _>>();
    let mut generated =
        HashMap::<(PathBuf, crate::rules::GeneratedNamespace, String), PathBuf>::new();
    let mut runtime_generated =
        HashMap::<(String, crate::rules::GeneratedNamespace, String), (String, PathBuf)>::new();
    for file in &parsed {
        if is_pack_manifest(&file.path) {
            continue;
        }
        let pack_root = file
            .path
            .ancestors()
            .find(|parent| pack_roots.contains(*parent));
        let Some(pack_root) = pack_root else {
            emit(
                file,
                0,
                "",
                Severity::Error,
                "semantic.orphan-document",
                "YAML document is outside a pack with pack.yml",
                &mut result,
            );
            continue;
        };
        let source = SourceMap::parse(&file.text).ok();
        for (index, document) in file.documents.iter().enumerate() {
            if document.is_null() {
                continue;
            }
            if !document.is_object() {
                emit(
                    file,
                    index,
                    "",
                    Severity::Error,
                    "semantic.document-map",
                    "document must be a YAML mapping",
                    &mut result,
                );
                continue;
            }
            let kind = document_kind(&file.path, document, file.documents.len() > 1);
            let kind = match kind {
                DocumentKind::Skip => continue,
                DocumentKind::Kind(kind) => Some(kind),
                DocumentKind::Unknown => None,
            };
            let Some(kind) = kind else {
                let message = if file.documents.len() > 1 {
                    "every document in a multi-document YAML file requires explicit type"
                } else {
                    "missing type and filename has no recognized <name>.<type>.yml suffix"
                };
                emit(
                    file,
                    index,
                    "/type",
                    Severity::Error,
                    "semantic.missing-type",
                    message,
                    &mut result,
                );
                continue;
            };
            result.documents += 1;
            let mut working = document.clone();
            let sites = if kind == "config" {
                Vec::new()
            } else {
                strip_element_conditions(&mut working)
            };
            let outcome = validate_one(
                file,
                index,
                &working,
                &kind,
                &registry,
                options,
                source.as_ref(),
                &mut result,
            );
            crate::rules::semantic_document(
                crate::rules::SemanticContext {
                    file,
                    index,
                    document: &working,
                    kind: &kind,
                    pack_root,
                    pack_ref: active_pack_refs.get(pack_root).map(String::as_str),
                    include_documents: &include_documents,
                    datapack,
                },
                &mut generated,
                &mut runtime_generated,
                &mut result,
            );
            crate::reflection_validation::validate(
                file,
                index,
                document,
                source.as_ref(),
                options,
                datapack,
                &mut result,
            );
            if kind != "config" {
                let env = Env {
                    file,
                    source: source.as_ref(),
                    index,
                    kind: &kind,
                    pack_ref: config_index
                        .pack_for_path(&file.path)
                        .map(|pack| pack.pack_ref.as_str()),
                    config: Some(&config_index),
                    datapack,
                    options,
                };
                check_document_references(&env, &registry, &working, &sites, &outcome, &mut result);
            }
        }
    }
    if selected.is_none() {
        validate_redirects(&manifests, &generated, &mut result);
    } else if let Some(selected) = selected {
        // The manifest supplies pack identity and policy, but only this file's
        // diagnostics belong to an explicitly selected file run.
        result
            .diagnostics
            .retain(|diagnostic| diagnostic.file.as_deref() == Some(selected));
    }
    crate::reflection_validation::apply_policy(options, &mut result);
    result.finish();
    if options.merge_duplicate_diagnostics {
        result.merge_redundant();
    }
    Ok(result)
}

fn ignored_directory_diagnostic(path: &Path) -> Diagnostic {
    Diagnostic::new(
        Severity::Warning,
        "root.ignored-directory",
        "Runtime ignores documents inside dot-prefixed directories, Docs, and Scripts",
        Some(path.to_path_buf()),
        None,
        "$",
        Range::default(),
        "semantic",
    )
}

pub fn validate_document(
    path: &Path,
    text: &str,
    options: &LintOptions,
) -> Result<LintResult, String> {
    let registry = registry_for(options)?;
    let mut result = LintResult::default();
    if let Some(root) = path.ancestors().find(|parent| {
        parent.file_name().is_some_and(|name| {
            name.eq_ignore_ascii_case("DataForge") || name.eq_ignore_ascii_case("data-editor")
        })
    }) {
        if !crate::is_runtime_authored_path(path, root) {
            result.diagnostics.push(ignored_directory_diagnostic(path));
        }
    }
    if let Some(file) = parse_file(path.to_path_buf(), text.to_owned(), &mut result) {
        let is_manifest = is_pack_manifest(path);
        let source = SourceMap::parse(&file.text).ok();
        let datapack = options.datapack_index.as_deref();
        // A single document knows only its own definitions: a config definition is indexed on its
        // own, other documents get no workspace index (cross references stay unchecked).
        let single_index = if is_manifest {
            ConfigIndex::default()
        } else {
            config_build::build_single(&file, datapack)
        };
        emit_definition_issues(&single_index, std::slice::from_ref(&file), &mut result);
        for (index, document) in file.documents.iter().enumerate() {
            if document.is_null() {
                continue;
            }
            let kind = if is_manifest {
                DocumentKind::Kind("pack".to_owned())
            } else {
                document_kind(path, document, file.documents.len() > 1)
            };
            match kind {
                DocumentKind::Skip => {}
                DocumentKind::Kind(kind) => {
                    let mut working = document.clone();
                    let sites = if kind == "config" || kind == "pack" {
                        Vec::new()
                    } else {
                        strip_element_conditions(&mut working)
                    };
                    let outcome = validate_one(
                        &file,
                        index,
                        &working,
                        &kind,
                        &registry,
                        options,
                        source.as_ref(),
                        &mut result,
                    );
                    crate::reflection_validation::validate(
                        &file,
                        index,
                        document,
                        source.as_ref(),
                        options,
                        datapack,
                        &mut result,
                    );
                    if is_manifest {
                        crate::rules::validate_condition_mod_specs(
                            &file,
                            index,
                            document,
                            &mut result,
                        );
                        check_manifest(
                            &file,
                            document,
                            source.as_ref(),
                            options,
                            datapack,
                            None,
                            None,
                            path.parent(),
                            &mut result,
                        );
                    } else if document.is_object() {
                        crate::rules::semantic_document_local(
                            &file,
                            index,
                            &working,
                            &kind,
                            datapack,
                            &mut result,
                        );
                        if kind != "config" {
                            let env = Env {
                                file: &file,
                                source: source.as_ref(),
                                index,
                                kind: &kind,
                                pack_ref: None,
                                config: None,
                                datapack,
                                options,
                            };
                            check_document_references(
                                &env,
                                &registry,
                                &working,
                                &sites,
                                &outcome,
                                &mut result,
                            );
                        }
                    }
                    if is_manifest {
                        result.packs += 1;
                    } else {
                        result.documents += 1;
                    }
                }
                DocumentKind::Unknown => {
                    emit(
                        &file,
                        index,
                        "/type",
                        Severity::Error,
                        "semantic.missing-type",
                        "document requires type",
                        &mut result,
                    );
                    crate::reflection_validation::validate(
                        &file,
                        index,
                        document,
                        source.as_ref(),
                        options,
                        options.datapack_index.as_deref(),
                        &mut result,
                    );
                }
            }
        }
    }
    crate::reflection_validation::apply_policy(options, &mut result);
    result.finish();
    if options.merge_duplicate_diagnostics {
        result.merge_redundant();
    }
    Ok(result)
}

enum DocumentKind {
    /// Reported elsewhere (config file with a foreign type, `type: config` in another file).
    Skip,
    Kind(String),
    Unknown,
}

/// Document kind. `*.config.yml` files are config definitions whatever their (absent) type is.
fn document_kind(path: &Path, document: &Value, multi: bool) -> DocumentKind {
    if is_config_file(path) {
        let explicit = document
            .get("type")
            .and_then(Value::as_str)
            .map(|text| text.trim().to_ascii_lowercase())
            .filter(|text| !text.is_empty());
        return match explicit.as_deref() {
            None | Some("config") => DocumentKind::Kind("config".to_owned()),
            Some(_) => DocumentKind::Skip,
        };
    }
    match infer_type(path, document, multi) {
        Some(kind) if kind == "config" => DocumentKind::Skip,
        Some(kind) => DocumentKind::Kind(kind),
        None => DocumentKind::Unknown,
    }
}

/// Emits the definition problems found while indexing config documents.
fn emit_definition_issues(index: &ConfigIndex, files: &[ParsedFile], result: &mut LintResult) {
    let mut grouped = BTreeMap::<&Path, Vec<&crate::config::DefinitionIssue>>::new();
    for issue in &index.issues {
        grouped.entry(issue.file.as_path()).or_default().push(issue);
    }
    for (path, issues) in grouped {
        let Some(file) = files.iter().find(|file| file.path == path) else {
            continue;
        };
        let source = SourceMap::parse(&file.text).ok();
        let reporter = Reporter {
            file,
            source: source.as_ref(),
        };
        for issue in issues {
            reporter.emit(
                result,
                issue.document,
                &issue.pointer,
                issue.severity,
                issue.code,
                issue.message.clone(),
            );
        }
    }
}

/// Icon and `config:` condition checks of a pack manifest.
#[expect(
    clippy::too_many_arguments,
    reason = "manifest checks need the file, its parse state and lint context"
)]
fn check_manifest(
    file: &ParsedFile,
    document: &Value,
    source: Option<&SourceMap<'_>>,
    options: &LintOptions,
    datapack: Option<&ReflectionIndex>,
    config: Option<&ConfigIndex>,
    pack_ref: Option<&str>,
    pack_dir: Option<&Path>,
    result: &mut LintResult,
) {
    let reporter = Reporter { file, source };
    crate::pack_rules::check_icon(&reporter, 0, document, pack_dir, result);
    crate::pack_rules::check_scripts(&reporter, document, result);
    if let Some(conditions) = document.get("conditions") {
        let env = Env {
            file,
            source,
            index: 0,
            kind: "pack",
            pack_ref: pack_ref.or_else(|| document.get("ref").and_then(Value::as_str)),
            config,
            datapack,
            options,
        };
        check_conditions(
            &env,
            &ConditionPlace {
                pointer: "/conditions",
                owner: &["conditions"],
            },
            conditions,
            &mut result.diagnostics,
        );
    }
}

/// Conditions, typed references and templates of a content document.
fn check_document_references(
    env: &Env<'_>,
    registry: &SchemaRegistry,
    working: &Value,
    sites: &[ConditionSite],
    outcome: &SchemaOutcome,
    result: &mut LintResult,
) {
    if let Some(conditions) = working.get("conditions") {
        check_conditions(
            env,
            &ConditionPlace {
                pointer: "/conditions",
                owner: &["conditions"],
            },
            conditions,
            &mut result.diagnostics,
        );
    }
    check_sites(env, registry, sites, result);
    result
        .diagnostics
        .extend(refs::check_document(env, working, &outcome.conflicts));
}

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("yml") || s.eq_ignore_ascii_case("yaml"))
}

pub(crate) fn is_pack_manifest(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if cfg!(windows) {
        name.eq_ignore_ascii_case("pack.yml")
    } else {
        name == "pack.yml"
    }
}

/// Datapack bound to the project of `root` by `[datapack] path` of its `.kdf-editor.toml` (the
/// project folder is the parent of a `DataForge` root).
pub fn bound_datapack_path(root: &Path) -> Option<PathBuf> {
    let project = if root.file_name().and_then(|s| s.to_str()) == Some("DataForge") {
        root.parent()?
    } else {
        root
    };
    let config = crate::text_encoding::read_text_file(
        &project.join(".kdf-editor.toml"),
        MAX_YAML_BYTES as u64,
    )
    .ok()?;
    let value: toml::Value = config.parse().ok()?;
    value
        .get("datapack")?
        .get("path")?
        .as_str()
        .map(PathBuf::from)
}
