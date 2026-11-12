mod support;
use serde_json::json;
use std::fs;
use url::Url;

#[test]
fn discovered_type_is_completed_and_exported() {
    let root = tempfile::tempdir().unwrap();
    let schemas = root
        .path()
        .join("Mods/Example/Resources/KDataForge/Schemas");
    fs::create_dir_all(&schemas).unwrap();
    fs::write(root.path().join("Mods/Example/Example.uplugin"), "{}").unwrap();
    fs::write(schemas.join("sfpmodule.schema.yml"), include_str!("../../kdf-validation/tests/fixtures/mod-schema/Mods/GameFeatures/SatisfactoryPlus/Resources/KDataForge/Schemas/sfpmodule.schema.yml")).unwrap();
    let uri = Url::from_file_path(root.path().join("test.sfpmodule.yml"))
        .unwrap()
        .to_string();
    let root_uri = Url::from_directory_path(root.path()).unwrap().to_string();
    let (mut client, _) = support::Client::start(
        env!("CARGO_BIN_EXE_kdf-language-server"),
        &root_uri,
        json!({}),
    );
    client.open(&uri, "type: sfp\n");
    let response = client.request(
        "textDocument/completion",
        json!({"textDocument": {"uri": uri}, "position": {"line": 0, "character": 9}}),
    );
    let result = &response["result"];
    let items = result
        .as_array()
        .or_else(|| result["items"].as_array())
        .unwrap();
    assert!(
        items.iter().any(|item| item["label"] == "sfpmodule"),
        "{response}"
    );
    let schema = client.request("kdf/effectiveSchema", json!({"kind": "sfpmodule"}));
    assert_eq!(
        schema["result"]["schema"]["x-kdf-type"], "sfpmodule",
        "{schema}"
    );
}

#[test]
fn condition_provider_completion_and_hover() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("Mods/Example/Resources/KDataForge");
    fs::create_dir_all(&dir).unwrap();
    fs::write(root.path().join("Mods/Example/Example.uplugin"), "{}").unwrap();
    fs::write(dir.join("providers.yml"), "providers:\n  - prefix: sfp\n    description: SatisfactoryPlus module availability.\n    valueType: bool\n").unwrap();
    let uri = Url::from_file_path(root.path().join("test.cdo.yml"))
        .unwrap()
        .to_string();
    let root_uri = Url::from_directory_path(root.path()).unwrap().to_string();
    let (mut client, _) = support::Client::start(
        env!("CARGO_BIN_EXE_kdf-language-server"),
        &root_uri,
        json!({}),
    );
    client.open(
        &uri,
        "type: cdo\nconditions:\n  config: 'sfp:module.example == true'\n",
    );
    let hover = client.request(
        "textDocument/hover",
        json!({"textDocument": {"uri": uri}, "position": {"line": 2, "character": 15}}),
    );
    assert!(
        hover
            .to_string()
            .contains("SatisfactoryPlus module availability"),
        "{hover}"
    );
    let uri2 = Url::from_file_path(root.path().join("second.cdo.yml"))
        .unwrap()
        .to_string();
    client.open(&uri2, "type: cdo\nconditions:\n  config: 'sf'\n");
    let response = client.request(
        "textDocument/completion",
        json!({"textDocument": {"uri": uri2}, "position": {"line": 2, "character": 13}}),
    );
    let result = &response["result"];
    let items = result
        .as_array()
        .or_else(|| result["items"].as_array())
        .unwrap();
    assert!(
        items.iter().any(|item| item["label"] == "sfp:"),
        "{response}"
    );
}
