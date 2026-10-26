//! Pack configuration: definitions, references, conditions, templates and manifest rules,
//! checked on the fixture workspace and on small synthetic workspaces.
mod support;

use kdf_validation::config::{scan_workspace, Apply, FieldKind, Resolution, ValueType};
use kdf_validation::{
    lint_path, lint_workspace, validate_document, Diagnostic, FileBuffer, LintOptions, LintResult,
    Severity,
};
use std::path::{Path, PathBuf};
use support::{find, triples, write};
use tempfile::tempdir;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/config/DataForge")
}

fn lint(root: &Path) -> LintResult {
    lint_path(root, &LintOptions::default()).unwrap()
}

fn codes(result: &LintResult) -> Vec<String> {
    let mut codes = result
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect::<Vec<_>>();
    codes.sort();
    codes
}

fn single(name: &str, text: &str) -> LintResult {
    validate_document(Path::new(name), text, &LintOptions::default()).unwrap()
}

#[test]
fn keyed_pack_name_is_validated_and_exposed_as_native_source() {
    let temp = tempdir().unwrap();
    let text = "ref: Example\nname: {ns: Packs, key: Example.Name, source: Localized pack}\nversion: 1.0.0\ncontributer: Kyri123\n";
    write(temp.path(), "pack.yml", text);
    let result = lint(temp.path());
    assert!(result.ok, "{:?}", result.diagnostics);
    let index = scan_workspace(temp.path(), &[]);
    assert_eq!(
        index.pack("Example").unwrap().name.as_deref(),
        Some("Localized pack")
    );
    let invalid = single("pack.yml", &text.replace("key: Example.Name", "key: ''"));
    assert!(!invalid.ok);
}

fn one<'a>(result: &'a LintResult, code: &str) -> &'a Diagnostic {
    let found = find(result, code);
    assert_eq!(found.len(), 1, "{code}: {:?}", result.diagnostics);
    found[0]
}

fn triple(file: &str, code: &str, path: &str) -> (String, String, String) {
    (file.to_owned(), code.to_owned(), path.to_owned())
}

// ---------------------------------------------------------------------------------------------
// Fixture workspace
// ---------------------------------------------------------------------------------------------

#[test]
fn fixture_workspace_reports_exactly_the_planned_diagnostics() {
    let root = fixture_root();
    let result = lint(&root);
    assert_eq!((result.packs, result.documents), (2, 8));
    let broken = "ConfigDemo/broken.config.yml";
    let errors = "ConfigDemo/errors.cdo.yml";
    let mut expected = vec![
        triple(broken, "config.duplicate-id", "$.fields[0].id"),
        triple(broken, "config.prefix-clash", "$.fields[1].id"),
        triple(broken, "config.id-reserved", "$.fields[2].id"),
        triple(broken, "config.default-invalid", "$.fields[3].default"),
        triple(broken, "config.range-invalid", "$.fields[3].min"),
        triple(broken, "config.range-invalid", "$.fields[4].max"),
        triple(broken, "config.default-invalid", "$.fields[5].default"),
        triple(broken, "config.default-invalid", "$.fields[6].default"),
        triple(
            broken,
            "config.option-invalid",
            "$.fields[6].options[1].value",
        ),
        triple(broken, "config.category-missing", "$.fields[7]"),
        triple(
            broken,
            "config.reserved-key",
            "$.fields[8].descriptionClass",
        ),
        triple(
            errors,
            "config.unknown-id",
            "$.patches[0].properties[0].value",
        ),
        triple(
            errors,
            "config.unknown-pack",
            "$.patches[0].properties[1].value",
        ),
        triple(
            errors,
            "config.case-mismatch",
            "$.patches[0].properties[2].value",
        ),
        triple(
            errors,
            "template.unbalanced",
            "$.patches[0].properties[3].value",
        ),
        triple(
            errors,
            "template.unquoted-start",
            "$.patches[0].properties[4].value",
        ),
        triple(
            errors,
            "template.unknown-provider",
            "$.patches[0].properties[5].value",
        ),
        triple(
            errors,
            "config.type-mismatch",
            "$.patches[0].properties[6].index",
        ),
        triple(
            errors,
            "config.type-mismatch",
            "$.patches[0].properties[7].value",
        ),
        triple(
            errors,
            "config.type-mismatch",
            "$.patches[0].properties[8].min",
        ),
        triple(
            errors,
            "template.malformed",
            "$.patches[0].properties[9].value",
        ),
        triple(
            errors,
            "config.expression-type",
            "$.patches[0].properties[10].conditions.config",
        ),
        triple(
            errors,
            "config.unknown-id",
            "$.patches[0].properties[10].conditions.config",
        ),
    ];
    expected.sort();
    assert_eq!(triples(&result, &root), expected);
}

#[test]
fn clean_fixture_documents_have_no_diagnostics() {
    let result = lint(&fixture_root());
    for clean in [
        "ConfigDemo/pack.yml",
        "ConfigDemo/balance.config.yml",
        "ConfigDemo/categories.class.yml",
        "ConfigDemo/tuning.cdo.yml",
        "ConfigDemo/texts.localization.yml",
        "ConfigDemo/gated.recipe.yml",
        "Other/pack.yml",
        "Other/shared.config.yml",
    ] {
        assert!(
            !triples(&result, &fixture_root())
                .iter()
                .any(|(file, _, _)| file == clean),
            "{clean} should be clean: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn diagnostic_ranges_point_inside_the_expression() {
    let result = lint(&fixture_root());
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.code == "config.unknown-id" && d.yaml_path.ends_with("conditions.config"))
        .unwrap();
    // `          config: balance.speed contains 1 && missing.id` -> `missing.id` starts at column 47.
    assert_eq!(unknown.range.start.line, 32);
    assert_eq!(unknown.range.start.character, 46);
    assert_eq!(unknown.range.end.character, 56);
    let provider = result
        .diagnostics
        .iter()
        .find(|d| d.code == "template.unknown-provider")
        .unwrap();
    // `        value: "{cfgx:balance.speed}"`: the range covers `cfgx` only.
    assert_eq!(
        provider.range.end.character - provider.range.start.character,
        4
    );
}

#[test]
fn config_index_exposes_fields_resolution_and_metadata() {
    let index = scan_workspace(&fixture_root(), &[]);
    assert_eq!(index.packs().len(), 2);
    let demo = index.pack("ConfigDemo").expect("pack");
    assert!(demo.enabled);
    assert_eq!(demo.name.as_deref(), Some("Config Demo"));
    assert_eq!(demo.files.len(), 2, "balance and broken config files");
    // Skipped fields (duplicate, clash, reserved, invalid) are not indexed.
    let ids = demo
        .fields
        .iter()
        .map(|f| f.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "balance.enabled",
            "balance.speed",
            "balance.stack",
            "balance.mode",
            "balance.tiers",
            "balance.title",
            "balance.tag",
            "balance.names",
            "broken.reserved",
        ]
    );
    let speed = demo.field("balance.speed").unwrap();
    assert_eq!(speed.kind, FieldKind::Number);
    assert_eq!(speed.value_type, ValueType::Float);
    assert_eq!(speed.apply, Apply::Hot);
    assert_eq!(speed.min, Some(0.5));
    assert_eq!(speed.max, Some(4.0));
    assert_eq!(speed.tooltip_title.as_deref(), Some("Machine speed"));
    assert_eq!(speed.category.as_deref(), Some("DemoCategory"));
    assert_eq!(speed.key(), "ConfigDemo:balance.speed");
    let mode = demo.field("balance.mode").unwrap();
    assert_eq!(mode.options.len(), 2);
    assert_eq!(mode.options[1].label, "Hard");
    let tiers = demo.field("balance.tiers").unwrap();
    assert!(tiers.multiple);
    assert_eq!(tiers.value_type, ValueType::Int32);
    assert_eq!(
        demo.field("balance.title").unwrap().max_length,
        Some(40),
        "explicit maxLength"
    );
    assert!(speed.order < demo.field("balance.stack").unwrap().order);
    assert_eq!(speed.id_range.start.line, 8);
    // Resolution.
    assert!(matches!(
        index.resolve(Some("ConfigDemo"), None, "balance.speed"),
        Resolution::Found(_)
    ));
    assert!(matches!(
        index.resolve(Some("ConfigDemo"), None, "Balance.Speed"),
        Resolution::CaseMismatch(_)
    ));
    assert!(matches!(
        index.resolve(Some("ConfigDemo"), Some("Other"), "shared.level"),
        Resolution::Found(_)
    ));
    assert!(matches!(
        index.resolve(Some("ConfigDemo"), Some("Missing"), "x"),
        Resolution::UnknownPack
    ));
    assert!(matches!(
        index.resolve(Some("ConfigDemo"), None, "balance.nope"),
        Resolution::UnknownId
    ));
    assert!(index
        .generated_class("ConfigDemo", "democategory")
        .is_some());
    let file = demo.files[0].clone();
    assert_eq!(
        index.pack_for_path(&file).map(|p| p.pack_ref.as_str()),
        Some("ConfigDemo")
    );
}

#[test]
fn scan_workspace_prefers_unsaved_buffers() {
    let root = fixture_root();
    let path = root.join("Other/shared.config.yml");
    let buffer = FileBuffer {
        path: path.clone(),
        text: "category: /Script/FactoryGame.FGCategory\nfields:\n  - {id: shared.extra, type: boolean, name: X, description: Y, default: true}\n".into(),
    };
    let index = scan_workspace(&root, &[buffer]);
    let other = index.pack("Other").unwrap();
    assert_eq!(other.fields.len(), 1);
    assert_eq!(other.fields[0].id, "shared.extra");
}

// ---------------------------------------------------------------------------------------------
// Definition rules (single document)
// ---------------------------------------------------------------------------------------------

fn definition(field: &str) -> LintResult {
    single(
        "demo.config.yml",
        &format!("category: /Script/FactoryGame.FGCategory\nfields:\n  - {field}\n"),
    )
}

#[test]
fn definition_defaults_are_checked_against_type_width_range_and_step() {
    let cases = [
        // (field yaml, expected code or None)
        ("{id: a, type: number, name: N, description: D, default: 5, min: 0, max: 10, step: 5}", None),
        ("{id: a, type: number, name: N, description: D, default: 3, min: 0, max: 10, step: 5}", Some("config.default-invalid")),
        ("{id: a, type: number, name: N, description: D, default: 11, min: 0, max: 10}", Some("config.default-invalid")),
        ("{id: a, type: number, valueType: int8, name: N, description: D, default: 200}", Some("config.default-invalid")),
        ("{id: a, type: number, valueType: uint8, name: N, description: D, default: -1}", Some("config.default-invalid")),
        ("{id: a, type: number, valueType: float, name: N, description: D, default: 0.25, min: 0, max: 1, step: 0.25}", None),
        ("{id: a, type: number, name: N, description: D, default: [1, 2]}", Some("config.default-invalid")),
        ("{id: a, type: number, multiple: true, name: N, description: D, default: [1, 2, 3], maxCount: 2}", Some("config.default-invalid")),
        ("{id: a, type: number, multiple: true, name: N, description: D, default: [1], minCount: 2}", Some("config.default-invalid")),
        ("{id: a, type: number, multiple: true, name: N, description: D, default: [1], minCount: 3, maxCount: 2}", Some("config.range-invalid")),
        ("{id: a, type: number, multiple: true, display: slider, min: 0, max: 2, name: N, description: D, default: [1]}", Some("config.range-invalid")),
        ("{id: a, type: number, display: slider, min: 2, max: 2, name: N, description: D, default: 2}", Some("config.range-invalid")),
        ("{id: a, type: text, valueType: tag, name: N, description: D, default: 'Bad Tag'}", Some("config.default-invalid")),
        ("{id: a, type: text, valueType: tag, name: N, description: D, default: 'A.B'}", None),
        ("{id: a, type: text, name: N, description: D, default: abcd, maxLength: 3}", Some("config.default-invalid")),
        ("{id: a, type: select, name: N, description: D, default: x, options: [{value: x, label: X}]}", None),
        ("{id: a, type: select, valueType: int8, name: N, description: D, default: 1, options: [{value: 1, label: X}, {value: 300, label: Y}]}", Some("config.option-invalid")),
        ("{id: a, type: select, valueType: bool, name: N, description: D, default: true, options: [{value: true, label: X}]}", None),
        ("{id: a, type: select, name: N, description: D, default: x, options: [{value: x, label: X}], minSelected: 1}", Some("config.range-invalid")),
        ("{id: a, type: select, multiple: true, name: N, description: D, default: [x], options: [{value: x, label: X}], maxSelected: 2}", Some("config.range-invalid")),
        ("{id: a, type: select, multiple: true, name: N, description: D, default: [x, y], options: [{value: x, label: X}, {value: y, label: Y}], maxSelected: 1}", Some("config.default-invalid")),
    ];
    for (field, expected) in cases {
        let result = definition(field);
        let non_schema = result
            .diagnostics
            .iter()
            .filter(|d| d.source != "schema")
            .map(|d| d.code.as_str())
            .collect::<Vec<_>>();
        match expected {
            Some(code) => assert!(
                non_schema.contains(&code),
                "{field}: {:?}",
                result.diagnostics
            ),
            None => assert!(non_schema.is_empty(), "{field}: {:?}", result.diagnostics),
        }
    }
}

/// Duplicate options are found by value (`1` and `1.0` are the same number) with a set lookup, so a
/// long option list stays cheap.
#[test]
fn select_options_detect_duplicates_by_value() {
    let numbers = definition(
        "{id: a, type: select, valueType: float, name: N, description: D, default: 1, options: [{value: 1, label: X}, {value: 1.0, label: Y}]}",
    );
    let twice = find(&numbers, "config.option-invalid");
    assert_eq!(twice.len(), 1, "{:?}", numbers.diagnostics);
    assert!(twice[0].message.contains("used twice"));
    let words = definition(
        "{id: a, type: select, name: N, description: D, default: x, options: [{value: x, label: X}, {value: y, label: Y}, {value: x, label: Z}]}",
    );
    assert_eq!(find(&words, "config.option-invalid").len(), 1);
    let options = (0..6000)
        .map(|n| format!("{{value: v{n}, label: L{n}}}"))
        .collect::<Vec<_>>()
        .join(", ");
    let long = definition(&format!(
        "{{id: a, type: select, name: N, description: D, default: v5999, options: [{options}]}}"
    ));
    assert!(
        find(&long, "config.option-invalid").is_empty()
            && find(&long, "config.default-invalid").is_empty(),
        "{:?}",
        long.diagnostics.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
    let missing = definition(&format!(
        "{{id: a, type: select, name: N, description: D, default: nope, options: [{options}]}}"
    ));
    assert_eq!(find(&missing, "config.default-invalid").len(), 1);
}

#[test]
fn definition_reserved_properties_warn_but_do_not_block() {
    let result = definition(
        "{id: a, type: boolean, name: N, description: D, default: true, properties: [{path: x}], descriptionClass: Foo}",
    );
    assert_eq!(find(&result, "config.reserved-key").len(), 2);
    assert!(result.ok, "{:?}", result.diagnostics);
}

#[test]
fn definition_texts_accept_keyed_form_and_require_name_and_description() {
    let keyed = definition(
        "{id: a, type: boolean, name: {key: K, ns: N, source: Name}, description: {key: D}, default: true}",
    );
    assert!(keyed.ok, "{:?}", keyed.diagnostics);
    let missing = definition("{id: a, type: boolean, default: true}");
    assert!(missing
        .diagnostics
        .iter()
        .any(|d| d.code == "schema.required"));
}

#[test]
fn definition_schema_rejects_keys_of_other_field_types() {
    for (field, bad_key) in [
        ("{id: a, type: boolean, name: N, description: D, default: true, min: 1}", "min"),
        ("{id: a, type: text, name: N, description: D, default: x, options: []}", "options"),
        ("{id: a, type: select, name: N, description: D, default: x, options: [{value: x, label: X}], step: 1}", "step"),
        ("{id: a, type: number, name: N, description: D, default: 1, maxLength: 3}", "maxLength"),
        ("{id: a, type: boolean, name: N, description: D, default: true, multiple: true}", "multiple"),
    ] {
        let result = definition(field);
        assert!(
            result.diagnostics.iter().any(|d| {
                d.code == "schema.unevaluatedProperties" && d.yaml_path.ends_with(bad_key)
            }),
            "{field}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn definition_rules_for_ids() {
    let text = "category: /Script/FactoryGame.FGCategory\nfields:\n  - {id: a.b, type: boolean, name: N, description: D, default: true}\n  - {id: a.b.c, type: boolean, name: N, description: D, default: true}\n  - {id: x.y.z, type: boolean, name: N, description: D, default: true}\n  - {id: x.y, type: boolean, name: N, description: D, default: true}\n  - {id: in, type: boolean, name: N, description: D, default: true}\n  - {id: bad-id, type: boolean, name: N, description: D, default: true}\n  - {id: A.B, type: boolean, name: N, description: D, default: true}\n";
    let result = single("ids.config.yml", text);
    let pairs = result
        .diagnostics
        .iter()
        .map(|d| (d.code.as_str(), d.yaml_path.as_str()))
        .collect::<Vec<_>>();
    assert!(
        pairs.contains(&("config.prefix-clash", "$.fields[1].id")),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&("config.prefix-clash", "$.fields[3].id")),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&("config.id-reserved", "$.fields[4].id")),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&("schema.pattern", "$.fields[5].id")),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&("config.duplicate-id", "$.fields[6].id")),
        "{pairs:?}"
    );
}

#[test]
fn definition_category_rules_without_reflection() {
    let none = single(
        "demo.config.yml",
        "fields:\n  - {id: a, type: boolean, name: N, description: D, default: true}\n",
    );
    assert_eq!(codes(&none), ["config.category-missing"]);
    // A bare id cannot be judged from a single document (the class may live in another file).
    let bare = single(
        "demo.config.yml",
        "category: SomeCategory\nfields:\n  - {id: a, type: boolean, name: N, description: D, default: true}\n",
    );
    assert!(
        bare.ok && bare.diagnostics.is_empty(),
        "{:?}",
        bare.diagnostics
    );
    let field = single(
        "demo.config.yml",
        "category: DocCategory\nfields:\n  - {id: a, type: boolean, name: N, description: D, default: true, category: /KDataForge/Gen/P/Own}\n",
    );
    assert!(field.diagnostics.is_empty(), "{:?}", field.diagnostics);
}

#[test]
fn config_conditions_are_rejected_inside_definitions() {
    let result = single(
        "demo.config.yml",
        "category: /Script/FactoryGame.FGCategory\nconditions:\n  - hasMod: SML\n  - config: other\nfields:\n  - {id: a, type: boolean, name: N, description: D, default: true}\n",
    );
    let diagnostic = one(&result, "config.condition-in-config");
    assert_eq!(diagnostic.yaml_path, "$.conditions[1].config");
}

// ---------------------------------------------------------------------------------------------
// File naming
// ---------------------------------------------------------------------------------------------

#[test]
fn config_document_type_and_file_name_must_agree() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(root, "pack.yml", "ref: P\nname: P\nversion: 1.0.0\n");
    write(
        root,
        "wrong.cdo.yml",
        "type: config\nfields:\n  - {id: a, type: boolean, name: N, description: D, default: true, category: /Script/FactoryGame.FGCategory}\n",
    );
    write(
        root,
        "other.config.yml",
        "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: m\n        value: 1\n",
    );
    write(
        root,
        "ok.config.yaml",
        "fields:\n  - {id: ok, type: boolean, name: N, description: D, default: true, category: /Script/FactoryGame.FGCategory}\n",
    );
    let result = lint(root);
    let pairs = triples(&result, root);
    assert!(
        pairs.contains(&triple("wrong.cdo.yml", "config.filename", "$.type")),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&triple(
            "other.config.yml",
            "config.filename-type",
            "$.type"
        )),
        "{pairs:?}"
    );
    assert_eq!(pairs.len(), 2, "{pairs:?}");
    // The misnamed config document does not define fields.
    let index = scan_workspace(root, &[]);
    assert_eq!(index.pack("P").unwrap().fields.len(), 1);
}

#[test]
fn config_documents_never_join_generated_identity_namespaces() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(root, "pack.yml", "ref: P\nname: P\nversion: 1.0.0\n");
    write(
        root,
        "x.config.yml",
        "category: /Script/FactoryGame.FGCategory\nfields:\n  - {id: Shared, type: boolean, name: N, description: D, default: true}\n",
    );
    write(
        root,
        "x.item.yml",
        "type: item\nitems:\n  - id: Shared\n    parent: /Script/FactoryGame.FGItemDescriptor\n",
    );
    let result = lint(root);
    assert!(
        result.ok && result.diagnostics.is_empty(),
        "{:?}",
        result.diagnostics
    );
}

// ---------------------------------------------------------------------------------------------
// Pack manifest
// ---------------------------------------------------------------------------------------------

#[test]
fn pack_icon_rules() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let manifest = |icon: &str| format!("ref: P\nname: P\nversion: 1.0.0\nicon: {icon}\n");
    write(root, "art/logo.png", "png");
    write(root, "art/logo.gif", "gif");
    for (icon, expected) in [
        ("art/logo.png", None),
        ("art\\logo.png", None),
        ("/Game/UI/Logo.Logo", None),
        ("../logo.png", Some("pack.icon-escape")),
        ("/C:/logo.png", Some("pack.icon-path")),
        ("C:/logo.png", Some("pack.icon-escape")),
        ("art/logo.gif", Some("pack.icon-format")),
        ("art/missing.png", Some("pack.icon-missing")),
        ("\"/Game/Bad Name\"", Some("pack.icon-path")),
    ] {
        write(
            root,
            "pack.yml",
            &manifest(&format!("'{}'", icon.trim_matches('"'))),
        );
        let result = lint(root);
        let pack_codes = codes(&result);
        match expected {
            Some(code) => assert_eq!(pack_codes, [code], "{icon}: {:?}", result.diagnostics),
            None => assert!(pack_codes.is_empty(), "{icon}: {:?}", result.diagnostics),
        }
    }
    // Single-document validation has no folder: only syntax rules apply.
    let single_doc = single("pack.yml", &manifest("art/missing.png"));
    assert!(
        single_doc.diagnostics.is_empty(),
        "{:?}",
        single_doc.diagnostics
    );
    let escape = single("pack.yml", &manifest("../x.png"));
    assert_eq!(codes(&escape), ["pack.icon-escape"]);
}

#[cfg(unix)]
#[test]
fn pack_icon_symlink_outside_pack_is_rejected() {
    use std::fs;
    let dir = tempdir().unwrap();
    let root = dir.path();
    let pack = root.join("P");
    fs::create_dir_all(&pack).unwrap();
    fs::write(root.join("outside.png"), "png").unwrap();
    std::os::unix::fs::symlink(root.join("outside.png"), pack.join("logo.png")).unwrap();
    fs::write(
        pack.join("pack.yml"),
        "ref: P\nname: P\nversion: 1.0.0\nicon: logo.png\n",
    )
    .unwrap();
    let result = lint(root);
    assert!(
        codes(&result).contains(&"pack.icon-symlink".to_owned()),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn pack_description_accepts_keyed_text() {
    let ok = single(
        "pack.yml",
        "ref: P\nname: P\nversion: 1.0.0\ndescription:\n  key: D\n  ns: P\n  source: Text\n",
    );
    assert!(ok.diagnostics.is_empty(), "{:?}", ok.diagnostics);
    let bad = single(
        "pack.yml",
        "ref: P\nname: P\nversion: 1.0.0\ndescription: {text: x}\n",
    );
    assert!(!bad.ok);
}

#[test]
fn pack_conditions_check_config_expressions() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "pack.yml",
        "ref: P\nname: P\nversion: 1.0.0\nconditions:\n  - config: [flag, count > 3]\n  - config: missing && flag == 1\n",
    );
    write(
        root,
        "cfg.config.yml",
        "category: /Script/FactoryGame.FGCategory\nfields:\n  - {id: flag, type: boolean, name: F, description: D, default: true}\n  - {id: count, type: number, name: C, description: D, default: 5, apply: hot}\n",
    );
    let result = lint(root);
    let pairs = triples(&result, root);
    assert!(
        pairs.contains(&triple(
            "pack.yml",
            "config.hot-in-condition",
            "$.conditions[0].config[1]"
        )),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&triple(
            "pack.yml",
            "config.unknown-id",
            "$.conditions[1].config"
        )),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&triple(
            "pack.yml",
            "config.expression-type",
            "$.conditions[1].config"
        )),
        "{pairs:?}"
    );
    assert_eq!(pairs.len(), 3, "{pairs:?}");
}

// ---------------------------------------------------------------------------------------------
// Typed references
// ---------------------------------------------------------------------------------------------

fn pack_with_fields(root: &Path) {
    write(root, "pack.yml", "ref: P\nname: P\nversion: 1.0.0\n");
    write(
        root,
        "cfg.config.yml",
        "category: /Script/FactoryGame.FGCategory\nfields:\n  - {id: hotnum, type: number, name: H, description: D, default: 5, min: 1, max: 9, apply: hot}\n  - {id: coldnum, type: number, name: C, description: D, default: 5}\n  - {id: title, type: text, name: T, description: D, default: x}\n  - {id: ratio, type: number, valueType: float, name: R, description: D, default: 0.5}\n  - {id: mode, type: select, name: M, description: D, default: a, options: [{value: a, label: A}]}\n",
    );
}

#[test]
fn typed_references_are_resolved_only_when_unquoted_and_plain() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    pack_with_fields(root);
    write(
        root,
        "x.sinkpoints.yml",
        "type: sinkpoints\nentries:\n  - item: A\n    points: cfg:coldnum\n  - item: B\n    points: 'cfg:missing'\n  - item: C\n    points: \"cfg:missing\"\n  - item: D\n    points: cfg:title\n  - item: E\n    points: cfg:ratio\n  - item: F\n    points: cfg:missing\n",
    );
    let result = lint(root);
    let pairs = triples(&result, root);
    // Quoted references stay literal strings: the schema keeps its type error for them.
    assert!(
        pairs.contains(&triple(
            "x.sinkpoints.yml",
            "schema.type",
            "$.entries[1].points"
        )),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&triple(
            "x.sinkpoints.yml",
            "schema.type",
            "$.entries[2].points"
        )),
        "{pairs:?}"
    );
    // Plain references: the schema `type` error becomes a compatibility check.
    assert!(!pairs.iter().any(|p| p.0 == "x.sinkpoints.yml"
        && p.1 == "schema.type"
        && p.2 == "$.entries[0].points"));
    assert!(
        pairs.contains(&triple(
            "x.sinkpoints.yml",
            "config.type-mismatch",
            "$.entries[3].points"
        )),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&triple(
            "x.sinkpoints.yml",
            "config.type-mismatch",
            "$.entries[4].points"
        )),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&triple(
            "x.sinkpoints.yml",
            "config.unknown-id",
            "$.entries[5].points"
        )),
        "{pairs:?}"
    );
    let float_into_int = result
        .diagnostics
        .iter()
        .find(|d| d.yaml_path == "$.entries[4].points")
        .unwrap();
    assert_eq!(
        float_into_int.severity,
        Severity::Warning,
        "lossy conversions only warn"
    );
    let text_into_int = result
        .diagnostics
        .iter()
        .find(|d| d.yaml_path == "$.entries[3].points")
        .unwrap();
    assert_eq!(text_into_int.severity, Severity::Error);
}

#[test]
fn block_scalars_are_literal_but_flow_items_are_references() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    pack_with_fields(root);
    write(
        root,
        "x.cdo.yml",
        "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mA\n        value: |-\n          cfg:missing\n      - path: mB\n        value: [cfg:missing, 'cfg:missing']\n",
    );
    let result = lint(root);
    let pairs = triples(&result, root);
    assert_eq!(
        pairs,
        [triple(
            "x.cdo.yml",
            "config.unknown-id",
            "$.patches[0].properties[1].value[0]"
        )]
    );
}

#[test]
fn references_in_unresolved_positions_warn() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    pack_with_fields(root);
    write(
        root,
        "x.item.yml",
        "type: item\nitems:\n  - id: cfg:coldnum\n    parent: /Script/FactoryGame.FGItemDescriptor\n    properties:\n      - path: mA\n        value:\n          cfg:coldnum: 1\n",
    );
    write(
        root,
        "y.cdo.yml",
        "type: cdo\ninclude: cfg:coldnum\npatches:\n  - target: /Game/A.A_C\n    conditions:\n      hasMod: cfg:coldnum\n    properties:\n      - path: mA\n        value: 1\n",
    );
    let result = lint(root);
    let unresolved = find(&result, "config.ref-not-resolved");
    let paths = unresolved
        .iter()
        .map(|d| d.yaml_path.as_str())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"$.items[0].id"), "{paths:?}");
    assert!(
        paths.contains(&"$.items[0].properties[0].value.cfg:coldnum"),
        "{paths:?}"
    );
    assert!(paths.contains(&"$.include"), "{paths:?}");
    assert!(
        paths.contains(&"$.patches[0].conditions.hasMod"),
        "{paths:?}"
    );
}

#[test]
fn hot_values_in_cold_places_warn() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    pack_with_fields(root);
    write(
        root,
        "tags.gametag.yml",
        "type: gametag\ntags:\n  - \"X.{cfg:hotnum}\"\n  - \"X.{cfg:coldnum}\"\n",
    );
    write(
        root,
        "r.recipe.yml",
        "type: recipe\nrecipes:\n  - id: R\n    parent: /Script/FactoryGame.FGRecipe\n    properties:\n      - path: mManufactoringDuration\n        value: cfg:hotnum\n",
    );
    write(
        root,
        "n.sinkpoints.yml",
        "type: sinkpoints\nentries:\n  - item: Desc_A_C\n    points: cfg:hotnum\n",
    );
    write(
        root,
        "g.cdo.yml",
        "type: cdo\nconditions:\n  config: hotnum > 1\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mA\n        value: cfg:hotnum\n        conditions:\n          config: hotnum > 2\n",
    );
    let result = lint(root);
    let pairs = triples(&result, root);
    assert!(
        pairs.contains(&triple(
            "tags.gametag.yml",
            "config.hot-in-cold-stage",
            "$.tags[0]"
        )),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&triple(
            "n.sinkpoints.yml",
            "config.hot-in-cold-stage",
            "$.entries[0].points"
        )),
        "{pairs:?}"
    );
    // properties: subtrees of content kinds and whole cdo documents are hot-capable.
    assert!(
        !pairs
            .iter()
            .any(|p| p.0 == "r.recipe.yml" || p.0 == "g.cdo.yml"),
        "{pairs:?}"
    );
    assert_eq!(
        pairs
            .iter()
            .filter(|p| p.1 == "config.hot-in-cold-stage")
            .count(),
        2,
        "{pairs:?}"
    );
}

#[test]
fn disabled_and_foreign_packs_are_reported() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(root, "A/pack.yml", "ref: A\nname: A\nversion: 1.0.0\n");
    write(
        root,
        "B/pack.yml",
        "ref: B\nname: B\nversion: 1.0.0\nenabled: false\n",
    );
    write(
        root,
        "B/b.config.yml",
        "category: /Script/FactoryGame.FGCategory\nfields:\n  - {id: v, type: number, name: V, description: D, default: 1}\n",
    );
    write(
        root,
        "B/own.cdo.yml",
        "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mA\n        value: cfg:v\n",
    );
    write(
        root,
        "A/use.cdo.yml",
        "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mA\n        value: cfg:B:v\n      - path: mB\n        value: cfg:B:nope\n",
    );
    let result = lint(root);
    // Both references name the disabled pack; the disabled pack's own documents stay quiet.
    assert_eq!(
        triples(&result, root),
        [
            triple(
                "A/use.cdo.yml",
                "config.pack-disabled",
                "$.patches[0].properties[0].value"
            ),
            triple(
                "A/use.cdo.yml",
                "config.pack-disabled",
                "$.patches[0].properties[1].value"
            ),
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// Conditions
// ---------------------------------------------------------------------------------------------

#[test]
fn expression_syntax_errors_point_into_the_scalar() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    pack_with_fields(root);
    write(
        root,
        "x.cdo.yml",
        "type: cdo\nconditions:\n  - config: \"hotnum >= \"\n  - config: ['coldnum ==', '(coldnum']\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mA\n        value: 1\n",
    );
    let result = lint(root);
    let syntax = find(&result, "config.expression-syntax");
    assert_eq!(syntax.len(), 3, "{:?}", result.diagnostics);
    assert!(syntax[0].message.starts_with("expected-operand"));
    // `  - config: "hotnum >= "`: the error is at offset 10 (end of the string). A zero-width range
    // is invisible, so it is anchored on the last character of the string (offset 9..10).
    assert_eq!(syntax[0].range.start.line, 2);
    assert_eq!(syntax[0].range.start.character, 13 + 9);
    assert_eq!(syntax[0].range.end.character, 13 + 10);
    assert!(syntax[2].message.starts_with("expected-close-paren"));
}

#[test]
fn condition_values_must_be_expressions_and_tags_are_quote_errors() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    pack_with_fields(root);
    write(
        root,
        "x.cdo.yml",
        "type: cdo\nconditions:\n  - config: 5\n  - config: !hotnum\n  - config: []\n  - config: [coldnum, 7]\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mA\n        value: 1\n",
    );
    let result = lint(root);
    let pairs = triples(&result, root);
    assert!(
        pairs.contains(&triple(
            "x.cdo.yml",
            "config.condition-invalid",
            "$.conditions[0].config"
        )),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&triple(
            "x.cdo.yml",
            "config.expression-quote",
            "$.conditions[1].config"
        )),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&triple(
            "x.cdo.yml",
            "config.condition-invalid",
            "$.conditions[2].config"
        )),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&triple(
            "x.cdo.yml",
            "config.condition-invalid",
            "$.conditions[3].config[1]"
        )),
        "{pairs:?}"
    );
}

#[test]
fn unquoted_expressions_and_templates_that_break_yaml_get_a_hint() {
    let quote = single(
        "x.cdo.yml",
        "type: cdo\nconditions:\n  config: \"a\" == b\npatches: []\n",
    );
    assert!(quote.diagnostics.iter().any(|d| d.code == "yaml.syntax"));
    let hint = one(&quote, "config.expression-quote");
    assert_eq!((hint.range.start.line, hint.range.start.character), (2, 10));
    let template = single(
        "x.cdo.yml",
        "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mA\n        value: {item:/Game/A.A_C:Fn({cfg:n})}\n",
    );
    assert!(template.diagnostics.iter().any(|d| d.code == "yaml.syntax"));
    let hint = one(&template, "template.unquoted-start");
    assert_eq!(hint.range.start.line, 5);
}

#[test]
fn element_conditions_are_validated_at_any_depth() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    pack_with_fields(root);
    write(
        root,
        "x.cdo.yml",
        "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mA\n        value:\n          - a: 1\n            conditions:\n              - config: coldnum > 1\n                hasMod: 'SML@'\n            conditionBehavior: maybe\n      - path: mB\n        value: 1\n        conditions:\n          - { config: nope }\n",
    );
    let result = lint(root);
    let pairs = triples(&result, root);
    assert!(
        pairs.contains(&triple(
            "x.cdo.yml",
            "semantic.condition-mod-spec",
            "$.patches[0].properties[0].value[0].conditions[0].hasMod"
        )),
        "{pairs:?}"
    );
    assert!(
        pairs.iter().any(|p| p.1.starts_with("schema.")
            && p.2 == "$.patches[0].properties[0].value[0].conditionBehavior"),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&triple(
            "x.cdo.yml",
            "config.unknown-id",
            "$.patches[0].properties[1].conditions[0].config"
        )),
        "{pairs:?}"
    );
    // Element conditions never leak into the kind schema.
    assert!(
        !pairs.iter().any(|p| p.1 == "schema.unevaluatedProperties"),
        "{pairs:?}"
    );
}

#[test]
fn conditions_inside_conditions_subtrees_are_not_element_conditions() {
    let result = single(
        "x.cdo.yml",
        "type: cdo\nconditions:\n  - hasMod: A\n    conditions: nested\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: m\n        value: 1\n",
    );
    // `nested` is an unexpected key of the condition object, reported by the schema.
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code.starts_with("schema.") && d.yaml_path.contains("conditions[0]")));
}

// ---------------------------------------------------------------------------------------------
// Single document behaviour
// ---------------------------------------------------------------------------------------------

#[test]
fn single_document_checks_syntax_but_not_cross_references() {
    let text = "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mA\n        value: cfg:anything\n      - path: mB\n        value: \"{cfg:\"\n      - path: mC\n        op: insert\n        index: cfg:x\n        value: 1\n        conditions:\n          config: unknown && 5 < 'a'\n";
    let result = single("x.cdo.yml", text);
    let pairs = result
        .diagnostics
        .iter()
        .map(|d| d.code.as_str())
        .collect::<Vec<_>>();
    assert!(pairs.contains(&"template.unbalanced"), "{pairs:?}");
    assert!(pairs.contains(&"config.expression-type"), "{pairs:?}");
    assert!(!pairs.contains(&"config.unknown-id"), "{pairs:?}");
}

#[test]
fn validate_document_and_workspace_agree_on_local_findings() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    pack_with_fields(root);
    let text = "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mA\n        value: \"{cfg:\"\n";
    write(root, "x.cdo.yml", text);
    let workspace = lint(root);
    let local = single("x.cdo.yml", text);
    assert_eq!(codes(&workspace), codes(&local));
}

#[test]
fn unsaved_buffers_feed_the_config_index() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    pack_with_fields(root);
    write(
        root,
        "x.cdo.yml",
        "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mA\n        value: cfg:fresh\n",
    );
    let before = lint_workspace(root, &[], &LintOptions::default()).unwrap();
    assert!(codes(&before).contains(&"config.unknown-id".to_owned()));
    let buffer = FileBuffer {
        path: root.join("fresh.config.yml"),
        text: "category: /Script/FactoryGame.FGCategory\nfields:\n  - {id: fresh, type: boolean, name: F, description: D, default: true}\n".into(),
    };
    let after = lint_workspace(root, &[buffer], &LintOptions::default()).unwrap();
    assert!(
        !codes(&after).contains(&"config.unknown-id".to_owned()),
        "{:?}",
        after.diagnostics
    );
}

#[test]
fn informational_diagnostics_do_not_count_as_warnings() {
    // Without reflection nothing is informational; the counting contract is checked here.
    let result = lint(&fixture_root());
    assert_eq!(
        result.warnings,
        result
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning && d.advisory.is_none())
            .count()
    );
}

#[test]
fn template_nesting_is_limited_to_four_levels() {
    let ok = "{item:A:F({item:B:F({item:C:F({item:D:g})})})}";
    let deep = "{item:A:F({item:B:F({item:C:F({item:D:F({item:E:g})})})})}";
    let text =
        |value: &str| format!("type: localization\nnamespace: N\nentries:\n  Key: \"{value}\"\n");
    let fine = single("x.localization.yml", &text(ok));
    assert!(fine.diagnostics.is_empty(), "{:?}", fine.diagnostics);
    let too_deep = single("x.localization.yml", &text(deep));
    assert_eq!(
        one(&too_deep, "template.nesting-depth").yaml_path,
        "$.entries.Key"
    );
}

#[test]
fn nested_pre_pass_tokens_inside_apply_tokens_are_checked() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    pack_with_fields(root);
    write(
        root,
        "x.localization.yml",
        "type: localization\nnamespace: N\nentries:\n  Ok: \"{item:/Script/FactoryGame.FGItemDescriptor:Fn({cfg:coldnum})}\"\n  Unknown: \"{item:/Script/FactoryGame.FGItemDescriptor:Fn({cfg:nope})}\"\n  Malformed: \"{item:/Script/FactoryGame.FGItemDescriptor:Fn({cfg:})}\"\n",
    );
    let result = lint(root);
    let pairs = triples(&result, root);
    assert_eq!(
        pairs,
        [
            triple(
                "x.localization.yml",
                "config.unknown-id",
                "$.entries.Unknown"
            ),
            triple(
                "x.localization.yml",
                "template.malformed",
                "$.entries.Malformed"
            ),
        ]
    );
}

#[test]
fn multi_document_config_files_share_one_id_space() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(root, "pack.yml", "ref: P\nname: P\nversion: 1.0.0\n");
    write(
        root,
        "multi.config.yml",
        "category: /Script/FactoryGame.FGCategory\nfields:\n  - {id: a, type: boolean, name: A, description: D, default: true}\n---\ntype: config\ncategory: /Script/FactoryGame.FGCategory\nfields:\n  - {id: b, type: boolean, name: B, description: D, default: true}\n  - {id: A, type: boolean, name: Again, description: D, default: true}\n",
    );
    let result = lint(root);
    assert_eq!(result.documents, 2);
    let duplicate = one(&result, "config.duplicate-id");
    assert_eq!(duplicate.document, Some(1));
    assert_eq!(duplicate.yaml_path, "$.fields[1].id");
    assert_eq!(duplicate.range.start.line, 8);
    let index = scan_workspace(root, &[]);
    let ids = index
        .pack("P")
        .unwrap()
        .fields
        .iter()
        .map(|field| (field.id.as_str(), field.document))
        .collect::<Vec<_>>();
    assert_eq!(ids, [("a", 0), ("b", 1)]);
}

#[test]
fn crlf_documents_report_the_same_positions() {
    let lf = "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mA\n        value: \"{cfgx:a}\"\n";
    let crlf = lf.replace('\n', "\r\n");
    let first = single("x.cdo.yml", lf);
    let second = single("x.cdo.yml", &crlf);
    let a = one(&first, "template.unknown-provider");
    let b = one(&second, "template.unknown-provider");
    assert_eq!(a.range, b.range);
    assert_eq!((a.range.start.line, a.range.start.character), (5, 17));
}

// ---------------------------------------------------------------------------------------------
// Runtime limits
// ---------------------------------------------------------------------------------------------

fn limit_diagnostics(result: &LintResult) -> Vec<(String, Severity)> {
    result
        .diagnostics
        .iter()
        .filter(|d| d.code == "config.limit-exceeded")
        .map(|d| (d.yaml_path.clone(), d.severity))
        .collect()
}

#[test]
fn definition_limits_of_the_runtime_are_enforced() {
    let defaults = |count: usize| vec!["1"; count].join(", ");
    let cases: Vec<(String, Option<&str>)> = vec![
        // maxCount / minCount of lists
        ("{id: a, type: number, multiple: true, name: N, description: D, default: [1], maxCount: 64}".into(), None),
        ("{id: a, type: number, multiple: true, name: N, description: D, default: [1], maxCount: 65}".into(), Some("$.fields[0].maxCount")),
        ("{id: a, type: text, multiple: true, name: N, description: D, default: [x], maxCount: 1000}".into(), Some("$.fields[0].maxCount")),
        ("{id: a, type: number, multiple: true, name: N, description: D, default: [1], minCount: 65}".into(), Some("$.fields[0].minCount")),
        // lists without maxCount are capped at 64
        (format!("{{id: a, type: number, multiple: true, name: N, description: D, default: [{}]}}", defaults(64)), None),
        (format!("{{id: a, type: number, multiple: true, name: N, description: D, default: [{}]}}", defaults(65)), Some("$.fields[0].default")),
        // text maxLength
        ("{id: a, type: text, name: N, description: D, default: x, maxLength: 1024}".into(), None),
        ("{id: a, type: text, name: N, description: D, default: x, maxLength: 1025}".into(), Some("$.fields[0].maxLength")),
        // multi-select maxSelected
        ("{id: a, type: select, multiple: true, name: N, description: D, default: [x], options: [{value: x, label: X}], maxSelected: 65}".into(), Some("$.fields[0].maxSelected")),
        // id length
        (format!("{{id: {}, type: boolean, name: N, description: D, default: true}}", "a".repeat(200)), None),
        (format!("{{id: {}, type: boolean, name: N, description: D, default: true}}", "a".repeat(201)), Some("$.fields[0].id")),
        (format!("{{id: {}, type: boolean, name: N, description: D, default: true}}", vec!["abcdefghi"; 25].join(".")), Some("$.fields[0].id")),
    ];
    for (field, expected) in cases {
        let result = definition(&field);
        let limits = limit_diagnostics(&result);
        match expected {
            Some(path) => assert_eq!(
                limits,
                vec![(path.to_owned(), Severity::Error)],
                "{field}: {:?}",
                result.diagnostics
            ),
            None => assert!(limits.is_empty(), "{field}: {:?}", result.diagnostics),
        }
    }
}

#[test]
fn definition_limit_violations_drop_the_field_like_the_runtime() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(root, "pack.yml", "ref: P\nname: P\nversion: 1.0.0\n");
    write(
        root,
        "cfg.config.yml",
        "category: /Script/FactoryGame.FGCategory\nfields:\n  - {id: ok, type: boolean, name: N, description: D, default: true}\n  - {id: tooLong, type: text, name: N, description: D, default: x, maxLength: 5000}\n",
    );
    let index = scan_workspace(root, &[]);
    let pack = index.pack("P").unwrap();
    assert!(pack.field("ok").is_some());
    assert!(pack.field("tooLong").is_none());
}

#[test]
fn pack_ref_longer_than_fifty_characters_is_rejected_for_packs_with_configs() {
    let config = "category: /Script/FactoryGame.FGCategory\nfields:\n  - {id: a, type: boolean, name: N, description: D, default: true}\n";
    for (length, with_config, expected) in [
        (50usize, true, false),
        (51, true, true),
        (51, false, false),
        (200, true, true),
    ] {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let pack_ref = "p".repeat(length);
        write(
            root,
            "pack.yml",
            &format!("ref: {pack_ref}\nname: P\nversion: 1.0.0\n"),
        );
        if with_config {
            write(root, "cfg.config.yml", config);
        }
        let result = lint(root);
        let limits = result
            .diagnostics
            .iter()
            .filter(|d| d.code == "config.limit-exceeded")
            .collect::<Vec<_>>();
        assert_eq!(
            !limits.is_empty(),
            expected,
            "{length} {with_config}: {:?}",
            result.diagnostics
        );
        if expected {
            assert_eq!(limits.len(), 1, "{:?}", result.diagnostics);
            assert!(
                limits[0]
                    .file
                    .as_deref()
                    .is_some_and(|file| file.ends_with("pack.yml")),
                "{:?}",
                limits[0]
            );
            assert_eq!(limits[0].yaml_path, "$.ref");
            assert_eq!(limits[0].severity, Severity::Error);
            assert_eq!(limits[0].range.start.line, 0);
            assert_eq!(limits[0].range.start.character, 5);
            assert_eq!(limits[0].range.end.character, 5 + length as u32);
        }
    }
}
