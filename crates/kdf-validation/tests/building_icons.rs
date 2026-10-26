use kdf_validation::ReflectionIndex;
use serde_json::{json, Value};
use std::{fs::File, io::Write};

fn archive(descriptors: Vec<Value>, flattened: Vec<Value>) -> tempfile::NamedTempFile {
    let file = tempfile::Builder::new()
        .suffix(".kdfdata")
        .tempfile()
        .unwrap();
    let mut zip = zip::ZipWriter::new(File::create(file.path()).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    let mut records = vec![
        json!({"path":"/Script/FactoryGame.FGBuildable","properties":[]}),
        json!({"path":"/Script/FactoryGame.FGBuildingDescriptor","properties":[]}),
        json!({"path":"/Game/Test.Build_Test_C","super":"/Script/FactoryGame.FGBuildable","properties":[]}),
    ];
    records.extend(descriptors);
    zip.start_file("manifest.json", options).unwrap();
    write!(
        zip,
        "{}",
        json!({"schemaVersion":1,"counts":{"classes":records.len()}})
    )
    .unwrap();
    zip.start_file("classes.ndjson", options).unwrap();
    for record in records {
        writeln!(zip, "{record}").unwrap();
    }
    if !flattened.is_empty() {
        zip.start_file("classes.flat.ndjson", options).unwrap();
        for record in flattened {
            writeln!(zip, "{record}").unwrap();
        }
    }
    zip.finish().unwrap();
    file
}

fn descriptor(path: &str, value: &str) -> Value {
    json!({"path":path,"super":"/Script/FactoryGame.FGBuildingDescriptor","properties":[{"name":"mBuildableClass","default":value}]})
}

#[test]
fn flattened_export_indexes_exact_unique_descriptor_for_building_icons() {
    let desc = "/Game/Test.Desc_Test_C";
    let file = archive(
        vec![
            json!({"path":desc,"super":"/Script/FactoryGame.FGBuildingDescriptor","properties":[]}),
        ],
        vec![descriptor(
            desc,
            "/Script/Engine.BlueprintGeneratedClass'/Game/Test.Build_Test_C'",
        )],
    );
    let index = ReflectionIndex::load(file.path());
    let summaries = serde_json::to_value(&index.classes).unwrap();
    assert_eq!(
        summaries
            .as_array()
            .unwrap()
            .iter()
            .find(|class| class["path"] == "/Game/Test.Build_Test_C")
            .unwrap()["iconDescriptorPath"],
        desc
    );
    // Preserve lazy/raw metadata: relationship is a summary hint, not an invented texture/default.
    assert!(index
        .class_info("/Game/Test.Build_Test_C")
        .unwrap()
        .get("mSmallIcon")
        .is_none());
}

#[test]
fn ambiguous_or_unrelated_descriptor_fields_leave_building_hint_absent() {
    let file = archive(
        vec![
            descriptor("/Game/Test.Desc_One_C", "/Game/Test.Build_Test_C"),
            descriptor("/Game/Test.Desc_Two_C", "/Game/Test.Build_Test_C"),
        ],
        vec![],
    );
    let index = ReflectionIndex::load(file.path());
    let summaries = serde_json::to_value(&index.classes).unwrap();
    assert!(summaries
        .as_array()
        .unwrap()
        .iter()
        .find(|class| class["path"] == "/Game/Test.Build_Test_C")
        .unwrap()
        .get("iconDescriptorPath")
        .is_none());
    let file = archive(
        vec![
            json!({"path":"/Game/Test.NotDescriptor_C","properties":[{"name":"mBuildableClass","default":"/Game/Test.Build_Test_C"}]}),
        ],
        vec![],
    );
    let index = ReflectionIndex::load(file.path());
    let summaries = serde_json::to_value(&index.classes).unwrap();
    assert!(summaries
        .as_array()
        .unwrap()
        .iter()
        .find(|class| class["path"] == "/Game/Test.Build_Test_C")
        .unwrap()
        .get("iconDescriptorPath")
        .is_none());
}
