//! Pack configuration definitions (`<name>.config.yml`) and the workspace index built from them.
//!
//! Contract: runtime design section 1.1 (document keys, field keys, value types, ids).

use crate::document::{parse_file, MAX_YAML_BYTES};
use crate::project::{is_pack_manifest, ParsedFile};
use crate::{FileBuffer, LintResult, Range};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use walkdir::WalkDir;

pub const RESERVED_IDS: [&str; 4] = ["true", "false", "in", "contains"];
/// Every config category class must derive from this class (`UFGCategory`).
pub const CATEGORY_BASE_CLASS: &str = "/Script/FactoryGame.FGCategory";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    Text,
    Number,
    Boolean,
    Select,
}

impl FieldKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Number => "number",
            FieldKind::Boolean => "boolean",
            FieldKind::Select => "select",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "text" => Some(FieldKind::Text),
            "number" => Some(FieldKind::Number),
            "boolean" => Some(FieldKind::Boolean),
            "select" => Some(FieldKind::Select),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueType {
    String,
    Text,
    Name,
    Tag,
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    Float,
    Double,
    Bool,
}

impl ValueType {
    pub fn as_str(self) -> &'static str {
        match self {
            ValueType::String => "string",
            ValueType::Text => "text",
            ValueType::Name => "name",
            ValueType::Tag => "tag",
            ValueType::Int8 => "int8",
            ValueType::Int16 => "int16",
            ValueType::Int32 => "int32",
            ValueType::Int64 => "int64",
            ValueType::UInt8 => "uint8",
            ValueType::Float => "float",
            ValueType::Double => "double",
            ValueType::Bool => "bool",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "string" => ValueType::String,
            "text" => ValueType::Text,
            "name" => ValueType::Name,
            "tag" => ValueType::Tag,
            "int8" => ValueType::Int8,
            "int16" => ValueType::Int16,
            "int32" => ValueType::Int32,
            "int64" => ValueType::Int64,
            "uint8" => ValueType::UInt8,
            "float" => ValueType::Float,
            "double" => ValueType::Double,
            "bool" => ValueType::Bool,
            _ => return None,
        })
    }
    pub fn is_integer(self) -> bool {
        matches!(
            self,
            ValueType::Int8
                | ValueType::Int16
                | ValueType::Int32
                | ValueType::Int64
                | ValueType::UInt8
        )
    }
    pub fn is_float(self) -> bool {
        matches!(self, ValueType::Float | ValueType::Double)
    }
    pub fn is_number(self) -> bool {
        self.is_integer() || self.is_float()
    }
    pub fn is_string_like(self) -> bool {
        matches!(
            self,
            ValueType::String | ValueType::Text | ValueType::Name | ValueType::Tag
        )
    }
    /// Representable range of integer types (`None` for the others).
    pub fn integer_range(self) -> Option<(i64, i64)> {
        match self {
            ValueType::Int8 => Some((i8::MIN as i64, i8::MAX as i64)),
            ValueType::Int16 => Some((i16::MIN as i64, i16::MAX as i64)),
            ValueType::Int32 => Some((i32::MIN as i64, i32::MAX as i64)),
            ValueType::Int64 => Some((i64::MIN, i64::MAX)),
            ValueType::UInt8 => Some((0, u8::MAX as i64)),
            _ => None,
        }
    }
    /// Default value type of a field kind (`string`, `int32`, `bool`).
    pub fn default_for(kind: FieldKind) -> Self {
        match kind {
            FieldKind::Text | FieldKind::Select => ValueType::String,
            FieldKind::Number => ValueType::Int32,
            FieldKind::Boolean => ValueType::Bool,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Apply {
    Hot,
    Cold,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NumberDisplay {
    Slider,
    Input,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FieldOption {
    pub value: Value,
    pub label: String,
}

/// One accepted `fields[]` entry. Fields the runtime would skip (invalid, duplicate, clashing)
/// are not part of the index.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConfigField {
    pub pack_ref: String,
    pub id: String,
    pub kind: FieldKind,
    pub value_type: ValueType,
    pub multiple: bool,
    pub apply: Apply,
    /// Display text (`source`, else the localization `key`, of keyed texts).
    pub name: String,
    pub description: String,
    pub tooltip_title: Option<String>,
    pub tooltip_text: Option<String>,
    /// Field category, else the document default.
    pub category: Option<String>,
    pub default: Value,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub decimals: Option<u8>,
    /// Resolved display (`slider` when min and max exist and no display is set).
    pub display: NumberDisplay,
    pub options: Vec<FieldOption>,
    pub min_selected: Option<u64>,
    pub max_selected: Option<u64>,
    pub min_count: Option<u64>,
    pub max_count: Option<u64>,
    pub max_length: Option<u64>,
    pub multiline: bool,
    pub tag_root: Option<String>,
    /// Explicit `order`, else the declaration index across the pack's config files.
    pub order: i64,
    pub file: PathBuf,
    pub document: usize,
    /// JSON pointer of the field entry (`/fields/3`).
    pub pointer: String,
    /// Range of the whole entry.
    pub range: Range,
    /// Range of the `id` value.
    pub id_range: Range,
}

impl ConfigField {
    /// `<pack>:<id>` key as used by the runtime registry.
    pub fn key(&self) -> String {
        format!("{}:{}", self.pack_ref, self.id)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PackConfig {
    pub pack_ref: String,
    pub name: Option<String>,
    pub dir: PathBuf,
    pub manifest: PathBuf,
    /// `enabled: false` packs are skipped by the runtime but still indexed.
    pub enabled: bool,
    pub fields: Vec<ConfigField>,
    /// Config files of the pack, sorted by relative path.
    pub files: Vec<PathBuf>,
}

impl PackConfig {
    /// Exact (case-sensitive) id lookup.
    pub fn field(&self, id: &str) -> Option<&ConfigField> {
        self.fields.iter().find(|field| field.id == id)
    }
    /// Case-insensitive id lookup (ids are unique per pack case-insensitively).
    pub fn field_ci(&self, id: &str) -> Option<&ConfigField> {
        self.fields
            .iter()
            .find(|field| field.id.eq_ignore_ascii_case(id))
    }
}

/// How a `cfg:` reference resolves.
#[derive(Clone, Debug, PartialEq)]
pub enum Resolution<'a> {
    /// Exact match.
    Found(&'a ConfigField),
    /// Matches only when letters are compared case-insensitively.
    CaseMismatch(&'a ConfigField),
    /// Pack exists but has no such id.
    UnknownId,
    /// `cfg:<pack>:...` names a pack that is not part of the project.
    UnknownPack,
    /// The pack exists (field returned when the id resolves) but has `enabled: false`.
    PackDisabled(Option<&'a ConfigField>),
}

/// A class a pack generates (`type: class`, `item`, `building`, ...): its id and parent class.
/// Items and the like without an explicit `parent` get the default base class of their kind.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GeneratedClass {
    pub id: String,
    pub parent: Option<String>,
    pub file: PathBuf,
}

/// Diagnostic collected while building the index (emitted by the project pass with ranges).
#[derive(Clone, Debug)]
pub(crate) struct DefinitionIssue {
    pub(crate) severity: crate::Severity,
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) file: PathBuf,
    pub(crate) document: usize,
    pub(crate) pointer: String,
}

/// Generated classes by pack ref and lower-case id.
pub(crate) type ClassMap = HashMap<String, HashMap<String, GeneratedClass>>;

#[derive(Clone, Debug, Default)]
pub struct ConfigIndex {
    packs: Vec<PackConfig>,
    classes: ClassMap,
    /// `(pack position, lower-case id)` to the field position inside the pack.
    lookup: HashMap<(usize, String), usize>,
    pub(crate) issues: Vec<DefinitionIssue>,
}

impl ConfigIndex {
    pub(crate) fn from_parts(
        packs: Vec<PackConfig>,
        classes: ClassMap,
        issues: Vec<DefinitionIssue>,
    ) -> Self {
        let mut lookup = HashMap::new();
        for (pack_position, pack) in packs.iter().enumerate() {
            for (field_position, field) in pack.fields.iter().enumerate() {
                lookup.insert(
                    (pack_position, field.id.to_ascii_lowercase()),
                    field_position,
                );
            }
        }
        Self {
            packs,
            classes,
            lookup,
            issues,
        }
    }

    pub fn packs(&self) -> &[PackConfig] {
        &self.packs
    }
    pub fn is_empty(&self) -> bool {
        self.packs.is_empty()
    }
    /// Pack by `ref` (case-sensitive, like the runtime).
    pub fn pack(&self, pack_ref: &str) -> Option<&PackConfig> {
        self.pack_position(pack_ref)
            .map(|position| &self.packs[position])
    }
    fn pack_position(&self, pack_ref: &str) -> Option<usize> {
        self.packs.iter().position(|pack| pack.pack_ref == pack_ref)
    }
    /// Pack whose folder contains `path`.
    pub fn pack_for_path(&self, path: &Path) -> Option<&PackConfig> {
        self.packs
            .iter()
            .filter(|pack| path.starts_with(&pack.dir))
            .max_by_key(|pack| pack.dir.components().count())
    }
    /// Exact field lookup.
    pub fn field(&self, pack_ref: &str, id: &str) -> Option<&ConfigField> {
        let position = self.pack_position(pack_ref)?;
        self.field_at(position, id).filter(|field| field.id == id)
    }
    /// Field by case-insensitive id.
    fn field_at(&self, pack_position: usize, id: &str) -> Option<&ConfigField> {
        let field = self.lookup.get(&(pack_position, id.to_ascii_lowercase()))?;
        self.packs[pack_position].fields.get(*field)
    }
    /// Every field of every pack, packs in discovery order.
    pub fn fields(&self) -> impl Iterator<Item = &ConfigField> {
        self.packs.iter().flat_map(|pack| pack.fields.iter())
    }
    /// Resolves `id` (own pack when `pack` is `None`, otherwise `cfg:<pack>:<id>`).
    pub fn resolve<'a>(
        &'a self,
        own_pack: Option<&str>,
        pack: Option<&str>,
        id: &str,
    ) -> Resolution<'a> {
        let Some(target) = pack.or(own_pack) else {
            return Resolution::UnknownId;
        };
        let Some(position) = self.pack_position(target) else {
            return Resolution::UnknownPack;
        };
        let found = &self.packs[position];
        let ci = self.field_at(position, id);
        let field = ci.filter(|field| field.id == id);
        // Documents of a disabled pack are never loaded, so its own references are not reported.
        if !found.enabled && Some(target) != own_pack {
            return Resolution::PackDisabled(field.or(ci));
        }
        match (field, ci) {
            (Some(field), _) => Resolution::Found(field),
            (None, Some(field)) => Resolution::CaseMismatch(field),
            (None, None) => Resolution::UnknownId,
        }
    }
    /// Generated class declaration of a pack by bare id (case-insensitive); resolves bare category
    /// ids and generated targets.
    pub fn generated_class(&self, pack_ref: &str, id: &str) -> Option<&GeneratedClass> {
        self.classes.get(pack_ref)?.get(&id.to_ascii_lowercase())
    }
}

/// `true` for `<name>.config.yml` and `<name>.config.yaml` (case-insensitive).
pub fn is_config_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower.ends_with(".config.yml") || lower.ends_with(".config.yaml"))
        && lower.len() > ".config.yml".len()
}

pub fn is_config_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_config_file_name)
}

/// `pack.yml`, `*.config.yml` and `*.class.yml` files are what the configuration index is built from.
pub fn is_index_source(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            let lower = name.to_ascii_lowercase();
            is_pack_manifest(path)
                || is_config_file_name(name)
                || lower.ends_with(".class.yml")
                || lower.ends_with(".class.yaml")
        })
}

/// A parsed manifest, config definition or class document, kept between index builds so that an
/// incremental rebuild only parses the files that changed.
#[derive(Clone, Debug)]
pub struct ConfigSource {
    parsed: Arc<ParsedFile>,
}

impl ConfigSource {
    pub fn path(&self) -> &Path {
        &self.parsed.path
    }
}

/// Parses one index source (see [`is_index_source`]); `None` when the file is too large or the
/// YAML is unusable. Never reports: broken files are skipped like in [`scan_workspace`].
pub fn parse_source(path: PathBuf, text: String) -> Option<ConfigSource> {
    let mut scratch = LintResult::default();
    parse_file(path, text, &mut scratch).map(|parsed| ConfigSource {
        parsed: Arc::new(parsed),
    })
}

/// Index of the given sources (any order; they are sorted by path like the project pass does).
pub fn build_index(sources: &[ConfigSource]) -> ConfigIndex {
    let mut files: Vec<&ParsedFile> = sources.iter().map(|source| &*source.parsed).collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    crate::config_build::build(&files, None)
}

/// Reads `pack.yml`, `*.config.yml` and `*.class.yml` files below `root` (with unsaved `buffers`
/// overriding disk) into an index. Never fails and reports nothing: broken files are skipped.
pub fn scan_workspace(root: &Path, buffers: &[FileBuffer]) -> ConfigIndex {
    let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut texts = BTreeMap::<PathBuf, String>::new();
    for entry in WalkDir::new(&root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            crate::runtime_authored_entry(entry)
                && (!entry.file_type().is_dir()
                    || !matches!(
                        entry.file_name().to_str(),
                        Some(".git" | "node_modules" | "target")
                    ))
        })
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() || !is_index_source(entry.path()) {
            continue;
        }
        if entry
            .metadata()
            .ok()
            .is_some_and(|meta| meta.len() > MAX_YAML_BYTES as u64)
        {
            continue;
        }
        if let Ok(text) = fs::read_to_string(entry.path()) {
            texts.insert(entry.path().to_path_buf(), text);
        }
    }
    for buffer in buffers {
        let path = if buffer.path.is_absolute() {
            buffer.path.clone()
        } else {
            root.join(&buffer.path)
        };
        if crate::is_runtime_authored_path(&path, &root) && is_index_source(&path) {
            texts.insert(path, buffer.text.clone());
        }
    }
    let sources = texts
        .into_iter()
        .filter_map(|(path, text)| parse_source(path, text))
        .collect::<Vec<_>>();
    build_index(&sources)
}
