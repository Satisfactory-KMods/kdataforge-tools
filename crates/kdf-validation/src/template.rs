//! Template tokens `{prefix:body}` inside string scalars.
//!
//! * Pre-pass prefixes (`cfg`, `session`, `ags`, `option`) are replaced before the stages run.
//! * Apply-time prefixes are reflection targets (`class`, `item`, `resource`, `building`, `recipe`,
//!   `schematic`, `research`, `category`, `dataasset`, `asset`); body is `<targetRef>:<member>`.
//!
//! Escapes: `\{` directly before `{<registered prefix>:` is an escape; each phase strips only its
//! own escapes. Unknown `{...}` stays literal. Nested tokens are allowed up to [`MAX_DEPTH`].
//! All spans/offsets are UTF-16 code units relative to the scanned string.

use crate::expression::Span;
use serde::Serialize;
use serde_json::{json, Map, Value};

pub const PREPASS_PREFIXES: [&str; 4] = ["cfg", "session", "ags", "option"];
pub const APPLY_PREFIXES: [&str; 10] = [
    "class",
    "item",
    "resource",
    "building",
    "recipe",
    "schematic",
    "research",
    "category",
    "dataasset",
    "asset",
];
/// Maximum nesting of tokens (outermost = depth 1).
pub const MAX_DEPTH: u8 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum Phase {
    PrePass,
    Apply,
}

/// Which prefixes a scan recognises. `All` is used for validation and completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum ScanPhase {
    PrePass,
    Apply,
    All,
}

impl ScanPhase {
    pub fn accepts(self, prefix: &str) -> bool {
        match (self, phase_of(prefix)) {
            (_, None) => false,
            (ScanPhase::All, Some(_)) => true,
            (ScanPhase::PrePass, Some(Phase::PrePass)) => true,
            (ScanPhase::Apply, Some(Phase::Apply)) => true,
            _ => false,
        }
    }
}

/// Phase of a built-in prefix, `None` for anything else (third-party prefixes are not known here).
pub fn phase_of(prefix: &str) -> Option<Phase> {
    if PREPASS_PREFIXES.contains(&prefix) {
        Some(Phase::PrePass)
    } else if APPLY_PREFIXES.contains(&prefix) {
        Some(Phase::Apply)
    } else {
        None
    }
}

/// Class every target of this provider must derive from (`None` = any class / any asset).
pub fn base_class(prefix: &str) -> Option<&'static str> {
    match prefix {
        "item" => Some("/Script/FactoryGame.FGItemDescriptor"),
        "resource" => Some("/Script/FactoryGame.FGResourceDescriptor"),
        "building" => Some("/Script/FactoryGame.FGBuildable"),
        "recipe" => Some("/Script/FactoryGame.FGRecipe"),
        "schematic" => Some("/Script/FactoryGame.FGSchematic"),
        "research" => Some("/Script/FactoryGame.FGResearchTree"),
        "category" => Some("/Script/FactoryGame.FGCategory"),
        "dataasset" => Some("/Script/Engine.DataAsset"),
        _ => None,
    }
}

/// `dataasset` and `asset` target object instances; every other apply prefix targets a class (CDO).
pub fn is_object_target(prefix: &str) -> bool {
    matches!(prefix, "dataasset" | "asset")
}

/// Short human description used by completion and hover.
pub fn describe(prefix: &str) -> &'static str {
    match prefix {
        "cfg" => "Pack configuration value ({cfg:id} or {cfg:Pack:id})",
        "session" => "Session setting (SML session setting, falls back to advanced game settings)",
        "ags" => "Advanced game setting",
        "option" => "Local game option",
        "class" => "Any class: property or function (function libraries allowed)",
        "item" => "Item descriptor class",
        "resource" => "Resource descriptor class",
        "building" => "Buildable class",
        "recipe" => "Recipe class",
        "schematic" => "Schematic class",
        "research" => "Research tree class",
        "category" => "Category class",
        "dataasset" => "Data asset instance (asset path or KDataForge dataasset id)",
        "asset" => "Any asset object by path",
        _ => "",
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TemplateToken {
    pub prefix: String,
    /// Text between `prefix:` and the closing brace (nested tokens included, unexpanded).
    pub body: String,
    /// Whole token including braces.
    pub span: Span,
    pub prefix_span: Span,
    pub body_span: Span,
    /// 1 = outermost.
    pub depth: u8,
    /// Tokens nested in the body (same phase filter as the scan).
    pub children: Vec<TemplateToken>,
}

impl TemplateToken {
    pub fn phase(&self) -> Option<Phase> {
        phase_of(&self.prefix)
    }
    /// Depth-first walk including `self`.
    pub fn walk<'a>(&'a self, visit: &mut dyn FnMut(&'a TemplateToken)) {
        visit(self);
        for child in &self.children {
            child.walk(visit);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum TemplateSegment {
    /// Literal text with this phase's escapes already removed.
    Text {
        text: String,
        span: Span,
    },
    Token(TemplateToken),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum ScanIssueKind {
    /// `{prefix:` without a matching `}`; the rest of the string stays literal.
    Unbalanced,
    /// Token deeper than [`MAX_DEPTH`].
    NestingDepth,
    /// `{prefix:` where `prefix` is within one edit of a built-in prefix (`suggestion`).
    UnknownPrefix { prefix: String, suggestion: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScanIssue {
    pub kind: ScanIssueKind,
    pub span: Span,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Scan {
    pub segments: Vec<TemplateSegment>,
    pub issues: Vec<ScanIssue>,
}

impl Scan {
    /// Tokens (all depths) in source order.
    pub fn tokens(&self) -> Vec<&TemplateToken> {
        let mut out = Vec::new();
        for segment in &self.segments {
            if let TemplateSegment::Token(token) = segment {
                token.walk(&mut |token| out.push(token));
            }
        }
        out
    }
    /// Vector form: `["text", s]` / `["token", prefix, body]`.
    pub fn to_vector(&self) -> Value {
        Value::Array(
            self.segments
                .iter()
                .map(|segment| match segment {
                    TemplateSegment::Text { text, .. } => json!(["text", text]),
                    TemplateSegment::Token(token) => json!(["token", token.prefix, token.body]),
                })
                .collect(),
        )
    }
}

struct Source {
    chars: Vec<char>,
    offsets: Vec<u32>,
}

impl Source {
    fn new(src: &str) -> Self {
        let chars = src.chars().collect::<Vec<_>>();
        let mut offsets = Vec::with_capacity(chars.len() + 1);
        let mut total = 0u32;
        for ch in &chars {
            offsets.push(total);
            total += ch.len_utf16() as u32;
        }
        offsets.push(total);
        Self { chars, offsets }
    }
    fn span(&self, start: usize, end: usize) -> Span {
        Span::new(self.offsets[start], self.offsets[end])
    }
    fn text(&self, start: usize, end: usize) -> String {
        self.chars[start..end].iter().collect()
    }
    /// Index of the character starting at (or covering) UTF-16 `offset`.
    fn index_at(&self, offset: u32) -> usize {
        match self.offsets.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        }
    }
}

fn is_prefix_start(ch: char) -> bool {
    ch.is_ascii_alphabetic()
}

fn is_prefix_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// Reads `[A-Za-z][A-Za-z0-9_]*:` starting at `start`; returns the prefix and the colon index.
fn prefix_at(chars: &[char], start: usize, hi: usize) -> Option<(String, usize)> {
    if start >= hi || !is_prefix_start(chars[start]) {
        return None;
    }
    let mut end = start;
    while end < hi && is_prefix_char(chars[end]) {
        end += 1;
    }
    if end < hi && chars[end] == ':' {
        Some((chars[start..end].iter().collect(), end))
    } else {
        None
    }
}

/// Index of the closing brace of the token whose `:` is at `colon`. Brackets nest, quoted spans are
/// opaque (backslash escapes the next character).
fn find_close(chars: &[char], colon: usize, hi: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut index = colon + 1;
    while index < hi {
        let ch = chars[index];
        if let Some(open) = quote {
            if ch == '\\' {
                index += 2;
                continue;
            }
            if ch == open {
                quote = None;
            }
            index += 1;
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            '}' => {
                if depth == 0 {
                    return Some(index);
                }
                depth -= 1;
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn within_one_edit(a: &str, b: &str) -> bool {
    let a = a.chars().collect::<Vec<_>>();
    let b = b.chars().collect::<Vec<_>>();
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (short, long) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    let mut i = 0;
    let mut j = 0;
    let mut edits = 0;
    while i < short.len() && j < long.len() {
        if short[i] == long[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        if short.len() == long.len() {
            i += 1;
        }
        j += 1;
    }
    edits + (long.len() - j) + (short.len() - i) <= 1
}

fn near_miss(prefix: &str) -> Option<&'static str> {
    let lower = prefix.to_ascii_lowercase();
    PREPASS_PREFIXES
        .iter()
        .chain(APPLY_PREFIXES.iter())
        .copied()
        .find(|known| within_one_edit(&lower, known))
}

struct Scanner<'a> {
    source: &'a Source,
    phase: ScanPhase,
    issues: Vec<ScanIssue>,
}

impl Scanner<'_> {
    fn scan_region(&mut self, lo: usize, hi: usize, depth: u8) -> Vec<TemplateSegment> {
        let source = self.source;
        let chars = &source.chars;
        let mut segments = Vec::new();
        let mut text = String::new();
        let mut text_start = lo;
        let mut index = lo;
        macro_rules! flush {
            ($end:expr) => {
                if !text.is_empty() {
                    segments.push(TemplateSegment::Text {
                        text: std::mem::take(&mut text),
                        span: source.span(text_start, $end),
                    });
                }
            };
        }
        while index < hi {
            let ch = chars[index];
            if ch == '\\' && index + 1 < hi && chars[index + 1] == '{' {
                if let Some((prefix, _)) = prefix_at(chars, index + 2, hi) {
                    if self.phase.accepts(&prefix) {
                        if text.is_empty() {
                            text_start = index;
                        }
                        text.push('{');
                        index += 2;
                        continue;
                    }
                }
            }
            if ch == '{' {
                if let Some((prefix, colon)) = prefix_at(chars, index + 1, hi) {
                    if self.phase.accepts(&prefix) {
                        let Some(close) = find_close(chars, colon, hi) else {
                            self.issues.push(ScanIssue {
                                kind: ScanIssueKind::Unbalanced,
                                span: source.span(index, hi),
                            });
                            if text.is_empty() {
                                text_start = index;
                            }
                            text.extend(&chars[index..hi]);
                            index = hi;
                            break;
                        };
                        flush!(index);
                        let token = self.read_token(index, colon, close, prefix, depth);
                        segments.push(TemplateSegment::Token(token));
                        index = close + 1;
                        text_start = index;
                        continue;
                    }
                    if phase_of(&prefix).is_none() {
                        if let Some(suggestion) = near_miss(&prefix) {
                            self.issues.push(ScanIssue {
                                kind: ScanIssueKind::UnknownPrefix {
                                    prefix: prefix.clone(),
                                    suggestion: suggestion.to_owned(),
                                },
                                span: source.span(index + 1, colon),
                            });
                        }
                    }
                }
            }
            if text.is_empty() {
                text_start = index;
            }
            text.push(ch);
            index += 1;
        }
        flush!(index.min(hi));
        segments
    }

    fn read_token(
        &mut self,
        open: usize,
        colon: usize,
        close: usize,
        prefix: String,
        depth: u8,
    ) -> TemplateToken {
        let span = self.source.span(open, close + 1);
        let mut children = Vec::new();
        if depth > MAX_DEPTH {
            self.issues.push(ScanIssue {
                kind: ScanIssueKind::NestingDepth,
                span,
            });
        } else {
            for segment in self.scan_region(colon + 1, close, depth + 1) {
                if let TemplateSegment::Token(token) = segment {
                    children.push(token);
                }
            }
        }
        TemplateToken {
            body: self.source.text(colon + 1, close),
            prefix_span: self.source.span(open + 1, colon),
            body_span: self.source.span(colon + 1, close),
            prefix,
            span,
            depth,
            children,
        }
    }
}

/// Splits `src` into literal text and tokens of the requested phase.
pub fn scan(src: &str, phase: ScanPhase) -> Scan {
    scan_at(src, phase, 1)
}

/// [`scan`] for text that sits at nesting `depth` (1 = outermost) of a larger token.
fn scan_at(src: &str, phase: ScanPhase, depth: u8) -> Scan {
    if !src.contains('{') {
        return Scan {
            segments: if src.is_empty() {
                Vec::new()
            } else {
                vec![TemplateSegment::Text {
                    text: src.to_owned(),
                    span: Span::new(0, src.encode_utf16().count() as u32),
                }]
            },
            issues: Vec::new(),
        };
    }
    let source = Source::new(src);
    let mut scanner = Scanner {
        source: &source,
        phase,
        issues: Vec::new(),
    };
    let segments = scanner.scan_region(0, source.chars.len(), depth);
    let issues = scanner.issues;
    Scan { segments, issues }
}

/// True when `src` may contain a token (cheap pre-check before [`scan`]).
pub fn may_contain_tokens(src: &str) -> bool {
    src.contains('{')
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Expansion {
    pub output: String,
    /// Number of tokens replaced.
    pub replaced: usize,
    /// Scan problems (`Unbalanced` leaves the rest literal).
    pub issues: Vec<ScanIssue>,
}

/// Pre-pass expansion: every pre-pass token (also those inside apply-time bodies) is replaced by
/// `resolve(prefix, body)`; inner tokens resolve first; unresolved tokens and substituted values are
/// never re-scanned. Escapes of the pre-pass prefixes are stripped, apply-time escapes stay.
pub fn expand_prepass(
    src: &str,
    resolve: &mut dyn FnMut(&str, &str) -> Option<String>,
) -> Expansion {
    expand_at(src, resolve, 1)
}

/// Expansion of text at nesting `depth`: tokens deeper than [`MAX_DEPTH`] keep their body literal
/// (the scan reports them as `NestingDepth`), so the recursion is bounded.
fn expand_at(
    src: &str,
    resolve: &mut dyn FnMut(&str, &str) -> Option<String>,
    depth: u8,
) -> Expansion {
    let scan = scan_at(src, ScanPhase::PrePass, depth);
    let mut out = Expansion {
        issues: scan.issues.clone(),
        ..Expansion::default()
    };
    for segment in &scan.segments {
        match segment {
            TemplateSegment::Text { text, .. } => out.output.push_str(text),
            TemplateSegment::Token(token) => {
                let body = if token.children.is_empty() {
                    token.body.clone()
                } else {
                    let inner = expand_at(&token.body, resolve, depth.saturating_add(1));
                    out.replaced += inner.replaced;
                    inner.output
                };
                match resolve(&token.prefix, &body) {
                    Some(value) => {
                        out.replaced += 1;
                        out.output.push_str(&value);
                    }
                    None => {
                        out.output.push('{');
                        out.output.push_str(&token.prefix);
                        out.output.push(':');
                        out.output.push_str(&body);
                        out.output.push('}');
                    }
                }
            }
        }
    }
    out
}

/// Splits an apply-time body into `(targetRef, member, member_offset)`. The target never contains
/// `:`; the member starts after the first colon. `member_offset` is in UTF-16 units of `body`.
pub fn split_body(body: &str) -> Option<(&str, &str, u32)> {
    let index = body.find(':')?;
    let member = &body[index + 1..];
    let offset = body[..=index].encode_utf16().count() as u32;
    Some((&body[..index], member, offset))
}

// ---------------------------------------------------------------------------------------------
// Member grammar
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum StructFields {
    Positional(Vec<Arg>),
    Named(Vec<(String, Arg)>),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum ArgKind {
    Number(f64),
    Str(String),
    Bool(bool),
    /// Bare name such as `EResourceForm::RF_SOLID`.
    Ident(String),
    Struct {
        type_name: Option<String>,
        fields: StructFields,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Arg {
    pub kind: ArgKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Member {
    /// Bare name: a property or a function called without parentheses.
    Ident { name: String, span: Span },
    /// Property path with `.field` / `[index]` accessors.
    Path { path: String, span: Span },
    Call {
        name: String,
        name_span: Span,
        args: Vec<Arg>,
        /// From the function name to the closing parenthesis.
        span: Span,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum MemberErrorKind {
    Empty,
    Unbalanced,
    ExpectedArgument,
    Malformed,
}

impl MemberErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MemberErrorKind::Empty => "empty",
            MemberErrorKind::Unbalanced => "unbalanced",
            MemberErrorKind::ExpectedArgument => "expected-argument",
            MemberErrorKind::Malformed => "malformed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemberError {
    pub kind: MemberErrorKind,
    pub at: u32,
    pub message: String,
}

impl Arg {
    pub fn to_vector(&self) -> Value {
        match &self.kind {
            ArgKind::Number(number) => json!(["num", crate::expression::json_number(*number)]),
            ArgKind::Str(text) => json!(["str", text]),
            ArgKind::Bool(flag) => json!(["bool", flag]),
            ArgKind::Ident(name) => json!(["ident", name]),
            ArgKind::Struct { type_name, fields } => {
                let body = match fields {
                    StructFields::Positional(items) => {
                        json!([
                            "positional",
                            items.iter().map(Arg::to_vector).collect::<Vec<_>>()
                        ])
                    }
                    StructFields::Named(items) => {
                        let mut map = Map::new();
                        for (name, arg) in items {
                            map.insert(name.clone(), arg.to_vector());
                        }
                        json!(["named", Value::Object(map)])
                    }
                };
                json!(["struct", type_name, body])
            }
        }
    }
}

impl Member {
    pub fn to_vector(&self) -> Value {
        match self {
            Member::Ident { name, .. } => json!({"kind": "ident", "name": name}),
            Member::Path { path, .. } => json!({"kind": "path", "path": path}),
            Member::Call { name, args, .. } => json!({
                "kind": "call",
                "name": name,
                "args": args.iter().map(Arg::to_vector).collect::<Vec<_>>(),
            }),
        }
    }
    /// Function (or property) name for `Ident` and `Call`.
    pub fn name(&self) -> Option<&str> {
        match self {
            Member::Ident { name, .. } | Member::Call { name, .. } => Some(name),
            Member::Path { .. } => None,
        }
    }
}

/// Classifies a bare argument word.
fn bare_word_kind(word: String) -> ArgKind {
    let numeric_start = word
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_digit() || matches!(first, '-' | '+' | '.'));
    if numeric_start {
        if let Ok(number) = word.parse::<f64>() {
            if number.is_finite() {
                return ArgKind::Number(number);
            }
        }
    }
    match word.to_ascii_lowercase().as_str() {
        "true" => ArgKind::Bool(true),
        "false" => ArgKind::Bool(false),
        _ => ArgKind::Ident(word),
    }
}

/// Maximum nesting of struct literals in call arguments (the runtime's `GMaxMemberNesting`).
pub const MAX_MEMBER_NESTING: u32 = 64;

struct MemberParser<'a> {
    source: &'a Source,
    index: usize,
    /// Currently open struct literals.
    nesting: u32,
}

type MemberResult<T> = Result<T, MemberError>;

impl MemberParser<'_> {
    fn len(&self) -> usize {
        self.source.chars.len()
    }
    fn peek(&self) -> Option<char> {
        self.source.chars.get(self.index).copied()
    }
    fn at(&self, index: usize) -> u32 {
        self.source.offsets[index.min(self.len())]
    }
    fn fail<T>(&self, kind: MemberErrorKind, index: usize, message: &str) -> MemberResult<T> {
        Err(MemberError {
            kind,
            at: self.at(index),
            message: message.to_owned(),
        })
    }
    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.index += 1;
        }
    }
    fn read_word(&mut self) -> Option<String> {
        let start = self.index;
        if !self
            .peek()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        {
            return None;
        }
        while self
            .peek()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            self.index += 1;
        }
        Some(self.source.text(start, self.index))
    }

    fn parse_member(&mut self) -> MemberResult<Member> {
        self.skip_ws();
        if self.index >= self.len() {
            return self.fail(MemberErrorKind::Empty, 0, "member is empty");
        }
        let start = self.index;
        let Some(name) = self.read_word() else {
            return self.fail(
                MemberErrorKind::Malformed,
                self.index,
                "expected a property or function name",
            );
        };
        let name_end = self.index;
        self.skip_ws();
        match self.peek() {
            None => Ok(Member::Ident {
                name,
                span: self.source.span(start, name_end),
            }),
            Some('(') => {
                self.index += 1;
                let args = self.parse_args(')')?;
                let end = self.index;
                self.skip_ws();
                if self.index < self.len() {
                    return self.fail(
                        MemberErrorKind::Malformed,
                        self.index,
                        "unexpected text after the call",
                    );
                }
                Ok(Member::Call {
                    name,
                    name_span: self.source.span(start, name_end),
                    args,
                    span: self.source.span(start, end),
                })
            }
            Some('.') | Some('[') => {
                self.index = name_end;
                let mut depth = 0i32;
                let mut quote: Option<char> = None;
                while let Some(ch) = self.peek() {
                    if let Some(open) = quote {
                        if ch == '\\' {
                            self.index += 1;
                        } else if ch == open {
                            quote = None;
                        }
                        self.index += 1;
                        continue;
                    }
                    match ch {
                        '\'' | '"' => quote = Some(ch),
                        '[' => depth += 1,
                        ']' => depth -= 1,
                        c if c.is_whitespace() && depth == 0 => break,
                        c if depth == 0 && !(c.is_ascii_alphanumeric() || c == '_' || c == '.') => {
                            return self.fail(
                                MemberErrorKind::Malformed,
                                self.index,
                                "invalid character in property path",
                            );
                        }
                        _ => {}
                    }
                    self.index += 1;
                }
                if depth > 0 || quote.is_some() {
                    return self.fail(MemberErrorKind::Unbalanced, self.len(), "unbalanced '['");
                }
                let end = self.index;
                self.skip_ws();
                if self.index < self.len() {
                    return self.fail(
                        MemberErrorKind::Malformed,
                        self.index,
                        "unexpected text after the property path",
                    );
                }
                Ok(Member::Path {
                    path: self.source.text(start, end),
                    span: self.source.span(start, end),
                })
            }
            Some(_) => self.fail(
                MemberErrorKind::Malformed,
                self.index,
                "unexpected text after the member name",
            ),
        }
    }

    /// Parses comma separated arguments up to and including `close`.
    fn parse_args(&mut self, close: char) -> MemberResult<Vec<Arg>> {
        let mut args = Vec::new();
        self.skip_ws();
        if self.peek() == Some(close) {
            self.index += 1;
            return Ok(args);
        }
        loop {
            args.push(self.parse_arg()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.index += 1;
                    self.skip_ws();
                    if self.peek() == Some(close) {
                        return self.fail(
                            MemberErrorKind::ExpectedArgument,
                            self.index,
                            "expected an argument after ','",
                        );
                    }
                }
                Some(ch) if ch == close => {
                    self.index += 1;
                    return Ok(args);
                }
                None => {
                    return self.fail(
                        MemberErrorKind::Unbalanced,
                        self.len(),
                        "missing closing bracket",
                    )
                }
                Some(_) => {
                    return self.fail(
                        MemberErrorKind::Malformed,
                        self.index,
                        "expected ',' or a closing bracket",
                    )
                }
            }
        }
    }

    fn parse_arg(&mut self) -> MemberResult<Arg> {
        self.skip_ws();
        let start = self.index;
        let Some(ch) = self.peek() else {
            return self.fail(MemberErrorKind::Unbalanced, self.len(), "missing argument");
        };
        let kind = match ch {
            ',' | ')' | '}' | ']' => {
                return self.fail(
                    MemberErrorKind::ExpectedArgument,
                    self.index,
                    "expected an argument",
                )
            }
            '\'' | '"' => ArgKind::Str(self.parse_string(ch)?),
            '{' => self.parse_struct(None)?,
            _ => {
                // A bare word: number, boolean, enum entry (`EType::Value`), unquoted path or the
                // type name of a struct literal.
                let word = self.read_bare_word();
                let after_word = self.index;
                self.skip_ws();
                let type_like = word
                    .chars()
                    .next()
                    .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
                    && !word.contains("::");
                if self.peek() == Some('{') && type_like {
                    self.parse_struct(Some(word))?
                } else {
                    self.index = after_word;
                    bare_word_kind(word)
                }
            }
        };
        Ok(Arg {
            kind,
            span: self.source.span(start, self.index),
        })
    }

    /// Characters up to the next delimiter (`,` `)` `}` `]`, whitespace, `{` or a quote).
    fn read_bare_word(&mut self) -> String {
        let start = self.index;
        while self.peek().is_some_and(|ch| {
            !(ch.is_whitespace() || matches!(ch, ',' | ')' | '}' | ']' | '{' | '\'' | '"'))
        }) {
            self.index += 1;
        }
        if self.index == start {
            // Always make progress on stray delimiters.
            self.index += 1;
        }
        self.source.text(start, self.index)
    }

    fn parse_string(&mut self, quote: char) -> MemberResult<String> {
        self.index += 1;
        let mut out = String::new();
        while let Some(ch) = self.peek() {
            self.index += 1;
            if ch == quote {
                return Ok(out);
            }
            if ch == '\\' {
                match self.peek() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => break,
                }
                self.index += 1;
                continue;
            }
            out.push(ch);
        }
        self.fail(
            MemberErrorKind::Unbalanced,
            self.len(),
            "string is not closed",
        )
    }

    fn parse_struct(&mut self, type_name: Option<String>) -> MemberResult<ArgKind> {
        // At '{'.
        if self.nesting >= MAX_MEMBER_NESTING {
            return self.fail(
                MemberErrorKind::Malformed,
                self.index,
                "struct literals are nested too deeply (limit 64)",
            );
        }
        self.nesting += 1;
        let result = self.parse_struct_body(type_name);
        self.nesting -= 1;
        result
    }

    fn parse_struct_body(&mut self, type_name: Option<String>) -> MemberResult<ArgKind> {
        // At '{'.
        self.index += 1;
        self.skip_ws();
        if self.peek() == Some('}') {
            self.index += 1;
            return Ok(ArgKind::Struct {
                type_name,
                fields: StructFields::Positional(Vec::new()),
            });
        }
        if self.named_field_ahead() {
            let mut fields = Vec::new();
            loop {
                self.skip_ws();
                let Some(name) = self.read_word() else {
                    return self.fail(
                        MemberErrorKind::ExpectedArgument,
                        self.index,
                        "expected a field name",
                    );
                };
                self.skip_ws();
                if self.peek() != Some('=') {
                    return self.fail(
                        MemberErrorKind::Malformed,
                        self.index,
                        "expected '=' after the field name",
                    );
                }
                self.index += 1;
                fields.push((name, self.parse_arg()?));
                self.skip_ws();
                match self.peek() {
                    Some(',') => {
                        self.index += 1;
                        self.skip_ws();
                        if self.peek() == Some('}') {
                            return self.fail(
                                MemberErrorKind::ExpectedArgument,
                                self.index,
                                "expected a field after ','",
                            );
                        }
                    }
                    Some('}') => {
                        self.index += 1;
                        return Ok(ArgKind::Struct {
                            type_name,
                            fields: StructFields::Named(fields),
                        });
                    }
                    None => {
                        return self.fail(MemberErrorKind::Unbalanced, self.len(), "missing '}'")
                    }
                    Some(_) => {
                        return self.fail(
                            MemberErrorKind::Malformed,
                            self.index,
                            "expected ',' or '}'",
                        )
                    }
                }
            }
        }
        let args = self.parse_args('}')?;
        Ok(ArgKind::Struct {
            type_name,
            fields: StructFields::Positional(args),
        })
    }

    /// `Name =` (but not `==`) ahead of the cursor.
    fn named_field_ahead(&self) -> bool {
        let mut probe = self.index;
        let chars = &self.source.chars;
        if !chars
            .get(probe)
            .is_some_and(|ch| ch.is_ascii_alphabetic() || *ch == '_')
        {
            return false;
        }
        while chars
            .get(probe)
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        {
            probe += 1;
        }
        while chars.get(probe).is_some_and(|ch| ch.is_whitespace()) {
            probe += 1;
        }
        chars.get(probe) == Some(&'=') && chars.get(probe + 1) != Some(&'=')
    }
}

/// Parses the member part of an apply-time body (`mDisplayName`, `mStruct.Field`, `mArr[0]`,
/// `GetItemName()`, `Fn(1, 'a', Type{...}, {Field=2})`).
pub fn parse_member(src: &str) -> Result<Member, MemberError> {
    let source = Source::new(src);
    let mut parser = MemberParser {
        source: &source,
        index: 0,
        nesting: 0,
    };
    parser.parse_member()
}

// ---------------------------------------------------------------------------------------------
// Completion support
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Stage {
    /// Typing the prefix after `{`.
    Provider,
    /// Body of a pre-pass provider (`{cfg:|`, `{cfg:Pack:|`, `{session:|`).
    Body,
    /// Target reference of an apply-time provider (`{item:|`).
    Target,
    /// Member name or property path (`{item:Desc_X_C:|`).
    Member,
    /// Inside the parentheses of a function call.
    Argument,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FunctionContext {
    pub name: String,
    /// Index of the argument under the cursor (0-based).
    pub arg_index: u32,
    /// Raw text of the arguments completed before the current one.
    pub args_before: Vec<String>,
    pub in_quote: Option<char>,
}

/// The token the cursor is inside of (innermost when nested).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Partial {
    pub stage: Stage,
    /// Provider prefix as typed (complete once the colon is typed).
    pub prefix: String,
    /// Whether `prefix` is a built-in prefix.
    pub known: bool,
    /// Offset of the opening brace.
    pub token_start: u32,
    /// Apply-time target text (complete in the `Member`/`Argument` stages).
    pub target: Option<String>,
    /// `cfg:Pack:` pack part already typed.
    pub pack: Option<String>,
    /// Text of the current segment before the cursor.
    pub typed: String,
    /// Span to replace: current segment start to the end of the word under the cursor.
    pub replace: Span,
    pub function: Option<FunctionContext>,
    /// Token that contains this one, when nested.
    pub outer: Option<Box<Partial>>,
    pub closed: bool,
}

/// Longest scalar [`partial_at`] looks at; a value this large is data, not a template being typed.
pub const MAX_PARTIAL_BYTES: usize = 64 * 1024;

/// Locates the innermost template token around `offset` (UTF-16) and describes what is being typed.
/// Returns `None` when the cursor is not inside `{...}` (or inside an unknown third-party prefix),
/// and for values over [`MAX_PARTIAL_BYTES`]. Tokens nested deeper than [`MAX_DEPTH`] + 1 are text.
pub fn partial_at(src: &str, offset: u32) -> Option<Partial> {
    if src.len() > MAX_PARTIAL_BYTES {
        return None;
    }
    let source = Source::new(src);
    let cursor = source.index_at(offset).min(source.chars.len());
    let cursor = if source.offsets[cursor] < offset {
        // Offset falls in the middle of a surrogate pair: use the next character boundary.
        (cursor + 1).min(source.chars.len())
    } else {
        cursor
    };
    locate(&source, 0, source.chars.len(), cursor, 1)
}

/// Innermost token around `cursor` inside `lo..hi`, a region at nesting `depth`. The scan reads
/// tokens up to depth `MAX_DEPTH + 1` (the deepest is reported as `NestingDepth`); a `{` below that
/// is plain text, which bounds the recursion for pathological input.
fn locate(source: &Source, lo: usize, hi: usize, cursor: usize, depth: u8) -> Option<Partial> {
    if depth > MAX_DEPTH + 1 {
        return None;
    }
    let chars = &source.chars;
    let mut index = lo;
    while index < hi {
        let ch = chars[index];
        if ch == '\\' && index + 1 < hi && chars[index + 1] == '{' {
            index += 2;
            continue;
        }
        if ch != '{' {
            index += 1;
            continue;
        }
        if index >= cursor {
            return None;
        }
        if let Some((prefix, colon)) = prefix_at(chars, index + 1, hi) {
            if phase_of(&prefix).is_some() {
                let close = find_close(chars, colon, hi);
                let end = close.unwrap_or(hi);
                if cursor > index && cursor <= end && (close.is_none() || cursor <= end) {
                    if cursor <= colon {
                        // Cursor inside the prefix itself.
                        return Some(provider_partial(source, index, cursor, hi, close.is_some()));
                    }
                    if let Some(mut inner) = locate(source, colon + 1, end, cursor, depth + 1) {
                        let outer = describe_token(source, &prefix, index, colon, cursor, close);
                        attach_outer(&mut inner, outer);
                        return Some(inner);
                    }
                    return Some(describe_token(source, &prefix, index, colon, cursor, close));
                }
                if let Some(close) = close {
                    index = close + 1;
                    continue;
                }
                return None;
            }
            // Third-party or misspelled prefix: no completion inside, skip the braces.
            let close = find_close(chars, colon, hi);
            if let Some(close) = close {
                if cursor > index && cursor <= close {
                    return None;
                }
                index = close + 1;
                continue;
            }
            return None;
        }
        // `{` followed by an identifier run without colon: typing the provider.
        let mut end = index + 1;
        while end < hi && is_prefix_char(chars[end]) {
            end += 1;
        }
        if cursor > index
            && cursor <= end
            && chars[index + 1..cursor]
                .iter()
                .all(|ch| is_prefix_char(*ch))
            && (cursor == index + 1 || chars[index + 1].is_ascii_alphabetic())
        {
            return Some(provider_partial(source, index, cursor, end, false));
        }
        index += 1;
    }
    None
}

/// Appends `outer` at the end of the `outer` chain of `inner` (iteratively: the chain is as long as
/// the nesting).
fn attach_outer(inner: &mut Partial, outer: Partial) {
    let mut slot = &mut inner.outer;
    while slot.is_some() {
        slot = &mut slot.as_mut().expect("checked above").outer;
    }
    *slot = Some(Box::new(outer));
}

fn provider_partial(
    source: &Source,
    open: usize,
    cursor: usize,
    limit: usize,
    closed: bool,
) -> Partial {
    let chars = &source.chars;
    let mut end = cursor;
    while end < limit && is_prefix_char(chars[end]) {
        end += 1;
    }
    let typed = source.text(open + 1, cursor);
    Partial {
        stage: Stage::Provider,
        known: phase_of(&typed).is_some(),
        prefix: typed.clone(),
        token_start: source.offsets[open],
        target: None,
        pack: None,
        typed,
        replace: source.span(open + 1, end),
        function: None,
        outer: None,
        closed,
    }
}

fn describe_token(
    source: &Source,
    prefix: &str,
    open: usize,
    colon: usize,
    cursor: usize,
    close: Option<usize>,
) -> Partial {
    let chars = &source.chars;
    let body_before = &chars[colon + 1..cursor];
    let limit = close.unwrap_or(chars.len());
    let word_end = |from: usize| {
        let mut end = from;
        while end < limit
            && (chars[end].is_ascii_alphanumeric() || matches!(chars[end], '_' | '.' | '/' | '-'))
        {
            end += 1;
        }
        end
    };
    let mut partial = Partial {
        stage: Stage::Body,
        prefix: prefix.to_owned(),
        known: true,
        token_start: source.offsets[open],
        target: None,
        pack: None,
        typed: String::new(),
        replace: source.span(cursor, cursor),
        function: None,
        outer: None,
        closed: close.is_some(),
    };
    if phase_of(prefix) == Some(Phase::PrePass) {
        let text: String = body_before.iter().collect();
        let (pack, typed_start) = match (prefix, text.rfind(':')) {
            ("cfg", Some(position)) => (
                Some(text[..position].to_owned()),
                colon + 1 + text[..=position].chars().count(),
            ),
            _ => (None, colon + 1),
        };
        partial.pack = pack;
        partial.typed = source.text(typed_start, cursor);
        partial.replace = source.span(typed_start, word_end(cursor));
        return partial;
    }
    // Apply-time: `<target>:<member>`; find the first top-level colon.
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut target_end: Option<usize> = None;
    let mut call_open: Option<usize> = None;
    let mut commas: Vec<usize> = Vec::new();
    let mut member_start = colon + 1;
    let mut call_depth_base = 0i32;
    for (offset, ch) in body_before.iter().enumerate() {
        let position = colon + 1 + offset;
        if let Some(open_quote) = quote {
            if *ch == '\\' {
                continue;
            }
            if *ch == open_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(*ch),
            ':' if depth == 0 && target_end.is_none() => {
                target_end = Some(position);
                member_start = position + 1;
            }
            '(' => {
                if target_end.is_some() && call_open.is_none() && depth == 0 {
                    call_open = Some(position);
                    call_depth_base = depth + 1;
                }
                depth += 1;
            }
            '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if *ch == ')' && call_open.is_some() && depth < call_depth_base {
                    call_open = None;
                    commas.clear();
                }
            }
            ',' if call_open.is_some() && depth == call_depth_base => commas.push(position),
            _ => {}
        }
    }
    let Some(target_end) = target_end else {
        partial.stage = Stage::Target;
        partial.typed = source.text(colon + 1, cursor);
        partial.replace = source.span(colon + 1, word_end(cursor));
        return partial;
    };
    partial.target = Some(source.text(colon + 1, target_end));
    if let Some(open_paren) = call_open {
        let name = source.text(member_start, open_paren).trim().to_owned();
        let mut args_before = Vec::new();
        let mut segment_start = open_paren + 1;
        for comma in &commas {
            args_before.push(source.text(segment_start, *comma).trim().to_owned());
            segment_start = comma + 1;
        }
        let raw = source.text(segment_start, cursor);
        let typed = raw.trim_start().to_owned();
        let leading = raw.chars().count() - typed.chars().count();
        let in_quote = typed.chars().next().filter(|ch| matches!(ch, '\'' | '"'));
        partial.stage = Stage::Argument;
        partial.function = Some(FunctionContext {
            name,
            arg_index: commas.len() as u32,
            args_before,
            in_quote,
        });
        let start = segment_start + leading;
        partial.typed = typed;
        partial.replace = source.span(start, word_end(cursor).max(cursor));
        return partial;
    }
    partial.stage = Stage::Member;
    partial.typed = source.text(member_start, cursor);
    partial.replace = source.span(member_start, word_end(cursor));
    partial
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_miss_prefixes() {
        assert_eq!(near_miss("cfgx"), Some("cfg"));
        assert_eq!(near_miss("itm"), Some("item"));
        assert_eq!(near_miss("CFG"), Some("cfg"));
        assert_eq!(near_miss("banana"), None);
    }

    #[test]
    fn nesting_depth_five_is_reported() {
        let src = "{item:A:F({item:B:F({item:C:F({item:D:F({item:E:g})})})})}";
        let scan = scan(src, ScanPhase::All);
        assert!(scan
            .issues
            .iter()
            .any(|issue| issue.kind == ScanIssueKind::NestingDepth));
        let ok = "{item:A:F({item:B:F({item:C:F({item:D:g})})})}";
        assert!(scan_issues(ok).is_empty());
    }

    #[test]
    fn struct_literal_nesting_is_limited_to_64() {
        let nested = |depth: usize| format!("Fn({}1{})", "{".repeat(depth), "}".repeat(depth));
        assert!(parse_member(&nested(64)).is_ok());
        let error = parse_member(&nested(65)).unwrap_err();
        assert_eq!(error.kind, MemberErrorKind::Malformed);
        // At the `{` that opens level 65: `Fn(` is 3 characters, then 64 opened braces.
        assert_eq!(error.at, 3 + 64);
        // Typed and named literals count too, and closed literals release their level.
        let typed = format!("Fn({}1{})", "T{f=".repeat(65), "}".repeat(65));
        assert_eq!(
            parse_member(&typed).unwrap_err().kind,
            MemberErrorKind::Malformed
        );
        let flat = format!("Fn({})", vec!["{1}"; 300].join(", "));
        assert!(parse_member(&flat).is_ok());
    }

    fn scan_issues(src: &str) -> Vec<ScanIssue> {
        scan(src, ScanPhase::All).issues
    }

    #[test]
    fn partial_stages() {
        let partial = partial_at("Made {ite", 9).unwrap();
        assert_eq!(partial.stage, Stage::Provider);
        assert_eq!(partial.typed, "ite");
        let partial = partial_at("{cfg:Other:sp", 13).unwrap();
        assert_eq!(partial.stage, Stage::Body);
        assert_eq!(partial.pack.as_deref(), Some("Other"));
        assert_eq!(partial.typed, "sp");
        let partial = partial_at("{item:/Game/A.A_C:m", 19).unwrap();
        assert_eq!(partial.stage, Stage::Member);
        assert_eq!(partial.target.as_deref(), Some("/Game/A.A_C"));
        let partial = partial_at("{class:/Script/E.K:Max(1, ", 26).unwrap();
        assert_eq!(partial.stage, Stage::Argument);
        let function = partial.function.unwrap();
        assert_eq!(function.name, "Max");
        assert_eq!(function.arg_index, 1);
        assert_eq!(function.args_before, ["1"]);
        assert!(partial_at("plain {0} text", 8).is_none());
        let nested = partial_at("{item:X:Fn({cfg:n", 17).unwrap();
        assert_eq!(nested.prefix, "cfg");
        assert_eq!(
            nested.outer.as_ref().map(|outer| outer.prefix.as_str()),
            Some("item")
        );
    }

    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    const PIECES: [&str; 30] = [
        "{",
        "}",
        "{cfg:",
        "{item:",
        "{class:",
        ":",
        "(",
        ")",
        "[",
        "]",
        ",",
        "'",
        "\"",
        "\\",
        "\\{item:",
        "a",
        "Fn",
        "mX",
        "1",
        "-2.5",
        ".",
        "=",
        " ",
        "::",
        "\u{1F600}",
        "\u{e9}",
        "true",
        "{0}",
        "{cfgx:",
        "X",
    ];

    #[test]
    fn arbitrary_input_never_panics_and_spans_stay_in_bounds() {
        let mut random = Lcg(11);
        for _ in 0..4000 {
            let length = random.next() % 14;
            let source = (0..length)
                .map(|_| PIECES[(random.next() as usize) % PIECES.len()])
                .collect::<String>();
            let total = source.encode_utf16().count() as u32;
            for phase in [ScanPhase::PrePass, ScanPhase::Apply, ScanPhase::All] {
                let scan = scan(&source, phase);
                for token in scan.tokens() {
                    assert!(token.span.end <= total, "{source:?}");
                    assert!(token.body_span.end <= total, "{source:?}");
                }
                for issue in &scan.issues {
                    assert!(issue.span.end <= total, "{source:?}");
                }
            }
            let _ = expand_prepass(&source, &mut |_, _| Some("v".to_owned()));
            let _ = parse_member(&source);
            let mut offset = 0u32;
            for ch in source.chars().chain(std::iter::once(' ')) {
                if let Some(partial) = partial_at(&source, offset) {
                    assert!(partial.replace.end <= total, "{source:?}@{offset}");
                }
                offset += ch.len_utf16() as u32;
            }
        }
    }

    #[test]
    fn expansion_keeps_unresolved_tokens_and_counts_replacements() {
        let mut resolve =
            |prefix: &str, body: &str| (prefix == "cfg" && body == "a").then(|| "1".to_owned());
        let expansion = expand_prepass("{cfg:a} {cfg:b} {cfg:{cfg:a}}", &mut resolve);
        assert_eq!(expansion.output, "1 {cfg:b} {cfg:1}");
        assert_eq!(expansion.replaced, 2);
    }

    #[test]
    fn scan_reports_each_token_with_utf16_spans() {
        let scan = scan("\u{1F600}{cfg:a}x", ScanPhase::PrePass);
        let tokens = scan.tokens();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].span, Span::new(2, 9));
        assert_eq!(tokens[0].prefix_span, Span::new(3, 6));
        assert_eq!(tokens[0].body_span, Span::new(7, 8));
        assert_eq!(tokens[0].body, "a");
    }

    /// Runs `work` on a thread with a small stack: unbounded recursion would overflow it.
    fn on_small_stack<R: Send + 'static>(work: impl FnOnce() -> R + Send + 'static) -> R {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(work)
            .expect("spawn")
            .join()
            .expect("deeply nested input must not overflow the stack")
    }

    fn outer_chain(partial: &Partial) -> usize {
        1 + partial.outer.as_ref().map_or(0, |outer| outer_chain(outer))
    }

    /// Regression: a 20 KB `{cfg:{cfg:...` used to recurse once per level (quadratically) in
    /// `locate`, `attach_outer` and `describe_token` and crashed the server with a stack overflow.
    #[test]
    fn four_thousand_nesting_levels_are_bounded() {
        on_small_stack(|| {
            let open = "{cfg:".repeat(4000);
            let balanced = format!("{open}x{}", "}".repeat(4000));
            for src in [open, balanced] {
                let total = src.encode_utf16().count() as u32;
                for offset in [0, 1, 5, 6, 12, 26, 27, 1000, total / 2, total - 1, total] {
                    if let Some(partial) = partial_at(&src, offset) {
                        assert!(
                            outer_chain(&partial) <= usize::from(MAX_DEPTH) + 1,
                            "offset {offset}"
                        );
                    }
                }
                let scanned = scan(&src, ScanPhase::All);
                assert!(scanned
                    .tokens()
                    .iter()
                    .all(|token| token.depth <= MAX_DEPTH + 1));
                let _ = expand_prepass(&src, &mut |_, _| Some("v".to_owned()));
            }
        });
    }

    #[test]
    fn nesting_beyond_the_limit_is_text_for_completion() {
        let nested = |levels: usize| format!("{}a", "{cfg:".repeat(levels));
        for levels in [4, 5, 6, 12] {
            let src = nested(levels);
            let end = src.encode_utf16().count() as u32;
            let partial = partial_at(&src, end).expect("a token is open");
            assert_eq!(
                outer_chain(&partial),
                levels.min(usize::from(MAX_DEPTH) + 1),
                "{levels} levels"
            );
        }
    }

    #[test]
    fn partial_at_skips_values_over_64_kib() {
        let big = format!("{{cfg:{}", "a".repeat(MAX_PARTIAL_BYTES));
        assert!(partial_at(&big, big.len() as u32).is_none());
        let small = format!("{{cfg:{}", "a".repeat(1000));
        assert!(partial_at(&small, small.len() as u32).is_some());
    }

    #[test]
    fn expansion_stops_at_the_nesting_limit() {
        // Six levels deep: the sixth token lies past MAX_DEPTH + 1 and stays part of the literal
        // body of the fifth (the scan reports the fifth as `NestingDepth`).
        let src = format!("{}a{}", "{cfg:".repeat(6), "}".repeat(6));
        let mut seen = Vec::new();
        let expansion = expand_prepass(&src, &mut |_, body| {
            seen.push(body.to_owned());
            Some("v".to_owned())
        });
        assert_eq!(expansion.output, "v");
        assert_eq!(
            seen.first().map(String::as_str),
            Some("{cfg:a}"),
            "{seen:?}"
        );
        assert_eq!(seen.len(), 5);
    }

    #[test]
    fn partial_in_arguments_tracks_quotes_and_structs() {
        let src = "{class:/Script/E.K:Fn(1, {A=2, B=3}, 'te";
        let partial = partial_at(src, src.encode_utf16().count() as u32).unwrap();
        let function = partial.function.unwrap();
        assert_eq!(function.arg_index, 2);
        assert_eq!(function.in_quote, Some('\''));
        assert_eq!(function.args_before, ["1", "{A=2, B=3}"]);
        assert_eq!(partial.typed, "'te");
    }
}
