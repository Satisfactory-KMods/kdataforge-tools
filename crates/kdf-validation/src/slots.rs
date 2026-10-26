//! Slot tables: what a config value may fill (compatibility) and where hot values are allowed.
//!
//! One table shared by validation, completion and hover (runtime design 1.7, tooling design 2.4).

use crate::config::{ConfigField, FieldKind, ValueType};
use crate::reflection_query::{PropertyInfo, PropertyKind};
use crate::setting_index::{SettingRecord, SettingValueType};
use serde::Serialize;
use serde_json::Value;

/// Integer widths a property or operand can have.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum IntWidth {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
}

impl IntWidth {
    pub fn range(self) -> (f64, f64) {
        match self {
            IntWidth::I8 => (i8::MIN as f64, i8::MAX as f64),
            IntWidth::I16 => (i16::MIN as f64, i16::MAX as f64),
            IntWidth::I32 => (i32::MIN as f64, i32::MAX as f64),
            IntWidth::I64 => (i64::MIN as f64, i64::MAX as f64),
            IntWidth::U8 => (0.0, u8::MAX as f64),
            IntWidth::U16 => (0.0, u16::MAX as f64),
            IntWidth::U32 => (0.0, u32::MAX as f64),
            IntWidth::U64 => (0.0, u64::MAX as f64),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            IntWidth::I8 => "int8",
            IntWidth::I16 => "int16",
            IntWidth::I32 => "int32",
            IntWidth::I64 => "int64",
            IntWidth::U8 => "uint8",
            IntWidth::U16 => "uint16",
            IntWidth::U32 => "uint32",
            IntWidth::U64 => "uint64",
        }
    }
}

/// What a value position expects.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Slot {
    Bool,
    Int(IntWidth),
    Float,
    Double,
    /// Any number (numeric operations such as `add`, `clamp` bounds, `where` comparisons).
    Number,
    /// Non-negative integer array index (`index`, `with`).
    Index,
    Str,
    Name,
    Text,
    /// Enum with its entry names (empty when unknown).
    Enum(Vec<String>),
    Class,
    SoftClass,
    Object,
    SoftObject,
    GameplayTag,
    TagContainer,
    Array(Box<Slot>),
    Set(Box<Slot>),
    Map,
    /// Struct with its path when known.
    Struct(Option<String>),
    Other,
    /// Nothing known: every value is accepted.
    Unknown,
}

impl Slot {
    /// Slot for the value of `property`.
    pub fn from_property(property: &PropertyInfo) -> Slot {
        match property.kind {
            PropertyKind::Bool => Slot::Bool,
            PropertyKind::Int8 => Slot::Int(IntWidth::I8),
            PropertyKind::Int16 => Slot::Int(IntWidth::I16),
            PropertyKind::Int32 => Slot::Int(IntWidth::I32),
            PropertyKind::Int64 => Slot::Int(IntWidth::I64),
            PropertyKind::UInt16 => Slot::Int(IntWidth::U16),
            PropertyKind::UInt32 => Slot::Int(IntWidth::U32),
            PropertyKind::UInt64 => Slot::Int(IntWidth::U64),
            PropertyKind::Byte if property.enum_path.is_some() => {
                Slot::Enum(property.enum_options.clone())
            }
            PropertyKind::Byte => Slot::Int(IntWidth::U8),
            PropertyKind::Float => Slot::Float,
            PropertyKind::Double => Slot::Double,
            PropertyKind::Str => Slot::Str,
            PropertyKind::Name => Slot::Name,
            PropertyKind::Text => Slot::Text,
            PropertyKind::Enum => Slot::Enum(property.enum_options.clone()),
            PropertyKind::Class => Slot::Class,
            PropertyKind::SoftClass => Slot::SoftClass,
            PropertyKind::Object | PropertyKind::WeakObject | PropertyKind::LazyObject => {
                Slot::Object
            }
            PropertyKind::SoftObject => Slot::SoftObject,
            PropertyKind::Struct => match property.struct_path.as_deref() {
                Some(path) if path.ends_with(".GameplayTag") => Slot::GameplayTag,
                Some(path) if path.ends_with(".GameplayTagContainer") => Slot::TagContainer,
                other => Slot::Struct(other.map(str::to_owned)),
            },
            PropertyKind::Array => Slot::Array(Box::new(
                property
                    .inner
                    .as_deref()
                    .map_or(Slot::Unknown, Slot::from_property),
            )),
            PropertyKind::Set => Slot::Set(Box::new(
                property
                    .inner
                    .as_deref()
                    .map_or(Slot::Unknown, Slot::from_property),
            )),
            PropertyKind::Map => Slot::Map,
            PropertyKind::Optional => property
                .inner
                .as_deref()
                .map_or(Slot::Unknown, Slot::from_property),
            PropertyKind::Interface | PropertyKind::Delegate | PropertyKind::Other => Slot::Other,
        }
    }
    /// Element slot of an array or set (`Unknown` otherwise).
    pub fn element(&self) -> Slot {
        match self {
            Slot::Array(inner) | Slot::Set(inner) => (**inner).clone(),
            _ => Slot::Unknown,
        }
    }
    /// Short text for messages and hovers.
    pub fn label(&self) -> String {
        match self {
            Slot::Bool => "bool".into(),
            Slot::Int(width) => width.label().into(),
            Slot::Float => "float".into(),
            Slot::Double => "double".into(),
            Slot::Number => "number".into(),
            Slot::Index => "array index".into(),
            Slot::Str => "string".into(),
            Slot::Name => "name".into(),
            Slot::Text => "text".into(),
            Slot::Enum(_) => "enum".into(),
            Slot::Class => "class".into(),
            Slot::SoftClass => "soft class".into(),
            Slot::Object => "object".into(),
            Slot::SoftObject => "soft object".into(),
            Slot::GameplayTag => "gameplay tag".into(),
            Slot::TagContainer => "gameplay tag container".into(),
            Slot::Array(inner) => format!("array of {}", inner.label()),
            Slot::Set(inner) => format!("set of {}", inner.label()),
            Slot::Map => "map".into(),
            Slot::Struct(Some(path)) => {
                format!("struct {}", path.rsplit('.').next().unwrap_or(path))
            }
            Slot::Struct(None) => "struct".into(),
            Slot::Other => "unsupported type".into(),
            Slot::Unknown => "any".into(),
        }
    }
}

/// How well a config value fits a slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum Compat {
    /// Direct fit.
    Exact,
    /// Fits after a conversion the runtime performs (text to class path, ordinal to enum, ...).
    Convertible,
    /// May lose information or fail for some values; reported as a warning, not offered.
    Lossy,
    /// Never fits; reported as an error.
    Incompatible,
}

impl Compat {
    /// Offered by completion (`Exact` and `Convertible`).
    pub fn offered(self) -> bool {
        matches!(self, Compat::Exact | Compat::Convertible)
    }
    /// Sort order for completion: 0 exact, 1 convertible, 2 lossy, 3 incompatible.
    pub fn rank(self) -> u8 {
        match self {
            Compat::Exact => 0,
            Compat::Convertible => 1,
            Compat::Lossy => 2,
            Compat::Incompatible => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum ShapeKind {
    Bool,
    Integer,
    Float,
    /// `text` field of the given value type (`string`, `text`, `name`, `tag`).
    Text(ValueType),
    /// Select with string-like option values (the option strings are listed for enum checks).
    SelectString(Vec<String>),
    SelectNumber {
        integer: bool,
    },
    SelectBool,
    Unknown,
}

/// The value a `cfg:`/`session:`/`ags:`/`option:` reference produces.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ValueShape {
    pub kind: ShapeKind,
    /// List value (`multiple: true`).
    pub multiple: bool,
    /// Explicit `[min, max]` of an integer field.
    pub bounds: Option<(f64, f64)>,
    /// Numeric defaults (used when no explicit bounds exist).
    pub defaults: Vec<f64>,
    /// Width of the integer value type.
    pub width: Option<(f64, f64)>,
}

impl ValueShape {
    fn plain(kind: ShapeKind) -> Self {
        Self {
            kind,
            multiple: false,
            bounds: None,
            defaults: Vec::new(),
            width: None,
        }
    }
}

/// Shape of a config field's value.
pub fn field_shape(field: &ConfigField) -> ValueShape {
    let numbers = |value: &Value| -> Vec<f64> {
        match value {
            Value::Array(items) => items.iter().filter_map(Value::as_f64).collect(),
            other => other.as_f64().into_iter().collect(),
        }
    };
    let mut shape = match field.kind {
        FieldKind::Boolean => ValueShape::plain(ShapeKind::Bool),
        FieldKind::Number if field.value_type.is_integer() => ValueShape::plain(ShapeKind::Integer),
        FieldKind::Number => ValueShape::plain(ShapeKind::Float),
        FieldKind::Text => ValueShape::plain(ShapeKind::Text(field.value_type)),
        FieldKind::Select => {
            if field.value_type.is_string_like() {
                ValueShape::plain(ShapeKind::SelectString(
                    field
                        .options
                        .iter()
                        .filter_map(|option| option.value.as_str().map(str::to_owned))
                        .collect(),
                ))
            } else if field.value_type == ValueType::Bool {
                ValueShape::plain(ShapeKind::SelectBool)
            } else {
                ValueShape::plain(ShapeKind::SelectNumber {
                    integer: field.value_type.is_integer(),
                })
            }
        }
    };
    shape.multiple = field.multiple;
    if let (Some(min), Some(max)) = (field.min, field.max) {
        shape.bounds = Some((min, max));
    }
    shape.defaults = numbers(&field.default);
    shape.width = field
        .value_type
        .integer_range()
        .map(|(min, max)| (min as f64, max as f64));
    shape
}

/// Shape of a setting's value (`session:`/`ags:`/`option:`).
pub fn setting_shape(setting: &SettingRecord) -> ValueShape {
    let mut shape = match setting.value_type {
        SettingValueType::Bool => ValueShape::plain(ShapeKind::Bool),
        SettingValueType::Int => ValueShape::plain(ShapeKind::Integer),
        SettingValueType::Float => ValueShape::plain(ShapeKind::Float),
        SettingValueType::String => ValueShape::plain(ShapeKind::Text(ValueType::String)),
        SettingValueType::Color | SettingValueType::Other => ValueShape::plain(ShapeKind::Unknown),
    };
    shape.bounds = setting.range;
    shape.width = Some(IntWidth::I32.range());
    shape
        .defaults
        .extend(setting.default.as_ref().and_then(Value::as_f64));
    shape
}

/// Slot expected by operand `operand` (`value`, `min`, `max`, `index`, `with`, `lessThan`, ...) of an
/// operation `op` (`None` = `set`) on `property` (when known).
pub fn expected_slot(op: Option<&str>, operand: &str, property: Option<&PropertyInfo>) -> Slot {
    match operand {
        "min" | "max" | "lessThan" | "lessOrEqual" | "greaterThan" | "greaterOrEqual" => {
            Slot::Number
        }
        "index" | "with" => Slot::Index,
        "from" if op.is_some_and(|op| op.eq_ignore_ascii_case("replace_refs")) => Slot::Unknown,
        "value" => {
            let op = op.unwrap_or("set").to_ascii_lowercase();
            match op.as_str() {
                "add" | "subtract" | "multiply" | "divide" | "min" | "max" => Slot::Number,
                "set" => property.map_or(Slot::Unknown, Slot::from_property),
                "replace_refs" => Slot::Unknown,
                "append" | "append_unique" | "prepend" | "insert" | "replace" | "remove"
                | "update_where" => property
                    .map(Slot::from_property)
                    .map_or(Slot::Unknown, |slot| slot.element()),
                _ => Slot::Unknown,
            }
        }
        _ => Slot::Unknown,
    }
}

fn fits(range: (f64, f64), slot: (f64, f64)) -> bool {
    range.0 >= slot.0 && range.1 <= slot.1
}

fn integer_fit(shape: &ValueShape, slot: (f64, f64)) -> Compat {
    if let Some(bounds) = shape.bounds {
        return if fits(bounds, slot) {
            Compat::Exact
        } else {
            Compat::Lossy
        };
    }
    if !shape.defaults.is_empty() {
        return if shape
            .defaults
            .iter()
            .all(|value| fits((*value, *value), slot))
        {
            Compat::Exact
        } else {
            Compat::Lossy
        };
    }
    match shape.width {
        Some(width) if fits(width, slot) => Compat::Exact,
        _ => Compat::Lossy,
    }
}

fn scalar_compat(shape: &ValueShape, slot: &Slot) -> Compat {
    use Compat::*;
    match (&shape.kind, slot) {
        (_, Slot::Unknown) => Exact,
        (ShapeKind::Unknown, _) => Exact,
        // Boolean slot.
        (ShapeKind::Bool | ShapeKind::SelectBool, Slot::Bool) => Exact,
        (ShapeKind::Integer, Slot::Bool) => Lossy,
        (_, Slot::Bool) => Incompatible,
        // Integer slots.
        (ShapeKind::Integer, Slot::Int(width)) => integer_fit(shape, width.range()),
        (ShapeKind::Float, Slot::Int(_)) => Lossy,
        (ShapeKind::SelectNumber { .. }, Slot::Int(_)) => Convertible,
        (_, Slot::Int(_)) => Incompatible,
        // Index slots.
        (ShapeKind::Integer, Slot::Index) => integer_fit(shape, (0.0, i32::MAX as f64)),
        (ShapeKind::Float, Slot::Index) => Lossy,
        (ShapeKind::SelectNumber { integer: true }, Slot::Index) => Convertible,
        (_, Slot::Index) => Incompatible,
        // Floating point and generic numeric slots.
        (ShapeKind::Integer | ShapeKind::Float, Slot::Float | Slot::Double | Slot::Number) => Exact,
        (ShapeKind::SelectNumber { .. }, Slot::Float | Slot::Double | Slot::Number) => Convertible,
        (_, Slot::Float | Slot::Double | Slot::Number) => Incompatible,
        // String-like slots.
        (ShapeKind::Text(_) | ShapeKind::SelectString(_), Slot::Str) => Exact,
        (_, Slot::Str) => Incompatible,
        (ShapeKind::Text(ValueType::String | ValueType::Name | ValueType::Tag), Slot::Name) => {
            Exact
        }
        (ShapeKind::SelectString(_), Slot::Name) => Convertible,
        (_, Slot::Name) => Incompatible,
        (ShapeKind::Text(ValueType::String | ValueType::Text | ValueType::Name), Slot::Text) => {
            Exact
        }
        (ShapeKind::SelectString(_), Slot::Text) => Convertible,
        (_, Slot::Text) => Incompatible,
        // Enums.
        (ShapeKind::SelectString(options), Slot::Enum(entries)) => {
            let known = entries.is_empty()
                || options.iter().all(|option| {
                    entries
                        .iter()
                        .any(|entry| entry.eq_ignore_ascii_case(option))
                });
            if known {
                Exact
            } else {
                Lossy
            }
        }
        (ShapeKind::Text(_), Slot::Enum(_)) => Convertible,
        (ShapeKind::Integer | ShapeKind::SelectNumber { integer: true }, Slot::Enum(_)) => {
            Convertible
        }
        (_, Slot::Enum(_)) => Incompatible,
        // Object references are written as paths.
        (
            ShapeKind::Text(ValueType::String) | ShapeKind::SelectString(_),
            Slot::Class | Slot::SoftClass | Slot::Object | Slot::SoftObject,
        ) => Convertible,
        (_, Slot::Class | Slot::SoftClass | Slot::Object | Slot::SoftObject) => Incompatible,
        (ShapeKind::Text(ValueType::Tag), Slot::GameplayTag) => Exact,
        (ShapeKind::Text(ValueType::String), Slot::GameplayTag) => Convertible,
        (_, Slot::GameplayTag) => Incompatible,
        (_, Slot::TagContainer) => Incompatible,
        (_, Slot::Array(_) | Slot::Set(_) | Slot::Map | Slot::Struct(_) | Slot::Other) => {
            Incompatible
        }
    }
}

/// Compatibility of a config value with a slot (design 2.4).
pub fn compat(shape: &ValueShape, slot: &Slot) -> Compat {
    match slot {
        Slot::Unknown => Compat::Exact,
        Slot::Array(element) | Slot::Set(element) => {
            if shape.multiple {
                let single = ValueShape {
                    multiple: false,
                    ..shape.clone()
                };
                scalar_compat(&single, element)
            } else {
                Compat::Incompatible
            }
        }
        Slot::TagContainer => match (&shape.kind, shape.multiple) {
            (ShapeKind::Text(ValueType::Tag), true) => Compat::Exact,
            (ShapeKind::Text(ValueType::String), true) => Compat::Convertible,
            (ShapeKind::Unknown, _) => Compat::Exact,
            _ => Compat::Incompatible,
        },
        _ if shape.multiple && shape.kind != ShapeKind::Unknown => Compat::Incompatible,
        other => scalar_compat(shape, other),
    }
}

/// `true` when values in this place are read once at load time ("cold" context), so a hot config
/// field used there needs a restart. `segments` are the JSON pointer segments below the document
/// root (`["recipes", "0", "properties", "1", "value"]`, `["patches", "0", "properties", ...]`).
///
/// Hot-capable: every value of `cdo`, `dataasset`, `curve`, `asset` and `localization` documents,
/// and `properties:` subtrees of the content kinds (`item`, `resource`, `building`, `unlock`,
/// `class`, `recipe`, `schematic`, `research`). Everything else is cold.
pub fn is_cold_context(kind: &str, segments: &[&str]) -> bool {
    match kind {
        "cdo" | "dataasset" | "curve" | "asset" | "localization" => false,
        "item" | "resource" | "building" | "unlock" | "class" | "recipe" | "schematic"
        | "research" => {
            if segments.first() == Some(&"remove") {
                return true;
            }
            !segments
                .iter()
                .skip(1)
                .any(|segment| *segment == "properties")
        }
        _ => true,
    }
}

/// Class a generated entry derives from when it has no `parent` (`items` -> item descriptor, ...).
pub fn default_base_class(entries_key: &str) -> Option<&'static str> {
    match entries_key {
        "items" => Some("/Script/FactoryGame.FGItemDescriptor"),
        "resources" => Some("/Script/FactoryGame.FGResourceDescriptor"),
        // A `building` document generates building descriptors, not buildable actors.
        "buildings" => Some("/Script/FactoryGame.FGBuildingDescriptor"),
        "unlocks" => Some("/Script/FactoryGame.FGUnlock"),
        "recipes" => Some("/Script/FactoryGame.FGRecipe"),
        "schematics" => Some("/Script/FactoryGame.FGSchematic"),
        "research" => Some("/Script/FactoryGame.FGResearchTree"),
        _ => None,
    }
}

/// Document-level conditions of hot-capable kinds are still evaluated when reloading; of content
/// kinds and all others they are load-time only.
pub fn is_cold_condition(kind: &str, segments: &[&str]) -> bool {
    match kind {
        "cdo" | "dataasset" | "curve" | "asset" | "localization" => false,
        _ => is_cold_context(kind, segments),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn integer(min: Option<(f64, f64)>) -> ValueShape {
        ValueShape {
            bounds: min,
            width: Some(IntWidth::I32.range()),
            ..ValueShape::plain(ShapeKind::Integer)
        }
    }

    #[test]
    fn compat_table() {
        let shape = integer(Some((0.0, 10.0)));
        assert_eq!(compat(&shape, &Slot::Int(IntWidth::I8)), Compat::Exact);
        assert_eq!(compat(&shape, &Slot::Float), Compat::Exact);
        assert_eq!(compat(&shape, &Slot::Bool), Compat::Lossy);
        assert_eq!(compat(&shape, &Slot::Str), Compat::Incompatible);
        let wide = integer(Some((0.0, 1000.0)));
        assert_eq!(compat(&wide, &Slot::Int(IntWidth::I8)), Compat::Lossy);
        let text = ValueShape::plain(ShapeKind::Text(ValueType::Tag));
        assert_eq!(compat(&text, &Slot::GameplayTag), Compat::Exact);
        assert_eq!(compat(&text, &Slot::Class), Compat::Incompatible);
        let list = ValueShape {
            multiple: true,
            ..ValueShape::plain(ShapeKind::Text(ValueType::Tag))
        };
        assert_eq!(compat(&list, &Slot::TagContainer), Compat::Exact);
        assert_eq!(
            compat(&list, &Slot::Array(Box::new(Slot::GameplayTag))),
            Compat::Exact
        );
        assert_eq!(
            compat(&text, &Slot::Array(Box::new(Slot::GameplayTag))),
            Compat::Incompatible
        );
        assert_eq!(compat(&list, &Slot::Str), Compat::Incompatible);
    }

    #[test]
    fn cold_contexts() {
        assert!(!is_cold_context(
            "cdo",
            &["patches", "0", "properties", "0", "value"]
        ));
        assert!(!is_cold_context(
            "recipe",
            &["recipes", "0", "properties", "0", "value"]
        ));
        assert!(is_cold_context(
            "recipe",
            &["recipes", "0", "ingredients", "0", "amount"]
        ));
        assert!(is_cold_context("recipe", &["remove", "0"]));
        assert!(is_cold_context("gametag", &["tags", "0"]));
        assert!(!is_cold_context("localization", &["entries", "Key"]));
    }
}
