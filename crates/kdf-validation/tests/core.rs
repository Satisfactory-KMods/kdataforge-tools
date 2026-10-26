use kdf_validation::{
    lint_path, lint_workspace, validate_document, FileBuffer, LintOptions, Profile,
    ReflectionIndex, ReflectionStatus, SchemaRegistry, Severity, CANONICAL_TYPES,
};
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;
use zip::write::SimpleFileOptions;

fn manifest(path: &Path, extra: &str) {
    fs::write(
        path.join("pack.yml"),
        format!("ref: Example\nname: Example Pack\nversion: 1.0.0\n{extra}"),
    )
    .unwrap();
}

#[test]
fn registry_contains_all_canonical_types_and_pack() {
    let registry = SchemaRegistry::new(&LintOptions::default()).unwrap();
    let kinds = registry.kinds();
    assert_eq!(CANONICAL_TYPES.len(), 21);
    assert!(CANONICAL_TYPES.contains(&"category_reorder"));
    assert!(CANONICAL_TYPES.contains(&"config"));
    assert!(CANONICAL_TYPES.contains(&"sound"));
    assert!(CANONICAL_TYPES.contains(&"eventrule"));
    assert_eq!(kinds.len(), 22);
    for kind in CANONICAL_TYPES {
        assert!(kinds.contains(&kind.to_owned()), "missing {kind}");
    }
    assert!(kinds.contains(&"pack".to_owned()));
    assert!(!kinds.contains(&"mam".to_owned()));
}

#[test]
fn category_reorder_matches_runtime_contract() {
    let path = Path::new("logistics.category_reorder.yml");
    for text in [
        "type: category_reorder\nsubType: items\ncategory: /Game/Cat.Cat_C\nsubCategory: /Game/Sub.Sub_C\ncategoryPriority: 4.1\nsubCategoryPriority: 2\norder:\n  - /Game/A.A_C\n  - /Game/B.B_C\n",
        "type: category_reorder\nsubType: recipe\ncategory: /Game/Cat.Cat_C\norder: [/Game/R.R_C]\n",
        "type: category_reorder\nsubType: schematic\ncategory: /Game/Cat.Cat_C\norder: [/Game/S.S_C]\n",
        "type: category_reorder\nsubType: settings\ncategory: /Game/Settings/Cat.Cat_C\nsubCategory: /Game/Settings/Sub.Sub_C\norder: [/Game/Settings/Setting.Setting]\n",
    ] {
        let result = validate_document(path, text, &LintOptions::default()).unwrap();
        assert!(result.ok, "{:?}", result.diagnostics);
    }

    for text in [
        "type: category_reorder\nsubType: other\ncategory: /Game/Cat.Cat_C\norder: [/Game/A.A_C]\n",
        "type: category_reorder\nsubType: items\ncategory: /Game/Cat.Cat_C\norder: []\n",
        "type: category_reorder\nsubType: items\ncategory: /Game/Cat.Cat_C\norder: [/Game/A.A_C, /Game/A.A_C]\n",
        "type: category_reorder\nsubType: items\ncategory: /Game/Cat.Cat_C\nsubCategoryPriority: 2\norder: [/Game/A.A_C]\n",
        "type: category_reorder\nsubType: recipe\ncategory: /Game/Cat.Cat_C\nsubCategory: /Game/Sub.Sub_C\norder: [/Game/R.R_C]\n",
    ] {
        let result = validate_document(path, text, &LintOptions::default()).unwrap();
        assert!(!result.ok, "invalid document passed: {text}");
    }
}

#[test]
fn direct_pack_root_and_nested_pack_root_both_work() {
    let temp = tempdir().unwrap();
    manifest(temp.path(), "");
    fs::write(temp.path().join("balance.cdo.yml"), "type: cdo\npatches:\n  - target: /Game/Example.Example_C\n    properties:\n      - path: mValue\n        value: 2\n").unwrap();
    let result = lint_path(temp.path(), &LintOptions::default()).unwrap();
    assert!(result.ok, "{:?}", result.diagnostics);
    assert_eq!((result.packs, result.documents), (1, 1));

    let nested = tempdir().unwrap();
    let pack = nested.path().join("Example");
    fs::create_dir(&pack).unwrap();
    manifest(&pack, "contributer: Kyri123\n");
    fs::write(pack.join("balance.cdo.yml"), "type: cdo\npatches:\n  - target: /Game/Example.Example_C\n    properties:\n      - path: mValue\n        value: 2\n").unwrap();
    let result = lint_path(
        nested.path(),
        &LintOptions {
            profile: Profile::Legacy,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(result.ok, "{:?}", result.diagnostics);
}

#[cfg(windows)]
#[test]
fn mixed_case_pack_manifest_is_recognized_on_windows() {
    let project = tempdir().unwrap();
    let pack = project.path().join("KDFSample");
    fs::create_dir(&pack).unwrap();
    let manifest_path = pack.join("Pack.yml");
    let manifest_text = "ref: KDFSample\nname: Sample\nversion: 1.0.0\n";
    fs::write(&manifest_path, manifest_text).unwrap();
    fs::write(
        pack.join("super-ingot.item.yml"),
        "type: item\nitems:\n  - id: SuperIngot\n    parent: /Script/FactoryGame.FGItemDescriptor\n",
    )
    .unwrap();

    let workspace = lint_path(project.path(), &LintOptions::default()).unwrap();
    assert!(workspace.ok, "{:?}", workspace.diagnostics);
    assert_eq!((workspace.packs, workspace.documents), (1, 1));
    let single = validate_document(&manifest_path, manifest_text, &LintOptions::default()).unwrap();
    assert!(single.ok, "{:?}", single.diagnostics);
    assert_eq!(single.packs, 1);
}

#[test]
fn legacy_contributor_policy_does_not_affect_runtime_grammar() {
    let temp = tempdir().unwrap();
    manifest(temp.path(), "");
    let runtime = lint_path(temp.path(), &LintOptions::default()).unwrap();
    assert!(runtime.ok);
    let legacy = lint_path(
        temp.path(),
        &LintOptions {
            profile: Profile::Legacy,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(legacy
        .diagnostics
        .iter()
        .any(|d| d.code == "policy.contributer"));
    assert!(legacy
        .diagnostics
        .iter()
        .any(|d| d.code == "policy.legacy-layout"));
}

#[test]
fn pack_and_document_boolean_flags_match_runtime_scalar_conversion() {
    let manifest = "ref: ConveyorLogisticsSFPlus\nname: Logistics Patch\nversion: 1.0.0\nenabled: OFF\ndebug: \"yes\"\n";
    let valid =
        validate_document(Path::new("pack.yml"), manifest, &LintOptions::default()).unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);

    let numeric = validate_document(
        Path::new("pack.yml"),
        &manifest
            .replace("enabled: OFF", "enabled: 0")
            .replace("debug: \"yes\"", "debug: 1"),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(numeric.ok, "{:?}", numeric.diagnostics);

    let cdo = validate_document(
        Path::new("buffer-inventory-release-default.yml"),
        "type: cdo\ndebug: ON\npatches:\n  - target: /Script/KPrivateCodeLib.KPCLNetworkBuildingBase\n    properties:\n      - path: bHasBufferInventory\n        value: false\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(cdo.ok, "{:?}", cdo.diagnostics);

    let invalid = validate_document(
        Path::new("pack.yml"),
        &manifest.replace("enabled: OFF", "enabled: maybe"),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(!invalid.ok);
}

#[test]
fn pack_priority_must_fit_runtime_int32_sort_order() {
    let path = Path::new("pack.yml");
    let base = "ref: ConveyorLogisticsSFPlus\nname: Logistics Patch\nversion: 1.0.0\n";
    for priority in [i32::MIN as i64, i32::MAX as i64] {
        let text = format!("{base}priority: {priority}\n");
        let result = validate_document(path, &text, &LintOptions::default()).unwrap();
        assert!(result.ok, "{priority}: {:?}", result.diagnostics);
    }
    for priority in [i32::MIN as i64 - 1, i32::MAX as i64 + 1] {
        let text = format!("{base}priority: {priority}\n");
        let result = validate_document(path, &text, &LintOptions::default()).unwrap();
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.yaml_path == "$.priority"
                    && matches!(
                        diagnostic.code.as_str(),
                        "schema.minimum" | "schema.maximum"
                    )
            }),
            "{priority}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn research_coordinates_must_fit_runtime_int32_grid() {
    let path = Path::new("kdf-sample.research.yml");
    let base = "type: research\nresearch:\n  - id: KDFSampleTree\n    addNodes:\n      - schematic: Schematic_SuperIngot\n        coordinate: {X: 3, Y: 0}\n";
    let valid = validate_document(path, base, &LintOptions::default()).unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);
    for (axis, oversized) in [("X", "2147483648"), ("Y", "-2147483649")] {
        let needle = if axis == "X" { "X: 3" } else { "Y: 0" };
        let invalid = validate_document(
            path,
            &base.replace(needle, &format!("{axis}: {oversized}")),
            &LintOptions::default(),
        )
        .unwrap();
        assert!(
            invalid.diagnostics.iter().any(|diagnostic| {
                diagnostic
                    .yaml_path
                    .ends_with(&format!("coordinate.{axis}"))
                    && diagnostic.code == "semantic.coordinate-range"
            }),
            "{axis}: {:?}",
            invalid.diagnostics
        );
    }
}

#[test]
fn nested_boolean_flags_match_runtime_scalar_conversion() {
    let cdo = "type: cdo\npatches:\n  - target: /Script/KPrivateCodeLib.KPCLNetworkBuildingBase\n    applyToSubclasses: \"yes\"\n    applyToSpawnedActors: ON\n    deferOneGameTick: 0\n    properties:\n      - path: bHasBufferInventory\n        value: false\n";
    let valid_cdo = validate_document(
        Path::new("buffer-inventory-release-default.yml"),
        cdo,
        &LintOptions::default(),
    )
    .unwrap();
    assert!(valid_cdo.ok, "{:?}", valid_cdo.diagnostics);

    let manifest = "ref: FaxitReborn\nname: Faxit Reborn\nversion: 9999.9.9\nconditions:\n  - hasMod: SatisfactoryPlus\n    ifNotMatch: \"on\"\n";
    let valid_condition =
        validate_document(Path::new("pack.yml"), manifest, &LintOptions::default()).unwrap();
    assert!(valid_condition.ok, "{:?}", valid_condition.diagnostics);
    let map_condition = validate_document(
        Path::new("pack.yml"),
        "ref: ConveyorLogisticsSFPlus\nname: Logistics Patch\nversion: 1.0.0\nconditions:\n  hasMod: SatisfactoryPlus\n  ifNotMatch: 1\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(map_condition.ok, "{:?}", map_condition.diagnostics);

    let curve = "type: curve\ncurveType: float\ncurves:\n  - id: RewardCostMultiplier\n    isEventCurve: \"no\"\n    keys:\n      - time: 0\n        value: 1\n";
    let valid_curve = validate_document(
        Path::new("faxit-curves.curve.yml"),
        curve,
        &LintOptions::default(),
    )
    .unwrap();
    assert!(valid_curve.ok, "{:?}", valid_curve.diagnostics);

    let invalid = validate_document(
        Path::new("pack.yml"),
        &manifest.replace("ifNotMatch: \"on\"", "ifNotMatch: maybe"),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(!invalid.ok);
}

#[test]
fn condition_object_keys_match_runtime_case_insensitively() {
    let path = Path::new("pack.yml");
    let text = "ref: FaxitReborn\nname: Faxit Reborn\nversion: 9999.9.9\nconditions:\n  - HASMOD: [KAPI, KLib]\n    IFNOTMATCH: \"off\"\n  - GAMEVERSION: \">=416835.0.0\"\n    MODVERSION: {KAPI: \">=1.0.0\"}\n    HASCLASS: /Script/FactoryGame.FGRecipe\n";
    let valid = validate_document(path, text, &LintOptions::default()).unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);

    let invalid = validate_document(
        path,
        &text.replace("HASCLASS:", "UNKNOWN:"),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(!invalid.ok);

    let duplicate = validate_document(
        path,
        &text.replace("HASMOD: [KAPI, KLib]", "hasMod: KAPI\n    HASMOD: KLib"),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(!duplicate.ok);
}

#[test]
fn resource_node_types_match_runtime_enum_case_insensitively() {
    let path = Path::new("purge-uranium.resourcenode.yml");
    let text = "type: resourcenode\nremove:\n  - resource: Desc_OreGold_C\n    nodeTypes: [node, ERESOURCENODETYPE::frackingcore]\n    allowOccupied: false\n";
    let valid = validate_document(path, text, &LintOptions::default()).unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);

    let invalid = validate_document(
        path,
        &text.replace("frackingcore", "invalid"),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(!invalid.ok);
}

#[test]
fn resource_node_purge_flags_match_runtime_boolean_conversion() {
    let path = Path::new("purge-uranium.resourcenode.yml");
    let text = "type: resourcenode\nremove:\n  - resource: Desc_OreGold_C\n    allowOccupied: \"off\"\n    removeFromScanner: YES\n";
    let valid = validate_document(path, text, &LintOptions::default()).unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);

    let numeric = validate_document(
        path,
        &text
            .replace("allowOccupied: \"off\"", "allowOccupied: 0")
            .replace("removeFromScanner: YES", "removeFromScanner: 1"),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(numeric.ok, "{:?}", numeric.diagnostics);

    let invalid = validate_document(
        path,
        &text.replace("allowOccupied: \"off\"", "allowOccupied: maybe"),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(!invalid.ok);
}

#[test]
fn gameplay_tags_reject_runtime_invalid_edge_punctuation() {
    let path = Path::new("sample-tags.gametag.yml");
    let valid = validate_document(
        path,
        "type: gametag\ntags:\n  - tag: KDFSample.Showcase\n  - KDF.Sample.Plain\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);

    let invalid = validate_document(
        path,
        "type: gametag\ntags:\n  - tag: .KDFSample.Showcase\n  - KDF.Sample.Plain.\n  - \" KDFSample.Showcase\"\n",
        &LintOptions::default(),
    )
    .unwrap();
    let edge_errors = invalid
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.gametag-edge")
        .collect::<Vec<_>>();
    assert_eq!(edge_errors.len(), 3, "{:?}", invalid.diagnostics);
    assert_eq!(edge_errors[0].yaml_path, "$.tags[0].tag");
}

#[test]
fn has_mod_version_spec_requires_reference_and_range() {
    let path = Path::new("showcase-condition-forms.cdo.yml");
    let base = "type: cdo\nconditions: {hasMod: KAPI@>=9999.0.0}\npatches:\n  - target: /Game/FactoryGame/Recipes/Assembler/Recipe_ModularFrame.Recipe_ModularFrame_C\n    properties:\n      - path: mManufactoringDuration\n        value: 1\n";
    let valid = validate_document(path, base, &LintOptions::default()).unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);
    for bad in ["KAPI@", "\"@>=1.0.0\""] {
        let invalid = validate_document(
            path,
            &base.replace("KAPI@>=9999.0.0", bad),
            &LintOptions::default(),
        )
        .unwrap();
        assert!(
            invalid.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "semantic.condition-mod-spec"
                    && diagnostic.yaml_path == "$.conditions.hasMod"
            }),
            "{bad}: {:?}",
            invalid.diagnostics
        );
    }
    let list = validate_document(
        path,
        &base.replace("hasMod: KAPI@>=9999.0.0", "hasMod: [KAPI, SML@]"),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(list.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "semantic.condition-mod-spec"
            && diagnostic.yaml_path == "$.conditions.hasMod[1]"
    }));

    let pack = validate_document(
        Path::new("pack.yml"),
        "ref: FaxitReborn\nname: Faxit Reborn\nversion: 9999.9.9\nconditions: {hasMod: KAPI@}\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(pack.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "semantic.condition-mod-spec"
            && diagnostic.yaml_path == "$.conditions.hasMod"
    }));

    let project = tempdir().unwrap();
    manifest(project.path(), "conditions: {hasMod: KAPI@}\n");
    let workspace = lint_path(project.path(), &LintOptions::default()).unwrap();
    assert!(workspace
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "semantic.condition-mod-spec"));
}

#[test]
fn registration_flags_match_runtime_boolean_conversion() {
    let schematic = "type: schematic\nschematics:\n  - class: /ConveyorLogistics/Schematic/Schematic_ConveyorLogistics.Schematic_ConveyorLogistics_C\n    register: \"off\"\n";
    let valid_schematic = validate_document(
        Path::new("logistics-mk2.schematic.yml"),
        schematic,
        &LintOptions::default(),
    )
    .unwrap();
    assert!(valid_schematic.ok, "{:?}", valid_schematic.diagnostics);

    let research =
        "type: research\nresearch:\n  - id: ResearchExample\n    register: YES\n    autoPath: 0\n";
    let valid_research = validate_document(
        Path::new("research.research.yml"),
        research,
        &LintOptions::default(),
    )
    .unwrap();
    assert!(valid_research.ok, "{:?}", valid_research.diagnostics);

    let item = "type: item\nitems:\n  - id: DescExample\n    parent: /Script/FactoryGame.FGItemDescriptor\n    register: 1\n";
    let valid_item =
        validate_document(Path::new("item.item.yml"), item, &LintOptions::default()).unwrap();
    assert!(valid_item.ok, "{:?}", valid_item.diagnostics);

    let invalid = validate_document(
        Path::new("logistics-mk2.schematic.yml"),
        &schematic.replace("register: \"off\"", "register: future"),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(!invalid.ok);
}

#[test]
fn filter_boolean_flags_match_runtime_scalar_conversion() {
    let path = Path::new("inventory-filter.cdo.yml");
    let text = "type: cdo\npatches:\n  - ofClass: /Script/FactoryGame.FGRecipe\n    matchAssets: \"off\"\n    where:\n      path: mIngredients\n      isEmpty: YES\n    properties:\n      - path: mDisplayName\n        value: Filtered\n";
    let valid = validate_document(path, text, &LintOptions::default()).unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);

    let list = validate_document(
        path,
        &text.replace(
            "where:\n      path: mIngredients\n      isEmpty: YES",
            "where:\n      - path: mIngredients\n        ISEMPTY: 1",
        ),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(list.ok, "{:?}", list.diagnostics);

    let invalid = validate_document(
        path,
        &text.replace("isEmpty: YES", "isEmpty: perhaps"),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(!invalid.ok);
}

#[test]
fn schema_and_semantic_errors_share_precise_utf16_ranges() {
    let text = "type: cdo\npatches:\n  - target: /Game/Example.Example_C\n    properties:\n      - path: mValue\n        op: multiplly\n        value: 2\n";
    let result =
        validate_document(Path::new("test.cdo.yml"), text, &LintOptions::default()).unwrap();
    assert!(!result.ok);
    let operation = result
        .diagnostics
        .iter()
        .find(|d| d.code == "schema.enum" || d.code == "semantic.invalid-op")
        .unwrap();
    assert_eq!(operation.line, Some(6), "{:?}", result.diagnostics);
    assert_eq!(operation.range.start.line, 5);
    assert!(operation.range.start.character >= 8);
    assert_eq!(operation.severity, Severity::Error);
}

/// Regression: diagnostic ranges of plain scalars ended one column early (`multiplly` was
/// reported as `multipll`), which broke quick fixes and cursor-in-range checks of editors.
#[test]
fn diagnostic_range_covers_plain_and_quoted_scalars_to_their_last_character() {
    for (value, expected_len) in [("multiplly", 9u32), ("\"multiplly\"", 11)] {
        let text = format!(
            "type: cdo\npatches:\n  - target: /Game/Example.Example_C\n    properties:\n      - path: mValue\n        op: {value}\n        value: 2\n"
        );
        let result =
            validate_document(Path::new("test.cdo.yml"), &text, &LintOptions::default()).unwrap();
        let operation = result
            .diagnostics
            .iter()
            .find(|d| d.code == "schema.enum" || d.code == "semantic.invalid-op")
            .unwrap_or_else(|| panic!("{:?}", result.diagnostics));
        assert_eq!(operation.range.start.line, 5, "{value}");
        assert_eq!(operation.range.end.line, 5, "{value}");
        assert_eq!(operation.range.start.character, 12, "{value}");
        assert_eq!(
            operation.range.end.character - operation.range.start.character,
            expected_len,
            "{value}: {:?}",
            operation.range
        );
    }
}

#[test]
fn document_priority_diagnostic_points_to_ignored_field() {
    let text = "type: cdo\npriority: -2147483648\npatches:\n  - target: /Script/KPrivateCodeLib.KPCLNetworkBuildingBase\n    properties:\n      - path: bHasBufferInventory\n        value: false\n";
    let result = validate_document(
        Path::new("buffer-inventory-release-default.yml"),
        text,
        &LintOptions::default(),
    )
    .unwrap();
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "schema.unevaluatedProperties")
        .expect("document priority error");
    assert_eq!(diagnostic.range.start.line, 1);
    assert_eq!(diagnostic.yaml_path, "$.priority");
    assert!(diagnostic.message.contains("pack.yml"));
}

#[test]
fn unsupported_cdo_patch_field_does_not_mark_known_root_fields_unknown() {
    let text = "type: cdo\ndebug: true\npatches:\n  - target: /Game/FactoryGame/Buildable/Factory/StorageContainerMk1/Build_StorageContainerMk1.Build_StorageContainerMk1_C\n    applyToSpawnedActors: true\n    propagateToInstances: true\n    properties:\n      - path: mInventorySizeX\n        value: 16\n";
    let result = validate_document(
        Path::new("showcase-spawned-actor-patch.cdo.yml"),
        text,
        &LintOptions::default(),
    )
    .unwrap();
    let unexpected = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "schema.unevaluatedProperties")
        .collect::<Vec<_>>();
    assert_eq!(unexpected.len(), 1, "{:?}", result.diagnostics);
    assert_eq!(unexpected[0].yaml_path, "$.patches[0].propagateToInstances");
}

#[test]
fn malformed_yaml_and_multi_document_missing_type_are_reported() {
    let syntax = validate_document(
        Path::new("test.cdo.yml"),
        "type: cdo\npatches: [\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(syntax.diagnostics.iter().any(|d| d.code == "yaml.syntax"));
    let multi = validate_document(
        Path::new("test.cdo.yml"),
        "type: cdo\npatches: []\n---\npatches: []\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(multi
        .diagnostics
        .iter()
        .any(|d| d.code == "semantic.missing-type" && d.document == Some(1)));
}

#[test]
fn syntax_error_after_unicode_uses_utf16_column_when_yaml_cannot_parse() {
    let text = "type: cdo\nname: \"😀\" ]\n";
    assert!(kdf_validation::SourceMap::parse(text).is_err());
    let parse_error = serde_yaml::from_str::<serde_yaml::Value>(text).unwrap_err();
    let location = parse_error.location().unwrap();
    let line = text.lines().nth(location.line() - 1).unwrap();
    let expected = line
        .chars()
        .take(location.column() - 1)
        .map(char::len_utf16)
        .sum::<usize>() as u32;
    assert_ne!(expected, (location.column() - 1) as u32);

    let report =
        validate_document(Path::new("broken.cdo.yml"), text, &LintOptions::default()).unwrap();
    let syntax = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "yaml.syntax")
        .unwrap();
    assert_eq!(syntax.range.start.line, (location.line() - 1) as u32);
    assert_eq!(syntax.range.start.character, expected);
}

#[test]
fn single_document_filename_inference_matches_runtime_and_custom_schemas() {
    let project = tempdir().unwrap();
    let schemas = project.path().join("schemas");
    fs::create_dir(&schemas).unwrap();
    fs::write(
        schemas.join("custom.schema.yml"),
        "$schema: https://json-schema.org/draft/2020-12/schema\n$id: urn:kdf:test:custom\nx-kdf-type: custom\ntype: object\nrequired: [value]\nproperties:\n  value: {type: integer}\n",
    )
    .unwrap();
    let options = LintOptions {
        schema_dirs: vec![schemas],
        ..Default::default()
    };
    let custom = validate_document(Path::new("effect.custom.yml"), "value: 1\n", &options).unwrap();
    assert!(custom.ok, "{:?}", custom.diagnostics);

    let no_name = validate_document(
        Path::new("recipe.yml"),
        "remove: [Recipe_Example_C]\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(no_name
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "semantic.missing-type"));
}

#[test]
fn empty_explicit_type_uses_single_document_filename_fallback() {
    let project = tempdir().unwrap();
    manifest(project.path(), "");
    let path = project.path().join("super-ingot.item.yml");
    let text =
        "type: ''\nitems:\n  - id: SuperIngot\n    parent: /Script/FactoryGame.FGItemDescriptor\n";
    fs::write(&path, text).unwrap();

    let standalone = validate_document(&path, text, &LintOptions::default()).unwrap();
    let workspace = lint_path(project.path(), &LintOptions::default()).unwrap();
    assert!(standalone.ok, "{:?}", standalone.diagnostics);
    assert!(workspace.ok, "{:?}", workspace.diagnostics);
    assert_eq!(workspace.documents, 1);

    let multi = validate_document(
        &path,
        "type: ''\nitems: []\n---\ntype: item\nitems:\n  - id: Other\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(multi.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "semantic.missing-type" && diagnostic.document == Some(0)
    }));
}

#[test]
fn class_register_as_matches_runtime_case_insensitively() {
    let path = Path::new("showcase.class.yml");
    let valid = validate_document(
        path,
        "type: class\nclasses:\n  - id: ClassRegisterAsRecipe\n    parent: /Script/FactoryGame.FGRecipe\n    registerAs: ReCiPe\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);

    let unknown = validate_document(
        path,
        "type: class\nclasses:\n  - id: ClassRegisterAsRecipe\n    parent: /Script/FactoryGame.FGRecipe\n    registerAs: future\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(unknown
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "schema.enum"));
}

#[test]
fn where_operator_names_match_runtime_case_insensitively() {
    let path = Path::new("rebalance.cdo.yml");
    let valid = validate_document(
        path,
        "type: cdo\npatches:\n  - ofClass: /Script/FactoryGame.FGRecipe\n    where:\n      path: mManufactoringDuration\n      GREATERTHAN: 1\n    properties:\n      - path: mManufactoringDuration\n        value: 2\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);

    let unknown = validate_document(
        path,
        "type: cdo\npatches:\n  - ofClass: /Script/FactoryGame.FGRecipe\n    where:\n      path: mManufactoringDuration\n      BOGUS: 1\n    properties:\n      - path: mManufactoringDuration\n        value: 2\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(unknown
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "semantic.where-unknown-operator"));
}

#[test]
fn sinkpoints_track_matches_runtime_case_insensitively() {
    let path = Path::new("exploration.sinkpoints.yml");
    let valid = validate_document(
        path,
        "type: sinkpoints\ntrack: eXpLoRaTiOn\nentries:\n  - item: SuperIngot\n    points: 30\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);

    let unknown = validate_document(
        path,
        "type: sinkpoints\ntrack: Future\nentries:\n  - item: SuperIngot\n    points: 30\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(unknown
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "schema.enum"));
}

#[test]
fn curve_modes_match_runtime_case_insensitively() {
    let path = Path::new("faxit-curves.curve.yml");
    let text = "type: curve\ncurveType: FLOAT\ncurves:\n  - id: RecyclingCostMultiplier\n    defaultValue: 50\n    preInfinityExtrap: CONSTANT\n    postInfinityExtrap: CYCLEWITHOFFSET\n    keys:\n      - time: 0\n        value: 50\n        interpMode: CUBIC\n        tangentMode: SMARTAUTO\n        tangentWeightMode: BOTH\n";
    let valid = validate_document(path, text, &LintOptions::default()).unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);

    let invalid = validate_document(
        path,
        &text.replace("SMARTAUTO", "FUTURE"),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(invalid
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "schema.enum"));
}

#[test]
fn vector_curve_components_match_runtime_case_insensitively() {
    let path = Path::new("coordinates.curve.yml");
    let text = "type: curve\ncurveType: vector\ncurves:\n  - id: Coordinates\n    defaultValue: {x: 1, Y: 2, z: 3}\n    keys:\n      - time: 0\n        value: {x: 4, y: 5, Z: 6}\n        arriveTangent: {X: 0, y: 0, z: 0}\n";
    let valid = validate_document(path, text, &LintOptions::default()).unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);

    let duplicate = validate_document(
        path,
        &text.replace("{x: 4, y: 5, Z: 6}", "{X: 4, x: 5, Y: 6}"),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(!duplicate.ok, "{:?}", duplicate.diagnostics);
}

#[test]
fn curve_time_and_vector_components_must_fit_runtime_float32() {
    let text = "type: curve\ncurveType: vector\ncurves:\n  - id: Coordinates\n    keys:\n      - time: 1e100\n        value: {X: 1e100, Y: 2, Z: 3}\n";
    let result = validate_document(
        Path::new("coordinates.curve.yml"),
        text,
        &LintOptions::default(),
    )
    .unwrap();
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "semantic.curve-time-range"));
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "semantic.curve-value-range"));

    let sequence = validate_document(
        Path::new("coordinates.curve.yml"),
        &text.replace("{X: 1e100, Y: 2, Z: 3}", "[1, 2, 1e100]"),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(sequence
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "semantic.curve-value-range"));
}

#[test]
fn quoted_curve_numbers_match_runtime_scalar_conversion() {
    let path = Path::new("faxit-curves.curve.yml");
    let quoted = "type: curve\ncurveType: float\ncurves:\n  - id: RewardCostMultiplier\n    defaultValue: \"1\"\n    keys:\n      - time: \"0\"\n        value: \"1\"\n      - time: \"49\"\n        value: \"5\"\n";
    let valid = validate_document(path, quoted, &LintOptions::default()).unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);

    let vector = "type: curve\ncurveType: vector\ncurves:\n  - id: Coordinates\n    keys:\n      - time: \"0\"\n        value: {X: \"1\", Y: \"2\", Z: \"3\"}\n";
    let valid_vector = validate_document(path, vector, &LintOptions::default()).unwrap();
    assert!(valid_vector.ok, "{:?}", valid_vector.diagnostics);

    let duplicate = validate_document(
        path,
        &quoted.replace("time: \"49\"", "time: \"0\""),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(duplicate
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "semantic.duplicate-curve-time"));

    let invalid = validate_document(
        path,
        &quoted.replace("value: \"5\"", "value: \"not a number\""),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(!invalid.ok);

    let too_large = "9999999999999999999999999999999999999999999999";
    let time_overflow = validate_document(
        path,
        &quoted.replace("time: \"49\"", &format!("time: \"{too_large}\"")),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(time_overflow
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "semantic.curve-time-range"));
    let vector_overflow = validate_document(
        path,
        &vector.replace("Z: \"3\"", &format!("Z: \"{too_large}\"")),
        &LintOptions::default(),
    )
    .unwrap();
    assert!(vector_overflow
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "semantic.curve-value-range"));
}

#[test]
fn sinkpoints_values_cannot_wrap_when_runtime_casts_to_i32() {
    let path = Path::new("exploration.sinkpoints.yml");
    let valid = validate_document(
        path,
        "type: sinkpoints\ntrack: Exploration\nentries:\n  - item: SuperIngot\n    points: 30\n    overridden: 999\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);

    let overflow = validate_document(
        path,
        "type: sinkpoints\nentries:\n  - item: SuperIngot\n    points: 4294967296\n    overridden: -2147483649\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(
        overflow.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "schema.maximum" && diagnostic.yaml_path == "$.entries[0].points"
        }),
        "{:?}",
        overflow.diagnostics
    );
    assert!(
        overflow.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "schema.minimum" && diagnostic.yaml_path == "$.entries[0].overridden"
        }),
        "{:?}",
        overflow.diagnostics
    );
}

#[test]
fn curve_key_times_must_be_unique_after_runtime_float_conversion() {
    let text = "type: curve\ncurveType: float\ncurves:\n  - id: RewardCostMultiplier\n    keys:\n      - time: 0\n        value: 1\n      - time: 49\n        value: 5\n      - time: 49\n        value: 6\n  - id: RoundedTimes\n    keys:\n      - time: 0.1\n        value: 1\n      - time: 0.100000001\n        value: 2\n";
    let report = validate_document(
        Path::new("faxit-curves.curve.yml"),
        text,
        &LintOptions::default(),
    )
    .unwrap();
    let duplicates = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.duplicate-curve-time")
        .map(|diagnostic| diagnostic.yaml_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        duplicates,
        ["$.curves[0].keys[2].time", "$.curves[1].keys[1].time",],
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn curve_values_match_selected_float_or_vector_type() {
    let path = Path::new("faxit-curves.curve.yml");
    let vector = validate_document(
        path,
        "type: curve\ncurveType: vector\ncurves:\n  - id: VectorExample\n    defaultValue: 1\n    keys:\n      - time: 0\n        value: 1\n      - time: 1\n        value: [1, 2, 3]\n",
        &LintOptions::default(),
    )
    .unwrap();
    let invalid_vector = vector
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.curve-value-kind")
        .map(|diagnostic| diagnostic.yaml_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        invalid_vector,
        ["$.curves[0].defaultValue", "$.curves[0].keys[0].value"]
    );

    let scalar = validate_document(
        path,
        "type: curve\ncurveType: float\ncurves:\n  - id: FloatExample\n    defaultValue: [1, 2, 3]\n    keys:\n      - time: 0\n        value: [1, 2, 3]\n",
        &LintOptions::default(),
    )
    .unwrap();
    let invalid_scalar = scalar
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.curve-value-kind")
        .map(|diagnostic| diagnostic.yaml_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        invalid_scalar,
        ["$.curves[0].defaultValue", "$.curves[0].keys[0].value"]
    );
}

#[test]
fn sublevel_filters_require_of_class_even_with_recipe_shortcuts() {
    let result = validate_document(
        Path::new("block-sublevels.sublevel.yml"),
        "type: sublevel\nblock:\n  - producedIn: /Game/FactoryGame/Buildable/Factory/ConstructorMk1/Build_ConstructorMk1.Build_ConstructorMk1_C\n  - ofClass: /Script/FactoryGame.FGRecipe\n    producedIn: /Game/FactoryGame/Buildable/Factory/ConstructorMk1/Build_ConstructorMk1.Build_ConstructorMk1_C\n",
        &LintOptions::default(),
    )
    .unwrap();
    let missing_scope = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.scope-class")
        .collect::<Vec<_>>();
    assert_eq!(missing_scope.len(), 1, "{:?}", result.diagnostics);
    assert_eq!(missing_scope[0].yaml_path, "$.block[0]");
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.source == "schema"));

    let valid = validate_document(
        Path::new("block-sublevels.sublevel.yml"),
        "type: sublevel\nblock:\n  - ofClass: /Script/FactoryGame.FGRecipe\n    producedIn: /Game/FactoryGame/Buildable/Factory/ConstructorMk1/Build_ConstructorMk1.Build_ConstructorMk1_C\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);
}

#[test]
fn detects_pack_cycles_and_include_escape() {
    let temp = tempdir().unwrap();
    let a = temp.path().join("A");
    let b = temp.path().join("B");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    fs::write(
        a.join("pack.yml"),
        "ref: A\nname: A\nversion: 1.0.0\ndependencies: [B]\n",
    )
    .unwrap();
    fs::write(
        b.join("pack.yml"),
        "ref: B\nname: B\nversion: 1.0.0\ndependencies: [A]\n",
    )
    .unwrap();
    fs::write(a.join("bad.cdo.yml"), "type: cdo\ninclude: ../../../outside.yml\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mValue\n        value: 1\n").unwrap();
    let result = lint_path(temp.path(), &LintOptions::default()).unwrap();
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == "semantic.dependency-cycle"));
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == "semantic.include-escape"));
}

#[test]
fn unknown_type_can_be_downgraded_to_warning() {
    let options = LintOptions {
        allow_unknown_types: true,
        ..Default::default()
    };
    let result = validate_document(
        Path::new("extra.foo.yml"),
        "type: foo\nvalue: 1\n",
        &options,
    )
    .unwrap();
    assert!(result.ok);
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == "schema.unknown-type" && d.severity == Severity::Warning));
}

#[test]
fn mam_alias_uses_research_schema_without_becoming_new_root_type() {
    let result = validate_document(
        Path::new("remove.mam.yml"),
        "type: mam\nremove: [Research_Example_C]\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(result.ok, "{:?}", result.diagnostics);
}

#[test]
fn unsaved_buffer_overrides_disk_even_on_windows_paths() {
    let temp = tempdir().unwrap();
    manifest(temp.path(), "");
    let path = temp.path().join("balance.cdo.yml");
    fs::write(&path, "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mValue\n        value: 1\n").unwrap();
    #[cfg(windows)]
    let path = std::path::PathBuf::from(path.to_str().unwrap().to_ascii_uppercase());
    let buffer = FileBuffer { path, text: "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mValue\n        op: multiplly\n        value: 1\n".into() };
    let result = lint_workspace(temp.path(), &[buffer], &LintOptions::default()).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code == "semantic.invalid-op"),
        "{result:?}"
    );
}

#[test]
fn unsaved_new_document_uses_canonical_parent_and_stays_inside_workspace() {
    let temp = tempdir().unwrap();
    manifest(temp.path(), "");
    #[cfg(windows)]
    let client_root = std::path::PathBuf::from(temp.path().to_str().unwrap().to_ascii_uppercase());
    #[cfg(not(windows))]
    let client_root = temp.path().to_path_buf();
    let buffer = FileBuffer {
        path: client_root.join("new.cdo.yml"),
        text: "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mValue\n        op: multiplly\n        value: 1\n".into(),
    };
    let result = lint_workspace(temp.path(), &[buffer], &LintOptions::default()).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code == "semantic.invalid-op"),
        "{result:?}"
    );
    assert!(!temp.path().join("new.cdo.yml").exists());
    let outside = tempdir().unwrap();
    let result = lint_workspace(
        temp.path(),
        &[FileBuffer {
            path: outside.path().join("outside.cdo.yml"),
            text: "op: multiplly".into(),
        }],
        &LintOptions::default(),
    )
    .unwrap();
    assert_eq!(
        result.documents, 0,
        "outside buffer entered workspace: {result:?}"
    );
}

#[test]
fn external_schema_is_offline_and_cannot_shadow_builtin_without_opt_in() {
    let temp = tempdir().unwrap();
    let schemas = temp.path().join("schemas");
    fs::create_dir(&schemas).unwrap();
    fs::write(schemas.join("extra.schema.yml"), "$schema: https://json-schema.org/draft/2020-12/schema\n$id: urn:example:extra\nx-kdf-type: extra\ntype: object\nrequired: [type, value]\nproperties:\n  type: {const: extra}\n  value: {type: integer}\n").unwrap();
    let options = LintOptions {
        schema_dirs: vec![schemas.clone()],
        ..Default::default()
    };
    let valid = validate_document(
        Path::new("one.extra.yml"),
        "type: extra\nvalue: 1\n",
        &options,
    )
    .unwrap();
    assert!(valid.ok, "{:?}", valid.diagnostics);
    let invalid = validate_document(
        Path::new("one.extra.yml"),
        "type: extra\nvalue: no\n",
        &options,
    )
    .unwrap();
    assert!(invalid.diagnostics.iter().any(|d| d.code == "schema.type"));
    fs::write(schemas.join("duplicate.schema.yml"), "$schema: https://json-schema.org/draft/2020-12/schema\n$id: urn:example:extra\nx-kdf-type: extra\ntype: object\n").unwrap();
    assert!(SchemaRegistry::new(&options).is_err());
}

#[test]
fn all_canonical_document_shapes_validate() {
    let examples = [
        ("asset", "assets:\n  - id: Icon\n    file: icon.png\n"),
        ("building", "buildings:\n  - id: BuildExample\n    parent: /Script/FactoryGame.FGBuildable\n"),
        ("cdo", "patches:\n  - target: /Game/Example.Example_C\n    properties:\n      - path: mValue\n        value: 1\n"),
        ("category_reorder", "subType: items\ncategory: /Game/Example/Category.Category_C\norder: [/Game/Example/Item.Item_C]\n"),
        ("class", "classes:\n  - id: ClassExample\n    parent: /Script/FactoryGame.FGItemDescriptor\n"),
        ("curve", "curveType: float\ncurves:\n  - id: CurveExample\n    keys:\n      - time: 0\n        value: 1\n"),
        ("dataasset", "assets:\n  - id: DataExample\n    class: /Script/KAPI.KAPIDataAssetBase\n"),
        ("gametag", "tags: [Example.Tag]\n"),
        ("item", "items:\n  - id: ItemExample\n    parent: /Script/FactoryGame.FGItemDescriptor\n"),
        ("localization", "namespace: Example\nentries:\n  Hello: World\n"),
        ("recipe", "remove: [Recipe_Example_C]\n"),
        ("research", "remove: [Research_Example_C]\n"),
        ("resource", "resources:\n  - id: ResourceExample\n    parent: /Script/FactoryGame.FGResourceDescriptor\n"),
        ("resourcenode", "remove: [/Game/Example.Example_C]\n"),
        ("schematic", "remove: [Schematic_Example_C]\n"),
        ("sinkpoints", "entries:\n  - item: Desc_Example_C\n    points: 5\n"),
        ("sublevel", "block: [/Game/World/Example.Example]\n"),
        ("unlock", "unlocks:\n  - id: UnlockExample\n    parent: /Script/FactoryGame.FGUnlock\n"),
    ];
    for (kind, body) in examples {
        let text = format!("type: {kind}\n{body}");
        let filename = format!("example.{kind}.yml");
        let result =
            validate_document(Path::new(&filename), &text, &LintOptions::default()).unwrap();
        assert!(result.ok, "{kind}: {:?}", result.diagnostics);
    }
}

#[test]
fn frozen_legacy_differential_fixture_matches_python_rule_identity() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/DataForge");
    let result = lint_path(
        &root,
        &LintOptions {
            profile: Profile::Legacy,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        (
            result.packs,
            result.documents,
            result.errors,
            result.warnings
        ),
        (1, 1, 1, 0)
    );
    let diagnostic = &result.diagnostics[0];
    assert_eq!(diagnostic.code, "schema.unevaluatedProperties");
    assert!(diagnostic.message.contains("priority"));
    assert_eq!((diagnostic.line, diagnostic.column), (Some(2), Some(11)));
    assert_eq!(diagnostic.yaml_path, "$.priority");
}

#[test]
fn detects_redirect_cycles_and_sources_that_still_exist() {
    let temp = tempdir().unwrap();
    fs::write(
        temp.path().join("pack.yml"),
        "ref: Example\nname: Example\nversion: 1.0.0\nredirects:\n  Old: New\n  New: Old\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("item.item.yml"),
        "type: item\nitems:\n  - id: Old\n",
    )
    .unwrap();
    let result = lint_path(temp.path(), &LintOptions::default()).unwrap();
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == "semantic.redirect-cycle"));
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == "semantic.redirect-source-present"));
}

#[test]
fn duplicate_merge_keeps_semantic_reason_for_same_operation_error() {
    let project = tempdir().unwrap();
    manifest(project.path(), "");
    fs::write(project.path().join("balance.cdo.yml"), "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mValue\n        op: multiplly\n        value: 1\n").unwrap();
    let unmerged = lint_path(project.path(), &LintOptions::default()).unwrap();
    assert!(unmerged
        .diagnostics
        .iter()
        .any(|item| item.code == "schema.enum"));
    assert!(unmerged
        .diagnostics
        .iter()
        .any(|item| item.code == "semantic.invalid-op"));
    let merged = lint_path(
        project.path(),
        &LintOptions {
            merge_duplicate_diagnostics: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!merged
        .diagnostics
        .iter()
        .any(|item| item.code == "schema.enum"));
    assert!(merged
        .diagnostics
        .iter()
        .any(|item| item.code == "semantic.invalid-op"));
}

#[test]
fn dataasset_creation_class_is_checked_against_bound_reflection() {
    let project = tempdir().unwrap();
    manifest(project.path(), "");
    fs::write(
        project.path().join("upgrades.dataasset.yml"),
        "type: dataasset\nassets:\n  - id: MissingTask\n    class: /Script/KAPI.KAPIMissingDeliveryTask\n  - id: KnownTask\n    class: /Script/KAPI.KAPIDeliveryTask\n",
    )
    .unwrap();

    let archive = tempfile::Builder::new()
        .suffix(".kdfdata")
        .tempfile()
        .unwrap();
    let mut zip = zip::ZipWriter::new(File::create(archive.path()).unwrap());
    zip.start_file("manifest.json", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(br#"{"schemaVersion":1,"metadataAvailable":true,"options":{"assets":true},"counts":{"classes":1},"failedLoads":[]}"#)
        .unwrap();
    zip.start_file("classes.ndjson", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"{\"path\":\"/Script/KAPI.KAPIDeliveryTask\",\"properties\":[]}\n")
        .unwrap();
    zip.finish().unwrap();
    let reflection = ReflectionIndex::load(archive.path());
    assert_eq!(reflection.status, ReflectionStatus::Ready);

    let report = lint_path(
        project.path(),
        &LintOptions {
            datapack_index: Some(Arc::new(reflection)),
            ..Default::default()
        },
    )
    .unwrap();
    let unknown = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "datapack.unknown-class")
        .collect::<Vec<_>>();
    assert_eq!(unknown.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(unknown[0].yaml_path, "$.assets[0].class");
    assert_eq!(unknown[0].source, "datapack");
    assert_eq!(unknown[0].severity, Severity::Warning);
}

#[test]
fn property_path_diagnostics_follow_runtime_map_key_syntax() {
    let project = tempdir().unwrap();
    manifest(project.path(), "");
    fs::write(
        project.path().join("input.cdo.yml"),
        "type: cdo\npatches:\n  - target: /KLib/Assets/Buildings/Faxit/Build_CS_InputSolid.Build_CS_InputSolid_C\n    ofClass: /Script/FactoryGame.FGBuildable\n    where:\n      - path: mItems[*]\n        equals: true\n    properties:\n      - path: mValues['Tier.High']\n        value: 1\n      - path: mValues[Tier.High]\n        value: 2\n      - path: mValues[]\n        value: 3\n      - path: mValues[*]\n        value: 4\n      - path: mValues['Tier[*]']\n        value: 5\n",
    )
    .unwrap();

    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    let invalid_paths = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.property-path")
        .collect::<Vec<_>>();
    assert_eq!(invalid_paths.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(
        invalid_paths[0].yaml_path,
        "$.patches[0].properties[2].path"
    );
    let schema_paths = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "schema.oneOf")
        .collect::<Vec<_>>();
    assert_eq!(schema_paths.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(schema_paths[0].yaml_path, "$.patches[0].properties[2].path");
    assert!(!report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "semantic.where-path"));
    let invalid_writes = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.property-path-wildcard")
        .collect::<Vec<_>>();
    assert_eq!(invalid_writes.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(
        invalid_writes[0].yaml_path,
        "$.patches[0].properties[3].path"
    );
}

#[test]
fn repeated_set_and_clear_then_set_are_valid_ordered_operations() {
    let project = tempdir().unwrap();
    manifest(project.path(), "");
    fs::write(
        project.path().join("input.cdo.yml"),
        "type: cdo\npatches:\n  - target: /Script/FactoryGame.FGItemDescriptor\n    properties:\n      - path: mStackSize\n        value: 1\n      - path: mStackSize\n        op: multiply\n        value: 2\n      - path: mstacksize\n        value: 3\n      - path: mForm\n        op: clear\n      - path: mForm\n        op: set\n        value: RF_LIQUID\n      - path: mDisplayName\n        value: A\n",
    )
    .unwrap();

    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    let duplicates = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.property-path-duplicate")
        .collect::<Vec<_>>();
    assert!(duplicates.is_empty(), "{:?}", report.diagnostics);
    assert!(report.ok, "{:?}", report.diagnostics);
}

#[test]
fn standalone_document_uses_same_local_semantic_diagnostics_as_project_lint() {
    let project = tempdir().unwrap();
    manifest(project.path(), "");
    let path = project.path().join("input.cdo.yml");
    let text = "type: cdo\npatches:\n  - target: /KLib/Assets/Buildings/Faxit/Build_CS_InputSolid.Build_CS_InputSolid_C\n    matchName: Build_CS_InputSolid\n    properties:\n      - path: bHasBufferInventory\n        value: true\n";
    fs::write(&path, text).unwrap();

    let standalone = validate_document(&path, text, &LintOptions::default()).unwrap();
    let project_lint = lint_path(project.path(), &LintOptions::default()).unwrap();
    let standalone_scope = standalone
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "semantic.scope-class")
        .expect("standalone semantic.scope-class");
    let project_scope = project_lint
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "semantic.scope-class")
        .expect("project semantic.scope-class");
    assert_eq!(standalone_scope.severity, project_scope.severity);
    assert_eq!(standalone_scope.yaml_path, project_scope.yaml_path);
    assert_eq!(standalone_scope.range, project_scope.range);
}

#[test]
fn disabled_pack_does_not_satisfy_an_active_dependency() {
    let project = tempdir().unwrap();
    let active = project.path().join("Active");
    let disabled = project.path().join("Disabled");
    fs::create_dir(&active).unwrap();
    fs::create_dir(&disabled).unwrap();
    fs::write(
        active.join("pack.yml"),
        "ref: Active\nname: Active\nversion: 1.0.0\nenabled: true\ndependencies: [Disabled]\n",
    )
    .unwrap();
    fs::write(
        disabled.join("pack.yml"),
        "ref: Disabled\nname: Disabled\nversion: 1.0.0\nenabled: false\n",
    )
    .unwrap();

    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    assert!(
        report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "semantic.dependency-missing"
                && diagnostic.file.as_deref()
                    == Some(
                        dunce::canonicalize(active.join("pack.yml"))
                            .unwrap()
                            .as_path(),
                    )
                && diagnostic.message.contains("Disabled")
        }),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn runtime_boolean_spelling_disables_pack_for_dependency_resolution() {
    for disabled_value in ["OFF", "0", "\"no\""] {
        let project = tempdir().unwrap();
        let active = project.path().join("Active");
        let disabled = project.path().join("Disabled");
        fs::create_dir(&active).unwrap();
        fs::create_dir(&disabled).unwrap();
        fs::write(
            active.join("pack.yml"),
            "ref: Active\nname: Active\nversion: 1.0.0\ndependencies: [Disabled]\n",
        )
        .unwrap();
        fs::write(
            disabled.join("pack.yml"),
            format!("ref: Disabled\nname: Disabled\nversion: 1.0.0\nenabled: {disabled_value}\n"),
        )
        .unwrap();

        let report = lint_path(project.path(), &LintOptions::default()).unwrap();
        assert!(
            report.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "semantic.dependency-missing"
                    && diagnostic.file.as_deref()
                        == Some(
                            dunce::canonicalize(active.join("pack.yml"))
                                .unwrap()
                                .as_path(),
                        )
            }),
            "{disabled_value}: {:?}",
            report.diagnostics
        );
    }
}

#[test]
fn include_paths_are_relative_to_pack_root_and_cannot_cross_packs() {
    let project = tempdir().unwrap();
    let pack = project.path().join("Sample");
    let other = project.path().join("Other");
    fs::create_dir_all(pack.join("Recipes")).unwrap();
    fs::create_dir(&other).unwrap();
    fs::write(
        pack.join("pack.yml"),
        "ref: Sample\nname: Sample\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        other.join("pack.yml"),
        "ref: Other\nname: Other\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        pack.join("super-ingot.item.yml"),
        "type: item\nitems:\n  - id: SuperIngot\n",
    )
    .unwrap();
    fs::write(
        other.join("foreign.item.yml"),
        "type: item\nitems:\n  - id: Foreign\n",
    )
    .unwrap();
    fs::write(
        pack.join("Recipes/super-ingot.recipe.yml"),
        "type: recipe\ninclude:\n  - super-ingot.item.yml\n  - ../Other/foreign.item.yml\nrecipes:\n  - id: Recipe_SuperIngot\n",
    )
    .unwrap();

    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    assert!(
        !report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "semantic.include-missing"
                && diagnostic.message.contains("super-ingot.item.yml")
        }),
        "{:?}",
        report.diagnostics
    );
    assert!(
        report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "semantic.include-escape"
                && diagnostic.message.contains("../Other/foreign.item.yml")
        }),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn include_document_suffix_matches_only_existing_multi_documents() {
    let project = tempdir().unwrap();
    manifest(project.path(), "");
    fs::write(
        project.path().join("super-ingot.item.yml"),
        "type: item\nitems:\n  - id: SuperIngotA\n---\ntype: item\nitems:\n  - id: SuperIngotB\n---\n",
    )
    .unwrap();
    fs::write(
        project.path().join("super-ingot.recipe.yml"),
        "type: recipe\ninclude:\n  - super-ingot.item.yml[1]\n  - super-ingot.item.yml[2]\n  - super-ingot.item.yml[01]\n  - super-ingot.item.yml[0]\n  - super-ingot.item.yml\nrecipes:\n  - id: Recipe_SuperIngot\n",
    )
    .unwrap();

    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    let missing = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.include-missing")
        .collect::<Vec<_>>();
    assert_eq!(missing.len(), 2, "{:?}", report.diagnostics);
    assert_eq!(missing[0].yaml_path, "$.include[1]");
    assert!(missing[0].message.contains("super-ingot.item.yml[2]"));
    assert_eq!(missing[1].yaml_path, "$.include[2]");
    assert!(missing[1].message.contains("super-ingot.item.yml[01]"));
}

#[test]
fn pack_references_and_dependencies_match_runtime_case_exactly() {
    let project = tempdir().unwrap();
    let faxit = project.path().join("Faxit");
    let dependent = project.path().join("Dependent");
    fs::create_dir(&faxit).unwrap();
    fs::create_dir(&dependent).unwrap();
    fs::write(
        faxit.join("pack.yml"),
        "ref: FaxitReborn\nname: Faxit Reborn\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        dependent.join("pack.yml"),
        "ref: DependsOnFaxit\nname: Dependent\nversion: 1.0.0\ndependencies: [faxitreborn]\n",
    )
    .unwrap();

    let missing = lint_path(project.path(), &LintOptions::default()).unwrap();
    assert!(missing
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "semantic.dependency-missing"));

    let lower = project.path().join("Lowercase");
    fs::create_dir(&lower).unwrap();
    fs::write(
        lower.join("pack.yml"),
        "ref: faxitreborn\nname: Lowercase Pack\nversion: 1.0.0\n",
    )
    .unwrap();
    let distinct = lint_path(project.path(), &LintOptions::default()).unwrap();
    assert!(
        !distinct.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "semantic.dependency-missing"
                || diagnostic.code == "semantic.duplicate-pack-ref"
        }),
        "{:?}",
        distinct.diagnostics
    );
}

#[test]
fn asset_file_must_be_relative_even_when_absolute_path_stays_inside_pack() {
    let project = tempdir().unwrap();
    manifest(project.path(), "");
    let image = project.path().join("mod-logo.png");
    fs::write(&image, b"fixture image bytes").unwrap();
    let absolute = image.to_string_lossy().replace('\\', "/");
    fs::write(
        project.path().join("super-ingot-icon.asset.yml"),
        format!(
            "type: asset\nassets:\n  - id: RelativeIcon\n    file: mod-logo.png\n  - id: AbsoluteIcon\n    file: {absolute}\n"
        ),
    )
    .unwrap();

    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    let escapes = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.asset-escape")
        .collect::<Vec<_>>();
    assert_eq!(escapes.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(escapes[0].yaml_path, "$.assets[1].file");
}

#[test]
fn resource_node_removal_accepts_explicit_resource_selector() {
    let project = tempdir().unwrap();
    manifest(project.path(), "");
    let path = project.path().join("purge-uranium.resourcenode.yml");
    let valid = "type: resourcenode\nremove:\n  - /Game/FactoryGame/Resource/RawResources/OreUranium/Desc_OreUranium.Desc_OreUranium_C\n  - resource: Desc_OreGold_C\n    nodeTypes: [Node, Deposit]\n";
    fs::write(&path, valid).unwrap();
    let accepted = lint_path(project.path(), &LintOptions::default()).unwrap();
    assert!(accepted.ok, "{:?}", accepted.diagnostics);

    fs::write(&path, format!("{valid}  - {{}}\n")).unwrap();
    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    let missing_selector = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.scope-filter")
        .collect::<Vec<_>>();
    assert_eq!(missing_selector.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(missing_selector[0].yaml_path, "$.remove[2]");
}

#[test]
fn cdo_warns_when_of_class_has_no_scope_filter() {
    let project = tempdir().unwrap();
    manifest(project.path(), "");
    fs::write(
        project.path().join("rebalance.cdo.yml"),
        "type: cdo\npatches:\n  - target: /Game/FactoryGame/Recipes/Constructor/Recipe_IronPlate.Recipe_IronPlate_C\n    ofClass: /Script/FactoryGame.FGRecipe\n    properties:\n      - path: mManufactoringDuration\n        value: 5\n  - matchTag: KDFSample.Rebalanced\n    ofClass: /Script/FactoryGame.FGRecipe\n    properties:\n      - path: mManufactoringDuration\n        op: multiply\n        value: 0.8\n",
    )
    .unwrap();

    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    let unused = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.scope-unused")
        .collect::<Vec<_>>();
    assert_eq!(unused.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(unused[0].yaml_path, "$.patches[0].ofClass");
    assert_eq!(unused[0].severity, Severity::Warning);
}

#[test]
fn divide_operation_reports_zero_operand_before_runtime() {
    let project = tempdir().unwrap();
    manifest(project.path(), "");
    fs::write(
        project.path().join("operations.item.yml"),
        "type: item\nitems:\n  - id: DivideExample\n    properties:\n      - path: mStackSize\n        op: divide\n        value: 2\n      - path: mStackSize\n        op: divide\n        value: 0\n      - path: mStackSize\n        op: divide\n        value: \"0\"\n",
    )
    .unwrap();

    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    let zero = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.divide-zero")
        .collect::<Vec<_>>();
    assert_eq!(zero.len(), 2, "{:?}", report.diagnostics);
    assert_eq!(zero[0].yaml_path, "$.items[0].properties[1].value");
    assert_eq!(zero[0].severity, Severity::Error);
    assert_eq!(zero[1].yaml_path, "$.items[0].properties[2].value");
}

#[test]
fn array_operation_indices_stay_in_nonnegative_i32_range() {
    let project = tempdir().unwrap();
    manifest(project.path(), "");
    fs::write(
        project.path().join("operations.recipe.yml"),
        "type: recipe\nrecipes:\n  - id: Recipe_ArrayIndex\n    properties:\n      - path: mIngredients\n        op: insert\n        index: 0\n        value: 1\n      - path: mIngredients\n        op: insert\n        index: -1\n        value: 1\n      - path: mIngredients\n        op: insert\n        index: 4294967296\n        value: 1\n      - path: mIngredients\n        op: swap\n        index: 0\n        with: -2\n",
    )
    .unwrap();

    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    let invalid = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.index-range")
        .map(|diagnostic| diagnostic.yaml_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        invalid,
        [
            "$.recipes[0].properties[1].index",
            "$.recipes[0].properties[2].index",
            "$.recipes[0].properties[3].with",
        ],
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn generated_class_and_asset_ids_use_separate_runtime_namespaces() {
    let project = tempdir().unwrap();
    fs::write(
        project.path().join("pack.yml"),
        "ref: KDFSample\nname: Sample\nversion: 1.0.0\nredirects:\n  IconOnly: Renamed\n",
    )
    .unwrap();
    fs::write(project.path().join("mod-logo.png"), b"fixture image bytes").unwrap();
    fs::write(
        project.path().join("super-ingot-icon.asset.yml"),
        "type: asset\nassets:\n  - id: Shared\n    file: mod-logo.png\n  - id: IconOnly\n    file: mod-logo.png\n",
    )
    .unwrap();
    fs::write(
        project.path().join("super-ingot.item.yml"),
        "type: item\nitems:\n  - id: Shared\n    parent: /Script/FactoryGame.FGItemDescriptor\n",
    )
    .unwrap();

    let separate = lint_path(project.path(), &LintOptions::default()).unwrap();
    assert!(
        !separate.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "semantic.duplicate-id"
                || diagnostic.code == "semantic.redirect-source-present"
        }),
        "{:?}",
        separate.diagnostics
    );

    fs::write(
        project.path().join("task.dataasset.yml"),
        "type: dataasset\nassets:\n  - id: Shared\n    class: /Script/KAPI.KAPIDeliveryTask\n",
    )
    .unwrap();
    let shared_asset = lint_path(project.path(), &LintOptions::default()).unwrap();
    let duplicates = shared_asset
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.duplicate-id")
        .collect::<Vec<_>>();
    assert_eq!(duplicates.len(), 1, "{:?}", shared_asset.diagnostics);
    assert_eq!(duplicates[0].yaml_path, "$.assets[0].id");
}

#[test]
fn generated_ids_that_sanitize_to_same_runtime_name_are_rejected() {
    let project = tempdir().unwrap();
    manifest(project.path(), "redirects:\n  Legacy-Id: NewId\n");
    fs::write(
        project.path().join("super-ingot.item.yml"),
        "type: item\nitems:\n  - id: Super-Ingot\n    parent: /Script/FactoryGame.FGItemDescriptor\n  - id: Super_Ingot\n    parent: /Script/FactoryGame.FGItemDescriptor\n  - id: Legacy_Id\n    parent: /Script/FactoryGame.FGItemDescriptor\n",
    )
    .unwrap();

    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    let collisions = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.duplicate-id")
        .collect::<Vec<_>>();
    assert_eq!(collisions.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(collisions[0].yaml_path, "$.items[1].id");
    assert!(collisions[0].message.contains("runtime"));
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "semantic.redirect-source-present"
            && diagnostic.yaml_path == "$.redirects.Legacy-Id"
    }));
}

#[test]
fn generated_identity_collision_crosses_packs_with_same_sanitized_ref() {
    let project = tempdir().unwrap();
    let dashed = project.path().join("Dashed");
    let underscored = project.path().join("Underscored");
    fs::create_dir(&dashed).unwrap();
    fs::create_dir(&underscored).unwrap();
    fs::write(
        dashed.join("pack.yml"),
        "ref: KDF-Sample\nname: Dashed\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        underscored.join("pack.yml"),
        "ref: KDF_Sample\nname: Underscored\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        dashed.join("super-ingot.item.yml"),
        "type: item\nitems:\n  - id: SuperIngot\n    parent: /Script/FactoryGame.FGItemDescriptor\n",
    )
    .unwrap();
    fs::write(
        underscored.join("super-ingot.item.yml"),
        "type: item\nitems:\n  - id: SuperIngot\n    parent: /Script/FactoryGame.FGItemDescriptor\n  - id: Different\n    parent: /Script/FactoryGame.FGItemDescriptor\n",
    )
    .unwrap();

    let report = lint_path(project.path(), &LintOptions::default()).unwrap();
    let collision = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "semantic.runtime-identity-collision")
        .collect::<Vec<_>>();
    assert_eq!(collision.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(collision[0].yaml_path, "$.items[0].id");
    assert_eq!(
        collision[0].file.as_deref(),
        Some(
            dunce::canonicalize(underscored.join("super-ingot.item.yml"))
                .unwrap()
                .as_path()
        )
    );
}
