# kdf-validation: pack configuration API (for the language server)

Public API of package T1, frozen. Everything below is `pub` in `kdf_validation` (modules `expression`, `template`,
`config`, `slots`, `conditions`, `cold`, plus re-exports at the crate root). All text offsets and spans are UTF-16 code
units, relative to the string that was passed in. The whole surface is implemented and covered by tests
(`tests/config*.rs`, `tests/diagnostic_catalog.rs`, unit tests in the modules).

Contract sources: runtime design section 1 (YAML contract), tooling design sections 2 and 3, vectors in
`tests/fixtures/config-contract-vectors.json`, diagnostic catalogue in `tests/fixtures/diagnostic-codes.json`
(a test keeps the catalogue and the codes in the source in sync).

## 1. `expression` (config expressions, `config:` conditions)

```rust
pub struct Span { pub start: u32, pub end: u32 }            // half-open; new/len/is_empty/touches(offset)/join
pub enum CmpOp { Eq, Ne, Lt, Le, Gt, Ge, Contains, In }      // as_str() -> "==", "contains", ...; parse(&str); ALL
pub enum TokenKind { Ident, Reference, Number, InvalidNumber, String, UnterminatedString, True, False,
    Not, AndAnd, OrOr, LParen, RParen, LBracket, RBracket, Comma, Compare(CmpOp), Unknown }
pub struct Token { pub kind: TokenKind, pub span: Span, pub text: String }
pub fn tokenize(src: &str) -> Vec<Token>                     // total: never fails, lexical errors become tokens

pub enum ErrorCode { Empty, UnexpectedChar, UnexpectedToken, ExpectedOperand, ExpectedCloseParen,
    ExpectedCloseBracket, UnterminatedString, InvalidNumber, InvalidRef, ReservedId, ChainedComparison,
    ListElementNotLiteral, TrailingInput, TooDeep }          // as_str() -> "expected-operand", ..., "too-deep"
pub struct ParseError { pub code: ErrorCode, pub at: u32, pub message: String }
pub fn parse(src: &str) -> Result<Expr, ParseError>          // first error only; `at` = end of input when input ended early

pub struct Expr { pub span: Span, pub kind: ExprKind }       // parentheses are not part of spans
pub enum ExprKind { Or(Vec<Expr>), And(Vec<Expr>), Not(Box<Expr>),
    Compare { op: CmpOp, left: Box<Expr>, right: Box<Expr> }, Operand(Operand), Literal(Literal), List(Vec<ListItem>) }
pub enum Literal { Number(f64), Str(String), Bool(bool) }
pub struct ListItem { pub literal: Literal, pub span: Span }
pub struct Operand { pub kind: OperandKind, pub span: Span }
pub enum OperandKind { Config { pack: Option<String>, id: String }, Setting { provider: String, key: String } }
impl Operand { pub fn key(&self) -> String      // own-pack id, "Pack:id" or "provider:key"
               pub fn canonical(&self) -> String // "cfg:Pack:id" / "session:K" }
pub fn operands(expr: &Expr) -> Vec<Operand>                 // source order

pub enum ExprType { Unknown, Bool, Number, String, List(Box<ExprType>) }   // parse("list<string>") / label()
pub enum TypeIssueCode { CompareMismatch, OrderNonNumber, ContainsNonList, InNonList, ListElementMismatch, UnknownId }
pub struct TypeIssue { pub code: TypeIssueCode, pub span: Span }
pub fn type_check(expr: &Expr, operand_type: &dyn Fn(&Operand) -> Option<ExprType>) -> Vec<TypeIssue>
    // closure: None = unknown id (issue UnknownId on the operand), Some(Unknown) = known id, unknown type

pub enum EvalValue { Bool(bool), Number(f64), Str(String), List(Vec<EvalValue>) }   // from_json(&Value), truthy()
pub enum EvalErrorCode { UnknownId, TypeError }
pub struct EvalError { pub code: EvalErrorCode, pub span: Span, pub message: String }
pub fn eval(expr: &Expr, operand_value: &dyn Fn(&Operand) -> Option<EvalValue>) -> Result<bool, EvalError>

pub enum Expect { Operand, Value, ListElement, Operator, String }
pub enum WordKind { Ident, Reference, Number }
pub struct CursorState {
    pub expect: Expect,            // what the grammar allows at the cursor
    pub word: String,              // token text BEFORE the cursor ("" between tokens)
    pub word_span: Span,           // whole token under/touching the cursor (empty span at the cursor if none)
    pub word_kind: Option<WordKind>,
    pub provider: Option<String>,  // "cfg" | "session" | "ags" | "option" while typing a reference
    pub pack: Option<String>,      // `cfg:Pack:|` pack already typed
    pub operator: Option<CmpOp>,   // Expect::Value: the comparison operator before the cursor
    pub left: Option<Operand>,     // Expect::Value: left operand of the comparison; Expect::Operator: operand left of the cursor
    pub in_list: bool,             // inside `[...]`
    pub in_string: Option<char>,   // quote char when the cursor is inside a string literal
}
pub fn cursor_state(src: &str, offset: u32) -> CursorState   // works on incomplete input
// helpers: is_config_id, is_reserved_id, is_pack_ref_text, json_number; Expr::to_vector() (vector AST)
```

Parse error codes (`ErrorCode::as_str()`, positions in UTF-16 units): `empty`, `unexpected-char`, `unexpected-token`,
`expected-operand`, `expected-close-paren`, `expected-close-bracket`, `unterminated-string`, `invalid-number`, `invalid-ref`,
`reserved-id`, `chained-comparison`, `list-element-not-literal`, `trailing-input`, `too-deep`.

Grammar facts: `||` < `&&` < comparison (non-associative, one operator) < unary `!`; keywords `true false in contains`
are matched against the whole dotted identifier run (`in.x` is an id); `cfg:`/`session:`/`ags:`/`option:` prefixes make a
reference run `[A-Za-z0-9_.:-]*`; numbers have no exponent; strings use `'` or `"` with escapes `\\ \' \" \n \t`.
Nesting of `(` and `!` shares one counter capped at `expression::MAX_NESTING` (64); the token that opens level 65 is
`TooDeep` at its start offset. Prefixes and keywords are case-sensitive: `True`/`In` are ordinary ids, `Cfg:x` is not a
reference, and only the exact lowercase words are reserved ids (config id uniqueness stays case-insensitive).
Unknown ids / type errors evaluate as errors (runtime: condition false + error diagnostic).

## 2. `template` (templates `{prefix:body}`)

```rust
pub const PREPASS_PREFIXES: [&str; 4];   // cfg session ags option
pub const APPLY_PREFIXES: [&str; 10];    // class item resource building recipe schematic research category dataasset asset
pub const MAX_DEPTH: u8;                 // 4
pub enum Phase { PrePass, Apply }        // phase_of(prefix) -> Option<Phase>
pub enum ScanPhase { PrePass, Apply, All }
pub fn base_class(prefix: &str) -> Option<&'static str>   // item -> /Script/FactoryGame.FGItemDescriptor, ... (None: class, asset)
pub fn is_object_target(prefix: &str) -> bool             // dataasset | asset (object instance targets)
pub fn describe(prefix: &str) -> &'static str             // short documentation for completion

pub struct TemplateToken { pub prefix: String, pub body: String, pub span: Span, pub prefix_span: Span,
    pub body_span: Span, pub depth: u8 /*1 = outermost*/, pub children: Vec<TemplateToken> }   // walk(), phase()
pub enum TemplateSegment { Text { text: String, span: Span }, Token(TemplateToken) }
pub enum ScanIssueKind { Unbalanced, NestingDepth, UnknownPrefix { prefix: String, suggestion: String } }
pub struct ScanIssue { pub kind: ScanIssueKind, pub span: Span }
pub struct Scan { pub segments: Vec<TemplateSegment>, pub issues: Vec<ScanIssue> }    // tokens() (all depths), to_vector()
pub fn scan(src: &str, phase: ScanPhase) -> Scan
    // Text segments have this phase's `\{` escapes removed; `ScanPhase::All` recognises every prefix (use it for validation).
pub fn expand_prepass(src: &str, resolve: &mut dyn FnMut(&str, &str) -> Option<String>) -> Expansion
pub fn split_body(body: &str) -> Option<(&str /*target*/, &str /*member*/, u32 /*member offset*/)>

pub enum Member { Ident { name, span }, Path { path, span }, Call { name, name_span, args: Vec<Arg>, span } }
pub struct Arg { pub kind: ArgKind, pub span: Span }
pub enum ArgKind { Number(f64), Str(String), Bool(bool), Ident(String),
    Struct { type_name: Option<String>, fields: StructFields } }
pub enum StructFields { Positional(Vec<Arg>), Named(Vec<(String, Arg)>) }
pub enum MemberErrorKind { Empty, Unbalanced, ExpectedArgument, Malformed }           // as_str()
pub struct MemberError { pub kind: MemberErrorKind, pub at: u32, pub message: String }
pub const MAX_MEMBER_NESTING: u32;     // 64 nested struct literals, deeper is MemberErrorKind::Malformed at the `{`
pub fn parse_member(src: &str) -> Result<Member, MemberError>
    // bare argument words (`RF_SOLID`, `EType::Value`, unquoted paths, `1e3`) are numbers, booleans or `Ident`s

pub enum Stage { Provider, Body, Target, Member, Argument }
pub struct Partial {
    pub stage: Stage,              // Provider `{ite|` / Body `{cfg:Pack:|` / Target `{item:|` / Member `{item:T:|` / Argument `{item:T:Fn(a, |`
    pub prefix: String,            // provider as typed (Provider stage: the partial prefix)
    pub known: bool,               // prefix is a built-in prefix
    pub token_start: u32,          // offset of `{`
    pub target: Option<String>,    // apply-time target text (Member/Argument stages)
    pub pack: Option<String>,      // `cfg:Pack:` part
    pub typed: String,             // current segment text before the cursor
    pub replace: Span,             // segment start .. end of the word under the cursor
    pub function: Option<FunctionContext>,   // { name, arg_index, args_before, in_quote }
    pub outer: Option<Box<Partial>>,         // enclosing token when nested (innermost is returned)
    pub closed: bool,              // the token already has its closing brace
}
pub fn partial_at(src: &str, offset: u32) -> Option<Partial>   // None outside `{...}` or inside unknown/third-party prefixes
pub const MAX_PARTIAL_BYTES: usize;   // 64 KiB: partial_at returns None for a longer value (it is data, not a template being typed)
```

Nesting is bounded everywhere: the scan reads tokens up to depth `MAX_DEPTH + 1` (the deepest is reported as
`NestingDepth`, its body is not scanned), `expand_prepass` leaves anything below that literal, and `partial_at` treats a `{`
below depth `MAX_DEPTH + 1` as text, so the `outer` chain of a `Partial` has at most `MAX_DEPTH + 1` links. A 20 KB
`{cfg:{cfg:...` neither recurses nor costs more than a few linear passes.

`src` is the string VALUE of the scalar (after YAML unescaping); map offsets back to the document yourself.

## 3. `config` (definitions and workspace index)

```rust
pub fn scan_workspace(root: &Path, buffers: &[FileBuffer]) -> ConfigIndex   // never fails; buffers override disk
pub fn is_config_file_name(name: &str) -> bool; pub fn is_config_file(path: &Path) -> bool   // *.config.yml|yaml
// Incremental builds (a host that walks the tree itself and keeps what it parsed between builds):
pub fn is_index_source(path: &Path) -> bool                    // pack.yml, *.config.yml, *.class.yml
pub fn parse_source(path: PathBuf, text: String) -> Option<ConfigSource>   // keep the result; `ConfigSource::path()`
pub fn build_index(sources: &[ConfigSource]) -> ConfigIndex    // any order; scan_workspace is walk + parse_source + build_index
pub const CATEGORY_BASE_CLASS: &str;      // "/Script/FactoryGame.FGCategory"
pub const RESERVED_IDS: [&str; 4];

impl ConfigIndex {
    pub fn packs(&self) -> &[PackConfig];  pub fn is_empty(&self) -> bool;
    pub fn pack(&self, pack_ref: &str) -> Option<&PackConfig>;                 // exact (case-sensitive) ref
    pub fn pack_for_path(&self, path: &Path) -> Option<&PackConfig>;           // pack folder containing the file
    pub fn field(&self, pack_ref: &str, id: &str) -> Option<&ConfigField>;     // exact id
    pub fn fields(&self) -> impl Iterator<Item = &ConfigField>;
    pub fn resolve<'a>(&'a self, own_pack: Option<&str>, pack: Option<&str>, id: &str) -> Resolution<'a>;
    pub fn generated_class(&self, pack_ref: &str, id: &str) -> Option<&GeneratedClass>;
        // classes a pack generates (`type: class`, item, resource, building, unlock, recipe, schematic, research) by bare id,
        // with their `parent` (default base class of the kind when omitted)
}
pub enum Resolution<'a> { Found(&'a ConfigField), CaseMismatch(&'a ConfigField), UnknownId, UnknownPack,
    PackDisabled(Option<&'a ConfigField>) }   // PackDisabled never applies to the pack's own documents
pub struct PackConfig { pub pack_ref: String, pub name: Option<String>, pub dir: PathBuf, pub manifest: PathBuf,
    pub enabled: bool, pub fields: Vec<ConfigField>, pub files: Vec<PathBuf> }       // field(id), field_ci(id)
pub struct ConfigField {
    pub pack_ref, id, kind: FieldKind, value_type: ValueType, multiple: bool, apply: Apply,
    pub name, description: String, tooltip_title, tooltip_text: Option<String>, category: Option<String>,
    pub default: serde_json::Value, min, max, step: Option<f64>, decimals: Option<u8>, display: NumberDisplay,
    pub options: Vec<FieldOption { value: Value, label: String }>, min_selected, max_selected, min_count, max_count,
    pub max_length: Option<u64>, multiline: bool, tag_root: Option<String>, order: i64,
    pub file: PathBuf, document: usize, pointer: String, range: Range, id_range: Range,
}   // key() -> "<pack>:<id>"
pub enum FieldKind { Text, Number, Boolean, Select }       // as_str()/parse()
pub enum ValueType { String, Text, Name, Tag, Int8, Int16, Int32, Int64, UInt8, Float, Double, Bool }
pub enum Apply { Hot, Cold }   pub enum NumberDisplay { Slider, Input }
```
Runtime limits (`config.limit-exceeded`, Error): list `minCount`/`maxCount` and multi-select `maxSelected` above 64 (lists
without `maxCount` hold at most 64 elements), text `maxLength` above 1024, field ids longer than 200 characters, pack refs
longer than 50 characters (reported on `pack.yml` `/ref`, only for packs with config files; the fields stay indexed).
Fields above a limit are not indexed.
The index only holds fields the runtime would accept (invalid, duplicate or clashing fields are skipped and reported
as diagnostics by the lint run). `name`/`description` are display strings (`source`, else the localization `key`).
A pack needs `ref` in its `pack.yml` to be indexed.

## 4. `slots` (compatibility and hot/cold tables)

```rust
pub enum Slot { Bool, Int(IntWidth), Float, Double, Number, Index, Str, Name, Text, Enum(Vec<String>), Class, SoftClass,
    Object, SoftObject, GameplayTag, TagContainer, Array(Box<Slot>), Set(Box<Slot>), Map, Struct(Option<String>), Other, Unknown }
impl Slot { pub fn from_property(&PropertyInfo) -> Slot; pub fn element(&self) -> Slot; pub fn label(&self) -> String }
pub enum Compat { Exact, Convertible, Lossy, Incompatible }    // offered() = Exact|Convertible; rank() 0..3 (sortText)
pub struct ValueShape { pub kind: ShapeKind, pub multiple: bool, pub bounds, defaults, width }
pub fn field_shape(&ConfigField) -> ValueShape;   pub fn setting_shape(&SettingRecord) -> ValueShape;
pub fn expected_slot(op: Option<&str>, operand: &str, property: Option<&PropertyInfo>) -> Slot
    // operand: "value" | "min" | "max" | "index" | "with" | "lessThan" | "lessOrEqual" | "greaterThan" | "greaterOrEqual"
    // numeric ops (add/subtract/multiply/divide/min/max) -> Number; set -> property slot; append/prepend/insert/replace/remove -> element slot
pub fn compat(shape: &ValueShape, slot: &Slot) -> Compat       // Unknown slot = Exact (no information)
pub fn is_cold_context(kind: &str, segments: &[&str]) -> bool  // segments below the document root (["recipes","0","properties",...])
pub fn is_cold_condition(kind: &str, segments: &[&str]) -> bool
pub fn default_base_class(entries_key: &str) -> Option<&'static str>   // "items" -> FGItemDescriptor, ...
```

## 5. Reflection (methods on `ReflectionIndex`, no JSON is cloned; lookups are case-insensitive)

```rust
impl ReflectionIndex {
    pub fn resolve_class(&self, reference: &str) -> Option<ResolvedClass<'_>>   // { path: &str, record: &Value }
        // accepts /Script/X.Y, /Game/A/B.B_C, /Game/A/B.B, /Game/A/B, Class'...'  and unique short names (Recipe_X_C)
    pub fn class_path(&self, reference: &str) -> Option<&str>
    pub fn super_chain_inclusive(&self, class: &str) -> Vec<&str>               // class first
    pub fn is_subclass_of(&self, class: &str, ancestor: &str) -> bool          // true for equal classes
    pub fn descendants(&self, base: &str, concrete_only: bool) -> Vec<&str>    // sorted; base excluded
    pub fn is_abstract(&self, class: &str) -> bool
    pub fn find_property(&self, class: &str, name: &str) -> Option<PropertyInfo>
    pub fn resolve_property_path(&self, class: &str, path: &str) -> Option<PropertyInfo>   // mA.mB[0]['k'].mC, structs, objects
    pub fn find_struct_field(&self, struct_path: &str, name: &str) -> Option<PropertyInfo>
    pub fn struct_properties(&self, struct_path: &str) -> Option<&[PropertyInfo]>
    pub fn find_function(&self, class: &str, name: &str) -> Option<FunctionInfo>            // class, then super chain
    pub fn callable(&self, f: &FunctionInfo, target: TargetKind, target_class: Option<&str>) -> Callable
    pub fn signature(&self, f: &FunctionInfo, implicit: Implicit) -> String                 // "Name(a: T, b: U) -> R"
    pub fn list_members(&self, class: &str, target: TargetKind) -> Vec<MemberInfo>          // properties, then callable functions
    pub fn list_members_matching(&self, class: &str, target: TargetKind, keep: &dyn Fn(&str) -> bool) -> Vec<MemberInfo>
        // only members whose display name passes `keep` are built (the call allowlist runs for those only)
    pub fn property_reload(&self, class: &str, path: &str) -> (Reload, String)              // applyToSpawnedActors = false
    pub fn property_reload_with(&self, class: &str, path: &str, apply_to_spawned_actors: bool) -> (Reload, String)
    pub fn reload_classifier(&self, class: &str, apply_to_spawned_actors: bool) -> ReloadClassifier<'_>
        // class chain, catalog entries and actor rule resolved once; `classify(&PropertyInfo)` / `classify_name(&str)` are
        // hash lookups. Use it for every property of one class (completion); same answers as property_reload_with.
    pub fn warm(&self)    // builds every lazy lookup (lower-case map, children, names, structs, assets, settings, catalog) now
    pub fn settings(&self) -> Option<&SettingsIndex>        // indexes/settings.json (<= 16 MiB), optional
    pub fn cold_catalog(&self) -> Option<&ColdCatalog>      // catalog/cold_properties.json, optional
    pub fn assets_of_class(&self, class: &str) -> Vec<&AssetEntry>   // indexes/assets.json, indexed under that property class
    pub fn asset_listing_complete(&self, class: &str) -> Option<bool> // None: class not indexed; false: listing capped (truncated)
    pub fn asset_index_classes(&self) -> Vec<&str>
    pub fn find_asset(&self, path: &str) -> Option<&AssetEntry>      // object path or package path
}
pub struct AssetEntry { pub path: String, pub name: String, pub asset_class: String, pub index_class: String }
pub struct PropertyInfo { name, authored_name, kind: PropertyKind, kind_name, cpp_type, owner_path, class_path, meta_class,
    struct_path, enum_path, enum_options: Vec<String>, inner/key/value: Option<Box<PropertyInfo>>, reload: Option<Reload>,
    reload_reason: Option<String>, flags: Vec<String> }       // display_name() prefers authored_name
pub struct FunctionInfo { name, authored_name, path, owner_path, flags, params: Vec<ParamInfo>, ret: Option<PropertyInfo> }
pub enum Callable { Yes { implicit: Implicit, explicit_args: usize, unverified: Option<String> }, No(String) }
pub enum Implicit { None, Class, Object }   pub enum TargetKind { Class, Object }
pub struct MemberInfo { name, kind: MemberKind, owner_path, type_label, signature: Option<String>, implicit: Implicit,
    reload: Option<Reload>, callable: Option<Callable>, property: Option<PropertyInfo>, function: Option<FunctionInfo> }
pub enum Reload { Hot, Cold, NewActors }   // as_str() "hot" | "cold" | "newActors"
pub struct SettingsIndex { settings: Vec<SettingRecord>, .. }  // find("session"|"ags"|"option", id), of_provider(provider)
pub fn explicit_params(f: &FunctionInfo, implicit: Implicit) -> Vec<&ParamInfo>
```
`property_reload` order: catalog entry (`hot` whitelists), exporter annotation `kdf.reload`, actor rule
(`newActors` for AActor/UActorComponent class defaults unless `applyToSpawnedActors`).
The lint checks the call allowlist for class targets and, with `indexes/assets.json`, for `dataasset`/`asset` objects.

## 5b. `conditions` and `cold`

```rust
pub const HOLDER_KEYS: [&str; 3];                                       // conditions, conditionBehavior, conditionBehaivor
pub struct ConditionSite { pub pointer: String, pub holder: Map<String, Value> }
pub fn strip_element_conditions(doc: &mut Value) -> Vec<ConditionSite>  // lifts the keys out of every sequence element map (document order)
pub fn condition_objects(conditions: &Value) -> Vec<(String /*relative pointer*/, &Map<String, Value>)>
pub fn config_expressions(object: &Map<String, Value>) -> Vec<(String /*relative pointer*/, &str)>
pub fn holder_passes(holder: &Map<String, Value>, values: &dyn Fn(&Operand) -> Option<EvalValue>) -> bool
pub fn prune_config_conditions(doc: &Value, values: &dyn Fn(&Operand) -> Option<EvalValue>) -> Value   // runtime pre-pass preview
// cold: Reload, ColdCatalog { parse, lookup }, ColdEntry, NEW_ACTORS_REASON
```

## 6. Diagnostics and options

* New diagnostic codes: see `tests/fixtures/diagnostic-codes.json`; all are emitted by `lint_workspace` / `validate_document`.
  `validate_document` knows only its own file: it checks syntax, definitions, expressions, templates and reflection data, but
  cannot resolve `cfg:` ids of other files (no `config.unknown-id`) or bare category ids.
* **Informational diagnostics**: `property.cold` has `severity: Warning` plus `Diagnostic::advisory: Some(Advisory::Information | Advisory::Hint)`
  (JSON field `advisory`: `"information"` / `"hint"`, absent otherwise). They are not counted in `LintResult::warnings`.
  Map to LSP severity `INFORMATION` / `HINT` when `advisory` is set. `Severity` itself is unchanged (no new variant),
  so `kdf-cli` and existing consumers keep compiling and show the diagnostic as a warning.
* `LintOptions::cold_properties: ColdPropertyLevel { Info, Hint, Off }` (default Info); `LintSettings::cold_properties` is read from
  `.kdf-editor.toml` `[lint] coldProperties = "info" | "hint" | "off"` (also `cold_properties`).
* Expression diagnostics carry the expression sub-code at the start of the message (`config.expression-syntax`:
  `expected-operand: ...`, `config.expression-type`: `compare-mismatch: ...`); their range points into the expression when the
  scalar is a single-line plain or quoted string without escapes, otherwise it covers the scalar.
* `template.unquoted-start` and `config.expression-quote` are also reported (next to `yaml.syntax`) when an unquoted template or
  expression makes the YAML itself unparsable; the range then is the offending character.
* `template.unknown-provider` message holds the suggested prefix; `config.case-mismatch` message holds the defined spelling.
* Ranges are never zero-width: an error "at the end of input" (`expected-operand` after `a >=`) is anchored on the last
  character of the value (`at-1..at`); a value without characters falls back to the range of the whole scalar.
* Host inputs (all additive; the environment variables stay the fallback):
  `datapack_selection(&str) -> Option<PathBuf>` turns a host-supplied datapack setting into a path (an empty text is `None`,
  which selects schema-only validation, like an empty `KDF_EFFECTIVE_DATAPACK_PATH`); `bound_datapack_path(root)` reads
  `[datapack] path` of the project's `.kdf-editor.toml`; `LintSettings::load(root, Some(user_settings))` takes the user
  `settings.toml` explicitly (`KDF_USER_SETTINGS_PATH` is used when `None`).
* `clamped_byte_offset_at_utf16(text, Position) -> usize` converts a protocol position the way LSP demands (a character past
  the end of its line is the line end, a line past the end is the document end, inside a surrogate pair rounds down);
  `byte_offset_at_utf16` stays the strict variant (`None` for positions that do not exist).
* Config documents (`*.config.yml`, root type `config`) are linted and counted in `LintResult::documents`; they are never part of
  generated-id registration. `kdf_validation::infer_type(path, document, multi)` is exported for the generated-id logic.
