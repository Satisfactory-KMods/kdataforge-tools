//! Read-only queries on a loaded reflection datapack: class resolution, inheritance, properties,
//! functions, members, reload classification, settings and the cold catalog.
//!
//! Everything here borrows from the index. Lookup maps and the struct/settings/catalog sections
//! are built lazily on first use (`OnceLock`) and never serialised.

use crate::cold::{ColdCatalog, ColdEntry, Reload, NEW_ACTORS_REASON};
use crate::reflection::{ArchiveRef, ReflectionIndex};
use crate::setting_index::SettingsIndex;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::sync::OnceLock;
use zip::ZipArchive;

const MAX_STRUCTS: u64 = 256 * 1024 * 1024;
const MAX_ASSETS: u64 = 256 * 1024 * 1024;
const MAX_SETTINGS: u64 = 16 * 1024 * 1024;
const MAX_COLD_CATALOG: u64 = 4 * 1024 * 1024;
const MAX_WALK: usize = 256;

#[derive(Clone, Copy, Eq, PartialEq)]
enum SuperChainStop {
    Complete,
    Limit,
    Cycle,
}

const ACTOR: &str = "/Script/Engine.Actor";
const ACTOR_COMPONENT: &str = "/Script/Engine.ActorComponent";
const SUBSYSTEM: &str = "/Script/Engine.Subsystem";
const CDO_HELPER: &str = "/Script/KBFL.KBFLCDOOverwriteBase";
const CDO_HELPER_REASON: &str =
    "Native CDO helpers run during startup; restart the game after changing their configuration.";

/// Reload classification of many properties of one class, with the class-level work (class
/// resolution, ancestor chain, catalog entries that apply, actor rule) done once. Built by
/// [`ReflectionIndex::reload_classifier`]; completion classifies every property of a class with it.
#[derive(Debug)]
pub struct ReloadClassifier<'a> {
    /// The class resolved; an unknown class classifies everything as hot.
    known: bool,
    /// The class derives from `AActor` or `UActorComponent`.
    actor: bool,
    startup_bound: bool,
    apply_to_spawned_actors: bool,
    /// Catalog entries that apply to the class chain by lower-case property name (closest class
    /// wins, a `hot` entry wins a tie).
    entries: HashMap<String, &'a ColdEntry>,
}

impl ReloadClassifier<'_> {
    fn settle(&self, reload: Reload, reason: String) -> (Reload, String) {
        if reload == Reload::NewActors && self.apply_to_spawned_actors {
            (Reload::Hot, String::new())
        } else {
            (reload, reason)
        }
    }

    fn catalog_entry(&self, name: &str) -> Option<&ColdEntry> {
        if name.is_empty() || self.entries.is_empty() {
            return None;
        }
        self.entries.get(&name.to_ascii_lowercase()).copied()
    }

    /// Classification of a property record. Order: catalog entry, the exporter annotation
    /// `kdf.reload` (read straight from the record), then the actor rule.
    pub fn classify(&self, property: &PropertyInfo) -> (Reload, String) {
        if self.startup_bound {
            return (Reload::Cold, CDO_HELPER_REASON.to_owned());
        }
        if !self.known {
            return (Reload::Hot, String::new());
        }
        let entry = self
            .catalog_entry(&property.name)
            .or_else(|| self.catalog_entry(&property.authored_name));
        if let Some(entry) = entry {
            return self.settle(entry.reload, entry.reason.clone());
        }
        if let Some(reload) = property.reload {
            return self.settle(
                reload,
                property
                    .reload_reason
                    .clone()
                    .unwrap_or_else(|| NEW_ACTORS_REASON.to_owned()),
            );
        }
        self.actor_rule()
    }

    /// Classification of a property that is only known by name (no record).
    pub fn classify_name(&self, name: &str) -> (Reload, String) {
        if self.startup_bound {
            return (Reload::Cold, CDO_HELPER_REASON.to_owned());
        }
        if !self.known {
            return (Reload::Hot, String::new());
        }
        if let Some(entry) = self.catalog_entry(name) {
            return self.settle(entry.reload, entry.reason.clone());
        }
        self.actor_rule()
    }

    fn actor_rule(&self) -> (Reload, String) {
        if !self.apply_to_spawned_actors && self.actor {
            (Reload::NewActors, NEW_ACTORS_REASON.to_owned())
        } else {
            (Reload::Hot, String::new())
        }
    }
}

/// Lazily built lookup structures (all derived from the archive).
#[derive(Clone, Debug, Default)]
pub(crate) struct Lazy {
    lower: OnceLock<HashMap<String, String>>,
    children: OnceLock<HashMap<String, Vec<String>>>,
    by_name: OnceLock<HashMap<String, Vec<String>>>,
    structs: OnceLock<StructIndex>,
    assets: OnceLock<AssetIndex>,
    functions: OnceLock<bool>,
    settings: OnceLock<Option<SettingsIndex>>,
    cold: OnceLock<Option<ColdCatalog>>,
}

/// Compact view of `structs.ndjson`: struct path (lowercase) to its properties.
#[derive(Clone, Debug, Default)]
pub(crate) struct StructIndex {
    structs: HashMap<String, StructEntry>,
}

#[derive(Clone, Debug, Default)]
struct StructEntry {
    path: String,
    properties: Vec<PropertyInfo>,
}

/// One asset of `indexes/assets.json`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AssetEntry {
    /// Object path (`/Game/Dir/DA_X.DA_X`).
    pub path: String,
    pub name: String,
    /// Class of the asset as the asset registry reports it (`/Script/KAPI.KAPIDeliveryTask`).
    pub asset_class: String,
    /// Property class the asset was indexed under.
    pub index_class: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AssetIndex {
    entries: Vec<AssetEntry>,
    by_class: HashMap<String, Vec<usize>>,
    by_path: HashMap<String, usize>,
    /// Lower-case indexed class -> whether its listing holds every asset of that class.
    complete: HashMap<String, bool>,
}

#[derive(serde::Deserialize)]
struct RawAssetClass {
    class: Option<String>,
    assets: Option<Vec<RawAsset>>,
    /// Assets of the class before the exporter's asset cap.
    total: Option<usize>,
    #[serde(default)]
    truncated: bool,
}

impl RawAssetClass {
    fn is_complete(&self, index: &ReflectionIndex, class: &str) -> bool {
        self.assets.as_ref().is_some_and(|assets| {
            let mut paths = HashSet::new();
            !self.truncated
                && self.total.is_none_or(|total| total == assets.len())
                && self
                    .class
                    .as_deref()
                    .is_none_or(|declared| declared.eq_ignore_ascii_case(class))
                && assets.iter().all(|asset| {
                    valid_asset_object_path(&asset.path)
                        && paths.insert(asset.path.to_lowercase())
                        && asset.asset_name.as_deref().is_none_or(|name| {
                            name.eq_ignore_ascii_case(asset_object_name(&asset.path))
                        })
                        && index.asset_class_compatible(
                            asset.asset_class.as_deref().unwrap_or_default(),
                            class,
                        )
                })
        })
    }
}

fn asset_object_name(path: &str) -> &str {
    // FPackageName::ObjectPathToObjectName returns the leaf after the last object/subobject delimiter.
    path.rsplit(['.', ':']).next().unwrap_or(path)
}

/// Exporter `StableObjectPath` preserves mount roots, object names and optional subobjects.
/// Character exclusions follow UE's INVALID_LONGPACKAGE/OBJECTNAME_CHARACTERS (NameTypes.h).
fn valid_asset_object_path(path: &str) -> bool {
    let Some((package, object)) = path.split_once('.') else {
        return false;
    };
    let Some(package) = package.strip_prefix('/') else {
        return false;
    };
    let package_parts = package.split('/').collect::<Vec<_>>();
    if package_parts.len() < 2
        || package_parts.iter().any(|part| part.is_empty())
        || package.contains([
            '\\', ':', '*', '?', '"', '<', '>', '|', '\'', ' ', ',', '.', '&', '!', '~', '\n',
            '\r', '\t', '@', '#',
        ])
    {
        return false;
    }
    let valid_name = |name: &str| {
        !name.is_empty()
            && !name.contains([
                '\\', '"', '\'', ' ', ',', '/', '.', ':', '|', '&', '!', '~', '\n', '\r', '\t',
                '@', '#', '(', ')', '{', '}', '[', ']', '=', ';', '^', '%', '$', '`',
            ])
            && !name.chars().any(char::is_control)
    };
    let (object, subobjects) = object
        .split_once(':')
        .map_or((object, None), |(object, subobjects)| {
            (object, Some(subobjects))
        });
    valid_name(object)
        && subobjects.is_none_or(|subobjects| subobjects.split('.').all(valid_name))
        && !package.chars().any(char::is_control)
}

#[derive(serde::Deserialize)]
struct RawAsset {
    path: String,
    #[serde(rename = "assetName")]
    asset_name: Option<String>,
    #[serde(rename = "assetClass")]
    asset_class: Option<String>,
}

fn read_archive_text(archive: &ArchiveRef, name: &str, max: u64) -> Option<String> {
    let file = File::open(&archive.path).ok()?;
    let mut zip = ZipArchive::new(file).ok()?;
    let entry = zip.by_name(&format!("{}{name}", archive.prefix)).ok()?;
    if entry.size() > max {
        return None;
    }
    let mut bytes = Vec::new();
    entry.take(max + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > max {
        return None;
    }
    String::from_utf8(bytes).ok()
}

// ---------------------------------------------------------------------------------------------
// Public data types
// ---------------------------------------------------------------------------------------------

/// A class record resolved from any accepted spelling of a class reference.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedClass<'a> {
    /// Canonical path as exported (`/Script/FactoryGame.FGRecipe`).
    pub path: &'a str,
    pub record: &'a Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum PropertyKind {
    Bool,
    Int8,
    Int16,
    Int32,
    Int64,
    UInt16,
    UInt32,
    UInt64,
    Byte,
    Float,
    Double,
    Str,
    Name,
    Text,
    Enum,
    Class,
    SoftClass,
    Object,
    SoftObject,
    WeakObject,
    LazyObject,
    Interface,
    Struct,
    Array,
    Set,
    Map,
    Optional,
    Delegate,
    Other,
}

impl PropertyKind {
    /// Parses the exporter's `kind` (`FloatProperty`, `StructProperty`, ...), case-insensitively.
    pub fn from_kind_name(name: &str) -> PropertyKind {
        let lower = name.to_ascii_lowercase();
        let base = lower.strip_suffix("property").unwrap_or(&lower);
        match base {
            "bool" => PropertyKind::Bool,
            "int8" => PropertyKind::Int8,
            "int16" => PropertyKind::Int16,
            "int" | "int32" => PropertyKind::Int32,
            "int64" => PropertyKind::Int64,
            "uint16" => PropertyKind::UInt16,
            "uint32" => PropertyKind::UInt32,
            "uint64" => PropertyKind::UInt64,
            "byte" => PropertyKind::Byte,
            "float" => PropertyKind::Float,
            "double" => PropertyKind::Double,
            "str" | "string" => PropertyKind::Str,
            "name" => PropertyKind::Name,
            "text" => PropertyKind::Text,
            "enum" => PropertyKind::Enum,
            "class" => PropertyKind::Class,
            "softclass" => PropertyKind::SoftClass,
            "object" => PropertyKind::Object,
            "softobject" => PropertyKind::SoftObject,
            "weakobject" => PropertyKind::WeakObject,
            "lazyobject" => PropertyKind::LazyObject,
            "interface" => PropertyKind::Interface,
            "struct" => PropertyKind::Struct,
            "array" => PropertyKind::Array,
            "set" => PropertyKind::Set,
            "map" => PropertyKind::Map,
            "optional" => PropertyKind::Optional,
            other if other.contains("delegate") => PropertyKind::Delegate,
            _ => PropertyKind::Other,
        }
    }
    pub fn is_object_reference(self) -> bool {
        matches!(
            self,
            PropertyKind::Object
                | PropertyKind::SoftObject
                | PropertyKind::WeakObject
                | PropertyKind::LazyObject
        )
    }
}

/// Compact copy of a property record (no JSON is retained).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PropertyInfo {
    /// As exported (lowercase for Blueprint-authored members).
    pub name: String,
    pub authored_name: String,
    pub kind: PropertyKind,
    pub kind_name: String,
    pub cpp_type: String,
    pub owner_path: String,
    /// `propertyClass` / `interfaceClass`.
    pub class_path: Option<String>,
    /// Meta class of class and soft-class properties.
    pub meta_class: Option<String>,
    pub struct_path: Option<String>,
    pub enum_path: Option<String>,
    /// Enum entry names (without `*_MAX`) for enum and enum-byte properties.
    pub enum_options: Vec<String>,
    /// Array inner, set element or optional value.
    pub inner: Option<Box<PropertyInfo>>,
    pub key: Option<Box<PropertyInfo>>,
    /// Map value.
    pub value: Option<Box<PropertyInfo>>,
    /// `kdf.reload` annotation of the exporter.
    pub reload: Option<Reload>,
    pub reload_reason: Option<String>,
    pub flags: Vec<String>,
}

impl PropertyInfo {
    pub fn from_record(record: &Value) -> PropertyInfo {
        let text = |key: &str| {
            record
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        let boxed = |key: &str| {
            record
                .get(key)
                .filter(|value| value.is_object())
                .map(|value| Box::new(PropertyInfo::from_record(value)))
        };
        let kind_name = text("kind").unwrap_or_default();
        let kind = PropertyKind::from_kind_name(&kind_name);
        let (inner, value) = match kind {
            PropertyKind::Optional => (boxed("value"), None),
            _ => (boxed("inner").or_else(|| boxed("element")), boxed("value")),
        };
        let enum_options = if text("enum").is_some() {
            record
                .get("options")
                .and_then(Value::as_array)
                .map(|entries| {
                    entries
                        .iter()
                        .filter(|entry| entry.get("isMax").and_then(Value::as_bool) != Some(true))
                        .filter(|entry| {
                            crate::reflection_validation::enum_option_is_valid(record, entry)
                        })
                        .filter_map(|entry| entry.get("name").and_then(Value::as_str))
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        PropertyInfo {
            name: text("name").unwrap_or_default(),
            authored_name: text("authoredName").unwrap_or_default(),
            kind,
            kind_name,
            cpp_type: text("cppType").unwrap_or_default(),
            owner_path: text("ownerPath").unwrap_or_default(),
            class_path: text("propertyClass").or_else(|| text("interfaceClass")),
            meta_class: text("metaClass"),
            struct_path: text("struct"),
            enum_path: text("enum"),
            enum_options,
            inner,
            key: boxed("key"),
            value,
            reload: record
                .pointer("/kdf/reload")
                .and_then(Value::as_str)
                .and_then(Reload::parse),
            reload_reason: record
                .pointer("/kdf/reloadReason")
                .and_then(Value::as_str)
                .map(str::to_owned),
            flags: record
                .pointer("/flags/names")
                .and_then(Value::as_array)
                .map(|names| {
                    names
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
    /// `authoredName`, else `name`.
    pub fn display_name(&self) -> &str {
        if self.authored_name.is_empty() {
            &self.name
        } else {
            &self.authored_name
        }
    }
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|name| name == flag)
    }
    pub fn is_container(&self) -> bool {
        matches!(
            self.kind,
            PropertyKind::Array | PropertyKind::Set | PropertyKind::Map
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ParamInfo {
    pub property: PropertyInfo,
    pub is_out: bool,
    pub is_reference: bool,
    pub is_const: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FunctionInfo {
    /// Exported (lowercase) name.
    pub name: String,
    pub authored_name: String,
    pub path: String,
    pub owner_path: String,
    pub flags: Vec<String>,
    /// Input and output parameters (the return value is `ret`).
    pub params: Vec<ParamInfo>,
    pub ret: Option<PropertyInfo>,
}

impl FunctionInfo {
    pub fn display_name(&self) -> &str {
        if self.authored_name.is_empty() {
            &self.name
        } else {
            &self.authored_name
        }
    }
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|name| name == flag)
    }
    pub fn is_static(&self) -> bool {
        self.has_flag("Static")
    }
    fn from_record(record: &Value) -> FunctionInfo {
        let text = |key: &str| {
            record
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let params = record
            .get("params")
            .and_then(Value::as_array)
            .map(|params| {
                params
                    .iter()
                    .map(|param| {
                        let property = PropertyInfo::from_record(param);
                        ParamInfo {
                            is_out: property.has_flag("OutParm"),
                            is_reference: property.has_flag("ReferenceParm"),
                            is_const: property.has_flag("ConstParm"),
                            property,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        FunctionInfo {
            name: text("name"),
            authored_name: text("authoredName"),
            path: text("path"),
            owner_path: text("ownerPath"),
            flags: record
                .pointer("/flags/names")
                .and_then(Value::as_array)
                .map(|names| {
                    names
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            params,
            ret: record
                .get("return")
                .filter(|value| value.is_object())
                .map(PropertyInfo::from_record),
        }
    }
}

/// What a template target is: a class (functions run on the CDO) or an object instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum TargetKind {
    Class,
    Object,
}

/// Argument supplied by the runtime without being written in the template.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum Implicit {
    None,
    /// The target class is passed to the single class parameter.
    Class,
    /// The target instance is passed to the single object parameter of a static function.
    Object,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum Callable {
    Yes {
        implicit: Implicit,
        /// Number of arguments that must be written.
        explicit_args: usize,
        /// Set when a parameter type could not be verified (struct data missing in the datapack).
        unverified: Option<String>,
    },
    /// Violates the call allowlist (reason for diagnostics).
    No(String),
}

impl Callable {
    pub fn is_callable(&self) -> bool {
        matches!(self, Callable::Yes { .. })
    }
    pub fn implicit(&self) -> Implicit {
        match self {
            Callable::Yes { implicit, .. } => *implicit,
            Callable::No(_) => Implicit::None,
        }
    }
}

/// Input parameters that must be written explicitly (the implicit one is dropped).
pub fn explicit_params(function: &FunctionInfo, implicit: Implicit) -> Vec<&ParamInfo> {
    let mut skip = implicit != Implicit::None;
    function
        .params
        .iter()
        .filter(|param| !param.is_out || (param.is_reference && param.is_const))
        .filter(|_| {
            if skip {
                skip = false;
                return false;
            }
            true
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum MemberKind {
    Property,
    Function,
}

/// A property or callable function offered after `{item:Target:`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MemberInfo {
    /// Display name (`authoredName` preferred).
    pub name: String,
    pub kind: MemberKind,
    pub owner_path: String,
    /// C++ type of a property or the return type of a function.
    pub type_label: String,
    /// `Name(a: T, b: U) -> R` without the implicit parameter (functions only).
    pub signature: Option<String>,
    pub implicit: Implicit,
    /// Exporter annotation `kdf.reload` (properties only).
    pub reload: Option<Reload>,
    pub callable: Option<Callable>,
    pub property: Option<PropertyInfo>,
    pub function: Option<FunctionInfo>,
}

// ---------------------------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
enum Segment {
    Field(String),
    Index,
}

fn parse_property_path(path: &str) -> Option<Vec<Segment>> {
    let chars = path.chars().collect::<Vec<_>>();
    let mut segments = Vec::new();
    let mut index = 0usize;
    let read_word = |index: &mut usize| {
        let start = *index;
        if !chars
            .get(*index)
            .is_some_and(|ch| ch.is_ascii_alphabetic() || *ch == '_')
        {
            return None;
        }
        while chars
            .get(*index)
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        {
            *index += 1;
        }
        Some(chars[start..*index].iter().collect::<String>())
    };
    segments.push(Segment::Field(read_word(&mut index)?));
    while index < chars.len() {
        match chars[index] {
            '.' => {
                index += 1;
                segments.push(Segment::Field(read_word(&mut index)?));
            }
            '[' => {
                index += 1;
                let mut quote: Option<char> = None;
                loop {
                    let ch = *chars.get(index)?;
                    index += 1;
                    if let Some(open) = quote {
                        if ch == '\\' {
                            index += 1;
                        } else if ch == open {
                            quote = None;
                        }
                    } else if ch == '\'' || ch == '"' {
                        quote = Some(ch);
                    } else if ch == ']' {
                        break;
                    }
                }
                segments.push(Segment::Index);
            }
            _ => return None,
        }
    }
    Some(segments)
}

impl ReflectionIndex {
    fn lower_map(&self) -> &HashMap<String, String> {
        self.lazy.lower.get_or_init(|| {
            self.full
                .keys()
                .map(|path| (path.to_lowercase(), path.clone()))
                .collect()
        })
    }

    fn children_map(&self) -> &HashMap<String, Vec<String>> {
        self.lazy.children.get_or_init(|| {
            let mut children = HashMap::<String, Vec<String>>::new();
            for path in self.full.keys() {
                if let Some(parent) = self.parent_path(path) {
                    children
                        .entry(parent.to_owned())
                        .or_default()
                        .push(path.clone());
                }
            }
            for list in children.values_mut() {
                list.sort();
            }
            children
        })
    }

    fn name_map(&self) -> &HashMap<String, Vec<String>> {
        self.lazy.by_name.get_or_init(|| {
            let mut names = HashMap::<String, Vec<String>>::new();
            for path in self.full.keys() {
                let tail = path.rsplit(['.', '/']).next().unwrap_or(path);
                names
                    .entry(tail.to_lowercase())
                    .or_default()
                    .push(path.clone());
            }
            for list in names.values_mut() {
                list.sort();
            }
            names
        })
    }

    fn struct_index(&self) -> &StructIndex {
        self.lazy.structs.get_or_init(|| {
            let mut index = StructIndex::default();
            let Some(archive) = &self.archive else {
                return index;
            };
            let Some(text) = read_archive_text(archive, "structs.ndjson", MAX_STRUCTS) else {
                return index;
            };
            for line in text.lines().filter(|line| !line.trim().is_empty()) {
                let Ok(record) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                let Some(path) = record.get("path").and_then(Value::as_str) else {
                    continue;
                };
                let properties = record
                    .get("properties")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().map(PropertyInfo::from_record).collect())
                    .unwrap_or_default();
                index.structs.insert(
                    path.to_lowercase(),
                    StructEntry {
                        path: path.to_owned(),
                        properties,
                    },
                );
            }
            index
        })
    }

    fn asset_index(&self) -> &AssetIndex {
        self.lazy.assets.get_or_init(|| {
            let mut index = AssetIndex::default();
            let Some(archive) = &self.archive else {
                return index;
            };
            let Some(text) = read_archive_text(archive, "indexes/assets.json", MAX_ASSETS) else {
                return index;
            };
            let Ok(classes) = serde_json::from_str::<HashMap<String, RawAssetClass>>(&text) else {
                return index;
            };
            let mut keys = classes.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                let complete = classes[&key].is_complete(self, &key);
                index
                    .complete
                    .entry(key.to_lowercase())
                    // Multiple inventories for one class cannot establish a unique complete list,
                    // even if each bucket separately claims completeness.
                    .and_modify(|previous| *previous = false)
                    .or_insert(complete);
                for asset in classes[&key].assets.as_deref().unwrap_or_default() {
                    let position = index.entries.len();
                    index.entries.push(AssetEntry {
                        path: asset.path.clone(),
                        name: asset
                            .asset_name
                            .clone()
                            .unwrap_or_else(|| asset_object_name(&asset.path).to_owned()),
                        asset_class: asset.asset_class.clone().unwrap_or_default(),
                        index_class: key.clone(),
                    });
                    index
                        .by_class
                        .entry(key.to_lowercase())
                        .or_default()
                        .push(position);
                    index
                        .by_path
                        .entry(asset.path.to_lowercase())
                        .or_insert(position);
                }
            }
            index
        })
    }

    /// Assets indexed under a property class (`indexes/assets.json`), in export order.
    pub fn assets_of_class(&self, class: &str) -> Vec<&AssetEntry> {
        let index = self.asset_index();
        let key = self
            .class_path(class)
            .map_or_else(|| class.to_lowercase(), str::to_lowercase);
        index
            .by_class
            .get(&key)
            .into_iter()
            .flatten()
            .map(|position| &index.entries[*position])
            .collect()
    }

    /// Whether `indexes/assets.json` lists every asset of `class` and its descendants:
    /// `None` when the class was not indexed, `Some(false)` when its listing is missing,
    /// truncated, malformed or inconsistent.
    pub fn asset_listing_complete(&self, class: &str) -> Option<bool> {
        let key = self
            .class_path(class)
            .map_or_else(|| class.to_lowercase(), str::to_lowercase);
        self.asset_index().complete.get(&key).copied()
    }

    fn asset_class_compatible(&self, asset_class: &str, index_class: &str) -> bool {
        // Older exports omit assetClass. The registry query includes derived classes.
        asset_class.is_empty()
            || self.class_path(asset_class).is_some_and(|canonical| {
                // Serialized registry identities must be canonical paths, not query aliases.
                asset_class.eq_ignore_ascii_case(canonical)
                    && (canonical.eq_ignore_ascii_case(index_class) || {
                        let Some(ancestor) = self.class_path(index_class) else {
                            return false;
                        };
                        let (mut chain, mut stop) =
                            self.super_chain_with_limit(canonical, MAX_WALK);
                        if stop == SuperChainStop::Limit {
                            // Asset coverage needs proof beyond the interactive query budget. Each
                            // visited class is distinct, so the index size bounds a complete walk.
                            (chain, stop) = self.super_chain_with_limit(canonical, self.full.len());
                        }
                        stop == SuperChainStop::Complete && chain.contains(&ancestor)
                    })
            })
    }

    pub(crate) fn asset_identity_verified(&self, asset: &AssetEntry) -> bool {
        valid_asset_object_path(&asset.path)
            && asset
                .name
                .eq_ignore_ascii_case(asset_object_name(&asset.path))
            && self.asset_class_compatible(&asset.asset_class, &asset.index_class)
    }

    /// Classes that have an entry in the asset index.
    pub fn asset_index_classes(&self) -> Vec<&str> {
        let mut classes = self
            .asset_index()
            .entries
            .iter()
            .map(|entry| entry.index_class.as_str())
            .collect::<Vec<_>>();
        classes.sort_unstable();
        classes.dedup();
        classes
    }

    /// Asset by object path (`/Game/Dir/DA_X.DA_X`; the package form `/Game/Dir/DA_X` also works).
    pub fn find_asset(&self, path: &str) -> Option<&AssetEntry> {
        let index = self.asset_index();
        let lower = path.trim().to_lowercase();
        let position = index.by_path.get(&lower).or_else(|| {
            if lower.contains('.') {
                return None;
            }
            let name = lower.rsplit('/').next().unwrap_or(&lower);
            index.by_path.get(&format!("{lower}.{name}"))
        })?;
        index.entries.get(*position)
    }

    /// Builds every lazy lookup structure now: lower-case path map, children, name map, structs,
    /// assets, the function flag, settings and the cold catalog. The first query of a freshly loaded
    /// index otherwise pays for whichever structure it touches (hundreds of milliseconds for the
    /// struct and asset sections of a full datapack); call this once on a worker thread after the
    /// load. Idempotent and safe to call concurrently with queries.
    pub fn warm(&self) {
        let _ = self.lower_map();
        let _ = self.children_map();
        let _ = self.name_map();
        let _ = self.struct_index();
        let _ = self.asset_index();
        let _ = self.has_function_data();
        let _ = self.settings();
        let _ = self.cold_catalog();
    }

    /// Whether the export carries function records at all (`/kdf-reflection` full profile). Without
    /// them an unknown function name proves nothing.
    pub fn has_function_data(&self) -> bool {
        *self
            .lazy
            .functions
            .get_or_init(|| self.full.values().any(|record| record.functions_present))
    }

    /// Settings index of the datapack (`indexes/settings.json`), when present.
    pub fn settings(&self) -> Option<&SettingsIndex> {
        self.lazy
            .settings
            .get_or_init(|| {
                let archive = self.archive.as_ref()?;
                let text = read_archive_text(archive, "indexes/settings.json", MAX_SETTINGS)?;
                SettingsIndex::parse(&text).ok()
            })
            .as_ref()
    }

    /// Cold-property catalog of the datapack (`catalog/cold_properties.json`), when present.
    pub fn cold_catalog(&self) -> Option<&ColdCatalog> {
        self.lazy
            .cold
            .get_or_init(|| {
                let archive = self.archive.as_ref()?;
                let text =
                    read_archive_text(archive, "catalog/cold_properties.json", MAX_COLD_CATALOG)?;
                ColdCatalog::parse(&text).ok()
            })
            .as_ref()
    }

    /// Resolves a class reference: exact path, case-insensitive path, Blueprint spellings
    /// (`/Game/A/B.B`, `/Game/A/B`, `Class'/Game/A/B.B_C'`) and finally a unique short name.
    pub fn resolve_class(&self, reference: &str) -> Option<ResolvedClass<'_>> {
        let path = self.class_path(reference)?;
        Some(ResolvedClass {
            path,
            record: self.class_info(path)?,
        })
    }

    /// Canonical path from compact metadata; no full class record is decoded.
    pub fn class_path(&self, reference: &str) -> Option<&str> {
        let mut text = reference.trim();
        if let Some((_, inside)) = text.split_once('\'') {
            text = inside.split('\'').next().unwrap_or(inside);
        }
        if text.is_empty() {
            return None;
        }
        let lower = self.lower_map();
        let lookup = |candidate: &str| {
            self.full
                .get_key_value(candidate)
                .map(|(path, _)| path.as_str())
                .or_else(|| lower.get(&candidate.to_lowercase()).map(String::as_str))
        };
        if let Some(path) = lookup(text) {
            return Some(path);
        }
        if text.starts_with('/') && !text.starts_with("/Script/") {
            let tail = text.rsplit('/').next().unwrap_or("");
            let candidates = if text.contains('.') {
                vec![format!("{text}_C")]
            } else {
                vec![format!("{text}.{tail}_C"), format!("{text}.{tail}")]
            };
            return candidates
                .into_iter()
                .find_map(|candidate| lookup(&candidate));
        }
        if !text.contains('/') {
            let short = text.rsplit('.').next().unwrap_or(text).to_lowercase();
            for candidate in [short.clone(), format!("{short}_c")] {
                if let Some(paths) = self.name_map().get(&candidate) {
                    // Two exported classes with the same short name are ambiguous.
                    if paths.len() != 1 {
                        return None;
                    }
                    return paths.first().and_then(|path| lookup(path));
                }
            }
        }
        None
    }

    /// `class` itself followed by its ancestors (nearest first).
    pub fn super_chain_inclusive(&self, class: &str) -> Vec<&str> {
        self.super_chain_with_limit(class, MAX_WALK).0
    }

    /// Distinguishes budget exhaustion and malformed cycles from roots or absent parents.
    fn super_chain_with_limit(&self, class: &str, limit: usize) -> (Vec<&str>, SuperChainStop) {
        let mut chain = Vec::new();
        let mut visited = HashSet::new();
        let mut current = self.class_path(class);
        while let Some(path) = current {
            if !visited.insert(path) {
                return (chain, SuperChainStop::Cycle);
            }
            if chain.len() >= limit {
                return (chain, SuperChainStop::Limit);
            }
            chain.push(path);
            current = self
                .parent_path(path)
                .and_then(|parent| self.class_path(parent));
        }
        (chain, SuperChainStop::Complete)
    }

    /// `class` equals `ancestor` or derives from it. Unknown classes are not subclasses.
    pub fn is_subclass_of(&self, class: &str, ancestor: &str) -> bool {
        let Some(ancestor) = self.class_path(ancestor) else {
            return false;
        };
        self.super_chain_inclusive(class).contains(&ancestor)
    }

    /// Classes deriving from `base` (not including it), sorted by path.
    pub fn descendants(&self, base: &str, concrete_only: bool) -> Vec<&str> {
        let Some(base) = self.class_path(base) else {
            return Vec::new();
        };
        let children = self.children_map();
        let mut out = Vec::new();
        let mut stack = vec![base];
        let mut seen = std::collections::HashSet::new();
        while let Some(current) = stack.pop() {
            for child in children.get(current).into_iter().flatten() {
                if !seen.insert(child.as_str()) {
                    continue;
                }
                stack.push(child.as_str());
                if concrete_only && self.is_abstract(child) {
                    continue;
                }
                out.push(child.as_str());
            }
        }
        out.sort_unstable();
        out
    }

    /// `Abstract` class flag.
    pub fn is_abstract(&self, class: &str) -> bool {
        let Some(path) = self.class_path(class) else {
            return false;
        };
        self.classes
            .binary_search_by(|class| class.path.as_str().cmp(path))
            .ok()
            .is_some_and(|index| self.classes[index].abstract_class)
    }

    /// Property by name (case-insensitive on `name` and `authoredName`), searching the class and
    /// then its ancestors.
    pub fn find_property(&self, class: &str, name: &str) -> Option<PropertyInfo> {
        for path in self.super_chain_inclusive(class) {
            let record = self.class_info(path)?;
            // Unknown child coverage may hide an override of any ancestor field.
            let properties = record.get("properties").and_then(Value::as_array)?;
            let found = properties.iter().find(|property| {
                ["name", "authoredName"].iter().any(|key| {
                    property
                        .get(*key)
                        .and_then(Value::as_str)
                        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
                })
            });
            if let Some(property) = found {
                return Some(PropertyInfo::from_record(property));
            }
        }
        None
    }

    /// Property of a struct (case-insensitive).
    pub fn find_struct_field(&self, struct_path: &str, name: &str) -> Option<PropertyInfo> {
        let entry = self
            .struct_index()
            .structs
            .get(&struct_path.to_lowercase())?;
        entry
            .properties
            .iter()
            .find(|property| {
                property.name.eq_ignore_ascii_case(name)
                    || property.authored_name.eq_ignore_ascii_case(name)
            })
            .cloned()
    }

    /// All properties of a struct, in declaration order.
    pub fn struct_properties(&self, struct_path: &str) -> Option<&[PropertyInfo]> {
        self.struct_index()
            .structs
            .get(&struct_path.to_lowercase())
            .map(|entry| entry.properties.as_slice())
    }

    /// Canonical path of a struct known to the datapack.
    pub fn struct_path(&self, reference: &str) -> Option<&str> {
        self.struct_index()
            .structs
            .get(&reference.to_lowercase())
            .map(|entry| entry.path.as_str())
    }

    /// Resolves a property path (`mA.mB[0]['key'].mC`) starting at `class`: struct fields, array
    /// and set elements, map values and object references are followed.
    pub fn resolve_property_path(&self, class: &str, path: &str) -> Option<PropertyInfo> {
        let segments = parse_property_path(path)?;
        let mut iter = segments.into_iter();
        let Some(Segment::Field(first)) = iter.next() else {
            return None;
        };
        let mut current = self.find_property(class, &first)?;
        for segment in iter {
            current = match segment {
                Segment::Index => match current.kind {
                    PropertyKind::Array | PropertyKind::Set => *current.inner?,
                    PropertyKind::Map => *current.value?,
                    _ => return None,
                },
                Segment::Field(name) => match current.kind {
                    PropertyKind::Struct => {
                        self.find_struct_field(current.struct_path.as_deref()?, &name)?
                    }
                    kind if kind.is_object_reference() => {
                        self.find_property(current.class_path.as_deref()?, &name)?
                    }
                    _ => return None,
                },
            };
        }
        Some(current)
    }

    /// Function by name (case-insensitive on `name` and `authoredName`), searching the class and
    /// then its ancestors.
    pub fn find_function(&self, class: &str, name: &str) -> Option<FunctionInfo> {
        for path in self.super_chain_inclusive(class) {
            let record = self.class_info(path)?;
            let found = record
                .get("functions")
                .and_then(Value::as_array)
                .and_then(|functions| {
                    functions.iter().find(|function| {
                        ["name", "authoredName"].iter().any(|key| {
                            function
                                .get(*key)
                                .and_then(Value::as_str)
                                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
                        })
                    })
                });
            if let Some(function) = found {
                return Some(FunctionInfo::from_record(function));
            }
        }
        None
    }

    fn derives_from_any(&self, class: &str, ancestors: &[&str]) -> bool {
        let chain = self.super_chain_inclusive(class);
        ancestors
            .iter()
            .filter_map(|ancestor| self.class_path(ancestor))
            .any(|ancestor| chain.contains(&ancestor))
    }

    /// Whether a type may cross the call boundary. `Err(None)` = not allowed, `Err(Some(..))` =
    /// reason; `Ok(false)` = allowed; `Ok(true)` = could not be verified.
    fn type_allowed(
        &self,
        property: &PropertyInfo,
        depth: u8,
        allow_object: bool,
    ) -> Result<bool, String> {
        match property.kind {
            PropertyKind::Bool
            | PropertyKind::Int8
            | PropertyKind::Int16
            | PropertyKind::Int32
            | PropertyKind::Int64
            | PropertyKind::UInt16
            | PropertyKind::UInt32
            | PropertyKind::UInt64
            | PropertyKind::Byte
            | PropertyKind::Float
            | PropertyKind::Double
            | PropertyKind::Str
            | PropertyKind::Name
            | PropertyKind::Text
            | PropertyKind::Enum
            | PropertyKind::Class
            | PropertyKind::SoftClass => Ok(false),
            PropertyKind::Object | PropertyKind::SoftObject if allow_object => Ok(false),
            PropertyKind::Struct => {
                let Some(path) = property.struct_path.as_deref() else {
                    return Ok(true);
                };
                if path.ends_with(".GameplayTag") {
                    return Ok(false);
                }
                if depth >= 3 {
                    return Err(format!(
                        "struct {} nests deeper than 3 levels",
                        short_name(path)
                    ));
                }
                let Some(fields) = self.struct_properties(path) else {
                    return Ok(true);
                };
                let mut unverified = false;
                for field in fields {
                    match self.type_allowed(field, depth + 1, false) {
                        Ok(unknown) => unverified |= unknown,
                        Err(reason) => {
                            return Err(format!(
                                "struct {} has unsupported field {}: {reason}",
                                short_name(path),
                                field.display_name()
                            ))
                        }
                    }
                }
                Ok(unverified)
            }
            _ => Err(format!("type {} is not supported", display_type(property))),
        }
    }

    /// Applies the call allowlist of the template engine to `function`.
    ///
    /// `target_class` (when known) is used to decide whether a single class parameter is
    /// compatible with the implicit class argument.
    pub fn callable(
        &self,
        function: &FunctionInfo,
        target: TargetKind,
        target_class: Option<&str>,
    ) -> Callable {
        let no = |reason: &str| Callable::No(reason.to_owned());
        if !function.has_flag("Native") {
            return no("only native functions can be called");
        }
        let is_static = function.is_static();
        for forbidden in [
            "Net",
            "NetReliable",
            "NetRequest",
            "NetMulticast",
            "NetServer",
            "NetClient",
            "Exec",
            "Delegate",
            "MulticastDelegate",
            "BlueprintEvent",
        ] {
            if function.has_flag(forbidden) {
                return Callable::No(format!("{forbidden} functions cannot be called"));
            }
        }
        if !(function.has_flag("BlueprintPure") || (!is_static && function.has_flag("Const"))) {
            return no("the function must be BlueprintPure or a const member");
        }
        let Some(ret) = &function.ret else {
            return no("the function returns no value");
        };
        let mut unverified = None;
        let mut inputs: Vec<&ParamInfo> = Vec::new();
        for param in &function.params {
            let input = !param.is_out || (param.is_reference && param.is_const);
            if !input {
                return Callable::No(format!(
                    "output parameter {} is not supported",
                    param.property.display_name()
                ));
            }
            match self.type_allowed(&param.property, 0, true) {
                Ok(false) => {}
                Ok(true) => {
                    unverified = Some(format!("parameter {}", param.property.display_name()))
                }
                Err(reason) => {
                    return Callable::No(format!(
                        "parameter {}: {reason}",
                        param.property.display_name()
                    ))
                }
            }
            inputs.push(param);
        }
        match self.type_allowed(ret, 0, false) {
            Ok(false) => {}
            Ok(true) => unverified = Some("return value".to_owned()),
            Err(reason) => return Callable::No(format!("return value: {reason}")),
        }
        if !is_static
            && self.derives_from_any(&function.owner_path, &[ACTOR, ACTOR_COMPONENT, SUBSYSTEM])
        {
            return no("non-static members of actors, components and subsystems are not callable");
        }
        let mut implicit = Implicit::None;
        if inputs.len() == 1 {
            let only = &inputs[0].property;
            match target {
                TargetKind::Class if only.kind == PropertyKind::Class => {
                    let compatible = match (only.meta_class.as_deref(), target_class) {
                        (Some(meta), Some(class)) => {
                            self.class_path(meta).is_none() || self.is_subclass_of(class, meta)
                        }
                        _ => true,
                    };
                    if compatible {
                        implicit = Implicit::Class;
                    }
                }
                TargetKind::Object if is_static && only.kind.is_object_reference() => {
                    let compatible = match (only.class_path.as_deref(), target_class) {
                        (Some(expected), Some(class)) => {
                            self.class_path(expected).is_none()
                                || self.is_subclass_of(class, expected)
                        }
                        _ => true,
                    };
                    if compatible {
                        implicit = Implicit::Object;
                    }
                }
                _ => {}
            }
        }
        Callable::Yes {
            implicit,
            explicit_args: inputs.len() - usize::from(implicit != Implicit::None),
            unverified,
        }
    }

    /// `Name(a: T, b: U) -> R`; the implicit parameter is left out.
    pub fn signature(&self, function: &FunctionInfo, implicit: Implicit) -> String {
        let parameters = explicit_params(function, implicit)
            .into_iter()
            .map(|param| {
                format!(
                    "{}: {}",
                    param.property.display_name(),
                    display_type(&param.property)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let ret = function
            .ret
            .as_ref()
            .map(|ret| format!(" -> {}", display_type(ret)))
            .unwrap_or_default();
        format!("{}({parameters}){ret}", function.display_name())
    }

    /// Properties and callable functions of `class` and its ancestors (nearest class wins on
    /// duplicate names), properties first, each group sorted by display name.
    pub fn list_members(&self, class: &str, target: TargetKind) -> Vec<MemberInfo> {
        self.list_members_matching(class, target, &|_| true)
    }

    /// Like [`Self::list_members`], but only members whose display name passes `keep` are built
    /// (the call allowlist is evaluated for those only). Shadowing still follows the full chain.
    pub fn list_members_matching(
        &self,
        class: &str,
        target: TargetKind,
        keep: &dyn Fn(&str) -> bool,
    ) -> Vec<MemberInfo> {
        let chain = self.super_chain_inclusive(class);
        let target_class = chain.first().copied();
        let mut seen = std::collections::HashSet::<(MemberKind, String)>::new();
        let mut properties = Vec::new();
        let mut properties_available = true;
        let mut functions = Vec::new();
        let names = |record: &Value| -> (String, String) {
            let text = |key: &str| {
                record
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            (text("name"), text("authoredName"))
        };
        let wanted =
            |name: &str, authored: &str| keep(if authored.is_empty() { name } else { authored });
        for path in &chain {
            let Some(record) = self.class_info(path) else {
                properties_available = false;
                continue;
            };
            properties_available &= record.get("properties").is_some_and(Value::is_array);
            for property in record
                .get("properties")
                .and_then(Value::as_array)
                .filter(|_| properties_available)
                .into_iter()
                .flatten()
            {
                let (name, authored) = names(property);
                if !seen.insert((MemberKind::Property, name.to_lowercase())) {
                    continue;
                }
                if !wanted(&name, &authored) {
                    continue;
                }
                let info = PropertyInfo::from_record(property);
                properties.push(MemberInfo {
                    name: info.display_name().to_owned(),
                    kind: MemberKind::Property,
                    owner_path: info.owner_path.clone(),
                    type_label: display_type(&info),
                    signature: None,
                    implicit: Implicit::None,
                    reload: info.reload,
                    callable: None,
                    property: Some(info),
                    function: None,
                });
            }
            for function in record
                .get("functions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let (name, authored) = names(function);
                if !seen.insert((MemberKind::Function, name.to_lowercase())) {
                    continue;
                }
                if !wanted(&name, &authored) {
                    continue;
                }
                let info = FunctionInfo::from_record(function);
                let callable = self.callable(&info, target, target_class);
                let implicit = match &callable {
                    Callable::Yes { implicit, .. } => *implicit,
                    _ => Implicit::None,
                };
                if !callable.is_callable() {
                    continue;
                }
                functions.push(MemberInfo {
                    name: info.display_name().to_owned(),
                    kind: MemberKind::Function,
                    owner_path: info.owner_path.clone(),
                    type_label: info.ret.as_ref().map(display_type).unwrap_or_default(),
                    signature: Some(self.signature(&info, implicit)),
                    implicit,
                    reload: None,
                    callable: Some(callable),
                    property: None,
                    function: Some(info),
                });
            }
        }
        properties.sort_by_key(|member| member.name.to_lowercase());
        functions.sort_by_key(|member| member.name.to_lowercase());
        properties.extend(functions);
        properties
    }

    /// Classifier for the properties of `class`; `apply_to_spawned_actors` turns `newActors` into
    /// `hot`. Cheaper than calling [`Self::property_reload_with`] once per property: the class chain,
    /// the catalog entries that apply and the actor check are computed once.
    pub fn reload_classifier(
        &self,
        class: &str,
        apply_to_spawned_actors: bool,
    ) -> ReloadClassifier<'_> {
        let Some(class_path) = self.class_path(class) else {
            return ReloadClassifier {
                known: false,
                actor: false,
                startup_bound: false,
                apply_to_spawned_actors,
                entries: HashMap::new(),
            };
        };
        let chain = self.super_chain_inclusive(class_path);
        let actor = self.derives_from_any(class_path, &[ACTOR, ACTOR_COMPONENT]);
        // A datapack without catalog (exported before it existed) falls back to the built-in one.
        let catalog = self
            .cold_catalog()
            .unwrap_or_else(|| crate::cold::builtin_catalog());
        let mut best: HashMap<String, (usize, &ColdEntry)> = HashMap::new();
        for entry in &catalog.entries {
            let Some(wanted) = self.class_path(&entry.class) else {
                continue;
            };
            let Some(distance) = chain.iter().position(|path| *path == wanted) else {
                continue;
            };
            let key = entry.property.to_ascii_lowercase();
            let better = match best.get(&key) {
                None => true,
                Some((current, existing)) => {
                    distance < *current
                        || (distance == *current
                            && entry.reload == Reload::Hot
                            && existing.reload != Reload::Hot)
                }
            };
            if better {
                best.insert(key, (distance, entry));
            }
        }
        ReloadClassifier {
            known: true,
            actor,
            startup_bound: chain
                .iter()
                .any(|path| path.eq_ignore_ascii_case(CDO_HELPER)),
            apply_to_spawned_actors,
            entries: best
                .into_iter()
                .map(|(key, (_, entry))| (key, entry))
                .collect(),
        }
    }

    /// Reload classification of the property `path` of `class` (class defaults patched, existing
    /// actors not updated). Same as [`Self::property_reload_with`] with `false`.
    pub fn property_reload(&self, class: &str, path: &str) -> (Reload, String) {
        self.property_reload_with(class, path, false)
    }

    /// Order: catalog entry (a `hot` entry whitelists; the datapack catalog, else the built-in
    /// one), exporter annotation `kdf.reload`, then the actor rule. `apply_to_spawned_actors` turns `newActors` into `hot`.
    pub fn property_reload_with(
        &self,
        class: &str,
        path: &str,
        apply_to_spawned_actors: bool,
    ) -> (Reload, String) {
        let top = path
            .split(['.', '['])
            .next()
            .unwrap_or(path)
            .trim()
            .to_owned();
        let Some(class_path) = self.class_path(class).map(str::to_owned) else {
            return (Reload::Hot, String::new());
        };
        let classifier = self.reload_classifier(&class_path, apply_to_spawned_actors);
        match self.find_property(&class_path, &top) {
            // The catalog is matched by the name that was asked for, like the classifier does for
            // the record's own names.
            Some(property) => match classifier.catalog_entry(&top) {
                Some(entry) => classifier.settle(entry.reload, entry.reason.clone()),
                None => classifier.classify(&property),
            },
            None => classifier.classify_name(&top),
        }
    }
}

fn short_name(path: &str) -> &str {
    path.rsplit(['.', '/']).next().unwrap_or(path)
}

/// Type text for signatures: the C++ type, else the property kind.
pub fn display_type(property: &PropertyInfo) -> String {
    if property.cpp_type.is_empty() {
        property.kind_name.clone()
    } else {
        property.cpp_type.clone()
    }
}
