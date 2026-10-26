//! Lookups shared by completion, hover, signature help and navigation: class resolution through
//! generated classes, operand slots, template targets and type helpers.

use crate::context::Resolved;
use crate::state::ClientCaps;
use crate::workspace::{generated_path, Workspace};
use kdf_validation::config::{ConfigField, FieldKind, ValueType};
use kdf_validation::expression::ExprType;
use kdf_validation::slots::{expected_slot, Slot};
use kdf_validation::template::is_object_target;
use kdf_validation::{
    PropertyInfo, PropertyKind, ReflectionIndex, Reload, SettingRecord, SettingValueType,
    TargetKind,
};
use std::path::Path;

const MAX_CLASS_HOPS: usize = 8;

/// Everything a feature needs to answer a request about one document.
pub(crate) struct Env<'a> {
    pub(crate) path: &'a Path,
    pub(crate) text: &'a str,
    pub(crate) reflection: &'a ReflectionIndex,
    pub(crate) workspace: &'a Workspace,
    /// What the client can render and which optional behaviours it asked for.
    pub(crate) caps: ClientCaps,
}

impl Env<'_> {
    /// `ref` of the pack that owns the document.
    pub(crate) fn own_pack(&self) -> Option<&str> {
        self.workspace.pack_of(self.path)
    }

    /// Reflection class path of `raw`, walking generated classes through their parents. The
    /// reflection spelling wins over a generated id, like in the validator.
    pub(crate) fn class_in(&self, pack: Option<&str>, raw: &str) -> Option<String> {
        self.class_hops(pack, raw, 0)
    }

    fn class_hops(&self, pack: Option<&str>, raw: &str, depth: usize) -> Option<String> {
        if depth >= MAX_CLASS_HOPS {
            return None;
        }
        let raw = raw.trim();
        if let Some(path) = self.reflection.class_path(raw) {
            return Some(path.to_owned());
        }
        let decl = pack
            .and_then(|pack| self.workspace.class_of_pack(pack, raw))
            .or_else(|| {
                self.workspace.generated.iter().find(|decl| {
                    decl.kind == crate::generated::GeneratedKind::Class
                        && generated_path(decl).eq_ignore_ascii_case(raw)
                })
            })?;
        // The parent is resolved in the pack that declared the class.
        self.class_hops(Some(&decl.pack_ref), decl.parent.as_deref()?, depth + 1)
    }

    pub(crate) fn class_of(&self, raw: &str) -> Option<String> {
        self.class_in(self.own_pack(), raw)
    }

    /// `class` (a reflection path) equals or derives from `base`.
    pub(crate) fn derives(&self, class: &str, base: &str) -> bool {
        self.reflection.is_subclass_of(class, base)
    }
}

/// Class a patch or generated entry works on.
#[derive(Clone, Debug)]
pub(crate) struct PatchClass {
    pub(crate) class: String,
    /// Existing actors are not the target: spawned actors get the change (or the kind generates
    /// classes), so `newActors` properties are fine.
    pub(crate) apply_to_spawned: bool,
}

pub(crate) fn patch_class(env: &Env<'_>, resolved: &Resolved) -> Option<PatchClass> {
    let patch = resolved.patch_entry()?;
    let native = resolved
        .position()
        .map(|position| crate::intellisense::context(env.text, position));
    let raw = if let Some(native) = native.as_ref().filter(|context| context.has_selector) {
        // Produced-in / ingredient / product select recipe sets. Static aliases and
        // scalar/list target resolution use the same proven context as native completion.
        match native.targets.as_slice() {
            [target] => Some(target.clone()),
            [] => patch
                .class_text()
                .filter(|reference| env.class_of(reference).is_some()),
            _ => return None,
        }
    } else {
        patch
            .class_text()
            .or_else(|| patch.default_base().map(str::to_owned))
    }?;
    let class = env.class_of(&raw)?;
    Some(PatchClass {
        class,
        // Only cdo patches change defaults of classes that may already have actors.
        apply_to_spawned: patch.apply_to_spawned_actors()
            || resolved.doc.kind.as_deref() != Some("cdo"),
    })
}

/// Operand position of an operation (`value`, `min`, ...) or a `where` clause.
#[derive(Clone, Debug)]
pub(crate) struct OperandCtx {
    pub(crate) slot: Slot,
}

const OP_OPERANDS: [&str; 5] = ["value", "min", "max", "index", "with"];
const WHERE_OPERANDS: [&str; 10] = [
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
];

pub(crate) fn operand_context(env: &Env<'_>, resolved: &Resolved) -> Option<OperandCtx> {
    if let Some(op) = resolved.op_entry() {
        let operand = op.below.first()?.clone();
        if !OP_OPERANDS.contains(&operand.as_str()) {
            return None;
        }
        let op_name = op.op_name();
        let property = op.key_value("path").and_then(|path| {
            kdf_validation::tag_container_selector_property(path).or_else(|| {
                patch_class(env, resolved)
                    .and_then(|patch| env.reflection.resolve_property_path(&patch.class, path))
            })
        });
        let slot = if op.below.len() == 1 {
            expected_slot(Some(&op_name), &operand, property.as_ref())
        } else if operand == "value" {
            nested_slot(env, &op_name, property.as_ref(), &op.below[1..])
        } else {
            Slot::Unknown
        };
        return Some(OperandCtx { slot });
    }
    let key = resolved.key()?;
    if WHERE_OPERANDS.contains(&key) && resolved.path().iter().any(|segment| segment == "where") {
        return Some(OperandCtx {
            slot: expected_slot(None, key, None),
        });
    }
    None
}

/// Slot of a value nested in a struct, array or map (mirrors the validator).
fn nested_slot(
    env: &Env<'_>,
    op: &str,
    property: Option<&PropertyInfo>,
    relative: &[String],
) -> Slot {
    let Some(property) = property else {
        return Slot::Unknown;
    };
    let mut current = match op {
        "append" | "append_unique" | "prepend" | "insert" | "replace" | "remove"
        | "update_where" => match property.inner.as_deref() {
            Some(inner) => inner.clone(),
            None => return Slot::Unknown,
        },
        "set" => property.clone(),
        _ => return Slot::Unknown,
    };
    for segment in relative {
        current = match current.kind {
            PropertyKind::Array | PropertyKind::Set if segment == "#" => {
                match current.inner.as_deref() {
                    Some(inner) => inner.clone(),
                    None => return Slot::Unknown,
                }
            }
            PropertyKind::Map if segment == "#" => match current.value.as_deref() {
                Some(value) => value.clone(),
                None => return Slot::Unknown,
            },
            PropertyKind::Struct => {
                match current
                    .struct_path
                    .as_deref()
                    .and_then(|path| env.reflection.find_struct_field(path, segment))
                {
                    Some(field) => field,
                    None => return Slot::Unknown,
                }
            }
            _ => return Slot::Unknown,
        };
    }
    Slot::from_property(&current)
}

/// Class and kind a template target stands for.
#[derive(Clone, Debug)]
pub(crate) struct TemplateTarget {
    pub(crate) class: String,
    pub(crate) kind: TargetKind,
}

pub(crate) fn template_target(env: &Env<'_>, prefix: &str, target: &str) -> Option<TemplateTarget> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    if is_object_target(prefix) {
        let asset = env.reflection.find_asset(target)?;
        let class = env.reflection.class_path(&asset.asset_class)?.to_owned();
        return Some(TemplateTarget {
            class,
            kind: TargetKind::Object,
        });
    }
    Some(TemplateTarget {
        class: env.class_of(target)?,
        kind: TargetKind::Class,
    })
}

// ---- types -------------------------------------------------------------------------------------

/// Expression type of a config field's value.
pub(crate) fn field_expr_type(field: &ConfigField) -> ExprType {
    let scalar = match field.kind {
        FieldKind::Boolean => ExprType::Bool,
        FieldKind::Number => ExprType::Number,
        FieldKind::Text => ExprType::String,
        FieldKind::Select => {
            if field.value_type == ValueType::Bool {
                ExprType::Bool
            } else if field.value_type.is_number() {
                ExprType::Number
            } else {
                ExprType::String
            }
        }
    };
    if field.multiple {
        ExprType::List(Box::new(scalar))
    } else {
        scalar
    }
}

pub(crate) fn setting_expr_type(setting: &SettingRecord) -> ExprType {
    match setting.value_type {
        SettingValueType::Bool => ExprType::Bool,
        SettingValueType::Int | SettingValueType::Float => ExprType::Number,
        SettingValueType::String => ExprType::String,
        SettingValueType::Color | SettingValueType::Other => ExprType::Unknown,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Family {
    Any,
    Bool,
    Number,
    Text,
    Enum,
    Class,
    Object,
    Tag,
    TagContainer,
    List(Box<Family>),
    Struct(Option<String>),
    Other,
}

fn family(slot: &Slot) -> Family {
    match slot {
        Slot::Unknown => Family::Any,
        Slot::Bool => Family::Bool,
        Slot::Int(_) | Slot::Float | Slot::Double | Slot::Number | Slot::Index => Family::Number,
        Slot::Str | Slot::Name | Slot::Text => Family::Text,
        Slot::Enum(_) => Family::Enum,
        Slot::Class | Slot::SoftClass => Family::Class,
        Slot::Object | Slot::SoftObject => Family::Object,
        Slot::GameplayTag => Family::Tag,
        Slot::TagContainer => Family::TagContainer,
        Slot::Array(inner) | Slot::Set(inner) => Family::List(Box::new(family(inner))),
        Slot::Struct(path) => Family::Struct(path.clone()),
        Slot::Map | Slot::Other => Family::Other,
    }
}

/// A member of type `member` can fill `expected` without conversion (completion sorts these first).
pub(crate) fn slot_fits(member: &Slot, expected: &Slot) -> bool {
    let expected = family(expected);
    expected == Family::Any || family(member) == expected
}

/// `cold` / `newActors` tag of a property (hot properties get none).
pub(crate) fn reload_tag(reload: Reload) -> Option<&'static str> {
    match reload {
        Reload::Hot => None,
        Reload::Cold => Some("cold"),
        Reload::NewActors => Some("newActors"),
    }
}

/// Keys whose values the runtime never resolves (`id`, `type`, `include`, `ref`): no typed
/// references or templates there, so neither completion nor hover treat them as such.
pub(crate) fn unresolved_key(key: &str) -> bool {
    matches!(key, "id" | "type" | "include" | "ref")
}

/// Properties of `class` and its ancestors, nearest class first, without shadowed duplicates; only
/// properties whose display name passes `keep` are built (shadowing follows the whole chain).
pub(crate) fn class_properties_matching(
    reflection: &ReflectionIndex,
    class: &str,
    keep: &dyn Fn(&str) -> bool,
) -> Vec<PropertyInfo> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for path in reflection.super_chain_inclusive(class) {
        let Some(record) = reflection.class_info(path) else {
            continue;
        };
        for property in record
            .get("properties")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let text = |key: &str| {
                property
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
            };
            let (name, authored) = (text("name"), text("authoredName"));
            if !seen.insert(name.to_ascii_lowercase()) {
                continue;
            }
            if keep(if authored.is_empty() { name } else { authored }) {
                out.push(PropertyInfo::from_record(property));
            }
        }
    }
    out
}
