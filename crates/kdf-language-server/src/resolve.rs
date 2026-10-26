//! Completion item documentation on demand.
//!
//! A completion list of a few hundred members must not render a markdown card per item. Items carry
//! a small JSON `data` object instead (what is needed to find the card again) and the client asks for
//! the card of the item it shows with `completionItem/resolve`. Clients that cannot resolve lazily
//! get the card inline; the same [`card`] builds it in both cases.

use crate::cards::{config_card, function_card, property_card, setting_card};
use crate::state::ClientCaps;
use crate::workspace::Workspace;
use kdf_validation::config::ConfigField;
use kdf_validation::{ReflectionIndex, SettingRecord, TargetKind};
use serde_json::{json, Value};
use tower_lsp::lsp_types::{Documentation, MarkupContent, MarkupKind};

/// Pack configuration field.
pub(crate) fn config_data(field: &ConfigField) -> Value {
    json!({"k": "cfg", "pack": field.pack_ref, "id": field.id})
}

/// Session / advanced game / local option setting.
pub(crate) fn setting_data(setting: &SettingRecord) -> Value {
    json!({"k": "setting", "provider": setting.provider.as_str(), "id": setting.id})
}

/// Property of the patch target (`path:`), with its reload classification.
pub(crate) fn path_property_data(class: &str, name: &str, apply_to_spawned: bool) -> Value {
    json!({"k": "path", "class": class, "name": name, "apply": apply_to_spawned})
}

/// Field of a struct or property of an object reference below a `path:`.
pub(crate) fn nested_property_data(owner: &str, is_struct: bool, name: &str) -> Value {
    json!({"k": "nested", "owner": owner, "struct": is_struct, "name": name})
}

/// Property read of a template member (`{item:Class:Property}`).
pub(crate) fn member_property_data(class: &str, name: &str) -> Value {
    json!({"k": "member", "class": class, "name": name})
}

/// Function call of a template member.
pub(crate) fn function_data(class: &str, name: &str, kind: TargetKind) -> Value {
    json!({"k": "fn", "class": class, "name": name, "object": kind == TargetKind::Object})
}

/// The markdown card an item's `data` stands for (`None` when the subject is gone).
pub(crate) fn card(
    data: &Value,
    reflection: &ReflectionIndex,
    workspace: &Workspace,
) -> Option<String> {
    let text = |key: &str| data.get(key).and_then(Value::as_str);
    match text("k")? {
        "cfg" => {
            let field = workspace.config.field(text("pack")?, text("id")?)?;
            Some(config_card(field, workspace.root()))
        }
        "setting" => {
            let setting = reflection
                .settings()?
                .find(text("provider")?, text("id")?)?;
            Some(setting_card(setting))
        }
        "path" => {
            let (class, name) = (text("class")?, text("name")?);
            let apply = data.get("apply").and_then(Value::as_bool).unwrap_or(false);
            let property = reflection.find_property(class, name)?;
            let (reload, reason) = reflection.property_reload_with(class, name, apply);
            Some(property_card(
                &property,
                Some((reload, reason.as_str())),
                Some(class),
            ))
        }
        "nested" => {
            let (owner, name) = (text("owner")?, text("name")?);
            let property = if data.get("struct").and_then(Value::as_bool) == Some(true) {
                reflection.find_struct_field(owner, name)?
            } else {
                reflection.find_property(owner, name)?
            };
            Some(property_card(&property, None, None))
        }
        "member" => {
            let (class, name) = (text("class")?, text("name")?);
            let property = reflection.find_property(class, name)?;
            let (reload, reason) = reflection.property_reload(class, name);
            Some(property_card(
                &property,
                Some((reload, reason.as_str())),
                None,
            ))
        }
        "fn" => {
            let (class, name) = (text("class")?, text("name")?);
            let kind = if data.get("object").and_then(Value::as_bool) == Some(true) {
                TargetKind::Object
            } else {
                TargetKind::Class
            };
            let function = reflection.find_function(class, name)?;
            let callable = reflection.callable(&function, kind, Some(class));
            let signature = reflection.signature(&function, callable.implicit());
            Some(function_card(&function, &signature, &callable))
        }
        _ => None,
    }
}

/// `text` as protocol documentation: markdown when the client renders it.
pub(crate) fn documentation(text: String, caps: ClientCaps) -> Documentation {
    if caps.completion_markdown {
        Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: text,
        })
    } else {
        Documentation::String(plain_text(&text))
    }
}

/// Render the Markdown subset emitted by our cards as text. Code contents stay literal;
/// formatting delimiters disappear and links retain both their labels and destinations.
pub(crate) fn plain_text(markdown: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;
    static INLINE: OnceLock<Regex> = OnceLock::new();
    let inline = INLINE.get_or_init(|| {
        Regex::new(r"`([^`]+)`|\*\*([^*]+)\*\*|\*([^*]+)\*|\[([^\]]+)\]\(([^)]+)\)")
            .expect("card inline markup regex")
    });
    let mut fenced = false;
    let mut lines = Vec::new();
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            lines.push(line.to_owned());
            continue;
        }
        let line = line.strip_prefix("> ").unwrap_or(line);
        let line = line
            .strip_prefix("### ")
            .or_else(|| line.strip_prefix("## "))
            .or_else(|| line.strip_prefix("# "))
            .unwrap_or(line);
        lines.push(
            inline
                .replace_all(line, |caps: &regex::Captures<'_>| {
                    if let (Some(label), Some(url)) = (caps.get(4), caps.get(5)) {
                        format!("{} ({})", plain_text(label.as_str()), url.as_str())
                    } else if let Some(code) = caps.get(1) {
                        code.as_str().to_owned()
                    } else {
                        plain_text(
                            caps.get(2)
                                .or_else(|| caps.get(3))
                                .expect("emphasis content")
                                .as_str(),
                        )
                    }
                })
                .into_owned(),
        );
    }
    lines.join("\n")
}

pub(crate) fn markup(text: String, markdown: bool) -> MarkupContent {
    MarkupContent {
        kind: if markdown {
            MarkupKind::Markdown
        } else {
            MarkupKind::PlainText
        },
        value: if markdown { text } else { plain_text(&text) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_cards_keep_code_examples_and_link_destinations() {
        let card = "**mDisplay_Name**\n\nType: `TArray<Item_Class>`\n\n*Example*\n\n```yaml\nvalue:\n  ItemClass: \"/Game/Desc_Iron.Desc_Iron_C\"\n  Formula: \"2 * 3\"\n```\n\n[Documentation](https://example.com/guide)";
        assert_eq!(plain_text(card), "mDisplay_Name\n\nType: TArray<Item_Class>\n\nExample\n\nvalue:\n  ItemClass: \"/Game/Desc_Iron.Desc_Iron_C\"\n  Formula: \"2 * 3\"\n\nDocumentation (https://example.com/guide)");
        assert_eq!(plain_text("**Sets `value`.**"), "Sets value.");
    }
}
