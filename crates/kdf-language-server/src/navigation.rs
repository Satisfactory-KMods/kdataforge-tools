use crate::docs::{self, Sources};
use crate::features::{is_word_char, word_at, yaml_content_prefix};
use crate::generated::{bare_sound_references, generated_declarations, GeneratedKind};
use crate::pack_scope::{
    has_manifest, is_loadable_manifest_path, is_manifest_path, manifest_path, owns_document,
};
use crate::rename::sanitize_runtime_token;
use kdf_validation::SourceMap;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::{Location, Position, Range, Url};

pub fn definition(
    root: &Path,
    uri: &Url,
    text: &str,
    position: Position,
    sources: &Sources,
) -> Option<Location> {
    let line = text.lines().nth(position.line as usize)?;
    let value = word_at(line, position.character as usize);
    if value.is_empty() {
        return None;
    }
    if let Some(include) = include_value_at(text, position) {
        let (include_file, document_index) = include_file_and_index(&include)?;
        let source_path = docs::path_of(uri)?;
        let pack_root = source_path
            .ancestors()
            .find(|directory| has_manifest(directory))?;
        if !pack_root.starts_with(root) {
            return None;
        }
        let path = pack_root.join(include_file);
        if include_target_within_pack(&path, pack_root, sources) {
            let range = if let Some(document_index) = document_index {
                let contents = sources.text(&path)?;
                let source = SourceMap::parse(&contents).ok()?;
                if source.document_count() <= 1 || !source.document_is_mapping(document_index) {
                    return None;
                }
                let located = source.location(document_index, "");
                Range {
                    start: Position {
                        line: located.start.line,
                        character: located.start.character,
                    },
                    end: Position {
                        line: located.end.line,
                        character: located.end.character,
                    },
                }
            } else {
                Range::default()
            };
            return Some(Location {
                uri: Url::from_file_path(path).ok()?,
                range,
            });
        }
    }
    if let Some((pack, id, kind)) = generated_identity(&value) {
        for pack_root in pack_roots_by_generated_ref(root, &pack, sources) {
            for path in sources.yaml_paths(&pack_root) {
                if !owns_document(&pack_root, &path) {
                    continue;
                }
                if is_manifest_path(&path) {
                    continue;
                }
                let Some(contents) = sources.text(&path) else {
                    continue;
                };
                if let Some(declaration) = generated_declarations(&path, &contents)
                    .into_iter()
                    .find(|declaration| {
                        declaration.kind == kind && sanitize_runtime_token(&declaration.id) == id
                    })
                {
                    return Some(Location {
                        uri: Url::from_file_path(path).ok()?,
                        range: declaration.range,
                    });
                }
            }
        }
    }
    let bare_reference = bare_class_id_at(line, position)
        .map(|id| (id, GeneratedKind::Class))
        .or_else(|| {
            bare_sound_references(text)
                .into_iter()
                .find(|reference| {
                    position.line == reference.range.start.line
                        && position.character >= reference.range.start.character
                        && position.character < reference.range.end.character
                })
                .map(|reference| (reference.id, GeneratedKind::Asset))
        });
    if let Some((id, kind)) = bare_reference {
        let source_path = docs::path_of(uri)?;
        let pack_root = source_path
            .ancestors()
            .find(|directory| has_manifest(directory))?;
        if pack_root.starts_with(root) {
            for path in sources.yaml_paths(pack_root) {
                if !owns_document(pack_root, &path) || is_manifest_path(&path) {
                    continue;
                }
                let Some(contents) = sources.text(&path) else {
                    continue;
                };
                if let Some(declaration) = generated_declarations(&path, &contents)
                    .into_iter()
                    .find(|declaration| declaration.kind == kind && declaration.id == id)
                {
                    return Some(Location {
                        uri: Url::from_file_path(path).ok()?,
                        range: declaration.range,
                    });
                }
            }
        }
    }
    if docs::path_of(uri).is_some_and(|path| is_loadable_manifest_path(&path)) {
        if let Some(dependency) = pack_dependency_at(text, position) {
            let pack_root = pack_root_by_ref(root, &dependency, sources)?;
            return Some(Location {
                uri: Url::from_file_path(manifest_path(&pack_root)?).ok()?,
                range: Range::default(),
            });
        }
    }
    None
}

fn bare_class_id_at(line: &str, position: Position) -> Option<String> {
    let (key, _) = line.split_once(':')?;
    if !matches!(
        key.trim().trim_start_matches('-').trim(),
        "class" | "parent" | "schematic" | "category"
    ) {
        return None;
    }
    let value_start = line[..=key.len()].encode_utf16().count() as u32;
    if position.character <= value_start {
        return None;
    }
    let id = word_at(line, position.character as usize);
    (!id.is_empty() && !id.contains(['/', '.'])).then_some(id)
}

fn pack_dependency_at(text: &str, position: Position) -> Option<String> {
    let manifest: serde_yaml::Value = serde_yaml::from_str(text).ok()?;
    let dependencies = manifest
        .get("dependencies")
        .and_then(serde_yaml::Value::as_sequence)?;
    let source = SourceMap::parse(text).ok()?;
    for (index, dependency) in dependencies.iter().enumerate() {
        let range = source.location(0, &format!("/dependencies/{index}"));
        if position.line == range.start.line
            && position.character >= range.start.character
            && position.character < range.end.character
        {
            return dependency.as_str().map(str::to_owned);
        }
    }
    None
}

fn include_target_within_pack(path: &Path, pack_root: &Path, sources: &Sources) -> bool {
    let Ok(canonical_pack_root) = dunce::canonicalize(pack_root) else {
        return false;
    };
    if path.symlink_metadata().is_ok() {
        return path.is_file()
            && dunce::canonicalize(path)
                .is_ok_and(|target| target.starts_with(&canonical_pack_root));
    }
    sources.has_buffer(path)
        && path
            .parent()
            .and_then(|parent| dunce::canonicalize(parent).ok())
            .is_some_and(|parent| parent.starts_with(&canonical_pack_root))
}

fn include_file_and_index(include: &str) -> Option<(&str, Option<usize>)> {
    if let Some(prefix) = include.strip_suffix(']') {
        if let Some((file, digits)) = prefix.rsplit_once('[') {
            if !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit()) {
                return Some((file, Some(digits.parse().ok()?)));
            }
        }
    }
    Some((include, None))
}

fn include_value_at(text: &str, position: Position) -> Option<String> {
    let line = text.lines().nth(position.line as usize)?;
    if !line.contains("include") && !line.trim_start().starts_with('-') {
        return None;
    }
    let source = SourceMap::parse(text).ok()?;
    for (document_index, deserializer) in serde_yaml::Deserializer::from_str(text).enumerate() {
        if document_index >= source.document_count() {
            break;
        }
        let Ok(document) = serde_yaml::Value::deserialize(deserializer) else {
            continue;
        };
        let Some(include) = document.get("include") else {
            continue;
        };
        if let Some(value) = include.as_str() {
            if source_range_contains(source.location(document_index, "/include"), position) {
                return Some(value.to_owned());
            }
        } else if let Some(items) = include.as_sequence() {
            for (item_index, item) in items.iter().enumerate() {
                let range = source.location(document_index, &format!("/include/{item_index}"));
                if source_range_contains(range, position) {
                    return item.as_str().map(str::to_owned);
                }
            }
        }
    }
    None
}

fn source_range_contains(range: kdf_validation::Range, position: Position) -> bool {
    position.line == range.start.line
        && position.character >= range.start.character
        && position.character < range.end.character
}

pub fn references(
    root: &Path,
    uri: &Url,
    text: &str,
    position: Position,
    sources: &Sources,
    include_declaration: bool,
) -> Vec<Location> {
    if bare_sound_references(text).iter().any(|reference| {
        position.line == reference.range.start.line
            && position.character >= reference.range.start.character
            && position.character < reference.range.end.character
    }) {
        if let Some(target) = definition(root, uri, text, position, sources) {
            if let Some(contents) = target
                .uri
                .to_file_path()
                .ok()
                .and_then(|path| sources.text(&path))
            {
                let mut position = target.range.end;
                position.character = position.character.saturating_sub(1);
                return references(
                    root,
                    &target.uri,
                    &contents,
                    position,
                    sources,
                    include_declaration,
                );
            }
        }
        return Vec::new();
    }
    let line = text.lines().nth(position.line as usize).unwrap_or("");
    let mut word = word_at(line, position.character as usize);
    let mut declaration = None;
    if let Some((id, response)) = crate::rename::prepare_id_rename(text, position) {
        if let Some(path) = docs::path_of(uri) {
            if let Some(pack_root) = path.ancestors().find(|directory| has_manifest(directory)) {
                if let Some(pack_ref) =
                    manifest_path(pack_root).and_then(|path| pack_ref(&path, sources))
                {
                    if let Some(kind) = generated_declarations(&path, text)
                        .into_iter()
                        .find(|candidate| {
                            candidate.id == id && candidate.range.start.line == position.line
                        })
                        .map(|candidate| candidate.kind)
                    {
                        let package = if kind == GeneratedKind::Asset {
                            "GenAssets"
                        } else {
                            "Gen"
                        };
                        let pack_name = sanitize_runtime_token(&pack_ref);
                        let id_name = sanitize_runtime_token(&id);
                        word =
                            format!("/KDataForge/{package}/{pack_name}.Gen_{pack_name}_{id_name}");
                        if let tower_lsp::lsp_types::PrepareRenameResponse::RangeWithPlaceholder {
                            range,
                            ..
                        } = response
                        {
                            declaration = Some(Location {
                                uri: uri.clone(),
                                range,
                            });
                        }
                    }
                }
            }
        }
    }
    if word.len() < 2 {
        return Vec::new();
    }
    let mut locations = Vec::new();
    if include_declaration {
        if let Some(location) = declaration {
            locations.push(location);
        }
    }
    for path in sources.yaml_paths(root) {
        let Ok(uri) = Url::from_file_path(&path) else {
            continue;
        };
        let Some(contents) = sources.text(&path) else {
            continue;
        };
        if let Some((pack, id, GeneratedKind::Asset)) = generated_identity(&word) {
            if let Some(pack_root) = path.ancestors().find(|directory| has_manifest(directory)) {
                if manifest_path(pack_root)
                    .and_then(|path| pack_ref(&path, sources))
                    .is_some_and(|reference| sanitize_runtime_token(&reference) == pack)
                {
                    for reference in bare_sound_references(&contents) {
                        if sanitize_runtime_token(&reference.id) == id {
                            locations.push(Location {
                                uri: uri.clone(),
                                range: reference.range,
                            });
                        }
                    }
                }
            }
        }
        for (line_number, line) in contents.lines().enumerate() {
            let content = yaml_content_prefix(line);
            for (byte, _) in content.match_indices(&word) {
                let before = content[..byte].chars().next_back();
                let after = content[byte + word.len()..].chars().next();
                if before.is_some_and(is_word_char) || after.is_some_and(is_word_char) {
                    continue;
                }
                let start = line[..byte].encode_utf16().count() as u32;
                locations.push(Location {
                    uri: uri.clone(),
                    range: Range {
                        start: Position {
                            line: line_number as u32,
                            character: start,
                        },
                        end: Position {
                            line: line_number as u32,
                            character: start + word.encode_utf16().count() as u32,
                        },
                    },
                });
            }
        }
    }
    locations
}

fn generated_identity(value: &str) -> Option<(String, String, GeneratedKind)> {
    let (rest, kind) = if let Some(rest) = value.strip_prefix("/KDataForge/GenAssets/") {
        (rest, GeneratedKind::Asset)
    } else {
        (
            value.strip_prefix("/KDataForge/Gen/")?,
            GeneratedKind::Class,
        )
    };
    let (pack, class) = rest.split_once(".Gen_")?;
    let id = class.strip_prefix(&format!("{pack}_"))?;
    if id.is_empty() {
        None
    } else {
        Some((pack.to_owned(), id.to_owned(), kind))
    }
}

fn pack_root_by_ref(root: &Path, reference: &str, sources: &Sources) -> Option<PathBuf> {
    sources
        .yaml_paths(root)
        .into_iter()
        .filter(|path| is_loadable_manifest_path(path))
        .find(|path| pack_ref(path, sources).as_deref() == Some(reference))
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

fn pack_roots_by_generated_ref(root: &Path, reference: &str, sources: &Sources) -> Vec<PathBuf> {
    sources
        .yaml_paths(root)
        .into_iter()
        .filter(|path| is_loadable_manifest_path(path))
        .filter_map(|path| {
            let actual = pack_ref(&path, sources)?;
            (sanitize_runtime_token(&actual) == reference)
                .then(|| path.parent().map(Path::to_path_buf))
                .flatten()
        })
        .collect()
}

fn pack_ref(path: &Path, sources: &Sources) -> Option<String> {
    let content = sources.text(path)?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&content).ok()?;
    yaml.get("ref")?.as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::Buffers;
    use kdf_validation::FileBuffer;
    use std::fs;

    #[test]
    fn generated_paths_resolve_to_id_and_reference_sites() {
        let root = tempfile::tempdir().unwrap();
        let pack = root.path().join("Pack");
        fs::create_dir(&pack).unwrap();
        fs::write(
            pack.join("pack.yml"),
            "ref: Pack\nname: Pack\nversion: 1.0.0\n",
        )
        .unwrap();
        let item = pack.join("item.item.yml");
        let item_source = "type: item\nitems:\n  - id: Thing\n";
        fs::write(&item, item_source).unwrap();
        let recipe = pack.join("recipe.recipe.yml");
        let recipe_source = "type: recipe\nremove: [/KDataForge/Gen/Pack.Gen_Pack_Thing]\n";
        fs::write(&recipe, recipe_source).unwrap();
        let recipe_uri = Url::from_file_path(&recipe).unwrap();
        let definition = definition(
            root.path(),
            &recipe_uri,
            recipe_source,
            Position {
                line: 1,
                character: 27,
            },
            &Sources::default(),
        )
        .unwrap();
        assert_eq!(definition.uri, Url::from_file_path(&item).unwrap());
        let item_uri = Url::from_file_path(&item).unwrap();
        let refs = references(
            root.path(),
            &item_uri,
            item_source,
            Position {
                line: 2,
                character: 9,
            },
            &Sources::default(),
            true,
        );
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn bare_generated_class_id_resolves_inside_current_pack() {
        let root = tempfile::tempdir().unwrap();
        let pack = root.path().join("KDFSample");
        let nested = pack.join("AChild");
        fs::create_dir_all(&nested).unwrap();
        fs::write(pack.join("pack.yml"), "ref: KDFSample\nversion: 1.0.0\n").unwrap();
        fs::write(nested.join("pack.yml"), "ref: Child\nversion: 1.0.0\n").unwrap();
        fs::write(
            nested.join("tree.research.yml"),
            "type: research\nresearch:\n  - id: KDFSampleTree\n",
        )
        .unwrap();
        let source_path = pack.join("kdf-sample.research.yml");
        let source = "type: research\nresearch:\n  - id: KDFSampleTree\n  - class: KDFSampleTree\n    register: false\n";
        fs::write(&source_path, source).unwrap();
        let uri = Url::from_file_path(&source_path).unwrap();
        let target = definition(
            root.path(),
            &uri,
            source,
            Position {
                line: 3,
                character: 15,
            },
            &Sources::default(),
        )
        .expect("bare generated class definition");
        assert_eq!(target.uri, uri);
        assert_eq!(target.range.start.line, 2);
        assert!(definition(
            root.path(),
            &Url::from_file_path(source_path).unwrap(),
            source,
            Position {
                line: 3,
                character: 5,
            },
            &Sources::default(),
        )
        .is_none());
    }

    #[test]
    fn bare_parent_and_schematic_ids_resolve_generated_classes() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("pack.yml"),
            "ref: KDFSample\nversion: 1.0.0\n",
        )
        .unwrap();
        let item = root.path().join("super-ingot.item.yml");
        fs::write(&item, "type: item\nitems:\n  - id: SuperIngot\n").unwrap();
        let schematic = root.path().join("super-ingot.schematic.yml");
        fs::write(
            &schematic,
            "type: schematic\nschematics:\n  - id: Schematic_SuperIngot\n",
        )
        .unwrap();
        let child = root.path().join("pure-super-ingot.item.yml");
        let child_source = "type: item\nitems:\n  - id: PureSuperIngot\n    parent: SuperIngot\n";
        let parent_target = definition(
            root.path(),
            &Url::from_file_path(child).unwrap(),
            child_source,
            Position {
                line: 3,
                character: 17,
            },
            &Sources::default(),
        )
        .expect("bare parent definition");
        assert_eq!(parent_target.uri, Url::from_file_path(item).unwrap());
        assert_eq!(parent_target.range.start.line, 2);

        let research = root.path().join("kdf-sample.research.yml");
        let research_source = "type: research\nresearch:\n  - id: KDFSampleTree\n    addNodes:\n      - schematic: Schematic_SuperIngot\n";
        let schematic_target = definition(
            root.path(),
            &Url::from_file_path(research).unwrap(),
            research_source,
            Position {
                line: 4,
                character: 22,
            },
            &Sources::default(),
        )
        .expect("bare schematic definition");
        assert_eq!(
            schematic_target.uri,
            Url::from_file_path(schematic).unwrap()
        );
        assert_eq!(schematic_target.range.start.line, 2);
    }

    #[test]
    fn pack_definition_navigation_only_uses_dependencies_field() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("Main");
        let dependency = root.path().join("KAPI");
        fs::create_dir(&main).unwrap();
        fs::create_dir(&dependency).unwrap();
        let dependency_manifest = dependency.join("pack.yml");
        fs::write(&dependency_manifest, "ref: KAPI\nversion: 1.0.0\n").unwrap();
        let manifest = main.join("pack.yml");
        let source = "ref: Main\nversion: 1.0.0\nconditions:\n  hasMod:\n    - KAPI\ndependencies:\n  - KAPI\n";
        fs::write(&manifest, source).unwrap();
        let uri = Url::from_file_path(&manifest).unwrap();
        assert!(definition(
            root.path(),
            &uri,
            source,
            Position {
                line: 4,
                character: 7,
            },
            &Sources::default(),
        )
        .is_none());
        let target = definition(
            root.path(),
            &uri,
            source,
            Position {
                line: 6,
                character: 5,
            },
            &Sources::default(),
        )
        .expect("pack dependency definition");
        assert_eq!(
            target.uri,
            Url::from_file_path(dependency_manifest).unwrap()
        );
    }

    #[test]
    fn generated_asset_path_resolves_to_asset_declaration() {
        let root = tempfile::tempdir().unwrap();
        let pack = root.path().join("KDFSample");
        fs::create_dir(&pack).unwrap();
        fs::write(
            pack.join("pack.yml"),
            "ref: KDFSample\nname: Sample\nversion: 1.0.0\n",
        )
        .unwrap();
        let wrong_class = pack.join("a.item.yml");
        fs::write(&wrong_class, "type: item\nitems:\n  - id: SuperIngotIcon\n").unwrap();
        let asset = pack.join("super-ingot-icon.asset.yml");
        fs::write(
            &asset,
            "type: asset\nassets:\n  - id: SuperIngotIcon\n    file: mod-logo.png\n",
        )
        .unwrap();
        let item = pack.join("super-ingot.item.yml");
        let source = "type: item\nitems:\n  - id: SuperIngot\n    properties:\n      - path: mSmallIcon\n        value: /KDataForge/GenAssets/KDFSample.Gen_KDFSample_SuperIngotIcon\n";
        fs::write(&item, source).unwrap();
        let uri = Url::from_file_path(&item).unwrap();
        let target = definition(
            root.path(),
            &uri,
            source,
            Position {
                line: 5,
                character: 26,
            },
            &Sources::default(),
        )
        .expect("generated asset definition");
        assert_eq!(target.uri, Url::from_file_path(asset).unwrap());
        assert_eq!(target.range.start.line, 2);
    }

    #[test]
    fn sound_event_references_resolve_to_sound_declarations() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("pack.yml"),
            "ref: VoicePack\nname: Voice Pack\nversion: 1.0.0\n",
        )
        .unwrap();
        let sound = root.path().join("welcome.sound.yml");
        fs::write(
            &sound,
            "type: sound\nsounds:\n  - id: AdaWelcome\n    file: audio/welcome.wav\n",
        )
        .unwrap();
        let message = root.path().join("message.dataasset.yml");
        let text = "type: dataasset\nassets:\n  - id: Welcome\n    class: /Script/FactoryGame.FGMessage\n    properties:\n      - path: mAudioEvent\n        value: /KDataForge/GenAssets/VoicePack.Gen_VoicePack_AdaWelcome\n";
        fs::write(&message, text).unwrap();
        let target = definition(
            root.path(),
            &Url::from_file_path(&message).unwrap(),
            text,
            Position {
                line: 6,
                character: 25,
            },
            &Sources::default(),
        )
        .expect("sound definition");
        assert_eq!(target.uri, Url::from_file_path(&sound).unwrap());
        assert_eq!(target.range.start.line, 2);
    }

    #[test]
    fn bare_sound_reference_definition_and_references_support_quotes_and_flow() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("pack.yml"),
            "ref: VoicePack\nversion: 1.0.0\n",
        )
        .unwrap();
        let sound = root.path().join("welcome.sound.yml");
        let declaration =
            "type: sound\nsounds:\n  - id: ADA.Welcome-1\n    file: audio/welcome.wav\n";
        fs::write(&sound, declaration).unwrap();
        let message = root.path().join("message.dataasset.yml");
        for assignment in [
            "      - path: mAudioEvent\n        value: 'ADA.Welcome-1' # ADA.Welcome-1\n",
            "      - { path: mAudioEvent, value: ADA.Welcome-1 }\n",
        ] {
            let text =
                format!("type: dataasset\nassets:\n  - id: Welcome\n    properties:\n{assignment}");
            fs::write(&message, &text).unwrap();
            let reference = bare_sound_references(&text).pop().expect("audio reference");
            let uri = Url::from_file_path(&message).unwrap();
            let target = definition(
                root.path(),
                &uri,
                &text,
                reference.range.start,
                &Sources::default(),
            )
            .expect("sound definition");
            assert_eq!(target.uri, Url::from_file_path(&sound).unwrap());
            let locations = references(
                root.path(),
                &uri,
                &text,
                reference.range.start,
                &Sources::default(),
                true,
            );
            assert_eq!(locations.len(), 2);
            assert!(locations
                .iter()
                .any(|location| location.uri == uri && location.range == reference.range));
        }
    }

    #[test]
    fn unsaved_new_yaml_buffers_participate_in_definition_and_references() {
        let root = tempfile::tempdir().unwrap();
        let pack = root.path().join("FaxitReborn");
        fs::create_dir(&pack).unwrap();
        fs::write(
            pack.join("pack.yml"),
            "ref: FaxitReborn\nversion: 9999.9.9\n",
        )
        .unwrap();
        let asset = pack.join("huge.dataasset.yml");
        let asset_text = "type: dataasset\nassets:\n  - id: Upgrade_DiskBytes_Huge_1\n";
        let consumer = pack.join("consumer.cdo.yml");
        let consumer_text = "type: cdo\nvalue: /KDataForge/GenAssets/FaxitReborn.Gen_FaxitReborn_Upgrade_DiskBytes_Huge_1\n";
        let asset_uri = Url::from_file_path(&asset).unwrap();
        let consumer_uri = Url::from_file_path(&consumer).unwrap();
        let mut buffers = Buffers::from_files([
            FileBuffer {
                path: asset.clone(),
                text: asset_text.to_owned(),
            },
            FileBuffer {
                path: consumer,
                text: consumer_text.to_owned(),
            },
        ]);
        let outside = tempfile::tempdir().unwrap();
        let outside_path = outside.path().join("consumer.cdo.yml");
        buffers.insert(FileBuffer {
            path: outside_path,
            text: consumer_text.to_owned(),
        });
        let buffers = Sources::from_buffers(buffers);
        let target = definition(
            root.path(),
            &consumer_uri,
            consumer_text,
            Position {
                line: 1,
                character: 35,
            },
            &buffers,
        )
        .expect("unsaved generated asset definition");
        assert_eq!(target.uri, asset_uri);
        let found = references(
            root.path(),
            &asset_uri,
            asset_text,
            Position {
                line: 2,
                character: 11,
            },
            &buffers,
            false,
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].uri, consumer_uri);
    }

    #[cfg(windows)]
    #[test]
    fn mixed_case_manifest_resolves_generated_asset_and_pack_reference() {
        let root = tempfile::tempdir().unwrap();
        let pack = root.path().join("FaxitReborn");
        fs::create_dir(&pack).unwrap();
        let manifest = pack.join("Pack.yml");
        fs::write(&manifest, "ref: FaxitReborn\nversion: 9999.9.9\n").unwrap();
        let asset = pack.join("huge.dataasset.yml");
        fs::write(
            &asset,
            "type: dataasset\nassets:\n  - id: Upgrade_DiskBytes_Huge_1\n",
        )
        .unwrap();
        let consumer = pack.join("consumer.cdo.yml");
        let source = "type: cdo\nvalue: /KDataForge/GenAssets/FaxitReborn.Gen_FaxitReborn_Upgrade_DiskBytes_Huge_1\n";
        fs::write(&consumer, source).unwrap();
        let asset_target = definition(
            root.path(),
            &Url::from_file_path(&consumer).unwrap(),
            source,
            Position {
                line: 1,
                character: 35,
            },
            &Sources::default(),
        )
        .expect("generated asset definition with Pack.yml");
        assert_eq!(asset_target.uri, Url::from_file_path(asset).unwrap());

        let deps_source = "dependencies: [FaxitReborn]\n";
        let manifest_target = definition(
            root.path(),
            &Url::from_file_path(&manifest).unwrap(),
            deps_source,
            Position {
                line: 0,
                character: 16,
            },
            &Sources::default(),
        )
        .expect("pack reference definition with Pack.yml");
        assert_eq!(manifest_target.uri, Url::from_file_path(manifest).unwrap());
    }

    #[test]
    fn generated_asset_declaration_finds_only_asset_references() {
        let root = tempfile::tempdir().unwrap();
        let pack = root.path().join("KDFSample");
        fs::create_dir(&pack).unwrap();
        fs::write(
            pack.join("pack.yml"),
            "ref: KDFSample\nname: Sample\nversion: 1.0.0\n",
        )
        .unwrap();
        let asset = pack.join("super-ingot-icon.asset.yml");
        let asset_text = "type: asset\nassets:\n  - id: SuperIngotIcon\n    file: mod-logo.png\n";
        fs::write(&asset, asset_text).unwrap();
        let item = pack.join("super-ingot.item.yml");
        fs::write(
            &item,
            "type: item\nitems:\n  - id: SuperIngot\n    properties:\n      - path: mSmallIcon\n        value: /KDataForge/GenAssets/KDFSample.Gen_KDFSample_SuperIngotIcon\n",
        )
        .unwrap();
        let class_reference = pack.join("class-reference.cdo.yml");
        fs::write(
            &class_reference,
            "type: cdo\npatches:\n  - target: /KDataForge/Gen/KDFSample.Gen_KDFSample_SuperIngotIcon\n    properties: []\n",
        )
        .unwrap();
        let asset_uri = Url::from_file_path(&asset).unwrap();
        let found = references(
            root.path(),
            &asset_uri,
            asset_text,
            Position {
                line: 2,
                character: 11,
            },
            &Sources::default(),
            false,
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].uri, Url::from_file_path(item).unwrap());
    }

    #[test]
    fn generated_declarations_in_mixed_multi_document_file_resolve() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("pack.yml"),
            "ref: KDFSample\nversion: 1.0.0\n",
        )
        .unwrap();
        let mixed = root.path().join("gilded-nugget.yml");
        let mixed_text = "type: item\nitems:\n  - id: GildedNugget\n---\ntype: sinkpoints\nentries:\n  - item: /KDataForge/Gen/KDFSample.Gen_KDFSample_GildedNugget\n    points: 15\n";
        fs::write(&mixed, mixed_text).unwrap();
        let target = definition(
            root.path(),
            &Url::from_file_path(&mixed).unwrap(),
            mixed_text,
            Position {
                line: 6,
                character: 34,
            },
            &Sources::default(),
        )
        .expect("multi-document generated item definition");
        assert_eq!(target.uri, Url::from_file_path(&mixed).unwrap());
        assert_eq!(target.range.start.line, 2);

        let support = root.path().join("showcase-delivery-task-support.yml");
        let support_text = "type: dataasset\nassets:\n  - id: AdaMessage_TaskComplete\n---\ntype: curve\ncurveType: float\ncurves:\n  - id: DeliveryTaskAmountMultiplier\n    keys:\n      - time: 0\n        value: 1\n";
        fs::write(&support, support_text).unwrap();
        let consumer = root.path().join("consumer.dataasset.yml");
        fs::write(
            &consumer,
            "type: dataasset\nvalue: /KDataForge/GenAssets/KDFSample.Gen_KDFSample_DeliveryTaskAmountMultiplier\n",
        )
        .unwrap();
        let found = references(
            root.path(),
            &Url::from_file_path(&support).unwrap(),
            support_text,
            Position {
                line: 7,
                character: 11,
            },
            &Sources::default(),
            false,
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].uri, Url::from_file_path(consumer).unwrap());
    }

    #[test]
    fn generated_asset_references_ignore_inline_comments() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("pack.yml"),
            "ref: FaxitReborn\nversion: 9999.9.9\n",
        )
        .unwrap();
        let curve = root.path().join("faxit-curves.curve.yml");
        let curve_text = "type: curve\ncurves:\n  - id: RecyclingCostMultiplier\n";
        fs::write(&curve, curve_text).unwrap();
        let consumer = root.path().join("consumer.dataasset.yml");
        let generated = "/KDataForge/GenAssets/FaxitReborn.Gen_FaxitReborn_RecyclingCostMultiplier";
        fs::write(
            &consumer,
            format!("value: {generated} # example: {generated}\n"),
        )
        .unwrap();
        let found = references(
            root.path(),
            &Url::from_file_path(curve).unwrap(),
            curve_text,
            Position {
                line: 2,
                character: 11,
            },
            &Sources::default(),
            false,
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].uri, Url::from_file_path(consumer).unwrap());
    }

    #[test]
    fn generated_asset_definition_ignores_inline_comment_cursor() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("pack.yml"),
            "ref: FaxitReborn\nversion: 9999.9.9\n",
        )
        .unwrap();
        let curve = root.path().join("faxit-curves.curve.yml");
        fs::write(
            &curve,
            "type: curve\ncurves:\n  - id: RecyclingCostMultiplier\n",
        )
        .unwrap();
        let consumer = root.path().join("consumer.dataasset.yml");
        let generated = "/KDataForge/GenAssets/FaxitReborn.Gen_FaxitReborn_RecyclingCostMultiplier";
        let source = format!("value: {generated} # {generated}\n");
        let uri = Url::from_file_path(consumer).unwrap();
        let actual = source.find(generated).unwrap() as u32 + 10;
        let comment = source.rfind(generated).unwrap() as u32 + 10;
        assert!(definition(
            root.path(),
            &uri,
            &source,
            Position {
                line: 0,
                character: actual,
            },
            &Sources::default(),
        )
        .is_some());
        assert!(definition(
            root.path(),
            &uri,
            &source,
            Position {
                line: 0,
                character: comment,
            },
            &Sources::default(),
        )
        .is_none());
    }

    #[test]
    fn sanitized_generated_id_resolves_definition_and_references() {
        let root = tempfile::tempdir().unwrap();
        let pack = root.path().join("KDFSample");
        fs::create_dir(&pack).unwrap();
        fs::write(
            pack.join("pack.yml"),
            "ref: KDF-Sample\nname: Sample\nversion: 1.0.0\n",
        )
        .unwrap();
        let item = pack.join("super-ingot.item.yml");
        let item_text = "type: item\nitems:\n  - id: Super-Ingot\n    parent: /Script/FactoryGame.FGItemDescriptor\n";
        fs::write(&item, item_text).unwrap();
        let recipe = pack.join("super-ingot.recipe.yml");
        let recipe_text =
            "type: recipe\nremove: [/KDataForge/Gen/KDF_Sample.Gen_KDF_Sample_Super_Ingot]\n";
        fs::write(&recipe, recipe_text).unwrap();
        let recipe_uri = Url::from_file_path(&recipe).unwrap();
        let definition = definition(
            root.path(),
            &recipe_uri,
            recipe_text,
            Position {
                line: 1,
                character: 32,
            },
            &Sources::default(),
        )
        .expect("sanitized class definition");
        assert_eq!(definition.uri, Url::from_file_path(&item).unwrap());
        assert_eq!(definition.range.start.line, 2);
        let item_uri = Url::from_file_path(&item).unwrap();
        let references = references(
            root.path(),
            &item_uri,
            item_text,
            Position {
                line: 2,
                character: 11,
            },
            &Sources::default(),
            false,
        );
        assert_eq!(references.len(), 1, "{references:?}");
        assert_eq!(references[0].uri, recipe_uri);
    }

    #[test]
    fn generated_definition_ignores_ids_owned_by_nested_pack() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("Parent");
        let child = parent.join("AChild");
        fs::create_dir_all(&child).unwrap();
        fs::write(parent.join("pack.yml"), "ref: Parent\nversion: 1.0.0\n").unwrap();
        fs::write(child.join("pack.yml"), "ref: Child\nversion: 1.0.0\n").unwrap();
        fs::write(
            child.join("item.item.yml"),
            "type: item\nitems:\n  - id: Thing\n",
        )
        .unwrap();
        let owned = parent.join("z-parent.item.yml");
        fs::write(&owned, "type: item\nitems:\n  - id: Thing\n").unwrap();
        let source_path = parent.join("source.cdo.yml");
        let source = "type: cdo\npatches:\n  - target: /KDataForge/Gen/Parent.Gen_Parent_Thing\n";
        fs::write(&source_path, source).unwrap();
        let target = definition(
            root.path(),
            &Url::from_file_path(source_path).unwrap(),
            source,
            Position {
                line: 2,
                character: 30,
            },
            &Sources::default(),
        )
        .expect("parent pack class definition");
        assert_eq!(target.uri, Url::from_file_path(owned).unwrap());
    }

    #[test]
    fn include_list_definition_uses_pack_relative_path() {
        let root = tempfile::tempdir().unwrap();
        let pack = root.path().join("KDFSample");
        fs::create_dir_all(pack.join("Recipes")).unwrap();
        fs::write(
            pack.join("pack.yml"),
            "ref: KDFSample\nname: Sample\nversion: 1.0.0\n",
        )
        .unwrap();
        let included = pack.join("super-ingot.item.yml");
        fs::write(&included, "type: item\nitems:\n  - id: SuperIngot\n").unwrap();
        let recipe = pack.join("Recipes/super-ingot.recipe.yml");
        let source = "type: recipe\ninclude:\n  - super-ingot.item.yml\nrecipes:\n  - id: Recipe_SuperIngot\n";
        fs::write(&recipe, source).unwrap();
        let uri = Url::from_file_path(&recipe).unwrap();
        let target = definition(
            root.path(),
            &uri,
            source,
            Position {
                line: 2,
                character: 9,
            },
            &Sources::default(),
        )
        .expect("include definition");
        assert_eq!(target.uri, Url::from_file_path(&included).unwrap());
    }

    #[test]
    fn inline_include_list_navigates_to_item_under_cursor() {
        let root = tempfile::tempdir().unwrap();
        let pack = root.path().join("KDFSample");
        fs::create_dir(&pack).unwrap();
        fs::write(
            pack.join("pack.yml"),
            "ref: KDFSample\nname: Sample\nversion: 1.0.0\n",
        )
        .unwrap();
        let first = pack.join("super-ingot.item.yml");
        let second = pack.join("alt.item.yml");
        fs::write(&first, "type: item\nitems:\n  - id: SuperIngot\n").unwrap();
        fs::write(&second, "type: item\nitems:\n  - id: Alt\n").unwrap();
        let recipe = pack.join("super-ingot.recipe.yml");
        let source = "type: recipe\ninclude: [super-ingot.item.yml, alt.item.yml]\nrecipes:\n  - id: Recipe_SuperIngot\n";
        fs::write(&recipe, source).unwrap();
        let uri = Url::from_file_path(&recipe).unwrap();
        let include_line = source.lines().nth(1).unwrap();
        for (name, path) in [("super-ingot.item.yml", first), ("alt.item.yml", second)] {
            let target = definition(
                root.path(),
                &uri,
                source,
                Position {
                    line: 1,
                    character: (include_line.find(name).unwrap() + 2) as u32,
                },
                &Sources::default(),
            )
            .expect("inline include definition");
            assert_eq!(target.uri, Url::from_file_path(path).unwrap());
        }
    }

    #[test]
    fn include_definition_requires_value_cursor_and_respects_document_index() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("pack.yml"),
            "ref: KDFSample\nversion: 1.0.0\n",
        )
        .unwrap();
        let first = root.path().join("new.item.yml");
        let second = root.path().join("other.item.yml");
        fs::write(&first, "type: item\nitems:\n  - id: First\n").unwrap();
        fs::write(&second, "type: item\nitems:\n  - id: Second\n").unwrap();
        let source_path = root.path().join("consumer.recipe.yml");
        let uri = Url::from_file_path(source_path).unwrap();

        let scalar = "type: recipe\ninclude: new.item.yml\nrecipes:\n  - id: R\n";
        assert!(definition(
            root.path(),
            &uri,
            scalar,
            Position {
                line: 1,
                character: 3,
            },
            &Sources::default(),
        )
        .is_none());
        assert_eq!(
            definition(
                root.path(),
                &uri,
                scalar,
                Position {
                    line: 1,
                    character: 12,
                },
                &Sources::default(),
            )
            .unwrap()
            .uri,
            Url::from_file_path(&first).unwrap()
        );

        let block = "type: recipe\ninclude:\n  - new.item.yml\nrecipes:\n  - id: R\n";
        assert!(definition(
            root.path(),
            &uri,
            block,
            Position {
                line: 2,
                character: 2,
            },
            &Sources::default(),
        )
        .is_none());

        let multi = "type: item\nitems:\n  - id: First\n---\ntype: recipe\ninclude: [new.item.yml, other.item.yml]\nrecipes:\n  - id: R\n";
        let line = multi.lines().nth(5).unwrap();
        let target = definition(
            root.path(),
            &uri,
            multi,
            Position {
                line: 5,
                character: (line.find("other.item.yml").unwrap() + 2) as u32,
            },
            &Sources::default(),
        )
        .expect("include from second document");
        assert_eq!(target.uri, Url::from_file_path(second).unwrap());
    }

    #[test]
    fn indexed_include_definition_opens_selected_document_in_file() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("pack.yml"),
            "ref: KDFSample\nversion: 1.0.0\n",
        )
        .unwrap();
        let included = root.path().join("shared.cdo.yml");
        fs::write(
            &included,
            "type: cdo\npatches: []\n---\ntype: cdo\npatches: []\n",
        )
        .unwrap();
        let source_path = root.path().join("consumer.recipe.yml");
        let source =
            "type: recipe\ninclude:\n  - shared.cdo.yml[1]\nrecipes:\n  - id: Recipe_SuperIngot\n";
        let target = definition(
            root.path(),
            &Url::from_file_path(&source_path).unwrap(),
            source,
            Position {
                line: 2,
                character: 11,
            },
            &Sources::default(),
        )
        .expect("indexed include definition");
        assert_eq!(target.uri, Url::from_file_path(&included).unwrap());
        assert_eq!(target.range.start.line, 3);
        let index_character = source.lines().nth(2).unwrap().find("[1]").unwrap() as u32 + 1;
        let on_index = definition(
            root.path(),
            &Url::from_file_path(&source_path).unwrap(),
            source,
            Position {
                line: 2,
                character: index_character,
            },
            &Sources::default(),
        )
        .expect("indexed include suffix definition");
        assert_eq!(on_index.range.start.line, 3);

        let invalid = source.replace("[1]", "[2]");
        assert!(definition(
            root.path(),
            &Url::from_file_path(&source_path).unwrap(),
            &invalid,
            Position {
                line: 2,
                character: 11,
            },
            &Sources::default(),
        )
        .is_none());

        fs::write(&included, "type: cdo\npatches: []\n").unwrap();
        let single_document = source.replace("[1]", "[0]");
        assert!(definition(
            root.path(),
            &Url::from_file_path(&source_path).unwrap(),
            &single_document,
            Position {
                line: 2,
                character: 11,
            },
            &Sources::default(),
        )
        .is_none());

        fs::write(&included, "type: cdo\npatches: []\n---\n").unwrap();
        assert!(definition(
            root.path(),
            &Url::from_file_path(&source_path).unwrap(),
            source,
            Position {
                line: 2,
                character: 11,
            },
            &Sources::default(),
        )
        .is_none());
    }

    #[test]
    fn include_definition_uses_unsaved_new_buffer_within_pack() {
        let root = tempfile::tempdir().unwrap();
        let pack = root.path().join("Pack");
        let outside = root.path().join("Outside");
        fs::create_dir(&pack).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(pack.join("pack.yml"), "ref: Pack\nversion: 1.0.0\n").unwrap();
        let included = pack.join("new.item.yml");
        let included_text =
            "type: item\nitems:\n  - id: First\n---\ntype: item\nitems:\n  - id: Second\n";
        let outside_path = outside.join("new.item.yml");
        let buffers = Sources::from_buffers(Buffers::from_files([
            FileBuffer {
                path: included.clone(),
                text: included_text.to_owned(),
            },
            FileBuffer {
                path: outside_path,
                text: included_text.to_owned(),
            },
        ]));
        let source_path = pack.join("consumer.recipe.yml");
        let source =
            "type: recipe\ninclude:\n  - new.item.yml\nrecipes:\n  - id: Recipe_SuperIngot\n";
        let target = definition(
            root.path(),
            &Url::from_file_path(&source_path).unwrap(),
            source,
            Position {
                line: 2,
                character: 9,
            },
            &buffers,
        )
        .expect("unsaved include definition");
        assert_eq!(target.uri, Url::from_file_path(&included).unwrap());

        let indexed = source.replace("new.item.yml", "new.item.yml[1]");
        let indexed_target = definition(
            root.path(),
            &Url::from_file_path(&source_path).unwrap(),
            &indexed,
            Position {
                line: 2,
                character: 9,
            },
            &buffers,
        )
        .expect("unsaved indexed include definition");
        assert_eq!(indexed_target.range.start.line, 4);

        let escaped = source.replace("new.item.yml", "../Outside/new.item.yml");
        assert!(definition(
            root.path(),
            &Url::from_file_path(source_path).unwrap(),
            &escaped,
            Position {
                line: 2,
                character: 17,
            },
            &buffers,
        )
        .is_none());
    }
}
