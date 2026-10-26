//! Datapack snapshot selection, shared by all hosts. Runtime authority:
//! KDFCdoHandler::GatherTargets and KDFTargetFilter::Matches in the mod sources.
//! This predicts selection against exported defaults, not live game/pack execution.
use crate::{AssetEntry, ReflectionIndex, ReflectionStatus};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffectedElement {
    pub path: String,
    pub name: String,
    pub display_name: Option<String>,
    pub class_path: Option<String>,
    pub source: Option<String>,
    pub kind: String,
    pub certainty: String,
    pub reason: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffectedElements {
    pub status: String,
    pub reasons: Vec<String>,
    pub matches: Vec<AffectedElement>,
    pub matched_count: usize,
    pub potential_count: usize,
    pub truncated: bool,
}

fn strings(value: Option<&Value>) -> Option<Vec<&str>> {
    match value? {
        Value::String(s) if !s.trim().is_empty() => Some(vec![s]),
        Value::Array(values) if !values.is_empty() && values.len() <= 256 => values
            .iter()
            .map(|v| v.as_str().filter(|s| !s.trim().is_empty()))
            .collect(),
        _ => None,
    }
}
fn text(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.get("sourceString").and_then(Value::as_str))
        .or_else(|| value.get("source").and_then(Value::as_str))
}
fn wildcard(pattern: &str, value: &str) -> bool {
    // Linear glob algorithm: UE name filters use * and ?, case-insensitive.
    let pattern = pattern.to_lowercase().chars().collect::<Vec<_>>();
    let value = value.to_lowercase().chars().collect::<Vec<_>>();
    let (mut p, mut v, mut star, mut retry) = (0, 0, None, 0);
    while v < value.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == value[v]) {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            p += 1;
            retry = v;
        } else if let Some(s) = star {
            retry += 1;
            v = retry;
            p = s + 1;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}
fn reference_match(index: &ReflectionIndex, actual: &str, expected: &str) -> Option<bool> {
    if let Some(base) = index.class_path(expected) {
        return index
            .class_path(actual)
            .map(|path| index.is_subclass_of(path, base));
    }
    let normalize = |s: &str| s.trim().trim_end_matches("_C").to_lowercase();
    Some(
        normalize(actual) == normalize(expected)
            || (!expected.contains('/')
                && normalize(actual.rsplit(['.', '/']).next().unwrap_or(actual))
                    == normalize(expected)),
    )
}
/// A cleared (`None`) reference names no class, so it matches no expected reference.
fn reference_leaf_match(index: &ReflectionIndex, leaf: &Value, expected: &[&str]) -> Option<bool> {
    match leaf {
        Value::Null => Some(false),
        Value::String(actual) => any(expected
            .iter()
            .map(|expected| reference_match(index, actual, expected))),
        _ => None,
    }
}
fn and(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    if a == Some(false) || b == Some(false) {
        Some(false)
    } else if a == Some(true) && b == Some(true) {
        Some(true)
    } else {
        None
    }
}
fn any(values: impl Iterator<Item = Option<bool>>) -> Option<bool> {
    let mut unknown = false;
    for value in values {
        if value == Some(true) {
            return Some(true);
        }
        unknown |= value.is_none();
    }
    if unknown {
        None
    } else {
        Some(false)
    }
}

/// Resolve JSON defaults without treating missing values as empty containers.
/// Wildcards use existential semantics, including negative operators, as the runtime does.
fn leaves<'a>(
    index: &'a ReflectionIndex,
    value: &'a Value,
    path: &str,
    depth: usize,
) -> Option<Vec<&'a Value>> {
    if depth > 32 {
        return None;
    }
    if path.is_empty() {
        return Some(vec![value]);
    }
    if let Some(rest) = path.strip_prefix("[*]") {
        let values = value.as_array()?;
        let mut result = Vec::new();
        for value in values {
            result.extend(leaves(
                index,
                value,
                rest.trim_start_matches('.'),
                depth + 1,
            )?);
        }
        return Some(result);
    }
    if let Some(rest) = path.strip_prefix('[') {
        let (number, rest) = rest.split_once(']')?;
        return leaves(
            index,
            value.as_array()?.get(number.parse::<usize>().ok()?)?,
            rest.trim_start_matches('.'),
            depth + 1,
        );
    }
    let end = path.find(['.', '[']).unwrap_or(path.len());
    let name = &path[..end];
    let next = value
        .as_object()
        .and_then(|map| {
            map.iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value)
        })
        .or_else(|| {
            value
                .as_str()
                .and_then(|class| index.snapshot_default(class, name))
        })?;
    leaves(index, next, path[end..].trim_start_matches('.'), depth + 1)
}
fn property_leaves<'a>(
    index: &'a ReflectionIndex,
    class: &str,
    path: &str,
) -> Option<Vec<&'a Value>> {
    let end = path.find(['.', '[']).unwrap_or(path.len());
    let value = index.snapshot_default(class, &path[..end])?;
    leaves(index, value, path[end..].trim_start_matches('.'), 0)
}
/// Recipes without a display override take their name from the first product, even when their
/// own text is nonempty. Missing defaults and unresolved string tables leave the name unknown.
fn effective_display_name<'a>(index: &'a ReflectionIndex, class: &str) -> Option<&'a str> {
    let source = if index.is_subclass_of(class, "/Script/FactoryGame.FGRecipe") {
        match index.snapshot_bool_default(class, "mDisplayNameOverride")? {
            true => class,
            false => {
                let products = property_leaves(index, class, "mProduct[0].ItemClass")?;
                index.class_path(products.first()?.as_str()?)?
            }
        }
    } else {
        class
    };
    index
        .snapshot_default(source, "mDisplayName")
        .and_then(text)
        .filter(|name| !name.trim().is_empty())
}
fn compare(index: &ReflectionIndex, actual: &Value, op: &str, expected: &Value) -> Option<bool> {
    let equal = |a: &Value, b: &Value| -> Option<bool> {
        if let (Some(a), Some(b)) = (a.as_str(), b.as_str()) {
            // Without a typed leaf, a string path could be a class reference or an FString.
            // Their equality rules differ; do not infer ancestry from spelling alone.
            if a.starts_with('/')
                && (index.class_path(a).is_some() || index.class_path(b).is_some())
            {
                return None;
            }
            return Some(a.eq_ignore_ascii_case(b));
        }
        if let (Some(a), Some(b)) = (a.as_f64(), b.as_f64()) {
            return Some((a - b).abs() <= 0.000001);
        }
        if a.is_boolean() && b.is_boolean() {
            return Some(a == b);
        }
        None
    };
    let contains = |expected: &Value| {
        if let Some(values) = actual.as_array() {
            any(values.iter().map(|v| equal(v, expected)))
        } else if let (Some(a), Some(b)) = (actual.as_str(), expected.as_str()) {
            Some(a.to_lowercase().contains(&b.to_lowercase()))
        } else {
            None
        }
    };
    match op {
        "equals" => equal(actual, expected),
        "notEquals" => equal(actual, expected).map(|v| !v),
        "contains" => contains(expected),
        "notContains" => contains(expected).map(|v| !v),
        "in" | "notIn" => {
            let alternatives = expected
                .as_array()
                .map(|v| v.as_slice())
                .unwrap_or(std::slice::from_ref(expected));
            let found = any(alternatives.iter().map(|v| {
                if actual.is_array() {
                    contains(v)
                } else {
                    equal(actual, v)
                }
            }));
            found.map(|v| if op == "notIn" { !v } else { v })
        }
        "greaterThan" | "greaterOrEqual" | "lessThan" | "lessOrEqual" => {
            let a = actual.as_f64()?;
            let b = expected.as_f64()?;
            Some(match op {
                "greaterThan" => a > b,
                "greaterOrEqual" => a >= b,
                "lessThan" => a < b,
                _ => a <= b,
            })
        }
        "matches" => Some(wildcard(expected.as_str()?, text(actual)?)),
        "isEmpty" => {
            let empty = match actual {
                Value::Array(values) => values.is_empty(),
                Value::Object(members) => members.is_empty(),
                Value::String(string) => string.is_empty(),
                _ => return None,
            };
            Some(empty == expected.as_bool()?)
        }
        _ => None,
    }
}
fn name_filter(path: &str, name: &str, entry: &Value) -> Option<bool> {
    let Some(value) = entry.get("matchName") else {
        return Some(true);
    };
    let patterns = strings(Some(value))?;
    if path.is_empty() || name.is_empty() {
        return None;
    }
    Some(patterns.iter().any(|pattern| {
        if pattern.contains('/') {
            wildcard(pattern, path)
        } else {
            wildcard(pattern, name) || wildcard(pattern, name.trim_end_matches("_C"))
        }
    }))
}
fn asset_filters(index: &ReflectionIndex, asset: &AssetEntry, entry: &Value) -> Option<bool> {
    if !index.asset_identity_verified(asset) {
        return None;
    }
    let named = and(
        name_filter(&asset.path, &asset.name, entry),
        class_identity_filter(index, &asset.asset_class, entry),
    );
    // KDFTargetFilter::Matches names assets by object identity. Instance property defaults and
    // owned interface tags are not exported; a negative name match still proves exclusion.
    if ["matchTag", "where", "producedIn", "ingredient", "product"]
        .iter()
        .any(|key| entry.get(key).is_some())
    {
        and(named, None)
    } else {
        named
    }
}
fn class_identity_filter(index: &ReflectionIndex, class: &str, entry: &Value) -> Option<bool> {
    if let Some(excluded) = entry.get("excludeClasses") {
        if strings(Some(excluded))?.iter().any(|pattern| {
            let pattern = pattern.trim();
            if pattern.contains(['*', '?']) {
                pattern.starts_with('/') && wildcard(pattern, class)
            } else {
                pattern == class
            }
        }) {
            return Some(false);
        }
    }
    match entry
        .get("classType")
        .and_then(Value::as_str)
        .unwrap_or("any")
    {
        "any" => Some(true),
        expected @ ("blueprint" | "native") => {
            let info = index.class_info(class)?;
            match info.get("kind").and_then(Value::as_str) {
                Some(actual @ ("blueprint" | "native")) => Some(actual == expected),
                _ => None,
            }
        }
        _ => None,
    }
}
fn filters(index: &ReflectionIndex, class: &str, entry: &Value) -> Option<bool> {
    let mut matched = and(
        name_filter(class, class.rsplit('.').next().unwrap_or(class), entry),
        class_identity_filter(index, class, entry),
    );
    // Filters are ANDed: a proven name exclusion needs no defaults, which are decoded per class.
    if matched == Some(false) {
        return matched;
    }
    for (key, path) in [
        ("producedIn", "mProducedIn[*]"),
        ("ingredient", "mIngredients[*].ItemClass"),
        ("product", "mProduct[*].ItemClass"),
    ] {
        if let Some(value) = entry.get(key) {
            let expected = strings(Some(value))?;
            let test = property_leaves(index, class, path).and_then(|values| {
                any(values
                    .into_iter()
                    .map(|leaf| reference_leaf_match(index, leaf, &expected)))
            });
            matched = and(matched, test);
        }
    }
    if entry.get("matchTag").is_some() {
        // Runtime asks IGameplayTagAssetInterface first. Exported containers do not prove
        // the interface's owned tags; substituting mGameplayTags would exclude valid targets.
        matched = and(matched, None);
    }
    if let Some(where_value) = entry.get("where") {
        let clauses = where_value
            .as_array()
            .map(|v| v.as_slice())
            .unwrap_or(std::slice::from_ref(where_value));
        if clauses.len() > 64 {
            return None;
        }
        for clause in clauses {
            let test = (|| {
                let path = clause.get("path")?.as_str().filter(|s| s.len() <= 512)?;
                let operations = clause
                    .as_object()?
                    .iter()
                    .filter(|(key, _)| key.as_str() != "path")
                    .collect::<Vec<_>>();
                if operations.len() != 1 {
                    return None;
                }
                let (op, expected) = operations[0];
                let values = property_leaves(index, class, path)?;
                any(values
                    .into_iter()
                    .map(|value| compare(index, value, op, expected)))
            })();
            matched = and(matched, test);
        }
    }
    matched
}
/// Why the asset index cannot prove the asset list of a class scope complete, if it cannot.
/// An indexed class with zero assets proves that no asset exists; a capped listing does not.
fn asset_coverage_gap(index: &ReflectionIndex, classes: &[&str]) -> Option<&'static str> {
    // KDFReflectionExporter::IndexAssetsForClass queries GetAssetsByClass(..., true), as do
    // KDFCdoHandler::GatherTargets and KDFTargetFilter::ScanScope. The first class is the scope base;
    // its complete recursive listing covers descendants without separate child buckets.
    if classes
        .first()
        .is_some_and(|base| index.asset_listing_complete(base) == Some(true))
    {
        return None;
    }
    let listings = classes
        .iter()
        .map(|class| index.asset_listing_complete(class))
        .collect::<Vec<_>>();
    if listings.iter().all(Option::is_none) {
        Some("Asset instance index is unavailable for this scope; asset coverage cannot be proved.")
    } else if listings.iter().any(|listing| *listing != Some(true)) {
        Some("Asset index lists only part of this scope; more assets may be selected.")
    } else {
        None
    }
}

pub fn affected_elements(
    index: &ReflectionIndex,
    root_type: &str,
    entry: &Value,
    limit: usize,
) -> AffectedElements {
    let mut result = AffectedElements {
        status: "ready".into(),
        reasons: Vec::new(),
        matches: Vec::new(),
        matched_count: 0,
        potential_count: 0,
        truncated: false,
    };
    if root_type == "resourcenode"
        && entry.is_object()
        && (entry.get("visuals").is_some() || entry.get("hideMeshes").is_some())
    {
        result.status = "limited".into();
        result.reasons.push("Resource mesh visuals require live node/mesh pairing and streamed actor state; exported descriptors cannot prove instance matches.".into());
        return result;
    }
    if !matches!(
        root_type,
        "cdo"
            | "class"
            | "item"
            | "resource"
            | "building"
            | "recipe"
            | "schematic"
            | "research"
            | "unlock"
    ) || !entry.is_object()
    {
        result.status = "unsupported".into();
        result.reasons.push("This entry has no supported class/asset selector. Select a CDO patch or content class entry.".into());
        return result;
    }
    if root_type == "cdo"
        && (entry.get("actorSelector").is_some()
            || entry.get("actorAction").is_some()
            || entry.get("inventoryRemap").is_some())
    {
        result.status = "limited".into();
        result.reasons.push("Actor instance selection requires live world state; exported class defaults cannot prove instance matches.".into());
        return result;
    }
    if index.status != ReflectionStatus::Ready {
        result.reasons.push(
            index
                .reason
                .clone()
                .unwrap_or_else(|| "Reflection export is incomplete.".into()),
        );
    }
    let mut selected: BTreeMap<String, (bool, String, Option<String>)> = BTreeMap::new();
    let mut add = |path: &str, proven: bool, reason: &str, class: Option<&str>| {
        let value = (proven, reason.into(), class.map(str::to_owned));
        selected
            .entry(path.into())
            .and_modify(|previous| {
                if proven {
                    *previous = value.clone();
                }
            })
            .or_insert(value);
    };
    let target = if root_type == "cdo" {
        entry.get("target")
    } else {
        entry.get("class")
    };
    if let Some(value) = target {
        if let Some(paths) = strings(Some(value)) {
            for path in paths {
                if let Some(class) = index.class_path(path) {
                    add(class, true, "Direct target", Some(class));
                    if root_type == "cdo"
                        && entry.get("applyToSubclasses").and_then(Value::as_bool) == Some(true)
                    {
                        for child in index.descendants(class, false) {
                            add(child, true, "Subclass of target", Some(child));
                        }
                    }
                } else if let Some(asset) = index.find_asset(path) {
                    let verified = index.asset_identity_verified(asset);
                    add(
                        &asset.path,
                        verified,
                        if verified {
                            "Direct asset target"
                        } else {
                            "Asset identity is missing, malformed or inconsistent"
                        },
                        index.class_path(&asset.asset_class),
                    );
                } else {
                    add(
                        path,
                        false,
                        "Target was not exported; it may load or be created at runtime",
                        None,
                    );
                }
            }
        } else {
            result
                .reasons
                .push("Target must contain non-empty class or asset references.".into());
        }
    }
    if let Some(selector) = entry.get("allAssetsOfClass") {
        if let Some(reference) = selector.as_str().filter(|s| !s.trim().is_empty()) {
            if let Some(base) = index.class_path(reference) {
                let mut classes = vec![base];
                classes.extend(index.descendants(base, false));
                if let Some(gap) = asset_coverage_gap(index, &classes) {
                    result.reasons.push(gap.into());
                }
                for class in classes {
                    for asset in index.assets_of_class(class) {
                        let verified = index.asset_identity_verified(asset);
                        add(
                            &asset.path,
                            verified,
                            if !verified {
                                "Asset identity is missing, malformed or inconsistent"
                            } else {
                                "Asset in selected class"
                            },
                            index.class_path(&asset.asset_class),
                        );
                    }
                }
            } else {
                result.reasons.push(
                    "allAssetsOfClass scope is not present in the bound export; asset selection cannot be proved.".into(),
                );
            }
        } else {
            result
                .reasons
                .push("allAssetsOfClass must contain a non-empty class reference.".into());
        }
    }
    let filter_keys = [
        "classType",
        "excludeClasses",
        "matchName",
        "matchTag",
        "where",
        "producedIn",
        "ingredient",
        "product",
    ];
    if root_type == "cdo" && filter_keys.iter().any(|key| entry.get(key).is_some()) {
        let implicit = match root_type {
            "recipe" => Some("/Script/FactoryGame.FGRecipe"),
            "item" => Some("/Script/FactoryGame.FGItemDescriptor"),
            "schematic" => Some("/Script/FactoryGame.FGSchematic"),
            "research" => Some("/Script/FactoryGame.FGResearchTree"),
            _ if ["producedIn", "ingredient", "product"]
                .iter()
                .any(|key| entry.get(key).is_some()) =>
            {
                Some("/Script/FactoryGame.FGRecipe")
            }
            _ => None,
        };
        if let Some(base) = entry
            .get("ofClass")
            .and_then(Value::as_str)
            .or(implicit)
            .and_then(|base| index.class_path(base))
        {
            let mut candidates = vec![base];
            candidates.extend(index.descendants(base, false));
            if entry.get("matchAssets").and_then(Value::as_bool) != Some(false) {
                if let Some(gap) = asset_coverage_gap(index, &candidates) {
                    result.reasons.push(gap.into());
                }
            }
            for class in candidates {
                match filters(index, class, entry) {
                    Some(true) => add(class, true, "Matches selector filters", Some(class)),
                    None => add(
                        class,
                        false,
                        "Filter needs defaults or reference metadata absent from export",
                        Some(class),
                    ),
                    _ => {}
                }
                // The asset index exports identity, not per-instance defaults. Never use its CDO
                // as a substitute for an asset's overridden values.
                if entry.get("matchAssets").and_then(Value::as_bool) != Some(false) {
                    for asset in index.assets_of_class(class) {
                        match asset_filters(index, asset, entry) {
                            Some(true) => add(
                                &asset.path,
                                true,
                                "Matches exported asset identity",
                                index.class_path(&asset.asset_class),
                            ),
                            None => add(
                                &asset.path,
                                false,
                                "Asset instance defaults or identity were not exported",
                                index.class_path(&asset.asset_class),
                            ),
                            Some(false) => {}
                        }
                    }
                }
            }
        } else {
            result
                .reasons
                .push("Filter scope is absent or not present in the bound export.".into());
        }
    } else if target.is_none() && entry.get("allAssetsOfClass").is_none() {
        result.status = "unsupported".into();
        result.reasons.push("Select an entry with target, class, allAssetsOfClass or scope filters. ofClass alone has no effect.".into());
    }
    let conditional = entry.get("conditions").is_some();
    if root_type == "cdo" && entry.get("matchTag").is_some() {
        result.reasons.push(
            "Owned gameplay tags may come from a runtime interface; tag matches remain possible."
                .into(),
        );
    }
    if conditional {
        result
            .reasons
            .push("Runtime conditions are not evaluated; these are possible targets.".into());
    }
    if entry.get("applyToSpawnedActors").and_then(Value::as_bool) == Some(true) {
        result
            .reasons
            .push("Live actor instances cannot be enumerated from an export.".into());
    }
    for (path, (proven, reason, class_path)) in selected {
        let proven = proven && !conditional;
        if proven {
            result.matched_count += 1;
        } else {
            result.potential_count += 1;
        }
        if result.matches.len() >= limit.clamp(1, 2000) {
            result.truncated = true;
            continue;
        }
        let summary = index
            .classes
            .binary_search_by(|c| c.path.cmp(&path))
            .ok()
            .map(|position| &index.classes[position]);
        let asset = summary.is_none().then(|| index.find_asset(&path)).flatten();
        let display_name = class_path
            .as_deref()
            .filter(|class| *class == path)
            .and_then(|class| effective_display_name(index, class))
            .map(str::to_owned);
        result.matches.push(AffectedElement {
            name: summary
                .map(|c| c.name.clone())
                .or_else(|| asset.map(|asset| asset.name.clone()))
                .unwrap_or_else(|| path.rsplit('.').next().unwrap_or(&path).into()),
            display_name,
            source: summary.and_then(|c| c.source.clone()),
            kind: if summary.is_some() {
                "class"
            } else if asset.is_some() {
                "asset"
            } else {
                "unknown"
            }
            .into(),
            path,
            class_path,
            certainty: if proven { "matched" } else { "potential" }.into(),
            reason: if conditional {
                "Runtime condition not evaluated".into()
            } else {
                reason
            },
        });
    }
    if result.status == "ready" && (!result.reasons.is_empty() || result.potential_count > 0) {
        result.status = "limited".into();
    }
    result
}
