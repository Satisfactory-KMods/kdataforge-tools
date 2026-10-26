use kdf_validation::{validate_document, LintOptions};
use std::path::Path;

#[test]
fn class_identity_filters_validate_types_and_require_scope() {
    for (selector, valid) in [
        ("ofClass: /Script/Test.Base\n    classType: blueprint", true),
        ("ofClass: /Script/Test.Base\n    classType: native", true),
        ("ofClass: /Script/Test.Base\n    classType: any", true),
        (
            "ofClass: /Script/Test.Base\n    classType: blueprints",
            false,
        ),
        ("classType: blueprint", false),
        (
            "ofClass: /Script/Test.Base\n    excludeClasses: [/Game/A.A_C]",
            true,
        ),
        ("ofClass: /Script/Test.Base\n    excludeClasses: []", false),
        (
            "ofClass: /Script/Test.Base\n    excludeClasses: '/KLib/*'",
            true,
        ),
        (
            "ofClass: /Script/Test.Base\n    excludeClasses: ['/KLib/*', '/Game/A.A_C']",
            true,
        ),
        (
            "ofClass: /Script/Test.Base\n    excludeClasses: 'Recipe_*'",
            false,
        ),
        (
            "ofClass: /Script/Test.Base\n    excludeClasses: [null]",
            false,
        ),
    ] {
        let text = format!("type: cdo\npatches:\n  - {selector}\n    properties:\n      - path: mValue\n        value: 1\n");
        let result = validate_document(
            Path::new("identity.cdo.yml"),
            &text,
            &LintOptions::default(),
        )
        .unwrap();
        let errors: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.code.starts_with("schema."))
            .collect();
        assert_eq!(errors.is_empty(), valid, "{selector}: {errors:?}");
        if valid {
            assert!(!result
                .diagnostics
                .iter()
                .any(|d| d.code == "semantic.scope-unused"));
        }
    }
}

#[test]
fn append_unique_requires_value_including_explicit_empty_sequence() {
    for (operand, valid) in [("value: [RF_GAS]", true), ("value: []", true), ("", false)] {
        let text = format!("type: cdo\npatches:\n  - target: /Script/Test.Base\n    properties:\n      - path: mForms\n        op: append_unique\n        {operand}\n");
        let result =
            validate_document(Path::new("merge.cdo.yml"), &text, &LintOptions::default()).unwrap();
        assert_eq!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.severity == kdf_validation::Severity::Error),
            valid,
            "{operand}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn update_where_requires_predicate_and_value() {
    for (operands, valid) in [
        (
            "where: {path: ItemClass, equals: Desc_Old_C}\n        value: {ItemClass: Desc_New_C}",
            true,
        ),
        (
            "whereMode: OR\n        where:\n          - {path: ItemClass, equals: Desc_OldA_C}\n          - {path: ItemClass, equals: Desc_OldB_C}\n        value: {ItemClass: Desc_New_C}",
            true,
        ),
        (
            "whereMode: and\n        where:\n          - {path: ItemClass, equals: Desc_OldA_C}\n          - {path: Amount, greaterThan: 0}\n        value: {ItemClass: Desc_New_C}",
            true,
        ),
        (
            "whereMode: XOR\n        where: {path: ItemClass, equals: Desc_Old_C}\n        value: {ItemClass: Desc_New_C}",
            false,
        ),
        ("whereMode: OR\n        value: {ItemClass: Desc_New_C}", false),
        ("value: {ItemClass: Desc_New_C}", false),
        ("where: {path: ItemClass, equals: Desc_Old_C}", false),
        ("where: []\n        value: {ItemClass: Desc_New_C}", false),
    ] {
        let text = format!("type: cdo\npatches:\n  - target: /Script/Test.Base\n    properties:\n      - path: mAmounts\n        op: update_where\n        {operands}\n");
        let result =
            validate_document(Path::new("update.cdo.yml"), &text, &LintOptions::default()).unwrap();
        assert_eq!(
            !result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == kdf_validation::Severity::Error),
            valid,
            "{operands}: {:?}",
            result.diagnostics
        );
    }

    let wrong_operation = "type: cdo\npatches:\n  - target: /Script/Test.Base\n    properties:\n      - path: mAmounts\n        whereMode: OR\n        where: {path: ItemClass, equals: Desc_Old_C}\n        value: {ItemClass: Desc_New_C}\n";
    let result = validate_document(
        Path::new("where-mode-set.cdo.yml"),
        wrong_operation,
        &LintOptions::default(),
    )
    .unwrap();
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == kdf_validation::Severity::Error));
}

#[test]
fn scope_where_mode_defaults_to_and_and_accepts_or() {
    for (filter, valid) in [
        (
            "where:\n      - {path: mForm, equals: RF_LIQUID}\n      - {path: mResourceSinkPoints, greaterThan: 0}",
            true,
        ),
        (
            "whereMode: OR\n    where:\n      - {path: mForm, equals: RF_LIQUID}\n      - {path: mForm, equals: RF_GAS}",
            true,
        ),
        ("whereMode: OR", false),
        (
            "whereMode: neither\n    where: {path: mForm, equals: RF_LIQUID}",
            false,
        ),
    ] {
        let text = format!(
            "type: cdo\npatches:\n  - ofClass: /Script/Test.Base\n    {filter}\n    properties:\n      - path: mValue\n        value: 1\n"
        );
        let result =
            validate_document(Path::new("where-mode.cdo.yml"), &text, &LintOptions::default())
                .unwrap();
        assert_eq!(
            !result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == kdf_validation::Severity::Error),
            valid,
            "{filter}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn replace_refs_requires_source_and_target_references() {
    for (operands, valid) in [
        (
            "from: /Game/Example/Recipe_Old.Recipe_Old_C\n        value: /Game/Example/Recipe_New.Recipe_New_C",
            true,
        ),
        ("value: /Game/Example/Recipe_New.Recipe_New_C", false),
        ("from: /Game/Example/Recipe_Old.Recipe_Old_C", false),
        ("from: ''\n        value: /Game/Example/Recipe_New.Recipe_New_C", false),
    ] {
        let text = format!("type: cdo\npatches:\n  - target: /Script/Test.Base\n    properties:\n      - path: mUnlocks\n        op: replace_refs\n        {operands}\n");
        let result = validate_document(
            Path::new("replace-refs.cdo.yml"),
            &text,
            &LintOptions::default(),
        )
        .unwrap();
        assert_eq!(
            !result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == kdf_validation::Severity::Error),
            valid,
            "{operands}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn string_table_text_is_available_in_config_and_rejects_mixed_identity() {
    for (value, valid) in [
        ("{stringTable: /Example/ST_Text.ST_Text, key: Name}", true),
        ("{ns: Example, key: Name, source: Name}", true),
        ("{stringTable: '', key: Name}", false),
        ("{stringTable: /Example/ST_Text.ST_Text}", false),
        (
            "{stringTable: /Example/ST_Text.ST_Text, key: Name, source: Name}",
            false,
        ),
        (
            "{stringTable: /Example/ST_Text.ST_Text, key: Name, ns: Example}",
            false,
        ),
    ] {
        let text = format!("category: /Script/FactoryGame.FGCategory\nfields:\n  - id: enabled\n    type: boolean\n    name: {value}\n    description: Example\n    default: true\n");
        let result = validate_document(
            Path::new("example.config.yml"),
            &text,
            &LintOptions::default(),
        )
        .unwrap();
        let errors: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.code.starts_with("schema."))
            .collect();
        assert_eq!(errors.is_empty(), valid, "{value}: {errors:?}");
    }
}

#[test]
fn target_lists_require_nonempty_string_members_for_cdo_and_sublevel() {
    for kind in ["cdo", "sublevel"] {
        for (target, valid) in [
            ("/Game/A.A_C", true),
            ("[/Game/A.A_C, /Game/B.B_C]", true),
            ("[]", false),
            ("''", false),
            ("null", false),
            ("{}", false),
            ("[/Game/A.A_C, '']", false),
            ("[/Game/A.A_C, null]", false),
            ("[/Game/A.A_C, {}]", false),
            ("[/Game/A.A_C, []]", false),
            ("[/Game/A.A_C, 42]", false),
            ("[/Game/A.A_C, true]", false),
        ] {
            let text = if kind == "cdo" {
                format!("type: cdo\npatches:\n  - target: {target}\n    properties:\n      - path: mValue\n        value: 1\n")
            } else {
                format!("type: sublevel\nblock:\n  - target: {target}\n")
            };
            let result =
                validate_document(Path::new("targets.yml"), &text, &LintOptions::default())
                    .unwrap();
            let schema_errors = result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code.starts_with("schema."))
                .collect::<Vec<_>>();
            assert_eq!(
                schema_errors.is_empty(),
                valid,
                "{kind} {target}: {schema_errors:?}"
            );
        }
    }
}

#[test]
fn invalid_leaf_operation_does_not_reject_declared_parent_property() {
    let text = "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mValue\n        op: multiplly\n        value: 2\n";
    let result =
        validate_document(Path::new("invalid.cdo.yml"), text, &LintOptions::default()).unwrap();
    let operation = result
        .diagnostics
        .iter()
        .find(|item| item.code == "schema.enum")
        .expect("specific schema enum");
    assert_eq!(operation.yaml_path, "$.patches[0].properties[0].op");
    assert_eq!(operation.range.start.line, 5);
    assert_eq!(operation.range.start.character, 12);
    assert_eq!(operation.range.end.character, 21);
    assert!(operation.message.contains("multiply"));
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|item| item.code == "schema.unevaluatedProperties"
                && item.yaml_path == "$.patches[0].properties"),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn invalid_leaf_does_not_hide_independent_unknown_patch_property() {
    let text = "type: cdo\npatches:\n  - target: /Game/A.A_C\n    propagateToInstances: true\n    properties:\n      - path: mValue\n        op: multiplly\n        value: 2\n";
    let result =
        validate_document(Path::new("invalid.cdo.yml"), text, &LintOptions::default()).unwrap();
    let unexpected = result
        .diagnostics
        .iter()
        .filter(|item| item.code == "schema.unevaluatedProperties")
        .collect::<Vec<_>>();
    assert_eq!(unexpected.len(), 1, "{:?}", result.diagnostics);
    assert_eq!(unexpected[0].yaml_path, "$.patches[0].propagateToInstances");
}

#[test]
fn merged_cli_policy_keeps_semantic_leaf_without_spurious_parent() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("pack.yml"),
        "ref: Leaf\nname: Leaf\nversion: 1.0.0\n",
    )
    .unwrap();
    std::fs::write(temp.path().join("invalid.cdo.yml"), "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mValue\n        op: multiplly\n        value: 2\n").unwrap();
    let result = kdf_validation::lint_path(
        temp.path(),
        &LintOptions {
            merge_duplicate_diagnostics: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.errors, 1, "{:?}", result.diagnostics);
    assert_eq!(result.diagnostics[0].code, "semantic.invalid-op");
}

#[test]
fn leaf_operand_union_error_preserves_unknown_operation_field() {
    let text = "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mValue\n        op: add\n        index: wrong\n        surprise: true\n        value: 2\n";
    let result =
        validate_document(Path::new("invalid.cdo.yml"), text, &LintOptions::default()).unwrap();
    assert!(result
        .diagnostics
        .iter()
        // index accepts an integer OR an unquoted typed reference after Config merge.
        .any(|item| item.code == "schema.anyOf"
            && item.yaml_path == "$.patches[0].properties[0].index"));
    assert!(result
        .diagnostics
        .iter()
        .any(|item| item.code == "schema.additionalProperties"
            && item.yaml_path == "$.patches[0].properties[0].surprise"));
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|item| item.code == "schema.unevaluatedProperties"
                && item.yaml_path == "$.patches[0].properties"),
        "{:?}",
        result.diagnostics
    );
}
