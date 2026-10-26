use super::*;

#[test]
fn class_sequence_uses_active_property_and_preserves_yaml_prefix() {
    let target = "/Game/Recipes/InlineRecipe.InlineRecipe_C";
    let building = "/Game/Factory/Build_Packager.Build_Packager_C";
    let mut session = Session::new();
    let mut zip =
        zip::ZipWriter::new(fs::File::create(session.root.path().join("game.kdfdata")).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", options).unwrap();
    zip.write_all(br#"{"schemaVersion":1,"counts":{"classes":2},"failedLoads":[]}"#)
        .unwrap();
    zip.start_file("classes.ndjson", options).unwrap();
    for class in [
        json!({"path":target,"name":"InlineRecipe","super":"/Script/FactoryGame.FGRecipe","properties":[
            {"name":"mProducedIn","cppType":"TArray<TSoftClassPtr<AActor>>","kind":"ArrayProperty","inner":{"metaClass":"/Script/Engine.Actor"}}
        ]}),
        json!({"path":building,"name":"Packager","super":"/Script/Engine.Actor","properties":[]}),
    ] {
        writeln!(zip, "{class}").unwrap();
    }
    zip.finish().unwrap();
    let text = format!("type: cdo\npatches:\n  - target: {target}\n    properties:\n      - path: mProducedIn\n        value:\n          - /Game/Factory/Build_");
    let result = session.at("textDocument/completion", &text, 6, 32);
    let items = result
        .as_array()
        .expect("class array scalar must offer completions");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["textEdit"]["newText"], building);
    assert_eq!(items[0]["textEdit"]["range"]["start"]["character"], 12);
}

#[test]
fn mistyped_operation_offers_reflected_choices_and_preserves_comment() {
    let mut session = Session::new();
    let text = format!("type: cdo\npatches:\n  - target: {RECIPE}\n    properties:\n      - path: mManufacturingDuration\n        op: addd # preserve comment");
    let result = session.at("textDocument/completion", &text, 5, 16);
    let add = result
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["label"] == "add")
        .expect("typo must not hide valid operation choices");
    assert_eq!(add["textEdit"]["range"]["start"]["character"], 12);
    assert_eq!(add["textEdit"]["range"]["end"]["character"], 16);
    assert_eq!(add["textEdit"]["newText"], "add");
    assert!(add["filterText"].as_str().unwrap().contains("addd"));
}
