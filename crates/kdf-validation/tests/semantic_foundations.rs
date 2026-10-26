use kdf_validation::{
    authoring::*, evidence::*, validate_document, GeneratedNamespace, LintOptions, Position,
    ReflectionIndex,
};
use serde_json::{json, Value};
use std::{io::Write, path::Path};

fn input(name: &str, text: &str) -> VersionedInput {
    VersionedInput {
        path: Path::new("/workspace").join(name),
        version: Some(7),
        text: text.into(),
    }
}
fn snapshot(mut extra: Vec<VersionedInput>) -> SemanticSnapshot {
    let mut inputs = vec![input("pack.yml", "ref: Pack-Test\nname: Test\nversion: 1.0.0\n"),
        input("items.item.yml", "type: item\nitems:\n  - id: Item-A\n"),
        input("patch.cdo.yml", "type: cdo\npatches:\n  - target: /KDataForge/Gen/Pack_Test.Gen_Pack_Test_Item_A\n    properties: []\n")];
    inputs.append(&mut extra);
    SemanticSnapshot::from_inputs(
        Path::new("/workspace"),
        inputs,
        &ReflectionIndex::unavailable("none"),
        3,
    )
}
fn at() -> Position {
    Position {
        line: 2,
        character: 9,
    }
}

#[test]
fn current_runtime_script_manifest_and_sdk_starter_validate() {
    let starter = include_str!("fixtures/scripts/typed-starter.pack.yml");
    let result =
        validate_document(Path::new("pack.yml"), starter, &LintOptions::default()).unwrap();
    assert!(result.ok, "{:?}", result.diagnostics);
    let sdk = include_str!("../../../extension/sdk/kdataforgejs/template/DataForge/pack.yml")
        .replace("{{ref}}", "SdkStarter");
    let result = validate_document(Path::new("pack.yml"), &sdk, &LintOptions::default()).unwrap();
    assert!(result.ok, "{:?}", result.diagnostics);
    for aliases in [
        "classes: [/Script/Engine.Actor]",
        "reflectionClasses: [/Script/Engine.Actor]",
    ] {
        let manifest = format!("ref: Test\nname: Test\nversion: 1.0.0\nscripts:\n  - language: kmods.typescript\n    entry: scripts/main.ts\n    {aliases}\n    imports:\n      - key: item/data\n        kind: structured\n        path: files/data.yml\n        digest: '{}'\n", "a".repeat(64));
        assert!(
            validate_document(Path::new("pack.yml"), &manifest, &LintOptions::default())
                .unwrap()
                .ok
        );
    }
}

#[test]
fn script_grammar_rejects_extension_only_and_unsafe_forms() {
    let base = "ref: Test\nname: Test\nversion: 1.0.0\nscripts:\n  - language: kmods.typescript\n    entry: scripts/main.ts\n";
    let bad = [
        format!("{base}    id: manual\n"), format!("{base}    capabilities: []\n"),
        format!("{base}    reflectionClasses: []\n    classes: []\n"),
        format!("{base}    languageVersion: 0\n"), format!("{base}    realm: local\n"),
        base.replace("scripts/main.ts", "../main.ts"), base.replace("scripts/main.ts", "scripts/CON.ts"),
        base.replace("scripts/main.ts", "scripts/other.ts"), base.replace("scripts/main.ts", "scripts//main.ts"),
        base.replace("scripts:\n  - language: kmods.typescript\n    entry: scripts/main.ts", "scripts: {main: scripts/main.ts}"),
        format!("{base}  - language: kmods.typescript\n    entry: scripts/main.ts\n"),
        format!("{base}    imports:\n      - key: x\n        kind: executable\n        path: x.bin\n        sha256: no\n"),
        format!("{base}    imports:\n      - {{key: a, kind: text, path: A.txt, sha256: '{}'}}\n      - {{key: b, kind: text, path: a.TXT, sha256: '{}'}}\n", "b".repeat(64), "a".repeat(64)),
    ];
    for manifest in bad {
        assert!(
            !validate_document(Path::new("pack.yml"), &manifest, &LintOptions::default())
                .unwrap()
                .ok,
            "accepted {manifest}"
        );
    }
}

#[test]
fn identities_and_versions_are_separate_and_stale_snapshots_refuse() {
    let mut data = snapshot(vec![input(
        "icon.asset.yml",
        "type: asset\nassets:\n  - id: Item-A\n",
    )]);
    assert!(data.complete, "{:?}", data.limitations);
    assert_eq!(data.symbols[0].identity.pack, "pack_test");
    assert_eq!(
        ContentIdentity::new("PACK.Test", "item", "item_a"),
        ContentIdentity::new("Pack-Test", "recipe", "Item-A")
    );
    assert_ne!(
        ContentIdentity::new("Pack", "item", "A"),
        ContentIdentity::new("Pack", "asset", "A")
    );
    assert_eq!(data.documents[0].version, Some(7));
    let token = data.snapshot.clone();
    data.snapshot.generation += 1;
    assert!(data
        .rename(&token, Path::new("/workspace/items.item.yml"), at(), "New")
        .unwrap_err()
        .contains("stale"));
}

#[test]
fn loaded_snapshot_matches_runtime_case_insensitive_yaml_extensions() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("pack.yml"),
        "ref: Test\nname: Test\nversion: 1.0.0\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("items.item.YML"),
        "type: item\nitems: [{id: Upper}]\n",
    )
    .unwrap();
    let data =
        SemanticSnapshot::load(root.path(), &[], &ReflectionIndex::unavailable("none"), 1).unwrap();
    assert!(data.complete, "{:?}", data.limitations);
    assert!(data.symbols.iter().any(|symbol| symbol.id == "Upper"));
}

#[test]
fn rename_declaration_and_typed_reference_uses_exact_versioned_ranges() {
    let data = snapshot(vec![]);
    assert!(data.complete, "{:?}", data.limitations);
    assert_eq!(data.references.len(), 1);
    let plan = data
        .rename(
            &data.snapshot,
            Path::new("/workspace/items.item.yml"),
            at(),
            "New.Item",
        )
        .unwrap();
    assert_eq!(plan.documents.len(), 2);
    assert!(plan
        .documents
        .iter()
        .all(|document| document.version == Some(7) && document.digest.len() == 64));
    assert!(plan
        .documents
        .iter()
        .all(|document| !document.path.ends_with("pack.yml")));
    assert_eq!(plan.documents[0].edits[0].expected_text, "Item-A");
    assert_eq!(
        plan.documents[1].edits[0].new_text,
        "/KDataForge/Gen/Pack_Test.Gen_Pack_Test_New_Item"
    );
    let by_reference = data
        .rename(
            &data.snapshot,
            Path::new("/workspace/patch.cdo.yml"),
            Position {
                line: 2,
                character: 20,
            },
            "New.Item",
        )
        .unwrap();
    assert_eq!(by_reference.identity, plan.identity);
}

#[test]
fn rename_refuses_collisions_ambiguity_dynamic_and_untyped_references() {
    for extra in [
        input("duplicate.item.yml", "type: item\nitems: [{id: item_a}]\n"),
        input("dynamic.cdo.yml", "type: cdo\npatches: [{target: '{cfg:target}', properties: []}]\n"),
        input("literal.item.yml", "type: item\nitems: [{id: Other, displayName: '/KDataForge/Gen/Pack_Test.Gen_Pack_Test_Item_A'}]\n"),
        input("broken.item.yml", "type: item\nitems: [\n"),
    ] {
        let data = snapshot(vec![extra]);
        assert!(data.rename(&data.snapshot, Path::new("/workspace/items.item.yml"), at(), "New").is_err());
    }
    let data = snapshot(vec![input(
        "other.item.yml",
        "type: item\nitems: [{id: New_A}]\n",
    )]);
    assert!(data
        .rename(
            &data.snapshot,
            Path::new("/workspace/items.item.yml"),
            at(),
            "new-a"
        )
        .is_err());
    assert!(data
        .rename(
            &data.snapshot,
            Path::new("/workspace/pack.yml"),
            Position {
                line: 0,
                character: 6
            },
            "New"
        )
        .is_err());
    let config = snapshot(vec![input(
        "settings.config.yml",
        "type: config\nfields: [{id: Item-A, type: boolean, default: true}]\n",
    )]);
    assert!(config
        .rename(
            &config.snapshot,
            Path::new("/workspace/settings.config.yml"),
            Position {
                line: 1,
                character: 15
            },
            "New"
        )
        .is_err());
}

#[test]
fn quoted_flow_unicode_ranges_nested_pack_and_asset_namespace() {
    let data = snapshot(vec![
        input("sub/pack.yml", "ref: Child\nname: Child\nversion: 1.0.0\n"),
        input("sub/a.item.yml", "type: item\nitems: [{id: Item-A}]\n"),
        input("a.asset.yml", "type: asset\nassets: [{id: Item-A}]\n"),
        input("unicode.cdo.yml", "type: cdo\npatches: [{label: '🦀', target: '/KDataForge/Gen/Pack_Test.Gen_Pack_Test_Item_A', properties: []}]\n"),
    ]);
    let plan = data
        .rename(
            &data.snapshot,
            Path::new("/workspace/items.item.yml"),
            at(),
            "New",
        )
        .unwrap();
    assert_eq!(plan.documents.len(), 3);
    assert!(plan
        .documents
        .iter()
        .all(|document| !document.path.ends_with("a.asset.yml")
            && !document.path.ends_with("a.item.yml")));
    let unicode = plan
        .documents
        .iter()
        .find(|document| document.path.ends_with("unicode.cdo.yml"))
        .unwrap();
    assert_eq!(unicode.edits[0].range.start.character, 33);
}

#[test]
fn completion_is_bounded_and_reports_incompleteness() {
    let mut text = "type: item\nitems:\n".to_string();
    for index in 0..240 {
        text.push_str(&format!("  - id: Many{index:03}\n"));
    }
    let data = snapshot(vec![input("many.item.yml", &text)]);
    let reflection = ReflectionIndex::unavailable("none");
    let query: CompletionQuery =
        serde_json::from_value(json!({"kind":"content", "query":"Many", "limit":9999})).unwrap();
    let page = complete(&data, &reflection, &query).unwrap();
    assert_eq!(page.items.len(), 200);
    assert!(page.is_incomplete);
    let query: CompletionQuery =
        serde_json::from_value(json!({"kind":"content", "query":"Many239", "limit":1})).unwrap();
    let page = complete(&data, &reflection, &query).unwrap();
    assert_eq!(
        page.items[0].identity.as_ref().unwrap().namespace,
        GeneratedNamespace::Class
    );
    assert!(!page.is_incomplete);
    let query: CompletionQuery = serde_json::from_value(json!({"kind":"classes"})).unwrap();
    assert!(complete(&data, &reflection, &query).unwrap().is_incomplete);
}

fn reflection(metadata: Value, limited: bool) -> (tempfile::NamedTempFile, ReflectionIndex) {
    let file = tempfile::Builder::new()
        .suffix(".kdfdata")
        .tempfile()
        .unwrap();
    let mut zip = zip::ZipWriter::new(file.reopen().unwrap());
    zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    write!(zip, "{}", json!({"schemaVersion":1,"counts":{"classes":1},"failedLoads":[],"metadataAvailable":!limited,
        "replacementMetadata":metadata})).unwrap();
    zip.start_file("classes.ndjson", zip::write::SimpleFileOptions::default())
        .unwrap();
    writeln!(zip,"{}",json!({"path":"/Script/Test.New","name":"New","properties":[{"name":"mNew","kind":"FloatProperty","cppType":"float"}]})).unwrap();
    zip.finish().unwrap();
    let index = ReflectionIndex::load(file.path());
    (file, index)
}

#[test]
fn replacement_requires_verified_authoritative_unique_metadata_and_same_evidence_drives_why() {
    let record = json!({"kind":"class","oldName":"/Script/Test.Old","newName":"/Script/Test.New","authority":"unreal-core-redirects","verified":true});
    let metadata = json!({"schemaVersion":1,"replacements":[record.clone()]});
    let (_file, index) = reflection(metadata.clone(), false);
    assert!(index
        .replacement_evidence("class", None, "/Script/Test.Old")
        .replacement
        .is_some());
    let result = validate_document(
        Path::new("patch.cdo.yml"),
        "type: cdo\npatches: [{target: /Script/Test.Old, properties: []}]\n",
        &LintOptions {
            datapack_index: Some(std::sync::Arc::new(index)),
            ..Default::default()
        },
    )
    .unwrap();
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|item| item.code == "datapack.unknown-class")
        .unwrap();
    let explanation = explain(diagnostic);
    assert_eq!(explanation.evidence.status, "verified");
    assert_eq!(
        explanation.evidence.replacement.unwrap().new_name,
        "/Script/Test.New"
    );
    let mut unverified = record.clone();
    unverified["verified"] = json!(false);
    for (metadata, limited) in [
        (json!({}), false),
        (metadata.clone(), true),
        (
            json!({"schemaVersion":1,"replacements":[record.clone(),record.clone()]}),
            false,
        ),
        (
            json!({"schemaVersion":1,"replacements":[unverified]}),
            false,
        ),
        (
            json!({"schemaVersion":1,"replacements":[{
                "kind":"class","oldName":"/Script/Test.Old","newName":"/Script/Test.New",
                "owner":7,"authority":"unreal-core-redirects","verified":true
            }]}),
            false,
        ),
    ] {
        let (_file, index) = reflection(metadata, limited);
        assert!(index
            .replacement_evidence("class", None, "/Script/Test.Old")
            .replacement
            .is_none());
    }
}

#[test]
fn rename_assets_in_multidoc_yaml_and_typed_audio_refs_preserves_class_namespace() {
    let data = snapshot(vec![
        input("mixed.yml", "type: sound\nsounds:\n  - id: Voice\n    file: voice.wav\n---\ntype: item\nitems:\n  - id: Voice\n"),
        input("audio.cdo.yml", "type: cdo\npatches:\n  - target: /Script/Test.Message\n    properties:\n      - path: mAudioEvent\n        value: Voice # keep comment\n"),
    ]);
    let plan = data
        .rename(
            &data.snapshot,
            Path::new("/workspace/mixed.yml"),
            Position {
                line: 2,
                character: 9,
            },
            "VoiceNew",
        )
        .unwrap();
    assert_eq!(plan.identity.namespace, GeneratedNamespace::Asset);
    assert_eq!(plan.documents.len(), 2);
    let edits = plan
        .documents
        .iter()
        .find(|document| document.path.ends_with("mixed.yml"))
        .unwrap();
    assert_eq!(edits.edits.len(), 1);
    assert_eq!(edits.edits[0].range.start.line, 2);
    let audio = plan
        .documents
        .iter()
        .find(|document| document.path.ends_with("audio.cdo.yml"))
        .unwrap();
    assert_eq!(audio.edits[0].expected_text, "Voice");
}

#[test]
fn unresolved_operands_aliases_and_plain_config_targets_block_rename() {
    for extra in [
        input("operand.cdo.yml","type: cdo\npatches: [{target: /Script/Test.Unknown, properties: [{path: mUnknown, value: Item-A}]}]\n"),
        input("alias.cdo.yml","type: cdo\npatches: [{target: &old /KDataForge/Gen/Pack_Test.Gen_Pack_Test_Item_A, properties: []}, {target: *old, properties: []}]\n"),
        input("cfg.cdo.yml","type: cdo\npatches: [{target: cfg:chosenClass, properties: []}]\n"),
        input("other/pack.yml","ref: pack.test\nname: Collision\nversion: 1.0.0\n"),
    ] {
        let data=snapshot(vec![extra]);
        assert!(data.rename(&data.snapshot,Path::new("/workspace/items.item.yml"),at(),"New").is_err());
    }
}

#[test]
fn snapshot_keeps_event_rule_and_resource_placement_identity_rules() {
    let data = snapshot(vec![
        input(
            "event.eventrule.yml",
            "type: eventrule\nrules: [{id: 1-Event, event: worldReady, actions: []}]\n",
        ),
        input(
            "placements.resourcenode.yml",
            "type: resourcenode\nadd: [{id: Node-A}]\nreplace: [{id: node-a}]\n",
        ),
    ]);
    assert!(data.complete, "{:?}", data.limitations);
    let events: Vec<_> = data
        .symbols
        .iter()
        .filter(|symbol| symbol.identity.namespace == GeneratedNamespace::EventRule)
        .collect();
    assert_eq!(events[0].identity.id, "1-event");
    let placements: Vec<_> = data
        .symbols
        .iter()
        .filter(|symbol| symbol.identity.namespace == GeneratedNamespace::ResourcePlacement)
        .collect();
    assert_eq!(placements.len(), 2);
    assert_ne!(placements[0].identity, placements[1].identity);
}

#[test]
fn reflected_classes_and_assets_have_hard_completion_caps_and_digest_guards() {
    let file = tempfile::Builder::new()
        .suffix(".kdfdata")
        .tempfile()
        .unwrap();
    let mut zip = zip::ZipWriter::new(file.reopen().unwrap());
    zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    write!(
        zip,
        "{}",
        json!({"schemaVersion":1,"counts":{"classes":241},"failedLoads":[]})
    )
    .unwrap();
    zip.start_file("classes.ndjson", zip::write::SimpleFileOptions::default())
        .unwrap();
    writeln!(
        zip,
        "{}",
        json!({"path":"/Script/Test.Base","properties":[]})
    )
    .unwrap();
    for index in 0..240 {
        writeln!(zip,"{}",json!({"path":format!("/Script/Test.Class{index:03}"),"super":"/Script/Test.Base","properties":[]})).unwrap();
    }
    zip.start_file(
        "indexes/assets.json",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    let assets:Vec<_>=(0..240).map(|index|json!({"path":format!("/Game/Assets/A{index:03}.A{index:03}"),"assetClass":"/Script/Test.Base"})).collect();
    write!(zip,"{}",json!({"/Script/Test.Base":{"class":"/Script/Test.Base","total":240,"truncated":false,"assets":assets}})).unwrap();
    zip.finish().unwrap();
    let reflection = ReflectionIndex::load(file.path());
    assert!(reflection.archive_is_current());
    let data = SemanticSnapshot::from_inputs(Path::new("/workspace"), vec![], &reflection, 1);
    for kind in ["classes", "assets"] {
        let query: CompletionQuery = serde_json::from_value(
            json!({"kind":kind,"baseClass":"/Script/Test.Base","limit":20000}),
        )
        .unwrap();
        let page = complete(&data, &reflection, &query).unwrap();
        assert_eq!(page.items.len(), 200);
        assert!(page.complete && page.is_incomplete, "{page:?}");
        assert!(complete(&snapshot(vec![]), &reflection, &query).is_err());
    }
    std::fs::write(file.path(), b"changed").unwrap();
    assert!(!reflection.archive_is_current());
}

#[test]
fn property_replacement_metadata_is_exact_and_rejects_missing_targets_or_authority() {
    let record = json!({"kind":"property","owner":"/Script/Test.New","oldName":"mOld","newName":"mNew","verified":true,"authority":"unreal-core-redirects"});
    let (_file, index) = reflection(
        json!({"schemaVersion":1,"replacements":[record.clone()]}),
        false,
    );
    let lint = validate_document(
        Path::new("patch.cdo.yml"),
        "type: cdo\npatches: [{target: /Script/Test.New, properties: [{path: mOld, value: 1}]}]\n",
        &LintOptions {
            datapack_index: Some(std::sync::Arc::new(index)),
            ..Default::default()
        },
    )
    .unwrap();
    let diagnostic = lint
        .diagnostics
        .iter()
        .find(|item| item.code == "datapack.unknown-property")
        .unwrap();
    assert_eq!(
        diagnostic
            .evidence
            .as_ref()
            .unwrap()
            .replacement
            .as_ref()
            .unwrap()
            .new_name,
        "mNew"
    );
    for (key, value) in [
        ("newName", json!("missing")),
        ("authority", json!("similarity")),
        ("owner", json!("/Script/Test.Other")),
        ("verified", json!(false)),
    ] {
        let mut invalid = record.clone();
        invalid[key] = value;
        let (_file, index) = reflection(json!({"schemaVersion":1,"replacements":[invalid]}), false);
        assert!(index
            .replacement_evidence("property", Some("/Script/Test.New"), "mOld")
            .replacement
            .is_none());
    }
}
