use kdf_validation::{validate_document, LintOptions, Severity};
use std::path::Path;

fn action_errors(fields: &str) -> Vec<String> {
    let text = format!("type: cdo\npatches:\n  - {fields}\n");
    validate_document(Path::new("actions.cdo.yml"), &text, &LintOptions::default())
        .unwrap()
        .diagnostics
        .into_iter()
        .filter(|entry| entry.severity == Severity::Error)
        .map(|entry| entry.message)
        .collect()
}

#[test]
fn lifecycle_actions_accept_explicit_actor_lists_without_properties() {
    for action in ["destroy", "dismantle"] {
        let fields = format!("target: [/Script/Engine.Actor, /Script/Engine.StaticMeshActor]\n    applyToSpawnedActors: true\n    applyToSubclasses: true\n    actorAction: {action}");
        assert!(action_errors(&fields).is_empty(), "{action}");
    }
    assert!(action_errors("target: /Script/Engine.Actor\n    applyToSpawnedActors: true\n    actorAction: destroy\n    actorSelector: {tags: Retired}").is_empty());
}

#[test]
fn lifecycle_actions_require_explicit_spawn_application_and_valid_action() {
    for fields in [
        "target: /Script/Engine.Actor\n    actorAction: destroy",
        "target: /Script/Engine.Actor\n    applyToSpawnedActors: false\n    actorAction: destroy",
        "allAssetsOfClass: /Script/Engine.Actor\n    applyToSpawnedActors: true\n    actorAction: destroy",
        "target: /Script/Engine.Actor\n    applyToSpawnedActors: true\n    actorAction: explode",
        "target: /Script/Engine.Actor\n    applyToSpawnedActors: true\n    actorAction: [destroy]",
    ] {
        assert!(!action_errors(fields).is_empty(), "{fields}");
    }
}

#[test]
fn lifecycle_actions_reject_property_writes_and_class_scope_filters() {
    let base =
        "target: /Script/Engine.Actor\n    applyToSpawnedActors: true\n    actorAction: dismantle";
    for extra in [
        "properties: []",
        "properties: [{path: Tags, value: []}]",
        "ofClass: /Script/Engine.Actor",
        "matchAssets: false",
        "where: {path: Tags, isEmpty: true}",
    ] {
        assert!(
            !action_errors(&format!("{base}\n    {extra}")).is_empty(),
            "{extra}"
        );
    }
}
