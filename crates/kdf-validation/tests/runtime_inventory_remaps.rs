use kdf_validation::{validate_document, LintOptions, Severity};
use std::path::Path;

fn errors(fields: &str) -> Vec<String> {
    let text = format!("type: cdo\npatches:\n  - {fields}\n");
    validate_document(
        Path::new("inventory.cdo.yml"),
        &text,
        &LintOptions::default(),
    )
    .unwrap()
    .diagnostics
    .into_iter()
    .filter(|entry| entry.severity == Severity::Error)
    .map(|entry| entry.message)
    .collect()
}

const BASE: &str = "target: /Script/FactoryGame.FGCharacterPlayer\n    applyToSpawnedActors: true";

#[test]
fn accepts_item_pairs_and_explicit_integer_multipliers() {
    for rule in [
        "{from: Old, to: New}",
        "{from: Old, to: New, multiplier: 10}",
    ] {
        assert!(errors(&format!("{BASE}\n    inventoryRemap: [{rule}]")).is_empty());
    }
}

#[test]
fn rejects_ambiguous_operation_modes_and_invalid_quantities() {
    for rule in [
        "[]",
        "[{from: Old}]",
        "[{from: '', to: New}]",
        "[{from: Old, to: New, multiplier: 0}]",
        "[{from: Old, to: New, multiplier: -1}]",
        "[{from: Old, to: New, multiplier: 1.5}]",
        "[{from: Old, to: New, multiplier: 1.0}]",
        "[{from: Old, to: New, multiplier: '10'}]",
        "[{from: Old, to: New, multiplier: 2147483648}]",
        "[{from: Old, to: New, unknown: true}]",
    ] {
        assert!(
            !errors(&format!("{BASE}\n    inventoryRemap: {rule}")).is_empty(),
            "{rule}"
        );
    }
    for extra in [
        "properties: []",
        "actorAction: destroy",
        "matchAssets: true",
    ] {
        assert!(!errors(&format!(
            "{BASE}\n    inventoryRemap: [{{from: Old, to: New}}]\n    {extra}"
        ))
        .is_empty());
    }
    assert!(!errors(
        "target: /Script/FactoryGame.FGCharacterPlayer\n    inventoryRemap: [{from: Old, to: New}]"
    )
    .is_empty());
}

#[test]
fn rejects_duplicate_sources_and_chained_or_cyclic_rules() {
    for rules in [
        "[{from: Old, to: Old}]",
        "[{from: Old, to: New}, {from: OLD, to: Other}]",
        "[{from: Old, to: New}, {from: New, to: Final}]",
        "[{from: A, to: B}, {from: B, to: A}]",
    ] {
        assert!(
            errors(&format!("{BASE}\n    inventoryRemap: {rules}"))
                .iter()
                .any(|error| error.contains("inventoryRemap")),
            "{rules}"
        );
    }
}
