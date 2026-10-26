//! Windows file aliases must preserve unsaved native document buffers.
use super::Backend;
use std::path::{Path, PathBuf};
use tower_lsp::{lsp_types::*, LspService};

fn short_path(path: &Path) -> Option<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    #[link(name = "kernel32")]
    extern "system" {
        fn GetShortPathNameW(long: *const u16, short: *mut u16, length: u32) -> u32;
    }
    let input: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut output = vec![0u16; 32_768];
    // Both buffers are valid UTF-16 allocations for the complete synchronous call.
    let length =
        unsafe { GetShortPathNameW(input.as_ptr(), output.as_mut_ptr(), output.len() as u32) };
    if length == 0 || length as usize >= output.len() {
        return None;
    }
    let short = PathBuf::from(std::ffi::OsString::from_wide(&output[..length as usize]));
    (short != path).then_some(short)
}

async fn assert_alias_lifecycle(open_from_short: bool) {
    let dir = tempfile::Builder::new()
        .prefix("kdf-unsaved-document-identity-")
        .tempdir()
        .unwrap();
    let Some(short) = short_path(dir.path()) else {
        eprintln!("8.3 aliases unavailable on this volume; short-path regression not exercised");
        return;
    };
    let plain = Url::from_file_path(dir.path().join("new.cdo.yml")).unwrap();
    let client = Url::from_file_path(short.join("new.cdo.yml")).unwrap();
    assert_ne!(plain, client);
    assert!(!dir.path().join("new.cdo.yml").exists());
    let (service, _socket) = LspService::new(Backend::new);
    let backend = service.inner();
    let opened = if open_from_short { &client } else { &plain };
    let closed = if open_from_short { &plain } else { &client };

    backend
        .open_document(opened.clone(), 1, "unsaved\n".into())
        .await;
    assert_eq!(backend.source(&client).await.as_deref(), Some("unsaved\n"));
    assert_eq!(backend.source(&plain).await.as_deref(), Some("unsaved\n"));
    assert_eq!(
        backend.open_urls().await.values().collect::<Vec<_>>(),
        [opened]
    );
    for (version, uri) in [(2, &plain), (3, &client)] {
        backend
            .change_document(
                uri,
                version,
                vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: format!("version {version}\n"),
                }],
            )
            .await
            .unwrap();
    }
    assert_eq!(
        backend.source(&client).await.as_deref(),
        Some("version 3\n")
    );
    // Once opened, hot client lookups keep working even if the folder disappears.
    // Recanonicalizing on every request would lose the 8.3 identity here.
    drop(dir);
    assert_eq!(
        backend.source(&client).await.as_deref(),
        Some("version 3\n")
    );
    backend.close_document(closed).await;
    assert!(backend.open_urls().await.is_empty());
    assert!(backend.source(&plain).await.is_none());
}

#[tokio::test]
async fn short_path_unsaved_buffers_keep_identity_through_changes_and_close() {
    assert_alias_lifecycle(true).await;
}

#[tokio::test]
async fn alternate_short_path_is_cached_after_first_buffer_lookup() {
    assert_alias_lifecycle(false).await;
}
