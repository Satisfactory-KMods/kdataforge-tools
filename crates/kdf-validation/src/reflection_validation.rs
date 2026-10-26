//! Selective checks against actual exported property metadata, never guessed types.
use crate::{
    document::{emit, runtime_bool},
    project::{LintOptions, ParsedFile},
    LintResult, ReflectionIndex, ReflectionStatus, Severity, SourceMap, UnknownClassSeverity,
};
use serde_json::Value;
use std::collections::HashSet;

pub(crate) fn validate(
    file: &ParsedFile,
    document_index: usize,
    document: &Value,
    source: Option<&SourceMap<'_>>,
    options: &LintOptions,
    datapack: Option<&ReflectionIndex>,
    result: &mut LintResult,
) {
    let Some(index) = datapack.filter(|index| {
        matches!(
            index.status,
            ReflectionStatus::Ready | ReflectionStatus::Limited
        )
    }) else {
        return;
    };
    // Runtime substitutes only unquoted whole-scalar refs. Config/reference checks
    // own those typed values and templates; literal quoted references retain ordinary
    // reflected type checks. Reuse the file's source map instead of parsing per field.
    let typed = source
        .map(|source| crate::refs::typed_ref_pointers(document, source, document_index))
        .unwrap_or_default();
    let value_context = ValueContext {
        file,
        document: document_index,
        typed: &typed,
    };
    let policy = &options.datapack_validation;
    for (root, entries) in document.as_object().into_iter().flatten() {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for (position, entry) in entries.iter().enumerate() {
            let pointer = format!("/{root}/{position}");
            let mut target = None;
            for key in ["target", "allAssetsOfClass", "ofClass", "class", "parent"] {
                let Some(reference) = entry.get(key).and_then(Value::as_str) else {
                    continue;
                };
                if !reference.starts_with('/') || reference.starts_with("/KDataForge/Gen/") {
                    continue;
                }
                let reference_pointer = format!("{pointer}/{key}");
                if !index.contains_class(reference)
                    && policy.check_classes
                    && policy.unknown_class_severity != UnknownClassSeverity::Off
                {
                    // Existing semantic class-family checks can already own this range.
                    if !result.diagnostics.iter().any(|item| {
                        item.code == "datapack.unknown-class"
                            && item.document == Some(document_index)
                            && item.file.as_ref() == Some(&file.path)
                            && item.yaml_path
                                == crate::schema::pointer_to_yaml_path(&reference_pointer)
                    }) {
                        emit(file, document_index, &reference_pointer, Severity::Warning, "datapack.unknown-class", format!("class {reference:?} is absent from bound reflection snapshot; runtime may still load it"), result);
                    }
                }
                if target.is_none() && index.contains_class(reference) {
                    target = Some(reference);
                }
                attach_evidence(
                    file,
                    document_index,
                    &reference_pointer,
                    reference,
                    index.replacement_evidence("class", None, reference),
                    source,
                    result,
                );
            }
            if !policy.check_properties {
                continue;
            }
            let Some(target) = target else {
                continue;
            };
            let Some(properties) = entry.get("properties").and_then(Value::as_array) else {
                continue;
            };
            let (fields, complete) = inherited_fields(index, target);
            for (field_index, change) in properties.iter().enumerate() {
                let Some(path) = change.get("path").and_then(Value::as_str) else {
                    continue;
                };
                let name = path.split(['.', '[']).next().unwrap_or(path);
                let field_pointer = format!("{pointer}/properties/{field_index}");
                let evidence = index.replacement_evidence("property", Some(target), name);
                let Some(field) = fields.iter().find(|field| {
                    field
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|value| value.eq_ignore_ascii_case(name))
                }) else {
                    if complete {
                        emit(
                            file,
                            document_index,
                            &format!("{field_pointer}/path"),
                            Severity::Warning,
                            "datapack.unknown-property",
                            format!("property {name:?} is absent from exported fields of {target}"),
                            result,
                        );
                    }
                    attach_evidence(
                        file,
                        document_index,
                        &format!("{field_pointer}/path"),
                        name,
                        evidence,
                        source,
                        result,
                    );
                    continue;
                };
                attach_evidence(
                    file,
                    document_index,
                    &format!("{field_pointer}/path"),
                    name,
                    evidence,
                    source,
                    result,
                );
                // Nested structs may not be exported. Only validate an exact field
                // or container inner type whose metadata is actually available.
                let Some(value) = change.get("value") else {
                    continue;
                };
                if path != name {
                    continue;
                }
                let operation = change
                    .get("op")
                    .and_then(Value::as_str)
                    .unwrap_or("set")
                    .to_ascii_lowercase();
                if operation == "clear" {
                    continue;
                }
                if operation == "replace_refs" {
                    continue;
                }
                validate_value(
                    &value_context,
                    field,
                    value,
                    &operation,
                    &format!("{field_pointer}/value"),
                    result,
                );
            }
        }
    }
}

fn attach_evidence(
    file: &ParsedFile,
    document: usize,
    pointer: &str,
    name: &str,
    evidence: crate::evidence::Evidence,
    source: Option<&SourceMap<'_>>,
    result: &mut LintResult,
) {
    let yaml_path = crate::schema::pointer_to_yaml_path(pointer);
    let existing = result.diagnostics.iter().position(|diagnostic| {
        diagnostic.file.as_ref() == Some(&file.path)
            && diagnostic.document == Some(document)
            && diagnostic.yaml_path == yaml_path
            && matches!(
                diagnostic.code.as_str(),
                "datapack.unknown-class" | "datapack.unknown-property"
            )
    });
    let selected = if let Some(existing) = existing {
        Some(existing)
    } else if evidence.replacement.is_some() {
        emit(
            file,
            document,
            pointer,
            Severity::Warning,
            "datapack.outdated-name",
            format!("{name:?} has an explicitly verified reflection replacement"),
            result,
        );
        Some(result.diagnostics.len() - 1)
    } else {
        None
    };
    if let Some(selected) = selected {
        let diagnostic = &mut result.diagnostics[selected];
        // Nested paths need segment-aware metadata; do not turn a prefix guess into an edit.
        let exact = source.and_then(|source| source.scalar_value_range(document, pointer, name));
        let mut evidence = evidence;
        if let Some(range) = exact {
            diagnostic.range = range;
            diagnostic.line = Some(range.start.line as usize + 1);
            diagnostic.column = Some(range.start.character as usize + 1);
        } else if evidence.replacement.is_some() {
            evidence = crate::evidence::Evidence::unknown(
                "Replacement has no exact scalar source range",
                evidence.reflection_digest,
            );
        }
        diagnostic.evidence = Some(evidence);
    }
}

fn inherited_fields<'a>(index: &'a ReflectionIndex, target: &'a str) -> (Vec<&'a Value>, bool) {
    let mut fields = Vec::new();
    let mut names = HashSet::new();
    let mut seen = HashSet::new();
    let mut current = Some(target);
    for _ in 0..128 {
        let Some(path) = current else {
            return (fields, true);
        };
        if !seen.insert(path) {
            return (fields, false);
        }
        let Some(class) = index.class_info(path) else {
            return (fields, false);
        };
        let Some(properties) = class.get("properties").and_then(Value::as_array) else {
            return (fields, false);
        };
        for field in properties {
            if let Some(name) = field.get("name").and_then(Value::as_str) {
                if names.insert(name.to_ascii_lowercase()) {
                    fields.push(field);
                }
            }
        }
        current = class.get("super").and_then(Value::as_str);
    }
    (fields, false)
}

struct ValueContext<'a> {
    file: &'a ParsedFile,
    document: usize,
    typed: &'a HashSet<String>,
}

fn validate_value(
    context: &ValueContext<'_>,
    field: &Value,
    value: &Value,
    operation: &str,
    pointer: &str,
    result: &mut LintResult,
) {
    let file = context.file;
    let document = context.document;
    if context.typed.contains(pointer)
        || value.as_str().is_some_and(|text| {
            text.contains('{')
                && !crate::template::scan(text, crate::template::ScanPhase::All)
                    .tokens()
                    .is_empty()
        })
    {
        return;
    }
    if operation == "set"
        && field.get("bitflags").and_then(Value::as_bool) != Some(true)
        && field.get("enum").and_then(Value::as_str).is_some()
    {
        if let Some(options) = field
            .get("options")
            .and_then(Value::as_array)
            .filter(|options| !options.is_empty())
        {
            let supplied = value
                .as_str()
                .map(str::to_owned)
                .or_else(|| value.as_i64().map(|number| number.to_string()))
                .or_else(|| value.as_u64().map(|number| number.to_string()));
            let valid = supplied.as_deref().is_some_and(|supplied| {
                options.iter().any(|option| {
                    ["name", "fullName", "valueExact"].iter().any(|key| {
                        option
                            .get(key)
                            .and_then(Value::as_str)
                            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(supplied))
                    }) || option.get("value").is_some_and(|candidate| {
                        supplied
                            .parse::<i64>()
                            .ok()
                            .is_some_and(|number| candidate.as_i64() == Some(number))
                    })
                })
            });
            if !valid {
                emit(
                    file,
                    document,
                    pointer,
                    Severity::Warning,
                    "datapack.enum-value",
                    "value is absent from exported enum options",
                    result,
                );
            }
            return;
        }
    }
    let Some(cpp) = field.get("cppType").and_then(Value::as_str) else {
        return;
    };
    if matches!(operation, "append_unique" | "update_where") && !cpp.starts_with("TArray<") {
        emit(
            file,
            document,
            pointer,
            Severity::Warning,
            "datapack.type-mismatch",
            format!("{operation} requires an array property"),
            result,
        );
        return;
    }
    if operation == "update_where"
        && field.pointer("/inner/kind").and_then(Value::as_str) != Some("StructProperty")
    {
        emit(
            file,
            document,
            pointer,
            Severity::Warning,
            "datapack.type-mismatch",
            "update_where requires an array of structs",
            result,
        );
        return;
    }
    if field.get("bitflags").and_then(Value::as_bool) == Some(true) {
        if is_enum_backed_bitflags(field) {
            let valid = match operation {
                "set" => enum_backed_bitflag_value_valid(field, value),
                "append" | "prepend" | "remove" | "insert" | "clear" => Some(false),
                _ => None,
            };
            if let Some(false) = valid {
                emit(
                    file,
                    document,
                    pointer,
                    Severity::Warning,
                    "datapack.enum-value",
                    if operation == "set" {
                        "value is not a valid enum-backed bitflags value"
                    } else {
                        "enum-backed bitflags support set, copy, and move operations only"
                    },
                    result,
                );
            }
            return;
        }
        if matches!(operation, "set" | "append" | "prepend" | "remove") {
            let named_valid = named_bitmask_value_valid(field, value).or_else(|| {
                // Mutation operands resolve flag names; only set accepts raw masks.
                (operation != "set"
                    && (value.is_number() || value.as_str().is_some_and(looks_like_raw_bitmask)))
                .then_some(false)
            });
            if let Some(valid) = named_valid {
                if !valid {
                    emit(
                        file,
                        document,
                        pointer,
                        Severity::Warning,
                        "datapack.enum-value",
                        "flag names are absent from exported enum options or invalid",
                        result,
                    );
                }
                return;
            }
        }
        if let Some(valid) = (operation == "set")
            .then(|| raw_bitmask_value_valid(field, value))
            .flatten()
        {
            if !valid {
                emit(
                    file,
                    document,
                    pointer,
                    Severity::Warning,
                    "datapack.type-mismatch",
                    format!(
                        "value does not match exported {cpp} bitmask storage or declared flags"
                    ),
                    result,
                );
            }
            return;
        }
    }
    let numeric = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()));
    let valid = match cpp {
        "bool" => Some(runtime_bool(value).is_some()),
        "float" => {
            Some(numeric.is_some_and(|number| number.is_finite() && (number as f32).is_finite()))
        }
        "double" => Some(numeric.is_some_and(f64::is_finite)),
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64" => {
            let bits = cpp
                .trim_start_matches('u')
                .trim_start_matches("int")
                .parse::<u32>()
                .unwrap_or(64);
            if matches!(operation, "add" | "multiply" | "divide" | "min" | "max") {
                // Numeric operations round their result for integer fields; a
                // fractional operand is valid and current runtime value is unknown.
                Some(numeric.is_some_and(f64::is_finite))
            } else {
                let integer = if cpp.starts_with('u') {
                    value
                        .as_u64()
                        .or_else(|| {
                            value
                                .as_str()
                                .and_then(|text| text.trim().parse::<u64>().ok())
                        })
                        .map(i128::from)
                } else {
                    value
                        .as_i64()
                        .or_else(|| {
                            value
                                .as_str()
                                .and_then(|text| text.trim().parse::<i64>().ok())
                        })
                        .map(i128::from)
                };
                Some(integer.is_some_and(|number| {
                    if cpp.starts_with('u') {
                        number >= 0 && number < 2_i128.pow(bits)
                    } else {
                        number >= -2_i128.pow(bits - 1) && number < 2_i128.pow(bits - 1)
                    }
                }))
            }
        }
        "FString" | "FName" => Some(!value.is_array() && !value.is_object() && !value.is_null()),
        "FText" => Some(match value {
            Value::Object(map) => {
                let key_valid = map
                    .get("key")
                    .and_then(Value::as_str)
                    .is_some_and(|key| !key.is_empty());
                let table_valid = map.get("stringTable").is_none_or(|table| {
                    table.as_str().is_some_and(|id| !id.is_empty())
                        && !map.contains_key("ns")
                        && !map.contains_key("source")
                });
                key_valid && table_valid
            }
            _ => !value.is_array(),
        }),
        _ if cpp.starts_with("TArray<") => {
            if matches!(
                operation,
                "append" | "append_unique" | "remove" | "insert" | "update_where"
            ) {
                if let Some(inner) = field.get("inner") {
                    if let Some(values) = value.as_array() {
                        for (position, value) in values.iter().enumerate() {
                            validate_value(
                                context,
                                inner,
                                value,
                                "set",
                                &format!("{pointer}/{position}"),
                                result,
                            );
                        }
                    } else {
                        validate_value(context, inner, value, "set", pointer, result);
                    }
                }
                None
            } else {
                Some(value.is_array() || value.is_string())
            }
        }
        _ if cpp.starts_with("TMap<") => Some(value.is_object() || value.is_string()),
        _ => None,
    };
    if valid == Some(false) {
        emit(
            file,
            document,
            pointer,
            Severity::Warning,
            "datapack.type-mismatch",
            format!("value does not match exported {cpp} field type"),
            result,
        );
    }
}

fn is_enum_backed_bitflags(field: &Value) -> bool {
    match field.get("kind").and_then(Value::as_str) {
        Some("EnumProperty") => true,
        Some("ByteProperty") => field.get("enum").and_then(Value::as_str).is_some(),
        _ => false,
    }
}

/// Completion and hover use the same metadata checks as literal validation.
/// Incomplete metadata keeps a choice unless it proves that choice invalid.
pub(crate) fn enum_option_is_valid(field: &Value, option: &Value) -> bool {
    if field.get("bitflags").and_then(Value::as_bool) != Some(true) {
        return true;
    }
    let Some(name) = option.get("name").and_then(Value::as_str) else {
        return false;
    };
    let value = Value::String(name.to_owned());
    let valid = if is_enum_backed_bitflags(field) {
        enum_backed_bitflag_value_valid(field, &value)
    } else {
        named_bitmask_value_valid(field, &value)
    };
    valid != Some(false)
}

fn enum_backed_bitflag_value_valid(field: &Value, value: &Value) -> Option<bool> {
    let enum_value = if let Some(text) = value.as_str() {
        if let Some(option) = field
            .get("options")
            .and_then(Value::as_array)
            .and_then(|options| {
                options.iter().find(|option| {
                    ["name", "fullName"].iter().any(|key| {
                        option
                            .get(key)
                            .and_then(Value::as_str)
                            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(text.trim()))
                    })
                })
            })
        {
            let Some(value) = exact_enum_option_value(option) else {
                return Some(true);
            };
            value
        } else if let Ok(value) = text.trim().parse::<i64>() {
            value
        } else {
            return Some(false);
        }
    } else if let Some(value) = value.as_i64() {
        value
    } else {
        return Some(false);
    };
    if !matches!(
        field.get("bitmaskMode").and_then(Value::as_str),
        Some("maskValues" | "bitIndices")
    ) {
        // EEnumFlags::Flags alone does not enable runtime Bitflags metadata.
        // Without a mode, runtime validates exact enum membership, including
        // declared negative values, rather than interpreting a bit pattern.
        let options = field.get("options")?.as_array()?.as_slice();
        if options.is_empty() {
            return None;
        }
        let mut unknown = false;
        for option in options {
            match exact_enum_option_value(option) {
                Some(candidate) if candidate == enum_value => return Some(true),
                Some(_) => {}
                None => unknown = true,
            }
        }
        return (!unknown).then_some(false);
    }
    if enum_value < 0 {
        return Some(false);
    }
    let value = enum_value as u64;
    if let Some(bytes @ (1 | 2 | 4 | 8)) = field.get("elementSize").and_then(Value::as_u64) {
        let maximum = u64::MAX >> (64 - bytes * 8);
        if value > maximum {
            return Some(false);
        }
    }
    Some(declared_bitmask_mask(field).is_none_or(|allowed| value & !allowed == 0))
}

fn named_bitmask_value_valid(field: &Value, value: &Value) -> Option<bool> {
    let names = match value {
        Value::String(text) if !looks_like_raw_bitmask(text) => text
            .split('|')
            .filter(|name| !name.is_empty())
            .map(str::trim)
            .collect::<Vec<_>>(),
        Value::Array(values) => {
            let Some(names) = values.iter().map(Value::as_str).collect::<Option<Vec<_>>>() else {
                return Some(false);
            };
            names
        }
        _ => return None,
    };
    if names.is_empty() || names.iter().any(|name| name.is_empty()) {
        return Some(false);
    }
    let Some(options) = field
        .get("options")
        .and_then(Value::as_array)
        .filter(|options| !options.is_empty())
    else {
        // Runtime accepts pipe-separated and sequence names. Missing enum metadata
        // cannot establish whether any supplied name exists.
        return Some(true);
    };
    let Some(selected) = names
        .iter()
        .map(|name| {
            options.iter().find(|option| {
                ["name", "fullName"].iter().any(|key| {
                    option
                        .get(key)
                        .and_then(Value::as_str)
                        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
                })
            })
        })
        .collect::<Option<Vec<_>>>()
    else {
        return Some(false);
    };
    let Some(mode @ ("maskValues" | "bitIndices")) =
        field.get("bitmaskMode").and_then(Value::as_str)
    else {
        return Some(true);
    };
    let mut combined = 0_u64;
    for option in selected {
        let Some(value) = exact_enum_option_value(option) else {
            // Missing exact numeric metadata cannot disprove supported names.
            continue;
        };
        if value < 0 || (mode == "bitIndices" && value >= 64) {
            return Some(false);
        }
        combined |= if mode == "maskValues" {
            value as u64
        } else {
            1_u64 << value
        };
    }
    if let Some(bytes @ (1 | 2 | 4 | 8)) = field.get("elementSize").and_then(Value::as_u64) {
        if combined > u64::MAX >> (64 - bytes * 8) {
            return Some(false);
        }
    }
    Some(declared_bitmask_mask(field).is_none_or(|allowed| combined & !allowed == 0))
}

fn looks_like_raw_bitmask(text: &str) -> bool {
    let text = text.trim();
    text.starts_with(['+', '-', '.']) || text.as_bytes().first().is_some_and(u8::is_ascii_digit)
}

fn raw_bitmask_value_valid(field: &Value, value: &Value) -> Option<bool> {
    let bytes = field.get("elementSize")?.as_u64()?;
    if !matches!(bytes, 1 | 2 | 4 | 8) {
        return None;
    }
    let text = value.as_str().map(str::trim);
    // Named forms are checked separately; this path only handles raw numeric set.
    if !value.is_number() && !text.is_some_and(looks_like_raw_bitmask) {
        return None;
    }
    // KDFValueCodec::GetBitmaskPropertyMask uses all storage bits, including
    // signed properties' sign bit. Never round a raw bit pattern through f64.
    let integer = value
        .as_u64()
        .or_else(|| text.and_then(|text| text.parse::<u64>().ok()));
    let maximum = u64::MAX >> (64 - bytes * 8);
    let declared_mask = declared_bitmask_mask(field);
    Some(integer.is_some_and(|integer| {
        integer <= maximum && declared_mask.is_none_or(|allowed| integer & !allowed == 0)
    }))
}

fn declared_bitmask_mask(field: &Value) -> Option<u64> {
    let mode = field.get("bitmaskMode")?.as_str()?;
    if !matches!(mode, "maskValues" | "bitIndices") {
        return None;
    }
    let options = field.get("options")?.as_array()?;
    let mut allowed = 0_u64;
    // Mirror KDFValueCodec::IsValidBitmaskImportValue. A known mode means the
    // exporter resolved an enum carrying Bitflags metadata.
    for option in options {
        let name = option.get("name")?.as_str()?;
        if name.to_ascii_uppercase().ends_with("_MAX") {
            continue;
        }
        let value = exact_enum_option_value(option)?;
        if value < 0 {
            continue;
        }
        if mode == "maskValues" {
            allowed |= value as u64;
        } else if value < 64 {
            allowed |= 1_u64 << value;
        }
    }
    Some(allowed)
}

fn exact_enum_option_value(option: &Value) -> Option<i64> {
    match option.get("valueExact") {
        Some(value) => value.as_str()?.trim().parse::<i64>().ok(),
        None => option.get("value")?.as_i64(),
    }
}

pub(crate) fn apply_policy(options: &LintOptions, result: &mut LintResult) {
    let policy = &options.datapack_validation;
    result.diagnostics.retain(|item| {
        item.code != "datapack.unknown-class"
            || (policy.check_classes && policy.unknown_class_severity != UnknownClassSeverity::Off)
    });
    for item in &mut result.diagnostics {
        if item.code == "datapack.unknown-class" {
            item.severity = match policy.unknown_class_severity {
                UnknownClassSeverity::Error => Severity::Error,
                UnknownClassSeverity::Information => Severity::Information,
                _ => Severity::Warning,
            };
        }
    }
}
