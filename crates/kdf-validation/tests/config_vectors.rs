//! Runs every contract vector from `tests/fixtures/config-contract-vectors.json` against the Rust
//! implementation (expression grammar, templates, element conditions).
use kdf_validation::conditions::{prune_config_conditions, strip_element_conditions};
use kdf_validation::expression::{self, EvalValue, ExprType, Operand};
use kdf_validation::template::{self, ScanIssueKind, ScanPhase};
use serde_json::Value;
use std::collections::BTreeMap;

fn vectors() -> Value {
    serde_json::from_str(include_str!("fixtures/config-contract-vectors.json")).unwrap()
}

fn items<'a>(root: &'a Value, path: &str) -> &'a [Value] {
    root.pointer(path)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("missing vector list {path}"))
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("{value} has no {key}"))
}

#[test]
fn contract_version_is_one() {
    assert_eq!(vectors()["contractVersion"], 1);
}

#[test]
fn expression_parse_vectors() {
    let root = vectors();
    let list = items(&root, "/expression/parse");
    assert_eq!(list.len(), 22);
    for vector in list {
        let src = text(vector, "src");
        let expr = expression::parse(src)
            .unwrap_or_else(|error| panic!("{} {src:?}: {error:?}", vector["id"]));
        assert_eq!(expr.to_vector(), vector["ast"], "{} {src:?}", vector["id"]);
    }
}

#[test]
fn expression_error_vectors() {
    let root = vectors();
    let list = items(&root, "/expression/errors");
    assert_eq!(list.len(), 24);
    for vector in list {
        let src = text(vector, "src");
        let error = expression::parse(src)
            .expect_err(&format!("{} {src:?} should not parse", vector["id"]));
        assert_eq!(
            error.code.as_str(),
            text(&vector["error"], "code"),
            "{} {src:?}: {error:?}",
            vector["id"]
        );
        assert_eq!(
            u64::from(error.at),
            vector["error"]["at"].as_u64().unwrap(),
            "{} {src:?}: {error:?}",
            vector["id"]
        );
    }
}

#[test]
fn expression_typecheck_vectors() {
    let root = vectors();
    let list = items(&root, "/expression/typecheck");
    assert_eq!(list.len(), 11);
    for vector in list {
        let src = text(vector, "src");
        let expr = expression::parse(src).unwrap();
        let types = vector["operands"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    ExprType::parse(value.as_str().unwrap()).unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let issues = expression::type_check(&expr, &|operand: &Operand| {
            types.get(&operand.key()).cloned()
        });
        let actual = issues
            .iter()
            .map(|issue| {
                serde_json::json!({
                    "code": issue.code.as_str(),
                    "at": issue.span.start,
                    "len": issue.span.len(),
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            Value::Array(actual),
            vector["issues"],
            "{} {src:?}",
            vector["id"]
        );
    }
}

#[test]
fn expression_eval_vectors() {
    let root = vectors();
    let list = items(&root, "/expression/eval");
    assert_eq!(list.len(), 17);
    for vector in list {
        let src = text(vector, "src");
        let expr = expression::parse(src).unwrap();
        let values = vector["values"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), EvalValue::from_json(value).unwrap()))
            .collect::<BTreeMap<_, _>>();
        let result = expression::eval(&expr, &|operand: &Operand| {
            values.get(&operand.key()).cloned()
        });
        match (result, vector.get("result"), vector.get("error")) {
            (Ok(actual), Some(expected), _) => {
                assert_eq!(Value::Bool(actual), *expected, "{} {src:?}", vector["id"])
            }
            (Err(error), _, Some(expected)) => {
                assert_eq!(
                    error.code.as_str(),
                    expected.as_str().unwrap(),
                    "{} {src:?}",
                    vector["id"]
                )
            }
            (other, _, _) => panic!("{} {src:?}: unexpected {other:?}", vector["id"]),
        }
    }
}

#[test]
fn template_expand_vectors() {
    let root = vectors();
    let list = items(&root, "/template/expand");
    assert_eq!(list.len(), 12);
    for vector in list {
        let src = text(vector, "src");
        let values = vector["values"].as_object().unwrap();
        let expansion = template::expand_prepass(src, &mut |prefix, body| {
            let key = format!("{prefix}:{body}");
            values.get(&key).map(|value| match value {
                Value::Array(items) => items
                    .iter()
                    .map(|item| item.as_str().unwrap_or_default().to_owned())
                    .collect::<Vec<_>>()
                    .join(", "),
                other => other.as_str().unwrap_or_default().to_owned(),
            })
        });
        assert_eq!(
            expansion.output,
            text(vector, "output"),
            "{} {src:?}",
            vector["id"]
        );
        let unterminated = expansion
            .issues
            .iter()
            .any(|issue| issue.kind == ScanIssueKind::Unbalanced);
        assert_eq!(
            unterminated,
            vector["unterminated"].as_bool().unwrap_or(false),
            "{} {src:?}",
            vector["id"]
        );
    }
}

#[test]
fn template_scan_vectors() {
    let root = vectors();
    let list = items(&root, "/template/scan");
    assert_eq!(list.len(), 6);
    for vector in list {
        let src = text(vector, "src");
        assert_eq!(text(vector, "phase"), "apply");
        let scan = template::scan(src, ScanPhase::Apply);
        assert_eq!(
            scan.to_vector(),
            vector["segments"],
            "{} {src:?}",
            vector["id"]
        );
    }
}

#[test]
fn template_member_vectors() {
    let root = vectors();
    let list = items(&root, "/template/member");
    assert_eq!(list.len(), 10);
    for vector in list {
        let src = text(vector, "src");
        match (
            template::parse_member(src),
            vector.get("member"),
            vector.get("error"),
        ) {
            (Ok(member), Some(expected), _) => {
                assert_eq!(member.to_vector(), *expected, "{} {src:?}", vector["id"])
            }
            (Err(error), _, Some(expected)) => {
                assert_eq!(
                    error.kind.as_str(),
                    expected.as_str().unwrap(),
                    "{} {src:?}",
                    vector["id"]
                )
            }
            (other, _, _) => panic!("{} {src:?}: unexpected {other:?}", vector["id"]),
        }
    }
}

#[test]
fn element_condition_vectors() {
    let root = vectors();
    let list = items(&root, "/elementConditions");
    assert_eq!(list.len(), 4);
    for vector in list {
        let mut stripped = vector["input"].clone();
        let sites = strip_element_conditions(&mut stripped);
        assert_eq!(stripped, vector["stripped"], "{} stripped", vector["id"]);
        let pointers = sites
            .iter()
            .map(|site| Value::String(site.pointer.clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            Value::Array(pointers),
            vector["sites"],
            "{} sites",
            vector["id"]
        );
        let values = vector["values"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), EvalValue::from_json(value).unwrap()))
            .collect::<BTreeMap<_, _>>();
        let pruned = prune_config_conditions(&vector["input"], &|operand: &Operand| {
            values.get(&operand.key()).cloned()
        });
        assert_eq!(pruned, vector["pruned"], "{} pruned", vector["id"]);
    }
}
