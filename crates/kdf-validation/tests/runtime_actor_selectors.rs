use kdf_validation::{validate_document, LintOptions, Severity};
use std::path::Path;

fn errors(selector: &str, extra: &str) -> Vec<String> {
    let text = format!("type: cdo\npatches:\n  - target: /Script/Engine.Actor\n    applyToSpawnedActors: true\n    actorSelector: {selector}\n    properties: [{{path: Tags, op: append, value: [Selected]}}]\n{extra}");
    validate_document(Path::new("actors.cdo.yml"), &text, &LintOptions::default())
        .unwrap()
        .diagnostics
        .into_iter()
        .filter(|entry| entry.severity == Severity::Error)
        .map(|entry| entry.message)
        .collect()
}

#[test]
fn actor_predicates_accept_combined_world_bounds_and_recipe_choices() {
    for selector in [
        "{tags: FactoryNorth}",
        "{tags: [FactoryNorth, Boosted], sphere: {center: [0, 0, 0], radius: 500}}",
        "{box: {min: [-100, -200, -300], max: [100, 200, 300]}, currentRecipe: [Recipe_A, Recipe_B]}",
    ] {
        assert!(errors(selector, "").is_empty(), "{selector}");
    }
}

#[test]
fn actor_bounds_require_positive_radius_and_strictly_ordered_box_components() {
    for selector in [
        "{sphere: {center: [0, 0, 0], radius: 0}}",
        "{sphere: {center: [0, 0], radius: 500}}",
        "{box: {min: [0, 0, 0], max: [0, 100, 100]}}",
        "{box: {min: [0, 200, 0], max: [100, 100, 100]}}",
    ] {
        assert!(!errors(selector, "").is_empty(), "{selector}");
    }
}

#[test]
fn actor_scope_cannot_mix_with_asset_or_class_filters() {
    for filter in [
        "    ofClass: /Script/Engine.Actor\n",
        "    allAssetsOfClass: /Script/Engine.Actor\n",
        "    matchTag: FactoryNorth\n",
        "    matchAssets: false\n",
        "    where: {path: mValue, equals: 1}\n",
    ] {
        assert!(!errors("{tags: Selected}", filter).is_empty(), "{filter}");
    }
}

#[test]
fn actor_selector_rejects_empty_or_unknown_predicates() {
    for selector in ["{}", "{owner: 0}", "{tags: []}", "{currentRecipe: '   '}"] {
        assert!(!errors(selector, "").is_empty(), "{selector}");
    }
}
