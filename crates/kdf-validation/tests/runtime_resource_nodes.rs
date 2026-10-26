use kdf_validation::{lint_path, LintOptions, LintResult, Severity};
use std::fs;

const RESOURCE: &str =
    "/Game/FactoryGame/Resource/RawResources/OreIron/Desc_OreIron.Desc_OreIron_C";

fn validate(collection: &str, fields: &str) -> LintResult {
    validate_at(collection, fields, "[10000, 20000, 1200]")
}

fn validate_at(collection: &str, fields: &str, position: &str) -> LintResult {
    let pack = tempfile::tempdir().unwrap();
    fs::write(
        pack.path().join("pack.yml"),
        "ref: ResourceNodes\nname: Resource nodes\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        pack.path().join("nodes.resourcenode.yml"),
        format!(
            "type: resourcenode\n{collection}:\n  - id: northern-iron\n    resource: {RESOURCE}\n    position: {position}\n{fields}"
        ),
    )
    .unwrap();
    lint_path(pack.path(), &LintOptions::default()).unwrap()
}

fn assert_valid(collection: &str, fields: &str) {
    let result = validate(collection, fields);
    assert!(
        result.ok,
        "{collection}: {fields}\n{:?}",
        result.diagnostics
    );
}

fn assert_invalid(collection: &str, fields: &str) {
    let result = validate(collection, fields);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error),
        "Expected authoring error for {collection}: {fields}\n{:?}",
        result.diagnostics
    );
}

#[test]
fn placement_accepts_minimal_addition_and_replacement_using_runtime_defaults() {
    assert_valid("add", "");
    assert_valid("replace", "");
}

#[test]
fn addition_exposes_kbfl_style_range_reuse_move_and_overlap_controls() {
    assert_valid(
        "add",
        "    checkRange: 500\n    reuseExisting: true\n    allowToMove: false\n    preventOverlap: true\n    overlapClass: /Script/FactoryGame.FGResourceNodeBase\n    rotation: [0, 90, -15]\n    purity: Pure\n",
    );
    assert_valid(
        "add",
        "    reuseExisting: false\n    preventOverlap: false\n",
    );
}

#[test]
fn replacement_accepts_original_resource_filter_and_exclusive_radius_alias() {
    assert_valid(
        "replace",
        &format!("    fromResource: {RESOURCE}\n    radius: 250\n    purity: Impure\n"),
    );
    assert_invalid("replace", "    radius: 250\n    checkRange: 500\n");
    assert_invalid("add", "    radius: 250\n");
}

#[test]
fn visual_assets_accept_material_shortcut_or_indexed_nullable_slots() {
    assert_valid(
        "add",
        "    mesh: /Game/Example/SM_Node.SM_Node\n    material: /Game/Example/MI_Node.MI_Node\n",
    );
    assert_valid("replace", "    mesh: /Game/Example/SM_Node.SM_Node\n    materials: [/Game/Example/MI_Node.MI_Node, null]\n");
    assert_valid("add", "    materials: []\n");
    assert_invalid(
        "add",
        "    material: /Game/Example/MI_Node.MI_Node\n    materials: []\n",
    );
    assert_invalid("add", "    material: null\n");
    assert_invalid("replace", "    mesh: ''\n");
}

#[test]
fn ranges_and_scalar_types_reject_values_runtime_cannot_use() {
    for fields in [
        "    checkRange: 0\n",
        "    checkRange: -1\n",
        "    checkRange: 100001\n",
        "    checkRange: unlimited\n",
        "    reuseExisting: perhaps\n",
        "    preventOverlap: 2\n",
        "    allowToMove: sometimes\n",
        "    purity: normal\n",
        "    rotation: [0, 90]\n",
    ] {
        assert_invalid("add", fields);
    }
    assert_valid("add", "    checkRange: 0.1\n");
    assert_valid("replace", "    checkRange: 100000\n");
    assert_valid(
        "add",
        "    preventOverlap: 1\n    reuseExisting: off\n    allowToMove: yes\n",
    );
}

#[test]
fn malformed_world_coordinates_are_not_accepted_as_valid_placements() {
    for position in [
        "[0, 1]",
        "[0, 1, 2, 3]",
        "[0, wrong, 2]",
        "[0, .nan, 2]",
        "[0, .inf, 2]",
    ] {
        let result = validate_at("add", "", position);
        assert!(!result.ok, "{position}: {:?}", result.diagnostics);
    }
}

#[test]
fn material_slot_count_stays_within_runtime_bound() {
    let slots = vec!["null"; 64].join(", ");
    assert_valid("add", &format!("    materials: [{slots}]\n"));
    let too_many = vec!["null"; 65].join(", ");
    assert_invalid("add", &format!("    materials: [{too_many}]\n"));
}

#[test]
fn addition_and_replacement_cannot_claim_same_identity() {
    let fields = format!(
        "replace:\n  - id: northern-iron\n    resource: {RESOURCE}\n    position: [0, 0, 0]\n"
    );
    let result = validate("add", &fields);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "semantic.runtime-identity"),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn separate_files_cannot_claim_same_placement_identity_in_one_pack() {
    let pack = tempfile::tempdir().unwrap();
    fs::write(
        pack.path().join("pack.yml"),
        "ref: Nodes\nname: Nodes\nversion: 1.0.0\n",
    )
    .unwrap();
    for (name, collection) in [("one", "add"), ("two", "replace")] {
        fs::write(pack.path().join(format!("{name}.resourcenode.yml")), format!(
            "type: resourcenode\n{collection}:\n  - id: SharedNode\n    resource: {RESOURCE}\n    position: [0, 0, 0]\n"
        )).unwrap();
    }
    let result = lint_path(pack.path(), &LintOptions::default()).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "semantic.duplicate-id"),
        "{:?}",
        result.diagnostics
    );
}
