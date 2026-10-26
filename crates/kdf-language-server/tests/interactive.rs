//! Real stdio tests for interactive LSP client behavior.
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
use url::Url;

const ITEM: &str = "/Game/Items/Desc_Iron.Desc_Iron_C";
const RECIPE: &str = "/Game/Recipes/Recipe_Iron.Recipe_Iron_C";

#[path = "support/inline_values.rs"]
mod inline_values;

struct Session {
    child: Child,
    stdin: ChildStdin,
    responses: Receiver<Value>,
    root: tempfile::TempDir,
    uri: Url,
    id: u64,
    version: i32,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Session {
    fn new() -> Self {
        Self::with_classes(Vec::new())
    }

    fn with_classes(extra: Vec<Value>) -> Self {
        Self::with_recipe_fields(extra, Vec::new())
    }

    fn with_recipe_fields(extra: Vec<Value>, recipe_fields: Vec<Value>) -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("pack.yml"),
            "ref: Interactive\nname: Interactive\nversion: 1.0.0\n",
        )
        .unwrap();
        let archive = root.path().join("game.kdfdata");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(
            serde_json::to_string(
                &json!({"schemaVersion":1,"counts":{"classes":4 + extra.len()},"failedLoads":[]}),
            )
            .unwrap()
            .as_bytes(),
        )
        .unwrap();
        zip.start_file("classes.ndjson", options).unwrap();
        for record in [
            json!({"path":"/Script/FactoryGame.FGItemDescriptor","name":"FGItemDescriptor","kind":"native","properties":[]}),
            json!({"path":ITEM,"name":"Iron Ingot","kind":"blueprint","super":"/Script/FactoryGame.FGItemDescriptor","source":{"ref":"FactoryGame"},"metadata":{"ToolTip":"Iron item from reflected metadata."},"properties":[]}),
            json!({"path":RECIPE,"name":"Iron Recipe","kind":"blueprint","super":"/Script/FactoryGame.FGRecipe","source":{"ref":"FactoryGame"},"properties":[
                {"name":"mProduct","cppType":"TArray<FItemAmount>","kind":"ArrayProperty","ownerPath":"/Script/FactoryGame.FGRecipe","metadata":{"ToolTip":"Items this recipe produces."},"kdf":{"ops":["set","append","clear"],"valueForm":"sequence"}},
                {"name":"mManufacturingDuration","cppType":"float","kind":"FloatProperty","metadata":{"Units":"s"},"kdf":{"ops":["set","add","multiply","divide"]}},
                {"name":"mEnabled","cppType":"bool","kind":"BoolProperty","kdf":{"ops":["set"]}}
            ]}),
            json!({"path":"/Script/FactoryGame.FGRecipe","name":"FGRecipe","kind":"native","properties":recipe_fields}),
        ].into_iter().chain(extra) {
            writeln!(zip, "{record}").unwrap();
        }
        zip.finish().unwrap();
        fs::write(
            root.path().join(".kdf-editor.toml"),
            format!(
                "[lint]\ntrigger = \"manual\"\n[datapack]\npath = {:?}\n",
                archive.to_string_lossy()
            ),
        )
        .unwrap();
        let uri = Url::from_file_path(root.path().join("interactive.cdo.yml")).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_kdf-language-server"));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .env_remove("KDF_EFFECTIVE_DATAPACK_PATH")
            .env_remove("KDF_USER_SETTINGS_PATH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, responses) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut length = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("Content-Length: ") {
                        length = value.trim().parse::<usize>().ok();
                    }
                }
                let Some(length) = length else {
                    return;
                };
                let mut bytes = vec![0; length];
                if reader.read_exact(&mut bytes).is_err() {
                    return;
                }
                if let Ok(message) = serde_json::from_slice(&bytes) {
                    if tx.send(message).is_err() {
                        return;
                    }
                }
            }
        });
        let mut session = Self {
            child,
            stdin,
            responses,
            root,
            uri,
            id: 0,
            version: 0,
        };
        let root_uri = Url::from_directory_path(session.root.path()).unwrap();
        session.request(
            "initialize",
            json!({"processId":null,"rootUri":root_uri,"capabilities":{"textDocument":{
                "hover":{"contentFormat":["markdown","plaintext"]},
                "completion":{"completionItem":{"documentationFormat":["markdown","plaintext"]}}
            }}}),
        );
        session.notify("initialized", json!({}));
        session
    }

    fn write(&mut self, message: Value) {
        let bytes = serde_json::to_vec(&message).unwrap();
        write!(self.stdin, "Content-Length: {}\r\n\r\n", bytes.len()).unwrap();
        self.stdin.write_all(&bytes).unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.write(json!({"jsonrpc":"2.0","method":method,"params":params}));
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        self.write(json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params}));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let message = self
                .responses
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("LSP response timed out");
            if message["id"] == self.id {
                assert!(message.get("error").is_none(), "{message}");
                return message["result"].clone();
            }
        }
    }

    fn edit(&mut self, text: &str) {
        self.version += 1;
        if self.version == 1 {
            self.notify("textDocument/didOpen", json!({"textDocument":{"uri":self.uri,"languageId":"yaml","version":self.version,"text":text}}));
        } else {
            self.notify("textDocument/didChange", json!({"textDocument":{"uri":self.uri,"version":self.version},"contentChanges":[{"text":text}]}));
        }
    }

    fn at(&mut self, method: &str, text: &str, line: u32, character: u32) -> Value {
        self.edit(text);
        let result = self.request(
            method,
            json!({"textDocument":{"uri":self.uri},"position":{"line":line,"character":character}}),
        );
        // Legacy assertions concern items. Dedicated stdio assertions retain the
        // actual CompletionList and never flatten a capped response.
        if method == "textDocument/completion" && result["isIncomplete"] == false {
            result["items"].clone()
        } else {
            result
        }
    }
}

#[test]
fn item_class_completion_replaces_full_path_and_filters_to_items() {
    let mut session = Session::new();
    let items = session.at(
        "textDocument/completion",
        "    - ItemClass: /Game/Items/Desc_",
        0,
        32,
    );
    let items = items
        .as_array()
        .expect("ItemClass should offer item descriptors");
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["insertText"], ITEM);
    assert_eq!(items[0]["label"], "Iron Ingot");
    assert_eq!(items[0]["labelDetails"]["description"], "FactoryGame");
    assert_eq!(items[0]["detail"], ITEM);
    assert_eq!(items[0]["textEdit"]["range"]["start"]["character"], 17);
    assert_eq!(items[0]["textEdit"]["newText"], ITEM);
    assert!(items[0]["documentation"].to_string().contains("Iron Ingot"));
}

#[test]
fn property_completion_and_hover_use_active_target_and_reflection_details() {
    let mut session = Session::new();
    let source = format!("type: cdo\npatches:\n  - target: {ITEM}\n    properties:\n      - path: mStackSize\n        value: 100\n  - target: {RECIPE}\n    properties:\n      - path: mP");
    let result = session.at("textDocument/completion", &source, 8, 16);
    let items = result
        .as_array()
        .expect("property completion should offer reflected fields");
    let product = items
        .iter()
        .find(|item| item["label"] == "mProduct")
        .expect("active recipe mProduct missing");
    assert!(product["detail"]
        .as_str()
        .unwrap()
        .contains("TArray<FItemAmount>"));
    assert!(product["documentation"].to_string().contains("append"));
    assert_eq!(
        product["labelDetails"]["description"],
        "TArray<FItemAmount>"
    );
    assert!(product["documentation"]
        .to_string()
        .contains("YAML value example"));
    let hover_source = source.replace("path: mP", "path: mProduct");
    let hover = session.at("textDocument/hover", &hover_source, 8, 17);
    assert!(
        hover["contents"]
            .to_string()
            .contains("TArray<FItemAmount>"),
        "{hover}"
    );
    assert!(
        hover["contents"]
            .to_string()
            .contains("Items this recipe produces"),
        "{hover}"
    );
}

#[test]
fn tag_container_selector_completion_hover_and_operations_match_runtime() {
    let mut session = Session::new();
    let source = format!("type: cdo\npatches:\n  - target: {ITEM}\n    properties:\n      - path: '@tag'\n        op: append\n        value: []\n");
    let result = session.at("textDocument/completion", &source, 4, 17);
    let selector = result
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["label"] == "@tagContainers")
        .expect("selector completion missing");
    assert!(selector["documentation"].to_string().contains("inherited"));
    let plain = source.replace("path: '@tag'", "path: ");
    let suggestions = session.at("textDocument/completion", &plain, 4, 14);
    let quoted = suggestions
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["label"] == "@tagContainers")
        .expect("empty path should offer selector");
    assert_eq!(quoted["textEdit"]["newText"], "'@tagContainers'");
    let complete = source.replace("'@tag'", "'@tagContainers'");
    let hover = session.at("textDocument/hover", &complete, 4, 18);
    assert!(
        hover["contents"].to_string().contains("late-loaded"),
        "{hover}"
    );
    let filtered = session.at("textDocument/completion", &complete, 5, 18);
    assert_eq!(filtered.as_array().unwrap().len(), 1, "{filtered}");
    assert_eq!(filtered[0]["label"], "append");
    let operations = session.at(
        "textDocument/completion",
        &complete.replace("op: append", "op: "),
        5,
        12,
    );
    let labels: Vec<_> = operations
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["label"].as_str())
        .collect();
    assert_eq!(labels.len(), 5, "{labels:?}");
    for name in ["set", "append", "prepend", "remove", "clear"] {
        assert!(labels.contains(&name));
    }
}

#[test]
fn class_hover_formats_existing_metadata_without_loading_other_class_details() {
    let mut session = Session::new();
    let hover = session.at("textDocument/hover", &format!("target: {ITEM}"), 0, 20);
    assert_eq!(hover["contents"]["kind"], "markdown");
    let contents = hover["contents"]["value"].as_str().unwrap();
    assert!(contents.starts_with("### Iron Ingot"));
    assert!(contents.contains(ITEM));
    assert!(contents.contains("**Source** FactoryGame"));
    assert!(contents.contains("Iron item from reflected metadata."));
}

#[test]
fn warm_interactive_requests_finish_below_two_hundred_milliseconds() {
    let mut session = Session::new();
    session.request("kdf/classes", json!({}));
    for _ in 0..20 {
        let start = Instant::now();
        let result = session.at("textDocument/completion", "- ItemClass: /", 0, 14);
        assert!(
            result.as_array().is_some_and(|items| !items.is_empty()),
            "{result}"
        );
        assert!(
            start.elapsed() < Duration::from_millis(200),
            "warm completion took {:?}",
            start.elapsed()
        );
    }
}

#[test]
fn target_lists_offer_common_properties_and_record_existing_uses() {
    let other = "/Game/Recipes/Recipe_Alt.Recipe_Alt_C";
    let mut session = Session::with_classes(vec![
        json!({"path":other,"name":"Alt Recipe","super":"/Script/FactoryGame.FGRecipe","properties":[
            {"name":"mProduct","cppType":"TArray<FItemAmount>","kind":"ArrayProperty","ownerPath":"/Script/FactoryGame.FGRecipe","kdf":{"ops":["set","append"]}},
            {"name":"mAltOnly","cppType":"int32","kind":"IntProperty"}
        ]}),
    ]);
    let source = format!("type: cdo\npatches:\n  - target:\n      - {RECIPE}\n      - {other}\n    properties:\n      - path: mProduct\n        value: []\n      - path: m");
    let items = session.at("textDocument/completion", &source, 8, 15);
    let items = items
        .as_array()
        .expect("list target context should complete properties");
    assert_eq!(
        items.len(),
        1,
        "Only fields usable on every selected target are safe: {items:?}"
    );
    assert_eq!(items[0]["label"], "mProduct");
    assert!(items[0]["documentation"]["value"]
        .as_str()
        .unwrap()
        .contains("**Used on line** 7"));
    assert_eq!(items[0]["sortText"], "0:00000:mProduct");
    let docs = items[0]["documentation"]["value"].as_str().unwrap();
    assert!(
        !docs.contains("`clear`"),
        "Only operations supported on every target may be offered"
    );
    // Verify the sibling operation list separately from the field documentation.
    let operation_source = format!("type: cdo\npatches:\n  - target: ['{RECIPE}', '{other}']\n    properties:\n      - path: mProduct\n        op: ");
    let operations = session.at("textDocument/completion", &operation_source, 5, 12);
    assert_eq!(
        operations
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["label"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["set", "append"]
    );

    let inline = source.replace(
        &format!("target:\n      - {RECIPE}\n      - {other}"),
        &format!("target: ['{RECIPE}', '{other}']"),
    );
    let inline_items = session.at("textDocument/completion", &inline, 6, 15);
    assert_eq!(inline_items[0]["label"], "mProduct");
    let class_items = session.at(
        "textDocument/completion",
        "type: cdo\npatches:\n  - target:\n      - /Game/Recipes/Recipe_",
        3,
        28,
    );
    assert!(class_items
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["insertText"] == RECIPE));
    assert_eq!(class_items[0]["textEdit"]["range"]["start"]["character"], 8);
}

#[test]
fn current_property_values_and_units_come_from_reflection() {
    let mut session = Session::new();
    let source = format!("type: cdo\npatches:\n  - target: {RECIPE}\n    properties:\n      - path: mEnabled\n        value: ");
    let result = session.at("textDocument/completion", &source, 5, 15);
    let labels = result
        .as_array()
        .expect("boolean value suggestions")
        .iter()
        .map(|item| item["label"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(labels, ["true", "false"]);
    let source = format!(
        "type: cdo\npatches:\n  - target: {RECIPE}\n    properties:\n      - path: mManufact"
    );
    let result = session.at("textDocument/completion", &source, 4, 21);
    assert!(result[0]["documentation"]["value"]
        .as_str()
        .unwrap()
        .contains("**Units** `s`"));
}

#[test]
fn recipe_filter_selectors_use_recipe_fields_not_machine_or_item_fields() {
    const MACHINE: &str = "/Game/Buildable/Blender/Build_Blender.Build_Blender_C";
    let mut session = Session::with_recipe_fields(
        vec![json!({"path":MACHINE,"name":"Blender","properties":[
            {"name":"mPowerConsumption","cppType":"float","kdf":{"ops":["set"]}}
        ]})],
        vec![
            json!({"name":"mProduct","cppType":"TArray<FItemAmount>","kind":"ArrayProperty","ownerPath":"/Script/FactoryGame.FGRecipe","metadata":{"ToolTip":"Items this recipe produces."},"kdf":{"ops":["set","append","clear"]}}),
            json!({"name":"mManufacturingDuration","cppType":"float","kind":"FloatProperty","ownerPath":"/Script/FactoryGame.FGRecipe","metadata":{"Units":"s"},"kdf":{"ops":["set","add","multiply","divide"]}}),
            json!({"name":"mEnabled","cppType":"bool","kind":"BoolProperty","ownerPath":"/Script/FactoryGame.FGRecipe","kdf":{"ops":["set"]}}),
        ],
    );
    for key in ["producedIn", "ingredient", "product"] {
        for selector in [
            format!("{key}: {MACHINE}"),
            format!("{key}: [{MACHINE}]"),
            format!("{key}:\n      - {MACHINE}"),
        ] {
            let source =
                format!("type: cdo\npatches:\n  - {selector}\n    properties:\n      - path: m");
            let position = source.lines().count() as u32 - 1;
            let items = session.at("textDocument/completion", &source, position, 15);
            let items = items.as_array().expect("recipe scope property list");
            let duration = items
                .iter()
                .find(|item| item["label"] == "mManufacturingDuration")
                .expect("recipe shortcut must complete FGRecipe fields");
            assert_eq!(duration["detail"], "float");
            assert!(duration["documentation"]["value"]
                .as_str()
                .unwrap()
                .contains("multiply"));
            assert!(!items
                .iter()
                .any(|item| item["label"] == "mPowerConsumption"));
        }
    }
    let source = format!("type: cdo\npatches:\n  - producedIn: {MACHINE}\n    properties:\n      - path: mManufacturingDuration\n        op: ");
    let ops = session.at("textDocument/completion", &source, 5, 12);
    let ops = ops
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["label"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ops, ["set", "add", "multiply", "divide"]);
    let hover = session.at("textDocument/hover", &source, 4, 20);
    assert!(
        hover["contents"]["value"]
            .as_str()
            .is_some_and(|value| value.contains("**Type** `float`")),
        "{hover}"
    );
    let from = source.replace("op: ", "from: m");
    let fields = session.at("textDocument/completion", &from, 5, 15);
    assert!(
        fields.as_array().is_some_and(|fields| fields
            .iter()
            .any(|field| field["label"] == "mManufacturingDuration")),
        "{fields}"
    );
}

#[test]
fn property_owner_inference_requires_unique_exported_signatures_and_never_overrides_unknown_selector(
) {
    let first = "/Script/Test.FirstOwner";
    let second = "/Script/Test.SecondOwner";
    let mut session = Session::with_classes(vec![
        json!({"path":first,"properties":[
            {"name":"mUnique","ownerPath":first,"cppType":"float","kdf":{"ops":["set","multiply"]}},
            {"name":"mAvailable","ownerPath":first,"cppType":"bool","kdf":{"ops":["set"]}},
            {"name":"mShared","ownerPath":first,"cppType":"int32"}
        ]}),
        json!({"path":second,"properties":[{"name":"mShared","ownerPath":second,"cppType":"float"}]}),
    ]);
    let source = "type: cdo\npatches:\n  - properties:\n      - path: mUnique\n        op: ";
    let ops = session.at("textDocument/completion", source, 4, 12);
    let labels = ops
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["label"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        ["set", "multiply"],
        "A unique reflected owner supplies actual operations"
    );
    let fields = session.at(
        "textDocument/completion",
        &format!("{source}set\n      - path: mA"),
        5,
        16,
    );
    assert_eq!(fields[0]["label"], "mAvailable");
    let ambiguous = source.replace("mUnique", "mShared");
    let fields = session.at(
        "textDocument/completion",
        &format!("{ambiguous}set\n      - path: mA"),
        5,
        16,
    );
    assert!(
        fields.as_array().is_some_and(Vec::is_empty),
        "Ambiguous owner must stay unavailable: {fields}"
    );
    let explicit_unknown = source.replace(
        "- properties:",
        "- target: /Game/Missing.Missing_C\n    properties:",
    );
    let fields = session.at(
        "textDocument/completion",
        &format!("{explicit_unknown}set\n      - path: mA"),
        6,
        16,
    );
    assert!(
        fields.as_array().is_some_and(Vec::is_empty),
        "Unknown explicit selector must not be replaced: {fields}"
    );
    assert_eq!(
        session.request("kdf/inferPropertyOwner", json!({"properties":["mUnique"]}))["path"],
        first
    );
    assert!(
        session.request("kdf/inferPropertyOwner", json!({"properties":["mShared"]}))["path"]
            .is_null()
    );
}

#[test]
fn selector_aliases_explicit_scopes_and_document_boundaries_preserve_proven_context() {
    let scope = "/Script/Test.CustomScope";
    let mut session = Session::with_classes(vec![json!({"path":scope,"properties":[
        {"name":"mCustom","ownerPath":scope,"cppType":"bool","kdf":{"ops":["set"]}}
    ]})]);
    for key in ["target", "ofClass", "allAssetsOfClass"] {
        let filter = if key == "ofClass" {
            "    producedIn: /Game/Build.Build_C\n"
        } else {
            ""
        };
        let source = format!("type: cdo\nvariables:\n  scope: &known {scope}\npatches:\n  - {key}: *known\n{filter}    properties:\n      - path: m");
        let fields = session.at(
            "textDocument/completion",
            &source,
            source.lines().count() as u32 - 1,
            15,
        );
        assert_eq!(fields[0]["label"], "mCustom", "{key}: {fields}");
        assert!(!fields
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field["label"] == "mProduct"));
    }
    let source = format!("type: cdo\nvariables:\n  scope: &known {scope}\n---\ntype: cdo\npatches:\n  - target: *known\n    properties:\n      - path: m");
    let fields = session.at("textDocument/completion", &source, 8, 15);
    assert!(
        fields.as_array().is_some_and(Vec::is_empty),
        "Alias in another YAML document stays unavailable: {fields}"
    );
}

#[test]
fn ingredient_and_product_selector_values_offer_reflected_item_classes() {
    let mut session = Session::new();
    for key in ["ingredient", "product"] {
        let line = format!("- {key}: /");
        let items = session.at("textDocument/completion", &line, 0, line.len() as u32);
        let items = items.as_array().expect("item selector completion");
        assert!(items.iter().any(|item| item["detail"] == ITEM));
        assert!(
            items.iter().all(|item| item["detail"] == ITEM
                || item["detail"] == "/Script/FactoryGame.FGItemDescriptor"),
            "Selector values are items, never recipe or machine CDOs"
        );
        for suffix in ["", "Iron"] {
            let source = format!("type: cdo\npatches:\n  - {key}:\n      - {suffix}");
            let items = session.at(
                "textDocument/completion",
                &source,
                3,
                source.lines().last().unwrap().len() as u32,
            );
            let items = items
                .as_array()
                .expect("block-list selector should keep its semantic item context");
            assert!(items.iter().any(|item| item["detail"] == ITEM));
            assert!(!items.iter().any(|item| item["detail"] == RECIPE));
        }
    }
}

#[test]
fn indexed_building_descriptor_hint_is_shared_by_class_catalog_and_class_info() {
    let building = "/Game/Test.Build_Assembler_C";
    let descriptor = "/Game/Test.Desc_Assembler_C";
    let mut session = Session::with_classes(vec![
        json!({"path":"/Script/FactoryGame.FGBuildable","properties":[]}),
        json!({"path":"/Script/FactoryGame.FGBuildingDescriptor","super":"/Script/FactoryGame.FGItemDescriptor","properties":[]}),
        json!({"path":building,"super":"/Script/FactoryGame.FGBuildable","properties":[]}),
        json!({"path":descriptor,"super":"/Script/FactoryGame.FGBuildingDescriptor","properties":[{"name":"mBuildableClass","default":format!("/Script/Engine.BlueprintGeneratedClass'{building}'")}]}),
    ]);
    let classes = session.request("kdf/classes", json!({}));
    let class = classes["classes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|class| class["path"] == building)
        .unwrap();
    assert_eq!(class["iconDescriptorPath"], descriptor);
    let detail = session.request("kdf/classInfo", json!({"path":building}));
    assert_eq!(detail["class"]["iconDescriptorPath"], descriptor);
    assert!(
        detail["class"]["properties"].as_array().unwrap().is_empty(),
        "Do not invent building texture/default fields"
    );
}

#[test]
fn capped_class_results_request_fresh_data_when_typing_refines_search() {
    let classes = (0..251).map(|n| json!({"path":format!("/Game/Items/Desc_{n:03}.Desc_{n:03}_C"),"name":format!("Item {n:03}"),"super":"/Script/FactoryGame.FGItemDescriptor","properties":[]})).collect();
    let mut session = Session::with_classes(classes);
    let result = session.at("textDocument/completion", "- ItemClass: /", 0, 14);
    assert_eq!(
        result["isIncomplete"], true,
        "200 results require native-client refresh"
    );
    assert_eq!(result["items"].as_array().unwrap().len(), 200);
    let narrowed = session.at(
        "textDocument/completion",
        "- ItemClass: /Game/Items/Desc_250",
        0,
        33,
    );
    assert_eq!(narrowed[0]["label"], "Item 250");
}

#[test]
fn native_completion_uses_a_real_completion_list_without_losing_item_metadata() {
    let mut session = Session::new();
    session.edit("- ItemClass: /");
    let response = session.request(
        "textDocument/completion",
        json!({"textDocument":{"uri":session.uri},"position":{"line":0,"character":14}}),
    );
    assert_eq!(response["isIncomplete"], false);
    let items = response["items"]
        .as_array()
        .expect("actual CompletionList.items");
    assert!(!items.is_empty());
    assert!(items
        .iter()
        .all(|item| item["textEdit"]["newText"].as_str().is_some()));
    assert!(items.iter().any(|item| item["label"] == "Iron Ingot"
        && item["documentation"].to_string().contains("FactoryGame")));
}
