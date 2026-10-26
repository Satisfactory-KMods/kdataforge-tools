//! Project-scoped, bounded context for AI clients. No function in this module writes to disk.

use crate::{
    validate_document, ClassSummary, LintOptions, LintResult, ReflectionIndex, ReflectionStatus,
};
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use walkdir::WalkDir;

const MAX_SCAN_FILES: usize = 10_000;
const MAX_YAML_BYTES: u64 = 1024 * 1024;
const MAX_RESULTS: usize = 50;
const MAX_EXCERPT_CHARS: usize = 1200;

#[derive(Clone)]
pub struct AssistContext {
    root: PathBuf,
    data_roots: Vec<PathBuf>,
    datapack: Arc<ReflectionIndex>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistOverview {
    pub project_name: String,
    pub packs: Vec<AssistPack>,
    pub class_count: usize,
    pub datapack_status: ReflectionStatus,
    pub datapack_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistPack {
    pub path: String,
    pub reference: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistDocumentHit {
    pub path: String,
    pub root_type: Option<String>,
    pub ids: Vec<String>,
    pub excerpt: String,
}

impl AssistContext {
    pub fn open(project_root: &Path, datapack_path: Option<&Path>) -> Result<Self, String> {
        let root = dunce::canonicalize(project_root)
            .map_err(|error| format!("Cannot open project: {error}"))?;
        if !root.is_dir() {
            return Err("Project root is not a directory".into());
        }
        let data_roots = discover_data_roots(&root)?;
        let datapack = datapack_path
            .map(ReflectionIndex::load)
            .unwrap_or_else(|| ReflectionIndex::unavailable("No datapack is bound"));
        Ok(Self {
            root,
            data_roots,
            datapack: Arc::new(datapack),
        })
    }

    pub fn overview(&self) -> Result<AssistOverview, String> {
        let mut packs = Vec::new();
        for entry in self.yaml_files()? {
            if entry.file_name().and_then(|name| name.to_str()) != Some("pack.yml") {
                continue;
            }
            let text = fs::read_to_string(&entry).map_err(|error| error.to_string())?;
            let reference = serde_yaml::from_str::<Value>(&text)
                .ok()
                .and_then(|value| value.get("ref")?.as_str().map(str::to_owned));
            packs.push(AssistPack {
                path: self.relative_path(&entry)?,
                reference,
            });
        }
        Ok(AssistOverview {
            project_name: self
                .root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Project".into()),
            packs,
            class_count: self.datapack.classes.len(),
            datapack_status: self.datapack.status.clone(),
            datapack_reason: self.datapack.reason.clone(),
        })
    }

    pub fn search_classes(&self, query: &str, limit: usize) -> Vec<ClassSummary> {
        let query = query.trim().to_lowercase();
        self.datapack
            .classes
            .iter()
            .filter(|class| {
                query.is_empty()
                    || class.path.to_lowercase().contains(&query)
                    || class.name.to_lowercase().contains(&query)
                    || class
                        .source
                        .as_deref()
                        .is_some_and(|source| source.to_lowercase().contains(&query))
            })
            .take(limit.clamp(1, MAX_RESULTS))
            .cloned()
            .collect()
    }

    pub fn class_info(&self, path: &str) -> Option<Value> {
        self.datapack.class_info(path).cloned()
    }

    pub fn search_documents(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<AssistDocumentHit>, String> {
        let query = query.trim().to_lowercase();
        let mut results = Vec::new();
        for path in self.yaml_files()? {
            let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
            let relative = self.relative_path(&path)?;
            if !query.is_empty()
                && !relative.to_lowercase().contains(&query)
                && !text.to_lowercase().contains(&query)
            {
                continue;
            }
            let (root_type, ids) = document_metadata(&text);
            results.push(AssistDocumentHit {
                path: relative,
                root_type,
                ids,
                excerpt: text.chars().take(MAX_EXCERPT_CHARS).collect(),
            });
            if results.len() >= limit.clamp(1, MAX_RESULTS) {
                break;
            }
        }
        Ok(results)
    }

    pub fn read_yaml(&self, relative_path: &str) -> Result<String, String> {
        let path = self.resolve_yaml(relative_path, false)?;
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        if metadata.len() > MAX_YAML_BYTES {
            return Err("YAML file exceeds the 1 MiB context limit".into());
        }
        fs::read_to_string(path).map_err(|error| error.to_string())
    }

    pub fn validate_draft(&self, relative_path: &str, text: &str) -> Result<LintResult, String> {
        if text.len() as u64 > MAX_YAML_BYTES {
            return Err("YAML draft exceeds the 1 MiB context limit".into());
        }
        let path = self.resolve_yaml(relative_path, true)?;
        validate_document(
            &path,
            text,
            &LintOptions {
                datapack_index: Some(Arc::clone(&self.datapack)),
                ..Default::default()
            },
        )
    }

    fn yaml_files(&self) -> Result<Vec<PathBuf>, String> {
        let mut files = Vec::new();
        for data_root in &self.data_roots {
            for entry in WalkDir::new(data_root)
                .follow_links(false)
                .max_depth(20)
                .into_iter()
                .filter_entry(crate::runtime_authored_entry)
            {
                let entry = entry.map_err(|error| error.to_string())?;
                if !entry.file_type().is_file() || !is_yaml(entry.path()) {
                    continue;
                }
                if entry.metadata().map_err(|error| error.to_string())?.len() > MAX_YAML_BYTES {
                    continue;
                }
                let canonical =
                    dunce::canonicalize(entry.path()).map_err(|error| error.to_string())?;
                if !canonical.starts_with(data_root) {
                    continue;
                }
                files.push(canonical);
                if files.len() > MAX_SCAN_FILES {
                    return Err("Project contains too many YAML files for AI context".into());
                }
            }
        }
        files.sort();
        files.dedup();
        Ok(files)
    }

    fn resolve_yaml(&self, relative_path: &str, allow_new: bool) -> Result<PathBuf, String> {
        let relative = Path::new(relative_path);
        if relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
            || !is_yaml(relative)
        {
            return Err("Expected a project-relative .yml or .yaml path".into());
        }
        let candidate = self.root.join(relative);
        let parent = dunce::canonicalize(candidate.parent().ok_or("YAML path has no parent")?)
            .map_err(|error| format!("YAML parent directory is unavailable: {error}"))?;
        if !self.data_roots.iter().any(|root| {
            parent.starts_with(root) && crate::is_runtime_authored_path(&candidate, root)
        }) {
            return Err("YAML path resolves outside the selected DataForge root".into());
        }
        if allow_new && !candidate.exists() {
            return Ok(candidate);
        }
        let canonical = dunce::canonicalize(candidate).map_err(|error| error.to_string())?;
        if !self
            .data_roots
            .iter()
            .any(|root| crate::is_runtime_authored_path(&canonical, root))
            || !canonical.is_file()
        {
            return Err("YAML path is not a file inside the selected DataForge root".into());
        }
        Ok(canonical)
    }

    fn relative_path(&self, path: &Path) -> Result<String, String> {
        path.strip_prefix(&self.root)
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .map_err(|error| error.to_string())
    }
}

fn discover_data_roots(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut roots = Vec::new();
    if root.join("DataForge").is_dir() {
        roots.push(dunce::canonicalize(root.join("DataForge")).map_err(|error| error.to_string())?);
    } else if root.join("pack.yml").is_file()
        || root.file_name().and_then(|name| name.to_str()) == Some("DataForge")
    {
        roots.push(root.to_path_buf());
    } else {
        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(8)
            .into_iter()
            .filter_entry(|entry| {
                crate::runtime_authored_entry(entry)
                    && (!entry.file_type().is_dir()
                        || !matches!(
                            entry.file_name().to_str(),
                            Some(".git" | "node_modules" | "target" | "Binaries" | "Intermediate")
                        ))
            })
        {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry.file_type().is_dir() && entry.file_name() == "DataForge" {
                let canonical =
                    dunce::canonicalize(entry.path()).map_err(|error| error.to_string())?;
                if canonical.starts_with(root) {
                    roots.push(canonical);
                }
                if roots.len() > 100 {
                    return Err("Project has too many DataForge roots for AI context".into());
                }
            }
        }
    }
    roots.sort();
    roots.dedup();
    Ok(roots)
}

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "yml" | "yaml"))
}

fn document_metadata(text: &str) -> (Option<String>, Vec<String>) {
    let mut root_type = None;
    let mut ids = Vec::new();
    for document in serde_yaml::Deserializer::from_str(text).take(8) {
        let Ok(value) = Value::deserialize(document) else {
            break;
        };
        root_type.get_or_insert_with(|| {
            value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned()
        });
        if let Some(map) = value.as_object() {
            for entries in map.values().filter_map(Value::as_array) {
                for id in entries.iter().filter_map(|entry| entry.get("id")) {
                    if let Some(id) = id.as_str() {
                        if ids.len() < 100 {
                            ids.push(id.to_owned());
                        }
                    }
                }
            }
        }
    }
    (root_type, ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_project_yaml_and_rejects_paths_outside_dataforge() {
        let project = tempfile::tempdir().unwrap();
        let data = project.path().join("DataForge").join("Example");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("pack.yml"), "ref: Example\n").unwrap();
        fs::write(
            data.join("recipe.yml"),
            "type: recipe\nrecipes:\n  - id: SuperIngot\n",
        )
        .unwrap();
        fs::write(project.path().join("secret.yml"), "token: private\n").unwrap();
        let context = AssistContext::open(project.path(), None).unwrap();

        let hits = context.search_documents("SuperIngot", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].ids, ["SuperIngot"]);
        assert_eq!(
            context.overview().unwrap().packs[0].reference.as_deref(),
            Some("Example")
        );
        assert!(context.read_yaml("secret.yml").is_err());
        assert!(context.read_yaml("../secret.yml").is_err());
        assert!(context
            .validate_draft("../secret.yml", "type: recipe")
            .is_err());
    }

    #[test]
    fn does_not_expose_unrelated_yaml_when_project_has_no_pack() {
        let project = tempfile::tempdir().unwrap();
        fs::write(project.path().join("secret.yml"), "token: private\n").unwrap();
        let context = AssistContext::open(project.path(), None).unwrap();
        assert!(context.search_documents("private", 10).unwrap().is_empty());
        assert!(context.read_yaml("secret.yml").is_err());
    }
}
