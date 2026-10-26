//! Reflection-backed behaviour: class and member queries, call allowlist, reload classification,
//! settings index, cold catalog, and the diagnostics that need a bound datapack.
mod support;

use kdf_validation::{
    explicit_params, lint_path, validate_document, Advisory, Callable, ColdPropertyLevel, Implicit,
    LintOptions, LintResult, MemberKind, PropertyKind, ReflectionIndex, ReflectionStatus, Reload,
    Severity, TargetKind,
};
use std::path::Path;
use std::sync::Arc;
use support::{archive, find, triples, write, ArchiveOptions};
use tempfile::{tempdir, NamedTempFile, TempDir};

const ITEM: &str = "/Script/FactoryGame.FGItemDescriptor";

#[test]
fn native_cdo_helpers_require_restart_even_with_old_hot_annotations() {
    use std::io::Write;
    let archive = tempfile::Builder::new()
        .suffix(".kdfdata")
        .tempfile()
        .unwrap();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(archive.path()).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", options).unwrap();
    zip.write_all(
        br#"{"schemaVersion":1,"metadataAvailable":true,"counts":{"classes":2},"failedLoads":[]}"#,
    )
    .unwrap();
    zip.start_file("classes.ndjson", options).unwrap();
    for record in [
        support::class("/Script/KBFL.KBFLCDOOverwriteBase", None, vec![], vec![]),
        support::class(
            "/Script/Test.WorldHelper",
            Some("/Script/KBFL.KBFLCDOOverwriteBase"),
            vec![support::merge(
                support::prop("mSetting", "IntProperty", "int32"),
                serde_json::json!({"kdf":{"reload":"hot"}}),
            )],
            vec![],
        ),
    ] {
        writeln!(zip, "{record}").unwrap();
    }
    zip.finish().unwrap();
    let index = ReflectionIndex::load(archive.path());
    assert_eq!(index.status, ReflectionStatus::Ready, "{:?}", index.reason);
    for apply_to_spawned in [false, true] {
        let classifier = index.reload_classifier("/Script/Test.WorldHelper", apply_to_spawned);
        let property = index
            .find_property("/Script/Test.WorldHelper", "mSetting")
            .unwrap();
        assert_eq!(classifier.classify(&property).0, Reload::Cold);
        let (reload, reason) = classifier.classify_name("mRequirements");
        assert_eq!(reload, Reload::Cold);
        assert!(reason.contains("restart"));
    }
    assert_eq!(index.property_reload(ITEM, "mUnknown").0, Reload::Hot);
}

fn load(options: &ArchiveOptions) -> (NamedTempFile, ReflectionIndex) {
    let file = archive(options);
    let index = ReflectionIndex::load(file.path());
    assert_eq!(index.status, ReflectionStatus::Ready, "{:?}", index.reason);
    (file, index)
}

fn triple(file: &str, code: &str, path: &str) -> (String, String, String) {
    (file.to_owned(), code.to_owned(), path.to_owned())
}

// ---------------------------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------------------------

#[test]
fn class_references_resolve_in_every_accepted_spelling() {
    let (_keep, index) = load(&ArchiveOptions::default());
    let expected = "/Game/Items/Desc_Test.Desc_Test_C";
    for spelling in [
        expected,
        "/game/items/desc_test.desc_test_c",
        "/Game/Items/Desc_Test.Desc_Test",
        "/Game/Items/Desc_Test",
        "Desc_Test_C",
        "desc_test_c",
        "Class'/Game/Items/Desc_Test.Desc_Test_C'",
    ] {
        assert_eq!(index.class_path(spelling), Some(expected), "{spelling}");
    }
    assert_eq!(index.class_path(ITEM), Some(ITEM));
    assert_eq!(index.class_path("FGItemDescriptor"), Some(ITEM));
    assert!(index.class_path("/Script/FactoryGame.Nope").is_none());
    assert!(index.class_path("").is_none());
    let resolved = index.resolve_class(ITEM).unwrap();
    assert_eq!(resolved.record["name"], "FGItemDescriptor");
}

#[test]
fn inheritance_queries() {
    let (_keep, index) = load(&ArchiveOptions::default());
    let test = "/Game/Items/Desc_Test.Desc_Test_C";
    assert!(index.is_subclass_of(test, ITEM));
    assert!(index.is_subclass_of(test, test));
    assert!(!index.is_subclass_of(ITEM, test));
    assert!(!index.is_subclass_of("/Script/Missing.Class", ITEM));
    assert_eq!(
        index.super_chain_inclusive("Desc_Test_C"),
        [test, ITEM, "/Script/CoreUObject.Object"]
    );
    let all = index.descendants(ITEM, false);
    assert_eq!(
        all,
        [
            "/Game/Items/Desc_Test.Desc_Test_C",
            "/Script/FactoryGame.FGResourceDescriptor"
        ]
    );
    assert_eq!(
        index.descendants(ITEM, true),
        ["/Game/Items/Desc_Test.Desc_Test_C"]
    );
    assert!(index.is_abstract("/Script/FactoryGame.FGResourceDescriptor"));
    assert!(index.descendants("/Script/Missing.Class", false).is_empty());
}

#[test]
fn property_lookup_follows_supers_names_and_paths() {
    let (_keep, index) = load(&ArchiveOptions::default());
    let test = "Desc_Test_C";
    assert_eq!(
        index.find_property(test, "mdisplayname").unwrap().kind,
        PropertyKind::Text
    );
    let custom = index
        .find_property(test, "mCustomValue")
        .expect("authored name");
    assert_eq!(custom.name, "mcustomvalue");
    assert_eq!(custom.display_name(), "mCustomValue");
    let stack = index.find_property(ITEM, "mStackSize").unwrap();
    assert_eq!(stack.kind, PropertyKind::Byte);
    assert_eq!(
        stack.enum_options,
        ["SS_ONE", "SS_SMALL", "SS_MEDIUM", "SS_BIG", "SS_HUGE"]
    );
    // Paths: array element -> struct field, wildcard, class-typed struct member.
    let amount = index
        .resolve_property_path(ITEM, "mAmounts[0].Amount")
        .unwrap();
    assert_eq!(amount.kind, PropertyKind::Int32);
    assert!(index
        .resolve_property_path(ITEM, "mAmounts[*].Amount")
        .is_some());
    let class_member = index
        .resolve_property_path(ITEM, "mAmounts[3].ItemClass")
        .unwrap();
    assert_eq!(class_member.kind, PropertyKind::Class);
    assert_eq!(class_member.meta_class.as_deref(), Some(ITEM));
    assert!(index
        .resolve_property_path(ITEM, "mAmounts[0].Missing")
        .is_none());
    assert!(index
        .resolve_property_path(ITEM, "mCanBeDiscarded.x")
        .is_none());
    assert!(index
        .resolve_property_path(ITEM, "mEnergyValue[0]")
        .is_none());
    assert!(index.resolve_property_path(ITEM, "m Bad").is_none());
    assert!(index
        .struct_properties("/Script/FactoryGame.ItemAmount")
        .is_some());
    assert_eq!(
        index
            .find_struct_field("/script/factorygame.itemamount", "amount")
            .unwrap()
            .kind,
        PropertyKind::Int32
    );
}

#[test]
fn call_allowlist() {
    let (_keep, index) = load(&ArchiveOptions::default());
    let callable = |class: &str, name: &str, target: TargetKind| {
        let function = index
            .find_function(class, name)
            .unwrap_or_else(|| panic!("{name}"));
        (index.callable(&function, target, Some(class)), function)
    };
    let (get_name, function) = callable(ITEM, "GetItemName", TargetKind::Class);
    assert_eq!(
        get_name,
        Callable::Yes {
            implicit: Implicit::Class,
            explicit_args: 0,
            unverified: None
        }
    );
    assert_eq!(
        index.signature(&function, Implicit::Class),
        "GetItemName() -> FText"
    );
    assert_eq!(
        index.signature(&function, Implicit::None),
        "GetItemName(ItemClass: TSubclassOf<UObject>) -> FText"
    );
    let (energy, function) = callable(ITEM, "GetEnergyValue", TargetKind::Class);
    assert_eq!(
        energy,
        Callable::Yes {
            implicit: Implicit::None,
            explicit_args: 2,
            unverified: None
        }
    );
    assert_eq!(explicit_params(&function, Implicit::None).len(), 2);
    let (total, _) = callable(ITEM, "Total", TargetKind::Class);
    assert_eq!(
        total,
        Callable::Yes {
            implicit: Implicit::None,
            explicit_args: 1,
            unverified: None
        },
        "struct parameter built from allowed fields (ClassProperty and IntProperty)"
    );
    assert!(
        matches!(
            callable(ITEM, "GetObjectLabel", TargetKind::Class).0,
            Callable::Yes { .. }
        ),
        "const member of a plain object"
    );
    for (name, reason) in [
        ("ForEachSlot", "Exec"),
        ("Blueprinty", "BlueprintEvent"),
        ("Replicated", "Net"),
        ("Split", "output parameter"),
        ("Fetch", "return value"),
    ] {
        match callable(ITEM, name, TargetKind::Class).0 {
            Callable::No(message) => assert!(message.contains(reason), "{name}: {message}"),
            other => panic!("{name} should not be callable: {other:?}"),
        }
    }
    // Owner policy: non-static members of actors are out, statics stay.
    let building = "/Script/FactoryGame.FGBuildable";
    assert!(matches!(
        callable(building, "GetHealthPercent", TargetKind::Class).0,
        Callable::No(_)
    ));
    assert!(matches!(
        callable(building, "StaticLabel", TargetKind::Class).0,
        Callable::Yes { .. }
    ));
    // Inherited lookup.
    assert!(index.find_function("Desc_Test_C", "GetItemName").is_some());
    assert!(index.find_function(ITEM, "Missing").is_none());
}

#[test]
fn list_members_orders_properties_before_callable_functions() {
    let (_keep, index) = load(&ArchiveOptions::default());
    let members = index.list_members("Desc_Test_C", TargetKind::Class);
    let first_function = members
        .iter()
        .position(|m| m.kind == MemberKind::Function)
        .unwrap();
    assert!(members[..first_function]
        .iter()
        .all(|m| m.kind == MemberKind::Property));
    assert!(
        members.iter().any(|m| m.name == "mCustomValue"),
        "authored name is displayed"
    );
    assert!(members.iter().any(|m| m.name == "mDisplayName"));
    let functions = members[first_function..]
        .iter()
        .map(|m| m.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        functions,
        [
            "GetEnergyValue",
            "GetItemName",
            "GetObjectLabel",
            "GetStackSize",
            "Total"
        ]
    );
    let name = members.iter().find(|m| m.name == "GetItemName").unwrap();
    assert_eq!(name.implicit, Implicit::Class);
    assert_eq!(name.signature.as_deref(), Some("GetItemName() -> FText"));
    assert_eq!(name.type_label, "FText");
}

#[test]
fn reload_classification_combines_catalog_annotation_and_actor_rule() {
    let (_keep, index) = load(&ArchiveOptions::default());
    let (cold, reason) = index.property_reload(ITEM, "mForm");
    assert_eq!(cold, Reload::Cold);
    assert!(reason.contains("Item form"), "{reason}");
    assert_eq!(
        index.property_reload("Desc_Test_C", "mForm").0,
        Reload::Cold,
        "subclasses match"
    );
    assert_eq!(
        index.property_reload("Desc_Test_C", "mform[0]").0,
        Reload::Cold,
        "case and path tail"
    );
    assert_eq!(
        index.property_reload(ITEM, "mStackSize").0,
        Reload::Hot,
        "hot entries whitelist"
    );
    assert_eq!(index.property_reload(ITEM, "mEnergyValue").0, Reload::Hot);
    assert_eq!(
        index
            .property_reload("/Game/Schematics/Test.Test_C", "mUnlocks")
            .0,
        Reload::Cold
    );
    let building = "/Script/FactoryGame.FGBuildable";
    let (actor, reason) = index.property_reload(building, "mHealth");
    assert_eq!(actor, Reload::NewActors);
    assert!(reason.contains("applyToSpawnedActors"), "{reason}");
    assert_eq!(
        index.property_reload_with(building, "mHealth", true).0,
        Reload::Hot
    );
    let (annotated, reason) = index.property_reload("/Script/FactoryGame.FGRecipe", "mDisplayName");
    assert_eq!(
        (annotated, reason.as_str()),
        (Reload::Cold, "annotated by the exporter")
    );
    assert_eq!(
        index.property_reload("/Script/Missing.Class", "mX").0,
        Reload::Hot
    );
    assert!(Reload::NewActors.is_not_live() && !Reload::Hot.is_not_live());
}

/// The classifier does the class-level work once; it must agree with the per-property query for
/// every property of every class, with and without `applyToSpawnedActors`.
#[test]
fn reload_classifier_agrees_with_the_per_property_query() {
    let (_keep, index) = load(&ArchiveOptions::default());
    let mut checked = 0;
    for class in [
        ITEM,
        "Desc_Test_C",
        "/Script/FactoryGame.FGBuildable",
        "/Script/FactoryGame.FGRecipe",
        "/Game/Schematics/Test.Test_C",
        "/Script/Missing.Class",
    ] {
        for apply in [false, true] {
            let classifier = index.reload_classifier(class, apply);
            for member in index.list_members(class, TargetKind::Class) {
                let Some(property) = member.property else {
                    continue;
                };
                assert_eq!(
                    classifier.classify(&property),
                    index.property_reload_with(class, &member.name, apply),
                    "{class} {} apply={apply}",
                    member.name
                );
                checked += 1;
            }
            assert_eq!(
                classifier.classify_name("mNoSuchThing"),
                index.property_reload_with(class, "mNoSuchThing", apply)
            );
        }
    }
    assert!(checked > 10, "{checked}");
}

#[test]
fn filtered_member_listing_builds_only_the_matching_members() {
    let (_keep, index) = load(&ArchiveOptions::default());
    let all = index.list_members("Desc_Test_C", TargetKind::Class);
    let keep = |name: &str| name.to_ascii_lowercase().contains("stack");
    let filtered = index.list_members_matching("Desc_Test_C", TargetKind::Class, &keep);
    let expected = all
        .iter()
        .filter(|member| keep(&member.name))
        .map(|member| (member.kind, member.name.clone()))
        .collect::<Vec<_>>();
    assert!(!expected.is_empty());
    assert_eq!(
        filtered
            .iter()
            .map(|member| (member.kind, member.name.clone()))
            .collect::<Vec<_>>(),
        expected
    );
    assert!(index
        .list_members_matching("Desc_Test_C", TargetKind::Class, &|_| false)
        .is_empty());
}

#[test]
fn warm_builds_every_lazy_section_and_is_idempotent() {
    let (_keep, index) = load(&ArchiveOptions::default());
    index.warm();
    index.warm();
    assert!(index.settings().is_some());
    assert!(index.cold_catalog().is_some());
    assert!(!index.asset_index_classes().is_empty());
    assert!(index
        .struct_path("/Script/FactoryGame.ItemAmount")
        .is_some());
}

/// Datapacks exported before `catalog/cold_properties.json` existed (the bundled vanilla pack
/// among them) still get the built-in classification instead of reporting everything as hot.
#[test]
fn datapack_without_cold_catalog_uses_the_builtin_one() {
    let (_keep, bare) = load(&ArchiveOptions {
        cold: false,
        ..ArchiveOptions::default()
    });
    assert!(bare.cold_catalog().is_none());
    let (reload, reason) = bare.property_reload("/Game/Schematics/Test.Test_C", "mUnlocks");
    assert_eq!(reload, Reload::Cold);
    assert!(reason.contains("Unlocks are granted"), "{reason}");
    assert_eq!(bare.property_reload(ITEM, "mForm").0, Reload::Cold);
    assert_eq!(bare.property_reload(ITEM, "mEnergyValue").0, Reload::Hot);
    // A catalog in the datapack replaces the built-in one entirely.
    let (_keep, full) = load(&ArchiveOptions::default());
    assert_eq!(
        full.property_reload("/Game/Schematics/Test.Test_C", "mUnlocks")
            .0,
        Reload::Cold
    );
}

/// The built-in catalog is a copy of the plugin resource; when the plugin checkout is next to
/// this repository (the usual layout under `C:\Modding`) both must stay equal.
#[test]
fn builtin_cold_catalog_matches_the_plugin_resource() {
    let builtin = kdf_validation::builtin_catalog();
    assert!(!builtin.entries.is_empty());
    let fixture = kdf_validation::ColdCatalog::parse(support::COLD_CATALOG).unwrap();
    assert_eq!(
        *builtin, fixture,
        "tests/fixtures/cold-properties.json differs"
    );
    let plugin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../Mods/GameFeatures/KDataForge/Resources/Config/cold-properties.json");
    if let Ok(text) = std::fs::read_to_string(&plugin) {
        assert_eq!(
            *builtin,
            kdf_validation::ColdCatalog::parse(&text).unwrap(),
            "data/cold-properties.json differs from {}",
            plugin.display()
        );
    }
}

#[test]
fn settings_index_and_cold_catalog_are_optional_sections() {
    let (_keep, index) = load(&ArchiveOptions::default());
    let settings = index.settings().expect("settings index");
    assert_eq!(settings.settings.len(), 3);
    // `session:` reads session settings and falls back to advanced game settings.
    assert_eq!(
        settings
            .find("session", "FG.NoPower")
            .unwrap()
            .provider
            .as_str(),
        "ags"
    );
    assert_eq!(
        settings
            .find("session", "kmods.sfp.productionmultiplier")
            .unwrap()
            .id,
        "KMods.SFP.ProductionMultiplier"
    );
    assert!(settings
        .find("ags", "KMods.SFP.ProductionMultiplier")
        .is_none());
    let fov = settings.find("option", "Gameplay.FOV").unwrap();
    assert_eq!(fov.range, Some((70.0, 120.0)));
    assert_eq!(fov.placements.len(), 1);
    assert_eq!(fov.placements[0].display_path(), "Video → Camera");
    assert_eq!(fov.placements[0].menu_priority, 10.0);
    assert_eq!(settings.of_provider("session").len(), 2);
    assert_eq!(index.cold_catalog().unwrap().entries.len(), 4);

    let (_keep, bare) = load(&ArchiveOptions {
        settings: false,
        cold: false,
        structs: false,
        assets: false,
        functions: true,
    });
    assert!(bare.assets_of_class("/Script/Engine.DataAsset").is_empty());
    assert!(bare.find_asset("/Game/KMods/DA_Test.DA_Test").is_none());
    assert!(bare.settings().is_none());
    assert!(bare.cold_catalog().is_none());
    // Without struct data a struct parameter cannot be verified, but stays callable.
    let function = bare.find_function(ITEM, "Total").unwrap();
    match bare.callable(&function, TargetKind::Class, None) {
        Callable::Yes { unverified, .. } => assert!(unverified.is_some()),
        other => panic!("{other:?}"),
    }
    assert!(bare
        .struct_properties("/Script/FactoryGame.ItemAmount")
        .is_none());
    // The actor rule works without a catalog, and the built-in catalog replaces the missing one.
    assert_eq!(
        bare.property_reload("/Script/FactoryGame.FGBuildable", "mHealth")
            .0,
        Reload::NewActors
    );
    assert_eq!(bare.property_reload(ITEM, "mForm").0, Reload::Cold);
}

// ---------------------------------------------------------------------------------------------
// Lint with a bound datapack
// ---------------------------------------------------------------------------------------------

fn workspace() -> TempDir {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "RefPack/pack.yml",
        "ref: RefPack\nname: Ref Pack\nversion: 1.0.0\nconditions:\n  config: stack > 5\n",
    );
    write(
        root,
        "RefPack/cats.class.yml",
        "type: class\nclasses:\n  - id: Cat1\n    parent: /Script/FactoryGame.FGItemCategory\n  - id: Cat2\n    parent: /Script/FactoryGame.FGItemDescriptor\n",
    );
    write(
        root,
        "RefPack/cfg.config.yml",
        "category: Cat1\nfields:\n  - {id: speed, type: number, valueType: float, name: Speed, description: D, default: 1, apply: hot}\n  - {id: stack, type: number, name: Stack, description: D, default: 10, min: 1, max: 100, apply: hot}\n  - id: mode\n    type: select\n    name: Mode\n    description: D\n    default: easy\n    options: [{value: easy, label: Easy}, {value: hard, label: Hard}]\n  - {id: flag, type: boolean, name: Flag, description: D, default: true}\n  - {id: title, type: text, name: Title, description: D, default: x}\n  - {id: tags, type: text, valueType: tag, multiple: true, name: Tags, description: D, default: [A.B]}\n",
    );
    write(
        root,
        "RefPack/bad.config.yml",
        "fields:\n  - {id: c2, type: boolean, name: A, description: D, default: true, category: Cat2}\n  - {id: c3, type: boolean, name: A, description: D, default: true, category: /Script/FactoryGame.FGItemDescriptor}\n  - {id: c4, type: boolean, name: A, description: D, default: true, category: /Script/FactoryGame.Missing}\n  - {id: c5, type: boolean, name: A, description: D, default: true, category: /Script/FactoryGame.FGItemCategory}\n  - {id: c6, type: boolean, name: A, description: D, default: true, category: Nope}\n",
    );
    write(
        root,
        "RefPack/patch.cdo.yml",
        "type: cdo
patches:
  - target: /Script/FactoryGame.FGItemDescriptor
    properties:
      - path: mEnergyValue
        value: cfg:title
      - path: mCanBeDiscarded
        value: cfg:stack
      - path: mStackSize
        value: cfg:mode
      - path: mForm
        value: cfg:stack
      - path: mDisplayName
        value: \"{item:/Script/FactoryGame.FGItemDescriptor:mDisplayName}\"
      - path: mEnergyValue
        value: \"{item:/Script/FactoryGame.FGItemDescriptor:mDisplayName}\"
      - path: mAmounts
        op: append
        value:
          ItemClass: /Game/Items/Desc_Test.Desc_Test_C
          Amount: cfg:stack
      - path: mAmounts
        op: append
        value:
          ItemClass: /Game/Items/Desc_Test.Desc_Test_C
          Amount: cfg:title
      - path: mTags
        op: set
        value: cfg:tags
      - path: mTags
        op: set
        value: cfg:title
      - path: mEnergyValue
        value: session:FG.NoPower
      - path: mEnergyValue
        value: option:Gameplay.FOV
      - path: mEnergyValue
        value: ags:Missing.Setting
  - target: /Script/FactoryGame.FGBuildable
    properties:
      - path: mHealth
        value: 5
  - target: /Script/FactoryGame.FGBuildable
    applyToSpawnedActors: true
    properties:
      - path: mHealth
        value: 5
",
    );
    write(
        root,
        "RefPack/texts.localization.yml",
        "type: localization
namespace: T
entries:
  Ok1: \"{item:/Script/FactoryGame.FGItemDescriptor:GetItemName}\"
  Ok2: \"{class:/Script/Engine.KismetMathLibrary:Max_DoubleDouble(1, 2)}\"
  Ok3: \"{item:Desc_Test_C:mCustomValue}\"
  Ok4: \"{item:/Script/FactoryGame.FGItemDescriptor:mAmounts[0].Amount}\"
  Ok5: \"{session:FG.NoPower}\"
  Ok6: \"{building:/Script/FactoryGame.FGBuildable:StaticLabel}\"
  Ok7: \"{item:/Script/FactoryGame.FGItemDescriptor:GetObjectLabel}\"
  Bad1: \"{item:/Script/FactoryGame.FGItemDescriptor:mMissing}\"
  Bad2: \"{item:/Script/FactoryGame.FGItemDescriptor:ForEachSlot(1)}\"
  Bad3: \"{building:/Script/FactoryGame.FGBuildable:GetHealthPercent}\"
  Bad4: \"{recipe:/Script/FactoryGame.FGItemDescriptor:mDisplayName}\"
  Bad5: \"{item:/Script/FactoryGame.Missing:mX}\"
  Bad6: \"{class:/Script/Engine.KismetMathLibrary:Max_DoubleDouble(1)}\"
  Bad7: \"{class:/Script/Engine.KismetMathLibrary:Max_DoubleDouble(1, 'x')}\"
  Bad8: \"{session:Nope}\"
  Bad9: \"{item:/Script/FactoryGame.FGItemDescriptor:GetEnergyValue(2.0)}\"
  Bad10: \"{item:/Script/FactoryGame.FGItemDescriptor:Blueprinty}\"
  Bad11: \"{item:/Script/FactoryGame.FGItemDescriptor:Replicated}\"
  Bad12: \"{item:/Script/FactoryGame.FGItemDescriptor:Split(1)}\"
  Bad13: \"{item:/Script/FactoryGame.FGItemDescriptor:Fetch}\"
  Bad14: \"{item:/Script/FactoryGame.FGItemDescriptor:mAmounts[0].Missing}\"
",
    );
    write(
        root,
        "RefPack/tags.gametag.yml",
        "type: gametag\ntags:\n  - \"{cfg:stack}.Tag\"\n",
    );
    write(
        root,
        "RefPack/gated.recipe.yml",
        "type: recipe\nconditions:\n  config: stack > 5\nrecipes:\n  - id: R\n    parent: /Script/FactoryGame.FGRecipe\n",
    );
    dir
}

fn lint_bound(
    root: &Path,
    options: &ArchiveOptions,
    cold: ColdPropertyLevel,
) -> (NamedTempFile, LintResult) {
    let (keep, index) = load(options);
    let result = lint_path(
        root,
        &LintOptions {
            datapack_index: Some(Arc::new(index)),
            cold_properties: cold,
            ..Default::default()
        },
    )
    .unwrap();
    (keep, result)
}

#[test]
fn bound_datapack_enables_reflection_diagnostics() {
    let dir = workspace();
    let root = dir.path();
    let (_keep, result) = lint_bound(root, &ArchiveOptions::default(), ColdPropertyLevel::Info);
    let patch = "RefPack/patch.cdo.yml";
    let texts = "RefPack/texts.localization.yml";
    let bad = "RefPack/bad.config.yml";
    let mut expected = vec![
        triple(bad, "config.category-invalid", "$.fields[0].category"),
        triple(bad, "config.category-invalid", "$.fields[1].category"),
        triple(bad, "config.category-invalid", "$.fields[2].category"),
        triple(bad, "config.category-invalid", "$.fields[4].category"),
        triple(
            "RefPack/gated.recipe.yml",
            "config.hot-in-condition",
            "$.conditions.config",
        ),
        triple(
            "RefPack/pack.yml",
            "config.hot-in-condition",
            "$.conditions.config",
        ),
        triple(
            "RefPack/tags.gametag.yml",
            "config.hot-in-cold-stage",
            "$.tags[0]",
        ),
        triple(
            patch,
            "config.type-mismatch",
            "$.patches[0].properties[0].value",
        ),
        triple(
            patch,
            "config.type-mismatch",
            "$.patches[0].properties[1].value",
        ),
        triple(
            patch,
            "config.type-mismatch",
            "$.patches[0].properties[2].value",
        ),
        triple(patch, "property.cold", "$.patches[0].properties[3].path"),
        triple(
            patch,
            "config.hot-into-cold-property",
            "$.patches[0].properties[3].value",
        ),
        triple(
            patch,
            "config.type-mismatch",
            "$.patches[0].properties[5].value",
        ),
        triple(
            patch,
            "config.type-mismatch",
            "$.patches[0].properties[7].value.Amount",
        ),
        triple(
            patch,
            "config.type-mismatch",
            "$.patches[0].properties[9].value",
        ),
        triple(
            patch,
            "setting.type-mismatch",
            "$.patches[0].properties[10].value",
        ),
        triple(
            patch,
            "setting.unknown-id",
            "$.patches[0].properties[12].value",
        ),
        triple(patch, "property.cold", "$.patches[1].properties[0].path"),
        triple(texts, "template.unknown-member", "$.entries.Bad1"),
        triple(texts, "template.member-not-callable", "$.entries.Bad2"),
        triple(texts, "template.member-not-callable", "$.entries.Bad3"),
        triple(texts, "template.class-family", "$.entries.Bad4"),
        triple(texts, "datapack.unknown-class", "$.entries.Bad5"),
        triple(texts, "template.argument-count", "$.entries.Bad6"),
        triple(texts, "template.argument-type", "$.entries.Bad7"),
        triple(texts, "setting.unknown-id", "$.entries.Bad8"),
        triple(texts, "template.argument-count", "$.entries.Bad9"),
        triple(texts, "template.member-not-callable", "$.entries.Bad10"),
        triple(texts, "template.member-not-callable", "$.entries.Bad11"),
        triple(texts, "template.member-not-callable", "$.entries.Bad12"),
        triple(texts, "template.member-not-callable", "$.entries.Bad13"),
        triple(texts, "template.unknown-member", "$.entries.Bad14"),
    ];
    expected.sort();
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "semantic.property-path-duplicate"),
        "repeated CDO writes are valid ordered operations; reflection must still check each value"
    );
    assert_eq!(triples(&result, root), expected);

    // Severities: lossy and unknown things warn, impossible things are errors.
    let severity = |code: &str, path: &str| {
        result
            .diagnostics
            .iter()
            .find(|d| d.code == code && d.yaml_path == path)
            .unwrap()
            .severity
    };
    assert_eq!(
        severity("config.type-mismatch", "$.patches[0].properties[0].value"),
        Severity::Error
    );
    assert_eq!(
        severity("config.type-mismatch", "$.patches[0].properties[1].value"),
        Severity::Warning
    );
    assert_eq!(
        severity("setting.type-mismatch", "$.patches[0].properties[10].value"),
        Severity::Warning
    );
    assert_eq!(
        severity("config.category-invalid", "$.fields[2].category"),
        Severity::Warning
    );
    assert_eq!(
        severity("config.category-invalid", "$.fields[0].category"),
        Severity::Error
    );

    // property.cold is informational: warning severity for old consumers, advisory for new ones,
    // and never part of the warning count.
    let cold = find(&result, "property.cold");
    assert_eq!(cold.len(), 2);
    assert!(cold
        .iter()
        .all(|d| d.severity == Severity::Warning && d.advisory == Some(Advisory::Information)));
    assert_eq!(
        result.warnings,
        result
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning && d.advisory.is_none())
            .count()
    );
    let json = serde_json::to_value(cold[0]).unwrap();
    assert_eq!(json["advisory"], "information");
}

#[test]
fn cold_property_level_is_configurable() {
    let dir = workspace();
    let root = dir.path();
    let (_keep, hint) = lint_bound(root, &ArchiveOptions::default(), ColdPropertyLevel::Hint);
    assert!(find(&hint, "property.cold")
        .iter()
        .all(|d| d.advisory == Some(Advisory::Hint)));
    assert_eq!(find(&hint, "property.cold").len(), 2);
    let (_keep, off) = lint_bound(root, &ArchiveOptions::default(), ColdPropertyLevel::Off);
    assert!(find(&off, "property.cold").is_empty());
    // The hot-into-cold warning is independent of how the information is presented.
    assert_eq!(find(&off, "config.hot-into-cold-property").len(), 1);
}

#[test]
fn without_optional_sections_dependent_checks_stay_silent() {
    let dir = workspace();
    let root = dir.path();
    let (_keep, result) = lint_bound(
        root,
        &ArchiveOptions {
            settings: false,
            cold: false,
            structs: false,
            assets: false,
            functions: true,
        },
        ColdPropertyLevel::Info,
    );
    let pairs = triples(&result, root);
    assert!(
        !pairs
            .iter()
            .any(|p| p.1 == "setting.unknown-id" || p.1 == "setting.type-mismatch"),
        "{pairs:?}"
    );
    // Without a datapack catalog the built-in one classifies the cold property, next to the
    // actor rule.
    assert_eq!(find(&result, "property.cold").len(), 2);
    // Without struct data member paths cannot be proven wrong.
    assert!(!pairs.iter().any(|p| p.2 == "$.entries.Bad14"), "{pairs:?}");
}

#[test]
fn without_a_datapack_reflection_checks_are_skipped() {
    let dir = workspace();
    let root = dir.path();
    let result = lint_path(root, &LintOptions::default()).unwrap();
    let pairs = triples(&result, root);
    for code in [
        "template.unknown-member",
        "template.member-not-callable",
        "template.class-family",
        "template.argument-count",
        "template.argument-type",
        "datapack.unknown-class",
        "property.cold",
        "setting.unknown-id",
        "setting.type-mismatch",
    ] {
        assert!(
            !pairs.iter().any(|p| p.1 == code),
            "{code} needs reflection: {pairs:?}"
        );
    }
    // Slot checks that need no reflection still work (operand slots), reflection slots do not.
    assert!(
        !pairs
            .iter()
            .any(|p| p.2 == "$.patches[0].properties[0].value"),
        "{pairs:?}"
    );
}

#[test]
fn single_document_uses_the_supplied_datapack() {
    let (_keep, index) = load(&ArchiveOptions::default());
    let result = validate_document(
        Path::new("x.cdo.yml"),
        "type: cdo\npatches:\n  - target: /Script/FactoryGame.FGItemDescriptor\n    properties:\n      - path: mForm\n        value: 1\n",
        &LintOptions {
            datapack_index: Some(Arc::new(index)),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        find(&result, "property.cold").len(),
        1,
        "{:?}",
        result.diagnostics
    );
    assert!(result.ok);
}

#[test]
fn dataasset_instances_reject_abstract_classes() {
    let (_keep, index) = load(&ArchiveOptions::default());
    let result = validate_document(
        Path::new("helper.dataasset.yml"),
        "type: dataasset\nassets:\n  - id: Abstract\n    class: /Script/FactoryGame.FGResourceDescriptor\n",
        &LintOptions { datapack_index: Some(Arc::new(index)), ..Default::default() },
    ).unwrap();
    assert!(!result.ok);
    assert_eq!(find(&result, "class.abstract-no-blueprint").len(), 1);
}

#[test]
fn generated_parent_chain_resolves_target_classes() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(root, "P/pack.yml", "ref: P\nname: P\nversion: 1.0.0\n");
    write(
        root,
        "P/my.item.yml",
        "type: item\nitems:\n  - id: MyItem\n    parent: /Script/FactoryGame.FGItemDescriptor\n    properties:\n      - path: mForm\n        value: 1\n",
    );
    write(
        root,
        "P/patch.cdo.yml",
        "type: cdo\npatches:\n  - target: MyItem\n    properties:\n      - path: mForm\n        value: 1\n",
    );
    let (_keep, result) = lint_bound(root, &ArchiveOptions::default(), ColdPropertyLevel::Info);
    let pairs = triples(&result, root);
    assert!(
        pairs.contains(&triple(
            "P/my.item.yml",
            "property.cold",
            "$.items[0].properties[0].path"
        )),
        "{pairs:?}"
    );
    // `target: MyItem` is a generated id: its parent class decides the classification.
    assert!(
        pairs.contains(&triple(
            "P/patch.cdo.yml",
            "property.cold",
            "$.patches[0].properties[0].path"
        )),
        "{pairs:?}"
    );
}

#[test]
fn asset_index_lists_assets_by_class_and_path() {
    let (_keep, index) = load(&ArchiveOptions::default());
    let data_assets = index.assets_of_class("/Script/Engine.DataAsset");
    assert_eq!(
        data_assets
            .iter()
            .map(|asset| asset.name.as_str())
            .collect::<Vec<_>>(),
        ["DA_Test", "DA_Recipe"]
    );
    assert_eq!(
        index.assets_of_class("DataAsset").len(),
        2,
        "short class names resolve"
    );
    assert_eq!(
        index.asset_index_classes(),
        ["/Script/Engine.DataAsset", "/Script/Engine.Texture2D"]
    );
    let found = index.find_asset("/game/kmods/da_test.da_test").unwrap();
    assert_eq!(found.asset_class, "/Script/KAPI.KAPIDataAssetBase");
    assert_eq!(
        index.find_asset("/Game/KMods/DA_Test").unwrap().path,
        found.path,
        "package form"
    );
    assert!(index.find_asset("/Game/KMods/Missing.Missing").is_none());
}

#[test]
fn object_targets_use_the_asset_class_for_member_checks() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "P/pack.yml",
        "ref: P
name: P
version: 1.0.0
",
    );
    write(
        root,
        "P/texts.localization.yml",
        "type: localization
namespace: T
entries:
  Ok1: \"{dataasset:/Game/KMods/DA_Test.DA_Test:mTitle}\"
  Ok2: \"{asset:/Game/KMods/DA_Test.DA_Test:mTitle}\"
  Ok3: \"{dataasset:/Game/KMods/DA_Test.DA_Test:Describe}\"
  Ok4: \"{dataasset:/Game/KMods/Unlisted.Unlisted:mAnything}\"
  Bad1: \"{dataasset:/Game/KMods/DA_Test.DA_Test:mMissing}\"
  Bad2: \"{dataasset:/Game/KMods/DA_Recipe.DA_Recipe:mDisplayName}\"
  Bad3: \"{dataasset:/Game/KMods/DA_Test.DA_Test:Describe(1)}\"
",
    );
    let (_keep, result) = lint_bound(root, &ArchiveOptions::default(), ColdPropertyLevel::Info);
    let texts = "P/texts.localization.yml";
    let mut expected = vec![
        triple(texts, "template.unknown-member", "$.entries.Bad1"),
        triple(texts, "template.class-family", "$.entries.Bad2"),
        triple(texts, "template.argument-count", "$.entries.Bad3"),
    ];
    expected.sort();
    assert_eq!(triples(&result, root), expected);
}

#[test]
fn exports_without_function_records_do_not_prove_functions_missing() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "P/pack.yml",
        "ref: P
name: P
version: 1.0.0
",
    );
    write(
        root,
        "P/texts.localization.yml",
        "type: localization
namespace: T
entries:
  Maybe: \"{item:/Script/FactoryGame.FGItemDescriptor:GetItemName}\"
  Call: \"{item:/Script/FactoryGame.FGItemDescriptor:Compute(1)}\"
  Property: \"{item:/Script/FactoryGame.FGItemDescriptor:mMissing.Field}\"
",
    );
    let with_functions = lint_bound(root, &ArchiveOptions::default(), ColdPropertyLevel::Info).1;
    assert_eq!(
        find(&with_functions, "template.unknown-member").len(),
        2,
        "{:?}",
        with_functions.diagnostics
    );
    let options = ArchiveOptions {
        functions: false,
        ..ArchiveOptions::default()
    };
    let (keep, index) = load(&options);
    assert!(!index.has_function_data());
    drop(index);
    let without = lint_bound(root, &options, ColdPropertyLevel::Info).1;
    // Only the property path is definitive without function data.
    let unknown = find(&without, "template.unknown-member");
    assert_eq!(unknown.len(), 1, "{:?}", without.diagnostics);
    assert_eq!(unknown[0].yaml_path, "$.entries.Property");
    drop(keep);
    let (_keep, full) = load(&ArchiveOptions::default());
    assert!(full.has_function_data());
}

#[test]
fn actor_rule_applies_to_cdo_patches_but_not_to_generated_classes() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "P/pack.yml",
        "ref: P
name: P
version: 1.0.0
",
    );
    write(
        root,
        "P/b.building.yml",
        "type: building
buildings:
  - id: MyBuilding
    properties:
      - path: mHealth
        value: 5
",
    );
    write(
        root,
        "P/patch.cdo.yml",
        "type: cdo
patches:
  - target: /Script/FactoryGame.FGBuildable
    properties:
      - path: mHealth
        value: 5
",
    );
    let (_keep, result) = lint_bound(root, &ArchiveOptions::default(), ColdPropertyLevel::Info);
    assert_eq!(
        triples(&result, root),
        [triple(
            "P/patch.cdo.yml",
            "property.cold",
            "$.patches[0].properties[0].path"
        )]
    );
}
