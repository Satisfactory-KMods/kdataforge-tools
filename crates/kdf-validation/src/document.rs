use crate::project::{LintOptions, ParsedFile, Profile};
use crate::schema::{canonical_where_operator, pointer_to_yaml_path, TypeConflict};
use crate::source::utf16_position;
use crate::{Diagnostic, LintResult, Range, SchemaRegistry, Severity, SourceMap};
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

pub(crate) const MAX_YAML_BYTES: usize = 8 * 1024 * 1024;
const MAX_DOCUMENTS_PER_FILE: usize = 1024;

pub(crate) fn parse_file(
    path: PathBuf,
    text: String,
    result: &mut LintResult,
) -> Option<ParsedFile> {
    if text.len() > MAX_YAML_BYTES {
        result.diagnostics.push(Diagnostic::new(
            Severity::Error,
            "yaml.too-large",
            "YAML file exceeds 8 MiB safety limit",
            Some(path),
            None,
            "$",
            Range::default(),
            "yaml",
        ));
        return None;
    }
    let mut documents = Vec::new();
    for (index, deserializer) in serde_yaml::Deserializer::from_str(&text).enumerate() {
        if index >= MAX_DOCUMENTS_PER_FILE {
            result.diagnostics.push(Diagnostic::new(
                Severity::Error,
                "yaml.too-many-documents",
                "YAML file exceeds 1024-document safety limit",
                Some(path),
                Some(index),
                "$",
                Range::default(),
                "yaml",
            ));
            return None;
        }
        let yaml = match serde_yaml::Value::deserialize(deserializer) {
            Ok(value) => value,
            Err(error) => {
                let (line, column) = error
                    .location()
                    .map(|location| (location.line(), location.column()))
                    .unwrap_or((1, 1));
                let source = SourceMap::parse(&text).ok();
                let range = source
                    .map(|s| s.syntax_position(line, column))
                    .unwrap_or_else(|| {
                        let start = utf16_position(&text, line, column);
                        Range {
                            start,
                            end: crate::Position {
                                character: start.character + 1,
                                ..start
                            },
                        }
                    });
                let hint = syntax_hint(&text, line);
                result.diagnostics.push(Diagnostic::new(
                    Severity::Error,
                    "yaml.syntax",
                    error.to_string(),
                    Some(path.clone()),
                    Some(index),
                    "$",
                    range,
                    "yaml",
                ));
                if let Some((code, message, column)) = hint {
                    let start = utf16_position(&text, line, column + 1);
                    result.diagnostics.push(Diagnostic::new(
                        Severity::Error,
                        code,
                        message,
                        Some(path),
                        Some(index),
                        "$",
                        Range {
                            start,
                            end: crate::Position {
                                character: start.character + 1,
                                ..start
                            },
                        },
                        code.split('.').next().unwrap_or("yaml"),
                    ));
                }
                return None;
            }
        };
        match serde_json::to_value(yaml) {
            Ok(value) => documents.push(value),
            Err(error) => {
                result.diagnostics.push(Diagnostic::new(
                    Severity::Error,
                    "yaml.non-string-key",
                    error.to_string(),
                    Some(path),
                    Some(index),
                    "$",
                    Range::default(),
                    "yaml",
                ));
                return None;
            }
        }
    }
    if documents.is_empty() || documents.iter().all(Value::is_null) {
        result.diagnostics.push(Diagnostic::new(
            Severity::Error,
            "yaml.empty",
            "empty YAML document",
            Some(path),
            None,
            "$",
            Range::default(),
            "yaml",
        ));
        return None;
    }
    Some(ParsedFile {
        path,
        text,
        documents,
    })
}

/// Reports diagnostics against one parsed file. The source map is built once per file by the
/// caller and shared by every check.
pub(crate) struct Reporter<'a> {
    pub(crate) file: &'a ParsedFile,
    pub(crate) source: Option<&'a SourceMap<'a>>,
}

impl Reporter<'_> {
    pub(crate) fn range(&self, index: usize, pointer: &str) -> Range {
        self.source
            .map(|source| source.location(index, pointer))
            .unwrap_or_default()
    }

    pub(crate) fn diagnostic(
        &self,
        index: usize,
        pointer: &str,
        severity: Severity,
        code: &str,
        range: Range,
        message: impl Into<String>,
    ) -> Diagnostic {
        let source = code.split('.').next().unwrap_or("semantic");
        Diagnostic::new(
            severity,
            code,
            message,
            Some(self.file.path.clone()),
            Some(index),
            pointer_to_yaml_path(pointer),
            range,
            source,
        )
    }

    /// Emits at the range of the node at `pointer`.
    pub(crate) fn emit(
        &self,
        result: &mut LintResult,
        index: usize,
        pointer: &str,
        severity: Severity,
        code: &str,
        message: impl Into<String>,
    ) {
        let range = self.range(index, pointer);
        result
            .diagnostics
            .push(self.diagnostic(index, pointer, severity, code, range, message));
    }
}

/// Outcome of the schema pass that later passes need.
#[derive(Default)]
pub(crate) struct SchemaOutcome {
    /// Schema `type` errors at unquoted typed references (become compatibility checks).
    pub(crate) conflicts: Vec<TypeConflict>,
}

/// Explains a YAML syntax error that is probably an unquoted template or `config:` expression.
/// Returns the diagnostic code, message and the zero-based character column to point at.
fn syntax_hint(text: &str, line_1: usize) -> Option<(&'static str, String, usize)> {
    let line = text.lines().nth(line_1.checked_sub(1)?)?;
    let chars = line.chars().collect::<Vec<_>>();
    let quotes_before = |index: usize| {
        chars[..index]
            .iter()
            .filter(|ch| matches!(ch, '"' | '\''))
            .count()
    };
    // `config: <expression>` whose first character YAML cannot start a plain scalar with.
    if let Some(position) = line.find("config") {
        let start = line[..position].chars().count();
        let before_ok = start == 0 || matches!(chars[start - 1], ' ' | '{' | ',' | '-' | '\t');
        let rest = &chars[start + "config".len()..];
        let skipped = rest.iter().take_while(|ch| **ch == ' ').count();
        if before_ok && rest.get(skipped) == Some(&':') {
            let value = start + "config".len() + skipped + 1;
            let offset = chars[value..].iter().take_while(|ch| **ch == ' ').count();
            if let Some(first) = chars.get(value + offset) {
                if matches!(first, '!' | '&' | '*' | '%' | '@' | '`' | '\'' | '"' | '[') {
                    return Some((
                        "config.expression-quote",
                        format!("an expression starting with {first:?} must be wrapped in quotes: config: \"...\""),
                        value + offset,
                    ));
                }
            }
        }
    }
    // `{prefix:...}` where the braces nest and YAML reads a flow mapping.
    for prefix in crate::template::PREPASS_PREFIXES
        .iter()
        .chain(crate::template::APPLY_PREFIXES.iter())
    {
        let needle = format!("{{{prefix}:");
        let mut from = 0usize;
        while let Some(found) = line[from..].find(&needle) {
            let byte = from + found;
            let index = line[..byte].chars().count();
            from = byte + needle.len();
            let before = chars[..index].iter().rev().find(|ch| **ch != ' ').copied();
            if quotes_before(index) % 2 == 0 && matches!(before, Some(':' | '-' | '[' | ',')) {
                return Some((
                    "template.unquoted-start",
                    "a plain scalar starting with '{' is parsed as a flow mapping; wrap the value in quotes".to_owned(),
                    index,
                ));
            }
        }
    }
    None
}

#[expect(
    clippy::too_many_arguments,
    reason = "validation inputs are distinct and passed by the project passes"
)]
pub(crate) fn validate_one(
    file: &ParsedFile,
    index: usize,
    document: &Value,
    kind: &str,
    registry: &SchemaRegistry,
    options: &LintOptions,
    source: Option<&SourceMap<'_>>,
    result: &mut LintResult,
) -> SchemaOutcome {
    let mut outcome = SchemaOutcome::default();
    if registry.schema(kind).is_none() {
        emit(
            file,
            index,
            "/type",
            if options.allow_unknown_types {
                Severity::Warning
            } else {
                Severity::Error
            },
            "schema.unknown-type",
            format!("no schema registered for root type {kind:?}"),
            result,
        );
        return outcome;
    }
    let owned_source;
    let source = match source {
        Some(source) => Some(source),
        None => {
            owned_source = SourceMap::parse(&file.text).ok();
            owned_source.as_ref()
        }
    };
    if let Some(source) = source {
        // `mam` is a spelling alias. Validate its payload against canonical research grammar.
        let mut canonical = document.clone();
        if canonical.get("type").and_then(Value::as_str) == Some("") {
            canonical["type"] = Value::String(kind.to_owned());
        } else if kind == "research"
            && canonical
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|value| value.eq_ignore_ascii_case("mam"))
        {
            canonical["type"] = Value::String("research".into());
        }
        normalize_runtime_bool_field(&mut canonical, "debug");
        if kind == "pack" {
            normalize_runtime_bool_field(&mut canonical, "enabled");
        }
        normalize_nested_runtime_fields(&mut canonical, kind);
        if kind == "class" {
            if let Some(entries) = canonical.get_mut("classes").and_then(Value::as_array_mut) {
                for entry in entries {
                    if let Some(register_as) = entry.get("registerAs").and_then(Value::as_str) {
                        entry["registerAs"] = Value::String(register_as.to_lowercase());
                    }
                }
            }
        }
        if kind == "sinkpoints" {
            if let Some(track) = canonical.get("track").and_then(Value::as_str) {
                if track.eq_ignore_ascii_case("Exploration") {
                    canonical["track"] = Value::String("Exploration".into());
                } else if track.eq_ignore_ascii_case("Default") {
                    canonical["track"] = Value::String("Default".into());
                }
            }
        }
        if kind == "curve" {
            normalize_curve_enums(&mut canonical);
        }
        if kind == "resourcenode" {
            normalize_resource_node_types(&mut canonical);
        }
        normalize_filter_fields(&mut canonical, kind);
        let typed = if matches!(kind, "config" | "pack") {
            std::collections::HashSet::new()
        } else {
            crate::refs::typed_ref_pointers(&canonical, source, index)
        };
        let (diagnostics, conflicts) =
            registry.validate_typed(kind, &canonical, &file.path, index, source, &typed);
        result.diagnostics.extend(diagnostics);
        outcome.conflicts = conflicts;
    }
    if kind == "pack" && options.profile == Profile::Legacy && document.get("contributer").is_none()
    {
        emit(
            file,
            index,
            "/contributer",
            Severity::Error,
            "policy.contributer",
            "contributer must be a non-empty GitHub login or sequence of logins",
            result,
        );
    }
    outcome
}

fn normalize_runtime_bool_field(document: &mut Value, field: &str) {
    if let Some(normalized) = document.get(field).and_then(runtime_bool) {
        document[field] = Value::Bool(normalized);
    }
}

pub(crate) fn runtime_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(text) => match text.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Some(true),
            "false" | "no" | "off" | "0" => Some(false),
            _ => None,
        },
        Value::Number(number) => match number.as_i64() {
            Some(1) => Some(true),
            Some(0) => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn normalize_nested_runtime_fields(document: &mut Value, kind: &str) {
    if let Some(conditions) = document.get_mut("conditions") {
        if let Some(entries) = conditions.as_array_mut() {
            for entry in entries {
                normalize_condition_object(entry);
            }
        } else {
            normalize_condition_object(conditions);
        }
    }
    if kind == "cdo" {
        if let Some(patches) = document.get_mut("patches").and_then(Value::as_array_mut) {
            for patch in patches {
                for field in [
                    "applyToSubclasses",
                    "applyToSpawnedActors",
                    "deferOneGameTick",
                ] {
                    normalize_runtime_bool_field(patch, field);
                }
            }
        }
    }
    if kind == "curve" {
        if let Some(curves) = document.get_mut("curves").and_then(Value::as_array_mut) {
            for curve in curves {
                normalize_runtime_bool_field(curve, "isEventCurve");
            }
        }
    }
    if kind == "resourcenode" {
        for collection in ["add", "replace"] {
            if let Some(entries) = document.get_mut(collection).and_then(Value::as_array_mut) {
                for entry in entries {
                    for field in ["reuseExisting", "allowToMove", "preventOverlap"] {
                        normalize_runtime_bool_field(entry, field);
                    }
                }
            }
        }
        if let Some(entries) = document.get_mut("remove").and_then(Value::as_array_mut) {
            for entry in entries {
                normalize_runtime_bool_field(entry, "allowOccupied");
                normalize_runtime_bool_field(entry, "removeFromScanner");
            }
        }
    }
    let entry_key = match kind {
        "eventrule" => Some("rules"),
        "item" => Some("items"),
        "building" => Some("buildings"),
        "resource" => Some("resources"),
        "unlock" => Some("unlocks"),
        "recipe" => Some("recipes"),
        "schematic" => Some("schematics"),
        "research" => Some("research"),
        _ => None,
    };
    if let Some(entries) = entry_key
        .and_then(|key| document.get_mut(key))
        .and_then(Value::as_array_mut)
    {
        for entry in entries {
            normalize_runtime_bool_field(entry, "register");
            if kind == "research" {
                normalize_runtime_bool_field(entry, "autoPath");
            }
        }
    }
}

pub(crate) fn normalize_condition_object(object: &mut Value) {
    const CONDITION_KEYS: [&str; 6] = [
        "gameVersion",
        "hasMod",
        "modVersion",
        "hasClass",
        "config",
        "ifNotMatch",
    ];
    let Some(map) = object.as_object_mut() else {
        return;
    };
    let keys = map.keys().cloned().collect::<Vec<_>>();
    for key in keys {
        let Some(canonical) = CONDITION_KEYS
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(&key))
        else {
            continue;
        };
        if key != *canonical && !map.contains_key(*canonical) {
            if let Some(value) = map.remove(&key) {
                map.insert((*canonical).to_owned(), value);
            }
        }
    }
    normalize_runtime_bool_field(object, "ifNotMatch");
}

fn normalize_curve_enums(document: &mut Value) {
    const EXTRAPOLATION: [&str; 6] = [
        "none",
        "constant",
        "linear",
        "cycle",
        "cycleWithOffset",
        "oscillate",
    ];
    normalize_enum_field(document, "curveType", &["float", "vector"]);
    let Some(curves) = document.get_mut("curves").and_then(Value::as_array_mut) else {
        return;
    };
    for curve in curves {
        normalize_enum_field(curve, "preInfinityExtrap", &EXTRAPOLATION);
        normalize_enum_field(curve, "postInfinityExtrap", &EXTRAPOLATION);
        let Some(keys) = curve.get_mut("keys").and_then(Value::as_array_mut) else {
            continue;
        };
        for key in keys {
            normalize_enum_field(key, "interpMode", &["constant", "linear", "cubic"]);
            normalize_enum_field(key, "tangentMode", &["auto", "smartAuto", "user", "break"]);
            normalize_enum_field(
                key,
                "tangentWeightMode",
                &["none", "arrive", "leave", "both"],
            );
        }
    }
}

fn normalize_resource_node_types(document: &mut Value) {
    const NODE_TYPES: [&str; 5] = [
        "Node",
        "FrackingSatellite",
        "FrackingCore",
        "Geyser",
        "Deposit",
    ];
    let Some(entries) = document.get_mut("remove").and_then(Value::as_array_mut) else {
        return;
    };
    for entry in entries {
        let Some(node_types) = entry.get_mut("nodeTypes").and_then(Value::as_array_mut) else {
            continue;
        };
        for node_type in node_types {
            let Some(raw) = node_type.as_str() else {
                continue;
            };
            let (prefixed, name) = match raw.split_once("::") {
                Some((prefix, name)) if prefix.eq_ignore_ascii_case("EResourceNodeType") => {
                    (true, name)
                }
                Some(_) => continue,
                None => (false, raw),
            };
            if let Some(canonical) = NODE_TYPES
                .iter()
                .find(|candidate| candidate.eq_ignore_ascii_case(name))
            {
                *node_type = Value::String(if prefixed {
                    format!("EResourceNodeType::{canonical}")
                } else {
                    (*canonical).to_owned()
                });
            }
        }
    }
}

fn normalize_enum_field(object: &mut Value, field: &str, values: &[&str]) {
    let Some(raw) = object.get(field).and_then(Value::as_str) else {
        return;
    };
    if let Some(value) = values.iter().find(|value| value.eq_ignore_ascii_case(raw)) {
        object[field] = Value::String((*value).to_owned());
    }
}

fn normalize_filter_fields(document: &mut Value, kind: &str) {
    let entry_key = match kind {
        "cdo" => "patches",
        "sublevel" => "block",
        "sinkpoints" => "entries",
        "recipe" | "schematic" | "research" | "resourcenode" => "remove",
        _ => return,
    };
    let Some(entries) = document.get_mut(entry_key).and_then(Value::as_array_mut) else {
        return;
    };
    for entry in entries {
        normalize_runtime_bool_field(entry, "matchAssets");
        let Some(where_value) = entry.get_mut("where") else {
            continue;
        };
        if let Some(clauses) = where_value.as_array_mut() {
            for clause in clauses {
                normalize_where_clause(clause);
                normalize_runtime_bool_field(clause, "isEmpty");
            }
        } else {
            normalize_where_clause(where_value);
            normalize_runtime_bool_field(where_value, "isEmpty");
        }
    }
}

fn normalize_where_clause(clause: &mut Value) {
    let Some(map) = clause.as_object_mut() else {
        return;
    };
    let keys = map.keys().cloned().collect::<Vec<_>>();
    for key in keys {
        let Some(canonical) = canonical_where_operator(&key) else {
            continue;
        };
        if key != canonical && !map.contains_key(canonical) {
            if let Some(value) = map.remove(&key) {
                map.insert(canonical.to_owned(), value);
            }
        }
    }
}

pub(crate) fn emit(
    file: &ParsedFile,
    index: usize,
    pointer: &str,
    severity: Severity,
    code: &str,
    message: impl Into<String>,
    result: &mut LintResult,
) {
    let range = SourceMap::parse(&file.text)
        .map(|map| map.location(index, pointer))
        .unwrap_or_default();
    let source = code.split('.').next().unwrap_or("semantic");
    result.diagnostics.push(Diagnostic::new(
        severity,
        code,
        message,
        Some(file.path.clone()),
        Some(index),
        pointer_to_yaml_path(pointer),
        range,
        source,
    ));
}
