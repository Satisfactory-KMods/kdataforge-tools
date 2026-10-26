//! Hover cards: config ids, settings, template members, `path:` properties and classes.

use crate::cards::{config_card, function_card, property_card, setting_card};
use crate::completion::provider_doc;
use crate::context::{Place, Quote, Resolved, Scalar};
use crate::semantic::{patch_class, template_target, unresolved_key, Env};
use kdf_validation::config::Resolution;
use kdf_validation::expression::{self, ExprKind, Operand, OperandKind, Token, TokenKind};
use kdf_validation::template::{self, Member, ScanPhase, TemplateToken};
use kdf_validation::{Callable, Implicit};
use serde_json::Value;
use tower_lsp::lsp_types::{Hover, HoverContents, Range};

/// Hover with `text`; markdown only when the client declared it renders markdown.
fn card(env: &Env<'_>, range: Range, text: String) -> Option<Hover> {
    if text.trim().is_empty() {
        return None;
    }
    Some(Hover {
        contents: HoverContents::Markup(crate::resolve::markup(text, env.caps.hover_markdown)),
        range: Some(range),
    })
}

/// Card of an operand (`cfg:` reference or bare expression id).
pub(crate) fn operand_card(env: &Env<'_>, operand: &Operand) -> Option<String> {
    match &operand.kind {
        OperandKind::Config { pack, id } => {
            let resolution = env
                .workspace
                .config
                .resolve(env.own_pack(), pack.as_deref(), id);
            match resolution {
                Resolution::Found(field)
                | Resolution::CaseMismatch(field)
                | Resolution::PackDisabled(Some(field)) => {
                    Some(config_card(field, env.workspace.root()))
                }
                Resolution::UnknownPack => Some(format!(
                    "Unknown pack `{}`: it is not part of this project.",
                    pack.as_deref().unwrap_or_default()
                )),
                _ => Some(format!("No config field `{id}` is defined here.")),
            }
        }
        OperandKind::Setting { provider, key } => {
            let settings = env.reflection.settings()?;
            match settings.find(provider, key) {
                Some(setting) => Some(setting_card(setting)),
                None => Some(format!(
                    "`{provider}:{key}` is not in the datapack settings index."
                )),
            }
        }
    }
}

fn operand_from_text(text: &str) -> Option<Operand> {
    match expression::parse(text).ok()?.kind {
        ExprKind::Operand(operand) => Some(operand),
        _ => None,
    }
}

/// Short summary of a class of the reflection data.
pub(crate) fn class_card(env: &Env<'_>, path: &str) -> Option<String> {
    let record = env.reflection.class_info(path)?;
    let text = |pointer: &str| {
        record
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let name = text("/name").unwrap_or_else(|| path.to_owned());
    let kind = text("/kind").unwrap_or_else(|| "class".into());
    let mut out = format!("**{name}** ({kind})\n\n`{path}`\n\n");
    if let Some(parent) = text("/super") {
        out.push_str(&format!("- Parent: `{parent}`\n"));
    }
    if let Some(source) = text("/source/ref") {
        out.push_str(&format!("- Source: `{source}`\n"));
    }
    let is_abstract = env.reflection.is_abstract(path);
    out.push_str(&format!(
        "- Abstract: {}\n",
        if is_abstract { "yes" } else { "no" }
    ));
    if let Some(count) = record
        .get("properties")
        .and_then(Value::as_array)
        .map(Vec::len)
    {
        out.push_str(&format!("- Properties: {count}\n"));
    }
    if let Some(count) = record
        .get("functions")
        .and_then(Value::as_array)
        .map(Vec::len)
    {
        out.push_str(&format!("- Functions: {count}\n"));
    }
    Some(out)
}

pub(crate) fn hover(env: &Env<'_>, resolved: &Resolved) -> Option<Hover> {
    let Place::Value(slot) = &resolved.place else {
        return None;
    };
    let scalar = &slot.scalar;
    let key = resolved.key().unwrap_or("").to_owned();
    if resolved.in_conditions() {
        return if key == "config" {
            expression_hover(env, scalar)
        } else {
            None
        };
    }
    if resolved.is_config_doc() && key == "id" && resolved.field_entry().is_some() {
        let pack = env.own_pack()?;
        let field = env.workspace.config.field(pack, scalar.value.trim())?;
        return card(env, whole(scalar), config_card(field, env.workspace.root()));
    }
    if resolved.op_entry().is_some_and(|op| op.below == ["path"]) {
        return path_hover(env, resolved, scalar);
    }
    if unresolved_key(&key) {
        return None;
    }
    if scalar.value.contains('{') {
        if let Some(found) = template_hover(env, scalar) {
            return Some(found);
        }
    }
    if scalar.quote == Quote::Plain {
        if let Some((provider, _)) = scalar.value.split_once(':') {
            if template::PREPASS_PREFIXES.contains(&provider) {
                let operand = operand_from_text(scalar.value.trim())?;
                return card(env, whole(scalar), operand_card(env, &operand)?);
            }
        }
    }
    if matches!(
        key.as_str(),
        "category" | "parent" | "class" | "target" | "ofClass" | "allAssetsOfClass"
    ) {
        let class = env.class_of(scalar.value.trim())?;
        return card(env, whole(scalar), class_card(env, &class)?);
    }
    None
}

fn whole(scalar: &Scalar) -> Range {
    scalar.range(0, scalar.units())
}

fn expression_hover(env: &Env<'_>, scalar: &Scalar) -> Option<Hover> {
    let tokens = expression::tokenize(&scalar.value);
    let token: &Token = tokens.iter().find(|token| {
        matches!(token.kind, TokenKind::Ident | TokenKind::Reference)
            && token.span.start <= scalar.cursor
            && scalar.cursor < token.span.end.max(token.span.start + 1)
    })?;
    let operand = operand_from_text(&token.text)?;
    card(
        env,
        scalar.range(token.span.start, token.span.end),
        operand_card(env, &operand)?,
    )
}

fn path_hover(env: &Env<'_>, resolved: &Resolved, scalar: &Scalar) -> Option<Hover> {
    let path = scalar.value.trim();
    if resolved.op_entry().is_some() && path == kdf_validation::TAG_CONTAINER_SELECTOR {
        return card(
            env,
            whole(scalar),
            kdf_validation::TAG_CONTAINER_SELECTOR_DOC.to_owned(),
        );
    }
    let patch = patch_class(env, resolved)?;
    let property = env.reflection.resolve_property_path(&patch.class, path)?;
    let (reload, reason) =
        env.reflection
            .property_reload_with(&patch.class, path, patch.apply_to_spawned);
    card(
        env,
        whole(scalar),
        property_card(
            &property,
            Some((reload, reason.as_str())),
            Some(patch.class.as_str()),
        ),
    )
}

/// Innermost token whose span holds `offset`.
fn token_at(tokens: Vec<&TemplateToken>, offset: u32) -> Option<&TemplateToken> {
    tokens
        .into_iter()
        .filter(|token| token.span.start <= offset && offset < token.span.end)
        .max_by_key(|token| token.depth)
}

fn template_hover(env: &Env<'_>, scalar: &Scalar) -> Option<Hover> {
    let scan = template::scan(&scalar.value, ScanPhase::All);
    let token = token_at(scan.tokens(), scalar.cursor)?;
    let offset = scalar.cursor;
    if token.prefix_span.start <= offset && offset <= token.prefix_span.end {
        return card(
            env,
            scalar.range(token.prefix_span.start, token.prefix_span.end),
            provider_doc(&token.prefix),
        );
    }
    match template::phase_of(&token.prefix)? {
        template::Phase::PrePass => {
            let operand = operand_from_text(&format!("{}:{}", token.prefix, token.body))?;
            card(
                env,
                scalar.range(token.body_span.start, token.body_span.end),
                operand_card(env, &operand)?,
            )
        }
        template::Phase::Apply => {
            let (target, member, member_offset) = template::split_body(&token.body)?;
            let target_units = crate::context::utf16_len(target);
            let target_end = token.body_span.start + target_units;
            let member_start = token.body_span.start + member_offset;
            if offset <= target_end {
                let resolved = template_target(env, &token.prefix, target)?;
                return card(
                    env,
                    scalar.range(token.body_span.start, target_end),
                    class_card(env, &resolved.class)?,
                );
            }
            if offset < member_start {
                return None;
            }
            let target = template_target(env, &token.prefix, target)?;
            let member_range = scalar.range(member_start, token.body_span.end);
            member_card(env, &target.class, target.kind, member)
                .and_then(|text| card(env, member_range, text))
        }
    }
}

/// Card of a property read or function call on a template target.
fn member_card(
    env: &Env<'_>,
    class: &str,
    kind: kdf_validation::TargetKind,
    member: &str,
) -> Option<String> {
    let parsed = template::parse_member(member).ok()?;
    match parsed {
        Member::Call { name, .. } => function_text(env, class, kind, &name),
        Member::Ident { name, .. } => {
            let property = env.reflection.find_property(class, &name);
            match property {
                Some(property) => {
                    let (reload, reason) = env.reflection.property_reload(class, &name);
                    Some(property_card(
                        &property,
                        Some((reload, reason.as_str())),
                        Some(class),
                    ))
                }
                None => function_text(env, class, kind, &name),
            }
        }
        Member::Path { path, .. } => {
            let property = env.reflection.resolve_property_path(class, &path)?;
            let (reload, reason) = env.reflection.property_reload(class, &path);
            Some(property_card(
                &property,
                Some((reload, reason.as_str())),
                Some(class),
            ))
        }
    }
}

fn function_text(
    env: &Env<'_>,
    class: &str,
    kind: kdf_validation::TargetKind,
    name: &str,
) -> Option<String> {
    let function = env.reflection.find_function(class, name)?;
    let callable = env.reflection.callable(&function, kind, Some(class));
    let implicit = match &callable {
        Callable::Yes { implicit, .. } => *implicit,
        Callable::No(_) => Implicit::None,
    };
    let signature = env.reflection.signature(&function, implicit);
    Some(function_card(&function, &signature, &callable))
}
