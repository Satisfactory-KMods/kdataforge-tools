use kdf_validation::{ReflectionIndex, ReflectionStatus};
use std::fs::File;
use std::io::Write;

#[test]
fn deeply_nested_property_metadata_cannot_panic_class_lookup() {
    let file = tempfile::Builder::new()
        .suffix(".kdfdata")
        .tempfile()
        .unwrap();
    let mut archive = zip::ZipWriter::new(File::create(file.path()).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    archive.start_file("manifest.json", options).unwrap();
    archive
        .write_all(br#"{"schemaVersion":1,"counts":{"classes":1},"failedLoads":[]}"#)
        .unwrap();
    archive.start_file("classes.ndjson", options).unwrap();
    let nested = format!("{}0{}", "[".repeat(160), "]".repeat(160));
    write!(
        archive,
        "{{\"path\":\"/Script/Test.Deep\",\"properties\":[{{\"name\":\"mDeep\",\"value\":{nested}}}]}}"
    )
    .unwrap();
    archive.finish().unwrap();

    let index = ReflectionIndex::load(file.path());
    assert!(matches!(index.status, ReflectionStatus::Ready));
    assert!(index.class_info("/Script/Test.Deep").is_none());
    // A failed lazy parse is cached and remains harmless on repeated inspection.
    assert!(index.class_info("/Script/Test.Deep").is_none());
}
