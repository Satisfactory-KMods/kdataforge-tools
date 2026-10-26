//! Static selector references from incomplete YAML. No catalog scan or disk read.
use serde_json::Value;
use std::collections::HashMap;

// KDFCdoHandler.cpp:204–225: recipe shortcuts imply this scope, not their
// referenced machine/item class. Reflected fields still require indexed metadata.
pub(crate) const RECIPE_SCOPE: &str = "/Script/FactoryGame.FGRecipe";

#[derive(Clone, Copy)]
pub(crate) enum SelectorKind {
    Target,
    RecipeFilter(&'static str),
}

pub(crate) fn references(value: &str, anchors: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut value = value.trim();
    if let Some(anchor) = value.strip_prefix('&') {
        let Some((_, rest)) = anchor.split_once(char::is_whitespace) else {
            return Vec::new();
        };
        value = rest.trim();
    }
    if let Some(alias) = value.strip_prefix('*') {
        return anchors.get(alias).cloned().unwrap_or_default();
    }
    let parsed = serde_yaml::from_str::<Value>(value);
    let values = match parsed {
        Ok(Value::Array(values)) => values,
        Ok(value) => vec![value],
        Err(_) => return Vec::new(),
    };
    values
        .into_iter()
        .filter_map(|value| {
            value
                .as_str()
                .filter(|path| path.starts_with('/'))
                .map(str::to_owned)
        })
        .take(65)
        .collect()
}

pub(crate) fn remember_anchor(value: &str, anchors: &mut HashMap<String, Vec<String>>) {
    let Some(anchor) = value.trim().strip_prefix('&') else {
        return;
    };
    let Some((name, _)) = anchor.split_once(char::is_whitespace) else {
        return;
    };
    if !name.is_empty() && anchors.len() < 256 {
        anchors.insert(name.to_owned(), references(value, anchors));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_scalar_list_and_alias_references_are_bounded_and_unknowns_stay_unknown() {
        let mut anchors = HashMap::new();
        remember_anchor("&scope /Script/Test.Owner", &mut anchors);
        assert_eq!(references("*scope", &anchors), ["/Script/Test.Owner"]);
        assert_eq!(
            references("['/Script/Test.One', /Script/Test.Two]", &anchors),
            ["/Script/Test.One", "/Script/Test.Two"]
        );
        assert!(references("*unknown", &anchors).is_empty());
        assert!(references("${dynamic}", &anchors).is_empty());
        assert!(references("[unterminated", &anchors).is_empty());
    }
}
