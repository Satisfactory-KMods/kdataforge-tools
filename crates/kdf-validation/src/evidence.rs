//! Versioned diagnostic evidence shared by lint, explanations and editor actions.
use crate::{Diagnostic, Range, ReflectionIndex, ReflectionStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub const CONTRACT_VERSION: u32 = 1;

pub fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedReplacement {
    pub kind: String,
    pub old_name: String,
    pub new_name: String,
    pub owner: Option<String>,
    pub authority: String,
    pub reflection_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub schema_version: u32,
    /// `verified` only for an explicit, unique, validated exporter redirect.
    pub status: String,
    pub reason: String,
    pub reflection_digest: Option<String>,
    pub replacement: Option<VerifiedReplacement>,
}

impl Evidence {
    pub fn unknown(reason: impl Into<String>, reflection_digest: Option<String>) -> Self {
        Self {
            schema_version: CONTRACT_VERSION,
            status: "unknown".into(),
            reason: reason.into(),
            reflection_digest,
            replacement: None,
        }
    }
}

impl ReflectionIndex {
    /// Opt-in exporter metadata, never a similarity guess or native/Blueprint remap.
    /// A complete bound export, explicit verification and an existing destination are required.
    pub fn replacement_evidence(&self, kind: &str, owner: Option<&str>, old: &str) -> Evidence {
        let unknown = |reason| Evidence::unknown(reason, self.digest.clone());
        if self.status != ReflectionStatus::Ready || self.digest.is_none() {
            return unknown("A complete reflection snapshot is required to verify a replacement");
        }
        let candidates: Vec<_> = self
            .replacements
            .iter()
            .filter(|record| {
                record.get("kind").and_then(Value::as_str) == Some(kind)
                    && record.get("oldName").and_then(Value::as_str) == Some(old)
                    && record.get("owner").and_then(Value::as_str) == owner
            })
            .collect();
        if candidates.len() != 1 {
            return unknown(
                "No unique authoritative replacement metadata in the bound reflection snapshot",
            );
        }
        let record = candidates[0];
        let new = record.get("newName").and_then(Value::as_str).unwrap_or("");
        if record.get("verified").and_then(Value::as_bool) != Some(true)
            || record.get("authority").and_then(Value::as_str) != Some("unreal-core-redirects")
            || new.is_empty()
            || new == old
            || new.contains(['\n', '\r', '\'', '"', '{', '}'])
        {
            return unknown("Replacement metadata is unverified or has an unsupported authority");
        }
        let target_exists = match kind {
            "class" => {
                owner.is_none()
                    && new.starts_with('/')
                    && new
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_'))
                    && self.contains_class(new)
            }
            "property" => owner.is_some_and(|owner| {
                new.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
                    && new
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                    && self.find_property(owner, new).is_some()
            }),
            _ => false,
        };
        if !target_exists
            || self.replacements.iter().any(|record| {
                record.get("kind").and_then(Value::as_str) == Some(kind)
                    && record.get("owner").and_then(Value::as_str) == owner
                    && record.get("oldName").and_then(Value::as_str) == Some(new)
            })
        {
            return unknown("Replacement destination is absent or itself redirected");
        }
        Evidence {
            schema_version: CONTRACT_VERSION,
            status: "verified".into(),
            reason: "Explicit verified Unreal Core Redirect; destination exists in this complete bound export".into(),
            reflection_digest: self.digest.clone(),
            replacement: Some(VerifiedReplacement {
                kind: kind.into(), old_name: old.into(), new_name: new.into(), owner: owner.map(str::to_owned),
                authority: "unreal-core-redirects".into(), reflection_digest: self.digest.clone().unwrap(),
            }),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Explanation {
    pub schema_version: u32,
    pub id: String,
    pub rule_id: String,
    pub message: String,
    pub source: String,
    pub file: Option<PathBuf>,
    pub document: Option<usize>,
    pub yaml_path: String,
    pub range: Range,
    pub evidence: Evidence,
    pub action: String,
}

/// Stable under diagnostic reordering; changes when the rule, location or evidence changes.
pub fn explain(diagnostic: &Diagnostic) -> Explanation {
    let evidence = diagnostic.evidence.clone().unwrap_or_else(|| {
        Evidence::unknown(
            "Lint rule reports this location; no verified automatic replacement is available",
            None,
        )
    });
    let identity = serde_json::to_vec(&serde_json::json!([
        diagnostic.code,
        diagnostic.file,
        diagnostic.document,
        diagnostic.yaml_path,
        diagnostic.range,
        diagnostic.message,
        evidence,
    ]))
    .expect("serializable evidence");
    Explanation {
        schema_version: CONTRACT_VERSION,
        id: digest(identity),
        rule_id: diagnostic.code.clone(),
        message: diagnostic.message.clone(),
        source: diagnostic.source.clone(),
        file: diagnostic.file.clone(),
        document: diagnostic.document,
        yaml_path: diagnostic.yaml_path.clone(),
        range: diagnostic.range,
        action: if evidence.replacement.is_some() {
            "Review the verified replacement against the current document version"
        } else {
            "Inspect the reported YAML location and rule; no automatic replacement is proven"
        }
        .into(),
        evidence,
    }
}
