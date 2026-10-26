//! Conservative, versioned content model. All methods are read-only; edits are reviewable plans.
use crate::{
    evidence::{digest, CONTRACT_VERSION},
    generated_identity, infer_type, GeneratedNamespace, Position, Range, ReflectionIndex,
    ReflectionStatus, SourceMap,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const COMPLETION_LIMIT: usize = 200;
const MAX_FILES: usize = 10_000;
const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct VersionedInput {
    pub path: PathBuf,
    pub version: Option<i32>,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotToken {
    pub schema_version: u32,
    pub generation: u64,
    pub digest: String,
    pub reflection_digest: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentIdentity {
    /// Runtime-normalized pack, not a filesystem path or display name.
    pub pack: String,
    pub namespace: GeneratedNamespace,
    pub id: String,
}

impl ContentIdentity {
    pub fn new(pack: &str, kind: &str, id: &str) -> Option<Self> {
        if entries(kind).is_empty() {
            return None;
        }
        let (namespace, id) = generated_identity(kind, id)?;
        Some(Self {
            pack: crate::rules::runtime_identity(pack),
            namespace,
            id,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSnapshot {
    pub path: PathBuf,
    /// LSP version for open buffers; null for disk-only documents. Digest is always required.
    pub version: Option<i32>,
    pub digest: String,
    #[serde(skip)]
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentSymbol {
    pub identity: ContentIdentity,
    pub pack_ref: String,
    pub id: String,
    pub kind: String,
    pub path: PathBuf,
    pub document: usize,
    pub range: Range,
    pub generated_path: Option<String>,
    pub parent: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentReference {
    pub path: PathBuf,
    pub document: usize,
    pub range: Option<Range>,
    pub value: String,
    pub candidates: Vec<ContentIdentity>,
    pub typed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSnapshot {
    pub snapshot: SnapshotToken,
    pub root: PathBuf,
    /// Complete authored YAML inventory, independent of reflection completeness.
    pub complete: bool,
    pub reflection_complete: bool,
    pub limitations: Vec<String>,
    pub rename_blockers: Vec<String>,
    pub documents: Vec<DocumentSnapshot>,
    pub symbols: Vec<ContentSymbol>,
    pub references: Vec<ContentReference>,
}

fn token(value: &str) -> bool {
    value.starts_with(|ch: char| ch.is_ascii_alphabetic())
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
}

fn runtime_token(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn generated_path(pack: &str, id: &str, namespace: GeneratedNamespace) -> Option<String> {
    let folder = match namespace {
        GeneratedNamespace::Class => "Gen",
        GeneratedNamespace::Asset => "GenAssets",
        _ => return None,
    };
    let pack = runtime_token(pack);
    Some(format!(
        "/KDataForge/{folder}/{pack}.Gen_{pack}_{}",
        runtime_token(id)
    ))
}

fn entries(kind: &str) -> &'static [&'static str] {
    match kind {
        "asset" | "dataasset" => &["assets"],
        "curve" => &["curves"],
        "sound" => &["sounds"],
        "item" => &["items"],
        "building" => &["buildings"],
        "resource" => &["resources"],
        "unlock" => &["unlocks"],
        "recipe" => &["recipes"],
        "schematic" => &["schematics"],
        "research" => &["research"],
        "class" => &["classes"],
        "eventrule" => &["rules"],
        "resourcenode" => &["add", "replace"],
        _ => &[],
    }
}

fn yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("yml") || extension.eq_ignore_ascii_case("yaml")
        })
}

fn parsed(text: &str) -> Result<Vec<Value>, String> {
    if text.len() > crate::document::MAX_YAML_BYTES {
        return Err("file exceeds 8 MiB".into());
    }
    serde_yaml::Deserializer::from_str(text)
        .take(1025)
        .map(|document| {
            let value =
                serde_yaml::Value::deserialize(document).map_err(|error| error.to_string())?;
            serde_json::to_value(value).map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()
        .and_then(|documents| {
            if documents.len() > 1024 {
                Err("file exceeds 1024 YAML documents".into())
            } else {
                Ok(documents)
            }
        })
}

impl SemanticSnapshot {
    pub fn load(
        root: &Path,
        buffers: &[VersionedInput],
        reflection: &ReflectionIndex,
        generation: u64,
    ) -> Result<Self, String> {
        let root = dunce::canonicalize(root).map_err(|error| error.to_string())?;
        if !root.is_dir() {
            return Err("snapshot root must be a directory".into());
        }
        let mut inputs = BTreeMap::new();
        let mut limitations = Vec::new();
        let mut total = 0;
        for entry in walkdir::WalkDir::new(&root)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(crate::runtime_authored_entry)
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    limitations.push(error.to_string());
                    continue;
                }
            };
            if entry.file_type().is_symlink() {
                limitations.push(format!("symlink omitted: {}", entry.path().display()));
                continue;
            }
            if !entry.file_type().is_file() || !yaml(entry.path()) {
                continue;
            }
            if inputs.len() >= MAX_FILES || total >= MAX_BYTES {
                limitations.push("snapshot file/byte limit reached".into());
                break;
            }
            match crate::text_encoding::read_text_file(
                entry.path(),
                crate::document::MAX_YAML_BYTES as u64,
            ) {
                Ok(text) => {
                    total += text.len();
                    inputs.insert(
                        entry.path().to_path_buf(),
                        VersionedInput {
                            path: entry.path().to_path_buf(),
                            version: None,
                            text,
                        },
                    );
                }
                Err(error) => limitations.push(format!("{}: {error}", entry.path().display())),
            }
        }
        for buffer in buffers {
            let path =
                crate::canonical_file_path(&buffer.path).map_err(|error| error.to_string())?;
            if path.starts_with(&root)
                && yaml(&path)
                && crate::is_runtime_authored_path(&path, &root)
            {
                inputs.insert(
                    path.clone(),
                    VersionedInput {
                        path,
                        ..buffer.clone()
                    },
                );
            }
        }
        let mut snapshot = Self::from_inputs(
            &root,
            inputs.into_values().collect(),
            reflection,
            generation,
        );
        snapshot.limitations.extend(limitations);
        snapshot.complete = snapshot.limitations.is_empty();
        snapshot.seal();
        Ok(snapshot)
    }

    /// Caller supplies a complete inventory; useful to reuse an existing workspace service.
    pub fn from_inputs(
        root: &Path,
        mut inputs: Vec<VersionedInput>,
        reflection: &ReflectionIndex,
        generation: u64,
    ) -> Self {
        inputs.sort_by(|a, b| a.path.cmp(&b.path));
        let mut result = Self {
            snapshot: SnapshotToken {
                schema_version: CONTRACT_VERSION,
                generation,
                digest: String::new(),
                reflection_digest: reflection.digest.clone(),
            },
            root: root.into(),
            complete: true,
            reflection_complete: reflection.status == ReflectionStatus::Ready,
            limitations: Vec::new(),
            rename_blockers: Vec::new(),
            documents: Vec::new(),
            symbols: Vec::new(),
            references: Vec::new(),
        };
        let mut parsed_files = Vec::new();
        let mut manifests = BTreeMap::new();
        let mut pack_names = BTreeSet::new();
        let mut total = 0;
        static ALIASES: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let aliases =
            ALIASES.get_or_init(|| regex::Regex::new(r"(?:^|[\s\[,{:])[*&][A-Za-z0-9_-]").unwrap());
        for input in inputs {
            total += input.text.len();
            if result.documents.len() >= MAX_FILES || total > MAX_BYTES {
                result
                    .limitations
                    .push("snapshot file/byte limit reached".into());
                break;
            }
            if !input.path.starts_with(root) {
                result
                    .limitations
                    .push("document outside snapshot root".into());
                continue;
            }
            // Aliases and anchors can share source ranges or hide generated references.
            if aliases.is_match(&input.text) {
                result
                    .rename_blockers
                    .push("YAML anchors/aliases require manual reference review".into());
            }
            let docs = parsed(&input.text);
            if crate::project::is_pack_manifest(&input.path) {
                if let Ok(docs) = &docs {
                    if docs.len() == 1 {
                        if let Some(reference) = docs[0]
                            .get("ref")
                            .and_then(Value::as_str)
                            .filter(|value| token(value))
                        {
                            if !pack_names.insert(crate::rules::runtime_identity(reference)) {
                                result
                                    .limitations
                                    .push(format!("ambiguous pack identity: {reference}"));
                            }
                            manifests.insert(
                                input.path.parent().unwrap().to_path_buf(),
                                reference.to_owned(),
                            );
                        } else {
                            result
                                .limitations
                                .push("manifest has no static pack ref".into());
                        }
                        if docs[0]
                            .get("scripts")
                            .and_then(Value::as_array)
                            .is_some_and(|scripts| !scripts.is_empty())
                        {
                            result.rename_blockers.push("Scripts can construct content references; static rename cannot prove coverage".into());
                        }
                        if docs[0]
                            .get("redirects")
                            .and_then(Value::as_object)
                            .is_some_and(|redirects| !redirects.is_empty())
                        {
                            result.rename_blockers.push(
                                "Manifest redirects require manual review before content rename"
                                    .into(),
                            );
                        }
                    } else {
                        result.limitations.push("multi-document manifest".into());
                    }
                }
            }
            match docs {
                Ok(docs) => parsed_files.push((result.documents.len(), docs)),
                Err(error) => result
                    .limitations
                    .push(format!("{}: {error}", input.path.display())),
            }
            result.documents.push(DocumentSnapshot {
                path: input.path,
                version: input.version,
                digest: digest(&input.text),
                text: input.text,
            });
        }
        for (file, docs) in &parsed_files {
            let input = &result.documents[*file];
            if crate::project::is_pack_manifest(&input.path) {
                continue;
            }
            let Some(pack) = input
                .path
                .ancestors()
                .find_map(|parent| manifests.get(parent))
            else {
                result
                    .limitations
                    .push(format!("no owning manifest: {}", input.path.display()));
                continue;
            };
            let Ok(source) = SourceMap::parse(&input.text) else {
                result.limitations.push("source mapping unavailable".into());
                continue;
            };
            for (document, value) in docs.iter().enumerate() {
                if value.is_null() {
                    continue;
                }
                let Some(kind) = infer_type(&input.path, value, docs.len() > 1) else {
                    result.limitations.push("unknown document type".into());
                    continue;
                };
                if value.get("include").is_some() {
                    result
                        .rename_blockers
                        .push("Includes require manual review before content rename".into());
                }
                if !crate::ROOT_TYPES.contains(&kind.as_str()) || !value.is_object() {
                    result
                        .limitations
                        .push("unknown or malformed document type".into());
                    continue;
                }
                for key in entries(&kind) {
                    for (offset, entry) in value
                        .get(*key)
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .enumerate()
                    {
                        let Some(id) = entry.get("id").and_then(Value::as_str) else {
                            continue;
                        };
                        let pointer = format!("/{key}/{offset}/id");
                        let Some(range) = source.scalar_value_range(document, &pointer, id).filter(
                            |_| match kind.as_str() {
                                "eventrule" => {
                                    !id.is_empty()
                                        && id.len() <= 128
                                        && id.chars().all(|ch| {
                                            ch.is_ascii_alphanumeric()
                                                || matches!(ch, '_' | '.' | '-')
                                        })
                                }
                                "resourcenode" => !id.trim().is_empty() && !id.contains(['{', '}']),
                                _ => token(id),
                            },
                        ) else {
                            result.limitations.push(format!(
                                "nonliteral content declaration: {}",
                                input.path.display()
                            ));
                            continue;
                        };
                        let identity = ContentIdentity::new(pack, &kind, id).unwrap();
                        result.symbols.push(ContentSymbol {
                            generated_path: generated_path(pack, id, identity.namespace),
                            identity,
                            pack_ref: pack.clone(),
                            id: id.into(),
                            kind: kind.clone(),
                            path: input.path.clone(),
                            document,
                            range,
                            parent: entry
                                .get("parent")
                                .and_then(Value::as_str)
                                .or_else(|| crate::slots::default_base_class(key))
                                .map(str::to_owned),
                        });
                    }
                }
            }
        }
        let reference_index = ReferenceIndex {
            full: result
                .symbols
                .iter()
                .filter_map(|symbol| {
                    symbol
                        .generated_path
                        .as_ref()
                        .map(|path| (path.to_lowercase(), symbol.identity.clone()))
                })
                .collect(),
            local: result
                .symbols
                .iter()
                .map(|symbol| symbol.identity.clone())
                .collect(),
        };
        for (file, docs) in &parsed_files {
            let input = &result.documents[*file];
            if crate::project::is_pack_manifest(&input.path) {
                continue;
            }
            let Some(pack) = input
                .path
                .ancestors()
                .find_map(|parent| manifests.get(parent))
            else {
                continue;
            };
            let Ok(source) = SourceMap::parse(&input.text) else {
                continue;
            };
            for (document, value) in docs.iter().enumerate() {
                let Some(kind) = infer_type(&input.path, value, docs.len() > 1) else {
                    continue;
                };
                let mut references = Vec::new();
                let mut blockers = Vec::new();
                walk_references(
                    value,
                    "",
                    value,
                    &kind,
                    pack,
                    input,
                    document,
                    &source,
                    reflection,
                    &reference_index,
                    &mut references,
                    &mut blockers,
                );
                result.references.extend(references);
                result.rename_blockers.extend(blockers);
            }
        }
        result.limitations.sort();
        result.limitations.dedup();
        result.rename_blockers.sort();
        result.rename_blockers.dedup();
        result.complete = result.limitations.is_empty();
        result.seal();
        result
    }

    fn seal(&mut self) {
        self.snapshot.digest = digest(
            serde_json::to_vec(&serde_json::json!([
                self.snapshot.schema_version,
                self.root,
                self.documents,
                self.snapshot.reflection_digest,
                self.complete,
                self.reflection_complete,
                self.limitations,
                self.rename_blockers,
            ]))
            .expect("snapshot serialization"),
        );
    }

    pub fn require_current(&self, expected: &SnapshotToken) -> Result<(), String> {
        if &self.snapshot != expected {
            Err("stale snapshot: refresh before retrying".into())
        } else {
            Ok(())
        }
    }

    pub fn symbol_at(&self, path: &Path, position: Position) -> Result<&ContentSymbol, String> {
        let identities: BTreeSet<_> = self
            .symbols
            .iter()
            .filter(|symbol| symbol.path == path && contains(symbol.range, position))
            .map(|symbol| &symbol.identity)
            .chain(
                self.references
                    .iter()
                    .filter(|reference| {
                        reference.path == path
                            && reference.typed
                            && reference
                                .range
                                .is_some_and(|range| contains(range, position))
                    })
                    .flat_map(|reference| reference.candidates.iter()),
            )
            .collect();
        if identities.len() != 1 {
            return Err("position is not an unambiguous content declaration or typed reference; pack/config IDs cannot be renamed".into());
        }
        let matches: Vec<_> = self
            .symbols
            .iter()
            .filter(|symbol| Some(&symbol.identity) == identities.first().copied())
            .collect();
        if matches.len() != 1 {
            return Err("ambiguous content identity".into());
        }
        Ok(matches[0])
    }

    pub fn rename(
        &self,
        expected: &SnapshotToken,
        path: &Path,
        position: Position,
        new_name: &str,
    ) -> Result<RenamePlan, String> {
        self.require_current(expected)?;
        if !self.complete || !self.rename_blockers.is_empty() {
            return Err(format!(
                "rename coverage is incomplete: {}",
                self.limitations
                    .iter()
                    .chain(&self.rename_blockers)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        if !token(new_name) {
            return Err(
                "content ID must start with an ASCII letter and contain letters, digits, _, . or -"
                    .into(),
            );
        }
        let symbol = self.symbol_at(path, position)?;
        if !matches!(
            symbol.identity.namespace,
            GeneratedNamespace::Class | GeneratedNamespace::Asset
        ) {
            return Err("this namespace has no verified rename reference model".into());
        }
        let new_identity = ContentIdentity::new(&symbol.pack_ref, &symbol.kind, new_name).unwrap();
        if self
            .symbols
            .iter()
            .any(|other| other.identity == new_identity && !std::ptr::eq(other, symbol))
        {
            return Err("new ID collides after runtime normalization".into());
        }
        if self.references.iter().any(|reference| {
            reference.candidates.contains(&symbol.identity)
                && (!reference.typed
                    || reference.candidates.len() != 1
                    || reference.range.is_none())
        }) {
            return Err("ambiguous, dynamic or unmapped content reference; rename refused".into());
        }
        let mut edits: BTreeMap<PathBuf, Vec<PlannedEdit>> = BTreeMap::new();
        if symbol.id != new_name {
            edits
                .entry(symbol.path.clone())
                .or_default()
                .push(PlannedEdit {
                    range: symbol.range,
                    expected_text: symbol.id.clone(),
                    new_text: new_name.into(),
                });
            for reference in &self.references {
                if !reference.candidates.contains(&symbol.identity) {
                    continue;
                }
                if !reference.typed || reference.candidates.len() != 1 || reference.range.is_none()
                {
                    return Err(
                        "ambiguous, dynamic or unmapped content reference; rename refused".into(),
                    );
                }
                let old_path = symbol.generated_path.as_deref().unwrap();
                let new_path =
                    generated_path(&symbol.pack_ref, new_name, symbol.identity.namespace).unwrap();
                let value = unwrap_object_path(&reference.value);
                let replacement = if value.eq_ignore_ascii_case(old_path) {
                    reference.value.replacen(value, &new_path, 1)
                } else {
                    new_name.into()
                };
                edits
                    .entry(reference.path.clone())
                    .or_default()
                    .push(PlannedEdit {
                        range: reference.range.unwrap(),
                        expected_text: reference.value.clone(),
                        new_text: replacement,
                    });
            }
        }
        let mut documents = Vec::new();
        for (path, mut edits) in edits {
            let document = self
                .documents
                .iter()
                .find(|document| document.path == path)
                .ok_or("missing document snapshot")?;
            edits.sort_by_key(|edit| edit.range.start);
            edits.dedup_by(|a, b| a.range == b.range && a.new_text == b.new_text);
            if edits
                .windows(2)
                .any(|pair| pair[0].range.end > pair[1].range.start)
            {
                return Err("overlapping rename ranges".into());
            }
            for edit in &edits {
                let start = crate::byte_offset_at_utf16(&document.text, edit.range.start)
                    .ok_or("invalid edit start")?;
                let end = crate::byte_offset_at_utf16(&document.text, edit.range.end)
                    .ok_or("invalid edit end")?;
                if document.text.get(start..end) != Some(&edit.expected_text) {
                    return Err("edit text does not match snapshot".into());
                }
            }
            documents.push(PlannedDocument {
                path,
                version: document.version,
                digest: document.digest.clone(),
                edits,
            });
        }
        Ok(RenamePlan {
            schema_version: CONTRACT_VERSION,
            snapshot: self.snapshot.clone(),
            identity: symbol.identity.clone(),
            new_identity,
            documents,
        })
    }
}

pub fn contains(range: Range, position: Position) -> bool {
    range.start <= position && position < range.end
}

fn unwrap_object_path(value: &str) -> &str {
    if let Some((_, path)) = value.split_once('\'') {
        path.strip_suffix('\'').unwrap_or(value)
    } else {
        value
    }
}

fn namespaces(
    kind: &str,
    pointer: &str,
    document: &Value,
    reflection: &ReflectionIndex,
) -> Vec<GeneratedNamespace> {
    let segments: Vec<_> = pointer.split('/').skip(1).collect();
    if let Some(offset) = segments.iter().position(|key| *key == "properties") {
        if segments.get(offset + 2) != Some(&"value") {
            return vec![];
        }
        let entry = document.pointer(&format!("/{}", segments[..offset].join("/")));
        let op = document.pointer(&format!("/{}", segments[..offset + 2].join("/")));
        let field = op
            .and_then(|op| op.get("path"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if field == "mAudioEvent" {
            return vec![GeneratedNamespace::Asset];
        }
        let owner = entry
            .and_then(|entry| {
                ["target", "parent", "class", "ofClass", "allAssetsOfClass"]
                    .iter()
                    .find_map(|key| entry.get(*key).and_then(Value::as_str))
            })
            .or_else(|| {
                segments
                    .first()
                    .and_then(|root| crate::slots::default_base_class(root))
            });
        let Some(mut property) =
            owner.and_then(|owner| reflection.resolve_property_path(owner, field))
        else {
            return vec![];
        };
        for segment in segments.iter().skip(offset + 3) {
            if segment.parse::<usize>().is_ok() {
                let Some(inner) = property.inner else {
                    return vec![];
                };
                property = *inner;
            } else {
                let Some(field) = property
                    .struct_path
                    .as_ref()
                    .and_then(|path| reflection.find_struct_field(path, segment))
                else {
                    return vec![];
                };
                property = field;
            }
        }
        return match property.kind {
            crate::PropertyKind::Class | crate::PropertyKind::SoftClass => {
                vec![GeneratedNamespace::Class]
            }
            kind if kind.is_object_reference() => vec![GeneratedNamespace::Asset],
            _ => vec![],
        };
    }
    let keys: Vec<_> = segments
        .into_iter()
        .filter(|key| key.parse::<usize>().is_err())
        .collect();
    let Some(doc) = crate::field_docs::field(kind, &keys) else {
        return vec![];
    };
    let mut result = Vec::new();
    if doc.type_label.contains("class") {
        result.push(GeneratedNamespace::Class);
    }
    if doc.type_label.contains("object path")
        || doc.type_label.contains("asset path")
        || doc.type_label.contains("texture path")
    {
        result.push(GeneratedNamespace::Asset);
    }
    result
}

struct ReferenceIndex {
    full: BTreeMap<String, ContentIdentity>,
    local: BTreeSet<ContentIdentity>,
}

#[allow(clippy::too_many_arguments)]
fn walk_references(
    value: &Value,
    pointer: &str,
    document_value: &Value,
    kind: &str,
    pack: &str,
    input: &DocumentSnapshot,
    document: usize,
    source: &SourceMap<'_>,
    reflection: &ReflectionIndex,
    reference_index: &ReferenceIndex,
    references: &mut Vec<ContentReference>,
    blockers: &mut Vec<String>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                walk_references(
                    child,
                    &format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1")),
                    document_value,
                    kind,
                    pack,
                    input,
                    document,
                    source,
                    reflection,
                    reference_index,
                    references,
                    blockers,
                );
            }
        }
        Value::Array(list) => {
            for (index, child) in list.iter().enumerate() {
                walk_references(
                    child,
                    &format!("{pointer}/{index}"),
                    document_value,
                    kind,
                    pack,
                    input,
                    document,
                    source,
                    reflection,
                    reference_index,
                    references,
                    blockers,
                );
            }
        }
        Value::String(value) => {
            if pointer.ends_with("/id") {
                return;
            }
            let types = namespaces(kind, pointer, document_value, reflection);
            let typed = !types.is_empty();
            let unknown_operand =
                !typed && pointer.contains("/properties/") && pointer.contains("/value");
            if ((typed || unknown_operand)
                && (value.contains(['{', '}', '*', '?'])
                    || crate::refs::typed_ref_parts(value).is_some()))
                || crate::template::APPLY_PREFIXES
                    .iter()
                    .any(|prefix| value.contains(&format!("{{{prefix}:")))
            {
                blockers.push(format!(
                    "dynamic reference at {} document {document} {pointer}",
                    input.path.display()
                ));
            }
            let bare = unwrap_object_path(value);
            let mut candidates = BTreeSet::new();
            if let Some(identity) = reference_index.full.get(&bare.to_lowercase()) {
                if !typed || types.contains(&identity.namespace) {
                    candidates.insert(identity.clone());
                }
            } else if !typed && value.to_ascii_lowercase().contains("/kdataforge/gen") {
                static PATHS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
                for found in PATHS
                    .get_or_init(|| {
                        regex::Regex::new(
                            r"(?i)/KDataForge/(?:Gen|GenAssets)/[A-Za-z0-9_]+\.[A-Za-z0-9_]+",
                        )
                        .unwrap()
                    })
                    .find_iter(value)
                {
                    if let Some(identity) = reference_index.full.get(&found.as_str().to_lowercase())
                    {
                        candidates.insert(identity.clone());
                    }
                }
            }
            if (typed || unknown_operand) && token(bare) {
                for namespace in [GeneratedNamespace::Class, GeneratedNamespace::Asset] {
                    if typed && !types.contains(&namespace) {
                        continue;
                    }
                    let identity = ContentIdentity {
                        pack: crate::rules::runtime_identity(pack),
                        namespace,
                        id: crate::rules::runtime_identity(bare),
                    };
                    if reference_index.local.contains(&identity) {
                        candidates.insert(identity);
                    }
                }
            }
            let candidates: Vec<_> = candidates.into_iter().collect();
            if !candidates.is_empty() {
                references.push(ContentReference {
                    path: input.path.clone(),
                    document,
                    range: source.scalar_value_range(document, pointer, value),
                    value: value.clone(),
                    candidates,
                    typed: typed && !(token(bare) && reflection.class_path(bare).is_some()),
                });
            }
        }
        _ => {}
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedEdit {
    pub range: Range,
    pub expected_text: String,
    pub new_text: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedDocument {
    pub path: PathBuf,
    pub version: Option<i32>,
    pub digest: String,
    pub edits: Vec<PlannedEdit>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePlan {
    pub schema_version: u32,
    pub snapshot: SnapshotToken,
    pub identity: ContentIdentity,
    pub new_identity: ContentIdentity,
    pub documents: Vec<PlannedDocument>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionQuery {
    /// content, classes, or assets; no catalog dumping endpoint is added.
    pub kind: String,
    #[serde(default)]
    pub query: String,
    pub limit: Option<usize>,
    pub pack: Option<String>,
    pub namespace: Option<GeneratedNamespace>,
    pub base_class: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub label: String,
    pub insert_text: String,
    pub kind: String,
    pub identity: Option<ContentIdentity>,
    pub detail: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionPage {
    pub schema_version: u32,
    pub snapshot: SnapshotToken,
    pub items: Vec<Candidate>,
    pub is_incomplete: bool,
    pub complete: bool,
}

pub fn complete(
    snapshot: &SemanticSnapshot,
    reflection: &ReflectionIndex,
    query: &CompletionQuery,
) -> Result<CompletionPage, String> {
    if snapshot.snapshot.reflection_digest != reflection.digest {
        return Err("stale reflection snapshot".into());
    }
    if query.query.len() > 1024 {
        return Err("completion query exceeds 1024 bytes".into());
    }
    let limit = query
        .limit
        .unwrap_or(COMPLETION_LIMIT)
        .clamp(1, COMPLETION_LIMIT);
    let needle = query.query.to_lowercase();
    let matches = |label: &str, value: &str| {
        label.to_lowercase().contains(&needle) || value.to_lowercase().contains(&needle)
    };
    let mut items = Vec::new();
    let mut complete = snapshot.complete;
    match query.kind.as_str() {
        "content" => {
            for symbol in &snapshot.symbols {
                if query.pack.as_ref().is_some_and(|pack| {
                    crate::rules::runtime_identity(pack) != symbol.identity.pack
                }) || query
                    .namespace
                    .is_some_and(|namespace| namespace != symbol.identity.namespace)
                {
                    continue;
                }
                let value = symbol.generated_path.as_deref().unwrap_or(&symbol.id);
                if matches(&symbol.id, value) {
                    items.push(Candidate {
                        label: symbol.id.clone(),
                        insert_text: value.into(),
                        kind: "content".into(),
                        identity: Some(symbol.identity.clone()),
                        detail: Some(symbol.pack_ref.clone()),
                    });
                    if items.len() > limit {
                        break;
                    }
                }
            }
        }
        "classes" => {
            complete = reflection.status == ReflectionStatus::Ready;
            for class in &reflection.classes {
                if query
                    .base_class
                    .as_deref()
                    .is_some_and(|base| !reflection.is_subclass_of(&class.path, base))
                {
                    continue;
                }
                if matches(&class.name, &class.path) {
                    items.push(Candidate {
                        label: class.name.clone(),
                        insert_text: class.path.clone(),
                        kind: "class".into(),
                        identity: None,
                        detail: class.parent.clone(),
                    });
                    if items.len() > limit {
                        break;
                    }
                }
            }
        }
        "assets" => {
            let class = query
                .base_class
                .as_deref()
                .ok_or("asset completion requires baseClass")?;
            complete = reflection.asset_listing_complete(class) == Some(true);
            for asset in reflection.assets_of_class(class) {
                if matches(&asset.name, &asset.path) {
                    items.push(Candidate {
                        label: asset.name.clone(),
                        insert_text: asset.path.clone(),
                        kind: "asset".into(),
                        identity: None,
                        detail: Some(asset.asset_class.clone()),
                    });
                    if items.len() > limit {
                        break;
                    }
                }
            }
        }
        _ => return Err("kind must be content, classes or assets".into()),
    }
    let is_incomplete = items.len() > limit || !complete;
    items.truncate(limit);
    Ok(CompletionPage {
        schema_version: CONTRACT_VERSION,
        snapshot: snapshot.snapshot.clone(),
        items,
        is_incomplete,
        complete,
    })
}
