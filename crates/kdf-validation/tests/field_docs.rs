//! Tests of the field documentation catalog (`data/field-docs.yml`).
//!
//! The catalog is checked against the bundled schemas: every property path of every schema has a
//! documentation entry, every example is valid YAML and every example that is a complete
//! document passes the document validation of this crate.

use kdf_validation::field_docs::{
    self, entries, field, load_error, prefix, to_markdown, value, value_at, DocEntry, Example,
    DOCS_BASE,
};
use kdf_validation::{validate_document, LintOptions, Severity, CANONICAL_TYPES, ROOT_TYPES};
use serde::Deserialize;
use serde_yaml::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

// ---- schema walking ------------------------------------------------------------------------------

struct Schemas {
    files: BTreeMap<String, Value>,
}

#[derive(Default)]
struct Walk {
    paths: BTreeSet<Vec<String>>,
    enums: Vec<(Vec<String>, Vec<String>)>,
}

impl Schemas {
    /// Loads `*.schema.yml`. `KDF_FIELD_DOCS_SCHEMA_DIR` points the test at another schema set,
    /// for example the schemas of a branch that adds a document kind.
    fn load() -> Self {
        let dir = std::env::var_os("KDF_FIELD_DOCS_SCHEMA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas"));
        let mut files = BTreeMap::new();
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name.ends_with(".schema.yml") {
                let text = std::fs::read_to_string(&path).unwrap();
                let schema: Value = serde_yaml::from_str(&text)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                files.insert(name, schema);
            }
        }
        assert!(!files.is_empty(), "no schemas in {}", dir.display());
        Self { files }
    }

    /// Resolves `file#/json/pointer` (URL, bare file name or `#/pointer`) from the file `from`.
    fn resolve(&self, from: &str, reference: &str) -> (String, &Value) {
        let (file, pointer) = reference.split_once('#').unwrap_or((reference, ""));
        let file = if file.is_empty() {
            from.to_owned()
        } else {
            file.rsplit('/').next().unwrap().to_owned()
        };
        let mut node = self
            .files
            .get(&file)
            .unwrap_or_else(|| panic!("schema {file} referenced from {from} is missing"));
        for segment in pointer.split('/').filter(|segment| !segment.is_empty()) {
            let segment = segment.replace("~1", "/").replace("~0", "~");
            node = node
                .get(segment.as_str())
                .unwrap_or_else(|| panic!("{reference}: no `{segment}` in {file}"));
        }
        (file, node)
    }

    /// Every property path below the root of `file`; `*` stands for arbitrary map keys.
    fn paths(&self, file: &str) -> BTreeSet<Vec<String>> {
        self.walk(file).paths
    }

    /// Every string `enum` of `file` with the property path it constrains.
    fn enums(&self, file: &str) -> Vec<(Vec<String>, Vec<String>)> {
        self.walk(file).enums
    }

    fn walk(&self, file: &str) -> Walk {
        let mut walk = Walk::default();
        let root = self.files.get(file).unwrap();
        self.collect(file, root, &mut Vec::new(), &mut walk, 0);
        walk
    }

    fn collect(
        &self,
        file: &str,
        node: &Value,
        path: &mut Vec<String>,
        walk: &mut Walk,
        depth: usize,
    ) {
        assert!(depth < 60, "schema recursion at {path:?}");
        if !node.is_mapping() {
            return;
        }
        if let Some(values) = node.get("enum").and_then(Value::as_sequence) {
            let values: Vec<String> = values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect();
            if !path.is_empty() && !values.is_empty() {
                walk.enums.push((path.clone(), values));
            }
        }
        if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
            let (target_file, target) = self.resolve(file, reference);
            self.collect(&target_file, target, path, walk, depth + 1);
        }
        if let Some(properties) = node.get("properties").and_then(Value::as_mapping) {
            for (key, sub) in properties {
                path.push(key.as_str().unwrap().to_owned());
                walk.paths.insert(path.clone());
                self.collect(file, sub, path, walk, depth + 1);
                path.pop();
            }
        }
        for keyword in ["items", "then", "else"] {
            if let Some(sub) = node.get(keyword) {
                self.collect(file, sub, path, walk, depth + 1);
            }
        }
        for keyword in ["allOf", "anyOf", "oneOf"] {
            if let Some(list) = node.get(keyword).and_then(Value::as_sequence) {
                for sub in list {
                    self.collect(file, sub, path, walk, depth + 1);
                }
            }
        }
        // A schema (not `false`) for additional properties documents arbitrary map keys.
        if let Some(sub) = node.get("additionalProperties") {
            if sub.is_mapping() {
                path.push("*".to_owned());
                walk.paths.insert(path.clone());
                self.collect(file, sub, path, walk, depth + 1);
                path.pop();
            }
        }
    }

    /// `(document kind, schema file)` for every schema that declares an `x-kdf-type`.
    fn kinds(&self) -> Vec<(String, String)> {
        self.files
            .iter()
            .filter_map(|(file, schema)| {
                schema
                    .get("x-kdf-type")
                    .and_then(Value::as_str)
                    .map(|kind| (kind.to_owned(), file.clone()))
            })
            .collect()
    }
}

fn dotted(path: &[String]) -> String {
    path.join(".")
}

fn refs(path: &[String]) -> Vec<&str> {
    path.iter().map(String::as_str).collect()
}

// ---- catalog helpers ---------------------------------------------------------------------------------

const KNOWN_PAGES: &[&str] = &[
    "features/actor-selectors",
    "features/category-reorder",
    "features/event-rules",
    "features/resource-node-placement",
    "features/resource-node-visuals",
    "features/buildings",
    "features/conditions-and-packs",
    "features/curves",
    "features/custom-classes",
    "features/data-assets",
    "features/game-tags",
    "features/index",
    "features/items",
    "features/localization",
    "features/operations",
    "features/pack-configuration",
    "features/patching-cdo",
    "features/property-types",
    "features/recipes",
    "features/removing-content",
    "features/research-trees",
    "features/resources",
    "features/schematics",
    "features/sinkpoints",
    "features/sounds",
    "features/sublevels",
    "features/targeting",
    "features/templates",
    "features/unlocks",
    "reference/api-design",
    "reference/availability-dependency-classes",
    "reference/base-game-unlock-classes",
    "reference/blueprint-config-api",
    "reference/config-expressions",
    "reference/editor-integration",
    "reference/handler-template",
    "reference/index",
    "reference/mod-unlock-classes",
    "reference/reflection-export",
    "reference/yaml-schema",
    "getting-started",
    "lint-your-pack",
];

fn describe(entry: &DocEntry) -> String {
    format!("{}.{}.{}", entry.section, entry.scope, entry.key)
}

/// Root types of the documents in `yaml` (documents without a `type` yield `None`).
fn document_types(yaml: &str) -> Result<Vec<Option<String>>, String> {
    let mut types = Vec::new();
    for document in serde_yaml::Deserializer::from_str(yaml) {
        let value = Value::deserialize(document).map_err(|error| error.to_string())?;
        types.push(value.get("type").and_then(Value::as_str).map(str::to_owned));
    }
    Ok(types)
}

fn slugify(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

fn heading_slugs(page: &Path) -> HashSet<String> {
    let text = std::fs::read_to_string(page).unwrap();
    text.lines()
        .filter(|line| line.starts_with('#'))
        .map(|line| slugify(line.trim_start_matches('#')))
        .collect()
}

// ---- tests --------------------------------------------------------------------------------------------

#[test]
fn catalog_parses() {
    assert_eq!(load_error(), None);
    let all = entries();
    assert!(all.len() > 300, "only {} entries", all.len());
    let sections: BTreeSet<_> = all.iter().map(|entry| entry.section).collect();
    assert_eq!(
        sections.into_iter().collect::<Vec<_>>(),
        ["common", "defs", "kinds", "prefixes", "values"]
    );
}

#[test]
fn every_entry_is_complete() {
    let mut problems = Vec::new();
    for entry in entries() {
        let doc = entry.doc;
        let name = describe(&entry);
        if doc.summary.trim().is_empty() {
            problems.push(format!("{name}: empty summary"));
        }
        if doc.summary.contains('\n') {
            problems.push(format!("{name}: summary has several lines"));
        }
        if doc.description.trim().is_empty() {
            problems.push(format!("{name}: empty description"));
        }
        if doc.type_label.is_empty() {
            problems.push(format!("{name}: no type label"));
        }
        if doc.examples.is_empty() || doc.examples.len() > 2 {
            problems.push(format!(
                "{name}: {} examples (1-2 expected)",
                doc.examples.len()
            ));
        }
        for example in &doc.examples {
            if example.title.trim().is_empty() {
                problems.push(format!("{name}: example without title"));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_schema_property_is_documented() {
    let schemas = Schemas::load();
    let mut missing = Vec::new();
    let mut checked = 0;
    for (kind, file) in schemas.kinds() {
        for path in schemas.paths(&file) {
            checked += 1;
            if field(&kind, &refs(&path)).is_none() {
                missing.push(format!("{kind}: {}", dotted(&path)));
            }
        }
    }
    assert!(checked > 500, "walked only {checked} paths");
    assert!(
        missing.is_empty(),
        "{} undocumented schema paths:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

/// Every catalog path of a schema-backed kind exists in its schema, so typos do not hide.
#[test]
fn kind_entries_match_the_schemas() {
    // Keys of the pack configuration branch (`pack.yml` icon, localized description) are
    // documented before that branch is merged; the schema walk picks them up afterwards.
    let ahead_of_schema: BTreeSet<(&str, &str)> = [
        ("pack", "icon"),
        ("pack", "description.key"),
        ("pack", "description.ns"),
        ("pack", "description.source"),
    ]
    .into_iter()
    .collect();
    let schemas = Schemas::load();
    let by_kind: HashMap<String, BTreeSet<String>> = schemas
        .kinds()
        .into_iter()
        .map(|(kind, file)| {
            let paths = schemas.paths(&file).iter().map(|p| dotted(p)).collect();
            (kind, paths)
        })
        .collect();
    let mut stray = Vec::new();
    for entry in entries() {
        if entry.section != "kinds" {
            continue;
        }
        let Some(paths) = by_kind.get(&entry.scope) else {
            // `config` has no schema in this build; `pack_configuration_paths_are_documented`
            // covers it until the pack configuration branch is merged.
            if entry.scope != "config" {
                stray.push(format!("{}: kind has no schema", describe(&entry)));
            }
            continue;
        };
        if !paths.contains(&entry.key)
            && !ahead_of_schema.contains(&(entry.scope.as_str(), entry.key.as_str()))
        {
            stray.push(format!("{}: not a schema path", describe(&entry)));
        }
    }
    assert!(stray.is_empty(), "{}", stray.join("\n"));
}

/// The pack configuration kind is not part of this branch's schemas yet. These are the paths of
/// `config.schema.yml` of the `agents/pack-config` branch; once that schema is merged the walk in
/// `every_schema_property_is_documented` covers them as well.
const CONFIG_PATHS: &[&str] = &[
    "type",
    "category",
    "debug",
    "conditions",
    "conditions.config",
    "conditionBehavior",
    "conditionBehaivor",
    "include",
    "fields",
    "fields.id",
    "fields.type",
    "fields.valueType",
    "fields.name",
    "fields.name.key",
    "fields.name.ns",
    "fields.name.source",
    "fields.description",
    "fields.description.key",
    "fields.description.ns",
    "fields.description.source",
    "fields.tooltip",
    "fields.tooltip.title",
    "fields.tooltip.title.key",
    "fields.tooltip.title.ns",
    "fields.tooltip.title.source",
    "fields.tooltip.text",
    "fields.tooltip.text.key",
    "fields.tooltip.text.ns",
    "fields.tooltip.text.source",
    "fields.category",
    "fields.default",
    "fields.apply",
    "fields.order",
    "fields.multiple",
    "fields.min",
    "fields.max",
    "fields.step",
    "fields.decimals",
    "fields.display",
    "fields.multiline",
    "fields.maxLength",
    "fields.tagRoot",
    "fields.options",
    "fields.options.value",
    "fields.options.label",
    "fields.options.label.key",
    "fields.options.label.ns",
    "fields.options.label.source",
    "fields.minSelected",
    "fields.maxSelected",
    "fields.minCount",
    "fields.maxCount",
    "fields.descriptionClass",
    "fields.properties",
];

#[test]
fn pack_configuration_paths_are_documented() {
    let missing: Vec<_> = CONFIG_PATHS
        .iter()
        .filter(|path| {
            let segments: Vec<_> = path.split('.').collect();
            field("config", &segments).is_none()
        })
        .collect();
    assert!(missing.is_empty(), "undocumented config paths: {missing:?}");
    for path in [
        "pack: icon",
        "pack: description.key",
        "pack: conditions.config",
        "cdo: patches.properties.conditions",
        "cdo: patches.properties.conditions.config",
    ] {
        let (kind, dotted) = path.split_once(": ").unwrap();
        let segments: Vec<_> = dotted.split('.').collect();
        assert!(field(kind, &segments).is_some(), "{path} is not documented");
    }
}

#[test]
fn every_operation_has_a_value_doc() {
    let schemas = Schemas::load();
    let (_, operations) = schemas.resolve(
        "common.schema.yml",
        "common.schema.yml#/$defs/operation/properties/op/enum",
    );
    let operations: Vec<_> = operations
        .as_sequence()
        .unwrap()
        .iter()
        .map(|op| op.as_str().unwrap())
        .collect();
    assert_eq!(operations.len(), 25);
    let missing: Vec<_> = operations
        .iter()
        .filter(|op| value("op", op).is_none())
        .collect();
    assert!(missing.is_empty(), "operations without docs: {missing:?}");
}

/// Every enum a schema constrains has a documentation entry per value, reachable from the key.
#[test]
fn every_schema_enum_value_is_documented() {
    let schemas = Schemas::load();
    let mut missing = BTreeSet::new();
    let mut checked = 0;
    for (kind, file) in schemas.kinds() {
        for (path, values) in schemas.enums(&file) {
            for value in values {
                checked += 1;
                if value_at(&kind, &refs(&path), &value).is_none() {
                    missing.insert(format!("{kind}: {} = {value}", dotted(&path)));
                }
            }
        }
    }
    assert!(checked > 50, "checked only {checked} enum values");
    assert!(
        missing.is_empty(),
        "{} undocumented enum values:
{}",
        missing.len(),
        missing.into_iter().collect::<Vec<_>>().join(
            "
"
        )
    );
}

#[test]
fn every_document_type_has_a_value_doc() {
    let mut types: Vec<&str> = CANONICAL_TYPES.to_vec();
    types.extend(["mam", "config"]);
    let missing: Vec<_> = types
        .iter()
        .filter(|kind| value("type", kind).is_none())
        .collect();
    assert!(missing.is_empty(), "types without docs: {missing:?}");
    // The pack manifest has no `type`, and every root type except `pack` is documented.
    for kind in ROOT_TYPES.iter().filter(|kind| **kind != "pack") {
        assert!(value("type", kind).is_some(), "{kind}");
    }
    for behavior in ["AND", "OR", "and", "Or"] {
        assert!(value("conditionBehavior", behavior).is_some(), "{behavior}");
        assert!(value("conditionBehaivor", behavior).is_some(), "{behavior}");
    }
}

#[test]
fn value_groups_exist() {
    let groups: BTreeSet<String> = entries()
        .iter()
        .filter(|entry| entry.section == "values")
        .map(|entry| entry.scope.clone())
        .collect();
    let mut missing = Vec::new();
    for entry in entries() {
        if let Some(group) = &entry.doc.values {
            if !groups.contains(group) {
                missing.push(format!("{} -> {group}", describe(&entry)));
            }
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn documentation_links_point_to_existing_pages() {
    let docs_dir = std::env::var_os("KDF_DOCS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:/Modding/KModsDocs/content/docs/kdataforge"));
    let live = docs_dir.is_dir();
    let mut slugs: HashMap<String, HashSet<String>> = HashMap::new();
    let mut problems = Vec::new();
    for entry in entries() {
        let Some(link) = &entry.doc.docs else {
            continue;
        };
        let Some(rest) = link.strip_prefix(DOCS_BASE) else {
            problems.push(format!(
                "{}: {link} is not below {DOCS_BASE}",
                describe(&entry)
            ));
            continue;
        };
        let (page, anchor) = rest.split_once('#').unwrap_or((rest, ""));
        if page.ends_with(".mdx") || page.ends_with('/') || page.contains("//") {
            problems.push(format!(
                "{}: {link} is not an extensionless page",
                describe(&entry)
            ));
        }
        if !KNOWN_PAGES.contains(&page) {
            problems.push(format!("{}: unknown page {link}", describe(&entry)));
            continue;
        }
        if live {
            let file = docs_dir.join(format!("{page}.mdx"));
            if !file.is_file() {
                problems.push(format!(
                    "{}: {} does not exist",
                    describe(&entry),
                    file.display()
                ));
            } else if !anchor.is_empty() {
                let known = slugs
                    .entry(page.to_owned())
                    .or_insert_with(|| heading_slugs(&file));
                if !known.contains(anchor) {
                    problems.push(format!(
                        "{}: no heading for #{anchor} in {page}",
                        describe(&entry)
                    ));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_example_is_valid_yaml() {
    let mut problems = Vec::new();
    for entry in entries() {
        for example in &entry.doc.examples {
            if let Err(error) = document_types(&example.yaml) {
                problems.push(format!("{} [{}]: {error}", describe(&entry), example.title));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn complete_examples_validate_without_errors() {
    let mut problems = Vec::new();
    let mut validated = 0;
    let mut parse_only = 0;
    for entry in entries() {
        for example in &entry.doc.examples {
            if example.partial {
                parse_only += 1;
                continue;
            }
            // Complete examples use the runtime filename contract; fragments stay parse-only.
            let types = document_types(&example.yaml).unwrap();
            let is_manifest = entry.section == "kinds" && entry.scope == "pack";
            if !is_manifest
                && types.iter().any(|kind| {
                    kind.as_deref()
                        .is_none_or(|kind| !ROOT_TYPES.contains(&kind))
                })
            {
                parse_only += 1;
                continue;
            }
            let name = if is_manifest {
                "pack.yml"
            } else if types.iter().any(|kind| kind.as_deref() == Some("config")) {
                "example.config.yml"
            } else {
                "example.yml"
            };
            let result = validate_document(Path::new(name), &example.yaml, &LintOptions::default())
                .unwrap_or_else(|error| panic!("{}: {error}", describe(&entry)));
            validated += 1;
            for diagnostic in result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == Severity::Error)
            {
                problems.push(format!(
                    "{} [{}]: {} {} at {}",
                    describe(&entry),
                    example.title,
                    diagnostic.code,
                    diagnostic.message,
                    diagnostic.yaml_path
                ));
            }
        }
    }
    assert!(
        validated > 250,
        "validated only {validated} examples ({parse_only} parse-only)"
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

// ---- lookups ----------------------------------------------------------------------------------------------

/// The example file of the user: `type: cdo` with a refinery patch.
#[test]
fn lookups_for_a_cdo_patch() {
    let doc = |path: &[&str]| field("cdo", path).unwrap_or_else(|| panic!("no doc for {path:?}"));
    assert_eq!(
        doc(&["type"]).summary,
        field("recipe", &["type"]).unwrap().summary
    );
    assert!(doc(&["type"])
        .summary
        .starts_with("Root type of the document"));
    assert!(doc(&["patches"]).summary.starts_with("Patches"));
    assert!(doc(&["patches", "producedIn"])
        .summary
        .contains("crafted in the listed building classes"));
    assert!(doc(&["patches", "properties"])
        .summary
        .starts_with("Operations applied"));
    assert!(doc(&["patches", "properties", "path"])
        .summary
        .contains("Property"));
    assert!(doc(&["patches", "properties", "op"])
        .summary
        .contains("combines"));
    assert!(doc(&["patches", "properties", "value"])
        .summary
        .starts_with("Operand"));

    let producedin = doc(&["patches", "producedIn"]);
    assert!(producedin.examples[0].yaml.contains("producedIn:"));
    assert_eq!(producedin.type_label, "class path | list");

    let op = value("op", "multiply").unwrap();
    assert!(op.summary.contains("Multiplies"));
    assert_eq!(
        value_at("cdo", &["patches", "properties", "op"], "multiply"),
        Some(op)
    );
    assert_eq!(value_at("cdo", &["type"], "cdo"), value("type", "cdo"));
    assert!(value("op", "frobnicate").is_none());
}

#[test]
fn lookups_share_definitions_between_kinds() {
    // The operation list of every document kind resolves through `defs.operation`.
    let shared = field("cdo", &["patches", "properties", "op"]).unwrap();
    for (kind, path) in [
        ("item", vec!["items", "properties", "op"]),
        ("recipe", vec!["recipes", "properties", "op"]),
        (
            "schematic",
            vec!["schematics", "unlocks", "properties", "op"],
        ),
        ("research", vec!["research", "addNodes", "properties", "op"]),
        ("mam", vec!["research", "addNodes", "properties", "op"]),
        ("dataasset", vec!["assets", "properties", "op"]),
        ("class", vec!["classes", "properties", "op"]),
    ] {
        assert_eq!(field(kind, &path), Some(shared), "{kind} {path:?}");
    }
    // A kind override wins over the shared structure: the default parent differs per type.
    assert!(field("item", &["items", "parent"])
        .unwrap()
        .default
        .as_deref()
        .unwrap()
        .ends_with("FGItemDescriptor"));
    assert!(field("recipe", &["recipes", "parent"])
        .unwrap()
        .default
        .as_deref()
        .unwrap()
        .ends_with("FGRecipe"));
}

#[test]
fn lookups_cover_wildcards_conditions_and_aliases() {
    assert!(field(
        "localization",
        &["entries", "Config_IronSpeed_Name", "text"]
    )
    .is_some());
    assert!(field("pack", &["redirects", "OldItemId"]).is_some());
    assert!(field("cdo", &["conditions", "modVersion", "SML"]).is_some());
    // Element conditions on any list entry, at any depth.
    let entry = field("cdo", &["patches", "properties", "conditions"]).unwrap();
    assert!(entry.summary.contains("list entry"));
    assert_ne!(entry, field("cdo", &["conditions"]).unwrap());
    assert_eq!(
        field("cdo", &["patches", "properties", "conditions", "config"]),
        field("cdo", &["conditions", "config"])
    );
    // Pack manifest conditions have their own text.
    assert!(field("pack", &["conditions"])
        .unwrap()
        .summary
        .contains("whole pack"));
    // Unknown paths and kinds.
    assert!(field("cdo", &["patches", "nonsense"]).is_none());
    assert!(field("cdo", &[]).is_none());
    assert!(field("nonsense", &["patches"]).is_none());
    assert!(field("nonsense", &["type"]).is_some());
    // Value group chosen by the key path, not just the last key.
    assert_eq!(
        value_at("config", &["fields", "type"], "select"),
        value("config.type", "select")
    );
    assert!(value_at("config", &["fields", "type"], "select")
        .unwrap()
        .summary
        .contains("options"));
    assert_eq!(
        value_at("curve", &["curves", "postInfinityExtrap"], "linear"),
        value("extrapolation", "linear")
    );
}

#[test]
fn prefix_lookups_accept_every_spelling() {
    let cfg = prefix("cfg:").unwrap();
    assert!(cfg.summary.contains("Typed reference"));
    for token in [
        "cfg",
        "cfg:balance.ironSpeed",
        "cfg:Other:shared.level",
        " cfg: ",
    ] {
        assert_eq!(prefix(token), Some(cfg), "{token}");
    }
    let template = prefix("{cfg:").unwrap();
    assert!(template.summary.contains("Template"));
    assert_ne!(template, cfg);
    for token in ["{cfg:}", "{cfg:balance.ironSpeed}", "{cfg"] {
        assert_eq!(prefix(token), Some(template), "{token}");
    }
    for provider in [
        "{class:",
        "{item:}",
        "{resource:",
        "{building:",
        "{recipe:",
        "{schematic:",
        "{research:",
        "{category:",
        "{dataasset:",
        "{asset:}",
    ] {
        assert!(prefix(provider).is_some(), "{provider}");
    }
    for typed in ["session:", "ags:", "option:"] {
        assert!(prefix(typed).is_some(), "{typed}");
    }
    assert_eq!(prefix("item"), prefix("{item:"));
    assert!(prefix(r"\{").is_some());
    assert!(prefix("{nonsense:").is_none());
    assert!(prefix("").is_none());
}

#[test]
fn markdown_of_a_value_doc() {
    let markdown = to_markdown(value("op", "multiply").unwrap());
    let expected = r#"**Multiplies a numeric property by `value`.**

Type: `op`

Scales the stored number: `0.5` halves a crafting time. Enum-backed integers are rejected.

*Halve a crafting time*

```yaml
type: cdo
patches:
  - target: /Game/FactoryGame/Recipes/Smelter/Recipe_IngotIron.Recipe_IngotIron_C
    properties:
      - path: mManufactoringDuration
        op: multiply
        value: 0.5
```

[Documentation](https://docs.k-mods.com/kdataforge/features/operations#multiply--divide-numeric)"#;
    assert_eq!(markdown, expected);
}

#[test]
fn markdown_of_a_field_doc_lists_type_required_and_default() {
    let markdown = to_markdown(field("cdo", &["patches", "applyToSubclasses"]).unwrap());
    let (head, rest) = markdown.split_once("\n\n").unwrap();
    assert_eq!(
        head,
        "**Also patches every class derived from the target.**"
    );
    let (facts, rest) = rest.split_once("\n\n").unwrap();
    assert_eq!(facts, "Type: `bool` | Default: `false`");
    assert!(rest.starts_with("With a class target"));
    assert!(markdown.contains("\n\n*Every extractor*\n\n```yaml\ntype: cdo\n"));
    assert!(markdown.ends_with(
        "(https://docs.k-mods.com/kdataforge/features/targeting#applytosubclasses-widen-a-class-target-to-every-derived-class)"
    ));
    assert!(!markdown.ends_with('\n'));

    let required = to_markdown(field("cdo", &["patches", "properties", "path"]).unwrap());
    assert!(
        required.contains("\n\nType: `property path` | Required: yes\n\n"),
        "{required}"
    );
}

#[test]
fn markdown_leaves_out_unset_parts() {
    let bare = field_docs::FieldDoc {
        summary: "Only a summary".to_owned(),
        ..Default::default()
    };
    assert_eq!(to_markdown(&bare), "**Only a summary**");
    let with_example = field_docs::FieldDoc {
        summary: "S".to_owned(),
        description: "D".to_owned(),
        examples: vec![Example {
            title: "T".to_owned(),
            yaml: "a: 1\n".to_owned(),
            partial: true,
        }],
        ..Default::default()
    };
    assert_eq!(
        to_markdown(&with_example),
        "**S**\n\nD\n\n*T*\n\n```yaml\na: 1\n```"
    );
}
