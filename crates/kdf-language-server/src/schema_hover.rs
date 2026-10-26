//! Generic YAML hover backed by the shared validation documentation catalog.
//! Cursor paths come from the text resolver, with sequence markers removed.
use crate::context::{Place, Resolved};
use crate::state::ClientCaps;
use kdf_validation::{byte_offset_at_utf16, field_docs, Position as KdfPosition};
use std::path::Path;
use tower_lsp::lsp_types::{Hover, HoverContents};

pub(crate) fn hover(
    path: &Path,
    text: &str,
    resolved: &Resolved,
    caps: ClientCaps,
) -> Option<Hover> {
    if resolved.in_comment {
        return None;
    }
    let kind = if matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("pack.yml" | "pack.yaml")
    ) {
        "pack"
    } else {
        resolved.doc.kind.as_deref()?
    };
    let mut keys = resolved
        .path()
        .into_iter()
        .filter(|key| key != "#")
        .collect::<Vec<_>>();
    let (doc, range) = match &resolved.place {
        Place::Key(slot) => {
            let line = text.lines().nth(slot.line as usize)?;
            let start = byte_offset_at_utf16(
                line,
                KdfPosition {
                    line: 0,
                    character: slot.start,
                },
            )?;
            let end = byte_offset_at_utf16(
                line,
                KdfPosition {
                    line: 0,
                    character: slot.end,
                },
            )?;
            let key = line.get(start..end)?.trim().trim_matches(['\'', '"']);
            if key.is_empty() {
                return None;
            }
            keys.push(key.to_owned());
            let segments = keys.iter().map(String::as_str).collect::<Vec<_>>();
            (
                field_docs::field(kind, &segments)?,
                crate::context::range_on(slot.line, slot.start, slot.end),
            )
        }
        Place::Value(slot) => {
            let scalar = &slot.scalar;
            let segments = keys.iter().map(String::as_str).collect::<Vec<_>>();
            let doc = field_docs::value_at(kind, &segments, scalar.value.trim())
                .or_else(|| field_docs::prefix(&scalar.value))?;
            (doc, scalar.range(0, scalar.units()))
        }
        Place::None => return None,
    };
    Some(Hover {
        contents: HoverContents::Markup(crate::resolve::markup(
            field_docs::to_markdown(doc),
            caps.hover_markdown,
        )),
        range: Some(range),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context;
    use tower_lsp::lsp_types::Position;

    fn at(file: &str, source: &str) -> Option<Hover> {
        let byte = source.find('|').expect("cursor marker");
        let position = Position {
            line: source[..byte].bytes().filter(|byte| *byte == b'\n').count() as u32,
            character: source[..byte]
                .rsplit('\n')
                .next()
                .unwrap()
                .encode_utf16()
                .count() as u32,
        };
        let text = source.replacen('|', "", 1);
        let path = Path::new(file);
        hover(
            path,
            &text,
            &context::resolve(path, &text, position),
            ClientCaps {
                hover_markdown: true,
                ..ClientCaps::default()
            },
        )
    }

    fn markdown(found: Hover) -> String {
        match found.contents {
            HoverContents::Markup(content) => content.value,
            _ => panic!("markdown"),
        }
    }

    #[test]
    fn nested_keys_and_enum_values_use_kind_specific_catalog_paths() {
        let key = at("x.cdo.yml", "type: cdo\npatches:\n  - target: /Script/Test.Recipe\n    properties:\n      - path: mX\n        o|p: multiply\n").unwrap();
        let expected = field_docs::to_markdown(
            field_docs::field("cdo", &["patches", "properties", "op"]).unwrap(),
        );
        assert_eq!(markdown(key), expected);
        let value = at("x.cdo.yml", "type: cdo\npatches:\n  - target: /Script/Test.Recipe\n    properties:\n      - path: mX\n        op: mul|tiply\n").unwrap();
        assert_eq!(
            markdown(value),
            field_docs::to_markdown(
                field_docs::value_at("cdo", &["patches", "properties", "op"], "multiply").unwrap()
            )
        );
        let config = at(
            "balance.config.yml",
            "fields:\n  - id: balance.mode\n    type: sel|ect\n",
        )
        .unwrap();
        assert_eq!(
            markdown(config),
            field_docs::to_markdown(
                field_docs::value_at("config", &["fields", "type"], "select").unwrap()
            )
        );
    }

    #[test]
    fn actor_action_hover_describes_delayed_removal_and_persistence() {
        let text = markdown(at("actions.cdo.yml", "type: cdo\npatches:\n  - target: /Script/Engine.Actor\n    applyToSpawnedActors: true\n    actorAc|tion: dismantle\n").expect("actor action hover"));
        for term in ["dismantle", "authority", "reload", "save"] {
            assert!(text.to_lowercase().contains(term), "missing {term}: {text}");
        }
        let value = markdown(at("actions.cdo.yml", "type: cdo\npatches:\n  - target: /Script/Engine.Actor\n    applyToSpawnedActors: true\n    actorAction: dis|mantle\n").expect("action enum hover"));
        assert!(value.contains("child"));
    }

    #[test]
    fn pack_sound_and_template_prefixes_have_real_catalog_docs() {
        let pack = at("pack.yml", "ref: Demo\nna|me: Demo\n").unwrap();
        assert_eq!(
            markdown(pack),
            field_docs::to_markdown(field_docs::field("pack", &["name"]).unwrap())
        );
        let sound = at(
            "voice.sound.yml",
            "type: sound\nsou|nds:\n  - id: Welcome\n    file: voice.wav\n",
        )
        .unwrap();
        assert_eq!(
            markdown(sound),
            field_docs::to_markdown(field_docs::field("sound", &["sounds"]).unwrap())
        );
        for token in [
            "cfg:tuning.amount",
            "session:Setting.Id",
            "ags:Setting.Id",
            "option:Setting.Id",
            "{item:/Game/Test.Test_C:GetItemName}",
            "{building:/Game/Test.Test_C:mName}",
        ] {
            let source = format!("type: cdo\npatches:\n  - target: /Script/Test.Recipe\n    properties:\n      - path: mName\n        value: '{token}|'\n");
            assert_eq!(
                markdown(at("x.cdo.yml", &source).unwrap()),
                field_docs::to_markdown(field_docs::prefix(token).unwrap())
            );
        }
    }

    #[test]
    fn comments_unknown_keys_and_unregistered_values_have_no_fabricated_docs() {
        assert!(at("x.cdo.yml", "type: cdo\n# op: mul|tiply\n").is_none());
        assert!(at("x.cdo.yml", "type: cdo\nmis|sing: something\n").is_none());
        assert!(at(
            "x.cdo.yml",
            "type: cdo\nconditions:\n  config: 'unknown|.field'\n"
        )
        .is_none());
    }
}
