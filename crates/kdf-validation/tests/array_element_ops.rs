//! Array element operations (`append`, `prepend`, `insert`) and empty list operands.
//!
//! The runtime (`KDFOpEngine.cpp`, `ApplyArrayOp`) treats a present but empty list as zero elements to
//! add: a no-op. Only an absent `value` key is an error there. The linter must give the same verdicts:
//! an empty list, literal or from a `cfg:` list field, is clean; a missing `value` is `semantic.op-required`.
//! `remove` keeps its documented strictness (an array remove that matches nothing fails), so a statically
//! empty `remove` operand is not asserted here.
mod support;

use kdf_validation::{lint_path, LintOptions, LintResult};
use support::{triples, write};
use tempfile::tempdir;

const PACK: &str = "ref: EmptyLists\nname: Empty lists\nversion: 1.0.0\n";

const CATEGORY: &str =
    "type: class\nclasses:\n  - id: ProbeCategory\n    parent: /Script/FactoryGame.FGCategory\n";

const CONFIG: &str = "type: config\ncategory: ProbeCategory\nfields:\n  - id: probe.unlockRecipes\n    type: text\n    valueType: string\n    multiple: true\n    name: Extra recipes\n    description: A list that may stay empty.\n    default: []\n    apply: hot\n";

const OPERATIONS: [&str; 3] = ["append", "prepend", "insert"];

fn lint_with_document(document: &str) -> (tempfile::TempDir, LintResult) {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(root, "Probe/pack.yml", PACK);
    write(root, "Probe/probe.class.yml", CATEGORY);
    write(root, "Probe/probe.config.yml", CONFIG);
    write(root, "Probe/probe.cdo.yml", document);
    let result = lint_path(root, &LintOptions::default()).unwrap();
    (dir, result)
}

fn document(operation: &str, value_line: &str) -> String {
    let index_line = if operation == "insert" {
        "        index: 0\n"
    } else {
        ""
    };
    format!(
        "type: cdo\npatches:\n  - target: /Game/FactoryGame/Schematics/Progression/Schematic_1-1.Schematic_1-1_C\n    properties:\n      - path: mUnlocks[0].mRecipes\n        op: {operation}\n{index_line}{value_line}"
    )
}

fn assert_clean(operation: &str, value_line: &str) {
    let (dir, result) = lint_with_document(&document(operation, value_line));
    assert!(
        result.diagnostics.is_empty(),
        "{operation} {value_line:?}: {:?}",
        triples(&result, dir.path())
    );
}

#[test]
fn empty_list_config_value_is_a_clean_operand_of_array_element_ops() {
    for operation in OPERATIONS {
        assert_clean(operation, "        value: cfg:probe.unlockRecipes\n");
    }
}

#[test]
fn literal_empty_list_is_a_clean_operand_of_array_element_ops() {
    for operation in OPERATIONS {
        assert_clean(operation, "        value: []\n");
    }
}

#[test]
fn a_missing_value_is_still_reported_for_array_element_ops() {
    for operation in OPERATIONS {
        let (dir, result) = lint_with_document(&document(operation, ""));
        assert_eq!(
            triples(&result, dir.path()),
            [(
                "Probe/probe.cdo.yml".to_owned(),
                "semantic.op-required".to_owned(),
                "$.patches[0].properties[0]".to_owned()
            )],
            "{operation}"
        );
    }
}

#[test]
fn operation_table_requires_value_for_array_element_ops() {
    for operation in OPERATIONS {
        let spec = kdf_validation::operation_spec(operation).unwrap();
        assert!(spec.required.contains(&"value"), "{operation}");
    }
}
