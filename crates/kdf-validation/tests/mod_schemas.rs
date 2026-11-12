use kdf_validation::{validate_document, LintOptions, SchemaRegistry};
use std::fs;
use std::path::{Path, PathBuf};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mod-schema")
}

#[test]
fn discovers_and_validates_mod_documents() {
    let root = fixture();
    let docs = root.join("DataForge/Example");
    let registry = SchemaRegistry::for_root(&LintOptions::default(), Some(&docs)).unwrap();
    assert!(registry.kinds().contains(&"sfpmodule".to_owned()));
    for (name, code) in [
        ("valid", None),
        ("missing-id", Some("schema.required")),
        ("unknown-field", Some("schema.additionalProperties")),
        ("invalid-id", Some("schema.pattern")),
    ] {
        let path = docs.join(format!("{name}.sfpmodule.yml"));
        let result = validate_document(
            &path,
            &fs::read_to_string(&path).unwrap(),
            &LintOptions::default(),
        )
        .unwrap();
        let schema_errors: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.code.starts_with("schema."))
            .collect();
        match code {
            Some(code) => assert!(
                schema_errors.iter().any(|d| d.code == code),
                "{name}: {schema_errors:?}"
            ),
            None => assert!(schema_errors.is_empty(), "{schema_errors:?}"),
        }
    }
}

#[test]
fn duplicate_and_builtin_shadow_are_diagnostics_and_first_wins() {
    let root = tempfile::tempdir().unwrap();
    let body = fs::read_to_string(fixture().join(
        "Mods/GameFeatures/SatisfactoryPlus/Resources/KDataForge/Schemas/sfpmodule.schema.yml",
    ))
    .unwrap();
    for (name, content) in [
        ("A", body.clone()),
        (
            "B",
            body.replace("example.invalid/satisfactoryplus", "example.invalid/other"),
        ),
        (
            "C",
            body.replace("sfpmodule", "recipe")
                .replace("example.invalid/satisfactoryplus", "example.invalid/shadow"),
        ),
    ] {
        let dir = root
            .path()
            .join(format!("Mods/{name}/Resources/KDataForge/Schemas"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.ancestors()
                .find(|p| p.file_name().is_some_and(|n| n == "Resources"))
                .unwrap()
                .parent()
                .unwrap()
                .join("Test.uplugin"),
            "{}",
        )
        .unwrap();
        fs::write(dir.join("custom.schema.yml"), content).unwrap();
    }
    let registry = SchemaRegistry::for_root(&LintOptions::default(), Some(root.path())).unwrap();
    let duplicate = registry
        .diagnostics()
        .iter()
        .find(|d| d.code == "schema.duplicate-type")
        .unwrap();
    assert!(
        duplicate.message.contains("Mods\\A") && duplicate.message.contains("Mods\\B")
            || duplicate.message.contains("Mods/A") && duplicate.message.contains("Mods/B")
    );
    assert!(registry
        .diagnostics()
        .iter()
        .any(|d| d.code == "schema.builtin-shadow"));
    assert_eq!(
        registry.schema("sfpmodule").unwrap()["$id"],
        "https://example.invalid/satisfactoryplus/sfpmodule.schema.yml"
    );
}

#[test]
fn explicit_directory_can_override_discovered_schema() {
    let dir = tempfile::tempdir().unwrap();
    let body = fs::read_to_string(fixture().join(
        "Mods/GameFeatures/SatisfactoryPlus/Resources/KDataForge/Schemas/sfpmodule.schema.yml",
    ))
    .unwrap();
    fs::write(
        dir.path().join("override.schema.yml"),
        body.replace("SatisfactoryPlus module", "Override"),
    )
    .unwrap();
    let options = LintOptions {
        schema_dirs: vec![dir.path().to_owned()],
        allow_schema_override: true,
        ..Default::default()
    };
    let registry = SchemaRegistry::for_root(&options, Some(&fixture())).unwrap();
    assert_eq!(registry.schema("sfpmodule").unwrap()["title"], "Override");
}

#[test]
fn missing_schema_remains_unknown() {
    let root = tempfile::tempdir().unwrap();
    let result = validate_document(
        &root.path().join("x.sfpmodule.yml"),
        "type: sfpmodule\n",
        &LintOptions::default(),
    )
    .unwrap();
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == "schema.unknown-type"));
}

#[test]
fn finds_each_supported_mod_container() {
    let root = tempfile::tempdir().unwrap();
    let source = fs::read_to_string(fixture().join(
        "Mods/GameFeatures/SatisfactoryPlus/Resources/KDataForge/Schemas/sfpmodule.schema.yml",
    ))
    .unwrap();
    for (container, kind) in [
        ("Mods/GameFeatures", "feature"),
        ("Mods", "direct"),
        ("FactoryGame/Mods", "installed"),
        ("tools", "toolmod"),
    ] {
        let dir = root
            .path()
            .join(container)
            .join(kind)
            .join("Resources/KDataForge/Schemas");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.ancestors()
                .find(|p| p.file_name().is_some_and(|n| n == "Resources"))
                .unwrap()
                .parent()
                .unwrap()
                .join("Test.uplugin"),
            "{}",
        )
        .unwrap();
        fs::write(
            dir.join("type.schema.yml"),
            source.replace("sfpmodule", kind),
        )
        .unwrap();
    }
    let registry = SchemaRegistry::for_root(&LintOptions::default(), Some(root.path())).unwrap();
    for kind in ["feature", "direct", "installed", "toolmod"] {
        assert!(registry.schema(kind).is_some(), "{kind}");
    }
}

#[test]
fn condition_provider_contract() {
    let root = fixture();
    let path = root.join("DataForge/Example/provider.cdo.yml");
    for (expression, expected) in [
        ("sfp:module.SFPAddonIndustrialFermentation == true", None),
        (
            "{sfp:module.example} == true",
            Some("config.expression-syntax"),
        ),
        (
            "other:module.example == true",
            Some("template.unknown-provider"),
        ),
        (
            "SFP:module.example == true",
            Some("template.unknown-provider"),
        ),
        ("sfp:bad/key == true", Some("config.expression-syntax")),
        ("sfp:key:extra == true", Some("config.expression-syntax")),
    ] {
        let source = format!("type: cdo\nconditions:\n  config: '{expression}'\npatches:\n  - target: /Game/Test.Test_C\n    properties:\n      mValue: 1\n");
        let result = validate_document(&path, &source, &LintOptions::default()).unwrap();
        let issues: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| {
                d.code == "config.expression-syntax" || d.code == "template.unknown-provider"
            })
            .collect();
        match expected {
            Some(code) => assert!(
                issues.iter().any(|d| d.code == code),
                "{expression}: {issues:?}"
            ),
            None => assert!(issues.is_empty(), "{expression}: {issues:?}"),
        }
        if expression.starts_with("SFP:") {
            assert!(issues.iter().any(|d| d.message.contains("sfp")));
        }
    }
}

#[test]
fn provider_collisions_report_both_owners_and_builtin() {
    let root = tempfile::tempdir().unwrap();
    for (name, prefix) in [("A", "sfp"), ("B", "sfp"), ("C", "cfg")] {
        let dir = root
            .path()
            .join("Mods")
            .join(name)
            .join("Resources/KDataForge");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.ancestors()
                .find(|p| p.file_name().is_some_and(|n| n == "Resources"))
                .unwrap()
                .parent()
                .unwrap()
                .join("Test.uplugin"),
            "{}",
        )
        .unwrap();
        fs::write(
            dir.join("providers.yml"),
            format!("providers:\n  - prefix: {prefix}\n    description: Example\n"),
        )
        .unwrap();
    }
    let registry = SchemaRegistry::for_root(&LintOptions::default(), Some(root.path())).unwrap();
    let duplicate = registry
        .diagnostics()
        .iter()
        .find(|d| d.code == "provider.duplicate-prefix")
        .unwrap();
    assert!(duplicate.message.contains("A") && duplicate.message.contains("B"));
    assert!(registry
        .diagnostics()
        .iter()
        .any(|d| d.code == "provider.builtin-shadow"));
    assert_eq!(registry.providers().len(), 1);
}

#[test]
fn nested_workspace_and_fixture_copies_do_not_duplicate_live_mod_contracts() {
    let root = tempfile::tempdir().unwrap();
    let body = fs::read_to_string(fixture().join(
        "Mods/GameFeatures/SatisfactoryPlus/Resources/KDataForge/Schemas/sfpmodule.schema.yml",
    ))
    .unwrap();
    for relative in [
        "Mods/GameFeatures/Live",
        "tools/tooling/tests/fixtures/copy/Mods/GameFeatures/Copy",
    ] {
        let owner = root.path().join(relative);
        let resources = owner.join("Resources/KDataForge");
        fs::create_dir_all(resources.join("Schemas")).unwrap();
        fs::write(owner.join("Example.uplugin"), "{}").unwrap();
        fs::write(resources.join("Schemas/example.schema.yml"), &body).unwrap();
        fs::write(
            resources.join("providers.yml"),
            "providers:\n  - prefix: example\n    description: Example\n",
        )
        .unwrap();
    }
    // Resource-shaped tooling directories without a descriptor are not mod owners.
    let fake = root.path().join("tools/fake/Resources/KDataForge");
    fs::create_dir_all(fake.join("Schemas")).unwrap();
    fs::write(fake.join("Schemas/example.schema.yml"), &body).unwrap();
    fs::write(
        fake.join("providers.yml"),
        "providers:\n  - prefix: example\n    description: Example\n",
    )
    .unwrap();
    for scope in [
        root.path().to_owned(),
        root.path().join("tools/tooling/tests/fixtures/copy"),
    ] {
        let registry = SchemaRegistry::for_root(&LintOptions::default(), Some(&scope)).unwrap();
        assert!(registry.schema("sfpmodule").is_some());
        assert_eq!(registry.providers().len(), 1);
        assert!(
            registry.diagnostics().is_empty(),
            "{:?}",
            registry.diagnostics()
        );
    }
}
