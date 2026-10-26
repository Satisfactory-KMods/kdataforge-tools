//! Builds the [`ConfigIndex`] from parsed files and applies the definition rules of
//! `<name>.config.yml` documents (runtime design 1.1).

use crate::config::{
    is_config_file, Apply, ClassMap, ConfigField, ConfigIndex, DefinitionIssue, FieldKind,
    FieldOption, GeneratedClass, NumberDisplay, PackConfig, ValueType, CATEGORY_BASE_CLASS,
    RESERVED_IDS,
};
use crate::document::runtime_bool;
use crate::expression::is_config_id;
use crate::project::{is_pack_manifest, ParsedFile};
use crate::schema::infer_type;
use crate::slots::default_base_class;
use crate::{Range, ReflectionIndex, ReflectionStatus, Severity, SourceMap};
use serde_json::{Map, Value};
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const DEFAULT_MAX_LENGTH: u64 = 1024;
const MAX_PARENT_HOPS: usize = 8;

// Definition limits enforced by the runtime (host sync, RPC and key size caps). A definition above
// a limit is `config.limit-exceeded`.
/// Largest list `maxCount` / multi-select `maxSelected`; lists without `maxCount` are capped here.
pub(crate) const MAX_ELEMENT_COUNT: u64 = 64;
/// Largest text `maxLength`.
pub(crate) const MAX_TEXT_LENGTH: u64 = 1024;
/// Longest field id.
pub(crate) const MAX_ID_LENGTH: usize = 200;
/// Longest pack ref of a pack that defines configuration.
pub(crate) const MAX_PACK_REF_LENGTH: usize = 50;

struct PackBuild {
    index: PackConfig,
    docs: Vec<(usize, usize)>,
    classes: Vec<GeneratedClass>,
}

/// Builds the index. `files` must be sorted by path (the project pass sorts them). Definition
/// problems are collected as [`DefinitionIssue`]s on the returned index.
pub(crate) fn build<F: Borrow<ParsedFile>>(
    files: &[F],
    reflection: Option<&ReflectionIndex>,
) -> ConfigIndex {
    build_with(files, reflection, false)
}

/// Index of one document on its own (no `pack.yml` needed): used by single-document validation.
pub(crate) fn build_single(file: &ParsedFile, reflection: Option<&ReflectionIndex>) -> ConfigIndex {
    build_with(std::slice::from_ref(file), reflection, true)
}

fn build_with<F: Borrow<ParsedFile>>(
    files: &[F],
    reflection: Option<&ReflectionIndex>,
    standalone: bool,
) -> ConfigIndex {
    let files: Vec<&ParsedFile> = files.iter().map(Borrow::borrow).collect();
    let files = files.as_slice();
    let mut issues = Vec::new();
    let mut packs: Vec<PackBuild> = Vec::new();
    let mut by_dir: HashMap<PathBuf, usize> = HashMap::new();
    for file in files.iter().filter(|file| is_pack_manifest(&file.path)) {
        let Some(document) = file.documents.first() else {
            continue;
        };
        let Some(pack_ref) = document
            .get("ref")
            .and_then(Value::as_str)
            .filter(|reference| !reference.is_empty())
        else {
            continue;
        };
        if packs.iter().any(|pack| pack.index.pack_ref == pack_ref) {
            continue;
        }
        let Some(dir) = file.path.parent() else {
            continue;
        };
        by_dir.insert(dir.to_path_buf(), packs.len());
        packs.push(PackBuild {
            index: PackConfig {
                pack_ref: pack_ref.to_owned(),
                name: document.get("name").and_then(text_form),
                dir: dir.to_path_buf(),
                manifest: file.path.clone(),
                enabled: document.get("enabled").and_then(runtime_bool) != Some(false),
                fields: Vec::new(),
                files: Vec::new(),
            },
            docs: Vec::new(),
            classes: Vec::new(),
        });
    }
    if standalone && packs.is_empty() {
        if let Some(dir) = files.first().and_then(|file| file.path.parent()) {
            by_dir.insert(dir.to_path_buf(), 0);
            packs.push(PackBuild {
                index: PackConfig {
                    pack_ref: String::new(),
                    name: None,
                    dir: dir.to_path_buf(),
                    manifest: dir.join("pack.yml"),
                    enabled: true,
                    fields: Vec::new(),
                    files: Vec::new(),
                },
                docs: Vec::new(),
                classes: Vec::new(),
            });
        }
    }
    for (file_position, file) in files.iter().enumerate() {
        if is_pack_manifest(&file.path) {
            continue;
        }
        let pack = file
            .path
            .ancestors()
            .skip(1)
            .find_map(|dir| by_dir.get(dir).copied());
        let config_file = is_config_file(&file.path);
        let multi = file.documents.len() > 1;
        for (document_index, document) in file.documents.iter().enumerate() {
            let Some(object) = document.as_object() else {
                continue;
            };
            let explicit = object
                .get("type")
                .and_then(Value::as_str)
                .map(|text| text.trim().to_ascii_lowercase())
                .filter(|text| !text.is_empty());
            if config_file {
                match explicit.as_deref() {
                    None | Some("config") => {}
                    Some(other) => {
                        issues.push(DefinitionIssue {
                            severity: Severity::Error,
                            code: "config.filename-type",
                            message: format!(
                                "*.config.yml files are config definitions; type {other:?} is not allowed (rename the file to <name>.{other}.yml)"
                            ),
                            file: file.path.clone(),
                            document: document_index,
                            pointer: "/type".into(),
                        });
                        continue;
                    }
                }
                if let Some(pack) = pack {
                    packs[pack].docs.push((file_position, document_index));
                }
                continue;
            }
            if explicit.as_deref() == Some("config") {
                issues.push(DefinitionIssue {
                    severity: Severity::Error,
                    code: "config.filename",
                    message: "type: config is only valid in files named <name>.config.yml or <name>.config.yaml".into(),
                    file: file.path.clone(),
                    document: document_index,
                    pointer: "/type".into(),
                });
                continue;
            }
            let Some(pack) = pack else {
                continue;
            };
            let entries_key = match infer_type(&file.path, document, multi).as_deref() {
                Some("class") => "classes",
                Some("item") => "items",
                Some("resource") => "resources",
                Some("building") => "buildings",
                Some("unlock") => "unlocks",
                Some("recipe") => "recipes",
                Some("schematic") => "schematics",
                Some("research") => "research",
                _ => continue,
            };
            for entry in object
                .get(entries_key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(id) = entry.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let parent = entry
                    .get("parent")
                    .and_then(Value::as_str)
                    .or_else(|| default_base_class(entries_key))
                    .map(str::to_owned);
                packs[pack].classes.push(GeneratedClass {
                    id: id.to_owned(),
                    parent,
                    file: file.path.clone(),
                });
            }
        }
    }

    let mut classes = ClassMap::new();
    for pack in &packs {
        let map = classes.entry(pack.index.pack_ref.clone()).or_default();
        for class in &pack.classes {
            map.entry(class.id.to_ascii_lowercase())
                .or_insert_with(|| class.clone());
        }
    }
    let class_view = ClassView {
        complete: !standalone,
        classes: &classes,
        reflection: reflection.filter(|reflection| {
            matches!(
                reflection.status,
                ReflectionStatus::Ready | ReflectionStatus::Limited
            )
        }),
    };

    let mut out_packs = Vec::new();
    for mut pack in packs {
        let mut state = PackState::default();
        let mut current_file: Option<usize> = None;
        let mut source_holder: Option<SourceMap<'_>> = None;
        for (file_position, document_index) in std::mem::take(&mut pack.docs) {
            let file = &files[file_position];
            if current_file != Some(file_position) {
                current_file = Some(file_position);
                source_holder = SourceMap::parse(&file.text).ok();
                pack.index.files.push(file.path.clone());
            }
            let Some(document) = file.documents.get(document_index) else {
                continue;
            };
            process_document(
                &mut state,
                &mut pack.index,
                &class_view,
                file,
                document_index,
                document,
                source_holder.as_ref(),
                &mut issues,
            );
        }
        pack.index.files.dedup();
        let ref_length = pack.index.pack_ref.encode_utf16().count();
        if !pack.index.files.is_empty() && ref_length > MAX_PACK_REF_LENGTH {
            issues.push(DefinitionIssue {
                severity: Severity::Error,
                code: "config.limit-exceeded",
                message: format!(
                    "pack ref {:?} is {ref_length} characters long; packs with configuration are limited to {MAX_PACK_REF_LENGTH} (the ref is part of every config key and of the values file name)",
                    pack.index.pack_ref
                ),
                file: pack.index.manifest.clone(),
                document: 0,
                pointer: "/ref".into(),
            });
        }
        out_packs.push(pack.index);
    }
    ConfigIndex::from_parts(out_packs, classes, issues)
}

struct ClassView<'a> {
    /// All class declarations of the pack are known (false for a single document).
    complete: bool,
    classes: &'a ClassMap,
    reflection: Option<&'a ReflectionIndex>,
}

impl ClassView<'_> {
    fn generated(&self, pack_ref: &str, id: &str) -> Option<&GeneratedClass> {
        self.classes.get(pack_ref)?.get(&id.to_ascii_lowercase())
    }
}

#[derive(Default)]
struct PackState {
    /// Lower-case id of every accepted field.
    ids: HashMap<String, String>,
    /// Lower-case proper dotted prefixes of accepted ids.
    prefixes: HashSet<String>,
    /// Declaration index across the pack's config files.
    declared: i64,
}

#[expect(
    clippy::too_many_arguments,
    reason = "definition processing needs the file, document and shared state"
)]
fn process_document(
    state: &mut PackState,
    pack: &mut PackConfig,
    classes: &ClassView<'_>,
    file: &ParsedFile,
    document_index: usize,
    document: &Value,
    source: Option<&SourceMap<'_>>,
    issues: &mut Vec<DefinitionIssue>,
) {
    let Some(object) = document.as_object() else {
        return;
    };
    let mut ctx = Ctx {
        pack_ref: pack.pack_ref.clone(),
        file: &file.path,
        document: document_index,
        source,
        issues,
    };
    // `config:` conditions make no sense in a definition document: values are not loaded yet.
    if let Some(conditions) = object.get("conditions") {
        for (relative, condition) in crate::conditions::condition_objects(conditions) {
            if let Some((key, _)) = condition
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("config"))
            {
                ctx.issue(
                    Severity::Error,
                    "config.condition-in-config",
                    format!("/conditions{relative}/{key}"),
                    "config: conditions cannot be used in a config definition; the values they read are defined by these files",
                );
            }
        }
    }
    let document_category = object
        .get("category")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty());
    let Some(fields) = object.get("fields").and_then(Value::as_array) else {
        return;
    };
    for (position, field) in fields.iter().enumerate() {
        let order = state.declared;
        state.declared += 1;
        if let Some(accepted) = process_field(
            state,
            &mut ctx,
            classes,
            document_category,
            position,
            order,
            field,
        ) {
            pack.fields.push(accepted);
        }
    }
}

struct Ctx<'a> {
    pack_ref: String,
    file: &'a Path,
    document: usize,
    source: Option<&'a SourceMap<'a>>,
    issues: &'a mut Vec<DefinitionIssue>,
}

impl Ctx<'_> {
    fn issue(
        &mut self,
        severity: Severity,
        code: &'static str,
        pointer: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.issues.push(DefinitionIssue {
            severity,
            code,
            message: message.into(),
            file: self.file.to_path_buf(),
            document: self.document,
            pointer: pointer.into(),
        });
    }
    fn range(&self, pointer: &str) -> Range {
        self.source
            .map(|source| source.location(self.document, pointer))
            .unwrap_or_default()
    }
}

/// Non-empty string or keyed text (`{key, ns, source}`): its display text.
fn text_form(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Object(map) => {
            let key = map
                .get("key")
                .and_then(Value::as_str)
                .filter(|k| !k.is_empty())?;
            Some(
                map.get("source")
                    .and_then(Value::as_str)
                    .filter(|source| !source.is_empty())
                    .unwrap_or(key)
                    .to_owned(),
            )
        }
        _ => None,
    }
}

fn is_integral(number: f64) -> bool {
    number.is_finite() && number.fract() == 0.0
}

fn allowed_value_type(kind: FieldKind, value_type: ValueType) -> bool {
    match kind {
        FieldKind::Text => matches!(
            value_type,
            ValueType::String | ValueType::Text | ValueType::Name | ValueType::Tag
        ),
        FieldKind::Number => value_type.is_number(),
        FieldKind::Boolean => matches!(value_type, ValueType::Bool | ValueType::UInt8),
        FieldKind::Select => true,
    }
}

/// Valid gameplay tag text (`A.B.C`, no edge periods or spaces).
fn is_tag_text(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with(['.', ' '])
        && !text.ends_with(['.', ' '])
        && text.split('.').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        })
}

fn process_field(
    state: &mut PackState,
    ctx: &mut Ctx<'_>,
    classes: &ClassView<'_>,
    document_category: Option<&str>,
    position: usize,
    order: i64,
    field: &Value,
) -> Option<ConfigField> {
    let base = format!("/fields/{position}");
    let object = field.as_object()?;
    for key in ["descriptionClass", "properties"] {
        if object.contains_key(key) {
            ctx.issue(
                Severity::Warning,
                "config.reserved-key",
                format!("{base}/{key}"),
                format!("{key} is reserved for a later version and is ignored"),
            );
        }
    }
    let id = object.get("id")?.as_str()?;
    if !is_config_id(id) {
        return None;
    }
    if id.len() > MAX_ID_LENGTH {
        ctx.issue(
            Severity::Error,
            "config.limit-exceeded",
            format!("{base}/id"),
            format!(
                "config id is {} characters long; the limit is {MAX_ID_LENGTH}",
                id.len()
            ),
        );
        return None;
    }
    if RESERVED_IDS.contains(&id) {
        ctx.issue(
            Severity::Error,
            "config.id-reserved",
            format!("{base}/id"),
            format!(
                "{id:?} is a reserved word of the expression grammar and cannot be a config id"
            ),
        );
        return None;
    }
    let kind = FieldKind::parse(object.get("type")?.as_str()?)?;
    let value_type = match object.get("valueType") {
        None => ValueType::default_for(kind),
        Some(value) => {
            let value_type = ValueType::parse(value.as_str()?)?;
            if !allowed_value_type(kind, value_type) {
                return None;
            }
            value_type
        }
    };
    let lower = id.to_ascii_lowercase();
    if let Some(previous) = state.ids.get(&lower) {
        ctx.issue(
            Severity::Error,
            "config.duplicate-id",
            format!("{base}/id"),
            format!("config id {id:?} is already defined as {previous:?} in this pack (ids are unique ignoring case)"),
        );
        return None;
    }
    let parts = lower.split('.').collect::<Vec<_>>();
    for end in 1..parts.len() {
        let prefix = parts[..end].join(".");
        if let Some(existing) = state.ids.get(&prefix) {
            ctx.issue(
                Severity::Error,
                "config.prefix-clash",
                format!("{base}/id"),
                format!("config id {id:?} cannot be used: {existing:?} is a value, so it cannot also be a group"),
            );
            return None;
        }
    }
    if state.prefixes.contains(&lower) {
        ctx.issue(
            Severity::Error,
            "config.prefix-clash",
            format!("{base}/id"),
            format!("config id {id:?} cannot be used: other ids are nested below it, so it cannot also be a value"),
        );
        return None;
    }
    let name = text_form(object.get("name")?)?;
    let description = text_form(object.get("description")?)?;
    let default = object.get("default")?;
    let multiple = object
        .get("multiple")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if multiple && kind == FieldKind::Boolean {
        return None;
    }
    let apply = match object.get("apply").and_then(Value::as_str) {
        Some("hot") => Apply::Hot,
        _ => Apply::Cold,
    };

    let mut rules = FieldRules {
        ctx: &mut *ctx,
        base: &base,
        object,
        value_type,
        multiple,
        valid: true,
    };
    let numbers = match kind {
        FieldKind::Number => rules.check_number(default),
        FieldKind::Text => {
            rules.check_text(default);
            Numbers::default()
        }
        FieldKind::Boolean => Numbers::default(),
        FieldKind::Select => {
            rules.check_select(default);
            Numbers::default()
        }
    };
    if !rules.valid {
        return None;
    }

    let category_value = object
        .get("category")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .or(document_category);
    let Some(category_value) = category_value else {
        ctx.issue(
            Severity::Error,
            "config.category-missing",
            base.clone(),
            "every config field needs a category: set category on the field or on the document",
        );
        return None;
    };
    let category_pointer = if object.get("category").is_some() {
        format!("{base}/category")
    } else {
        "/category".to_owned()
    };
    check_category(ctx, classes, category_value, &category_pointer);

    let option_values = object
        .get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    Some(FieldOption {
                        value: option.get("value")?.clone(),
                        label: option.get("label").and_then(text_form).unwrap_or_default(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tooltip = object.get("tooltip").and_then(Value::as_object);
    let min = numbers.min;
    let max = numbers.max;
    let display = match object.get("display").and_then(Value::as_str) {
        Some("input") => NumberDisplay::Input,
        Some("slider") => NumberDisplay::Slider,
        _ if min.is_some() && max.is_some() => NumberDisplay::Slider,
        _ => NumberDisplay::Input,
    };
    let count = |key: &str| object.get(key).and_then(Value::as_u64);

    state.ids.insert(lower.clone(), id.to_owned());
    for end in 1..parts.len() {
        state.prefixes.insert(parts[..end].join("."));
    }
    Some(ConfigField {
        pack_ref: ctx.pack_ref.clone(),
        id: id.to_owned(),
        kind,
        value_type,
        multiple,
        apply,
        name,
        description,
        tooltip_title: tooltip.and_then(|map| map.get("title")).and_then(text_form),
        tooltip_text: tooltip.and_then(|map| map.get("text")).and_then(text_form),
        category: Some(category_value.to_owned()),
        default: default.clone(),
        min,
        max,
        step: numbers.step,
        decimals: object
            .get("decimals")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok()),
        display,
        options: option_values,
        min_selected: count("minSelected"),
        max_selected: count("maxSelected"),
        min_count: count("minCount"),
        max_count: count("maxCount"),
        max_length: (kind == FieldKind::Text)
            .then(|| count("maxLength").unwrap_or(DEFAULT_MAX_LENGTH)),
        multiline: object
            .get("multiline")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        tag_root: object
            .get("tagRoot")
            .and_then(Value::as_str)
            .map(str::to_owned),
        order: object.get("order").and_then(Value::as_i64).unwrap_or(order),
        file: ctx.file.to_path_buf(),
        document: ctx.document,
        pointer: base.clone(),
        range: ctx.range(&base),
        id_range: ctx.range(&format!("{base}/id")),
    })
}

fn check_category(ctx: &mut Ctx<'_>, classes: &ClassView<'_>, value: &str, pointer: &str) {
    if value.starts_with("/KDataForge/Gen/") {
        return;
    }
    let mut current = value.to_owned();
    for hop in 0..MAX_PARENT_HOPS {
        let bare = !current.contains('/') && !current.contains('.');
        if bare {
            match classes.generated(&ctx.pack_ref, &current) {
                None if hop == 0 && !classes.complete => return,
                None if hop == 0 => {
                    ctx.issue(
                        Severity::Error,
                        "config.category-invalid",
                        pointer,
                        format!("category {current:?} is not a class declared in this pack; declare it with type: class or use a full class path"),
                    );
                    return;
                }
                None => return,
                Some(declared) => match &declared.parent {
                    Some(parent) => current = parent.clone(),
                    None => return,
                },
            }
            continue;
        }
        if current.starts_with("/KDataForge/Gen/") {
            return;
        }
        let Some(reflection) = classes.reflection else {
            return;
        };
        match reflection.resolve_class(&current) {
            None => {
                ctx.issue(
                    Severity::Warning,
                    "config.category-invalid",
                    pointer,
                    format!("category class {current:?} is absent from the bound reflection snapshot; the runtime may still load it"),
                );
            }
            Some(class) => {
                if !reflection.is_subclass_of(class.path, CATEGORY_BASE_CLASS) {
                    let message = if current == value {
                        format!("category {value:?} is not a UFGCategory class; the field falls back to the default category")
                    } else {
                        format!("category {value:?} derives from {current:?}, which is not a UFGCategory class; the field falls back to the default category")
                    };
                    ctx.issue(Severity::Error, "config.category-invalid", pointer, message);
                }
            }
        }
        return;
    }
}

#[derive(Default)]
struct Numbers {
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
}

struct FieldRules<'a, 'c> {
    ctx: &'a mut Ctx<'c>,
    base: &'a str,
    object: &'a Map<String, Value>,
    value_type: ValueType,
    multiple: bool,
    valid: bool,
}

impl FieldRules<'_, '_> {
    fn error(&mut self, code: &'static str, suffix: &str, message: impl Into<String>) {
        self.valid = false;
        let pointer = format!("{}{suffix}", self.base);
        self.ctx.issue(Severity::Error, code, pointer, message);
    }

    fn count_bounds(
        &mut self,
        min_key: &str,
        max_key: &str,
        limit: Option<u64>,
    ) -> (Option<u64>, Option<u64>) {
        let min = self.object.get(min_key).and_then(Value::as_u64);
        let max = self.object.get(max_key).and_then(Value::as_u64);
        if let (Some(min), Some(max)) = (min, max) {
            if min > max {
                self.error(
                    "config.range-invalid",
                    &format!("/{min_key}"),
                    format!("{min_key} ({min}) is larger than {max_key} ({max})"),
                );
            }
        }
        if let (Some(limit), Some(max)) = (limit, max) {
            if max > limit {
                self.error(
                    "config.range-invalid",
                    &format!("/{max_key}"),
                    format!("{max_key} ({max}) is larger than the number of options ({limit})"),
                );
            }
        }
        if let (Some(limit), Some(min)) = (limit, min) {
            if min > limit {
                self.error(
                    "config.range-invalid",
                    &format!("/{min_key}"),
                    format!("{min_key} ({min}) is larger than the number of options ({limit})"),
                );
            }
        }
        for (key, value) in [(min_key, min), (max_key, max)] {
            if let Some(value) = value.filter(|value| *value > MAX_ELEMENT_COUNT) {
                self.error(
                    "config.limit-exceeded",
                    &format!("/{key}"),
                    format!("{key} ({value}) is larger than the limit of {MAX_ELEMENT_COUNT} list elements"),
                );
            }
        }
        (min, max)
    }

    /// Default element list with pointers. `None` when the default has the wrong shape (the
    /// schema pass reports type problems; a list for a scalar is reported here).
    fn defaults<'v>(&mut self, default: &'v Value) -> Option<Vec<(&'v Value, String)>> {
        if self.multiple {
            let items = default.as_array()?;
            Some(
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| (item, format!("/default/{index}")))
                    .collect(),
            )
        } else if default.is_array() {
            self.error(
                "config.default-invalid",
                "/default",
                "default must be a single value unless multiple: true",
            );
            None
        } else {
            Some(vec![(default, "/default".to_owned())])
        }
    }

    fn check_list_size(&mut self, count: usize, min_key: &str, max_key: &str) {
        if !self.multiple {
            return;
        }
        let min = self.object.get(min_key).and_then(Value::as_u64);
        let max = self.object.get(max_key).and_then(Value::as_u64);
        if min.is_some_and(|min| (count as u64) < min) {
            self.error(
                "config.default-invalid",
                "/default",
                format!(
                    "default has {count} element(s), fewer than {min_key} ({})",
                    min.unwrap_or(0)
                ),
            );
        }
        if max.is_some_and(|max| (count as u64) > max) {
            self.error(
                "config.default-invalid",
                "/default",
                format!(
                    "default has {count} element(s), more than {max_key} ({})",
                    max.unwrap_or(0)
                ),
            );
        } else if max.is_none() && count as u64 > MAX_ELEMENT_COUNT {
            self.error(
                "config.limit-exceeded",
                "/default",
                format!(
                    "default has {count} elements; lists without {max_key} are limited to {MAX_ELEMENT_COUNT}"
                ),
            );
        }
    }

    fn check_number(&mut self, default: &Value) -> Numbers {
        let min = self.object.get("min").and_then(Value::as_f64);
        let max = self.object.get("max").and_then(Value::as_f64);
        let step = self.object.get("step").and_then(Value::as_f64);
        let integer = self.value_type.is_integer();
        let width = self
            .value_type
            .integer_range()
            .map(|(low, high)| (low as f64, high as f64));
        for (key, value) in [("min", min), ("max", max)] {
            let Some(value) = value else { continue };
            if !value.is_finite() {
                self.error(
                    "config.range-invalid",
                    &format!("/{key}"),
                    format!("{key} must be a finite number"),
                );
            } else if let Some((low, high)) = width {
                if value < low || value > high {
                    self.error(
                        "config.range-invalid",
                        &format!("/{key}"),
                        format!(
                            "{key} ({value}) is outside the range of {} ({low} to {high})",
                            self.value_type.as_str()
                        ),
                    );
                }
            }
        }
        if let (Some(min), Some(max)) = (min, max) {
            if min > max {
                self.error(
                    "config.range-invalid",
                    "/min",
                    format!("min ({min}) is larger than max ({max})"),
                );
            } else if self.object.get("display").and_then(Value::as_str) == Some("slider")
                && min >= max
            {
                self.error(
                    "config.range-invalid",
                    "/display",
                    "a slider needs min smaller than max",
                );
            }
        }
        if self.multiple && self.object.get("display").and_then(Value::as_str) == Some("slider") {
            self.error(
                "config.range-invalid",
                "/display",
                "a slider cannot be combined with multiple: true",
            );
        }
        self.count_bounds("minCount", "maxCount", None);
        if let Some(defaults) = self.defaults(default) {
            let count = defaults.len();
            for (value, suffix) in defaults {
                let Some(number) = value.as_f64() else {
                    continue;
                };
                if integer && !is_integral(number) {
                    self.error(
                        "config.default-invalid",
                        &suffix,
                        format!(
                            "default {number} is not a whole number but the value type is {}",
                            self.value_type.as_str()
                        ),
                    );
                    continue;
                }
                if let Some((low, high)) = width {
                    if number < low || number > high {
                        self.error(
                            "config.default-invalid",
                            &suffix,
                            format!(
                                "default {number} is outside the range of {}",
                                self.value_type.as_str()
                            ),
                        );
                        continue;
                    }
                }
                if min.is_some_and(|min| number < min) || max.is_some_and(|max| number > max) {
                    self.error(
                        "config.default-invalid",
                        &suffix,
                        format!("default {number} is outside min/max"),
                    );
                    continue;
                }
                if let Some(step) = step.filter(|step| *step > 0.0) {
                    let steps = (number - min.unwrap_or(0.0)) / step;
                    if (steps - steps.round()).abs() > 1e-6 {
                        self.error(
                            "config.default-invalid",
                            &suffix,
                            format!("default {number} is not a multiple of step {step} (counted from {})", min.unwrap_or(0.0)),
                        );
                    }
                }
            }
            self.check_list_size(count, "minCount", "maxCount");
        }
        Numbers { min, max, step }
    }

    fn check_text(&mut self, default: &Value) {
        let max_length = self
            .object
            .get("maxLength")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_MAX_LENGTH);
        if max_length > MAX_TEXT_LENGTH {
            self.error(
                "config.limit-exceeded",
                "/maxLength",
                format!("maxLength ({max_length}) is larger than the limit of {MAX_TEXT_LENGTH} characters"),
            );
        }
        self.count_bounds("minCount", "maxCount", None);
        if let Some(defaults) = self.defaults(default) {
            let count = defaults.len();
            for (value, suffix) in defaults {
                let Some(text) = value.as_str() else { continue };
                if text.chars().count() as u64 > max_length {
                    self.error(
                        "config.default-invalid",
                        &suffix,
                        format!("default is longer than maxLength ({max_length})"),
                    );
                } else if self.value_type == ValueType::Tag
                    && !text.is_empty()
                    && !is_tag_text(text)
                {
                    self.error(
                        "config.default-invalid",
                        &suffix,
                        format!("{text:?} is not a valid gameplay tag"),
                    );
                }
            }
            self.check_list_size(count, "minCount", "maxCount");
        }
    }

    fn option_matches(&self, value: &Value) -> bool {
        match self.value_type {
            ValueType::String | ValueType::Text | ValueType::Name | ValueType::Tag => {
                value.is_string()
            }
            ValueType::Bool => value.is_boolean(),
            ValueType::Float | ValueType::Double => value.is_number(),
            integer => {
                value.as_f64().is_some_and(is_integral)
                    && integer.integer_range().is_some_and(|(low, high)| {
                        value
                            .as_f64()
                            .is_some_and(|number| number >= low as f64 && number <= high as f64)
                    })
            }
        }
    }

    fn check_select(&mut self, default: &Value) {
        let options = self
            .object
            .get("options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // Canonical text of the accepted option values: duplicate and default checks are O(1).
        let mut seen: HashSet<String> = HashSet::new();
        for (index, option) in options.iter().enumerate() {
            let Some(value) = option.get("value") else {
                continue;
            };
            if !self.option_matches(value) {
                self.error(
                    "config.option-invalid",
                    &format!("/options/{index}/value"),
                    format!(
                        "option value {value} does not match value type {}",
                        self.value_type.as_str()
                    ),
                );
                continue;
            }
            if !seen.insert(canonical_key(value)) {
                self.error(
                    "config.option-invalid",
                    &format!("/options/{index}/value"),
                    format!("option value {value} is used twice"),
                );
            }
        }
        let limit = Some(options.len() as u64);
        if self.multiple {
            self.count_bounds("minSelected", "maxSelected", limit);
        } else {
            for key in ["minSelected", "maxSelected"] {
                if self.object.contains_key(key) {
                    self.error(
                        "config.range-invalid",
                        &format!("/{key}"),
                        format!("{key} only applies with multiple: true"),
                    );
                }
            }
        }
        if let Some(defaults) = self.defaults(default) {
            let count = defaults.len();
            for (value, suffix) in defaults {
                if !seen.contains(&canonical_key(value)) {
                    self.error(
                        "config.default-invalid",
                        &suffix,
                        format!("default {value} is not one of the options"),
                    );
                }
            }
            self.check_list_size(count, "minSelected", "maxSelected");
        }
    }
}

/// Canonical text of a JSON value for set membership: numbers by value (`1` and `1.0` are one
/// value), everything else as compact JSON (object keys are sorted).
fn canonical_key(value: &Value) -> String {
    match value.as_f64() {
        Some(number) => format!("n{}", if number == 0.0 { 0.0 } else { number }),
        None => format!("v{value}"),
    }
}
