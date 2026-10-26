//! Document identity and text sources.
//!
//! A client names the same file in different spellings: VS Code can send `file:///c%3A/Users/...`
//! (lower-case drive, percent-encoded colon), other clients `file:///C:/Users/...`, and walking the
//! workspace yields the on-disk spelling. Open buffers are therefore keyed by a normalized path
//! ([`key`]) and keep the client's original URL, which is the only URL diagnostics are published
//! under. [`canonical`] gives the on-disk spelling the validator matches against its own walk.

use crate::workspace::Workspace;
use kdf_validation::FileBuffer;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower_lsp::lsp_types::Url;

/// Path a file URL names (percent-decoded; every drive spelling the `url` crate knows).
pub(crate) fn path_of(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path().ok()
}

/// Identity of a path for lookups: on Windows case-insensitive with unified separators and without
/// a verbatim prefix; on other systems the path itself.
pub(crate) fn key(path: &Path) -> PathBuf {
    if cfg!(windows) {
        key_insensitive(path)
    } else {
        path.to_path_buf()
    }
}

/// [`key`] for case-insensitive file systems (separate so it is testable on every platform).
pub(crate) fn key_insensitive(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    PathBuf::from(text.replace('/', "\\").to_lowercase())
}

/// The on-disk spelling of `path` (real letter case, no verbatim prefix). A file that does not exist
/// yet (an unsaved buffer) is resolved through its folder; anything else is returned with an
/// upper-case drive letter.
pub(crate) fn canonical(path: &Path) -> PathBuf {
    if let Ok(real) = dunce::canonicalize(path) {
        return real;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        if let Ok(real) = dunce::canonicalize(parent) {
            return real.join(name);
        }
    }
    upper_drive(path)
}

fn upper_drive(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_lowercase() {
        let mut fixed = text.into_owned();
        fixed[..1].make_ascii_uppercase();
        return PathBuf::from(fixed);
    }
    path.to_path_buf()
}

/// Snapshot of the open buffers, keyed by [`key`].
#[derive(Clone, Debug, Default)]
pub(crate) struct Buffers {
    files: HashMap<PathBuf, FileBuffer>,
}

impl Buffers {
    pub(crate) fn from_files(files: impl IntoIterator<Item = FileBuffer>) -> Self {
        let mut buffers = Self::default();
        for file in files {
            buffers.insert(file);
        }
        buffers
    }

    pub(crate) fn insert(&mut self, file: FileBuffer) {
        self.files.insert(key(&file.path), file);
    }

    pub(crate) fn get(&self, path: &Path) -> Option<&FileBuffer> {
        self.files.get(&key(path))
    }

    pub(crate) fn contains(&self, path: &Path) -> bool {
        self.files.contains_key(&key(path))
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &FileBuffer> {
        self.files.values()
    }
}

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension == "yml" || extension == "yaml")
}

/// Where navigation reads text from: open buffers first, then the indexed workspace snapshot (so a
/// request does not read and walk the whole tree again), then the disk.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sources {
    buffers: Buffers,
    index: Option<Arc<Workspace>>,
}

impl Sources {
    pub(crate) fn new(buffers: Buffers, index: Option<Arc<Workspace>>) -> Self {
        Self { buffers, index }
    }

    /// Buffers only (no snapshot): every read goes to the disk.
    #[cfg(test)]
    pub(crate) fn from_buffers(buffers: Buffers) -> Self {
        Self::new(buffers, None)
    }

    pub(crate) fn has_buffer(&self, path: &Path) -> bool {
        self.buffers.contains(path)
    }

    /// Text of `path`; borrowed when a buffer or the snapshot has it.
    pub(crate) fn text(&self, path: &Path) -> Option<Cow<'_, str>> {
        if let Some(buffer) = self.buffers.get(path) {
            return Some(Cow::Borrowed(buffer.text.as_str()));
        }
        if let Some(text) = self.index.as_ref().and_then(|index| index.text(path)) {
            return Some(Cow::Borrowed(text));
        }
        fs::read_to_string(path).ok().map(Cow::Owned)
    }

    /// Every YAML file below `root` (sorted): from the snapshot when it covers `root`, else a walk;
    /// plus buffers of files that do not exist on disk yet.
    pub(crate) fn yaml_paths(&self, root: &Path) -> Vec<PathBuf> {
        let wanted = key(root);
        let mut paths: Vec<PathBuf> = match self.index.as_ref().filter(|index| {
            index
                .root()
                .is_some_and(|indexed| wanted.starts_with(key(indexed)))
        }) {
            Some(index) => index
                .paths()
                .filter(|path| key(path).starts_with(&wanted))
                .map(Path::to_path_buf)
                .collect(),
            None => walkdir::WalkDir::new(root)
                .follow_links(false)
                .into_iter()
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry.file_type().is_file() && is_yaml(entry.path()))
                .map(walkdir::DirEntry::into_path)
                .collect(),
        };
        if let Ok(canonical_root) = dunce::canonicalize(root) {
            paths.extend(self.buffers.values().filter_map(|buffer| {
                let path = &buffer.path;
                if !key(path).starts_with(&wanted) || !is_yaml(path) {
                    return None;
                }
                let parent = dunce::canonicalize(path.parent()?).ok()?;
                parent.starts_with(&canonical_root).then(|| path.clone())
            }));
        }
        paths.sort();
        paths.dedup_by(|a, b| key(a) == key(b));
        paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_ignore_drive_case_separators_and_the_verbatim_prefix() {
        let a = key_insensitive(Path::new(r"C:\Modding\DataForge\x.yml"));
        assert_eq!(a, key_insensitive(Path::new("c:/modding/dataforge/x.yml")));
        assert_eq!(
            a,
            key_insensitive(Path::new(r"\\?\C:\Modding\DataForge\x.yml"))
        );
        assert_ne!(a, key_insensitive(Path::new(r"C:\Modding\DataForge\y.yml")));
    }

    #[cfg(windows)]
    #[test]
    fn code_oss_and_plain_urls_name_the_same_buffer() {
        let vscode = Url::parse("file:///c%3A/Modding/DataForge/Pack/x.yml").unwrap();
        let plain = Url::parse("file:///C:/Modding/DataForge/Pack/x.yml").unwrap();
        let (a, b) = (path_of(&vscode).unwrap(), path_of(&plain).unwrap());
        assert_eq!(key(&a), key(&b));
        let mut buffers = Buffers::default();
        buffers.insert(FileBuffer {
            path: a,
            text: "x".into(),
        });
        assert!(buffers.contains(&b));
        assert_eq!(buffers.get(&b).unwrap().text, "x");
    }

    #[test]
    fn canonical_resolves_new_files_through_their_folder() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dunce::canonicalize(dir.path()).unwrap();
        let future = folder.join("not-yet.yml");
        assert_eq!(canonical(&future), future);
        let existing = folder.join("there.yml");
        fs::write(&existing, "a: 1\n").unwrap();
        assert_eq!(canonical(&existing), existing);
    }

    #[test]
    fn lexical_fallback_uppercases_the_drive_letter() {
        assert_eq!(
            upper_drive(Path::new("c:/nowhere/x.yml")),
            PathBuf::from("C:/nowhere/x.yml")
        );
        assert_eq!(
            upper_drive(Path::new("/unix/x.yml")),
            PathBuf::from("/unix/x.yml")
        );
    }

    #[test]
    fn sources_prefer_buffers_over_disk_and_list_unsaved_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        fs::write(root.join("a.yml"), "disk\n").unwrap();
        let sources = Sources::from_buffers(Buffers::from_files([
            FileBuffer {
                path: root.join("a.yml"),
                text: "buffer\n".into(),
            },
            FileBuffer {
                path: root.join("new.yml"),
                text: "unsaved\n".into(),
            },
        ]));
        assert_eq!(sources.text(&root.join("a.yml")).unwrap(), "buffer\n");
        assert_eq!(sources.text(&root.join("new.yml")).unwrap(), "unsaved\n");
        assert_eq!(
            sources.yaml_paths(&root),
            vec![root.join("a.yml"), root.join("new.yml")]
        );
    }
}
