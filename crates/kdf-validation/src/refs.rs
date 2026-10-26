//! Typed references (`cfg:`, `session:`, `ags:`, `option:`) and templates (`{prefix:body}`)
//! inside content documents: resolution, compatibility, hot/cold usage and cold properties.
//!
//! The runtime resolves unquoted whole-scalar references and `{...}` tokens in every string scalar
//! of a document, except values of `id`, `type`, `include` and `ref`, map keys and everything
//! inside `conditions` subtrees. This module mirrors that walk.

use crate::cold::Reload;
use crate::config::{Apply, ConfigField, ConfigIndex, Resolution};
use crate::document::{runtime_bool, Reporter};
use crate::expression::{is_config_id, is_pack_ref_text, is_reserved_id, Span};
use crate::project::{ColdPropertyLevel, LintOptions, ParsedFile};
use crate::reflection_query::{
    explicit_params, Callable, FunctionInfo, PropertyInfo, PropertyKind, TargetKind,
};
use crate::schema::TypeConflict;
use crate::slots::{
    self, compat, expected_slot, field_shape, is_cold_context, setting_shape, Compat, ShapeKind,
    Slot, ValueShape,
};
use crate::source::ScalarInfo;
use crate::template::{
    self, ArgKind, Member, MemberErrorKind, Phase, ScanIssueKind, ScanPhase, StructFields,
    TemplateSegment, TemplateToken,
};
use crate::{Advisory, Diagnostic, Range, ReflectionIndex, ReflectionStatus, Severity, SourceMap};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

const REF_PREFIXES: [&str; 4] = ["cfg", "session", "ags", "option"];
/// Values of these keys are never resolved by the runtime.
const UNRESOLVED_KEYS: [&str; 4] = ["id", "type", "include", "ref"];
const MAX_CLASS_HOPS: usize = 8;

/// Everything the checks of one document need.
pub(crate) struct Env<'a> {
    pub(crate) file: &'a ParsedFile,
    pub(crate) source: Option<&'a SourceMap<'a>>,
    pub(crate) index: usize,
    pub(crate) kind: &'a str,
    pub(crate) pack_ref: Option<&'a str>,
    pub(crate) config: Option<&'a ConfigIndex>,
    pub(crate) datapack: Option<&'a ReflectionIndex>,
    pub(crate) options: &'a LintOptions,
}

impl<'a> Env<'a> {
    pub(crate) fn reporter(&self) -> Reporter<'a> {
        Reporter {
            file: self.file,
            source: self.source,
        }
    }
    /// Reflection data usable for class checks (`Ready` or `Limited`).
    pub(crate) fn reflection(&self) -> Option<&'a ReflectionIndex> {
        self.datapack.filter(|reflection| {
            matches!(
                reflection.status,
                ReflectionStatus::Ready | ReflectionStatus::Limited
            )
        })
    }
}

/// `^(cfg|session|ags|option):\S+$` split into prefix and rest.
pub(crate) fn typed_ref_parts(text: &str) -> Option<(&str, &str)> {
    let (prefix, rest) = text.split_once(':')?;
    if !REF_PREFIXES.contains(&prefix) || rest.is_empty() || rest.chars().any(char::is_whitespace) {
        return None;
    }
    Some((prefix, rest))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RefTarget {
    Config { pack: Option<String>, id: String },
    Setting { provider: String, key: String },
}

/// Parses the part after the prefix. `Err` carries a message for `template.malformed`.
pub(crate) fn parse_target(prefix: &str, rest: &str) -> Result<RefTarget, String> {
    if rest.is_empty() {
        return Err(format!("{prefix}: needs an id"));
    }
    if prefix == "cfg" {
        let parts = rest.split(':').collect::<Vec<_>>();
        let (pack, id) = match parts.as_slice() {
            [id] => (None, *id),
            [pack, id] => (Some(*pack), *id),
            _ => return Err("expected cfg:<id> or cfg:<pack>:<id>".into()),
        };
        if let Some(pack) = pack {
            if !is_pack_ref_text(pack) {
                return Err(format!("{pack:?} is not a valid pack reference"));
            }
        }
        if id.is_empty() {
            return Err("the config id is empty".into());
        }
        if is_reserved_id(id) {
            return Err(format!(
                "{id:?} is a reserved word and cannot be a config id"
            ));
        }
        if !is_config_id(id) {
            return Err(format!(
                "{id:?} is not a valid config id (letters, digits and underscores separated by dots)"
            ));
        }
        return Ok(RefTarget::Config {
            pack: pack.map(str::to_owned),
            id: id.to_owned(),
        });
    }
    if rest.contains(':') || !is_pack_ref_text(rest) {
        return Err(format!("{rest:?} is not a valid setting id"));
    }
    Ok(RefTarget::Setting {
        provider: prefix.to_owned(),
        key: rest.to_owned(),
    })
}

/// Unquoted whole-scalar typed references of a document, by JSON pointer. Used by the schema pass:
/// `type` errors there become compatibility checks, range and pattern errors disappear.
pub(crate) fn typed_ref_pointers(
    document: &Value,
    source: &SourceMap<'_>,
    index: usize,
) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut pointer = String::new();
    collect_typed(document, None, &mut pointer, source, index, &mut out);
    out
}

fn collect_typed(
    value: &Value,
    key: Option<&str>,
    pointer: &mut String,
    source: &SourceMap<'_>,
    index: usize,
    out: &mut HashSet<String>,
) {
    match value {
        Value::String(text) => {
            if key.is_some_and(|key| UNRESOLVED_KEYS.contains(&key)) {
                return;
            }
            if typed_ref_parts(text).is_some()
                && source
                    .scalar_info(index, pointer)
                    .is_none_or(|info| info.style.is_plain())
            {
                out.insert(pointer.clone());
            }
        }
        Value::Array(items) => {
            for (position, item) in items.iter().enumerate() {
                let length = pointer.len();
                pointer.push('/');
                pointer.push_str(&position.to_string());
                collect_typed(item, key, pointer, source, index, out);
                pointer.truncate(length);
            }
        }
        Value::Object(map) => {
            for (name, child) in map {
                if crate::conditions::HOLDER_KEYS.contains(&name.as_str()) {
                    continue;
                }
                let length = pointer.len();
                pointer.push('/');
                pointer.push_str(&name.replace('~', "~0").replace('/', "~1"));
                collect_typed(child, Some(name), pointer, source, index, out);
                pointer.truncate(length);
            }
        }
        _ => {}
    }
}

/// Outcome of resolving a reference to a value producer.
pub(crate) struct Resolved<'a> {
    pub(crate) field: Option<&'a ConfigField>,
    pub(crate) shape: Option<ValueShape>,
    pub(crate) hot: bool,
}

pub(crate) struct ResolveIssue {
    pub(crate) severity: Severity,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

fn issue(severity: Severity, code: &'static str, message: String) -> Option<ResolveIssue> {
    Some(ResolveIssue {
        severity,
        code,
        message,
    })
}

/// Resolves a typed reference. Returns the resolved value (when the target exists) and the issue
/// to report (when the reference is problematic).
pub(crate) fn resolve_target<'a>(
    env: &Env<'a>,
    target: &RefTarget,
) -> (Option<Resolved<'a>>, Option<ResolveIssue>) {
    match target {
        RefTarget::Config { pack, id } => {
            let Some(index) = env.config else {
                return (None, None);
            };
            if pack.is_none() && env.pack_ref.is_none() {
                return (None, None);
            }
            let label = match pack {
                Some(pack) => format!("{pack}:{id}"),
                None => id.clone(),
            };
            let (field, problem) = match index.resolve(env.pack_ref, pack.as_deref(), id) {
                Resolution::Found(field) => (Some(field), None),
                Resolution::CaseMismatch(field) => (
                    Some(field),
                    issue(
                        Severity::Warning,
                        "config.case-mismatch",
                        format!("config id {id:?} differs only in letter case from the defined {:?}; use the exact spelling", field.id),
                    ),
                ),
                Resolution::UnknownId => (
                    None,
                    issue(
                        Severity::Error,
                        "config.unknown-id",
                        format!("no config field {label:?} exists in pack {}", pack.as_deref().or(env.pack_ref).unwrap_or("?")),
                    ),
                ),
                Resolution::UnknownPack => (
                    None,
                    issue(
                        Severity::Warning,
                        "config.unknown-pack",
                        format!("pack {:?} is not part of this project; the reference resolves only when it is installed", pack.as_deref().unwrap_or("")),
                    ),
                ),
                Resolution::PackDisabled(field) => (
                    field,
                    issue(
                        Severity::Warning,
                        "config.pack-disabled",
                        format!("pack {:?} has enabled: false; its config values are not available", pack.as_deref().or(env.pack_ref).unwrap_or("")),
                    ),
                ),
            };
            let resolved = field.map(|field| Resolved {
                field: Some(field),
                shape: Some(field_shape(field)),
                hot: field.apply == Apply::Hot,
            });
            (resolved, problem)
        }
        RefTarget::Setting { provider, key } => {
            let Some(settings) = env.reflection().and_then(ReflectionIndex::settings) else {
                return (None, None);
            };
            match settings.find(provider, key) {
                Some(setting) => (
                    Some(Resolved {
                        field: None,
                        shape: Some(setting_shape(setting)),
                        hot: false,
                    }),
                    None,
                ),
                None => (
                    None,
                    issue(
                        Severity::Warning,
                        "setting.unknown-id",
                        format!("{provider} setting {key:?} is not in the datapack settings index"),
                    ),
                ),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MapKind {
    Op,
    Where,
    Other,
}

struct OpFrame {
    /// Lower-case operation name (`set` when omitted).
    op: String,
    property: Option<PropertyInfo>,
    /// `Some` when the property does not update live.
    cold: Option<(Reload, String)>,
}

struct ValueScope {
    /// Segment count at the `value` key.
    depth: usize,
}

struct Checker<'a, 'e> {
    env: &'e Env<'a>,
    reporter: Reporter<'a>,
    root: &'e Value,
    conflicts: HashMap<&'e str, &'e [String]>,
    pointer: String,
    segments: Vec<String>,
    ops: Vec<OpFrame>,
    maps: Vec<MapKind>,
    value_scopes: Vec<ValueScope>,
    out: Vec<Diagnostic>,
}

/// Checks typed references and templates of `document` (element conditions already stripped).
/// `conflicts` are the schema `type` errors at typed references.
pub(crate) fn check_document(
    env: &Env<'_>,
    document: &Value,
    conflicts: &[TypeConflict],
) -> Vec<Diagnostic> {
    let mut checker = Checker {
        env,
        reporter: env.reporter(),
        root: document,
        conflicts: conflicts
            .iter()
            .map(|conflict| (conflict.pointer.as_str(), conflict.expected.as_slice()))
            .collect(),
        pointer: String::new(),
        segments: Vec::new(),
        ops: Vec::new(),
        maps: Vec::new(),
        value_scopes: Vec::new(),
        out: Vec::new(),
    };
    checker.walk(document, None);
    checker.out
}

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

impl<'a, 'e> Checker<'a, 'e> {
    fn push(&mut self, segment: &str) {
        self.pointer.push('/');
        self.pointer.push_str(&escape(segment));
        self.segments.push(segment.to_owned());
    }

    fn pop(&mut self) {
        if let Some(segment) = self.segments.pop() {
            let length = self.pointer.len() - escape(&segment).len() - 1;
            self.pointer.truncate(length);
        }
    }

    fn cold_context(&self) -> bool {
        let segments = self.segments.iter().map(String::as_str).collect::<Vec<_>>();
        is_cold_context(self.env.kind, &segments)
    }

    fn report(&mut self, severity: Severity, code: &str, range: Range, message: impl Into<String>) {
        self.out.push(self.reporter.diagnostic(
            self.env.index,
            &self.pointer,
            severity,
            code,
            range,
            message,
        ));
    }

    fn scalar_range(&self) -> Range {
        self.reporter.range(self.env.index, &self.pointer)
    }

    fn walk(&mut self, value: &Value, key: Option<&str>) {
        match value {
            Value::Object(map) => self.walk_object(map, key),
            Value::Array(items) => {
                for (position, item) in items.iter().enumerate() {
                    self.push(&position.to_string());
                    self.walk(item, key);
                    self.pop();
                }
            }
            Value::String(text) => self.scalar(text, key),
            _ => {}
        }
    }

    fn walk_object(&mut self, map: &Map<String, Value>, parent_key: Option<&str>) {
        if self.unquoted_template(map) {
            return;
        }
        let kind =
            if parent_key == Some("properties") && map.get("path").is_some_and(Value::is_string) {
                MapKind::Op
            } else if parent_key == Some("where") {
                MapKind::Where
            } else {
                MapKind::Other
            };
        if kind == MapKind::Op {
            let frame = self.op_frame(map);
            self.ops.push(frame);
        }
        self.maps.push(kind);
        for (name, child) in map {
            if crate::conditions::HOLDER_KEYS.contains(&name.as_str()) {
                continue;
            }
            self.push(name);
            self.key_name(name);
            let entering_value = kind == MapKind::Op && name == "value";
            if entering_value {
                self.value_scopes.push(ValueScope {
                    depth: self.segments.len(),
                });
            }
            self.walk(child, Some(name));
            if entering_value {
                self.value_scopes.pop();
            }
            self.pop();
        }
        self.maps.pop();
        if kind == MapKind::Op {
            self.ops.pop();
        }
    }

    /// Map keys are never resolved: warn when one looks like a reference or template.
    fn key_name(&mut self, name: &str) {
        let looks_typed = typed_ref_parts(name).is_some();
        let has_template =
            name.contains('{') && !template::scan(name, ScanPhase::All).tokens().is_empty();
        if looks_typed || has_template {
            let range = self.scalar_range();
            self.report(
                Severity::Warning,
                "config.ref-not-resolved",
                range,
                format!("map key {name:?} is never resolved; references and templates only work in values"),
            );
        }
    }

    /// `name: {cfg:a}` parses as a flow mapping; report it and skip the pseudo mapping.
    fn unquoted_template(&mut self, map: &Map<String, Value>) -> bool {
        if map.len() != 1 {
            return false;
        }
        let (name, value) = map.iter().next().expect("one entry");
        if !value.is_null() {
            return false;
        }
        let Some((prefix, _)) = name.split_once(':') else {
            return false;
        };
        if template::phase_of(prefix).is_none() {
            return false;
        }
        let starts_with_brace = self
            .env
            .source
            .and_then(|source| source.node_first_char(self.env.index, &self.pointer))
            == Some('{');
        if !starts_with_brace {
            return false;
        }
        let range = self.scalar_range();
        self.report(
            Severity::Error,
            "template.unquoted-start",
            range,
            "a plain scalar starting with '{' is parsed as a flow mapping; wrap the value in quotes",
        );
        true
    }

    // ---- operations -----------------------------------------------------------------------

    /// Class a property path of the enclosing entry refers to, and the `applyToSpawnedActors` flag.
    fn entry_class(&self) -> (Option<String>, bool) {
        let (Some(key), Some(position)) = (self.segments.first(), self.segments.get(1)) else {
            return (None, false);
        };
        let Some(entry) = self
            .root
            .get(key.as_str())
            .and_then(Value::as_array)
            .zip(position.parse::<usize>().ok())
            .and_then(|(entries, position)| entries.get(position))
            .and_then(Value::as_object)
        else {
            return (None, false);
        };
        let apply_spawned = entry
            .get("applyToSpawnedActors")
            .and_then(runtime_bool)
            .unwrap_or(false);
        let names: &[&str] = if self.env.kind == "cdo" && key == "patches" {
            &["target", "ofClass", "allAssetsOfClass"]
        } else {
            &["parent", "class"]
        };
        for name in names {
            if let Some(class) = entry
                .get(*name)
                .and_then(Value::as_str)
                .and_then(|raw| self.class_of(raw))
            {
                return (Some(class), apply_spawned);
            }
        }
        let default = slots::default_base_class(key).map(str::to_owned);
        (default, apply_spawned)
    }

    /// Resolves a class reference through workspace-generated classes and the reflection data.
    fn class_of(&self, raw: &str) -> Option<String> {
        let reflection = self.env.reflection()?;
        let mut current = raw.to_owned();
        for _ in 0..MAX_CLASS_HOPS {
            if let Some(path) = reflection.class_path(&current) {
                return Some(path.to_owned());
            }
            let generated = self
                .env
                .config
                .zip(self.env.pack_ref)
                .and_then(|(index, pack)| index.generated_class(pack, &current))?;
            current = generated.parent.clone()?;
        }
        None
    }

    fn op_frame(&mut self, map: &Map<String, Value>) -> OpFrame {
        let op = map
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or("set")
            .to_ascii_lowercase();
        let path = map.get("path").and_then(Value::as_str).unwrap_or_default();
        let mut frame = OpFrame {
            op,
            property: crate::tag_container_selector_property(path),
            cold: None,
        };
        if frame.property.is_some() {
            return frame;
        }
        let Some(reflection) = self.env.reflection() else {
            return frame;
        };
        let (class, mut apply_spawned) = self.entry_class();
        // Only `cdo` patches change class defaults of classes that may already have actors;
        // generated classes (items, buildings, ...) are created from these values.
        apply_spawned |= self.env.kind != "cdo";
        let Some(class) = class else {
            return frame;
        };
        frame.property = reflection.resolve_property_path(&class, path);
        let (reload, reason) = reflection.property_reload_with(&class, path, apply_spawned);
        if reload.is_not_live() {
            frame.cold = Some((reload, reason.clone()));
            if self.env.options.cold_properties != ColdPropertyLevel::Off {
                self.push("path");
                let range = self.scalar_range();
                let advisory = match self.env.options.cold_properties {
                    ColdPropertyLevel::Hint => Advisory::Hint,
                    _ => Advisory::Information,
                };
                let diagnostic = self
                    .reporter
                    .diagnostic(
                        self.env.index,
                        &self.pointer,
                        Severity::Warning,
                        "property.cold",
                        range,
                        format!("{path} is {}: {reason}", reload.as_str()),
                    )
                    .with_advisory(advisory);
                self.out.push(diagnostic);
                self.pop();
            }
        }
        frame
    }

    /// Slot the current scalar fills.
    fn slot_here(&self, key: Option<&str>) -> Slot {
        if let Some(expected) = self.conflicts.get(self.pointer.as_str()) {
            return slot_from_json_types(expected);
        }
        match (self.maps.last(), key) {
            (Some(MapKind::Op), Some(operand @ ("min" | "max" | "index" | "with"))) => {
                let frame = self.ops.last();
                expected_slot(
                    frame.map(|frame| frame.op.as_str()),
                    operand,
                    frame.and_then(|frame| frame.property.as_ref()),
                )
            }
            (Some(MapKind::Where), Some(operand)) => expected_slot(None, operand, None),
            _ => {
                let (Some(scope), Some(frame)) = (self.value_scopes.last(), self.ops.last()) else {
                    return Slot::Unknown;
                };
                let relative = &self.segments[scope.depth.min(self.segments.len())..];
                self.value_slot(frame, relative)
            }
        }
    }

    fn value_slot(&self, frame: &OpFrame, relative: &[String]) -> Slot {
        let property = frame.property.as_ref();
        if relative.is_empty() {
            return expected_slot(Some(&frame.op), "value", property);
        }
        let reflection = self.env.reflection();
        let (Some(property), Some(reflection)) = (property, reflection) else {
            return Slot::Unknown;
        };
        let mut current = match frame.op.as_str() {
            "append" | "prepend" | "insert" | "replace" | "remove" => {
                match property.inner.as_deref() {
                    Some(inner) => inner.clone(),
                    None => return Slot::Unknown,
                }
            }
            "set" => property.clone(),
            _ => return Slot::Unknown,
        };
        for segment in relative {
            current = match current.kind {
                PropertyKind::Array | PropertyKind::Set if segment.parse::<usize>().is_ok() => {
                    match current.inner.as_deref() {
                        Some(inner) => inner.clone(),
                        None => return Slot::Unknown,
                    }
                }
                PropertyKind::Map => match current.value.as_deref() {
                    Some(value) => value.clone(),
                    None => return Slot::Unknown,
                },
                PropertyKind::Struct => {
                    match current
                        .struct_path
                        .as_deref()
                        .and_then(|path| reflection.find_struct_field(path, segment))
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

    // ---- scalars ---------------------------------------------------------------------------

    fn scalar_info(&self) -> Option<ScalarInfo> {
        self.env
            .source
            .and_then(|source| source.scalar_info(self.env.index, &self.pointer))
    }

    fn scalar(&mut self, text: &str, key: Option<&str>) {
        let typed = typed_ref_parts(text);
        let may_template = text.contains('{');
        if typed.is_none() && !may_template {
            return;
        }
        let info = self.scalar_info();
        if key.is_some_and(|key| UNRESOLVED_KEYS.contains(&key)) {
            let plain = info.is_none_or(|info| info.style.is_plain());
            let has_template =
                may_template && !template::scan(text, ScanPhase::All).tokens().is_empty();
            if (typed.is_some() && plain) || has_template {
                let range = self.scalar_range();
                self.report(
                    Severity::Warning,
                    "config.ref-not-resolved",
                    range,
                    format!(
                        "values of {:?} are never resolved; references and templates are written literally here",
                        key.unwrap_or_default()
                    ),
                );
            }
            return;
        }
        if let Some((prefix, rest)) = typed {
            if info.is_none_or(|info| info.style.is_plain()) {
                self.typed_reference(prefix, rest, key);
                return;
            }
        }
        if may_template {
            self.templates(text, key, info);
        }
    }

    fn typed_reference(&mut self, prefix: &str, rest: &str, key: Option<&str>) {
        let range = self.scalar_range();
        let target = match parse_target(prefix, rest) {
            Ok(target) => target,
            Err(message) => {
                self.report(Severity::Error, "template.malformed", range, message);
                return;
            }
        };
        let slot = self.slot_here(key);
        self.use_target(&target, range, Some(slot));
    }

    /// Resolution, hot/cold and compatibility of one reference occurrence.
    fn use_target(&mut self, target: &RefTarget, range: Range, slot: Option<Slot>) {
        let (resolved, problem) = resolve_target(self.env, target);
        if let Some(problem) = problem {
            self.report(problem.severity, problem.code, range, problem.message);
        }
        let Some(resolved) = resolved else {
            return;
        };
        if resolved.hot {
            if self.cold_context() {
                let name = resolved
                    .field
                    .map(|field| field.id.as_str())
                    .unwrap_or_default();
                self.report(
                    Severity::Warning,
                    "config.hot-in-cold-stage",
                    range,
                    format!(
                        "hot config {name:?} is used where only load-time values apply; changing it requires a restart"
                    ),
                );
            }
            if let Some((reload, reason)) = self.ops.last().and_then(|frame| frame.cold.clone()) {
                let name = resolved
                    .field
                    .map(|field| field.id.as_str())
                    .unwrap_or_default();
                self.report(
                    Severity::Warning,
                    "config.hot-into-cold-property",
                    range,
                    format!(
                        "hot config {name:?} feeds a {} property: {reason}",
                        reload.as_str()
                    ),
                );
            }
        }
        if let (Some(slot), Some(shape)) = (slot, resolved.shape) {
            let setting = matches!(target, RefTarget::Setting { .. });
            let code = if setting {
                "setting.type-mismatch"
            } else {
                "config.type-mismatch"
            };
            match compat(&shape, &slot) {
                Compat::Incompatible => self.report(
                    if setting {
                        Severity::Warning
                    } else {
                        Severity::Error
                    },
                    code,
                    range,
                    format!(
                        "{} cannot be used where {} is expected",
                        shape_label(&shape),
                        slot.label()
                    ),
                ),
                Compat::Lossy => self.report(
                    Severity::Warning,
                    code,
                    range,
                    format!(
                        "{} may not fit {} for every value",
                        shape_label(&shape),
                        slot.label()
                    ),
                ),
                Compat::Exact | Compat::Convertible => {}
            }
        }
    }

    // ---- templates -------------------------------------------------------------------------

    fn templates(&mut self, text: &str, key: Option<&str>, info: Option<ScalarInfo>) {
        let scan = template::scan(text, ScanPhase::All);
        let units = text.encode_utf16().count() as u32;
        let range_of = |checker: &Self, span: Span| {
            info.and_then(|info| info.value_range(units, span.start, span.end))
                .unwrap_or_else(|| checker.scalar_range())
        };
        for scan_issue in &scan.issues {
            let range = range_of(self, scan_issue.span);
            match &scan_issue.kind {
                ScanIssueKind::Unbalanced => self.report(
                    Severity::Error,
                    "template.unbalanced",
                    range,
                    "template token has no closing '}'; the rest of the string stays literal",
                ),
                ScanIssueKind::NestingDepth => self.report(
                    Severity::Error,
                    "template.nesting-depth",
                    range,
                    format!("templates can be nested {} levels deep at most", template::MAX_DEPTH),
                ),
                ScanIssueKind::UnknownPrefix { prefix, suggestion } => self.report(
                    Severity::Warning,
                    "template.unknown-provider",
                    range,
                    format!("unknown template prefix {prefix:?}; did you mean {suggestion:?}? (other providers stay literal)"),
                ),
            }
        }
        let whole = scan.segments.len() == 1
            && matches!(scan.segments.first(), Some(TemplateSegment::Token(_)));
        let slot = if whole {
            Some(self.slot_here(key))
        } else {
            None
        };
        for segment in &scan.segments {
            if let TemplateSegment::Token(token) = segment {
                token.walk(&mut |item| {
                    let range = range_of(self, item.span);
                    let slot = if std::ptr::eq(item, token) {
                        slot.clone()
                    } else {
                        None
                    };
                    self.token(item, range, slot);
                });
            }
        }
    }

    fn token(&mut self, token: &TemplateToken, range: Range, slot: Option<Slot>) {
        match template::phase_of(&token.prefix) {
            Some(Phase::PrePass) => {
                if !token.children.is_empty() {
                    return;
                }
                match parse_target(&token.prefix, &token.body) {
                    Ok(target) => self.use_target(&target, range, None),
                    Err(message) => {
                        self.report(Severity::Error, "template.malformed", range, message)
                    }
                }
            }
            Some(Phase::Apply) => self.apply_token(token, range, slot),
            None => {}
        }
    }

    fn apply_token(&mut self, token: &TemplateToken, range: Range, slot: Option<Slot>) {
        let (target_text, member_text) = split_masked(token);
        let (Some(target_text), Some(member_text)) = (target_text, member_text) else {
            self.report(
                Severity::Error,
                "template.malformed",
                range,
                format!(
                    "{{{}:...}} needs a target and a member: {{{}:<target>:<member>}}",
                    token.prefix, token.prefix
                ),
            );
            return;
        };
        if target_text.trim().is_empty() {
            self.report(
                Severity::Error,
                "template.malformed",
                range,
                "the template target is empty",
            );
            return;
        }
        let member = match template::parse_member(&member_text) {
            Ok(member) => member,
            Err(error) => {
                let code = if error.kind == MemberErrorKind::Unbalanced {
                    "template.unbalanced"
                } else {
                    "template.malformed"
                };
                self.report(
                    Severity::Error,
                    code,
                    range,
                    format!("invalid member: {}", error.message),
                );
                return;
            }
        };
        let Some(reflection) = self.env.reflection() else {
            return;
        };
        if target_text.contains('{') {
            return;
        }
        if template::is_object_target(&token.prefix) {
            self.check_object_target(reflection, token, &target_text, &member, range, slot);
            return;
        }
        let Some(class) = self.class_of(&target_text) else {
            // Workspace-generated ids and classes outside the snapshot stay unchecked but are reported.
            if self
                .env
                .config
                .zip(self.env.pack_ref)
                .is_some_and(|(index, pack)| index.generated_class(pack, &target_text).is_some())
            {
                return;
            }
            self.report(
                Severity::Warning,
                "datapack.unknown-class",
                range,
                format!("class {target_text:?} is absent from bound reflection snapshot; runtime may still load it"),
            );
            return;
        };
        if let Some(base) = template::base_class(&token.prefix) {
            if !reflection.is_subclass_of(&class, base) {
                self.report(
                    Severity::Error,
                    "template.class-family",
                    range,
                    format!(
                        "{class} is not a {} class, which {{{}:...}} requires",
                        base.rsplit('.').next().unwrap_or(base),
                        token.prefix
                    ),
                );
                return;
            }
        }
        self.check_member(reflection, &class, TargetKind::Class, &member, range, slot);
    }

    /// `{dataasset:path:member}` and `{asset:path:member}`: the instance's class comes from the
    /// asset index; assets the index does not list stay unchecked.
    fn check_object_target(
        &mut self,
        reflection: &ReflectionIndex,
        token: &TemplateToken,
        target_text: &str,
        member: &Member,
        range: Range,
        slot: Option<Slot>,
    ) {
        let Some(asset) = reflection.find_asset(target_text) else {
            return;
        };
        let Some(class) = reflection.class_path(&asset.asset_class).map(str::to_owned) else {
            return;
        };
        if let Some(base) = template::base_class(&token.prefix) {
            if !reflection.is_subclass_of(&class, base) {
                self.report(
                    Severity::Error,
                    "template.class-family",
                    range,
                    format!(
                        "{target_text} is a {class}, not a {} object, which {{{}:...}} requires",
                        base.rsplit('.').next().unwrap_or(base),
                        token.prefix
                    ),
                );
                return;
            }
        }
        self.check_member(reflection, &class, TargetKind::Object, member, range, slot);
    }

    fn check_member(
        &mut self,
        reflection: &ReflectionIndex,
        class: &str,
        target: TargetKind,
        member: &Member,
        range: Range,
        slot: Option<Slot>,
    ) {
        match member {
            Member::Ident { name, .. } => {
                if let Some(property) = reflection.find_property(class, name) {
                    self.whole_token_slot(&property, range, slot);
                    return;
                }
                match reflection.find_function(class, name) {
                    Some(function) => {
                        self.check_call(reflection, class, target, &function, &[], range, slot)
                    }
                    None if !reflection.has_function_data() => {}
                    None => self.report(
                        Severity::Warning,
                        "template.unknown-member",
                        range,
                        format!("{name:?} is neither a property nor a function of {class}"),
                    ),
                }
            }
            Member::Path { path, .. } => match path_status(reflection, class, path) {
                PathStatus::Resolved(property) => self.whole_token_slot(&property, range, slot),
                PathStatus::Unknown(segment) => self.report(
                    Severity::Warning,
                    "template.unknown-member",
                    range,
                    format!("{segment:?} is not a property in {path:?} of {class}"),
                ),
                PathStatus::Unverifiable => {}
            },
            Member::Call { name, args, .. } => match reflection.find_function(class, name) {
                Some(function) => {
                    self.check_call(reflection, class, target, &function, args, range, slot)
                }
                None if !reflection.has_function_data() => {}
                None => self.report(
                    Severity::Warning,
                    "template.unknown-member",
                    range,
                    format!("{name:?} is not a function of {class}"),
                ),
            },
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "call checks need the reflection data, target and diagnostic context"
    )]
    fn check_call(
        &mut self,
        reflection: &ReflectionIndex,
        class: &str,
        target: TargetKind,
        function: &FunctionInfo,
        args: &[template::Arg],
        range: Range,
        slot: Option<Slot>,
    ) {
        match reflection.callable(function, target, Some(class)) {
            Callable::No(reason) => self.report(
                Severity::Error,
                "template.member-not-callable",
                range,
                format!(
                    "{} cannot be called from a template: {reason}",
                    function.display_name()
                ),
            ),
            Callable::Yes {
                implicit,
                explicit_args,
                ..
            } => {
                if args.len() != explicit_args {
                    self.report(
                        Severity::Error,
                        "template.argument-count",
                        range,
                        format!(
                            "{} takes {explicit_args} argument(s), {} given: {}",
                            function.display_name(),
                            args.len(),
                            reflection.signature(function, implicit)
                        ),
                    );
                    return;
                }
                for (param, arg) in explicit_params(function, implicit).into_iter().zip(args) {
                    if let Some(problem) = arg_problem(reflection, &param.property, arg) {
                        self.report(
                            Severity::Error,
                            "template.argument-type",
                            range,
                            format!(
                                "argument {:?} of {}: {problem}",
                                param.property.display_name(),
                                function.display_name()
                            ),
                        );
                    }
                }
                if let Some(ret) = &function.ret {
                    self.whole_token_slot(ret, range, slot);
                }
            }
        }
    }

    /// A scalar that is exactly one apply-time token takes the member's type.
    fn whole_token_slot(&mut self, member: &PropertyInfo, range: Range, slot: Option<Slot>) {
        let Some(slot) = slot else {
            return;
        };
        let shape = shape_of_property(member);
        match compat(&shape, &slot) {
            Compat::Incompatible => self.report(
                Severity::Error,
                "config.type-mismatch",
                range,
                format!(
                    "template value of type {} cannot be used where {} is expected",
                    member.cpp_type,
                    slot.label()
                ),
            ),
            Compat::Lossy => self.report(
                Severity::Warning,
                "config.type-mismatch",
                range,
                format!(
                    "template value of type {} may not fit {}",
                    member.cpp_type,
                    slot.label()
                ),
            ),
            _ => {}
        }
    }
}

fn shape_label(shape: &ValueShape) -> String {
    let base = match &shape.kind {
        ShapeKind::Bool => "boolean",
        ShapeKind::Integer => "integer number",
        ShapeKind::Float => "number",
        ShapeKind::Text(_) => "text",
        ShapeKind::SelectString(_) => "string select",
        ShapeKind::SelectNumber { .. } => "numeric select",
        ShapeKind::SelectBool => "boolean select",
        ShapeKind::Unknown => "value",
    };
    if shape.multiple {
        format!("list of {base} values")
    } else {
        format!("{base} value")
    }
}

/// Slot for the JSON types a schema `type` error expected.
fn slot_from_json_types(expected: &[String]) -> Slot {
    for kind in expected {
        match kind.as_str() {
            "integer" => return Slot::Int(slots::IntWidth::I32),
            "number" => return Slot::Number,
            "boolean" => return Slot::Bool,
            "array" => return Slot::Array(Box::new(Slot::Unknown)),
            "object" => return Slot::Struct(None),
            _ => {}
        }
    }
    Slot::Unknown
}

/// Shape of a reflected member value (for whole-token typed values).
pub(crate) fn shape_of_property(property: &PropertyInfo) -> ValueShape {
    use crate::config::ValueType;
    let (kind, multiple, element) = match property.kind {
        PropertyKind::Array | PropertyKind::Set => {
            (property.inner.as_deref().map(shape_of_property), true, true)
        }
        _ => (None, false, false),
    };
    if element {
        return match kind {
            Some(mut inner) => {
                inner.multiple = multiple;
                inner
            }
            None => ValueShape {
                kind: ShapeKind::Unknown,
                multiple: true,
                bounds: None,
                defaults: Vec::new(),
                width: None,
            },
        };
    }
    let shape_kind = match property.kind {
        PropertyKind::Bool => ShapeKind::Bool,
        PropertyKind::Int8
        | PropertyKind::Int16
        | PropertyKind::Int32
        | PropertyKind::Int64
        | PropertyKind::UInt16
        | PropertyKind::UInt32
        | PropertyKind::UInt64 => ShapeKind::Integer,
        PropertyKind::Byte if property.enum_path.is_none() => ShapeKind::Integer,
        PropertyKind::Float | PropertyKind::Double => ShapeKind::Float,
        PropertyKind::Str => ShapeKind::Text(ValueType::String),
        PropertyKind::Name => ShapeKind::Text(ValueType::Name),
        PropertyKind::Text => ShapeKind::Text(ValueType::Text),
        PropertyKind::Enum | PropertyKind::Byte => {
            ShapeKind::SelectString(property.enum_options.clone())
        }
        PropertyKind::Class | PropertyKind::SoftClass => ShapeKind::Text(ValueType::String),
        PropertyKind::Struct
            if property
                .struct_path
                .as_deref()
                .is_some_and(|path| path.ends_with(".GameplayTag")) =>
        {
            ShapeKind::Text(ValueType::Tag)
        }
        _ => ShapeKind::Unknown,
    };
    let width = match property.kind {
        PropertyKind::Int8 => Some(slots::IntWidth::I8.range()),
        PropertyKind::Int16 => Some(slots::IntWidth::I16.range()),
        PropertyKind::Int32 => Some(slots::IntWidth::I32.range()),
        PropertyKind::Int64 => Some(slots::IntWidth::I64.range()),
        PropertyKind::UInt16 => Some(slots::IntWidth::U16.range()),
        PropertyKind::UInt32 => Some(slots::IntWidth::U32.range()),
        PropertyKind::UInt64 => Some(slots::IntWidth::U64.range()),
        PropertyKind::Byte => Some(slots::IntWidth::U8.range()),
        _ => None,
    };
    ValueShape {
        kind: shape_kind,
        multiple: false,
        bounds: width,
        defaults: Vec::new(),
        width,
    }
}

/// Splits an apply-time body at the first colon outside nested tokens.
fn split_masked(token: &TemplateToken) -> (Option<String>, Option<String>) {
    let chars = token.body.chars().collect::<Vec<_>>();
    // Offsets of nested tokens are UTF-16 units relative to the whole string; rebase to the body.
    let base = token.body_span.start;
    let mut masked = vec![false; chars.len()];
    let mut offset = 0u32;
    let mut starts = Vec::with_capacity(chars.len());
    for ch in &chars {
        starts.push(offset);
        offset += ch.len_utf16() as u32;
    }
    for child in &token.children {
        let (from, to) = (
            child.span.start.saturating_sub(base),
            child.span.end.saturating_sub(base),
        );
        for (index, start) in starts.iter().enumerate() {
            if *start >= from && *start < to {
                masked[index] = true;
            }
        }
    }
    let split = chars
        .iter()
        .enumerate()
        .position(|(index, ch)| *ch == ':' && !masked[index]);
    let Some(split) = split else {
        return (None, None);
    };
    let target = chars[..split].iter().collect::<String>();
    // Nested tokens inside the member are replaced by a neutral identifier for the member parser.
    let mut member = String::new();
    let mut index = split + 1;
    while index < chars.len() {
        if masked[index] {
            member.push_str("__kdf_nested__");
            while index < chars.len() && masked[index] {
                index += 1;
            }
            continue;
        }
        member.push(chars[index]);
        index += 1;
    }
    (Some(target), Some(member))
}

pub(crate) enum PathStatus {
    Resolved(Box<PropertyInfo>),
    /// First segment that does not exist.
    Unknown(String),
    /// Data needed to follow the path is missing.
    Unverifiable,
}

/// Follows a property path and says why it stops.
pub(crate) fn path_status(reflection: &ReflectionIndex, class: &str, path: &str) -> PathStatus {
    if let Some(property) = reflection.resolve_property_path(class, path) {
        return PathStatus::Resolved(Box::new(property));
    }
    let first = path.split(['.', '[']).next().unwrap_or(path).to_owned();
    match reflection.find_property(class, &first) {
        None => PathStatus::Unknown(first),
        Some(mut current) => {
            // Walk the remaining segments to find the first one that is definitely missing.
            let rest = &path[first.len()..];
            let mut remaining = rest;
            while !remaining.is_empty() {
                if let Some(after) = remaining.strip_prefix('[') {
                    let Some(close) = after.find(']') else {
                        return PathStatus::Unverifiable;
                    };
                    current = match current.kind {
                        PropertyKind::Array | PropertyKind::Set => match current.inner.as_deref() {
                            Some(inner) => inner.clone(),
                            None => return PathStatus::Unverifiable,
                        },
                        PropertyKind::Map => match current.value.as_deref() {
                            Some(value) => value.clone(),
                            None => return PathStatus::Unverifiable,
                        },
                        _ => return PathStatus::Unknown("[]".into()),
                    };
                    remaining = &after[close + 1..];
                } else if let Some(after) = remaining.strip_prefix('.') {
                    let end = after.find(['.', '[']).unwrap_or(after.len());
                    let name = &after[..end];
                    let next = match current.kind {
                        PropertyKind::Struct => {
                            let Some(path) = current.struct_path.as_deref() else {
                                return PathStatus::Unverifiable;
                            };
                            if reflection.struct_properties(path).is_none() {
                                return PathStatus::Unverifiable;
                            }
                            reflection.find_struct_field(path, name)
                        }
                        kind if kind.is_object_reference() => {
                            let Some(class) = current.class_path.as_deref() else {
                                return PathStatus::Unverifiable;
                            };
                            if reflection.class_path(class).is_none() {
                                return PathStatus::Unverifiable;
                            }
                            reflection.find_property(class, name)
                        }
                        _ => return PathStatus::Unverifiable,
                    };
                    match next {
                        Some(property) => current = property,
                        None => return PathStatus::Unknown(name.to_owned()),
                    }
                    remaining = &after[end..];
                } else {
                    return PathStatus::Unverifiable;
                }
            }
            PathStatus::Resolved(Box::new(current))
        }
    }
}

/// Why `arg` cannot convert to `param`, or `None` when it can.
fn arg_problem(
    reflection: &ReflectionIndex,
    param: &PropertyInfo,
    arg: &template::Arg,
) -> Option<String> {
    if matches!(&arg.kind, ArgKind::Ident(name) if name == "__kdf_nested__") {
        return None;
    }
    let mismatch = |expected: &str| {
        Some(format!(
            "expected {expected}, found {}",
            match &arg.kind {
                ArgKind::Number(_) => "a number",
                ArgKind::Str(_) => "a string",
                ArgKind::Bool(_) => "a boolean",
                ArgKind::Ident(_) => "a name",
                ArgKind::Struct { .. } => "a struct literal",
            }
        ))
    };
    match param.kind {
        PropertyKind::Bool => match &arg.kind {
            ArgKind::Bool(_) => None,
            _ => mismatch("true or false"),
        },
        PropertyKind::Int8
        | PropertyKind::Int16
        | PropertyKind::Int32
        | PropertyKind::Int64
        | PropertyKind::UInt16
        | PropertyKind::UInt32
        | PropertyKind::UInt64 => match &arg.kind {
            ArgKind::Number(number) if number.fract() == 0.0 => None,
            ArgKind::Number(_) => Some("expected a whole number".into()),
            _ => mismatch("a whole number"),
        },
        PropertyKind::Byte if param.enum_path.is_none() => match &arg.kind {
            ArgKind::Number(number) if number.fract() == 0.0 && (0.0..=255.0).contains(number) => {
                None
            }
            ArgKind::Number(_) => Some("expected a whole number from 0 to 255".into()),
            _ => mismatch("a whole number"),
        },
        PropertyKind::Float | PropertyKind::Double => match &arg.kind {
            ArgKind::Number(_) => None,
            _ => mismatch("a number"),
        },
        PropertyKind::Str | PropertyKind::Name | PropertyKind::Text => match &arg.kind {
            ArgKind::Str(_) | ArgKind::Number(_) | ArgKind::Bool(_) | ArgKind::Ident(_) => None,
            _ => mismatch("text"),
        },
        PropertyKind::Enum | PropertyKind::Byte => match &arg.kind {
            ArgKind::Number(_) => None,
            ArgKind::Ident(name) | ArgKind::Str(name) => {
                let entry = name.rsplit("::").next().unwrap_or(name);
                if param.enum_options.is_empty()
                    || param
                        .enum_options
                        .iter()
                        .any(|option| option.eq_ignore_ascii_case(entry))
                {
                    None
                } else {
                    Some(format!("{entry:?} is not an entry of the enum"))
                }
            }
            _ => mismatch("an enum entry"),
        },
        PropertyKind::Class
        | PropertyKind::SoftClass
        | PropertyKind::Object
        | PropertyKind::SoftObject => match &arg.kind {
            ArgKind::Str(path) | ArgKind::Ident(path) => {
                let meta = param.meta_class.as_deref().or(param.class_path.as_deref());
                match (meta, reflection.class_path(path)) {
                    (Some(meta), Some(found))
                        if matches!(param.kind, PropertyKind::Class | PropertyKind::SoftClass)
                            && reflection.class_path(meta).is_some()
                            && !reflection.is_subclass_of(found, meta) =>
                    {
                        Some(format!("{found} is not a {meta}"))
                    }
                    _ => None,
                }
            }
            _ => mismatch("a path"),
        },
        PropertyKind::Struct => match &arg.kind {
            ArgKind::Struct { fields, .. } => {
                let properties = param
                    .struct_path
                    .as_deref()
                    .and_then(|path| reflection.struct_properties(path))?;
                match fields {
                    StructFields::Positional(items) if items.len() > properties.len() => {
                        Some(format!(
                            "struct has {} fields, {} values given",
                            properties.len(),
                            items.len()
                        ))
                    }
                    StructFields::Named(items) => items.iter().find_map(|(name, _)| {
                        (!properties.iter().any(|field| {
                            field.name.eq_ignore_ascii_case(name)
                                || field.authored_name.eq_ignore_ascii_case(name)
                        }))
                        .then(|| format!("struct has no field {name:?}"))
                    }),
                    StructFields::Positional(_) => None,
                }
            }
            ArgKind::Str(_)
                if param
                    .struct_path
                    .as_deref()
                    .is_some_and(|path| path.ends_with(".GameplayTag")) =>
            {
                None
            }
            _ => mismatch("a struct literal"),
        },
        _ => None,
    }
}
