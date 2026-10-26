use kdf_validation::{validate_document, LintOptions};
use std::path::Path;

fn errors(text: &str) -> Vec<String> {
    validate_document(
        Path::new("nodes.resourcenode.yml"),
        text,
        &LintOptions::default(),
    )
    .unwrap()
    .diagnostics
    .into_iter()
    .filter(|d| d.severity == kdf_validation::Severity::Error)
    .map(|d| d.code)
    .collect()
}

#[test]
fn visual_rules_validate_shape_bounds_and_unique_slots() {
    for (slot, valid) in [
        ("0", true),
        ("63", true),
        ("1.0", true),
        ("64", false),
        ("-1", false),
        ("'1'", false),
        ("true", false),
    ] {
        let text = format!("type: resourcenode\nvisuals:\n  - resource: /Game/Desc.Desc_C\n    materials:\n      - slot: {slot}\n        material: /Game/MI.MI\n");
        assert_eq!(
            errors(&text).is_empty(),
            valid,
            "{slot}: {:?}",
            errors(&text)
        );
    }
    let duplicate = "type: resourcenode\nvisuals:\n  - resource: /Game/Desc.Desc_C\n    materials: [{slot: 0, material: /Game/A.A}, {slot: 0, material: /Game/B.B}]\n";
    assert!(errors(duplicate).contains(&"semantic.visual-slot-duplicate".into()));
}

#[test]
fn hidden_mesh_only_documents_and_budgets_match_runtime() {
    for (extra, valid) in [
        ("", true),
        ("meshActorsPerTick: 1024", true),
        ("meshActorsPerTick: 0", false),
        ("meshActorsPerTick: 1025", false),
        ("meshActorsPerTick: '1'", false),
    ] {
        assert_eq!(
            errors(&format!(
                "type: resourcenode\nhideMeshes: [/Game/Mesh.Mesh]\n{extra}\n"
            ))
            .is_empty(),
            valid
        );
    }
    assert!(!errors("type: resourcenode\nadd: [{id: a, resource: /Game/A.A_C, position: [0,0,0]}]\nmeshActorsPerTick: 1\n").is_empty());
    assert!(!errors("type: resourcenode\nhideMeshes: []\n").is_empty());
}

#[test]
fn visual_resource_references_are_case_insensitively_unique() {
    let text = "type: resourcenode\nvisuals:\n  - resource: /Game/A.A_C\n    materials: [{slot: 0, material: /Game/M.M}]\n  - resource: /game/a.a_c\n    materials: [{slot: 1, material: /Game/M.M}]\n";
    assert!(errors(text).contains(&"semantic.visual-resource-duplicate".into()));
}
