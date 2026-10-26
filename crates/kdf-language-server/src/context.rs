//! Text-based, tolerant cursor resolver.
//!
//! Completion, hover, signature help and navigation all need to know "where am I" while the
//! document is still being typed (`value: cfg:` with the colon at the end of the line is not valid
//! YAML for any parser). This module therefore never parses YAML: it scans lines, builds a small
//! structural arena (mappings, sequences, their keys) for the document region around the cursor and
//! describes the cursor with it:
//!
//! * the document region (index, `type`),
//! * the place (key slot, value slot with the decoded scalar, comment, block scalar),
//! * the frames from the document root to the cursor (mapping keys and sequences),
//! * the enclosing operation entry (`path` / `op` / `value`) and the patch entry that owns it,
//! * the class the patch refers to (`ofClass` / `target` / `allAssetsOfClass` / `parent` / `class`).
//!
//! All columns handed out are UTF-16 code units, like the protocol.

use crate::features::yaml_content_prefix;
use kdf_validation::infer_type;
use kdf_validation::template::{APPLY_PREFIXES, PREPASS_PREFIXES};
use std::path::Path;
use tower_lsp::lsp_types::{Position, Range};

// -------------------------------------------------------------------------------------------------
// Small text helpers
// -------------------------------------------------------------------------------------------------

pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

pub(crate) fn utf16_len(text: &str) -> u32 {
    text.encode_utf16().count() as u32
}

/// Byte index of UTF-16 column `column` in `line` (clamped to the line end).
pub(crate) fn byte_at(line: &str, column: u32) -> usize {
    let mut units = 0u32;
    for (index, ch) in line.char_indices() {
        if units >= column {
            return index;
        }
        units += ch.len_utf16() as u32;
    }
    line.len()
}

/// UTF-16 column of byte index `byte`.
pub(crate) fn col_at(line: &str, byte: usize) -> u32 {
    utf16_len(&line[..byte.min(line.len())])
}

pub(crate) fn range_on(line: u32, start: u32, end: u32) -> Range {
    Range {
        start: Position {
            line,
            character: start,
        },
        end: Position {
            line,
            character: end,
        },
    }
}

fn is_known_prefix(name: &str) -> bool {
    PREPASS_PREFIXES.contains(&name) || APPLY_PREFIXES.contains(&name)
}

// -------------------------------------------------------------------------------------------------
// Scalars
// -------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Quote {
    Plain,
    Single,
    Double,
}

/// The scalar under (or at) the cursor, decoded, with the mapping back to document columns.
#[derive(Clone, Debug)]
pub(crate) struct Scalar {
    pub(crate) quote: Quote,
    pub(crate) line: u32,
    /// Column of the first raw character (the opening quote of quoted scalars).
    pub(crate) raw_start: u32,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) closed: bool,
    /// Decoded text (quotes removed, escapes resolved).
    pub(crate) value: String,
    /// Raw column of every decoded UTF-16 unit, plus the end of the content.
    map: Vec<u32>,
    /// Decoded UTF-16 units before the cursor.
    pub(crate) cursor: u32,
    /// The cursor lies within the content (quotes excluded).
    pub(crate) inside: bool,
}

impl Scalar {
    pub(crate) fn units(&self) -> u32 {
        self.map.len().saturating_sub(1) as u32
    }

    /// Document column of decoded offset `offset`.
    pub(crate) fn raw_col(&self, offset: u32) -> u32 {
        let index = (offset as usize).min(self.map.len().saturating_sub(1));
        self.map.get(index).copied().unwrap_or(self.raw_start)
    }

    /// Document range of the decoded span `start..end`.
    pub(crate) fn range(&self, start: u32, end: u32) -> Range {
        range_on(self.line, self.raw_col(start), self.raw_col(end.max(start)))
    }

    /// Decoded text before the cursor.
    pub(crate) fn before_cursor(&self) -> String {
        prefix_units(&self.value, self.cursor)
    }

    /// Raw spelling of `text` inside this scalar (escapes the quote style needs).
    pub(crate) fn encode(&self, text: &str) -> String {
        match self.quote {
            Quote::Plain => text.to_owned(),
            Quote::Single => text.replace('\'', "''"),
            Quote::Double => text.replace('\\', "\\\\").replace('"', "\\\""),
        }
    }

    /// Quote character for string literals written inside a scalar of this style.
    pub(crate) fn literal_quote(&self) -> char {
        if self.quote == Quote::Single {
            '"'
        } else {
            '\''
        }
    }
}

/// First `units` UTF-16 units of `text`.
pub(crate) fn prefix_units(text: &str, units: u32) -> String {
    let mut used = 0u32;
    let mut out = String::new();
    for ch in text.chars() {
        let width = ch.len_utf16() as u32;
        if used + width > units {
            break;
        }
        used += width;
        out.push(ch);
    }
    out
}

/// Decodes the scalar that starts at byte `start` of `line` and may extend to byte `limit`.
pub(crate) fn scalar_at(
    line: &str,
    line_no: u32,
    start: usize,
    limit: usize,
    quote: Quote,
    cursor_byte: usize,
) -> Scalar {
    let mut value = String::new();
    let mut map: Vec<u32> = Vec::new();
    let push = |value: &mut String, map: &mut Vec<u32>, ch: char, col: u32| {
        value.push(ch);
        for _ in 0..ch.len_utf16() {
            map.push(col);
        }
    };
    let limit = limit.min(line.len());
    let mut closed = false;
    let (content_start, content_end);
    match quote {
        Quote::Plain => {
            let mut end = limit;
            while end > start && line.as_bytes()[end - 1].is_ascii_whitespace() {
                end -= 1;
            }
            for (offset, ch) in line[start..end].char_indices() {
                push(&mut value, &mut map, ch, col_at(line, start + offset));
            }
            content_start = start;
            content_end = end;
            closed = true;
        }
        Quote::Single | Quote::Double => {
            content_start = start + 1;
            let mut end = limit;
            let mut chars = line[content_start.min(limit)..limit]
                .char_indices()
                .peekable();
            while let Some((offset, ch)) = chars.next() {
                let at = content_start + offset;
                if quote == Quote::Double && ch == '\\' {
                    if let Some((_, next)) = chars.next() {
                        let decoded = match next {
                            'n' => '\n',
                            't' => '\t',
                            'r' => '\r',
                            '0' => '\0',
                            other => other,
                        };
                        push(&mut value, &mut map, decoded, col_at(line, at));
                    } else {
                        push(&mut value, &mut map, ch, col_at(line, at));
                    }
                    continue;
                }
                if quote == Quote::Single && ch == '\'' {
                    if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                        chars.next();
                        push(&mut value, &mut map, '\'', col_at(line, at));
                        continue;
                    }
                    end = at;
                    closed = true;
                    break;
                }
                if quote == Quote::Double && ch == '"' {
                    end = at;
                    closed = true;
                    break;
                }
                push(&mut value, &mut map, ch, col_at(line, at));
            }
            content_end = end;
        }
    }
    map.push(col_at(line, content_end));
    let inside = cursor_byte >= content_start && cursor_byte <= content_end;
    let cursor_col = col_at(line, cursor_byte.clamp(content_start, content_end));
    // Decoded units before the cursor: escapes make raw and decoded widths differ, so count
    // through the column map.
    let decoded_cursor = map
        .iter()
        .take(map.len().saturating_sub(1))
        .take_while(|col| **col < cursor_col)
        .count() as u32;
    Scalar {
        quote,
        line: line_no,
        raw_start: col_at(line, start),
        closed,
        value,
        map,
        cursor: decoded_cursor,
        inside,
    }
}

// -------------------------------------------------------------------------------------------------
// Line scanning
// -------------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct KeyTok {
    name: String,
    name_start: usize,
    colon: usize,
}

#[derive(Clone, Debug)]
struct LineScan {
    indent: usize,
    dashes: Vec<usize>,
    content_col: usize,
    /// Byte index where the comment starts (or the line ends).
    end: usize,
    blank: bool,
    key: Option<KeyTok>,
}

fn find_key(line: &str, start: usize, end: usize) -> Option<KeyTok> {
    let text = line.get(start..end)?;
    let first = text.chars().next()?;
    if matches!(
        first,
        '[' | '{' | '|' | '>' | '&' | '*' | '!' | '%' | '@' | '`' | '#' | '?'
    ) {
        return None;
    }
    if first == '"' || first == '\'' {
        let mut chars = text.char_indices().skip(1).peekable();
        while let Some((offset, ch)) = chars.next() {
            if first == '"' && ch == '\\' {
                chars.next();
                continue;
            }
            if ch == first {
                if first == '\'' && chars.peek().is_some_and(|(_, next)| *next == '\'') {
                    chars.next();
                    continue;
                }
                let rest = &text[offset + 1..];
                let trimmed = rest.trim_start();
                if let Some(after) = trimmed.strip_prefix(':') {
                    if after.is_empty() || after.starts_with(' ') {
                        let colon = start + offset + 1 + (rest.len() - trimmed.len());
                        return Some(KeyTok {
                            name: text[1..offset].to_owned(),
                            name_start: start,
                            colon,
                        });
                    }
                }
                return None;
            }
        }
        return None;
    }
    let bytes = text.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b':' {
            continue;
        }
        let next = bytes.get(index + 1).copied();
        if next.is_none() || next == Some(b' ') {
            let name = text[..index].trim_end();
            if name.is_empty() {
                return None;
            }
            // `cfg:` typed at the end of a list item is a reference being typed, not a key.
            if next.is_none() && PREPASS_PREFIXES.contains(&name) {
                return None;
            }
            return Some(KeyTok {
                name: name.to_owned(),
                name_start: start,
                colon: start + index,
            });
        }
    }
    None
}

fn scan_line(line: &str) -> LineScan {
    let end = yaml_content_prefix(line).len();
    let bytes = line.as_bytes();
    let mut indent = 0usize;
    while indent < end && bytes[indent] == b' ' {
        indent += 1;
    }
    let mut pos = indent;
    let mut dashes = Vec::new();
    while pos < end && bytes[pos] == b'-' && (pos + 1 >= end || bytes[pos + 1] == b' ') {
        dashes.push(pos);
        pos += 1;
        while pos < end && bytes[pos] == b' ' {
            pos += 1;
        }
    }
    let blank = dashes.is_empty() && line[pos..end].trim().is_empty();
    let key = if blank {
        None
    } else {
        find_key(line, pos, end)
    };
    LineScan {
        indent,
        dashes,
        content_col: pos,
        end,
        blank,
        key,
    }
}

// -------------------------------------------------------------------------------------------------
// Flow collections (single line, plus carried state for multi-line ones)
// -------------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct FlowItem {
    start: usize,
    /// End without trailing whitespace.
    end: usize,
    /// End including trailing whitespace (the cursor can still be inside).
    raw_end: usize,
    /// Key of a `key: value` item and the byte index of its colon.
    key: Option<(String, usize)>,
}

#[derive(Clone, Debug)]
struct FlowContainer {
    open: char,
    start: usize,
    end: Option<usize>,
    inherited: bool,
    items: Vec<FlowItem>,
    item_start: usize,
    item_key: Option<(String, usize)>,
}

impl FlowContainer {
    fn new(open: char, start: usize, item_start: usize, inherited: bool) -> Self {
        Self {
            open,
            start,
            end: None,
            inherited,
            items: Vec::new(),
            item_start,
            item_key: None,
        }
    }

    /// Closes the item that ends at `end`; the next one starts at `next`.
    fn finish_item(&mut self, bytes: &[u8], end: usize, next: usize) {
        let mut trimmed = end;
        while trimmed > self.item_start && bytes[trimmed - 1].is_ascii_whitespace() {
            trimmed -= 1;
        }
        self.items.push(FlowItem {
            start: self.item_start,
            end: trimmed,
            raw_end: end,
            key: self.item_key.take(),
        });
        self.item_start = next;
    }
}

/// A quote starts a flow scalar only where a scalar can start (`it's` stays plain text).
fn at_scalar_start(bytes: &[u8], index: usize, from: usize) -> bool {
    let mut cursor = index;
    while cursor > from {
        cursor -= 1;
        if bytes[cursor] == b' ' || bytes[cursor] == b'\t' {
            continue;
        }
        return matches!(bytes[cursor], b'[' | b'{' | b',' | b':');
    }
    true
}

/// Splits `line[from..to]` (flow content) into containers in opening order. `inherited` are the
/// containers still open from previous lines.
fn flow_scan(line: &str, from: usize, to: usize, inherited: &[char]) -> Vec<FlowContainer> {
    let bytes = line.as_bytes();
    let to = to.min(bytes.len());
    let mut containers: Vec<FlowContainer> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for open in inherited {
        containers.push(FlowContainer::new(*open, from, from, true));
        stack.push(containers.len() - 1);
    }
    let mut index = from;
    while index < to {
        let byte = bytes[index];
        match byte {
            b'"' | b'\'' if at_scalar_start(bytes, index, from) => {
                let mut cursor = index + 1;
                while cursor < to {
                    if byte == b'"' && bytes[cursor] == b'\\' {
                        cursor += 2;
                        continue;
                    }
                    if bytes[cursor] == byte {
                        if byte == b'\'' && bytes.get(cursor + 1) == Some(&b'\'') {
                            cursor += 2;
                            continue;
                        }
                        break;
                    }
                    cursor += 1;
                }
                index = (cursor + 1).min(to);
                continue;
            }
            b'[' | b'{' => {
                containers.push(FlowContainer::new(byte as char, index, index + 1, false));
                stack.push(containers.len() - 1);
            }
            b']' | b'}' => {
                if let Some(top) = stack.pop() {
                    let container = &mut containers[top];
                    container.finish_item(bytes, index, index + 1);
                    container.end = Some(index);
                }
            }
            b',' => {
                if let Some(top) = stack.last().copied() {
                    containers[top].finish_item(bytes, index, index + 1);
                }
            }
            b':' => {
                let separator = matches!(
                    bytes.get(index + 1).copied(),
                    None | Some(b' ') | Some(b',') | Some(b'}')
                );
                if let Some(top) = stack.last().copied().filter(|_| separator) {
                    let container = &mut containers[top];
                    if container.open == '{' && container.item_key.is_none() {
                        let name = line[container.item_start..index]
                            .trim()
                            .trim_matches(['"', '\''])
                            .to_owned();
                        container.item_key = Some((name, index));
                    }
                }
            }
            _ => {}
        }
        index += 1;
    }
    for container in containers
        .iter_mut()
        .filter(|container| container.end.is_none())
    {
        container.finish_item(bytes, to, to);
    }
    containers
}

/// Unmatched openers at the end of `line[from..to]`.
fn flow_open_after(line: &str, from: usize, to: usize, inherited: &[char]) -> Vec<char> {
    flow_scan(line, from, to, inherited)
        .into_iter()
        .filter(|container| container.end.is_none())
        .map(|container| container.open)
        .collect()
}

// -------------------------------------------------------------------------------------------------
// Public result types
// -------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct KeyEntry {
    pub(crate) name: String,
    pub(crate) line: u32,
    /// Decoded single-line inline scalar.
    pub(crate) value: Option<String>,
    /// First scalar of a block sequence below the key (`target:` followed by `- /Game/...`).
    pub(crate) first_item: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum Frame {
    Map {
        keys: Vec<KeyEntry>,
        /// Key the cursor path continues through.
        selected: Option<String>,
    },
    Seq {
        via_key: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct KeySlot {
    /// Key text before the cursor.
    pub(crate) typed: String,
    pub(crate) line: u32,
    pub(crate) start: u32,
    pub(crate) end: u32,
    /// The key is already followed by its colon.
    pub(crate) has_colon: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ValueSlot {
    pub(crate) scalar: Scalar,
    /// The scalar is an element of a block or flow sequence.
    pub(crate) list_item: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) flow: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum Place {
    /// Comment, block scalar content, whitespace outside any scalar.
    None,
    Key(KeySlot),
    Value(ValueSlot),
}

#[derive(Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct DocInfo {
    pub(crate) index: usize,
    pub(crate) first_line: u32,
    /// Lower-case `type`, else inferred from the file name.
    pub(crate) kind: Option<String>,
    pub(crate) multi: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Resolved {
    pub(crate) doc: DocInfo,
    pub(crate) in_comment: bool,
    pub(crate) place: Place,
    pub(crate) frames: Vec<Frame>,
    /// `path:` values of preceding entries of the `properties:` list the cursor is in.
    pub(crate) peer_paths: Vec<String>,
}

/// View of the enclosing operation entry (a mapping that is an element of a `properties:` list).
#[derive(Clone, Debug)]
pub(crate) struct OpView<'a> {
    pub(crate) keys: &'a [KeyEntry],
    /// Keys from the entry down to the cursor (`["value", "Amount"]`, `"#"` for sequence elements).
    pub(crate) below: Vec<String>,
    pub(crate) frame: usize,
}

impl OpView<'_> {
    pub(crate) fn key_value(&self, name: &str) -> Option<&str> {
        self.keys
            .iter()
            .find(|entry| entry.name == name)
            .and_then(|entry| entry.value.as_deref())
    }
    /// Operation name (`set` when `op:` is omitted).
    pub(crate) fn op_name(&self) -> String {
        self.key_value("op").unwrap_or("set").to_ascii_lowercase()
    }
}

/// The entry that owns an operation list (`patches[i]`, `items[i]`, ...).
#[derive(Clone, Debug)]
pub(crate) struct PatchView<'a> {
    pub(crate) keys: &'a [KeyEntry],
    /// Key of the entry list (`patches`, `items`, `recipes`, ...).
    pub(crate) list_key: Option<String>,
    pub(crate) kind: Option<String>,
}

impl PatchView<'_> {
    /// Raw class text the entry refers to, in the order the runtime tries the keys.
    pub(crate) fn class_text(&self) -> Option<String> {
        let names: &[&str] =
            if self.kind.as_deref() == Some("cdo") && self.list_key.as_deref() == Some("patches") {
                &["target", "ofClass", "allAssetsOfClass"]
            } else {
                &["parent", "class"]
            };
        for name in names {
            if let Some(entry) = self.keys.iter().find(|entry| entry.name == *name) {
                if let Some(text) = entry.value.as_ref().or(entry.first_item.as_ref()) {
                    if !text.trim().is_empty() {
                        return Some(text.clone());
                    }
                }
            }
        }
        None
    }

    pub(crate) fn apply_to_spawned_actors(&self) -> bool {
        self.keys
            .iter()
            .find(|entry| entry.name == "applyToSpawnedActors")
            .and_then(|entry| entry.value.as_deref())
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    }

    /// Class every entry of this list derives from when it names no class itself.
    pub(crate) fn default_base(&self) -> Option<&'static str> {
        kdf_validation::slots::default_base_class(self.list_key.as_deref()?)
    }
}

/// View of a `fields[]` entry of a config document.
#[derive(Clone, Debug)]
pub(crate) struct FieldView<'a> {
    pub(crate) keys: &'a [KeyEntry],
}

impl FieldView<'_> {
    pub(crate) fn key_value(&self, name: &str) -> Option<&str> {
        self.keys
            .iter()
            .find(|entry| entry.name == name)
            .and_then(|entry| entry.value.as_deref())
    }
}

impl Resolved {
    /// Current document position, preserving decoded-scalar UTF-16 mapping.
    pub(crate) fn position(&self) -> Option<Position> {
        match &self.place {
            Place::Value(slot) => Some(Position {
                line: slot.scalar.line,
                character: slot.scalar.raw_col(slot.scalar.cursor),
            }),
            Place::Key(slot) => Some(Position {
                line: slot.line,
                character: slot.start + utf16_len(&slot.typed),
            }),
            Place::None => None,
        }
    }
    /// Key the cursor value belongs to (the closest selected mapping key).
    pub(crate) fn key(&self) -> Option<&str> {
        self.frames.iter().rev().find_map(|frame| match frame {
            Frame::Map { selected, .. } => selected.as_deref(),
            Frame::Seq { .. } => None,
        })
    }

    /// Keys on the way from the document root to the cursor (`"#"` for sequence elements).
    pub(crate) fn path(&self) -> Vec<String> {
        let mut path = Vec::new();
        for frame in &self.frames {
            match frame {
                Frame::Map { selected, .. } => {
                    if let Some(key) = selected {
                        path.push(key.clone());
                    }
                }
                Frame::Seq { .. } => path.push("#".to_owned()),
            }
        }
        path
    }

    /// Keys of the innermost mapping.
    pub(crate) fn siblings(&self) -> &[KeyEntry] {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| match frame {
                Frame::Map { keys, .. } => Some(keys.as_slice()),
                Frame::Seq { .. } => None,
            })
            .unwrap_or(&[])
    }

    pub(crate) fn op_entry(&self) -> Option<OpView<'_>> {
        let list = self.frames.iter().rposition(
            |frame| matches!(frame, Frame::Seq { via_key: Some(key) } if key == "properties"),
        )?;
        let Some(Frame::Map { keys, .. }) = self.frames.get(list + 1) else {
            return None;
        };
        let mut below = Vec::new();
        for frame in &self.frames[list + 1..] {
            match frame {
                Frame::Map { selected, .. } => {
                    if let Some(key) = selected {
                        below.push(key.clone());
                    }
                }
                Frame::Seq { .. } => below.push("#".to_owned()),
            }
        }
        Some(OpView {
            keys,
            below,
            frame: list + 1,
        })
    }

    pub(crate) fn patch_entry(&self) -> Option<PatchView<'_>> {
        let op = self.op_entry()?;
        let owner = op.frame.checked_sub(2)?;
        let Some(Frame::Map { keys, .. }) = self.frames.get(owner) else {
            return None;
        };
        let list_key = owner
            .checked_sub(1)
            .and_then(|index| match self.frames.get(index) {
                Some(Frame::Seq { via_key }) => via_key.clone(),
                _ => None,
            });
        Some(PatchView {
            keys,
            list_key,
            kind: self.doc.kind.clone(),
        })
    }

    /// The cursor is below a `conditions:` key.
    pub(crate) fn in_conditions(&self) -> bool {
        self.path().iter().any(|key| key == "conditions")
    }

    /// `fields[]` entry of a config document that holds the cursor.
    pub(crate) fn field_entry(&self) -> Option<FieldView<'_>> {
        let list = self.frames.iter().rposition(
            |frame| matches!(frame, Frame::Seq { via_key: Some(key) } if key == "fields"),
        )?;
        match self.frames.get(list + 1) {
            Some(Frame::Map { keys, .. }) => Some(FieldView { keys }),
            _ => None,
        }
    }

    /// The cursor is directly inside a `fields:` sequence (an element about to be written).
    pub(crate) fn in_fields_list(&self) -> bool {
        matches!(self.frames.last(), Some(Frame::Seq { via_key: Some(key) }) if key == "fields")
    }

    pub(crate) fn is_config_doc(&self) -> bool {
        self.doc.kind.as_deref() == Some("config")
    }
}

// -------------------------------------------------------------------------------------------------
// Resolver
// -------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NodeKind {
    Map,
    Seq,
}

#[derive(Clone, Debug)]
struct Node {
    kind: NodeKind,
    col: usize,
    parent: Option<usize>,
    via_key: Option<String>,
    keys: Vec<KeyEntry>,
}

#[derive(Clone, Debug)]
enum LineState {
    Skip,
    Normal {
        node: usize,
        scan: Box<LineScan>,
    },
    BlockScalar,
    FlowCont {
        node: usize,
        open: Vec<char>,
        selected: Option<String>,
    },
    CursorBlank {
        node: usize,
    },
}

struct Arena {
    nodes: Vec<Node>,
    stack: Vec<usize>,
}

impl Arena {
    fn top(&self) -> Option<usize> {
        self.stack.last().copied()
    }

    fn last_key(&self, node: usize) -> Option<String> {
        self.nodes[node].keys.last().map(|entry| entry.name.clone())
    }

    fn push(&mut self, kind: NodeKind, col: usize, parent: Option<usize>) -> usize {
        let via_key = parent.and_then(|parent| {
            if self.nodes[parent].kind == NodeKind::Map {
                self.last_key(parent)
            } else {
                None
            }
        });
        self.nodes.push(Node {
            kind,
            col,
            parent,
            via_key,
            keys: Vec::new(),
        });
        let id = self.nodes.len() - 1;
        self.stack.push(id);
        id
    }

    fn pop_deeper(&mut self, col: usize) {
        while let Some(top) = self.top() {
            if self.nodes[top].col > col {
                self.stack.pop();
            } else {
                break;
            }
        }
    }

    /// Makes the innermost open node a sequence at `col`.
    fn ensure_seq(&mut self, col: usize) -> usize {
        match self.top() {
            Some(top) if self.nodes[top].col == col && self.nodes[top].kind == NodeKind::Seq => top,
            Some(top) => self.push(NodeKind::Seq, col, Some(top)),
            None => self.push(NodeKind::Seq, col, None),
        }
    }

    /// Makes the innermost open node a mapping at `col`.
    fn ensure_map(&mut self, col: usize) -> usize {
        loop {
            match self.top() {
                Some(top)
                    if self.nodes[top].col == col && self.nodes[top].kind == NodeKind::Map =>
                {
                    return top
                }
                Some(top) if self.nodes[top].col == col => {
                    self.stack.pop();
                }
                Some(top) => return self.push(NodeKind::Map, col, Some(top)),
                None => return self.push(NodeKind::Map, col, None),
            }
        }
    }
}

fn decode_simple(text: &str) -> String {
    let trimmed = text.trim();
    for quote in ['"', '\''] {
        if trimmed.len() >= 2 && trimmed.starts_with(quote) && trimmed.ends_with(quote) {
            let inner = &trimmed[1..trimmed.len() - 1];
            return if quote == '\'' {
                inner.replace("''", "'")
            } else {
                inner.replace("\\\"", "\"").replace("\\\\", "\\")
            };
        }
        if trimmed.starts_with(quote) {
            return trimmed[1..].to_owned();
        }
    }
    trimmed.to_owned()
}

fn key_entry(line: &str, line_no: usize, scan: &LineScan, lines: &[&str]) -> Option<KeyEntry> {
    let key = scan.key.as_ref()?;
    let value_text = line.get(key.colon + 1..scan.end)?.trim();
    let mut value = None;
    let mut first_item = None;
    if value_text.is_empty() {
        // Block sequence below: remember its first scalar.
        for next in lines.iter().skip(line_no + 1) {
            let next_scan = scan_line(next);
            if next_scan.blank {
                continue;
            }
            if let Some(dash) = next_scan.dashes.first() {
                if *dash >= scan.indent.min(key.name_start) {
                    let content = next.get(next_scan.content_col..next_scan.end).unwrap_or("");
                    if find_key(next, next_scan.content_col, next_scan.end).is_none() {
                        first_item = Some(decode_simple(content));
                    }
                }
            }
            break;
        }
    } else if value_text.starts_with('[') {
        let inner = value_text.trim_start_matches('[');
        let inner = inner.split([',', ']']).next().unwrap_or("");
        let decoded = decode_simple(inner);
        if !decoded.is_empty() {
            first_item = Some(decoded);
        }
    } else if !value_text.starts_with(['{', '|', '>']) {
        value = Some(decode_simple(value_text));
    }
    Some(KeyEntry {
        name: key.name.clone(),
        line: line_no as u32,
        value,
        first_item,
    })
}

struct Built {
    arena: Arena,
    states: Vec<LineState>,
}

fn build(
    lines: &[&str],
    first: usize,
    last: usize,
    cursor_line: usize,
    cursor_byte: usize,
) -> Built {
    let mut arena = Arena {
        nodes: Vec::new(),
        stack: Vec::new(),
    };
    let mut states = vec![LineState::Skip; lines.len()];
    let mut block_threshold: Option<usize> = None;
    let mut flow: Option<(usize, Vec<char>, Option<String>)> = None;
    for no in first..=last.min(lines.len().saturating_sub(1)) {
        let line = lines[no];
        let scan = scan_line(line);
        if let Some(threshold) = block_threshold {
            if scan.blank || scan.indent > threshold {
                states[no] = LineState::BlockScalar;
                continue;
            }
            block_threshold = None;
        }
        if let Some((node, open, selected)) = flow.clone() {
            states[no] = LineState::FlowCont {
                node,
                open: open.clone(),
                selected: selected.clone(),
            };
            let carried = flow_open_after(line, 0, scan.end, &open);
            flow = if carried.is_empty() {
                None
            } else {
                Some((node, carried, selected))
            };
            continue;
        }
        if scan.blank {
            if no == cursor_line && line.trim().is_empty() {
                // About to type a key: attach to the mapping the indentation selects.
                let indent = cursor_byte;
                arena.pop_deeper(indent);
                let node = arena.ensure_map(indent);
                states[no] = LineState::CursorBlank { node };
            }
            continue;
        }
        let first_col = scan.dashes.first().copied().unwrap_or(scan.content_col);
        arena.pop_deeper(first_col);
        let mut node;
        if scan.dashes.is_empty() {
            node = arena.ensure_map(first_col);
        } else {
            node = 0;
            for dash in &scan.dashes {
                node = arena.ensure_seq(*dash);
            }
            if scan.key.is_some() {
                node = arena.push(NodeKind::Map, scan.content_col, Some(node));
            }
        }
        if let Some(entry) = key_entry(line, no, &scan, lines) {
            arena.nodes[node].keys.push(entry);
        }
        // Block scalar and flow openers.
        let value_from = match (&scan.key, scan.dashes.is_empty()) {
            (Some(key), _) => key.colon + 1,
            (None, _) => scan.content_col,
        };
        let value = line.get(value_from..scan.end).unwrap_or("").trim_start();
        if value.starts_with(['|', '>']) {
            block_threshold = Some(scan.key.as_ref().map_or(first_col, |key| key.name_start));
        } else if value.starts_with(['[', '{']) && !looks_like_template(value) {
            let from = scan.end - value.len();
            let carried = flow_open_after(line, from, scan.end, &[]);
            if !carried.is_empty() {
                flow = Some((node, carried, scan.key.as_ref().map(|key| key.name.clone())));
            }
        }
        states[no] = LineState::Normal {
            node,
            scan: Box::new(scan),
        };
    }
    Built { arena, states }
}

/// `{cfg:...` / `{item:...` at the start of a plain scalar is a template the author forgot to
/// quote, not a flow mapping.
pub(crate) fn looks_like_template(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('{') else {
        return false;
    };
    let name: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect();
    !name.is_empty() && rest[name.len()..].starts_with(':') && is_known_prefix(&name)
}

fn frames_for(arena: &Arena, node: usize, selected: Option<String>) -> Vec<Frame> {
    let mut chain = Vec::new();
    let mut current = Some(node);
    while let Some(id) = current {
        chain.push(id);
        current = arena.nodes[id].parent;
    }
    chain.reverse();
    let mut frames = Vec::new();
    for (position, id) in chain.iter().enumerate() {
        let n = &arena.nodes[*id];
        match n.kind {
            NodeKind::Map => {
                let selected = match chain.get(position + 1) {
                    Some(child) => arena.nodes[*child].via_key.clone(),
                    None => selected.clone(),
                };
                frames.push(Frame::Map {
                    keys: n.keys.clone(),
                    selected,
                });
            }
            NodeKind::Seq => frames.push(Frame::Seq {
                via_key: n.via_key.clone(),
            }),
        }
    }
    frames
}

/// `path:` values of entries preceding the one holding `node` in the innermost `properties:`
/// list around it. Nodes are allocated in document order; following operations do not make an
/// initial `set` overwrite anything.
fn peer_paths(arena: &Arena, node: usize) -> Vec<String> {
    let mut entry = None;
    let mut current = Some(node);
    while let Some(id) = current {
        let n = &arena.nodes[id];
        if n.kind == NodeKind::Seq && n.via_key.as_deref() == Some("properties") {
            return arena
                .nodes
                .iter()
                .enumerate()
                .filter(|(other, n)| {
                    n.parent == Some(id) && entry.is_some_and(|entry| *other < entry)
                })
                .filter_map(|(_, n)| n.keys.iter().find(|key| key.name == "path"))
                .filter_map(|key| key.value.clone())
                .collect();
        }
        entry = Some(id);
        current = n.parent;
    }
    Vec::new()
}

fn is_separator(line: &str) -> bool {
    line.starts_with("---") && (line.len() == 3 || line.as_bytes()[3] == b' ')
}

/// Line ranges of the documents (separator lines belong to none; empty documents are dropped).
fn doc_regions(lines: &[&str]) -> Vec<(usize, usize)> {
    let mut regions = Vec::new();
    let mut start = 0usize;
    for (index, line) in lines.iter().enumerate() {
        if is_separator(line) {
            if index > start {
                regions.push((start, index - 1));
            }
            start = index + 1;
        }
    }
    if start < lines.len() || regions.is_empty() {
        regions.push((
            start.min(lines.len().saturating_sub(1)),
            lines.len().saturating_sub(1),
        ));
    }
    regions
}

/// Kind of document `type:` names, normalised like the validator (`mam` is `research`).
fn kind_of(path: &Path, root_keys: &[KeyEntry], multi: bool) -> Option<String> {
    let explicit = root_keys
        .iter()
        .find(|entry| entry.name == "type")
        .and_then(|entry| entry.value.clone())
        .filter(|value| !value.is_empty());
    match explicit {
        Some(value) => {
            let value = value.to_ascii_lowercase();
            Some(if value == "mam" {
                "research".into()
            } else {
                value
            })
        }
        None => infer_type(path, &serde_json::Value::Null, multi),
    }
}

/// Byte range of the inline value of a `key: value` or `- value` line (comment and trailing
/// space excluded).
pub(crate) fn inline_value_span(line: &str) -> Option<(usize, usize)> {
    let scan = scan_line(line);
    if scan.blank {
        return None;
    }
    let from = match &scan.key {
        Some(key) => key.colon + 1,
        None => scan.content_col,
    };
    let mut start = from;
    while start < scan.end && line.as_bytes()[start] == b' ' {
        start += 1;
    }
    let mut end = scan.end;
    while end > start && line.as_bytes()[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    (start < end).then_some((start, end))
}

/// Lines after the cursor line the resolver still reads: sibling keys (existing keys, `path`, `type`
/// of the surrounding mapping) are found within them, and a long document is not scanned to its end
/// for every request.
const RESOLVE_LOOKAHEAD: usize = 600;

/// Resolves the cursor. `position` is the protocol position (UTF-16 column).
pub(crate) fn resolve(path: &Path, text: &str, position: Position) -> Resolved {
    let lines = split_lines(text);
    let cursor_line = (position.line as usize).min(lines.len().saturating_sub(1));
    let line = lines.get(cursor_line).copied().unwrap_or("");
    let cursor_byte = byte_at(line, position.character);
    let regions = doc_regions(&lines);
    let on_separator = is_separator(line);
    // A cursor on a separator belongs to the document that follows it.
    let doc_index = regions
        .iter()
        .position(|(first, last)| {
            (cursor_line >= *first && cursor_line <= *last)
                || (on_separator && cursor_line < *first)
        })
        .unwrap_or(regions.len().saturating_sub(1));
    let (first, last) = regions[doc_index];
    let last = last.min(cursor_line.saturating_add(RESOLVE_LOOKAHEAD));
    let built = build(&lines, first, last, cursor_line, cursor_byte);
    let root_keys = built
        .arena
        .nodes
        .iter()
        .find(|node| node.parent.is_none() && node.kind == NodeKind::Map)
        .map(|node| node.keys.clone())
        .unwrap_or_default();
    let multi = regions.len() > 1;
    let doc = DocInfo {
        index: doc_index,
        first_line: first as u32,
        kind: kind_of(path, &root_keys, multi),
        multi,
    };
    let peers = match &built.states[cursor_line] {
        LineState::Normal { node, .. }
        | LineState::FlowCont { node, .. }
        | LineState::CursorBlank { node } => peer_paths(&built.arena, *node),
        LineState::Skip | LineState::BlockScalar => Vec::new(),
    };
    let mut resolved = Resolved {
        doc,
        in_comment: false,
        place: Place::None,
        frames: Vec::new(),
        peer_paths: peers,
    };
    if on_separator {
        return resolved;
    }
    match &built.states[cursor_line] {
        LineState::Skip | LineState::BlockScalar => {
            let end = yaml_content_prefix(line).len();
            resolved.in_comment = cursor_byte > end;
            resolved
        }
        LineState::CursorBlank { node } => {
            resolved.frames = frames_for(&built.arena, *node, None);
            resolved.place = Place::Key(KeySlot {
                typed: String::new(),
                line: cursor_line as u32,
                start: position.character,
                end: position.character,
                has_colon: false,
            });
            resolved
        }
        LineState::FlowCont {
            node,
            open,
            selected,
        } => {
            let scan = scan_line(line);
            if cursor_byte > scan.end {
                resolved.in_comment = true;
                return resolved;
            }
            let mut frames = frames_for(&built.arena, *node, selected.clone());
            flow_place(
                &mut resolved,
                &mut frames,
                line,
                cursor_line,
                0,
                scan.end,
                cursor_byte,
                open,
            );
            resolved.frames = frames;
            resolved
        }
        LineState::Normal { node, scan } => {
            if cursor_byte > scan.end {
                resolved.in_comment = true;
                resolved.frames = frames_for(&built.arena, *node, None);
                return resolved;
            }
            normal_place(
                &mut resolved,
                &built.arena,
                *node,
                scan,
                line,
                cursor_line,
                cursor_byte,
            );
            resolved
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn normal_place(
    resolved: &mut Resolved,
    arena: &Arena,
    node: usize,
    scan: &LineScan,
    line: &str,
    line_no: usize,
    cursor_byte: usize,
) {
    let dash = !scan.dashes.is_empty();
    if dash && cursor_byte < scan.content_col {
        // On the list markers themselves.
        resolved.frames = frames_for(arena, node, None);
        return;
    }
    match &scan.key {
        Some(key) if cursor_byte <= key.colon => {
            resolved.frames = frames_for(arena, node, None);
            let typed = line[key.name_start.min(cursor_byte)..cursor_byte].to_owned();
            resolved.place = Place::Key(KeySlot {
                typed,
                line: line_no as u32,
                start: col_at(line, key.name_start),
                end: col_at(line, key.colon),
                has_colon: true,
            });
        }
        Some(key) => {
            let selected = Some(key.name.clone());
            let mut frames = frames_for(arena, node, selected);
            value_place(
                resolved,
                &mut frames,
                line,
                line_no,
                key.colon + 1,
                scan.end,
                cursor_byte,
                false,
            );
            resolved.frames = frames;
        }
        None => {
            if dash {
                // `- value` (or `- ` with nothing yet): an element of the sequence.
                let mut frames = frames_for(arena, node, None);
                value_place(
                    resolved,
                    &mut frames,
                    line,
                    line_no,
                    scan.content_col,
                    scan.end,
                    cursor_byte,
                    true,
                );
                resolved.frames = frames;
            } else {
                // Bare text without a colon: a key being typed.
                resolved.frames = frames_for(arena, node, None);
                let typed = line[scan.content_col.min(cursor_byte)..cursor_byte].to_owned();
                resolved.place = Place::Key(KeySlot {
                    typed,
                    line: line_no as u32,
                    start: col_at(line, scan.content_col),
                    end: col_at(line, scan.end),
                    has_colon: false,
                });
            }
        }
    }
}

/// Value (or sequence element) text starts at `from`.
#[allow(clippy::too_many_arguments)]
fn value_place(
    resolved: &mut Resolved,
    frames: &mut Vec<Frame>,
    line: &str,
    line_no: usize,
    from: usize,
    end: usize,
    cursor_byte: usize,
    list_item: bool,
) {
    let mut start = from;
    while start < end && line.as_bytes()[start] == b' ' {
        start += 1;
    }
    let rest = line.get(start..end).unwrap_or("");
    let first = rest.chars().next();
    match first {
        None => {
            // Nothing typed yet: an empty plain scalar at the cursor.
            let at = cursor_byte.max(from).min(line.len());
            let scalar = scalar_at(line, line_no as u32, at, at, Quote::Plain, at);
            resolved.place = Place::Value(ValueSlot {
                scalar,
                list_item,
                flow: false,
            });
        }
        Some('|' | '>') => {}
        Some('[' | '{') if !looks_like_template(rest) => {
            flow_place(
                resolved,
                frames,
                line,
                line_no,
                start,
                end,
                cursor_byte,
                &[],
            );
        }
        Some(quote @ ('"' | '\'')) => {
            let kind = if quote == '"' {
                Quote::Double
            } else {
                Quote::Single
            };
            let scalar = scalar_at(line, line_no as u32, start, end, kind, cursor_byte);
            if scalar.inside {
                resolved.place = Place::Value(ValueSlot {
                    scalar,
                    list_item,
                    flow: false,
                });
            }
        }
        Some(_) => {
            let scalar = scalar_at(line, line_no as u32, start, end, Quote::Plain, cursor_byte);
            resolved.place = Place::Value(ValueSlot {
                scalar,
                list_item,
                flow: false,
            });
        }
    }
}

/// Cursor inside flow collection text `line[from..to]`.
#[allow(clippy::too_many_arguments)]
fn flow_place(
    resolved: &mut Resolved,
    frames: &mut Vec<Frame>,
    line: &str,
    line_no: usize,
    from: usize,
    to: usize,
    cursor_byte: usize,
    inherited: &[char],
) {
    let containers = flow_scan(line, from, to, inherited);
    // Containers are created in opening order, so the ones that hold the cursor are already nested.
    let chain: Vec<&FlowContainer> = containers
        .iter()
        .filter(|container| {
            (container.inherited || container.start < cursor_byte)
                && container.end.is_none_or(|end| cursor_byte <= end)
        })
        .collect();
    if chain.is_empty() {
        return;
    }
    let last = chain.len() - 1;
    for (position, container) in chain.iter().enumerate() {
        let current = container
            .items
            .iter()
            .find(|item| item.start <= cursor_byte && cursor_byte <= item.raw_end);
        if container.open == '{' {
            let keys = container
                .items
                .iter()
                .filter_map(|item| {
                    item.key.as_ref().map(|(name, _)| KeyEntry {
                        name: name.clone(),
                        line: line_no as u32,
                        value: None,
                        first_item: None,
                    })
                })
                .collect();
            let selected = current
                .and_then(|item| item.key.as_ref())
                .filter(|(_, colon)| cursor_byte > *colon)
                .map(|(name, _)| name.clone());
            frames.push(Frame::Map { keys, selected });
        } else {
            let via_key = frames.iter().rev().find_map(|frame| match frame {
                Frame::Map { selected, .. } => selected.clone(),
                Frame::Seq { .. } => None,
            });
            frames.push(Frame::Seq { via_key });
        }
        if position != last {
            continue;
        }
        let Some(item) = current else {
            return;
        };
        match &item.key {
            Some((_, colon)) if container.open == '{' => {
                if cursor_byte <= *colon {
                    resolved.place = Place::Key(KeySlot {
                        typed: line[item.start..cursor_byte.max(item.start)]
                            .trim_start()
                            .to_owned(),
                        line: line_no as u32,
                        start: col_at(line, item.start),
                        end: col_at(line, *colon),
                        has_colon: true,
                    });
                    return;
                }
                let mut value_start = colon + 1;
                while value_start < item.end && line.as_bytes()[value_start] == b' ' {
                    value_start += 1;
                }
                flow_scalar(
                    resolved,
                    line,
                    line_no,
                    value_start,
                    item.end,
                    cursor_byte,
                    false,
                );
            }
            _ => {
                if container.open == '{' {
                    resolved.place = Place::Key(KeySlot {
                        typed: line[item.start..cursor_byte.max(item.start)]
                            .trim_start()
                            .to_owned(),
                        line: line_no as u32,
                        start: col_at(line, item.start),
                        end: col_at(line, item.end),
                        has_colon: false,
                    });
                    return;
                }
                let mut value_start = item.start;
                while value_start < item.end && line.as_bytes()[value_start] == b' ' {
                    value_start += 1;
                }
                flow_scalar(
                    resolved,
                    line,
                    line_no,
                    value_start,
                    item.end,
                    cursor_byte,
                    true,
                );
            }
        }
    }
}

fn flow_scalar(
    resolved: &mut Resolved,
    line: &str,
    line_no: usize,
    start: usize,
    end: usize,
    cursor_byte: usize,
    list_item: bool,
) {
    let rest = line.get(start..end).unwrap_or("");
    let quote = match rest.chars().next() {
        Some('"') => Quote::Double,
        Some('\'') => Quote::Single,
        _ => Quote::Plain,
    };
    let at = if rest.is_empty() {
        cursor_byte.max(start).min(line.len())
    } else {
        start
    };
    let limit = if rest.is_empty() { at } else { end };
    let scalar = scalar_at(line, line_no as u32, at, limit, quote, cursor_byte);
    if quote != Quote::Plain && !scalar.inside {
        return;
    }
    resolved.place = Place::Value(ValueSlot {
        scalar,
        list_item,
        flow: true,
    });
}

// -------------------------------------------------------------------------------------------------
// Helpers for completion
// -------------------------------------------------------------------------------------------------

/// Values of the `options:` entries of a config field, read from the lines below `options_line`.
pub(crate) fn option_values(text: &str, options_line: u32) -> Vec<String> {
    let lines = split_lines(text);
    let Some(head) = lines.get(options_line as usize) else {
        return Vec::new();
    };
    let head_scan = scan_line(head);
    let threshold = head_scan
        .key
        .as_ref()
        .map_or(head_scan.indent, |key| key.name_start);
    let mut values = Vec::new();
    // Inline flow list on the key line itself.
    if let Some(key) = &head_scan.key {
        let inline = head.get(key.colon + 1..head_scan.end).unwrap_or("");
        collect_flow_values(inline, &mut values);
    }
    for line in lines.iter().skip(options_line as usize + 1) {
        let scan = scan_line(line);
        if scan.blank {
            continue;
        }
        let first_col = scan.dashes.first().copied().unwrap_or(scan.indent);
        if first_col < threshold || (first_col == threshold && scan.dashes.is_empty()) {
            break;
        }
        let content = line.get(scan.content_col..scan.end).unwrap_or("");
        if let Some(rest) = content.strip_prefix("value:") {
            values.push(decode_simple(rest));
        } else if content.starts_with('{') {
            collect_flow_values(content, &mut values);
        }
    }
    values.retain(|value| !value.is_empty());
    values
}

fn collect_flow_values(text: &str, values: &mut Vec<String>) {
    // `[ { value: a, label: A }, { value: b } ]` or `{ value: a }`
    let mut rest = text;
    while let Some(position) = rest.find("value:") {
        let after = &rest[position + "value:".len()..];
        let end = after.find([',', '}']).unwrap_or(after.len());
        values.push(decode_simple(&after[..end]));
        rest = &after[end..];
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// Splits `text|more` at the cursor marker.
    pub(crate) fn marked(source: &str) -> (String, Position) {
        let offset = source.find('|').expect("cursor marker");
        let text = format!("{}{}", &source[..offset], &source[offset + 1..]);
        let before = &source[..offset];
        let line = before.matches('\n').count() as u32;
        let tail = before.rsplit('\n').next().unwrap_or("");
        (
            text,
            Position {
                line,
                character: utf16_len(tail),
            },
        )
    }

    pub(crate) fn at(path: &str, source: &str) -> Resolved {
        let (text, position) = marked(source);
        resolve(Path::new(path), &text, position)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    fn value(resolved: &Resolved) -> &ValueSlot {
        match &resolved.place {
            Place::Value(slot) => slot,
            other => panic!("value slot expected, got {other:?}"),
        }
    }

    const OP: &str = "type: cdo\npatches:\n  - target: /Script/FactoryGame.FGItemDescriptor\n    properties:\n      - path: mStackSize\n        op: add\n";

    /// What the resolver reports for one cursor.
    struct Row {
        name: &'static str,
        source: &'static str,
        /// `value`, `key` or `none`.
        place: &'static str,
        /// Decoded scalar text before the cursor.
        typed: &'static str,
        quote: Quote,
        /// Closest mapping key.
        key: Option<&'static str>,
        /// Path below the operation entry.
        below: &'static [&'static str],
    }

    const HEAD: &str = "type: cdo\npatches:\n  - target: /Script/A.B\n    properties:\n      - path: mX\n        op: set\n";

    #[test]
    fn resolver_table() {
        let rows = [
            Row {
                name: "plain reference",
                source: "        value: cfg:|\n",
                place: "value",
                typed: "cfg:",
                quote: Quote::Plain,
                key: Some("value"),
                below: &["value"],
            },
            Row {
                name: "empty value",
                source: "        value: |\n",
                place: "value",
                typed: "",
                quote: Quote::Plain,
                key: Some("value"),
                below: &["value"],
            },
            Row {
                name: "no space after colon",
                source: "        value:|\n",
                place: "value",
                typed: "",
                quote: Quote::Plain,
                key: Some("value"),
                below: &["value"],
            },
            Row {
                name: "double quoted template",
                source: "        value: \"a {cfg:b|}\"\n",
                place: "value",
                typed: "a {cfg:b",
                quote: Quote::Double,
                key: Some("value"),
                below: &["value"],
            },
            Row {
                name: "single quoted template",
                source: "        value: '{item:|'\n",
                place: "value",
                typed: "{item:",
                quote: Quote::Single,
                key: Some("value"),
                below: &["value"],
            },
            Row {
                name: "plain template inside text",
                source: "        value: Smelts {cfg:|\n",
                place: "value",
                typed: "Smelts {cfg:",
                quote: Quote::Plain,
                key: Some("value"),
                below: &["value"],
            },
            Row {
                name: "flow sequence",
                source: "        value: [a, cfg:|]\n",
                place: "value",
                typed: "cfg:",
                quote: Quote::Plain,
                key: Some("value"),
                below: &["value", "#"],
            },
            Row {
                name: "flow sequence after comma",
                source: "        value: [a, |]\n",
                place: "value",
                typed: "",
                quote: Quote::Plain,
                key: Some("value"),
                below: &["value", "#"],
            },
            Row {
                name: "flow map member",
                source: "        value: { A: 1, B: cfg:| }\n",
                place: "value",
                typed: "cfg:",
                quote: Quote::Plain,
                key: Some("B"),
                below: &["value", "B"],
            },
            Row {
                name: "flow map quoted member",
                source: "        value: { A: \"x, {y|\" }\n",
                place: "value",
                typed: "x, {y",
                quote: Quote::Double,
                key: Some("A"),
                below: &["value", "A"],
            },
            Row {
                name: "nested flow",
                source: "        value: [{ A: cfg:| }]\n",
                place: "value",
                typed: "cfg:",
                quote: Quote::Plain,
                key: Some("A"),
                below: &["value", "#", "A"],
            },
            Row {
                name: "block list item",
                source: "        value:\n          - cfg:|\n",
                place: "value",
                typed: "cfg:",
                quote: Quote::Plain,
                key: Some("value"),
                below: &["value", "#"],
            },
            Row {
                name: "block struct member",
                source: "        value:\n          - A: 1\n            B: cfg:|\n",
                place: "value",
                typed: "cfg:",
                quote: Quote::Plain,
                key: Some("B"),
                below: &["value", "#", "B"],
            },
            Row {
                name: "key being typed",
                source: "        va|\n",
                place: "key",
                typed: "",
                quote: Quote::Plain,
                key: None,
                below: &[],
            },
            Row {
                name: "blank line",
                source: "        |\n",
                place: "key",
                typed: "",
                quote: Quote::Plain,
                key: None,
                below: &[],
            },
            Row {
                name: "after the closing quote",
                source: "        value: \"a\"|\n",
                place: "none",
                typed: "",
                quote: Quote::Plain,
                key: None,
                below: &[],
            },
            Row {
                name: "comment",
                source: "        value: 1 # cfg:|\n",
                place: "none",
                typed: "",
                quote: Quote::Plain,
                key: None,
                below: &[],
            },
            Row {
                name: "path value",
                source: "        path: mFo|\n",
                place: "value",
                typed: "mFo",
                quote: Quote::Plain,
                key: Some("path"),
                below: &["path"],
            },
        ];
        for row in rows {
            let resolved = at("a.cdo.yml", &format!("{HEAD}{}", row.source));
            let (place, typed, quote) = match &resolved.place {
                Place::Value(slot) => ("value", slot.scalar.before_cursor(), slot.scalar.quote),
                Place::Key(_) => ("key", String::new(), Quote::Plain),
                Place::None => ("none", String::new(), Quote::Plain),
            };
            assert_eq!(place, row.place, "{}", row.name);
            assert_eq!(typed, row.typed, "{}", row.name);
            assert_eq!(quote, row.quote, "{}", row.name);
            if row.place == "value" {
                assert_eq!(resolved.key(), row.key, "{}", row.name);
                let op = resolved
                    .op_entry()
                    .unwrap_or_else(|| panic!("{}: no op entry", row.name));
                assert_eq!(op.below, row.below, "{}", row.name);
            }
        }
    }

    #[test]
    fn typed_reference_with_colon_at_line_end() {
        let resolved = at("a.cdo.yml", &format!("{OP}        value: cfg:|\n"));
        let slot = value(&resolved);
        assert_eq!(slot.scalar.before_cursor(), "cfg:");
        assert_eq!(slot.scalar.quote, Quote::Plain);
        assert_eq!(resolved.key(), Some("value"));
        let op = resolved.op_entry().expect("op entry");
        assert_eq!(op.op_name(), "add");
        assert_eq!(op.key_value("path"), Some("mStackSize"));
        assert_eq!(op.below, vec!["value".to_owned()]);
        let patch = resolved.patch_entry().expect("patch entry");
        assert_eq!(patch.list_key.as_deref(), Some("patches"));
        assert_eq!(
            patch.class_text().as_deref(),
            Some("/Script/FactoryGame.FGItemDescriptor")
        );
        assert_eq!(resolved.doc.kind.as_deref(), Some("cdo"));
    }

    #[test]
    fn empty_value_after_colon_and_space() {
        let resolved = at("a.cdo.yml", &format!("{OP}        value: |\n"));
        let slot = value(&resolved);
        assert_eq!(slot.scalar.value, "");
        assert_eq!(slot.scalar.quote, Quote::Plain);
        assert_eq!(resolved.key(), Some("value"));
    }

    #[test]
    fn quoted_scalar_decodes_and_maps_columns() {
        let resolved = at(
            "a.cdo.yml",
            &format!("{OP}        value: \"a {{cfg:x|}} b\"\n"),
        );
        let slot = value(&resolved);
        assert_eq!(slot.scalar.quote, Quote::Double);
        assert_eq!(slot.scalar.value, "a {cfg:x} b");
        assert_eq!(slot.scalar.cursor, 8);
        assert!(slot.scalar.closed);
        // Column of decoded offset 3 (the `c` of cfg) is raw start + opening quote + 3.
        assert_eq!(slot.scalar.raw_col(3), slot.scalar.raw_start + 1 + 3);
    }

    #[test]
    fn double_quoted_escapes_map_back() {
        let resolved = at(
            "a.cdo.yml",
            &format!("{OP}        value: \"\\\\{{cfg:|\"\n"),
        );
        let slot = value(&resolved);
        assert_eq!(slot.scalar.value, "\\{cfg:");
        // `\\` takes two raw columns, `{` is at decoded offset 1.
        assert_eq!(slot.scalar.raw_col(1) - slot.scalar.raw_col(0), 2);
        assert_eq!(slot.scalar.encode("a\"b\\"), "a\\\"b\\\\");
    }

    #[test]
    fn single_quoted_scalar_with_doubled_quote() {
        let resolved = at(
            "a.cdo.yml",
            &format!("{OP}        value: 'it''s {{cfg:|}}'\n"),
        );
        let slot = value(&resolved);
        assert_eq!(slot.scalar.quote, Quote::Single);
        assert_eq!(slot.scalar.value, "it's {cfg:}");
        assert_eq!(slot.scalar.literal_quote(), '"');
    }

    #[test]
    fn unterminated_quote_while_typing() {
        let resolved = at("a.cdo.yml", &format!("{OP}        value: \"{{it|\n"));
        let slot = value(&resolved);
        assert!(!slot.scalar.closed);
        assert_eq!(slot.scalar.value, "{it");
        assert_eq!(slot.scalar.cursor, 3);
    }

    #[test]
    fn auto_closed_quote_pair() {
        let resolved = at("a.cdo.yml", &format!("{OP}        value: \"|\"\n"));
        let slot = value(&resolved);
        assert!(slot.scalar.closed);
        assert_eq!(slot.scalar.value, "");
        assert!(slot.scalar.inside);
    }

    #[test]
    fn cursor_after_closing_quote_is_outside() {
        let resolved = at("a.cdo.yml", &format!("{OP}        value: \"abc\"|\n"));
        assert!(matches!(resolved.place, Place::None));
    }

    #[test]
    fn comment_is_not_a_place() {
        let resolved = at("a.cdo.yml", &format!("{OP}        value: 1 # cfg:|\n"));
        assert!(resolved.in_comment);
        assert!(matches!(resolved.place, Place::None));
    }

    #[test]
    fn key_slot_with_typed_prefix() {
        let resolved = at("a.cdo.yml", &format!("{OP}        va|\n"));
        let Place::Key(slot) = &resolved.place else {
            panic!("expected key slot, got {:?}", resolved.place);
        };
        assert_eq!(slot.typed, "va");
        assert!(resolved.op_entry().is_some());
        assert!(resolved.siblings().iter().any(|key| key.name == "op"));
    }

    #[test]
    fn blank_line_is_a_key_slot_of_the_mapping_by_indentation() {
        let resolved = at("a.cdo.yml", &format!("{OP}        |\n"));
        let Place::Key(slot) = &resolved.place else {
            panic!("expected key slot");
        };
        assert_eq!(slot.typed, "");
        assert!(resolved.siblings().iter().any(|key| key.name == "path"));
    }

    #[test]
    fn dash_list_item_under_value_key() {
        let resolved = at(
            "a.cdo.yml",
            &format!("{OP}        value:\n          - cfg:ba|\n"),
        );
        let slot = value(&resolved);
        assert!(slot.list_item);
        assert_eq!(resolved.key(), Some("value"));
        assert_eq!(slot.scalar.before_cursor(), "cfg:ba");
    }

    #[test]
    fn list_item_reference_ending_with_colon_is_not_a_key() {
        let resolved = at(
            "a.cdo.yml",
            &format!("{OP}        value:\n          - cfg:|\n"),
        );
        let slot = value(&resolved);
        assert_eq!(slot.scalar.value, "cfg:");
        assert!(slot.list_item);
    }

    #[test]
    fn nested_struct_member_below_value() {
        let resolved = at(
            "a.cdo.yml",
            &format!("{OP}        value:\n          - ItemClass: /Game/A.A_C\n            Amount: cfg:|\n"),
        );
        let op = resolved.op_entry().expect("op entry");
        assert_eq!(
            op.below,
            vec!["value".to_owned(), "#".to_owned(), "Amount".to_owned()]
        );
        assert_eq!(resolved.key(), Some("Amount"));
    }

    #[test]
    fn flow_map_value_member() {
        let resolved = at(
            "a.cdo.yml",
            &format!("{OP}        value: {{ ItemClass: /Game/A.A_C, Amount: cfg:| }}\n"),
        );
        let slot = value(&resolved);
        assert!(slot.flow);
        assert_eq!(slot.scalar.before_cursor(), "cfg:");
        let op = resolved.op_entry().expect("op entry");
        assert_eq!(op.below, vec!["value".to_owned(), "Amount".to_owned()]);
    }

    #[test]
    fn flow_sequence_item() {
        let resolved = at(
            "a.cdo.yml",
            &format!("{OP}        value: [cfg:a, cfg:b|]\n"),
        );
        let slot = value(&resolved);
        assert!(slot.flow && slot.list_item);
        assert_eq!(slot.scalar.value, "cfg:b");
        assert_eq!(resolved.key(), Some("value"));
    }

    #[test]
    fn flow_quoted_item_with_comma_inside() {
        let resolved = at(
            "a.cdo.yml",
            &format!("{OP}        value: [\"a, {{cfg:|\", 2]\n"),
        );
        let slot = value(&resolved);
        assert_eq!(slot.scalar.value, "a, {cfg:");
        assert_eq!(slot.scalar.quote, Quote::Double);
    }

    #[test]
    fn whole_op_in_flow_map_list_item() {
        let resolved = at(
            "a.cdo.yml",
            "type: cdo\npatches:\n  - target: /Script/A.B\n    properties:\n      - { path: mX, op: add, value: cfg:| }\n",
        );
        let op = resolved.op_entry().expect("flow op entry");
        assert_eq!(op.below, vec!["value".to_owned()]);
        assert!(op.keys.iter().any(|key| key.name == "path"));
        assert_eq!(
            resolved
                .patch_entry()
                .and_then(|patch| patch.class_text())
                .as_deref(),
            Some("/Script/A.B")
        );
    }

    #[test]
    fn unquoted_template_start_is_a_plain_scalar() {
        let resolved = at("a.cdo.yml", &format!("{OP}        value: {{cfg:ba|\n"));
        let slot = value(&resolved);
        assert!(!slot.flow);
        assert_eq!(slot.scalar.value, "{cfg:ba");
    }

    #[test]
    fn multi_line_flow_continuation() {
        let resolved = at(
            "a.cdo.yml",
            &format!("{OP}        value:\n          {{\n            ItemClass: /Game/A.A_C,\n            Amount: cfg:|,\n          }}\n"),
        );
        let slot = value(&resolved);
        assert_eq!(slot.scalar.value, "cfg:");
        assert_eq!(resolved.key(), Some("Amount"));
    }

    #[test]
    fn block_scalar_content_is_not_a_place() {
        let resolved = at(
            "a.item.yml",
            "type: item\nitems:\n  - id: A\n    description: >\n      cfg:|\n",
        );
        assert!(matches!(resolved.place, Place::None));
    }

    #[test]
    fn document_region_and_type_in_multi_document_files() {
        let resolved = at(
            "multi.yml",
            "type: item\nitems:\n  - id: A\n---\ntype: recipe\nrecipes:\n  - id: B\n    parent: |\n",
        );
        assert_eq!(resolved.doc.index, 1);
        assert_eq!(resolved.doc.kind.as_deref(), Some("recipe"));
        assert!(resolved.doc.multi);
        assert_eq!(resolved.doc.first_line, 4);
    }

    #[test]
    fn type_is_inferred_from_the_file_name() {
        let resolved = at(
            "pack/tuning.cdo.yml",
            "patches:\n  - target: /Script/A.B\n    properties:\n      - path: |\n",
        );
        assert_eq!(resolved.doc.kind.as_deref(), Some("cdo"));
        let config = at("pack/balance.config.yml", "fields:\n  - id: |\n");
        assert_eq!(config.doc.kind.as_deref(), Some("config"));
    }

    #[test]
    fn mam_is_research() {
        let resolved = at("x.yml", "type: mam\nresearch:\n  - id: |\n");
        assert_eq!(resolved.doc.kind.as_deref(), Some("research"));
    }

    #[test]
    fn generated_entry_uses_parent_then_default_base() {
        let resolved = at(
            "a.item.yml",
            "type: item\nitems:\n  - id: Thing\n    parent: /Script/FactoryGame.FGResourceDescriptor\n    properties:\n      - path: |\n",
        );
        let patch = resolved.patch_entry().expect("patch");
        assert_eq!(patch.list_key.as_deref(), Some("items"));
        assert_eq!(
            patch.class_text().as_deref(),
            Some("/Script/FactoryGame.FGResourceDescriptor")
        );
        let bare = at(
            "a.item.yml",
            "type: item\nitems:\n  - id: Thing\n    properties:\n      - path: |\n",
        );
        let patch = bare.patch_entry().expect("patch");
        assert_eq!(patch.class_text(), None);
        assert_eq!(
            patch.default_base(),
            Some("/Script/FactoryGame.FGItemDescriptor")
        );
    }

    #[test]
    fn target_sequence_uses_its_first_element() {
        let resolved = at(
            "a.cdo.yml",
            "type: cdo\npatches:\n  - target:\n      - /Script/A.One\n      - /Script/A.Two\n    applyToSpawnedActors: true\n    properties:\n      - path: |\n",
        );
        let patch = resolved.patch_entry().expect("patch");
        assert_eq!(patch.class_text().as_deref(), Some("/Script/A.One"));
        assert!(patch.apply_to_spawned_actors());
    }

    #[test]
    fn of_class_and_all_assets_are_tried_after_target() {
        let resolved = at(
            "a.cdo.yml",
            "type: cdo\npatches:\n  - ofClass: /Script/A.Of\n    properties:\n      - path: |\n",
        );
        assert_eq!(
            resolved
                .patch_entry()
                .and_then(|patch| patch.class_text())
                .as_deref(),
            Some("/Script/A.Of")
        );
        let all = at(
            "a.cdo.yml",
            "type: cdo\npatches:\n  - allAssetsOfClass: /Script/A.All\n    properties:\n      - path: |\n",
        );
        assert_eq!(
            all.patch_entry()
                .and_then(|patch| patch.class_text())
                .as_deref(),
            Some("/Script/A.All")
        );
    }

    #[test]
    fn compact_sequence_style_without_extra_indentation() {
        let resolved = at(
            "a.cdo.yml",
            "type: cdo\npatches:\n- target: /Script/A.B\n  properties:\n  - path: mX\n    value: cfg:|\n",
        );
        let op = resolved.op_entry().expect("op entry");
        assert_eq!(op.key_value("path"), Some("mX"));
        assert_eq!(
            resolved
                .patch_entry()
                .and_then(|patch| patch.class_text())
                .as_deref(),
            Some("/Script/A.B")
        );
    }

    #[test]
    fn conditions_are_detected_for_config_expressions() {
        let resolved = at(
            "a.cdo.yml",
            "type: cdo\nconditions:\n  config: balance.enabled && |\npatches: []\n",
        );
        assert!(resolved.in_conditions());
        assert_eq!(resolved.key(), Some("config"));
        let element = at(
            "a.cdo.yml",
            &format!(
                "{OP}        value: 1\n        conditions:\n          config: \"x == 'a' &&| \"\n"
            ),
        );
        assert!(element.in_conditions());
    }

    #[test]
    fn config_field_entry_is_recognised() {
        let resolved = at(
            "a.config.yml",
            "type: config\nfields:\n  - id: a.b\n    type: number\n    valueType: |\n",
        );
        let field = resolved.field_entry().expect("field");
        assert_eq!(field.key_value("type"), Some("number"));
        assert_eq!(resolved.key(), Some("valueType"));
    }

    #[test]
    fn dash_only_line_in_fields_list() {
        let resolved = at("a.config.yml", "type: config\nfields:\n  - |\n");
        assert!(resolved.in_fields_list());
    }

    #[test]
    fn unicode_columns_are_utf16() {
        let resolved = at(
            "a.cdo.yml",
            &format!("{OP}        value: \"\u{1F600} {{cfg:|\"\n"),
        );
        let slot = value(&resolved);
        assert_eq!(slot.scalar.value, "\u{1F600} {cfg:");
        assert_eq!(slot.scalar.cursor, 8);
        // The emoji takes two UTF-16 units; the `{` sits at raw column start + quote + 3.
        assert_eq!(slot.scalar.raw_col(3), slot.scalar.raw_start + 1 + 3);
    }

    #[test]
    fn option_values_are_collected_from_block_and_flow_lists() {
        let text = "type: config\nfields:\n  - id: a\n    type: select\n    options:\n      - value: easy\n        label: Easy\n      - { value: 'hard', label: Hard }\n    default: easy\n";
        assert_eq!(
            option_values(text, 4),
            vec!["easy".to_owned(), "hard".to_owned()]
        );
    }

    #[test]
    fn arbitrary_text_never_panics() {
        let pieces = [
            "a",
            ": ",
            ":",
            "- ",
            "\"",
            "'",
            "{",
            "}",
            "[",
            "]",
            ",",
            "\n",
            "  ",
            "#",
            "cfg:",
            "{cfg:",
            "|",
            ">",
            "---\n",
            "\u{1F600}",
            "\\",
            "value",
            "properties:",
            "path: ",
        ];
        let mut state = 7u64;
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as usize
        };
        for _ in 0..1500 {
            let length = next() % 14;
            let text: String = (0..length).map(|_| pieces[next() % pieces.len()]).collect();
            let lines = split_lines(&text);
            for (index, line) in lines.iter().enumerate() {
                let width = utf16_len(line);
                for column in 0..=width {
                    let _ = resolve(
                        Path::new("fuzz.cdo.yml"),
                        &text,
                        Position {
                            line: index as u32,
                            character: column,
                        },
                    );
                }
            }
        }
    }
}
