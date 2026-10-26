#[cfg(windows)]
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn is_manifest_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("pack.yml"))
}

pub(crate) fn manifest_path(directory: &Path) -> Option<PathBuf> {
    let canonical = directory.join("pack.yml");
    if !canonical.is_file() {
        return None;
    }
    #[cfg(windows)]
    {
        let physical = fs::read_dir(directory).ok().and_then(|entries| {
            let mut variant = None;
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() || !is_manifest_path(&path) {
                    continue;
                }
                if path.file_name().and_then(|name| name.to_str()) == Some("pack.yml") {
                    return Some(path);
                }
                variant.get_or_insert(path);
            }
            variant
        });
        Some(physical.unwrap_or(canonical))
    }
    #[cfg(not(windows))]
    {
        Some(canonical)
    }
}

pub(crate) fn has_manifest(directory: &Path) -> bool {
    directory.join("pack.yml").is_file()
}

pub(crate) fn is_loadable_manifest_path(path: &Path) -> bool {
    path.parent()
        .and_then(manifest_path)
        .is_some_and(|manifest| manifest == path)
}

pub(crate) fn owns_document(pack_root: &Path, document: &Path) -> bool {
    document
        .ancestors()
        .find(|directory| has_manifest(directory))
        == Some(pack_root)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn uppercase_only_manifest_is_not_loadable_on_linux() {
        let root = tempfile::tempdir().unwrap();
        let variant = root.path().join("Pack.yml");
        std::fs::write(&variant, "ref: Sample\n").unwrap();
        assert!(!has_manifest(root.path()));
        assert_eq!(manifest_path(root.path()), None);
        assert!(!is_loadable_manifest_path(&variant));
    }
}
