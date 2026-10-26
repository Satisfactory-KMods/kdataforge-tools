//! Element-level conditions and `config:` condition sites.
//!
//! Every map element of every sequence (at any depth) may carry `conditions`, `conditionBehavior`
//! and `conditionBehaivor`. The runtime evaluates them in a pre-pass and removes or strips the
//! element. The validator mirrors that: [`strip_element_conditions`] lifts those keys out of a
//! working copy (so kind schemas never see them) and returns the lifted sites.

use crate::expression::{self, EvalValue, Operand};
use serde_json::{Map, Value};

/// Keys that make a map element conditional.
pub const HOLDER_KEYS: [&str; 3] = ["conditions", "conditionBehavior", "conditionBehaivor"];

#[derive(Clone, Debug, PartialEq)]
pub struct ConditionSite {
    /// JSON pointer of the element that carried the keys.
    pub pointer: String,
    /// The lifted keys (`conditions`, `conditionBehavior`, `conditionBehaivor`).
    pub holder: Map<String, Value>,
}

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Removes the condition keys from every sequence element map and returns the sites in document
/// order. Condition holders at the document root and anything inside a `conditions` subtree stay.
pub fn strip_element_conditions(document: &mut Value) -> Vec<ConditionSite> {
    let mut sites = Vec::new();
    let mut pointer = String::new();
    walk(document, &mut pointer, &mut sites);
    sites
}

fn walk(value: &mut Value, pointer: &mut String, sites: &mut Vec<ConditionSite>) {
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                let length = pointer.len();
                pointer.push('/');
                pointer.push_str(&index.to_string());
                if let Value::Object(map) = item {
                    let mut holder = Map::new();
                    for key in HOLDER_KEYS {
                        if let Some(lifted) = map.remove(key) {
                            holder.insert(key.to_owned(), lifted);
                        }
                    }
                    if !holder.is_empty() {
                        sites.push(ConditionSite {
                            pointer: pointer.clone(),
                            holder,
                        });
                    }
                }
                walk(item, pointer, sites);
                pointer.truncate(length);
            }
        }
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if HOLDER_KEYS.contains(&key.as_str()) {
                    continue;
                }
                let length = pointer.len();
                pointer.push('/');
                pointer.push_str(&escape(key));
                walk(child, pointer, sites);
                pointer.truncate(length);
            }
        }
        _ => {}
    }
}

/// Condition objects of a `conditions` value (a map or a sequence of maps).
pub fn condition_objects(conditions: &Value) -> Vec<(String, &Map<String, Value>)> {
    match conditions {
        Value::Object(map) => vec![(String::new(), map)],
        Value::Array(items) => items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| item.as_object().map(|map| (format!("/{index}"), map)))
            .collect(),
        _ => Vec::new(),
    }
}

/// `config:` value of a condition object: one expression or a sequence of expressions.
/// Returns `(relative pointer, text)` pairs; entries that are not strings are skipped.
pub fn config_expressions(object: &Map<String, Value>) -> Vec<(String, &str)> {
    let Some((key, value)) = object
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("config"))
    else {
        return Vec::new();
    };
    match value {
        Value::String(text) => vec![(format!("/{key}"), text.as_str())],
        Value::Array(items) => items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| item.as_str().map(|text| (format!("/{key}/{index}"), text)))
            .collect(),
        _ => Vec::new(),
    }
}

fn behavior_is_or(holder: &Map<String, Value>) -> bool {
    HOLDER_KEYS
        .iter()
        .skip(1)
        .find_map(|key| holder.get(*key))
        .and_then(Value::as_str)
        .is_some_and(|text| text.eq_ignore_ascii_case("or"))
}

/// Evaluates the `config:` expressions of one condition object (all must hold). Static keys
/// (`hasMod`, ...) cannot be decided offline and count as satisfied. Evaluation errors fail.
fn object_passes(
    object: &Map<String, Value>,
    values: &dyn Fn(&Operand) -> Option<EvalValue>,
) -> bool {
    config_expressions(object).into_iter().all(|(_, text)| {
        expression::parse(text)
            .ok()
            .and_then(|expr| expression::eval(&expr, values).ok())
            .unwrap_or(false)
    })
}

/// Whether an element holder keeps its element, deciding `config:` expressions only.
pub fn holder_passes(
    holder: &Map<String, Value>,
    values: &dyn Fn(&Operand) -> Option<EvalValue>,
) -> bool {
    let Some(conditions) = holder.get("conditions") else {
        return true;
    };
    let objects = condition_objects(conditions);
    if objects.is_empty() {
        return true;
    }
    if behavior_is_or(holder) {
        objects
            .iter()
            .any(|(_, object)| object_passes(object, values))
    } else {
        objects
            .iter()
            .all(|(_, object)| object_passes(object, values))
    }
}

/// Mirrors the runtime pre-pass for `config:` conditions: elements whose holder fails are removed,
/// the others lose their condition keys. Used by the contract vectors and previews.
pub fn prune_config_conditions(
    document: &Value,
    values: &dyn Fn(&Operand) -> Option<EvalValue>,
) -> Value {
    match document {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .filter_map(|item| prune_element(item, values))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, child)| {
                    if HOLDER_KEYS.contains(&key.as_str()) {
                        (key.clone(), child.clone())
                    } else {
                        (key.clone(), prune_config_conditions(child, values))
                    }
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn prune_element(item: &Value, values: &dyn Fn(&Operand) -> Option<EvalValue>) -> Option<Value> {
    let Value::Object(map) = item else {
        return Some(prune_config_conditions(item, values));
    };
    let mut holder = Map::new();
    let mut rest = Map::new();
    for (key, child) in map {
        if HOLDER_KEYS.contains(&key.as_str()) {
            holder.insert(key.clone(), child.clone());
        } else {
            rest.insert(key.clone(), child.clone());
        }
    }
    if !holder.is_empty() && !holder_passes(&holder, values) {
        return None;
    }
    Some(prune_config_conditions(&Value::Object(rest), values))
}
