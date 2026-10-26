use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::Command;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../kdf-validation/tests/fixtures/legacy/DataForge")
}

#[test]
fn json_and_sarif_report_same_diagnostic_identity() {
    let json = Command::new(env!("CARGO_BIN_EXE_kdf"))
        .args([
            "lint",
            fixture().to_str().unwrap(),
            "--format",
            "json",
            "--profile",
            "legacy",
        ])
        .output()
        .unwrap();
    assert_eq!(json.status.code(), Some(1));
    let result: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(
        (result["packs"].as_u64(), result["documents"].as_u64()),
        (Some(1), Some(1))
    );
    assert_eq!(
        result["diagnostics"][0]["code"],
        "schema.unevaluatedProperties"
    );

    let sarif = Command::new(env!("CARGO_BIN_EXE_kdf"))
        .args(["lint", fixture().to_str().unwrap(), "--format", "sarif"])
        .output()
        .unwrap();
    assert_eq!(sarif.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&sarif.stdout).unwrap();
    assert_eq!(report["version"], "2.1.0");
    assert_eq!(
        report["runs"][0]["results"][0]["ruleId"],
        result["diagnostics"][0]["code"]
    );
    assert!(
        report["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]
            ["uri"]
            .as_str()
            .unwrap()
            .starts_with("file:///")
    );
}

#[test]
fn schema_errors_return_legacy_exit_code_two() {
    let output = Command::new(env!("CARGO_BIN_EXE_kdf"))
        .args([
            "schemas",
            "check",
            "--schema-dir",
            "missing-schema-directory",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("schema error"));
}

#[test]
fn preview_datapack_is_json_and_does_not_create_binding() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("pack.yml"),
        "ref: Pack\nname: Pack\nversion: 1.0.0\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join("item.unlock.yml"),
        "type: unlock\nunlocks:\n  - id: One\n    parent: /Script/Test.A\n",
    )
    .unwrap();
    let archive = project.path().join("candidate.kdfdata");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", options).unwrap();
    zip.write_all(b"{\"schemaVersion\":1,\"counts\":{\"classes\":1},\"failedLoads\":[]}")
        .unwrap();
    zip.start_file("classes.ndjson", options).unwrap();
    zip.write_all(b"{\"path\":\"/Script/Test.A\",\"name\":\"A\",\"kind\":\"native\",\"flags\":{\"names\":[]},\"properties\":[]}\n").unwrap();
    zip.finish().unwrap();
    let before = std::fs::read(project.path().join("item.unlock.yml")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_kdf"))
        .args([
            "preview-datapack",
            project.path().to_str().unwrap(),
            archive.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let preview: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(preview["candidateStatus"], "ready");
    assert_eq!(preview["removed"][0]["diagnostic"]["code"], "class.unknown");
    assert_eq!(preview["removed"][0]["path"], "item.unlock.yml");
    assert!(!project.path().join(".kdf-editor.toml").exists());
    assert_eq!(
        std::fs::read(project.path().join("item.unlock.yml")).unwrap(),
        before
    );
}

#[test]
fn project_lint_settings_change_default_format_and_exit_policy() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("pack.yml"),
        "ref: Pack\nname: Pack\nversion: 1.0.0\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join("custom.foo.yml"),
        "type: foo\nvalue: 1\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join(".kdf-editor.toml"),
        "[lint]\nallow_unknown_types = true\nwarnings_as_errors = true\noutput_format = 'json'\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_kdf"))
        .args(["lint", project.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["warnings"], 1);
    assert_eq!(result["diagnostics"][0]["code"], "schema.unknown-type");
}

#[test]
fn desktop_schema_only_override_ignores_legacy_binding() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("pack.yml"),
        "ref: Pack\nname: Pack\nversion: 1.0.0\n",
    )
    .unwrap();
    let missing = project.path().join("missing.kdfdata");
    std::fs::write(
        project.path().join(".kdf-editor.toml"),
        format!(
            "[datapack]\npath = {}\n",
            serde_json::to_string(&missing.to_string_lossy().as_ref()).unwrap()
        ),
    )
    .unwrap();

    let legacy = Command::new(env!("CARGO_BIN_EXE_kdf"))
        .args(["lint", project.path().to_str().unwrap(), "--format", "json"])
        .env_remove("KDF_EFFECTIVE_DATAPACK_PATH")
        .output()
        .unwrap();
    assert_eq!(legacy.status.code(), Some(0));
    let legacy_report: Value = serde_json::from_slice(&legacy.stdout).unwrap();
    assert!(legacy_report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["code"] == "datapack.unavailable"));

    let desktop = Command::new(env!("CARGO_BIN_EXE_kdf"))
        .args(["lint", project.path().to_str().unwrap(), "--format", "json"])
        .env("KDF_EFFECTIVE_DATAPACK_PATH", "")
        .output()
        .unwrap();
    assert_eq!(desktop.status.code(), Some(0));
    let desktop_report: Value = serde_json::from_slice(&desktop.stdout).unwrap();
    assert!(!desktop_report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["code"] == "datapack.unavailable"));
}
