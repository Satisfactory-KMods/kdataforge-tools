//! Quick fixes for the pack configuration diagnostics: quote an unquoted template or expression,
//! fix the letter case of an id, suggest the closest ids, packs, settings and template providers.

use crate::context::{byte_at, col_at, inline_value_span, split_lines};
use crate::semantic::Env;
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, Diagnostic, NumberOrString, Position, Range, TextEdit, Url,
    WorkspaceEdit,
};

fn regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("regex"))
}

fn action(
    uri: &Url,
    title: String,
    diagnostic: &Diagnostic,
    range: Range,
    new_text: String,
) -> CodeAction {
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), vec![TextEdit { range, new_text }]);
    CodeAction {
        title,
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..WorkspaceEdit::default()
        }),
        ..CodeAction::default()
    }
}

/// `text` as a YAML scalar in quotes: double quotes unless the text needs escapes.
pub(crate) fn quoted(text: &str) -> String {
    if text.contains(['"', '\\']) {
        format!("'{}'", text.replace('\'', "''"))
    } else {
        format!("\"{text}\"")
    }
}

/// Wraps the inline value of the diagnostic's line in quotes.
fn quote_wrap(uri: &Url, lines: &[&str], diagnostic: &Diagnostic) -> Option<CodeAction> {
    let line_no = diagnostic.range.start.line;
    let line = lines.get(line_no as usize)?;
    let (start, end) = inline_value_span(line)?;
    let value = &line[start..end];
    if value.starts_with(['"', '\'']) {
        return None;
    }
    let range = Range {
        start: Position {
            line: line_no,
            character: col_at(line, start),
        },
        end: Position {
            line: line_no,
            character: col_at(line, end),
        },
    };
    Some(action(
        uri,
        "Wrap the value in quotes".into(),
        diagnostic,
        range,
        quoted(value),
    ))
}

/// Candidates closest to `wanted` (case-insensitive edit distance), best first.
fn closest<'a>(wanted: &str, candidates: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let wanted_lower = wanted.to_lowercase();
    let limit = (wanted.chars().count() / 4).max(2);
    let mut ranked: Vec<(usize, &str)> = candidates
        .map(|candidate| {
            (
                strsim::levenshtein(&wanted_lower, &candidate.to_lowercase()),
                candidate,
            )
        })
        .filter(|(distance, candidate)| *distance <= limit && *candidate != wanted)
        .collect();
    ranked.sort();
    ranked.truncate(3);
    ranked.into_iter().map(|(_, candidate)| candidate).collect()
}

/// Byte offset of the first occurrence of `needle` in `haystack`, comparing characters one by one
/// and ignoring ASCII letter case only (config ids and packs are ASCII). A lower-cased copy of the
/// text would not do: lower-casing changes the byte length of some characters (the Kelvin sign
/// U+212A), so the offsets of the copy do not map back onto the original line.
fn find_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .char_indices()
        .map(|(index, _)| index)
        .find(|&index| {
            let mut rest = haystack[index..].chars();
            needle.chars().all(|wanted| {
                rest.next()
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(&wanted))
            })
        })
}

/// Replaces the first occurrence of `from` that starts inside `range` with `to`. The match may
/// extend past the range end (a pack-qualified id is reported as the whole scalar, `from` can be
/// a part of it).
fn replace_in_range(
    lines: &[&str],
    range: Range,
    from: &str,
    to: &str,
    ignore_case: bool,
) -> Option<(Range, String)> {
    let line = lines.get(range.start.line as usize)?;
    let start = byte_at(line, range.start.character);
    let end = if range.end.line == range.start.line {
        byte_at(line, range.end.character).max(start)
    } else {
        line.len()
    };
    let tail = line.get(start..)?;
    let offset = if ignore_case {
        find_ascii_ci(tail, from)?
    } else {
        tail.find(from)?
    };
    if start + offset > end {
        return None;
    }
    let from_start = start + offset;
    let from_end = from_start + from.len();
    Some((
        Range {
            start: Position {
                line: range.start.line,
                character: col_at(line, from_start),
            },
            end: Position {
                line: range.start.line,
                character: col_at(line, from_end),
            },
        },
        to.to_owned(),
    ))
}

pub(crate) fn code_actions(
    env: &Env<'_>,
    uri: &Url,
    diagnostics: &[Diagnostic],
) -> Vec<CodeAction> {
    static CASE: OnceLock<Regex> = OnceLock::new();
    static UNKNOWN_ID: OnceLock<Regex> = OnceLock::new();
    static UNKNOWN_PACK: OnceLock<Regex> = OnceLock::new();
    static PROVIDER: OnceLock<Regex> = OnceLock::new();
    static SETTING: OnceLock<Regex> = OnceLock::new();
    let lines = split_lines(env.text);
    let mut actions = Vec::new();
    for diagnostic in diagnostics {
        let Some(NumberOrString::String(code)) = &diagnostic.code else {
            continue;
        };
        let message = diagnostic.message.as_str();
        match code.as_str() {
            "template.unquoted-start" | "config.expression-quote" => {
                actions.extend(quote_wrap(uri, &lines, diagnostic));
            }
            "config.case-mismatch" => {
                let pattern = regex(
                    &CASE,
                    r#"config id "([^"]+)" differs only in letter case from the defined "([^"]+)""#,
                );
                if let Some(captures) = pattern.captures(message) {
                    let (wrong, right) = (&captures[1], &captures[2]);
                    if let Some((range, text)) =
                        replace_in_range(&lines, diagnostic.range, wrong, right, true)
                    {
                        actions.push(action(
                            uri,
                            format!("Change to \"{right}\""),
                            diagnostic,
                            range,
                            text,
                        ));
                    }
                }
            }
            "config.unknown-id" => {
                let pattern = regex(
                    &UNKNOWN_ID,
                    r#"no config field "([^"]+)" exists in pack (\S+)"#,
                );
                let Some(captures) = pattern.captures(message) else {
                    continue;
                };
                let label = captures[1].to_owned();
                let pack_name = captures[2].to_owned();
                let (qualifier, id) = match label.rsplit_once(':') {
                    Some((pack, id)) => (Some(pack.to_owned()), id.to_owned()),
                    None => (None, label.clone()),
                };
                let Some(pack) = env
                    .workspace
                    .config
                    .pack(qualifier.as_deref().unwrap_or(&pack_name))
                else {
                    continue;
                };
                for candidate in closest(&id, pack.fields.iter().map(|field| field.id.as_str())) {
                    let replacement = match &qualifier {
                        Some(pack) => format!("{pack}:{candidate}"),
                        None => candidate.to_owned(),
                    };
                    if let Some((range, text)) =
                        replace_in_range(&lines, diagnostic.range, &label, &replacement, false)
                    {
                        actions.push(action(
                            uri,
                            format!("Change to \"{replacement}\""),
                            diagnostic,
                            range,
                            text,
                        ));
                    }
                }
            }
            "config.unknown-pack" => {
                let pattern = regex(
                    &UNKNOWN_PACK,
                    r#"pack "([^"]*)" is not part of this project"#,
                );
                let Some(captures) = pattern.captures(message) else {
                    continue;
                };
                let wrong = captures[1].to_owned();
                let packs = env.workspace.config.packs();
                for candidate in closest(&wrong, packs.iter().map(|pack| pack.pack_ref.as_str())) {
                    if let Some((range, text)) =
                        replace_in_range(&lines, diagnostic.range, &wrong, candidate, false)
                    {
                        actions.push(action(
                            uri,
                            format!("Change pack to \"{candidate}\""),
                            diagnostic,
                            range,
                            text,
                        ));
                    }
                }
            }
            "template.unknown-provider" => {
                let pattern = regex(
                    &PROVIDER,
                    r#"unknown template prefix "([^"]*)"; did you mean "([^"]+)""#,
                );
                if let Some(captures) = pattern.captures(message) {
                    let (wrong, right) = (&captures[1], &captures[2]);
                    if let Some((range, text)) = replace_in_range(
                        &lines,
                        diagnostic.range,
                        &format!("{{{wrong}:"),
                        &format!("{{{right}:"),
                        false,
                    )
                    .or_else(|| replace_in_range(&lines, diagnostic.range, wrong, right, false))
                    {
                        actions.push(action(
                            uri,
                            format!("Change provider to \"{right}\""),
                            diagnostic,
                            range,
                            text,
                        ));
                    }
                }
            }
            "setting.unknown-id" => {
                let pattern = regex(
                    &SETTING,
                    r#"(\w+) setting "([^"]+)" is not in the datapack settings index"#,
                );
                let Some(captures) = pattern.captures(message) else {
                    continue;
                };
                let (provider, wrong) = (captures[1].to_owned(), captures[2].to_owned());
                let Some(settings) = env.reflection.settings() else {
                    continue;
                };
                let known = settings.of_provider(&provider);
                for candidate in closest(&wrong, known.iter().map(|setting| setting.id.as_str())) {
                    if let Some((range, text)) =
                        replace_in_range(&lines, diagnostic.range, &wrong, candidate, false)
                    {
                        actions.push(action(
                            uri,
                            format!("Change to \"{candidate}\""),
                            diagnostic,
                            range,
                            text,
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_prefers_double_quotes() {
        assert_eq!(quoted("{cfg:a}"), "\"{cfg:a}\"");
        assert_eq!(quoted("{x:'a'}"), "\"{x:'a'}\"");
        assert_eq!(quoted("say \"hi\" {cfg:a}"), "'say \"hi\" {cfg:a}'");
        assert_eq!(quoted("\\{cfg:a} it's"), "'\\{cfg:a} it''s'");
    }

    /// Regression: the search lower-cased the text and mapped offsets of the copy onto the original.
    /// The Kelvin sign (U+212A, three bytes) lower-cases to `k` (one byte), which shifted every
    /// later offset.
    #[test]
    fn case_insensitive_search_maps_offsets_onto_the_original_text() {
        let line = "k: \u{212A}\u{212A} Balance.Speed tail";
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 40,
            },
        };
        let (found, text) =
            replace_in_range(&[line], range, "balance.speed", "balance.speed", true).unwrap();
        // `k: ` + two single-unit characters + a space = column 6.
        assert_eq!((found.start.character, found.end.character), (6, 19));
        assert_eq!(text, "balance.speed");
        assert_eq!(find_ascii_ci("\u{212A}abc", "ABC"), Some(3));
        assert_eq!(
            find_ascii_ci("\u{212A}", "k"),
            None,
            "only ASCII letters fold"
        );
        assert_eq!(find_ascii_ci("abc", "abcd"), None);
    }

    #[test]
    fn closest_ranks_by_distance() {
        let ids = ["balance.speed", "balance.stack", "other"];
        assert_eq!(
            closest("balance.spede", ids.iter().copied()),
            vec!["balance.speed"]
        );
        assert!(closest("zzz", ids.iter().copied()).is_empty());
    }
}
