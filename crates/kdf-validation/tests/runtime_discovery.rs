use kdf_validation::{lint_path, lint_workspace, validate_document, FileBuffer, LintOptions};
use std::fs;
use std::path::Path;

fn write(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn runtime_discovery_ignores_development_disk_and_unsaved_documents() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("DataForge");
    write(&root, "pack.yml", "ref: Root\nname: Root\nversion: 1.0.0\n");
    write(&root, "authored.cdo.yml", "type: cdo\npatches:\n  - target: /Game/Example.Example_C\n    properties:\n      - path: mValue\n        value: 2\n");
    write(
        &root,
        "nested/pack.yml",
        "ref: Nested\nname: Nested\nversion: 1.0.0\n",
    );
    write(&root, "nested/authored.cdo.yml", "type: cdo\npatches:\n  - target: /Game/Example.Example_C\n    properties:\n      - path: mValue\n        value: 2\n");
    for directory in [".cache", "Docs", "sCrIpTs", "nested/.backup"] {
        write(&root, &format!("{directory}/bad.yml"), "[invalid YAML");
        write(&root, &format!("{directory}/pack.yml"), "[invalid YAML");
    }
    let result = lint_workspace(
        &root,
        &[FileBuffer {
            path: root.join("Docs/unsaved.yml"),
            text: "[invalid YAML".into(),
        }],
        &LintOptions::default(),
    )
    .unwrap();
    assert_eq!(
        (result.packs, result.documents, result.errors),
        (2, 2, 0),
        "{:?}",
        result.diagnostics
    );
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == "root.ignored-directory"));
    let config = kdf_validation::config::scan_workspace(
        &root,
        &[FileBuffer {
            path: root.join("Docs/unsaved/pack.yml"),
            text: "ref: Hidden\nname: Hidden\nversion: 1.0.0\n".into(),
        }],
    );
    assert!(config.pack("Hidden").is_none());
}

#[test]
fn runtime_discovery_selected_and_standalone_ignored_files_explain_exclusion() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("DataForge");
    write(&root, "pack.yml", "ref: Root\nname: Root\nversion: 1.0.0\n");
    write(
        &root,
        "Docs/pack.yml",
        "ref: Example\nname: Example\nversion: 1.0.0\n",
    );
    write(&root, "Docs/example.cdo.yml", "type: cdo\npatches:\n  - target: /Game/Example.Example_C\n    properties:\n      - path: mValue\n        value: 2\n");
    let file = root.join("Docs/example.cdo.yml");
    let direct = lint_path(&file, &LintOptions::default()).unwrap();
    assert_eq!(direct.documents, 0);
    assert!(direct
        .diagnostics
        .iter()
        .any(|d| d.code == "root.ignored-directory"));
    let standalone =
        validate_document(&file, "type: cdo\npatches:\n  - target: /Game/Example.Example_C\n    properties:\n      - path: mValue\n        value: 2\n", &LintOptions::default()).unwrap();
    assert!(standalone
        .diagnostics
        .iter()
        .any(|d| d.code == "root.ignored-directory"));
}
