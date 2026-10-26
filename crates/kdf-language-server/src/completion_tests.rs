//! Completion tests: one fixture workspace (two config packs) plus a synthetic reflection archive.

use crate::completion::Completions;
use crate::state::ClientCaps;
use crate::test_fixture::Fixture;
use crate::test_support as support;
use serde_json::json;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Documentation, InsertTextFormat,
    MarkupContent, MarkupKind,
};

const ITEM: &str = "/Script/FactoryGame.FGItemDescriptor";

#[test]
fn actor_lifecycle_action_completes_supported_values() {
    let fixture = Fixture::new();
    let base =
        "type: cdo\npatches:\n  - target: /Script/Engine.Actor\n    applyToSpawnedActors: true\n";
    let values = fixture.run(
        "ConfigDemo/actions.cdo.yml",
        &format!("{base}    actorAction: |\n"),
    );
    present(&values, &["destroy", "dismantle"]);
}

#[test]
fn ftext_map_keys_complete_without_mixing_string_table_and_namespace_identity() {
    let fixture = Fixture::new();
    let base = format!("type: cdo\npatches:\n  - target: {ITEM}\n    properties:\n      - path: mDisplayName\n        value:\n");
    let empty = fixture.run("ConfigDemo/text.cdo.yml", &format!("{base}          |\n"));
    present(&empty, &["key", "stringTable", "ns", "source"]);
    let table = fixture.run(
        "ConfigDemo/text.cdo.yml",
        &format!("{base}          stringTable: /Example/ST_Text.ST_Text\n          |\n"),
    );
    assert_eq!(labels(&table), ["key"]);
    let keyed = fixture.run(
        "ConfigDemo/text.cdo.yml",
        &format!("{base}          ns: Example\n          |\n"),
    );
    present(&keyed, &["key", "source"]);
    absent(&keyed, &["stringTable"]);
}

fn labels(completions: &Completions) -> Vec<&str> {
    completions
        .items
        .iter()
        .map(|item| item.label.as_str())
        .collect()
}

fn item<'a>(completions: &'a Completions, label: &str) -> &'a CompletionItem {
    completions
        .items
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("no item {label:?} in {:?}", labels(completions)))
}

fn markdown_documentation(item: &CompletionItem) -> &MarkupContent {
    let Some(Documentation::MarkupContent(documentation)) = item.documentation.as_ref() else {
        panic!("no markdown documentation on {}", item.label);
    };
    assert_eq!(documentation.kind, MarkupKind::Markdown);
    documentation
}

fn edit(item: &CompletionItem) -> (u32, u32, &str) {
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("no text edit on {}", item.label);
    };
    assert_eq!(
        edit.range.start.line, edit.range.end.line,
        "single-line edit"
    );
    (
        edit.range.start.character,
        edit.range.end.character,
        edit.new_text.as_str(),
    )
}

fn command(item: &CompletionItem) -> Option<&str> {
    item.command
        .as_ref()
        .map(|command| command.command.as_str())
}

fn absent(completions: &Completions, names: &[&str]) {
    for name in names {
        assert!(
            !labels(completions).contains(name),
            "{name:?} must not be offered: {:?}",
            labels(completions)
        );
    }
}

fn present(completions: &Completions, names: &[&str]) {
    for name in names {
        assert!(
            labels(completions).contains(name),
            "{name:?} missing in {:?}",
            labels(completions)
        );
    }
}

// ---- operand values -------------------------------------------------------------------------

#[test]
fn cfg_ids_are_filtered_by_the_numeric_slot() {
    let fixture = Fixture::new();
    let completions = fixture.op(ITEM, "mEnergyValue", "multiply", "cfg:|");
    present(&completions, &["balance.speed", "balance.stack", "Other:"]);
    absent(
        &completions,
        &[
            "balance.enabled",
            "balance.title",
            "balance.mode",
            "balance.tiers",
            "balance.tag",
            "balance.names",
        ],
    );
    // The edit covers the id segment after `cfg:` and stays on one line.
    let speed = item(&completions, "balance.speed");
    let (start, end, text) = edit(speed);
    assert_eq!((start, end, text), (19, 19, "balance.speed"));
    assert_eq!(speed.filter_text.as_deref(), Some("balance.speed"));
    assert_eq!(speed.kind, Some(CompletionItemKind::VARIABLE));
}

#[test]
fn cfg_ids_follow_the_boolean_slot_of_the_property() {
    let fixture = Fixture::new();
    let completions = fixture.op(ITEM, "mCanBeDiscarded", "set", "cfg:|");
    present(&completions, &["balance.enabled"]);
    absent(
        &completions,
        &["balance.speed", "balance.stack", "balance.title"],
    );
}

#[test]
fn integer_ids_are_convertible_for_enum_slots_and_sorted_after_exact_ones() {
    let fixture = Fixture::new();
    let completions = fixture.op(ITEM, "mStackSize", "set", "cfg:|");
    let stack = item(&completions, "balance.stack");
    assert!(
        stack.sort_text.as_deref().unwrap().starts_with('1'),
        "{:?}",
        stack.sort_text
    );
    absent(&completions, &["balance.speed"]);
}

#[test]
fn operand_stubs_list_every_compatible_reference_and_the_providers() {
    let fixture = Fixture::new();
    let completions = fixture.op(ITEM, "mEnergyValue", "multiply", "|");
    present(
        &completions,
        &[
            "cfg:balance.speed",
            "cfg:balance.stack",
            "cfg:Other:shared.level",
            "cfg:",
            "session:",
            "ags:",
            "option:",
        ],
    );
    let own = item(&completions, "cfg:balance.speed");
    let other = item(&completions, "cfg:Other:shared.level");
    assert!(own.sort_text < other.sort_text, "own pack first");
    assert_eq!(
        command(item(&completions, "cfg:")),
        Some("editor.action.triggerSuggest")
    );
    assert_eq!(edit(own).2, "cfg:balance.speed");
}

#[test]
fn empty_operands_offer_reflection_templates_and_keep_yaml_quotes() {
    let fixture = Fixture::new();
    for value in ["|", "\"|\"", "'|'", "ite|", "\"ite|\""] {
        let source = Fixture::op_doc(ITEM, "mDisplayName", "set", value);
        let plain = fixture.run_with("ConfigDemo/tuning.cdo.yml", &source, false, true);
        for prefix in kdf_validation::template::APPLY_PREFIXES
            .into_iter()
            .filter(|prefix| !value.contains("ite") || prefix.starts_with("ite"))
        {
            let entry = item(&plain, &format!("{{{prefix}:}}"));
            let expected = if value == "|" || value == "ite|" {
                format!("'{{{prefix}:}}'")
            } else {
                format!("{{{prefix}:}}")
            };
            assert_eq!(edit(entry).2, expected);
            assert_eq!(command(entry), Some("editor.action.triggerSuggest"));
        }
        let snippets = fixture.run_with("ConfigDemo/tuning.cdo.yml", &source, true, true);
        let entry = item(&snippets, "{item:}");
        assert_eq!(entry.insert_text_format, Some(InsertTextFormat::SNIPPET));
        assert_eq!(
            edit(entry).2,
            if value == "|" || value == "ite|" {
                "'{item:$0}'"
            } else {
                "{item:$0}"
            }
        );
    }
}

#[test]
fn selecting_item_template_continues_with_targets_and_members() {
    let fixture = Fixture::new();
    let source = Fixture::op_doc(ITEM, "mDisplayName", "set", "|");
    let stubs = fixture.run_with("ConfigDemo/tuning.cdo.yml", &source, true, true);
    let value = edit(item(&stubs, "{item:}")).2.replace("$0", "|");
    let targets = fixture.op(ITEM, "mDisplayName", "set", &value);
    let target = edit(item(&targets, ITEM)).2;
    let value = value.replace('|', &format!("{target}|"));
    let members = fixture.op(ITEM, "mDisplayName", "set", &value);
    present(
        &members,
        &[
            "mDisplayName",
            "mStackSize",
            "GetItemName",
            "GetEnergyValue",
        ],
    );
    absent(&members, &["ForEachSlot", "Blueprinty", "Replicated"]);
    let completed = value.replace('|', edit(item(&members, "mDisplayName")).2);
    let document = Fixture::op_doc(ITEM, "mDisplayName", "set", &completed);
    let parsed: serde_yaml::Value = serde_yaml::from_str(&document).expect("valid quoted YAML");
    assert_eq!(
        parsed["patches"][0]["properties"][0]["value"].as_str(),
        Some(format!("{{item:{ITEM}:mDisplayName}}").as_str())
    );
}

#[test]
fn many_config_fields_do_not_hide_template_providers() {
    let fields = (0..250).map(|number| format!("  - {{ id: flag{number}, type: boolean, name: Flag, description: Flag, default: true }}\n")).collect::<String>();
    let source = format!("category: /Script/FactoryGame.FGCategory\nfields:\n{fields}");
    let fixture = Fixture::with_files(&[("ConfigDemo/many.config.yml", &source)]);
    let completions = fixture.op(ITEM, "mCanBeDiscarded", "set", "|");
    assert!(completions.incomplete);
    present(&completions, &["cfg:", "{item:}", "{class:}", "{asset:}"]);
}

#[test]
fn reflection_template_stubs_follow_runtime_operand_limits() {
    let fixture = Fixture::new();
    for key in ["min", "max"] {
        let source = format!("type: cdo\npatches:\n  - target: {ITEM}\n    properties:\n      - path: mEnergyValue\n        op: clamp\n        {key}: |\n");
        present(
            &fixture.run("ConfigDemo/tuning.cdo.yml", &source),
            &["{item:}"],
        );
    }
    for key in ["index", "with"] {
        let source = format!("type: cdo\npatches:\n  - target: {ITEM}\n    properties:\n      - path: mTags\n        op: insert\n        {key}: |\n");
        absent(
            &fixture.run("ConfigDemo/tuning.cdo.yml", &source),
            &["{item:}"],
        );
    }
    let nested = format!("type: cdo\npatches:\n  - target: {ITEM}\n    properties:\n      - path: mAmounts\n        value:\n          - Amount: |\n");
    present(
        &fixture.run("ConfigDemo/tuning.cdo.yml", &nested),
        &["{item:}"],
    );
    let filter = format!("type: cdo\npatches:\n  - ofClass: {ITEM}\n    where:\n      - path: mEnergyValue\n        equals: |\n    properties: []\n");
    absent(
        &fixture.run("ConfigDemo/tuning.cdo.yml", &filter),
        &["{item:}"],
    );
    absent(
        &fixture.op(ITEM, "mDisplayName", "set", "\"literal ite|\""),
        &["{item:}"],
    );
}

#[test]
fn unknown_slot_offers_everything() {
    let fixture = Fixture::new();
    let completions = fixture.op(ITEM, "mNoSuchProperty", "set", "cfg:|");
    present(
        &completions,
        &[
            "balance.speed",
            "balance.enabled",
            "balance.title",
            "balance.mode",
        ],
    );
}

#[test]
fn clamp_bounds_are_numeric_operands() {
    let fixture = Fixture::new();
    let completions = fixture.run(
        "ConfigDemo/tuning.cdo.yml",
        "type: cdo\npatches:\n  - target: /Script/FactoryGame.FGItemDescriptor\n    properties:\n      - path: mEnergyValue\n        op: clamp\n        min: cfg:|\n",
    );
    present(&completions, &["balance.speed"]);
    absent(&completions, &["balance.enabled"]);
}

#[test]
fn other_pack_ids_after_the_pack_name() {
    let fixture = Fixture::new();
    let completions = fixture.op(ITEM, "mEnergyValue", "multiply", "cfg:Other:|");
    assert_eq!(labels(&completions), ["shared.level"]);
    let completions = fixture.op(ITEM, "mEnergyValue", "multiply", "cfg:Other:sh|");
    assert_eq!(labels(&completions), ["shared.level"]);
}

#[test]
fn cfg_references_narrow_own_and_selected_pack_ids_by_typed_segment() {
    let fixture = Fixture::with_files(&[(
        "Other/extra.config.yml",
        "category: /Script/FactoryGame.FGCategory\nfields:\n  - { id: shared.speed, type: number, valueType: float, name: Speed, description: Speed, default: 1.0 }\n",
    )]);
    for (typed, expected) in [
        ("cfg:balance.sp", "balance.speed"),
        ("cfg:Other:shared.sp", "shared.speed"),
        ("cfg:OtH", "Other:"),
    ] {
        for value in [format!("{typed}|"), format!("\"{{{typed}|}}\"")] {
            let completions = fixture.op(ITEM, "mEnergyValue", "multiply", &value);
            assert_eq!(labels(&completions), [expected], "{value}");
            assert!(!completions.incomplete, "{value}");
            assert_eq!(edit(item(&completions, expected)).2, expected);
        }
    }
    for value in ["cfg:missing|", "\"{cfg:Other:missing|}\""] {
        let completions = fixture.op(ITEM, "mEnergyValue", "multiply", value);
        assert!(completions.items.is_empty());
        assert!(!completions.incomplete);
    }
}

#[test]
fn cfg_reference_limits_count_only_matching_ids_for_bare_and_template_forms() {
    for count in [200, 201] {
        let fields = (0..count)
            .map(|number| format!("  - {{ id: match{number:03}, type: number, valueType: float, name: Match, description: Match, default: 1.0 }}\n"))
            .collect::<String>();
        let source = format!("category: /Script/FactoryGame.FGCategory\nfields:\n{fields}");
        let fixture = Fixture::with_files(&[
            ("ConfigDemo/many.config.yml", &source),
            ("Other/many.config.yml", &source),
        ]);
        for prefix in ["cfg:", "cfg:Other:"] {
            for value in [
                format!("{prefix}match|"),
                format!("\"{{{prefix}match|}}\""),
                format!("\"{{{prefix}match|\""),
            ] {
                let completions = fixture.op(ITEM, "mEnergyValue", "multiply", &value);
                assert_eq!(completions.items.len(), 200, "{count}: {value}");
                assert_eq!(completions.incomplete, count > 200, "{count}: {value}");
                let narrowed = value.replace("match|", "match199|");
                let completions = fixture.op(ITEM, "mEnergyValue", "multiply", &narrowed);
                assert_eq!(labels(&completions), ["match199"], "{narrowed}");
                assert!(!completions.incomplete, "{narrowed}");
            }
        }
    }
}

#[test]
fn cfg_reference_limits_include_matching_pack_stubs() {
    for count in [199, 200] {
        let fields = (0..count)
            .map(|number| format!("  - {{ id: other{number:03}, type: number, valueType: float, name: Other, description: Other, default: 1.0 }}\n"))
            .collect::<String>();
        let source = format!("category: /Script/FactoryGame.FGCategory\nfields:\n{fields}");
        let fixture = Fixture::with_files(&[("ConfigDemo/many.config.yml", &source)]);
        for value in ["cfg:other|", "\"{cfg:other|}\"", "\"{cfg:other|\""] {
            let completions = fixture.op(ITEM, "mEnergyValue", "multiply", value);
            assert_eq!(completions.items.len(), 200, "{count}: {value}");
            assert_eq!(completions.incomplete, count == 200, "{count}: {value}");
            present(&completions, &["Other:"]);
        }
    }
}

#[test]
fn cfg_reference_limit_applies_after_slot_compatibility() {
    let fields = (0..201)
        .map(|number| format!("  - {{ id: match{number:03}, type: boolean, name: Match, description: Match, default: true }}\n"))
        .collect::<String>();
    let source = format!("category: /Script/FactoryGame.FGCategory\nfields:\n{fields}  - {{ id: match_numeric, type: number, valueType: float, name: Match, description: Match, default: 1.0 }}\n");
    let fixture = Fixture::with_files(&[("ConfigDemo/many.config.yml", &source)]);
    let completions = fixture.op(ITEM, "mEnergyValue", "multiply", "cfg:match|");
    assert_eq!(labels(&completions), ["match_numeric"]);
    assert!(!completions.incomplete);
    let completions = expr(&fixture, "\"balance.speed == cfg:match|\"");
    assert_eq!(labels(&completions), ["match_numeric"]);
    assert!(!completions.incomplete);
}

#[test]
fn settings_come_from_the_settings_index() {
    let fixture = Fixture::new();
    let completions = fixture.op(ITEM, "mEnergyValue", "multiply", "session:|");
    assert_eq!(labels(&completions), ["KMods.SFP.ProductionMultiplier"]);
    let completions = fixture.op(ITEM, "mCanBeDiscarded", "set", "ags:|");
    assert_eq!(labels(&completions), ["FG.NoPower"]);
    let completions = fixture.op(ITEM, "mEnergyValue", "multiply", "option:|");
    assert_eq!(labels(&completions), ["Gameplay.FOV"]);
    let fov = item(&completions, "Gameplay.FOV");
    assert_eq!(fov.detail.as_deref(), Some("option · int · Video → Camera"));
    let Some(Documentation::MarkupContent(doc)) = &fov.documentation else {
        panic!("markdown documentation expected");
    };
    assert!(doc.value.contains("Field of View"));
    assert!(doc.value.contains("Options menu: Video → Camera"));
}

#[test]
fn documentation_is_plain_text_without_markdown_support() {
    let fixture = Fixture::new();
    let completions = fixture.run_with(
        "ConfigDemo/tuning.cdo.yml",
        "type: cdo\npatches:\n  - target: /Script/FactoryGame.FGItemDescriptor\n    properties:\n      - path: mEnergyValue\n        value: cfg:|\n",
        false,
        false,
    );
    let speed = item(&completions, "balance.speed");
    assert!(matches!(
        speed.documentation,
        Some(Documentation::String(_))
    ));
}

// ---- templates ------------------------------------------------------------------------------

#[test]
fn braces_list_every_provider_without_touching_the_auto_closed_brace() {
    let fixture = Fixture::new();
    let completions = fixture.op(ITEM, "mDisplayName", "set", "\"{|}\"");
    assert_eq!(completions.items.len(), 14);
    present(
        &completions,
        &[
            "cfg",
            "session",
            "ags",
            "option",
            "class",
            "item",
            "resource",
            "building",
            "recipe",
            "schematic",
            "research",
            "category",
            "dataasset",
            "asset",
        ],
    );
    let cfg = item(&completions, "cfg");
    // `        value: "{` is 17 columns; the replaced span is empty and `}` stays untouched.
    assert_eq!(edit(cfg), (17, 17, "cfg:"));
    assert_eq!(command(cfg), Some("editor.action.triggerSuggest"));
    assert!(
        matches!(&cfg.documentation, Some(Documentation::MarkupContent(doc)) if doc.value.contains("Pack configuration"))
    );
    // A half typed provider is replaced.
    let completions = fixture.op(ITEM, "mDisplayName", "set", "\"{ite|}\"");
    let item_entry = item(&completions, "item");
    assert_eq!(edit(item_entry), (17, 20, "item:"));
}

#[test]
fn unquoted_brace_in_front_of_a_plain_scalar_is_not_a_provider_list() {
    let fixture = Fixture::new();
    let completions = fixture.op(ITEM, "mDisplayName", "set", "{|");
    present(&completions, &["key", "stringTable"]);
    absent(&completions, &["item", "cfg"]);
    let completions = fixture.op(ITEM, "mDisplayName", "set", "Hello {|");
    assert_eq!(completions.items.len(), 14);
}

#[test]
fn cfg_templates_complete_ids_and_close_the_token_only_when_needed() {
    let fixture = Fixture::new();
    let closed = fixture.op(ITEM, "mDisplayName", "set", "\"{cfg:|}\"");
    present(&closed, &["balance.speed", "balance.title", "Other:"]);
    assert_eq!(edit(item(&closed, "balance.speed")).2, "balance.speed");
    assert_eq!(
        command(item(&closed, "Other:")),
        Some("editor.action.triggerSuggest")
    );
    let open = fixture.op(ITEM, "mDisplayName", "set", "\"{cfg:|\"");
    assert_eq!(edit(item(&open, "balance.speed")).2, "balance.speed}");
    let packed = fixture.op(ITEM, "mDisplayName", "set", "\"{cfg:Other:|}\"");
    assert_eq!(labels(&packed), ["shared.level"]);
    let settings = fixture.op(ITEM, "mDisplayName", "set", "\"{session:|\"");
    assert_eq!(
        edit(item(&settings, "KMods.SFP.ProductionMultiplier")).2,
        "KMods.SFP.ProductionMultiplier}"
    );
}

#[test]
fn item_targets_list_concrete_descendants_and_generated_items() {
    let fixture = Fixture::with_files(&[(
        "ConfigDemo/super.item.yml",
        "type: item\nitems:\n  - id: SuperIngot\n",
    )]);
    let completions = fixture.op(ITEM, "mDisplayName", "set", "\"{item:|}\"");
    present(
        &completions,
        &[
            "SuperIngot",
            "/Script/FactoryGame.FGItemDescriptor",
            "/Game/Items/Desc_Test.Desc_Test_C",
        ],
    );
    absent(
        &completions,
        &[
            "/Script/FactoryGame.FGResourceDescriptor",
            "/Script/FactoryGame.FGRecipe",
        ],
    );
    let desc = item(&completions, "/Game/Items/Desc_Test.Desc_Test_C");
    assert_eq!(edit(desc).2, "/Game/Items/Desc_Test.Desc_Test_C:");
    assert_eq!(command(desc), Some("editor.action.triggerSuggest"));
    assert!(
        item(&completions, "SuperIngot").sort_text < desc.sort_text,
        "generated first"
    );
    assert!(!completions.incomplete);
}

#[test]
fn recipe_and_category_targets_follow_their_base_class() {
    let fixture = Fixture::new();
    let completions = fixture.op(ITEM, "mDisplayName", "set", "\"{recipe:|}\"");
    assert_eq!(labels(&completions), ["/Script/FactoryGame.FGRecipe"]);
    let completions = fixture.op(ITEM, "mDisplayName", "set", "\"{category:|}\"");
    present(
        &completions,
        &[
            "DemoCategory",
            "/Script/FactoryGame.FGCategory",
            "/Script/FactoryGame.FGItemCategory",
        ],
    );
}

#[test]
fn long_class_lists_are_cut_and_marked_incomplete() {
    let extra = (0..250)
        .map(|index| {
            let path = format!("/Game/Many/Desc_N{index}.Desc_N{index}_C");
            let mut record = support::class(&path, Some(ITEM), vec![], vec![]);
            record["kind"] = json!("blueprint");
            record
        })
        .collect();
    let fixture = Fixture::build(&[], extra);
    let completions = fixture.op(ITEM, "mDisplayName", "set", "\"{item:|}\"");
    assert_eq!(completions.items.len(), 200);
    assert!(completions.incomplete);
    let filtered = fixture.op(ITEM, "mDisplayName", "set", "\"{item:Desc_N24|}\"");
    assert!(!filtered.incomplete);
    assert!(
        filtered.items.len() >= 11 && filtered.items.len() < 200,
        "{}",
        filtered.items.len()
    );
}

#[test]
fn class_targets_list_the_catalog_including_function_libraries() {
    let fixture = Fixture::with_files(&[(
        "ConfigDemo/super.item.yml",
        "type: item\nitems:\n  - id: SuperIngot\n",
    )]);
    let completions = fixture.op(ITEM, "mDisplayName", "set", "\"{class:|}\"");
    present(
        &completions,
        &["/Script/Engine.KismetMathLibrary", "SuperIngot", ITEM],
    );
    let math = completions
        .items
        .iter()
        .find(|item| item.label == "/Script/Engine.KismetMathLibrary")
        .unwrap();
    assert_eq!(edit(math).2, "/Script/Engine.KismetMathLibrary:");
}

#[test]
fn data_asset_and_asset_targets_use_the_asset_index_and_generated_assets() {
    let fixture = Fixture::with_files(&[(
        "ConfigDemo/mine.dataasset.yml",
        "type: dataasset\nassets:\n  - id: MyAsset\n---\ntype: curve\ncurves:\n  - id: Curve\n",
    )]);
    let data = fixture.op(ITEM, "mDisplayName", "set", "\"{dataasset:|}\"");
    present(
        &data,
        &[
            "/Game/KMods/DA_Test.DA_Test",
            "/Game/KMods/DA_Recipe.DA_Recipe",
            "/KDataForge/GenAssets/ConfigDemo.Gen_ConfigDemo_MyAsset",
        ],
    );
    absent(&data, &["/Game/UI/T_Icon.T_Icon"]);
    let any = fixture.op(ITEM, "mDisplayName", "set", "\"{asset:|}\"");
    present(
        &any,
        &[
            "/Game/UI/T_Icon.T_Icon",
            "/Game/KMods/DA_Test.DA_Test",
            "/KDataForge/GenAssets/ConfigDemo.Gen_ConfigDemo_MyAsset",
            "/KDataForge/GenAssets/ConfigDemo.Gen_ConfigDemo_Curve",
        ],
    );
    absent(
        &data,
        &["/KDataForge/GenAssets/ConfigDemo.Gen_ConfigDemo_Curve"],
    );
    assert_eq!(
        command(item(&any, "/Game/UI/T_Icon.T_Icon")),
        Some("editor.action.triggerSuggest")
    );
}

#[test]
fn members_list_properties_with_reload_tags_then_callable_functions() {
    let fixture = Fixture::new();
    let target = format!("\"{{item:{ITEM}:|}}\"");
    let completions = fixture.op(ITEM, "mDisplayName", "set", &target);
    present(
        &completions,
        &[
            "mDisplayName",
            "mForm",
            "mStackSize",
            "GetItemName",
            "GetEnergyValue",
            "Total",
            "DescribeForm",
        ],
    );
    // Not callable: exec, blueprint event, net, out parameter, object return.
    absent(
        &completions,
        &["ForEachSlot", "Blueprinty", "Replicated", "Split", "Fetch"],
    );
    let form = item(&completions, "mForm");
    assert_eq!(form.kind, Some(CompletionItemKind::FIELD));
    assert!(
        form.detail.as_deref().unwrap().contains("cold"),
        "{:?}",
        form.detail
    );
    assert!(!item(&completions, "mStackSize")
        .detail
        .as_deref()
        .unwrap()
        .contains("cold"));
    let name = item(&completions, "GetItemName");
    assert_eq!(name.kind, Some(CompletionItemKind::METHOD));
    assert_eq!(
        name.detail.as_deref(),
        Some("GetItemName() -> FText (implicit class)")
    );
    assert_eq!(edit(name).2, "GetItemName");
    let energy = item(&completions, "GetEnergyValue");
    // Two input parameters: nothing is passed implicitly.
    assert_eq!(
        energy.detail.as_deref(),
        Some("GetEnergyValue(ItemClass: TSubclassOf<UObject>, Multiplier: float) -> float")
    );
    assert_eq!(edit(energy).2, "GetEnergyValue(");
    assert_eq!(command(energy), Some("editor.action.triggerParameterHints"));
}

#[test]
fn unclosed_members_get_their_closing_brace() {
    let fixture = Fixture::new();
    let target = format!("\"{{item:{ITEM}:|\"");
    let completions = fixture.op(ITEM, "mDisplayName", "set", &target);
    assert_eq!(edit(item(&completions, "mDisplayName")).2, "mDisplayName}");
    assert_eq!(edit(item(&completions, "GetItemName")).2, "GetItemName}");
}

#[test]
fn function_snippets_only_when_the_client_supports_them() {
    let fixture = Fixture::new();
    let source = format!("type: cdo\npatches:\n  - target: {ITEM}\n    properties:\n      - path: mDisplayName\n        value: \"{{item:{ITEM}:|\"\n");
    let plain = fixture.run_with("ConfigDemo/tuning.cdo.yml", &source, false, true);
    let energy = item(&plain, "GetEnergyValue");
    assert_eq!(energy.insert_text_format, None);
    assert_eq!(edit(energy).2, "GetEnergyValue(");
    let snippet = fixture.run_with("ConfigDemo/tuning.cdo.yml", &source, true, true);
    let energy = item(&snippet, "GetEnergyValue");
    assert_eq!(energy.insert_text_format, Some(InsertTextFormat::SNIPPET));
    assert_eq!(edit(energy).2, "GetEnergyValue($0)}");
    // Functions without arguments never need a snippet.
    assert_eq!(item(&snippet, "GetItemName").insert_text_format, None);
}

#[test]
fn members_matching_the_slot_sort_first_for_whole_token_values() {
    let fixture = Fixture::new();
    let whole = format!("\"{{item:{ITEM}:|}}\"");
    let completions = fixture.op(ITEM, "mEnergyValue", "set", &whole);
    let float = item(&completions, "mEnergyValue")
        .sort_text
        .clone()
        .unwrap();
    let text = item(&completions, "mDisplayName")
        .sort_text
        .clone()
        .unwrap();
    let call = item(&completions, "GetEnergyValue")
        .sort_text
        .clone()
        .unwrap();
    assert!(
        float.starts_with('0') && call.starts_with('0'),
        "{float} {call}"
    );
    assert!(text.starts_with('1'), "{text}");
    // Inside a longer string every member is a text piece: no ordering by slot.
    let partial = format!("\"x {{item:{ITEM}:|}}\"");
    let completions = fixture.op(ITEM, "mEnergyValue", "set", &partial);
    assert!(item(&completions, "mDisplayName")
        .sort_text
        .as_deref()
        .unwrap()
        .starts_with('0'));
}

#[test]
fn nested_templates_complete_the_innermost_token() {
    let fixture = Fixture::new();
    let source = format!("\"{{item:{ITEM}:GetEnergyValue({{cfg:|}})}}\"");
    let completions = fixture.op(ITEM, "mDisplayName", "set", &source);
    present(&completions, &["balance.speed"]);
    assert_eq!(edit(item(&completions, "balance.speed")).2, "balance.speed");
}

#[test]
fn arguments_offer_enum_names_and_booleans() {
    let fixture = Fixture::new();
    let first = format!("\"{{item:{ITEM}:DescribeForm(|\"");
    let completions = fixture.op(ITEM, "mDisplayName", "set", &first);
    assert_eq!(
        labels(&completions),
        ["RF_INVALID", "RF_SOLID", "RF_LIQUID", "RF_GAS"]
    );
    assert_eq!(
        completions.items[0].kind,
        Some(CompletionItemKind::ENUM_MEMBER)
    );
    let second = format!("\"{{item:{ITEM}:DescribeForm(RF_SOLID, |\"");
    let completions = fixture.op(ITEM, "mDisplayName", "set", &second);
    assert_eq!(labels(&completions), ["true", "false"]);
    // A partly typed enum name is replaced.
    let typed = format!("\"{{item:{ITEM}:DescribeForm(RF_S|\"");
    let completions = fixture.op(ITEM, "mDisplayName", "set", &typed);
    let solid = item(&completions, "RF_SOLID");
    assert_eq!(edit(solid).2, "RF_SOLID");
}

#[test]
fn class_arguments_offer_class_paths_below_the_meta_class() {
    let fixture = Fixture::new();
    let source = format!("\"{{item:{ITEM}:PickClass(|\"");
    let completions = fixture.op(ITEM, "mDisplayName", "set", &source);
    assert_eq!(
        labels(&completions),
        [ITEM, "/Game/Items/Desc_Test.Desc_Test_C"]
    );
    let typed = format!("\"{{item:{ITEM}:PickClass(Desc_T|\"");
    let completions = fixture.op(ITEM, "mDisplayName", "set", &typed);
    assert_eq!(labels(&completions), ["/Game/Items/Desc_Test.Desc_Test_C"]);
    // The second argument is an integer: nothing to suggest.
    let second = format!("\"{{item:{ITEM}:PickClass({ITEM}, |\"");
    assert!(fixture
        .op(ITEM, "mDisplayName", "set", &second)
        .items
        .is_empty());
}

// ---- path: ----------------------------------------------------------------------------------

fn path_doc(target: &str, extra: &str) -> String {
    format!("type: cdo\npatches:\n  - target: {target}\n{extra}    properties:\n      - path: |\n")
}

#[test]
fn path_lists_properties_with_kind_and_reload_tag() {
    let fixture = Fixture::new();
    let completions = fixture.run("ConfigDemo/tuning.cdo.yml", &path_doc(ITEM, ""));
    present(
        &completions,
        &["mStackSize", "mForm", "mDisplayName", "mAmounts"],
    );
    let form = item(&completions, "mForm");
    assert!(
        form.detail.as_deref().unwrap().contains("cold"),
        "{:?}",
        form.detail
    );
    assert!(
        matches!(&form.documentation, Some(Documentation::MarkupContent(doc)) if doc.value.contains("Reload: **cold**"))
    );
    assert!(!item(&completions, "mStackSize")
        .detail
        .as_deref()
        .unwrap()
        .contains("cold"));
    assert_eq!(form.kind, Some(CompletionItemKind::FIELD));
}

#[test]
fn path_skips_properties_other_entries_already_set() {
    let fixture = Fixture::new();
    let head = format!(
        "type: cdo\npatches:\n  - target: {ITEM}\n    properties:\n      - path: mstacksize\n        value: 1\n"
    );
    let tail = "      - path: mAmounts[0].Amount\n        value: 2\n      - path: mForm\n        value: RF_SOLID\n";
    let set = fixture.run(
        "ConfigDemo/tuning.cdo.yml",
        &format!("{head}      - path: |\n        value: 3\n{tail}"),
    );
    absent(&set, &["mStackSize"]);
    // Following operations cannot be overwritten by the current initial set.
    present(&set, &["mDisplayName", "mAmounts", "mForm"]);
    let nested = fixture.run(
        "ConfigDemo/tuning.cdo.yml",
        &format!("{head}      - path: mAmounts[0].|\n{tail}"),
    );
    assert_eq!(labels(&nested), ["ItemClass", "Amount"]);
    // Chained operations on an already patched path stay available.
    let chained = fixture.run(
        "ConfigDemo/tuning.cdo.yml",
        &format!("{head}      - op: multiply\n        path: |\n        value: 3\n{tail}"),
    );
    present(&chained, &["mStackSize", "mForm"]);
}

#[test]
fn path_marks_actor_properties_as_new_actors_unless_applied_to_spawned_actors() {
    let fixture = Fixture::new();
    let buildable = "/Script/FactoryGame.FGBuildable";
    let completions = fixture.run("ConfigDemo/tuning.cdo.yml", &path_doc(buildable, ""));
    assert!(item(&completions, "mHealth")
        .detail
        .as_deref()
        .unwrap()
        .contains("newActors"));
    let spawned = fixture.run(
        "ConfigDemo/tuning.cdo.yml",
        &path_doc(buildable, "    applyToSpawnedActors: true\n"),
    );
    assert!(!item(&spawned, "mHealth")
        .detail
        .as_deref()
        .unwrap()
        .contains("newActors"));
}

#[test]
fn path_follows_struct_fields_and_generated_parents() {
    let fixture = Fixture::with_files(&[(
        "ConfigDemo/chain.item.yml",
        "type: item\nitems:\n  - id: Base\n    parent: /Game/Items/Desc_Test.Desc_Test_C\n",
    )]);
    let nested = fixture.run(
        "ConfigDemo/tuning.cdo.yml",
        "type: cdo\npatches:\n  - target: /Script/FactoryGame.FGItemDescriptor\n    properties:\n      - path: mAmounts[0].|\n",
    );
    assert_eq!(labels(&nested), ["ItemClass", "Amount"]);
    let none = fixture.run(
        "ConfigDemo/tuning.cdo.yml",
        "type: cdo\npatches:\n  - target: /Script/FactoryGame.FGItemDescriptor\n    properties:\n      - path: mAmounts[|\n",
    );
    assert!(none.items.is_empty());
    // A generated class without an explicit parent chain resolves through its parent.
    let generated = fixture.run(
        "ConfigDemo/derived.item.yml",
        "type: item\nitems:\n  - id: Derived\n    parent: Base\n    properties:\n      - path: |\n",
    );
    present(&generated, &["mCustomValue", "mDisplayName"]);
    // Items without a parent use the item descriptor.
    let default = fixture.run(
        "ConfigDemo/plain.item.yml",
        "type: item\nitems:\n  - id: Plain\n    properties:\n      - path: |\n",
    );
    present(&default, &["mForm"]);
}

#[test]
fn recipe_documents_use_the_recipe_base_class() {
    let fixture = Fixture::new();
    let completions = fixture.run(
        "ConfigDemo/gated.recipe.yml",
        "type: recipe\nrecipes:\n  - id: R\n    properties:\n      - path: |\n",
    );
    present(&completions, &["mManufactoringDuration", "mDisplayName"]);
    assert!(item(&completions, "mDisplayName")
        .detail
        .as_deref()
        .unwrap()
        .contains("cold"));
}

// ---- categories and the legacy value lists ----------------------------------------------------

#[test]
fn config_categories_offer_generated_and_reflection_categories() {
    let fixture = Fixture::new();
    let completions = fixture.run(
        "ConfigDemo/new.config.yml",
        "type: config\ncategory: |\nfields: []\n",
    );
    present(
        &completions,
        &[
            "DemoCategory",
            "/Script/FactoryGame.FGCategory",
            "/Script/FactoryGame.FGItemCategory",
        ],
    );
    assert!(
        item(&completions, "DemoCategory").sort_text
            < item(&completions, "/Script/FactoryGame.FGCategory").sort_text
    );
    let field = fixture.run(
        "ConfigDemo/new.config.yml",
        "type: config\nfields:\n  - id: a\n    category: Dem|\n",
    );
    present(&field, &["DemoCategory"]);
}

#[test]
fn legacy_lists_are_kept() {
    let fixture = Fixture::new();
    let types = fixture.run("ConfigDemo/tuning.cdo.yml", "type: |\n");
    present(&types, &["category_reorder", "cdo", "config", "item"]);
    let ops = fixture.run(
        "ConfigDemo/tuning.cdo.yml",
        "type: cdo\npatches:\n  - target: /Script/A.B\n    properties:\n      - path: mX\n        op: |\n",
    );
    present(&ops, &["multiply", "clamp"]);
    assert_eq!(
        item(&ops, "multiply").detail.as_deref(),
        Some("multiply(value)")
    );
    let behavior = fixture.run(
        "ConfigDemo/tuning.cdo.yml",
        "type: cdo\nconditionBehavior: |\n",
    );
    assert_eq!(labels(&behavior), ["AND", "OR"]);
    let classes = fixture.run(
        "ConfigDemo/tuning.cdo.yml",
        "type: cdo\npatches:\n  - ofClass: |\n",
    );
    // Classes of the pack come first, then the reflection catalog in export order.
    assert_eq!(
        classes.items.first().map(|item| item.label.as_str()),
        Some("DemoCategory")
    );
    present(&classes, &["/Script/Engine.DataAsset", ITEM]);
}

#[test]
fn category_reorder_completes_contract_keys_and_subtypes() {
    let fixture = Fixture::new();
    let keys = fixture.run(
        "ConfigDemo/logistics.category_reorder.yml",
        "type: category_reorder\n|\n",
    );
    present(
        &keys,
        &[
            "subType",
            "category",
            "subCategory",
            "categoryPriority",
            "subCategoryPriority",
            "order",
        ],
    );
    let subtypes = fixture.run(
        "ConfigDemo/logistics.category_reorder.yml",
        "type: category_reorder\nsubType: |\ncategory: /Game/Cat.Cat_C\norder: [/Game/A.A_C]\n",
    );
    assert_eq!(
        labels(&subtypes),
        ["items", "recipe", "schematic", "settings"]
    );
    assert!(markdown_documentation(item(&subtypes, "items"))
        .value
        .contains("item and building descriptors"));
    assert!(markdown_documentation(item(&subtypes, "settings"))
        .value
        .contains("WidgetsToCreate[0]"));
}

// ---- conditions: config expressions -----------------------------------------------------------

fn expr(fixture: &Fixture, source: &str) -> Completions {
    fixture.run(
        "ConfigDemo/tuning.cdo.yml",
        &format!("type: cdo\nconditions:\n  config: {source}\npatches: []\n"),
    )
}

#[test]
fn expression_operands_list_ids_and_providers() {
    let fixture = Fixture::new();
    let completions = expr(&fixture, "\"|\"");
    present(
        &completions,
        &[
            "balance.speed",
            "balance.mode",
            "cfg:",
            "session:",
            "ags:",
            "option:",
        ],
    );
    absent(&completions, &["shared.level"]);
    let packed = expr(&fixture, "\"balance.enabled && cfg:Other:|\"");
    assert_eq!(labels(&packed), ["shared.level"]);
    // `session:` falls back to the advanced game settings, like at runtime.
    let settings = expr(&fixture, "\"session:|\"");
    assert_eq!(
        labels(&settings),
        ["FG.NoPower", "KMods.SFP.ProductionMultiplier"]
    );
}

#[test]
fn expression_cfg_references_filter_the_typed_id_and_pack_segment() {
    let fixture = Fixture::new();
    for (typed, expected) in [
        ("cfg:balance.sp", Some("balance.speed")),
        ("cfg:Other:shared.le", Some("shared.level")),
        ("cfg:OtH", Some("Other:")),
        ("cfg:missing", None),
        ("cfg:Other:missing", None),
    ] {
        for prefix in ["", "balance.enabled && "] {
            for source in [
                format!("\"{prefix}{typed}|\""),
                format!("\"{prefix}{typed}|"),
                format!("{prefix}{typed}|"),
            ] {
                let completions = expr(&fixture, &source);
                assert_eq!(
                    labels(&completions),
                    expected.into_iter().collect::<Vec<_>>(),
                    "{source}"
                );
                assert!(!completions.incomplete, "{source}");
                if let Some(expected) = expected {
                    let (start, end, text) = edit(item(&completions, expected));
                    assert_eq!(text, expected, "{source}");
                    assert_eq!(
                        end - start,
                        typed.rsplit(':').next().unwrap().len() as u32,
                        "{source}"
                    );
                }
            }
        }
    }
}

#[test]
fn expression_cfg_reference_limit_counts_only_matching_fields() {
    for count in [200, 201] {
        let fields = (0..count)
            .map(|number| format!("  - {{ id: match{number:03}, type: number, valueType: float, name: Match, description: Match, default: 1.0 }}\n"))
            .collect::<String>();
        let source = format!("category: /Script/FactoryGame.FGCategory\nfields:\n{fields}");
        let fixture = Fixture::with_files(&[
            ("ConfigDemo/many.config.yml", &source),
            ("Other/many.config.yml", &source),
        ]);
        for prefix in ["cfg:", "cfg:Other:"] {
            for source in [
                format!("\"{prefix}match|\""),
                format!("\"{prefix}match|"),
                format!("{prefix}match|"),
            ] {
                let completions = expr(&fixture, &source);
                assert_eq!(completions.items.len(), 200, "{count}: {source}");
                assert_eq!(completions.incomplete, count > 200, "{count}: {source}");
                let narrowed = source.replace("match|", "match199|");
                let completions = expr(&fixture, &narrowed);
                assert_eq!(labels(&completions), ["match199"], "{narrowed}");
                assert!(!completions.incomplete, "{narrowed}");
            }
        }
    }
}

#[test]
fn expression_cfg_pack_stub_survives_when_matching_ids_fill_the_limit() {
    for count in [199, 200] {
        let fields = (0..count)
            .map(|number| format!("  - {{ id: other{number:03}, type: number, valueType: float, name: Other, description: Other, default: 1.0 }}\n"))
            .collect::<String>();
        let source = format!("category: /Script/FactoryGame.FGCategory\nfields:\n{fields}");
        let fixture = Fixture::with_files(&[("ConfigDemo/many.config.yml", &source)]);
        for source in ["\"cfg:other|\"", "\"cfg:other|", "cfg:other|"] {
            let completions = expr(&fixture, source);
            assert_eq!(completions.items.len(), 200, "{count}: {source}");
            assert_eq!(completions.incomplete, count == 200, "{count}: {source}");
            present(&completions, &["Other:"]);
            assert_eq!(
                command(item(&completions, "Other:")),
                Some("editor.action.triggerSuggest")
            );
        }
    }
}

/// Regression: an id typed up to its dot (`balance.|`) was treated as an operator position.
#[test]
fn expression_ids_complete_after_a_trailing_dot() {
    let fixture = Fixture::new();
    for source in [
        "\"balance.|\"",
        "\"balance.|",
        "balance.|",
        "\"!balance.|\"",
    ] {
        let completions = expr(&fixture, source);
        present(
            &completions,
            &["balance.speed", "balance.stack", "balance.mode"],
        );
        absent(&completions, &["==", "&&", "shared.level"]);
        let (start, end, new_text) = edit(item(&completions, "balance.speed"));
        assert_eq!(new_text, "balance.speed", "{source}");
        assert_eq!(end - start, "balance.".len() as u32, "{source}");
    }
}

#[test]
fn expression_values_are_filtered_by_the_left_operand() {
    let fixture = Fixture::new();
    let numbers = expr(&fixture, "\"balance.speed > |\"");
    present(&numbers, &["balance.speed", "balance.stack"]);
    absent(
        &numbers,
        &["balance.mode", "balance.enabled", "balance.tiers"],
    );
    let options = expr(&fixture, "\"balance.mode == |\"");
    present(&options, &["easy", "hard"]);
    let easy = item(&options, "easy");
    assert_eq!(
        edit(easy).2,
        "'easy'",
        "single quotes inside a double-quoted scalar"
    );
    assert_eq!(easy.kind, Some(CompletionItemKind::ENUM_MEMBER));
    absent(&options, &["balance.speed"]);
    let booleans = expr(&fixture, "\"balance.enabled == |\"");
    present(&booleans, &["true", "false"]);
}

#[test]
fn expression_string_literals_and_lists_offer_option_values() {
    let fixture = Fixture::new();
    let inside = expr(&fixture, "\"balance.mode == 'ha|'\"");
    present(&inside, &["easy", "hard"]);
    let (start, end, text) = edit(item(&inside, "hard"));
    assert_eq!(text, "'hard'");
    assert!(end > start, "the whole literal is replaced");
    let contains = expr(&fixture, "\"balance.tiers contains |\"");
    present(&contains, &["1", "2", "3"]);
    assert_eq!(edit(item(&contains, "2")).2, "2");
    let list = expr(&fixture, "\"balance.mode in ['easy', |]\"");
    present(&list, &["easy", "hard"]);
}

#[test]
fn expression_quotes_follow_the_yaml_quote_style() {
    let fixture = Fixture::new();
    let plain = expr(&fixture, "balance.mode == |");
    assert_eq!(edit(item(&plain, "easy")).2, "'easy'");
    let single = expr(&fixture, "'balance.mode == |'");
    assert_eq!(edit(item(&single, "easy")).2, "\"easy\"");
}

#[test]
fn expression_operators_depend_on_the_left_type() {
    let fixture = Fixture::new();
    let number = expr(&fixture, "\"balance.speed |\"");
    present(
        &number,
        &["==", "!=", "<", "<=", ">", ">=", "in", "&&", "||"],
    );
    let boolean = expr(&fixture, "\"balance.enabled |\"");
    present(&boolean, &["==", "!=", "&&", "||"]);
    absent(&boolean, &["<", ">", "in", "contains"]);
    let list = expr(&fixture, "\"balance.tiers |\"");
    present(&list, &["contains", "&&"]);
    absent(&list, &["==", "<"]);
    let text = expr(&fixture, "\"balance.mode |\"");
    present(&text, &["==", "!=", "in"]);
    absent(&text, &["<"]);
}

#[test]
fn expression_list_items_in_block_form() {
    let fixture = Fixture::new();
    let completions = fixture.run(
        "ConfigDemo/tuning.cdo.yml",
        "type: cdo\nconditions:\n  config:\n    - balance.enabled\n    - \"balance.mode == |\"\npatches: []\n",
    );
    present(&completions, &["easy", "hard"]);
}

#[test]
fn expressions_complete_in_pack_manifests_and_element_conditions() {
    let fixture = Fixture::new();
    let manifest = fixture.run(
        "ConfigDemo/pack.yml",
        "ref: ConfigDemo\nversion: 1.0.0\nconditions:\n  config: \"balance.mode == |\"\n",
    );
    present(&manifest, &["easy", "hard"]);
    let element = fixture.run(
        "ConfigDemo/tuning.cdo.yml",
        "type: cdo\npatches:\n  - target: /Script/A.B\n    properties:\n      - path: mX\n        value: 1\n        conditions:\n          config: balance.speed > |\n",
    );
    present(&element, &["balance.speed", "balance.stack"]);
    absent(&element, &["balance.mode"]);
}

#[test]
fn nothing_is_offered_in_other_condition_keys() {
    let fixture = Fixture::new();
    let completions = fixture.run(
        "ConfigDemo/tuning.cdo.yml",
        "type: cdo\nconditions:\n  hasMod: cfg:|\npatches: []\n",
    );
    assert!(completions.items.is_empty());
}

// ---- config documents -------------------------------------------------------------------------

const FIELD: &str = "type: config\nfields:\n  - id: a.b\n    type: number\n";

fn config(fixture: &Fixture, source: &str) -> Completions {
    fixture.run("ConfigDemo/new.config.yml", source)
}

#[test]
fn config_values_depend_on_the_sibling_type() {
    let fixture = Fixture::new();
    let kinds = config(&fixture, "type: config\nfields:\n  - id: a\n    type: |\n");
    assert_eq!(labels(&kinds), ["text", "number", "boolean", "select"]);
    assert_eq!(
        markdown_documentation(item(&kinds, "number")).value,
        kdf_validation::field_docs::to_markdown(
            kdf_validation::field_docs::value_at("config", &["fields", "type"], "number")
                .expect("config type catalog documentation")
        )
    );
    let number = config(&fixture, &format!("{FIELD}    valueType: |\n"));
    assert_eq!(
        labels(&number),
        ["int32", "int8", "int16", "int64", "uint8", "float", "double"]
    );
    let text = config(
        &fixture,
        "type: config\nfields:\n  - id: a\n    type: text\n    valueType: |\n",
    );
    assert_eq!(labels(&text), ["string", "text", "name", "tag"]);
    let boolean = config(
        &fixture,
        "type: config\nfields:\n  - id: a\n    type: boolean\n    valueType: |\n",
    );
    assert_eq!(labels(&boolean), ["bool", "uint8"]);
    let apply = config(&fixture, &format!("{FIELD}    apply: |\n"));
    assert_eq!(labels(&apply), ["hot", "cold"]);
    assert_eq!(
        markdown_documentation(item(&apply, "hot")).value,
        kdf_validation::field_docs::to_markdown(
            kdf_validation::field_docs::value_at("config", &["fields", "apply"], "hot")
                .expect("config apply catalog documentation")
        )
    );
    assert_eq!(
        labels(&config(&fixture, &format!("{FIELD}    display: |\n"))),
        ["slider", "input"]
    );
    assert_eq!(
        labels(&config(&fixture, &format!("{FIELD}    multiple: |\n"))),
        ["true", "false"]
    );
}

#[test]
fn config_defaults_follow_the_field_type() {
    let fixture = Fixture::new();
    let boolean = config(
        &fixture,
        "type: config\nfields:\n  - id: a\n    type: boolean\n    default: |\n",
    );
    assert_eq!(labels(&boolean), ["true", "false"]);
    let select = config(
        &fixture,
        "type: config\nfields:\n  - id: a\n    type: select\n    options:\n      - value: easy\n        label: Easy\n      - { value: hard, label: Hard }\n    default: |\n",
    );
    assert_eq!(labels(&select), ["easy", "hard"]);
    let number = config(&fixture, &format!("{FIELD}    default: |\n"));
    assert!(number.items.is_empty());
}

#[test]
fn config_keys_depend_on_the_type_and_skip_existing_keys() {
    let fixture = Fixture::new();
    let number = config(&fixture, &format!("{FIELD}    |\n"));
    present(
        &number,
        &[
            "min",
            "max",
            "step",
            "decimals",
            "display",
            "name",
            "description",
            "default",
        ],
    );
    absent(&number, &["id", "type", "options", "multiline", "tagRoot"]);
    let min = item(&number, "min");
    assert_eq!(edit(min).2, "min: ");
    assert_eq!(command(min), Some("editor.action.triggerSuggest"));
    assert_eq!(
        markdown_documentation(min).value,
        kdf_validation::field_docs::to_markdown(
            kdf_validation::field_docs::field("config", &["fields", "min"])
                .expect("config key catalog documentation")
        )
    );
    let select = config(
        &fixture,
        "type: config\nfields:\n  - id: a\n    type: select\n    |\n",
    );
    present(&select, &["options", "minSelected", "maxSelected"]);
    absent(&select, &["min", "step"]);
    let text = config(
        &fixture,
        "type: config\nfields:\n  - id: a\n    type: text\n    |\n",
    );
    present(&text, &["multiline", "maxLength", "tagRoot"]);
    // A key that already has its colon is only renamed.
    let typed = config(&fixture, &format!("{FIELD}    ma|x: 4\n"));
    assert_eq!(edit(item(&typed, "max")).2, "max");
}

#[test]
fn config_document_and_nested_keys() {
    let fixture = Fixture::new();
    let document = config(&fixture, "type: config\n|\n");
    present(
        &document,
        &[
            "category",
            "conditions",
            "conditionBehavior",
            "debug",
            "fields",
        ],
    );
    absent(&document, &["type"]);
    let element = config(&fixture, "type: config\nfields:\n  - |\n");
    present(&element, &["id", "type", "name", "description", "default"]);
    let tooltip = config(&fixture, &format!("{FIELD}    tooltip:\n      |\n"));
    assert_eq!(labels(&tooltip), ["title", "text"]);
    let option = config(
        &fixture,
        "type: config\nfields:\n  - id: a\n    type: select\n    options:\n      - value: x\n        |\n",
    );
    assert_eq!(labels(&option), ["label"]);
    for (completions, key, path) in [
        (&document, "fields", vec!["fields"]),
        (&element, "name", vec!["fields", "name"]),
        (&tooltip, "title", vec!["fields", "tooltip", "title"]),
        (&option, "label", vec!["fields", "options", "label"]),
    ] {
        assert_eq!(
            markdown_documentation(item(completions, key)).value,
            kdf_validation::field_docs::to_markdown(
                kdf_validation::field_docs::field("config", &path)
                    .expect("nested config key catalog documentation")
            )
        );
    }
}

#[test]
fn config_ids_of_a_field_are_not_completed() {
    let fixture = Fixture::new();
    let completions = config(&fixture, "type: config\nfields:\n  - id: |\n");
    assert!(completions.items.is_empty());
}

#[test]
fn windows_line_endings_do_not_shift_columns() {
    let fixture = Fixture::new();
    let source = Fixture::op_doc(ITEM, "mEnergyValue", "multiply", "cfg:|").replace('\n', "\r\n");
    let completions = fixture.run("ConfigDemo/tuning.cdo.yml", &source);
    present(&completions, &["balance.speed"]);
    assert_eq!(edit(item(&completions, "balance.speed")).0, 19);
}

#[test]
fn config_ids_still_complete_without_reflection_data() {
    use crate::completion::complete;
    use crate::context::{resolve, testing::marked};
    use crate::semantic::Env;
    let fixture = Fixture::new();
    let missing = kdf_validation::ReflectionIndex::unavailable("no datapack");
    let path = fixture.root.join("ConfigDemo/tuning.cdo.yml");
    let run = |value: &str| {
        let (text, position) = marked(&Fixture::op_doc(ITEM, "mEnergyValue", "set", value));
        let env = Env {
            path: &path,
            text: &text,
            reflection: &missing,
            workspace: &fixture.workspace,
            caps: crate::state::ClientCaps {
                completion_markdown: true,
                ..crate::state::ClientCaps::default()
            },
        };
        complete(&env, &resolve(&path, &text, position))
    };
    // Without the property type a plain `set` has an unknown slot: every id is offered.
    present(
        &run("cfg:|"),
        &["balance.speed", "balance.enabled", "Other:"],
    );
    assert!(run("session:|").items.is_empty());
    assert!(run("\"{item:|}\"").items.is_empty());
    present(&run("\"{cfg:|}\""), &["balance.speed"]);
}

#[test]
fn operation_entries_offer_their_missing_keys() {
    let fixture = Fixture::new();
    let head = "type: cdo\npatches:\n  - target: /Script/A.B\n    properties:\n      - path: mX\n        op: add\n";
    let blank = fixture.run("ConfigDemo/tuning.cdo.yml", &format!("{head}        |\n"));
    assert_eq!(
        labels(&blank),
        ["value", "min", "max", "index", "with", "from"]
    );
    assert_eq!(edit(item(&blank, "value")).2, "value: ");
    assert_eq!(
        markdown_documentation(item(&blank, "value")).value,
        kdf_validation::field_docs::to_markdown(
            kdf_validation::field_docs::field("cdo", &["patches", "properties", "value"])
                .expect("operation key catalog documentation")
        )
    );
    let partial = fixture.run("ConfigDemo/tuning.cdo.yml", &format!("{head}        va|\n"));
    present(&partial, &["value"]);
    assert_eq!(edit(item(&partial, "value")).0, 8);
}

#[test]
fn localization_strings_complete_templates_too() {
    let fixture = Fixture::new();
    let completions = fixture.run(
        "ConfigDemo/texts.localization.yml",
        "type: localization\ntexts:\n  - key: A\n    source: \"Hello {cfg:|}\"\n",
    );
    present(&completions, &["balance.title"]);
}

#[test]
fn comments_and_block_scalars_have_no_completion() {
    let fixture = Fixture::new();
    let comment = fixture.op(ITEM, "mEnergyValue", "multiply", "1 # cfg:|");
    assert!(comment.items.is_empty());
}

// ---- lazy documentation, client commands, narrowing ---------------------------------------------

fn lazy_caps() -> ClientCaps {
    ClientCaps {
        completion_markdown: true,
        resolve_documentation: true,
        ..ClientCaps::default()
    }
}

fn card_of(fixture: &Fixture, item: &CompletionItem) -> String {
    let data = item
        .data
        .as_ref()
        .unwrap_or_else(|| panic!("{} has no data", item.label));
    crate::resolve::card(data, &fixture.reflection, &fixture.workspace)
        .unwrap_or_else(|| panic!("no card for {data}"))
}

#[test]
fn clients_that_resolve_get_data_and_the_card_is_built_on_demand() {
    let fixture = Fixture::new();
    // Config ids.
    let list = fixture.run_caps(
        "ConfigDemo/tuning.cdo.yml",
        &Fixture::op_doc(ITEM, "mEnergyValue", "multiply", "cfg:|"),
        lazy_caps(),
    );
    let speed = item(&list, "balance.speed");
    assert!(speed.documentation.is_none(), "no inline card");
    assert!(speed.detail.is_some(), "the short detail stays");
    assert!(card_of(&fixture, speed).contains("Machine speed"));
    // Template members: property and function cards.
    let members = fixture.run_caps(
        "ConfigDemo/tuning.cdo.yml",
        &Fixture::op_doc(ITEM, "mDisplayName", "set", &format!("\"{{item:{ITEM}:|\"")),
        lazy_caps(),
    );
    let form = item(&members, "mForm");
    assert!(form.documentation.is_none());
    assert!(card_of(&fixture, form).contains("Reload: **cold**"));
    let call = item(&members, "GetEnergyValue");
    assert!(call.documentation.is_none());
    assert!(card_of(&fixture, call).contains("GetEnergyValue("));
    // `path:` properties.
    let path = fixture.run_caps(
        "ConfigDemo/tuning.cdo.yml",
        &format!("type: cdo\npatches:\n  - target: {ITEM}\n    properties:\n      - path: |\n"),
        lazy_caps(),
    );
    let form = item(&path, "mForm");
    assert!(form.documentation.is_none());
    assert!(card_of(&fixture, form).contains("Reload: **cold**"));
    // The same items carry the card inline for a client without resolve support.
    let inline = fixture.run_caps(
        "ConfigDemo/tuning.cdo.yml",
        &format!("type: cdo\npatches:\n  - target: {ITEM}\n    properties:\n      - path: |\n"),
        ClientCaps {
            completion_markdown: true,
            ..ClientCaps::default()
        },
    );
    let inline_form = item(&inline, "mForm");
    assert!(inline_form.data.is_none());
    let Some(Documentation::MarkupContent(markup)) = &inline_form.documentation else {
        panic!("inline card expected");
    };
    assert_eq!(markup.value, card_of(&fixture, form), "same card both ways");
}

#[test]
fn resolve_data_of_a_vanished_subject_builds_no_card() {
    let fixture = Fixture::new();
    for data in [
        json!({"k": "cfg", "pack": "ConfigDemo", "id": "no.such.id"}),
        json!({"k": "fn", "class": ITEM, "name": "NoSuchFunction"}),
        json!({"k": "member", "class": ITEM, "name": "mNoSuchThing"}),
        json!({"k": "unknown"}),
        json!({"label": "not ours"}),
    ] {
        assert!(
            crate::resolve::card(&data, &fixture.reflection, &fixture.workspace).is_none(),
            "{data}"
        );
    }
}

#[test]
fn hosts_can_switch_the_editor_commands_off() {
    let fixture = Fixture::new();
    let source = Fixture::op_doc(ITEM, "mEnergyValue", "multiply", "cfg:|");
    let on = fixture.run_caps("ConfigDemo/tuning.cdo.yml", &source, ClientCaps::default());
    assert_eq!(
        command(item(&on, "Other:")),
        Some("editor.action.triggerSuggest")
    );
    let off = fixture.run_caps(
        "ConfigDemo/tuning.cdo.yml",
        &source,
        ClientCaps {
            client_commands: false,
            ..ClientCaps::default()
        },
    );
    assert!(off.items.iter().all(|item| item.command.is_none()));
    assert!(!off.items.is_empty());
}

#[test]
fn ascii_matching_helpers() {
    use crate::completion::{contains_ci, matches_typed};
    assert!(contains_ci(
        "/Game/Items/Desc_Test.Desc_Test_C",
        "desc_test"
    ));
    assert!(contains_ci("anything", ""));
    assert!(!contains_ci("short", "much longer needle"));
    assert!(!contains_ci("abc", "abd"));
    // Letters in order, any case: what an editor's fuzzy filter keeps.
    assert!(matches_typed("mDisplayName", "mdn"));
    assert!(matches_typed("mDisplayName", "mDisp"));
    assert!(matches_typed("mDisplayName", ""));
    assert!(!matches_typed("mDisplayName", "mxyz"));
    assert!(!matches_typed("m", "mm"));
}

#[test]
fn long_property_lists_are_narrowed_by_what_is_typed_before_the_cap() {
    let properties: Vec<serde_json::Value> = (0..400)
        .map(|n| support::prop(&format!("mBulk{n:03}"), "IntProperty", "int32"))
        .collect();
    let fixture = Fixture::build(
        &[],
        vec![support::class(
            "/Script/Test.Bulk",
            Some("/Script/CoreUObject.Object"),
            properties,
            vec![],
        )],
    );
    let source = |typed: &str| {
        format!(
            "type: cdo\npatches:\n  - target: /Script/Test.Bulk\n    properties:\n      - path: {typed}|\n"
        )
    };
    let all = fixture.run("ConfigDemo/tuning.cdo.yml", &source(""));
    assert_eq!(all.items.len(), crate::completion::LIMIT);
    assert!(all.incomplete);
    let narrowed = fixture.run("ConfigDemo/tuning.cdo.yml", &source("mBulk399"));
    assert!(labels(&narrowed).contains(&"mBulk399"));
    assert!(!narrowed.incomplete);
    // Members of a template follow the same rule.
    let members = |typed: &str| {
        Fixture::op_doc(
            "/Script/Test.Bulk",
            "mA",
            "set",
            &format!("\"{{class:/Script/Test.Bulk:{typed}|\""),
        )
    };
    let all = fixture.run("ConfigDemo/tuning.cdo.yml", &members(""));
    assert_eq!(all.items.len(), crate::completion::LIMIT);
    assert!(all.incomplete);
    let narrowed = fixture.run("ConfigDemo/tuning.cdo.yml", &members("mBulk123"));
    assert!(labels(&narrowed).contains(&"mBulk123"));
}
