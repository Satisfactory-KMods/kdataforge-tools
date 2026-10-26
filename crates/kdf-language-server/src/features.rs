use kdf_validation::{byte_offset_at_utf16, operation_spec, Position as KdfPosition, OPERATIONS};
use tower_lsp::lsp_types::{Position, Range, TextEdit};

pub(crate) fn operation_fix(text: &str, range: Range) -> Option<(String, TextEdit)> {
    if range.start.line != range.end.line {
        return None;
    }
    let line = text.lines().nth(range.start.line as usize)?;
    let requested_start = byte_offset_at_utf16(
        line,
        KdfPosition {
            line: 0,
            character: range.start.character,
        },
    )?;
    let requested_end = byte_offset_at_utf16(
        line,
        KdfPosition {
            line: 0,
            character: range.end.character,
        },
    )?;
    let content = yaml_content_prefix(line);
    let separator = content.find(':')?;
    let key = content[..separator].trim();
    let key = key.strip_prefix("- ").unwrap_or(key).trim();
    if key != "op" {
        return None;
    }
    let original = content[separator + 1..].trim();
    let start = content.len() - content[separator + 1..].trim_start().len();
    let end = start + original.len();
    // A diagnostic may predate the current token or truncate its final character.
    // Only derive an edit when its range still identifies this op scalar, never
    // another key, a comment, or an invalid UTF-16 position.
    if original.is_empty()
        || requested_start < separator + 1
        || requested_start >= end
        || requested_end <= start
        || requested_end <= requested_start
    {
        return None;
    }
    let value = serde_yaml::from_str::<String>(original).ok()?;
    if value.is_empty() || operation_spec(&value).is_some() {
        return None;
    }
    let mut ranked = OPERATIONS
        .iter()
        .map(|spec| (strsim::levenshtein(&value, spec.name), spec.name))
        .collect::<Vec<_>>();
    ranked.sort();
    let (distance, best) = *ranked.first()?;
    if distance > 2 || ranked.get(1).is_some_and(|next| next.0 == distance) {
        return None;
    }
    let replacement = if original.starts_with('"') && original.ends_with('"') {
        format!("\"{best}\"")
    } else if original.starts_with('\'') && original.ends_with('\'') {
        format!("'{best}'")
    } else {
        best.to_owned()
    };
    Some((
        best.to_owned(),
        TextEdit {
            range: Range::new(
                Position::new(
                    range.start.line,
                    line[..start].encode_utf16().count() as u32,
                ),
                Position::new(range.start.line, line[..end].encode_utf16().count() as u32),
            ),
            new_text: replacement,
        },
    ))
}

pub(crate) fn word_at(line: &str, character: usize) -> String {
    let byte = byte_offset_at_utf16(
        line,
        KdfPosition {
            line: 0,
            character: character as u32,
        },
    )
    .unwrap_or(line.len());
    let content = yaml_content_prefix(line);
    if content.len() < line.len() && byte >= content.len() {
        return String::new();
    }
    let start = line[..byte]
        .char_indices()
        .rev()
        .find(|(_, ch)| !is_word_char(*ch))
        .map(|(i, ch)| i + ch.len_utf8())
        .unwrap_or(0);
    let end = line[byte..]
        .char_indices()
        .find(|(_, ch)| !is_word_char(*ch))
        .map(|(i, _)| byte + i)
        .unwrap_or(line.len());
    line[start..end].to_owned()
}

pub(crate) fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '-' | '+' | '.' | '/' | '\\')
}

/// A scalar may start after the text `before` (the line up to the candidate quote): at the start of
/// the line or after a key colon, list dash, `?`, a flow bracket or a comma.
fn scalar_can_start(before: &str) -> bool {
    before
        .trim_end()
        .chars()
        .next_back()
        .is_none_or(|ch| matches!(ch, ':' | '-' | '?' | '[' | '{' | ','))
}

pub(crate) fn yaml_content_prefix(line: &str) -> &str {
    let mut characters = line.char_indices().peekable();
    let mut quote = None;
    let mut escaped = false;
    while let Some((index, character)) = characters.next() {
        match quote {
            Some('"') => {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == '"' {
                    quote = None;
                }
            }
            Some('\'') => {
                if character == '\'' {
                    if characters.peek().is_some_and(|(_, next)| *next == '\'') {
                        characters.next();
                    } else {
                        quote = None;
                    }
                }
            }
            _ => match character {
                // A quote opens a scalar only where one can start (`it's` stays plain text).
                '\'' | '"' if scalar_can_start(&line[..index]) => quote = Some(character),
                '#' if index == 0
                    || line[..index]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_whitespace) =>
                {
                    return &line[..index];
                }
                _ => {}
            },
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::Position;

    #[test]
    fn extracts_unicode_adjacent_class_token_by_utf16_position() {
        let line = "x 😀 /Script/Test.Example";
        assert_eq!(word_at(line, 15), "/Script/Test.Example");
    }

    #[test]
    fn hover_token_is_empty_inside_yaml_comments() {
        let generated = "/KDataForge/GenAssets/FaxitReborn.Gen_FaxitReborn_RecyclingCostMultiplier";
        let line = format!("value: {generated} # {generated}");
        let actual = line.find(generated).unwrap() + 10;
        let comment = line.rfind(generated).unwrap() + 10;
        assert_eq!(word_at(&line, actual), generated);
        assert_eq!(word_at(&line, comment), "");
        assert_eq!(word_at(&format!("# {generated}"), 12), "");
    }

    #[test]
    fn keeps_plus_in_blueprint_class_path_token() {
        let path = "/SatisfactoryPlus/Content/SchematicManager/Schematics/Milestones/T1/Schematic_SF+_T1_4.Schematic_SF+_T1_4_C";
        let line = format!("  - class: {path}");
        let plus = line.find('+').unwrap();
        assert_eq!(word_at(&line, plus), path);
        assert_eq!(word_at(&line, plus + 2), path);
    }

    #[test]
    fn yaml_comment_boundary_respects_quotes_and_plain_hashes() {
        assert_eq!(yaml_content_prefix("value: /Path # /Path"), "value: /Path ");
        assert_eq!(
            yaml_content_prefix("value: \"/Path # /Path\" # note"),
            "value: \"/Path # /Path\" "
        );
        assert_eq!(yaml_content_prefix("value: foo#bar"), "value: foo#bar");
        assert_eq!(
            yaml_content_prefix("value: 'it''s # a value' # note"),
            "value: 'it''s # a value' "
        );
    }

    #[test]
    fn an_apostrophe_inside_plain_text_does_not_hide_the_comment() {
        assert_eq!(yaml_content_prefix("value: it's # note"), "value: it's ");
        assert_eq!(
            yaml_content_prefix("value: don't # a 'quoted' remark"),
            "value: don't "
        );
        // Quotes where a scalar starts still hide a `#`.
        assert_eq!(yaml_content_prefix("- 'a # b' # c"), "- 'a # b' ");
        assert_eq!(
            yaml_content_prefix("k: [\"a # b\", 'c'] # d"),
            "k: [\"a # b\", 'c'] "
        );
        assert_eq!(yaml_content_prefix("\"a # b\" # c"), "\"a # b\" ");
    }

    #[test]
    fn suggests_only_unambiguous_operation_fixes() {
        let text = "op: mulitply\n";
        let range = Range {
            start: Position {
                line: 0,
                character: 4,
            },
            end: Position {
                line: 0,
                character: 12,
            },
        };
        assert_eq!(operation_fix(text, range).unwrap().0, "multiply");
        assert!(operation_fix(
            "op: unknown\n",
            Range {
                start: Position {
                    line: 0,
                    character: 4
                },
                end: Position {
                    line: 0,
                    character: 11
                }
            }
        )
        .is_none());
    }

    fn apply_fix(text: &str, range: Range) -> String {
        let (_, edit) = operation_fix(text, range).expect("unambiguous operation correction");
        let start = byte_offset_at_utf16(
            text,
            KdfPosition {
                line: edit.range.start.line,
                character: edit.range.start.character,
            },
        )
        .unwrap();
        let end = byte_offset_at_utf16(
            text,
            KdfPosition {
                line: edit.range.end.line,
                character: edit.range.end.character,
            },
        )
        .unwrap();
        format!("{}{}{}", &text[..start], edit.new_text, &text[end..])
    }

    #[test]
    fn operation_fix_replaces_complete_scalar_from_truncated_diagnostic() {
        let text = "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mValue\n        op: multiplly\n        value: 1\n";
        let range = Range::new(Position::new(5, 12), Position::new(5, 20));
        assert_eq!(
            apply_fix(text, range),
            text.replace("op: multiplly", "op: multiply")
        );
    }

    #[test]
    fn operation_fix_preserves_quotes_comments_and_utf16_coordinates() {
        for (text, range, expected) in [
            (
                "op: 'multiplly' # keep 😀 comment\n",
                Range::new(Position::new(0, 3), Position::new(0, 12)),
                "op: 'multiply' # keep 😀 comment\n",
            ),
            (
                "label: 😀\r\nop: \"multiplly\"  # keep\r\n",
                Range::new(Position::new(1, 4), Position::new(1, 12)),
                "label: 😀\r\nop: \"multiply\"  # keep\r\n",
            ),
            (
                "op: \"multipl😀y\" # comment\n",
                Range::new(Position::new(0, 4), Position::new(0, 14)),
                "op: \"multiply\" # comment\n",
            ),
            (
                "  - op: multiplly  # keep\n",
                Range::new(Position::new(0, 9), Position::new(0, 15)),
                "  - op: multiply  # keep\n",
            ),
        ] {
            assert_eq!(apply_fix(text, range), expected, "input: {text:?}");
        }
    }

    #[test]
    fn operation_fix_rejects_unrelated_or_invalid_scalar_context() {
        for (text, range) in [
            (
                "path: multiplly\n",
                Range::new(Position::new(0, 6), Position::new(0, 14)),
            ),
            (
                "op: set # multiplly\n",
                Range::new(Position::new(0, 10), Position::new(0, 18)),
            ),
            (
                "op: multiplly # multiply\n",
                Range::new(Position::new(0, 15), Position::new(0, 23)),
            ),
            (
                "op: multiply\n",
                Range::new(Position::new(0, 4), Position::new(0, 10)),
            ),
            (
                "op: 'multiplly\n",
                Range::new(Position::new(0, 4), Position::new(0, 13)),
            ),
            (
                "op: \"multipl😀y\"\n",
                Range::new(Position::new(0, 13), Position::new(0, 15)),
            ),
            (
                "op: multiplly\nvalue: 1\n",
                Range::new(Position::new(0, 4), Position::new(1, 1)),
            ),
            (
                "op: multiplly\n",
                Range::new(Position::new(0, 12), Position::new(0, 4)),
            ),
        ] {
            assert!(operation_fix(text, range).is_none(), "input: {text:?}");
        }
    }
}
