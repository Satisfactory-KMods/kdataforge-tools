//! Synthetic reflection archives and workspace builders shared by the unit and stdio tests.
//! The dataset and the fixture files come from `kdf-validation`'s test suite.
#![allow(dead_code)]

use serde_json::{json, Map, Value};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use tempfile::NamedTempFile;
use zip::write::SimpleFileOptions;

pub const SETTINGS_INDEX: &str =
    include_str!("../../../kdf-validation/tests/fixtures/settings.index.sample.json");
pub const COLD_CATALOG: &str =
    include_str!("../../../kdf-validation/tests/fixtures/cold-properties.json");

pub fn flags(names: &[&str]) -> Value {
    json!({"names": names, "raw": "0x0"})
}

pub fn prop(name: &str, kind: &str, cpp: &str) -> Value {
    json!({
        "name": name, "authoredName": name, "kind": kind, "cppType": cpp, "ownerPath": "",
        "offset": 0, "elementSize": 4, "arrayDim": 1, "flags": flags(&[]), "kdf": {}
    })
}

pub fn merge(mut base: Value, extra: Value) -> Value {
    let map = base.as_object_mut().expect("object");
    for (key, value) in extra.as_object().expect("object") {
        map.insert(key.clone(), value.clone());
    }
    base
}

pub fn enum_prop(name: &str, enum_path: &str, entries: &[&str]) -> Value {
    let options = entries
        .iter()
        .map(|entry| json!({"name": entry, "isMax": entry.ends_with("_MAX")}))
        .collect::<Vec<_>>();
    merge(
        prop(name, "ByteProperty", "uint8"),
        json!({"enum": enum_path, "options": options}),
    )
}

pub fn enum_param(name: &str, enum_path: &str, entries: &[&str]) -> Value {
    merge(
        enum_prop(name, enum_path, entries),
        json!({"flags": flags(&["Parm"])}),
    )
}

pub fn class_param(name: &str, meta: &str) -> Value {
    merge(
        prop(name, "ClassProperty", "TSubclassOf<UObject>"),
        json!({"metaClass": meta, "propertyClass": "/Script/CoreUObject.Class", "flags": flags(&["Parm"])}),
    )
}

pub fn param(name: &str, kind: &str, cpp: &str) -> Value {
    merge(prop(name, kind, cpp), json!({"flags": flags(&["Parm"])}))
}

pub fn function(
    owner: &str,
    name: &str,
    function_flags: &[&str],
    params: Vec<Value>,
    ret: Option<Value>,
) -> Value {
    let mut map = Map::new();
    map.insert("name".into(), json!(name.to_lowercase()));
    map.insert("authoredName".into(), json!(name));
    map.insert(
        "path".into(),
        json!(format!("{owner}:{}", name.to_lowercase())),
    );
    map.insert("ownerPath".into(), json!(owner));
    map.insert("flags".into(), flags(function_flags));
    map.insert("params".into(), Value::Array(params));
    if let Some(ret) = ret {
        map.insert(
            "return".into(),
            merge(
                ret,
                json!({"flags": flags(&["Parm", "OutParm", "ReturnParm"])}),
            ),
        );
    }
    Value::Object(map)
}

pub fn class(path: &str, parent: Option<&str>, props: Vec<Value>, functions: Vec<Value>) -> Value {
    let properties = props
        .into_iter()
        .map(|property| merge(property, json!({"ownerPath": path})))
        .collect::<Vec<_>>();
    json!({
        "recordType": "class", "path": path,
        "name": path.rsplit('.').next().unwrap_or(path), "package": "/Script/Test",
        "source": {"kind": "game", "ref": "FactoryGame", "version": "t"},
        "super": parent, "kind": "native", "flags": flags(&[]),
        "properties": properties, "functions": functions
    })
}

pub fn abstract_class(mut record: Value) -> Value {
    record["flags"] = flags(&["Abstract"]);
    record
}

/// The dataset used by the reflection tests.
pub fn dataset() -> (Vec<Value>, Vec<Value>) {
    let item = "/Script/FactoryGame.FGItemDescriptor";
    let amount = merge(
        prop("mAmounts", "ArrayProperty", "TArray<FItemAmount>"),
        json!({"inner": merge(prop("mAmounts", "StructProperty", "FItemAmount"), json!({"struct": "/Script/FactoryGame.ItemAmount"}))}),
    );
    let tags = merge(
        prop("mTags", "ArrayProperty", "TArray<FGameplayTag>"),
        json!({"inner": merge(prop("mTags", "StructProperty", "FGameplayTag"), json!({"struct": "/Script/GameplayTags.GameplayTag"}))}),
    );
    let object = class("/Script/CoreUObject.Object", None, vec![], vec![]);
    let actor = class(
        "/Script/Engine.Actor",
        Some("/Script/CoreUObject.Object"),
        vec![],
        vec![],
    );
    let component = class(
        "/Script/Engine.ActorComponent",
        Some("/Script/CoreUObject.Object"),
        vec![],
        vec![],
    );
    let subsystem = class(
        "/Script/Engine.Subsystem",
        Some("/Script/CoreUObject.Object"),
        vec![],
        vec![],
    );
    let math = class(
        "/Script/Engine.KismetMathLibrary",
        Some("/Script/CoreUObject.Object"),
        vec![],
        vec![function(
            "/Script/Engine.KismetMathLibrary",
            "Max_DoubleDouble",
            &[
                "Native",
                "Static",
                "BlueprintPure",
                "Public",
                "BlueprintCallable",
            ],
            vec![
                param("A", "DoubleProperty", "double"),
                param("B", "DoubleProperty", "double"),
            ],
            Some(prop("ReturnValue", "DoubleProperty", "double")),
        )],
    );
    let pure_static = [
        "Native",
        "Static",
        "BlueprintPure",
        "Public",
        "BlueprintCallable",
    ];
    let item_class = class(
        item,
        Some("/Script/CoreUObject.Object"),
        vec![
            prop("mDisplayName", "TextProperty", "FText"),
            enum_prop(
                "mStackSize",
                "/Script/FactoryGame.EStackSize",
                &[
                    "SS_ONE",
                    "SS_SMALL",
                    "SS_MEDIUM",
                    "SS_BIG",
                    "SS_HUGE",
                    "SS_MAX",
                ],
            ),
            enum_prop(
                "mForm",
                "/Script/FactoryGame.EResourceForm",
                &["RF_INVALID", "RF_SOLID", "RF_LIQUID", "RF_GAS", "RF_MAX"],
            ),
            prop("mEnergyValue", "FloatProperty", "float"),
            prop("mCanBeDiscarded", "BoolProperty", "bool"),
            amount,
            tags,
        ],
        vec![
            function(
                item,
                "GetItemName",
                &pure_static,
                vec![class_param("ItemClass", item)],
                Some(prop("ReturnValue", "TextProperty", "FText")),
            ),
            function(
                item,
                "GetStackSize",
                &pure_static,
                vec![class_param("ItemClass", item)],
                Some(prop("ReturnValue", "IntProperty", "int32")),
            ),
            function(
                item,
                "GetEnergyValue",
                &pure_static,
                vec![
                    class_param("ItemClass", item),
                    param("Multiplier", "FloatProperty", "float"),
                ],
                Some(prop("ReturnValue", "FloatProperty", "float")),
            ),
            function(
                item,
                "Total",
                &pure_static,
                vec![merge(
                    param("Amount", "StructProperty", "FItemAmount"),
                    json!({"struct": "/Script/FactoryGame.ItemAmount"}),
                )],
                Some(prop("ReturnValue", "IntProperty", "int32")),
            ),
            function(
                item,
                "ForEachSlot",
                &["Native", "Exec"],
                vec![param("Slot", "IntProperty", "int32")],
                Some(prop("ReturnValue", "IntProperty", "int32")),
            ),
            function(
                item,
                "Blueprinty",
                &["Native", "BlueprintEvent", "BlueprintPure"],
                vec![],
                Some(prop("ReturnValue", "IntProperty", "int32")),
            ),
            function(
                item,
                "Replicated",
                &["Native", "Net", "NetServer", "BlueprintPure"],
                vec![],
                Some(prop("ReturnValue", "IntProperty", "int32")),
            ),
            function(
                item,
                "GetObjectLabel",
                &["Native", "Const", "Public"],
                vec![],
                Some(prop("ReturnValue", "StrProperty", "FString")),
            ),
            function(
                item,
                "Split",
                &pure_static,
                vec![
                    param("Value", "IntProperty", "int32"),
                    merge(
                        param("Rest", "IntProperty", "int32"),
                        json!({"flags": flags(&["Parm", "OutParm"])}),
                    ),
                ],
                Some(prop("ReturnValue", "IntProperty", "int32")),
            ),
            function(
                item,
                "Fetch",
                &pure_static,
                vec![],
                Some(prop("ReturnValue", "ObjectProperty", "UObject*")),
            ),
            function(
                item,
                "DescribeForm",
                &pure_static,
                vec![
                    enum_param(
                        "Form",
                        "/Script/FactoryGame.EResourceForm",
                        &["RF_INVALID", "RF_SOLID", "RF_LIQUID", "RF_GAS", "RF_MAX"],
                    ),
                    param("Detailed", "BoolProperty", "bool"),
                ],
                Some(prop("ReturnValue", "StrProperty", "FString")),
            ),
            function(
                item,
                "PickClass",
                &pure_static,
                vec![
                    class_param("Which", item),
                    param("Count", "IntProperty", "int32"),
                ],
                Some(prop("ReturnValue", "StrProperty", "FString")),
            ),
        ],
    );
    let resource = abstract_class(class(
        "/Script/FactoryGame.FGResourceDescriptor",
        Some(item),
        vec![],
        vec![],
    ));
    let recipe = class(
        "/Script/FactoryGame.FGRecipe",
        Some("/Script/CoreUObject.Object"),
        vec![
            prop("mManufactoringDuration", "FloatProperty", "float"),
            merge(
                prop("mDisplayName", "TextProperty", "FText"),
                json!({"kdf": {"reload": "cold", "reloadReason": "annotated by the exporter"}}),
            ),
        ],
        vec![],
    );
    let category = class(
        "/Script/FactoryGame.FGCategory",
        Some("/Script/CoreUObject.Object"),
        vec![],
        vec![],
    );
    let item_category = class(
        "/Script/FactoryGame.FGItemCategory",
        Some("/Script/FactoryGame.FGCategory"),
        vec![],
        vec![],
    );
    let buildable = class(
        "/Script/FactoryGame.FGBuildable",
        Some("/Script/Engine.Actor"),
        vec![prop("mHealth", "FloatProperty", "float")],
        vec![
            function(
                "/Script/FactoryGame.FGBuildable",
                "GetHealthPercent",
                &["Native", "Const", "Public"],
                vec![],
                Some(prop("ReturnValue", "FloatProperty", "float")),
            ),
            function(
                "/Script/FactoryGame.FGBuildable",
                "StaticLabel",
                &pure_static,
                vec![],
                Some(prop("ReturnValue", "StrProperty", "FString")),
            ),
        ],
    );
    let schematic = class(
        "/Script/FactoryGame.FGSchematic",
        Some("/Script/CoreUObject.Object"),
        vec![merge(
            prop("mUnlocks", "ArrayProperty", "TArray<UFGUnlock*>"),
            json!({"inner": prop("mUnlocks", "ObjectProperty", "UFGUnlock*")}),
        )],
        vec![],
    );
    let mut desc_test = class(
        "/Game/Items/Desc_Test.Desc_Test_C",
        Some(item),
        vec![merge(
            prop("mcustomvalue", "FloatProperty", "float"),
            json!({"authoredName": "mCustomValue"}),
        )],
        vec![],
    );
    desc_test["kind"] = json!("blueprint");
    let mut schematic_test = class(
        "/Game/Schematics/Test.Test_C",
        Some("/Script/FactoryGame.FGSchematic"),
        vec![],
        vec![],
    );
    schematic_test["kind"] = json!("blueprint");
    let data_asset = class(
        "/Script/Engine.DataAsset",
        Some("/Script/CoreUObject.Object"),
        vec![],
        vec![],
    );
    let kapi = class(
        "/Script/KAPI.KAPIDataAssetBase",
        Some("/Script/Engine.DataAsset"),
        vec![prop("mTitle", "TextProperty", "FText")],
        vec![function(
            "/Script/KAPI.KAPIDataAssetBase",
            "Describe",
            &pure_static,
            vec![merge(
                param("Target", "ObjectProperty", "UKAPIDataAssetBase*"),
                json!({"propertyClass": "/Script/KAPI.KAPIDataAssetBase"}),
            )],
            Some(prop("ReturnValue", "StrProperty", "FString")),
        )],
    );
    let classes = vec![
        data_asset,
        kapi,
        object,
        actor,
        component,
        subsystem,
        math,
        item_class,
        resource,
        recipe,
        category,
        item_category,
        buildable,
        schematic,
        desc_test,
        schematic_test,
    ];
    let structs = vec![
        json!({
            "recordType": "struct", "path": "/Script/FactoryGame.ItemAmount",
            "source": {"kind": "game", "ref": "FactoryGame", "version": "t"}, "size": 16, "alignment": 8,
            "flags": flags(&[]),
            "properties": [
                merge(prop("ItemClass", "ClassProperty", "TSubclassOf<UFGItemDescriptor>"), json!({"metaClass": item, "propertyClass": "/Script/CoreUObject.Class"})),
                prop("Amount", "IntProperty", "int32"),
            ]
        }),
        json!({
            "recordType": "struct", "path": "/Script/GameplayTags.GameplayTag",
            "source": {"kind": "engine", "ref": "Engine", "version": "t"}, "size": 8, "alignment": 4,
            "flags": flags(&[]), "properties": [prop("TagName", "NameProperty", "FName")]
        }),
    ];
    (classes, structs)
}

pub struct ArchiveOptions {
    pub settings: bool,
    pub cold: bool,
    pub structs: bool,
    pub assets: bool,
    pub functions: bool,
}

impl Default for ArchiveOptions {
    fn default() -> Self {
        Self {
            settings: true,
            cold: true,
            structs: true,
            assets: true,
            functions: true,
        }
    }
}

pub fn asset_index() -> String {
    json!({
        "/Script/Engine.DataAsset": {
            "class": "/Script/Engine.DataAsset", "total": 2, "truncated": false,
            "assets": [
                {"path": "/Game/KMods/DA_Test.DA_Test", "package": "/Game/KMods/DA_Test", "assetName": "DA_Test",
                 "assetClass": "/Script/KAPI.KAPIDataAssetBase", "tags": {"a": "b"}},
                {"path": "/Game/KMods/DA_Recipe.DA_Recipe", "package": "/Game/KMods/DA_Recipe", "assetName": "DA_Recipe",
                 "assetClass": "/Script/FactoryGame.FGRecipe", "tags": {}}
            ]
        },
        "/Script/Engine.Texture2D": {
            "class": "/Script/Engine.Texture2D", "total": 1, "truncated": false,
            "assets": [{"path": "/Game/UI/T_Icon.T_Icon", "package": "/Game/UI/T_Icon", "assetName": "T_Icon",
                        "assetClass": "/Script/Engine.Texture2D", "tags": {}}]
        }
    })
    .to_string()
}

/// Writes a `.kdfdata` archive with the dataset and the optional sections.
pub fn archive(options: &ArchiveOptions) -> NamedTempFile {
    archive_with(Vec::new(), options)
}

/// Like [`archive`], with more class records.
pub fn archive_with(extra: Vec<Value>, options: &ArchiveOptions) -> NamedTempFile {
    let (mut classes, structs) = dataset();
    classes.extend(extra);
    if !options.functions {
        for record in &mut classes {
            record.as_object_mut().expect("object").remove("functions");
        }
    }
    let file = tempfile::Builder::new()
        .suffix(".kdfdata")
        .tempfile()
        .expect("temp archive");
    let mut zip = zip::ZipWriter::new(File::create(file.path()).expect("create archive"));
    let zip_options = SimpleFileOptions::default();
    let mut add = |name: &str, body: &str| {
        zip.start_file(name, zip_options).expect("start file");
        zip.write_all(body.as_bytes()).expect("write entry");
    };
    add(
        "manifest.json",
        &json!({
            "schemaVersion": 1, "engineVersion": "5.6.1-test", "metadataAvailable": true,
            "options": {"assets": true}, "counts": {"classes": classes.len()}, "failedLoads": []
        })
        .to_string(),
    );
    add(
        "classes.ndjson",
        &classes
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    );
    if options.structs {
        add(
            "structs.ndjson",
            &structs
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    if options.settings {
        add("indexes/settings.json", SETTINGS_INDEX);
    }
    if options.assets {
        add("indexes/assets.json", &asset_index());
    }
    if options.cold {
        add("catalog/cold_properties.json", COLD_CATALOG);
    }
    zip.finish().expect("finish archive");
    file
}

pub fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
    fs::write(path, content).expect("write file");
}

const DEMO_PACK: &str =
    include_str!("../../../kdf-validation/tests/fixtures/config/DataForge/ConfigDemo/pack.yml");
const DEMO_BALANCE: &str = include_str!(
    "../../../kdf-validation/tests/fixtures/config/DataForge/ConfigDemo/balance.config.yml"
);
const DEMO_CATEGORIES: &str = include_str!(
    "../../../kdf-validation/tests/fixtures/config/DataForge/ConfigDemo/categories.class.yml"
);
const OTHER_PACK: &str =
    include_str!("../../../kdf-validation/tests/fixtures/config/DataForge/Other/pack.yml");
const OTHER_SHARED: &str =
    include_str!("../../../kdf-validation/tests/fixtures/config/DataForge/Other/shared.config.yml");

/// Writes the two-pack configuration workspace of the validation tests below `root` (the
/// `DataForge` folder): `ConfigDemo` (balance config, category class) and `Other` (shared config).
pub fn config_workspace(root: &Path) {
    write(root, "ConfigDemo/pack.yml", DEMO_PACK);
    write(root, "ConfigDemo/balance.config.yml", DEMO_BALANCE);
    write(root, "ConfigDemo/categories.class.yml", DEMO_CATEGORIES);
    write(root, "Other/pack.yml", OTHER_PACK);
    write(root, "Other/shared.config.yml", OTHER_SHARED);
}

/// Position of the `|` cursor marker (UTF-16 columns) and the text without it.
pub fn marker(source: &str) -> (String, Value) {
    let offset = source.find('|').expect("cursor marker");
    let before = &source[..offset];
    let line = before.matches('\n').count();
    let tail = before.rsplit('\n').next().unwrap_or("");
    (
        format!("{}{}", before, &source[offset + 1..]),
        json!({"line": line, "character": tail.encode_utf16().count()}),
    )
}

/// A language server process spoken to over stdio.
pub struct Client {
    child: Child,
    /// Closed on shutdown: the server only leaves its stdin read when the pipe ends.
    stdin: Option<ChildStdin>,
    rx: Receiver<Value>,
    next_id: i64,
    /// Notifications that arrived while a response was awaited.
    seen: Vec<Value>,
}

impl Client {
    /// Starts `binary`, initializes it for `root_uri` and returns the initialize result.
    pub fn start(binary: &str, root_uri: &str, capabilities: Value) -> (Self, Value) {
        Self::start_with(binary, root_uri, capabilities, Value::Null)
    }

    /// Like [`Self::start`], with `initializationOptions` (omitted when `Null`).
    pub fn start_with(
        binary: &str,
        root_uri: &str,
        capabilities: Value,
        options: Value,
    ) -> (Self, Value) {
        let mut child = Command::new(binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn language server");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut length = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("Content-Length: ") {
                        length = value.trim().parse::<usize>().ok();
                    }
                }
                let Some(length) = length else {
                    return;
                };
                let mut body = vec![0; length];
                if reader.read_exact(&mut body).is_err() {
                    return;
                }
                if let Ok(value) = serde_json::from_slice(&body) {
                    if tx.send(value).is_err() {
                        return;
                    }
                }
            }
        });
        let mut client = Self {
            child,
            stdin: Some(stdin),
            rx,
            next_id: 0,
            seen: Vec::new(),
        };
        let mut params =
            json!({"processId": null, "rootUri": root_uri, "capabilities": capabilities});
        if !options.is_null() {
            params["initializationOptions"] = options;
        }
        let init = client.request("initialize", params);
        client.notify("initialized", json!({}));
        (client, init)
    }

    fn send(&mut self, message: Value) {
        let bytes = serde_json::to_vec(&message).expect("json");
        let stdin = self.stdin.as_mut().expect("stdin is open");
        write!(stdin, "Content-Length: {}\r\n\r\n", bytes.len()).expect("write header");
        stdin.write_all(&bytes).expect("write body");
        stdin.flush().expect("flush");
    }

    pub fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Sends a request and returns the whole response message.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        for _ in 0..200 {
            let message = self
                .rx
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|_| panic!("no response to {method}"));
            if message.get("id") == Some(&json!(id)) && message.get("method").is_none() {
                return message;
            }
            self.seen.push(message);
        }
        panic!("response to {method} never arrived");
    }

    pub fn open(&mut self, uri: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": uri, "languageId": "yaml", "version": 1, "text": text}}),
        );
    }

    /// Diagnostics of the next `publishDiagnostics` for `uri`.
    pub fn diagnostics(&mut self, uri: &str) -> Vec<Value> {
        let matches = |message: &Value| {
            message.get("method") == Some(&json!("textDocument/publishDiagnostics"))
                && message["params"]["uri"] == json!(uri)
        };
        if let Some(index) = self.seen.iter().position(matches) {
            return self.seen.remove(index)["params"]["diagnostics"]
                .as_array()
                .cloned()
                .unwrap_or_default();
        }
        for _ in 0..200 {
            let message = self
                .rx
                .recv_timeout(Duration::from_secs(10))
                .expect("diagnostics timed out");
            if matches(&message) {
                return message["params"]["diagnostics"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
            }
            self.seen.push(message);
        }
        panic!("no diagnostics for {uri}");
    }

    /// Waits for a completed validation of the exact client URI and document version.
    /// All messages, including the matched publication, remain available to later collectors.
    #[track_caller]
    pub fn wait_for_published(&mut self, uri: &str, version: i64) -> Value {
        let matches = |message: &Value| {
            message.get("method") == Some(&json!("textDocument/publishDiagnostics"))
                && message["params"]["uri"] == json!(uri)
                && message["params"]["version"] == json!(version)
                && message["params"]["diagnostics"].is_array()
        };
        if let Some(message) = self.seen.iter().find(|message| matches(message)) {
            return message.clone();
        }
        let deadline = Instant::now() + Duration::from_secs(40);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let observed: Vec<_> = self.seen.iter().rev().take(20).collect();
                panic!(
                    "publication deadline reached for {uri} version {version}; last messages (newest first): {observed:?}"
                );
            }
            let message = match self.rx.recv_timeout(remaining) {
                Ok(message) => message,
                Err(error) => {
                    let observed: Vec<_> = self.seen.iter().rev().take(20).collect();
                    panic!(
                        "publication receive failed for {uri} version {version}: {error}; last messages (newest first): {observed:?}"
                    );
                }
            };
            let matched = matches(&message);
            self.seen.push(message);
            if matched {
                return self.seen.last().expect("received publication").clone();
            }
        }
    }

    /// Every `publishDiagnostics` message that is already waiting or arrives within `window`.
    pub fn published_within(&mut self, window: Duration) -> Vec<Value> {
        let is_publish = |message: &Value| {
            message.get("method") == Some(&json!("textDocument/publishDiagnostics"))
        };
        let mut found: Vec<Value> = Vec::new();
        let (kept, taken): (Vec<Value>, Vec<Value>) = std::mem::take(&mut self.seen)
            .into_iter()
            .partition(|message| !is_publish(message));
        self.seen = kept;
        found.extend(taken);
        let deadline = std::time::Instant::now() + window;
        while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
            match self.rx.recv_timeout(remaining) {
                Ok(message) if is_publish(&message) => found.push(message),
                Ok(message) => self.seen.push(message),
                Err(_) => break,
            }
        }
        found
    }

    pub fn shutdown(mut self) {
        let _ = self.request("shutdown", Value::Null);
        self.notify("exit", Value::Null);
        self.stdin = None;
        let started = std::time::Instant::now();
        while self.child.try_wait().ok().flatten().is_none() {
            if started.elapsed() > Duration::from_secs(5) {
                let _ = self.child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
