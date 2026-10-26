use kdf_validation::{affected_elements, ReflectionIndex};
use serde_json::{json, Value};
use std::io::Write;

#[test]
fn resource_visuals_do_not_claim_exported_descriptors_are_mesh_actor_instances() {
    let (_file, index) = fixture();
    let result = affected_elements(
        &index,
        "resourcenode",
        &json!({"hideMeshes":["/Game/Mesh.Mesh"]}),
        100,
    );
    assert_eq!(result.status, "limited");
    assert_eq!(result.matched_count, 0);
    assert!(result
        .reasons
        .iter()
        .any(|reason| reason.contains("live node/mesh")));
}

#[test]
fn inventory_remaps_need_live_player_inventory_state() {
    let (_file, index) = fixture();
    let result = affected_elements(
        &index,
        "cdo",
        &json!({
            "target":"/Script/FactoryGame.FGCharacterPlayer",
            "applyToSpawnedActors":true,
            "inventoryRemap":[{"from":"Old","to":"New","multiplier":10}]
        }),
        100,
    );
    assert_eq!(result.status, "limited");
    assert_eq!(result.matched_count, 0);
    assert!(result
        .reasons
        .iter()
        .any(|reason| reason.contains("live world")));
}

fn fixture() -> (tempfile::NamedTempFile, ReflectionIndex) {
    fixture_with_assets(None)
}

#[test]
fn lifecycle_actions_do_not_claim_class_defaults_are_live_actor_matches() {
    let (_file, index) = fixture();
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"target":"/Script/Engine.Actor","applyToSpawnedActors":true,"actorAction":"destroy"}),
        100,
    );
    assert_eq!(result.status, "limited");
    assert!(result.matches.is_empty());
}

#[test]
fn identity_filters_use_exported_kind_and_exclude_only_exact_class() {
    let native = "/Script/Test.Base";
    let parent = "/Game/Parent.Parent_C";
    let child = "/Game/Child.Child_C";
    let (_file, index) = archive(
        &[
            json!({"path":native,"kind":"native","properties":[]}),
            json!({"path":parent,"kind":"blueprint","super":native,"properties":[]}),
            json!({"path":child,"kind":"blueprint","super":parent,"properties":[]}),
        ],
        None,
    );
    for (kind, expected) in [("any", 3), ("native", 1), ("blueprint", 2)] {
        let result = affected_elements(
            &index,
            "cdo",
            &json!({"ofClass":native,"classType":kind,"matchAssets":false}),
            100,
        );
        assert_eq!(result.matched_count, expected, "{kind}: {result:?}");
    }
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"ofClass":native,"classType":"blueprint","matchAssets":false,"excludeClasses":[parent]}),
        100,
    );
    assert_eq!(result.matched_count, 1);
    assert_eq!(result.matches[0].path, child);
}
#[test]
fn class_path_exclusions_cover_owned_namespaces_without_excluding_foreign_children() {
    let base = "/Script/Test.Base";
    let own = "/KLib/Recipe_Own.Recipe_Own_C";
    let ignored = "/Other/Recipe_Ignore.Recipe_Ignore_C";
    let foreign_child = "/FutureMod/Recipe_Child.Recipe_Child_C";
    let (_file, index) = archive(
        &[
            json!({"path":base,"kind":"native","properties":[]}),
            json!({"path":own,"kind":"blueprint","super":base,"properties":[]}),
            json!({"path":ignored,"kind":"blueprint","super":base,"properties":[]}),
            json!({"path":foreign_child,"kind":"blueprint","super":ignored,"properties":[]}),
        ],
        None,
    );
    for pattern in ["/KLib/*", " /klib/Recipe_?wn.* "] {
        let result = affected_elements(
            &index,
            "cdo",
            &json!({"ofClass":base,"classType":"blueprint","matchAssets":false,
                    "excludeClasses":[pattern,ignored]}),
            100,
        );
        assert_eq!(result.matched_count, 1, "{pattern}: {result:?}");
        assert_eq!(result.matches[0].path, foreign_child);
    }
}
fn fixture_with_assets(assets: Option<Value>) -> (tempfile::NamedTempFile, ReflectionIndex) {
    archive(
        &[
            json!({"path":"/Script/FactoryGame.FGRecipe","properties":[]}),
            json!({"path":"/Script/Engine.Actor","properties":[]}),
            json!({"path":"/Game/Build_Base.Build_Base_C","super":"/Script/Engine.Actor","properties":[]}),
            json!({"path":"/Game/Build_Refinery.Build_Refinery_C","super":"/Game/Build_Base.Build_Base_C","properties":[]}),
            recipe(
                "Oil",
                json!(["/Game/Build_Refinery.Build_Refinery_C"]),
                true,
            ),
            recipe("Iron", json!([]), true),
            recipe("Unknown", Value::Null, false),
        ],
        assets,
    )
}
fn archive(classes: &[Value], assets: Option<Value>) -> (tempfile::NamedTempFile, ReflectionIndex) {
    let file = tempfile::Builder::new()
        .suffix(".kdfdata")
        .tempfile()
        .unwrap();
    let mut zip = zip::ZipWriter::new(file.reopen().unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", options).unwrap();
    write!(
        zip,
        "{}",
        json!({"schemaVersion":1,"counts":{"classes":classes.len()},"failedLoads":[]})
    )
    .unwrap();
    zip.start_file("classes.ndjson", options).unwrap();
    for class in classes {
        writeln!(zip, "{class}").unwrap();
    }
    if let Some(assets) = assets {
        zip.start_file("indexes/assets.json", options).unwrap();
        write!(zip, "{assets}").unwrap();
    }
    zip.finish().unwrap();
    let index = ReflectionIndex::load(file.path());
    assert_eq!(
        index.classes.len(),
        classes.len(),
        "{}",
        index.reason.as_deref().unwrap_or("No reason")
    );
    (file, index)
}
fn recipe(name: &str, machines: Value, exported: bool) -> Value {
    let mut properties = vec![
        json!({"name":"mDisplayName","default":name}),
        json!({"name":"mDisplayNameOverride","default":true}),
        json!({"name":"mProduct","default":[{"ItemClass":"/Game/Desc_Oil.Desc_Oil_C","Amount":1}]}),
        json!({"name":"mManufactoringDuration","default":6}),
    ];
    if exported {
        properties.push(json!({"name":"mProducedIn","default":machines}));
    }
    json!({"path":format!("/Game/Recipe_{name}.Recipe_{name}_C"),"super":"/Script/FactoryGame.FGRecipe","properties":properties})
}

#[test]
fn recipe_machine_filter_returns_recipes_and_unknown_defaults_separately() {
    let (_file, index) = fixture();
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"producedIn":"/Game/Build_Refinery.Build_Refinery_C"}),
        100,
    );
    assert_eq!(result.matched_count, 1);
    assert_eq!(
        result
            .matches
            .iter()
            .find(|m| m.certainty == "matched")
            .unwrap()
            .display_name
            .as_deref(),
        Some("Oil")
    );
    assert!(result
        .matches
        .iter()
        .all(|m| !m.path.contains("Build_Refinery")));
    assert!(result.potential_count > 0);
}
#[test]
fn filter_lists_are_or_and_different_filters_are_and() {
    let (_file, index) = fixture();
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"producedIn":["/Game/Missing.Missing_C","/Game/Build_Base.Build_Base_C"],"product":"/Game/Desc_Oil.Desc_Oil_C","matchName":"*Oil*"}),
        100,
    );
    assert_eq!(result.matched_count, 1);
    assert_eq!(result.potential_count, 0);
}
#[test]
fn direct_targets_union_filters_and_subclasses_without_duplicates() {
    let (_file, index) = fixture();
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"target":["/Game/Build_Base.Build_Base_C","/Game/Build_Refinery.Build_Refinery_C"],"applyToSubclasses":true,"producedIn":"/Game/Build_Refinery.Build_Refinery_C"}),
        100,
    );
    assert_eq!(result.matched_count, 3);
}
#[test]
fn numeric_where_and_wildcard_name_use_exported_defaults() {
    let (_file, index) = fixture();
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"ofClass":"/Script/FactoryGame.FGRecipe","matchName":["*oil*","*iron*"],"where":{"path":"mManufactoringDuration","greaterThan":5}}),
        100,
    );
    assert_eq!(result.matched_count, 2);
    assert_eq!(result.potential_count, 0);
}
#[test]
fn incomplete_export_never_reports_proven_empty() {
    let result = affected_elements(
        &ReflectionIndex::unavailable("No bound data"),
        "cdo",
        &json!({"producedIn":"/Game/Build_Refinery.Build_Refinery_C"}),
        100,
    );
    assert_ne!(result.status, "ready");
    assert!(!result.reasons.is_empty());
}
#[test]
fn unresolved_direct_target_and_runtime_conditions_stay_visible() {
    let (_file, index) = fixture();
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"target":"/Game/NotExported.NotExported_C","conditions":{"hasMod":"X"}}),
        100,
    );
    assert_eq!(result.potential_count, 1);
    assert_eq!(result.matches[0].certainty, "potential");
}
#[test]
fn results_are_bounded_but_counts_cover_whole_selection() {
    let (_file, index) = fixture();
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"target":"/Script/FactoryGame.FGRecipe","applyToSubclasses":true}),
        1,
    );
    assert_eq!(result.matched_count, 4);
    assert_eq!(result.matches.len(), 1);
    assert!(result.truncated);
}
#[test]
fn of_class_without_filter_is_not_a_target() {
    let (_file, index) = fixture();
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"ofClass":"/Script/FactoryGame.FGRecipe"}),
        100,
    );
    assert_eq!(result.matched_count, 0);
    assert_eq!(result.status, "unsupported");
}

fn asset_index() -> Value {
    json!({
        "/Script/FactoryGame.FGRecipe": {
            "assets": [
                {"path":"/Game/Assets/DA_Iron.DA_Iron", "assetName":"DA_Iron", "assetClass":"/Script/FactoryGame.FGRecipe"},
                {"path":"/Game/Assets/DA_Oil.DA_Oil", "assetName":"DA_Oil", "assetClass":"/Script/FactoryGame.FGRecipe"},
                {"path":"/Game/Assets/DA_Oil_Child.DA_Oil_Child", "assetClass":"/Game/Recipe_Oil.Recipe_Oil_C"},
                {"path":"", "assetName":"", "assetClass":"/Script/FactoryGame.FGRecipe"}
            ]
        },
        "/Game/Build_Refinery.Build_Refinery_C": {
            "assets": [{"path":"/Game/Assets/DA_Build.DA_Build", "assetClass":"/Game/Build_Refinery.Build_Refinery_C"}]
        },
        "/Game/Build_Base.Build_Base_C": {
            "assets": []
        }
    })
}

#[test]
fn unknown_asset_scope_is_limited_even_with_unrelated_asset_index() {
    let (_file, index) = fixture_with_assets(Some(asset_index()));
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"allAssetsOfClass":"/Script/Missing.Unexported"}),
        100,
    );
    assert_eq!(result.status, "limited");
    assert!(!result.reasons.is_empty());
    assert_eq!((result.matched_count, result.potential_count), (0, 0));
}

#[test]
fn exported_asset_scope_resolves_alias_and_inherited_classes() {
    let (_file, index) = fixture_with_assets(Some(asset_index()));
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"allAssetsOfClass":"build_base"}),
        100,
    );
    assert_eq!(result.status, "ready");
    assert_eq!((result.matched_count, result.potential_count), (1, 0));
    assert_eq!(result.matches[0].path, "/Game/Assets/DA_Build.DA_Build");
}

#[test]
fn exported_asset_scope_without_relevant_instance_index_is_limited() {
    let (_file, index) = fixture_with_assets(Some(json!({
        "/Game/Build_Refinery.Build_Refinery_C": {
            "assets": [{"path":"/Game/Assets/DA_Build.DA_Build", "assetClass":"/Game/Build_Refinery.Build_Refinery_C"}]
        }
    })));
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"allAssetsOfClass":"/Script/FactoryGame.FGRecipe"}),
        100,
    );
    assert_eq!(result.status, "limited");
    assert!(!result.reasons.is_empty());
    assert_eq!(result.matched_count, 0);
}

#[test]
fn asset_name_exclusion_is_proven_even_when_instance_defaults_are_missing() {
    let (_file, index) = fixture_with_assets(Some(asset_index()));
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"ofClass":"FGRecipe", "matchName":"*Oil*", "where":{"path":"mManufactoringDuration", "greaterThan":5}}),
        100,
    );
    assert_eq!(result.matched_count, 1);
    assert_eq!(result.potential_count, 3);
    assert!(!result
        .matches
        .iter()
        .any(|item| item.path.contains("DA_Iron")));
    assert!(result
        .matches
        .iter()
        .filter(|item| item.kind == "asset")
        .all(|item| item.certainty == "potential"));
}

#[test]
fn name_only_asset_filters_use_object_identity_not_class_name() {
    let (_file, index) = fixture_with_assets(Some(asset_index()));
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"ofClass":"FGRecipe", "matchName":["da_o?l", "/game/assets/da_oil_child.*"]}),
        100,
    );
    assert_eq!((result.matched_count, result.potential_count), (2, 1));
    assert!(result
        .matches
        .iter()
        .filter(|item| item.certainty == "matched")
        .all(|item| item.kind == "asset"));
    assert!(result
        .matches
        .iter()
        .any(|item| item.path.is_empty() && item.certainty == "potential"));
}

#[test]
fn asset_and_class_filters_respect_no_asset_switch_and_full_counts() {
    let (_file, index) = fixture_with_assets(Some(asset_index()));
    let entry = json!({"ofClass":"FGRecipe", "matchName":"*Oil*"});
    let result = affected_elements(&index, "cdo", &entry, 1);
    assert_eq!((result.matched_count, result.potential_count), (3, 1));
    assert_eq!(result.matches.len(), 1);
    assert!(result.truncated);
    let mut entry = entry;
    entry["matchAssets"] = json!(false);
    let result = affected_elements(&index, "cdo", &entry, 100);
    assert_eq!((result.matched_count, result.potential_count), (1, 0));
    assert_eq!(result.matches[0].kind, "class");
}

#[test]
fn missing_asset_index_cannot_prove_asset_coverage() {
    let (_file, index) = fixture();
    let result = affected_elements(&index, "cdo", &json!({"allAssetsOfClass":"FGRecipe"}), 100);
    assert_eq!(result.status, "limited");
    assert!(!result.reasons.is_empty());
    assert_eq!(result.matched_count, 0);
}

#[test]
fn malformed_asset_scope_never_claims_proven_empty() {
    let (_file, index) = fixture_with_assets(Some(asset_index()));
    for selector in [
        json!(null),
        json!(""),
        json!("  "),
        json!([]),
        json!(["FGRecipe"]),
        json!({"class":"FGRecipe"}),
        json!(true),
    ] {
        let result = affected_elements(&index, "cdo", &json!({"allAssetsOfClass":selector}), 100);
        assert_eq!(result.status, "limited");
        assert!(!result.reasons.is_empty());
        assert_eq!((result.matched_count, result.potential_count), (0, 0));
    }
}

#[test]
fn noncanonical_asset_scope_and_direct_target_union_preserve_exported_identity() {
    let (_file, index) = fixture_with_assets(Some(asset_index()));
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"allAssetsOfClass":"Class'/game/build_base.build_base_c'", "target":"/Game/Assets/DA_Build"}),
        100,
    );
    assert_eq!(result.status, "ready");
    assert_eq!((result.matched_count, result.potential_count), (1, 0));
    assert_eq!(result.matches[0].path, "/Game/Assets/DA_Build.DA_Build");
    assert_eq!(
        result.matches[0].class_path.as_deref(),
        Some("/Game/Build_Refinery.Build_Refinery_C")
    );
}

#[test]
fn invalid_asset_scope_keeps_valid_direct_target_and_unknown_direct_target_separate() {
    let (_file, index) = fixture_with_assets(Some(asset_index()));
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"allAssetsOfClass":[], "target":["/Game/Build_Base.Build_Base_C", "/Game/RuntimeOnly.RuntimeOnly_C"]}),
        100,
    );
    assert_eq!(result.status, "limited");
    assert_eq!((result.matched_count, result.potential_count), (1, 1));
    assert_eq!(
        result
            .matches
            .iter()
            .find(|entry| entry.kind == "unknown")
            .unwrap()
            .class_path,
        None
    );
}

#[test]
fn asset_identity_matches_remain_possible_under_runtime_conditions() {
    let (_file, index) = fixture_with_assets(Some(asset_index()));
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"ofClass":"FGRecipe", "matchName":"DA_Oil", "conditions":{"hasMod":"MaybeLoaded"}}),
        100,
    );
    assert_eq!(result.status, "limited");
    assert_eq!((result.matched_count, result.potential_count), (0, 2));
    assert!(result
        .matches
        .iter()
        .all(|entry| entry.certainty == "potential"));
}

// Real game exports store every default as Unreal ImportText. Shapes below are copied from the
// vanilla archive's classes.flat.ndjson (Recipe_Alternate_CoatedCable and its siblings).
const REFINERY: &str =
    "/Game/FactoryGame/Buildable/Factory/OilRefinery/Build_OilRefinery.Build_OilRefinery_C";
const CABLE: &str = "/Game/FactoryGame/Resource/Parts/Cable/Desc_Cable.Desc_Cable_C";

fn exported(name: &str, kind: &str, inner: Option<&str>, default: &str) -> Value {
    let mut property = json!({"name":name, "authoredName":name, "kind":kind, "default":default, "defaultVolatile":false});
    if let Some(inner) = inner {
        property["inner"] = json!({ "kind": inner });
    }
    property
}
fn exported_recipe(name: &str, produced_in: &str, display_name: &str) -> Value {
    json!({
        "path": format!("/Game/Recipes/{name}.{name}_C"),
        "super": "/Script/FactoryGame.FGRecipe",
        "properties": [
            exported("mDisplayName", "TextProperty", None, display_name),
            exported("mProducedIn", "ArrayProperty", Some("SoftClassProperty"), produced_in),
            exported(
                "mProduct",
                "ArrayProperty",
                Some("StructProperty"),
                &format!(r#"((ItemClass="/Script/Engine.BlueprintGeneratedClass'{CABLE}'",amount=9))"#),
            ),
            exported("mManufactoringDuration", "FloatProperty", None, "8.000000"),
            exported("mDisplayNameOverride", "BoolProperty", None, "True"),
        ]
    })
}
fn import_text_fixture(assets: Value) -> (tempfile::NamedTempFile, ReflectionIndex) {
    let mut volatile = exported_recipe(
        "Recipe_Volatile",
        &format!(r#"("{REFINERY}")"#),
        r#"INVTEXT("Volatile")"#,
    );
    volatile["properties"][3]["defaultVolatile"] = json!(true);
    // The native base exports empty lists, so it is a proven non-match rather than unknown.
    let base = json!({
        "path": "/Script/FactoryGame.FGRecipe",
        "properties": [
            exported("mProducedIn", "ArrayProperty", Some("SoftClassProperty"), "()"),
            exported("mProduct", "ArrayProperty", Some("StructProperty"), "()"),
            exported("mManufactoringDuration", "FloatProperty", None, "1.000000"),
        ]
    });
    archive(
        &[
            base,
            json!({"path":REFINERY,"properties":[]}),
            json!({"path":CABLE,"properties":[]}),
            exported_recipe(
                "Recipe_Alternate_CoatedCable",
                &format!(r#"("{REFINERY}")"#),
                r#"LOCTABLE("Items_Data", "Parts/CableCoated")"#,
            ),
            exported_recipe("Recipe_Cleared", "(None)", r#"INVTEXT("Cleared Machine")"#),
            // Unreal exports an empty array as no text at all.
            exported_recipe("Recipe_NoMachine", "", ""),
            volatile,
        ],
        Some(assets),
    )
}
fn proven_empty_recipe_assets() -> Value {
    let mut assets = json!({
        "/Script/FactoryGame.FGRecipe": {"assets": [], "total": 0, "truncated": false}
    });
    for name in [
        "Recipe_Alternate_CoatedCable",
        "Recipe_Cleared",
        "Recipe_NoMachine",
        "Recipe_Volatile",
    ] {
        assets[format!("/Game/Recipes/{name}.{name}_C")] =
            json!({"assets": [], "total": 0, "truncated": false});
    }
    assets
}

#[test]
fn import_text_produced_in_selects_real_refinery_recipes() {
    let (_file, index) = import_text_fixture(proven_empty_recipe_assets());
    let result = affected_elements(&index, "cdo", &json!({ "producedIn": REFINERY }), 100);
    assert_eq!(result.status, "ready", "{:?}", result.reasons);
    assert_eq!((result.matched_count, result.potential_count), (2, 0));
    assert!(!result
        .matches
        .iter()
        .any(|m| m.path.contains("Recipe_Cleared") || m.path.contains("Recipe_NoMachine")));
}

#[test]
fn import_text_product_and_numeric_where_are_decoded() {
    let (_file, index) = import_text_fixture(proven_empty_recipe_assets());
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"product":CABLE, "matchName":"Recipe_Alternate_*", "where":{"path":"mManufactoringDuration","greaterThan":5}}),
        100,
    );
    assert_eq!((result.matched_count, result.potential_count), (1, 0));
    assert_eq!(
        result.matches[0].path,
        "/Game/Recipes/Recipe_Alternate_CoatedCable.Recipe_Alternate_CoatedCable_C"
    );
}

#[test]
fn volatile_import_text_default_stays_possible() {
    let (_file, index) = import_text_fixture(proven_empty_recipe_assets());
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"ofClass":"FGRecipe", "matchName":"Recipe_Volatile", "where":{"path":"mManufactoringDuration","lessThan":5}}),
        100,
    );
    assert_eq!(result.status, "limited");
    assert_eq!((result.matched_count, result.potential_count), (0, 1));
}

#[test]
fn display_names_use_invariant_text_but_never_string_table_keys() {
    let (_file, index) = import_text_fixture(proven_empty_recipe_assets());
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"ofClass":"FGRecipe", "matchName":["Recipe_Alternate_CoatedCable", "Recipe_Cleared"]}),
        100,
    );
    let names = result
        .matches
        .iter()
        .map(|m| (m.name.as_str(), m.display_name.as_deref()))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            ("Recipe_Alternate_CoatedCable_C", None),
            ("Recipe_Cleared_C", Some("Cleared Machine"))
        ]
    );
}

const IONIZED_RECIPE: &str =
    "/Game/FactoryGame/Recipes/OilRefinery/Recipe_IonizedFuel.Recipe_IonizedFuel_C";
const IONIZED_ITEM: &str =
    "/Game/FactoryGame/Resource/Parts/IonizedFuel/Desc_IonizedFuel.Desc_IonizedFuel_C";
const SECOND_PRODUCT: &str = "/Game/Desc_Second.Desc_Second_C";

// Reduced ImportText shapes verified against classes.flat.ndjson in vanilla.kdfdata,
// SHA-256 69c84e99109222d84f029f2fb77923f7f46353a0c5ae172ac2bb1354d7d98694.
// The real descriptor uses unresolved LOCTABLE; readable labels and second product below are
// synthetic variants. The recipe rule is approved semantics, not proof from executable game code.
fn display_recipe(override_default: &str) -> Value {
    let mut override_property = exported(
        "mDisplayNameOverride",
        "BoolProperty",
        None,
        override_default,
    );
    override_property["isDefaultFromSuper"] = json!(override_default.is_empty());
    json!({
        "path": IONIZED_RECIPE,
        "super": "/Script/FactoryGame.FGRecipe",
        "firstNativeAncestor": "/Script/FactoryGame.FGRecipe",
        "properties": [
            override_property,
            exported("mDisplayName", "TextProperty", None,
                r#"NSLOCTEXT("", "34272A29446E4804EA516192520E5AE1", "Truck Chassis")"#),
            exported("mProduct", "ArrayProperty", Some("StructProperty"),
                &format!(r#"((ItemClass="/Script/Engine.BlueprintGeneratedClass'{IONIZED_ITEM}'",amount=16000),(ItemClass="/Script/Engine.BlueprintGeneratedClass'{SECOND_PRODUCT}'",amount=2))"#)),
        ]
    })
}

fn display_descriptor(display_name: &str) -> Value {
    json!({
        "path": IONIZED_ITEM,
        "super": "/Script/FactoryGame.FGItemDescriptor",
        "properties": [exported("mDisplayName", "TextProperty", None, display_name)]
    })
}

fn display_fixture(
    recipe: Value,
    descriptor: Option<Value>,
) -> (tempfile::NamedTempFile, ReflectionIndex) {
    let mut classes = vec![
        json!({"path":"/Script/FactoryGame.FGRecipe","properties":[]}),
        json!({"path":"/Script/FactoryGame.FGItemDescriptor","properties":[]}),
        json!({"path":SECOND_PRODUCT,"properties":[
            exported("mDisplayName", "TextProperty", None, r#"INVTEXT("Second product")"#)
        ]}),
        recipe,
    ];
    classes.extend(descriptor);
    archive(&classes, None)
}

fn display_name(index: &ReflectionIndex, target: &str) -> Option<String> {
    let result = affected_elements(index, "cdo", &json!({"target":target}), 100);
    assert_eq!(result.matches.len(), 1);
    result.matches[0].display_name.clone()
}

#[test]
fn recipe_false_or_empty_bool_uses_first_product_display_name() {
    for override_default in ["False", ""] {
        let (_file, index) = display_fixture(
            display_recipe(override_default),
            Some(display_descriptor(
                r#"LOCTEXT("IonizedFuel", "Ionized Fuel")"#,
            )),
        );
        assert_eq!(
            display_name(&index, IONIZED_RECIPE).as_deref(),
            Some("Ionized Fuel")
        );
    }
}

#[test]
fn recipe_true_override_uses_own_display_name_without_product_metadata() {
    let mut recipe = display_recipe("True");
    recipe["properties"]
        .as_array_mut()
        .unwrap()
        .retain(|p| p["name"] != "mProduct");
    let (_file, index) = display_fixture(recipe, None);
    assert_eq!(
        display_name(&index, IONIZED_RECIPE).as_deref(),
        Some("Truck Chassis")
    );
}

#[test]
fn recipe_child_inherits_classification_and_first_product_defaults() {
    let child = "/Game/Recipe_Child.Recipe_Child_C";
    let mut classes = vec![
        json!({"path":"/Script/FactoryGame.FGRecipe","properties":[]}),
        display_recipe(""),
        display_descriptor(r#"INVTEXT("Inherited product")"#),
        json!({"path":child,"super":IONIZED_RECIPE,"properties":[]}),
    ];
    // No native ancestry shortcut on the child: classification follows the exported super chain.
    let (_file, index) = archive(&classes, None);
    assert_eq!(
        display_name(&index, child).as_deref(),
        Some("Inherited product")
    );
    classes[3]["properties"] = json!([exported(
        "mDisplayNameOverride",
        "BoolProperty",
        None,
        "Unknown"
    )]);
    let (_file, index) = archive(&classes, None);
    assert_eq!(display_name(&index, child), None);
}

#[test]
fn non_recipe_keeps_own_display_name_even_with_false_override() {
    let mut class = display_recipe("False");
    class["super"] = json!("/Script/FactoryGame.FGItemDescriptor");
    class["firstNativeAncestor"] = class["super"].clone();
    let (_file, index) = display_fixture(
        class,
        Some(display_descriptor(r#"INVTEXT("Unrelated product")"#)),
    );
    assert_eq!(
        display_name(&index, IONIZED_RECIPE).as_deref(),
        Some("Truck Chassis")
    );
}

#[test]
fn recipe_missing_or_unresolved_first_product_never_uses_own_or_second_product_name() {
    for products in [
        None,
        Some(json!("")),
        Some(json!("()")),
        Some(json!("((amount=1))")),
        Some(json!(format!(
            r#"((ItemClass=None,amount=1),(ItemClass="{SECOND_PRODUCT}",amount=2))"#
        ))),
        Some(json!(format!(
            r#"((ItemClass="/Game/Missing.Missing_C",amount=1),(ItemClass="{SECOND_PRODUCT}",amount=2))"#
        ))),
    ] {
        let mut recipe = display_recipe("False");
        if let Some(products) = products {
            recipe["properties"][2]["default"] = products;
        } else {
            recipe["properties"].as_array_mut().unwrap().pop();
        }
        let (_file, index) = display_fixture(recipe, None);
        assert_eq!(display_name(&index, IONIZED_RECIPE), None);
    }
    let (_file, index) = display_fixture(display_recipe("False"), None);
    assert_eq!(display_name(&index, IONIZED_RECIPE), None);
}

#[test]
fn unresolved_recipe_product_string_table_preserves_class_name_fallback() {
    let (_file, index) = display_fixture(
        display_recipe(""),
        Some(display_descriptor(
            r#"LOCTABLE("Items_Data", "Materials/IonizedFuel")"#,
        )),
    );
    let result = affected_elements(&index, "cdo", &json!({"target":IONIZED_RECIPE}), 100);
    assert_eq!(result.matches[0].display_name, None);
    assert_eq!(result.matches[0].name, "Recipe_IonizedFuel_C");
}

#[test]
fn unknown_recipe_override_defaults_do_not_fabricate_display_names() {
    let unknown_properties = [
        None,
        Some(exported(
            "mDisplayNameOverride",
            "BoolProperty",
            None,
            "Unknown",
        )),
        Some(json!({"name":"mDisplayNameOverride","kind":"BoolProperty","default":null})),
        Some(json!({"name":"mDisplayNameOverride","kind":"BoolProperty"})),
        Some(
            json!({"name":"mDisplayNameOverride","kind":"BoolProperty","default":"","defaultVolatile":true}),
        ),
        Some(exported("mDisplayNameOverride", "StrProperty", None, "")),
        Some(json!({"name":"mDisplayNameOverride","default":""})),
        Some(json!({"name":"mDisplayNameOverride","default":"False"})),
    ];
    for unknown in unknown_properties {
        let mut recipe = display_recipe("False");
        let properties = recipe["properties"].as_array_mut().unwrap();
        properties.remove(0);
        properties.extend(unknown);
        let (_file, index) = display_fixture(
            recipe,
            Some(display_descriptor(r#"INVTEXT("Known product")"#)),
        );
        assert_eq!(display_name(&index, IONIZED_RECIPE), None);
    }
}

#[test]
fn indexed_class_without_assets_proves_asset_scope_empty() {
    let (_file, index) = import_text_fixture(proven_empty_recipe_assets());
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"allAssetsOfClass":"/Script/FactoryGame.FGRecipe"}),
        100,
    );
    assert_eq!(result.status, "ready", "{:?}", result.reasons);
    assert_eq!((result.matched_count, result.potential_count), (0, 0));
}

#[test]
fn missing_assets_field_never_proves_asset_scope_empty() {
    for listing in [json!({}), json!({"total": 0, "truncated": false})] {
        let (_file, index) = import_text_fixture(json!({
            "/Script/FactoryGame.FGRecipe": listing
        }));
        assert_eq!(index.asset_listing_complete("FGRecipe"), Some(false));
        let result = affected_elements(
            &index,
            "cdo",
            &json!({"allAssetsOfClass":"/Script/FactoryGame.FGRecipe"}),
            100,
        );
        assert_eq!(result.status, "limited");
        assert_eq!((result.matched_count, result.potential_count), (0, 0));
        assert!(result
            .reasons
            .iter()
            .any(|reason| reason.contains("only part")));
    }
}

#[test]
fn legacy_explicit_empty_assets_preserves_complete_scope() {
    let mut assets = proven_empty_recipe_assets();
    assets["/Script/FactoryGame.FGRecipe"] = json!({"assets": []});
    let (_file, index) = import_text_fixture(assets);
    assert_eq!(index.asset_listing_complete("FGRecipe"), Some(true));
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"allAssetsOfClass":"/Script/FactoryGame.FGRecipe"}),
        100,
    );
    assert_eq!(result.status, "ready", "{:?}", result.reasons);
    assert_eq!((result.matched_count, result.potential_count), (0, 0));
}

#[test]
fn truncated_asset_listing_never_claims_complete_scope() {
    let (_file, index) = import_text_fixture(json!({
        "/Script/FactoryGame.FGRecipe": {
            "assets": [{"path":"/Game/Assets/DA_One.DA_One", "assetClass":"/Script/FactoryGame.FGRecipe"}],
            "total": 2,
            "truncated": true
        }
    }));
    let result = affected_elements(
        &index,
        "cdo",
        &json!({"allAssetsOfClass":"/Script/FactoryGame.FGRecipe"}),
        100,
    );
    assert_eq!(result.status, "limited");
    assert_eq!(result.matched_count, 1);
    assert!(result
        .reasons
        .iter()
        .any(|reason| reason.contains("only part")));
}

#[test]
fn asset_listing_total_must_equal_rows_even_when_not_truncated() {
    let class = "/Script/Engine.DataAsset";
    for row_count in 0..=2 {
        for total in [None, Some(0), Some(1), Some(2)] {
            for truncated in [false, true] {
                let rows = (0..row_count)
                    .map(|number| {
                        json!({"path":format!("/Game/Assets/DA_{number}.DA_{number}"), "assetClass":class})
                    })
                    .collect::<Vec<_>>();
                let mut listing = json!({"assets":rows,"truncated":truncated});
                if let Some(total) = total {
                    listing["total"] = json!(total);
                }
                let (_file, index) = archive(
                    &[json!({"path":class,"properties":[]})],
                    Some(json!({class:listing})),
                );
                let complete = !truncated && total.is_none_or(|total| total == row_count);
                assert_eq!(
                    index.asset_listing_complete(class),
                    Some(complete),
                    "rows={row_count}, total={total:?}, truncated={truncated}"
                );
                let result =
                    affected_elements(&index, "cdo", &json!({"allAssetsOfClass":class}), 100);
                assert_eq!(result.status, if complete { "ready" } else { "limited" });
                assert_eq!(result.matched_count, row_count);
            }
        }
    }
}

#[test]
fn recursive_base_bucket_proves_asset_scope_completeness_without_child_buckets() {
    let paths = [
        "/Script/Test.Base",
        "/Script/Test.Child",
        "/Script/Test.Grandchild",
    ];
    let classes = [
        json!({"path":paths[0],"properties":[]}),
        json!({"path":paths[1],"super":paths[0],"properties":[]}),
        json!({"path":paths[2],"super":paths[1],"properties":[]}),
    ];
    for present in 0..8 {
        for incomplete in [None, Some(0), Some(1), Some(2)] {
            let mut assets = json!({});
            for (position, path) in paths.iter().enumerate() {
                if present & (1 << position) != 0 {
                    assets[*path] = json!({
                        "assets":[], "total":0, "truncated":incomplete == Some(position)
                    });
                }
            }
            let (_file, index) = archive(&classes, Some(assets));
            for entry in [
                json!({"allAssetsOfClass":paths[0]}),
                json!({"ofClass":paths[0],"matchName":"DoesNotMatch"}),
            ] {
                let result = affected_elements(&index, "cdo", &entry, 100);
                assert_eq!(
                    result.status,
                    if present & 1 != 0 && incomplete != Some(0) {
                        "ready"
                    } else {
                        "limited"
                    },
                    "present={present:03b}, incomplete={incomplete:?}, entry={entry}"
                );
                assert_eq!((result.matched_count, result.potential_count), (0, 0));
            }
        }
    }
}

#[test]
fn missing_asset_name_uses_subobject_name_like_unreal_object_path_to_object_name() {
    let class = "/Script/Test.Asset";
    let path = "/Game/Assets/DA.DA:Inner";
    let (_file, index) = archive(
        &[json!({"path":class,"properties":[]})],
        Some(json!({class:{"assets":[{"path":path,"assetClass":class}],"total":1}})),
    );
    assert_eq!(index.asset_listing_complete(class), Some(true));
    assert_eq!(index.find_asset(path).unwrap().name, "Inner");
    for (name, count) in [("Inner", 1), ("DA", 0)] {
        let result = affected_elements(
            &index,
            "cdo",
            &json!({"ofClass":class,"matchName":name}),
            100,
        );
        assert_eq!(result.status, "ready", "{:?}", result.reasons);
        assert_eq!((result.matched_count, result.potential_count), (count, 0));
    }
}

#[test]
fn asset_name_must_agree_with_object_identity_before_proving_name_exclusion() {
    let class = "/Script/Test.Asset";
    for (path, canonical_name) in [
        ("/Game/Assets/DA_Test.DA_Test", "DA_Test"),
        ("/Game/Assets/DA_Test.DA_Test:Nested", "Nested"),
        ("/Game/Map.Map:PersistentLevel.Actor.Component", "Component"),
    ] {
        for (name, verified) in [
            (None, true),
            (Some(canonical_name.to_owned()), true),
            (Some(canonical_name.to_lowercase()), true),
            (Some("WrongName".to_owned()), false),
            (Some(String::new()), false),
        ] {
            let mut asset = json!({"path":path,"assetClass":class});
            if let Some(name) = name {
                asset["assetName"] = json!(name);
            }
            let (_file, index) = archive(
                &[json!({"path":class,"properties":[]})],
                Some(json!({class:{"assets":[asset],"total":1}})),
            );
            assert_eq!(
                index.asset_listing_complete(class),
                Some(verified),
                "{asset}"
            );
            for entry in [
                json!({"allAssetsOfClass":class}),
                json!({"target":path}),
                json!({"ofClass":class,"matchName":canonical_name}),
            ] {
                let result = affected_elements(&index, "cdo", &entry, 100);
                assert_eq!(
                    result.status,
                    if verified { "ready" } else { "limited" },
                    "{asset}: {entry}"
                );
                assert_eq!(
                    (result.matched_count, result.potential_count),
                    if verified { (1, 0) } else { (0, 1) },
                    "{asset}: {entry}"
                );
            }
        }
    }
}

#[test]
fn blank_asset_paths_never_prove_selection_or_empty_scope() {
    let class = "/Script/Test.Asset";
    for path in ["", " ", "\t", "\r\n"] {
        let (_file, index) = archive(
            &[json!({"path":class,"properties":[]})],
            Some(json!({class:{"assets":[{"path":path,"assetClass":class}],"total":1}})),
        );
        assert_eq!(index.asset_listing_complete(class), Some(false), "{path:?}");
        for entry in [
            json!({"allAssetsOfClass":class}),
            json!({"ofClass":class,"matchName":"DoesNotMatch"}),
        ] {
            let result = affected_elements(&index, "cdo", &entry, 100);
            assert_eq!(result.status, "limited", "{path:?}: {entry}");
            assert_eq!((result.matched_count, result.potential_count), (0, 1));
            assert!(result
                .reasons
                .iter()
                .any(|reason| reason.contains("only part")));
        }
    }
}

#[test]
fn duplicate_canonical_asset_paths_never_prove_complete_scope() {
    let class = "/Script/Test.Asset";
    let path = "/Game/Assets/DA_Test.DA_Test";
    for duplicate in [path.to_owned(), path.to_lowercase()] {
        for total in [None, Some(2)] {
            let mut listing = json!({"assets":[
                {"path":path,"assetClass":class},
                {"path":duplicate,"assetClass":class}
            ]});
            if let Some(total) = total {
                listing["total"] = json!(total);
            }
            let (_file, index) = archive(
                &[json!({"path":class,"properties":[]})],
                Some(json!({class:listing})),
            );
            assert_eq!(index.asset_listing_complete(class), Some(false));
            for entry in [
                json!({"allAssetsOfClass":class}),
                json!({"ofClass":class,"matchName":"DoesNotMatch"}),
            ] {
                let result = affected_elements(&index, "cdo", &entry, 100);
                assert_eq!(result.status, "limited", "{duplicate}: {entry}");
                assert!(result
                    .reasons
                    .iter()
                    .any(|reason| reason.contains("only part")));
            }
        }
    }
}

#[test]
fn recursive_base_assets_include_child_identities_once_with_optional_child_buckets() {
    let base = "/Script/Test.Base";
    let child = "/Script/Test.Child";
    let grandchild = "/Script/Test.Grandchild";
    let child_path = "/Game/Assets/DA_Child.DA_Child";
    let grandchild_path = "/Game/Assets/DA_Grandchild.DA_Grandchild";
    let child_asset = json!({"path":child_path,"assetClass":child});
    let grandchild_asset = json!({"path":grandchild_path,"assetClass":grandchild});
    let classes = [
        json!({"path":base,"properties":[]}),
        json!({"path":child,"super":base,"properties":[]}),
        json!({"path":grandchild,"super":child,"properties":[]}),
    ];
    for child_buckets in [false, true] {
        let mut assets = json!({base:{"assets":[child_asset,grandchild_asset],"total":2}});
        if child_buckets {
            assets[child] = json!({"assets":[child_asset,grandchild_asset],"total":2});
            assets[grandchild] = json!({"assets":[grandchild_asset],"total":1});
        }
        let (_file, index) = archive(&classes, Some(assets));
        for entry in [
            json!({"allAssetsOfClass":base}),
            json!({"ofClass":base,"matchName":"DA_*"}),
        ] {
            let result = affected_elements(&index, "cdo", &entry, 100);
            assert_eq!(result.status, "ready", "{:?}", result.reasons);
            assert_eq!((result.matched_count, result.potential_count), (2, 0));
            assert_eq!(result.matches.len(), 2);
            assert!(result.matches.iter().any(|asset| asset.path == child_path));
            assert!(result
                .matches
                .iter()
                .any(|asset| asset.path == grandchild_path));
            assert!(result
                .matches
                .iter()
                .all(|asset| asset.class_path.as_deref()
                    == Some(if asset.path == child_path {
                        child
                    } else {
                        grandchild
                    })));
        }
    }
}

#[test]
fn malformed_nonempty_asset_paths_never_prove_selection_or_empty_scope() {
    let class = "/Script/Test.Asset";
    for path in [
        "garbage",
        "Game/Folder/DA.DA",
        "/Game/Folder/DA",
        "/Game/DA.",
        "/Game/.DA",
        "/Game//DA.DA",
        "/Game/DA.DA/Other",
        "/Game/DA.DA.Other",
        "/Game/DA.DA:",
        "/Game/DA.DA:Outer..Inner",
        "/Game/DA.DA:Outer:Inner",
        "/Game/Bad Name.DA",
        "/Game/DA.Bad Name",
        "/Game/DA.DA\n",
        "DataAsset'/Game/DA.DA'",
        "/Game/DA*.DA",
        "/Game/DA.DA\\Other",
    ] {
        let (_file, index) = archive(
            &[json!({"path":class,"properties":[]})],
            Some(json!({class:{"assets":[{"path":path,"assetClass":class}],"total":1}})),
        );
        assert_eq!(index.asset_listing_complete(class), Some(false), "{path}");
        for entry in [
            json!({"allAssetsOfClass":class}),
            json!({"ofClass":class,"matchName":"DoesNotMatch"}),
            json!({"target":path}),
        ] {
            let result = affected_elements(&index, "cdo", &entry, 100);
            assert_eq!(result.status, "limited", "{path}: {entry}");
            assert_eq!(result.matched_count, 0, "{path}: {entry}");
            assert_eq!(result.potential_count, 1, "{path}: {entry}");
        }
    }
}

#[test]
fn stable_unreal_asset_path_boundaries_keep_complete_scope() {
    let class = "/Script/Test.Asset";
    for path in [
        "/Game/FactoryGame/-Shared/DA-Iron.DA-Iron",
        "/MyPlugin/Assets/DA_Iron.DifferentObjectName",
        "/Game/Assets/DA_Iron.DA_Iron_C",
        "/Script/MyModule.NativeObject",
        "/Game/Map.Map:PersistentLevel.Actor_0.Component",
        "/Plugin_123/Ünicode/DA_鉄.DA_鉄",
    ] {
        let (_file, index) = archive(
            &[json!({"path":class,"properties":[]})],
            Some(
                json!({class:{"class":class.to_ascii_lowercase(),"assets":[{"path":path,"assetClass":class.to_ascii_lowercase()}],"total":1}}),
            ),
        );
        assert_eq!(index.asset_listing_complete(class), Some(true), "{path}");
        let result = affected_elements(&index, "cdo", &json!({"allAssetsOfClass":class}), 100);
        assert_eq!(result.status, "ready", "{path}: {:?}", result.reasons);
        assert_eq!((result.matched_count, result.potential_count), (1, 0));
    }
}

#[test]
fn asset_class_must_agree_with_bucket_or_derive_from_it() {
    let base = "/Script/Test.Base";
    let child = "/Script/Test.Child";
    let blueprint = "/Game/BP_Child.BP_Child_C";
    let unrelated = "/Script/Test.Unrelated";
    let path = "/Game/Assets/DA_Test.DA_Test";
    for (asset_class, compatible) in [
        (base, true),
        (child, true),
        ("/script/test.child", true),
        (blueprint, true),
        ("Base", false),
        ("Child", false),
        ("Test.Child", false),
        ("Class'/Script/Test.Child'", false),
        (" /Script/Test.Child ", false),
        ("BP_Child", false),
        ("/Game/BP_Child", false),
        ("/Game/BP_Child.BP_Child", false),
        ("BlueprintGeneratedClass'/Game/BP_Child.BP_Child_C'", false),
        (unrelated, false),
        ("/Script/Test.Unknown", false),
    ] {
        let (_file, index) = archive(
            &[
                json!({"path":base,"properties":[]}),
                json!({"path":child,"super":base,"properties":[]}),
                json!({"path":blueprint,"super":child,"properties":[]}),
                json!({"path":unrelated,"properties":[]}),
            ],
            Some(json!({
                base:{"class":base,"assets":[{"path":path,"assetClass":asset_class}],"total":1},
                child:{"assets":[],"total":0},
                blueprint:{"assets":[],"total":0},
            })),
        );
        assert_eq!(
            index.asset_listing_complete(base),
            Some(compatible),
            "{asset_class}"
        );
        for entry in [json!({"allAssetsOfClass":base}), json!({"target":path})] {
            let result = affected_elements(&index, "cdo", &entry, 100);
            assert_eq!(
                result.status,
                if compatible { "ready" } else { "limited" },
                "{asset_class}: {entry}"
            );
            assert_eq!(
                (result.matched_count, result.potential_count),
                if compatible { (1, 0) } else { (0, 1) },
                "{asset_class}: {entry}"
            );
        }
    }
}

#[test]
fn legacy_asset_class_fields_keep_complete_and_verified_identity() {
    let class = "/Script/Test.Asset";
    let path = "/Game/Assets/DA_Test.DA_Test";
    for asset in [json!({"path":path}), json!({"path":path,"assetClass":""})] {
        let (_file, index) = archive(
            &[json!({"path":class,"properties":[]})],
            Some(json!({class:{"assets":[asset],"total":1}})),
        );
        assert_eq!(index.asset_listing_complete(class), Some(true));
        for entry in [json!({"allAssetsOfClass":class}), json!({"target":path})] {
            let result = affected_elements(&index, "cdo", &entry, 100);
            assert_eq!(result.status, "ready", "{asset}: {entry}");
            assert_eq!((result.matched_count, result.potential_count), (1, 0));
        }
    }
}

#[test]
fn contradictory_bucket_class_never_proves_empty_scope() {
    let class = "/Script/Test.Asset";
    let (_file, index) = archive(
        &[json!({"path":class,"properties":[]})],
        Some(json!({class:{"class":"/Script/Test.Other","assets":[],"total":0}})),
    );
    assert_eq!(index.asset_listing_complete(class), Some(false));
    let result = affected_elements(&index, "cdo", &json!({"allAssetsOfClass":class}), 100);
    assert_eq!(result.status, "limited");
    assert_eq!((result.matched_count, result.potential_count), (0, 0));
}

#[test]
fn deep_asset_ancestry_proves_recursive_coverage_without_accepting_broken_chains() {
    let paths = (0..=258)
        .map(|level| format!("/Script/Test.Level{level:03}"))
        .collect::<Vec<_>>();
    let base = &paths[0];
    let leaf = paths.last().unwrap();
    let asset = "/Game/Assets/DA_Test.DA_Test";
    for (broken_parent, root_cycle) in [
        (None, false),
        (Some("/Script/Test.Missing"), false),
        (Some(paths[2].as_str()), false),
        (None, true),
    ] {
        let classes = paths
            .iter()
            .enumerate()
            .map(|(level, path)| {
                let mut record = json!({"path":path,"properties":[]});
                if level == 0 && root_cycle {
                    record["super"] = json!(paths[1]);
                }
                if level > 0 {
                    let parent = if level == 1 {
                        broken_parent.unwrap_or(&paths[level - 1])
                    } else {
                        &paths[level - 1]
                    };
                    record["super"] = json!(parent);
                }
                record
            })
            .collect::<Vec<_>>();
        let (_file, index) = archive(
            &classes,
            Some(json!({base:{
                "class":base,"assets":[{"path":asset,"assetClass":leaf}],"total":1
            }})),
        );
        let complete = broken_parent.is_none() && !root_cycle;
        assert_eq!(index.super_chain_inclusive(leaf).len(), 256);
        assert_eq!(index.asset_listing_complete(base), Some(complete));
        for entry in [
            json!({"allAssetsOfClass":base}),
            json!({"ofClass":base,"matchName":"DA_Test"}),
            json!({"target":asset}),
        ] {
            let result = affected_elements(&index, "cdo", &entry, 100);
            assert_eq!(
                result.status,
                if complete { "ready" } else { "limited" },
                "{broken_parent:?}, root_cycle={root_cycle}: {entry}: {:?}",
                result.reasons
            );
            assert_eq!(
                (result.matched_count, result.potential_count),
                if complete { (1, 0) } else { (0, 1) },
                "{broken_parent:?}, root_cycle={root_cycle}: {entry}"
            );
        }
    }
}

#[test]
fn duplicate_case_folded_asset_buckets_never_overwrite_incomplete_coverage() {
    let class = "/Script/Test.Asset";
    let folded = class.to_lowercase();
    let path = "/Game/Assets/DA_Test.DA_Test";
    let truncated = json!({
        "assets":[{"path":path,"assetClass":class}],"total":2,"truncated":true
    });
    let complete_empty = json!({"assets":[],"total":0});
    let complete_asset = json!({"assets":[{"path":path,"assetClass":class}],"total":1});
    for (first, second) in [
        (truncated.clone(), complete_empty.clone()),
        (complete_empty.clone(), truncated),
        (complete_asset, complete_empty),
    ] {
        let (_file, index) = archive(
            &[json!({"path":class,"properties":[]})],
            Some(json!({class:first,folded.clone():second})),
        );
        assert_eq!(index.asset_listing_complete(class), Some(false));
        for entry in [
            json!({"allAssetsOfClass":class}),
            json!({"ofClass":class,"matchName":"DA_Test"}),
        ] {
            let result = affected_elements(&index, "cdo", &entry, 100);
            assert_eq!(result.status, "limited", "{first}, {second}: {entry}");
            assert_eq!((result.matched_count, result.potential_count), (1, 0));
            assert!(result.reasons.iter().any(|reason| reason.contains("part")));
        }
    }
}
