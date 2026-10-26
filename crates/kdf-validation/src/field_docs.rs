//! Field documentation catalog for the editor hover.
//!
//! The catalog lives in `data/field-docs.yml` (embedded at build time and parsed once). It
//! describes every YAML key of every KDataForge document kind, the values of the keys that take a
//! fixed set of values (`op`, `type`, ...) and the typed-reference / template prefixes
//! (`cfg:`, `{item:`, ...).
//!
//! # Lookup
//!
//! * [`field`] resolves a YAML key path. The path contains the map keys only, list indexes are
//!   already removed: `patches[0].properties[2].op` is `["patches", "properties", "op"]`.
//! * [`value`] / [`value_at`] resolve the documentation of a single value such as `op: multiply`.
//! * [`prefix`] resolves a typed-reference or template prefix such as `cfg:` or `{item:`.
//! * [`to_markdown`] renders any of them as hover markdown.
//!
//! # Resolution order of [`field`]
//!
//! 1. The table of the document kind: an exact path, then the shared structures (`defs`) the kind
//!    attaches to a path through `uses`.
//! 2. Element conditions: `conditions`, `conditionBehavior` and `conditionBehaivor` are allowed on
//!    every map entry of every list, so a path that contains one of them below the document root
//!    resolves to the entry-condition documentation (and to the shared `conditions.*` keys).
//! 3. The `common` table that every kind shares (`type`, `debug`, `include`, `conditions`, ...).
//!
//! The kind `mam` is an alias of `research`.

use serde::{Deserialize, Deserializer, Serialize};
use serde_yaml::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use crate::ROOT_TYPES;

const CATALOG_SOURCE: &str = include_str!("../data/field-docs.yml");

/// Base of every documentation link in the catalog.
pub const DOCS_BASE: &str = "https://docs.k-mods.com/kdataforge/";

/// Separator of the normalised path keys, a character no YAML key of a document contains.
const KEY_SEPARATOR: char = '\u{1f}';

/// Longest chain of `uses` references followed while resolving one path.
const MAX_USES_DEPTH: usize = 8;

/// Keys that may sit on any map entry of any list (element conditions).
const CONDITION_KEYS: [&str; 3] = ["conditions", "conditionBehavior", "conditionBehaivor"];

/// Value groups reachable under a second YAML key.
const VALUE_GROUP_ALIASES: [(&str, &str); 4] = [
    ("conditionBehaivor", "conditionBehavior"),
    ("preInfinityExtrap", "extrapolation"),
    ("postInfinityExtrap", "extrapolation"),
    ("nodeTypes", "nodeType"),
];

/// One example of a documented key, value or prefix.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Example {
    /// Short caption shown above the snippet.
    pub title: String,
    /// The YAML snippet.
    pub yaml: String,
    /// `true` when the snippet is only a fragment of a document (parsed as YAML, not validated).
    pub partial: bool,
}

/// Documentation of one field, value or prefix.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct FieldDoc {
    /// One line that says what the key does.
    pub summary: String,
    /// One to four sentences with the details.
    pub description: String,
    /// Short type label such as `string | list`, `class path` or `operation list`.
    pub type_label: String,
    /// Set when the key is required; the text names the condition (`yes`, `yes, unless ...`).
    pub required: Option<String>,
    /// Value used when the key is omitted, when the schema or the docs define one.
    pub default: Option<String>,
    /// Short, valid YAML snippets.
    pub examples: Vec<Example>,
    /// Absolute link to the user documentation page.
    pub docs: Option<String>,
    /// Name of the value group that documents the values of this key (see [`value_at`]).
    pub values: Option<String>,
}

/// One catalog entry as listed by [`entries`].
#[derive(Clone, Debug)]
pub struct DocEntry {
    /// `common`, `defs`, `kinds`, `values` or `prefixes`.
    pub section: &'static str,
    /// Kind, shared structure or value group the entry belongs to (empty for `common`/`prefixes`).
    pub scope: String,
    /// Path key, value or prefix as written in the catalog (without `[]` markers).
    pub key: String,
    /// The documentation.
    pub doc: &'static FieldDoc,
}

/// Documentation of the key at `path` in a document of `kind`.
///
/// `kind` is the document type (`cdo`, `recipe`, ..., `mam`), `pack` for `pack.yml` or `config`.
/// `path` holds the map keys from the document root, list indexes already removed. A path that is
/// not documented returns `None`; so does everything when the catalog failed to load
/// ([`load_error`]).
pub fn field(kind: &str, path: &[&str]) -> Option<&'static FieldDoc> {
    if path.is_empty() {
        return None;
    }
    catalog()?.field(kind, path)
}

/// Documentation of the single value `value` of the key group `key`, for example
/// `("op", "multiply")`, `("type", "cdo")` or `("conditionBehavior", "OR")`.
///
/// `key` is the YAML key the value belongs to. Value groups of the config fields use a dotted name
/// (`config.type`); [`value_at`] picks the group from the key path and is the easier entry point.
pub fn value(key: &str, value: &str) -> Option<&'static FieldDoc> {
    catalog()?.value(key, value)
}

/// Documentation of a value found at `path` of a `kind` document.
///
/// The value group is taken from the documented key (`values` in the catalog) and falls back to
/// the last path key, so `("cdo", ["patches", "properties", "op"], "multiply")` and
/// `("config", ["fields", "type"], "select")` both resolve.
pub fn value_at(kind: &str, path: &[&str], value: &str) -> Option<&'static FieldDoc> {
    let catalog = catalog()?;
    let group = catalog
        .field(kind, path)
        .and_then(|doc| doc.values.as_deref())
        .or_else(|| path.last().copied())?;
    catalog.value(group, value)
}

/// Documentation of a typed-reference prefix (`cfg:`, `session:`, `ags:`, `option:`) or a template
/// provider (`{cfg:`, `{item:`, `{class:`, `{dataasset:`, `{asset:`, ...), plus the `\{` escape.
///
/// `token` is the text under the cursor. It may be the bare name (`item`), the prefix with its
/// colon, a braced prefix with or without the closing brace (`{item:}`) or a complete reference
/// (`cfg:balance.ironSpeed`, `{item:/Game/...:GetItemName}`). A braced token selects the template
/// provider, an unbraced one the typed reference.
pub fn prefix(token: &str) -> Option<&'static FieldDoc> {
    catalog()?.prefix(token)
}

/// Renders a documentation entry as markdown for a hover.
///
/// The output is the bold summary, a `Type | Required | Default` line, the description, every
/// example as a captioned `yaml` fenced block and the documentation link. Parts that are not set
/// are left out. The text carries no trailing newline.
pub fn to_markdown(doc: &FieldDoc) -> String {
    let mut out = format!("**{}**", doc.summary.trim());
    let mut facts = Vec::new();
    if !doc.type_label.is_empty() {
        facts.push(format!("Type: `{}`", doc.type_label));
    }
    if let Some(required) = &doc.required {
        facts.push(format!("Required: {required}"));
    }
    if let Some(default) = &doc.default {
        facts.push(format!("Default: `{default}`"));
    }
    if !facts.is_empty() {
        out.push_str("\n\n");
        out.push_str(&facts.join(" | "));
    }
    let description = doc.description.trim();
    if !description.is_empty() {
        out.push_str("\n\n");
        out.push_str(description);
    }
    for example in &doc.examples {
        out.push_str("\n\n*");
        out.push_str(example.title.trim());
        out.push_str("*\n\n```yaml\n");
        out.push_str(example.yaml.trim_end());
        out.push_str("\n```");
    }
    if let Some(docs) = &doc.docs {
        out.push_str("\n\n[Documentation](");
        out.push_str(docs);
        out.push(')');
    }
    out
}

/// Every entry of the catalog, for tooling and tests.
pub fn entries() -> Vec<DocEntry> {
    catalog().map(Catalog::entries).unwrap_or_default()
}

/// The parse error of the embedded catalog, if it failed to load.
///
/// The catalog is checked by the crate tests, so this is `None` in every shipped build.
pub fn load_error() -> Option<&'static str> {
    CATALOG
        .get_or_init(parse_catalog)
        .as_ref()
        .err()
        .map(String::as_str)
}

static CATALOG: OnceLock<Result<Catalog, String>> = OnceLock::new();

fn catalog() -> Option<&'static Catalog> {
    CATALOG.get_or_init(parse_catalog).as_ref().ok()
}

fn parse_catalog() -> Result<Catalog, String> {
    Catalog::parse(CATALOG_SOURCE)
}

// ---- catalog model -------------------------------------------------------------------------

struct Entry {
    key: String,
    doc: FieldDoc,
    uses: Vec<String>,
}

#[derive(Default)]
struct Table {
    exact: HashMap<String, Entry>,
    /// Entries whose path contains a `*` segment (any map key), in catalog order.
    wildcard: Vec<(Vec<String>, Entry)>,
}

impl Table {
    fn build(scope: &str, raw: BTreeMap<String, RawEntry>) -> Result<Self, String> {
        let mut table = Table::default();
        for (key, raw) in raw {
            let segments = normalise_key(&key).map_err(|error| format!("{scope}: {error}"))?;
            let entry = Entry {
                key: segments.join("."),
                ..convert(raw).map_err(|error| format!("{scope}.{key}: {error}"))?
            };
            if segments.iter().any(|segment| segment == "*") {
                if table.wildcard.iter().any(|(known, _)| known == &segments) {
                    return Err(format!("{scope}: duplicate path `{}`", entry.key));
                }
                table.wildcard.push((segments, entry));
            } else if table
                .exact
                .insert(segments.join(&KEY_SEPARATOR.to_string()), entry)
                .is_some()
            {
                return Err(format!("{scope}: duplicate path `{}`", segments.join(".")));
            }
        }
        Ok(table)
    }

    fn get(&self, path: &[&str]) -> Option<&Entry> {
        if let Some(entry) = self.exact.get(&path.join(&KEY_SEPARATOR.to_string())) {
            return Some(entry);
        }
        self.wildcard
            .iter()
            .find(|(pattern, _)| {
                pattern.len() == path.len()
                    && pattern
                        .iter()
                        .zip(path)
                        .all(|(pattern, segment)| pattern == "*" || pattern == segment)
            })
            .map(|(_, entry)| entry)
    }

    fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.exact
            .values()
            .chain(self.wildcard.iter().map(|(_, entry)| entry))
    }
}

struct Catalog {
    common: Table,
    defs: HashMap<String, Table>,
    kinds: HashMap<String, Table>,
    values: HashMap<String, HashMap<String, FieldDoc>>,
    prefixes: HashMap<String, FieldDoc>,
}

impl Catalog {
    fn parse(source: &str) -> Result<Self, String> {
        let raw: RawCatalog =
            serde_yaml::from_str(source).map_err(|error| format!("field-docs.yml: {error}"))?;
        if raw.version != 1 {
            return Err(format!(
                "field-docs.yml: unsupported version {}",
                raw.version
            ));
        }
        let common = Table::build("common", raw.common)?;
        let mut defs = HashMap::new();
        for (name, entries) in raw.defs {
            let table = Table::build(&format!("defs.{name}"), entries)?;
            defs.insert(name, table);
        }
        let mut kinds = HashMap::new();
        for (name, entries) in raw.kinds {
            if name != "config" && !ROOT_TYPES.contains(&name.as_str()) {
                return Err(format!("kinds: unknown document kind `{name}`"));
            }
            let table = Table::build(&format!("kinds.{name}"), entries)?;
            kinds.insert(name, table);
        }
        let mut values = HashMap::new();
        for (group, entries) in raw.values {
            let mut docs = HashMap::new();
            for (value, raw) in entries {
                let entry =
                    convert(raw).map_err(|error| format!("values.{group}.{value}: {error}"))?;
                docs.insert(value, entry.doc);
            }
            values.insert(group, docs);
        }
        let mut prefixes = HashMap::new();
        for (prefix, raw) in raw.prefixes {
            let entry = convert(raw).map_err(|error| format!("prefixes.{prefix}: {error}"))?;
            prefixes.insert(prefix, entry.doc);
        }
        let catalog = Catalog {
            common,
            defs,
            kinds,
            values,
            prefixes,
        };
        catalog.check_uses()?;
        Ok(catalog)
    }

    fn check_uses(&self) -> Result<(), String> {
        let tables = std::iter::once(("common", &self.common))
            .chain(self.defs.iter().map(|(name, table)| (name.as_str(), table)))
            .chain(
                self.kinds
                    .iter()
                    .map(|(name, table)| (name.as_str(), table)),
            );
        for (scope, table) in tables {
            for entry in table.iter() {
                for used in &entry.uses {
                    if !self.defs.contains_key(used) {
                        return Err(format!(
                            "{scope}.{}: `uses` names the unknown definition `{used}`",
                            entry.key
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn field(&self, kind: &str, path: &[&str]) -> Option<&FieldDoc> {
        let kind = kind.trim().to_ascii_lowercase();
        let kind = if kind == "mam" {
            "research"
        } else {
            kind.as_str()
        };
        if let Some(table) = self.kinds.get(kind) {
            if let Some(entry) = self.resolve(table, path, 0) {
                return Some(&entry.doc);
            }
        }
        if let Some(entry) = self.element_condition(path) {
            return Some(&entry.doc);
        }
        self.resolve(&self.common, path, 0).map(|entry| &entry.doc)
    }

    /// Exact path first, then the longest prefix that attaches a shared definition via `uses`.
    fn resolve<'a>(&'a self, table: &'a Table, path: &[&str], depth: usize) -> Option<&'a Entry> {
        if let Some(entry) = table.get(path) {
            return Some(entry);
        }
        if depth >= MAX_USES_DEPTH {
            return None;
        }
        for cut in (1..path.len()).rev() {
            let Some(parent) = table.get(&path[..cut]) else {
                continue;
            };
            for name in &parent.uses {
                let Some(def) = self.defs.get(name) else {
                    continue;
                };
                if let Some(found) = self.resolve(def, &path[cut..], depth + 1) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// `conditions` and friends on a list entry below the document root.
    fn element_condition(&self, path: &[&str]) -> Option<&Entry> {
        let start = path
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, key)| CONDITION_KEYS.contains(key))
            .map(|(index, _)| index)?;
        let tail = &path[start..];
        if tail.len() == 1 {
            self.defs.get("listEntry")?.get(tail)
        } else {
            self.common.get(tail)
        }
    }

    fn value(&self, key: &str, value: &str) -> Option<&FieldDoc> {
        let group = VALUE_GROUP_ALIASES
            .iter()
            .find(|(alias, _)| *alias == key)
            .map_or(key, |(_, group)| *group);
        let docs = self.values.get(group)?;
        if let Some(doc) = docs.get(value) {
            return Some(doc);
        }
        // `AND`/`OR` are case-insensitive in documents.
        if group == "conditionBehavior" {
            return docs.get(value.to_ascii_uppercase().as_str());
        }
        None
    }

    fn prefix(&self, token: &str) -> Option<&FieldDoc> {
        let token = token.trim();
        if let Some(doc) = self.prefixes.get(token) {
            return Some(doc);
        }
        let braced = token.starts_with('{');
        let name: String = token
            .trim_start_matches('{')
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .collect();
        if name.is_empty() {
            return None;
        }
        let template = format!("{{{name}:");
        if braced {
            return self.prefixes.get(&template);
        }
        self.prefixes
            .get(&format!("{name}:"))
            .or_else(|| self.prefixes.get(&template))
    }

    fn entries(&'static self) -> Vec<DocEntry> {
        fn from_table(
            out: &mut Vec<DocEntry>,
            section: &'static str,
            scope: &str,
            table: &'static Table,
        ) {
            let mut found: Vec<&'static Entry> = table.iter().collect();
            found.sort_by(|a, b| a.key.cmp(&b.key));
            for entry in found {
                out.push(DocEntry {
                    section,
                    scope: scope.to_owned(),
                    key: entry.key.clone(),
                    doc: &entry.doc,
                });
            }
        }

        let mut out = Vec::new();
        from_table(&mut out, "common", "", &self.common);
        let mut defs: Vec<_> = self.defs.iter().collect();
        defs.sort_by(|a, b| a.0.cmp(b.0));
        for (name, def) in defs {
            from_table(&mut out, "defs", name, def);
        }
        let mut kinds: Vec<_> = self.kinds.iter().collect();
        kinds.sort_by(|a, b| a.0.cmp(b.0));
        for (name, kind) in kinds {
            from_table(&mut out, "kinds", name, kind);
        }
        let mut groups: Vec<_> = self.values.iter().collect();
        groups.sort_by(|a, b| a.0.cmp(b.0));
        for (group, docs) in groups {
            let mut docs: Vec<_> = docs.iter().collect();
            docs.sort_by(|a, b| a.0.cmp(b.0));
            for (value, doc) in docs {
                out.push(DocEntry {
                    section: "values",
                    scope: group.clone(),
                    key: value.clone(),
                    doc,
                });
            }
        }
        let mut prefixes: Vec<_> = self.prefixes.iter().collect();
        prefixes.sort_by(|a, b| a.0.cmp(b.0));
        for (prefix, doc) in prefixes {
            out.push(DocEntry {
                section: "prefixes",
                scope: String::new(),
                key: prefix.clone(),
                doc,
            });
        }
        out
    }
}

fn normalise_key(key: &str) -> Result<Vec<String>, String> {
    let segments: Vec<String> = key
        .split('.')
        .map(|segment| segment.strip_suffix("[]").unwrap_or(segment).to_owned())
        .collect();
    if segments
        .iter()
        .any(|segment| segment.is_empty() || segment.contains(['[', ']']))
    {
        return Err(format!("malformed path key `{key}`"));
    }
    Ok(segments)
}

fn convert(raw: RawEntry) -> Result<Entry, String> {
    if raw.summary.trim().is_empty() {
        return Err("empty summary".to_owned());
    }
    let docs = match raw.docs {
        None => None,
        Some(link) if link.starts_with("https://") => {
            if !link.starts_with(DOCS_BASE) {
                return Err(format!(
                    "documentation link `{link}` is outside {DOCS_BASE}"
                ));
            }
            Some(link)
        }
        Some(page) => Some(format!("{DOCS_BASE}{}", page.trim_start_matches('/'))),
    };
    let mut examples = Vec::with_capacity(raw.examples.len());
    for example in raw.examples {
        if example.title.trim().is_empty() || example.yaml.trim().is_empty() {
            return Err("example without title or yaml".to_owned());
        }
        examples.push(Example {
            title: example.title,
            yaml: example.yaml,
            partial: example.partial,
        });
    }
    Ok(Entry {
        key: String::new(),
        doc: FieldDoc {
            summary: raw.summary.trim().to_owned(),
            description: raw.description.trim().to_owned(),
            type_label: raw.type_label.trim().to_owned(),
            required: raw.required,
            default: raw.default,
            examples,
            docs,
            values: raw.values,
        },
        uses: match raw.uses {
            None => Vec::new(),
            Some(OneOrMany::One(name)) => vec![name],
            Some(OneOrMany::Many(names)) => names,
        },
    })
}

// ---- file format ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalog {
    version: u32,
    #[serde(default)]
    common: BTreeMap<String, RawEntry>,
    #[serde(default)]
    defs: BTreeMap<String, BTreeMap<String, RawEntry>>,
    #[serde(default)]
    kinds: BTreeMap<String, BTreeMap<String, RawEntry>>,
    #[serde(default)]
    values: BTreeMap<String, BTreeMap<String, RawEntry>>,
    #[serde(default)]
    prefixes: BTreeMap<String, RawEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    summary: String,
    #[serde(default)]
    description: String,
    #[serde(rename = "type", default)]
    type_label: String,
    #[serde(default, deserialize_with = "required_text")]
    required: Option<String>,
    #[serde(default, deserialize_with = "scalar_text")]
    default: Option<String>,
    #[serde(default)]
    examples: Vec<RawExample>,
    #[serde(default)]
    docs: Option<String>,
    #[serde(default)]
    uses: Option<OneOrMany>,
    #[serde(default)]
    values: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExample {
    title: String,
    yaml: String,
    #[serde(default)]
    partial: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

/// `required: true` means `yes`, `false` means not required, text names the condition.
fn required_text<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Ok(None),
        Some(Value::Bool(true)) => Ok(Some("yes".to_owned())),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(other) => Err(serde::de::Error::custom(format!(
            "`required` must be a boolean or text, found {other:?}"
        ))),
    }
}

/// Accepts a scalar (`false`, `100`, `text`) and keeps its YAML spelling.
fn scalar_text<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(flag)) => Ok(Some(flag.to_string())),
        Some(Value::Number(number)) => Ok(Some(number.to_string())),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(other) => Err(serde::de::Error::custom(format!(
            "`default` must be a scalar, found {other:?}"
        ))),
    }
}
