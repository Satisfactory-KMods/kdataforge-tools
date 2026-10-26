use kdf_validation::{lint_path, property_path_identity, LintOptions};
use std::fs;
use tempfile::tempdir;

fn duplicates(operations: &[(&str, &str)]) -> Vec<String> {
    let project = tempdir().unwrap();
    fs::write(
        project.path().join("pack.yml"),
        "ref: Example\nname: Example Pack\nversion: 1.0.0\n",
    )
    .unwrap();
    let mut text = String::from(
        "type: cdo\npatches:\n  - target: /Script/FactoryGame.FGItemDescriptor\n    properties:\n",
    );
    for (path, operation) in operations {
        text.push_str(&format!(
            "      - path: {}\n        op: {operation}\n        value: 1\n",
            serde_json::to_string(path).unwrap()
        ));
    }
    fs::write(project.path().join("input.cdo.yml"), text).unwrap();
    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "semantic.property-path"),
        "invalid test paths: {:?}",
        report.diagnostics
    );
    assert!(report.ok, "invalid operations: {:?}", report.diagnostics);
    report
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.code == "semantic.property-path-duplicate")
        .map(|diagnostic| diagnostic.yaml_path)
        .collect()
}

#[test]
fn differently_cased_string_map_keys_are_distinct_targets() {
    for paths in [
        ["mValues['Tier.High']", "mValues['tier.high']"],
        ["mValues[Tier.High]", "mValues[tier.high]"],
    ] {
        assert_ne!(
            property_path_identity(paths[0]),
            property_path_identity(paths[1])
        );
        assert!(duplicates(&[(paths[0], "set"), (paths[1], "set")]).is_empty());
    }
}

#[test]
fn member_case_does_not_distinguish_the_same_map_key() {
    assert_eq!(
        property_path_identity("mValues['Tier.High'].Amount"),
        property_path_identity("MVALUES['Tier.High'].amount")
    );
    assert!(duplicates(&[
        ("mValues['Tier.High'].Amount", "set"),
        ("MVALUES['Tier.High'].amount", "set"),
    ])
    .is_empty());
}

#[test]
fn nested_accessors_preserve_keys_escaped_quotes_and_indices() {
    let paths = [
        r#"mGroups[2].Values["Tier\"High"].Amount"#,
        r#"mGroups[2].Values["tier\"high"].Amount"#,
        r#"mGroups[3].Values["Tier\"High"].Amount"#,
        r#"MGROUPS[2].values["Tier\"High"].amount"#,
    ];
    let identity = property_path_identity(paths[0]);
    assert_eq!(identity, property_path_identity(paths[3]));
    assert_ne!(identity, property_path_identity(paths[1]));
    assert_ne!(identity, property_path_identity(paths[2]));
    assert!(duplicates(&paths.map(|path| (path, "set"))).is_empty());
}

#[test]
fn chained_operations_remain_valid_on_identical_targets() {
    assert!(duplicates(&[
        ("mValues['Tier.High'].Amount", "set"),
        ("MVALUES['Tier.High'].amount", "multiply"),
        ("mValues['Tier.High'].Amount", "multiply"),
        ("mValues['Tier.High'].Amount", "append"),
    ])
    .is_empty());
}

#[test]
fn different_operations_share_canonical_paths_in_either_order() {
    for operation in ["append", "remove", "clear", "multiply"] {
        for operations in [
            [
                ("mValues['Tier.High'].Amount", "set"),
                ("MVALUES['Tier.High'].amount", operation),
            ],
            [
                ("mValues['Tier.High'].Amount", operation),
                ("MVALUES['Tier.High'].amount", "set"),
            ],
        ] {
            assert!(duplicates(&operations).is_empty(), "{operations:?}");
        }
    }
}

#[test]
fn set_append_remove_clear_chain_allows_repeated_paths_in_either_order() {
    let operations = [
        ("mValues['Tier.High']", "set"),
        ("MVALUES['Tier.High']", "append"),
        ("mValues['Tier.High']", "remove"),
        ("mValues['Tier.High']", "clear"),
    ];
    assert!(duplicates(&operations).is_empty());
    let reversed = operations.into_iter().rev().collect::<Vec<_>>();
    assert!(duplicates(&reversed).is_empty());
}

#[test]
fn quoted_accessor_contents_never_become_member_segments() {
    assert_eq!(
        property_path_identity(r#" MGroups[2].Values["Tier[HIGH].Foo"].Amount "#),
        r#"mgroups[2].values["Tier[HIGH].Foo"].amount"#
    );
    assert_eq!(
        property_path_identity(r#"MValues['Tier\'HIGH'].Amount"#),
        r#"mvalues['Tier\'HIGH'].amount"#
    );
    assert_eq!(
        property_path_identity("MValues[ÄKey].Amount"),
        "mvalues[ÄKey].amount"
    );
}

#[test]
fn incomplete_accessors_do_not_gain_an_assumed_identity() {
    for path in ["MValues['Tier", "MValues[]", "MValues[Key]."] {
        assert_eq!(property_path_identity(path), path);
    }
}
