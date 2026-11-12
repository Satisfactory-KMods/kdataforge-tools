mod support;
use kdf_validation::{ReflectionIndex, ReflectionStatus};
use serde_json::json;
use std::io::Write;

#[test]
fn aliases_resolve_inherited_and_struct_paths_with_current_names_winning() {
    let file = tempfile::Builder::new()
        .suffix(".kdfdata")
        .tempfile()
        .unwrap();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(file.path()).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", options).unwrap();
    write!(
        zip,
        "{}",
        json!({"schemaVersion":1,"metadataAvailable":true,"counts":{"classes":2},"failedLoads":[]})
    )
    .unwrap();
    zip.start_file("classes.ndjson", options).unwrap();
    for record in [
        support::class(
            "/Script/Test.Base",
            None,
            vec![
                json!({"name":"bEnabled","kind":"BoolProperty","kdf":{"aliases":["mEnabled","Collision"]}}),
            ],
            vec![],
        ),
        support::class(
            "/Script/Test.Child",
            Some("/Script/Test.Base"),
            vec![
                json!({"name":"Collision","kind":"IntProperty"}),
                json!({"name":"Data","kind":"StructProperty","struct":"/Script/Test.Data","kdf":{"aliases":["OldData"]}}),
            ],
            vec![],
        ),
    ] {
        writeln!(zip, "{record}").unwrap();
    }
    zip.start_file("structs.ndjson", options).unwrap();
    writeln!(zip, "{}", json!({"path":"/Script/Test.Data","properties":[{"name":"Value","kind":"IntProperty","kdf":{"aliases":["OldValue"]}}]})).unwrap();
    zip.finish().unwrap();
    let index = ReflectionIndex::load(file.path());
    assert_eq!(index.status, ReflectionStatus::Ready, "{:?}", index.reason);
    let class = "/Script/Test.Child";
    assert_eq!(
        index.find_property(class, "MENABLED").unwrap().name,
        "bEnabled"
    );
    assert_eq!(
        index.find_property(class, "Collision").unwrap().name,
        "Collision"
    );
    assert!(index
        .resolve_property_path_with_renames(class, "Collision")
        .unwrap()
        .1
        .is_empty());
    assert!(index
        .resolve_property_path_with_renames(class, "Data.Value")
        .unwrap()
        .1
        .is_empty());
    let (property, renames) = index
        .resolve_property_path_with_renames(class, "OldData.OldValue")
        .unwrap();
    assert_eq!(property.name, "Value");
    assert_eq!(
        renames,
        vec![
            (0, "OldData".into(), "Data".into()),
            (8, "OldValue".into(), "Value".into())
        ]
    );
    assert!(index.find_property(class, "Missing").is_none());
    let options = kdf_validation::LintOptions {
        datapack_index: Some(std::sync::Arc::new(index)),
        ..Default::default()
    };
    for (path, count) in [
        ("bEnabled", 0),
        ("mEnabled", 1),
        ("Collision", 0),
        ("OldData.Value", 1),
        ("Data.OldValue", 1),
    ] {
        let yaml = format!("type: cdo\npatches:\n  - target: /Script/Test.Child\n    properties:\n      - path: {path}\n        op: clear\n");
        let result = kdf_validation::validate_document(
            std::path::Path::new("aliases.kdf.yml"),
            &yaml,
            &options,
        )
        .unwrap();
        let hints = result
            .diagnostics
            .iter()
            .filter(|d| d.code == "property.renamed")
            .collect::<Vec<_>>();
        assert_eq!(hints.len(), count, "{path}: {:?}", result.diagnostics);
        for hint in hints {
            assert_eq!(hint.severity, kdf_validation::Severity::Information);
            assert!(hint.evidence.as_ref().unwrap().replacement.is_some());
        }
        assert!(!result
            .diagnostics
            .iter()
            .any(|d| d.code == "datapack.unknown-property"));
    }
    let yaml = "type: cdo\npatches:\n  - target: /Script/Test.Child\n    properties:\n      mEnabled: true\n";
    let result =
        kdf_validation::validate_document(std::path::Path::new("mapping.kdf.yml"), yaml, &options)
            .unwrap();
    let hint = result
        .diagnostics
        .iter()
        .find(|d| d.code == "property.renamed")
        .unwrap();
    assert_eq!(hint.range.start.character, 6);
    assert_eq!(hint.range.end.character, 14);
    assert!(hint.evidence.as_ref().unwrap().replacement.is_some());
}
