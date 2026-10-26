//! Pure, read-only inspection of ficsit-cli/SMM profile JSON.

use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const MAX_PROFILE_JSON_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmmProfiles {
    pub version: Option<i64>,
    pub selected_profile: Option<String>,
    /// `None` means the profile map is missing or malformed; an empty map is known empty.
    pub profiles: Option<BTreeMap<String, SmmProfile>>,
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmmProfile {
    pub name: Option<String>,
    pub required_targets: Option<Vec<String>>,
    /// `None` means mod membership is unknown, not that the profile has no mods.
    pub mods: Option<BTreeMap<String, SmmProfileMod>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmmProfileMod {
    pub version: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SmmModState {
    Enabled,
    Disabled,
    Missing,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmmModMatch {
    pub reference: String,
    pub state: SmmModState,
    pub version: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmmProfileComparison {
    pub selected_profile: Option<String>,
    pub mods: Vec<SmmModMatch>,
    pub issues: Vec<String>,
}

/// Parse caller-supplied `profiles.json` text without reading a private profile file.
///
/// ficsit-cli stores `profiles`, `selected_profile`, and `version` at the root;
/// each profile has `mods`, `name`, and `required_targets`. Missing or malformed
/// optional fields remain `None`, so callers do not mistake incomplete data for
/// an empty mod list or a disabled mod.
pub fn parse_profiles_json(text: &str) -> Result<SmmProfiles, String> {
    if text.len() > MAX_PROFILE_JSON_BYTES {
        return Err("SMM profiles JSON exceeds 4 MiB limit".into());
    }
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("invalid SMM profiles JSON: {error}"))?;
    let root = value
        .as_object()
        .ok_or("SMM profiles JSON root must be an object")?;
    let mut result = SmmProfiles {
        version: root.get("version").and_then(Value::as_i64),
        ..SmmProfiles::default()
    };
    if root.contains_key("version") && result.version.is_none() {
        result.issues.push("version is not an integer".into());
    }
    result.selected_profile = root
        .get("selected_profile")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if result.selected_profile.is_none() {
        result
            .issues
            .push("selected_profile is missing or invalid".into());
    }

    if let Some(profiles) = root.get("profiles").and_then(Value::as_object) {
        let mut parsed = BTreeMap::new();
        for (key, value) in profiles {
            parsed.insert(key.clone(), parse_profile(key, value, &mut result.issues));
        }
        result.profiles = Some(parsed);
    } else {
        result
            .issues
            .push("profiles map is missing or invalid".into());
    }
    Ok(result)
}

fn parse_profile(key: &str, value: &Value, issues: &mut Vec<String>) -> SmmProfile {
    let Some(profile) = value.as_object() else {
        issues.push(format!("profile {key:?} is not an object"));
        return SmmProfile::default();
    };
    let name = profile
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if profile.contains_key("name") && name.is_none() {
        issues.push(format!("profile {key:?} has invalid name"));
    }
    let required_targets = profile
        .get("required_targets")
        .and_then(Value::as_array)
        .filter(|targets| targets.iter().all(Value::is_string))
        .map(|targets| {
            targets
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        });
    if profile.contains_key("required_targets") && required_targets.is_none() {
        issues.push(format!("profile {key:?} has invalid required_targets"));
    }
    let mods = if let Some(mods) = profile.get("mods").and_then(Value::as_object) {
        let mut parsed = BTreeMap::new();
        for (reference, value) in mods {
            parsed.insert(
                reference.clone(),
                parse_profile_mod(key, reference, value, issues),
            );
        }
        Some(parsed)
    } else {
        issues.push(format!("profile {key:?} has no valid mods map"));
        None
    };
    SmmProfile {
        name,
        required_targets,
        mods,
    }
}

fn parse_profile_mod(
    profile: &str,
    reference: &str,
    value: &Value,
    issues: &mut Vec<String>,
) -> SmmProfileMod {
    let Some(record) = value.as_object() else {
        issues.push(format!(
            "mod {reference:?} in profile {profile:?} is not an object"
        ));
        return SmmProfileMod::default();
    };
    let version = record
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if record.contains_key("version") && version.is_none() {
        issues.push(format!(
            "mod {reference:?} in profile {profile:?} has invalid version"
        ));
    }
    let enabled = record.get("enabled").and_then(Value::as_bool);
    if enabled.is_none() {
        issues.push(format!(
            "mod {reference:?} in profile {profile:?} has no valid enabled flag"
        ));
    }
    SmmProfileMod { version, enabled }
}

/// Compare pack `conditions.hasMod` references with the selected SMM profile.
///
/// States describe profile intent, not the currently installed game. References
/// use exact matching, as in ficsit-cli's `mods` map. No profile is selected
/// implicitly and no profile or pack data is modified.
pub fn compare_required_mods(
    profiles: &SmmProfiles,
    required_mods: &[String],
) -> SmmProfileComparison {
    let mut issues = profiles.issues.clone();
    let selected = profiles.selected_profile.as_deref();
    let profile =
        selected.and_then(|name| profiles.profiles.as_ref().and_then(|all| all.get(name)));
    if let Some(name) = selected {
        if profile.is_none() && profiles.profiles.is_some() {
            issues.push(format!("selected profile {name:?} is absent"));
        }
    }
    let mut seen = BTreeSet::new();
    let mut mods = Vec::new();
    for reference in required_mods {
        if !seen.insert(reference.as_str()) {
            continue;
        }
        let (state, version, reason) = match profile.and_then(|profile| profile.mods.as_ref()) {
            None => (
                SmmModState::Unknown,
                None,
                Some("selected profile or its mods map is unavailable".into()),
            ),
            Some(_) if reference.is_empty() => (
                SmmModState::Unknown,
                None,
                Some("required mod reference is empty".into()),
            ),
            Some(available) => match available.get(reference) {
                None => (SmmModState::Missing, None, None),
                Some(entry) => match entry.enabled {
                    Some(true) => (SmmModState::Enabled, entry.version.clone(), None),
                    Some(false) => (SmmModState::Disabled, entry.version.clone(), None),
                    None => (
                        SmmModState::Unknown,
                        entry.version.clone(),
                        Some("mod enabled flag is unavailable".into()),
                    ),
                },
            },
        };
        mods.push(SmmModMatch {
            reference: reference.clone(),
            state,
            version,
            reason,
        });
    }
    SmmProfileComparison {
        selected_profile: profiles.selected_profile.clone(),
        mods,
        issues,
    }
}
