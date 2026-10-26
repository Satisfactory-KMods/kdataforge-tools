//! Token helpers for legacy navigation. Rename plans live in kdf_validation::authoring.
use regex::Regex;
use tower_lsp::lsp_types::{Position, PrepareRenameResponse, Range};

pub fn prepare_id_rename(
    text: &str,
    position: Position,
) -> Option<(String, PrepareRenameResponse)> {
    let line = text.lines().nth(position.line as usize)?;
    let regex = Regex::new(r#"^\s*(?:-\s*)?id:\s*['\"]?(?P<id>[A-Za-z][A-Za-z0-9_.-]*)"#).ok()?;
    let capture = regex.captures(line)?.name("id")?;
    let start = line[..capture.start()].encode_utf16().count() as u32;
    let end = line[..capture.end()].encode_utf16().count() as u32;
    if position.character < start || position.character > end {
        return None;
    }
    let range = Range {
        start: Position {
            line: position.line,
            character: start,
        },
        end: Position {
            line: position.line,
            character: end,
        },
    };
    Some((
        capture.as_str().to_owned(),
        PrepareRenameResponse::RangeWithPlaceholder {
            range,
            placeholder: capture.as_str().to_owned(),
        },
    ))
}

pub(crate) fn sanitize_runtime_token(token: &str) -> String {
    token
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}
