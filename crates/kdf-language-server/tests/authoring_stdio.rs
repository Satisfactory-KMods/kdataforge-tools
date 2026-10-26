mod support;
use serde_json::{json, Value};
use std::{fs, io::Write};
use support::Client;
use url::Url;

fn fixture() -> (tempfile::TempDir, String, String, String) {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("pack.yml"),
        "ref: Demo\nname: Demo\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        root.path().join(".kdf-editor.toml"),
        "[lint]\ntrigger = 'manual'\n",
    )
    .unwrap();
    let text = "type: item\nitems:\n  - id: Old\n";
    fs::write(root.path().join("items.item.yml"), text).unwrap();
    fs::write(
        root.path().join("patch.cdo.yml"),
        "type: cdo\npatches: [{target: /KDataForge/Gen/Demo.Gen_Demo_Old, properties: []}]\n",
    )
    .unwrap();
    let uri = Url::from_file_path(root.path().join("items.item.yml"))
        .unwrap()
        .to_string();
    let root_uri = Url::from_directory_path(root.path()).unwrap().to_string();
    (root, root_uri, uri, text.into())
}

fn result(value: Value) -> Value {
    assert!(value.get("error").is_none(), "{value}");
    value["result"].clone()
}

#[test]
fn versioned_authoring_requests_and_stale_guards_over_stdio() {
    let (root, root_uri, uri, text) = fixture();
    let (mut client, init) = Client::start(
        env!("CARGO_BIN_EXE_kdf-language-server"),
        &root_uri,
        json!({"workspace":{"workspaceEdit":{"documentChanges":true}}}),
    );
    assert_eq!(
        init["result"]["capabilities"]["experimental"]["kdfAuthoring"]["completionLimit"],
        200
    );
    client.open(&uri, &text);
    let exported = result(client.request(
        "kdf/exportSnapshot",
        json!({"textDocument":{"uri":uri,"version":1}}),
    ));
    assert_eq!(exported["snapshot"]["schemaVersion"], 1);
    assert_eq!(exported["complete"], true);
    assert!(
        !exported.to_string().contains("type: item"),
        "snapshot export must not contain raw buffers"
    );
    let token = exported["snapshot"].clone();
    let page = result(client.request(
        "kdf/contentCompletion",
        json!({"kind":"content","query":"Old","limit":9999,"snapshot":token,"requestGeneration":2}),
    ));
    assert_eq!(
        page["items"][0]["identity"],
        json!({"pack":"demo","namespace":"class","id":"old"})
    );
    assert_eq!(page["requestGeneration"], 2);
    let obsolete = client.request(
        "kdf/contentCompletion",
        json!({"kind":"content","requestGeneration":1}),
    );
    assert_eq!(obsolete["error"]["code"], -32801);
    let symbols = result(client.request(
        "kdf/contentSymbols",
        json!({"textDocument":{"uri":uri,"version":1}}),
    ));
    assert_eq!(symbols["items"][0]["id"], "Old");
    let outline = result(client.request(
        "textDocument/documentSymbol",
        json!({"textDocument":{"uri":uri}}),
    ));
    assert_eq!(outline[0]["name"], "Old");
    let schema = result(client.request("kdf/effectiveSchema", json!({"kind":"pack"})));
    assert!(schema["schema"]["properties"]["scripts"].is_object());
    assert!(!schema["resources"].as_object().unwrap().is_empty());
    let rename_params = json!({"snapshot":token,"textDocument":{"uri":uri,"version":1},"position":{"line":2,"character":9},"newName":"New"});
    let plan = result(client.request("kdf/renameContent", rename_params.clone()));
    assert_eq!(plan["documents"].as_array().unwrap().len(), 2);
    assert!(plan["workspaceEdit"]["documentChanges"]
        .as_array()
        .unwrap()
        .iter()
        .any(|document| document["textDocument"]["uri"] == uri
            && document["textDocument"]["version"] == 1));
    assert!(plan["documents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|document| document["digest"].as_str().unwrap().len() == 64));
    assert_eq!(
        fs::read_to_string(root.path().join("items.item.yml")).unwrap(),
        text
    );
    client.notify("textDocument/didChange",json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":text.replace("Old","Draft")}]}));
    let stale = client.request("kdf/renameContent", rename_params);
    assert_eq!(stale["error"]["code"], -32801, "{stale}");
    let old_version = client.request(
        "kdf/contentCompletion",
        json!({"kind":"content","textDocument":{"uri":uri,"version":1}}),
    );
    assert_eq!(old_version["error"]["code"], -32801);
    client.shutdown();
}

#[test]
fn disk_change_and_dynamic_references_refuse_rename() {
    let (root, root_uri, uri, _) = fixture();
    let (mut client, _) = Client::start(
        env!("CARGO_BIN_EXE_kdf-language-server"),
        &root_uri,
        json!({}),
    );
    let token = result(client.request("kdf/exportSnapshot", json!({})))["snapshot"].clone();
    fs::write(
        root.path().join("patch.cdo.yml"),
        "type: cdo\npatches: [{target: '{cfg:dynamic}', properties: []}]\n",
    )
    .unwrap();
    let params = json!({"snapshot":token,"textDocument":{"uri":uri,"version":null},"position":{"line":2,"character":9},"newName":"New"});
    let stale = client.request("kdf/renameContent", params.clone());
    assert_eq!(stale["error"]["code"], -32801, "{stale}");
    let fresh = result(client.request("kdf/exportSnapshot", json!({})));
    let mut params = params;
    params["snapshot"] = fresh["snapshot"].clone();
    let refused = client.request("kdf/renameContent", params);
    assert_eq!(refused["error"]["code"], -32602);
    assert!(refused["error"]["message"]
        .as_str()
        .unwrap()
        .contains("dynamic"));
    client.shutdown();
}

#[test]
fn why_and_verified_quick_fix_share_lint_evidence_and_reject_forgery() {
    let (root, root_uri, _, _) = fixture();
    let archive = root.path().join("reflection.kdfdata");
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    write!(zip,"{}",json!({"schemaVersion":1,"counts":{"classes":1},"failedLoads":[],"replacementMetadata":{"schemaVersion":1,"replacements":[
        {"kind":"class","oldName":"/Script/Test.Old","newName":"/Script/Test.New","verified":true,"authority":"unreal-core-redirects"}
    ]}})).unwrap();
    zip.start_file("classes.ndjson", zip::write::SimpleFileOptions::default())
        .unwrap();
    writeln!(
        zip,
        "{}",
        json!({"path":"/Script/Test.New","properties":[]})
    )
    .unwrap();
    zip.finish().unwrap();
    let (mut client, _) = Client::start_with(
        env!("CARGO_BIN_EXE_kdf-language-server"),
        &root_uri,
        json!({}),
        json!({"kdf":{"datapackPath":archive}}),
    );
    let uri = Url::from_file_path(root.path().join("outdated.cdo.yml"))
        .unwrap()
        .to_string();
    let text = "type: cdo\npatches: [{target: /Script/Test.Old, properties: []}]\n";
    client.open(&uri, text);
    let why = result(client.request(
        "kdf/why",
        json!({"textDocument":{"uri":uri,"version":1},"ruleId":"datapack.unknown-class"}),
    ));
    let record = &why["explanations"][0];
    assert_eq!(record["evidence"]["status"], "verified", "{why}");
    let diagnostic = json!({"range":record["range"],"code":record["ruleId"],"message":record["message"],"data":{"evidence":{"replacement":{"newName":"/Script/Evil.Guess"}}}});
    let actions=result(client.request("textDocument/codeAction",json!({"textDocument":{"uri":uri},"range":record["range"],"context":{"diagnostics":[diagnostic]}})));
    assert_eq!(actions.as_array().unwrap().len(), 1, "{actions}");
    let change = &actions[0]["edit"]["documentChanges"][0];
    assert_eq!(change["textDocument"]["version"], 1);
    assert_eq!(change["edits"][0]["newText"], "/Script/Test.New");
    let unknown = text.replace("Test.Old", "Test.Missing");
    client.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":unknown}]}),
    );
    let why = result(client.request(
        "kdf/explain",
        json!({"textDocument":{"uri":uri,"version":2},"ruleId":"datapack.unknown-class"}),
    ));
    assert_eq!(why["explanations"][0]["evidence"]["status"], "unknown");
    let stale_action=result(client.request("textDocument/codeAction",json!({"textDocument":{"uri":uri},"range":record["range"],"context":{"diagnostics":[diagnostic]}})));
    assert!(stale_action.as_array().unwrap().is_empty());
    client.shutdown();
}
