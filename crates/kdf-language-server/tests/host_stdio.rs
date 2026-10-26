//! Behaviour an LSP host can rely on over the real stdio protocol: client URL
//! spellings, coalesced validation, lazy completion documentation, host options and capability
//! gating.

mod support;

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use support::{ArchiveOptions, Client};
use url::Url;

const SERVER: &str = env!("CARGO_BIN_EXE_kdf-language-server");
const ITEM: &str = "/Script/FactoryGame.FGItemDescriptor";
const TUNING: &str = "ConfigDemo/tuning.cdo.yml";

struct Project {
    _dir: tempfile::TempDir,
    _archive: tempfile::NamedTempFile,
    project: PathBuf,
    data: PathBuf,
    archive_path: PathBuf,
}

impl Project {
    /// A project with the two-pack configuration workspace; `bind` writes `.kdf-editor.toml`.
    fn new(bind: bool, lint: &str) -> Self {
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
        let archive_path = archive.path().to_path_buf();
        if bind || !lint.is_empty() {
            let binding = if bind {
                format!("[datapack]\npath = {:?}\n", archive_path.to_string_lossy())
            } else {
                String::new()
            };
            fs::write(project.join(".kdf-editor.toml"), format!("{binding}{lint}")).unwrap();
        }
        Self {
            _dir: dir,
            _archive: archive,
            project,
            data,
            archive_path,
        }
    }

    fn root_uri(&self) -> String {
        Url::from_directory_path(&self.project).unwrap().to_string()
    }

    fn uri(&self, relative: &str) -> String {
        Url::from_file_path(self.data.join(relative))
            .unwrap()
            .to_string()
    }

    fn start(&self, capabilities: Value, options: Value) -> Client {
        Client::start_with(SERVER, &self.root_uri(), capabilities, options).0
    }
}

fn markdown_caps() -> Value {
    json!({"textDocument": {
        "completion": {"completionItem": {"snippetSupport": true, "documentationFormat": ["markdown", "plaintext"]}},
        "hover": {"contentFormat": ["markdown", "plaintext"]},
        "signatureHelp": {"signatureInformation": {
            "documentationFormat": ["markdown", "plaintext"],
            "parameterInformation": {"labelOffsetSupport": true},
            "activeParameterSupport": true
        }}
    }})
}

fn op(path: &str, op: &str, value: &str) -> String {
    format!(
        "type: cdo\npatches:\n  - target: {ITEM}\n    properties:\n      - path: {path}\n        op: {op}\n        value: {value}\n"
    )
}

fn complete(client: &mut Client, uri: &str, source: &str, version: i64) -> Value {
    let (text, position) = support::marker(source);
    if version == 1 {
        client.open(uri, &text);
    } else {
        client.notify(
            "textDocument/didChange",
            json!({"textDocument": {"uri": uri, "version": version}, "contentChanges": [{"text": text}]}),
        );
    }
    client.request(
        "textDocument/completion",
        json!({"textDocument": {"uri": uri}, "position": position}),
    )["result"]
        .clone()
}

fn items(list: &Value) -> Vec<Value> {
    list["items"].as_array().cloned().unwrap_or_default()
}

fn item<'a>(all: &'a [Value], label: &str) -> &'a Value {
    all.iter()
        .find(|item| item["label"] == label)
        .unwrap_or_else(|| panic!("{label} missing in {:?}", labels(all)))
}

fn labels(all: &[Value]) -> Vec<String> {
    all.iter()
        .filter_map(|item| item["label"].as_str().map(str::to_owned))
        .collect()
}

fn assert_plain_card(text: &str) {
    assert!(!text.trim().is_empty());
    for delimiter in ["**", "`", "]("] {
        assert!(
            !text.contains(delimiter),
            "Markdown delimiter {delimiter:?}: {text}"
        );
    }
}

#[test]
fn completion_cards_respect_plaintext_preference_inline_and_on_resolve() {
    let project = Project::new(true, "");
    let uri = project.uri(TUNING);
    for formats in [json!(["plaintext"]), json!(["plaintext", "markdown"])] {
        for lazy in [false, true] {
            let mut caps = json!({"textDocument": {"completion": {"completionItem": {
                "documentationFormat": formats
            }}}});
            if lazy {
                caps["textDocument"]["completion"]["completionItem"]["resolveSupport"] =
                    json!({"properties": ["documentation"]});
            }
            let mut client = project.start(caps, Value::Null);
            let cases = [
                ("type: c|\n".to_owned(), "cdo"),
                (
                    "type: cdo\npatches:\n  - target: /Script/FactoryGame.FGItem|\n".to_owned(),
                    "FGItemDescriptor",
                ),
                (op("m|", "set", "1"), "mEnergyValue"),
                (op("mEnergyValue", "multiply", "cfg:|"), "balance.speed"),
                (
                    op("mDisplayName", "set", &format!("\"{{item:{ITEM}:|\"")),
                    "GetEnergyValue",
                ),
            ];
            for (version, (source, label)) in cases.iter().enumerate() {
                let all = items(&complete(&mut client, &uri, source, version as i64 + 1));
                let mut selected = item(&all, label).clone();
                if selected.get("documentation").is_none() {
                    assert!(lazy, "missing inline documentation: {selected}");
                    selected = client.request("completionItem/resolve", selected)["result"].clone();
                }
                let text = selected["documentation"]
                    .as_str()
                    .unwrap_or_else(|| panic!("plaintext completion documentation: {selected}"));
                assert_plain_card(text);
                // Resolving an item that already carries native documentation follows the same format.
                let resolved = client.request("completionItem/resolve", selected)["result"].clone();
                assert_plain_card(resolved["documentation"].as_str().unwrap());
            }
            client.shutdown();
        }
    }
}

#[test]
fn path_completion_keeps_reload_documentation_in_the_preferred_format() {
    let project = Project::new(true, "");
    let uri = project.uri(TUNING);
    for formats in [
        json!(["plaintext"]),
        json!(["plaintext", "markdown"]),
        json!(["markdown", "plaintext"]),
    ] {
        let markdown = formats[0] == "markdown";
        let caps = json!({"textDocument": {"completion": {"completionItem": {
            "documentationFormat": formats
        }}}});
        let mut client = project.start(caps, Value::Null);
        let all = items(&complete(&mut client, &uri, &op("mF|", "set", "1"), 1));
        let form = item(&all, "mForm");
        assert!(form["detail"].as_str().unwrap().contains("cold"), "{form}");
        let card = if markdown {
            assert_eq!(form["documentation"]["kind"], "markdown");
            form["documentation"]["value"].as_str().unwrap()
        } else {
            let text = form["documentation"].as_str().unwrap();
            assert_plain_card(text);
            text
        };
        assert!(
            card.contains("mForm"),
            "native property card missing: {card}"
        );
        let reload = if markdown {
            "Reload: **cold**"
        } else {
            "Reload: cold"
        };
        assert!(card.contains(reload), "reload explanation missing: {card}");
        assert!(
            card.contains("existing logistics keep the old form until a restart"),
            "reload reason missing: {card}"
        );
        assert_eq!(card.matches("Reload:").count(), 1, "{card}");
        let resolved = client.request("completionItem/resolve", form.clone())["result"].clone();
        assert_eq!(resolved["documentation"], form["documentation"]);
        client.shutdown();
    }
}

#[test]
fn diagnostics_go_to_the_url_the_client_uses_and_nowhere_else() {
    let project = Project::new(true, "");
    // The spelling a client sends: a percent-encoded letter (VS Code can write `c%3A`).
    let plain = project.uri(TUNING);
    let mut client_uri = plain.replacen("tuning", "tunin%67", 1);
    if cfg!(windows) {
        client_uri = client_uri.replacen("file:///C:/", "file:///c%3A/", 1);
    }
    assert_ne!(plain, client_uri);
    let mut client = project.start(json!({}), Value::Null);
    client.open(&client_uri, &op("mA", "multiplly", "1"));
    client.wait_for_published(&client_uri, 1);
    let published = client.published_within(Duration::from_millis(2500));
    let mine: Vec<&Value> = published
        .iter()
        .filter(|message| {
            message["params"]["uri"] == json!(client_uri)
                && message["params"]["version"] == json!(1)
        })
        .collect();
    assert!(
        !mine.is_empty(),
        "nothing under the client's URL: {published:?}"
    );
    assert!(
        mine.iter().any(
            |message| message["params"]["diagnostics"]
                .as_array()
                .is_some_and(|list| list
                    .iter()
                    .any(|item| item["code"] == "semantic.invalid-op"))
        ),
        "{mine:?}"
    );
    // No second list for the same file under the server's own spelling.
    assert!(
        published
            .iter()
            .all(|message| message["params"]["uri"] != json!(plain)),
        "duplicate list under {plain}: {published:?}"
    );
    client.shutdown();
}

#[test]
fn publication_wait_preserves_messages_and_requires_the_requested_version() {
    let project = Project::new(true, "");
    let first_uri = project.uri(TUNING);
    let second_uri = project.uri("ConfigDemo/second.cdo.yml");
    let mut client = project.start(json!({}), Value::Null);
    client.open(&first_uri, &op("mA", "multiplly", "1"));
    let first = client.wait_for_published(&first_uri, 1);
    assert_eq!(first["params"]["uri"], first_uri);
    assert_eq!(first["params"]["version"], 1);
    assert_eq!(client.wait_for_published(&first_uri, 1), first);

    client.open(&second_uri, &op("mA", "multiplly", "2"));
    let second = client.wait_for_published(&second_uri, 1);
    assert_eq!(second["params"]["uri"], second_uri);
    assert_eq!(second["params"]["version"], 1);
    client.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": second_uri, "version": 2},
               "contentChanges": [{"text": op("mA", "multiplly", "3")}]}),
    );
    let changed = client.wait_for_published(&second_uri, 2);
    assert_eq!(changed["params"]["uri"], second_uri);
    assert_eq!(changed["params"]["version"], 2);
    assert!(changed["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "semantic.invalid-op"));

    let published = client.published_within(Duration::ZERO);
    for message in [first, second, changed] {
        assert!(
            published.contains(&message),
            "publication lost while waiting for another URI/version: {message}; collected: {published:?}"
        );
    }
    client.shutdown();
}

#[test]
fn edit_bursts_validate_once_and_report_the_last_text() {
    let project = Project::new(true, "");
    let uri = project.uri(TUNING);
    let mut client = project.start(json!({}), Value::Null);
    client.open(&uri, &op("mA", "set", "1"));
    client.published_within(Duration::from_millis(1500));
    for version in 2..=31 {
        let operation = if version == 31 { "multiplly" } else { "set" };
        client.notify(
            "textDocument/didChange",
            json!({"textDocument": {"uri": uri, "version": version},
                   "contentChanges": [{"text": op("mA", operation, &version.to_string())}]}),
        );
    }
    let published = client.published_within(Duration::from_millis(2000));
    let for_doc: Vec<&Value> = published
        .iter()
        .filter(|message| message["params"]["uri"] == json!(uri))
        .collect();
    assert!(
        (1..=3).contains(&for_doc.len()),
        "{} validations for one burst",
        for_doc.len()
    );
    let last = for_doc.last().unwrap();
    assert_eq!(last["params"]["version"], json!(31));
    assert!(last["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["code"] == "semantic.invalid-op"));
    client.shutdown();
}

#[test]
fn documentation_is_resolved_on_demand_for_clients_that_can() {
    let project = Project::new(true, "");
    let uri = project.uri(TUNING);
    let capabilities = json!({"textDocument": {"completion": {"completionItem": {
        "documentationFormat": ["markdown"],
        "resolveSupport": {"properties": ["documentation"]}
    }}}});
    let (mut client, init) =
        Client::start_with(SERVER, &project.root_uri(), capabilities, Value::Null);
    assert_eq!(
        init["result"]["capabilities"]["completionProvider"]["resolveProvider"],
        json!(true)
    );
    // Members: no card inline, `data` says which one to build.
    let list = complete(
        &mut client,
        &uri,
        &op("mDisplayName", "set", &format!("\"{{item:{ITEM}:|\"")),
        1,
    );
    let all = items(&list);
    let form = item(&all, "mForm");
    assert!(
        form.get("documentation").is_none_or(Value::is_null),
        "{form}"
    );
    assert!(form["data"].is_object(), "{form}");
    assert!(
        form["detail"].as_str().unwrap().contains("cold"),
        "the short detail stays inline"
    );
    let resolved = client.request("completionItem/resolve", form.clone())["result"].clone();
    let card = resolved["documentation"]["value"].as_str().unwrap();
    assert_eq!(resolved["documentation"]["kind"], "markdown");
    assert!(card.contains("Reload: **cold**"), "{card}");
    // Functions resolve to their signature card.
    let call = item(&all, "GetEnergyValue");
    let resolved = client.request("completionItem/resolve", call.clone())["result"].clone();
    assert!(
        resolved["documentation"]["value"]
            .as_str()
            .unwrap()
            .contains("GetEnergyValue("),
        "{resolved}"
    );
    // Config ids and settings resolve too.
    let list = complete(
        &mut client,
        &uri,
        &op("mEnergyValue", "multiply", "cfg:|"),
        2,
    );
    let all = items(&list);
    let speed = item(&all, "balance.speed");
    assert!(speed.get("documentation").is_none_or(Value::is_null));
    let resolved = client.request("completionItem/resolve", speed.clone())["result"].clone();
    assert!(resolved["documentation"]["value"]
        .as_str()
        .unwrap()
        .contains("Machine speed"));
    // An item that already has its documentation (or none to build) comes back unchanged.
    let plain = json!({"label": "x"});
    assert_eq!(
        client.request("completionItem/resolve", plain.clone())["result"]["label"],
        "x"
    );
    client.shutdown();
}

#[test]
fn long_member_lists_are_capped_and_narrowed_by_what_is_typed() {
    // A class with far more members than the cap.
    let properties: Vec<Value> = (0..400)
        .map(|n| support::prop(&format!("mBulk{n:03}"), "IntProperty", "int32"))
        .collect();
    let big = support::class(
        "/Script/Test.Bulk",
        Some("/Script/CoreUObject.Object"),
        properties,
        vec![],
    );
    let dir = tempfile::tempdir().unwrap();
    let project = dunce::canonicalize(dir.path()).unwrap();
    let data = project.join("DataForge");
    support::config_workspace(&data);
    let archive = support::archive_with(vec![big], &ArchiveOptions::default());
    fs::write(
        project.join(".kdf-editor.toml"),
        format!(
            "[datapack]\npath = {:?}\n",
            archive.path().to_string_lossy()
        ),
    )
    .unwrap();
    let uri = Url::from_file_path(data.join(TUNING)).unwrap().to_string();
    let root_uri = Url::from_directory_path(&project).unwrap().to_string();
    let (mut client, _) = Client::start_with(SERVER, &root_uri, markdown_caps(), Value::Null);
    let source = |typed: &str| {
        format!(
            "type: cdo\npatches:\n  - target: /Script/Test.Bulk\n    properties:\n      - path: {typed}|\n"
        )
    };
    let list = complete(&mut client, &uri, &source(""), 1);
    assert_eq!(items(&list).len(), 200);
    assert_eq!(list["isIncomplete"], json!(true));
    // Typing narrows the list on the server, so the one wanted item is not cut away.
    let list = complete(&mut client, &uri, &source("mBulk399"), 2);
    let all = items(&list);
    assert!(
        labels(&all).contains(&"mBulk399".to_owned()),
        "{:?}",
        labels(&all)
    );
    assert_eq!(list["isIncomplete"], json!(false));
    // The same for template members.
    let template = |typed: &str| {
        format!(
            "type: cdo\npatches:\n  - target: /Script/Test.Bulk\n    properties:\n      - path: mA\n        value: \"{{class:/Script/Test.Bulk:{typed}|\"\n"
        )
    };
    let list = complete(&mut client, &uri, &template(""), 3);
    assert_eq!(items(&list).len(), 200);
    assert_eq!(list["isIncomplete"], json!(true));
    let list = complete(&mut client, &uri, &template("mBulk123"), 4);
    assert!(labels(&items(&list)).contains(&"mBulk123".to_owned()));
    client.shutdown();
}

#[test]
fn host_options_bind_the_datapack_and_switch_off_client_commands() {
    // No `.kdf-editor.toml` binding: the datapack comes from `initializationOptions`.
    let project = Project::new(false, "");
    let uri = project.uri(TUNING);
    let options = json!({"kdf": {
        "datapackPath": project.archive_path,
        "clientCommands": false
    }});
    let mut client = project.start(markdown_caps(), options);
    let list = complete(
        &mut client,
        &uri,
        &op("mDisplayName", "set", &format!("\"{{item:{ITEM}:|\"")),
        1,
    );
    let all = items(&list);
    assert!(
        item(&all, "mForm")["detail"]
            .as_str()
            .unwrap()
            .contains("cold"),
        "the datapack of the host options is used"
    );
    // `GetEnergyValue` takes arguments: it would re-trigger parameter hints in a client that
    // knows the command. This host switched the commands off.
    assert!(all
        .iter()
        .all(|item| item.get("command").is_none_or(Value::is_null)));
    let list = complete(
        &mut client,
        &uri,
        &op("mEnergyValue", "multiply", "cfg:|"),
        2,
    );
    assert!(items(&list)
        .iter()
        .all(|item| item.get("command").is_none_or(Value::is_null)));

    // A configuration push replaces the selection; an empty path means schema-only.
    client.notify(
        "workspace/didChangeConfiguration",
        json!({"settings": {"kdf": {"datapackPath": ""}}}),
    );
    let classes = client.request("kdf/classes", json!({}));
    assert_eq!(classes["result"]["status"], "missing-data", "{classes}");
    client.notify(
        "workspace/didChangeConfiguration",
        json!({"settings": {"kdf": {"datapackPath": project.archive_path}}}),
    );
    let classes = client.request("kdf/classes", json!({}));
    assert_eq!(classes["result"]["status"], "ready", "{classes}");
    client.shutdown();
}

#[test]
fn commands_stay_on_by_default_for_the_desktop_client() {
    let project = Project::new(true, "");
    let uri = project.uri(TUNING);
    let mut client = project.start(markdown_caps(), Value::Null);
    let list = complete(
        &mut client,
        &uri,
        &op("mEnergyValue", "multiply", "cfg:|"),
        1,
    );
    let all = items(&list);
    assert_eq!(
        item(&all, "Other:")["command"]["command"],
        "editor.action.triggerSuggest"
    );
    client.shutdown();
}

#[test]
fn user_settings_path_comes_from_the_host_options() {
    let project = Project::new(true, "");
    let settings = project.project.join("host-settings.toml");
    fs::write(&settings, "[lint]\ntrigger = 'manual'\n").unwrap();
    let uri = project.uri(TUNING);
    let options = json!({"kdf": {"userSettingsPath": settings}});
    let mut client = project.start(json!({}), options);
    client.open(&uri, &op("mA", "multiplly", "1"));
    // The user settings select manual linting: nothing is published until it is asked for.
    let published = client.published_within(Duration::from_millis(1200));
    assert!(
        published
            .iter()
            .all(|message| message["params"]["uri"] != json!(uri)),
        "{published:?}"
    );
    client.request("kdf/revalidate", json!({}));
    let diagnostics = client.diagnostics(&uri);
    assert!(diagnostics
        .iter()
        .any(|item| item["code"] == "semantic.invalid-op"));
    client.shutdown();
}

#[test]
fn signature_labels_follow_the_declared_capabilities() {
    let project = Project::new(true, "");
    let uri = project.uri(TUNING);
    let call = op(
        "mDisplayName",
        "set",
        &format!("\"{{item:{ITEM}:GetEnergyValue(1, |\""),
    );
    let ask = |capabilities: Value| {
        let mut client = project.start(capabilities, Value::Null);
        let (text, position) = support::marker(&call);
        client.open(&uri, &text);
        let help = client.request(
            "textDocument/signatureHelp",
            json!({"textDocument": {"uri": uri}, "position": position}),
        )["result"]
            .clone();
        client.shutdown();
        help
    };
    // Offsets for a client that declared support (the desktop client does).
    let help = ask(markdown_caps());
    assert_eq!(help["signatures"][0]["activeParameter"], 1);
    assert_eq!(help["activeParameter"], 1);
    let parameters = help["signatures"][0]["parameters"].as_array().unwrap();
    assert!(parameters
        .iter()
        .all(|parameter| parameter["label"].is_array()));
    assert!(
        help["signatures"][0]["documentation"]["kind"] == "markdown"
            || help["signatures"][0].get("documentation").is_none()
    );
    // Plain labels for one that did not.
    let help = ask(json!({}));
    assert!(help["signatures"][0].get("activeParameter").is_none());
    assert_eq!(help["activeParameter"], 1);
    let label = help["signatures"][0]["label"].as_str().unwrap();
    let parameters = help["signatures"][0]["parameters"].as_array().unwrap();
    assert_eq!(parameters.len(), 2);
    for parameter in parameters {
        let text = parameter["label"].as_str().expect("a string label");
        assert!(label.contains(text), "{text} is not part of {label}");
    }
    assert!(
        help["signatures"][0]["documentation"].is_null()
            || help["signatures"][0]["documentation"].is_string(),
        "no markdown for a client that did not declare it: {help}"
    );
    let help = ask(
        json!({"textDocument": {"signatureHelp": {"signatureInformation": {
            "activeParameterSupport": false
        }}}}),
    );
    assert!(help["signatures"][0].get("activeParameter").is_none());
    assert_eq!(help["activeParameter"], 1);
}

#[test]
fn hover_markup_follows_the_declared_formats() {
    let project = Project::new(true, "");
    let uri = project.uri(TUNING);
    let source = op("mEnergyValue", "multiply", "cfg:balance.sp|eed");
    let hover = |capabilities: Value| {
        let mut client = project.start(capabilities, Value::Null);
        let (text, position) = support::marker(&source);
        client.open(&uri, &text);
        let result = client.request(
            "textDocument/hover",
            json!({"textDocument": {"uri": uri}, "position": position}),
        )["result"]
            .clone();
        client.shutdown();
        result
    };
    assert_eq!(hover(markdown_caps())["contents"]["kind"], "markdown");
    for capabilities in [
        json!({}),
        json!({"textDocument": {
            "hover": {"contentFormat": ["plaintext", "markdown"]}
        }}),
    ] {
        let result = hover(capabilities);
        assert_eq!(result["contents"]["kind"], "plaintext");
        assert_plain_card(result["contents"]["value"].as_str().unwrap());
    }
}

#[test]
fn schema_and_reflection_hover_follow_the_declared_formats() {
    let project = Project::new(true, "");
    let uri = project.uri(TUNING);
    let cases = [
        (
            "type: cdo\npat|ches: []\n".to_owned(),
            kdf_validation::field_docs::to_markdown(
                kdf_validation::field_docs::field("cdo", &["patches"]).unwrap(),
            ),
            true,
        ),
        (
            op("mEnergyValue", "mul|tiply", "1"),
            kdf_validation::field_docs::to_markdown(
                kdf_validation::field_docs::value_at(
                    "cdo",
                    &["patches", "properties", "op"],
                    "multiply",
                )
                .unwrap(),
            ),
            true,
        ),
        (
            "type: cdo\npatches:\n  - target: /Script/FactoryGame.FGItem|Descriptor\n".to_owned(),
            ITEM.to_owned(),
            false,
        ),
        (
            op("mDisplayName", "set", "mEn|ergyValue"),
            "mEnergyValue".to_owned(),
            false,
        ),
    ];
    for (capabilities, kind) in [
        (markdown_caps(), "markdown"),
        (
            json!({"textDocument": {"hover": {"contentFormat": ["plaintext"]}}}),
            "plaintext",
        ),
        (json!({}), "plaintext"),
        (
            json!({"textDocument": {"hover": {"contentFormat": ["plaintext", "markdown"]}}}),
            "plaintext",
        ),
    ] {
        let mut client = project.start(capabilities, Value::Null);
        for (index, (source, expected, schema)) in cases.iter().enumerate() {
            let (text, position) = support::marker(source);
            if index == 0 {
                client.open(&uri, &text);
            } else {
                client.notify(
                    "textDocument/didChange",
                    json!({"textDocument": {"uri": uri, "version": index + 1},
                           "contentChanges": [{"text": text}]}),
                );
            }
            let response = client.request(
                "textDocument/hover",
                json!({"textDocument": {"uri": uri}, "position": position}),
            );
            let hover = &response["result"];
            assert_eq!(hover["contents"]["kind"], kind, "{source}: {response}");
            let value = hover["contents"]["value"].as_str().unwrap();
            if kind == "plaintext" {
                assert_plain_card(value);
            }
            if *schema {
                if kind == "markdown" {
                    assert_eq!(value, expected, "{source}");
                }
                assert!(hover["range"].is_object(), "{source}: {response}");
            } else {
                assert!(value.contains(expected), "{source}: {response}");
                assert!(hover["range"].is_null(), "reflection fallback: {response}");
            }
        }
        client.shutdown();
    }
}

#[test]
fn the_workspace_index_follows_edits_of_config_definitions() {
    let project = Project::new(true, "");
    let config = "ConfigDemo/balance.config.yml";
    let config_uri = project.uri(config);
    let tuning_uri = project.uri(TUNING);
    let mut client = project.start(markdown_caps(), Value::Null);
    let list = complete(
        &mut client,
        &tuning_uri,
        &op("mEnergyValue", "multiply", "cfg:|"),
        1,
    );
    assert!(!labels(&items(&list)).contains(&"balance.fresh".to_owned()));

    let original = fs::read_to_string(project.data.join(config)).unwrap();
    let edited = format!(
        "{}  - id: balance.fresh\n    type: number\n    name: Fresh\n    description: A field typed after startup\n    default: 1\n    min: 0\n    max: 5\n",
        original.trim_end_matches('\n').to_owned() + "\n"
    );
    client.open(&config_uri, &edited);
    // The snapshot catches up in the background; a request never waits for the rebuild.
    let started = Instant::now();
    let mut version = 1;
    loop {
        version += 1;
        let list = complete(
            &mut client,
            &tuning_uri,
            &op("mEnergyValue", "multiply", "cfg:|"),
            version,
        );
        if labels(&items(&list)).contains(&"balance.fresh".to_owned()) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the new config field never appeared"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    client.shutdown();
}

#[test]
fn sound_asset_completion_follows_unsaved_edits_of_quoted_declarations() {
    let project = Project::new(true, "");
    let sound = "ConfigDemo/welcome.sound.yml";
    let original = "type: sound\n\"sounds\":\n  - id: AdaWelcome\n    file: audio/welcome.wav\n";
    support::write(&project.data, sound, original);
    let mut client = project.start(markdown_caps(), Value::Null);
    let tuning_uri = project.uri(TUNING);
    let source = op("mDisplayName", "set", "\"{asset:|\"");
    let original_path = "/KDataForge/GenAssets/ConfigDemo.Gen_ConfigDemo_AdaWelcome";
    let edited_path = "/KDataForge/GenAssets/ConfigDemo.Gen_ConfigDemo_AdaEdited";
    // Establish the watched, open-document snapshot before editing. No later disk event
    // or periodic poll should hide a missed Sound buffer invalidation.
    client.notify("workspace/didChangeWatchedFiles", json!({"changes": []}));
    let sound_uri = project.uri(sound);
    client.open(&sound_uri, original);
    let list = complete(&mut client, &tuning_uri, &source, 1);
    let all = items(&list);
    let welcome = item(&all, original_path);
    assert!(welcome["detail"].as_str().unwrap().contains("sound"));
    assert_eq!(welcome["textEdit"]["newText"], format!("{original_path}:"));
    client.notify(
        "textDocument/didChange",
        json!({
            "textDocument": {"uri": sound_uri, "version": 2},
            "contentChanges": [{"text": original.replace("AdaWelcome", "AdaEdited")}]
        }),
    );
    let started = Instant::now();
    let mut version = 1;
    loop {
        version += 1;
        let list = complete(&mut client, &tuning_uri, &source, version);
        let names = labels(&items(&list));
        if names.contains(&edited_path.to_owned()) {
            assert!(!names.contains(&original_path.to_owned()));
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "Sound buffer edit never reached completion: {names:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        fs::read_to_string(project.data.join(sound)).unwrap(),
        original
    );
    client.shutdown();
}

#[test]
fn an_empty_datapack_option_keeps_validation_schema_only() {
    // The project binds a datapack in `.kdf-editor.toml`; the host chose schema-only validation.
    let project = Project::new(true, "");
    let uri = project.uri("ConfigDemo/probe.unlock.yml");
    let unknown_class = "type: unlock\nunlocks:\n  - id: Probe\n    parent: /Script/Nope.Missing\n";
    let codes = |options: Value| {
        let mut client = project.start(json!({}), options);
        client.open(&uri, unknown_class);
        client.wait_for_published(&uri, 1);
        let published = client.published_within(Duration::from_millis(2500));
        assert!(
            published.iter().any(|message| {
                message["params"]["uri"] == json!(uri)
                    && message["params"]["version"] == json!(1)
                    && message["params"]["diagnostics"].is_array()
            }),
            "no completed validation for the opened document: {published:?}"
        );
        client.shutdown();
        let mut codes: Vec<String> = published
            .iter()
            .filter(|message| {
                message["params"]["uri"] == json!(uri) && message["params"]["version"] == json!(1)
            })
            .flat_map(|message| {
                message["params"]["diagnostics"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .filter_map(|item| item["code"].as_str().map(str::to_owned))
            .collect();
        codes.sort();
        codes.dedup();
        codes
    };
    let bound = codes(Value::Null);
    assert!(
        bound.contains(&"datapack.unknown-class".to_owned()),
        "the bound datapack knows the class is missing: {bound:?}"
    );
    let schema_only = codes(json!({"kdf": {"datapackPath": ""}}));
    assert!(
        schema_only
            .iter()
            .all(|code| !code.starts_with("datapack.")),
        "{schema_only:?}"
    );
}
