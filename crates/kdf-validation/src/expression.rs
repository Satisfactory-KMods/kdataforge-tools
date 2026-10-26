//! Pack-configuration expressions (`config:` conditions).
//!
//! Grammar (frozen, identical to the C++ runtime; vectors in `tests/fixtures/config-contract-vectors.json`):
//!
//! ```text
//! or := and ('||' and)* ; and := cmp ('&&' cmp)* ; cmp := unary [cmp_op unary]   (non-associative)
//! cmp_op := == != < <= > >= contains in ; unary := '!' unary | primary
//! primary := '(' or ')' | '[' [literal (',' literal)*] ']' | literal | operand
//! literal := number (optional '-', digits, optional '.' digits, NO exponent) | 'str' | "str" | true | false
//! operand := config_id | cfg:[pack:]config_id | session:K | ags:K | option:K
//! ```
//!
//! All spans and offsets are UTF-16 code units relative to the start of the expression string.
//! [`tokenize`] is total (it never fails); [`parse`] reports the first error only.

use serde::Serialize;
use serde_json::{json, Value};

/// Half-open range in UTF-16 code units.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
    pub const fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }
    pub const fn is_empty(self) -> bool {
        self.end <= self.start
    }
    /// True when the cursor at `offset` is inside the span or touches one of its ends.
    pub const fn touches(self, offset: u32) -> bool {
        self.start <= offset && offset <= self.end
    }
    pub fn join(self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }
}

/// Comparison operators. `Contains` and `In` are written as words.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
    In,
}

impl CmpOp {
    pub const ALL: [CmpOp; 8] = [
        CmpOp::Eq,
        CmpOp::Ne,
        CmpOp::Lt,
        CmpOp::Le,
        CmpOp::Gt,
        CmpOp::Ge,
        CmpOp::Contains,
        CmpOp::In,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
            CmpOp::Contains => "contains",
            CmpOp::In => "in",
        }
    }
    pub fn parse(text: &str) -> Option<CmpOp> {
        CmpOp::ALL.into_iter().find(|op| op.as_str() == text)
    }
    /// `< <= > >=` accept numbers only.
    pub fn is_ordering(self) -> bool {
        matches!(self, CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum TokenKind {
    /// Bare dotted identifier (`features.value`).
    Ident,
    /// `cfg:`, `session:`, `ags:` or `option:` reference (whole run, including the prefix).
    Reference,
    Number,
    /// A number followed by junk (`1.`, `1e5`, `12abc`).
    InvalidNumber,
    String,
    UnterminatedString,
    True,
    False,
    Not,
    AndAnd,
    OrOr,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Compare(CmpOp),
    /// A character that cannot start any token (`#`, a lone `=`, `&`, `|`, `-`, ...).
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    /// Raw source slice.
    pub text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum ErrorCode {
    Empty,
    UnexpectedChar,
    UnexpectedToken,
    ExpectedOperand,
    ExpectedCloseParen,
    ExpectedCloseBracket,
    UnterminatedString,
    InvalidNumber,
    InvalidRef,
    ReservedId,
    ChainedComparison,
    ListElementNotLiteral,
    TrailingInput,
    /// More than [`MAX_NESTING`] nested `(` and `!`.
    TooDeep,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Empty => "empty",
            ErrorCode::UnexpectedChar => "unexpected-char",
            ErrorCode::UnexpectedToken => "unexpected-token",
            ErrorCode::ExpectedOperand => "expected-operand",
            ErrorCode::ExpectedCloseParen => "expected-close-paren",
            ErrorCode::ExpectedCloseBracket => "expected-close-bracket",
            ErrorCode::UnterminatedString => "unterminated-string",
            ErrorCode::InvalidNumber => "invalid-number",
            ErrorCode::InvalidRef => "invalid-ref",
            ErrorCode::ReservedId => "reserved-id",
            ErrorCode::ChainedComparison => "chained-comparison",
            ErrorCode::ListElementNotLiteral => "list-element-not-literal",
            ErrorCode::TrailingInput => "trailing-input",
            ErrorCode::TooDeep => "too-deep",
        }
    }
}

/// First parse error. `at` is a UTF-16 offset (the end of the input when input ended too early).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ParseError {
    pub code: ErrorCode,
    pub at: u32,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Literal {
    Number(f64),
    Str(String),
    Bool(bool),
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum OperandKind {
    /// `id`, `cfg:id` (own pack, `pack == None`) or `cfg:Pack:id`.
    Config { pack: Option<String>, id: String },
    /// `session:K`, `ags:K`, `option:K` (provider is lowercase).
    Setting { provider: String, key: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct Operand {
    pub kind: OperandKind,
    pub span: Span,
}

impl Operand {
    /// Environment key used by [`type_check`]/[`eval`] vectors: own-pack id, `Pack:id` or `provider:key`.
    pub fn key(&self) -> String {
        match &self.kind {
            OperandKind::Config { pack: None, id } => id.clone(),
            OperandKind::Config {
                pack: Some(pack),
                id,
            } => format!("{pack}:{id}"),
            OperandKind::Setting { provider, key } => format!("{provider}:{key}"),
        }
    }
    /// Source-form reference (`cfg:Pack:id`, `session:K`).
    pub fn canonical(&self) -> String {
        match &self.kind {
            OperandKind::Config { pack: None, id } => format!("cfg:{id}"),
            OperandKind::Config {
                pack: Some(pack),
                id,
            } => format!("cfg:{pack}:{id}"),
            OperandKind::Setting { provider, key } => format!("{provider}:{key}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ListItem {
    pub literal: Literal,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Expr {
    /// Source span; parentheses are not part of the span.
    pub span: Span,
    pub kind: ExprKind,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum ExprKind {
    /// Flattened `a || b || c`.
    Or(Vec<Expr>),
    /// Flattened `a && b && c`.
    And(Vec<Expr>),
    Not(Box<Expr>),
    Compare {
        op: CmpOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Operand(Operand),
    Literal(Literal),
    List(Vec<ListItem>),
}

impl Expr {
    /// JSON AST used by the contract vectors (`["and", a, b]`, `["cmp", ">=", a, b]`, `["cfg", null, "x"]`, ...).
    pub fn to_vector(&self) -> Value {
        match &self.kind {
            ExprKind::Or(items) => nary("or", items),
            ExprKind::And(items) => nary("and", items),
            ExprKind::Not(inner) => json!(["not", inner.to_vector()]),
            ExprKind::Compare { op, left, right } => {
                json!(["cmp", op.as_str(), left.to_vector(), right.to_vector()])
            }
            ExprKind::Operand(operand) => match &operand.kind {
                OperandKind::Config { pack, id } => json!(["cfg", pack, id]),
                OperandKind::Setting { provider, key } => json!(["setting", provider, key]),
            },
            ExprKind::Literal(literal) => literal.to_vector(),
            ExprKind::List(items) => {
                let literals = items
                    .iter()
                    .map(|item| item.literal.to_vector())
                    .collect::<Vec<_>>();
                json!(["list", literals])
            }
        }
    }
}

fn nary(tag: &str, items: &[Expr]) -> Value {
    let mut out = vec![Value::String(tag.to_owned())];
    out.extend(items.iter().map(Expr::to_vector));
    Value::Array(out)
}

impl Literal {
    pub fn to_vector(&self) -> Value {
        match self {
            Literal::Number(number) => json!(["num", json_number(*number)]),
            Literal::Str(text) => json!(["str", text]),
            Literal::Bool(flag) => json!(["bool", flag]),
        }
    }
}

/// Integral values render as JSON integers (`100`, not `100.0`) so vectors compare structurally.
pub fn json_number(number: f64) -> Value {
    if number.is_finite() && number.fract() == 0.0 && number.abs() < 9_007_199_254_740_992.0 {
        Value::from(number as i64)
    } else {
        Value::from(number)
    }
}

// ---------------------------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------------------------

struct Chars {
    chars: Vec<char>,
    /// `offsets[i]` is the UTF-16 offset of `chars[i]`; the last element is the total length.
    offsets: Vec<u32>,
}

impl Chars {
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
    fn len(&self) -> usize {
        self.chars.len()
    }
    fn at(&self, index: usize) -> Option<char> {
        self.chars.get(index).copied()
    }
    fn span(&self, start: usize, end: usize) -> Span {
        Span::new(self.offsets[start], self.offsets[end])
    }
    fn slice(&self, start: usize, end: usize) -> String {
        self.chars[start..end].iter().collect()
    }
    fn total(&self) -> u32 {
        *self.offsets.last().unwrap_or(&0)
    }
}

fn is_word_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn is_ref_body(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | ':' | '-')
}

/// Splits `src` into tokens. Lexical problems become `Unknown`, `InvalidNumber` or
/// `UnterminatedString` tokens, so this function is total and can be used on half-typed input.
pub fn tokenize(src: &str) -> Vec<Token> {
    let text = Chars::new(src);
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < text.len() {
        let ch = text.chars[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let push = |tokens: &mut Vec<Token>, kind: TokenKind, end: usize| {
            tokens.push(Token {
                kind,
                span: text.span(start, end),
                text: text.slice(start, end),
            });
        };
        match ch {
            '(' => {
                i += 1;
                push(&mut tokens, TokenKind::LParen, i);
            }
            ')' => {
                i += 1;
                push(&mut tokens, TokenKind::RParen, i);
            }
            '[' => {
                i += 1;
                push(&mut tokens, TokenKind::LBracket, i);
            }
            ']' => {
                i += 1;
                push(&mut tokens, TokenKind::RBracket, i);
            }
            ',' => {
                i += 1;
                push(&mut tokens, TokenKind::Comma, i);
            }
            '!' => {
                if text.at(i + 1) == Some('=') {
                    i += 2;
                    push(&mut tokens, TokenKind::Compare(CmpOp::Ne), i);
                } else {
                    i += 1;
                    push(&mut tokens, TokenKind::Not, i);
                }
            }
            '=' => {
                if text.at(i + 1) == Some('=') {
                    i += 2;
                    push(&mut tokens, TokenKind::Compare(CmpOp::Eq), i);
                } else {
                    i += 1;
                    push(&mut tokens, TokenKind::Unknown, i);
                }
            }
            '<' | '>' => {
                let orderless = if ch == '<' { CmpOp::Lt } else { CmpOp::Gt };
                let with_eq = if ch == '<' { CmpOp::Le } else { CmpOp::Ge };
                if text.at(i + 1) == Some('=') {
                    i += 2;
                    push(&mut tokens, TokenKind::Compare(with_eq), i);
                } else {
                    i += 1;
                    push(&mut tokens, TokenKind::Compare(orderless), i);
                }
            }
            '&' | '|' => {
                if text.at(i + 1) == Some(ch) {
                    i += 2;
                    let kind = if ch == '&' {
                        TokenKind::AndAnd
                    } else {
                        TokenKind::OrOr
                    };
                    push(&mut tokens, kind, i);
                } else {
                    i += 1;
                    push(&mut tokens, TokenKind::Unknown, i);
                }
            }
            '\'' | '"' => {
                let quote = ch;
                i += 1;
                let mut closed = false;
                while i < text.len() {
                    let current = text.chars[i];
                    if current == '\\' && i + 1 < text.len() {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    if current == quote {
                        closed = true;
                        break;
                    }
                }
                let kind = if closed {
                    TokenKind::String
                } else {
                    TokenKind::UnterminatedString
                };
                push(&mut tokens, kind, i);
            }
            c if c.is_ascii_digit()
                || (c == '-' && text.at(i + 1).is_some_and(|n| n.is_ascii_digit())) =>
            {
                i += 1;
                while text.at(i).is_some_and(|n| n.is_ascii_digit()) {
                    i += 1;
                }
                let mut valid = true;
                if text.at(i) == Some('.') {
                    if text.at(i + 1).is_some_and(|n| n.is_ascii_digit()) {
                        i += 1;
                        while text.at(i).is_some_and(|n| n.is_ascii_digit()) {
                            i += 1;
                        }
                    } else {
                        i += 1;
                        valid = false;
                    }
                }
                if text.at(i).is_some_and(is_word) {
                    valid = false;
                    while text.at(i).is_some_and(is_word) {
                        i += 1;
                    }
                }
                let kind = if valid {
                    TokenKind::Number
                } else {
                    TokenKind::InvalidNumber
                };
                push(&mut tokens, kind, i);
            }
            c if is_word_start(c) => {
                i += 1;
                while text.at(i).is_some_and(is_word) {
                    i += 1;
                }
                while text.at(i) == Some('.') && text.at(i + 1).is_some_and(is_word) {
                    i += 1;
                    while text.at(i).is_some_and(is_word) {
                        i += 1;
                    }
                }
                let run = text.slice(start, i);
                if matches!(run.as_str(), "cfg" | "session" | "ags" | "option")
                    && text.at(i) == Some(':')
                {
                    i += 1;
                    while text.at(i).is_some_and(is_ref_body) {
                        i += 1;
                    }
                    push(&mut tokens, TokenKind::Reference, i);
                } else {
                    let kind = match run.as_str() {
                        "true" => TokenKind::True,
                        "false" => TokenKind::False,
                        "in" => TokenKind::Compare(CmpOp::In),
                        "contains" => TokenKind::Compare(CmpOp::Contains),
                        _ => TokenKind::Ident,
                    };
                    push(&mut tokens, kind, i);
                }
            }
            _ => {
                i += 1;
                push(&mut tokens, TokenKind::Unknown, i);
            }
        }
    }
    tokens
}

/// Decodes the escapes `\\ \' \" \n \t`; other backslash sequences are kept verbatim.
fn decode_string(raw: &str) -> String {
    let mut chars = raw.chars();
    chars.next();
    let mut inner = chars.collect::<Vec<_>>();
    if inner.last().is_some() {
        inner.pop();
    }
    let mut out = String::new();
    let mut iter = inner.into_iter();
    while let Some(ch) = iter.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match iter.next() {
            Some('\\') => out.push('\\'),
            Some('\'') => out.push('\''),
            Some('"') => out.push('"'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------------------------

const RESERVED_IDS: [&str; 4] = ["true", "false", "in", "contains"];

/// Maximum nesting of `(` and `!` in one expression (the runtime's `GMaxExpressionNesting`).
/// The 65th level is the parse error `too-deep`, positioned at the token that exceeds the limit.
pub const MAX_NESTING: u32 = 64;

/// `[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*`, the config id grammar.
pub fn is_config_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('.').all(|segment| {
            let mut chars = segment.chars();
            chars.next().is_some_and(is_word_start) && chars.all(is_word)
        })
}

/// Whole id is one of the words the expression grammar reserves.
pub fn is_reserved_id(id: &str) -> bool {
    RESERVED_IDS.contains(&id)
}

/// Pack part of `cfg:Pack:id` and setting keys: `[A-Za-z0-9_.-]+`.
pub fn is_pack_ref_text(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    end: u32,
    /// Currently open `(` and `!` levels.
    depth: u32,
}

type ParseResult<T> = Result<T, ParseError>;

fn error(code: ErrorCode, at: u32, message: impl Into<String>) -> ParseError {
    ParseError {
        code,
        at,
        message: message.into(),
    }
}

impl<'a> Parser<'a> {
    /// Next token; lexical error tokens are raised as soon as the parser looks at them.
    fn peek(&self) -> ParseResult<Option<&'a Token>> {
        let Some(token) = self.tokens.get(self.pos) else {
            return Ok(None);
        };
        match token.kind {
            TokenKind::Unknown => Err(error(
                ErrorCode::UnexpectedChar,
                token.span.start,
                format!("unexpected character {:?}", token.text),
            )),
            TokenKind::InvalidNumber => Err(error(
                ErrorCode::InvalidNumber,
                token.span.start,
                format!("invalid number {:?}", token.text),
            )),
            TokenKind::UnterminatedString => Err(error(
                ErrorCode::UnterminatedString,
                token.span.start,
                "string literal is not closed",
            )),
            _ => Ok(Some(token)),
        }
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    fn at_end(&self, token: Option<&Token>) -> u32 {
        token.map_or(self.end, |token| token.span.start)
    }

    fn parse_or(&mut self) -> ParseResult<Expr> {
        let first = self.parse_and()?;
        let mut items = vec![first];
        while matches!(self.peek()?, Some(token) if token.kind == TokenKind::OrOr) {
            self.bump();
            items.push(self.parse_and()?);
        }
        Ok(join_items(items, ExprKind::Or))
    }

    fn parse_and(&mut self) -> ParseResult<Expr> {
        let first = self.parse_cmp()?;
        let mut items = vec![first];
        while matches!(self.peek()?, Some(token) if token.kind == TokenKind::AndAnd) {
            self.bump();
            items.push(self.parse_cmp()?);
        }
        Ok(join_items(items, ExprKind::And))
    }

    fn parse_cmp(&mut self) -> ParseResult<Expr> {
        let left = self.parse_unary()?;
        let Some(token) = self.peek()? else {
            return Ok(left);
        };
        let TokenKind::Compare(op) = token.kind else {
            return Ok(left);
        };
        self.bump();
        let right = self.parse_unary()?;
        if let Some(next) = self.peek()? {
            if matches!(next.kind, TokenKind::Compare(_)) {
                return Err(error(
                    ErrorCode::ChainedComparison,
                    next.span.start,
                    "comparisons cannot be chained; combine them with && or ||",
                ));
            }
        }
        let span = left.span.join(right.span);
        Ok(Expr {
            span,
            kind: ExprKind::Compare {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
        })
    }

    fn too_deep(token: &Token) -> ParseError {
        error(
            ErrorCode::TooDeep,
            token.span.start,
            format!("expression is nested deeper than {MAX_NESTING} levels"),
        )
    }

    fn parse_unary(&mut self) -> ParseResult<Expr> {
        if let Some(token) = self.peek()? {
            if token.kind == TokenKind::Not {
                if self.depth >= MAX_NESTING {
                    return Err(Self::too_deep(token));
                }
                self.depth += 1;
                self.bump();
                let inner = self.parse_unary();
                self.depth -= 1;
                let inner = inner?;
                let span = Span::new(token.span.start, inner.span.end);
                return Ok(Expr {
                    span,
                    kind: ExprKind::Not(Box::new(inner)),
                });
            }
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> ParseResult<Expr> {
        let token = self.peek()?;
        let Some(token) = token else {
            return Err(error(
                ErrorCode::ExpectedOperand,
                self.end,
                "expected an operand",
            ));
        };
        match token.kind {
            TokenKind::LParen => {
                if self.depth >= MAX_NESTING {
                    return Err(Self::too_deep(token));
                }
                self.depth += 1;
                self.bump();
                let inner = self.parse_or();
                self.depth -= 1;
                let inner = inner?;
                match self.peek()? {
                    Some(close) if close.kind == TokenKind::RParen => {
                        self.bump();
                        Ok(inner)
                    }
                    other => Err(error(
                        ErrorCode::ExpectedCloseParen,
                        self.at_end(other),
                        "expected ')'",
                    )),
                }
            }
            TokenKind::LBracket => self.parse_list(token),
            TokenKind::Number => {
                self.bump();
                let number = token.text.parse::<f64>().map_err(|_| {
                    error(
                        ErrorCode::InvalidNumber,
                        token.span.start,
                        format!("invalid number {:?}", token.text),
                    )
                })?;
                Ok(Expr {
                    span: token.span,
                    kind: ExprKind::Literal(Literal::Number(number)),
                })
            }
            TokenKind::String => {
                self.bump();
                Ok(Expr {
                    span: token.span,
                    kind: ExprKind::Literal(Literal::Str(decode_string(&token.text))),
                })
            }
            TokenKind::True | TokenKind::False => {
                self.bump();
                Ok(Expr {
                    span: token.span,
                    kind: ExprKind::Literal(Literal::Bool(token.kind == TokenKind::True)),
                })
            }
            TokenKind::Ident => {
                self.bump();
                Ok(Expr {
                    span: token.span,
                    kind: ExprKind::Operand(Operand {
                        kind: OperandKind::Config {
                            pack: None,
                            id: token.text.clone(),
                        },
                        span: token.span,
                    }),
                })
            }
            TokenKind::Reference => {
                self.bump();
                let operand = parse_reference(token)?;
                Ok(Expr {
                    span: token.span,
                    kind: ExprKind::Operand(operand),
                })
            }
            _ => Err(error(
                ErrorCode::ExpectedOperand,
                token.span.start,
                "expected an operand",
            )),
        }
    }

    fn parse_list(&mut self, open: &Token) -> ParseResult<Expr> {
        self.bump();
        let mut items = Vec::new();
        let close_end;
        loop {
            let token = self.peek()?;
            let Some(token) = token else {
                let code = if items.is_empty() {
                    ErrorCode::ExpectedCloseBracket
                } else {
                    ErrorCode::ExpectedOperand
                };
                return Err(error(code, self.end, "expected a list element or ']'"));
            };
            if items.is_empty() && token.kind == TokenKind::RBracket {
                self.bump();
                close_end = token.span.end;
                break;
            }
            let literal = match token.kind {
                TokenKind::Number => Literal::Number(token.text.parse::<f64>().map_err(|_| {
                    error(
                        ErrorCode::InvalidNumber,
                        token.span.start,
                        format!("invalid number {:?}", token.text),
                    )
                })?),
                TokenKind::String => Literal::Str(decode_string(&token.text)),
                TokenKind::True => Literal::Bool(true),
                TokenKind::False => Literal::Bool(false),
                TokenKind::Ident
                | TokenKind::Reference
                | TokenKind::LBracket
                | TokenKind::LParen => {
                    return Err(error(
                        ErrorCode::ListElementNotLiteral,
                        token.span.start,
                        "list elements must be literals",
                    ));
                }
                _ => {
                    return Err(error(
                        ErrorCode::ExpectedOperand,
                        token.span.start,
                        "expected a list element",
                    ));
                }
            };
            self.bump();
            items.push(ListItem {
                literal,
                span: token.span,
            });
            match self.peek()? {
                Some(next) if next.kind == TokenKind::Comma => self.bump(),
                Some(next) if next.kind == TokenKind::RBracket => {
                    self.bump();
                    close_end = next.span.end;
                    break;
                }
                other => {
                    return Err(error(
                        ErrorCode::ExpectedCloseBracket,
                        self.at_end(other),
                        "expected ',' or ']'",
                    ));
                }
            }
        }
        Ok(Expr {
            span: Span::new(open.span.start, close_end),
            kind: ExprKind::List(items),
        })
    }
}

fn join_items(mut items: Vec<Expr>, make: fn(Vec<Expr>) -> ExprKind) -> Expr {
    if items.len() == 1 {
        return items.remove(0);
    }
    let span = items
        .iter()
        .skip(1)
        .fold(items[0].span, |span, item| span.join(item.span));
    Expr {
        span,
        kind: make(items),
    }
}

fn parse_reference(token: &Token) -> ParseResult<Operand> {
    let invalid = |message: &str| error(ErrorCode::InvalidRef, token.span.start, message);
    let (provider, rest) = token
        .text
        .split_once(':')
        .ok_or_else(|| invalid("reference needs a prefix"))?;
    if rest.is_empty() {
        return Err(invalid("reference has no id after the prefix"));
    }
    let body_start = token.span.start + provider.len() as u32 + 1;
    if provider == "cfg" {
        let parts = rest.split(':').collect::<Vec<_>>();
        let (pack, id, id_offset) = match parts.as_slice() {
            [id] => (None, *id, body_start),
            [pack, id] => (Some(*pack), *id, body_start + pack.len() as u32 + 1),
            _ => return Err(invalid("expected cfg:<id> or cfg:<pack>:<id>")),
        };
        if let Some(pack) = pack {
            if !is_pack_ref_text(pack) {
                return Err(invalid("invalid pack reference"));
            }
        }
        if id.is_empty() {
            return Err(invalid("reference has no id"));
        }
        if is_reserved_id(id) {
            return Err(error(
                ErrorCode::ReservedId,
                id_offset,
                format!("{id:?} is a reserved word"),
            ));
        }
        if !is_config_id(id) {
            return Err(invalid("invalid config id"));
        }
        return Ok(Operand {
            kind: OperandKind::Config {
                pack: pack.map(str::to_owned),
                id: id.to_owned(),
            },
            span: token.span,
        });
    }
    if rest.contains(':') || !is_pack_ref_text(rest) {
        return Err(invalid("invalid setting id"));
    }
    Ok(Operand {
        kind: OperandKind::Setting {
            provider: provider.to_owned(),
            key: rest.to_owned(),
        },
        span: token.span,
    })
}

/// Parses a whole expression. Only the first error is reported.
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let tokens = tokenize(src);
    let end = Chars::new(src).total();
    if tokens.is_empty() {
        return Err(error(ErrorCode::Empty, 0, "expression is empty"));
    }
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
        end,
        depth: 0,
    };
    let expr = parser.parse_or()?;
    if let Some(token) = parser.peek()? {
        return Err(error(
            ErrorCode::TrailingInput,
            token.span.start,
            format!("unexpected {:?} after the expression", token.text),
        ));
    }
    Ok(expr)
}

/// All operands in source order.
pub fn operands(expr: &Expr) -> Vec<Operand> {
    let mut out = Vec::new();
    collect_operands(expr, &mut out);
    out
}

fn collect_operands(expr: &Expr, out: &mut Vec<Operand>) {
    match &expr.kind {
        ExprKind::Or(items) | ExprKind::And(items) => {
            items.iter().for_each(|item| collect_operands(item, out))
        }
        ExprKind::Not(inner) => collect_operands(inner, out),
        ExprKind::Compare { left, right, .. } => {
            collect_operands(left, out);
            collect_operands(right, out);
        }
        ExprKind::Operand(operand) => out.push(operand.clone()),
        ExprKind::Literal(_) | ExprKind::List(_) => {}
    }
}

// ---------------------------------------------------------------------------------------------
// Type check
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum ExprType {
    /// Unknown type; never produces issues.
    Unknown,
    Bool,
    Number,
    String,
    List(Box<ExprType>),
}

impl ExprType {
    /// `bool`, `number`, `string`, `unknown`, `list<T>`.
    pub fn parse(label: &str) -> Option<ExprType> {
        match label {
            "bool" => Some(ExprType::Bool),
            "number" => Some(ExprType::Number),
            "string" => Some(ExprType::String),
            "unknown" => Some(ExprType::Unknown),
            other => {
                let inner = other.strip_prefix("list<")?.strip_suffix('>')?;
                Some(ExprType::List(Box::new(ExprType::parse(inner)?)))
            }
        }
    }
    pub fn label(&self) -> String {
        match self {
            ExprType::Unknown => "unknown".into(),
            ExprType::Bool => "bool".into(),
            ExprType::Number => "number".into(),
            ExprType::String => "string".into(),
            ExprType::List(inner) => format!("list<{}>", inner.label()),
        }
    }
    fn is_list(&self) -> bool {
        matches!(self, ExprType::List(_))
    }
    /// Types match when either side is unknown or both are the same class.
    fn matches(&self, other: &ExprType) -> bool {
        match (self, other) {
            (ExprType::Unknown, _) | (_, ExprType::Unknown) => true,
            (ExprType::List(a), ExprType::List(b)) => a.matches(b),
            (a, b) => a == b,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum TypeIssueCode {
    CompareMismatch,
    OrderNonNumber,
    ContainsNonList,
    InNonList,
    ListElementMismatch,
    UnknownId,
}

impl TypeIssueCode {
    pub fn as_str(self) -> &'static str {
        match self {
            TypeIssueCode::CompareMismatch => "compare-mismatch",
            TypeIssueCode::OrderNonNumber => "order-non-number",
            TypeIssueCode::ContainsNonList => "contains-non-list",
            TypeIssueCode::InNonList => "in-non-list",
            TypeIssueCode::ListElementMismatch => "list-element-mismatch",
            TypeIssueCode::UnknownId => "unknown-id",
        }
    }
}

/// Issue span = the comparison, the list literal or the operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct TypeIssue {
    pub code: TypeIssueCode,
    pub span: Span,
}

/// Type-checks `expr`. `operand_type` returns `None` for an unknown id (reported as
/// `unknown-id` on the operand span) or the operand type (`ExprType::Unknown` is permissive).
pub fn type_check(
    expr: &Expr,
    operand_type: &dyn Fn(&Operand) -> Option<ExprType>,
) -> Vec<TypeIssue> {
    let mut issues = Vec::new();
    infer(expr, operand_type, &mut issues);
    issues
}

fn literal_type(literal: &Literal) -> ExprType {
    match literal {
        Literal::Number(_) => ExprType::Number,
        Literal::Str(_) => ExprType::String,
        Literal::Bool(_) => ExprType::Bool,
    }
}

fn infer(
    expr: &Expr,
    operand_type: &dyn Fn(&Operand) -> Option<ExprType>,
    issues: &mut Vec<TypeIssue>,
) -> ExprType {
    match &expr.kind {
        ExprKind::Or(items) | ExprKind::And(items) => {
            for item in items {
                infer(item, operand_type, issues);
            }
            ExprType::Bool
        }
        ExprKind::Not(inner) => {
            infer(inner, operand_type, issues);
            ExprType::Bool
        }
        ExprKind::Operand(operand) => match operand_type(operand) {
            Some(kind) => kind,
            None => {
                issues.push(TypeIssue {
                    code: TypeIssueCode::UnknownId,
                    span: operand.span,
                });
                ExprType::Unknown
            }
        },
        ExprKind::Literal(literal) => literal_type(literal),
        ExprKind::List(items) => {
            let mut element: Option<ExprType> = None;
            let mut mixed = false;
            for item in items {
                let kind = literal_type(&item.literal);
                match &element {
                    None => element = Some(kind),
                    Some(existing) if *existing == kind => {}
                    Some(_) => mixed = true,
                }
            }
            if mixed {
                issues.push(TypeIssue {
                    code: TypeIssueCode::ListElementMismatch,
                    span: expr.span,
                });
                return ExprType::List(Box::new(ExprType::Unknown));
            }
            ExprType::List(Box::new(element.unwrap_or(ExprType::Unknown)))
        }
        ExprKind::Compare { op, left, right } => {
            let left_type = infer(left, operand_type, issues);
            let right_type = infer(right, operand_type, issues);
            let mut report = |code: TypeIssueCode| {
                issues.push(TypeIssue {
                    code,
                    span: expr.span,
                });
            };
            match op {
                CmpOp::Eq | CmpOp::Ne => {
                    if left_type.is_list()
                        || right_type.is_list()
                        || !left_type.matches(&right_type)
                    {
                        report(TypeIssueCode::CompareMismatch);
                    }
                }
                CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge => {
                    let numeric =
                        |kind: &ExprType| matches!(kind, ExprType::Number | ExprType::Unknown);
                    if !numeric(&left_type) || !numeric(&right_type) {
                        report(TypeIssueCode::OrderNonNumber);
                    }
                }
                CmpOp::Contains => match &left_type {
                    ExprType::List(element) => {
                        if right_type.is_list() || !element.matches(&right_type) {
                            report(TypeIssueCode::ListElementMismatch);
                        }
                    }
                    ExprType::Unknown => {}
                    _ => report(TypeIssueCode::ContainsNonList),
                },
                CmpOp::In => match &right_type {
                    // A mixed list literal is typed `list<unknown>` and was already reported on the literal.
                    ExprType::List(element) => {
                        if left_type.is_list() || !element.matches(&left_type) {
                            report(TypeIssueCode::ListElementMismatch);
                        }
                    }
                    ExprType::Unknown => {}
                    _ => report(TypeIssueCode::InNonList),
                },
            }
            ExprType::Bool
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum EvalValue {
    Bool(bool),
    Number(f64),
    Str(String),
    List(Vec<EvalValue>),
}

impl EvalValue {
    /// Converts JSON (`true`, `3`, `"x"`, `["a", "b"]`); objects and `null` are not values.
    pub fn from_json(value: &Value) -> Option<EvalValue> {
        match value {
            Value::Bool(flag) => Some(EvalValue::Bool(*flag)),
            Value::Number(number) => number.as_f64().map(EvalValue::Number),
            Value::String(text) => Some(EvalValue::Str(text.clone())),
            Value::Array(items) => items
                .iter()
                .map(EvalValue::from_json)
                .collect::<Option<Vec<_>>>()
                .map(EvalValue::List),
            _ => None,
        }
    }
    /// `bool` itself, number != 0, non-empty string/list.
    pub fn truthy(&self) -> bool {
        match self {
            EvalValue::Bool(flag) => *flag,
            EvalValue::Number(number) => *number != 0.0,
            EvalValue::Str(text) => !text.is_empty(),
            EvalValue::List(items) => !items.is_empty(),
        }
    }
    fn class(&self) -> u8 {
        match self {
            EvalValue::Bool(_) => 0,
            EvalValue::Number(_) => 1,
            EvalValue::Str(_) => 2,
            EvalValue::List(_) => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum EvalErrorCode {
    UnknownId,
    TypeError,
}

impl EvalErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            EvalErrorCode::UnknownId => "unknown-id",
            EvalErrorCode::TypeError => "type-error",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EvalError {
    pub code: EvalErrorCode,
    pub span: Span,
    pub message: String,
}

/// Evaluates `expr` with short-circuit `&&`/`||`. Unknown ids and type errors are errors
/// (the runtime treats them as "condition false" plus an error diagnostic).
pub fn eval(
    expr: &Expr,
    operand_value: &dyn Fn(&Operand) -> Option<EvalValue>,
) -> Result<bool, EvalError> {
    eval_value(expr, operand_value).map(|value| value.truthy())
}

fn eval_value(
    expr: &Expr,
    operand_value: &dyn Fn(&Operand) -> Option<EvalValue>,
) -> Result<EvalValue, EvalError> {
    match &expr.kind {
        ExprKind::Or(items) => {
            for item in items {
                if eval_value(item, operand_value)?.truthy() {
                    return Ok(EvalValue::Bool(true));
                }
            }
            Ok(EvalValue::Bool(false))
        }
        ExprKind::And(items) => {
            for item in items {
                if !eval_value(item, operand_value)?.truthy() {
                    return Ok(EvalValue::Bool(false));
                }
            }
            Ok(EvalValue::Bool(true))
        }
        ExprKind::Not(inner) => Ok(EvalValue::Bool(!eval_value(inner, operand_value)?.truthy())),
        ExprKind::Operand(operand) => operand_value(operand).ok_or_else(|| EvalError {
            code: EvalErrorCode::UnknownId,
            span: operand.span,
            message: format!("unknown config value {}", operand.canonical()),
        }),
        ExprKind::Literal(literal) => Ok(match literal {
            Literal::Number(number) => EvalValue::Number(*number),
            Literal::Str(text) => EvalValue::Str(text.clone()),
            Literal::Bool(flag) => EvalValue::Bool(*flag),
        }),
        ExprKind::List(items) => Ok(EvalValue::List(
            items
                .iter()
                .map(|item| match &item.literal {
                    Literal::Number(number) => EvalValue::Number(*number),
                    Literal::Str(text) => EvalValue::Str(text.clone()),
                    Literal::Bool(flag) => EvalValue::Bool(*flag),
                })
                .collect(),
        )),
        ExprKind::Compare { op, left, right } => {
            let left_value = eval_value(left, operand_value)?;
            let right_value = eval_value(right, operand_value)?;
            let type_error = |message: &str| EvalError {
                code: EvalErrorCode::TypeError,
                span: expr.span,
                message: message.to_owned(),
            };
            let result = match op {
                CmpOp::Eq | CmpOp::Ne => {
                    if left_value.class() == 3 || right_value.class() == 3 {
                        return Err(type_error("lists cannot be compared with == or !="));
                    }
                    if left_value.class() != right_value.class() {
                        return Err(type_error("operands of == and != must have the same type"));
                    }
                    let equal = left_value == right_value;
                    if *op == CmpOp::Eq {
                        equal
                    } else {
                        !equal
                    }
                }
                CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge => {
                    let (EvalValue::Number(a), EvalValue::Number(b)) = (&left_value, &right_value)
                    else {
                        return Err(type_error("< <= > >= compare numbers only"));
                    };
                    match op {
                        CmpOp::Lt => a < b,
                        CmpOp::Le => a <= b,
                        CmpOp::Gt => a > b,
                        _ => a >= b,
                    }
                }
                CmpOp::Contains => {
                    let EvalValue::List(items) = &left_value else {
                        return Err(type_error("contains needs a list on the left"));
                    };
                    list_contains(items, &right_value).map_err(|message| type_error(&message))?
                }
                CmpOp::In => {
                    let EvalValue::List(items) = &right_value else {
                        return Err(type_error("in needs a list on the right"));
                    };
                    list_contains(items, &left_value).map_err(|message| type_error(&message))?
                }
            };
            Ok(EvalValue::Bool(result))
        }
    }
}

fn list_contains(items: &[EvalValue], needle: &EvalValue) -> Result<bool, String> {
    if needle.class() == 3 {
        return Err("a list cannot be a list element".into());
    }
    if let Some(first) = items.first() {
        if first.class() != needle.class() {
            return Err("element type does not match the list".into());
        }
    }
    Ok(items.iter().any(|item| item == needle))
}

// ---------------------------------------------------------------------------------------------
// Cursor state (completion support)
// ---------------------------------------------------------------------------------------------

/// What the grammar allows next at the cursor.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Expect {
    /// Start of an operand: id, `cfg:`/`session:`/`ags:`/`option:` reference, literal, `(` or `!`.
    Operand,
    /// Right-hand side of a comparison (operand or literal); `CursorState::operator` is set.
    Value,
    /// A literal inside a list literal.
    ListElement,
    /// After a complete operand: `&&`, `||`, a comparison operator, `)`.
    Operator,
    /// Inside a string literal (`CursorState::in_string` holds the quote).
    String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum WordKind {
    /// Identifier or keyword being typed (`feat`).
    Ident,
    /// `cfg:`/`session:`/`ags:`/`option:` reference being typed (`cfg:Oth`).
    Reference,
    Number,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CursorState {
    pub expect: Expect,
    /// Token text before the cursor (empty between tokens).
    pub word: String,
    /// Whole token touching the cursor (empty span at the cursor when `word` is empty).
    pub word_span: Span,
    pub word_kind: Option<WordKind>,
    /// For a reference being typed: provider (`cfg`, `session`, `ags`, `option`).
    pub provider: Option<String>,
    /// For `cfg:Pack:|`: the pack already typed.
    pub pack: Option<String>,
    /// Comparison operator before the cursor (set for `Expect::Value`).
    pub operator: Option<CmpOp>,
    /// Operand left of the operator (`Expect::Value`) or left of the cursor (`Expect::Operator`).
    pub left: Option<Operand>,
    pub in_list: bool,
    /// Quote character when the cursor is inside a string literal.
    pub in_string: Option<char>,
}

/// Grammar state at `offset` (UTF-16) of `src`. Works on incomplete input.
pub fn cursor_state(src: &str, offset: u32) -> CursorState {
    let tokens = join_trailing_dots(tokenize(src));
    let mut state = CursorState {
        expect: Expect::Operand,
        word: String::new(),
        word_span: Span::new(offset, offset),
        word_kind: None,
        provider: None,
        pack: None,
        operator: None,
        left: None,
        in_list: false,
        in_string: None,
    };
    // A token the cursor is inside of (or touches, for words).
    let mut current: Option<&Token> = None;
    for token in &tokens {
        match token.kind {
            TokenKind::String | TokenKind::UnterminatedString => {
                let closed = token.kind == TokenKind::String;
                let inside = token.span.start < offset && (offset < token.span.end || !closed);
                if inside && offset <= token.span.end {
                    state.expect = Expect::String;
                    state.in_string = token.text.chars().next();
                    state.word_span = token.span;
                    return state;
                }
            }
            TokenKind::Ident
            | TokenKind::Reference
            | TokenKind::Number
            | TokenKind::InvalidNumber
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Compare(CmpOp::In | CmpOp::Contains)
                if token.span.start < offset && offset <= token.span.end =>
            {
                current = Some(token);
            }
            _ => {}
        }
    }
    let word_start = current.map_or(offset, |token| token.span.start);
    if let Some(token) = current {
        let typed = utf16_prefix(&token.text, offset - token.span.start);
        state.word = typed;
        state.word_span = token.span;
        state.word_kind = Some(match token.kind {
            TokenKind::Reference => WordKind::Reference,
            TokenKind::Number | TokenKind::InvalidNumber => WordKind::Number,
            _ => WordKind::Ident,
        });
        if token.kind == TokenKind::Reference {
            if let Some((provider, rest)) = state.word.split_once(':') {
                state.provider = Some(provider.to_owned());
                if provider == "cfg" {
                    if let Some((pack, _)) = rest.split_once(':') {
                        state.pack = Some(pack.to_owned());
                    }
                }
            }
        }
    }
    let before = tokens
        .iter()
        .filter(|token| token.span.end <= word_start)
        .collect::<Vec<_>>();
    let mut brackets = 0i32;
    for token in &before {
        match token.kind {
            TokenKind::LBracket => brackets += 1,
            TokenKind::RBracket => brackets -= 1,
            _ => {}
        }
    }
    state.in_list = brackets > 0;
    let previous = before.last();
    match previous.map(|token| token.kind) {
        None | Some(TokenKind::Not | TokenKind::AndAnd | TokenKind::OrOr | TokenKind::LParen) => {
            state.expect = Expect::Operand
        }
        Some(TokenKind::LBracket | TokenKind::Comma) => {
            state.expect = if state.in_list {
                Expect::ListElement
            } else {
                Expect::Operand
            };
        }
        Some(TokenKind::Compare(op)) => {
            state.expect = Expect::Value;
            state.operator = Some(op);
            if before.len() >= 2 {
                state.left = operand_of(before[before.len() - 2]);
            }
        }
        Some(_) => {
            state.expect = if state.in_list {
                Expect::ListElement
            } else {
                Expect::Operator
            };
            if state.expect == Expect::Operator {
                state.left = previous.and_then(|token| operand_of(token));
            }
        }
    }
    state
}

/// A `.` right behind an identifier (`features.|`, typed on the way to `features.tier`) is not a
/// lexical error for the cursor: it continues the identifier. The parser keeps rejecting it.
fn join_trailing_dots(tokens: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    for token in tokens {
        if token.kind == TokenKind::Unknown && token.text == "." {
            if let Some(previous) = out.last_mut() {
                if previous.kind == TokenKind::Ident && previous.span.end == token.span.start {
                    previous.span.end = token.span.end;
                    previous.text.push('.');
                    continue;
                }
            }
        }
        out.push(token);
    }
    out
}

fn operand_of(token: &Token) -> Option<Operand> {
    match token.kind {
        TokenKind::Ident => Some(Operand {
            kind: OperandKind::Config {
                pack: None,
                id: token.text.clone(),
            },
            span: token.span,
        }),
        TokenKind::Reference => parse_reference(token).ok(),
        _ => None,
    }
}

fn utf16_prefix(text: &str, units: u32) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizer_is_total_and_tracks_utf16() {
        let tokens = tokenize("\"😀\" == #");
        assert_eq!(tokens[0].span, Span::new(0, 4));
        assert_eq!(tokens[1].span, Span::new(5, 7));
        assert_eq!(tokens[2].kind, TokenKind::Unknown);
        assert_eq!(tokens[2].span, Span::new(8, 9));
    }

    #[test]
    fn keywords_only_match_whole_runs() {
        let tokens = tokenize("in.x in true.x true");
        assert_eq!(tokens[0].kind, TokenKind::Ident);
        assert_eq!(tokens[1].kind, TokenKind::Compare(CmpOp::In));
        assert_eq!(tokens[2].kind, TokenKind::Ident);
        assert_eq!(tokens[3].kind, TokenKind::True);
    }

    #[test]
    fn cursor_states() {
        let state = cursor_state("features.value >= ", 18);
        assert_eq!(state.expect, Expect::Value);
        assert_eq!(state.operator, Some(CmpOp::Ge));
        assert_eq!(
            state.left.map(|operand| operand.key()),
            Some("features.value".to_owned())
        );
        let state = cursor_state("a && cfg:Oth", 12);
        assert_eq!(state.expect, Expect::Operand);
        assert_eq!(state.word, "cfg:Oth");
        assert_eq!(state.provider.as_deref(), Some("cfg"));
        let state = cursor_state("mode == 'ha", 11);
        assert_eq!(state.expect, Expect::String);
        assert_eq!(state.in_string, Some('\''));
        let state = cursor_state("flag ", 5);
        assert_eq!(state.expect, Expect::Operator);
        assert_eq!(state.left.map(|operand| operand.key()), Some("flag".into()));
        let state = cursor_state("x in [1, ", 9);
        assert_eq!(state.expect, Expect::ListElement);
        assert!(state.in_list);
    }

    /// Regression: `features.|` (an id typed up to its dot) was an operator position because the
    /// dot is a lexical error token; it must be the identifier being typed.
    #[test]
    fn identifier_typed_up_to_a_dot_stays_an_identifier() {
        for (src, offset, word) in [
            ("features.", 9, "features."),
            ("a && features.", 14, "a && features.".split_at(5).1),
            ("!features.tier.", 15, "features.tier."),
            ("features.ti", 11, "features.ti"),
        ] {
            let state = cursor_state(src, offset);
            assert_eq!(state.expect, Expect::Operand, "{src}");
            assert_eq!(state.word, word, "{src}");
            assert_eq!(state.word_kind, Some(WordKind::Ident), "{src}");
            assert_eq!(state.word_span.end, offset, "{src}");
            assert!(state.left.is_none(), "{src}");
        }
        // After a comparison it is the value position.
        let state = cursor_state("x == features.", 14);
        assert_eq!(state.expect, Expect::Value);
        assert_eq!(state.word, "features.");
        // A dot that is not glued to an identifier stays a plain error token.
        let state = cursor_state("a . ", 4);
        assert_eq!(state.expect, Expect::Operator);
        // The parser still rejects the half-typed id.
        assert_eq!(
            parse("features.").unwrap_err().code,
            ErrorCode::UnexpectedChar
        );
    }

    /// Tiny deterministic generator so the fuzz-style tests need no extra crate.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
            items[(self.next() as usize) % items.len()]
        }
    }

    const PIECES: [&str; 34] = [
        "a",
        "b.c",
        "cfg:",
        "cfg:P:",
        "session:",
        "x1",
        "1",
        "2.5",
        "-3",
        "1.",
        "'",
        "\"",
        "\\",
        "(",
        ")",
        "[",
        "]",
        ",",
        "!",
        "!=",
        "==",
        "<",
        ">=",
        "&&",
        "||",
        "in",
        "contains",
        "true",
        " ",
        "\t",
        "\u{1F600}",
        "\u{e9}",
        "#",
        "=",
    ];

    #[test]
    fn arbitrary_input_never_panics_and_spans_stay_in_bounds() {
        let mut random = Lcg(7);
        for _ in 0..4000 {
            let length = random.next() % 12;
            let source = (0..length)
                .map(|_| random.pick(&PIECES))
                .collect::<String>();
            let total = source.encode_utf16().count() as u32;
            for token in tokenize(&source) {
                assert!(
                    token.span.start <= token.span.end && token.span.end <= total,
                    "{source:?}"
                );
            }
            match parse(&source) {
                Ok(expr) => {
                    assert!(expr.span.end <= total, "{source:?}");
                    for operand in operands(&expr) {
                        assert!(operand.span.end <= total, "{source:?}");
                    }
                    let _ = type_check(&expr, &|_| Some(ExprType::Unknown));
                    let _ = eval(&expr, &|_| None);
                }
                Err(error) => assert!(error.at <= total, "{source:?}: {error:?}"),
            }
            let mut offset = 0u32;
            for ch in source.chars() {
                let state = cursor_state(&source, offset);
                assert!(state.word_span.end <= total, "{source:?}@{offset}");
                offset += ch.len_utf16() as u32;
            }
            let end = cursor_state(&source, total);
            assert!(end.word_span.end <= total, "{source:?}");
        }
    }

    #[test]
    fn string_escapes_decode_and_unknown_escapes_stay_verbatim() {
        let expr = parse("'a\\'b' == \"q\\\"x\\n\"").unwrap();
        let ExprKind::Compare { left, right, .. } = &expr.kind else {
            panic!("{expr:?}");
        };
        assert_eq!(left.kind, ExprKind::Literal(Literal::Str("a'b".into())));
        assert_eq!(right.kind, ExprKind::Literal(Literal::Str("q\"x\n".into())));
        let odd = parse("x == '\\q'").unwrap();
        let ExprKind::Compare { right, .. } = &odd.kind else {
            panic!();
        };
        assert_eq!(right.kind, ExprKind::Literal(Literal::Str("\\q".into())));
    }

    #[test]
    fn nesting_is_limited_to_64_levels_of_parens_and_negations() {
        let limit = MAX_NESTING as usize;
        // Exactly 64 levels parse, whatever mix of `!` and `(` opens them.
        assert!(parse(&format!("{}a", "!".repeat(limit))).is_ok());
        assert!(parse(&format!("{}a{}", "(".repeat(limit), ")".repeat(limit))).is_ok());
        assert!(parse(&format!(
            "{}a{}",
            "!(".repeat(limit / 2),
            ")".repeat(limit / 2)
        ))
        .is_ok());
        // The token that opens level 65 is the error, at its own offset.
        for src in [
            format!("{}a", "!".repeat(limit + 1)),
            format!("{}a{}", "(".repeat(limit + 1), ")".repeat(limit + 1)),
            format!(
                "{}a{}",
                "!(".repeat(limit / 2 + 1),
                ")".repeat(limit / 2 + 1)
            ),
        ] {
            let error = parse(&src).unwrap_err();
            assert_eq!(error.code, ErrorCode::TooDeep, "{src}");
            assert_eq!(error.at as usize, limit, "{src}");
            assert_eq!(error.code.as_str(), "too-deep");
        }
        // Closed levels are released: a long flat chain of groups stays valid.
        assert!(parse(&vec!["(!a)"; 500].join(" && ")).is_ok());
    }

    #[test]
    fn prefixes_and_keywords_are_case_sensitive() {
        for id in ["True", "FALSE", "In", "Contains", "IN", "CONTAINS"] {
            let expr = parse(&format!("{id} == 1")).unwrap();
            let operands = operands(&expr);
            assert_eq!(operands.len(), 1, "{id}");
            assert_eq!(operands[0].key(), id);
            assert!(!is_reserved_id(id), "{id}");
            assert!(parse(&format!("cfg:{id}")).is_ok(), "{id}");
        }
        for id in ["true", "false", "in", "contains"] {
            assert!(is_reserved_id(id), "{id}");
            assert_eq!(
                parse(&format!("cfg:{id}")).unwrap_err().code,
                ErrorCode::ReservedId
            );
        }
        // Wrong-case prefixes are not references.
        for src in ["Cfg:x", "CFG:x", "Session:K", "AGS:K", "Option:K"] {
            let tokens = tokenize(src);
            assert!(
                tokens.iter().all(|t| t.kind != TokenKind::Reference),
                "{src}"
            );
            assert_eq!(parse(src).unwrap_err().code, ErrorCode::UnexpectedChar);
        }
    }

    #[test]
    fn type_labels_round_trip() {
        for label in [
            "bool",
            "number",
            "string",
            "unknown",
            "list<string>",
            "list<list<number>>",
        ] {
            assert_eq!(ExprType::parse(label).unwrap().label(), label);
        }
        assert!(ExprType::parse("list<").is_none());
        assert!(ExprType::parse("float").is_none());
    }
}
