//! Schema-level checks of `config.schema.yml` (spike S1: `if/then` annotations must reach
//! `unevaluatedProperties`, otherwise per-type keys would be rejected or accepted wrongly).
use kdf_validation::{validate_document, LintOptions};
use std::path::Path;

fn lint(body: &str) -> Vec<(String, String)> {
    let text = format!("fields:\n{body}");
    let result =
        validate_document(Path::new("demo.config.yml"), &text, &LintOptions::default()).unwrap();
    result
        .diagnostics
        .into_iter()
        .map(|d| (d.code, d.yaml_path))
        .collect()
}

#[test]
fn spike_s1_if_then_properties_count_as_evaluated() {
    // Number keys under `type: number` are accepted: `min` comes from the `then` branch.
    let ok = lint(
        "  - id: a.b\n    type: number\n    name: A\n    description: D\n    default: 3\n    min: 1\n    max: 5\n    step: 1\n    display: slider\n    category: Cat\n",
    );
    assert!(ok.is_empty(), "{ok:?}");
}

#[test]
fn spike_s1_keys_of_another_type_are_rejected() {
    let bad = lint(
        "  - id: a\n    type: text\n    name: A\n    description: D\n    default: x\n    min: 1\n",
    );
    assert!(
        bad.iter()
            .any(|(code, path)| code == "schema.unevaluatedProperties" && path == "$.fields[0].min"),
        "{bad:?}"
    );
    let unknown = lint(
        "  - id: a\n    type: number\n    name: A\n    description: D\n    default: 3\n    bogus: 1\n",
    );
    assert!(
        unknown
            .iter()
            .any(|(code, path)| code == "schema.unevaluatedProperties"
                && path == "$.fields[0].bogus"),
        "{unknown:?}"
    );
}

/// Field snippets and the schema keyword (or `None` for "accepted") the frozen schema reports.
#[test]
fn spike_s1_schema_matrix() {
    let cases: [(&str, Option<&str>); 16] = [
        ("{id: a, type: number, name: N, description: D, default: 1.5}", Some("schema")),
        ("{id: a, type: number, valueType: float, name: N, description: D, default: 1.5}", None),
        ("{id: a, type: number, name: N, description: D, default: 1, display: slider}", Some("schema.required")),
        ("{id: a, type: number, name: N, description: D, default: 1, min: 0, max: 2, display: slider}", None),
        ("{id: a, type: number, name: N, description: D, default: 1, step: 0.5}", Some("schema")),
        ("{id: a, type: number, valueType: double, name: N, description: D, default: 1, step: 0.5, decimals: 2}", None),
        ("{id: a, type: number, name: N, description: D, default: 1, decimals: 12}", Some("schema.maximum")),
        ("{id: a, type: text, valueType: string, multiline: true, name: N, description: D, default: x}", Some("schema.const")),
        ("{id: a, type: text, valueType: text, multiline: true, name: N, description: D, default: x}", None),
        ("{id: a, type: text, multiple: true, name: N, description: D, default: x}", Some("schema.type")),
        ("{id: a, type: select, name: N, description: D, default: x}", Some("schema.required")),
        ("{id: a, type: select, multiple: true, name: N, description: D, default: x, options: [{value: x, label: X}]}", Some("schema.type")),
        ("{id: a, type: boolean, valueType: uint8, name: N, description: D, default: true}", None),
        ("{id: a, type: boolean, name: N, description: D, default: 1}", Some("schema.type")),
        ("{id: a, type: number, apply: later, name: N, description: D, default: 1}", Some("schema.enum")),
        ("{id: a, type: number, order: 1.5, name: N, description: D, default: 1}", Some("schema.type")),
    ];
    for (field, expected) in cases {
        let text = format!("category: /Script/FactoryGame.FGCategory\nfields:\n  - {field}\n");
        let found = validate_document(Path::new("demo.config.yml"), &text, &LintOptions::default())
            .unwrap()
            .diagnostics
            .into_iter()
            .map(|d| (d.code, d.yaml_path))
            .collect::<Vec<_>>();
        let codes = found
            .iter()
            .map(|(code, _)| code.as_str())
            .collect::<Vec<_>>();
        match expected {
            None => assert!(codes.is_empty(), "{field}: {codes:?}"),
            Some(prefix) => assert!(
                codes.iter().any(|code| code.starts_with(prefix)),
                "{field}: expected {prefix}, got {codes:?}"
            ),
        }
    }
}
