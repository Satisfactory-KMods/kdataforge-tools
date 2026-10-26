//! Property-path completion follows the operation order within one properties list.

use crate::context::testing::at;
use crate::test_fixture::{Fixture, ITEM};
use crate::test_support;
use serde_json::json;

fn offers(source: &str, name: &str) -> bool {
    Fixture::new()
        .run("ConfigDemo/order.cdo.yml", source)
        .items
        .iter()
        .any(|item| item.label == name)
}

fn document(operations: &str) -> String {
    format!("type: cdo\npatches:\n  - target: {ITEM}\n    properties:\n{operations}")
}

#[test]
fn initial_set_remains_available_before_following_multiply() {
    let source = document(
        "      - path: mSta|\n        value: 1\n      - path: mStackSize\n        op: multiply\n        value: 2\n",
    );
    assert!(offers(&source, "mStackSize"));
}

#[test]
fn initial_set_remains_available_before_following_append() {
    let source = document(
        "      - path: mAm|\n        op: set\n        value: []\n      - path: mAmounts\n        op: append\n        value: []\n",
    );
    assert!(offers(&source, "mAmounts"));
}

#[test]
fn later_set_is_suppressed_after_a_preceding_operation() {
    for operation in ["set", "multiply"] {
        let source = document(&format!(
            "      - path: mStackSize\n        op: {operation}\n        value: 1\n      - path: mSta|\n        value: 2\n",
        ));
        assert!(!offers(&source, "mStackSize"), "preceding {operation}");
    }
}

#[test]
fn later_chained_operation_can_use_a_preceding_path() {
    for operation in ["multiply", "add"] {
        let source = document(&format!(
            "      - path: mStackSize\n        value: 1\n      - path: mSta|\n        op: {operation}\n        value: 2\n",
        ));
        assert!(offers(&source, "mStackSize"), "current {operation}");
    }
}

#[test]
fn peer_paths_include_only_preceding_entries_not_current_or_future_paths() {
    let source = document(
        "      - path: mDisplayName\n        value: Name\n      - path: mSta|ckSize\n        value: 1\n      - path: mAmounts\n        op: append\n        value: []\n",
    );
    assert_eq!(at("order.cdo.yml", &source).peer_paths, ["mDisplayName"]);
    assert!(offers(&source, "mStackSize"));
}

#[test]
fn paths_from_a_separate_patch_do_not_suppress_completion() {
    let source = format!(
        "type: cdo\npatches:\n  - target: {ITEM}\n    properties:\n      - path: mStackSize\n        value: 1\n  - target: {ITEM}\n    properties:\n      - path: mSta|\n        value: 2\n",
    );
    assert!(offers(&source, "mStackSize"));
    assert!(at("order.cdo.yml", &source).peer_paths.is_empty());
}

#[test]
fn nested_property_entries_use_their_own_order_and_scope() {
    let source = document(
        "      - path: mAmounts\n        value:\n          properties:\n            - path: Amount\n              value: 1\n            - path: Item|Class\n              value: Test\n            - path: Future\n              value: 2\n      - path: mStackSize\n        value: 3\n",
    );
    assert_eq!(at("order.cdo.yml", &source).peer_paths, ["Amount"]);
}

#[test]
fn indexed_struct_completion_ignores_following_operations() {
    let source = document(
        "      - path: mAmounts[0].Am|\n        value: 1\n      - path: mAmounts[0].Amount\n        op: multiply\n        value: 2\n",
    );
    assert!(offers(&source, "Amount"));
}

#[test]
fn preceding_map_entry_preserves_case_sensitive_key_spelling() {
    const MAP_CLASS: &str = "/Script/Test.MapOwner";
    let fixture = Fixture::build(
        &[],
        vec![test_support::class(
            MAP_CLASS,
            None,
            vec![test_support::merge(
                test_support::prop("mValues", "MapProperty", "TMap<FString, FItemAmount>"),
                json!({
                    "key": test_support::prop("Key", "StrProperty", "FString"),
                    "value": test_support::merge(
                        test_support::prop("Value", "StructProperty", "FItemAmount"),
                        json!({"struct": "/Script/FactoryGame.ItemAmount"})
                    )
                }),
            )],
            vec![],
        )],
    );
    for (key, available) in [("tier.high", true), ("Tier.High", false)] {
        let source = format!(
            "type: cdo\npatches:\n  - target: {MAP_CLASS}\n    properties:\n      - path: mValues['Tier.High'].amount\n        value: 1\n      - path: mvalues['{key}'].Am|\n        value: 2\n",
        );
        let completions = fixture.run("ConfigDemo/map.cdo.yml", &source);
        assert_eq!(
            completions.items.iter().any(|item| item.label == "Amount"),
            available,
            "map key {key}",
        );
    }
}
