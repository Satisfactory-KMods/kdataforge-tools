use kdf_validation::{
    tag_container_selector_property, validate_document, LintOptions, ReflectionIndex, Severity,
};
use std::path::Path;

#[test]
fn tag_container_selector_accepts_only_supported_operations_and_defaults_to_set() {
    let fixture = include_str!("fixtures/tag-container-selector.cdo.yml");
    let result =
        validate_document(Path::new("tags.cdo.yml"), fixture, &LintOptions::default()).unwrap();
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error),
        "{:?}",
        result.diagnostics
    );
    for op in [
        None,
        Some("set"),
        Some("append"),
        Some("prepend"),
        Some("remove"),
        Some("clear"),
    ] {
        let operation = op
            .map(|op| format!("        op: {op}\n"))
            .unwrap_or_default();
        let value = if op == Some("clear") {
            ""
        } else {
            "        value: [KMods.PlannerResource]\n"
        };
        let text = format!("type: cdo\npatches:\n  - target: /Script/Test.Resource\n    applyToSubclasses: true\n    properties:\n      - path: '@tagContainers'\n{operation}{value}");
        let result =
            validate_document(Path::new("tags.cdo.yml"), &text, &LintOptions::default()).unwrap();
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.severity == Severity::Error),
            "{text}: {:?}",
            result.diagnostics
        );
    }
    for op in [
        "add",
        "append_unique",
        "copy",
        "move",
        "reverse",
        "remove_at",
    ] {
        let text = format!("type: cdo\npatches:\n  - target: /Script/Test.Resource\n    properties:\n      - path: '@tagContainers'\n        op: {op}\n        value: [KMods.PlannerResource]\n        from: mTags\n        index: 0\n");
        let result =
            validate_document(Path::new("tags.cdo.yml"), &text, &LintOptions::default()).unwrap();
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "semantic.tag-container-op"),
            "{op}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn tag_container_selector_is_not_a_readable_path_or_a_copy_source() {
    for selector in [
        "@tagContainers.Child",
        "@tagcontainers",
        "@tagContainers[0]",
    ] {
        let text = format!("type: cdo\npatches:\n  - target: /Script/Test.Resource\n    properties:\n      - path: '{selector}'\n        value: []\n");
        let result =
            validate_document(Path::new("tags.cdo.yml"), &text, &LintOptions::default()).unwrap();
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.severity == Severity::Error),
            "{selector}"
        );
    }
    let text = "type: cdo\npatches:\n  - ofClass: /Script/Test.Resource\n    where: {path: '@tagContainers', isEmpty: true}\n    properties:\n      - {path: mTags, op: copy, from: '@tagContainers'}\n";
    let result =
        validate_document(Path::new("tags.cdo.yml"), text, &LintOptions::default()).unwrap();
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code.starts_with("schema.")));
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == "semantic.where-path"));
}

#[test]
fn selector_operand_metadata_does_not_create_a_reflected_property() {
    let property = tag_container_selector_property("@tagContainers").unwrap();
    assert_eq!(property.cpp_type, "FGameplayTagContainer");
    assert_eq!(
        property.struct_path.as_deref(),
        Some("/Script/GameplayTags.GameplayTagContainer")
    );
    assert!(tag_container_selector_property("mGameplayTags").is_none());
    let reflection = ReflectionIndex::unavailable("No reflection needed for operation selectors");
    assert!(reflection
        .resolve_property_path("/Script/Test.Resource", "@tagContainers")
        .is_none());
}
