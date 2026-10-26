//! A `type: building` document generates building DESCRIPTORS. Its `parent` must derive from
//! `FGBuildingDescriptor`; a buildable (actor) class is rejected and, when exactly one descriptor
//! places that actor (`mBuildableClass`), the matching `Desc_*` class is suggested.
use kdf_validation::{lint_path, Diagnostic, LintOptions, LintSettings, Profile, ReflectionIndex};
use serde_json::{json, Value};
use std::{fs, io::Write, path::Path, sync::Arc};

const DESCRIPTOR_BASE: &str = "/Script/FactoryGame.FGBuildingDescriptor";
const BUILDABLE: &str = "/Script/FactoryGame.FGBuildable";
const ITEM_DESCRIPTOR: &str = "/Script/FactoryGame.FGItemDescriptor";
const ACTOR_BUILDING: &str = "/Game/Test/Build_Test.Build_Test_C";
const DESCRIPTOR: &str = "/Game/Test/Desc_Test.Desc_Test_C";
const UNPLACED_BUILDING: &str = "/Game/Test/Build_NoDescriptor.Build_NoDescriptor_C";
const SHARED_BUILDING: &str = "/Game/Test/Build_Shared.Build_Shared_C";
const ITEM: &str = "/Game/Test/Desc_Item.Desc_Item_C";
const TRUNCATED: &str = "/Game/Test/Desc_Truncated.Desc_Truncated_C";

fn record(path: &str, parent: Option<&str>) -> Value {
    json!({"path": path, "super": parent, "properties": []})
}

fn descriptor(path: &str, buildable: &str) -> Value {
    json!({
        "path": path,
        "super": DESCRIPTOR_BASE,
        "properties": [{"name": "mBuildableClass", "default": format!("/Script/Engine.BlueprintGeneratedClass'{buildable}'")}]
    })
}

fn archive() -> tempfile::NamedTempFile {
    let records = vec![
        record("/Script/CoreUObject.Object", None),
        record("/Script/Engine.Actor", Some("/Script/CoreUObject.Object")),
        record(BUILDABLE, Some("/Script/Engine.Actor")),
        record(ITEM_DESCRIPTOR, Some("/Script/CoreUObject.Object")),
        record(DESCRIPTOR_BASE, Some(ITEM_DESCRIPTOR)),
        record(ACTOR_BUILDING, Some(BUILDABLE)),
        record(UNPLACED_BUILDING, Some(BUILDABLE)),
        record(SHARED_BUILDING, Some(BUILDABLE)),
        record(ITEM, Some(ITEM_DESCRIPTOR)),
        descriptor(DESCRIPTOR, ACTOR_BUILDING),
        descriptor(
            "/Game/Test/Desc_SharedOne.Desc_SharedOne_C",
            SHARED_BUILDING,
        ),
        descriptor(
            "/Game/Test/Desc_SharedTwo.Desc_SharedTwo_C",
            SHARED_BUILDING,
        ),
        record(TRUNCATED, Some("/Script/NotExported.MissingBase")),
    ];
    let file = tempfile::Builder::new()
        .suffix(".kdfdata")
        .tempfile()
        .unwrap();
    let mut zip = zip::ZipWriter::new(fs::File::create(file.path()).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", options).unwrap();
    write!(
        zip,
        "{}",
        json!({"schemaVersion": 1, "metadataAvailable": true, "counts": {"classes": records.len()}, "failedLoads": []})
    )
    .unwrap();
    zip.start_file("classes.ndjson", options).unwrap();
    for record in records {
        writeln!(zip, "{record}").unwrap();
    }
    zip.finish().unwrap();
    file
}

fn lint(parent: &str) -> Vec<Diagnostic> {
    lint_text(&format!(
        "type: building\nbuildings:\n  - id: MyBuilding\n    parent: {parent}\n"
    ))
}

fn lint_text(text: &str) -> Vec<Diagnostic> {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("pack.yml"),
        "ref: Example\nname: Example\nversion: 1.0.0\n",
    )
    .unwrap();
    let archive = archive();
    let index = Arc::new(ReflectionIndex::load(archive.path()));
    let selected = directory.path().join("example.building.yml");
    fs::write(&selected, text).unwrap();
    let mut options = options(directory.path());
    options.datapack_index = Some(index);
    lint_path(&selected, &options).unwrap().diagnostics
}

fn options(directory: &Path) -> LintOptions {
    LintSettings::load(directory, None)
        .unwrap()
        .lint_options(Profile::Runtime)
}

fn wrong_family(diagnostics: &[Diagnostic]) -> Vec<&Diagnostic> {
    diagnostics
        .iter()
        .filter(|item| item.code == "class.wrong-family")
        .collect()
}

#[test]
fn actor_parent_is_an_error_naming_the_descriptor_base_and_suggesting_the_matching_descriptor() {
    let diagnostics = lint(ACTOR_BUILDING);
    let found = wrong_family(&diagnostics);
    assert_eq!(found.len(), 1, "{diagnostics:?}");
    let item = found[0];
    assert_eq!(item.severity, kdf_validation::Severity::Error);
    assert!(
        item.yaml_path.ends_with("parent"),
        "points at the parent: {}",
        item.yaml_path
    );
    assert!(item.message.contains(DESCRIPTOR_BASE), "{}", item.message);
    assert!(item.message.contains(ACTOR_BUILDING), "{}", item.message);
    assert!(
        item.message.contains(&format!("Did you mean {DESCRIPTOR}")),
        "{}",
        item.message
    );
}

#[test]
fn actor_parent_without_a_placing_descriptor_gets_the_error_without_a_suggestion() {
    let diagnostics = lint(UNPLACED_BUILDING);
    let found = wrong_family(&diagnostics);
    assert_eq!(found.len(), 1, "{diagnostics:?}");
    assert!(found[0].message.contains(DESCRIPTOR_BASE));
    assert!(!found[0].message.contains("Did you mean"));
}

#[test]
fn actor_parent_placed_by_several_descriptors_gets_no_ambiguous_suggestion() {
    let diagnostics = lint(SHARED_BUILDING);
    let found = wrong_family(&diagnostics);
    assert_eq!(found.len(), 1, "{diagnostics:?}");
    assert!(!found[0].message.contains("Did you mean"));
}

#[test]
fn the_native_buildable_base_is_rejected_too() {
    let diagnostics = lint(BUILDABLE);
    assert_eq!(wrong_family(&diagnostics).len(), 1, "{diagnostics:?}");
}

#[test]
fn descriptor_parents_pass() {
    for parent in [DESCRIPTOR, DESCRIPTOR_BASE] {
        let diagnostics = lint(parent);
        assert!(
            wrong_family(&diagnostics).is_empty(),
            "{parent}: {diagnostics:?}"
        );
        assert!(
            !diagnostics
                .iter()
                .any(|item| item.code.starts_with("datapack.")),
            "{parent}: {diagnostics:?}"
        );
    }
}

#[test]
fn an_unknown_class_keeps_the_existing_unresolved_diagnostic() {
    let diagnostics = lint("/Game/Test/Desc_Missing.Desc_Missing_C");
    assert!(wrong_family(&diagnostics).is_empty(), "{diagnostics:?}");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "datapack.unknown-class")
            .count(),
        1,
        "{diagnostics:?}"
    );
}

#[test]
fn a_complete_non_descriptor_chain_is_rejected_without_the_actor_wording() {
    let diagnostics = lint(ITEM);
    let found = wrong_family(&diagnostics);
    assert_eq!(found.len(), 1, "{diagnostics:?}");
    assert!(found[0].message.contains(DESCRIPTOR_BASE));
    assert!(!found[0].message.contains("Did you mean"));
}

#[test]
fn a_chain_cut_off_by_the_snapshot_cannot_prove_a_wrong_parent() {
    let diagnostics = lint(TRUNCATED);
    assert!(wrong_family(&diagnostics).is_empty(), "{diagnostics:?}");
}

#[test]
fn generated_parents_are_left_to_their_own_definition() {
    let diagnostics = lint("/KDataForge/Gen/Example/MyOtherBuilding");
    assert!(wrong_family(&diagnostics).is_empty(), "{diagnostics:?}");
}

#[test]
fn unreal_wrapped_actor_reference_uses_the_same_descriptor_relationship() {
    let diagnostics = lint(&format!(
        "/Script/Engine.BlueprintGeneratedClass'{ACTOR_BUILDING}'"
    ));
    let found = wrong_family(&diagnostics);
    assert_eq!(found.len(), 1, "{diagnostics:?}");
    assert!(found[0]
        .message
        .contains(&format!("Did you mean {DESCRIPTOR}")));
}

#[test]
fn mixed_entries_report_only_actor_parent_at_its_exact_yaml_path() {
    let diagnostics = lint_text(&format!(
        "type: building\nbuildings:\n  - id: Good\n    parent: {DESCRIPTOR}\n  - id: Bad\n    parent: {ACTOR_BUILDING}\n  - id: Default\n"
    ));
    let found = wrong_family(&diagnostics);
    assert_eq!(found.len(), 1, "{diagnostics:?}");
    assert_eq!(found[0].yaml_path, "$.buildings[1].parent");
}

#[test]
fn generated_building_parent_and_building_template_keep_distinct_base_classes() {
    assert_eq!(
        kdf_validation::slots::default_base_class("buildings"),
        Some(DESCRIPTOR_BASE)
    );
    assert_eq!(
        kdf_validation::template::base_class("building"),
        Some(BUILDABLE)
    );
}
