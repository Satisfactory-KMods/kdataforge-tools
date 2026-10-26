//! Navigation for pack configuration ids: typed references (`cfg:id`), template tokens
//! (`{cfg:id}`) and expression operands (`config: "id > 1"`) lead to the `fields[].id` declaration;
//! references find every use of a field across the workspace.

use crate::context::{scalar_at, split_lines, Place, Quote, Scalar};
use crate::docs::Sources;
use crate::semantic::Env;
use crate::workspace::Workspace;
use kdf_validation::config::{ConfigField, Resolution};
use kdf_validation::expression::{self, ExprKind, OperandKind, TokenKind};
use regex::Regex;
use std::path::Path;
use std::sync::OnceLock;
use tower_lsp::lsp_types::{Location, Position, Range, Url};

use crate::features::yaml_content_prefix;

/// One occurrence of a config id in a document.
#[derive(Clone, Debug)]
pub(crate) struct ConfigRef {
    pub(crate) pack: Option<String>,
    pub(crate) id: String,
    /// Range of the whole reference text (`cfg:Other:id`, the body of `{cfg:...}` or the operand).
    pub(crate) range: Range,
}

fn config_key_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^(\s*(?:-\s+)*)config:[ \t]*(.*)$").expect("regex"))
}

fn is_ref_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | ':' | '-')
}

fn split_pack(body: &str) -> (Option<String>, String) {
    match body.rsplit_once(':') {
        Some((pack, id)) => (Some(pack.to_owned()), id.to_owned()),
        None => (None, body.to_owned()),
    }
}

/// `true` when byte `index` of `content` is inside a quoted span of the line.
fn in_quotes(content: &str, index: usize) -> bool {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (position, ch) in content.char_indices() {
        if position >= index {
            break;
        }
        match quote {
            Some('"') => {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    quote = None;
                }
            }
            Some(_) => {
                if ch == '\'' {
                    quote = None;
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    quote = Some(ch);
                }
            }
        }
    }
    quote.is_some()
}

fn range_of(line: u32, content: &str, start: usize, end: usize) -> Range {
    crate::context::range_on(
        line,
        crate::context::col_at(content, start),
        crate::context::col_at(content, end),
    )
}

fn expression_refs(scalar: &Scalar, out: &mut Vec<ConfigRef>) {
    for token in expression::tokenize(&scalar.value) {
        if !matches!(token.kind, TokenKind::Ident | TokenKind::Reference) {
            continue;
        }
        let Ok(parsed) = expression::parse(&token.text) else {
            continue;
        };
        let ExprKind::Operand(operand) = parsed.kind else {
            continue;
        };
        if let OperandKind::Config { pack, id } = operand.kind {
            out.push(ConfigRef {
                pack,
                id,
                range: scalar.range(token.span.start, token.span.end),
            });
        }
    }
}

fn expression_scalar(line: &str, no: usize, start: usize, end: usize) -> Option<Scalar> {
    let text = line.get(start..end)?;
    let first = text.chars().next()?;
    if first == '[' || first == '{' {
        return None;
    }
    let quote = match first {
        '"' => Quote::Double,
        '\'' => Quote::Single,
        _ => Quote::Plain,
    };
    Some(scalar_at(line, no as u32, start, end, quote, start))
}

/// Every config id used in `text`: typed references, template tokens and condition expressions.
pub(crate) fn refs_in_text(text: &str) -> Vec<ConfigRef> {
    let lines = split_lines(text);
    let mut out = Vec::new();
    // Indentation of a `config:` key whose value is a block list of expressions.
    let mut list_indent: Option<usize> = None;
    for (no, line) in lines.iter().enumerate() {
        let end = yaml_content_prefix(line).len();
        let content = &line[..end];
        let trimmed = content.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let indent = content.len() - trimmed.len();
        if let Some(key_indent) = list_indent {
            let item = trimmed
                .strip_prefix('-')
                .filter(|rest| rest.is_empty() || rest.starts_with(' '));
            if let (Some(rest), true) = (item, indent >= key_indent) {
                let mut start = end - rest.len();
                while start < end && line.as_bytes()[start] == b' ' {
                    start += 1;
                }
                if let Some(scalar) = expression_scalar(line, no, start, end) {
                    expression_refs(&scalar, &mut out);
                }
                continue;
            }
            list_indent = None;
        }
        if let Some(captures) = config_key_regex().captures(content) {
            let prefix = captures.get(1).map_or(0, |m| m.end());
            let value = captures.get(2).map_or("", |m| m.as_str());
            if value.trim().is_empty() {
                list_indent = Some(prefix);
            } else {
                let start = end - value.len();
                if let Some(scalar) = expression_scalar(line, no, start, end) {
                    expression_refs(&scalar, &mut out);
                }
            }
            continue;
        }
        if !content.contains("cfg:") {
            continue;
        }
        let mut from = 0usize;
        while let Some(found) = content[from..].find("cfg:") {
            let index = from + found;
            from = index + 4;
            let before = content[..index].chars().next_back();
            let body_end = content[from..]
                .find(|ch: char| !is_ref_char(ch))
                .map_or(content.len(), |offset| from + offset);
            let body = &content[from..body_end];
            if body.is_empty() || body.ends_with(':') {
                continue;
            }
            let (pack, id) = split_pack(body);
            if before == Some('{') {
                out.push(ConfigRef {
                    pack,
                    id,
                    range: range_of(no as u32, line, from, body_end),
                });
                continue;
            }
            // A typed reference is a whole plain scalar in a value position.
            let previous = content[..index].trim_end().chars().next_back();
            let boundary = before.is_none_or(|ch| ch.is_whitespace() || ch == '[' || ch == ',');
            let value_position = previous.is_none_or(|ch| matches!(ch, ':' | '-' | '[' | ','));
            if boundary && value_position && !in_quotes(content, index) {
                out.push(ConfigRef {
                    pack,
                    id,
                    range: range_of(no as u32, line, index, body_end),
                });
            }
        }
    }
    out
}

fn contains(range: &Range, position: Position) -> bool {
    position.line == range.start.line
        && position.character >= range.start.character
        && position.character <= range.end.character
}

fn field_location(field: &ConfigField) -> Option<Location> {
    Some(Location {
        uri: Url::from_file_path(&field.file).ok()?,
        range: Range {
            start: Position {
                line: field.id_range.start.line,
                character: field.id_range.start.character,
            },
            end: Position {
                line: field.id_range.end.line,
                character: field.id_range.end.character,
            },
        },
    })
}

fn resolve_field<'a>(
    workspace: &'a Workspace,
    own: Option<&str>,
    reference: &ConfigRef,
) -> Option<&'a ConfigField> {
    match workspace
        .config
        .resolve(own, reference.pack.as_deref(), &reference.id)
    {
        Resolution::Found(field)
        | Resolution::CaseMismatch(field)
        | Resolution::PackDisabled(Some(field)) => Some(field),
        _ => None,
    }
}

/// Declaration of the config id under the cursor.
pub(crate) fn definition(env: &Env<'_>, position: Position) -> Option<Location> {
    let reference = refs_in_text(env.text)
        .into_iter()
        .find(|reference| contains(&reference.range, position))?;
    let field = resolve_field(env.workspace, env.own_pack(), &reference)?;
    field_location(field)
}

/// The field under the cursor: a reference to it or its own `fields[].id`.
fn field_at<'a>(env: &'a Env<'a>, position: Position) -> Option<&'a ConfigField> {
    if let Some(reference) = refs_in_text(env.text)
        .into_iter()
        .find(|reference| contains(&reference.range, position))
    {
        return resolve_field(env.workspace, env.own_pack(), &reference);
    }
    if !kdf_validation::config::is_config_file(env.path) {
        return None;
    }
    let resolved = crate::context::resolve(env.path, env.text, position);
    let Place::Value(slot) = &resolved.place else {
        return None;
    };
    if resolved.key() != Some("id") || resolved.field_entry().is_none() {
        return None;
    }
    env.workspace
        .config
        .field(env.own_pack()?, slot.scalar.value.trim())
}

/// Every use of the config id under the cursor in the workspace.
pub(crate) fn references(
    env: &Env<'_>,
    root: &Path,
    position: Position,
    sources: &Sources,
    include_declaration: bool,
) -> Option<Vec<Location>> {
    let field = field_at(env, position)?;
    let mut locations = Vec::new();
    if include_declaration {
        locations.extend(field_location(field));
    }
    for path in sources.yaml_paths(root) {
        let Some(text) = sources.text(&path) else {
            continue;
        };
        // Cheap pre-check: most documents mention no config id at all.
        if !text.contains("cfg:") && !text.contains("config:") {
            continue;
        }
        let Ok(uri) = Url::from_file_path(&path) else {
            continue;
        };
        let own = env.workspace.pack_of(&path);
        for reference in refs_in_text(&text) {
            let Some(found) = resolve_field(env.workspace, own, &reference) else {
                continue;
            };
            if found.pack_ref == field.pack_ref && found.id == field.id {
                locations.push(Location {
                    uri: uri.clone(),
                    range: reference.range,
                });
            }
        }
    }
    Some(locations)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<(String, Option<String>)> {
        refs_in_text(text)
            .into_iter()
            .map(|reference| (reference.id, reference.pack))
            .collect()
    }

    #[test]
    fn typed_template_and_expression_references_are_found() {
        let text = "type: cdo\npatches:\n  - target: /Script/A.B\n    properties:\n      - path: mX\n        value: cfg:balance.speed\n      - path: mY\n        value: \"{cfg:Other:shared.level} x {cfg:balance.stack}\"\n        conditions:\n          config: balance.mode == 'hard' && cfg:Other:shared.level > 1\n";
        let refs = found(text);
        assert!(refs.contains(&("balance.speed".into(), None)));
        assert!(refs.contains(&("shared.level".into(), Some("Other".into()))));
        assert!(refs.contains(&("balance.stack".into(), None)));
        assert!(refs.contains(&("balance.mode".into(), None)));
        // Template and expression use of the same field.
        assert_eq!(
            refs.iter()
                .filter(|reference| reference.0 == "shared.level")
                .count(),
            2
        );
    }

    #[test]
    fn quoted_and_embedded_text_is_not_a_typed_reference() {
        let text = "a:\n  - path: x\n    value: 'cfg:balance.speed stays literal'\n    note: see cfg:balance.speed here\n";
        assert!(found(text).is_empty());
    }

    #[test]
    fn expression_list_items_are_scanned() {
        let text = "conditions:\n  config:\n    - balance.enabled\n    - \"balance.speed > 1\"\npatches: []\n";
        let refs = found(text);
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn ranges_cover_the_reference_text() {
        let text = "value: cfg:a.b\n";
        let refs = refs_in_text(text);
        assert_eq!(refs[0].range.start.character, 7);
        assert_eq!(refs[0].range.end.character, 14);
    }
}
