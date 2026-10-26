//! Hover, signature help, navigation and quick fix tests on the shared fixture workspace.

use crate::actions::code_actions;
use crate::config_nav;
use crate::test_fixture::{Fixture, ITEM};
use kdf_validation::{lint_workspace, LintSettings, Profile};
use tower_lsp::lsp_types::{
    CodeAction, Diagnostic, NumberOrString, ParameterLabel, Position, Range, TextEdit, Url,
};

const TUNING: &str =
    include_str!("../../kdf-validation/tests/fixtures/config/DataForge/ConfigDemo/tuning.cdo.yml");
const GATED: &str = include_str!(
    "../../kdf-validation/tests/fixtures/config/DataForge/ConfigDemo/gated.recipe.yml"
);

fn card(text: &str) -> String {
    text.to_owned()
}

/// Every cursor position of every document must be answered without panicking.
#[test]
fn features_survive_every_cursor_position_of_realistic_documents() {
    use crate::completion::complete;
    use crate::hover::hover;
    use crate::signature::signature_help;
    let fixture = Fixture::new();
    let documents = vec![
        Fixture::op_doc(ITEM, "mEnergyValue", "multiply", "cfg:balance.speed"),
        Fixture::op_doc(
            ITEM,
            "mDisplayName",
            "set",
            "\"{cfg:balance.title} {item:/Script/FactoryGame.FGItemDescriptor:GetEnergyValue(1, {cfg:balance.stack})} \\\\{x}\"",
        ),
        Fixture::op_doc(ITEM, "mAmounts[0].Amount", "append", "{ ItemClass: /Game/A.A_C, Amount: cfg:balance.stack }"),
        Fixture::op_doc(ITEM, "mTags", "set", "[cfg:balance.names, 'x', \"y\"]"),
        "type: cdo\nconditions:\n  config:\n    - \"balance.mode == 'hard' && cfg:Other:shared.level in [1, 2]\"\n    - !x\npatches: []\n".to_owned(),
        "type: config\ncategory: DemoCategory\nfields:\n  - id: a.b\n    type: select\n    options:\n      - { value: x, label: X }\n    default: x # note\n  - id: c\n    tooltip: { title: T }\n".to_owned(),
        "type: item\nitems:\n  - id: A\n    properties:\n      - path: mForm\n---\ntype: recipe\nrecipes:\n  - id: B\n".to_owned(),
        "value: \u{1F600} \"\u{e9}{cfg:\"\r\n- [a, {b: c\n".to_owned(),
    ];
    for source in documents {
        for (line_no, line) in source.split('\n').enumerate() {
            let width = line.encode_utf16().count() as u32;
            for column in 0..=width {
                let position = tower_lsp::lsp_types::Position {
                    line: line_no as u32,
                    character: column,
                };
                fixture.with_text("ConfigDemo/tuning.cdo.yml", &source, |env| {
                    let resolved = crate::context::resolve(env.path, env.text, position);
                    let _ = complete(env, &resolved);
                    let _ = hover(env, &resolved);
                    let _ = signature_help(env, &resolved);
                    let _ = config_nav::definition(env, position);
                });
            }
        }
    }
}

// ---- hover ------------------------------------------------------------------------------------

#[test]
fn typed_reference_hover_shows_the_config_card() {
    let fixture = Fixture::new();
    let source = Fixture::op_doc(ITEM, "mEnergyValue", "multiply", "cfg:balance.sp|eed");
    let (text, covered) = fixture
        .hover("ConfigDemo/tuning.cdo.yml", &source)
        .expect("hover");
    assert_eq!(covered, "cfg:balance.speed");
    for part in [
        "Machine speed",
        "`balance.speed`",
        "number · float",
        "hot",
        "Multiplier applied to production time.",
        "Values above 2 are very fast.",
        "Default: `1.5`",
        "Range: `0.5` to `4`",
        "step `0.5`",
        "Pack: `ConfigDemo`",
        "File: `ConfigDemo/balance.config.yml`",
        "Category: `DemoCategory`",
    ] {
        assert!(text.contains(part), "{part:?} missing in {}", card(&text));
    }
}

#[test]
fn select_options_and_other_pack_references() {
    let fixture = Fixture::new();
    let source = Fixture::op_doc(
        ITEM,
        "mDisplayName",
        "set",
        "\"{cfg:balance.mo|de} / {cfg:Other:shared.level}\"",
    );
    let (text, covered) = fixture
        .hover("ConfigDemo/tuning.cdo.yml", &source)
        .expect("hover");
    assert_eq!(covered, "balance.mode");
    assert!(
        text.contains("`easy` (Easy)") && text.contains("`hard` (Hard)"),
        "{text}"
    );
    assert!(text.contains("cold"), "{text}");
    let source = Fixture::op_doc(ITEM, "mDisplayName", "set", "\"{cfg:Other:shared.le|vel}\"");
    let (text, _) = fixture
        .hover("ConfigDemo/tuning.cdo.yml", &source)
        .expect("hover");
    assert!(
        text.contains("Pack: `Other`") && text.contains("Range: `1` to `5`"),
        "{text}"
    );
}

#[test]
fn setting_hover_shows_the_setting_card() {
    let fixture = Fixture::new();
    let source = Fixture::op_doc(
        ITEM,
        "mEnergyValue",
        "multiply",
        "session:KMods.SFP.Production|Multiplier",
    );
    let (text, _) = fixture
        .hover("ConfigDemo/tuning.cdo.yml", &source)
        .expect("hover");
    assert!(
        text.contains("Production multiplier") && text.contains("`float` setting"),
        "{text}"
    );
    assert!(
        text.contains("Range: `0.1` to `10`") && text.contains("Session wide: yes"),
        "{text}"
    );
}

#[test]
fn expression_operand_hover() {
    let fixture = Fixture::new();
    let source = "type: cdo\nconditions:\n  config: \"balance.mode == 'hard' && cfg:Other:shared.le|vel > 1\"\npatches: []\n";
    let (text, covered) = fixture
        .hover("ConfigDemo/tuning.cdo.yml", source)
        .expect("hover");
    assert_eq!(covered, "cfg:Other:shared.level");
    assert!(text.contains("Shared level."), "{text}");
    let source = "type: cdo\nconditions:\n  config: \"balance.mo|de == 'hard'\"\npatches: []\n";
    let (text, covered) = fixture
        .hover("ConfigDemo/tuning.cdo.yml", source)
        .expect("hover");
    assert_eq!(covered, "balance.mode");
    assert!(text.contains("Difficulty"), "{text}");
}

#[test]
fn unknown_references_say_so() {
    let fixture = Fixture::new();
    let source = Fixture::op_doc(ITEM, "mEnergyValue", "multiply", "cfg:balance.nothing|");
    let (text, _) = fixture
        .hover("ConfigDemo/tuning.cdo.yml", &source)
        .expect("hover");
    assert!(text.contains("No config field `balance.nothing`"), "{text}");
}

#[test]
fn config_declaration_hover() {
    let fixture = Fixture::new();
    let (text, _) = fixture
        .hover(
            "ConfigDemo/balance.config.yml",
            "type: config\ncategory: DemoCategory\nfields:\n  - id: balance.sta|ck\n",
        )
        .expect("hover");
    assert!(
        text.contains("Stack size") && text.contains("Items per stack."),
        "{text}"
    );
}

#[test]
fn path_hover_shows_the_reload_classification_and_reason() {
    let fixture = Fixture::new();
    let hover = |target: &str, path: &str, extra: &str| {
        let source = format!(
            "type: cdo\npatches:\n  - target: {target}\n{extra}    properties:\n      - path: {path}\n"
        );
        fixture
            .hover("ConfigDemo/tuning.cdo.yml", &source)
            .expect("hover")
            .0
    };
    let form = hover(ITEM, "mFo|rm", "");
    assert!(
        form.contains("Reload: **cold**") && form.contains("Item form is read when"),
        "{form}"
    );
    let stack = hover(ITEM, "mStack|Size", "");
    assert!(stack.contains("Reload: hot"), "{stack}");
    let health = hover("/Script/FactoryGame.FGBuildable", "mHea|lth", "");
    assert!(health.contains("Reload: **newActors**"), "{health}");
    assert!(
        health.contains("`float`") && health.contains("FGBuildable"),
        "{health}"
    );
    let spawned = hover(
        "/Script/FactoryGame.FGBuildable",
        "mHea|lth",
        "    applyToSpawnedActors: true\n",
    );
    assert!(spawned.contains("Reload: hot"), "{spawned}");
    let nested = hover(ITEM, "mAmounts[0].Amo|unt", "");
    assert!(nested.contains("`int32`"), "{nested}");
}

#[test]
fn template_member_hover_shows_signature_and_flags() {
    let fixture = Fixture::new();
    let value = |body: &str| format!("\"{{item:{ITEM}:{body}}}\"");
    let call = Fixture::op_doc(ITEM, "mDisplayName", "set", &value("GetEnergy|Value(2)"));
    let (text, covered) = fixture
        .hover("ConfigDemo/tuning.cdo.yml", &call)
        .expect("hover");
    assert_eq!(covered, "GetEnergyValue(2)");
    assert!(
        text.contains(
            "GetEnergyValue(ItemClass: TSubclassOf<UObject>, Multiplier: float) -> float"
        ),
        "{text}"
    );
    assert!(
        text.contains("`Static`") && text.contains("`BlueprintPure`"),
        "{text}"
    );
    assert!(text.contains("Callable in templates"), "{text}");
    let implicit = Fixture::op_doc(ITEM, "mDisplayName", "set", &value("GetItem|Name"));
    let (text, _) = fixture
        .hover("ConfigDemo/tuning.cdo.yml", &implicit)
        .expect("hover");
    assert!(
        text.contains("the target class is passed implicitly"),
        "{text}"
    );
    let property = Fixture::op_doc(ITEM, "mDisplayName", "set", &value("mFo|rm"));
    let (text, _) = fixture
        .hover("ConfigDemo/tuning.cdo.yml", &property)
        .expect("hover");
    assert!(
        text.contains("Reload: **cold**") && text.contains("EResourceForm")
            || text.contains("RF_SOLID"),
        "{text}"
    );
    let blocked = Fixture::op_doc(ITEM, "mDisplayName", "set", &value("Fore|achSlot"));
    let (text, _) = fixture
        .hover("ConfigDemo/tuning.cdo.yml", &blocked)
        .expect("hover");
    assert!(text.contains("Not callable in templates"), "{text}");
}

#[test]
fn template_target_and_provider_hover() {
    let fixture = Fixture::new();
    let target = Fixture::op_doc(
        ITEM,
        "mDisplayName",
        "set",
        "\"{item:/Script/FactoryGame.FGItemDesc|riptor:mForm}\"",
    );
    let (text, covered) = fixture
        .hover("ConfigDemo/tuning.cdo.yml", &target)
        .expect("hover");
    assert_eq!(covered, "/Script/FactoryGame.FGItemDescriptor");
    assert!(
        text.contains("FGItemDescriptor") && text.contains("Abstract: no"),
        "{text}"
    );
    let provider = Fixture::op_doc(
        ITEM,
        "mDisplayName",
        "set",
        "\"{it|em:/Script/FactoryGame.FGItemDescriptor:mForm}\"",
    );
    let (text, _) = fixture
        .hover("ConfigDemo/tuning.cdo.yml", &provider)
        .expect("hover");
    assert!(
        text.contains("Item descriptor class") && text.contains("FGItemDescriptor"),
        "{text}"
    );
}

#[test]
fn category_hover_summarises_the_class() {
    let fixture = Fixture::new();
    let (text, _) = fixture
        .hover(
            "ConfigDemo/new.config.yml",
            "type: config\ncategory: /Script/FactoryGame.FGItem|Category\nfields: []\n",
        )
        .expect("hover");
    assert!(
        text.contains("FGItemCategory")
            && text.contains("Parent: `/Script/FactoryGame.FGCategory`"),
        "{text}"
    );
    let (text, _) = fixture
        .hover(
            "ConfigDemo/new.config.yml",
            "type: config\ncategory: DemoCa|tegory\nfields: []\n",
        )
        .expect("hover");
    assert!(text.contains("FGCategory"), "{text}");
}

// ---- signature help ---------------------------------------------------------------------------

fn call(fixture: &Fixture, arguments: &str) -> Option<tower_lsp::lsp_types::SignatureHelp> {
    let source = Fixture::op_doc(
        ITEM,
        "mDisplayName",
        "set",
        &format!("\"{{item:{ITEM}:{arguments}\""),
    );
    fixture.with_env(
        "ConfigDemo/tuning.cdo.yml",
        &source,
        true,
        true,
        |env, resolved| {
            let negotiated = crate::semantic::Env {
                caps: crate::state::ClientCaps {
                    signature_active_parameter: true,
                    ..env.caps
                },
                ..*env
            };
            crate::signature::signature_help(&negotiated, resolved)
        },
    )
}

#[test]
fn signature_help_tracks_the_active_parameter() {
    let fixture = Fixture::new();
    let help = call(&fixture, "GetEnergyValue(|").expect("help");
    let signature = &help.signatures[0];
    assert_eq!(
        signature.label,
        "GetEnergyValue(ItemClass: TSubclassOf<UObject>, Multiplier: float) -> float"
    );
    assert_eq!(help.active_parameter, Some(0));
    assert_eq!(signature.active_parameter, Some(0));
    let parameters = signature.parameters.as_ref().unwrap();
    assert_eq!(parameters.len(), 2);
    let ParameterLabel::LabelOffsets([start, end]) = parameters[1].label else {
        panic!("offsets expected");
    };
    let label: Vec<char> = signature.label.chars().collect();
    assert_eq!(
        label[start as usize..end as usize]
            .iter()
            .collect::<String>(),
        "Multiplier: float"
    );
    let second = call(&fixture, "GetEnergyValue(/Script/A.B, |").expect("help");
    assert_eq!(second.active_parameter, Some(1));
    // Past the last parameter the last one stays active.
    let third = call(&fixture, "GetEnergyValue(1, 2, |").expect("help");
    assert_eq!(third.active_parameter, Some(1));
}

#[test]
fn signature_help_shows_the_implicit_parameter_in_the_notes() {
    let fixture = Fixture::new();
    let help = call(&fixture, "GetItemName(|").expect("help");
    let signature = &help.signatures[0];
    assert_eq!(signature.label, "GetItemName() -> FText");
    assert_eq!(help.active_parameter, None);
    let Some(tower_lsp::lsp_types::Documentation::MarkupContent(notes)) = &signature.documentation
    else {
        panic!("notes expected");
    };
    assert!(notes.value.contains("target class is passed implicitly"));
}

#[test]
fn signature_help_closes_outside_calls_and_for_unknown_functions() {
    let fixture = Fixture::new();
    assert!(call(&fixture, "mForm|").is_none());
    assert!(call(&fixture, "NoSuchFunction(|").is_none());
    let source = Fixture::op_doc(ITEM, "mDisplayName", "set", "plain text|");
    assert!(fixture
        .signature("ConfigDemo/tuning.cdo.yml", &source)
        .is_none());
}

#[test]
fn signature_help_for_operations_is_kept() {
    let fixture = Fixture::new();
    let help = fixture
        .signature("ConfigDemo/tuning.cdo.yml", "op: multiply|")
        .expect("help");
    assert_eq!(help.signatures[0].label, "multiply(value)");
}

// ---- navigation -------------------------------------------------------------------------------

fn nav_fixture() -> Fixture {
    Fixture::with_files(&[
        ("ConfigDemo/tuning.cdo.yml", TUNING),
        ("ConfigDemo/gated.recipe.yml", GATED),
    ])
}

fn position_of(text: &str, needle: &str, extra: usize) -> Position {
    let (line, content) = text
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not found"));
    Position {
        line: line as u32,
        character: (content.find(needle).unwrap() + extra) as u32,
    }
}

#[test]
fn definition_leads_to_the_field_id() {
    let fixture = nav_fixture();
    let balance =
        std::fs::read_to_string(fixture.root.join("ConfigDemo/balance.config.yml")).unwrap();
    let other = std::fs::read_to_string(fixture.root.join("Other/shared.config.yml")).unwrap();
    let cases = [
        (
            "value: cfg:balance.stack",
            12,
            "balance.stack",
            "ConfigDemo/balance.config.yml",
            &balance,
        ),
        (
            "cfg:Other:shared.level}",
            8,
            "shared.level",
            "Other/shared.config.yml",
            &other,
        ),
        (
            "{cfg:balance.title}",
            8,
            "balance.title",
            "ConfigDemo/balance.config.yml",
            &balance,
        ),
        (
            "balance.mode == \"hard\"",
            3,
            "balance.mode",
            "ConfigDemo/balance.config.yml",
            &balance,
        ),
    ];
    for (needle, extra, id, file, declared) in cases {
        let position = position_of(TUNING, needle, extra);
        let location = fixture
            .with_text("ConfigDemo/tuning.cdo.yml", TUNING, |env| {
                config_nav::definition(env, position)
            })
            .unwrap_or_else(|| panic!("no definition for {needle}"));
        assert_eq!(
            location.uri,
            Url::from_file_path(fixture.root.join(file)).unwrap(),
            "{needle}"
        );
        let line = declared
            .lines()
            .nth(location.range.start.line as usize)
            .unwrap();
        assert!(line.contains(&format!("id: {id}")), "{needle}: {line}");
    }
}

#[test]
fn bare_category_ids_lead_to_the_class_declaration() {
    let fixture = nav_fixture();
    let source = "type: config\ncategory: DemoCategory\nfields: []\n";
    let location = crate::navigation::definition(
        &fixture.root,
        &Url::from_file_path(fixture.root.join("ConfigDemo/new.config.yml")).unwrap(),
        source,
        Position {
            line: 1,
            character: 14,
        },
        &crate::docs::Sources::default(),
    )
    .expect("category definition");
    assert_eq!(
        location.uri,
        Url::from_file_path(fixture.root.join("ConfigDemo/categories.class.yml")).unwrap()
    );
}

#[test]
fn definition_ignores_unknown_ids_and_plain_text() {
    let fixture = nav_fixture();
    let source = "type: cdo\npatches:\n  - target: /Script/A.B\n    properties:\n      - path: mX\n        value: cfg:nothing.here\n        note: cfg:balance.speed inside text\n";
    let missing = fixture.with_text("ConfigDemo/tuning.cdo.yml", source, |env| {
        config_nav::definition(env, position_of(source, "cfg:nothing", 6))
    });
    assert!(missing.is_none());
}

#[test]
fn references_find_typed_template_and_expression_uses() {
    let fixture = nav_fixture();
    let balance_text =
        std::fs::read_to_string(fixture.root.join("ConfigDemo/balance.config.yml")).unwrap();
    let root = fixture.root.clone();
    let buffers = crate::docs::Sources::default();
    let usages = |text: &str, path: &str, needle: &str, extra: usize, declaration: bool| {
        let position = position_of(text, needle, extra);
        fixture.with_text(path, text, |env| {
            config_nav::references(env, &root, position, &buffers, declaration)
        })
    };
    // From the declaration: the three uses in the pack plus the declaration itself.
    let from_declaration = usages(
        &balance_text,
        "ConfigDemo/balance.config.yml",
        "id: balance.speed",
        6,
        true,
    )
    .expect("references");
    let in_tuning = from_declaration
        .iter()
        .filter(|l| l.uri.path().ends_with("tuning.cdo.yml"))
        .count();
    let in_gated = from_declaration
        .iter()
        .filter(|l| l.uri.path().ends_with("gated.recipe.yml"))
        .count();
    let in_config = from_declaration
        .iter()
        .filter(|l| l.uri.path().ends_with("balance.config.yml"))
        .count();
    assert_eq!(
        (in_tuning, in_gated, in_config),
        (2, 1, 1),
        "{from_declaration:?}"
    );
    // From a use, without the declaration.
    let from_use = usages(
        TUNING,
        "ConfigDemo/tuning.cdo.yml",
        "cfg:balance.speed",
        6,
        false,
    )
    .expect("references");
    assert_eq!(from_use.len(), 3);
    // The pack-qualified reference belongs to the other pack's field.
    let other = usages(
        TUNING,
        "ConfigDemo/tuning.cdo.yml",
        "cfg:Other:shared.level",
        14,
        false,
    )
    .expect("references");
    assert!(other.iter().all(|l| l.uri.path().ends_with("tuning.cdo.yml") || l.uri.path().ends_with("pack.yml")), "{other:?}");
    assert!(other.len() >= 2, "{other:?}");
}

// ---- quick fixes ------------------------------------------------------------------------------

struct Linted {
    fixture: Fixture,
    uri: Url,
    text: String,
    diagnostics: Vec<Diagnostic>,
}

/// Writes `text` as `ConfigDemo/lint.cdo.yml`, lints the workspace and keeps the diagnostics.
fn lint(text: &str) -> Linted {
    let fixture = Fixture::with_files(&[("ConfigDemo/lint.cdo.yml", text)]);
    let settings = LintSettings::default();
    let mut options = settings.lint_options(Profile::Runtime);
    options.datapack_index = Some(fixture.reflection.clone());
    let report = lint_workspace(&fixture.root, &[], &options).expect("lint");
    let path = dunce::canonicalize(fixture.root.join("ConfigDemo/lint.cdo.yml")).unwrap();
    let diagnostics = report
        .diagnostics
        .into_iter()
        .filter(|item| {
            item.file
                .as_deref()
                .map(|file| dunce::canonicalize(file).ok())
                == Some(Some(path.clone()))
        })
        .map(|item| Diagnostic {
            range: Range {
                start: Position {
                    line: item.range.start.line,
                    character: item.range.start.character,
                },
                end: Position {
                    line: item.range.end.line,
                    character: item.range.end.character,
                },
            },
            code: Some(NumberOrString::String(item.code)),
            message: item.message,
            ..Diagnostic::default()
        })
        .collect();
    Linted {
        uri: Url::from_file_path(&path).unwrap(),
        fixture,
        text: text.to_owned(),
        diagnostics,
    }
}

impl Linted {
    fn fixes(&self, code: &str) -> Vec<CodeAction> {
        let matching: Vec<Diagnostic> = self
            .diagnostics
            .iter()
            .filter(|d| matches!(&d.code, Some(NumberOrString::String(c)) if c == code))
            .cloned()
            .collect();
        assert!(
            !matching.is_empty(),
            "lint produced no {code}: {:?}",
            self.diagnostics
                .iter()
                .map(|d| (&d.code, &d.message))
                .collect::<Vec<_>>()
        );
        self.fixture
            .with_text("ConfigDemo/lint.cdo.yml", &self.text, |env| {
                code_actions(env, &self.uri, &matching)
            })
    }

    /// The text after applying the single edit of `action`.
    fn apply(&self, action: &CodeAction) -> String {
        let changes = action.edit.as_ref().unwrap().changes.as_ref().unwrap();
        let edits: &Vec<TextEdit> = changes.get(&self.uri).unwrap();
        assert_eq!(edits.len(), 1);
        let edit = &edits[0];
        let mut lines: Vec<String> = self.text.lines().map(str::to_owned).collect();
        let line = &mut lines[edit.range.start.line as usize];
        let start = crate::context::byte_at(line, edit.range.start.character);
        let end = crate::context::byte_at(line, edit.range.end.character);
        line.replace_range(start..end, &edit.new_text);
        lines.join("\n")
    }
}

fn doc(value: &str) -> String {
    Fixture::op_doc(ITEM, "mEnergyValue", "multiply", value)
}

#[test]
fn unquoted_templates_and_expressions_get_a_quote_wrap() {
    let linted = lint(&Fixture::op_doc(
        ITEM,
        "mDisplayName",
        "set",
        "{cfg:balance.title} / {cfg:balance.mode}",
    ));
    let fixes = linted.fixes("template.unquoted-start");
    assert_eq!(fixes[0].title, "Wrap the value in quotes");
    assert!(linted
        .apply(&fixes[0])
        .contains("value: \"{cfg:balance.title} / {cfg:balance.mode}\""));
    let expression =
        "type: cdo\nconditions:\n  config: !balance.enabled && balance.speed > 1\npatches: []\n";
    let linted = lint(expression);
    let fixes = linted.fixes("config.expression-quote");
    assert!(linted
        .apply(&fixes[0])
        .contains("config: \"!balance.enabled && balance.speed > 1\""));
}

#[test]
fn case_mismatches_are_fixed() {
    let linted = lint(&doc("cfg:Balance.Speed"));
    let fixes = linted.fixes("config.case-mismatch");
    assert_eq!(fixes[0].title, "Change to \"balance.speed\"");
    assert!(linted.apply(&fixes[0]).contains("value: cfg:balance.speed"));
}

#[test]
fn unknown_ids_get_did_you_mean_suggestions() {
    let linted = lint(&doc("cfg:balance.spede"));
    let fixes = linted.fixes("config.unknown-id");
    assert_eq!(fixes[0].title, "Change to \"balance.speed\"");
    assert!(linted.apply(&fixes[0]).contains("value: cfg:balance.speed"));
    let templated = lint(&Fixture::op_doc(
        ITEM,
        "mDisplayName",
        "set",
        "\"x {cfg:balance.stak} y\"",
    ));
    let fixes = templated.fixes("config.unknown-id");
    assert!(templated
        .apply(&fixes[0])
        .contains("\"x {cfg:balance.stack} y\""));
    let qualified = lint(&doc("cfg:Other:shared.levl"));
    let fixes = qualified.fixes("config.unknown-id");
    assert!(qualified
        .apply(&fixes[0])
        .contains("value: cfg:Other:shared.level"));
}

#[test]
fn unknown_packs_and_providers_and_settings_have_fixes() {
    let pack = lint(&doc("cfg:Otherr:shared.level"));
    let fixes = pack.fixes("config.unknown-pack");
    assert!(
        pack.apply(&fixes[0])
            .contains("value: cfg:Other:shared.level"),
        "{}",
        fixes[0].title
    );
    let provider = lint(&Fixture::op_doc(
        ITEM,
        "mDisplayName",
        "set",
        "\"{itm:/Script/FactoryGame.FGItemDescriptor:mForm}\"",
    ));
    let fixes = provider.fixes("template.unknown-provider");
    assert_eq!(fixes[0].title, "Change provider to \"item\"");
    assert!(provider
        .apply(&fixes[0])
        .contains("{item:/Script/FactoryGame.FGItemDescriptor:mForm}"));
    let setting = lint(&doc("session:KMods.SFP.ProductionMultiplyer"));
    let fixes = setting.fixes("setting.unknown-id");
    assert!(setting
        .apply(&fixes[0])
        .contains("session:KMods.SFP.ProductionMultiplier"));
}

/// Hover and completion share one list of keys whose values the runtime never resolves.
#[test]
fn keys_that_are_never_resolved_get_no_reference_hover() {
    use crate::semantic::unresolved_key;
    for key in ["id", "type", "include", "ref"] {
        assert!(unresolved_key(key), "{key}");
    }
    for key in ["value", "path", "default", "target"] {
        assert!(!unresolved_key(key), "{key}");
    }
    let fixture = Fixture::new();
    for key in ["ref", "include"] {
        let source = format!("type: cdo\n{key}: cfg:balance.sp|eed\npatches: []\n");
        assert!(
            fixture
                .hover("ConfigDemo/tuning.cdo.yml", &source)
                .is_none(),
            "{key} is not a value position"
        );
    }
    let source = "type: cdo\nvalue: cfg:balance.sp|eed\npatches: []\n";
    assert!(fixture.hover("ConfigDemo/tuning.cdo.yml", source).is_some());
}
