use kdf_validation::{ReflectionIndex, ReflectionStatus};
use serde_json::{json, Value};
use std::{fs::File, io::Write};

fn index(extra: Vec<Value>, failed: bool) -> ReflectionIndex {
    let file = tempfile::Builder::new()
        .suffix(".kdfdata")
        .tempfile()
        .unwrap();
    let mut archive = zip::ZipWriter::new(File::create(file.path()).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    archive.start_file("manifest.json", options).unwrap();
    write!(archive, "{}", json!({"schemaVersion":1,"counts":{"classes":extra.len()},"failedLoads":if failed {vec![json!("omitted")] } else {vec![]}})).unwrap();
    archive.start_file("classes.ndjson", options).unwrap();
    for record in extra {
        writeln!(archive, "{record}").unwrap();
    }
    archive.finish().unwrap();
    // The input is a temporary archive; returned inference is entirely indexed.
    ReflectionIndex::load(file.path())
}

#[test]
fn indexed_owner_requires_unique_typed_export_and_complete_snapshot() {
    let owner = "/Script/Test.Owner";
    let other = "/Script/Test.Other";
    let records = vec![
        json!({"path":owner,"properties":[{"name":"mUnique","ownerPath":owner,"cppType":"float"},{"name":"mShared","ownerPath":owner,"cppType":"float"}]}),
        json!({"path":other,"properties":[{"name":"mShared","ownerPath":other,"cppType":"float"},{"name":"mUntyped","ownerPath":other}]}),
    ];
    let complete = index(records.clone(), false);
    assert_eq!(complete.status, ReflectionStatus::Ready);
    assert_eq!(complete.infer_property_owner(&["mUnique"]), Some(owner));
    assert_eq!(
        complete.infer_property_owner(&["mUnique.Amount"]),
        Some(owner)
    );
    assert_eq!(
        complete.infer_property_owner(&["mUnique", "mShared"]),
        Some(owner)
    );
    assert!(complete.infer_property_owner(&["mShared"]).is_none());
    assert!(complete.infer_property_owner(&["mUntyped"]).is_none());
    assert!(complete.infer_property_owner(&["mUnknown"]).is_none());
    assert!(index(records, true)
        .infer_property_owner(&["mUnique"])
        .is_none());
}

#[test]
fn inconsistent_or_unavailable_declaring_owner_never_proves_inference() {
    let owner = "/Script/Test.Owner";
    let record = index(
        vec![
            json!({"path":owner,"properties":[{"name":"mConflict","ownerPath":owner,"cppType":"float"},{"name":"mMalformed","ownerPath":owner,"cppType":false}]}),
            json!({"path":"/Game/Test.Child_C","properties":[{"name":"mConflict","ownerPath":owner,"cppType":"int32"},{"name":"mMissing","ownerPath":"/Script/Test.Missing","cppType":"float"}]}),
        ],
        false,
    );
    assert_eq!(record.status, ReflectionStatus::Ready);
    assert!(record.infer_property_owner(&["mConflict"]).is_none());
    assert!(record.infer_property_owner(&["mMalformed"]).is_none());
    assert!(record.infer_property_owner(&["mMissing"]).is_none());
}
