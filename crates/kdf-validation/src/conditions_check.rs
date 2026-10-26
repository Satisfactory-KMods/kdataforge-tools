//! Validation of condition sites: `config:` expressions, lifted element-level keys and `hasMod`
//! specifications, for pack manifests, documents and sequence elements.

use crate::conditions::{condition_objects, ConditionSite};
use crate::expression::{self, ExprType, Operand, OperandKind, Span, TypeIssueCode};
use crate::refs::{resolve_target, typed_ref_parts, Env, RefTarget};
use crate::schema::SchemaRegistry;
use crate::slots::{is_cold_condition, ShapeKind, ValueShape};
use crate::template::{self, ScanPhase};
use crate::{Diagnostic, LintResult, Range, Severity};
use serde_json::Value;
use std::collections::HashMap;

fn expr_type(shape: &ValueShape) -> ExprType {
    let element = match &shape.kind {
        ShapeKind::Bool | ShapeKind::SelectBool => ExprType::Bool,
        ShapeKind::Integer | ShapeKind::Float | ShapeKind::SelectNumber { .. } => ExprType::Number,
        ShapeKind::Text(_) | ShapeKind::SelectString(_) => ExprType::String,
        ShapeKind::Unknown => ExprType::Unknown,
    };
    if shape.multiple {
        ExprType::List(Box::new(element))
    } else {
        element
    }
}

fn target_of(operand: &Operand) -> RefTarget {
    match &operand.kind {
        OperandKind::Config { pack, id } => RefTarget::Config {
            pack: pack.clone(),
            id: id.clone(),
        },
        OperandKind::Setting { provider, key } => RefTarget::Setting {
            provider: provider.clone(),
            key: key.clone(),
        },
    }
}

/// Where a `conditions` value sits.
pub(crate) struct ConditionPlace<'p> {
    /// JSON pointer of the `conditions` value.
    pub(crate) pointer: &'p str,
    /// Segments of the owning element (`["patches", "0", "properties", "1"]`; empty at the root).
    pub(crate) owner: &'p [&'p str],
}

/// Checks the `config:` expressions inside one `conditions` value.
pub(crate) fn check_conditions(
    env: &Env<'_>,
    place: &ConditionPlace<'_>,
    conditions: &Value,
    out: &mut Vec<Diagnostic>,
) {
    let cold = env.kind == "pack" || is_cold_condition(env.kind, place.owner);
    for (relative, object) in condition_objects(conditions) {
        let object_pointer = format!("{}{}", place.pointer, relative);
        for (key, value) in object {
            if !key.eq_ignore_ascii_case("config") {
                let pointer = format!(
                    "{object_pointer}/{}",
                    key.replace('~', "~0").replace('/', "~1")
                );
                warn_unresolved(env, value, &pointer, out);
            }
        }
        let Some((key, value)) = object
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("config"))
        else {
            continue;
        };
        let value_pointer = format!("{object_pointer}/{key}");
        match value {
            Value::String(text) => check_expression(env, text, &value_pointer, cold, out),
            Value::Array(items) if !items.is_empty() => {
                for (index, item) in items.iter().enumerate() {
                    let item_pointer = format!("{value_pointer}/{index}");
                    match item {
                        Value::String(text) => {
                            check_expression(env, text, &item_pointer, cold, out)
                        }
                        other => invalid_condition(env, other, &item_pointer, out),
                    }
                }
            }
            other => invalid_condition(env, other, &value_pointer, out),
        }
    }
}

/// Conditions are evaluated before references and templates are resolved: a reference or template
/// written in a condition value is never substituted.
fn warn_unresolved(env: &Env<'_>, value: &Value, pointer: &str, out: &mut Vec<Diagnostic>) {
    match value {
        Value::String(text) => {
            let typed = typed_ref_parts(text).is_some()
                && env
                    .source
                    .and_then(|source| source.scalar_info(env.index, pointer))
                    .is_none_or(|info| info.style.is_plain());
            let templated =
                text.contains('{') && !template::scan(text, ScanPhase::All).tokens().is_empty();
            if typed || templated {
                let reporter = env.reporter();
                let range = reporter.range(env.index, pointer);
                out.push(reporter.diagnostic(
                    env.index,
                    pointer,
                    Severity::Warning,
                    "config.ref-not-resolved",
                    range,
                    "references and templates are never resolved inside conditions; the text is used literally",
                ));
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                warn_unresolved(env, item, &format!("{pointer}/{index}"), out);
            }
        }
        Value::Object(map) => {
            for (key, child) in map {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                warn_unresolved(env, child, &format!("{pointer}/{escaped}"), out);
            }
        }
        _ => {}
    }
}

fn invalid_condition(env: &Env<'_>, value: &Value, pointer: &str, out: &mut Vec<Diagnostic>) {
    let reporter = env.reporter();
    let range = reporter.range(env.index, pointer);
    let tagged = value
        .as_object()
        .filter(|map| map.len() == 1)
        .and_then(|map| map.keys().next())
        .filter(|key| key.starts_with('!'));
    let (code, message) = match tagged {
        Some(tag) => (
            "config.expression-quote",
            format!("the unquoted expression starts with {tag:?}, which YAML reads as a tag; wrap the whole expression in quotes"),
        ),
        None => (
            "config.condition-invalid",
            "config conditions need an expression string or a list of expression strings"
                .to_owned(),
        ),
    };
    out.push(reporter.diagnostic(env.index, pointer, Severity::Error, code, range, message));
}

fn check_expression(
    env: &Env<'_>,
    text: &str,
    pointer: &str,
    cold: bool,
    out: &mut Vec<Diagnostic>,
) {
    let reporter = env.reporter();
    let units = text.encode_utf16().count() as u32;
    let info = env
        .source
        .and_then(|source| source.scalar_info(env.index, pointer));
    let whole = reporter.range(env.index, pointer);
    let range_of = |span: Span| -> Range {
        info.and_then(|info| info.value_range(units, span.start, span.end))
            .unwrap_or(whole)
    };
    let mut push = |severity: Severity, code: &str, span: Span, message: String| {
        out.push(reporter.diagnostic(env.index, pointer, severity, code, range_of(span), message));
    };
    let expr = match expression::parse(text) {
        Ok(expr) => expr,
        Err(error) => {
            push(
                Severity::Error,
                "config.expression-syntax",
                Span::new(error.at, error.at.saturating_add(1)),
                format!("{}: {}", error.code.as_str(), error.message),
            );
            return;
        }
    };
    let mut types: HashMap<String, ExprType> = HashMap::new();
    for operand in expression::operands(&expr) {
        let (resolved, problem) = resolve_target(env, &target_of(&operand));
        if let Some(problem) = problem {
            push(
                problem.severity,
                problem.code,
                operand.span,
                problem.message,
            );
        }
        if let Some(resolved) = resolved {
            if resolved.hot && cold {
                let name = resolved
                    .field
                    .map(|field| field.id.clone())
                    .unwrap_or_default();
                push(
                    Severity::Warning,
                    "config.hot-in-condition",
                    operand.span,
                    format!("hot config {name:?} is read by a condition that is only evaluated at load time; changing it requires a restart"),
                );
            }
            if let Some(shape) = &resolved.shape {
                types.insert(operand.key(), expr_type(shape));
            }
        }
    }
    let issues = expression::type_check(&expr, &|operand: &Operand| {
        Some(
            types
                .get(&operand.key())
                .cloned()
                .unwrap_or(ExprType::Unknown),
        )
    });
    for issue in issues {
        push(
            Severity::Error,
            "config.expression-type",
            issue.span,
            format!(
                "{}: {}",
                issue.code.as_str(),
                type_issue_message(issue.code)
            ),
        );
    }
}

fn type_issue_message(code: TypeIssueCode) -> &'static str {
    match code {
        TypeIssueCode::CompareMismatch => {
            "== and != compare values of the same type (numbers, strings or booleans), never lists"
        }
        TypeIssueCode::OrderNonNumber => "< <= > >= compare numbers only",
        TypeIssueCode::ContainsNonList => "contains needs a list on the left",
        TypeIssueCode::InNonList => "in needs a list on the right",
        TypeIssueCode::ListElementMismatch => "the element type does not match the list",
        TypeIssueCode::UnknownId => "unknown id",
    }
}

/// Validates every element-level condition site of a document: the lifted keys against the
/// `conditionHolder` schema, `hasMod` specifications and `config:` expressions.
pub(crate) fn check_sites(
    env: &Env<'_>,
    registry: &SchemaRegistry,
    sites: &[ConditionSite],
    result: &mut LintResult,
) {
    for site in sites {
        let mut holder = Value::Object(site.holder.clone());
        if let Some(conditions) = holder.get_mut("conditions") {
            match conditions.as_array_mut() {
                Some(items) => items
                    .iter_mut()
                    .for_each(crate::document::normalize_condition_object),
                None => crate::document::normalize_condition_object(conditions),
            }
        }
        if let Some(source) = env.source {
            result.diagnostics.extend(registry.validate_holder(
                &holder,
                &site.pointer,
                &env.file.path,
                env.index,
                source,
            ));
        }
        let Some(conditions) = holder.get("conditions") else {
            continue;
        };
        let owner = site
            .pointer
            .split('/')
            .skip(1)
            .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
            .collect::<Vec<_>>();
        let owner_refs = owner.iter().map(String::as_str).collect::<Vec<_>>();
        let pointer = format!("{}/conditions", site.pointer);
        crate::rules::validate_mod_specs_in(env.file, env.index, conditions, &pointer, result);
        let place = ConditionPlace {
            pointer: &pointer,
            owner: &owner_refs,
        };
        check_conditions(env, &place, conditions, &mut result.diagnostics);
    }
}
