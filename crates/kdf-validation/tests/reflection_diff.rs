use kdf_validation::{diff_reflection_archives, ReflectionChangeKind, ReflectionDiffStatus};
use serde_json::{json, Value};
use std::fs::File;
use std::io::Write;
use zip::write::SimpleFileOptions;

fn archive(
    classes: &[Value],
    structs: &[Value],
    enums: &[Value],
    manifest_change: impl FnOnce(&mut Value),
    omit: Option<&str>,
) -> tempfile::NamedTempFile {
    let file = tempfile::Builder::new()
        .suffix(".kdfdata")
        .tempfile()
        .unwrap();
    let mut manifest = json!({
        "schemaVersion": 1,
        "metadataAvailable": true,
        "options": {"assets": true, "functions": true, "flattened": true},
        "failedLoads": [],
        "counts": {"classes": classes.len(), "structs": structs.len(), "enums": enums.len()}
    });
    manifest_change(&mut manifest);
    let mut zip = zip::ZipWriter::new(File::create(file.path()).unwrap());
    zip.start_file("manifest.json", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(manifest.to_string().as_bytes()).unwrap();
    for (name, records) in [
        ("classes.ndjson", classes),
        ("structs.ndjson", structs),
        ("enums.ndjson", enums),
    ] {
        if omit == Some(name) {
            continue;
        }
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        for record in records {
            writeln!(zip, "{record}").unwrap();
        }
    }
    zip.finish().unwrap();
    file
}

fn archive_with_prefixes(prefixes: &[&str]) -> tempfile::NamedTempFile {
    let file = tempfile::Builder::new()
        .suffix(".kdfdata")
        .tempfile()
        .unwrap();
    let mut zip = zip::ZipWriter::new(File::create(file.path()).unwrap());
    for prefix in prefixes {
        zip.start_file(
            format!("{prefix}manifest.json"),
            SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(br#"{"schemaVersion":1,"metadataAvailable":true,"options":{"assets":true,"functions":true,"flattened":true},"failedLoads":[],"counts":{"classes":0,"structs":0,"enums":0}}"#)
            .unwrap();
        zip.start_file(
            format!("{prefix}classes.ndjson"),
            SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"").unwrap();
    }
    zip.finish().unwrap();
    file
}

#[test]
fn rejects_competing_reflection_manifests_at_every_supported_root_pair() {
    let valid = archive(&[], &[], &[], |_| {}, None);
    for prefix in ["", "reflection/", "kdf/reflection/"] {
        let selected = archive_with_prefixes(&[prefix]);
        assert!(
            diff_reflection_archives(selected.path(), valid.path()).is_ok(),
            "single root {prefix} must remain supported"
        );
    }

    for prefixes in [
        &["", "reflection/"][..],
        &["", "kdf/reflection/"][..],
        &["reflection/", "kdf/reflection/"][..],
    ] {
        let ambiguous = archive_with_prefixes(prefixes);
        assert_eq!(
            diff_reflection_archives(ambiguous.path(), valid.path()).unwrap_err(),
            "datapack has multiple KDF reflection manifests",
            "{prefixes:?}"
        );
    }

    let missing = archive_with_prefixes(&[]);
    assert_eq!(
        diff_reflection_archives(missing.path(), valid.path()).unwrap_err(),
        "datapack has no KDF reflection manifest"
    );
}

#[test]
fn reports_class_property_struct_and_enum_changes() {
    let baseline = archive(
        &[
            json!({"path":"/A","properties":[{"name":"P","cppType":"int"}]}),
            json!({"path":"/Gone","properties":[]}),
        ],
        &[json!({"path":"/S","properties":[{"name":"Field","kind":"Int"}]})],
        &[json!({"path":"/E","entries":[{"name":"One","value":1}]})],
        |_| {},
        None,
    );
    let candidate = archive(
        &[
            json!({"path":"/A","properties":[{"name":"P","cppType":"float"},{"name":"Q","cppType":"int"}]}),
            json!({"path":"/New","properties":[]}),
        ],
        &[json!({"path":"/S","properties":[{"name":"Field","kind":"Float"}]})],
        &[json!({"path":"/E","entries":[{"name":"One","value":2}]})],
        |_| {},
        None,
    );
    let diff = diff_reflection_archives(baseline.path(), candidate.path()).unwrap();
    assert_eq!(diff.status, ReflectionDiffStatus::Ready);
    assert_eq!(
        diff.classes
            .iter()
            .map(|entry| (entry.path.as_str(), entry.change))
            .collect::<Vec<_>>(),
        [
            ("/A", ReflectionChangeKind::Changed),
            ("/Gone", ReflectionChangeKind::Removed),
            ("/New", ReflectionChangeKind::Added),
        ]
    );
    assert_eq!(
        diff.properties
            .iter()
            .map(|entry| (entry.owner_path.as_str(), entry.name.as_str(), entry.change))
            .collect::<Vec<_>>(),
        [
            ("/A", "P", ReflectionChangeKind::Changed),
            ("/A", "Q", ReflectionChangeKind::Added),
            ("/S", "Field", ReflectionChangeKind::Changed),
        ]
    );
    assert_eq!(diff.structs[0].change, ReflectionChangeKind::Changed);
    assert_eq!(diff.enums[0].change, ReflectionChangeKind::Changed);
}

#[test]
fn incomplete_side_does_not_claim_missing_records_were_added_or_removed() {
    let baseline = archive(
        &[json!({"path":"/Old","properties":[]})],
        &[],
        &[],
        |manifest| manifest["failedLoads"] = json!([{"path":"/Failed","reason":"load error"}]),
        None,
    );
    let candidate = archive(
        &[json!({"path":"/New","properties":[]})],
        &[],
        &[],
        |manifest| manifest["metadataAvailable"] = json!(false),
        None,
    );
    let diff = diff_reflection_archives(baseline.path(), candidate.path()).unwrap();
    assert_eq!(diff.status, ReflectionDiffStatus::Limited);
    assert_eq!(diff.classes.len(), 2);
    assert!(diff
        .classes
        .iter()
        .all(|entry| entry.change == ReflectionChangeKind::Unknown));
    assert!(diff
        .reasons
        .iter()
        .any(|reason| reason.contains("failedLoads")));
    assert!(diff
        .reasons
        .iter()
        .any(|reason| reason.contains("metadata")));
}

#[test]
fn failed_loads_only_weaken_the_side_that_could_have_omitted_paths() {
    let baseline = archive(
        &[json!({"path":"/Old","properties":[]})],
        &[],
        &[],
        |manifest| manifest["failedLoads"] = json!([{"path":"/Unloaded"}]),
        None,
    );
    let candidate = archive(
        &[json!({"path":"/New","properties":[]})],
        &[],
        &[],
        |_| {},
        None,
    );
    let diff = diff_reflection_archives(baseline.path(), candidate.path()).unwrap();
    assert_eq!(diff.status, ReflectionDiffStatus::Limited);
    assert_eq!(diff.classes[0].change, ReflectionChangeKind::Unknown);
    assert_eq!(diff.classes[1].change, ReflectionChangeKind::Removed);
}

#[test]
fn count_mismatch_marks_absent_candidate_records_unknown() {
    let baseline = archive(
        &[json!({"path":"/Old","properties":[]})],
        &[],
        &[],
        |_| {},
        None,
    );
    let candidate = archive(
        &[json!({"path":"/New","properties":[]})],
        &[],
        &[],
        |manifest| manifest["counts"]["classes"] = json!(2),
        None,
    );
    let diff = diff_reflection_archives(baseline.path(), candidate.path()).unwrap();
    assert_eq!(diff.status, ReflectionDiffStatus::Limited);
    assert_eq!(diff.classes[0].change, ReflectionChangeKind::Added);
    assert_eq!(diff.classes[1].change, ReflectionChangeKind::Unknown);
}

#[test]
fn missing_property_list_is_limited_and_does_not_claim_removal() {
    let baseline = archive(
        &[json!({"path":"/A","properties":[{"name":"P","cppType":"int"}]})],
        &[],
        &[],
        |_| {},
        None,
    );
    let candidate = archive(&[json!({"path":"/A"})], &[], &[], |_| {}, None);
    let diff = diff_reflection_archives(baseline.path(), candidate.path()).unwrap();
    assert_eq!(diff.status, ReflectionDiffStatus::Limited);
    assert_eq!(diff.properties[0].change, ReflectionChangeKind::Unknown);
    assert!(diff
        .reasons
        .iter()
        .any(|reason| reason.contains("property list")));
}

#[test]
fn absent_section_is_unknown_and_wrong_extension_is_rejected() {
    let baseline = archive(
        &[],
        &[json!({"path":"/Struct","properties":[]})],
        &[],
        |_| {},
        None,
    );
    let candidate = archive(&[], &[], &[], |_| {}, Some("structs.ndjson"));
    let diff = diff_reflection_archives(baseline.path(), candidate.path()).unwrap();
    assert_eq!(diff.status, ReflectionDiffStatus::Limited);
    assert_eq!(diff.structs[0].change, ReflectionChangeKind::Unknown);

    let wrong_extension = tempfile::Builder::new().suffix(".zip").tempfile().unwrap();
    assert!(
        diff_reflection_archives(wrong_extension.path(), candidate.path())
            .unwrap_err()
            .contains(".kdfdata")
    );
}

#[test]
fn accepts_vanilla_scale_class_and_struct_records_within_bounded_limit() {
    let large_class = json!({
        "path": "/Script/Test.LargeClass",
        "properties": [{"name": "mData", "defaultValue": "x".repeat(1_500_000)}]
    });
    let large_struct = json!({
        "path": "/Script/Test.LargeStruct",
        "properties": [{"name": "mData", "defaultValue": "x".repeat(1_350_000)}]
    });
    let baseline = archive(
        std::slice::from_ref(&large_class),
        std::slice::from_ref(&large_struct),
        &[],
        |_| {},
        None,
    );
    let candidate = archive(&[large_class], &[large_struct], &[], |_| {}, None);
    let diff = diff_reflection_archives(baseline.path(), candidate.path()).unwrap();
    assert_eq!(diff.status, ReflectionDiffStatus::Ready);
    assert!(diff.classes.is_empty());
    assert!(diff.structs.is_empty());
}
