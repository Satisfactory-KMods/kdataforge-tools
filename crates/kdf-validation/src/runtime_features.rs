//! Stateful runtime document constraints that JSON Schema cannot express.
use crate::{document::emit, project::ParsedFile, LintResult, Severity};
use serde_json::Value;
use std::collections::HashSet;

pub(crate) fn validate(
    file: &ParsedFile,
    index: usize,
    document: &Value,
    kind: &str,
    result: &mut LintResult,
) {
    if kind == "resourcenode" {
        validate_node_visuals(file, index, document, result);
    }
    let collections: &[&str] = match kind {
        "eventrule" => &["rules"],
        "resourcenode" => &["add", "replace"],
        _ => &[],
    };
    let mut ids = HashSet::new();
    for collection in collections {
        for (entry_index, entry) in document
            .get(*collection)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            if let Some(id) = entry.get("id").and_then(Value::as_str) {
                let identity = if kind == "eventrule" {
                    id.to_lowercase()
                } else {
                    id.to_owned()
                };
                if !ids.insert(identity) {
                    emit(
                        file,
                        index,
                        &format!("/{collection}/{entry_index}/id"),
                        Severity::Error,
                        "semantic.runtime-identity",
                        "Runtime IDs must be unique within the pack (event-rule IDs ignore case).",
                        result,
                    );
                }
            }
        }
    }
    if kind != "cdo" {
        return;
    }
    for (patch_index, patch) in document
        .get("patches")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        validate_inventory_remaps(file, index, patch_index, patch, result);
        let Some(bounds) = patch.pointer("/actorSelector/box") else {
            continue;
        };
        let min = bounds.get("min").and_then(Value::as_array);
        let max = bounds.get("max").and_then(Value::as_array);
        if let (Some(min), Some(max)) = (min, max) {
            if min
                .iter()
                .zip(max)
                .any(|(a, b)| a.as_f64().zip(b.as_f64()).is_some_and(|(a, b)| a >= b))
            {
                emit(
                    file,
                    index,
                    &format!("/patches/{patch_index}/actorSelector/box"),
                    Severity::Error,
                    "semantic.actor-bounds",
                    "Actor selector box min must be less than max on every axis.",
                    result,
                );
            }
        }
    }
}

fn validate_node_visuals(
    file: &ParsedFile,
    index: usize,
    document: &Value,
    result: &mut LintResult,
) {
    let mut resources = HashSet::new();
    for (rule_index, rule) in document
        .get("visuals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        if let Some(resource) = rule.get("resource").and_then(Value::as_str) {
            if !resources.insert(resource.trim().to_lowercase()) {
                emit(
                    file,
                    index,
                    &format!("/visuals/{rule_index}/resource"),
                    Severity::Error,
                    "semantic.visual-resource-duplicate",
                    "Visual resources must be unique within a document.",
                    result,
                );
            }
        }
        let mut slots = HashSet::new();
        for (slot_index, material) in rule
            .get("materials")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            if let Some(slot) = material
                .get("slot")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && n.fract() == 0.0)
            {
                if !slots.insert(slot as i64) {
                    emit(
                        file,
                        index,
                        &format!("/visuals/{rule_index}/materials/{slot_index}/slot"),
                        Severity::Error,
                        "semantic.visual-slot-duplicate",
                        "Material slots must be unique within each visual resource rule.",
                        result,
                    );
                }
            }
        }
    }
}

fn validate_inventory_remaps(
    file: &ParsedFile,
    index: usize,
    patch_index: usize,
    patch: &Value,
    result: &mut LintResult,
) {
    let Some(rules) = patch.get("inventoryRemap").and_then(Value::as_array) else {
        return;
    };
    let mut sources = HashSet::new();
    for (rule_index, rule) in rules.iter().enumerate() {
        if rule
            .get("multiplier")
            .is_some_and(|value| value.as_i64().is_none())
        {
            emit(
                file,
                index,
                &format!("/patches/{patch_index}/inventoryRemap/{rule_index}/multiplier"),
                Severity::Error,
                "semantic.inventory-remap",
                "inventoryRemap multiplier must use an integer literal, not a float or string.",
                result,
            );
        }
        let Some(source) = rule.get("from").and_then(Value::as_str) else {
            continue;
        };
        if !sources.insert(source.trim().to_lowercase()) {
            emit(
                file,
                index,
                &format!("/patches/{patch_index}/inventoryRemap/{rule_index}/from"),
                Severity::Error,
                "semantic.inventory-remap",
                "inventoryRemap source classes must be unique.",
                result,
            );
        }
    }
    for (rule_index, rule) in rules.iter().enumerate() {
        if rule
            .get("to")
            .and_then(Value::as_str)
            .is_some_and(|to| sources.contains(&to.trim().to_lowercase()))
        {
            emit(
                file,
                index,
                &format!("/patches/{patch_index}/inventoryRemap/{rule_index}/to"),
                Severity::Error,
                "semantic.inventory-remap",
                "inventoryRemap destinations cannot also be sources; chained and cyclic remaps are unsupported.",
                result,
            );
        }
    }
}
