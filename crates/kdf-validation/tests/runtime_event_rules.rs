use kdf_validation::{lint_path, validate_document, LintOptions, LintResult};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn rule() -> Value {
    json!({"id": "first-rule", "event": "worldReady", "actions": [{"log": "Ready"}]})
}

fn validate_rules(rules: Value) -> LintResult {
    let document = json!({"type": "eventrule", "rules": rules}).to_string();
    validate_document(
        Path::new("progression.eventrule.yml"),
        &document,
        &LintOptions::default(),
    )
    .unwrap()
}

fn assert_valid(rules: Value) {
    let result = validate_rules(rules);
    assert!(result.ok, "{:?}", result.diagnostics);
}

fn assert_invalid(rules: Value) {
    let result = validate_rules(rules);
    assert!(
        !result.ok,
        "unexpected acceptance: {:?}",
        result.diagnostics
    );
}

fn write_pack(root: &Path) {
    fs::write(
        root.join("pack.yml"),
        "ref: EventExample\nname: Event Example\nversion: 1.0.0\n",
    )
    .unwrap();
}

fn write_rule(root: &Path, file: &str, id: &str) {
    let mut entry = rule();
    entry["id"] = json!(id);
    fs::write(
        root.join(file),
        json!({"type": "eventrule", "rules": [entry]}).to_string(),
    )
    .unwrap();
}

#[test]
fn minimal_rule_uses_runtime_defaults_without_generated_class_requirements() {
    let result = validate_rules(json!([rule()]));
    assert!(result.ok, "{:?}", result.diagnostics);
    assert!(
        !result.diagnostics.iter().any(|diagnostic| {
            diagnostic.yaml_path.contains("parent") || diagnostic.yaml_path.contains("register")
        }),
        "event rules must not inherit generated-class requirements: {:?}",
        result.diagnostics
    );
}

#[test]
fn once_accepts_booleans_and_rejects_strings_numbers_and_legacy_scalar_aliases() {
    for value in [json!(true), json!(false)] {
        let mut entry = rule();
        entry["once"] = value;
        assert_valid(json!([entry]));
    }
    for value in [
        json!("true"),
        json!("false"),
        json!("yes"),
        json!("off"),
        json!(1),
        json!(0),
        Value::Null,
    ] {
        let mut entry = rule();
        entry["once"] = value;
        assert_invalid(json!([entry]));
    }
    for scalar in ["yes", "no", "on", "off", "1", "0"] {
        let text = format!("type: eventrule\nrules:\n  - id: aliases\n    event: worldReady\n    once: {scalar}\n    actions:\n      - log: Ready\n");
        let result = validate_document(
            Path::new("aliases.eventrule.yml"),
            &text,
            &LintOptions::default(),
        )
        .unwrap();
        assert!(!result.ok, "{scalar}: {:?}", result.diagnostics);
    }
}

#[test]
fn purchase_event_allows_optional_filter_and_empty_or_nonempty_requirements() {
    for filter in [None, Some("/Game/Example.Schematic_C")] {
        for required in [json!([]), json!(["/Game/Prerequisite.Schematic_C"])] {
            let mut entry = rule();
            entry["event"] = json!("schematicPurchased");
            entry["requiresSchematics"] = required;
            entry["actions"] = json!([{"unlockSchematic": "/Game/Reward.Schematic_C"}]);
            if let Some(filter) = filter {
                entry["schematic"] = json!(filter);
            }
            assert_valid(json!([entry]));
        }
    }
}

#[test]
fn world_ready_rejects_schematic_filter() {
    let mut entry = rule();
    entry["schematic"] = json!("/Game/Example.Schematic_C");
    assert_invalid(json!([entry]));
}

#[test]
fn event_names_rule_keys_and_action_keys_require_exact_case() {
    for event in [
        "WorldReady",
        "worldready",
        "SchematicPurchased",
        "deliveryCompleted",
    ] {
        let mut entry = rule();
        entry["event"] = json!(event);
        assert_invalid(json!([entry]));
    }
    for key in ["Once", "schematics", "unexpected"] {
        let mut entry = rule();
        entry[key] = json!(true);
        assert_invalid(json!([entry]));
    }
    for action in [
        json!({"Log": "Ready"}),
        json!({"UnlockSchematic": "/Game/A.A_C"}),
    ] {
        let mut entry = rule();
        entry["actions"] = json!([action]);
        assert_invalid(json!([entry]));
    }
}

#[test]
fn required_fields_and_id_characters_are_enforced() {
    for key in ["id", "event", "actions"] {
        let mut entry = rule();
        entry.as_object_mut().unwrap().remove(key);
        assert_invalid(json!([entry]));
    }
    for id in ["", "two words", "pack:id", "slash/id", "ümlaut"] {
        let mut entry = rule();
        entry["id"] = json!(id);
        assert_invalid(json!([entry]));
    }
    let mut entry = rule();
    entry["id"] = json!("A_valid-id.123");
    assert_valid(json!([entry]));
}

#[test]
fn each_action_requires_exactly_one_supported_nonblank_field() {
    for action in [
        json!({}),
        json!({"unlockSchematic": "/Game/Reward.Schematic_C", "log": "Ready"}),
        json!({"log": "Ready", "toast": "Unexpected"}),
        json!({"log": ""}),
        json!({"log": " \t\n"}),
        json!({"unlockSchematic": ""}),
        json!({"unlockSchematic": " \t"}),
        json!({"log": true}),
    ] {
        let mut entry = rule();
        entry["actions"] = json!([action]);
        assert_invalid(json!([entry]));
    }
}

#[test]
fn schematic_filters_and_requirements_reject_blank_or_nonstring_values() {
    for value in [
        json!(""),
        json!(" \t"),
        Value::Null,
        json!(false),
        json!({}),
    ] {
        let mut entry = rule();
        entry["event"] = json!("schematicPurchased");
        entry["schematic"] = value.clone();
        assert_invalid(json!([entry]));

        let mut entry = rule();
        entry["requiresSchematics"] = json!([value]);
        assert_invalid(json!([entry]));
    }
    for value in [Value::Null, json!("/Game/A.A_C"), json!({})] {
        let mut entry = rule();
        entry["requiresSchematics"] = value;
        assert_invalid(json!([entry]));
    }
}

#[test]
fn document_rule_count_is_bounded() {
    assert_invalid(json!([]));
    let entries = (0..256)
        .map(|index| {
            let mut entry = rule();
            entry["id"] = json!(format!("rule-{index}"));
            entry
        })
        .collect::<Vec<_>>();
    assert_valid(json!(entries));
    let mut oversized = entries;
    let mut extra = rule();
    extra["id"] = json!("rule-256");
    oversized.push(extra);
    assert_invalid(json!(oversized));
}

#[test]
fn per_rule_action_requirement_id_and_log_limits_match_runtime() {
    let mut entry = rule();
    entry["id"] = json!("x".repeat(128));
    entry["actions"] = json!(vec![json!({"log": "x".repeat(2048)}); 32]);
    entry["requiresSchematics"] = json!(vec!["/Game/A.A_C"; 64]);
    assert_valid(json!([entry.clone()]));

    for (field, value) in [
        ("id", json!("x".repeat(129))),
        ("actions", json!([])),
        ("actions", json!(vec![json!({"log": "Ready"}); 33])),
        ("actions", json!([{"log": "x".repeat(2049)}])),
        ("requiresSchematics", json!(vec!["/Game/A.A_C"; 65])),
    ] {
        let mut oversized = entry.clone();
        oversized[field] = value;
        assert_invalid(json!([oversized]));
    }
}

#[test]
fn rule_identity_collisions_ignore_case_within_and_across_files() {
    let mut second = rule();
    second["id"] = json!("FIRST-RULE");
    assert_invalid(json!([rule(), second]));

    let project = tempdir().unwrap();
    write_pack(project.path());
    write_rule(project.path(), "first.eventrule.yml", "OneRule");
    write_rule(project.path(), "second.eventrule.yml", "onerule");
    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "semantic.duplicate-id"),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn event_identity_is_separate_from_generated_classes_and_preserves_punctuation() {
    let project = tempdir().unwrap();
    write_pack(project.path());
    write_rule(project.path(), "first.eventrule.yml", "Shared-Id");
    write_rule(project.path(), "second.eventrule.yml", "Shared_Id");
    fs::write(
        project.path().join("shared.item.yml"),
        "type: item\nitems:\n  - id: Shared-Id\n    parent: /Script/FactoryGame.FGItemDescriptor\n",
    )
    .unwrap();
    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    assert!(report.ok, "{:?}", report.diagnostics);
}
