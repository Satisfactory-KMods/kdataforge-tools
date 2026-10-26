//! Test fixture: a two-pack configuration workspace on disk plus a synthetic reflection archive,
//! with helpers that run the language features on text with a `|` cursor marker.

use crate::completion::{complete, Completions};
use crate::context::{resolve, testing::marked};
use crate::semantic::Env;
use crate::state::ClientCaps;
use crate::test_support::{self as support, ArchiveOptions};
use crate::workspace::Workspace;
use kdf_validation::ReflectionIndex;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::{NamedTempFile, TempDir};
use tower_lsp::lsp_types::{Hover, HoverContents, SignatureHelp};

pub(crate) struct Fixture {
    _dir: TempDir,
    _archive: NamedTempFile,
    pub(crate) root: PathBuf,
    pub(crate) reflection: Arc<ReflectionIndex>,
    pub(crate) workspace: Workspace,
}

pub(crate) const ITEM: &str = "/Script/FactoryGame.FGItemDescriptor";

impl Fixture {
    pub(crate) fn new() -> Self {
        Self::build(&[], Vec::new())
    }

    pub(crate) fn with_files(files: &[(&str, &str)]) -> Self {
        Self::build(files, Vec::new())
    }

    pub(crate) fn build(files: &[(&str, &str)], extra_classes: Vec<Value>) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dunce::canonicalize(dir.path())
            .expect("canonical")
            .join("DataForge");
        support::config_workspace(&root);
        for (relative, content) in files {
            support::write(&root, relative, content);
        }
        let archive = support::archive_with(extra_classes, &ArchiveOptions::default());
        let reflection = Arc::new(ReflectionIndex::load(archive.path()));
        let workspace = Workspace::build(&root, &[]);
        Self {
            _dir: dir,
            _archive: archive,
            root,
            reflection,
            workspace,
        }
    }

    /// Runs `feature` for the cursor marked with `|` in `source`.
    pub(crate) fn with_env<R>(
        &self,
        relative: &str,
        source: &str,
        snippets: bool,
        markdown: bool,
        feature: impl FnOnce(&Env<'_>, &crate::context::Resolved) -> R,
    ) -> R {
        let path = self.root.join(relative);
        let (text, position) = marked(source);
        let env = Env {
            path: &path,
            text: &text,
            reflection: &self.reflection,
            workspace: &self.workspace,
            caps: ClientCaps {
                snippets,
                completion_markdown: markdown,
                hover_markdown: true,
                signature_markdown: markdown,
                label_offsets: true,
                ..ClientCaps::default()
            },
        };
        feature(&env, &resolve(&path, &text, position))
    }

    /// Runs `feature` on a whole document (no cursor).
    pub(crate) fn with_text<R>(
        &self,
        relative: &str,
        text: &str,
        feature: impl FnOnce(&Env<'_>) -> R,
    ) -> R {
        let path = self.root.join(relative);
        let env = Env {
            path: &path,
            text,
            reflection: &self.reflection,
            workspace: &self.workspace,
            caps: ClientCaps {
                completion_markdown: true,
                hover_markdown: true,
                signature_markdown: true,
                label_offsets: true,
                ..ClientCaps::default()
            },
        };
        feature(&env)
    }

    pub(crate) fn run_with(
        &self,
        relative: &str,
        source: &str,
        snippets: bool,
        markdown: bool,
    ) -> Completions {
        self.with_env(relative, source, snippets, markdown, complete)
    }

    pub(crate) fn run(&self, relative: &str, source: &str) -> Completions {
        self.run_with(relative, source, false, true)
    }

    /// Runs completion for a client with exactly these capabilities.
    pub(crate) fn run_caps(&self, relative: &str, source: &str, caps: ClientCaps) -> Completions {
        let path = self.root.join(relative);
        let (text, position) = marked(source);
        let env = Env {
            path: &path,
            text: &text,
            reflection: &self.reflection,
            workspace: &self.workspace,
            caps,
        };
        complete(&env, &resolve(&path, &text, position))
    }

    /// Document with one operation of `ConfigDemo/tuning.cdo.yml`.
    pub(crate) fn op_doc(target: &str, path: &str, op: &str, value: &str) -> String {
        format!(
            "type: cdo\npatches:\n  - target: {target}\n    properties:\n      - path: {path}\n        op: {op}\n        value: {value}\n"
        )
    }

    /// Completion inside an operation of `ConfigDemo/tuning.cdo.yml`.
    pub(crate) fn op(&self, target: &str, path: &str, op: &str, value: &str) -> Completions {
        self.run(
            "ConfigDemo/tuning.cdo.yml",
            &Self::op_doc(target, path, op, value),
        )
    }

    /// Hover text and the hovered text of the range.
    pub(crate) fn hover(&self, relative: &str, source: &str) -> Option<(String, String)> {
        let (text, _) = marked(source);
        let found: Option<Hover> =
            self.with_env(relative, source, false, true, crate::hover::hover);
        let found = found?;
        let HoverContents::Markup(markup) = found.contents else {
            panic!("markdown hover expected");
        };
        let range = found.range.expect("hover range");
        let line = text.lines().nth(range.start.line as usize).unwrap_or("");
        let start = crate::context::byte_at(line, range.start.character);
        let end = crate::context::byte_at(line, range.end.character);
        Some((markup.value, line[start..end].to_owned()))
    }

    pub(crate) fn signature(&self, relative: &str, source: &str) -> Option<SignatureHelp> {
        self.with_env(
            relative,
            source,
            false,
            true,
            crate::signature::signature_help,
        )
    }
}
