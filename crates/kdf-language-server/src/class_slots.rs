//! Class-reference authority shared by completion and Catalog, using existing slot/reflection contracts.
use crate::{
    context::{self, Place, Resolved, Scalar},
    intellisense,
};
use kdf_validation::{field_docs, slots, PropertyInfo, ReflectionIndex};
use std::path::Path;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionResponse, CompletionTextEdit,
    Position, TextEdit,
};

pub(crate) const CLASS_KEYS: &[&str] = &[
    "ItemClass",
    "itemClass",
    "target",
    "ofClass",
    "excludeClasses",
    "allAssetsOfClass",
    "parent",
    "class",
    "item",
    "resource",
    "fromResource",
    "overlapClass",
    "currentRecipe",
    "unlockSchematic",
    "requiresSchematics",
    "schematic",
    "producedIn",
    "ingredient",
    "product",
];
pub(crate) const BATCH_LIMIT: usize = 64;
pub(crate) const TEXT_LIMIT: usize = 512 * 1024;

pub(crate) fn is_inventory_item_reference(resolved: &Resolved) -> bool {
    resolved.doc.kind.as_deref() == Some("cdo")
        && matches!(resolved.key(), Some("from" | "to"))
        && resolved.path().iter().any(|key| key == "inventoryRemap")
}

pub(crate) struct ClassSlot {
    pub(crate) scalar: Scalar,
    pub(crate) base_class: Option<String>,
    pub(crate) concrete_only: bool,
}

pub(crate) fn at(
    path: &Path,
    text: &str,
    position: Position,
    index: &ReflectionIndex,
) -> Option<ClassSlot> {
    let resolved = context::resolve(path, text, position);
    from_resolved(text, &resolved, index)
}

pub(crate) fn from_resolved(
    text: &str,
    resolved: &Resolved,
    index: &ReflectionIndex,
) -> Option<ClassSlot> {
    let slot = for_completion(text, resolved, index)?;
    if resolved.key() == Some("excludeClasses") && slot.scalar.value.contains(['*', '?']) {
        // A namespace pattern selects a set; it is not one class Catalog can open.
        return None;
    }
    if resolved.op_entry().is_none() {
        let kind = resolved.doc.kind.as_deref()?;
        let path = resolved.path();
        let keys = path
            .iter()
            .filter(|key| key.as_str() != "#")
            .map(String::as_str)
            .collect::<Vec<_>>();
        field_docs::field(kind, &keys)?;
    }
    Some(slot)
}

/// Completion keeps explicit class keys useful in incomplete YAML; Catalog needs schema proof.
pub(crate) fn for_completion(
    text: &str,
    resolved: &Resolved,
    index: &ReflectionIndex,
) -> Option<ClassSlot> {
    let Place::Value(value) = &resolved.place else {
        return None;
    };
    if resolved.in_comment || !value.scalar.inside {
        return None;
    }
    let key = resolved.key()?;
    let inventory_target = key == "target"
        && resolved.doc.kind.as_deref() == Some("cdo")
        && resolved.frames.windows(2).any(|frames| {
            matches!(&frames[0], context::Frame::Seq { via_key: Some(key) } if key == "patches")
                && matches!(&frames[1], context::Frame::Map { keys, .. }
                    if keys.iter().any(|key| key.name == "inventoryRemap"))
        });
    let inventory_item = is_inventory_item_reference(resolved);
    let actor_action_target = key == "target"
        && resolved.doc.kind.as_deref() == Some("cdo")
        && resolved.frames.windows(2).any(|frames| {
            matches!(&frames[0], context::Frame::Seq { via_key: Some(key) } if key == "patches")
                && matches!(&frames[1], context::Frame::Map { keys, .. }
                    if keys.iter().any(|key| key.name == "actorAction"))
        });
    let base = if let Some(op) = resolved.op_entry() {
        if op.op_name() == "clear" {
            return None;
        }
        let mut ctx = intellisense::context(text, resolved.position()?);
        let field = intellisense::property(index, &ctx)?;
        if matches!(key, "ItemClass" | "itemClass")
            && field["cppType"]
                .as_str()
                .is_some_and(|value| value.contains("FItemAmount"))
        {
            slots::default_base_class("items").map(str::to_owned)
        } else {
            if !matches!(op.below.as_slice(), [key] if key == "value")
                && !matches!(op.below.as_slice(), [key, item] if key == "value" && item == "#")
            {
                return None;
            }
            let property = PropertyInfo::from_record(field);
            let slot = slots::Slot::from_property(&property);
            let expected = if value.list_item {
                slot.element()
            } else {
                slot
            };
            let base = intellisense::compatible_class_base(field).map(str::to_owned);
            if !matches!(expected, slots::Slot::Class | slots::Slot::SoftClass) && base.is_none() {
                return None;
            }
            // Multiple targets must agree on the same constraint, not only on cppType.
            for target in std::mem::take(&mut ctx.targets) {
                ctx.targets.clear();
                ctx.targets.push(target);
                let other = intellisense::property(index, &ctx)?;
                if intellisense::compatible_class_base(other).map(str::to_owned) != base {
                    return None;
                }
            }
            base
        }
    } else {
        if !CLASS_KEYS.contains(&key) && !inventory_item {
            return None;
        }
        let path = resolved.path();
        match key {
            "target" if inventory_target => Some("/Script/FactoryGame.FGCharacterPlayer"),
            "target" if actor_action_target => Some("/Script/Engine.Actor"),
            "from" | "to" if inventory_item => slots::default_base_class("items"),
            "ItemClass" | "itemClass" | "item" | "ingredient" | "product" => {
                slots::default_base_class("items")
            }
            "resource" | "fromResource" => slots::default_base_class("resources"),
            "schematic" | "unlockSchematic" | "requiresSchematics" => {
                slots::default_base_class("schematics")
            }
            "currentRecipe" => slots::default_base_class("recipes"),
            "overlapClass" => Some("/Script/Engine.Actor"),
            "producedIn" => kdf_validation::template::base_class("building"),
            "parent" | "class" => path
                .iter()
                .rev()
                .skip(1)
                .find_map(|key| slots::default_base_class(key)),
            _ => None,
        }
        .map(str::to_owned)
    };
    Some(ClassSlot {
        scalar: value.scalar.clone(),
        concrete_only: base.is_some()
            && key != "parent"
            && !actor_action_target
            && !inventory_target,
        base_class: base,
    })
}

pub(crate) fn completion(slot: ClassSlot, index: &ReflectionIndex) -> CompletionResponse {
    let typed = slot.scalar.before_cursor().to_ascii_lowercase();
    let range = slot.scalar.range(0, slot.scalar.units());
    let mut items = index
        .classes
        .iter()
        .filter(|class| {
            (class.path.to_ascii_lowercase().contains(&typed)
                || class.name.to_ascii_lowercase().contains(&typed))
                && class_matches(class, index, &slot)
        })
        .take(201)
        .map(|class| CompletionItem {
            label: class.name.clone(),
            kind: Some(CompletionItemKind::CLASS),
            label_details: Some(tower_lsp::lsp_types::CompletionItemLabelDetails {
                detail: class.kind.as_ref().map(|kind| format!("  {kind}")),
                description: class.source.clone(),
            }),
            insert_text: Some(class.path.clone()),
            sort_text: Some(class.path.clone()),
            detail: Some(class.path.clone()),
            filter_text: Some(format!("{} {}", class.path, class.name)),
            documentation: Some(tower_lsp::lsp_types::Documentation::MarkupContent(
                tower_lsp::lsp_types::MarkupContent {
                    kind: tower_lsp::lsp_types::MarkupKind::Markdown,
                    value: intellisense::class_fields(
                        &class.name,
                        &class.path,
                        class.source.as_deref(),
                        class.parent.as_deref(),
                    ),
                },
            )),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range,
                new_text: slot.scalar.encode(&class.path),
            })),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    if items.len() > 200 {
        items.truncate(200);
        CompletionResponse::List(CompletionList {
            is_incomplete: true,
            items,
        })
    } else {
        CompletionResponse::Array(items)
    }
}

pub(crate) fn class_matches(
    class: &kdf_validation::ClassSummary,
    index: &ReflectionIndex,
    constraint: &ClassSlot,
) -> bool {
    (!constraint.concrete_only || !class.abstract_class)
        && constraint
            .base_class
            .as_deref()
            .is_none_or(|base| matches_base(index, &class.path, base))
}

pub(crate) fn matches_base(index: &ReflectionIndex, class: &str, base: &str) -> bool {
    if index.is_subclass_of(class, base) {
        return true;
    }
    // Limited archives can still prove their last exported parent edge without a base record.
    index.class_path(base).is_none()
        && index
            .super_chain_inclusive(class)
            .iter()
            .any(|ancestor| index.parent_path(ancestor) == Some(base))
}

/// Protocol offsets are absolute UTF-16 units, never Rust bytes or clamped document positions.
pub(crate) fn position_at(text: &str, wanted: u64) -> Option<Position> {
    let (mut offset, mut line, mut column) = (0u64, 0u32, 0u32);
    for character in text.chars() {
        if offset == wanted {
            return Some(Position::new(line, column));
        }
        offset += character.len_utf16() as u64;
        if offset > wanted {
            return None;
        }
        if character == '\n' {
            line += 1;
            column = 0;
        } else {
            column += character.len_utf16() as u32;
        }
    }
    (offset == wanted).then_some(Position::new(line, column))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{context::testing::marked, test_fixture::Fixture};

    #[test]
    fn class_exclusion_patterns_are_not_catalog_class_references() {
        let fixture = Fixture::build(&[], vec![]);
        for (reference, exact) in [
            ("/KLib/*", false),
            ("/KLib/Recipe_?.Recipe_?_C", false),
            ("/Game/A.A_C", true),
        ] {
            let source = format!("type: cdo\npatches:\n  - ofClass: /Script/Test.Base\n    excludeClasses: '{reference}|'\n");
            let (text, position) = marked(&source);
            let slot = at(
                Path::new("exclude.cdo.yml"),
                &text,
                position,
                &fixture.reflection,
            );
            assert_eq!(slot.is_some(), exact, "{reference}");
        }
    }

    #[test]
    fn inventory_remaps_constrain_players_and_descriptor_endpoints() {
        let fixture = Fixture::build(
            &[],
            vec![crate::test_support::class(
                "/Script/FactoryGame.FGCharacterPlayer",
                Some("/Script/Engine.Actor"),
                vec![],
                vec![],
            )],
        );
        for (source, expected, concrete) in [
            ("type: cdo\npatches:\n  - target: |\n    applyToSpawnedActors: true\n    inventoryRemap: [{from: Old, to: New}]\n", "/Script/FactoryGame.FGCharacterPlayer", false),
            ("type: cdo\npatches:\n  - target: /Script/FactoryGame.FGCharacterPlayer\n    applyToSpawnedActors: true\n    inventoryRemap:\n      - from: |\n        to: New\n", "/Script/FactoryGame.FGItemDescriptor", true),
            ("type: cdo\npatches:\n  - target: /Script/FactoryGame.FGCharacterPlayer\n    applyToSpawnedActors: true\n    inventoryRemap:\n      - from: Old\n        to: |\n", "/Script/FactoryGame.FGItemDescriptor", true),
        ] {
            let (text, position) = marked(source);
            let slot = at(Path::new("inventory.cdo.yml"), &text, position, &fixture.reflection).expect(source);
            assert_eq!(slot.base_class.as_deref(), Some(expected), "{source}");
            assert_eq!(slot.concrete_only, concrete, "{source}");
        }
    }

    #[test]
    fn actor_actions_constrain_targets_to_actor_classes_without_requiring_concrete_bases() {
        let fixture = Fixture::build(
            &[],
            vec![crate::test_support::class(
                "/Game/Test.ActorChild_C",
                Some("/Script/Engine.Actor"),
                vec![],
                vec![],
            )],
        );
        let (text, position) = marked("type: cdo\npatches:\n  - target: |\n    applyToSpawnedActors: true\n    actorAction: dismantle\n");
        let slot = at(
            Path::new("actions.cdo.yml"),
            &text,
            position,
            &fixture.reflection,
        )
        .unwrap();
        assert_eq!(slot.base_class.as_deref(), Some("/Script/Engine.Actor"));
        assert!(!slot.concrete_only);
        assert!(
            !fixture.reflection.classes.is_empty(),
            "{:?}",
            fixture.reflection.reason
        );
        let items = match completion(slot, &fixture.reflection) {
            CompletionResponse::Array(items) => items,
            CompletionResponse::List(list) => list.items,
        };
        assert!(items
            .iter()
            .any(|item| item.detail.as_deref() == Some("/Game/Test.ActorChild_C")));
        assert!(!items
            .iter()
            .any(|item| item.detail.as_deref() == Some("/Script/FactoryGame.FGItemDescriptor")));
    }

    #[test]
    fn scalar_sequence_and_flow_selector_items_share_one_class_slot() {
        let fixture = Fixture::new();
        for source in [
            "type: cdo\npatches:\n  - target: /Game/|\n",
            "type: cdo\npatches:\n  - target:\n      - /Game/|\n",
            "type: cdo\npatches:\n  - target: [/Game/One, '/Game/|']\n",
            "type: cdo\npatches:\n  - ofClass: /Game/|\n",
            "type: cdo\npatches:\n  - allAssetsOfClass: /Game/|\n",
        ] {
            let (text, position) = marked(source);
            let slot = at(
                Path::new("example.cdo.yml"),
                &text,
                position,
                &fixture.reflection,
            )
            .expect(source);
            assert_eq!(slot.base_class, None);
            assert!(matches!(
                completion(slot, &fixture.reflection),
                CompletionResponse::Array(_) | CompletionResponse::List(_)
            ));
        }
    }

    #[test]
    fn generated_parent_reuses_canonical_default_base_without_duplicate_family_table() {
        let fixture = Fixture::new();
        for (kind, entries, base) in [
            (
                "building",
                "buildings",
                "/Script/FactoryGame.FGBuildingDescriptor",
            ),
            ("item", "items", "/Script/FactoryGame.FGItemDescriptor"),
            (
                "resource",
                "resources",
                "/Script/FactoryGame.FGResourceDescriptor",
            ),
        ] {
            let (text, position) = marked(&format!(
                "type: {kind}\n{entries}:\n  - id: Variant\n    parent: /Game/|\n"
            ));
            let slot = at(
                Path::new("document.yml"),
                &text,
                position,
                &fixture.reflection,
            )
            .unwrap();
            assert_eq!(slot.base_class.as_deref(), Some(base));
        }
    }

    #[test]
    fn arbitrary_strings_comments_and_nonclass_values_have_no_class_slot() {
        let fixture = Fixture::new();
        for source in ["type: cdo\nnotes: /Game/|\n", "type: cdo\n# target: /Game/|\n", "type: cdo\npatches:\n  - target: /Game/Test\n    properties:\n      - path: mCount\n        value: 1|\n"] {
            let (text, position) = marked(source);
            assert!(at(Path::new("document.yml"), &text, position, &fixture.reflection).is_none());
        }
    }

    #[test]
    fn absolute_utf16_offsets_reject_surrogate_split_and_beyond_document() {
        assert_eq!(position_at("😀\n/Path", 3), Some(Position::new(1, 0)));
        assert_eq!(position_at("😀", 1), None);
        assert_eq!(position_at("😀", 3), None);
        assert_eq!(position_at("😀", 2), Some(Position::new(0, 2)));
    }

    #[test]
    fn incomplete_explicit_item_keys_keep_completion_without_fabricating_catalog_proof() {
        let fixture = Fixture::new();
        assert_eq!(
            fixture.reflection.status,
            kdf_validation::ReflectionStatus::Ready
        );
        for key in ["ItemClass", "ingredient", "product"] {
            let (text, position) = marked(&format!("- {key}: /|"));
            let resolved = context::resolve(Path::new("interactive.cdo.yml"), &text, position);
            assert!(from_resolved(&text, &resolved, &fixture.reflection).is_none());
            let slot = for_completion(&text, &resolved, &fixture.reflection)
                .expect("explicit incomplete class key remains useful during typing");
            assert_eq!(
                slot.base_class.as_deref(),
                Some("/Script/FactoryGame.FGItemDescriptor")
            );
            assert!(slot.concrete_only);
        }
    }

    #[test]
    fn exported_terminal_parent_proves_partial_ancestry_but_unknown_leaf_does_not() {
        const BASE: &str = "/Script/Test.UnexportedBase";
        const LEAF: &str = "/Game/Test.KnownLeaf_C";
        let fixture = Fixture::build(
            &[],
            vec![crate::test_support::class(LEAF, Some(BASE), vec![], vec![])],
        );
        assert_eq!(
            fixture.reflection.status,
            kdf_validation::ReflectionStatus::Ready
        );
        assert!(fixture.reflection.class_path(BASE).is_none());
        assert!(!fixture.reflection.is_subclass_of(LEAF, BASE));
        assert!(
            matches_base(&fixture.reflection, LEAF, BASE),
            "exported leaf parent edge is actual evidence"
        );
        assert!(!matches_base(
            &fixture.reflection,
            "/Game/Test.UnknownLeaf_C",
            BASE
        ));
        assert!(!matches_base(
            &fixture.reflection,
            LEAF,
            "/Script/Test.OtherBase"
        ));
    }

    const RECIPE: &str = "/Game/Test.Recipe_ClassSlots_C";

    fn class_property(owner: &str, base: &str) -> serde_json::Value {
        serde_json::json!({
            "name": "mClassReference",
            "ownerPath": owner,
            "kind": "SoftClassProperty",
            "cppType": "TSoftClassPtr<UObject>",
            "metaClass": base
        })
    }

    fn class_property_slot(fixture: &Fixture, targets: &[String]) -> Option<ClassSlot> {
        let targets = targets.join(", ");
        let (text, position) = marked(&format!(
            "type: cdo\npatches:\n  - target: [{targets}]\n    properties:\n      - path: mClassReference\n        value: /Game/|\n"
        ));
        at(
            Path::new("classes.cdo.yml"),
            &text,
            position,
            &fixture.reflection,
        )
    }

    #[test]
    fn class_property_targets_preserve_empty_single_and_limit_boundaries() {
        const BASE: &str = "/Script/FactoryGame.FGItemDescriptor";
        let targets = (0..65)
            .map(|index| format!("/Script/Test.ClassSlotOwner{index}"))
            .collect::<Vec<_>>();
        let fixture = Fixture::build(
            &[],
            targets
                .iter()
                .map(|target| {
                    crate::test_support::class(
                        target,
                        None,
                        vec![class_property(target, BASE)],
                        vec![],
                    )
                })
                .collect(),
        );
        assert_eq!(
            fixture.reflection.status,
            kdf_validation::ReflectionStatus::Ready
        );
        for count in [0, 1, 64, 65] {
            let slot = class_property_slot(&fixture, &targets[..count]);
            if matches!(count, 1 | 64) {
                let slot = slot.expect("agreeing targets within the metadata budget");
                assert_eq!(slot.base_class.as_deref(), Some(BASE), "{count} targets");
                assert!(slot.concrete_only);
            } else {
                assert!(
                    slot.is_none(),
                    "{count} targets must not prove a class slot"
                );
            }
        }
    }

    #[test]
    fn class_property_targets_require_agreeing_constraints_and_available_metadata() {
        const FIRST: &str = "/Script/Test.FirstClassSlotOwner";
        const SECOND: &str = "/Script/Test.SecondClassSlotOwner";
        const BASE: &str = "/Script/FactoryGame.FGItemDescriptor";
        let mut missing_constraint = class_property(SECOND, BASE);
        missing_constraint
            .as_object_mut()
            .unwrap()
            .remove("metaClass");
        for (second_property, agrees) in [
            (Some(class_property(SECOND, BASE)), true),
            (
                Some(class_property(SECOND, "/Script/CoreUObject.Object")),
                false,
            ),
            (Some(missing_constraint), false),
            (None, false),
        ] {
            let fixture = Fixture::build(
                &[],
                vec![
                    crate::test_support::class(
                        FIRST,
                        None,
                        vec![class_property(FIRST, BASE)],
                        vec![],
                    ),
                    crate::test_support::class(
                        SECOND,
                        None,
                        second_property.into_iter().collect(),
                        vec![],
                    ),
                ],
            );
            let mut limit_targets = vec![FIRST.to_owned(); 63];
            limit_targets.push(SECOND.to_owned());
            for targets in [
                vec![FIRST.to_owned(), SECOND.to_owned()],
                vec![SECOND.to_owned(), FIRST.to_owned()],
                limit_targets,
            ] {
                assert_eq!(class_property_slot(&fixture, &targets).is_some(), agrees);
            }
            assert!(class_property_slot(
                &fixture,
                &[FIRST.to_owned(), "/Script/Test.UnavailableOwner".to_owned()],
            )
            .is_none());
        }
    }

    fn recipe_fixture(
        properties: Vec<serde_json::Value>,
        extra: Vec<serde_json::Value>,
    ) -> Fixture {
        let mut recipe = crate::test_support::class(
            RECIPE,
            Some("/Script/FactoryGame.FGRecipe"),
            properties,
            vec![],
        );
        for property in recipe["properties"].as_array_mut().unwrap() {
            property["ownerPath"] = serde_json::json!("/Script/FactoryGame.FGRecipe");
        }
        let fixture = Fixture::build(&[], [vec![recipe], extra].concat());
        assert_eq!(
            fixture.reflection.status,
            kdf_validation::ReflectionStatus::Ready,
            "valid class-slot fixture: {:?}",
            fixture.reflection.reason
        );
        fixture
    }

    fn produced_in_property() -> serde_json::Value {
        serde_json::json!({
            "name":"mProducedIn", "kind":"ArrayProperty", "cppType":"TArray<TSoftClassPtr<UObject>>",
            "inner":{"kind":"SoftClassProperty", "cppType":"TSoftClassPtr<UObject>", "metaClass":"/Script/CoreUObject.Object"}
        })
    }

    #[test]
    fn produced_in_items_filter_actors_using_existing_recipe_constraint() {
        let fixture = recipe_fixture(
            vec![produced_in_property()],
            vec![
                crate::test_support::class(
                    "/Game/Test.Build_Machine_C",
                    Some("/Script/FactoryGame.FGBuildable"),
                    vec![],
                    vec![],
                ),
                crate::test_support::class(
                    "/Game/Test.Desc_Item_C",
                    Some("/Script/FactoryGame.FGItemDescriptor"),
                    vec![],
                    vec![],
                ),
            ],
        );
        for selected in ["/Game/|, /Other", "/Other, '/Game/|'"] {
            let (text, position) = marked(&format!("type: cdo\npatches:\n  - target: {RECIPE}\n    properties:\n      - path: mProducedIn\n        value: [{selected}]\n"));
            let slot = at(
                Path::new("recipe.cdo.yml"),
                &text,
                position,
                &fixture.reflection,
            )
            .unwrap();
            assert_eq!(
                slot.base_class.as_deref(),
                Some("/Script/FactoryGame.FGBuildable")
            );
            assert!(slot.concrete_only);
            let items = match completion(slot, &fixture.reflection) {
                CompletionResponse::Array(items) => items,
                CompletionResponse::List(list) => list.items,
            };
            assert!(items
                .iter()
                .any(|item| item.detail.as_deref() == Some("/Game/Test.Build_Machine_C")));
            assert!(!items
                .iter()
                .any(|item| item.detail.as_deref() == Some("/Game/Test.Desc_Item_C")));
        }
    }

    #[test]
    fn clear_has_no_class_slot_even_inside_item_amount_array() {
        let fixture = recipe_fixture(
            vec![serde_json::json!({
                "name":"mIngredients", "cppType":"TArray<FItemAmount>", "kind":"ArrayProperty",
                "inner":{"kind":"StructProperty", "struct":"/Script/FactoryGame.ItemAmount"}
            })],
            vec![],
        );
        for operation in ["set", "clear"] {
            let (text, position) = marked(&format!("type: cdo\npatches:\n  - target: {RECIPE}\n    properties:\n      - path: mIngredients\n        op: {operation}\n        value: [{{ItemClass: /Game/|}}]\n"));
            let slot = at(
                Path::new("recipe.cdo.yml"),
                &text,
                position,
                &fixture.reflection,
            );
            if operation == "set" {
                let slot =
                    slot.expect("positive FItemAmount ItemClass slot proves available reflection");
                assert_eq!(
                    slot.base_class.as_deref(),
                    Some("/Script/FactoryGame.FGItemDescriptor")
                );
                assert!(slot.concrete_only);
            } else {
                assert!(
                    slot.is_none(),
                    "clear has no value slot, even inside ItemClass"
                );
            }
        }
    }

    #[test]
    fn abstract_descriptor_parent_is_legal_inheritance_but_abstract_machine_is_not_produced_in() {
        const DESCRIPTOR: &str = "/Game/Test.Desc_AbstractBase_C";
        const MACHINE: &str = "/Game/Test.Build_AbstractBase_C";
        let fixture = recipe_fixture(
            vec![produced_in_property()],
            vec![
                crate::test_support::abstract_class(crate::test_support::class(
                    DESCRIPTOR,
                    Some("/Script/FactoryGame.FGItemDescriptor"),
                    vec![],
                    vec![],
                )),
                crate::test_support::abstract_class(crate::test_support::class(
                    MACHINE,
                    Some("/Script/FactoryGame.FGBuildable"),
                    vec![],
                    vec![],
                )),
            ],
        );
        assert!(
            fixture
                .reflection
                .classes
                .iter()
                .find(|class| class.path == DESCRIPTOR)
                .unwrap()
                .abstract_class
        );
        assert!(
            fixture
                .reflection
                .classes
                .iter()
                .find(|class| class.path == MACHINE)
                .unwrap()
                .abstract_class
        );
        let (text, position) = marked("type: item\nitems:\n  - id: Variant\n    parent: |\n");
        let parent = at(Path::new("item.yml"), &text, position, &fixture.reflection).unwrap();
        assert!(!parent.concrete_only);
        let items = match completion(parent, &fixture.reflection) {
            CompletionResponse::Array(items) => items,
            CompletionResponse::List(list) => list.items,
        };
        assert!(items
            .iter()
            .any(|item| item.detail.as_deref() == Some("/Script/FactoryGame.FGItemDescriptor")));
        assert!(
            items
                .iter()
                .any(|item| item.detail.as_deref() == Some(DESCRIPTOR)),
            "abstract descriptor inheritance parent remains valid"
        );
        let (text, position) = marked(&format!("type: cdo\npatches:\n  - target: {RECIPE}\n    properties:\n      - path: mProducedIn\n        value: [|]\n"));
        let machine = at(
            Path::new("recipe.cdo.yml"),
            &text,
            position,
            &fixture.reflection,
        )
        .unwrap();
        assert!(machine.concrete_only);
        let items = match completion(machine, &fixture.reflection) {
            CompletionResponse::Array(items) => items,
            CompletionResponse::List(list) => list.items,
        };
        assert!(
            !items
                .iter()
                .any(|item| item.detail.as_deref() == Some(MACHINE)),
            "abstract machine remains excluded from mProducedIn"
        );
    }
}
