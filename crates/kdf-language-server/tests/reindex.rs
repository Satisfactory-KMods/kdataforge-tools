use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn archive(path: &std::path::Path, name: &str) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(
        br#"{"schemaVersion":1,"metadataAvailable":true,"counts":{"classes":1},"failedLoads":[]}"#,
    )
    .unwrap();
    zip.start_file("classes.ndjson", zip::write::SimpleFileOptions::default())
        .unwrap();
    writeln!(
        zip,
        "{}",
        json!({"path":"/Script/Test.Example","name":name,"properties":[]})
    )
    .unwrap();
    zip.finish().unwrap();
}
fn request(stdin: &mut impl Write, id: usize, method: &str, params: Value) {
    let message =
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .unwrap();
    write!(stdin, "Content-Length: {}\r\n\r\n", message.len()).unwrap();
    stdin.write_all(&message).unwrap();
    stdin.flush().unwrap();
}
fn response(receiver: &mpsc::Receiver<Value>, id: usize) -> Value {
    loop {
        let value = receiver
            .recv_timeout(Duration::from_secs(15))
            .expect("LSP response timed out");
        if value["id"] == id {
            return value;
        }
    }
}

#[test]
fn actual_reindex_discards_cached_metadata_even_when_archive_mtime_is_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("data.kdfdata");
    archive(&path, "Old");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_kdf-language-server"));
    command
        .env("KDF_EFFECTIVE_DATAPACK_PATH", &path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = Running(command.spawn().unwrap());
    let mut stdin = child.0.stdin.take().unwrap();
    let stdout = child.0.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut length = None;
            loop {
                let mut header = String::new();
                if reader
                    .read_line(&mut header)
                    .ok()
                    .is_none_or(|bytes| bytes == 0)
                {
                    return;
                }
                if header.trim().is_empty() {
                    break;
                }
                if let Some(value) = header.strip_prefix("Content-Length:") {
                    length = value.trim().parse::<usize>().ok();
                }
            }
            let Some(length) = length.filter(|length| *length < 1024 * 1024) else {
                return;
            };
            let mut message = vec![0; length];
            if reader.read_exact(&mut message).is_err() {
                return;
            }
            let Ok(value) = serde_json::from_slice(&message) else {
                return;
            };
            if sender.send(value).is_err() {
                return;
            }
        }
    });
    request(
        &mut stdin,
        0,
        "initialize",
        json!({"capabilities":{},"rootUri":url::Url::from_directory_path(directory.path()).unwrap()}),
    );
    assert!(response(&receiver, 0).get("result").is_some());
    request(&mut stdin, 1, "kdf/classes", json!({}));
    assert_eq!(
        response(&receiver, 1)["result"]["classes"][0]["name"],
        "Old"
    );
    archive(&path, "New");
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    request(&mut stdin, 2, "kdf/reindex", json!({}));
    let reindexed = response(&receiver, 2);
    assert_eq!(reindexed["result"]["status"], "ready", "{reindexed}");
    assert_eq!(reindexed["result"]["classes"], 1);
    request(&mut stdin, 3, "kdf/classes", json!({}));
    assert_eq!(
        response(&receiver, 3)["result"]["classes"][0]["name"],
        "New"
    );
}
