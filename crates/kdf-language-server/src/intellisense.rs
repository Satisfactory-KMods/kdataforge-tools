//! Incomplete YAML context without reparsing a workspace or loading assets per keystroke.
use crate::features::yaml_content_prefix;
use crate::selector_context::{references, remember_anchor, SelectorKind, RECIPE_SCOPE};
use kdf_validation::{
    byte_offset_at_utf16, field_docs, Position as KdfPosition, ReflectionIndex, OPERATIONS,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use tower_lsp::lsp_types::*;

#[derive(Default)]
struct Scope {
    indent: usize,
    start_line: usize,
    targets: Vec<String>,
    recipe_filter: bool,
    of_class: bool,
    has_selector: bool,
    property: Option<String>,
}

#[derive(Default)]
pub(crate) struct Context {
    pub(crate) targets: Vec<String>,
    class_sequence: bool,
    class_sequence_key: Option<&'static str>,
    pub(crate) property: Option<String>,
    pub(crate) used_properties: HashMap<String, u32>,
    pub(crate) has_selector: bool,
}

fn scalar(value: &str) -> String {
    serde_yaml::from_str::<Value>(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| value.trim().trim_matches(['\'', '"']).to_owned())
}

pub(crate) fn context(text: &str, position: Position) -> Context {
    let mut scopes = vec![Scope::default()];
    let mut target_list: Option<(usize, usize, SelectorKind)> = None;
    let mut anchors = HashMap::new();
    for (line_number, line) in text.lines().take(position.line as usize + 1).enumerate() {
        let content = yaml_content_prefix(line);
        let trimmed = content.trim();
        if trimmed == "---" || trimmed == "..." {
            scopes = vec![Scope::default()];
            target_list = None;
            anchors.clear();
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        let indent = content.len() - content.trim_start().len();
        let sequence = content.trim_start().starts_with("- ");
        if let Some((owner, key_indent, kind)) = target_list {
            if indent >= key_indent && sequence && scopes.get(owner).is_some() {
                let value = content.trim_start().strip_prefix("- ").unwrap_or("").trim();
                let values = references(value, &anchors);
                if !value.contains(": ") {
                    match kind {
                        SelectorKind::Target => scopes[owner].targets.extend(values),
                        SelectorKind::RecipeFilter(_) => {
                            scopes[owner].recipe_filter |= !values.is_empty()
                        }
                    }
                    continue;
                }
            }
            target_list = None;
        }
        if sequence {
            while scopes.len() > 1 && scopes.last().is_some_and(|scope| scope.indent >= indent) {
                scopes.pop();
            }
            scopes.push(Scope {
                indent,
                start_line: line_number,
                ..Default::default()
            });
        } else {
            while scopes.len() > 1 && scopes.last().is_some_and(|scope| scope.indent + 2 > indent) {
                scopes.pop();
            }
        }
        let mapping = if sequence {
            content
                .trim_start()
                .strip_prefix("- ")
                .unwrap_or("")
                .trim_start()
        } else {
            trimmed
        };
        let Some((key, value)) = mapping.split_once(':') else {
            continue;
        };
        let value = value.trim();
        remember_anchor(value, &mut anchors);
        let scope_index = scopes.len() - 1;
        let scope = scopes.last_mut().unwrap();
        match key.trim() {
            "target" | "class" | "parent" | "ofClass" | "allAssetsOfClass" => {
                scope.has_selector = true;
                scope.targets = references(value, &anchors);
                scope.of_class |= key.trim() == "ofClass";
                if value.is_empty() {
                    target_list = Some((
                        scope_index,
                        indent + if sequence { 2 } else { 0 },
                        SelectorKind::Target,
                    ));
                }
            }
            "producedIn" | "ingredient" | "product" => {
                scope.has_selector = true;
                scope.recipe_filter |= !references(value, &anchors).is_empty();
                if value.is_empty() {
                    target_list = Some((
                        scope_index,
                        indent + if sequence { 2 } else { 0 },
                        SelectorKind::RecipeFilter(match key.trim() {
                            "ingredient" => "ingredient",
                            "product" => "product",
                            _ => "producedIn",
                        }),
                    ));
                }
            }
            "path" => scope.property = Some(scalar(value)),
            _ => {}
        }
    }
    let owner = scopes
        .iter()
        .rev()
        .find(|scope| scope.has_selector)
        .or_else(|| scopes.get(1));
    let mut targets = owner.map(|scope| scope.targets.clone()).unwrap_or_default();
    if owner.is_some_and(|scope| scope.recipe_filter && !scope.of_class) {
        targets.push(RECIPE_SCOPE.into());
    }
    let field_indent = scopes
        .iter()
        .rev()
        .find(|scope| scope.property.is_some())
        .map(|scope| scope.indent + 2);
    let mut used_properties = HashMap::new();
    if let Some(owner) = owner {
        for (line_number, line) in text.lines().enumerate().skip(owner.start_line + 1) {
            let content = yaml_content_prefix(line);
            let trimmed = content.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let indent = content.len() - content.trim_start().len();
            if trimmed == "---"
                || trimmed == "..."
                || indent < owner.indent
                || (indent == owner.indent && trimmed.starts_with("- "))
            {
                break;
            }
            if line_number == position.line as usize {
                continue;
            }
            let mapping_indent = indent + if trimmed.starts_with("- ") { 2 } else { 0 };
            if field_indent != Some(mapping_indent) {
                continue;
            }
            let mapping = trimmed.trim_start_matches("- ");
            if let Some(("path", value)) = mapping
                .split_once(':')
                .map(|(key, value)| (key.trim(), value.trim()))
            {
                let name = scalar(value);
                if !name.is_empty() {
                    used_properties
                        .entry(name)
                        .or_insert(line_number as u32 + 1);
                }
            }
        }
    }
    Context {
        targets,
        class_sequence: target_list.is_some(),
        class_sequence_key: target_list.map(|(_, _, kind)| match kind {
            SelectorKind::Target => "target",
            SelectorKind::RecipeFilter(key) => key,
        }),
        property: scopes.iter().rev().find_map(|scope| scope.property.clone()),
        used_properties,
        has_selector: owner.is_some_and(|scope| scope.has_selector),
    }
}

fn context_properties<'a>(index: &'a ReflectionIndex, context: &Context) -> Vec<&'a Value> {
    // Never silently ignore selected targets: unavailable or conflicting metadata
    // cannot prove a property safe for the complete target list.
    if context.targets.is_empty() {
        if context.has_selector {
            return Vec::new();
        }
        let signatures = context
            .used_properties
            .keys()
            .map(String::as_str)
            .chain(context.property.as_deref())
            .take(65)
            .collect::<Vec<_>>();
        return index
            .infer_property_owner(&signatures)
            .map(|owner| properties(index, owner))
            .unwrap_or_default();
    }
    if context.targets.len() > 64 {
        return Vec::new();
    }
    let mut result = properties(index, &context.targets[0]);
    for target in context.targets.iter().skip(1) {
        let fields = properties(index, target);
        result.retain(|field| {
            fields
                .iter()
                .any(|other| other["name"] == field["name"] && other["cppType"] == field["cppType"])
        });
    }
    result
}

pub(crate) fn properties<'a>(index: &'a ReflectionIndex, target: &str) -> Vec<&'a Value> {
    let mut result = Vec::new();
    let mut seen_classes = HashSet::new();
    let mut seen_names = HashSet::new();
    let mut path = Some(target);
    while let Some(current) = path {
        if !seen_classes.insert(current) {
            break;
        }
        let Some(class) = index.class_info(current) else {
            break;
        };
        let Some(properties) = class.get("properties").and_then(Value::as_array) else {
            break;
        };
        for property in properties {
            if let Some(name) = property.get("name").and_then(Value::as_str) {
                if seen_names.insert(name) {
                    result.push(property);
                }
            }
        }
        path = class.get("super").and_then(Value::as_str);
    }
    result
}

pub(crate) fn property<'a>(index: &'a ReflectionIndex, ctx: &Context) -> Option<&'a Value> {
    let name = ctx.property.as_deref()?.split(['.', '[']).next()?;
    context_properties(index, ctx)
        .into_iter()
        .find(|property| property["name"] == name)
}

fn supported_operations<'a>(
    index: &'a ReflectionIndex,
    ctx: &Context,
    field: &'a Value,
) -> Option<Vec<&'a str>> {
    let mut ops = field
        .pointer("/kdf/ops")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    for target in ctx.targets.iter().skip(1) {
        let other = properties(index, target)
            .into_iter()
            .find(|other| other["name"] == field["name"])?;
        let supported = other.pointer("/kdf/ops")?.as_array()?;
        ops.retain(|op| supported.iter().any(|other| other.as_str() == Some(op)));
    }
    Some(ops)
}

pub(crate) fn contextual_property_markdown(
    index: &ReflectionIndex,
    ctx: &Context,
    field: &Value,
) -> String {
    if ctx.targets.len() < 2 {
        return property_markdown(field);
    }
    let mut common = field.clone();
    common["kdf"] =
        serde_json::json!({"ops": supported_operations(index, ctx, field).unwrap_or_default()});
    property_markdown(&common)
}

fn tooltip(property: &Value) -> &str {
    property
        .pointer("/metadata/ToolTip")
        .or_else(|| property.pointer("/metadata/Tooltip"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// Documentation examples describe the reflected value shape, never pretend to be defaults.
fn value_example(property: &Value) -> Option<&'static str> {
    let cpp_type = property["cppType"].as_str()?;
    if cpp_type == "FItemAmount" {
        Some("value:\n  ItemClass: /Game/.../Desc_Item.Desc_Item_C\n  Amount: 1")
    } else if cpp_type == "TArray<FItemAmount>" {
        Some("value:\n  - ItemClass: /Game/.../Desc_Item.Desc_Item_C\n    Amount: 1")
    } else if cpp_type.starts_with("TArray<") {
        Some("value: []")
    } else if cpp_type.starts_with("TMap<") {
        Some("value: {}")
    } else if cpp_type == "bool" {
        Some("value: true")
    } else if matches!(
        cpp_type,
        "float"
            | "double"
            | "int8"
            | "uint8"
            | "int16"
            | "uint16"
            | "int32"
            | "uint32"
            | "int64"
            | "uint64"
    ) {
        Some("value: 1")
    } else if cpp_type == "FText" {
        Some("value: { stringTable: /MyMod/ST_Text.ST_Text, key: DisplayName }\n# Or: value: { ns: MyMod, key: DisplayName, source: Example }")
    } else if matches!(cpp_type, "FString" | "FName") {
        Some("value: \"example\"")
    } else {
        None
    }
}

pub(crate) fn property_markdown(property: &Value) -> String {
    let name = property["name"].as_str().unwrap_or("Property");
    let cpp_type = property["cppType"].as_str().unwrap_or("unknown type");
    let owner = property["ownerPath"].as_str().unwrap_or("");
    let ops = property
        .pointer("/kdf/ops")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(|op| format!("`{op}`"))
                .collect::<Vec<_>>()
                .join(" · ")
        })
        .unwrap_or_default();
    let mut result = format!(
        "### {name}\n\n**Type** `{cpp_type}`{}\n",
        if owner.is_empty() {
            String::new()
        } else {
            format!("\n\n**Declared in** `{owner}`")
        }
    );
    if !tooltip(property).is_empty() {
        result.push_str(&format!("\n{}\n", tooltip(property)));
    }
    if let Some(units) = property
        .pointer("/metadata/Units")
        .and_then(Value::as_str)
        .filter(|units| !units.is_empty())
    {
        result.push_str(&format!("\n**Units** `{units}`\n"));
    }
    if !ops.is_empty() {
        result.push_str(&format!("\n**Operations**\n\n{ops}\n"));
    }
    if let Some(example) = value_example(property) {
        result.push_str(&format!(
            "\n**YAML value example**\n\n```yaml\n{example}\n```\n"
        ));
    }
    result
}

fn markdown(text: String) -> Documentation {
    Documentation::MarkupContent(MarkupContent {
        kind: MarkupKind::Markdown,
        value: text,
    })
}

fn catalog_value_documentation(key: &str, value: &str) -> Option<Documentation> {
    field_docs::value(key, value)
        .map(field_docs::to_markdown)
        .map(markdown)
}

pub(crate) fn class_fields(
    name: &str,
    path: &str,
    source: Option<&str>,
    parent: Option<&str>,
) -> String {
    let mut result = format!("### {name}\n\n**Class path**\n\n`{path}`\n");
    if let Some(source) = source {
        result.push_str(&format!("\n**Source** {source}\n"));
    }
    if let Some(parent) = parent {
        result.push_str(&format!("\n**Parent** `{parent}`\n"));
    }
    result
}

/// The hover shares completion's hierarchy; explanatory prose comes only from reflection.
pub(crate) fn class_markdown(class: &Value, path: &str) -> String {
    let name = class.get("name").and_then(Value::as_str).unwrap_or(path);
    let source = class
        .pointer("/source/ref")
        .or_else(|| class.get("source"))
        .and_then(Value::as_str);
    let parent = class.get("super").and_then(Value::as_str);
    let mut result = class_fields(name, path, source, parent);
    if let Some(kind) = class.get("kind").and_then(Value::as_str) {
        result.push_str(&format!("\n**Kind** `{kind}`\n"));
    }
    if !tooltip(class).is_empty() {
        result.push_str(&format!("\n{}\n", tooltip(class)));
    }
    result
}

/// Runtime reflection erases some domain constraints to `UObject`. Bind those fields to an
/// existing selector contract by reflected owner and name; every other field keeps its exported
/// meta class.
pub(crate) fn compatible_class_base(property: &Value) -> Option<&str> {
    if property["ownerPath"] == RECIPE_SCOPE && property["name"] == "mProducedIn" {
        return kdf_validation::template::base_class("building");
    }
    property
        .get("metaClass")
        .or_else(|| property.pointer("/inner/metaClass"))
        .and_then(Value::as_str)
}

#[cfg(test)]
pub(crate) fn completion(
    text: &str,
    position: Position,
    index: &ReflectionIndex,
) -> Option<CompletionResponse> {
    let resolved = crate::context::resolve(std::path::Path::new("document.yml"), text, position);
    completion_resolved(text, &resolved, index)
}

pub(crate) fn completion_resolved(
    text: &str,
    resolved: &crate::context::Resolved,
    index: &ReflectionIndex,
) -> Option<CompletionResponse> {
    let position = resolved.position()?;
    if let Some(slot) = crate::class_slots::for_completion(text, resolved, index) {
        return Some(crate::class_slots::completion(slot, index));
    }
    let line = text.lines().nth(position.line as usize).unwrap_or("");
    let content = yaml_content_prefix(line);
    let byte = byte_offset_at_utf16(
        line,
        KdfPosition {
            line: 0,
            character: position.character,
        },
    )
    .unwrap_or(line.len());
    if byte > content.len() {
        return None;
    }
    let ctx = context(text, position);
    let current_property = property(index, &ctx);
    let (key, value_start) = if let Some(colon) = content.find(':') {
        if byte <= colon {
            return None;
        }
        (
            content[..colon].trim().trim_start_matches('-').trim(),
            colon + 1 + content[colon + 1..].len() - content[colon + 1..].trim_start().len(),
        )
    } else {
        let indent = content.len() - content.trim_start().len();
        if !content[indent..].starts_with("- ")
            || (current_property.is_none() && !ctx.class_sequence)
        {
            return None;
        }
        (
            if ctx.class_sequence {
                ctx.class_sequence_key.unwrap_or("target")
            } else {
                "value"
            },
            indent + 2,
        )
    };
    let quote = content[value_start..].starts_with(['\'', '"']);
    let start = value_start + usize::from(quote);
    if byte < start {
        return None;
    }
    let fragment = content[start..byte]
        .trim_end_matches(['\'', '"'])
        .to_lowercase();
    let end = content
        .trim_end()
        .trim_end_matches(['\'', '"'])
        .len()
        .max(start);
    let range = Range::new(
        Position::new(position.line, line[..start].encode_utf16().count() as u32),
        Position::new(position.line, line[..end].encode_utf16().count() as u32),
    );
    let mut values: Vec<&str> = match key {
        "type" => kdf_validation::CANONICAL_TYPES.to_vec(),
        "subType" if resolved.doc.kind.as_deref() == Some("category_reorder") => {
            vec!["items", "recipe", "schematic", "settings"]
        }
        "op" => current_property
            .and_then(|field| supported_operations(index, &ctx, field))
            .unwrap_or_else(|| OPERATIONS.iter().map(|spec| spec.name).collect()),
        "conditionBehavior" | "conditionBehaivor" => vec!["AND", "OR"],
        "track" => vec!["Default", "Exploration"],
        "value" if current_property.is_some_and(|property| property["cppType"] == "bool") => {
            vec!["true", "false"]
        }
        _ => Vec::new(),
    };
    if key == "op"
        && resolved
            .op_entry()
            .is_some_and(|op| op.key_value("path") == Some(kdf_validation::TAG_CONTAINER_SELECTOR))
    {
        values.retain(|name| kdf_validation::tag_container_operation(name));
    }
    if !values.is_empty() {
        // An invalid op still offers the verified vocabulary. It never silently edits
        // a typo, and filterText allows the native client to display those choices.
        let recover_operation = key == "op"
            && !values
                .iter()
                .any(|value| value.to_lowercase().contains(&fragment));
        return Some(CompletionResponse::Array(
            values
                .into_iter()
                .filter(|value| recover_operation || value.to_lowercase().contains(&fragment))
                .map(|value| CompletionItem {
                    label: value.into(),
                    kind: Some(CompletionItemKind::ENUM_MEMBER),
                    insert_text: Some(value.into()),
                    filter_text: recover_operation.then(|| format!("{value} {fragment}")),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range,
                        new_text: value.into(),
                    })),
                    documentation: catalog_value_documentation(key, value),
                    ..Default::default()
                })
                .collect(),
        ));
    }
    if matches!(key, "path" | "from") {
        return Some(CompletionResponse::Array(
            context_properties(index, &ctx)
                .into_iter()
                .enumerate()
                .filter_map(|(order, property)| {
                    let name = property["name"].as_str()?;
                    if !name.to_lowercase().contains(&fragment) {
                        return None;
                    }
                    let used = ctx.used_properties.get(name);
                    let mut documentation = contextual_property_markdown(index, &ctx, property);
                    if let Some(line) = used {
                        documentation.push_str(&format!("\n**Used on line** {line}\n"));
                    }
                    Some(CompletionItem {
                        label: name.into(),
                        label_details: Some(CompletionItemLabelDetails {
                            detail: None,
                            description: property["cppType"].as_str().map(str::to_owned),
                        }),
                        kind: Some(CompletionItemKind::PROPERTY),
                        detail: property["cppType"].as_str().map(str::to_owned),
                        insert_text: Some(name.into()),
                        documentation: Some(markdown(documentation)),
                        sort_text: Some(format!("0:{order:05}:{name}")),
                        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                            range,
                            new_text: name.into(),
                        })),
                        ..Default::default()
                    })
                })
                .collect(),
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::testing::marked;
    use crate::test_fixture::Fixture;
    use crate::test_support::class;

    const BUILDABLE: &str = "/Script/FactoryGame.FGBuildable";
    const BUILDING: &str = "/Game/Factory/Build_Manufacturer.Build_Manufacturer_C";
    const ITEM: &str = "/Game/Items/Desc_Test.Desc_Test_C";
    const RECIPE: &str = "/Game/Recipes/Recipe_Test.Recipe_Test_C";

    fn class_array_property(name: &str, owner: &str, meta_class: &str) -> Value {
        serde_json::json!({
            "name": name,
            "ownerPath": owner,
            "kind": "ArrayProperty",
            "cppType": "TArray<TSoftClassPtr<UObject>>",
            "inner": {
                "kind": "SoftClassProperty",
                "cppType": "TSoftClassPtr<UObject>",
                "metaClass": meta_class
            }
        })
    }

    fn class_value_fixture() -> Fixture {
        let recipe = serde_json::json!({
            "recordType": "class",
            "path": RECIPE,
            "name": "Recipe_Test_C",
            "super": RECIPE_SCOPE,
            "properties": [
                class_array_property("mProducedIn", RECIPE_SCOPE, "/Script/CoreUObject.Object"),
                class_array_property("mItemClasses", RECIPE, "/Script/FactoryGame.FGItemDescriptor"),
                class_array_property("mAnyClasses", RECIPE, "/Script/CoreUObject.Object")
            ]
        });
        Fixture::build(
            &[],
            vec![recipe, class(BUILDING, Some(BUILDABLE), vec![], vec![])],
        )
    }

    fn complete_source(source: &str) -> CompletionResponse {
        let fixture = class_value_fixture();
        let (text, position) = marked(source);
        completion(&text, position, &fixture.reflection).expect("class completion")
    }

    fn items(response: CompletionResponse) -> Vec<CompletionItem> {
        match response {
            CompletionResponse::Array(items) => items,
            CompletionResponse::List(list) => list.items,
        }
    }

    fn assert_catalog_documentation(source: &str, key: &str, label: &str) {
        let items = items(complete_source(source));
        let item = items
            .iter()
            .find(|item| item.label == label)
            .unwrap_or_else(|| panic!("no completion {label:?}"));
        let Some(Documentation::MarkupContent(documentation)) = item.documentation.as_ref() else {
            panic!("catalog markdown for {label:?}");
        };
        assert_eq!(documentation.kind, MarkupKind::Markdown);
        assert_eq!(
            documentation.value,
            field_docs::to_markdown(field_docs::value(key, label).expect("catalog value"))
        );
        let wire = serde_json::to_value(item).expect("completion JSON");
        assert_eq!(
            wire.pointer("/documentation/kind").and_then(Value::as_str),
            Some("markdown")
        );
    }

    #[test]
    fn explicit_child_completion_stops_at_unknown_properties() {
        const BASE: &str = "/Script/Test.CompletionBase";
        const CHILD: &str = "/Script/Test.CompletionChild";
        let base = class(
            BASE,
            None,
            vec![serde_json::json!({
                "name":"mConflicting", "cppType":"bool", "kind":"BoolProperty", "default":"True"
            })],
            vec![],
        );
        let mut children = vec![serde_json::json!({"path":CHILD,"super":BASE})];
        for properties in [
            Value::Null,
            serde_json::json!(false),
            serde_json::json!(0),
            serde_json::json!("[]"),
            serde_json::json!({}),
            serde_json::json!([]),
        ] {
            children.push(serde_json::json!({"path":CHILD,"super":BASE,"properties":properties}));
        }
        for child in children {
            let available = child.get("properties").is_some_and(Value::is_array);
            let fixture = Fixture::build(&[], vec![base.clone(), child.clone()]);
            let source = format!(
                "type: cdo\npatches:\n  - target: {CHILD}\n    properties:\n      - path: m|"
            );
            let (text, position) = marked(&source);
            let suggestions =
                items(completion(&text, position, &fixture.reflection).expect("path completion"));
            assert_eq!(
                suggestions.iter().any(|item| item.label == "mConflicting"),
                available,
                "{child}"
            );
            assert_eq!(
                properties(&fixture.reflection, CHILD).len(),
                usize::from(available)
            );
            assert_eq!(properties(&fixture.reflection, BASE).len(), 1);
        }
    }

    #[test]
    fn enum_completions_use_catalog_markdown_with_valid_wire_kind() {
        assert_catalog_documentation("type: |", "type", "cdo");
        assert_catalog_documentation(
            &format!(
                "type: cdo\npatches:\n  - target: {RECIPE}\n    properties:\n      - path: mAnyClasses\n        op: |"
            ),
            "op",
            "set",
        );
        assert_catalog_documentation("type: cdo\nconditionBehavior: |", "conditionBehavior", "OR");
        assert_catalog_documentation(
            "type: cdo\nconditionBehaivor: |",
            "conditionBehaivor",
            "AND",
        );
        assert_catalog_documentation("type: research\ntrack: |", "track", "Default");
    }

    #[test]
    fn property_docs_preserve_reflection_and_distinguish_struct_array_shapes() {
        let mut property = serde_json::json!({"name":"mProduct","cppType":"TArray<FItemAmount>","ownerPath":"/Script/FactoryGame.FGRecipe","metadata":{"ToolTip":"Items produced."},"kdf":{"ops":["set","append"]}});
        let docs = property_markdown(&property);
        assert!(docs.contains("**Type** `TArray<FItemAmount>`"));
        assert!(docs.contains("Items produced."));
        assert!(docs.contains("**Declared in** `/Script/FactoryGame.FGRecipe`"));
        assert!(docs.contains("`set` · `append`"));
        assert!(docs.contains("value:\n  - ItemClass:"));
        property["cppType"] = Value::String("FItemAmount".into());
        assert!(property_markdown(&property).contains("value:\n  ItemClass:"));
    }
    #[test]
    fn absent_metadata_never_invents_tooltip_operations_or_unknown_value_shape() {
        let property = serde_json::json!({"name":"mCustom","cppType":"FUnknownStruct"});
        let docs = property_markdown(&property);
        assert_eq!(docs, "### mCustom\n\n**Type** `FUnknownStruct`\n");
        assert!(!docs.contains("Operations"));
        assert!(!docs.contains("YAML value example"));
    }
    #[test]
    fn class_hover_uses_authoritative_metadata_and_omits_missing_fields() {
        let class = serde_json::json!({"name":"Iron Ingot","kind":"blueprint","source":{"ref":"FactoryGame"},"super":"/Script/FactoryGame.FGItemDescriptor","metadata":{"ToolTip":"Reflected item tooltip."}});
        let docs = class_markdown(&class, "/Game/Items/Desc_Iron.Desc_Iron_C");
        assert!(docs.starts_with("### Iron Ingot"));
        assert!(docs.contains("**Source** FactoryGame"));
        assert!(docs.contains("**Kind** `blueprint`"));
        assert!(docs.contains("Reflected item tooltip."));
        let missing = class_markdown(&serde_json::json!({}), "/Script/Test.Example");
        assert!(!missing.contains("Source"));
        assert!(!missing.contains("Parent"));
        assert!(!missing.contains("unknown"));
    }
    #[test]
    fn context_discards_previous_patch_and_follows_nested_properties() {
        let text = "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mFirst\n        value: []\n  - target: /Game/B.B_C\n    properties:\n      - path: mSecond\n        value:\n          - ItemClass: /";
        let ctx = context(text, Position::new(10, 24));
        assert_eq!(ctx.targets, ["/Game/B.B_C"]);
        assert_eq!(ctx.property.as_deref(), Some("mSecond"));
    }
    #[test]
    fn context_resets_at_document_separator() {
        let ctx = context(
            "patches:\n  - target: /Game/A.A_C\n---\npath: mValue",
            Position::new(3, 12),
        );
        assert!(ctx.targets.is_empty());
    }
    #[test]
    fn used_fields_do_not_include_value_maps_or_adjacent_patches() {
        let text = "patches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mFirst\n        value:\n          path: mNested\n      - path: m\n  - target: /Game/B.B_C\n    properties:\n      - path: mOther";
        let ctx = context(text, Position::new(6, 15));
        assert_eq!(ctx.used_properties, HashMap::from([("mFirst".into(), 4)]));
    }

    #[test]
    fn repeated_paths_keep_native_completion_priority() {
        let fixture = class_value_fixture();
        let complete_paths = |source: &str| {
            let (text, position) = marked(source);
            items(completion(&text, position, &fixture.reflection).expect("path completion"))
        };
        let prefix = format!("type: cdo\npatches:\n  - target: {RECIPE}\n    properties:\n");
        let unused = complete_paths(&format!("{prefix}      - path: |"));
        for operation in ["set", "append", "remove", "clear"] {
            let repeated = complete_paths(&format!(
                "{prefix}      - path: mAnyClasses\n        op: {operation}\n        value: []\n      - path: |"
            ));
            assert_eq!(repeated.len(), unused.len(), "preceding {operation}");
            for expected in &unused {
                let actual = repeated
                    .iter()
                    .find(|item| item.label == expected.label)
                    .expect("used and unused properties remain available");
                assert_eq!(
                    actual.sort_text, expected.sort_text,
                    "preceding {operation}"
                );
            }
            let used = repeated
                .iter()
                .find(|item| item.label == "mAnyClasses")
                .unwrap();
            let Some(Documentation::MarkupContent(documentation)) = used.documentation.as_ref()
            else {
                panic!("property markdown");
            };
            assert!(documentation.value.contains("**Used on line** 5"));
        }
    }

    #[test]
    fn repeated_path_completion_preserves_quoted_replacement() {
        let fixture = class_value_fixture();
        let (text, position) = marked(&format!(
            "type: cdo\npatches:\n  - target: {RECIPE}\n    properties:\n      - path: mAnyClasses\n        op: append\n        value: []\n      - path: 'mAny|Classes' # keep"
        ));
        let completions = items(completion(&text, position, &fixture.reflection).unwrap());
        let used = completions
            .iter()
            .find(|item| item.label == "mAnyClasses")
            .unwrap();
        assert_eq!(
            used.text_edit,
            Some(CompletionTextEdit::Edit(TextEdit {
                range: Range::new(Position::new(7, 15), Position::new(7, 26)),
                new_text: "mAnyClasses".into(),
            }))
        );
    }

    #[test]
    fn produced_in_empty_list_offers_buildings_with_valid_wire_documentation() {
        let response = complete_source(&format!(
            "type: cdo\npatches:\n  - target: {RECIPE}\n    properties:\n      - path: mProducedIn\n        value:\n          - |"
        ));
        let items = items(response);
        let building = items
            .iter()
            .find(|item| item.insert_text.as_deref() == Some(BUILDING))
            .expect("compatible building");
        assert!(!items
            .iter()
            .any(|item| item.insert_text.as_deref() == Some(ITEM)));
        let wire = serde_json::to_value(building).expect("completion JSON");
        assert_eq!(
            wire.pointer("/documentation/kind").and_then(Value::as_str),
            Some("markdown")
        );
    }

    #[test]
    fn produced_in_contract_ignores_mixed_or_unknown_prior_values() {
        let response = complete_source(&format!(
            "type: cdo\npatches:\n  - target: {RECIPE}\n    properties:\n      - path: mProducedIn\n        value:\n          - /Game/Unknown.Unknown_C\n          - {ITEM}\n          - |"
        ));
        let items = items(response);
        assert!(items
            .iter()
            .any(|item| item.insert_text.as_deref() == Some(BUILDING)));
        assert!(!items
            .iter()
            .any(|item| item.insert_text.as_deref() == Some(ITEM)));
    }

    #[test]
    fn unrelated_typed_class_array_does_not_leak_buildings() {
        let response = complete_source(&format!(
            "type: cdo\npatches:\n  - target: {RECIPE}\n    properties:\n      - path: mItemClasses\n        value:\n          - |"
        ));
        let items = items(response);
        assert!(items
            .iter()
            .any(|item| item.insert_text.as_deref() == Some(ITEM)));
        assert!(!items
            .iter()
            .any(|item| item.insert_text.as_deref() == Some(BUILDING)));
    }

    #[test]
    fn broad_reflected_class_array_keeps_existing_fallback() {
        let response = complete_source(&format!(
            "type: cdo\npatches:\n  - target: {RECIPE}\n    properties:\n      - path: mAnyClasses\n        value:\n          - /Game/Unknown.Unknown_C\n          - {ITEM}\n          - |"
        ));
        let items = items(response);
        assert!(items
            .iter()
            .any(|item| item.insert_text.as_deref() == Some(BUILDING)));
        assert!(items
            .iter()
            .any(|item| item.insert_text.as_deref() == Some(ITEM)));
    }
}
