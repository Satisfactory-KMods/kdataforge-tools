//! Pack configuration editor features over the real stdio protocol.

mod support;

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use support::{ArchiveOptions, Client};
use url::Url;

const SERVER: &str = env!("CARGO_BIN_EXE_kdf-language-server");
const ITEM: &str = "/Script/FactoryGame.FGItemDescriptor";
const TUNING: &str = "ConfigDemo/tuning.cdo.yml";

struct Session {
    _dir: tempfile::TempDir,
    _archive: tempfile::NamedTempFile,
    data: PathBuf,
    client: Client,
    versions: std::collections::HashMap<String, i64>,
}

impl Session {
    fn start(lint: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let project = dunce::canonicalize(dir.path()).unwrap();
        let data = project.join("DataForge");
        support::config_workspace(&data);
        support::write(
            &data,
            "ConfigDemo/tuning.cdo.yml",
            "type: cdo\npatches: []\n",
        );
        let archive = support::archive(&ArchiveOptions::default());
        fs::write(
            project.join(".kdf-editor.toml"),
            format!(
                "[datapack]\npath = {:?}\n{lint}",
                archive.path().to_string_lossy()
            ),
        )
        .unwrap();
        let root_uri = Url::from_directory_path(&project).unwrap().to_string();
        let capabilities = json!({"textDocument": {
            "completion": {"completionItem": {"snippetSupport": true, "documentationFormat": ["markdown", "plaintext"]}},
            "hover": {"contentFormat": ["markdown", "plaintext"]},
            "signatureHelp": {"signatureInformation": {"parameterInformation": {"labelOffsetSupport": true}}}
        }});
        let (client, init) = Client::start(SERVER, &root_uri, capabilities);
        let provider = &init["result"]["capabilities"];
        let triggers: Vec<&str> = provider["completionProvider"]["triggerCharacters"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        for expected in [":", " ", ".", "{", "(", ",", "'", "\""] {
            assert!(
                triggers.contains(&expected),
                "missing trigger {expected:?}: {triggers:?}"
            );
        }
        assert_eq!(
            provider["signatureHelpProvider"]["triggerCharacters"],
            json!(["(", ","])
        );
        assert_eq!(provider["renameProvider"]["prepareProvider"], json!(true));
        Self {
            _dir: dir,
            _archive: archive,
            data,
            client,
            versions: Default::default(),
        }
    }

    fn uri(&self, relative: &str) -> String {
        Url::from_file_path(self.data.join(relative))
            .unwrap()
            .to_string()
    }

    /// Opens or replaces the buffer of `relative`; returns the cursor position of the marker.
    fn set(&mut self, relative: &str, source: &str) -> Value {
        let (text, position) = if source.contains('|') {
            support::marker(source)
        } else {
            (source.to_owned(), Value::Null)
        };
        let uri = self.uri(relative);
        let version = self.versions.entry(uri.clone()).or_insert(0);
        *version += 1;
        if *version == 1 {
            self.client.open(&uri, &text);
        } else {
            let version = *version;
            self.client.notify(
                "textDocument/didChange",
                json!({"textDocument": {"uri": uri, "version": version}, "contentChanges": [{"text": text}]}),
            );
        }
        position
    }

    fn at(&mut self, method: &str, relative: &str, source: &str) -> Value {
        let position = self.set(relative, source);
        let uri = self.uri(relative);
        self.client.request(
            method,
            json!({"textDocument": {"uri": uri}, "position": position}),
        )["result"]
            .clone()
    }

    fn complete(&mut self, source: &str) -> Value {
        self.at("textDocument/completion", TUNING, source)
    }
}

fn op(path: &str, op: &str, value: &str) -> String {
    format!(
        "type: cdo\npatches:\n  - target: {ITEM}\n    properties:\n      - path: {path}\n        op: {op}\n        value: {value}\n"
    )
}

fn labels(list: &Value) -> Vec<String> {
    list["items"]
        .as_array()
        .unwrap_or_else(|| panic!("completion list expected: {list}"))
        .iter()
        .map(|item| item["label"].as_str().unwrap().to_owned())
        .collect()
}

fn find<'a>(list: &'a Value, label: &str) -> &'a Value {
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["label"] == label)
        .unwrap_or_else(|| panic!("{label} missing in {:?}", labels(list)))
}

#[test]
fn completion_hover_signature_and_navigation() {
    let mut session = Session::start("");

    // Empty values expose a quoted entry point before the author has typed a brace.
    let list = session.complete(&op("mDisplayName", "set", "|"));
    let template = find(&list, "{item:}");
    assert_eq!(template["textEdit"]["newText"], "'{item:$0}'");
    assert_eq!(template["insertTextFormat"], json!(2));
    assert_eq!(
        template["command"]["command"],
        "editor.action.triggerSuggest"
    );
    for prefix in kdf_validation::template::APPLY_PREFIXES {
        find(&list, &format!("{{{prefix}:}}"));
    }

    // `value: cfg:` is filtered by the numeric slot of the operation.
    let list = session.complete(&op("mEnergyValue", "multiply", "cfg:|"));
    assert_eq!(list["isIncomplete"], json!(false));
    let names = labels(&list);
    for expected in ["balance.speed", "balance.stack", "Other:"] {
        assert!(names.contains(&expected.to_owned()), "{names:?}");
    }
    for refused in ["balance.enabled", "balance.title", "balance.mode"] {
        assert!(!names.contains(&refused.to_owned()), "{names:?}");
    }
    let speed = find(&list, "balance.speed");
    assert_eq!(speed["textEdit"]["newText"], "balance.speed");
    assert_eq!(speed["filterText"], "balance.speed");
    assert_eq!(speed["documentation"]["kind"], "markdown");
    assert!(speed["documentation"]["value"]
        .as_str()
        .unwrap()
        .contains("Machine speed"));
    assert_eq!(
        find(&list, "Other:")["command"]["command"],
        "editor.action.triggerSuggest"
    );

    // `"{item:` lists classes and keeps the colon for the member step.
    let list = session.complete(&op("mDisplayName", "set", "\"{item:|}\""));
    let desc = find(&list, "/Game/Items/Desc_Test.Desc_Test_C");
    assert_eq!(
        desc["textEdit"]["newText"],
        "/Game/Items/Desc_Test.Desc_Test_C:"
    );

    // Members: properties with reload tags, then functions (snippets only because the client asked).
    let list = session.complete(&op("mDisplayName", "set", &format!("\"{{item:{ITEM}:|\"")));
    assert!(find(&list, "mForm")["detail"]
        .as_str()
        .unwrap()
        .contains("cold"));
    let call = find(&list, "GetEnergyValue");
    assert_eq!(call["insertTextFormat"], json!(2));
    assert_eq!(call["textEdit"]["newText"], "GetEnergyValue($0)}");
    assert_eq!(
        call["command"]["command"],
        "editor.action.triggerParameterHints"
    );
    assert_eq!(
        find(&list, "GetItemName")["detail"],
        "GetItemName() -> FText (implicit class)"
    );
    for refused in ["ForEachSlot", "Blueprinty", "Replicated", "Split"] {
        assert!(!labels(&list).contains(&refused.to_owned()));
    }

    // `path:` shows the reload tag of each property.
    let list = session.complete(
        "type: cdo\npatches:\n  - target: /Script/FactoryGame.FGBuildable\n    properties:\n      - path: |\n",
    );
    assert!(
        find(&list, "mHealth")["detail"]
            .as_str()
            .unwrap()
            .contains("newActors"),
        "{list}"
    );

    // Signature help for a template function call.
    let help = session.at(
        "textDocument/signatureHelp",
        TUNING,
        &op(
            "mDisplayName",
            "set",
            &format!("\"{{item:{ITEM}:GetEnergyValue(1, |\""),
        ),
    );
    assert_eq!(
        help["signatures"][0]["label"],
        "GetEnergyValue(ItemClass: TSubclassOf<UObject>, Multiplier: float) -> float"
    );
    assert_eq!(help["activeParameter"], json!(1));
    assert_eq!(
        help["signatures"][0]["parameters"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // Hover cards.
    let hover = session.at(
        "textDocument/hover",
        TUNING,
        &op("mEnergyValue", "multiply", "cfg:balance.sp|eed"),
    );
    let text = hover["contents"]["value"].as_str().unwrap();
    assert!(
        text.contains("Machine speed") && text.contains("Default: `1.5`"),
        "{text}"
    );
    let hover = session.at("textDocument/hover", TUNING, &op("mForm|", "set", "1"));
    let text = hover["contents"]["value"].as_str().unwrap();
    assert!(text.contains("Reload: **cold**"), "{text}");

    // Definition leads to the field.
    let definition = session.at(
        "textDocument/definition",
        TUNING,
        &op("mEnergyValue", "multiply", "cfg:Other:shared.le|vel"),
    );
    assert!(
        definition["uri"]
            .as_str()
            .unwrap()
            .ends_with("Other/shared.config.yml"),
        "{definition}"
    );
    let shared = fs::read_to_string(session.data.join("Other/shared.config.yml")).unwrap();
    let line = shared
        .lines()
        .position(|line| line.contains("id: shared.level"))
        .unwrap();
    assert_eq!(definition["range"]["start"]["line"], json!(line));

    // Config ids cannot be renamed.
    let balance = fs::read_to_string(session.data.join("ConfigDemo/balance.config.yml")).unwrap();
    let marked = balance.replacen("id: balance.speed", "id: balance.sp|eed", 1);
    let prepared = session.at(
        "textDocument/prepareRename",
        "ConfigDemo/balance.config.yml",
        &marked,
    );
    assert!(prepared.is_null(), "{prepared}");
    let uri = session.uri("ConfigDemo/balance.config.yml");
    let renamed = session.client.request(
        "textDocument/rename",
        json!({"textDocument": {"uri": uri}, "position": {"line": 0, "character": 0}, "newName": "other"}),
    );
    assert!(renamed.get("error").is_some(), "{renamed}");

    session.client.shutdown();
}

#[test]
fn references_over_stdio_find_every_use() {
    let mut session = Session::start("");
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../kdf-validation/tests/fixtures/config/DataForge/ConfigDemo");
    for name in ["tuning.cdo.yml", "gated.recipe.yml"] {
        let text = fs::read_to_string(fixtures.join(name)).unwrap();
        support::write(&session.data, &format!("ConfigDemo/{name}"), &text);
    }
    let balance = fs::read_to_string(session.data.join("ConfigDemo/balance.config.yml")).unwrap();
    let (line, content) = balance
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("id: balance.speed"))
        .unwrap();
    let uri = session.uri("ConfigDemo/balance.config.yml");
    session.set("ConfigDemo/balance.config.yml", &balance);
    let position = json!({"line": line, "character": content.find("balance.speed").unwrap() + 3});
    let result = session.client.request(
        "textDocument/references",
        json!({"textDocument": {"uri": uri}, "position": position, "context": {"includeDeclaration": true}}),
    )["result"]
        .clone();
    let found = result.as_array().expect("locations");
    let count = |suffix: &str| {
        found
            .iter()
            .filter(|location| location["uri"].as_str().unwrap().ends_with(suffix))
            .count()
    };
    assert_eq!(count("tuning.cdo.yml"), 2, "{found:?}");
    assert_eq!(count("gated.recipe.yml"), 1, "{found:?}");
    assert_eq!(count("balance.config.yml"), 1, "{found:?}");
    session.client.shutdown();
}

fn severity_of_cold(lint: &str) -> Option<i64> {
    let mut session = Session::start(lint);
    let uri = session.uri(TUNING);
    session.set(TUNING, &op("mForm", "set", "1"));
    let diagnostics = session.client.diagnostics(&uri);
    let cold = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "property.cold")
        .map(|diagnostic| diagnostic["severity"].as_i64().unwrap());
    session.client.shutdown();
    cold
}

#[test]
fn cold_properties_are_information_by_default_and_follow_the_setting() {
    assert_eq!(severity_of_cold(""), Some(3));
    assert_eq!(
        severity_of_cold("[lint]\ncoldProperties = \"hint\"\n"),
        Some(4)
    );
    assert_eq!(severity_of_cold("[lint]\ncoldProperties = \"off\"\n"), None);
}

#[test]
fn quick_fixes_come_from_the_published_diagnostics() {
    let mut session = Session::start("");
    let uri = session.uri(TUNING);
    session.set(
        TUNING,
        &op(
            "mDisplayName",
            "set",
            "{cfg:balance.title} and {cfg:balance.mode}",
        ),
    );
    let diagnostics = session.client.diagnostics(&uri);
    let unquoted = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "template.unquoted-start")
        .expect("template.unquoted-start")
        .clone();
    let actions = session.client.request(
        "textDocument/codeAction",
        json!({"textDocument": {"uri": uri}, "range": unquoted["range"], "context": {"diagnostics": [unquoted]}}),
    )["result"]
        .clone();
    let wrap = actions
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["title"] == "Wrap the value in quotes")
        .expect("quote-wrap action");
    let edit = &wrap["edit"]["changes"][uri.as_str()][0];
    assert_eq!(
        edit["newText"],
        "\"{cfg:balance.title} and {cfg:balance.mode}\""
    );

    session.set(TUNING, &op("mEnergyValue", "multiply", "cfg:balance.spede"));
    let diagnostics = session.client.diagnostics(&uri);
    let unknown = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "config.unknown-id")
        .expect("config.unknown-id")
        .clone();
    let actions = session.client.request(
        "textDocument/codeAction",
        json!({"textDocument": {"uri": uri}, "range": unknown["range"], "context": {"diagnostics": [unknown]}}),
    )["result"]
        .clone();
    assert_eq!(actions[0]["title"], "Change to \"balance.speed\"");
    session.client.shutdown();
}

#[test]
fn generated_schematic_default_base_keeps_specialized_reflected_path_completion() {
    let mut session = Session::start("");
    let list = session.complete(
        "type: schematic\nschematics:\n  - id: NewMilestone\n    properties:\n      - path: |\n",
    );
    assert_eq!(list["isIncomplete"], false);
    assert!(labels(&list).contains(&"mUnlocks".to_owned()), "{list}");
    assert!(find(&list, "mUnlocks")["detail"]
        .as_str()
        .unwrap()
        .contains("cold"));
}
