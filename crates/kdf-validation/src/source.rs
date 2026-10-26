use saphyr::{LoadableYamlNode, MarkedYaml};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug)]
pub struct SourceMap<'a> {
    text: &'a str,
    documents: Vec<MarkedYaml<'a>>,
    /// Byte offset of every line start (lines split at `\n`).
    line_starts: Vec<usize>,
}

/// How a YAML scalar was written in the source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScalarStyle {
    Plain,
    SingleQuoted,
    DoubleQuoted,
    /// `|` or `>` block scalar.
    Block,
}

impl ScalarStyle {
    /// Only plain scalars are subject to typed references; quoted and block scalars are literals.
    pub(crate) fn is_plain(self) -> bool {
        self == ScalarStyle::Plain
    }
}

/// Exact position data of one scalar node (UTF-16 columns).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScalarInfo {
    pub(crate) style: ScalarStyle,
    /// First character of the scalar as written (the opening quote for quoted scalars).
    pub(crate) start: Position,
    /// Position after the last character as written.
    pub(crate) end: Position,
    /// First content character (after the opening quote).
    pub(crate) content_start: Position,
    pub(crate) single_line: bool,
}

impl ScalarInfo {
    /// Range of the UTF-16 offsets `from..to` of the scalar VALUE, when the value maps one-to-one
    /// onto the source text (single line, no escapes, no folding). `None` otherwise.
    pub(crate) fn value_range(&self, value_units: u32, from: u32, to: u32) -> Option<Range> {
        if !self.single_line || self.style == ScalarStyle::Block {
            return None;
        }
        let quote = u32::from(self.style != ScalarStyle::Plain);
        let raw = self.end.character.checked_sub(self.start.character)?;
        if raw != value_units + 2 * quote {
            return None;
        }
        let at = |offset: u32| Position {
            line: self.content_start.line,
            character: self.content_start.character + offset.min(value_units),
        };
        let (start, end) = (from.min(value_units), to.max(from).min(value_units));
        // A zero-width range is invisible in editors: anchor it on a character of the value
        // (the last one when it sits at the end of the input, e.g. an error "at end of input").
        let (start, end) = if end > start {
            (start, end)
        } else if value_units == 0 {
            return None;
        } else if start >= value_units {
            (value_units - 1, value_units)
        } else {
            (start, start + 1)
        };
        Some(Range {
            start: at(start),
            end: at(end),
        })
    }
}

impl<'a> SourceMap<'a> {
    pub fn parse(text: &'a str) -> Result<Self, String> {
        let mut line_starts = vec![0usize];
        line_starts.extend(
            text.bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(index, _)| index + 1),
        );
        MarkedYaml::load_from_str(text)
            .map(|documents| Self {
                text,
                documents,
                line_starts,
            })
            .map_err(|error| error.to_string())
    }

    /// Text of a zero-based line without its line terminator.
    fn line_text(&self, line: usize) -> &'a str {
        let Some(start) = self.line_starts.get(line).copied() else {
            return "";
        };
        let end = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }

    fn step<'n>(node: &'n MarkedYaml<'a>, component: &str) -> Option<&'n MarkedYaml<'a>> {
        let key = component.replace("~1", "/").replace("~0", "~");
        node.data
            .as_mapping_get(&key)
            .or_else(|| {
                node.data.as_mapping().and_then(|mapping| {
                    mapping.iter().find_map(|(mapping_key, value)| {
                        mapping_key
                            .data
                            .as_integer()
                            .is_some_and(|number| number.to_string() == key)
                            .then_some(value)
                    })
                })
            })
            .or_else(|| {
                key.parse::<usize>()
                    .ok()
                    .and_then(|index| node.data.as_sequence_get(index))
            })
    }

    /// Exact node for a JSON pointer, `None` when any step is missing.
    pub(crate) fn node_at(&self, document: usize, json_pointer: &str) -> Option<&MarkedYaml<'a>> {
        let mut node = self.documents.get(document)?;
        for component in json_pointer.split('/').skip(1) {
            node = Self::step(node, component)?;
        }
        Some(node)
    }

    /// Exact UTF-16 position of zero-based `line` and character column `column`.
    fn exact_position(&self, line: usize, column: usize) -> Position {
        let units: usize = self
            .line_text(line)
            .chars()
            .take(column)
            .map(char::len_utf16)
            .sum();
        Position {
            line: line as u32,
            character: units as u32,
        }
    }

    /// Source style and exact position of the node at `json_pointer`.
    ///
    /// The parser reports zero-based columns; quoted scalars start at the quote, block scalars at
    /// their first content line (the style is recovered from the header on the previous line).
    pub(crate) fn scalar_info(&self, document: usize, json_pointer: &str) -> Option<ScalarInfo> {
        let node = self.node_at(document, json_pointer)?;
        let start_line = node.span.start.line().checked_sub(1)?;
        let start_col = node.span.start.col();
        let end_line = node.span.end.line().checked_sub(1)?;
        let end_col = node.span.end.col();
        let line = self.line_text(start_line);
        let style = match line.chars().nth(start_col) {
            Some('"') => ScalarStyle::DoubleQuoted,
            Some('\'') => ScalarStyle::SingleQuoted,
            _ => {
                let before: String = line.chars().take(start_col).collect();
                if before.trim().is_empty() && self.block_header_above(start_line) {
                    ScalarStyle::Block
                } else {
                    ScalarStyle::Plain
                }
            }
        };
        let start = self.exact_position(start_line, start_col);
        let end = self.exact_position(end_line, end_col);
        let content_start =
            if matches!(style, ScalarStyle::SingleQuoted | ScalarStyle::DoubleQuoted) {
                Position {
                    character: start.character + 1,
                    ..start
                }
            } else {
                start
            };
        Some(ScalarInfo {
            style,
            start,
            end,
            content_start,
            single_line: start_line == end_line,
        })
    }

    /// First character of the node's source text (`{` for flow mappings, `[` for flow sequences).
    pub(crate) fn node_first_char(&self, document: usize, json_pointer: &str) -> Option<char> {
        let node = self.node_at(document, json_pointer)?;
        let line = node.span.start.line().checked_sub(1)?;
        self.line_text(line).chars().nth(node.span.start.col())
    }

    /// Whether the nearest non-blank line above ends with a block scalar header (`|`, `>-`, `|2+`).
    fn block_header_above(&self, line: usize) -> bool {
        let mut index = line;
        while index > 0 {
            index -= 1;
            let text = self.line_text(index);
            let code = match text.find(" #") {
                Some(position) => &text[..position],
                None => text,
            };
            let trimmed = code.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            let Some(last) = trimmed.rfind(['|', '>']) else {
                return false;
            };
            let header = &trimmed[last + 1..];
            let before = &trimmed[..last];
            return header
                .chars()
                .all(|ch| ch.is_ascii_digit() || ch == '+' || ch == '-')
                && (before.is_empty()
                    || before.ends_with(' ')
                    || before.ends_with(':')
                    || before.ends_with('-'));
        }
        false
    }

    pub fn document_count(&self) -> usize {
        self.documents.len()
    }

    pub fn document_is_mapping(&self, document: usize) -> bool {
        self.documents
            .get(document)
            .and_then(|node| node.data.as_mapping())
            .is_some()
    }

    pub fn location(&self, document: usize, json_pointer: &str) -> Range {
        let Some(mut node) = self.documents.get(document) else {
            return Range::default();
        };
        for component in json_pointer.split('/').skip(1) {
            let Some(child) = Self::step(node, component) else {
                break;
            };
            node = child;
        }
        self.marked_range(node)
    }

    /// Exact content-only scalar range, or unknown for aliases, escapes and folded text.
    /// Never derives an edit range by searching for another occurrence of the value.
    pub fn scalar_value_range(&self, document: usize, pointer: &str, value: &str) -> Option<Range> {
        let info = self.scalar_info(document, pointer)?;
        if info.style == ScalarStyle::Block || value.is_empty() || value.contains(['\n', '\r']) {
            return None;
        }
        let start = info.content_start;
        let byte = byte_offset_at_utf16(self.text, start)?;
        let tail = self.text.get(byte..)?;
        if !tail.starts_with(value) {
            return None;
        }
        let after = &tail[value.len()..];
        let boundary = match info.style {
            ScalarStyle::SingleQuoted => after.starts_with('\''),
            ScalarStyle::DoubleQuoted => after.starts_with('"'),
            ScalarStyle::Plain => {
                after.is_empty()
                    || after
                        .starts_with(|c: char| c.is_whitespace() || matches!(c, ',' | ']' | '}'))
            }
            ScalarStyle::Block => false,
        };
        boundary.then_some(Range {
            start,
            end: Position {
                line: start.line,
                character: start.character + value.encode_utf16().count() as u32,
            },
        })
    }

    /// Range of a parsed node. Saphyr markers carry a 1-based line and a 0-based character
    /// column, so both ends are mapped with [`Self::exact_position`] (reading the column as
    /// 1-based shifts the range one column to the left). Nodes spanning several lines, and empty
    /// nodes, collapse to one character at their start.
    pub fn marked_range(&self, node: &MarkedYaml<'_>) -> Range {
        // saphyr-parser's scanner starts at Marker::new(0, 1, 0): its columns
        // are zero-based, unlike the one-based serde_yaml syntax positions.
        let raw_start = self.position(
            node.span.start.line(),
            node.span.start.col().saturating_add(1),
        );
        let raw_end = self.position(node.span.end.line(), node.span.end.col().saturating_add(1));
        let start = self.trim_separator_space(raw_start, raw_end);
        // Quoted scalar spans can include the trailing comment consumed by the
        // scanner. Keep a single-line string's range on its literal token.
        let raw_end = if node.data.as_str().is_some() {
            self.quoted_scalar_end(start).unwrap_or(raw_end)
        } else {
            raw_end
        };
        let end = if raw_end > start && raw_end.line == start.line {
            raw_end
        } else {
            Position {
                character: start.character + 1,
                ..start
            }
        };
        Range { start, end }
    }

    pub fn syntax_position(&self, line: usize, column: usize) -> Range {
        let start = self.position(line, column);
        Range {
            start,
            end: Position {
                character: start.character + 1,
                ..start
            },
        }
    }

    fn position(&self, line_1: usize, column_1: usize) -> Position {
        utf16_position(self.text, line_1, column_1)
    }

    fn quoted_scalar_end(&self, start: Position) -> Option<Position> {
        let line = self.text.lines().nth(start.line as usize)?;
        let byte = byte_offset_at_utf16(
            line,
            Position {
                line: 0,
                character: start.character,
            },
        )?;
        let token = line.get(byte..)?;
        let quote = token.chars().next()?;
        if !matches!(quote, '\'' | '"') {
            return None;
        }
        let mut characters = token.char_indices().skip(1).peekable();
        while let Some((index, character)) = characters.next() {
            if quote == '"' && character == '\\' {
                characters.next();
            } else if character == quote {
                if quote == '\'' && characters.peek().is_some_and(|(_, next)| *next == '\'') {
                    characters.next();
                } else {
                    return Some(Position {
                        line: start.line,
                        character: line[..byte + index + 1].encode_utf16().count() as u32,
                    });
                }
            }
        }
        None
    }

    fn trim_separator_space(&self, start: Position, end: Position) -> Position {
        let Some(line) = self.text.lines().nth(start.line as usize) else {
            return start;
        };
        let Some(byte) = byte_offset_at_utf16(
            line,
            Position {
                line: 0,
                character: start.character,
            },
        ) else {
            return start;
        };
        let spaces = line[byte..]
            .chars()
            .take_while(|character| *character == ' ' || *character == '\t')
            .map(char::len_utf16)
            .sum::<usize>() as u32;
        if end.line == start.line && start.character.saturating_add(spaces) >= end.character {
            return start;
        }
        Position {
            character: start.character.saturating_add(spaces),
            ..start
        }
    }
}

pub(crate) fn utf16_position(text: &str, line_1: usize, column_1: usize) -> Position {
    let line = line_1.saturating_sub(1);
    let prefix = text.lines().nth(line).unwrap_or("");
    let units = prefix
        .chars()
        .take(column_1.saturating_sub(1))
        .map(char::len_utf16)
        .sum::<usize>();
    Position {
        line: line as u32,
        character: units as u32,
    }
}

/// Byte offset of a protocol position, clamped the way the LSP specification demands: a line past
/// the last line is the end of the text, a character past the end of its line is the end of the line
/// (before the line terminator). A position inside a surrogate pair rounds down to the start of that
/// character. Unlike [`byte_offset_at_utf16`] this never fails.
pub fn clamped_byte_offset_at_utf16(text: &str, position: Position) -> usize {
    let mut line_start = 0usize;
    for _ in 0..position.line {
        match text[line_start..].find('\n') {
            Some(newline) => line_start += newline + 1,
            None => return text.len(),
        }
    }
    let rest = &text[line_start..];
    let full_line = rest.split('\n').next().unwrap_or("");
    let line = full_line.strip_suffix('\r').unwrap_or(full_line);
    let wanted = position.character as usize;
    let mut utf16 = 0usize;
    for (index, ch) in line.char_indices() {
        let next = utf16 + ch.len_utf16();
        if utf16 >= wanted || next > wanted {
            return line_start + index;
        }
        utf16 = next;
    }
    line_start + line.len()
}

pub fn byte_offset_at_utf16(text: &str, position: Position) -> Option<usize> {
    let mut line_start = 0usize;
    for _ in 0..position.line {
        let newline = text.get(line_start..)?.find('\n')?;
        line_start += newline + 1;
    }
    let full_line = text.get(line_start..)?.split('\n').next()?;
    let line = full_line.strip_suffix('\r').unwrap_or(full_line);
    let mut utf16 = 0usize;
    for (index, ch) in line.char_indices() {
        if utf16 == position.character as usize {
            return Some(line_start + index);
        }
        utf16 += ch.len_utf16();
        if utf16 > position.character as usize {
            return None;
        }
    }
    (utf16 == position.character as usize).then_some(line_start + line.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_position_handles_non_bmp() {
        let text = "a😀b\nsecond";
        assert_eq!(
            byte_offset_at_utf16(
                text,
                Position {
                    line: 0,
                    character: 3
                }
            ),
            Some(5)
        );
        assert_eq!(
            byte_offset_at_utf16(
                text,
                Position {
                    line: 0,
                    character: 2
                }
            ),
            None
        );
        assert_eq!(
            byte_offset_at_utf16(
                text,
                Position {
                    line: 1,
                    character: 1
                }
            ),
            Some(8)
        );
        assert_eq!(
            byte_offset_at_utf16(
                "a\n",
                Position {
                    line: 1,
                    character: 0
                }
            ),
            Some(2)
        );
        assert_eq!(
            byte_offset_at_utf16(
                "a\n",
                Position {
                    line: 2,
                    character: 0
                }
            ),
            None
        );
        assert_eq!(
            byte_offset_at_utf16(
                "a",
                Position {
                    line: 1,
                    character: 0
                }
            ),
            None
        );
        assert_eq!(
            byte_offset_at_utf16(
                "a\r\n",
                Position {
                    line: 0,
                    character: 1
                }
            ),
            Some(1)
        );
    }

    #[test]
    fn clamped_offsets_follow_the_lsp_specification() {
        let at = |line: u32, character: u32| Position { line, character };
        let text = "ab\r\ncd\ne😀f";
        // Inside the text: like the strict conversion.
        assert_eq!(clamped_byte_offset_at_utf16(text, at(0, 1)), 1);
        assert_eq!(clamped_byte_offset_at_utf16(text, at(1, 1)), 5);
        // A character past the end of the line is the end of the line, before `\r\n`.
        assert_eq!(clamped_byte_offset_at_utf16(text, at(0, 99)), 2);
        assert_eq!(clamped_byte_offset_at_utf16(text, at(1, 99)), 6);
        // A line past the last line is the end of the document.
        assert_eq!(clamped_byte_offset_at_utf16(text, at(99, 0)), text.len());
        assert_eq!(clamped_byte_offset_at_utf16(text, at(3, 0)), text.len());
        assert_eq!(clamped_byte_offset_at_utf16("", at(5, 5)), 0);
        // Inside a surrogate pair: the start of the character.
        assert_eq!(clamped_byte_offset_at_utf16(text, at(2, 2)), 8);
        assert_eq!(clamped_byte_offset_at_utf16(text, at(2, 3)), 12);
        // The empty last line after a trailing newline is a valid position.
        assert_eq!(clamped_byte_offset_at_utf16("a\n", at(1, 0)), 2);
        assert_eq!(clamped_byte_offset_at_utf16("a\n", at(1, 7)), 2);
    }

    #[test]
    fn value_range_anchors_zero_width_ranges_on_a_character() {
        let source = SourceMap::parse("a: \"abc\"\nb: \"\"\n").unwrap();
        let info = source.scalar_info(0, "/a").unwrap();
        // Normal range.
        let range = info.value_range(3, 1, 2).unwrap();
        assert_eq!((range.start.character, range.end.character), (5, 6));
        // At the end of the input: the last character.
        let range = info.value_range(3, 3, 4).unwrap();
        assert_eq!((range.start.character, range.end.character), (6, 7));
        let range = info.value_range(3, 3, 3).unwrap();
        assert_eq!((range.start.character, range.end.character), (6, 7));
        // Zero-width at the start of a non-empty value: its first character.
        let range = info.value_range(3, 0, 0).unwrap();
        assert_eq!(
            (range.start.character, range.end.character),
            (
                info.content_start.character,
                info.content_start.character + 1
            )
        );
        // An empty value has no character to anchor on: the caller falls back to the whole node.
        let empty = source.scalar_info(0, "/b").unwrap();
        assert!(empty.value_range(0, 0, 1).is_none());
    }

    #[test]
    fn numeric_mapping_key_resolves_to_its_value_in_yaml_source() {
        let text = "type: dataasset\nassets:\n  - id: ManufacturerMods\n    properties:\n      - path: mOutput.mFluidSlotIcons\n        value:\n          0: /KDataForge/GenAssets/KDFSample.Gen_KDFSample_SuperIngotIcon\n";
        let source = SourceMap::parse(text).unwrap();
        let range = source.location(0, "/assets/0/properties/0/value/0");
        assert_eq!(range.start.line, 6);
        assert_eq!(range.start.character, 13);
    }

    /// Spike S2: the raw first character of a scalar tells quoted from plain; block scalars start
    /// at their content, so the header line above is inspected.
    #[test]
    fn scalar_style_oracle_distinguishes_plain_quoted_and_block() {
        let text = "a: cfg:x\nb: 'cfg:y'\nc: \"cfg:z\"\nd: |\n  cfg:w\ne: >-\n  cfg:v\nf: [cfg:a,'cfg:b', \"cfg:c\"]\ng:\n  - cfg:q\n  - |\n    cfg:p\nh: !!str cfg:r\nj: \"é😀 cfg:s\"\nk: &anc cfg:t\n";
        let source = SourceMap::parse(text).unwrap();
        let style = |pointer: &str| source.scalar_info(0, pointer).unwrap().style;
        assert_eq!(style("/a"), ScalarStyle::Plain);
        assert_eq!(style("/b"), ScalarStyle::SingleQuoted);
        assert_eq!(style("/c"), ScalarStyle::DoubleQuoted);
        assert_eq!(style("/d"), ScalarStyle::Block);
        assert_eq!(style("/e"), ScalarStyle::Block);
        assert_eq!(style("/f/0"), ScalarStyle::Plain);
        assert_eq!(style("/f/1"), ScalarStyle::SingleQuoted);
        assert_eq!(style("/f/2"), ScalarStyle::DoubleQuoted);
        assert_eq!(style("/g/0"), ScalarStyle::Plain);
        assert_eq!(style("/g/1"), ScalarStyle::Block);
        assert_eq!(style("/h"), ScalarStyle::Plain);
        assert_eq!(style("/k"), ScalarStyle::Plain);
        // Exact UTF-16 columns: `j: "é😀 cfg:s"` has the emoji as a surrogate pair.
        let info = source.scalar_info(0, "/j").unwrap();
        assert_eq!(info.start.character, 3);
        assert_eq!(info.content_start.character, 4);
        assert_eq!(info.end.character, 14);
        let value_units = "é😀 cfg:s".encode_utf16().count() as u32;
        let range = info.value_range(value_units, 4, 7).unwrap();
        assert_eq!(range.start.character, 8);
        assert_eq!(range.end.character, 11);
        // Flow mapping started by an unquoted `{`.
        let flow = SourceMap::parse("v: {item:X:mName}\n").unwrap();
        assert_eq!(flow.node_first_char(0, "/v"), Some('{'));
    }

    /// Regression: the saphyr marker column is 0-based, so reading it as 1-based made every
    /// scalar range end one column early (`value: abc` covered `ab`). The range must cover the
    /// scalar exactly as written, quotes included, with UTF-16 columns.
    #[test]
    fn marked_range_covers_scalar_exactly_as_written() {
        let text = "a: abc\nb: 'abc'\nc: \"abc\"\nd: 42\ne: [x, yy]\nf:\n  - plain\n  - 'q'\ng: é😀z\ni: abc # c\nj: {k: v}\nl:\n  m: abc\nn: cfg:balance.speed\n";
        let source = SourceMap::parse(text).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let slice = |range: Range| -> String {
            assert_eq!(range.start.line, range.end.line, "single-line range");
            let units: Vec<u16> = lines[range.start.line as usize].encode_utf16().collect();
            String::from_utf16(&units[range.start.character as usize..range.end.character as usize])
                .unwrap()
        };
        for (pointer, expected) in [
            ("/a", "abc"),
            ("/b", "'abc'"),
            ("/c", "\"abc\""),
            ("/d", "42"),
            ("/e/0", "x"),
            ("/e/1", "yy"),
            ("/f/0", "plain"),
            ("/f/1", "'q'"),
            ("/g", "é😀z"),
            ("/i", "abc"),
            ("/j/k", "v"),
            ("/l/m", "abc"),
            ("/n", "cfg:balance.speed"),
        ] {
            assert_eq!(slice(source.location(0, pointer)), expected, "{pointer}");
        }
        // Multi-line and block nodes collapse to one character at their start.
        let block = SourceMap::parse("h: |\n  blk\nl:\n  m: abc\n").unwrap();
        for pointer in ["/h", "/l"] {
            let range = block.location(0, pointer);
            assert_eq!(range.end.character, range.start.character + 1, "{pointer}");
            assert_eq!(range.end.line, range.start.line, "{pointer}");
        }
    }

    #[test]
    fn marked_range_starts_on_scalar_token_after_yaml_separator() {
        let text = "type: cdo\nvalue: 42\n";
        let source = SourceMap::parse(text).unwrap();
        let range = source.location(0, "/value");
        assert_eq!(
            range.start,
            Position {
                line: 1,
                character: 7
            }
        );
        let byte = byte_offset_at_utf16(text, range.start).unwrap();
        assert_eq!(text.as_bytes()[byte], b'4');
    }

    #[test]
    fn marked_range_covers_complete_scalar_with_quotes_and_unicode() {
        for (text, token) in [
            ("op: multiplly\n", "multiplly"),
            ("op: 'multiplly' # comment\n", "'multiplly'"),
            ("op: \"multipl😀y\" # comment\n", "\"multipl😀y\""),
            ("op: 'it''s # quoted' # comment\n", "'it''s # quoted'"),
            (
                "op: \"a \\\"quoted\\\" # value\" # comment\n",
                "\"a \\\"quoted\\\" # value\"",
            ),
            ("'😀': before\nop: multiplly\n", "multiplly"),
        ] {
            let source = SourceMap::parse(text).unwrap();
            let range = source.location(0, "/op");
            let start = byte_offset_at_utf16(text, range.start).unwrap();
            let end = byte_offset_at_utf16(text, range.end).unwrap();
            assert_eq!(&text[start..end], token, "input: {text:?}");
        }
    }
}
