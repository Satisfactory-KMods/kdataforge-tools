use crate::document::emit;
use crate::project::ParsedFile;
use crate::schema::{canonical_where_operator, path_within};
use crate::slots::default_base_class;
use crate::{operation_spec, LintResult, ReflectionIndex, ReflectionStatus, Severity};
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

struct CatalogClass {
    family: String,
    blueprint: Option<String>,
    abstract_class: bool,
}

#[derive(
    Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "camelCase")]
pub enum GeneratedNamespace {
    Class,
    Asset,
    EventRule,
    ResourcePlacement,
}

impl GeneratedNamespace {
    fn for_kind(kind: &str) -> Self {
        match kind {
            "eventrule" => Self::EventRule,
            "resourcenode" => Self::ResourcePlacement,
            "asset" | "curve" | "dataasset" | "sound" => Self::Asset,
            _ => Self::Class,
        }
    }
}

pub(crate) fn runtime_identity(token: &str) -> String {
    token
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>()
        .to_lowercase()
}

/// Identity shared by validation and additive import collision checks. Config field IDs
/// are definitions rather than generated objects and have their own validation rules.
pub fn generated_identity(kind: &str, id: &str) -> Option<(GeneratedNamespace, String)> {
    if kind == "config" {
        return None;
    }
    let identity = match kind {
        "eventrule" => id.to_lowercase(),
        "resourcenode" => id.to_owned(),
        _ => runtime_identity(id),
    };
    Some((GeneratedNamespace::for_kind(kind), identity))
}

static CLASS_CATALOG: OnceLock<HashMap<String, CatalogClass>> = OnceLock::new();

fn class_catalog() -> &'static HashMap<String, CatalogClass> {
    CLASS_CATALOG.get_or_init(|| {
        let document: Value = serde_json::from_str(include_str!("../data/instanced_classes.json"))
            .expect("bundled class catalog");
        let mut classes = HashMap::new();
        if let Some(families) = document.get("families").and_then(Value::as_object) {
            for (family, data) in families {
                if let Some(entries) = data.get("entries").and_then(Value::as_array) {
                    for entry in entries {
                        if let Some(path) = entry.get("path").and_then(Value::as_str) {
                            classes.insert(
                                normalize_class(path),
                                CatalogClass {
                                    family: family.clone(),
                                    blueprint: entry
                                        .get("blueprint")
                                        .and_then(Value::as_str)
                                        .map(str::to_owned),
                                    abstract_class: entry
                                        .get("abstract")
                                        .and_then(Value::as_bool)
                                        .unwrap_or(false),
                                },
                            );
                        }
                    }
                }
            }
        }
        classes
    })
}

pub(crate) struct SemanticContext<'a> {
    pub file: &'a ParsedFile,
    pub index: usize,
    pub document: &'a Value,
    pub kind: &'a str,
    pub pack_root: &'a Path,
    pub pack_ref: Option<&'a str>,
    pub include_documents: &'a HashMap<PathBuf, HashSet<usize>>,
    pub datapack: Option<&'a ReflectionIndex>,
}

pub(crate) fn semantic_document(
    context: SemanticContext<'_>,
    generated: &mut HashMap<(PathBuf, GeneratedNamespace, String), PathBuf>,
    runtime_generated: &mut HashMap<(String, GeneratedNamespace, String), (String, PathBuf)>,
    result: &mut LintResult,
) {
    let SemanticContext {
        file,
        index,
        document,
        kind,
        pack_root,
        pack_ref,
        include_documents,
        datapack,
    } = context;
    if let Some(includes) = document.get("include") {
        let items = includes
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![includes.clone()]);
        for (item_index, item) in items.iter().enumerate() {
            let pointer = if includes.is_array() {
                format!("/include/{item_index}")
            } else {
                "/include".to_owned()
            };
            if let Some(relative) = item.as_str() {
                let (relative_file, document_index) = split_include_document_index(relative);
                let target = pack_root.join(relative_file);
                if !path_within(&target, pack_root) {
                    emit(
                        file,
                        index,
                        &pointer,
                        Severity::Error,
                        "semantic.include-escape",
                        format!("include escapes pack root: {relative}"),
                        result,
                    );
                } else if !target.is_file() {
                    emit(
                        file,
                        index,
                        &pointer,
                        Severity::Error,
                        "semantic.include-missing",
                        format!("include does not exist: {relative}"),
                        result,
                    );
                } else if dunce::canonicalize(&target)
                    .ok()
                    .is_some_and(|resolved| !resolved.starts_with(pack_root))
                {
                    emit(
                        file,
                        index,
                        &pointer,
                        Severity::Error,
                        "semantic.include-symlink",
                        format!("include follows symlink outside pack root: {relative}"),
                        result,
                    );
                } else if document_index.is_some_and(|document_index| {
                    !include_documents
                        .get(&target)
                        .is_some_and(|indices| indices.contains(&document_index))
                }) {
                    emit(
                        file,
                        index,
                        &pointer,
                        Severity::Error,
                        "semantic.include-missing",
                        format!("include document does not exist: {relative}"),
                        result,
                    );
                }
            }
        }
    }
    // Config definitions (`fields[].id`) are not generated content and never take part in the
    // runtime identity namespaces.
    let registrable = if kind == "config" {
        None
    } else {
        document.as_object()
    };
    for (key, entries) in registrable.into_iter().flat_map(|map| map.iter()) {
        if let Some(entries) = entries.as_array() {
            for (entry_index, entry) in entries.iter().enumerate() {
                if let Some(id) = entry.get("id").and_then(Value::as_str) {
                    let (namespace, runtime_id) = generated_identity(kind, id)
                        .expect("config definitions are excluded from generated identities");
                    let lookup = (pack_root.to_path_buf(), namespace, runtime_id.clone());
                    if let Some(previous) = generated.get(&lookup) {
                        emit(
                            file,
                            index,
                            &format!("/{key}/{entry_index}/id"),
                            Severity::Error,
                            "semantic.duplicate-id",
                            format!(
                                "generated runtime identity for id {id:?} is already used by {}",
                                previous.display()
                            ),
                            result,
                        );
                    } else {
                        generated.insert(lookup, file.path.clone());
                    }
                    if let Some(pack_ref) = pack_ref {
                        let runtime_pack = match kind {
                            "eventrule" => pack_ref.to_lowercase(),
                            "resourcenode" => pack_ref.to_owned(),
                            _ => runtime_identity(pack_ref),
                        };
                        let runtime_key = (runtime_pack, namespace, runtime_id);
                        match runtime_generated.get(&runtime_key) {
                            Some((previous_ref, previous_file)) if previous_ref != pack_ref => {
                                emit(
                                    file,
                                    index,
                                    &format!("/{key}/{entry_index}/id"),
                                    Severity::Error,
                                    "semantic.runtime-identity-collision",
                                    format!(
                                        "generated id {id:?} in pack {pack_ref:?} has the same runtime identity as pack {previous_ref:?} in {}",
                                        previous_file.display()
                                    ),
                                    result,
                                );
                            }
                            Some(_) => {}
                            None => {
                                runtime_generated
                                    .insert(runtime_key, (pack_ref.to_owned(), file.path.clone()));
                            }
                        }
                    }
                }
                if (kind == "asset" && key == "assets") || (kind == "sound" && key == "sounds") {
                    let sound = kind == "sound";
                    let code = |suffix: &str| format!("semantic.{kind}-{suffix}");
                    if let Some(asset_file) = entry.get("file").and_then(Value::as_str) {
                        let normalized = asset_file.replace('\\', "/");
                        let candidate = pack_root.join(&normalized);
                        let pointer = format!("/{key}/{entry_index}/file");
                        if (sound && (normalized.contains(':') || normalized.starts_with('/')))
                            || Path::new(&normalized).is_absolute()
                            || !path_within(&candidate, pack_root)
                        {
                            emit(
                                file,
                                index,
                                &pointer,
                                Severity::Error,
                                &code("escape"),
                                format!("{kind} file must be relative and stay inside pack"),
                                result,
                            );
                        } else if !candidate.is_file() {
                            emit(
                                file,
                                index,
                                &pointer,
                                Severity::Error,
                                &code("missing"),
                                format!("{kind} file does not exist: {asset_file}"),
                                result,
                            );
                        } else if dunce::canonicalize(&candidate)
                            .ok()
                            .is_some_and(|resolved| !resolved.starts_with(pack_root))
                        {
                            emit(
                                file,
                                index,
                                &pointer,
                                Severity::Error,
                                &code("symlink"),
                                format!("{kind} file must not follow symlink outside pack"),
                                result,
                            );
                        } else if !(if sound {
                            &["wav", "flac"][..]
                        } else {
                            &["png", "jpg", "jpeg", "tga", "bmp"][..]
                        })
                        .contains(
                            &candidate
                                .extension()
                                .and_then(|value| value.to_str())
                                .unwrap_or("")
                                .to_lowercase()
                                .as_str(),
                        ) {
                            emit(
                                file,
                                index,
                                &pointer,
                                Severity::Error,
                                &code("format"),
                                if sound {
                                    "sound file must be a PCM16 .wav or .flac"
                                } else {
                                    "asset file must be png/jpg/jpeg/tga/bmp"
                                },
                                result,
                            );
                        } else if sound {
                            if let Err(error) = crate::sound::validate_file(&candidate) {
                                emit(
                                    file,
                                    index,
                                    &pointer,
                                    Severity::Error,
                                    &code("format"),
                                    error,
                                    result,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    semantic_document_local(file, index, document, kind, datapack, result);
}

fn split_include_document_index(relative: &str) -> (&str, Option<usize>) {
    let Some(without_closing) = relative.strip_suffix(']') else {
        return (relative, None);
    };
    let Some((file, index)) = without_closing.rsplit_once('[') else {
        return (relative, None);
    };
    if file.is_empty() || index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
        return (relative, None);
    }
    match index.parse::<usize>() {
        Ok(number) if number.to_string() == index => (file, Some(number)),
        _ => (relative, None),
    }
}

pub(crate) fn semantic_document_local(
    file: &ParsedFile,
    index: usize,
    document: &Value,
    kind: &str,
    datapack: Option<&ReflectionIndex>,
    result: &mut LintResult,
) {
    validate_condition_mod_specs(file, index, document, result);
    validate_scope_filters(file, index, document, kind, result);
    crate::runtime_features::validate(file, index, document, kind, result);
    if kind != "config" {
        // `properties` inside a config field is a reserved key, not an operation list.
        validate_operations(file, index, document, "", result);
    }
    validate_class_references(file, index, document, kind, datapack, result);
    if kind == "building" {
        validate_building_parents(file, index, document, datapack, result);
    }
    if kind == "curve" {
        validate_curve_times(file, index, document, result);
        validate_curve_value_shapes(file, index, document, result);
    }
    if kind == "research" {
        validate_research_node_coordinates(file, index, document, result);
    }
    if kind == "gametag" {
        validate_gametag_edges(file, index, document, result);
    }
}

fn validate_gametag_edges(
    file: &ParsedFile,
    index: usize,
    document: &Value,
    result: &mut LintResult,
) {
    let Some(tags) = document.get("tags").and_then(Value::as_array) else {
        return;
    };
    for (tag_index, entry) in tags.iter().enumerate() {
        let (tag, pointer) = if let Some(tag) = entry.as_str() {
            (tag, format!("/tags/{tag_index}"))
        } else if let Some(tag) = entry.get("tag").and_then(Value::as_str) {
            (tag, format!("/tags/{tag_index}/tag"))
        } else {
            continue;
        };
        if tag.starts_with(['.', ' ']) || tag.ends_with(['.', ' ']) {
            emit(
                file,
                index,
                &pointer,
                Severity::Error,
                "semantic.gametag-edge",
                "gameplay tag must not begin or end with a period or space",
                result,
            );
        }
    }
}

pub(crate) fn validate_condition_mod_specs(
    file: &ParsedFile,
    index: usize,
    document: &Value,
    result: &mut LintResult,
) {
    let Some(conditions) = document.get("conditions") else {
        return;
    };
    validate_mod_specs_in(file, index, conditions, "/conditions", result);
}

/// `hasMod` specifications of a `conditions` value located at `pointer` (document, pack manifest
/// or sequence element).
pub(crate) fn validate_mod_specs_in(
    file: &ParsedFile,
    index: usize,
    conditions: &Value,
    pointer: &str,
    result: &mut LintResult,
) {
    if let Some(objects) = conditions.as_array() {
        for (object_index, object) in objects.iter().enumerate() {
            validate_condition_mod_object(
                file,
                index,
                object,
                &format!("{pointer}/{object_index}"),
                result,
            );
        }
    } else {
        validate_condition_mod_object(file, index, conditions, pointer, result);
    }
}

fn validate_condition_mod_object(
    file: &ParsedFile,
    index: usize,
    object: &Value,
    pointer: &str,
    result: &mut LintResult,
) {
    let Some((key, specs)) = object.as_object().and_then(|fields| {
        fields
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("hasMod"))
    }) else {
        return;
    };
    let pointer = format!("{pointer}/{key}");
    if let Some(items) = specs.as_array() {
        for (item_index, item) in items.iter().enumerate() {
            if let Some(spec) = item.as_str() {
                validate_mod_spec_parts(
                    file,
                    index,
                    spec,
                    &format!("{pointer}/{item_index}"),
                    result,
                );
            }
        }
    } else if let Some(spec) = specs.as_str() {
        validate_mod_spec_parts(file, index, spec, &pointer, result);
    }
}

fn validate_mod_spec_parts(
    file: &ParsedFile,
    index: usize,
    spec: &str,
    pointer: &str,
    result: &mut LintResult,
) {
    let invalid = if let Some((reference, range)) = spec.split_once('@') {
        reference.trim().is_empty() || range.trim().is_empty()
    } else {
        spec.trim().is_empty()
    };
    if invalid {
        emit(
            file,
            index,
            pointer,
            Severity::Error,
            "semantic.condition-mod-spec",
            "hasMod requires a mod reference and, after @, a non-empty version range",
            result,
        );
    }
}

fn validate_research_node_coordinates(
    file: &ParsedFile,
    index: usize,
    document: &Value,
    result: &mut LintResult,
) {
    let Some(entries) = document.get("research").and_then(Value::as_array) else {
        return;
    };
    for (entry_index, entry) in entries.iter().enumerate() {
        for nodes_key in ["addNodes", "nodes"] {
            let Some(nodes) = entry.get(nodes_key).and_then(Value::as_array) else {
                continue;
            };
            for (node_index, node) in nodes.iter().enumerate() {
                let Some(coordinate) = node.get("coordinate") else {
                    continue;
                };
                for axis in ["X", "Y"] {
                    let Some(value) = coordinate.get(axis) else {
                        continue;
                    };
                    if coordinate_outside_int32(value) {
                        emit(
                            file,
                            index,
                            &format!("/research/{entry_index}/{nodes_key}/{node_index}/coordinate/{axis}"),
                            Severity::Error,
                            "semantic.coordinate-range",
                            "research node coordinate must be between -2147483648 and 2147483647",
                            result,
                        );
                    }
                }
            }
        }
    }
}

fn coordinate_outside_int32(value: &Value) -> bool {
    let number = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()));
    number.is_some_and(|number| number < i32::MIN as f64 || number > i32::MAX as f64)
}

fn validate_curve_times(
    file: &ParsedFile,
    index: usize,
    document: &Value,
    result: &mut LintResult,
) {
    let Some(curves) = document.get("curves").and_then(Value::as_array) else {
        return;
    };
    for (curve_index, curve) in curves.iter().enumerate() {
        let Some(keys) = curve.get("keys").and_then(Value::as_array) else {
            continue;
        };
        let mut seen = HashMap::<u32, usize>::new();
        for (key_index, key) in keys.iter().enumerate() {
            let Some(time) = key.get("time").and_then(curve_number) else {
                continue;
            };
            let time = time as f32;
            if !time.is_finite() {
                emit(
                    file,
                    index,
                    &format!("/curves/{curve_index}/keys/{key_index}/time"),
                    Severity::Error,
                    "semantic.curve-time-range",
                    "curve key time must fit a finite float32 value",
                    result,
                );
                continue;
            }
            let identity = if time == 0.0 { 0 } else { time.to_bits() };
            if let Some(previous) = seen.insert(identity, key_index) {
                emit(
                    file,
                    index,
                    &format!("/curves/{curve_index}/keys/{key_index}/time"),
                    Severity::Error,
                    "semantic.duplicate-curve-time",
                    format!("curve key time {time} duplicates key {previous}"),
                    result,
                );
            }
        }
    }
}

fn validate_curve_value_shapes(
    file: &ParsedFile,
    index: usize,
    document: &Value,
    result: &mut LintResult,
) {
    let Some(curve_type) = document.get("curveType").and_then(Value::as_str) else {
        return;
    };
    let is_vector = if curve_type.eq_ignore_ascii_case("vector") {
        true
    } else if curve_type.eq_ignore_ascii_case("float") {
        false
    } else {
        return;
    };
    let Some(curves) = document.get("curves").and_then(Value::as_array) else {
        return;
    };
    for (curve_index, curve) in curves.iter().enumerate() {
        if let Some(value) = curve.get("defaultValue") {
            check_curve_value_shape(
                file,
                index,
                value,
                is_vector,
                &format!("/curves/{curve_index}/defaultValue"),
                result,
            );
        }
        let Some(keys) = curve.get("keys").and_then(Value::as_array) else {
            continue;
        };
        for (key_index, key) in keys.iter().enumerate() {
            for field in [
                "value",
                "arriveTangent",
                "leaveTangent",
                "arriveTangentWeight",
                "leaveTangentWeight",
            ] {
                if let Some(value) = key.get(field) {
                    check_curve_value_shape(
                        file,
                        index,
                        value,
                        is_vector,
                        &format!("/curves/{curve_index}/keys/{key_index}/{field}"),
                        result,
                    );
                }
            }
        }
    }
}

fn check_curve_value_shape(
    file: &ParsedFile,
    index: usize,
    value: &Value,
    is_vector: bool,
    pointer: &str,
    result: &mut LintResult,
) {
    let valid = if is_vector {
        value.is_array() || value.is_object()
    } else {
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
            .is_some_and(|number| (number as f32).is_finite())
    };
    if !valid {
        emit(
            file,
            index,
            pointer,
            Severity::Error,
            "semantic.curve-value-kind",
            if is_vector {
                "vector curve value must be an {X,Y,Z} map or three-number sequence"
            } else {
                "float curve value must be a finite number"
            },
            result,
        );
        return;
    }
    if !is_vector {
        return;
    }
    match value {
        Value::Array(components) => {
            for (component_index, component) in components.iter().enumerate() {
                check_curve_component_range(
                    file,
                    index,
                    component,
                    &format!("{pointer}/{component_index}"),
                    result,
                );
            }
        }
        Value::Object(components) => {
            for (component_name, component) in components {
                check_curve_component_range(
                    file,
                    index,
                    component,
                    &format!(
                        "{pointer}/{}",
                        component_name.replace('~', "~0").replace('/', "~1")
                    ),
                    result,
                );
            }
        }
        _ => {}
    }
}

fn check_curve_component_range(
    file: &ParsedFile,
    index: usize,
    value: &Value,
    pointer: &str,
    result: &mut LintResult,
) {
    if curve_number(value).is_some_and(|number| !(number as f32).is_finite()) {
        emit(
            file,
            index,
            pointer,
            Severity::Error,
            "semantic.curve-value-range",
            "vector curve component must fit a finite float32 value",
            result,
        );
    }
}

fn curve_number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
}

fn validate_scope_filters(
    file: &ParsedFile,
    index: usize,
    document: &Value,
    kind: &str,
    result: &mut LintResult,
) {
    fn has_filter(entry: &serde_json::Map<String, Value>) -> bool {
        [
            "matchTag",
            "matchName",
            "classType",
            "excludeClasses",
            "where",
            "producedIn",
            "ingredient",
            "product",
        ]
        .iter()
        .any(|key| entry.contains_key(*key))
    }
    for (key, needs_class) in match kind {
        "cdo" => vec![("patches", true)],
        "sublevel" => vec![("block", true)],
        "sinkpoints" => vec![("entries", false)],
        "recipe" | "schematic" | "research" | "resourcenode" => vec![("remove", false)],
        _ => Vec::new(),
    } {
        let Some(entries) = document.get(key).and_then(Value::as_array) else {
            continue;
        };
        for (entry_index, value) in entries.iter().enumerate() {
            let Some(entry) = value.as_object() else {
                continue;
            };
            let pointer = format!("/{key}/{entry_index}");
            if kind == "cdo"
                && needs_class
                && [
                    "matchTag",
                    "matchName",
                    "where",
                    "classType",
                    "excludeClasses",
                ]
                .iter()
                .any(|key| entry.contains_key(*key))
                && !entry.contains_key("ofClass")
                && !["producedIn", "ingredient", "product"]
                    .iter()
                    .any(|key| entry.contains_key(*key))
            {
                emit(
                    file,
                    index,
                    &pointer,
                    Severity::Error,
                    "semantic.scope-class",
                    "scope filters require ofClass or a recipe filter",
                    result,
                );
            }
            if kind == "sublevel" && has_filter(entry) && !entry.contains_key("ofClass") {
                emit(
                    file,
                    index,
                    &pointer,
                    Severity::Error,
                    "semantic.scope-class",
                    "sublevel filters require ofClass",
                    result,
                );
            }
            if kind == "cdo"
                && key == "patches"
                && !has_filter(entry)
                && entry
                    .get("ofClass")
                    .and_then(Value::as_str)
                    .is_some_and(|class| !class.is_empty())
            {
                emit(
                    file,
                    index,
                    &format!("{pointer}/ofClass"),
                    Severity::Warning,
                    "semantic.scope-unused",
                    "ofClass has no effect without a scope filter",
                    result,
                );
            }
            if key == "patches"
                && !entry.contains_key("target")
                && !entry.contains_key("allAssetsOfClass")
                && !has_filter(entry)
            {
                emit(
                    file,
                    index,
                    &pointer,
                    Severity::Error,
                    "semantic.scope-target",
                    "cdo patch requires target, allAssetsOfClass or a scope filter",
                    result,
                );
            }
            let has_resource = kind == "resourcenode"
                && entry
                    .get("resource")
                    .and_then(Value::as_str)
                    .is_some_and(|resource| !resource.is_empty());
            if key == "remove" && !has_filter(entry) && !has_resource {
                emit(
                    file,
                    index,
                    &pointer,
                    Severity::Error,
                    "semantic.scope-filter",
                    if kind == "resourcenode" {
                        "resource node removal requires resource or a scope filter"
                    } else {
                        "remove map entries need a filter"
                    },
                    result,
                );
            }
            if key == "entries"
                && kind == "sinkpoints"
                && !entry.contains_key("item")
                && !has_filter(entry)
            {
                emit(
                    file,
                    index,
                    &pointer,
                    Severity::Error,
                    "semantic.scope-filter",
                    "sinkpoints entry requires item or a filter",
                    result,
                );
            }
        }
    }
}

fn validate_operations(
    file: &ParsedFile,
    index: usize,
    node: &Value,
    pointer: &str,
    result: &mut LintResult,
) {
    match node {
        Value::Object(map) => {
            if let Some(properties) = map.get("properties").and_then(Value::as_array) {
                // Property entries execute in order; repeated paths are valid operation chains.
                for (i, item) in properties.iter().enumerate() {
                    let location = format!("{pointer}/properties/{i}");
                    if let Some(operation) = item.as_object() {
                        let op = operation
                            .get("op")
                            .and_then(Value::as_str)
                            .unwrap_or("set")
                            .to_lowercase();
                        let required = match operation_spec(&op) {
                            Some(spec) => spec.required,
                            None => {
                                emit(
                                    file,
                                    index,
                                    &format!("{location}/op"),
                                    Severity::Error,
                                    "semantic.invalid-op",
                                    format!("unknown property operation {op:?}"),
                                    result,
                                );
                                &[]
                            }
                        };
                        for key in required {
                            if !operation.contains_key(*key) {
                                emit(
                                    file,
                                    index,
                                    &location,
                                    Severity::Error,
                                    "semantic.op-required",
                                    format!("operation {op:?} requires {key:?}"),
                                    result,
                                );
                            }
                        }
                        for key in ["index", "with"] {
                            if required.contains(&key)
                                && operation.get(key).is_some_and(is_invalid_array_index)
                            {
                                emit(
                                    file,
                                    index,
                                    &format!("{location}/{key}"),
                                    Severity::Error,
                                    "semantic.index-range",
                                    "array index must be between 0 and 2147483647",
                                    result,
                                );
                            }
                        }
                        if op == "divide" && operation.get("value").is_some_and(is_numeric_zero) {
                            emit(
                                file,
                                index,
                                &format!("{location}/value"),
                                Severity::Error,
                                "semantic.divide-zero",
                                "divide operation cannot use zero",
                                result,
                            );
                        }
                        if let Some(path) = operation.get("path").and_then(Value::as_str) {
                            if path == crate::TAG_CONTAINER_SELECTOR {
                                if !crate::tag_container_operation(&op) {
                                    emit(
                                        file, index, &format!("{location}/op"), Severity::Error,
                                        "semantic.tag-container-op",
                                        "@tagContainers supports only set, append, prepend, remove and clear",
                                        result,
                                    );
                                }
                            } else if !valid_property_path(path) {
                                emit(
                                    file,
                                    index,
                                    &format!("{location}/path"),
                                    Severity::Error,
                                    "semantic.property-path",
                                    "path is not a valid property path",
                                    result,
                                );
                            } else if has_wildcard_accessor(path) {
                                emit(
                                    file,
                                    index,
                                    &format!("{location}/path"),
                                    Severity::Error,
                                    "semantic.property-path-wildcard",
                                    "[*] is only valid inside a where filter",
                                    result,
                                );
                            }
                        }
                    }
                }
            }
            if let Some(where_value) = map.get("where") {
                let clauses = where_value
                    .as_array()
                    .cloned()
                    .unwrap_or_else(|| vec![where_value.clone()]);
                for (i, clause) in clauses.iter().enumerate() {
                    let location = if where_value.is_array() {
                        format!("{pointer}/where/{i}")
                    } else {
                        format!("{pointer}/where")
                    };
                    if let Some(map) = clause.as_object() {
                        let operators = map
                            .keys()
                            .filter(|key| key.as_str() != "path")
                            .collect::<Vec<_>>();
                        if operators.len() != 1 {
                            emit(
                                file,
                                index,
                                &location,
                                Severity::Error,
                                "semantic.where-operator",
                                "where clause needs exactly one operator",
                                result,
                            );
                        }
                        if let Some(path) = map.get("path").and_then(Value::as_str) {
                            if !valid_property_path(path) {
                                emit(
                                    file,
                                    index,
                                    &format!("{location}/path"),
                                    Severity::Error,
                                    "semantic.where-path",
                                    "path is not a valid property path",
                                    result,
                                );
                            }
                        }
                        if let Some(operator) = operators.first() {
                            if canonical_where_operator(operator).is_none() {
                                emit(
                                    file,
                                    index,
                                    &location,
                                    Severity::Error,
                                    "semantic.where-unknown-operator",
                                    format!("{operator} is not a where operator"),
                                    result,
                                );
                            }
                        }
                    }
                }
            }
            for (key, value) in map {
                validate_operations(
                    file,
                    index,
                    value,
                    &format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1")),
                    result,
                );
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                validate_operations(file, index, item, &format!("{pointer}/{i}"), result);
            }
        }
        _ => {}
    }
}

fn is_numeric_zero(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::String(text) => text.parse::<f64>().is_ok_and(|number| number == 0.0),
        _ => false,
    }
}

fn is_invalid_array_index(value: &Value) -> bool {
    value
        .as_i64()
        .is_some_and(|index| index < 0 || index > i32::MAX as i64)
        || value.as_u64().is_some_and(|index| index > i32::MAX as u64)
}

fn valid_property_path(path: &str) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r#"^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*|\[(?:[^\[\]"'*]+|\*|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*')\])*$"#).expect("static regex")).is_match(path)
}

/// Comparison key for property paths without reflected map-key type information.
///
/// Runtime KDFPropertyPath.cpp resolves member names case-insensitively, but imports
/// FString map keys exactly. Preserve accessor spelling (including escapes and indices)
/// rather than assume that every map uses case-insensitive FName keys. Invalid paths
/// remain literal, so incomplete completion prefixes cannot acquire a false identity.
pub fn property_path_identity(path: &str) -> String {
    let path = path.trim();
    if !valid_property_path(path) {
        return path.to_owned();
    }
    static ACCESSOR: OnceLock<Regex> = OnceLock::new();
    let accessor = ACCESSOR.get_or_init(|| {
        Regex::new(r#"\[(?:[^\[\]"'*]+|\*|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*')\]"#)
            .expect("static regex")
    });
    let mut identity = String::with_capacity(path.len());
    let mut start = 0;
    for literal in accessor.find_iter(path) {
        identity.push_str(&path[start..literal.start()].to_ascii_lowercase());
        identity.push_str(literal.as_str());
        start = literal.end();
    }
    identity.push_str(&path[start..].to_ascii_lowercase());
    identity
}

fn has_wildcard_accessor(path: &str) -> bool {
    let mut characters = path.chars().peekable();
    let mut quote = None;
    while let Some(character) = characters.next() {
        if let Some(delimiter) = quote {
            if character == '\\' {
                characters.next();
            } else if character == delimiter {
                quote = None;
            }
            continue;
        }
        if character == '"' || character == '\'' {
            quote = Some(character);
        } else if character == '[' && characters.peek() == Some(&'*') {
            characters.next();
            if characters.peek() == Some(&']') {
                return true;
            }
        }
    }
    false
}

fn validate_class_references(
    file: &ParsedFile,
    index: usize,
    document: &Value,
    kind: &str,
    datapack: Option<&ReflectionIndex>,
    result: &mut LintResult,
) {
    let classes = class_catalog();
    let check = |path: &str, family: &str, pointer: &str, result: &mut LintResult| {
        if path.starts_with("/KDataForge/Gen/") || (!path.contains('/') && !path.contains('.')) {
            return;
        }
        check_reflection_class(file, index, path, family, pointer, datapack, result);
        match classes.get(&normalize_class(path)) {
            None if datapack.is_none() => emit(file, index, pointer, Severity::Warning, "class.unknown", format!("{family} class {path:?} is not in the bundled Unreal class catalog; runtime will verify it"), result),
            Some(entry) if entry.family != family => emit(file, index, pointer, Severity::Warning, "class.wrong-family", format!("{path:?} is a {} class, not a {family} class", entry.family), result),
            Some(entry) if entry.blueprint.is_some() && path.starts_with("/Script/") => emit(file, index, pointer, Severity::Warning, "class.native-remapped", format!("native class {path:?} is remapped at runtime to concrete Blueprint {}", entry.blueprint.as_deref().unwrap_or("")), result),
            Some(entry) if entry.abstract_class => emit(file, index, pointer, Severity::Warning, "class.abstract-no-blueprint", format!("abstract {family} class {path:?} has no known concrete Blueprint variant"), result),
            _ => {}
        }
    };
    let roots: &[(&str, &str)] = match kind {
        "unlock" => &[("unlocks", "unlock")],
        "schematic" => &[("schematics", "")],
        "research" => &[("research", "")],
        "dataasset" => &[("assets", "")],
        _ => &[],
    };
    for (key, default_family) in roots {
        if let Some(entries) = document.get(*key).and_then(Value::as_array) {
            for (i, entry) in entries.iter().enumerate() {
                if kind == "dataasset" && *key == "assets" {
                    if let Some(path) = entry.get("class").and_then(Value::as_str) {
                        check_reflection_class(
                            file,
                            index,
                            path,
                            "data asset",
                            &format!("/{key}/{i}/class"),
                            datapack,
                            result,
                        );
                    }
                }
                if *default_family == "unlock" {
                    if let Some(path) = entry.get("parent").and_then(Value::as_str) {
                        check(path, "unlock", &format!("/{key}/{i}/parent"), result);
                    }
                }
                for (child_key, family) in [
                    ("unlocks", "unlock"),
                    ("dependencies", "dependency"),
                    ("unlockDependencies", "dependency"),
                    ("visibilityDependencies", "dependency"),
                ] {
                    if let Some(items) = entry.get(child_key).and_then(Value::as_array) {
                        for (j, child) in items.iter().enumerate() {
                            if let Some(path) = child.get("class").and_then(Value::as_str) {
                                check(
                                    path,
                                    family,
                                    &format!("/{key}/{i}/{child_key}/{j}/class"),
                                    result,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

const ACTOR_BASES: [&str; 2] = ["/Script/Engine.Actor", "/Script/FactoryGame.FGBuildable"];

/// A `building` entry generates a building DESCRIPTOR, so its `parent` has to derive from
/// `FGBuildingDescriptor`; the runtime refuses any other class. Only classes the snapshot resolves
/// are judged: an unknown class keeps `datapack.unknown-class`, and a chain cut off by the snapshot
/// cannot prove a wrong parent unless it already passed through an actor class.
fn validate_building_parents(
    file: &ParsedFile,
    index: usize,
    document: &Value,
    datapack: Option<&ReflectionIndex>,
    result: &mut LintResult,
) {
    let Some(reflection) = datapack.filter(|reflection| {
        matches!(
            reflection.status,
            ReflectionStatus::Ready | ReflectionStatus::Limited
        )
    }) else {
        return;
    };
    let Some(base) = default_base_class("buildings") else {
        return;
    };
    let entries = document
        .get("buildings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for (position, entry) in entries.enumerate() {
        let Some(parent) = entry.get("parent").and_then(Value::as_str) else {
            continue;
        };
        let names_class = parent.starts_with('/') || parent.contains('\'');
        if !names_class || parent.contains("/KDataForge/Gen/") || parent.contains('{') {
            continue;
        }
        let Some(class) = reflection.class_path(parent) else {
            continue;
        };
        let chain = reflection.super_chain_inclusive(class);
        if chain.contains(&base) {
            continue;
        }
        let is_actor = chain.iter().any(|path| ACTOR_BASES.contains(path));
        let reaches_root = chain
            .last()
            .is_some_and(|top| reflection.parent_path(top).is_none());
        if !is_actor && !reaches_root {
            continue;
        }
        let message = if is_actor {
            let suggestion = reflection
                .icon_descriptor_path(class)
                .map(|descriptor| format!(" Did you mean {descriptor}?"))
                .unwrap_or_default();
            format!("{parent:?} is a buildable (actor) class, but a building entry generates a building descriptor and its parent must derive from {base}.{suggestion}")
        } else {
            format!("{parent:?} is not a building descriptor; the parent of a building entry must derive from {base}.")
        };
        emit(
            file,
            index,
            &format!("/buildings/{position}/parent"),
            Severity::Error,
            "class.wrong-family",
            message,
            result,
        );
    }
}

fn check_reflection_class(
    file: &ParsedFile,
    index: usize,
    path: &str,
    family: &str,
    pointer: &str,
    datapack: Option<&ReflectionIndex>,
    result: &mut LintResult,
) {
    let Some(reflection) = datapack.filter(|reflection| {
        matches!(
            reflection.status,
            ReflectionStatus::Ready | ReflectionStatus::Limited
        )
    }) else {
        return;
    };
    if !reflection.contains_class(path) {
        emit(file, index, pointer, Severity::Warning, "datapack.unknown-class", format!("{family} class {path:?} is absent from bound reflection snapshot; runtime may still load it"), result);
    } else if family == "data asset" && reflection.is_abstract(path) {
        emit(
            file,
            index,
            pointer,
            Severity::Error,
            "class.abstract-no-blueprint",
            format!("Data assets require a concrete class; {path:?} is abstract"),
            result,
        );
    }
}

fn normalize_class(path: &str) -> String {
    let mut value = path.trim().to_owned();
    if let Some((_, inside)) = value.split_once('\'') {
        value = inside.split('\'').next().unwrap_or(inside).to_owned();
    }
    if value.starts_with('/') && !value.starts_with("/Script/") {
        let asset = value.rsplit('/').next().unwrap_or("").to_owned();
        if !asset.contains('.') {
            value = format!("{value}.{asset}_C");
        } else if !value.ends_with("_C") {
            value.push_str("_C");
        }
    }
    value.to_lowercase()
}
