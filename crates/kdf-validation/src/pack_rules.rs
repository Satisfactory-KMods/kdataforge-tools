//! Pack manifest rules that go beyond the schema: the pack icon.

use crate::document::Reporter;
use crate::{LintResult, Severity};
use serde_json::Value;
use std::path::{Component, Path};

const ICON_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "tga", "bmp"];

/// Mirrors FKDFScriptPortablePath, including Windows aliases on every host.
fn script_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && path.split('/').count() <= 32
        && path.split('/').all(|part| {
            let base = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            let device = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || (base.len() == 4
                    && (base.starts_with("COM") || base.starts_with("LPT"))
                    && matches!(base.as_bytes()[3], b'1'..=b'9'));
            !device
                && part.len() <= 255
                && part.starts_with(|ch: char| ch.is_ascii_alphanumeric())
                && !part.ends_with('.')
                && part
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        })
}

pub(crate) fn check_scripts(reporter: &Reporter<'_>, document: &Value, result: &mut LintResult) {
    let Some(scripts) = document.get("scripts").and_then(Value::as_array) else {
        return;
    };
    let mut entries = std::collections::HashSet::new();
    for (index, script) in scripts.iter().enumerate() {
        let base = format!("/scripts/{index}");
        let mut issue = |pointer: String, message: &str| {
            reporter.emit(
                result,
                0,
                &pointer,
                Severity::Error,
                "semantic.script-policy",
                message,
            );
        };
        if let Some(entry) = script.get("entry").and_then(Value::as_str) {
            if !script_path(entry) {
                issue(format!("{base}/entry"), "script entry must be a portable relative path (no traversal, device names or empty segments)");
            } else if !entries.insert(entry) {
                issue(
                    format!("{base}/entry"),
                    "duplicate entry generates the same runtime script id",
                );
            }
            if script
                .get("language")
                .and_then(Value::as_str)
                .is_some_and(|language| language.trim().eq_ignore_ascii_case("kmods.typescript"))
                && entry != "scripts/main.ts"
            {
                issue(
                    format!("{base}/entry"),
                    "KDataForgeJS TypeScript entry must be scripts/main.ts",
                );
            }
        }
        for key in ["reflectionClasses", "classes"] {
            for (offset, class) in script
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                if class.as_str().is_some_and(|class| class.contains("..")) {
                    issue(
                        format!("{base}/{key}/{offset}"),
                        "reflection class paths cannot contain '..'",
                    );
                }
            }
        }
        let mut keys = std::collections::HashSet::new();
        let mut paths = std::collections::HashSet::new();
        for (offset, import) in script
            .get("imports")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            if let Some(key) = import.get("key").and_then(Value::as_str) {
                if key.ends_with('/') || key.contains("//") || !keys.insert(key) {
                    issue(
                        format!("{base}/imports/{offset}/key"),
                        "import key is invalid or duplicated",
                    );
                }
            }
            if let Some(path) = import.get("path").and_then(Value::as_str) {
                if !script_path(path) || !paths.insert(path.to_ascii_lowercase()) {
                    issue(
                        format!("{base}/imports/{offset}/path"),
                        "import path is invalid or collides under portable case folding",
                    );
                }
            }
        }
    }
}

/// `/Mount/Path/Asset.Asset` style object path of a texture.
fn texture_path_is_valid(path: &str) -> bool {
    let segments = path.split('/').skip(1).collect::<Vec<_>>();
    segments.len() >= 2
        && segments.iter().enumerate().all(|(index, segment)| {
            !segment.is_empty()
                && *segment != ".."
                && segment.chars().all(|ch| {
                    ch.is_ascii_alphanumeric()
                        || ch == '_'
                        || ch == '-'
                        || (ch == '.' && index + 1 == segments.len())
                })
                && (segment.matches('.').count() <= 1)
        })
}

/// Whether a relative path stays inside its folder (lexically).
fn stays_inside(relative: &str) -> bool {
    let path = Path::new(relative);
    if path.is_absolute() || relative.starts_with('/') {
        return false;
    }
    let mut chars = relative.chars();
    if let (Some(first), Some(':')) = (chars.next(), chars.next()) {
        if first.is_ascii_alphabetic() {
            return false;
        }
    }
    let mut depth = 0i32;
    for component in path.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

/// `pack.icon-*` rules. Existence and symlink checks run only when `pack_dir` is a real folder.
pub(crate) fn check_icon(
    reporter: &Reporter<'_>,
    index: usize,
    document: &Value,
    pack_dir: Option<&Path>,
    result: &mut LintResult,
) {
    let Some(icon) = document
        .get("icon")
        .and_then(Value::as_str)
        .filter(|icon| !icon.is_empty())
    else {
        return;
    };
    let mut emit = |severity: Severity, code: &str, message: String| {
        reporter.emit(result, index, "/icon", severity, code, message);
    };
    if icon.starts_with('/') {
        if !texture_path_is_valid(icon) {
            emit(
                Severity::Warning,
                "pack.icon-path",
                format!("{icon:?} is not a valid texture object path (expected /Mount/Folder/Asset.Asset)"),
            );
        }
        return;
    }
    let normalized = icon.replace('\\', "/");
    if !stays_inside(&normalized) {
        emit(
            Severity::Error,
            "pack.icon-escape",
            "the icon must be a file inside the pack folder".into(),
        );
        return;
    }
    let extension = Path::new(&normalized)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !ICON_EXTENSIONS.contains(&extension.as_str()) {
        emit(
            Severity::Error,
            "pack.icon-format",
            "the icon must be a png, jpg, jpeg, tga or bmp file (or a /Game/... texture path)"
                .into(),
        );
        return;
    }
    let Some(dir) = pack_dir.filter(|dir| dir.is_dir()) else {
        return;
    };
    let candidate = dir.join(&normalized);
    if !candidate.is_file() {
        emit(
            Severity::Error,
            "pack.icon-missing",
            format!("icon file {icon:?} does not exist in the pack folder"),
        );
        return;
    }
    let resolved = dunce::canonicalize(&candidate).ok();
    let root = dunce::canonicalize(dir).ok();
    if let (Some(resolved), Some(root)) = (resolved, root) {
        if !resolved.starts_with(&root) {
            emit(
                Severity::Error,
                "pack.icon-symlink",
                "the icon resolves outside the pack folder through a symbolic link".into(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_paths() {
        assert!(texture_path_is_valid("/Game/UI/Icon.Icon"));
        assert!(texture_path_is_valid(
            "/KDataForge/GenAssets/KDFSample.Gen_Icon"
        ));
        assert!(!texture_path_is_valid("/Game"));
        assert!(!texture_path_is_valid("/Game/UI Icon/Icon.Icon"));
        assert!(!texture_path_is_valid("/Game//Icon"));
        assert!(!texture_path_is_valid("/Game/../Icon"));
        assert!(!texture_path_is_valid("/Game/A.B.C"));
    }

    #[test]
    fn containment() {
        assert!(stays_inside("icon.png"));
        assert!(stays_inside("art/../icon.png"));
        assert!(!stays_inside("../icon.png"));
        assert!(!stays_inside("C:/icon.png"));
        assert!(!stays_inside("art/../../icon.png"));
    }
}
