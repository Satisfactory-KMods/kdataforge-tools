//! Shared helpers: synthetic reflection archives (classes, functions, structs, settings index and
//! cold catalog) and small workspace builders.
#![allow(dead_code)]

use kdf_validation::{Diagnostic, LintResult};
use serde_json::{json, Map, Value};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use tempfile::NamedTempFile;
use zip::write::SimpleFileOptions;

pub const SETTINGS_INDEX: &str = include_str!("../fixtures/settings.index.sample.json");
pub const COLD_CATALOG: &str = include_str!("../fixtures/cold-properties.json");

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
    let (mut classes, structs) = dataset();
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

/// `(file name, code, yaml path)` triples, sorted.
pub fn triples(result: &LintResult, root: &Path) -> Vec<(String, String, String)> {
    let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut out = result
        .diagnostics
        .iter()
        .map(|diagnostic| {
            let file = diagnostic
                .file
                .as_ref()
                .map(|path| {
                    path.strip_prefix(&root)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .unwrap_or_default();
            (file, diagnostic.code.clone(), diagnostic.yaml_path.clone())
        })
        .collect::<Vec<_>>();
    out.sort();
    out
}

pub fn find<'a>(result: &'a LintResult, code: &str) -> Vec<&'a Diagnostic> {
    result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == code)
        .collect()
}
