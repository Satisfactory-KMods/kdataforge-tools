use jsonschema::{error::ValidationErrorKind, Resource, Validator};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::project::LintOptions;
use crate::{Diagnostic, Severity, SourceMap};

pub const CANONICAL_TYPES: [&str; 21] = [
    "asset",
    "building",
    "cdo",
    "category_reorder",
    "class",
    "config",
    "curve",
    "dataasset",
    "eventrule",
    "gametag",
    "item",
    "localization",
    "recipe",
    "research",
    "resource",
    "resourcenode",
    "schematic",
    "sinkpoints",
    "sound",
    "sublevel",
    "unlock",
];
pub const ROOT_TYPES: [&str; 22] = [
    "asset",
    "building",
    "cdo",
    "category_reorder",
    "class",
    "config",
    "curve",
    "dataasset",
    "eventrule",
    "gametag",
    "item",
    "localization",
    "pack",
    "recipe",
    "research",
    "resource",
    "resourcenode",
    "schematic",
    "sinkpoints",
    "sound",
    "sublevel",
    "unlock",
];

const WHERE_OPERATORS: [&str; 12] = [
    "equals",
    "notEquals",
    "lessThan",
    "lessOrEqual",
    "greaterThan",
    "greaterOrEqual",
    "contains",
    "notContains",
    "in",
    "notIn",
    "matches",
    "isEmpty",
];

pub(crate) fn canonical_where_operator(value: &str) -> Option<&'static str> {
    WHERE_OPERATORS
        .iter()
        .copied()
        .find(|known| known.eq_ignore_ascii_case(value))
}

const BUILTIN: [(&str, &str); 25] = [
    (
        "actor-selector.schema.yml",
        include_str!("../schemas/actor-selector.schema.yml"),
    ),
    (
        "eventrule.schema.yml",
        include_str!("../schemas/eventrule.schema.yml"),
    ),
    (
        "sound.schema.yml",
        include_str!("../schemas/sound.schema.yml"),
    ),
    (
        "asset.schema.yml",
        include_str!("../schemas/asset.schema.yml"),
    ),
    (
        "building.schema.yml",
        include_str!("../schemas/building.schema.yml"),
    ),
    ("cdo.schema.yml", include_str!("../schemas/cdo.schema.yml")),
    (
        "category_reorder.schema.yml",
        include_str!("../schemas/category_reorder.schema.yml"),
    ),
    (
        "class.schema.yml",
        include_str!("../schemas/class.schema.yml"),
    ),
    (
        "common.schema.yml",
        include_str!("../schemas/common.schema.yml"),
    ),
    (
        "config.schema.yml",
        include_str!("../schemas/config.schema.yml"),
    ),
    (
        "curve.schema.yml",
        include_str!("../schemas/curve.schema.yml"),
    ),
    (
        "dataasset.schema.yml",
        include_str!("../schemas/dataasset.schema.yml"),
    ),
    (
        "gametag.schema.yml",
        include_str!("../schemas/gametag.schema.yml"),
    ),
    (
        "item.schema.yml",
        include_str!("../schemas/item.schema.yml"),
    ),
    (
        "localization.schema.yml",
        include_str!("../schemas/localization.schema.yml"),
    ),
    ("mam.schema.yml", include_str!("../schemas/mam.schema.yml")),
    (
        "pack.schema.yml",
        include_str!("../schemas/pack.schema.yml"),
    ),
    (
        "recipe.schema.yml",
        include_str!("../schemas/recipe.schema.yml"),
    ),
    (
        "research.schema.yml",
        include_str!("../schemas/research.schema.yml"),
    ),
    (
        "resource.schema.yml",
        include_str!("../schemas/resource.schema.yml"),
    ),
    (
        "resourcenode.schema.yml",
        include_str!("../schemas/resourcenode.schema.yml"),
    ),
    (
        "schematic.schema.yml",
        include_str!("../schemas/schematic.schema.yml"),
    ),
    (
        "sinkpoints.schema.yml",
        include_str!("../schemas/sinkpoints.schema.yml"),
    ),
    (
        "sublevel.schema.yml",
        include_str!("../schemas/sublevel.schema.yml"),
    ),
    (
        "unlock.schema.yml",
        include_str!("../schemas/unlock.schema.yml"),
    ),
];

struct Loaded {
    name: String,
    schema: Value,
}

pub struct SchemaRegistry {
    resources: HashMap<String, Value>,
    validators: HashMap<String, Validator>,
    names: HashMap<String, String>,
    schemas: HashMap<String, Value>,
    /// Validates lifted element-level condition keys (`common#/$defs/conditionHolder`).
    holder: Option<Validator>,
    diagnostics: Vec<Diagnostic>,
    providers: Vec<ProviderDeclaration>,
}

/// A schema `type` error at an unquoted typed reference. The reference resolves to a config value
/// at runtime, so the error becomes a compatibility check against the expected JSON types.
#[derive(Clone, Debug)]
pub(crate) struct TypeConflict {
    pub(crate) pointer: String,
    /// JSON type names the schema accepted (`integer`, `number`, `boolean`, `array`, ...).
    pub(crate) expected: Vec<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderDeclaration {
    pub prefix: String,
    pub description: String,
    #[serde(default)]
    pub value_type: Option<String>,
    #[serde(default)]
    pub free_form_keys: Option<bool>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderFile {
    providers: Vec<ProviderDeclaration>,
}

const COMMON_SCHEMA_SUFFIX: &str = "/common.schema.yml";

impl SchemaRegistry {
    pub fn new(options: &LintOptions) -> Result<Self, String> {
        Self::for_root(options, None)
    }

    /// Loads mod-owned schemas visible from a workspace or authored file.
    pub fn for_root(options: &LintOptions, root: Option<&Path>) -> Result<Self, String> {
        match root {
            Some(root) => Self::for_roots(options, &[root]),
            None => Self::for_roots(options, &[]),
        }
    }

    /// Loads mod-owned schemas visible from one or more workspace roots.
    pub fn for_roots(options: &LintOptions, roots: &[&Path]) -> Result<Self, String> {
        let mut loaded = HashMap::<String, Loaded>::new();
        let mut ids = HashMap::<String, Value>::new();
        for (name, body) in BUILTIN {
            insert_schema(name, body, false, &mut loaded, &mut ids)?;
        }
        let mut diagnostics = Vec::new();
        let providers = if roots.is_empty() {
            Vec::new()
        } else {
            load_providers_for_roots(roots, &mut diagnostics)?
        };
        if !roots.is_empty() {
            for path in discover_mod_schemas_for_roots(roots) {
                let body = fs::read_to_string(&path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                let yaml: serde_yaml::Value = serde_yaml::from_str(&body)
                    .map_err(|e| format!("invalid schema YAML {}: {e}", path.display()))?;
                let schema = serde_json::to_value(yaml).map_err(|e| e.to_string())?;
                let kind = schema
                    .get("x-kdf-type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_lowercase();
                if kind == "mam" {
                    diagnostics.push(Diagnostic::new(
                        Severity::Error,
                        "schema.builtin-shadow",
                        format!(
                            "schema type {kind:?} in {} conflicts with built-in mam.schema.yml",
                            path.display()
                        ),
                        Some(path.clone()),
                        None,
                        "$",
                        crate::Range::default(),
                        "schema",
                    ));
                    continue;
                }
                let previous = loaded.get(&kind);
                if let Some(previous) = previous {
                    let builtin = BUILTIN.iter().any(|(name, _)| *name == previous.name);
                    diagnostics.push(Diagnostic::new(
                        Severity::Error,
                        if builtin {
                            "schema.builtin-shadow"
                        } else {
                            "schema.duplicate-type"
                        },
                        format!(
                            "schema type {kind:?} in {} conflicts with {}; first schema wins",
                            path.display(),
                            previous.name
                        ),
                        Some(path.clone()),
                        None,
                        "$",
                        crate::Range::default(),
                        "schema",
                    ));
                    continue;
                }
                insert_schema(&path.to_string_lossy(), &body, false, &mut loaded, &mut ids)?;
            }
        }
        for dir in &options.schema_dirs {
            if !dir.is_dir() {
                return Err(format!(
                    "schema directory does not exist: {}",
                    dir.display()
                ));
            }
            let mut paths = fs::read_dir(dir)
                .map_err(|e| format!("cannot read schema directory {}: {e}", dir.display()))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|v| v.to_str())
                        .is_some_and(|v| v.ends_with(".schema.yml") || v.ends_with(".schema.yaml"))
                })
                .collect::<Vec<_>>();
            paths.sort();
            if paths.is_empty() {
                return Err(format!("no *.schema.yml files found in {}", dir.display()));
            }
            for path in paths {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("schema");
                let body = fs::read_to_string(&path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                insert_schema(
                    name,
                    &body,
                    options.allow_schema_override,
                    &mut loaded,
                    &mut ids,
                )?;
            }
        }

        for item in loaded.values() {
            check_references(&item.schema, &item.name, &ids)?;
        }

        let holder = ids
            .keys()
            .find(|id| id.ends_with(COMMON_SCHEMA_SUFFIX))
            .and_then(|common| {
                let resources = ids
                    .iter()
                    .map(|(id, value)| {
                        Resource::from_contents(value.clone())
                            .ok()
                            .map(|resource| (id.clone(), resource))
                    })
                    .collect::<Option<Vec<_>>>()?;
                jsonschema::options()
                    .with_resources(resources.into_iter())
                    .build(&serde_json::json!({"$ref": format!("{common}#/$defs/conditionHolder")}))
                    .ok()
            });
        let mut validators = HashMap::new();
        let mut names = HashMap::new();
        let mut schemas = HashMap::new();
        for (kind, mut item) in loaded {
            // `contributer` is a repository policy, not a runtime requirement.
            if kind == "pack" {
                if let Some(required) = item
                    .schema
                    .get_mut("required")
                    .and_then(Value::as_array_mut)
                {
                    required.retain(|v| v.as_str() != Some("contributer"));
                }
            }
            let resources = ids
                .iter()
                .map(|(id, value)| {
                    Resource::from_contents(value.clone())
                        .map(|resource| (id.clone(), resource))
                        .map_err(|e| e.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let validator = jsonschema::options()
                .with_resources(resources.into_iter())
                .build(&item.schema)
                .map_err(|e| format!("invalid JSON Schema {}: {e}", item.name))?;
            names.insert(kind.clone(), item.name);
            schemas.insert(kind.clone(), item.schema);
            validators.insert(kind, validator);
        }
        Ok(Self {
            resources: ids,
            validators,
            names,
            schemas,
            holder,
            diagnostics,
            providers,
        })
    }

    pub fn providers(&self) -> &[ProviderDeclaration] {
        &self.providers
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn kinds(&self) -> Vec<String> {
        let mut kinds = self.validators.keys().cloned().collect::<Vec<_>>();
        kinds.sort();
        kinds
    }

    pub fn schema(&self, kind: &str) -> Option<&Value> {
        self.schemas.get(kind)
    }

    /// Effective local schema plus all resources needed to resolve its references offline.
    pub fn export(&self, kind: &str) -> Option<Value> {
        Some(
            serde_json::json!({"schemaVersion": crate::evidence::CONTRACT_VERSION,
            "kind": kind, "schema": self.schema(kind)?, "resources": self.resources}),
        )
    }

    pub fn validate(
        &self,
        kind: &str,
        value: &Value,
        path: &Path,
        index: usize,
        source: &SourceMap<'_>,
    ) -> Vec<Diagnostic> {
        self.validate_typed(kind, value, path, index, source, &HashSet::new())
            .0
    }

    /// Validates the lifted condition keys of one element (`conditions`, `conditionBehavior`,
    /// `conditionBehaivor`). `site` is the JSON pointer of the element; reported pointers include it.
    pub(crate) fn validate_holder(
        &self,
        holder: &Value,
        site: &str,
        path: &Path,
        index: usize,
        source: &SourceMap<'_>,
    ) -> Vec<Diagnostic> {
        let Some(validator) = &self.holder else {
            return Vec::new();
        };
        self.collect(
            validator,
            "common",
            holder,
            site,
            path,
            index,
            source,
            &HashSet::new(),
        )
        .0
    }

    /// Like [`Self::validate`]. `typed` holds the pointers of unquoted typed references: `type`
    /// errors there become [`TypeConflict`]s, `pattern`/`enum`/range errors are dropped.
    pub(crate) fn validate_typed(
        &self,
        kind: &str,
        value: &Value,
        path: &Path,
        index: usize,
        source: &SourceMap<'_>,
        typed: &HashSet<String>,
    ) -> (Vec<Diagnostic>, Vec<TypeConflict>) {
        let Some(validator) = self.validators.get(kind) else {
            return (Vec::new(), Vec::new());
        };
        self.collect(validator, kind, value, "", path, index, source, typed)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "shared by document and holder validation"
    )]
    fn collect(
        &self,
        validator: &Validator,
        kind: &str,
        value: &Value,
        prefix: &str,
        path: &Path,
        index: usize,
        source: &SourceMap<'_>,
        typed: &HashSet<String>,
    ) -> (Vec<Diagnostic>, Vec<TypeConflict>) {
        let mut diagnostics = Vec::new();
        let errors = validator.iter_errors(value).collect::<Vec<_>>();
        // Index precise error paths and their containing fields once. This keeps
        // large invalid arrays linear in error count rather than rescanning all
        // errors for every unevaluated field.
        let mut specifically_invalid_fields = HashSet::new();
        for child in &errors {
            if matches!(
                child.kind,
                ValidationErrorKind::UnevaluatedProperties { .. }
                    | ValidationErrorKind::AnyOf { .. }
                    | ValidationErrorKind::OneOfNotValid { .. }
                    | ValidationErrorKind::OneOfMultipleValid { .. }
            ) {
                continue;
            }
            let mut pointer = child.instance_path.as_str();
            while !pointer.is_empty() {
                specifically_invalid_fields.insert(pointer);
                pointer = pointer.rsplit_once('/').map_or("", |(parent, _)| parent);
            }
        }
        let mut conflicts = Vec::new();
        for error in &errors {
            let pointer = format!("{prefix}{}", error.instance_path);
            if !typed.is_empty() && typed.contains(&pointer) {
                match &error.kind {
                    ValidationErrorKind::Type { kind } => {
                        let expected = match kind {
                            jsonschema::error::TypeKind::Single(single) => vec![single.to_string()],
                            jsonschema::error::TypeKind::Multiple(set) => {
                                set.iter().map(|item| item.to_string()).collect()
                            }
                        };
                        conflicts.push(TypeConflict { pointer, expected });
                        continue;
                    }
                    ValidationErrorKind::Pattern { .. }
                    | ValidationErrorKind::Enum { .. }
                    | ValidationErrorKind::Constant { .. }
                    | ValidationErrorKind::Minimum { .. }
                    | ValidationErrorKind::Maximum { .. }
                    | ValidationErrorKind::ExclusiveMinimum { .. }
                    | ValidationErrorKind::ExclusiveMaximum { .. }
                    | ValidationErrorKind::MultipleOf { .. }
                    | ValidationErrorKind::MinLength { .. }
                    | ValidationErrorKind::MaxLength { .. }
                    | ValidationErrorKind::Format { .. } => continue,
                    _ => {}
                }
            }
            let keyword = error
                .schema_path
                .to_string()
                .split('/')
                .next_back()
                .unwrap_or("validation")
                .to_string();
            let unexpected = match &error.kind {
                ValidationErrorKind::AdditionalProperties { unexpected }
                | ValidationErrorKind::UnevaluatedProperties { unexpected } => Some(unexpected),
                _ => None,
            };
            if let Some(unexpected) = unexpected.filter(|fields| !fields.is_empty()) {
                for field in unexpected {
                    if pointer.is_empty()
                        && (DOCUMENT_BASE_FIELDS.contains(&field.as_str())
                            || self
                                .schemas
                                .get(kind)
                                .is_some_and(|schema| schema_declares_root_property(schema, field)))
                    {
                        continue;
                    }
                    let field_pointer =
                        format!("{pointer}/{}", field.replace('~', "~0").replace('/', "~1"));
                    // Draft 2020-12 drops evaluation annotations when a property's
                    // child schema fails. Keep that specific error instead of
                    // mislabeling the declared containing property as unknown.
                    // Only an actual declaration in this keyword's owning schema
                    // plus an independent child error authorizes this cleanup.
                    if matches!(
                        error.kind,
                        ValidationErrorKind::UnevaluatedProperties { .. }
                    ) && error
                        .schema_path
                        .as_str()
                        .rsplit_once('/')
                        .is_some_and(|(owner, _)| {
                            self.schemas
                                .get(kind)
                                .and_then(|schema| schema.pointer(owner))
                                .is_some_and(|schema| schema_declares_root_property(schema, field))
                        })
                        && specifically_invalid_fields
                            .contains(field_pointer.strip_prefix(prefix).unwrap_or(&field_pointer))
                    {
                        continue;
                    }
                    let message = if kind != "pack" && pointer.is_empty() && field == "priority" {
                        "document-level priority is ignored by KDataForge; set priority in pack.yml to control pack load order".to_owned()
                    } else {
                        format!("unexpected property {field:?}")
                    };
                    let mut diagnostic = Diagnostic::new(
                        Severity::Error,
                        format!("schema.{keyword}"),
                        message,
                        Some(path.to_path_buf()),
                        Some(index),
                        pointer_to_yaml_path(&field_pointer),
                        source.location(index, &field_pointer),
                        "schema",
                    );
                    diagnostic.schema = self.names.get(kind).cloned();
                    diagnostics.push(diagnostic);
                }
                continue;
            }
            let mut diagnostic = Diagnostic::new(
                Severity::Error,
                format!("schema.{keyword}"),
                match &error.kind {
                    ValidationErrorKind::Enum { options } => {
                        format!("{} is not one of {options}", error.instance)
                    }
                    _ => error.to_string(),
                },
                Some(path.to_path_buf()),
                Some(index),
                pointer_to_yaml_path(&pointer),
                source.location(index, &pointer),
                "schema",
            );
            diagnostic.schema = self.names.get(kind).cloned();
            diagnostics.push(diagnostic);
        }
        (diagnostics, conflicts)
    }
}

const DOCUMENT_BASE_FIELDS: [&str; 7] = [
    "$schema",
    "type",
    "debug",
    "conditionBehaivor",
    "conditionBehavior",
    "conditions",
    "include",
];

fn schema_declares_root_property(schema: &Value, field: &str) -> bool {
    if schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| properties.contains_key(field))
    {
        return true;
    }
    ["allOf", "anyOf", "oneOf"].iter().any(|composition| {
        schema
            .get(*composition)
            .and_then(Value::as_array)
            .is_some_and(|branches| {
                branches
                    .iter()
                    .any(|branch| schema_declares_root_property(branch, field))
            })
    })
}

/// Search known source and installed-game mod containers without walking pack trees.
fn discover_mod_dirs(root: &Path) -> std::collections::BTreeSet<PathBuf> {
    let mut dirs = std::collections::BTreeSet::new();
    for ancestor in root.ancestors() {
        if is_mod_root(ancestor) {
            dirs.insert(ancestor.join("Resources/KDataForge"));
        }
        let mut workspace = false;
        for container in ["Mods", "Mods/GameFeatures", "FactoryGame/Mods", "tools"] {
            if let Ok(entries) = fs::read_dir(ancestor.join(container)) {
                workspace = true;
                for entry in entries.flatten() {
                    if is_mod_root(&entry.path()) {
                        dirs.insert(entry.path().join("Resources/KDataForge"));
                    }
                }
            }
        }
        // A nested workspace owns its schema scope; never inherit an outer checkout's mods.
        if workspace {
            break;
        }
    }
    dirs
}

fn is_mod_root(path: &Path) -> bool {
    fs::read_dir(path).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry.path().is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("uplugin"))
        })
    })
}

fn discover_mod_dirs_for_roots(roots: &[&Path]) -> std::collections::BTreeSet<PathBuf> {
    let mut dirs = std::collections::BTreeSet::new();
    for root in roots {
        dirs.extend(discover_mod_dirs(root));
    }
    dirs
}

fn discover_mod_schemas_for_roots(roots: &[&Path]) -> Vec<PathBuf> {
    let mut paths = std::collections::BTreeSet::new();
    for dir in discover_mod_dirs_for_roots(roots) {
        if let Ok(entries) = fs::read_dir(dir.join("Schemas")) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file()
                    && path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| s.ends_with(".schema.yml") || s.ends_with(".schema.yaml"))
                {
                    paths.insert(dunce::canonicalize(&path).unwrap_or(path));
                }
            }
        }
    }
    paths.into_iter().collect()
}

fn load_providers_for_roots(
    roots: &[&Path],
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<ProviderDeclaration>, String> {
    let mut files = std::collections::BTreeSet::new();
    for dir in discover_mod_dirs_for_roots(roots) {
        let path = dir.join("providers.yml");
        if path.is_file() {
            files.insert(dunce::canonicalize(&path).unwrap_or(path));
        }
    }
    let mut owners = HashMap::<String, PathBuf>::new();
    let mut result = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let file: ProviderFile = serde_yaml::from_str(&text)
            .map_err(|e| format!("invalid provider declaration {}: {e}", path.display()))?;
        for provider in file.providers {
            if provider.prefix.is_empty()
                || !provider
                    .prefix
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                || provider.prefix.split('.').any(|segment| {
                    segment.is_empty()
                        || !segment
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_')
                })
                || provider.description.trim().is_empty()
                || provider
                    .value_type
                    .as_deref()
                    .is_some_and(|v| !["bool", "number", "string"].contains(&v))
            {
                return Err(format!(
                    "invalid provider declaration in {}",
                    path.display()
                ));
            }
            let builtin = ["cfg", "session", "ags", "option"].contains(&provider.prefix.as_str());
            let previous = owners.get(&provider.prefix);
            if builtin || previous.is_some() {
                let code = if builtin {
                    "provider.builtin-shadow"
                } else {
                    "provider.duplicate-prefix"
                };
                let other = previous.map_or_else(
                    || "built-in provider".to_owned(),
                    |p| p.display().to_string(),
                );
                diagnostics.push(Diagnostic::new(
                    Severity::Error,
                    code,
                    format!(
                        "provider {:?} in {} conflicts with {}; first declaration wins",
                        provider.prefix,
                        path.display(),
                        other
                    ),
                    Some(path.clone()),
                    None,
                    "$",
                    crate::Range::default(),
                    "schema",
                ));
                continue;
            }
            owners.insert(provider.prefix.clone(), path.clone());
            result.push(provider);
        }
    }
    Ok(result)
}

fn insert_schema(
    name: &str,
    body: &str,
    allow_override: bool,
    loaded: &mut HashMap<String, Loaded>,
    ids: &mut HashMap<String, Value>,
) -> Result<(), String> {
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(body).map_err(|e| format!("invalid schema YAML {name}: {e}"))?;
    let schema: Value =
        serde_json::to_value(yaml).map_err(|e| format!("invalid schema {name}: {e}"))?;
    let id = schema
        .get("$id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("schema requires non-empty $id: {name}"))?
        .to_owned();
    let kind = schema
        .get("x-kdf-type")
        .and_then(Value::as_str)
        .map(|s| s.to_lowercase());
    if ids.contains_key(&id) && !allow_override {
        return Err(format!("schema id {id} from {name} duplicates an already loaded schema; use --allow-schema-override"));
    }
    if let Some(kind) = &kind {
        if kind.is_empty() {
            return Err(format!("x-kdf-type must be a non-empty string: {name}"));
        }
        if loaded.contains_key(kind) && !allow_override {
            return Err(format!("schema type {kind} from {name} duplicates an already loaded schema; use --allow-schema-override"));
        }
    }
    ids.insert(id.clone(), schema.clone());
    if let Some(kind) = kind {
        if kind != "mam" {
            loaded.insert(
                kind,
                Loaded {
                    name: name.to_owned(),
                    schema,
                },
            );
        }
    }
    Ok(())
}

fn check_references(value: &Value, name: &str, ids: &HashMap<String, Value>) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                let base = reference.split('#').next().unwrap_or("");
                if !base.is_empty() && !ids.contains_key(base) {
                    return Err(format!(
                        "schema {name} references unavailable or network schema: {reference}"
                    ));
                }
            }
            for child in map.values() {
                check_references(child, name, ids)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                check_references(child, name, ids)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn pointer_to_yaml_path(pointer: &str) -> String {
    let mut output = String::from("$");
    for part in pointer.split('/').skip(1) {
        let decoded = part.replace("~1", "/").replace("~0", "~");
        if decoded.parse::<usize>().is_ok() {
            output.push_str(&format!("[{decoded}]"));
        } else {
            output.push('.');
            output.push_str(&decoded);
        }
    }
    output
}

pub fn infer_type(path: &Path, document: &Value, multi: bool) -> Option<String> {
    if let Some(raw) = document.get("type").and_then(Value::as_str) {
        if !raw.is_empty() {
            let kind = raw.trim().to_lowercase();
            return Some(if kind == "mam" {
                "research".into()
            } else {
                kind
            });
        }
    }
    if multi {
        return None;
    }
    let filename = path.file_name()?.to_str()?.to_lowercase();
    let extension = filename
        .strip_suffix(".yml")
        .or_else(|| filename.strip_suffix(".yaml"))?;
    let (name, kind) = extension.rsplit_once('.')?;
    if name.is_empty() || kind.is_empty() {
        return None;
    }
    if kind == "mam" {
        return Some("research".into());
    }
    Some(kind.to_owned())
}

pub fn path_within(path: &Path, root: &Path) -> bool {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                clean.pop();
            }
            std::path::Component::CurDir => {}
            other => clean.push(other.as_os_str()),
        }
    }
    clean.starts_with(root)
}
