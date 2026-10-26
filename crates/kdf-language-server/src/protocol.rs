use crate::docs;
use crate::features::{operation_fix, word_at};
use crate::semantic::Env;
use crate::state::{Backend, ClientCaps, Services, ValidationCause};
use crate::{actions, completion, config_nav, context, hover, navigation, resolve, signature};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tower_lsp::jsonrpc::{Error as RpcError, Result};
use tower_lsp::lsp_types::*;
use tower_lsp::LanguageServer;

/// Characters that open completion: references, providers, templates, calls, arguments, quotes.
const COMPLETION_TRIGGERS: [&str; 9] = [":", " ", ".", "/", "{", "(", ",", "'", "\""];
/// Characters that open signature help for a template function call.
const SIGNATURE_TRIGGERS: [&str; 2] = ["(", ","];

fn env<'a>(path: &'a Path, text: &'a str, services: &'a Services) -> Env<'a> {
    Env {
        path,
        text,
        reflection: &services.reflection,
        workspace: &services.workspace,
        caps: services.caps,
    }
}

fn mentions_config(text: &str) -> bool {
    text.contains("cfg:") || text.contains("config:")
}

fn offers(formats: Option<&Vec<MarkupKind>>) -> bool {
    formats.and_then(|formats| formats.first()) == Some(&MarkupKind::Markdown)
}

/// What the client declared it can render.
pub(crate) fn client_caps(capabilities: &ClientCapabilities) -> ClientCaps {
    let text_document = capabilities.text_document.as_ref();
    let completion_item = text_document
        .and_then(|document| document.completion.as_ref())
        .and_then(|completion| completion.completion_item.as_ref());
    let signature_information = text_document
        .and_then(|document| document.signature_help.as_ref())
        .and_then(|help| help.signature_information.as_ref());
    ClientCaps {
        versioned_edits: capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.workspace_edit.as_ref())
            .and_then(|edit| edit.document_changes)
            .unwrap_or(false),
        snippets: completion_item
            .and_then(|item| item.snippet_support)
            .unwrap_or(false),
        completion_markdown: offers(
            completion_item.and_then(|item| item.documentation_format.as_ref()),
        ),
        resolve_documentation: completion_item
            .and_then(|item| item.resolve_support.as_ref())
            .is_some_and(|support| {
                support
                    .properties
                    .iter()
                    .any(|property| property == "documentation")
            }),
        hover_markdown: offers(
            text_document
                .and_then(|document| document.hover.as_ref())
                .and_then(|hover| hover.content_format.as_ref()),
        ),
        signature_markdown: offers(
            signature_information.and_then(|information| information.documentation_format.as_ref()),
        ),
        label_offsets: signature_information
            .and_then(|information| information.parameter_information.as_ref())
            .and_then(|parameters| parameters.label_offset_support)
            .unwrap_or(false),
        signature_active_parameter: signature_information
            .and_then(|information| information.active_parameter_support)
            .unwrap_or(false),
        client_commands: true,
    }
}

/// Edits and locations name open documents the way the client does.
fn client_url(url: Url, open: &HashMap<PathBuf, Url>) -> Url {
    docs::path_of(&url)
        .and_then(|path| open.get(&docs::key(&path)).cloned())
        .unwrap_or(url)
}

/// Only new semantic contexts select the specialized provider; its legacy lists
/// remain a fallback when native IntelliSense has no domain result.
fn specialized_hover(resolved: &context::Resolved) -> bool {
    if resolved.is_config_doc() || resolved.in_conditions() {
        return true;
    }
    let context::Place::Value(slot) = &resolved.place else {
        return false;
    };
    let value = &slot.scalar.value;
    resolved.key() == Some("category")
        || resolved.op_entry().is_some_and(|op| op.below == ["path"])
        || kdf_validation::template::partial_at(value, slot.scalar.cursor).is_some()
        || value
            .split_once(':')
            .is_some_and(|(prefix, _)| kdf_validation::template::PREPASS_PREFIXES.contains(&prefix))
}

fn has_completion_items(response: &CompletionResponse) -> bool {
    match response {
        CompletionResponse::Array(items) => !items.is_empty(),
        CompletionResponse::List(list) => !list.items.is_empty(),
    }
}

fn specialized_completion(
    env: &Env<'_>,
    resolved: &context::Resolved,
    native: Option<&CompletionResponse>,
) -> bool {
    specialized_hover(resolved)
        || matches!(&resolved.place, context::Place::Key(_)) && resolved.op_entry().is_some()
        || !native.is_some_and(has_completion_items)
            && env.workspace.config.fields().next().is_some()
            && crate::semantic::operand_context(env, resolved).is_some()
}

/// Native fields determine safe scope and existing-use ordering; append config
/// reload documentation without replacing those semantics or list incompleteness.
fn enrich_path_completion(
    native: CompletionResponse,
    configured: completion::Completions,
) -> CompletionResponse {
    let mut native = completion_list(native);
    let items = match &mut native {
        CompletionResponse::Array(items) => items,
        CompletionResponse::List(list) => &mut list.items,
    };
    if let Some(selector) = configured
        .items
        .iter()
        .find(|item| item.label == kdf_validation::TAG_CONTAINER_SELECTOR)
    {
        if !items.iter().any(|item| item.label == selector.label) {
            items.insert(0, selector.clone());
        }
    }
    let truncated = items.len() > completion::LIMIT;
    if truncated {
        items.truncate(completion::LIMIT);
    }
    for item in items {
        if let Some(extra) = configured
            .items
            .iter()
            .find(|other| other.label == item.label)
        {
            if item.data.is_none() {
                item.data = extra.data.clone();
            }
            if let Some(tag) = extra
                .detail
                .as_deref()
                .and_then(|detail| detail.rsplit_once(" · ").map(|(_, tag)| tag))
            {
                if matches!(tag, "cold" | "newActors") {
                    let detail = item.detail.get_or_insert_with(String::new);
                    if !detail.contains(tag) {
                        detail.push_str(&format!(" · {tag}"));
                    }
                }
            }
            if let (Some(existing), Some(doc)) =
                (item.documentation.as_mut(), extra.documentation.as_ref())
            {
                let (text, markdown) = match existing {
                    Documentation::String(text) => (text, false),
                    Documentation::MarkupContent(content) => {
                        (&mut content.value, content.kind == MarkupKind::Markdown)
                    }
                };
                if !text.contains("Reload:") {
                    let addition = match doc {
                        Documentation::String(text) => text.clone(),
                        Documentation::MarkupContent(content)
                            if !markdown && content.kind == MarkupKind::Markdown =>
                        {
                            resolve::plain_text(&content.value)
                        }
                        Documentation::MarkupContent(content) => content.value.clone(),
                    };
                    text.push_str("\n\n");
                    text.push_str(&addition);
                }
            }
        }
    }
    if truncated {
        if let CompletionResponse::List(list) = &mut native {
            list.is_incomplete = true;
        }
    }
    completion_list(native)
}

fn completion_list(response: CompletionResponse) -> CompletionResponse {
    match response {
        CompletionResponse::Array(mut items) => {
            let is_incomplete = items.len() > completion::LIMIT;
            if is_incomplete {
                items.sort_by(|left, right| left.sort_text.cmp(&right.sort_text));
                items.truncate(completion::LIMIT);
            }
            CompletionResponse::List(CompletionList {
                is_incomplete,
                items,
            })
        }
        CompletionResponse::List(mut list) => {
            if list.items.len() > completion::LIMIT {
                list.items.truncate(completion::LIMIT);
                list.is_incomplete = true;
            }
            CompletionResponse::List(list)
        }
    }
}

/// Native providers build Markdown cards without client state. Negotiate at the wire boundary.
fn completion_documentation(
    mut response: CompletionResponse,
    caps: ClientCaps,
) -> CompletionResponse {
    let items = match &mut response {
        CompletionResponse::Array(items) => items,
        CompletionResponse::List(list) => &mut list.items,
    };
    for item in items {
        negotiate_documentation(item, caps);
    }
    response
}

fn negotiate_documentation(item: &mut CompletionItem, caps: ClientCaps) {
    if caps.completion_markdown {
        return;
    }
    if let Some(Documentation::MarkupContent(content)) = item.documentation.as_ref() {
        if content.kind == MarkupKind::Markdown {
            item.documentation = Some(resolve::documentation(content.value.clone(), caps));
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let root = params
            .root_uri
            .as_ref()
            .and_then(docs::path_of)
            .or_else(|| {
                params.workspace_folders.as_ref().and_then(|folders| {
                    folders
                        .first()
                        .and_then(|folder| docs::path_of(&folder.uri))
                })
            });
        if let Some(root) = root {
            let root = docs::canonical(&root);
            let dataforge = root.join("DataForge");
            *self.state.root.lock().await = Some(if dataforge.is_dir() { dataforge } else { root });
        }
        self.set_caps(client_caps(&params.capabilities));
        if let Some(options) = &params.initialization_options {
            self.apply_host_options(options);
        }
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(
                        COMPLETION_TRIGGERS
                            .iter()
                            .map(|ch| (*ch).to_owned())
                            .collect(),
                    ),
                    resolve_provider: Some(true),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                document_formatting_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                experimental: Some(serde_json::json!({"kdfAuthoring": {"schemaVersion": 1,
                    "completionLimit": 200, "snapshotGuards": true, "renamePlans": true}})),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(
                        SIGNATURE_TRIGGERS
                            .iter()
                            .map(|ch| (*ch).to_owned())
                            .collect(),
                    ),
                    ..Default::default()
                }),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "kdf-language-server".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        match self.reload_settings().await {
            Ok(()) => self.schedule_validation(ValidationCause::Startup).await,
            Err(error) => {
                self.client
                    .log_message(MessageType::ERROR, format!("lint settings: {error}"))
                    .await
            }
        }
        // Have the workspace snapshot ready for the first request.
        self.touch();
    }
    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let item = params.text_document;
        if docs::path_of(&item.uri).is_some() {
            self.open_document(item.uri, item.version, item.text).await;
            self.schedule_validation(ValidationCause::Edit).await;
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        match self
            .change_document(&uri, params.text_document.version, params.content_changes)
            .await
        {
            Ok(()) => self.schedule_validation(ValidationCause::Edit).await,
            Err(message) => self.client.log_message(MessageType::ERROR, message).await,
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.close_document(&params.text_document.uri).await;
        self.schedule_validation(ValidationCause::Closed).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.touch_document(&params.text_document.uri);
        self.schedule_validation(ValidationCause::Saved).await;
    }

    async fn did_change_watched_files(&self, _: DidChangeWatchedFilesParams) {
        self.note_watched_files();
        self.touch_disk();
        self.invalidate_project();
        match self.reload_settings().await {
            Ok(()) => {
                self.schedule_validation(ValidationCause::SettingsChanged)
                    .await
            }
            Err(error) => {
                self.client
                    .log_message(MessageType::ERROR, format!("lint settings: {error}"))
                    .await
            }
        }
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        self.apply_host_options(&params.settings);
        self.invalidate_project();
        match self.reload_settings().await {
            Ok(()) => {
                self.schedule_validation(ValidationCause::SettingsChanged)
                    .await
            }
            Err(error) => {
                self.client
                    .log_message(MessageType::ERROR, format!("lint settings: {error}"))
                    .await
            }
        }
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let epoch = self.epoch();
        let response = async {
            let uri = params.text_document_position.text_document.uri;
            let position = params.text_document_position.position;
            let Some((path, text)) = self.document(&uri).await else {
                return Ok(None);
            };
            let services = self.services().await;
            let env = env(&path, &text, &services);
            let resolved = context::resolve(&path, &text, position);
            let native =
                crate::intellisense::completion_resolved(&text, &resolved, &services.reflection);
            // Specialized config/template/category/path contexts have precedence. Ordinary
            // class/property/op completion keeps native filtering, common-target semantics,
            // existing-use labels, exact replacements and bounded incomplete lists.
            let specialized = specialized_completion(&env, &resolved, native.as_ref());
            if specialized {
                let configured = completion::complete(&env, &resolved);
                if resolved.op_entry().is_some_and(|op| op.below == ["path"]) {
                    if let Some(native) = native {
                        let has_fields = has_completion_items(&native);
                        // Generated entries may rely on their documented default base
                        // class. An empty native list must not erase proven base fields.
                        if has_fields || configured.items.is_empty() {
                            return Ok(Some(completion_documentation(
                                enrich_path_completion(native, configured),
                                services.caps,
                            )));
                        }
                    }
                }
                return Ok((!configured.items.is_empty() || configured.incomplete)
                    .then(|| completion_documentation(configured.into_response(), services.caps)));
            }
            if native.is_some() {
                return Ok(native.map(|response| {
                    completion_documentation(completion_list(response), services.caps)
                }));
            }
            let fallback = completion::complete(&env, &resolved);
            Ok((!fallback.items.is_empty() || fallback.incomplete)
                .then(|| completion_documentation(fallback.into_response(), services.caps)))
        }
        .await;
        self.check_epoch(epoch)?;
        response
    }

    /// Documentation of a completion item, built from the `data` the item carries.
    async fn completion_resolve(&self, mut item: CompletionItem) -> Result<CompletionItem> {
        if item.documentation.is_some() {
            negotiate_documentation(&mut item, self.caps());
            return Ok(item);
        }
        let Some(data) = item.data.as_ref() else {
            return Ok(item);
        };
        let services = self.services().await;
        if let Some(card) = resolve::card(data, &services.reflection, &services.workspace) {
            item.documentation = Some(resolve::documentation(card, services.caps));
        }
        Ok(item)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some((path, text)) = self.document(&uri).await else {
            return Ok(None);
        };
        let caps = {
            let services = self.services().await;
            let env = env(&path, &text, &services);
            let resolved = context::resolve(&path, &text, position);
            if specialized_hover(&resolved) {
                if let Some(mut found) = hover::hover(&env, &resolved) {
                    if resolved.op_entry().is_some_and(|op| op.below == ["path"]) {
                        let native = crate::intellisense::context(&text, position);
                        if let Some(property) =
                            crate::intellisense::property(&services.reflection, &native)
                        {
                            if let HoverContents::Markup(content) = &mut found.contents {
                                content.value.push_str("\n\n");
                                let extra = crate::intellisense::contextual_property_markdown(
                                    &services.reflection,
                                    &native,
                                    property,
                                );
                                content.value.push_str(
                                    &resolve::markup(extra, services.caps.hover_markdown).value,
                                );
                            }
                        }
                    }
                    return Ok(Some(found));
                }
            }
            services.caps
        };
        let resolved = context::resolve(&path, &text, position);
        if let Some(found) = crate::schema_hover::hover(&path, &text, &resolved, caps) {
            return Ok(Some(found));
        }
        let line = text.lines().nth(position.line as usize).unwrap_or("");
        let word = word_at(line, position.character as usize);
        if word.is_empty() {
            return Ok(None);
        }
        let index = self.reflection(&serde_json::Value::Null).await;
        let mut context = crate::intellisense::context(&text, position);
        if !word.starts_with('/') {
            context.property = Some(word.clone());
            if let Some(property) = crate::intellisense::property(&index, &context) {
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(resolve::markup(
                        crate::intellisense::contextual_property_markdown(
                            &index, &context, property,
                        ),
                        caps.hover_markdown,
                    )),
                    range: None,
                }));
            }
        }
        Ok(index.class_info(&word).map(|class| Hover {
            contents: HoverContents::Markup(resolve::markup(
                crate::intellisense::class_markdown(class, &word),
                caps.hover_markdown,
            )),
            range: None,
        }))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(root) = self.root().await else {
            return Ok(None);
        };
        let Some((path, text)) = self.document(&uri).await else {
            return Ok(None);
        };
        let sources = self.sources().await;
        let services = if mentions_config(&text) {
            Some(self.services().await)
        } else {
            None
        };
        let open = self.open_urls().await;
        let found = tokio::task::spawn_blocking(move || {
            if let Some(services) = &services {
                let env = env(&path, &text, services);
                if let Some(location) = config_nav::definition(&env, position) {
                    return Some(location);
                }
            }
            navigation::definition(&root, &uri, &text, position, &sources)
        })
        .await
        .map_err(|error| RpcError::invalid_params(format!("definition task failed: {error}")))?;
        Ok(found.map(|location| {
            GotoDefinitionResponse::Scalar(Location {
                uri: client_url(location.uri, &open),
                range: location.range,
            })
        }))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let include_declaration = params.context.include_declaration;
        let Some(root) = self.root().await else {
            return Ok(None);
        };
        let Some((path, text)) = self.document(&uri).await else {
            return Ok(None);
        };
        let sources = self.sources().await;
        let services = self.services().await;
        let open = self.open_urls().await;
        // The search reads every document: off the async threads, on the indexed texts.
        let found = tokio::task::spawn_blocking(move || {
            if mentions_config(&text) || kdf_validation::config::is_config_file(&path) {
                let env = env(&path, &text, &services);
                if let Some(found) =
                    config_nav::references(&env, &root, position, &sources, include_declaration)
                {
                    return found;
                }
            }
            navigation::references(&root, &uri, &text, position, &sources, include_declaration)
        })
        .await
        .map_err(|error| RpcError::invalid_params(format!("references task failed: {error}")))?;
        Ok(Some(
            found
                .into_iter()
                .map(|location| Location {
                    uri: client_url(location.uri, &open),
                    range: location.range,
                })
                .collect(),
        ))
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let Some((path, text)) = self.document(&params.text_document.uri).await else {
            return Ok(None);
        };
        let mut actions = Vec::new();
        let configured = params.context.diagnostics.iter().any(|diagnostic| {
            matches!(&diagnostic.code, Some(NumberOrString::String(code))
                if code.starts_with("config.") || code.starts_with("template.") || code.starts_with("setting."))
        });
        if configured {
            let services = self.services().await;
            let env = env(&path, &text, &services);
            actions.extend(
                actions::code_actions(&env, &params.text_document.uri, &params.context.diagnostics)
                    .into_iter()
                    .map(CodeActionOrCommand::CodeAction),
            );
        }
        if params.context.diagnostics.iter().any(|diagnostic| matches!(&diagnostic.code,
            Some(NumberOrString::String(code)) if matches!(code.as_str(), "datapack.unknown-class" | "datapack.unknown-property" | "datapack.outdated-name"))) {
            actions.extend(self.reflection_fixes(&params.text_document.uri, &params.context.diagnostics).await?);
        }
        for diagnostic in params.context.diagnostics {
            let code = diagnostic.code.as_ref().and_then(|code| match code {
                NumberOrString::String(value) => Some(value.as_str()),
                _ => None,
            });
            if matches!(code, Some("schema.enum" | "semantic.invalid-op")) {
                let Some((suggestion, edit)) = operation_fix(&text, diagnostic.range) else {
                    continue;
                };
                let mut changes = HashMap::new();
                changes.insert(params.text_document.uri.clone(), vec![edit]);
                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: format!("Change operation to {suggestion}"),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diagnostic]),
                    edit: Some(WorkspaceEdit {
                        changes: Some(changes),
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
            }
        }
        Ok(Some(actions))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let Some(path) =
            docs::path_of(&params.text_document.uri).map(|path| docs::canonical(&path))
        else {
            return Ok(None);
        };
        let (snapshot, _, epoch) = self.semantic_snapshot(false).await?;
        let symbols = snapshot
            .symbols
            .iter()
            .filter(|symbol| docs::key(&symbol.path) == docs::key(&path))
            .map(|symbol| {
                #[allow(deprecated)]
                DocumentSymbol {
                    name: symbol.id.clone(),
                    detail: Some(format!("{} · {}", symbol.pack_ref, symbol.kind)),
                    kind: if symbol.identity.namespace == kdf_validation::GeneratedNamespace::Class
                    {
                        SymbolKind::CLASS
                    } else {
                        SymbolKind::OBJECT
                    },
                    tags: None,
                    deprecated: None,
                    range: crate::authoring::lsp_range(symbol.range),
                    selection_range: crate::authoring::lsp_range(symbol.range),
                    children: None,
                }
            })
            .collect();
        self.check_epoch(epoch)?;
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        if !self.caps().versioned_edits {
            return Ok(None);
        }
        let Some(path) =
            docs::path_of(&params.text_document.uri).map(|path| docs::canonical(&path))
        else {
            return Ok(None);
        };
        let (snapshot, _, epoch) = self.semantic_snapshot(true).await?;
        let at = kdf_validation::Position {
            line: params.position.line,
            character: params.position.character,
        };
        let Ok(symbol) = snapshot.symbol_at(&path, at) else {
            return Ok(None);
        };
        if snapshot
            .rename(&snapshot.snapshot, &path, at, &symbol.id)
            .is_err()
        {
            return Ok(None);
        }
        let range = snapshot
            .references
            .iter()
            .find(|reference| {
                reference.path == path
                    && reference
                        .range
                        .is_some_and(|range| kdf_validation::authoring::contains(range, at))
            })
            .and_then(|reference| reference.range)
            .unwrap_or(symbol.range);
        self.check_epoch(epoch)?;
        Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
            range: crate::authoring::lsp_range(range),
            placeholder: symbol.id.clone(),
        }))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        if !self.caps().versioned_edits {
            return Err(RpcError::invalid_params("safe rename requires workspace.workspaceEdit.documentChanges; custom clients can use kdf/renameContent"));
        }
        let uri = params.text_document_position.text_document.uri;
        let path = docs::path_of(&uri)
            .map(|path| docs::canonical(&path))
            .ok_or_else(|| RpcError::invalid_params("rename requires a file URI"))?;
        let (snapshot, _, _) = self.semantic_snapshot(true).await?;
        let document = snapshot
            .documents
            .iter()
            .find(|document| docs::key(&document.path) == docs::key(&path))
            .ok_or_else(|| RpcError::invalid_params("source document is unavailable"))?;
        let plan = self
            .rename_content(serde_json::json!({"snapshot": snapshot.snapshot,
            "textDocument": {"uri": uri, "version": document.version},
            "position": params.text_document_position.position, "newName": params.new_name}))
            .await?;
        serde_json::from_value(plan["workspaceEdit"].clone())
            .map(Some)
            .map_err(|_| RpcError::internal_error())
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;
        let Some(text) = self.source(&uri).await else {
            return Ok(None);
        };
        if text.is_empty() || text.ends_with('\n') {
            return Ok(None);
        }
        // Full reserialization would erase comments and reorder user-authored YAML. Keep formatting lossless.
        if serde_yaml::Deserializer::from_str(&text)
            .any(|document| serde_yaml::Value::deserialize(document).is_err())
        {
            return Ok(None);
        }
        let line = text.bytes().filter(|byte| *byte == b'\n').count() as u32;
        let tail = text.rsplit('\n').next().unwrap_or("");
        let position = Position {
            line,
            character: tail.encode_utf16().count() as u32,
        };
        let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
        Ok(Some(vec![TextEdit {
            range: Range {
                start: position,
                end: position,
            },
            new_text: ending.into(),
        }]))
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some((path, text)) = self.document(&uri).await else {
            return Ok(None);
        };
        let services = self.services().await;
        let env = env(&path, &text, &services);
        let resolved = context::resolve(&path, &text, position);
        Ok(signature::signature_help(&env, &resolved))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixture::Fixture;
    use serde_json::json;

    fn caps(value: serde_json::Value) -> ClientCaps {
        let capabilities: ClientCapabilities = serde_json::from_value(value).unwrap();
        client_caps(&capabilities)
    }

    #[test]
    fn native_domain_items_precede_optional_operand_sources() {
        let fixture = Fixture::new();
        let source = Fixture::op_doc(
            "/Script/FactoryGame.FGRecipe",
            "mManufactoringDuration",
            "set",
            "|",
        );
        fixture.with_env(
            "ConfigDemo/tuning.cdo.yml",
            &source,
            false,
            true,
            |env, resolved| {
                assert!(env.workspace.config.fields().next().is_some());
                assert!(crate::semantic::operand_context(env, resolved).is_some());
                let native = CompletionResponse::Array(vec![CompletionItem::default()]);
                assert!(!specialized_completion(env, resolved, Some(&native)));
                let empty = CompletionResponse::Array(Vec::new());
                assert!(specialized_completion(env, resolved, Some(&empty)));
                assert!(specialized_completion(env, resolved, None));
                let native_list = CompletionResponse::List(tower_lsp::lsp_types::CompletionList {
                    is_incomplete: true,
                    items: vec![CompletionItem::default()],
                });
                assert!(!specialized_completion(env, resolved, Some(&native_list)));
                let empty_list = CompletionResponse::List(tower_lsp::lsp_types::CompletionList {
                    is_incomplete: false,
                    items: Vec::new(),
                });
                assert!(specialized_completion(env, resolved, Some(&empty_list)));
            },
        );
    }

    #[test]
    fn explicit_config_and_template_contexts_keep_priority_over_native_items() {
        let fixture = Fixture::new();
        let native = CompletionResponse::Array(vec![CompletionItem::default()]);
        let sources = [
            "type: config\nfields:\n  - id: speed\n    type: |\n".to_owned(),
            Fixture::op_doc(
                "/Script/FactoryGame.FGRecipe",
                "mManufactoringDuration",
                "set",
                "cfg:|",
            ),
            Fixture::op_doc(
                "/Script/FactoryGame.FGRecipe",
                "mManufactoringDuration",
                "set",
                "\"{cfg:|\"",
            ),
        ];
        for source in sources {
            fixture.with_env(
                "ConfigDemo/tuning.cdo.yml",
                &source,
                false,
                true,
                |env, resolved| {
                    assert!(
                        specialized_completion(env, resolved, Some(&native)),
                        "explicit specialized context: {source}"
                    );
                },
            );
        }
    }

    #[test]
    fn nothing_declared_means_plain_text_without_offsets_and_default_commands() {
        let none = caps(json!({}));
        assert!(!none.snippets && !none.completion_markdown && !none.resolve_documentation);
        assert!(!none.hover_markdown && !none.signature_markdown && !none.label_offsets);
        assert!(!none.signature_active_parameter);
        assert!(none.client_commands, "the desktop client sends no options");
    }

    #[test]
    fn the_desktop_declarations_are_read() {
        let desktop = caps(json!({"textDocument": {
            "completion": {"completionItem": {"snippetSupport": true, "documentationFormat": ["markdown", "plaintext"]}},
            "hover": {"contentFormat": ["markdown", "plaintext"]},
            "signatureHelp": {"signatureInformation": {
                "documentationFormat": ["markdown", "plaintext"],
                "parameterInformation": {"labelOffsetSupport": true},
                "activeParameterSupport": true
            }}
        }}));
        assert!(desktop.snippets && desktop.completion_markdown && desktop.hover_markdown);
        assert!(desktop.signature_markdown && desktop.label_offsets);
        assert!(desktop.signature_active_parameter);
        assert!(!desktop.resolve_documentation, "no resolveSupport declared");
    }

    #[test]
    fn resolve_support_needs_the_documentation_property() {
        let lazy = caps(json!({"textDocument": {"completion": {"completionItem": {
            "resolveSupport": {"properties": ["documentation", "detail"]}
        }}}}));
        assert!(lazy.resolve_documentation);
        let other = caps(json!({"textDocument": {"completion": {"completionItem": {
            "resolveSupport": {"properties": ["detail"]}
        }}}}));
        assert!(!other.resolve_documentation);
    }

    #[test]
    fn plaintext_only_clients_are_not_offered_markdown() {
        let plain = caps(json!({"textDocument": {
            "hover": {"contentFormat": ["plaintext"]},
            "signatureHelp": {"signatureInformation": {"documentationFormat": ["plaintext"]}}
        }}));
        assert!(!plain.hover_markdown && !plain.signature_markdown);
    }

    #[test]
    fn markup_formats_follow_client_preference_order() {
        for formats in [json!(["plaintext", "markdown"]), json!([])] {
            let plain = caps(json!({"textDocument": {
                "completion": {"completionItem": {"documentationFormat": formats}},
                "hover": {"contentFormat": formats},
                "signatureHelp": {"signatureInformation": {"documentationFormat": formats}}
            }}));
            assert!(
                !plain.completion_markdown && !plain.hover_markdown && !plain.signature_markdown
            );
        }
    }

    #[test]
    fn edits_name_open_documents_like_the_client() {
        let plain = Url::parse("file:///C:/Modding/DataForge/x.yml").unwrap();
        let vscode = Url::parse("file:///c%3A/Modding/DataForge/x.yml").unwrap();
        let mut open = HashMap::new();
        if let Some(path) = docs::path_of(&vscode) {
            open.insert(docs::key(&path), vscode.clone());
        }
        // Where the file URL names a Windows drive, the plain spelling maps to the client's.
        if cfg!(windows) {
            assert_eq!(client_url(plain, &open), vscode);
        }
        let other = Url::parse("file:///elsewhere/y.yml").unwrap();
        assert_eq!(client_url(other.clone(), &open), other);
    }
}
