//! Typed reading of exported CDO defaults.
//!
//! A game export stores every default as Unreal ImportText (`"8.000000"`, `"True"`,
//! `((ItemClass="/Script/Engine.BlueprintGeneratedClass'/Game/A.A_C'",amount=9))`). Synthetic and
//! older exports store structured JSON instead. [`typed_default`] turns both into the structured
//! form selectors compare against. Text it cannot read with certainty yields `None` (unknown);
//! it never guesses a value.
use serde_json::{Map, Number, Value};

const MAX_TEXT: usize = 256 * 1024;
const MAX_DEPTH: usize = 32;

/// Structured value of one exported default.
///
/// `kind` and `inner_kind` are the exported property kind and, for arrays and sets, the element
/// kind. Without a `kind` the export carries no type information and the JSON is returned as is.
/// Maps, delegates, unknown kinds and malformed or oversized text are unknown (`None`).
/// References decode to their object path, `None` references to JSON `null`. Text decodes like a
/// KDF text value: `INVTEXT` to its string, `NSLOCTEXT`/`LOCTEXT` to `{key, ns?, source}` and
/// `LOCTABLE` to `{table, key}`, which has no source text.
pub(super) fn typed_default(
    kind: Option<&str>,
    inner_kind: Option<&str>,
    raw: Value,
) -> Option<Value> {
    let (kind, text) = match (kind, raw) {
        (Some(kind), Value::String(text)) => (kind, text),
        (_, structured) => return Some(structured),
    };
    if text.len() > MAX_TEXT {
        return None;
    }
    match kind {
        // FArrayProperty::ExportTextInnerItem writes nothing for an empty array; sets write "()".
        "ArrayProperty" if text.is_empty() => Some(Value::Array(Vec::new())),
        "ArrayProperty" | "SetProperty" => typed_sequence(inner_kind, Reader::new(&text).read()?),
        // The engine exports empty text as an empty string, without a text macro.
        "TextProperty" if text.is_empty() => Some(Value::String(text)),
        "StructProperty" | "TextProperty" => typed_node(kind, Reader::new(&text).read()?),
        // Top-level scalars are written without delimiters: the whole text is the value.
        _ => typed_scalar(kind, &text),
    }
}

fn is_reference(kind: &str) -> bool {
    matches!(
        kind,
        "ObjectProperty"
            | "ClassProperty"
            | "SoftObjectProperty"
            | "SoftClassProperty"
            | "WeakObjectProperty"
            | "LazyObjectProperty"
            | "InterfaceProperty"
    )
}

fn is_number(kind: &str) -> bool {
    matches!(
        kind,
        "FloatProperty"
            | "DoubleProperty"
            | "IntProperty"
            | "Int8Property"
            | "Int16Property"
            | "Int64Property"
            | "UInt16Property"
            | "UInt32Property"
            | "UInt64Property"
    )
}

fn typed_scalar(kind: &str, text: &str) -> Option<Value> {
    match kind {
        "BoolProperty" => match text {
            "True" => Some(Value::Bool(true)),
            "False" => Some(Value::Bool(false)),
            _ => None,
        },
        // A byte property without an enum exports a number; with an enum, the enumerator name.
        "ByteProperty" => number(text).or_else(|| Some(Value::String(text.to_owned()))),
        "StrProperty" | "NameProperty" | "EnumProperty" => Some(Value::String(text.to_owned())),
        _ if is_number(kind) => number(text),
        _ if is_reference(kind) => reference(text),
        _ => None,
    }
}

/// `None` clears a reference; `Type'/Path'` and bare `/Path` name an object.
fn reference(text: &str) -> Option<Value> {
    if text == "None" {
        return Some(Value::Null);
    }
    object_path(text).map(|path| Value::String(path.to_owned()))
}

fn object_path(text: &str) -> Option<&str> {
    let path = match text.split_once('\'') {
        Some((class, rest)) if !class.contains(['"', '(', ')']) => rest.strip_suffix('\'')?,
        Some(_) => return None,
        None => text,
    };
    (path.starts_with('/') && !path.contains(['\'', '"'])).then_some(path)
}

fn number(text: &str) -> Option<Value> {
    if text.is_empty()
        || !text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))
    {
        return None;
    }
    if let Ok(integer) = text.parse::<i64>() {
        return Some(Value::Number(integer.into()));
    }
    if let Ok(integer) = text.parse::<u64>() {
        return Some(Value::Number(integer.into()));
    }
    Number::from_f64(text.parse::<f64>().ok()?).map(Value::Number)
}

fn typed_sequence(inner_kind: Option<&str>, node: Node) -> Option<Value> {
    let Node::Group(entries) = node else {
        return None;
    };
    entries
        .into_iter()
        .map(|(name, element)| match (name, inner_kind) {
            (Some(_), _) => None,
            (None, Some(kind)) => typed_node(kind, element),
            (None, None) => untyped_node(element),
        })
        .collect::<Option<Vec<_>>>()
        .map(Value::Array)
}

fn typed_node(kind: &str, node: Node) -> Option<Value> {
    match (kind, node) {
        ("TextProperty", Node::Call { name, args }) => text_value(&name, args),
        ("StructProperty", node @ Node::Group(_)) => untyped_node(node),
        (_, Node::Scalar { text, .. }) => typed_scalar(kind, &text),
        _ => None,
    }
}

/// Struct members carry no exported member types; read them by their ImportText spelling.
fn untyped_node(node: Node) -> Option<Value> {
    match node {
        Node::Scalar { text, quoted: true } => {
            let path = object_path(&text).map(str::to_owned);
            Some(Value::String(path.unwrap_or(text)))
        }
        Node::Scalar {
            text,
            quoted: false,
        } => Some(bare_value(text)),
        Node::Call { name, args } => text_value(&name, args),
        Node::Group(entries) if entries.iter().all(|(name, _)| name.is_some()) => entries
            .into_iter()
            .map(|(name, value)| Some((name?, untyped_node(value)?)))
            .collect::<Option<Map<_, _>>>()
            .map(Value::Object),
        Node::Group(entries) if entries.iter().all(|(name, _)| name.is_none()) => entries
            .into_iter()
            .map(|(_, value)| untyped_node(value))
            .collect::<Option<Vec<_>>>()
            .map(Value::Array),
        Node::Group(_) => None,
    }
}

fn bare_value(text: String) -> Value {
    match text.as_str() {
        "True" => return Value::Bool(true),
        "False" => return Value::Bool(false),
        "None" => return Value::Null,
        _ => {}
    }
    if let Some(value) = number(&text) {
        return value;
    }
    let path = object_path(&text).map(str::to_owned);
    Value::String(path.unwrap_or(text))
}

fn text_value(macro_name: &str, args: Vec<String>) -> Option<Value> {
    let localized = |ns: Option<String>, key: String, source: String| {
        let mut text = Map::new();
        text.insert("key".into(), Value::String(key));
        if let Some(ns) = ns.filter(|ns| !ns.is_empty()) {
            text.insert("ns".into(), Value::String(ns));
        }
        text.insert("source".into(), Value::String(source));
        Value::Object(text)
    };
    let mut args = args.into_iter();
    let value = match (macro_name, args.len()) {
        ("INVTEXT", 1) => Value::String(args.next()?),
        ("LOCTEXT", 2) => localized(None, args.next()?, args.next()?),
        ("NSLOCTEXT", 3) => localized(args.next(), args.next()?, args.next()?),
        ("LOCTABLE", 2) => {
            let mut text = Map::new();
            text.insert("table".into(), Value::String(args.next()?));
            text.insert("key".into(), Value::String(args.next()?));
            Value::Object(text)
        }
        _ => return None,
    };
    Some(value)
}

enum Node {
    /// `(a,b)` or `(Name=a,Other=b)`; member names are kept per entry.
    Group(Vec<(Option<String>, Node)>),
    Scalar {
        text: String,
        quoted: bool,
    },
    /// Text macro such as `NSLOCTEXT("ns", "key", "source")`.
    Call {
        name: String,
        args: Vec<String>,
    },
}

/// Bounded reader for the ImportText grammar of exported defaults.
struct Reader<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, at: 0 }
    }

    fn read(mut self) -> Option<Node> {
        let node = self.value(0)?;
        self.space();
        (self.at == self.text.len()).then_some(node)
    }

    fn peek(&self) -> Option<char> {
        self.text[self.at..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += c.len_utf8();
        Some(c)
    }

    fn space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn value(&mut self, depth: usize) -> Option<Node> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.space();
        match self.peek()? {
            '(' => self.group(depth),
            '"' => Some(Node::Scalar {
                text: self.quoted()?,
                quoted: true,
            }),
            _ => self.bare(),
        }
    }

    fn group(&mut self, depth: usize) -> Option<Node> {
        self.bump();
        let mut entries = Vec::new();
        self.space();
        if self.peek() == Some(')') {
            self.bump();
            return Some(Node::Group(entries));
        }
        loop {
            self.space();
            let name = self.member_name()?;
            entries.push((name, self.value(depth + 1)?));
            self.space();
            match self.bump()? {
                ')' => return Some(Node::Group(entries)),
                ',' => {}
                _ => return None,
            }
        }
    }

    /// `Some(None)`: positional entry. `None`: static array member (`Values[0]=`), whose
    /// element layout the export does not describe.
    fn member_name(&mut self) -> Option<Option<String>> {
        let text = self.text;
        let rest = &text[self.at..];
        let length = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if length == 0 || rest.starts_with(|c: char| c.is_ascii_digit()) {
            return Some(None);
        }
        let after = rest[length..].trim_start();
        if after.starts_with('=') {
            let name = rest[..length].to_owned();
            self.at += rest.len() - after.len() + 1;
            return Some(Some(name));
        }
        let static_index = after
            .strip_prefix(['[', '('])
            .map(|index| index.trim_start_matches(|c: char| c.is_ascii_digit()))
            .and_then(|index| index.strip_prefix([']', ')']))
            .is_some_and(|index| index.trim_start().starts_with('='));
        if static_index {
            None
        } else {
            Some(None)
        }
    }

    fn quoted(&mut self) -> Option<String> {
        self.bump();
        let mut result = String::new();
        loop {
            match self.bump()? {
                '"' => return Some(result),
                '\\' => result.push(match self.bump()? {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    other => other,
                }),
                other => result.push(other),
            }
        }
    }

    fn bare(&mut self) -> Option<Node> {
        let text = self.text;
        let rest = &text[self.at..];
        let macro_length = rest
            .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            .unwrap_or(rest.len());
        if macro_length > 0 && rest[macro_length..].starts_with('(') {
            let name = rest[..macro_length].to_owned();
            self.at += macro_length;
            return Some(Node::Call {
                name,
                args: self.call_arguments()?,
            });
        }
        let start = self.at;
        let mut in_path = false;
        while let Some(c) = self.peek() {
            match c {
                '\'' => in_path = !in_path,
                ',' | ')' if !in_path => break,
                '(' | '"' if !in_path => return None,
                _ => {}
            }
            self.bump();
        }
        if in_path {
            return None;
        }
        Some(Node::Scalar {
            text: self.text[start..self.at].trim().to_owned(),
            quoted: false,
        })
    }

    fn call_arguments(&mut self) -> Option<Vec<String>> {
        self.bump();
        let mut args = Vec::new();
        self.space();
        if self.peek() == Some(')') {
            self.bump();
            return Some(args);
        }
        loop {
            self.space();
            if self.peek() != Some('"') {
                return None;
            }
            args.push(self.quoted()?);
            self.space();
            match self.bump()? {
                ')' => return Some(args),
                ',' => {}
                _ => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::typed_default;
    use serde_json::{json, Value};

    fn decode(kind: &str, inner: Option<&str>, text: &str) -> Option<Value> {
        typed_default(Some(kind), inner, json!(text))
    }

    #[test]
    fn exported_recipe_defaults_decode_to_structured_values() {
        assert_eq!(
            decode(
                "ArrayProperty",
                Some("SoftClassProperty"),
                r#"("/Game/FactoryGame/Buildable/Factory/OilRefinery/Build_OilRefinery.Build_OilRefinery_C")"#,
            ),
            Some(json!([
                "/Game/FactoryGame/Buildable/Factory/OilRefinery/Build_OilRefinery.Build_OilRefinery_C"
            ]))
        );
        assert_eq!(
            decode(
                "ArrayProperty",
                Some("StructProperty"),
                r#"((ItemClass="/Script/Engine.BlueprintGeneratedClass'/Game/Parts/Desc_Cable.Desc_Cable_C'",amount=9),(ItemClass=None,amount=2000))"#,
            ),
            Some(json!([
                {"ItemClass": "/Game/Parts/Desc_Cable.Desc_Cable_C", "amount": 9},
                {"ItemClass": null, "amount": 2000}
            ]))
        );
        assert_eq!(decode("FloatProperty", None, "8.000000"), Some(json!(8.0)));
        assert_eq!(decode("IntProperty", None, "-1"), Some(json!(-1)));
        assert_eq!(decode("BoolProperty", None, "True"), Some(json!(true)));
        assert_eq!(
            decode("ArrayProperty", Some("StructProperty"), "()"),
            Some(json!([]))
        );
        assert_eq!(
            decode("ArrayProperty", Some("SoftClassProperty"), ""),
            Some(json!([]))
        );
        assert_eq!(decode("SetProperty", Some("NameProperty"), ""), None);
    }

    #[test]
    fn text_macros_never_invent_source_text() {
        assert_eq!(
            decode(
                "TextProperty",
                None,
                r#"NSLOCTEXT("", "C600", "Charged \"Battery\"")"#
            ),
            Some(json!({"key": "C600", "source": "Charged \"Battery\""}))
        );
        assert_eq!(
            decode(
                "TextProperty",
                None,
                r#"INVTEXT("Default Foundation Customization")"#
            ),
            Some(json!("Default Foundation Customization"))
        );
        assert_eq!(
            decode(
                "TextProperty",
                None,
                r#"LOCTABLE("Items_Data", "Parts/CableCoated")"#
            ),
            Some(json!({"table": "Items_Data", "key": "Parts/CableCoated"}))
        );
        assert_eq!(decode("TextProperty", None, "plain"), None);
    }

    #[test]
    fn references_unwrap_class_and_clear_to_null() {
        assert_eq!(
            decode(
                "ClassProperty",
                None,
                "/Script/CoreUObject.Class'/Script/AIModule.AIController'"
            ),
            Some(json!("/Script/AIModule.AIController"))
        );
        assert_eq!(decode("ObjectProperty", None, "None"), Some(Value::Null));
        assert_eq!(
            decode("ArrayProperty", Some("ObjectProperty"), "(None)"),
            Some(json!([null]))
        );
        assert_eq!(decode("SoftClassProperty", None, "Desc_Cable_C"), None);
    }

    #[test]
    fn unreadable_or_untyped_text_stays_unknown() {
        for (kind, inner, text) in [
            ("FloatProperty", None, "8.0f"),
            ("BoolProperty", None, "Yes"),
            ("MapProperty", None, "()"),
            ("MulticastSparseDelegateProperty", None, "()"),
            (
                "ArrayProperty",
                Some("EnumProperty"),
                "(RF_LIQUID,RF_GAS,(INVALID))",
            ),
            ("ArrayProperty", Some("StructProperty"), "((A=1),"),
            ("StructProperty", None, "(Values[0]=1,Values[1]=2)"),
            ("StructProperty", None, "(A=1,2)"),
            ("SomeFutureProperty", None, "1"),
        ] {
            assert_eq!(decode(kind, inner, text), None, "{kind} {text}");
        }
        assert_eq!(decode("FloatProperty", None, &"1".repeat(300 * 1024)), None);
    }

    #[test]
    fn structured_or_kindless_exports_keep_their_json() {
        let structured = json!([{"ItemClass": "/Game/Desc_Oil.Desc_Oil_C", "Amount": 1}]);
        assert_eq!(
            typed_default(
                Some("ArrayProperty"),
                Some("StructProperty"),
                structured.clone()
            ),
            Some(structured)
        );
        assert_eq!(
            typed_default(None, None, json!("8.000000")),
            Some(json!("8.000000"))
        );
    }

    #[test]
    fn scalar_strings_keep_their_exact_text() {
        assert_eq!(decode("StrProperty", None, "a, (b)"), Some(json!("a, (b)")));
        assert_eq!(
            decode("ByteProperty", None, "ROLE_Authority"),
            Some(json!("ROLE_Authority"))
        );
        assert_eq!(decode("ByteProperty", None, "3"), Some(json!(3)));
    }
}
