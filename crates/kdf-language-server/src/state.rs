use crate::docs::{self, Buffers, Sources};
use crate::workspace::{affects_index, Workspace, WorkspaceIndexer};
use kdf_validation::{
    bound_datapack_path, clamped_byte_offset_at_utf16, datapack_selection,
    effective_datapack_override, lint_workspace, validate_document, Advisory, ColdPropertyLevel,
    Diagnostic as KdfDiagnostic, FileBuffer, LintSettings, LintTrigger, Position as KdfPosition,
    Profile, ReflectionIndex, Severity, OPERATIONS,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{watch, Mutex, OnceCell};
use tower_lsp::jsonrpc::{Error as RpcError, Result};
use tower_lsp::lsp_types::{
    DiagnosticSeverity, MessageType, NumberOrString, Position, Range,
    TextDocumentContentChangeEvent, Url,
};
use tower_lsp::Client;

#[cfg(all(test, windows))]
#[path = "document_identity_tests.rs"]
mod document_identity_tests;

/// What the client told us it can render (`initialize` capabilities) and the host options that
/// switch optional behaviour (`initializationOptions.kdf`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClientCaps {
    /// Versioned WorkspaceEdit support; required for automatic content/reflection edits.
    pub(crate) versioned_edits: bool,
    /// `completionItem.snippetSupport`.
    pub(crate) snippets: bool,
    /// Completion documentation can be markdown (`completionItem.documentationFormat`).
    pub(crate) completion_markdown: bool,
    /// The client resolves `documentation` lazily (`completionItem.resolveSupport`).
    pub(crate) resolve_documentation: bool,
    /// Hover contents can be markdown (`hover.contentFormat`).
    pub(crate) hover_markdown: bool,
    /// Signature documentation can be markdown (`signatureInformation.documentationFormat`).
    pub(crate) signature_markdown: bool,
    /// Parameter labels may be `[start, end]` offsets (`parameterInformation.labelOffsetSupport`).
    pub(crate) label_offsets: bool,
    /// `signatureInformation.activeParameterSupport`.
    pub(crate) signature_active_parameter: bool,
    /// Items may carry the Monaco / VS Code commands `editor.action.triggerSuggest` and
    /// `editor.action.triggerParameterHints` (`initializationOptions.kdf.clientCommands`, default
    /// true: the desktop client sends no options and relies on them).
    pub(crate) client_commands: bool,
}

impl Default for ClientCaps {
    fn default() -> Self {
        Self {
            versioned_edits: false,
            snippets: false,
            completion_markdown: false,
            resolve_documentation: false,
            hover_markdown: false,
            signature_markdown: false,
            label_offsets: false,
            signature_active_parameter: false,
            client_commands: true,
        }
    }
}

/// Host-provided inputs that complement the environment variables and `.kdf-editor.toml`: sent in
/// `initializationOptions.kdf` and updated through `workspace/didChangeConfiguration`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HostOptions {
    /// `datapackPath`: `Some(None)` (an empty text) selects schema-only validation, like an empty
    /// `KDF_EFFECTIVE_DATAPACK_PATH`.
    pub(crate) datapack: Option<Option<PathBuf>>,
    /// `userSettingsPath`: the user `settings.toml` (the `KDF_USER_SETTINGS_PATH` fallback applies
    /// when absent).
    pub(crate) user_settings: Option<PathBuf>,
}

/// The `kdf` section of `initializationOptions` / `didChangeConfiguration.settings` (a flat object
/// with the same keys is accepted too).
fn kdf_section(value: &Value) -> Option<&Value> {
    let object = value.as_object()?;
    if let Some(section) = object.get("kdf").filter(|section| section.is_object()) {
        return Some(section);
    }
    ["datapackPath", "userSettingsPath", "clientCommands"]
        .iter()
        .any(|key| object.contains_key(*key))
        .then_some(value)
}

/// Longest time a workspace snapshot is trusted without looking at the disk.
const DISK_RECHECK: Duration = Duration::from_secs(2);
/// The same once the client has sent watched-file notifications (it tells us about disk changes).
const DISK_RECHECK_WATCHED: Duration = Duration::from_secs(30);
/// `.kdf-editor.toml` and the bound datapack are looked at most this often.
const PROJECT_RECHECK: Duration = Duration::from_millis(500);

/// Workspace snapshot, rebuilt in the background (see [`Backend::workspace`]).
struct WorkspaceCell {
    state: StdMutex<CellState>,
    indexer: Arc<StdMutex<WorkspaceIndexer>>,
    /// Counts finished rebuild rounds; waiters for the first snapshot subscribe before they check.
    rounds: watch::Sender<u64>,
}

#[derive(Default)]
struct CellState {
    snapshot: Option<Arc<Workspace>>,
    /// Root the snapshot was built for.
    root: Option<PathBuf>,
    /// Buffer revision the snapshot reflects.
    revision: u64,
    /// When the disk was last looked at.
    checked: Option<Instant>,
    building: bool,
}

/// Look at the datapack file at most every [`PROJECT_RECHECK`]; one load per file version.
struct ReflectionSlot {
    cell: Arc<OnceCell<Arc<ReflectionIndex>>>,
    stamp: Option<(SystemTime, u64)>,
    /// `None` forces the next request to look at the file.
    checked: Option<Instant>,
}

/// `.kdf-editor.toml` binding, re-read at most every [`PROJECT_RECHECK`].
#[derive(Default)]
struct ProjectCache {
    root: Option<PathBuf>,
    checked: Option<Instant>,
    bound: Option<PathBuf>,
}

/// Coalesces validation requests: one runner, the newest request wins.
struct Validation {
    /// Counts validation requests (unlike `State::generation`, which changed lint settings bump too:
    /// those drop results but do not ask for a new run).
    requested: AtomicU64,
    /// `State::generation` right after the newest request: the run answers that request only while
    /// the buffers and lint settings still are the ones it was made for.
    request_generation: AtomicU64,
    /// Debounce of the newest request.
    delay_ms: AtomicU64,
    /// A runner task is alive.
    running: StdMutex<bool>,
}

pub(crate) struct State {
    pub(crate) authoring: Mutex<Option<crate::authoring::Cache>>,
    pub(crate) completion_generation: AtomicU64,
    pub(crate) root: Mutex<Option<PathBuf>>,
    documents: Mutex<Documents>,
    published: Mutex<HashSet<Url>>,
    /// Bumped by every validation request and by changed lint settings; results of an older
    /// generation are dropped.
    pub(crate) generation: AtomicU64,
    validation: Validation,
    reflection: StdMutex<HashMap<PathBuf, ReflectionSlot>>,
    project: StdMutex<ProjectCache>,
    settings: Mutex<LintSettings>,
    caps: StdMutex<ClientCaps>,
    host: StdMutex<HostOptions>,
    /// Bumped (after the change was applied) whenever a buffer or file the index is built from
    /// changed; the workspace snapshot catches up in the background.
    pub(crate) revision: AtomicU64,
    workspace: WorkspaceCell,
    /// The client sends `workspace/didChangeWatchedFiles`.
    watched_files: AtomicBool,
}

/// What a language feature needs besides the document text.
pub(crate) struct Services {
    pub(crate) reflection: Arc<ReflectionIndex>,
    pub(crate) workspace: Arc<Workspace>,
    pub(crate) caps: ClientCaps,
}

#[derive(Clone, Copy)]
pub(crate) enum ValidationCause {
    Startup,
    Edit,
    Saved,
    Closed,
    SettingsChanged,
    Manual,
}

impl ValidationCause {
    fn enabled(self, trigger: LintTrigger) -> bool {
        matches!(self, Self::Manual)
            || match trigger {
                LintTrigger::OnType => true,
                LintTrigger::OnSave => !matches!(self, Self::Edit),
                LintTrigger::Manual => false,
            }
    }
}

pub(crate) struct OpenDocument {
    pub(crate) file: FileBuffer,
    pub(crate) version: i32,
    /// The URL the client uses for the document; diagnostics are published under it only.
    pub(crate) uri: Url,
}

/// Open documents by [`docs::key`] of their path.
#[derive(Default)]
pub(crate) struct Documents {
    by_key: HashMap<PathBuf, OpenDocument>,
    /// Client path aliases (including Windows 8.3 names) resolved once per open buffer.
    aliases: HashMap<PathBuf, PathBuf>,
}

impl Documents {
    fn key(&mut self, uri: &Url) -> Option<PathBuf> {
        let path = docs::path_of(uri)?;
        let client = docs::key(&path);
        if let Some(key) = self.aliases.get(&client) {
            return Some(key.clone());
        }
        if self.by_key.contains_key(&client) {
            return Some(client);
        }
        // Only a previously unseen spelling consults the filesystem. Active client
        // requests keep their buffer identity even after its parent folder is removed.
        let key = docs::key(&docs::canonical(&path));
        if self.by_key.contains_key(&key) {
            self.aliases.insert(client, key.clone());
        }
        Some(key)
    }

    pub(crate) fn get(&mut self, uri: &Url) -> Option<&OpenDocument> {
        let key = self.key(uri)?;
        self.by_key.get(&key)
    }

    pub(crate) fn buffers(&self) -> Buffers {
        Buffers::from_files(self.by_key.values().map(|document| document.file.clone()))
    }

    pub(crate) fn files(&self) -> Vec<FileBuffer> {
        self.by_key
            .values()
            .map(|document| document.file.clone())
            .collect()
    }
}

/// Applies protocol content changes in order. Positions beyond the end of a line are clamped to
/// the end of that line and lines beyond the document to its end, as the specification demands; a
/// reversed range is read as the range between its ends.
pub(crate) fn apply_content_changes(
    text: &mut String,
    changes: Vec<TextDocumentContentChangeEvent>,
) {
    for change in changes {
        let Some(range) = change.range else {
            *text = change.text;
            continue;
        };
        let offset = |position: Position| {
            clamped_byte_offset_at_utf16(
                text,
                KdfPosition {
                    line: position.line,
                    character: position.character,
                },
            )
        };
        let (start, end) = (offset(range.start), offset(range.end));
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        text.replace_range(start..end, &change.text);
    }
}

#[derive(Clone)]
pub(crate) struct Backend {
    pub(crate) client: Client,
    pub(crate) state: Arc<State>,
}

impl Backend {
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client,
            state: Arc::new(State {
                authoring: Mutex::new(None),
                completion_generation: AtomicU64::new(0),
                root: Mutex::new(None),
                documents: Mutex::new(Documents::default()),
                published: Mutex::new(HashSet::new()),
                generation: AtomicU64::new(0),
                validation: Validation {
                    requested: AtomicU64::new(0),
                    request_generation: AtomicU64::new(0),
                    delay_ms: AtomicU64::new(0),
                    running: StdMutex::new(false),
                },
                reflection: StdMutex::new(HashMap::new()),
                project: StdMutex::new(ProjectCache::default()),
                settings: Mutex::new(LintSettings::default()),
                caps: StdMutex::new(ClientCaps::default()),
                host: StdMutex::new(HostOptions::default()),
                revision: AtomicU64::new(0),
                workspace: WorkspaceCell {
                    state: StdMutex::new(CellState::default()),
                    indexer: Arc::new(StdMutex::new(WorkspaceIndexer::default())),
                    rounds: watch::channel(0).0,
                },
                watched_files: AtomicBool::new(false),
            }),
        }
    }

    // ---- client capabilities and host options ---------------------------------------------

    pub(crate) fn caps(&self) -> ClientCaps {
        *self.state.caps.lock().expect("caps lock")
    }

    pub(crate) fn set_caps(&self, caps: ClientCaps) {
        *self.state.caps.lock().expect("caps lock") = caps;
    }

    /// Applies `initializationOptions` / `didChangeConfiguration.settings`: a configuration push
    /// replaces the host options as a whole. Returns whether the datapack or the user settings
    /// selection changed.
    pub(crate) fn apply_host_options(&self, options: &Value) -> bool {
        let Some(section) = kdf_section(options) else {
            return false;
        };
        let text = |key: &str| section.get(key).and_then(Value::as_str);
        let next = HostOptions {
            datapack: text("datapackPath").map(datapack_selection),
            user_settings: text("userSettingsPath").and_then(datapack_selection),
        };
        if let Some(commands) = section.get("clientCommands").and_then(Value::as_bool) {
            self.state.caps.lock().expect("caps lock").client_commands = commands;
        }
        let mut host = self.state.host.lock().expect("host lock");
        let changed = *host != next;
        *host = next;
        changed
    }

    fn host_options(&self) -> HostOptions {
        self.state.host.lock().expect("host lock").clone()
    }

    // ---- documents -------------------------------------------------------------------------

    pub(crate) async fn open_document(&self, uri: Url, version: i32, text: String) {
        let Some(path) = docs::path_of(&uri) else {
            return;
        };
        let client_key = docs::key(&path);
        // The on-disk spelling is what the validator matches against its own walk.
        let path = docs::canonical(&path);
        let key = docs::key(&path);
        {
            let mut documents = self.state.documents.lock().await;
            documents.aliases.insert(client_key, key.clone());
            documents.by_key.insert(
                key,
                OpenDocument {
                    file: FileBuffer { path, text },
                    version,
                    uri: uri.clone(),
                },
            );
        }
        self.touch_document(&uri);
    }

    /// Applies a `didChange`. `Err` carries the reason a change was ignored.
    pub(crate) async fn change_document(
        &self,
        uri: &Url,
        version: i32,
        changes: Vec<TextDocumentContentChangeEvent>,
    ) -> std::result::Result<(), String> {
        {
            let mut documents = self.state.documents.lock().await;
            let Some(key) = documents.key(uri) else {
                return Err(format!("didChange for a non-file URL {uri}"));
            };
            let Some(document) = documents.by_key.get_mut(&key) else {
                return Err(format!("didChange for a document that is not open: {uri}"));
            };
            if version <= document.version {
                return Err(format!(
                    "ignored stale didChange version {version} for {uri}"
                ));
            }
            apply_content_changes(&mut document.file.text, changes);
            document.version = version;
        }
        // After the buffer changed: a request that sees the new revision also sees the new text.
        self.touch_document(uri);
        Ok(())
    }

    pub(crate) async fn close_document(&self, uri: &Url) {
        {
            let mut documents = self.state.documents.lock().await;
            if let Some(key) = documents.key(uri) {
                documents.by_key.remove(&key);
                documents.aliases.retain(|_, target| target != &key);
            }
        }
        self.touch();
    }

    pub(crate) async fn source(&self, uri: &Url) -> Option<String> {
        self.document(uri).await.map(|(_, text)| text)
    }

    /// Path (in its on-disk spelling) and text of a document: the open buffer, else the file.
    pub(crate) async fn document(&self, uri: &Url) -> Option<(PathBuf, String)> {
        if let Some(document) = self.state.documents.lock().await.get(uri) {
            return Some((document.file.path.clone(), document.file.text.clone()));
        }
        let path = docs::canonical(&docs::path_of(uri)?);
        let text = fs::read_to_string(&path).ok()?;
        Some((path, text))
    }

    pub(crate) async fn buffer_snapshot(&self) -> Buffers {
        self.state.documents.lock().await.buffers()
    }

    pub(crate) async fn versioned_inputs(&self) -> Vec<kdf_validation::authoring::VersionedInput> {
        self.state
            .documents
            .lock()
            .await
            .by_key
            .values()
            .map(|document| kdf_validation::authoring::VersionedInput {
                path: document.file.path.clone(),
                version: Some(document.version),
                text: document.file.text.clone(),
            })
            .collect()
    }

    pub(crate) async fn authoring_options(
        &self,
        reflection: Arc<ReflectionIndex>,
    ) -> kdf_validation::LintOptions {
        let mut options = self
            .state
            .settings
            .lock()
            .await
            .lint_options(Profile::Runtime);
        let schema_only = self.host_options().datapack == Some(None);
        options.no_datapack = schema_only;
        options.datapack_index = (!schema_only
            && self.reflection_path(&Value::Null).await.is_some())
        .then_some(reflection);
        options
    }

    /// Client URLs of the open documents by [`docs::key`] (to name them the way the client does).
    pub(crate) async fn open_urls(&self) -> HashMap<PathBuf, Url> {
        self.state
            .documents
            .lock()
            .await
            .by_key
            .iter()
            .map(|(key, document)| (key.clone(), document.uri.clone()))
            .collect()
    }

    /// Open buffers and a refreshed workspace snapshot, for navigation.
    pub(crate) async fn sources(&self) -> Sources {
        let workspace = self.fresh_workspace().await;
        Sources::new(self.buffer_snapshot().await, Some(workspace))
    }

    // ---- revisions and the workspace snapshot --------------------------------------------

    /// Marks the buffers as changed so the workspace snapshot catches up. Call it after the change
    /// was applied.
    pub(crate) fn touch(&self) {
        self.state.revision.fetch_add(1, Ordering::SeqCst);
        self.kick_workspace();
    }

    /// Like [`Self::touch`], but only for documents the index is built from.
    pub(crate) fn touch_document(&self, uri: &Url) {
        if docs::path_of(uri).is_none_or(|path| affects_index(&path)) {
            self.touch();
        }
    }

    /// Records a change of files on disk (watched files, saves).
    pub(crate) fn touch_disk(&self) {
        {
            let mut cell = self.state.workspace.state.lock().expect("cell lock");
            cell.checked = None;
        }
        self.touch();
    }

    pub(crate) fn note_watched_files(&self) {
        self.state.watched_files.store(true, Ordering::Relaxed);
    }

    fn disk_recheck(&self) -> Duration {
        if self.state.watched_files.load(Ordering::Relaxed) {
            DISK_RECHECK_WATCHED
        } else {
            DISK_RECHECK
        }
    }

    /// Starts a background rebuild of the snapshot when it is missing or stale and none runs.
    fn kick_workspace(&self) {
        let backend = self.clone();
        // `root()` is async; the kick must not block a notification handler.
        tokio::spawn(async move {
            if let Some(root) = backend.root().await {
                backend.refresh_workspace(&root);
            }
        });
    }

    /// Spawns the single rebuild task when the snapshot of `root` is stale. Returns the snapshot
    /// that is usable right now with the buffer revision it reflects.
    fn refresh_workspace(&self, root: &PathBuf) -> Option<(Arc<Workspace>, u64)> {
        let cell = &self.state.workspace;
        let (usable, start) = {
            let mut guard = cell.state.lock().expect("cell lock");
            let revision = self.state.revision.load(Ordering::SeqCst);
            let usable = guard
                .snapshot
                .clone()
                .filter(|_| guard.root.as_ref() == Some(root))
                .map(|snapshot| (snapshot, guard.revision));
            let stale = match &usable {
                None => true,
                Some(_) => {
                    guard.revision != revision
                        || guard
                            .checked
                            .is_none_or(|checked| checked.elapsed() >= self.disk_recheck())
                }
            };
            let start = stale && !guard.building;
            if start {
                guard.building = true;
            }
            (usable, start)
        };
        if start {
            let backend = self.clone();
            let root = root.clone();
            tokio::spawn(async move { backend.rebuild_workspace(root).await });
        }
        usable
    }

    /// The single rebuild task: rebuilds until the snapshot reflects the latest revision.
    async fn rebuild_workspace(&self, root: PathBuf) {
        let cell = &self.state.workspace;
        loop {
            let revision = self.state.revision.load(Ordering::SeqCst);
            let files = self.state.documents.lock().await.files();
            let indexer = cell.indexer.clone();
            let build_root = root.clone();
            let built = tokio::task::spawn_blocking(move || {
                indexer
                    .lock()
                    .expect("indexer lock")
                    .rebuild(&build_root, &files)
            })
            .await
            .unwrap_or_else(|_| Arc::new(Workspace::default()));
            let again = {
                let mut guard = cell.state.lock().expect("cell lock");
                guard.snapshot = Some(built);
                guard.root = Some(root.clone());
                guard.revision = revision;
                guard.checked = Some(Instant::now());
                let again = self.state.revision.load(Ordering::SeqCst) != revision;
                if !again {
                    guard.building = false;
                }
                again
            };
            cell.rounds.send_modify(|rounds| *rounds += 1);
            if !again {
                return;
            }
        }
    }

    /// Pack configuration index and generated classes below the workspace root. Answers with the
    /// last good snapshot and never rebuilds inline: a stale snapshot is refreshed in the background
    /// (one rebuild at a time, revisions coalesced, only changed files re-read). Only the very first
    /// request waits for the first build.
    pub(crate) async fn workspace(&self) -> Arc<Workspace> {
        let Some(root) = self.root().await else {
            return Arc::new(Workspace::default());
        };
        loop {
            // Subscribe before looking, so a build that finishes in between is not missed.
            let mut rounds = self.state.workspace.rounds.subscribe();
            if let Some((snapshot, _)) = self.refresh_workspace(&root) {
                return snapshot;
            }
            let _ = rounds.changed().await;
        }
    }

    /// The workspace as it is on disk right now, for explicit navigation requests (definition,
    /// references): the snapshot is refreshed first. That is cheap: the tree is walked and
    /// stat-ed, and only files that changed are read and parsed again.
    pub(crate) async fn fresh_workspace(&self) -> Arc<Workspace> {
        let Some(root) = self.root().await else {
            return Arc::new(Workspace::default());
        };
        let wanted = self.state.revision.fetch_add(1, Ordering::SeqCst) + 1;
        loop {
            let mut rounds = self.state.workspace.rounds.subscribe();
            if let Some((snapshot, revision)) = self.refresh_workspace(&root) {
                if revision >= wanted {
                    return snapshot;
                }
            }
            let _ = rounds.changed().await;
        }
    }

    /// Waits until no rebuild is running (tests).
    #[cfg(test)]
    pub(crate) async fn workspace_idle(&self) {
        loop {
            let mut rounds = self.state.workspace.rounds.subscribe();
            let building = self
                .state
                .workspace
                .state
                .lock()
                .expect("cell lock")
                .building;
            if !building {
                return;
            }
            let _ = rounds.changed().await;
        }
    }

    // ---- validation ------------------------------------------------------------------------

    /// Requests a validation. Requests coalesce: one runner debounces, validates the newest buffers
    /// and publishes; a request that arrives meanwhile makes it run once more.
    pub(crate) async fn schedule_validation(&self, cause: ValidationCause) {
        let settings = self.state.settings.lock().await.clone();
        if !cause.enabled(settings.trigger) {
            return;
        }
        let delay = if matches!(cause, ValidationCause::Edit) {
            settings.debounce_ms
        } else {
            0
        };
        self.state
            .validation
            .delay_ms
            .store(delay, Ordering::SeqCst);
        let generation = self.state.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.state
            .validation
            .request_generation
            .store(generation, Ordering::SeqCst);
        // Last: whoever sees the new request count also sees its generation.
        self.state
            .validation
            .requested
            .fetch_add(1, Ordering::SeqCst);
        let mut running = self.state.validation.running.lock().expect("runner lock");
        if !*running {
            *running = true;
            let backend = self.clone();
            tokio::spawn(async move { backend.run_validation().await });
        }
    }

    async fn run_validation(&self) {
        let state = &self.state;
        loop {
            let requested = state.validation.requested.load(Ordering::SeqCst);
            // Buffers or lint settings that change after the request make this run obsolete.
            let generation = state.validation.request_generation.load(Ordering::SeqCst);
            let delay = state.validation.delay_ms.load(Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(delay)).await;
            let newer = state.validation.requested.load(Ordering::SeqCst) != requested;
            // A newer request replaces this one; changed settings without a new request (a manual
            // trigger, for instance) cancel it.
            if !newer && state.generation.load(Ordering::SeqCst) == generation {
                // Manual triggers can suppress new requests while watched files still change.
                let revision = state.revision.load(Ordering::SeqCst);
                if let Some((report, cold, documents)) = self.validate().await {
                    if self.validation_is_current(generation, revision) {
                        self.publish(report, cold, documents, generation, revision)
                            .await;
                    }
                    // A revision-only invalidation still owes this request a current report.
                    if state.generation.load(Ordering::SeqCst) == generation
                        && state.revision.load(Ordering::SeqCst) != revision
                    {
                        continue;
                    }
                }
            }
            let mut running = state.validation.running.lock().expect("runner lock");
            if state.validation.requested.load(Ordering::SeqCst) == requested {
                *running = false;
                return;
            }
        }
    }

    /// Lints the current buffers.
    async fn validate(
        &self,
    ) -> Option<(
        kdf_validation::LintResult,
        ColdPropertyLevel,
        HashMap<PathBuf, (Url, i32)>,
    )> {
        let settings = self.state.settings.lock().await.clone();
        let cold = settings.cold_properties;
        let root = self.root().await;
        let datapack_index = if root.is_some() && self.reflection_path(&Value::Null).await.is_some()
        {
            Some(self.reflection(&Value::Null).await)
        } else {
            None
        };
        let (buffers, documents) = {
            let open = self.state.documents.lock().await;
            let documents = open
                .by_key
                .iter()
                .map(|(key, document)| (key.clone(), (document.uri.clone(), document.version)))
                .collect::<HashMap<_, _>>();
            (open.files(), documents)
        };
        // A host that chose schema-only validation must not get the project's binding through the
        // validator's own lookup.
        let schema_only = self.host_options().datapack == Some(None);
        let report = tokio::task::spawn_blocking(move || {
            let mut options = settings.lint_options(Profile::Runtime);
            options.datapack_index = datapack_index;
            options.no_datapack = schema_only;
            if let Some(root) = root {
                lint_workspace(&root, &buffers, &options)
            } else {
                let mut merged = kdf_validation::LintResult::default();
                for buffer in buffers {
                    let result = validate_document(&buffer.path, &buffer.text, &options)?;
                    merged.diagnostics.extend(result.diagnostics);
                }
                merged.finish();
                Ok(merged)
            }
        })
        .await;
        match report {
            Ok(Ok(report)) => Some((report, cold, documents)),
            Ok(Err(error)) => {
                self.client
                    .log_message(MessageType::ERROR, format!("validation failed: {error}"))
                    .await;
                None
            }
            Err(error) => {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        format!("validation task failed: {error}"),
                    )
                    .await;
                None
            }
        }
    }

    /// Publishes the report. Diagnostics of an open document go under the URL the client uses for
    /// it (and nowhere else); other files get the URL of their path.
    async fn publish(
        &self,
        report: kdf_validation::LintResult,
        cold: ColdPropertyLevel,
        documents: HashMap<PathBuf, (Url, i32)>,
        generation: u64,
        revision: u64,
    ) {
        let mut grouped =
            HashMap::<Url, (Option<i32>, Vec<tower_lsp::lsp_types::Diagnostic>)>::new();
        for diagnostic in report.diagnostics {
            let Some(path) = &diagnostic.file else {
                continue;
            };
            let target = match documents.get(&docs::key(path)) {
                Some((uri, version)) => Some((uri.clone(), Some(*version))),
                None => Url::from_file_path(path).ok().map(|uri| (uri, None)),
            };
            if let (Some((uri, version)), Some(mapped)) = (target, to_lsp(diagnostic, cold)) {
                grouped
                    .entry(uri)
                    .or_insert_with(|| (version, Vec::new()))
                    .1
                    .push(mapped);
            }
        }
        // Open documents without findings get an empty list (it clears what was shown before).
        for (uri, version) in documents.values() {
            grouped
                .entry(uri.clone())
                .or_insert_with(|| (Some(*version), Vec::new()));
        }
        let mut published = self.state.published.lock().await;
        if !self.validation_is_current(generation, revision) {
            return;
        }
        let stale = published
            .iter()
            .filter(|uri| !grouped.contains_key(*uri))
            .cloned()
            .collect::<Vec<_>>();
        for uri in stale {
            if !self.validation_is_current(generation, revision) {
                return;
            }
            self.client
                .publish_diagnostics(uri.clone(), Vec::new(), None)
                .await;
            // Track completed notifications even if the report became obsolete during the await.
            published.remove(&uri);
        }
        for (uri, (version, diagnostics)) in grouped {
            if !self.validation_is_current(generation, revision) {
                return;
            }
            self.client
                .publish_diagnostics(uri.clone(), diagnostics, version)
                .await;
            published.insert(uri);
        }
    }

    fn validation_is_current(&self, generation: u64, revision: u64) -> bool {
        self.state.generation.load(Ordering::SeqCst) == generation
            && self.state.revision.load(Ordering::SeqCst) == revision
    }

    // ---- services --------------------------------------------------------------------------

    /// Reflection data, workspace index and client capabilities for one request.
    pub(crate) async fn services(&self) -> Services {
        Services {
            reflection: self.reflection(&Value::Null).await,
            workspace: self.workspace().await,
            caps: self.caps(),
        }
    }

    pub(crate) async fn root(&self) -> Option<PathBuf> {
        self.state.root.lock().await.clone()
    }

    pub(crate) async fn reload_settings(&self) -> std::result::Result<(), String> {
        if let Some(root) = self.root().await {
            let user_settings = self.host_options().user_settings;
            let settings = LintSettings::load(&root, user_settings.as_deref())?;
            let mut current = self.state.settings.lock().await;
            if *current != settings {
                *current = settings;
                self.state.generation.fetch_add(1, Ordering::SeqCst);
            }
        }
        Ok(())
    }

    /// Forgets what was cached about `.kdf-editor.toml` and the datapack file (configuration
    /// changed, watched files changed).
    pub(crate) fn invalidate_project(&self) {
        *self.state.project.lock().expect("project lock") = ProjectCache::default();
        for slot in self
            .state
            .reflection
            .lock()
            .expect("reflection lock")
            .values_mut()
        {
            slot.checked = None;
        }
    }

    pub(crate) async fn revalidate(&self, _params: serde_json::Value) -> Result<serde_json::Value> {
        self.reload_settings()
            .await
            .map_err(RpcError::invalid_params)?;
        self.schedule_validation(ValidationCause::Manual).await;
        Ok(serde_json::json!({"scheduled": true}))
    }

    pub(crate) async fn reindex(&self, _params: serde_json::Value) -> Result<serde_json::Value> {
        let params = serde_json::Value::Null;
        let path = self
            .reflection_path(&params)
            .await
            .ok_or_else(|| RpcError::invalid_params("No datapack is bound to this project"))?;
        // A current cold load is shared. Let it finish before replacing its cell;
        // concurrent readers keep their Arc and no background load is orphaned.
        let cell = self.reflection_cell(&path, None);
        if cell.get().is_none() {
            let _ = self.reflection(&params).await;
        }
        self.state
            .reflection
            .lock()
            .expect("reflection lock")
            .remove(&path);
        let index = self.reflection(&params).await;
        if self.reflection_path(&params).await.as_ref() != Some(&path) {
            return Err(RpcError::invalid_params(
                "Datapack binding changed during re-index",
            ));
        }
        self.state.generation.fetch_add(1, Ordering::SeqCst);
        self.schedule_validation(ValidationCause::Manual).await;
        Ok(
            serde_json::json!({"status":index.status,"reason":index.reason,"classes":index.classes.len()}),
        )
    }

    /// Datapack bound to the project: `[datapack] path` of `.kdf-editor.toml`, read at most every
    /// [`PROJECT_RECHECK`].
    fn bound_path(&self, root: &PathBuf) -> Option<PathBuf> {
        {
            let cache = self.state.project.lock().expect("project lock");
            if cache.root.as_ref() == Some(root)
                && cache
                    .checked
                    .is_some_and(|checked| checked.elapsed() < PROJECT_RECHECK)
            {
                return cache.bound.clone();
            }
        }
        let bound = bound_datapack_path(root);
        *self.state.project.lock().expect("project lock") = ProjectCache {
            root: Some(root.clone()),
            checked: Some(Instant::now()),
            bound: bound.clone(),
        };
        bound
    }

    /// Datapack the features use: the request's `datapackPath`, else the host option
    /// (`initializationOptions.kdf.datapackPath`), else `KDF_EFFECTIVE_DATAPACK_PATH`, else the
    /// binding in `.kdf-editor.toml`.
    async fn reflection_path(&self, params: &serde_json::Value) -> Option<PathBuf> {
        if let Some(path) = params
            .get("datapackPath")
            .and_then(serde_json::Value::as_str)
        {
            return Some(PathBuf::from(path));
        }
        if let Some(selection) = self.host_options().datapack {
            return selection;
        }
        if let Some(override_path) = effective_datapack_override() {
            return override_path;
        }
        let root = self.root().await?;
        self.bound_path(&root)
    }

    /// Cell of the loaded index of `path`. One cell per file version: concurrent requests share
    /// the load, and the file is only looked at every [`PROJECT_RECHECK`].
    fn reflection_cell(
        &self,
        path: &PathBuf,
        keep: Option<&PathBuf>,
    ) -> Arc<OnceCell<Arc<ReflectionIndex>>> {
        let now = Instant::now();
        let stat = || {
            fs::metadata(path)
                .ok()
                .and_then(|meta| meta.modified().ok().map(|modified| (modified, meta.len())))
        };
        let mut slots = self.state.reflection.lock().expect("reflection lock");
        let cell = match slots.get_mut(path) {
            Some(slot)
                if slot
                    .checked
                    .is_some_and(|checked| now.duration_since(checked) < PROJECT_RECHECK) =>
            {
                slot.cell.clone()
            }
            Some(slot) => {
                let stamp = stat();
                if slot.checked.is_none() || slot.stamp != stamp {
                    slot.cell = Arc::new(OnceCell::new());
                    slot.stamp = stamp;
                }
                slot.checked = Some(now);
                slot.cell.clone()
            }
            None => {
                let cell = Arc::new(OnceCell::new());
                slots.insert(
                    path.clone(),
                    ReflectionSlot {
                        cell: cell.clone(),
                        stamp: stat(),
                        checked: Some(now),
                    },
                );
                cell
            }
        };
        // Datapacks that are no longer bound do not stay in memory.
        slots.retain(|known, _| known == path || keep == Some(known));
        cell
    }

    pub(crate) async fn reflection(&self, params: &serde_json::Value) -> Arc<ReflectionIndex> {
        let Some(path) = self.reflection_path(params).await else {
            return Arc::new(ReflectionIndex::unavailable(
                "no datapack is bound to this project",
            ));
        };
        // An explicit path (previewing another datapack) must not evict the bound one.
        let bound = if params.get("datapackPath").is_some() {
            self.reflection_path(&serde_json::Value::Null).await
        } else {
            None
        };
        let cell = self.reflection_cell(&path, bound.as_ref());
        cell.get_or_init(|| async {
            let loaded = tokio::task::spawn_blocking({
                let path = path.clone();
                move || ReflectionIndex::load(&path)
            })
            .await
            .unwrap_or_else(|error| {
                ReflectionIndex::unavailable(format!("reflection load task failed: {error}"))
            });
            let loaded = Arc::new(loaded);
            // Build the lazily computed lookups once, off the request path.
            let warm = loaded.clone();
            tokio::task::spawn_blocking(move || warm.warm());
            loaded
        })
        .await
        .clone()
    }

    pub(crate) async fn classes(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(&*self.reflection(&params).await)
            .unwrap_or(serde_json::Value::Null))
    }

    pub(crate) async fn class_reference_slots(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let uri = params
            .get("uri")
            .and_then(Value::as_str)
            .and_then(|value| Url::parse(value).ok())
            .filter(|uri| uri.scheme() == "file")
            .ok_or_else(|| RpcError::invalid_params("file document uri is required"))?;
        let version = params
            .get("version")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .filter(|value| *value >= 0)
            .ok_or_else(|| RpcError::invalid_params("native document version is required"))?;
        let text = params
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| {
                text.len() <= crate::class_slots::TEXT_LIMIT * 4
                    && text.encode_utf16().count() <= crate::class_slots::TEXT_LIMIT
            })
            .ok_or_else(|| RpcError::invalid_params("text exceeds class-slot limit"))?
            .to_owned();
        let requested = params
            .get("offsets")
            .and_then(Value::as_array)
            .filter(|offsets| offsets.len() <= crate::class_slots::BATCH_LIMIT)
            .ok_or_else(|| RpcError::invalid_params("offset batch exceeds class-slot limit"))?;
        let mut offsets = Vec::new();
        let mut seen = HashSet::new();
        for offset in requested {
            let offset = offset
                .as_u64()
                .filter(|offset| seen.insert(*offset))
                .ok_or_else(|| {
                    RpcError::invalid_params("offsets must be unique UTF-16 integers")
                })?;
            let position = crate::class_slots::position_at(&text, offset).ok_or_else(|| {
                RpcError::invalid_params("offset is outside document or splits a surrogate")
            })?;
            offsets.push((offset, position));
        }
        let generation = self.state.generation.load(Ordering::SeqCst);
        let revision = self.state.revision.load(Ordering::SeqCst);
        let path = {
            let mut documents = self.state.documents.lock().await;
            let document = documents
                .get(&uri)
                .filter(|document| document.version == version && document.file.text == text)
                .ok_or_else(|| {
                    RpcError::invalid_params("native document changed; retry class-slot query")
                })?;
            document.file.path.clone()
        };
        let index = self.reflection(&serde_json::Value::Null).await;
        let work_text = text.clone();
        let slots = tokio::task::spawn_blocking(move || {
            offsets
                .into_iter()
                .filter_map(|(offset, position)| {
                    crate::class_slots::at(&path, &work_text, position, &index).map(
                        |slot| serde_json::json!({"offset": offset, "baseClass": slot.base_class, "concreteOnly": slot.concrete_only}),
                    )
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|error| RpcError::invalid_params(format!("class-slot query failed: {error}")))?;
        let mut documents = self.state.documents.lock().await;
        if generation != self.state.generation.load(Ordering::SeqCst)
            || revision != self.state.revision.load(Ordering::SeqCst)
            || !documents
                .get(&uri)
                .is_some_and(|document| document.version == version && document.file.text == text)
        {
            return Err(RpcError::invalid_params(
                "native document or project changed during class-slot query",
            ));
        }
        Ok(serde_json::json!({"version": version, "slots": slots}))
    }

    pub(crate) async fn class_info(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let index = self.reflection(&params).await;
        let path = params
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let mut class = index.class_info(path).cloned();
        if let Some(record) = class.as_mut().and_then(Value::as_object_mut) {
            record.remove("iconDescriptorPath");
            if let Some(descriptor) = index.icon_descriptor_path(path) {
                record.insert(
                    "iconDescriptorPath".into(),
                    Value::String(descriptor.into()),
                );
            }
            record.insert(
                "properties".into(),
                Value::Array(
                    crate::intellisense::properties(&index, path)
                        .into_iter()
                        .cloned()
                        .collect(),
                ),
            );
        }
        Ok(serde_json::json!({
            "status": index.status,
            "reason": index.reason,
            "schemaVersion": index.schema_version,
            "engineVersion": index.engine_version,
            "class": class,
        }))
    }

    pub(crate) async fn affected_elements(&self, params: Value) -> Result<Value> {
        let entry = params
            .get("entry")
            .filter(|entry| entry.is_object())
            .ok_or_else(|| RpcError::invalid_params("entry must be a selector map"))?
            .clone();
        if entry.to_string().len() > 256 * 1024 {
            return Err(RpcError::invalid_params("selector exceeds preview limit"));
        }
        let root_type = params
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(500)
            .min(2000) as usize;
        let index = self.reflection(&params).await;
        let result = tokio::task::spawn_blocking(move || {
            kdf_validation::affected_elements(&index, &root_type, &entry, limit)
        })
        .await
        .map_err(|error| RpcError::invalid_params(format!("selector preview failed: {error}")))?;
        serde_json::to_value(result).map_err(|error| RpcError::invalid_params(error.to_string()))
    }

    pub(crate) async fn infer_property_owner(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let Some(names) = params
            .get("properties")
            .and_then(Value::as_array)
            .filter(|names| names.len() <= 64)
        else {
            return Ok(serde_json::json!({ "path": null }));
        };
        let names = names
            .iter()
            .filter_map(Value::as_str)
            .filter(|name| name.len() <= 256)
            .collect::<Vec<_>>();
        let index = self.reflection(&params).await;
        Ok(serde_json::json!({ "path": index.infer_property_owner(&names) }))
    }

    pub(crate) async fn preview_binding(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let candidate = params
            .get("datapackPath")
            .and_then(serde_json::Value::as_str)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| RpcError::invalid_params("datapackPath is required"))?;
        let root = self
            .root()
            .await
            .ok_or_else(|| RpcError::invalid_params("no workspace root"))?;
        let candidate = PathBuf::from(candidate);
        self.reload_settings()
            .await
            .map_err(RpcError::invalid_params)?;
        let buffers = self.state.documents.lock().await.files();
        let generation = self.state.generation.load(Ordering::SeqCst);
        let options = self
            .state
            .settings
            .lock()
            .await
            .lint_options(Profile::Runtime);
        let preview = tokio::task::spawn_blocking(move || {
            kdf_validation::preview_binding(&root, &buffers, &candidate, &options)
        })
        .await
        .map_err(|error| RpcError::invalid_params(format!("preview task failed: {error}")))?
        .map_err(RpcError::invalid_params)?;
        if self.state.generation.load(Ordering::SeqCst) != generation {
            return Err(RpcError::invalid_params(
                "buffers changed during binding preview; retry",
            ));
        }
        serde_json::to_value(preview)
            .map_err(|error| RpcError::invalid_params(format!("cannot serialize preview: {error}")))
    }

    pub(crate) async fn operations(&self, _params: serde_json::Value) -> Result<serde_json::Value> {
        Ok(serde_json::Value::Array(OPERATIONS.iter().map(|spec| serde_json::json!({
            "name": spec.name, "required": spec.required, "description": spec.description,
        })).collect()))
    }
}

/// Protocol severity of a diagnostic. Informational diagnostics (`property.cold`) keep the
/// validation severity `warning`; they are shown as information or hint, or not at all, according
/// to `[lint] coldProperties` in `.kdf-editor.toml`.
fn presentation(item: &KdfDiagnostic, cold: ColdPropertyLevel) -> Option<DiagnosticSeverity> {
    if item.code == "property.cold" {
        return match cold {
            ColdPropertyLevel::Off => None,
            ColdPropertyLevel::Hint => Some(DiagnosticSeverity::HINT),
            ColdPropertyLevel::Info => Some(DiagnosticSeverity::INFORMATION),
        };
    }
    Some(match (item.advisory, item.severity) {
        (Some(Advisory::Hint), _) => DiagnosticSeverity::HINT,
        (Some(Advisory::Information), _) => DiagnosticSeverity::INFORMATION,
        (None, Severity::Error) => DiagnosticSeverity::ERROR,
        (None, Severity::Warning) => DiagnosticSeverity::WARNING,
        (None, Severity::Information) => DiagnosticSeverity::INFORMATION,
    })
}

fn to_lsp(
    item: KdfDiagnostic,
    cold: ColdPropertyLevel,
) -> Option<tower_lsp::lsp_types::Diagnostic> {
    let severity = presentation(&item, cold)?;
    let explanation = kdf_validation::evidence::explain(&item);
    Some(tower_lsp::lsp_types::Diagnostic {
        range: Range {
            start: Position {
                line: item.range.start.line,
                character: item.range.start.character,
            },
            end: Position {
                line: item.range.end.line,
                character: item.range.end.character,
            },
        },
        severity: Some(severity),
        code: Some(NumberOrString::String(item.code)),
        code_description: None,
        source: Some(item.source),
        message: item.message,
        related_information: None,
        tags: None,
        data: Some(
            serde_json::json!({"yamlPath":item.yaml_path,"document":item.document,"schema":item.schema,
                "schemaVersion":1,"explanationId":explanation.id,"evidence":explanation.evidence}),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::Range as LspRange;

    #[test]
    fn lsp_diagnostic_preserves_validation_source() {
        let diagnostic = KdfDiagnostic::new(
            Severity::Error,
            "schema.enum",
            "invalid enum value",
            None,
            None,
            "$",
            kdf_validation::Range::default(),
            "schema",
        );
        let mapped = to_lsp(diagnostic, ColdPropertyLevel::Info).expect("mapped");
        assert_eq!(mapped.source.as_deref(), Some("schema"));
        assert_eq!(mapped.severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn lint_trigger_controls_automatic_revalidation() {
        assert!(ValidationCause::Edit.enabled(LintTrigger::OnType));
        assert!(!ValidationCause::Edit.enabled(LintTrigger::OnSave));
        assert!(ValidationCause::Saved.enabled(LintTrigger::OnSave));
        assert!(!ValidationCause::Startup.enabled(LintTrigger::Manual));
        assert!(!ValidationCause::Saved.enabled(LintTrigger::Manual));
        assert!(ValidationCause::Manual.enabled(LintTrigger::Manual));
    }

    fn change(
        range: Option<((u32, u32), (u32, u32))>,
        text: &str,
    ) -> TextDocumentContentChangeEvent {
        TextDocumentContentChangeEvent {
            range: range.map(|((sl, sc), (el, ec))| LspRange {
                start: Position {
                    line: sl,
                    character: sc,
                },
                end: Position {
                    line: el,
                    character: ec,
                },
            }),
            range_length: None,
            text: text.to_owned(),
        }
    }

    #[test]
    fn positions_past_the_end_are_clamped_not_rejected() {
        // Past the end of a line: the end of that line.
        let mut text = "ab\ncd\n".to_owned();
        apply_content_changes(&mut text, vec![change(Some(((0, 99), (0, 99))), "X")]);
        assert_eq!(text, "abX\ncd\n");
        // Past the last line: the end of the document.
        let mut text = "ab\ncd".to_owned();
        apply_content_changes(&mut text, vec![change(Some(((99, 0), (99, 0))), "\nef")]);
        assert_eq!(text, "ab\ncd\nef");
        // A range that ends past the line replaces up to the line end only.
        let mut text = "ab\r\ncd\r\n".to_owned();
        apply_content_changes(&mut text, vec![change(Some(((0, 1), (0, 50))), "Z")]);
        assert_eq!(text, "aZ\r\ncd\r\n");
        // A reversed range is read between its ends; a full replacement wins over earlier changes.
        let mut text = "abcd".to_owned();
        apply_content_changes(&mut text, vec![change(Some(((0, 3), (0, 1))), "-")]);
        assert_eq!(text, "a-d");
        apply_content_changes(
            &mut text,
            vec![change(Some(((0, 0), (0, 1))), "!"), change(None, "new")],
        );
        assert_eq!(text, "new");
    }

    // ---- in-process tests of the backend ---------------------------------------------------

    use crate::test_support::{self as support, ArchiveOptions};
    use serde_json::json;
    use tower_lsp::LspService;

    fn backend() -> (Backend, LspService<Backend>) {
        let (service, socket) = LspService::new(Backend::new);
        // Nothing reads the socket in these tests; keep it alive for the length of the test run.
        std::mem::forget(socket);
        (service.inner().clone(), service)
    }

    #[tokio::test]
    async fn superseded_validation_does_not_clear_or_replace_published_diagnostics() {
        assert_obsolete_validation_preserves_published(|backend| {
            backend.state.generation.fetch_add(1, Ordering::SeqCst);
        })
        .await;
    }

    #[tokio::test]
    async fn disk_changes_with_manual_trigger_drop_closed_file_validation() {
        assert_obsolete_validation_preserves_published(|backend| {
            backend.touch_disk();
            assert_eq!(backend.state.generation.load(Ordering::SeqCst), 0);
            assert_eq!(backend.state.revision.load(Ordering::SeqCst), 1);
        })
        .await;
    }

    #[tokio::test]
    async fn startup_workspace_touch_does_not_lose_automatic_client_diagnostics() {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

        async fn read_message(
            reader: &mut BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
        ) -> Value {
            let mut length = None;
            loop {
                let mut line = String::new();
                assert_ne!(
                    reader.read_line(&mut line).await.unwrap(),
                    0,
                    "LSP output closed"
                );
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.strip_prefix("Content-Length: ") {
                    length = Some(value.trim().parse::<usize>().unwrap());
                }
            }
            let mut body = vec![0; length.expect("LSP content length")];
            reader.read_exact(&mut body).await.unwrap();
            serde_json::from_slice(&body).unwrap()
        }

        let (_directory, root) = project_with_item();
        support::write(
            &root,
            "ConfigDemo/tuning.cdo.yml",
            "type: cdo\npatches: []\n",
        );
        let path = root.join("ConfigDemo/tuning.cdo.yml");
        let plain_uri = Url::from_file_path(&path).unwrap();
        let mut spelling = plain_uri.as_str().replacen("tuning", "tunin%67", 1);
        if cfg!(windows) {
            spelling = spelling.replacen("file:///C:/", "file:///c%3A/", 1);
        }
        let client_uri = Url::parse(&spelling).unwrap();
        assert_ne!(plain_uri, client_uri);
        let version = 7;
        let (service, socket) = LspService::new(Backend::new);
        let backend = service.inner().clone();
        let (client_pipe, server_pipe) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server_pipe);
        let transport = tower_lsp::Server::new(server_read, server_write, socket);
        let server = tokio::spawn(transport.serve(service));
        let (client_read, mut client_write) = tokio::io::split(client_pipe);
        let mut reader = BufReader::new(client_read);
        let initialize = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"rootUri": Url::from_directory_path(&root).unwrap(), "capabilities": {}}
        }))
        .unwrap();
        client_write
            .write_all(format!("Content-Length: {}\r\n\r\n", initialize.len()).as_bytes())
            .await
            .unwrap();
        client_write.write_all(&initialize).await.unwrap();
        client_write.flush().await.unwrap();
        let response = read_message(&mut reader).await;
        assert_eq!(response["id"], 1);
        assert!(response.get("error").is_none(), "{response}");

        backend
            .open_document(
                client_uri.clone(),
                version,
                "type: cdo\npatches:\n  - target: /Script/FactoryGame.FGItemDescriptor\n    properties:\n      - path: mA\n        op: multiplly\n        value: 1\n".into(),
            )
            .await;
        let mut publication = Some(backend.state.published.lock().await);
        backend.schedule_validation(ValidationCause::Startup).await;

        loop {
            tokio::task::yield_now().await;
            drop(publication.take());
            match backend.state.published.try_lock() {
                Ok(guard) => {
                    publication = Some(guard);
                    assert!(
                        *backend.state.validation.running.lock().unwrap(),
                        "automatic validation stopped before its publication boundary"
                    );
                }
                // A queued Mutex::lock owns the released permit before it resumes.
                Err(_) => break,
            }
        }
        // No await between the fair handoff and the actual startup warm-up touch.
        let requested = backend.state.validation.requested.load(Ordering::SeqCst);
        let generation = backend.state.generation.load(Ordering::SeqCst);
        let revision = backend.state.revision.load(Ordering::SeqCst);
        assert_eq!(requested, 1);
        backend.touch();
        assert_eq!(
            backend.state.validation.requested.load(Ordering::SeqCst),
            requested
        );
        assert_eq!(backend.state.generation.load(Ordering::SeqCst), generation);
        assert_eq!(backend.state.revision.load(Ordering::SeqCst), revision + 1);

        while *backend.state.validation.running.lock().unwrap() {
            tokio::task::yield_now().await;
        }
        // This real client notification drains earlier publications; it is not the oracle.
        let marker = "revision-progress-publications-drained";
        backend.client.log_message(MessageType::INFO, marker).await;
        let mut publications = Vec::new();
        loop {
            let message = read_message(&mut reader).await;
            if message["method"] == "window/logMessage" && message["params"]["message"] == marker {
                break;
            }
            if message["method"] == "textDocument/publishDiagnostics" {
                publications.push(message);
            }
        }
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());

        let mine = publications
            .iter()
            .filter(|message| message["params"]["uri"] == json!(client_uri))
            .collect::<Vec<_>>();
        assert!(
            !mine.is_empty(),
            "automatic diagnostics lost after workspace touch: {publications:?}"
        );
        assert!(
            mine.iter().any(|message| {
                message["params"]["version"] == version
                    && message["params"]["diagnostics"]
                        .as_array()
                        .is_some_and(|diagnostics| {
                            diagnostics
                                .iter()
                                .any(|item| item["code"] == "semantic.invalid-op")
                        })
            }),
            "latest stored client URI/version/finding missing: {mine:?}"
        );
        assert!(
            publications
                .iter()
                .all(|message| message["params"]["uri"] != json!(plain_uri)),
            "duplicate diagnostics under canonical URI: {publications:?}"
        );
    }

    async fn assert_obsolete_validation_preserves_published(invalidate: impl FnOnce(&Backend)) {
        let (backend, _service) = backend();
        backend.state.settings.lock().await.trigger = LintTrigger::Manual;
        let directory = tempfile::tempdir().unwrap();
        let previous_uri = Url::from_file_path(directory.path().join("previous.item.yml")).unwrap();
        let closed_path = directory.path().join("closed.item.yml");
        let mut published = backend.state.published.lock().await;
        published.insert(previous_uri.clone());
        let mut report = kdf_validation::LintResult::default();
        report.diagnostics.push(KdfDiagnostic::new(
            Severity::Error,
            "schema.enum",
            "obsolete finding",
            Some(closed_path),
            None,
            "$",
            kdf_validation::Range::default(),
            "schema",
        ));
        let publisher = backend.clone();
        let task = tokio::spawn(async move {
            publisher
                .publish(report, ColdPropertyLevel::Info, HashMap::new(), 0, 0)
                .await;
        });
        tokio::task::yield_now().await;
        invalidate(&backend);
        backend.schedule_validation(ValidationCause::Saved).await;
        drop(published);
        task.await.unwrap();
        assert_eq!(
            *backend.state.published.lock().await,
            HashSet::from([previous_uri])
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_requests_share_one_reflection_load() {
        let archive = support::archive(&ArchiveOptions::default());
        let (backend, _service) = backend();
        let params = json!({"datapackPath": archive.path()});
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let backend = backend.clone();
                let params = params.clone();
                tokio::spawn(async move { backend.reflection(&params).await })
            })
            .collect();
        let mut loaded = Vec::new();
        for task in tasks {
            loaded.push(task.await.unwrap());
        }
        assert!(
            loaded.iter().all(|index| Arc::ptr_eq(index, &loaded[0])),
            "each request loaded the archive on its own"
        );
        assert!(!loaded[0].classes.is_empty());
        // A later request reuses it as well.
        assert!(Arc::ptr_eq(&backend.reflection(&params).await, &loaded[0]));
    }

    #[tokio::test]
    async fn indexes_of_datapacks_that_are_no_longer_bound_are_dropped() {
        let (first, second, third) = (
            support::archive(&ArchiveOptions::default()),
            support::archive(&ArchiveOptions::default()),
            support::archive(&ArchiveOptions::default()),
        );
        let (backend, _service) = backend();
        let slots = || backend.state.reflection.lock().unwrap().len();
        let option = |path: &std::path::Path| json!({"kdf": {"datapackPath": path}});
        backend.apply_host_options(&option(first.path()));
        backend.reflection(&Value::Null).await;
        assert_eq!(slots(), 1);
        // Binding another datapack replaces the first one in memory.
        backend.apply_host_options(&option(second.path()));
        backend.reflection(&Value::Null).await;
        assert_eq!(slots(), 1);
        // A previewed candidate lives next to the bound pack, and goes with the next request.
        backend
            .reflection(&json!({"datapackPath": third.path()}))
            .await;
        assert_eq!(slots(), 2);
        backend.reflection(&Value::Null).await;
        assert_eq!(slots(), 1);
    }

    #[tokio::test]
    async fn a_changed_datapack_file_is_loaded_again_after_the_recheck() {
        let (small, large) = (
            support::archive(&ArchiveOptions::default()),
            support::archive_with(
                vec![support::class("/Script/Test.Extra", None, vec![], vec![])],
                &ArchiveOptions::default(),
            ),
        );
        let (backend, _service) = backend();
        backend.apply_host_options(&json!({"kdf": {"datapackPath": small.path()}}));
        let before = backend.reflection(&Value::Null).await;
        assert!(Arc::ptr_eq(
            &before,
            &backend.reflection(&Value::Null).await
        ));
        std::fs::copy(large.path(), small.path()).unwrap();
        // Inside the recheck interval the file is not looked at again...
        assert!(Arc::ptr_eq(
            &before,
            &backend.reflection(&Value::Null).await
        ));
        // ...a watched-file event or the interval makes the next request notice.
        backend.invalidate_project();
        let after = backend.reflection(&Value::Null).await;
        assert!(!Arc::ptr_eq(&before, &after));
        assert_eq!(after.classes.len(), before.classes.len() + 1);
    }

    #[tokio::test]
    async fn an_empty_host_datapack_selects_schema_only_validation() {
        let archive = support::archive(&ArchiveOptions::default());
        let (backend, _service) = backend();
        backend.apply_host_options(&json!({"kdf": {"datapackPath": archive.path()}}));
        assert_eq!(
            backend.reflection(&Value::Null).await.status,
            kdf_validation::ReflectionStatus::Ready
        );
        // An empty path unbinds it.
        assert!(backend.apply_host_options(&json!({"kdf": {"datapackPath": ""}})));
        assert_eq!(
            backend.reflection(&Value::Null).await.status,
            kdf_validation::ReflectionStatus::MissingData
        );
    }

    fn project_with_item() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap().join("DataForge");
        support::config_workspace(&root);
        support::write(
            &root,
            "ConfigDemo/a.item.yml",
            "type: item\nitems:\n  - id: Alpha\n    parent: /Script/FactoryGame.FGItemDescriptor\n",
        );
        (dir, root)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn the_workspace_is_built_once_and_stale_snapshots_refresh_in_the_background() {
        let (_dir, root) = project_with_item();
        let (backend, _service) = backend();
        *backend.state.root.lock().await = Some(root.clone());
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let backend = backend.clone();
                tokio::spawn(async move { backend.workspace().await })
            })
            .collect();
        let mut snapshots = Vec::new();
        for task in tasks {
            snapshots.push(task.await.unwrap());
        }
        assert!(
            snapshots.iter().all(|one| Arc::ptr_eq(one, &snapshots[0])),
            "concurrent first requests share one build"
        );
        assert!(snapshots[0].class_of_pack("ConfigDemo", "Alpha").is_some());

        // A file changes on disk: a later snapshot has it, nobody rebuilds inline.
        support::write(
            &root,
            "ConfigDemo/b.item.yml",
            "type: item\nitems:\n  - id: Beta\n    parent: /Script/FactoryGame.FGItemDescriptor\n",
        );
        backend.touch_disk();
        let mut found = false;
        for _ in 0..200 {
            if backend
                .workspace()
                .await
                .class_of_pack("ConfigDemo", "Beta")
                .is_some()
            {
                found = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(found, "the background rebuild never delivered the new file");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn edit_bursts_coalesce_into_few_rebuilds() {
        let (_dir, root) = project_with_item();
        let (backend, _service) = backend();
        *backend.state.root.lock().await = Some(root.clone());
        backend.workspace().await;
        backend.workspace_idle().await;
        let rounds = backend.state.workspace.rounds.subscribe();
        let before = *rounds.borrow();
        let wanted = backend.state.revision.load(Ordering::SeqCst) + 100;
        {
            // Keep the rebuild pending until every revision in this burst has been submitted.
            let _indexer = backend
                .state
                .workspace
                .indexer
                .lock()
                .expect("indexer lock");
            for _ in 0..100 {
                backend.touch();
            }
            backend.refresh_workspace(&root);
        }
        tokio::time::timeout(Duration::from_secs(5), backend.workspace_idle())
            .await
            .expect("the coalesced burst must finish");
        assert_eq!(
            backend
                .state
                .workspace
                .state
                .lock()
                .expect("cell lock")
                .revision,
            wanted,
            "the final snapshot must include every revision in the burst"
        );
        let used = *rounds.borrow() - before;
        assert!(
            (1..=6).contains(&used),
            "{used} rebuild rounds for 100 revisions"
        );
    }

    #[tokio::test]
    async fn open_buffers_are_found_under_every_spelling_of_their_url() {
        let (_dir, root) = project_with_item();
        let (backend, _service) = backend();
        // The client's spelling (`c%3A`, a percent-encoded letter) differs from the one
        // `from_file_path` produces.
        let path = root.join("ConfigDemo/a.item.yml");
        let plain = Url::from_file_path(&path).unwrap();
        let client = Url::parse(
            &plain
                .as_str()
                .replacen("file:///C:/", "file:///c%3A/", 1)
                .replace("DataForge", "DataForg%65"),
        )
        .unwrap();
        assert_ne!(plain, client);
        backend
            .open_document(client.clone(), 1, "buffer text\n".into())
            .await;
        assert_eq!(backend.source(&plain).await.unwrap(), "buffer text\n");
        assert_eq!(backend.source(&client).await.unwrap(), "buffer text\n");
        assert_eq!(
            backend.open_urls().await.values().collect::<Vec<_>>(),
            [&client]
        );
        // A change through the other spelling edits the same document.
        backend
            .change_document(&plain, 2, vec![change(None, "changed\n")])
            .await
            .unwrap();
        assert_eq!(backend.source(&client).await.unwrap(), "changed\n");
        backend.close_document(&plain).await;
        assert!(backend.open_urls().await.is_empty());
        // The file on disk is what is read once the buffer is gone.
        assert!(backend.source(&client).await.unwrap().contains("Alpha"));
    }

    #[test]
    fn host_options_are_read_from_the_kdf_section_or_a_flat_object() {
        let value = serde_json::json!({"kdf": {"datapackPath": "C:/x/vanilla.kdfdata", "userSettingsPath": "C:/u/settings.toml", "clientCommands": false}});
        assert!(kdf_section(&value).is_some());
        assert!(kdf_section(&serde_json::json!({"datapackPath": ""})).is_some());
        assert!(kdf_section(&serde_json::json!({})).is_none());
        assert!(kdf_section(&serde_json::json!({"other": 1})).is_none());
        assert!(kdf_section(&Value::Null).is_none());
    }
}
