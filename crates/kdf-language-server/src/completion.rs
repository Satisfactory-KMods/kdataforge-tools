//! Completion for pack documents: operand references, templates, `path:` properties, categories,
//! config expressions and config documents.
//!
//! Items carry an explicit single-line `textEdit` (ids contain dots, so word-based replacement
//! would cut them), a `filterText` equal to the replaced text, a `sortText` (0 exact, 1
//! convertible, own pack first) and re-trigger suggestions after provider, pack and class items.

#[cfg(test)]
#[path = "completion_order_tests.rs"]
mod completion_order_tests;

use crate::context::{
    option_values, utf16_len, KeySlot, Place, Quote, Resolved, Scalar, ValueSlot,
};
use crate::generated::GeneratedKind;
use crate::resolve;
use crate::semantic::{
    class_properties_matching, field_expr_type, operand_context, patch_class, reload_tag,
    setting_expr_type, slot_fits, template_target, unresolved_key, Env,
};
use crate::workspace::generated_path;
use kdf_validation::config::{Apply, ConfigField, Resolution};
use kdf_validation::expression::{
    self, CmpOp, Expect, ExprKind, ExprType, Operand, OperandKind, Token, TokenKind,
};
use kdf_validation::slots::{compat, field_shape, setting_shape, Slot};
use kdf_validation::template::{self, Partial, Stage};
use kdf_validation::{
    explicit_params, field_docs, property_path_identity, Callable, Implicit, MemberKind,
    PropertyInfo, SettingRecord, CANONICAL_TYPES, OPERATIONS,
};
use serde_json::Value;
use tower_lsp::lsp_types::{
    Command, CompletionItem, CompletionItemKind, CompletionList, CompletionResponse,
    CompletionTextEdit, InsertTextFormat, Range, TextEdit,
};

/// Items returned for one list before the list is marked incomplete.
pub(crate) const LIMIT: usize = 200;

const SUGGEST: (&str, &str) = ("Suggest", "editor.action.triggerSuggest");
const HINTS: (&str, &str) = ("Parameter hints", "editor.action.triggerParameterHints");

#[derive(Default)]
pub(crate) struct Completions {
    pub(crate) items: Vec<CompletionItem>,
    pub(crate) incomplete: bool,
}

impl Completions {
    pub(crate) fn into_response(self) -> CompletionResponse {
        CompletionResponse::List(CompletionList {
            is_incomplete: self.incomplete,
            items: self.items,
        })
    }
}

/// Documentation of an item: a short text, or a card built on demand from `data` (see `resolve`).
enum Doc {
    None,
    Text(String),
    Lazy(Value),
}

/// One item before it is turned into the protocol type.
struct Entry {
    label: String,
    kind: CompletionItemKind,
    detail: Option<String>,
    doc: Doc,
    range: Range,
    text: String,
    snippet: Option<String>,
    sort: String,
    command: Option<(&'static str, &'static str)>,
}

impl Entry {
    fn new(
        label: impl Into<String>,
        kind: CompletionItemKind,
        range: Range,
        text: impl Into<String>,
    ) -> Self {
        let label = label.into();
        let text = text.into();
        Self {
            sort: format!("1{}", label.to_lowercase()),
            label,
            kind,
            detail: None,
            doc: Doc::None,
            range,
            text,
            snippet: None,
            command: None,
        }
    }
    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
    fn doc(mut self, doc: impl Into<String>) -> Self {
        let doc = doc.into();
        if !doc.is_empty() {
            self.doc = Doc::Text(doc);
        }
        self
    }
    /// Card built when the client asks for it (inline for clients that cannot resolve).
    fn lazy(mut self, data: Value) -> Self {
        self.doc = Doc::Lazy(data);
        self
    }
    fn sort(mut self, rank: u8, group: u8) -> Self {
        self.sort = format!("{rank}{group}{}", self.label.to_lowercase());
        self
    }
    fn suggest(mut self) -> Self {
        self.command = Some(SUGGEST);
        self
    }
    fn hints(mut self) -> Self {
        self.command = Some(HINTS);
        self
    }
    fn snippet(mut self, snippet: impl Into<String>) -> Self {
        self.snippet = Some(snippet.into());
        self
    }
    fn finish(self, env: &Env<'_>) -> CompletionItem {
        let use_snippet = env.caps.snippets && self.snippet.is_some();
        let new_text = if use_snippet {
            self.snippet.clone().unwrap_or_default()
        } else {
            self.text.clone()
        };
        let (documentation, data) = match self.doc {
            Doc::None => (None, None),
            Doc::Text(text) => (Some(resolve::documentation(text, env.caps)), None),
            Doc::Lazy(data) if env.caps.resolve_documentation => (None, Some(data)),
            Doc::Lazy(data) => (
                resolve::card(&data, env.reflection, env.workspace)
                    .map(|text| resolve::documentation(text, env.caps)),
                None,
            ),
        };
        CompletionItem {
            label: self.label,
            kind: Some(self.kind),
            detail: self.detail,
            documentation,
            data,
            sort_text: Some(self.sort),
            filter_text: Some(self.text),
            insert_text: Some(new_text.clone()),
            insert_text_format: use_snippet.then_some(InsertTextFormat::SNIPPET),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: self.range,
                new_text,
            })),
            // The commands belong to Monaco / VS Code; other hosts switch them off
            // (`initializationOptions.kdf.clientCommands`).
            command: self
                .command
                .filter(|_| env.caps.client_commands)
                .map(|(title, command)| Command {
                    title: title.into(),
                    command: command.into(),
                    arguments: None,
                }),
            ..CompletionItem::default()
        }
    }
}

fn finish(env: &Env<'_>, entries: Vec<Entry>, incomplete: bool) -> Completions {
    Completions {
        items: entries.into_iter().map(|entry| entry.finish(env)).collect(),
        incomplete,
    }
}

/// Apply the limit after filtering, keeping entry points into packs and providers available.
fn limit_reference_entries(entries: &mut Vec<Entry>) -> bool {
    let incomplete = entries.len() > LIMIT;
    if incomplete {
        entries.sort_by_key(|entry| entry.command != Some(SUGGEST));
        entries.truncate(LIMIT);
    }
    incomplete
}

/// `typed` occurs in `candidate`, ignoring ASCII letter case (class paths and ids are ASCII).
/// Allocation free: it runs for every candidate of every list.
pub(crate) fn contains_ci(candidate: &str, typed: &str) -> bool {
    let needle = typed.as_bytes();
    if needle.is_empty() {
        return true;
    }
    candidate
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

/// The letters of `typed` appear in `name` in order, ignoring ASCII case: a superset of what the
/// editor's own fuzzy filter keeps, so narrowing a long list with it never drops a visible item.
pub(crate) fn matches_typed(name: &str, typed: &str) -> bool {
    let mut wanted = typed.bytes().map(|byte| byte.to_ascii_lowercase());
    let mut next = wanted.next();
    for byte in name.bytes() {
        match next {
            None => return true,
            Some(letter) if letter == byte.to_ascii_lowercase() => next = wanted.next(),
            Some(_) => {}
        }
    }
    next.is_none()
}

// -------------------------------------------------------------------------------------------------
// Entry point
// -------------------------------------------------------------------------------------------------

pub(crate) fn complete(env: &Env<'_>, resolved: &Resolved) -> Completions {
    match &resolved.place {
        Place::Value(slot) => value(env, resolved, slot),
        Place::Key(slot) => key(env, resolved, slot),
        Place::None => Completions::default(),
    }
}

fn whole_range(scalar: &Scalar) -> Range {
    scalar.range(0, scalar.units())
}

fn value(env: &Env<'_>, resolved: &Resolved, slot: &ValueSlot) -> Completions {
    let scalar = &slot.scalar;
    let key = resolved.key().unwrap_or("").to_owned();
    if resolved.in_conditions() {
        return if key == "config" {
            expression_items(env, scalar)
        } else {
            Completions::default()
        };
    }
    if resolved.is_config_doc() {
        if resolved.in_fields_list() && slot.list_item {
            return config_keys(env, resolved, scalar, false);
        }
        if let Some(found) = config_values(env, resolved, scalar, &key) {
            return found;
        }
    }
    if let Some(op) = resolved.op_entry() {
        match op.below.as_slice() {
            [first] if first == "path" => return path_items(env, resolved, scalar),
            [first] if first == "op" => return operation_items(env, resolved, scalar),
            _ => {}
        }
    }
    if !unresolved_key(&key) {
        if let Some(partial) = template::partial_at(&scalar.value, scalar.cursor) {
            return template_items(env, resolved, scalar, &partial);
        }
        let typed = scalar.before_cursor();
        if scalar.quote == Quote::Plain {
            if let Some((provider, rest)) = typed_reference(&typed) {
                return reference_items(env, resolved, scalar, provider, rest);
            }
            if scalar.value.is_empty() || !typed.contains(':') {
                if let Some(context) = operand_context(env, resolved) {
                    return operand_items(env, resolved, scalar, &context.slot, &typed);
                }
            }
        } else {
            let entries = apply_provider_stubs(resolved, scalar, &typed);
            if !entries.is_empty() {
                return finish(env, entries, false);
            }
        }
    }
    legacy_items(env, resolved, scalar, &key)
}

fn key(env: &Env<'_>, resolved: &Resolved, slot: &KeySlot) -> Completions {
    if resolved.is_config_doc() {
        return config_keys_for(env, resolved, slot);
    }
    if resolved.op_entry().is_some_and(|op| op.below.is_empty()) {
        return op_keys(env, resolved, slot);
    }
    if resolved.doc.kind.as_deref() == Some("category_reorder") && resolved.path().is_empty() {
        return category_reorder_keys(env, resolved, slot);
    }
    if let Some(op) = resolved.op_entry().filter(|op| op.below == ["value"]) {
        let property = patch_class(env, resolved).and_then(|patch| {
            op.key_value("path")
                .and_then(|path| env.reflection.resolve_property_path(&patch.class, path))
        });
        if property.is_some_and(|property| property.cpp_type == "FText") {
            let existing = existing_keys(resolved, slot);
            let candidates: &[&str] = if existing.contains(&"stringTable") {
                &["key"]
            } else if existing.contains(&"ns") || existing.contains(&"source") {
                &["key", "ns", "source"]
            } else {
                &["key", "stringTable", "ns", "source"]
            };
            let range = crate::context::range_on(slot.line, slot.start, slot.end.max(slot.start));
            let entries = candidates
                .iter()
                .copied()
                .filter(|name| !existing.contains(name))
                .map(|name| {
                    let documentation = field_docs::field("config", &["fields", "name", name])
                        .map(field_docs::to_markdown);
                    key_entry(name, documentation, range, slot.has_colon)
                })
                .collect();
            return finish(env, entries, false);
        }
    }
    Completions::default()
}

// -------------------------------------------------------------------------------------------------
// Legacy value lists (document type, operation, class keys)
// -------------------------------------------------------------------------------------------------

use crate::class_slots::CLASS_KEYS;

fn legacy_items(env: &Env<'_>, resolved: &Resolved, scalar: &Scalar, key: &str) -> Completions {
    let range = whole_range(scalar);
    let simple = |values: Vec<(String, String)>| {
        let entries = values
            .into_iter()
            .map(|(value, doc)| {
                Entry::new(
                    value.clone(),
                    CompletionItemKind::ENUM_MEMBER,
                    range,
                    scalar.encode(&value),
                )
                .doc(doc)
            })
            .collect();
        finish(env, entries, false)
    };
    match key {
        "type" => simple(
            CANONICAL_TYPES
                .iter()
                .map(|kind| ((*kind).to_owned(), String::new()))
                .collect(),
        ),
        "subType" if resolved.doc.kind.as_deref() == Some("category_reorder") => simple(
            ["items", "recipe", "schematic", "settings"]
                .into_iter()
                .map(|value| {
                    (
                        value.to_owned(),
                        catalog_value_documentation(resolved, value).unwrap_or_default(),
                    )
                })
                .collect(),
        ),
        "op" => operation_items(env, resolved, scalar),
        "actorAction"
            if resolved.doc.kind.as_deref() == Some("cdo")
                && resolved.path().iter().map(String::as_str).eq([
                    "patches",
                    "#",
                    "actorAction",
                ]) =>
        {
            simple(
                ["destroy", "dismantle"]
                    .into_iter()
                    .map(|value| {
                        (
                            value.to_owned(),
                            catalog_value_documentation(resolved, value).unwrap_or_default(),
                        )
                    })
                    .collect(),
            )
        }
        "classType" => simple(vec![
            ("any".into(), "Match all class kinds (default).".into()),
            (
                "blueprint".into(),
                "Require CLASS_CompiledFromBlueprint.".into(),
            ),
            (
                "native".into(),
                "Exclude CLASS_CompiledFromBlueprint.".into(),
            ),
        ]),
        "conditionBehavior" | "conditionBehaivor" => simple(vec![
            ("AND".into(), "All conditions must pass.".into()),
            ("OR".into(), "One passing condition is enough.".into()),
        ]),
        "track" => simple(vec![
            ("Default".into(), String::new()),
            ("Exploration".into(), String::new()),
        ]),
        _ if crate::class_slots::is_inventory_item_reference(resolved) => {
            class_items(env, resolved, scalar)
        }
        key if CLASS_KEYS.contains(&key) => class_items(env, resolved, scalar),
        _ => Completions::default(),
    }
}

fn operation_items(env: &Env<'_>, resolved: &Resolved, scalar: &Scalar) -> Completions {
    let range = whole_range(scalar);
    let tag_selector = resolved
        .op_entry()
        .is_some_and(|op| op.key_value("path") == Some(kdf_validation::TAG_CONTAINER_SELECTOR));
    let entries = OPERATIONS
        .iter()
        .filter(|spec| !tag_selector || kdf_validation::tag_container_operation(spec.name))
        .map(|spec| {
            Entry::new(
                spec.name,
                CompletionItemKind::ENUM_MEMBER,
                range,
                scalar.encode(spec.name),
            )
            .detail(if spec.required.is_empty() {
                format!("{}()", spec.name)
            } else {
                format!("{}({})", spec.name, spec.required.join(", "))
            })
            .doc(spec.description)
        })
        .collect();
    finish(env, entries, false)
}

/// Reflection classes filtered by what is typed (insertion order, like the exporter wrote them).
fn class_items(env: &Env<'_>, resolved: &Resolved, scalar: &Scalar) -> Completions {
    let Some(constraint) = crate::class_slots::for_completion(env.text, resolved, env.reflection)
    else {
        return Completions::default();
    };
    let typed = scalar.before_cursor();
    let typed = typed.trim();
    let range = whole_range(scalar);
    let mut entries = Vec::new();
    let mut incomplete = false;
    // Classes the pack itself generates are valid by bare id.
    if let Some(pack) = env.own_pack() {
        if !matches!(resolved.key(), Some("item" | "resource" | "producedIn")) {
            for decl in env.workspace.decls_of_pack(pack) {
                let compatible = constraint.base_class.as_deref().is_none_or(|base| {
                    env.class_in(Some(pack), &decl.id).is_some_and(|class| {
                        crate::class_slots::matches_base(env.reflection, &class, base)
                    })
                });
                if decl.kind == GeneratedKind::Class && contains_ci(&decl.id, typed) && compatible {
                    if entries.len() >= LIMIT {
                        return finish(env, entries, true);
                    }
                    entries.push(
                        Entry::new(
                            decl.id.clone(),
                            CompletionItemKind::CLASS,
                            range,
                            scalar.encode(&decl.id),
                        )
                        .detail(format!(
                            "generated by {} ({})",
                            decl.pack_ref, decl.doc_type
                        ))
                        .sort(0, 0),
                    );
                }
            }
        }
    }
    for class in &env.reflection.classes {
        if !crate::class_slots::class_matches(class, env.reflection, &constraint) {
            continue;
        }
        if !(typed.is_empty() || contains_ci(&class.path, typed) || contains_ci(&class.name, typed))
        {
            continue;
        }
        if entries.len() >= LIMIT {
            incomplete = true;
            break;
        }
        entries.push(
            Entry::new(
                class.path.clone(),
                CompletionItemKind::CLASS,
                range,
                scalar.encode(&class.path),
            )
            .detail(class.source.clone().unwrap_or_default())
            .sort(1, 0),
        );
    }
    finish(env, entries, incomplete)
}

// -------------------------------------------------------------------------------------------------
// path: properties
// -------------------------------------------------------------------------------------------------

fn path_items(env: &Env<'_>, resolved: &Resolved, scalar: &Scalar) -> Completions {
    let selector = kdf_validation::TAG_CONTAINER_SELECTOR;
    let selector_entry = || {
        Entry::new(
            selector,
            CompletionItemKind::FIELD,
            whole_range(scalar),
            if scalar.quote == Quote::Plain {
                format!("'{selector}'")
            } else {
                selector.to_owned()
            },
        )
        .detail("All top-level FGameplayTagContainer fields")
        .doc(kdf_validation::TAG_CONTAINER_SELECTOR_DOC)
    };
    let offer_selector =
        scalar.before_cursor().is_empty() || selector.starts_with(&scalar.before_cursor());
    if scalar.before_cursor().starts_with('@') {
        return finish(
            env,
            if offer_selector {
                vec![selector_entry()]
            } else {
                Vec::new()
            },
            false,
        );
    }
    let Some(patch) = patch_class(env, resolved) else {
        return finish(
            env,
            if offer_selector {
                vec![selector_entry()]
            } else {
                Vec::new()
            },
            false,
        );
    };
    let typed = scalar.before_cursor();
    // Property being typed: everything after the last `.` (or the whole text).
    let (prefix, segment) = match typed.rfind('.') {
        Some(dot) if !typed[dot..].contains('[') => (&typed[..dot], &typed[dot + 1..]),
        _ => ("", typed.as_str()),
    };
    if segment.contains(['[', ']', '\'', '"']) {
        return Completions::default();
    }
    let start = utf16_len(&typed) - utf16_len(segment);
    let range = scalar.range(start, scalar.units().max(start));
    // A `set` on a path an earlier entry patches overwrites it (the validator flags that);
    // chained operations (`multiply`, `append`, ...) still offer every path.
    let taken: Vec<String> = if resolved.op_entry().is_some_and(|op| op.op_name() == "set") {
        resolved
            .peer_paths
            .iter()
            .map(|path| property_path_identity(path))
            .collect()
    } else {
        Vec::new()
    };
    let free = |name: &str| {
        let full = if prefix.is_empty() {
            property_path_identity(name)
        } else {
            property_path_identity(&format!("{prefix}.{name}"))
        };
        !taken.contains(&full)
    };
    // What is typed narrows the list before anything is built for it.
    let keep = |name: &str| (segment.is_empty() || matches_typed(name, segment)) && free(name);
    let top_level = prefix.is_empty();
    let (properties, owner, in_struct): (Vec<PropertyInfo>, String, bool) = if top_level {
        (
            class_properties_matching(env.reflection, &patch.class, &keep),
            patch.class.clone(),
            false,
        )
    } else {
        let Some(parent) = env.reflection.resolve_property_path(&patch.class, prefix) else {
            return Completions::default();
        };
        match parent.kind {
            kdf_validation::PropertyKind::Struct => {
                let Some(path) = parent.struct_path.as_deref() else {
                    return Completions::default();
                };
                (
                    env.reflection
                        .struct_properties(path)
                        .map(|fields| {
                            fields
                                .iter()
                                .filter(|field| keep(field.display_name()))
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default(),
                    path.to_owned(),
                    true,
                )
            }
            kind if kind.is_object_reference() => {
                let Some(class) = parent.class_path.as_deref() else {
                    return Completions::default();
                };
                (
                    class_properties_matching(env.reflection, class, &keep),
                    class.to_owned(),
                    false,
                )
            }
            _ => return Completions::default(),
        }
    };
    // The class-level work of the reload classification is done once, not per property.
    let classifier = top_level.then(|| {
        env.reflection
            .reload_classifier(&patch.class, patch.apply_to_spawned)
    });
    let mut ranked: Vec<(u8, String, PropertyInfo)> = properties
        .into_iter()
        .map(|property| {
            let tag = classifier
                .as_ref()
                .and_then(|classifier| reload_tag(classifier.classify(&property).0));
            (
                u8::from(tag.is_some()),
                property.display_name().to_lowercase(),
                property,
            )
        })
        .collect();
    // A list that fits keeps its natural order; a longer one keeps the best `LIMIT` by the order the
    // client would show them in (the editor re-asks while typing, see `isIncomplete`).
    let property_limit = LIMIT - usize::from(top_level && offer_selector);
    let incomplete = ranked.len() > property_limit;
    if incomplete {
        ranked.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        ranked.truncate(property_limit);
    }
    let mut entries = Vec::with_capacity(ranked.len());
    if top_level && offer_selector {
        entries.push(selector_entry());
    }
    for (rank, _, property) in ranked {
        let name = property.display_name().to_owned();
        let mut detail = kdf_validation::display_type(&property);
        if let Some(tag) = classifier
            .as_ref()
            .and_then(|classifier| reload_tag(classifier.classify(&property).0))
        {
            detail.push_str(&format!(" · {tag}"));
        }
        let data = if top_level {
            resolve::path_property_data(&patch.class, &name, patch.apply_to_spawned)
        } else {
            resolve::nested_property_data(&owner, in_struct, &name)
        };
        entries.push(
            Entry::new(name.clone(), CompletionItemKind::FIELD, range, name)
                .detail(detail)
                .lazy(data)
                .sort(rank, 0),
        );
    }
    finish(env, entries, incomplete)
}

// -------------------------------------------------------------------------------------------------
// Typed references (`cfg:`, `session:`, ...) and operand stubs
// -------------------------------------------------------------------------------------------------

fn typed_reference(typed: &str) -> Option<(&str, &str)> {
    let (provider, rest) = typed.split_once(':')?;
    template::PREPASS_PREFIXES
        .contains(&provider)
        .then_some((provider, rest))
}

fn field_entry_detail(field: &ConfigField) -> String {
    let mut detail = format!("{} · {}", field.kind.as_str(), field.value_type.as_str());
    if field.multiple {
        detail.push_str(" · list");
    }
    let apply = if field.apply == Apply::Hot {
        "hot"
    } else {
        "cold"
    };
    detail.push_str(&format!(" · {apply}"));
    detail
}

fn setting_entry_detail(setting: &SettingRecord) -> String {
    let mut detail = format!(
        "{} · {}",
        setting.provider.as_str(),
        setting.value_type.as_str()
    );
    if let Some(placement) = setting.placements.first() {
        detail.push_str(" · ");
        detail.push_str(&placement.display_path());
    }
    detail
}

/// `cfg:`/`session:`/`ags:`/`option:` followed by the id being typed.
fn reference_items(
    env: &Env<'_>,
    resolved: &Resolved,
    scalar: &Scalar,
    provider: &str,
    rest: &str,
) -> Completions {
    let slot = operand_context(env, resolved)
        .map(|context| context.slot)
        .unwrap_or(Slot::Unknown);
    let typed_units = utf16_len(&scalar.before_cursor());
    let mut entries = Vec::new();
    let incomplete = if provider == "cfg" {
        let (pack, segment) = match rest.rsplit_once(':') {
            Some((pack, segment)) => (Some(pack), segment),
            None => (None, rest),
        };
        let start = typed_units - utf16_len(segment);
        let range = scalar.range(start, scalar.units().max(start));
        let own = env.own_pack().map(str::to_owned);
        config_entries(
            env,
            &mut entries,
            pack,
            own.as_deref(),
            &slot,
            range,
            "",
            segment,
            &|_| true,
        );
        if pack.is_none() {
            pack_stubs(env, &mut entries, own.as_deref(), range, segment);
        }
        limit_reference_entries(&mut entries)
    } else {
        let start = typed_units - utf16_len(rest);
        let range = scalar.range(start, scalar.units().max(start));
        setting_entries(env, &mut entries, provider, rest, &slot, range, &|_| true)
    };
    finish(env, entries, incomplete)
}

/// Config fields of `pack` (own pack when `None`) that fit `slot`, pass `keep` and contain
/// `typed`. Label and text both start with `text_prefix`.
#[allow(clippy::too_many_arguments)]
fn config_entries(
    env: &Env<'_>,
    entries: &mut Vec<Entry>,
    pack: Option<&str>,
    own: Option<&str>,
    slot: &Slot,
    range: Range,
    text_prefix: &str,
    typed: &str,
    keep: &dyn Fn(&ConfigField) -> bool,
) {
    let packs: Vec<&str> = match pack {
        Some(pack) => vec![pack],
        None => own.into_iter().collect(),
    };
    for pack_ref in packs {
        let Some(pack) = env.workspace.config.pack(pack_ref) else {
            continue;
        };
        for field in &pack.fields {
            let verdict = compat(&field_shape(field), slot);
            if !verdict.offered() || !keep(field) {
                continue;
            }
            let text = format!("{text_prefix}{}", field.id);
            if !contains_ci(&text, typed) {
                continue;
            }
            let own_rank = u8::from(Some(pack_ref) != own);
            entries.push(
                Entry::new(text.clone(), CompletionItemKind::VARIABLE, range, text)
                    .detail(field_entry_detail(field))
                    .lazy(resolve::config_data(field))
                    .sort(verdict.rank(), own_rank),
            );
        }
    }
}

/// `Pack:` stubs for the other packs (selecting one opens its ids).
fn pack_stubs(
    env: &Env<'_>,
    entries: &mut Vec<Entry>,
    own: Option<&str>,
    range: Range,
    typed: &str,
) {
    for pack in env.workspace.config.packs() {
        if Some(pack.pack_ref.as_str()) == own || pack.fields.is_empty() {
            continue;
        }
        let text = format!("{}:", pack.pack_ref);
        if !contains_ci(&text, typed) {
            continue;
        }
        entries.push(
            Entry::new(text.clone(), CompletionItemKind::MODULE, range, text)
                .detail(format!(
                    "pack {} ({} config field{})",
                    pack.pack_ref,
                    pack.fields.len(),
                    if pack.fields.len() == 1 { "" } else { "s" }
                ))
                .sort(2, 0)
                .suggest(),
        );
    }
}

/// Settings of one provider; returns whether the list was cut at [`LIMIT`].
fn setting_entries(
    env: &Env<'_>,
    entries: &mut Vec<Entry>,
    provider: &str,
    typed: &str,
    slot: &Slot,
    range: Range,
    keep: &dyn Fn(&SettingRecord) -> bool,
) -> bool {
    let Some(settings) = env.reflection.settings() else {
        return false;
    };
    let mut cut = false;
    for setting in settings.of_provider(provider) {
        if !(contains_ci(&setting.id, typed) || contains_ci(&setting.display_name, typed)) {
            continue;
        }
        let verdict = compat(&setting_shape(setting), slot);
        if !verdict.offered() || !keep(setting) {
            continue;
        }
        if entries.len() >= LIMIT {
            cut = true;
            break;
        }
        entries.push(
            Entry::new(
                setting.id.clone(),
                CompletionItemKind::VARIABLE,
                range,
                setting.id.clone(),
            )
            .detail(setting_entry_detail(setting))
            .lazy(resolve::setting_data(setting))
            .sort(verdict.rank(), 0),
        );
    }
    cut
}

/// Operand position without a provider yet: `cfg:<id>` of every compatible field plus provider stubs.
fn operand_items(
    env: &Env<'_>,
    resolved: &Resolved,
    scalar: &Scalar,
    slot: &Slot,
    typed: &str,
) -> Completions {
    let range = whole_range(scalar);
    let own = env.own_pack().map(str::to_owned);
    // Keep provider entry points even when config fields fill the completion limit.
    let mut entries = apply_provider_stubs(resolved, scalar, typed);
    for provider in template::PREPASS_PREFIXES {
        if !typed.is_empty() && !provider.starts_with(&typed.to_lowercase()) {
            continue;
        }
        let text = format!("{provider}:");
        entries.push(
            Entry::new(text.clone(), CompletionItemKind::KEYWORD, range, text)
                .detail(template::describe(provider))
                .sort(3, 0)
                .suggest(),
        );
    }
    config_entries(
        env,
        &mut entries,
        None,
        own.as_deref(),
        slot,
        range,
        "cfg:",
        typed,
        &|_| true,
    );
    for pack in env.workspace.config.packs() {
        if Some(pack.pack_ref.as_str()) != own.as_deref() {
            config_entries(
                env,
                &mut entries,
                Some(&pack.pack_ref),
                own.as_deref(),
                slot,
                range,
                &format!("cfg:{}:", pack.pack_ref),
                typed,
                &|_| true,
            );
        }
    }
    let incomplete = entries.len() > LIMIT;
    entries.truncate(LIMIT);
    finish(env, entries, incomplete)
}

fn apply_provider_stubs(resolved: &Resolved, scalar: &Scalar, typed: &str) -> Vec<Entry> {
    if !resolved.op_entry().is_some_and(|op| {
        op.below
            .first()
            .is_some_and(|key| matches!(key.as_str(), "value" | "min" | "max"))
    }) {
        return Vec::new();
    }
    let (open, close) = match scalar.quote {
        Quote::Plain => ("'", "'"),
        Quote::Single => ("", if scalar.closed { "" } else { "'" }),
        Quote::Double => ("", if scalar.closed { "" } else { "\"" }),
    };
    let typed = typed.to_lowercase();
    template::APPLY_PREFIXES
        .into_iter()
        .filter(|prefix| prefix.starts_with(&typed))
        .map(|prefix| {
            Entry::new(
                format!("{{{prefix}:}}"),
                CompletionItemKind::KEYWORD,
                whole_range(scalar),
                format!("{open}{{{prefix}:}}{close}"),
            )
            .snippet(format!("{open}{{{prefix}:$0}}{close}"))
            .detail(template::describe(prefix))
            .doc(provider_doc(prefix))
            .sort(3, 0)
            .suggest()
        })
        .collect()
}

// -------------------------------------------------------------------------------------------------
// Templates
// -------------------------------------------------------------------------------------------------

fn closer(partial: &Partial) -> &'static str {
    if partial.closed {
        ""
    } else {
        "}"
    }
}

pub(crate) fn provider_doc(prefix: &str) -> String {
    let catalog = kdf_validation::field_docs::prefix(&format!("{{{prefix}:"))
        .map(kdf_validation::field_docs::to_markdown);

    let phase = if template::phase_of(prefix) == Some(template::Phase::PrePass) {
        "Resolved before the stages run."
    } else {
        "Read from the game when the operation applies."
    };
    let shape = match prefix {
        "cfg" => "`{cfg:id}` or `{cfg:Pack:id}`",
        "session" | "ags" | "option" => "`{{{}:Setting.Id}}`",
        "class" => "`{class:/Script/Engine.KismetMathLibrary:Max(1, 2)}`",
        "dataasset" | "asset" => "`{dataasset:/Game/Path/DA_Name.DA_Name:Property}`",
        _ => "`{prefix:ClassPath:Property}` or `{prefix:ClassPath:Function(args)}`",
    };
    let shape = shape.replace("{{{}", &format!("{{{prefix}"));
    let base = template::base_class(prefix)
        .map(|base| format!("\n\nTarget must derive from `{base}`."))
        .unwrap_or_default();
    let summary = format!("{}\n\n{phase}\n\n{shape}{base}", template::describe(prefix));
    catalog.map_or_else(|| summary.clone(), |doc| format!("{doc}\n\n{summary}"))
}

fn template_items(
    env: &Env<'_>,
    resolved: &Resolved,
    scalar: &Scalar,
    partial: &Partial,
) -> Completions {
    let range = scalar.range(partial.replace.start, partial.replace.end);
    match partial.stage {
        Stage::Provider => {
            // An unquoted `{` at the start of a plain scalar is more likely a flow mapping.
            if scalar.quote == Quote::Plain
                && scalar.value.starts_with('{')
                && partial.outer.is_none()
            {
                return Completions::default();
            }
            let mut entries = Vec::new();
            for prefix in template::PREPASS_PREFIXES
                .iter()
                .chain(template::APPLY_PREFIXES.iter())
            {
                let text = format!("{prefix}:");
                entries.push(
                    Entry::new(*prefix, CompletionItemKind::KEYWORD, range, text)
                        .detail(template::describe(prefix))
                        .doc(provider_doc(prefix))
                        .sort(
                            u8::from(template::phase_of(prefix) == Some(template::Phase::Apply)),
                            0,
                        )
                        .suggest(),
                );
            }
            finish(env, entries, false)
        }
        Stage::Body => body_items(env, partial, range),
        Stage::Target => target_items(env, resolved, scalar, partial, range),
        Stage::Member => member_items(env, resolved, scalar, partial, range),
        Stage::Argument => argument_items(env, scalar, partial),
    }
}

fn body_items(env: &Env<'_>, partial: &Partial, range: Range) -> Completions {
    let mut entries = Vec::new();
    let incomplete;
    let end = closer(partial);
    if partial.prefix == "cfg" {
        let own = env.own_pack().map(str::to_owned);
        config_entries(
            env,
            &mut entries,
            partial.pack.as_deref(),
            own.as_deref(),
            &Slot::Unknown,
            range,
            "",
            &partial.typed,
            &|_| true,
        );
        for entry in &mut entries {
            entry.text.push_str(end);
        }
        if partial.pack.is_none() {
            pack_stubs(env, &mut entries, own.as_deref(), range, &partial.typed);
        }
        incomplete = limit_reference_entries(&mut entries);
    } else {
        incomplete = setting_entries(
            env,
            &mut entries,
            &partial.prefix,
            &partial.typed,
            &Slot::Unknown,
            range,
            &|_| true,
        );
        for entry in &mut entries {
            entry.text.push_str(end);
        }
    }
    finish(env, entries, incomplete)
}

fn class_summary_detail(env: &Env<'_>, path: &str) -> String {
    env.reflection
        .class_info(path)
        .map(|record| {
            let kind = record
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("class");
            let source = record
                .pointer("/source/ref")
                .and_then(Value::as_str)
                .unwrap_or("");
            format!("{kind} {source}").trim().to_owned()
        })
        .unwrap_or_default()
}

fn target_items(
    env: &Env<'_>,
    resolved: &Resolved,
    scalar: &Scalar,
    partial: &Partial,
    range: Range,
) -> Completions {
    let _ = (resolved, scalar);
    let typed = partial.typed.trim();
    let prefix = partial.prefix.as_str();
    let mut entries: Vec<Entry> = Vec::new();
    let mut incomplete = false;
    let own = env.own_pack().map(str::to_owned);
    let push_class = |entries: &mut Vec<Entry>, path: &str, detail: String, group: u8| {
        entries.push(
            Entry::new(path, CompletionItemKind::CLASS, range, format!("{path}:"))
                .detail(detail)
                .sort(group, 1)
                .suggest(),
        );
    };
    if template::is_object_target(prefix) {
        // Assets the packs generate: data assets only for `dataasset`, every generated asset
        // (data assets, curves, textures, ...) for `asset`.
        for decl in env.workspace.generated.iter().filter(|decl| {
            decl.kind == GeneratedKind::Asset && (prefix == "asset" || decl.doc_type == "dataasset")
        }) {
            let path = generated_path(decl);
            if contains_ci(&path, typed) {
                if entries.len() >= LIMIT {
                    return finish(env, entries, true);
                }
                entries.push(
                    Entry::new(
                        path.clone(),
                        CompletionItemKind::VALUE,
                        range,
                        format!("{path}:"),
                    )
                    .detail(format!("{} generated by {}", decl.doc_type, decl.pack_ref))
                    .sort(0, 0)
                    .suggest(),
                );
            }
        }
        let assets: Vec<&kdf_validation::AssetEntry> = if prefix == "dataasset" {
            env.reflection.assets_of_class("/Script/Engine.DataAsset")
        } else {
            let mut seen = std::collections::HashSet::new();
            env.reflection
                .asset_index_classes()
                .into_iter()
                .flat_map(|class| env.reflection.assets_of_class(class))
                .filter(|asset| seen.insert(asset.path.as_str()))
                .collect()
        };
        for asset in assets {
            if !(contains_ci(&asset.path, typed) || contains_ci(&asset.name, typed)) {
                continue;
            }
            if entries.len() >= LIMIT {
                incomplete = true;
                break;
            }
            entries.push(
                Entry::new(
                    asset.path.clone(),
                    CompletionItemKind::VALUE,
                    range,
                    format!("{}:", asset.path),
                )
                .detail(asset.asset_class.clone())
                .sort(1, 1)
                .suggest(),
            );
        }
        return finish(env, entries, incomplete);
    }
    if prefix == "class" {
        for decl in env.workspace.generated.iter().filter(|decl| {
            decl.kind == GeneratedKind::Class && Some(decl.pack_ref.as_str()) == own.as_deref()
        }) {
            if contains_ci(&decl.id, typed) {
                if entries.len() >= LIMIT {
                    return finish(env, entries, true);
                }
                push_class(
                    &mut entries,
                    &decl.id,
                    format!("generated by {} ({})", decl.pack_ref, decl.doc_type),
                    0,
                );
            }
        }
        for class in &env.reflection.classes {
            if !(contains_ci(&class.path, typed) || contains_ci(&class.name, typed)) {
                continue;
            }
            if entries.len() >= LIMIT {
                incomplete = true;
                break;
            }
            push_class(
                &mut entries,
                &class.path,
                class.source.clone().unwrap_or_default(),
                1,
            );
        }
        return finish(env, entries, incomplete);
    }
    let Some(base) = template::base_class(prefix) else {
        return Completions::default();
    };
    // Classes the workspace generates below the base class.
    for decl in env
        .workspace
        .generated
        .iter()
        .filter(|decl| decl.kind == GeneratedKind::Class)
    {
        let resolved_parent = decl
            .parent
            .as_deref()
            .and_then(|parent| env.class_in(Some(&decl.pack_ref), parent));
        let reaches = resolved_parent
            .as_deref()
            .is_some_and(|parent| env.derives(parent, base));
        if !reaches {
            continue;
        }
        let text = if Some(decl.pack_ref.as_str()) == own.as_deref() {
            decl.id.clone()
        } else {
            generated_path(decl)
        };
        if contains_ci(&text, typed) || contains_ci(&decl.id, typed) {
            if entries.len() >= LIMIT {
                return finish(env, entries, true);
            }
            push_class(
                &mut entries,
                &text,
                format!("generated by {} ({})", decl.pack_ref, decl.doc_type),
                0,
            );
        }
    }
    if let Some(base_path) = env.reflection.class_path(base) {
        if !env.reflection.is_abstract(base_path) && contains_ci(base_path, typed) {
            if entries.len() >= LIMIT {
                return finish(env, entries, true);
            }
            push_class(
                &mut entries,
                base_path,
                class_summary_detail(env, base_path),
                1,
            );
        }
    }
    for path in env.reflection.descendants(base, true) {
        if !contains_ci(path, typed) {
            continue;
        }
        if entries.len() >= LIMIT {
            incomplete = true;
            break;
        }
        push_class(&mut entries, path, class_summary_detail(env, path), 1);
    }
    finish(env, entries, incomplete)
}

/// Whether the token under the cursor is the whole scalar (its typed result fills the slot).
fn whole_token(scalar: &Scalar, partial: &Partial) -> bool {
    if partial.outer.is_some() || partial.token_start != 0 {
        return false;
    }
    if !partial.closed {
        return true;
    }
    let scan = template::scan(&scalar.value, template::ScanPhase::All);
    scan.segments.len() == 1
        && matches!(
            scan.segments.first(),
            Some(template::TemplateSegment::Token(_))
        )
}

fn member_items(
    env: &Env<'_>,
    resolved: &Resolved,
    scalar: &Scalar,
    partial: &Partial,
    range: Range,
) -> Completions {
    let Some(target_text) = partial.target.as_deref() else {
        return Completions::default();
    };
    let Some(target) = template_target(env, &partial.prefix, target_text) else {
        return Completions::default();
    };
    let expected = if whole_token(scalar, partial) {
        operand_context(env, resolved).map(|context| context.slot)
    } else {
        None
    };
    let end = closer(partial);
    // What is typed narrows the members before the call allowlist runs on their functions.
    let typed = partial.typed.trim();
    let keep = |name: &str| typed.is_empty() || matches_typed(name, typed);
    let members = env
        .reflection
        .list_members_matching(&target.class, target.kind, &keep);
    // Members whose type fits the slot first, properties before functions, then by name.
    let mut ranked: Vec<(u8, u8, String, usize)> = members
        .iter()
        .enumerate()
        .map(|(index, member)| {
            let member_slot = match (&member.property, &member.function) {
                (Some(property), _) => Some(Slot::from_property(property)),
                (None, Some(function)) => function.ret.as_ref().map(Slot::from_property),
                _ => None,
            };
            let rank = match (&expected, &member_slot) {
                (Some(expected), Some(slot)) if !slot_fits(slot, expected) => 1,
                _ => 0,
            };
            let group = u8::from(member.kind == MemberKind::Function);
            (rank, group, member.name.to_lowercase(), index)
        })
        .collect();
    let incomplete = ranked.len() > LIMIT;
    if incomplete {
        ranked.sort();
        ranked.truncate(LIMIT);
    }
    let classifier = env.reflection.reload_classifier(&target.class, false);
    let mut entries = Vec::with_capacity(ranked.len());
    for (rank, _, _, index) in ranked {
        let member = &members[index];
        match member.kind {
            MemberKind::Property => {
                let mut detail = member.type_label.clone();
                // The exporter's annotation is read straight from the property.
                let reload = member
                    .property
                    .as_ref()
                    .map(|property| classifier.classify(property).0);
                if let Some(tag) = reload.and_then(reload_tag) {
                    detail.push_str(&format!(" · {tag}"));
                }
                entries.push(
                    Entry::new(
                        member.name.clone(),
                        CompletionItemKind::FIELD,
                        range,
                        format!("{}{end}", member.name),
                    )
                    .detail(detail)
                    .lazy(resolve::member_property_data(&target.class, &member.name))
                    .sort(rank, 0),
                );
            }
            MemberKind::Function => {
                let Some(function) = &member.function else {
                    continue;
                };
                let signature = member
                    .signature
                    .clone()
                    .unwrap_or_else(|| member.name.clone());
                let marker = match member.implicit {
                    Implicit::Class => " (implicit class)",
                    Implicit::Object => " (implicit object)",
                    Implicit::None => "",
                };
                let explicit = explicit_params(function, member.implicit).len();
                let mut entry = Entry::new(
                    member.name.clone(),
                    CompletionItemKind::METHOD,
                    range,
                    if explicit == 0 {
                        format!("{}{end}", member.name)
                    } else {
                        format!("{}(", member.name)
                    },
                )
                .detail(format!("{signature}{marker}"))
                .lazy(resolve::function_data(
                    &target.class,
                    &member.name,
                    target.kind,
                ))
                .sort(rank, 1);
                if explicit > 0 {
                    entry = entry.snippet(format!("{}($0){end}", member.name)).hints();
                }
                entries.push(entry);
            }
        }
    }
    finish(env, entries, incomplete)
}

/// Enum names, booleans and class paths for the argument under the cursor.
fn argument_items(env: &Env<'_>, scalar: &Scalar, partial: &Partial) -> Completions {
    let (Some(target_text), Some(function)) =
        (partial.target.as_deref(), partial.function.as_ref())
    else {
        return Completions::default();
    };
    let Some(target) = template_target(env, &partial.prefix, target_text) else {
        return Completions::default();
    };
    let Some(info) = env.reflection.find_function(&target.class, &function.name) else {
        return Completions::default();
    };
    let target_class = Some(target.class.as_str());
    let implicit = match env.reflection.callable(&info, target.kind, target_class) {
        Callable::Yes { implicit, .. } => implicit,
        Callable::No(_) => Implicit::None,
    };
    let params = explicit_params(&info, implicit);
    let Some(param) = params.get(function.arg_index as usize) else {
        return Completions::default();
    };
    let property = &param.property;
    // Typing inside a quote leaves the quote alone.
    let quoted = function.in_quote.is_some();
    let start = partial.replace.start + u32::from(quoted);
    let range = scalar.range(start, partial.replace.end.max(start));
    let mut entries = Vec::new();
    let mut incomplete = false;
    let typed = partial.typed.trim_start_matches(['\'', '"']);
    match Slot::from_property(property) {
        Slot::Bool if !quoted => {
            for value in ["true", "false"] {
                entries.push(Entry::new(value, CompletionItemKind::VALUE, range, value));
            }
        }
        Slot::Enum(options) if !quoted => {
            for option in options.iter().filter(|name| !name.ends_with("_MAX")) {
                let name = option.rsplit("::").next().unwrap_or(option);
                if contains_ci(name, typed) {
                    entries.push(
                        Entry::new(name, CompletionItemKind::ENUM_MEMBER, range, name)
                            .detail(property.display_name()),
                    );
                }
            }
        }
        Slot::Class | Slot::SoftClass => {
            let base = property.meta_class.as_deref();
            let mut paths: Vec<&str> = Vec::new();
            if let Some(base) = base {
                if let Some(path) = env.reflection.class_path(base) {
                    paths.push(path);
                }
                paths.extend(env.reflection.descendants(base, true));
            }
            for path in paths {
                if !contains_ci(path, typed) {
                    continue;
                }
                if entries.len() >= LIMIT {
                    incomplete = true;
                    break;
                }
                entries.push(
                    Entry::new(path, CompletionItemKind::CLASS, range, path)
                        .detail(class_summary_detail(env, path)),
                );
            }
        }
        _ => {}
    }
    finish(env, entries, incomplete)
}

// -------------------------------------------------------------------------------------------------
// Config expressions (`config: "..."` in conditions)
// -------------------------------------------------------------------------------------------------

/// What an expression operand stands for.
struct OperandInfo<'a> {
    expr_type: ExprType,
    field: Option<&'a ConfigField>,
    setting: Option<&'a SettingRecord>,
}

fn resolve_operand<'a>(env: &'a Env<'a>, operand: &Operand) -> Option<OperandInfo<'a>> {
    match &operand.kind {
        OperandKind::Config { pack, id } => {
            let own = env.own_pack();
            match env.workspace.config.resolve(own, pack.as_deref(), id) {
                Resolution::Found(field)
                | Resolution::CaseMismatch(field)
                | Resolution::PackDisabled(Some(field)) => Some(OperandInfo {
                    expr_type: field_expr_type(field),
                    field: Some(field),
                    setting: None,
                }),
                _ => None,
            }
        }
        OperandKind::Setting { provider, key } => {
            let setting = env.reflection.settings()?.find(provider, key)?;
            Some(OperandInfo {
                expr_type: setting_expr_type(setting),
                field: None,
                setting: Some(setting),
            })
        }
    }
}

fn operand_of(token: &Token) -> Option<Operand> {
    match expression::parse(&token.text).ok()?.kind {
        ExprKind::Operand(operand) => Some(operand),
        _ => None,
    }
}

/// Operand on the left of the comparison the cursor is part of.
fn left_operand(tokens: &[Token], state: &expression::CursorState) -> Option<Operand> {
    match state.expect {
        Expect::Value | Expect::Operator => state.left.clone(),
        Expect::String => {
            let index = tokens
                .iter()
                .position(|token| token.span.start == state.word_span.start)?;
            let operator = tokens.get(index.checked_sub(1)?)?;
            if !matches!(operator.kind, TokenKind::Compare(_)) {
                return None;
            }
            operand_of(tokens.get(index.checked_sub(2)?)?)
        }
        Expect::ListElement => {
            // `x in [a, |`: the operand before the `in` that precedes the open bracket.
            let mut open = None;
            let mut depth = 0i32;
            for (index, token) in tokens.iter().enumerate() {
                if token.span.start >= state.word_span.start
                    && state.word_span.start != state.word_span.end
                {
                    break;
                }
                if token.span.end > state.word_span.start {
                    break;
                }
                match token.kind {
                    TokenKind::LBracket => {
                        depth += 1;
                        open = Some(index);
                    }
                    TokenKind::RBracket => depth -= 1,
                    _ => {}
                }
            }
            if depth <= 0 {
                return None;
            }
            let open = open?;
            let operator = tokens.get(open.checked_sub(1)?)?;
            if operator.kind != TokenKind::Compare(CmpOp::In) {
                return None;
            }
            operand_of(tokens.get(open.checked_sub(2)?)?)
        }
        Expect::Operand => None,
    }
}

/// Type a candidate must have to follow `operator` after an operand of type `left`.
fn wanted_type(operator: Option<CmpOp>, left: &ExprType) -> Option<ExprType> {
    match operator? {
        CmpOp::Contains => match left {
            ExprType::List(inner) => Some((**inner).clone()),
            _ => None,
        },
        CmpOp::In => Some(ExprType::List(Box::new(left.clone()))),
        _ => Some(left.clone()),
    }
}

fn type_fits(wanted: Option<&ExprType>, candidate: &ExprType) -> bool {
    match wanted {
        None | Some(ExprType::Unknown) => true,
        Some(wanted) => candidate == wanted || *candidate == ExprType::Unknown,
    }
}

/// Select options and booleans of the left operand as literals.
fn option_literals(
    left: Option<&OperandInfo<'_>>,
    entries: &mut Vec<Entry>,
    range: Range,
    quote: char,
    only_strings: bool,
) {
    let Some(info) = left else {
        return;
    };
    let mut options: Vec<(Value, String)> = Vec::new();
    if let Some(field) = info.field {
        options.extend(
            field
                .options
                .iter()
                .map(|option| (option.value.clone(), option.label.clone())),
        );
    }
    if let Some(setting) = info.setting {
        options.extend(setting.options.iter().cloned());
    }
    for (value, label) in options {
        let (name, text) = match &value {
            Value::String(text) => {
                let clean = text.replace(quote, "");
                (clean.clone(), format!("{quote}{clean}{quote}"))
            }
            other if !only_strings => (other.to_string(), other.to_string()),
            _ => continue,
        };
        entries.push(
            Entry::new(name, CompletionItemKind::ENUM_MEMBER, range, text)
                .detail(label)
                .sort(0, 0),
        );
    }
    let boolean = matches!(info.expr_type, ExprType::Bool)
        || matches!(&info.expr_type, ExprType::List(inner) if **inner == ExprType::Bool);
    if boolean && !only_strings {
        for value in ["true", "false"] {
            entries.push(Entry::new(value, CompletionItemKind::VALUE, range, value).sort(0, 1));
        }
    }
}

/// Comparison and logic operators valid after an operand of type `left`.
fn operator_entries(left: Option<&ExprType>, entries: &mut Vec<Entry>, range: Range) {
    const ORDER: [(&str, &str); 6] = [
        ("==", "equals"),
        ("!=", "does not equal"),
        ("<", "less than"),
        ("<=", "less than or equal"),
        (">", "greater than"),
        (">=", "greater than or equal"),
    ];
    let mut operators: Vec<(&str, &str)> = Vec::new();
    match left {
        Some(ExprType::List(_)) => operators.push(("contains", "the list contains the element")),
        Some(ExprType::Bool) | Some(ExprType::String) => {
            operators.extend(&ORDER[..2]);
            if matches!(left, Some(ExprType::String)) {
                operators.push(("in", "is one of a list"));
            }
        }
        Some(ExprType::Number) => {
            operators.extend(ORDER);
            operators.push(("in", "is one of a list"));
        }
        _ => {
            operators.extend(ORDER);
            operators.push(("contains", "the list contains the element"));
            operators.push(("in", "is one of a list"));
        }
    }
    operators.push(("&&", "and"));
    operators.push(("||", "or"));
    for (index, (operator, doc)) in operators.into_iter().enumerate() {
        entries.push(
            Entry::new(operator, CompletionItemKind::OPERATOR, range, operator)
                .detail(doc)
                .sort(0, index as u8),
        );
    }
}

fn expression_items(env: &Env<'_>, scalar: &Scalar) -> Completions {
    let value = scalar.value.as_str();
    let state = expression::cursor_state(value, scalar.cursor);
    let tokens = expression::tokenize(value);
    let left = left_operand(&tokens, &state);
    let left_info = left
        .as_ref()
        .and_then(|operand| resolve_operand(env, operand));
    let range = scalar.range(state.word_span.start, state.word_span.end);
    let own = env.own_pack().map(str::to_owned);
    let mut entries: Vec<Entry> = Vec::new();
    let mut incomplete = false;
    let quote = state.in_string.unwrap_or_else(|| scalar.literal_quote());

    match state.expect {
        Expect::String => option_literals(left_info.as_ref(), &mut entries, range, quote, true),
        Expect::ListElement => {
            option_literals(left_info.as_ref(), &mut entries, range, quote, false)
        }
        Expect::Operator => operator_entries(
            left_info.as_ref().map(|info| &info.expr_type),
            &mut entries,
            range,
        ),
        Expect::Operand | Expect::Value => {
            let wanted = left_info
                .as_ref()
                .and_then(|info| wanted_type(state.operator, &info.expr_type))
                .filter(|_| state.expect == Expect::Value);
            let word = state.word.as_str();
            match state.provider.as_deref() {
                Some("cfg") => {
                    let segment = word.rsplit(':').next().unwrap_or("");
                    let start =
                        state.word_span.start + utf16_len(&word[..word.len() - segment.len()]);
                    let segment_range = scalar.range(start, state.word_span.end.max(start));
                    config_entries(
                        env,
                        &mut entries,
                        state.pack.as_deref(),
                        own.as_deref(),
                        &Slot::Unknown,
                        segment_range,
                        "",
                        segment,
                        &|field| type_fits(wanted.as_ref(), &field_expr_type(field)),
                    );
                    if state.pack.is_none() {
                        pack_stubs(env, &mut entries, own.as_deref(), segment_range, segment);
                    }
                }
                Some(provider) => {
                    let typed = word.split_once(':').map_or("", |(_, rest)| rest);
                    let start =
                        state.word_span.start + utf16_len(&word[..word.len() - typed.len()]);
                    let segment_range = scalar.range(start, state.word_span.end.max(start));
                    incomplete = setting_entries(
                        env,
                        &mut entries,
                        provider,
                        typed,
                        &Slot::Unknown,
                        segment_range,
                        &|setting| type_fits(wanted.as_ref(), &setting_expr_type(setting)),
                    );
                }
                None => {
                    if state.expect == Expect::Value {
                        option_literals(left_info.as_ref(), &mut entries, range, quote, false);
                    }
                    config_entries(
                        env,
                        &mut entries,
                        None,
                        own.as_deref(),
                        &Slot::Unknown,
                        range,
                        "",
                        "",
                        &|field| type_fits(wanted.as_ref(), &field_expr_type(field)),
                    );
                    for provider in template::PREPASS_PREFIXES {
                        let text = format!("{provider}:");
                        entries.push(
                            Entry::new(text.clone(), CompletionItemKind::KEYWORD, range, text)
                                .detail(template::describe(provider))
                                .sort(3, 0)
                                .suggest(),
                        );
                    }
                }
            }
        }
    }
    // The texts go into the YAML scalar: spell them for its quote style.
    for entry in &mut entries {
        entry.text = scalar.encode(&entry.text);
    }
    incomplete |= limit_reference_entries(&mut entries);
    finish(env, entries, incomplete)
}

// -------------------------------------------------------------------------------------------------
// Config documents
// -------------------------------------------------------------------------------------------------

fn field_type_of(resolved: &Resolved) -> Option<String> {
    resolved
        .field_entry()
        .and_then(|field| field.key_value("type").map(str::to_ascii_lowercase))
}

const COMMON_FIELD_KEYS: [&str; 9] = [
    "id",
    "type",
    "name",
    "description",
    "tooltip",
    "category",
    "default",
    "apply",
    "order",
];

fn field_keys(kind: Option<&str>) -> Vec<&'static str> {
    let mut keys = COMMON_FIELD_KEYS.to_vec();
    match kind {
        Some("number") => keys.extend([
            "valueType",
            "multiple",
            "min",
            "max",
            "step",
            "decimals",
            "display",
            "minCount",
            "maxCount",
        ]),
        Some("text") => keys.extend([
            "valueType",
            "multiple",
            "multiline",
            "maxLength",
            "tagRoot",
            "minCount",
            "maxCount",
        ]),
        Some("boolean") => keys.push("valueType"),
        Some("select") => keys.extend([
            "valueType",
            "multiple",
            "options",
            "minSelected",
            "maxSelected",
        ]),
        _ => keys.extend(["valueType", "multiple"]),
    }
    keys
}

const DOC_KEYS: [&str; 6] = [
    "type",
    "category",
    "conditions",
    "conditionBehavior",
    "debug",
    "fields",
];

fn catalog_kind(resolved: &Resolved) -> Option<&str> {
    if resolved.is_config_doc() {
        Some("config")
    } else {
        resolved.doc.kind.as_deref()
    }
}

fn catalog_key_documentation(resolved: &Resolved, key: &str) -> Option<String> {
    let mut path = resolved
        .path()
        .into_iter()
        .filter(|segment| segment != "#")
        .collect::<Vec<_>>();
    path.push(key.to_owned());
    let segments = path.iter().map(String::as_str).collect::<Vec<_>>();
    field_docs::field(catalog_kind(resolved)?, &segments).map(field_docs::to_markdown)
}

fn catalog_value_documentation(resolved: &Resolved, value: &str) -> Option<String> {
    let path = resolved
        .path()
        .into_iter()
        .filter(|segment| segment != "#")
        .collect::<Vec<_>>();
    let segments = path.iter().map(String::as_str).collect::<Vec<_>>();
    field_docs::value_at(catalog_kind(resolved)?, &segments, value).map(field_docs::to_markdown)
}

fn config_keys_for(env: &Env<'_>, resolved: &Resolved, slot: &KeySlot) -> Completions {
    const TOOLTIP_KEYS: [&str; 2] = ["title", "text"];
    const OPTION_KEYS: [&str; 2] = ["value", "label"];
    let path = resolved.path();
    let segments: Vec<&str> = path.iter().map(String::as_str).collect();
    let candidates: Vec<&str> = match segments.as_slice() {
        [] => DOC_KEYS.to_vec(),
        ["fields", "#"] => field_keys(field_type_of(resolved).as_deref()),
        ["fields", "#", "tooltip"] => TOOLTIP_KEYS.to_vec(),
        ["fields", "#", "options", "#"] => OPTION_KEYS.to_vec(),
        _ => return Completions::default(),
    };
    let existing = existing_keys(resolved, slot);
    let range = crate::context::range_on(slot.line, slot.start, slot.end.max(slot.start));
    let entries = candidates
        .into_iter()
        .filter(|name| !existing.contains(name))
        .map(|name| {
            key_entry(
                name,
                catalog_key_documentation(resolved, name),
                range,
                slot.has_colon,
            )
        })
        .collect();
    finish(env, entries, false)
}

/// Keys of the mapping that are already written (the key being edited does not count).
fn existing_keys<'a>(resolved: &'a Resolved, slot: &KeySlot) -> Vec<&'a str> {
    resolved
        .siblings()
        .iter()
        .filter(|key| !(key.line == slot.line && key.name.starts_with(&slot.typed)))
        .map(|key| key.name.as_str())
        .collect()
}

fn key_entry(name: &str, documentation: Option<String>, range: Range, has_colon: bool) -> Entry {
    let mut entry = Entry::new(
        name,
        CompletionItemKind::PROPERTY,
        range,
        if has_colon {
            name.to_owned()
        } else {
            format!("{name}: ")
        },
    );
    if let Some(documentation) = documentation {
        entry = entry.doc(documentation);
    }
    if !has_colon {
        entry = entry.suggest();
    }
    entry
}

/// Keys offered at a new `fields[]` element (the cursor is on a list marker line).
fn config_keys(
    env: &Env<'_>,
    resolved: &Resolved,
    scalar: &Scalar,
    has_colon: bool,
) -> Completions {
    let range = whole_range(scalar);
    let entries = COMMON_FIELD_KEYS
        .iter()
        .copied()
        .map(|name| {
            key_entry(
                name,
                catalog_key_documentation(resolved, name),
                range,
                has_colon,
            )
        })
        .collect();
    finish(env, entries, false)
}

fn op_keys(env: &Env<'_>, resolved: &Resolved, slot: &KeySlot) -> Completions {
    const KEYS: [&str; 8] = ["path", "op", "value", "min", "max", "index", "with", "from"];
    let existing = existing_keys(resolved, slot);
    let range = crate::context::range_on(slot.line, slot.start, slot.end.max(slot.start));
    let entries = KEYS
        .iter()
        .copied()
        .filter(|name| !existing.contains(name))
        .map(|name| {
            key_entry(
                name,
                catalog_key_documentation(resolved, name),
                range,
                slot.has_colon,
            )
        })
        .collect();
    finish(env, entries, false)
}

fn category_reorder_keys(env: &Env<'_>, resolved: &Resolved, slot: &KeySlot) -> Completions {
    const KEYS: [&str; 8] = [
        "type",
        "subType",
        "category",
        "subCategory",
        "categoryPriority",
        "subCategoryPriority",
        "order",
        "debug",
    ];
    let existing = existing_keys(resolved, slot);
    let range = crate::context::range_on(slot.line, slot.start, slot.end.max(slot.start));
    let entries = KEYS
        .iter()
        .copied()
        .filter(|name| !existing.contains(name))
        .map(|name| {
            key_entry(
                name,
                catalog_key_documentation(resolved, name),
                range,
                slot.has_colon,
            )
        })
        .collect();
    finish(env, entries, false)
}

fn simple_values(
    env: &Env<'_>,
    resolved: &Resolved,
    scalar: &Scalar,
    values: &[&str],
) -> Completions {
    let range = whole_range(scalar);
    let entries = values
        .iter()
        .copied()
        .map(|value| {
            let mut entry = Entry::new(
                value,
                CompletionItemKind::ENUM_MEMBER,
                range,
                scalar.encode(value),
            );
            if let Some(documentation) = catalog_value_documentation(resolved, value) {
                entry = entry.doc(documentation);
            }
            entry
        })
        .collect();
    finish(env, entries, false)
}

fn value_type_values(kind: Option<&str>) -> Vec<&'static str> {
    match kind {
        Some("text") => vec!["string", "text", "name", "tag"],
        Some("number") => vec![
            "int32", "int8", "int16", "int64", "uint8", "float", "double",
        ],
        Some("boolean") => vec!["bool", "uint8"],
        _ => vec![
            "string", "text", "name", "tag", "int8", "int16", "int32", "int64", "uint8", "float",
            "double", "bool",
        ],
    }
}

fn config_values(
    env: &Env<'_>,
    resolved: &Resolved,
    scalar: &Scalar,
    key: &str,
) -> Option<Completions> {
    let kind = field_type_of(resolved);
    let in_field = resolved.field_entry().is_some();
    Some(match key {
        "type" if in_field => simple_values(
            env,
            resolved,
            scalar,
            &["text", "number", "boolean", "select"],
        ),
        "valueType" if in_field => {
            simple_values(env, resolved, scalar, &value_type_values(kind.as_deref()))
        }
        "apply" if in_field => simple_values(env, resolved, scalar, &["hot", "cold"]),
        "display" if in_field => simple_values(env, resolved, scalar, &["slider", "input"]),
        "multiple" | "multiline" if in_field => {
            simple_values(env, resolved, scalar, &["true", "false"])
        }
        "category" => category_items(env, scalar),
        "default" if in_field => {
            let field = resolved.field_entry()?;
            match kind.as_deref() {
                Some("boolean") => simple_values(env, resolved, scalar, &["true", "false"]),
                Some("select") => {
                    let line = field
                        .keys
                        .iter()
                        .find(|entry| entry.name == "options")?
                        .line;
                    let values = option_values(env.text, line);
                    let range = whole_range(scalar);
                    let entries = values
                        .iter()
                        .map(|value| {
                            Entry::new(
                                value.clone(),
                                CompletionItemKind::ENUM_MEMBER,
                                range,
                                scalar.encode(value),
                            )
                            .detail("option value")
                        })
                        .collect();
                    finish(env, entries, false)
                }
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// `UFGCategory` subclasses plus the category classes the packs generate.
fn category_items(env: &Env<'_>, scalar: &Scalar) -> Completions {
    let typed = scalar.before_cursor();
    let typed = typed.trim();
    let range = whole_range(scalar);
    let base = kdf_validation::config::CATEGORY_BASE_CLASS;
    let own = env.own_pack().map(str::to_owned);
    let mut entries = Vec::new();
    let mut incomplete = false;
    for decl in env
        .workspace
        .generated
        .iter()
        .filter(|decl| decl.kind == GeneratedKind::Class)
    {
        let reaches = decl
            .parent
            .as_deref()
            .and_then(|parent| env.class_in(Some(&decl.pack_ref), parent))
            .is_some_and(|parent| env.derives(&parent, base));
        if !reaches {
            continue;
        }
        let text = if Some(decl.pack_ref.as_str()) == own.as_deref() {
            decl.id.clone()
        } else {
            generated_path(decl)
        };
        if contains_ci(&text, typed) {
            if entries.len() >= LIMIT {
                return finish(env, entries, true);
            }
            entries.push(
                Entry::new(
                    text.clone(),
                    CompletionItemKind::CLASS,
                    range,
                    scalar.encode(&text),
                )
                .detail(format!("category generated by {}", decl.pack_ref))
                .sort(0, 0),
            );
        }
    }
    if let Some(path) = env.reflection.class_path(base) {
        if !env.reflection.is_abstract(path) && contains_ci(path, typed) {
            if entries.len() >= LIMIT {
                return finish(env, entries, true);
            }
            entries.push(
                Entry::new(path, CompletionItemKind::CLASS, range, scalar.encode(path))
                    .detail(class_summary_detail(env, path))
                    .sort(1, 0),
            );
        }
    }
    for path in env.reflection.descendants(base, true) {
        if !contains_ci(path, typed) {
            continue;
        }
        if entries.len() >= LIMIT {
            incomplete = true;
            break;
        }
        entries.push(
            Entry::new(path, CompletionItemKind::CLASS, range, scalar.encode(path))
                .detail(class_summary_detail(env, path))
                .sort(1, 0),
        );
    }
    finish(env, entries, incomplete)
}
