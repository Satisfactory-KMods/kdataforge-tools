use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use url::Url;

fn send(stdin: &mut impl Write, message: Value) {
    let bytes = serde_json::to_vec(&message).unwrap();
    write!(stdin, "Content-Length: {}\r\n\r\n", bytes.len()).unwrap();
    stdin.write_all(&bytes).unwrap();
    stdin.flush().unwrap();
}

#[track_caller]
fn receive_until(rx: &Receiver<Value>, matches: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(40);
    let mut observed = std::collections::VecDeque::new();
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            panic!("LSP response deadline reached; last observed messages: {observed:?}");
        };
        let value = match rx.recv_timeout(remaining) {
            Ok(value) => value,
            Err(error) => {
                panic!("LSP receive failed: {error}; last observed messages: {observed:?}")
            }
        };
        if matches(&value) {
            return value;
        }
        if observed.len() == 20 {
            observed.pop_front();
        }
        observed.push_back(value);
    }
}

#[test]
fn stdio_protocol_validates_unsaved_buffer_and_exposes_missing_reflection() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("pack.yml"),
        "ref: Example\nname: Example Pack\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        root.path().join(".kdf-editor.toml"),
        "[lint]\ntrigger = 'onType'\n",
    )
    .unwrap();
    let document = root.path().join("balance.cdo.yml");
    fs::write(&document, "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mValue\n        value: 1\n").unwrap();
    // Windows clients can use different casing or short names for the same path.
    #[cfg(windows)]
    let client_root = std::path::PathBuf::from(root.path().to_str().unwrap().to_ascii_uppercase());
    #[cfg(not(windows))]
    let client_root = root.path().to_path_buf();
    let root_uri = Url::from_directory_path(root.path()).unwrap();
    let document_uri = Url::from_file_path(client_root.join("balance.cdo.yml")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_kdf-language-server"))
        .env_remove("KDF_USER_SETTINGS_PATH")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
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
            let mut body = vec![0; length];
            if reader.read_exact(&mut body).is_err() {
                return;
            }
            if let Ok(value) = serde_json::from_slice(&body) {
                if tx.send(value).is_err() {
                    return;
                }
            }
        }
    });

    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":root_uri,
            "capabilities":{"workspace":{"workspaceEdit":{"documentChanges":true}}}}}),
    );
    let init = receive_until(&rx, |value| value.get("id") == Some(&json!(1)));
    assert_eq!(init["result"]["serverInfo"]["name"], "kdf-language-server");
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    );
    let unsaved = "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mValue\n        op: multiplly\n        value: 1\n";
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":document_uri,"languageId":"yaml","version":1,"text":unsaved}}}),
    );
    let published = receive_until(&rx, |value| {
        value.get("method") == Some(&json!("textDocument/publishDiagnostics"))
            && value["params"]["uri"] == json!(document_uri)
            // Startup validation may already have published the valid disk file.
            // Wait for the unsaved document requested by didOpen, not that report.
            && value["params"]["version"] == json!(1)
    });
    assert!(
        published["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "semantic.invalid-op"),
        "unsaved diagnostics missing invalid operation: {published}; root: {}",
        root.path().display()
    );
    let invalid_op = published["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "semantic.invalid-op")
        .unwrap()
        .clone();
    assert_eq!(invalid_op["source"], "semantic");
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":6,"method":"textDocument/codeAction","params":{
            "textDocument":{"uri":document_uri},"range":invalid_op["range"],
            "context":{"diagnostics":[invalid_op]}
        }}),
    );
    let action = receive_until(&rx, |value| value.get("id") == Some(&json!(6)));
    assert_eq!(action["result"][0]["title"], "Change operation to multiply");
    let edits = &action["result"][0]["edit"]["changes"][document_uri.as_str()];
    assert_eq!(edits[0]["range"]["start"], json!({"line":5,"character":12}));
    assert_eq!(edits[0]["range"]["end"], json!({"line":5,"character":21}));
    assert_eq!(edits[0]["newText"], "multiply");
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document_uri,"version":1},"contentChanges":[{"text":"op: set"}]
        }}),
    );
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":13,"method":"textDocument/formatting","params":{
            "textDocument":{"uri":document_uri},"options":{"tabSize":2,"insertSpaces":true}
        }}),
    );
    let stale = receive_until(&rx, |value| value.get("id") == Some(&json!(13)));
    assert!(
        stale["result"].is_null(),
        "stale didChange altered buffer: {stale}"
    );
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document_uri,"version":2},"contentChanges":[{"range":{"start":{"line":99,"character":0},"end":{"line":99,"character":99}},"text":"# end"}]
        }}),
    );
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":14,"method":"textDocument/formatting","params":{
            "textDocument":{"uri":document_uri},"options":{"tabSize":2,"insertSpaces":true}
        }}),
    );
    let past_the_end = receive_until(&rx, |value| value.get("id") == Some(&json!(14)));
    // A position past the end is clamped (the end of the document), not rejected: the comment was
    // appended after the last line, so the document no longer ends with a newline.
    assert_eq!(
        past_the_end["result"][0]["newText"],
        "
",
        "clamped insert missing: {past_the_end}"
    );
    assert_eq!(
        past_the_end["result"][0]["range"]["start"],
        json!({"line":7,"character":5})
    );

    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":2,"method":"kdf/classes","params":{}}),
    );
    let classes = receive_until(&rx, |value| value.get("id") == Some(&json!(2)));
    assert_eq!(classes["result"]["status"], "missing-data");
    assert!(classes["result"]["classes"].as_array().unwrap().is_empty());
    let archive = root.path().join("fixture.kdfdata");
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", options).unwrap();
    zip.write_all(b"{\"schemaVersion\":1,\"engineVersion\":\"fixture\",\"counts\":{\"classes\":1},\"failedLoads\":[]}").unwrap();
    zip.start_file("classes.ndjson", options).unwrap();
    zip.write_all(b"{\"path\":\"/Script/Test.Example\",\"name\":\"Example\",\"kind\":\"native\",\"source\":{\"ref\":\"Test\"},\"flags\":{\"names\":[]},\"properties\":[{\"name\":\"mValue\"}]}\n").unwrap();
    zip.finish().unwrap();
    fs::write(
        root.path().join(".kdf-editor.toml"),
        format!(
            "[datapack]\npath = {:?}\n[lint]\ntrigger = 'onType'\n",
            archive.to_string_lossy()
        ),
    )
    .unwrap();
    // The binding is cached for half a second; the watched-file event invalidates it at once.
    let config_uri = Url::from_file_path(root.path().join(".kdf-editor.toml")).unwrap();
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{
            "changes":[{"uri":config_uri,"type":1}]
        }}),
    );
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":4,"method":"kdf/classes","params":{}}),
    );
    let classes = receive_until(&rx, |value| value.get("id") == Some(&json!(4)));
    assert_eq!(classes["result"]["status"], "ready");
    assert_eq!(
        classes["result"]["classes"][0]["path"],
        "/Script/Test.Example"
    );
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":5,"method":"kdf/classInfo","params":{"path":"/Script/Test.Example"}}),
    );
    let info = receive_until(&rx, |value| value.get("id") == Some(&json!(5)));
    assert_eq!(info["result"]["class"]["properties"][0]["name"], "mValue");
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document_uri,"version":3},"contentChanges":[{"text":"class: "}]
        }}),
    );
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":7,"method":"textDocument/completion","params":{
            "textDocument":{"uri":document_uri},"position":{"line":0,"character":7}
        }}),
    );
    let completion = receive_until(&rx, |value| value.get("id") == Some(&json!(7)));
    let items = if completion["result"].is_array() {
        &completion["result"]
    } else {
        &completion["result"]["items"]
    };
    assert_eq!(items[0]["label"], "Example");
    assert_eq!(items[0]["textEdit"]["newText"], "/Script/Test.Example");
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":10,"method":"kdf/operations","params":null}),
    );
    let operations = receive_until(&rx, |value| value.get("id") == Some(&json!(10)));
    assert!(operations["result"].as_array().is_some(), "{operations}");
    assert!(operations["result"]
        .as_array()
        .unwrap()
        .iter()
        .any(|op| op["name"] == "multiply" && op["required"] == json!(["value"])));
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document_uri,"version":4},"contentChanges":[{"text":"op: multiply"}]
        }}),
    );
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":11,"method":"textDocument/signatureHelp","params":{
            "textDocument":{"uri":document_uri},"position":{"line":0,"character":11}
        }}),
    );
    let signature = receive_until(&rx, |value| value.get("id") == Some(&json!(11)));
    assert_eq!(
        signature["result"]["signatures"][0]["label"],
        "multiply(value)"
    );
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":12,"method":"textDocument/formatting","params":{
            "textDocument":{"uri":document_uri},"options":{"tabSize":2,"insertSpaces":true}
        }}),
    );
    let formatting = receive_until(&rx, |value| value.get("id") == Some(&json!(12)));
    assert_eq!(formatting["result"][0]["newText"], "\n");
    let item = root.path().join("item.unlock.yml");
    let item_source =
        "type: unlock\nunlocks:\n  - id: Old\n    parent: /Script/FactoryGame.FGItemDescriptor\n";
    let disk_item = item_source.replace(
        "/Script/FactoryGame.FGItemDescriptor",
        "/Script/Test.Example",
    );
    fs::write(&item, &disk_item).unwrap();
    let item_uri = Url::from_file_path(&item).unwrap();
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":item_uri,"languageId":"yaml","version":1,"text":item_source}
        }}),
    );
    let candidate_archive = root.path().join("candidate.kdfdata");
    let mut candidate_zip = zip::ZipWriter::new(fs::File::create(&candidate_archive).unwrap());
    candidate_zip.start_file("manifest.json", options).unwrap();
    candidate_zip
        .write_all(b"{\"schemaVersion\":1,\"counts\":{\"classes\":1},\"failedLoads\":[]}")
        .unwrap();
    candidate_zip.start_file("classes.ndjson", options).unwrap();
    candidate_zip.write_all(b"{\"path\":\"/Script/FactoryGame.FGItemDescriptor\",\"name\":\"FGItemDescriptor\",\"kind\":\"native\",\"flags\":{\"names\":[]},\"properties\":[]}\n").unwrap();
    candidate_zip.finish().unwrap();
    let binding_before = fs::read(root.path().join(".kdf-editor.toml")).unwrap();
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":15,"method":"kdf/previewBinding","params":{"datapackPath":candidate_archive}}),
    );
    let preview = receive_until(&rx, |value| value.get("id") == Some(&json!(15)));
    assert_eq!(preview["result"]["candidateStatus"], "ready", "{preview}");
    assert_eq!(
        preview["result"]["removed"][0]["diagnostic"]["code"], "datapack.unknown-class",
        "{preview}"
    );
    assert!(
        preview["result"]["added"].as_array().unwrap().is_empty(),
        "{preview}"
    );
    assert_eq!(
        fs::read(root.path().join(".kdf-editor.toml")).unwrap(),
        binding_before
    );
    assert_eq!(fs::read_to_string(&item).unwrap(), disk_item);
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":8,"method":"textDocument/prepareRename","params":{
            "textDocument":{"uri":item_uri},"position":{"line":2,"character":9}
        }}),
    );
    let prepared = receive_until(&rx, |value| value.get("id") == Some(&json!(8)));
    assert_eq!(prepared["result"]["placeholder"], "Old");
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":9,"method":"textDocument/rename","params":{
            "textDocument":{"uri":item_uri},"position":{"line":2,"character":9},"newName":"New"
        }}),
    );
    let renamed = receive_until(&rx, |value| value.get("id") == Some(&json!(9)));
    let edits = renamed["result"]["documentChanges"]
        .as_array()
        .expect("versioned rename edits");
    assert!(
        edits
            .iter()
            .any(|edit| edit["textDocument"]["uri"] == json!(item_uri)
                && edit["textDocument"]["version"] == 1
                && edit["edits"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|edit| edit["newText"] == "New")),
        "{renamed}"
    );
    assert!(edits.iter().all(|edit| !edit["textDocument"]["uri"]
        .as_str()
        .unwrap()
        .ends_with("pack.yml")));
    fs::write(
        root.path().join(".kdf-editor.toml"),
        format!(
            "[datapack]\npath = {:?}\n[lint]\ntrigger = 'manual'\n",
            archive.to_string_lossy()
        ),
    )
    .unwrap();
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"workspace/didChangeConfiguration","params":{"settings":{}}}),
    );
    std::thread::sleep(Duration::from_millis(200));
    while rx.try_recv().is_ok() {}
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document_uri,"version":5},"contentChanges":[{"text":unsaved}]
        }}),
    );
    let deadline = std::time::Instant::now() + Duration::from_millis(550);
    while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
        match rx.recv_timeout(remaining) {
            Ok(message) => assert!(
                message["method"] != "textDocument/publishDiagnostics"
                    || message["params"]["uri"] != json!(document_uri),
                "manual trigger published without request: {message}"
            ),
            Err(_) => break,
        }
    }
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":16,"method":"kdf/revalidate","params":{}}),
    );
    let scheduled = receive_until(&rx, |value| value.get("id") == Some(&json!(16)));
    assert_eq!(scheduled["result"]["scheduled"], true);
    let manual_result = receive_until(&rx, |value| {
        value.get("method") == Some(&json!("textDocument/publishDiagnostics"))
            && value["params"]["uri"] == json!(document_uri)
    });
    assert!(manual_result["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["code"] == "semantic.invalid-op"));
    fs::write(
        root.path().join(".kdf-editor.toml"),
        format!(
            "[datapack]\npath = {:?}\n[lint]\ntrigger = 'onSave'\n",
            archive.to_string_lossy()
        ),
    )
    .unwrap();
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"workspace/didChangeConfiguration","params":{"settings":{}}}),
    );
    receive_until(&rx, |value| {
        value.get("method") == Some(&json!("textDocument/publishDiagnostics"))
            && value["params"]["uri"] == json!(document_uri)
    });
    while rx.try_recv().is_ok() {}
    let changed_on_save = unsaved.replace("multiplly", "mulitply");
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document_uri,"version":6},"contentChanges":[{"text":changed_on_save}]
        }}),
    );
    let deadline = std::time::Instant::now() + Duration::from_millis(550);
    while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
        match rx.recv_timeout(remaining) {
            Ok(message) => assert!(
                message["method"] != "textDocument/publishDiagnostics"
                    || message["params"]["uri"] != json!(document_uri),
                "onSave trigger published on didChange: {message}"
            ),
            Err(_) => break,
        }
    }
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"textDocument/didSave","params":{"textDocument":{"uri":document_uri}}}),
    );
    let saved_result = receive_until(&rx, |value| {
        value.get("method") == Some(&json!("textDocument/publishDiagnostics"))
            && value["params"]["uri"] == json!(document_uri)
    });
    assert!(saved_result["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["message"]
            .as_str()
            .is_some_and(|message| message.contains("mulitply"))));
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":3,"method":"shutdown","params":null}),
    );
    receive_until(&rx, |value| value.get("id") == Some(&json!(3)));
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"exit","params":null}),
    );
    drop(stdin);
    if child.wait_timeout(Duration::from_secs(5)).is_err() {
        let _ = child.kill();
    }
    reader.join().unwrap();
}

trait WaitTimeout {
    fn wait_timeout(&mut self, timeout: Duration) -> std::io::Result<()>;
}

impl WaitTimeout for std::process::Child {
    fn wait_timeout(&mut self, timeout: Duration) -> std::io::Result<()> {
        let started = std::time::Instant::now();
        while self.try_wait()?.is_none() {
            if started.elapsed() >= timeout {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "child did not exit",
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Ok(())
    }
}
