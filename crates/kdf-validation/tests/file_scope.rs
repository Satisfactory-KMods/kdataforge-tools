use kdf_validation::{lint_path, LintOptions};
use std::fs;

#[test]
fn file_lint_validates_only_selected_document_with_nearest_pack_context() {
    let temp = tempfile::tempdir().unwrap();
    let pack = temp.path().join("DataForge/example");
    fs::create_dir_all(pack.join("nested")).unwrap();
    fs::write(
        pack.join("pack.yml"),
        "ref: Example\nname: Example\nversion: 1.0.0\n",
    )
    .unwrap();
    let target = pack.join("nested/balance.cdo.yml");
    fs::write(&target, "type: cdo\npatches:\n  - target: /Game/Example.Example_C\n    properties:\n      - path: mValue\n        value: 2\n").unwrap();
    fs::write(pack.join("broken.cdo.yml"), "[broken YAML").unwrap();
    let result = lint_path(&target, &LintOptions::default()).unwrap();
    assert!(result.ok, "{:?}", result.diagnostics);
    assert_eq!(result.documents, 1);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.file.as_ref().is_none_or(|file| file == &target)));
    fs::write(&target, "type: cdo\npatches: wrong\n").unwrap();
    let invalid = lint_path(&target, &LintOptions::default()).unwrap();
    assert!(!invalid.ok);
    assert!(
        invalid
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.starts_with("schema")),
        "{:?}",
        invalid.diagnostics
    );
    assert!(!invalid
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "root.missing"));
}

#[test]
fn file_lint_keeps_manifest_scope_and_rejects_non_yaml() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("pack.yml");
    fs::write(&manifest, "ref: Example\nname: Example\nversion: 1.0.0\n").unwrap();
    fs::write(temp.path().join("broken.yml"), "[broken YAML").unwrap();
    let result = lint_path(&manifest, &LintOptions::default()).unwrap();
    assert!(result.ok, "{:?}", result.diagnostics);
    assert_eq!((result.packs, result.documents), (1, 0));
    let non_yaml = temp.path().join("readme.txt");
    fs::write(&non_yaml, "text").unwrap();
    assert!(lint_path(&non_yaml, &LintOptions::default()).is_err());
}

#[test]
fn file_lint_checks_selected_include_indices_without_linting_included_siblings() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("pack.yml"),
        "ref: Example\nname: Example\nversion: 1.0.0\ndependencies: [External]\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("items.item.yml"),
        "type: item\nitems: wrong\n---\ntype: item\nitems: []\n",
    )
    .unwrap();
    let selected = temp.path().join("recipes.recipe.yml");
    fs::write(
        &selected,
        "type: recipe\ninclude: ['items.item.yml[1]']\nrecipes:\n  - id: Recipe_Example\n",
    )
    .unwrap();
    let result = lint_path(&selected, &LintOptions::default()).unwrap();
    assert_eq!(result.documents, 1);
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "semantic.include-missing"),
        "{:?}",
        result.diagnostics
    );
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic
        .file
        .as_ref()
        .is_none_or(|file| file == &selected)));
}

#[test]
fn selected_file_loads_project_lint_preferences_and_relative_schema_paths() {
    let temp = tempfile::tempdir().unwrap();
    let project = dunce::canonicalize(temp.path()).unwrap();
    let directory = project.join("DataForge/example/nested");
    fs::create_dir_all(&directory).unwrap();
    let selected = directory.join("custom.yml");
    fs::write(&selected, "type: custom\n").unwrap();
    fs::write(
        project.join(".kdf-editor.toml"),
        "[lint]\nallow_unknown_types = true\nschema_dirs = ['schemas']\n",
    )
    .unwrap();
    let preferences = kdf_validation::LintSettings::load(&selected, None).unwrap();
    assert!(preferences.allow_unknown_types);
    assert_eq!(preferences.schema_dirs, vec![project.join("schemas")]);
}

#[test]
fn selected_pack_loads_actual_project_preferences_and_relative_custom_schema() {
    let temp = tempfile::tempdir().unwrap();
    let project = dunce::canonicalize(temp.path()).unwrap().join("Project");
    let selected = project.join("DataForge/category/selected");
    fs::create_dir_all(&selected).unwrap();
    fs::create_dir_all(project.join("schemas")).unwrap();
    fs::write(
        selected.join("pack.yml"),
        "ref: Example\nname: Example\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        selected.join("value.custom.yml"),
        "type: custom\nvalue: 4\n",
    )
    .unwrap();
    fs::write(project.join("schemas/custom.schema.yml"), "$id: kdf://schema/custom\nx-kdf-type: custom\ntype: object\nrequired: [type, value]\nproperties:\n  type: {const: custom}\n  value: {type: integer}\nadditionalProperties: false\n").unwrap();
    fs::write(
        project.join(".kdf-editor.toml"),
        "[lint]\nschema_dirs = ['schemas']\nwarnings_as_errors = true\n",
    )
    .unwrap();
    // An unrelated outer scope must not replace the actual project's policy.
    fs::write(
        temp.path().join(".kdf-editor.toml"),
        "[lint]\nschema_dirs = ['wrong']\n",
    )
    .unwrap();
    let preferences = kdf_validation::LintSettings::load(&selected, None).unwrap();
    assert_eq!(preferences.schema_dirs, vec![project.join("schemas")]);
    assert!(preferences.warnings_as_errors);
    let result = lint_path(
        &selected,
        &preferences.lint_options(kdf_validation::Profile::Runtime),
    )
    .unwrap();
    assert!(result.ok, "{:?}", result.diagnostics);
    assert_eq!(result.documents, 1);
}

#[test]
fn explicit_project_directory_without_settings_does_not_inherit_unrelated_parent_project() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join(".kdf-editor.toml"),
        "[lint]\nallow_unknown_types = true\nschema_dirs = ['outer']\n",
    )
    .unwrap();
    let project = temp.path().join("IndependentProject");
    fs::create_dir_all(project.join("DataForge")).unwrap();
    let selected = project.join("DataForge/selected");
    fs::create_dir_all(&selected).unwrap();
    fs::write(selected.join("pack.yml"), "ref: Independent\n").unwrap();
    for scope in [&project, &project.join("DataForge"), &selected] {
        let preferences = kdf_validation::LintSettings::load(scope, None).unwrap();
        assert!(!preferences.allow_unknown_types);
        assert!(preferences.schema_dirs.is_empty());
    }
    let standalone = temp.path().join("StandalonePack");
    fs::create_dir(&standalone).unwrap();
    fs::write(standalone.join("pack.yml"), "ref: Standalone\n").unwrap();
    let preferences = kdf_validation::LintSettings::load(&standalone, None).unwrap();
    assert!(preferences.schema_dirs.is_empty());
    fs::write(
        standalone.join(".kdf-editor.toml"),
        "[lint]\nschema_dirs = ['local']\n",
    )
    .unwrap();
    let preferences = kdf_validation::LintSettings::load(&standalone, None).unwrap();
    assert_eq!(
        preferences.schema_dirs,
        vec![dunce::canonicalize(&standalone).unwrap().join("local")]
    );
}
