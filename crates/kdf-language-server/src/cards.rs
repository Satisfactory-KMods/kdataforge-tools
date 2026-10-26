//! Markdown cards shared by completion documentation and hover.

use crate::semantic::reload_tag;
use kdf_validation::config::{Apply, ConfigField, FieldKind, NumberDisplay};
use kdf_validation::{
    display_type, Callable, FunctionInfo, Implicit, PropertyInfo, Reload, SettingRecord,
};
use serde_json::Value;
use std::path::Path;

fn json_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn number_text(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Card of a pack configuration field.
pub(crate) fn config_card(field: &ConfigField, root: Option<&Path>) -> String {
    let mut out = String::new();
    out.push_str(&format!("**{}** (`{}`)\n\n", field.name, field.id));
    let mut kind = format!("{} · {}", field.kind.as_str(), field.value_type.as_str());
    if field.multiple {
        kind.push_str(" · list");
    }
    let apply = match field.apply {
        Apply::Hot => "hot (applies without a restart)",
        Apply::Cold => "cold (read at load time, needs a reload or restart)",
    };
    out.push_str(&format!("`{kind}` · {apply}\n\n"));
    if !field.description.is_empty() {
        out.push_str(&field.description);
        out.push_str("\n\n");
    }
    if field.tooltip_title.is_some() || field.tooltip_text.is_some() {
        let title = field.tooltip_title.as_deref().unwrap_or("");
        let text = field.tooltip_text.as_deref().unwrap_or("");
        out.push_str(&format!(
            "> {}{}{}\n\n",
            title,
            if title.is_empty() || text.is_empty() {
                ""
            } else {
                ": "
            },
            text
        ));
    }
    let mut lines = Vec::new();
    lines.push(format!("Default: `{}`", json_text(&field.default)));
    match (field.min, field.max) {
        (Some(min), Some(max)) => {
            let mut range = format!("Range: `{}` to `{}`", number_text(min), number_text(max));
            if let Some(step) = field.step {
                range.push_str(&format!(", step `{}`", number_text(step)));
            }
            if let Some(decimals) = field.decimals {
                range.push_str(&format!(", {decimals} decimals"));
            }
            if field.kind == FieldKind::Number {
                range.push_str(match field.display {
                    NumberDisplay::Slider => ", slider",
                    NumberDisplay::Input => ", input",
                });
            }
            lines.push(range);
        }
        (Some(min), None) => lines.push(format!("Minimum: `{}`", number_text(min))),
        (None, Some(max)) => lines.push(format!("Maximum: `{}`", number_text(max))),
        (None, None) => {}
    }
    if !field.options.is_empty() {
        let options = field
            .options
            .iter()
            .map(|option| format!("`{}` ({})", json_text(&option.value), option.label))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("Options: {options}"));
    }
    if let Some(length) = field.max_length {
        lines.push(format!("Maximum length: {length}"));
    }
    if let Some(root) = &field.tag_root {
        lines.push(format!("Tag root: `{root}`"));
    }
    if let (Some(min), Some(max)) = (field.min_count, field.max_count) {
        lines.push(format!("Elements: {min} to {max}"));
    } else if let Some(min) = field.min_count {
        lines.push(format!("Minimum elements: {min}"));
    } else if let Some(max) = field.max_count {
        lines.push(format!("Maximum elements: {max}"));
    }
    if let (Some(min), Some(max)) = (field.min_selected, field.max_selected) {
        lines.push(format!("Selected options: {min} to {max}"));
    } else if let Some(min) = field.min_selected {
        lines.push(format!("Minimum selected options: {min}"));
    } else if let Some(max) = field.max_selected {
        lines.push(format!("Maximum selected options: {max}"));
    }
    if let Some(category) = &field.category {
        lines.push(format!("Category: `{category}`"));
    }
    lines.push(format!("Pack: `{}`", field.pack_ref));
    let file = root
        .and_then(|root| field.file.strip_prefix(root).ok())
        .unwrap_or(&field.file);
    lines.push(format!(
        "File: `{}`",
        file.to_string_lossy().replace('\\', "/")
    ));
    for line in lines {
        out.push_str(&format!("- {line}\n"));
    }
    out
}

/// Card of a session / advanced game / local option setting.
pub(crate) fn setting_card(setting: &SettingRecord) -> String {
    let mut out = format!(
        "**{}** (`{}:{}`)\n\n",
        if setting.display_name.is_empty() {
            &setting.id
        } else {
            &setting.display_name
        },
        setting.provider.as_str(),
        setting.id
    );
    out.push_str(&format!("`{}` setting\n\n", setting.value_type.as_str()));
    if !setting.tooltip.is_empty() {
        out.push_str(&setting.tooltip);
        out.push_str("\n\n");
    }
    if let Some(default) = &setting.default {
        out.push_str(&format!("- Default: `{}`\n", json_text(default)));
    }
    if let Some((min, max)) = setting.range {
        out.push_str(&format!(
            "- Range: `{}` to `{}`\n",
            number_text(min),
            number_text(max)
        ));
    }
    if !setting.options.is_empty() {
        let options = setting
            .options
            .iter()
            .map(|(value, label)| format!("`{}` ({label})", json_text(value)))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("- Options: {options}\n"));
    }
    for placement in &setting.placements {
        out.push_str(&format!("- Options menu: {}\n", placement.display_path()));
    }
    out.push_str(&format!(
        "- Session wide: {} · dedicated server relevant: {}\n",
        yes_no(setting.session_wide),
        yes_no(setting.dedicated_server_relevant)
    ));
    if let Some(source) = &setting.source_ref {
        out.push_str(&format!("- Source: `{source}`\n"));
    }
    out
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

/// Card of a property (`path:` value or a template member).
pub(crate) fn property_card(
    property: &PropertyInfo,
    reload: Option<(Reload, &str)>,
    class: Option<&str>,
) -> String {
    let mut out = format!(
        "**{}**\n\n`{}` · {}\n\n",
        property.display_name(),
        display_type(property),
        property.kind_name
    );
    if let Some(class) = class {
        out.push_str(&format!("Class `{class}`\n\n"));
    }
    if !property.owner_path.is_empty() {
        out.push_str(&format!("Declared in `{}`\n\n", property.owner_path));
    }
    if let Some((reload, reason)) = reload {
        match reload_tag(reload) {
            Some(tag) => {
                out.push_str(&format!("Reload: **{tag}**"));
                if !reason.is_empty() {
                    out.push_str(&format!(" - {reason}"));
                }
                out.push_str("\n\n");
            }
            None => out.push_str("Reload: hot (changes apply live)\n\n"),
        }
    }
    if !property.enum_options.is_empty() {
        let options = property
            .enum_options
            .iter()
            .filter(|name| !name.ends_with("_MAX"))
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("Values: {options}\n\n"));
    }
    out
}

/// Card of a callable function.
pub(crate) fn function_card(
    function: &FunctionInfo,
    signature: &str,
    callable: &Callable,
) -> String {
    let mut out = format!("```\n{signature}\n```\n\n");
    if !function.owner_path.is_empty() {
        out.push_str(&format!("Declared in `{}`\n\n", function.owner_path));
    }
    let flags = ["Static", "BlueprintPure", "Const", "Native"]
        .iter()
        .filter(|flag| function.has_flag(flag))
        .map(|flag| format!("`{flag}`"))
        .collect::<Vec<_>>();
    if !flags.is_empty() {
        out.push_str(&format!("Flags: {}\n\n", flags.join(" ")));
    }
    match callable {
        Callable::Yes {
            implicit,
            explicit_args,
            unverified,
        } => {
            out.push_str("Callable in templates");
            match implicit {
                Implicit::Class => out.push_str("; the target class is passed implicitly"),
                Implicit::Object => out.push_str("; the target object is passed implicitly"),
                Implicit::None => {}
            }
            out.push_str(&format!(" ({explicit_args} argument(s) to write)."));
            if let Some(note) = unverified {
                out.push_str(&format!(
                    "\n\nNot fully verified: {note} (type data missing in the datapack)."
                ));
            }
        }
        Callable::No(reason) => out.push_str(&format!("**Not callable in templates:** {reason}")),
    }
    out.push('\n');
    out
}
