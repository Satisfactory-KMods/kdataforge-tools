//! Read-only comparison of current and candidate reflection bindings.

use crate::{
    canonical_file_path, lint_workspace, Diagnostic, FileBuffer, LintOptions, LintResult,
    ReflectionIndex, ReflectionStatus, Severity,
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use walkdir::WalkDir;

const MAX_YAML_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct LintSummary {
    pub ok: bool,
    pub packs: usize,
    pub documents: usize,
    pub errors: usize,
    pub warnings: usize,
}

impl From<&LintResult> for LintSummary {
    fn from(result: &LintResult) -> Self {
        Self {
            ok: result.ok,
            packs: result.packs,
            documents: result.documents,
            errors: result.errors,
            warnings: result.warnings,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticChange {
    pub identity: String,
    /// Root-relative path for stable display across machines and worktrees.
    pub path: String,
    pub diagnostic: Diagnostic,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingPreview {
    pub baseline: LintSummary,
    pub candidate: LintSummary,
    pub added: Vec<DiagnosticChange>,
    pub removed: Vec<DiagnosticChange>,
    pub unchanged: usize,
    pub candidate_status: ReflectionStatus,
    pub candidate_reason: Option<String>,
}

/// Compares two bindings against one stable YAML snapshot. Nothing is written to the project.
pub fn preview_binding(
    root: &Path,
    buffers: &[FileBuffer],
    candidate_path: &Path,
    options: &LintOptions,
) -> Result<BindingPreview, String> {
    let root = dunce::canonicalize(root)
        .map_err(|error| format!("cannot resolve project root {}: {error}", root.display()))?;
    let archive = ReflectionIndex::load(candidate_path);
    if !matches!(
        archive.status,
        ReflectionStatus::Ready | ReflectionStatus::Limited
    ) {
        return Err(archive
            .reason
            .unwrap_or_else(|| "candidate archive has no usable KDF reflection data".into()));
    }
    let candidate_status = archive.status.clone();
    let candidate_reason = archive.reason.clone();
    let disk_before = capture_yaml(&root)?;
    let mut snapshot = disk_before.clone();
    for buffer in buffers {
        let path = if buffer.path.is_absolute() {
            buffer.path.clone()
        } else {
            root.join(&buffer.path)
        };
        let path = canonical_file_path(&path).map_err(|error| {
            format!(
                "cannot resolve unsaved document {}: {error}",
                path.display()
            )
        })?;
        if !path.starts_with(&root) {
            return Err(format!(
                "unsaved buffer is outside project root: {}",
                path.display()
            ));
        }
        if buffer.text.len() > MAX_YAML_BYTES as usize {
            return Err(format!("unsaved YAML exceeds 8 MiB: {}", path.display()));
        }
        if is_yaml(&path) {
            snapshot.insert(path, buffer.text.clone());
        }
    }
    let shared_buffers = snapshot
        .into_iter()
        .map(|(path, text)| FileBuffer { path, text })
        .collect::<Vec<_>>();
    let config_before = binding_config(&root);
    let baseline = lint_workspace(&root, &shared_buffers, options)?;
    let mut candidate_options = options.clone();
    candidate_options.datapack_path = Some(candidate_path.to_path_buf());
    candidate_options.datapack_index = Some(Arc::new(archive));
    let candidate = lint_workspace(&root, &shared_buffers, &candidate_options)?;
    if disk_before != capture_yaml(&root)? || config_before != binding_config(&root) {
        return Err("project changed during binding preview; retry with current files".into());
    }
    Ok(diff_results(
        &root,
        baseline,
        candidate,
        candidate_status,
        candidate_reason,
    ))
}

fn capture_yaml(root: &Path) -> Result<BTreeMap<PathBuf, String>, String> {
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(crate::runtime_authored_entry)
    {
        let entry =
            entry.map_err(|error| format!("cannot inspect project during preview: {error}"))?;
        if !entry.file_type().is_file() || !is_yaml(entry.path()) {
            continue;
        }
        let meta = entry
            .metadata()
            .map_err(|error| format!("cannot inspect {}: {error}", entry.path().display()))?;
        if meta.len() > MAX_YAML_BYTES {
            return Err(format!(
                "YAML file exceeds 8 MiB: {}",
                entry.path().display()
            ));
        }
        let text = fs::read_to_string(entry.path())
            .map_err(|error| format!("cannot read {}: {error}", entry.path().display()))?;
        files.insert(entry.into_path(), text);
    }
    Ok(files)
}

fn binding_config(root: &Path) -> Option<String> {
    let project = if root.file_name().and_then(|s| s.to_str()) == Some("DataForge") {
        root.parent()?
    } else {
        root
    };
    fs::read_to_string(project.join(".kdf-editor.toml")).ok()
}

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yml") || ext.eq_ignore_ascii_case("yaml"))
}

fn diff_results(
    root: &Path,
    baseline: LintResult,
    candidate: LintResult,
    candidate_status: ReflectionStatus,
    candidate_reason: Option<String>,
) -> BindingPreview {
    let mut old = group_diagnostics(root, baseline.diagnostics.clone());
    let mut new = group_diagnostics(root, candidate.diagnostics.clone());
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut unchanged = 0;
    for key in old
        .keys()
        .chain(new.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
    {
        let before = old.remove(&key).unwrap_or_default();
        let after = new.remove(&key).unwrap_or_default();
        let shared = before.len().min(after.len());
        unchanged += shared;
        removed.extend(before.into_iter().skip(shared));
        added.extend(after.into_iter().skip(shared));
    }
    added.sort_by(|a, b| {
        (&a.path, &a.identity, &a.diagnostic.message).cmp(&(
            &b.path,
            &b.identity,
            &b.diagnostic.message,
        ))
    });
    removed.sort_by(|a, b| {
        (&a.path, &a.identity, &a.diagnostic.message).cmp(&(
            &b.path,
            &b.identity,
            &b.diagnostic.message,
        ))
    });
    BindingPreview {
        baseline: LintSummary::from(&baseline),
        candidate: LintSummary::from(&candidate),
        added,
        removed,
        unchanged,
        candidate_status,
        candidate_reason,
    }
}

fn group_diagnostics(
    root: &Path,
    diagnostics: Vec<Diagnostic>,
) -> BTreeMap<(String, String), Vec<DiagnosticChange>> {
    let mut groups = BTreeMap::new();
    for diagnostic in diagnostics {
        let path = diagnostic
            .file
            .as_ref()
            .and_then(|file| file.strip_prefix(root).ok())
            .map(|file| file.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| "<project>".into());
        let severity = match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Information => "information",
        };
        let stable = json!([
            path,
            diagnostic.document,
            diagnostic.code,
            severity,
            diagnostic.source,
            diagnostic.yaml_path,
            diagnostic.range.start.line,
            diagnostic.range.start.character,
            diagnostic.range.end.line,
            diagnostic.range.end.character
        ]);
        let identity = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&stable).expect("serialize diagnostic identity"))
        );
        let message = diagnostic.message.clone();
        groups
            .entry((identity.clone(), message))
            .or_insert_with(Vec::new)
            .push(DiagnosticChange {
                identity,
                path,
                diagnostic,
            });
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn archive(path: &Path, class_path: &str) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(b"{\"schemaVersion\":1,\"engineVersion\":\"fixture\",\"counts\":{\"classes\":1},\"failedLoads\":[]}").unwrap();
        zip.start_file("classes.ndjson", options).unwrap();
        zip.write_all(format!("{{\"path\":\"{class_path}\",\"name\":\"Example\",\"kind\":\"native\",\"flags\":{{\"names\":[]}},\"properties\":[]}}\n").as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn switch_diff_uses_unsaved_snapshot_and_never_changes_binding() {
        let project = tempfile::tempdir().unwrap();
        let baseline_archive = project.path().join("baseline.kdfdata");
        let candidate_archive = project.path().join("candidate.kdfdata");
        archive(&baseline_archive, "/Script/Test.A");
        archive(&candidate_archive, "/Script/Test.B");
        let config = project.path().join(".kdf-editor.toml");
        fs::write(
            &config,
            format!(
                "[datapack]\npath = {:?}\n",
                baseline_archive.to_string_lossy()
            ),
        )
        .unwrap();
        fs::write(
            project.path().join("pack.yml"),
            "ref: Pack\nname: Pack\nversion: 1.0.0\n",
        )
        .unwrap();
        let document = project.path().join("item.unlock.yml");
        let disk = "type: unlock\nunlocks:\n  - id: One\n    parent: /Script/Test.A\n";
        let unsaved = "type: unlock\nunlocks:\n  - id: One\n    parent: /Script/Test.B\n";
        fs::write(&document, disk).unwrap();
        let config_before = fs::read(&config).unwrap();
        let archive_before = fs::read(&baseline_archive).unwrap();
        let disk_preview = preview_binding(
            project.path(),
            &[],
            &candidate_archive,
            &LintOptions::default(),
        )
        .unwrap();
        assert_eq!(
            (disk_preview.added.len(), disk_preview.removed.len()),
            (1, 0)
        );
        assert_eq!(
            disk_preview.added[0].diagnostic.code,
            "datapack.unknown-class"
        );
        assert_eq!(disk_preview.added[0].path, "item.unlock.yml");
        let repeated = preview_binding(
            project.path(),
            &[],
            &candidate_archive,
            &LintOptions::default(),
        )
        .unwrap();
        assert_eq!(disk_preview.added[0].identity, repeated.added[0].identity);
        let live_preview = preview_binding(
            project.path(),
            &[FileBuffer {
                path: document.clone(),
                text: unsaved.into(),
            }],
            &candidate_archive,
            &LintOptions::default(),
        )
        .unwrap();
        assert_eq!(
            (live_preview.added.len(), live_preview.removed.len()),
            (0, 1)
        );
        assert_eq!(fs::read(&config).unwrap(), config_before);
        assert_eq!(fs::read_to_string(&document).unwrap(), disk);
        assert_eq!(fs::read(&baseline_archive).unwrap(), archive_before);
    }

    #[test]
    fn rejects_kdata_extension() {
        let project = tempfile::tempdir().unwrap();
        fs::write(
            project.path().join("pack.yml"),
            "ref: Pack\nname: Pack\nversion: 1.0.0\n",
        )
        .unwrap();
        let candidate = project.path().join("legacy.kdata");
        let mut zip = zip::ZipWriter::new(fs::File::create(&candidate).unwrap());
        zip.start_file(
            "catalog/parts.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"[]").unwrap();
        zip.finish().unwrap();
        assert!(
            preview_binding(project.path(), &[], &candidate, &LintOptions::default())
                .unwrap_err()
                .contains(".kdfdata")
        );
    }
}
