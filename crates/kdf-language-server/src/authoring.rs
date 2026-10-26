//! Additive v1 authoring requests; existing class/reflection requests keep their wire shapes.
use crate::{docs, state::Backend};
use kdf_validation::{
    authoring::{self, SemanticSnapshot, SnapshotToken},
    evidence::{explain, CONTRACT_VERSION},
    ReflectionIndex,
};
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc};
use std::time::{Duration, Instant};
use tower_lsp::{
    jsonrpc::{Error, ErrorCode, Result},
    lsp_types::*,
};

pub(crate) struct Cache {
    epoch: (u64, u64),
    built: Instant,
    snapshot: Arc<SemanticSnapshot>,
}

pub(crate) fn stale(message: impl Into<String>) -> Error {
    Error {
        code: ErrorCode::ServerError(-32801),
        message: message.into().into(),
        data: None,
    }
}

fn position(value: &Value) -> Result<kdf_validation::Position> {
    serde_json::from_value(value.clone()).map_err(|_| {
        Error::invalid_params("position requires zero-based UTF-16 line and character")
    })
}

fn snapshot_param(params: &Value) -> Result<Option<SnapshotToken>> {
    params
        .get("snapshot")
        .map(|value| {
            serde_json::from_value(value.clone())
                .map_err(|error| Error::invalid_params(format!("invalid snapshot: {error}")))
        })
        .transpose()
}

fn document_path(params: &Value) -> Result<std::path::PathBuf> {
    params
        .pointer("/textDocument/uri")
        .and_then(Value::as_str)
        .and_then(|uri| Url::parse(uri).ok())
        .and_then(|uri| docs::path_of(&uri))
        .map(|path| docs::canonical(&path))
        .ok_or_else(|| Error::invalid_params("textDocument.uri must be a workspace file URI"))
}

fn check_params(snapshot: &SemanticSnapshot, params: &Value) -> Result<()> {
    if let Some(expected) = snapshot_param(params)? {
        snapshot.require_current(&expected).map_err(stale)?;
    }
    if params.get("textDocument").is_some() {
        let path = document_path(params)?;
        let document = snapshot
            .documents
            .iter()
            .find(|document| docs::key(&document.path) == docs::key(&path))
            .ok_or_else(|| {
                Error::invalid_params("document is not in the authored workspace snapshot")
            })?;
        if let Some(value) = params.pointer("/textDocument/version") {
            let version: Option<i32> = serde_json::from_value(value.clone()).map_err(|_| {
                Error::invalid_params("document version must be an integer or null")
            })?;
            if version != document.version {
                return Err(stale("document version changed"));
            }
        }
    }
    Ok(())
}

fn has_snapshot_guard(params: &Value) -> bool {
    params.get("snapshot").is_some() || params.get("textDocument").is_some()
}

impl Backend {
    pub(crate) fn epoch(&self) -> (u64, u64) {
        (
            self.state.revision.load(Ordering::SeqCst),
            self.state.generation.load(Ordering::SeqCst),
        )
    }

    pub(crate) fn check_epoch(&self, epoch: (u64, u64)) -> Result<()> {
        if self.epoch() != epoch {
            Err(stale("workspace changed while request was running"))
        } else {
            Ok(())
        }
    }

    pub(crate) async fn semantic_snapshot(
        &self,
        force: bool,
    ) -> Result<(Arc<SemanticSnapshot>, Arc<ReflectionIndex>, (u64, u64))> {
        let root = self
            .root()
            .await
            .ok_or_else(|| Error::invalid_params("no workspace root"))?;
        let mut reflection = self.reflection(&Value::Null).await;
        if force {
            let verify = reflection.clone();
            let current = tokio::task::spawn_blocking(move || verify.archive_is_current())
                .await
                .map_err(|_| Error::internal_error())?;
            if !current {
                self.invalidate_project();
                reflection = self.reflection(&Value::Null).await;
            }
        }
        let epoch = self.epoch();
        // A single build is shared across concurrent requests. Cancellation drops the waiter;
        // any blocking read still finishing cannot publish an obsolete cache entry.
        let mut cache = self.state.authoring.lock().await;
        if !force {
            if let Some(cache) = cache.as_ref().filter(|cache| {
                cache.epoch == epoch
                    && cache.snapshot.root == root
                    && cache.snapshot.snapshot.reflection_digest == reflection.digest
                    && cache.built.elapsed() < Duration::from_secs(2)
            }) {
                return Ok((cache.snapshot.clone(), reflection, epoch));
            }
        }
        let inputs = self.versioned_inputs().await;
        self.check_epoch(epoch)?;
        let index = reflection.clone();
        let built = tokio::task::spawn_blocking(move || {
            SemanticSnapshot::load(&root, &inputs, &index, epoch.0.saturating_add(epoch.1))
        })
        .await
        .map_err(|error| Error::invalid_params(format!("snapshot task failed: {error}")))?
        .map_err(Error::invalid_params)?;
        self.check_epoch(epoch)?;
        let built = Arc::new(built);
        *cache = Some(Cache {
            epoch,
            built: Instant::now(),
            snapshot: built.clone(),
        });
        Ok((built, reflection, epoch))
    }

    pub(crate) async fn export_snapshot(&self, params: Value) -> Result<Value> {
        let (snapshot, _, epoch) = self.semantic_snapshot(true).await?;
        check_params(&snapshot, &params)?;
        self.check_epoch(epoch)?;
        serde_json::to_value(&*snapshot).map_err(|_| Error::internal_error())
    }

    pub(crate) async fn content_symbols(&self, params: Value) -> Result<Value> {
        let (snapshot, _, epoch) = self.semantic_snapshot(has_snapshot_guard(&params)).await?;
        check_params(&snapshot, &params)?;
        let path = params
            .get("textDocument")
            .map(|_| document_path(&params))
            .transpose()?;
        let limit = match params.get("limit") {
            Some(value) => value
                .as_u64()
                .ok_or_else(|| Error::invalid_params("limit must be a nonnegative integer"))?
                .clamp(1, 200) as usize,
            None => 200,
        };
        let mut items: Vec<_> = snapshot
            .symbols
            .iter()
            .filter(|symbol| {
                path.as_ref()
                    .is_none_or(|path| docs::key(&symbol.path) == docs::key(path))
            })
            .take(limit + 1)
            .collect();
        let truncated = items.len() > limit;
        items.truncate(limit);
        self.check_epoch(epoch)?;
        Ok(
            json!({"schemaVersion": CONTRACT_VERSION, "snapshot": snapshot.snapshot, "items": items,
            "complete": snapshot.complete, "isIncomplete": truncated || !snapshot.complete}),
        )
    }

    pub(crate) async fn content_completion(&self, params: Value) -> Result<Value> {
        let request_generation = params
            .get("requestGeneration")
            .map(|value| {
                value.as_u64().ok_or_else(|| {
                    Error::invalid_params("requestGeneration must be a nonnegative integer")
                })
            })
            .transpose()?;
        if let Some(generation) = request_generation {
            let latest = self
                .state
                .completion_generation
                .fetch_max(generation, Ordering::SeqCst);
            if generation < latest {
                return Err(stale("completion generation superseded"));
            }
        }
        let query: authoring::CompletionQuery = serde_json::from_value(params.clone())
            .map_err(|error| Error::invalid_params(error.to_string()))?;
        let (snapshot, reflection, epoch) =
            self.semantic_snapshot(has_snapshot_guard(&params)).await?;
        check_params(&snapshot, &params)?;
        let page = tokio::task::spawn_blocking(move || {
            authoring::complete(&snapshot, &reflection, &query)
        })
        .await
        .map_err(|_| Error::internal_error())?
        .map_err(Error::invalid_params)?;
        self.check_epoch(epoch)?;
        if request_generation.is_some_and(|generation| {
            generation != self.state.completion_generation.load(Ordering::SeqCst)
        }) {
            return Err(stale("completion generation superseded"));
        }
        let mut result = serde_json::to_value(page).map_err(|_| Error::internal_error())?;
        result["requestGeneration"] = json!(request_generation);
        Ok(result)
    }

    pub(crate) async fn rename_content(&self, params: Value) -> Result<Value> {
        if snapshot_param(&params)?.is_none() || params.pointer("/textDocument/version").is_none() {
            return Err(Error::invalid_params(
                "rename requires snapshot and textDocument.version (null for a closed document)",
            ));
        }
        let (snapshot, _, epoch) = self.semantic_snapshot(true).await?;
        check_params(&snapshot, &params)?;
        let path = document_path(&params)?;
        let at = position(&params["position"])?;
        let new_name = params
            .get("newName")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid_params("newName is required"))?;
        let plan = snapshot
            .rename(&snapshot.snapshot, &path, at, new_name)
            .map_err(Error::invalid_params)?;
        self.check_epoch(epoch)?;
        // A read-only plan does not own the filesystem. Recheck closed files, including manifests,
        // immediately before returning; the host must retain the returned digest/version guards.
        let (current, _, _) = self.semantic_snapshot(true).await?;
        current.require_current(&plan.snapshot).map_err(stale)?;
        let edit = self.plan_workspace_edit(&plan).await?;
        self.check_epoch(epoch)?;
        let mut result = serde_json::to_value(plan).map_err(|_| Error::internal_error())?;
        result["workspaceEdit"] = json!(edit);
        Ok(result)
    }

    pub(crate) async fn plan_workspace_edit(
        &self,
        plan: &authoring::RenamePlan,
    ) -> Result<WorkspaceEdit> {
        let open = self.open_urls().await;
        let edits = plan
            .documents
            .iter()
            .map(|document| {
                let uri = open
                    .get(&docs::key(&document.path))
                    .cloned()
                    .or_else(|| Url::from_file_path(&document.path).ok())
                    .ok_or_else(|| Error::invalid_params("invalid document path"))?;
                Ok(TextDocumentEdit {
                    text_document: OptionalVersionedTextDocumentIdentifier {
                        uri,
                        version: document.version,
                    },
                    edits: document
                        .edits
                        .iter()
                        .map(|edit| {
                            OneOf::Left(TextEdit {
                                range: lsp_range(edit.range),
                                new_text: edit.new_text.clone(),
                            })
                        })
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(WorkspaceEdit {
            document_changes: Some(DocumentChanges::Edits(edits)),
            ..Default::default()
        })
    }

    pub(crate) async fn effective_schema(&self, params: Value) -> Result<Value> {
        let kind = params
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid_params("kind is required"))?
            .to_owned();
        let (snapshot, reflection, epoch) =
            self.semantic_snapshot(has_snapshot_guard(&params)).await?;
        check_params(&snapshot, &params)?;
        let options = self.authoring_options(reflection).await;
        let mut result = tokio::task::spawn_blocking(move || {
            kdf_validation::SchemaRegistry::new(&options).and_then(|registry| {
                registry
                    .export(&kind)
                    .ok_or_else(|| "unknown schema kind".into())
            })
        })
        .await
        .map_err(|_| Error::internal_error())?
        .map_err(Error::invalid_params)?;
        self.check_epoch(epoch)?;
        result["snapshot"] = json!(snapshot.snapshot);
        Ok(result)
    }

    pub(crate) async fn explain(&self, params: Value) -> Result<Value> {
        let (snapshot, reflection, epoch) = self.semantic_snapshot(true).await?;
        check_params(&snapshot, &params)?;
        let options = self.authoring_options(reflection).await;
        let inputs: Vec<_> = snapshot
            .documents
            .iter()
            .map(|document| kdf_validation::FileBuffer {
                path: document.path.clone(),
                text: document.text.clone(),
            })
            .collect();
        let root = snapshot.root.clone();
        let lint = tokio::task::spawn_blocking(move || {
            kdf_validation::lint_workspace(&root, &inputs, &options)
        })
        .await
        .map_err(|_| Error::internal_error())?
        .map_err(Error::invalid_params)?;
        self.check_epoch(epoch)?;
        let path = params
            .get("textDocument")
            .map(|_| document_path(&params))
            .transpose()?;
        let at = params.get("position").map(position).transpose()?;
        let rule = params.get("ruleId").and_then(Value::as_str);
        let id = params.get("explanationId").and_then(Value::as_str);
        let records: Vec<_> = lint
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                path.as_ref().is_none_or(|path| {
                    diagnostic
                        .file
                        .as_ref()
                        .is_some_and(|file| docs::key(file) == docs::key(path))
                }) && rule.is_none_or(|rule| diagnostic.code == rule)
                    && at.is_none_or(|at| authoring::contains(diagnostic.range, at))
            })
            .map(explain)
            .filter(|record| id.is_none_or(|id| record.id == id))
            .collect();
        Ok(
            json!({"schemaVersion": CONTRACT_VERSION, "snapshot": snapshot.snapshot, "positionEncoding": "utf-16", "explanations": records}),
        )
    }

    /// Recompute evidence from trusted lint inputs. Client-provided diagnostic data cannot authorize edits.
    pub(crate) async fn reflection_fixes(
        &self,
        uri: &Url,
        diagnostics: &[Diagnostic],
    ) -> Result<Vec<CodeActionOrCommand>> {
        if !self.caps().versioned_edits {
            return Ok(vec![]);
        }
        let (snapshot, reflection, epoch) = self.semantic_snapshot(true).await?;
        let Some(path) = docs::path_of(uri).map(|path| docs::canonical(&path)) else {
            return Ok(vec![]);
        };
        let Some(document) = snapshot
            .documents
            .iter()
            .find(|document| docs::key(&document.path) == docs::key(&path))
        else {
            return Ok(vec![]);
        };
        let version = document.version;
        // Versionless quick fixes cannot be protected against unsaved client changes.
        if version.is_none() {
            return Ok(vec![]);
        }
        let text = document.text.clone();
        let options = self.authoring_options(reflection).await;
        let lint = tokio::task::spawn_blocking(move || {
            kdf_validation::validate_document(&path, &text, &options)
        })
        .await
        .map_err(|_| Error::internal_error())?
        .map_err(Error::invalid_params)?;
        self.check_epoch(epoch)?;
        let mut actions = Vec::new();
        for item in lint.diagnostics {
            let Some(replacement) = item
                .evidence
                .as_ref()
                .and_then(|evidence| evidence.replacement.as_ref())
            else {
                continue;
            };
            let range = lsp_range(item.range);
            let Some(diagnostic) = diagnostics.iter().find(|diagnostic| {
                diagnostic.range == range
                    && diagnostic.code == Some(NumberOrString::String(item.code.clone()))
                    && diagnostic.message == item.message
            }) else {
                continue;
            };
            let edit = WorkspaceEdit {
                document_changes: Some(DocumentChanges::Edits(vec![TextDocumentEdit {
                    text_document: OptionalVersionedTextDocumentIdentifier {
                        uri: uri.clone(),
                        version,
                    },
                    edits: vec![OneOf::Left(TextEdit {
                        range,
                        new_text: replacement.new_name.clone(),
                    })],
                }])),
                ..Default::default()
            };
            actions.push(CodeActionOrCommand::CodeAction(CodeAction { title: format!("Replace with verified {}", replacement.new_name),
                kind: Some(CodeActionKind::QUICKFIX), diagnostics: Some(vec![diagnostic.clone()]), edit: Some(edit),
                data: Some(json!({"schemaVersion": CONTRACT_VERSION, "snapshot": snapshot.snapshot, "evidence": item.evidence})), ..Default::default() }));
        }
        Ok(actions)
    }
}

pub(crate) fn lsp_range(range: kdf_validation::Range) -> Range {
    Range {
        start: Position {
            line: range.start.line,
            character: range.start.character,
        },
        end: Position {
            line: range.end.line,
            character: range.end.character,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::LspService;

    #[tokio::test]
    async fn a_newer_completion_generation_cancels_an_inflight_older_result() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("pack.yml"),
            "ref: Test\nname: Test\nversion: 1.0.0\n",
        )
        .unwrap();
        let mut backend = None;
        let (_service, _socket) = LspService::new(|client| {
            let instance = Backend::new(client);
            backend = Some(instance.clone());
            instance
        });
        let backend = backend.unwrap();
        *backend.state.root.lock().await = Some(root.path().to_path_buf());
        let gate = backend.state.authoring.lock().await;
        let old = backend.clone();
        let old = tokio::spawn(async move {
            old.content_completion(json!({"kind":"content","requestGeneration":1}))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while backend.state.completion_generation.load(Ordering::SeqCst) < 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let new = backend.clone();
        let new = tokio::spawn(async move {
            new.content_completion(json!({"kind":"content","requestGeneration":2}))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while backend.state.completion_generation.load(Ordering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(gate);
        assert_eq!(
            old.await.unwrap().unwrap_err().code,
            ErrorCode::ServerError(-32801)
        );
        assert_eq!(new.await.unwrap().unwrap()["requestGeneration"], 2);
    }
}
