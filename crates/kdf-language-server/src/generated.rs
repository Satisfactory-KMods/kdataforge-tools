use kdf_validation::{infer_type, SourceMap};
use serde::Deserialize;
use std::path::Path;
use tower_lsp::lsp_types::{Position, Range};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GeneratedKind {
    Class,
    Asset,
}

#[derive(Clone, Debug)]
pub(crate) struct GeneratedDeclaration {
    pub(crate) id: String,
    pub(crate) kind: GeneratedKind,
    pub(crate) range: Range,
    /// Document type that declares the entry (`item`, `class`, `dataasset`, ...).
    pub(crate) doc_type: String,
    /// Explicit `parent`, else the default base class of the entry list (`items` and the like).
    pub(crate) parent: Option<String>,
}

pub(crate) struct SoundReference {
    pub(crate) id: String,
    pub(crate) range: Range,
}

/// Recognize reflected ADA event assignments, without treating arbitrary string values as asset references.
pub(crate) fn bare_sound_references(text: &str) -> Vec<SoundReference> {
    fn visit(
        value: &serde_yaml::Value,
        pointer: &str,
        document: usize,
        source: &SourceMap,
        text: &str,
        references: &mut Vec<SoundReference>,
    ) {
        if let Some(mapping) = value.as_mapping() {
            if value
                .get("path")
                .and_then(serde_yaml::Value::as_str)
                .is_some_and(|path| {
                    path.rsplit('.')
                        .next()
                        .unwrap_or(path)
                        .eq_ignore_ascii_case("mAudioEvent")
                })
            {
                if let Some(id) = value
                    .get("value")
                    .and_then(serde_yaml::Value::as_str)
                    .filter(|id| {
                        id.starts_with(|c: char| c.is_ascii_alphabetic())
                            && id
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
                    })
                {
                    let located = source.location(document, &format!("{pointer}/value"));
                    if let Some(line) = text.lines().nth(located.start.line as usize) {
                        if let Some((start, end)) = line
                            .match_indices(id)
                            .map(|(byte, _)| {
                                let start = line[..byte].encode_utf16().count() as u32;
                                (start, start + id.encode_utf16().count() as u32)
                            })
                            .find(|(start, _)| {
                                // SourceMap can return a one-character range when a
                                // scalar's parser span ends on the following line.
                                *start >= located.start.character
                                    && *start <= located.start.character + 1
                            })
                        {
                            references.push(SoundReference {
                                id: id.to_owned(),
                                range: Range {
                                    start: Position {
                                        line: located.start.line,
                                        character: start,
                                    },
                                    end: Position {
                                        line: located.start.line,
                                        character: end,
                                    },
                                },
                            });
                        }
                    }
                }
            }
            for (key, child) in mapping {
                if let Some(key) = key.as_str() {
                    let key = key.replace('~', "~0").replace('/', "~1");
                    visit(
                        child,
                        &format!("{pointer}/{key}"),
                        document,
                        source,
                        text,
                        references,
                    );
                }
            }
        } else if let Some(items) = value.as_sequence() {
            for (index, child) in items.iter().enumerate() {
                visit(
                    child,
                    &format!("{pointer}/{index}"),
                    document,
                    source,
                    text,
                    references,
                );
            }
        }
    }
    let Ok(source) = SourceMap::parse(text) else {
        return Vec::new();
    };
    let mut references = Vec::new();
    for (index, document) in serde_yaml::Deserializer::from_str(text).enumerate() {
        if let Ok(value) = serde_yaml::Value::deserialize(document) {
            visit(&value, "", index, &source, text, &mut references);
        }
    }
    references
}

pub(crate) fn generated_declarations(path: &Path, text: &str) -> Vec<GeneratedDeclaration> {
    let Ok(source) = SourceMap::parse(text) else {
        return Vec::new();
    };
    let Ok(documents) = serde_yaml::Deserializer::from_str(text)
        .map(serde_yaml::Value::deserialize)
        .collect::<Result<Vec<_>, _>>()
    else {
        return Vec::new();
    };
    let multi = documents.len() > 1;
    let mut declarations = Vec::new();
    for (document_index, document) in documents.iter().enumerate() {
        // The same inference the validator uses: explicit `type`, else the `<name>.<type>.yml` suffix.
        let explicit = document
            .get("type")
            .and_then(serde_yaml::Value::as_str)
            .map(|kind| serde_json::json!({ "type": kind }))
            .unwrap_or(serde_json::Value::Null);
        let Some(kind) = infer_type(path, &explicit, multi) else {
            continue;
        };
        let Some((entry_key, generated_kind)) = generated_entry_root(&kind) else {
            continue;
        };
        let Some(entries) = document
            .get(entry_key)
            .and_then(serde_yaml::Value::as_sequence)
        else {
            continue;
        };
        for (entry_index, entry) in entries.iter().enumerate() {
            let Some(id) = entry.get("id").and_then(serde_yaml::Value::as_str) else {
                continue;
            };
            let located =
                source.location(document_index, &format!("/{entry_key}/{entry_index}/id"));
            let parent = entry
                .get("parent")
                .and_then(serde_yaml::Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    kdf_validation::slots::default_base_class(entry_key).map(str::to_owned)
                });
            declarations.push(GeneratedDeclaration {
                id: id.to_owned(),
                kind: generated_kind,
                range: Range {
                    start: Position {
                        line: located.start.line,
                        character: located.start.character,
                    },
                    end: Position {
                        line: located.end.line,
                        character: located.end.character,
                    },
                },
                doc_type: kind.clone(),
                parent,
            });
        }
    }
    declarations
}

fn generated_entry_root(kind: &str) -> Option<(&'static str, GeneratedKind)> {
    match kind {
        "asset" | "dataasset" => Some(("assets", GeneratedKind::Asset)),
        "curve" => Some(("curves", GeneratedKind::Asset)),
        "sound" => Some(("sounds", GeneratedKind::Asset)),
        "item" => Some(("items", GeneratedKind::Class)),
        "building" => Some(("buildings", GeneratedKind::Class)),
        "resource" => Some(("resources", GeneratedKind::Class)),
        "unlock" => Some(("unlocks", GeneratedKind::Class)),
        "recipe" => Some(("recipes", GeneratedKind::Class)),
        "schematic" => Some(("schematics", GeneratedKind::Class)),
        "research" => Some(("research", GeneratedKind::Class)),
        "class" => Some(("classes", GeneratedKind::Class)),
        _ => None,
    }
}
