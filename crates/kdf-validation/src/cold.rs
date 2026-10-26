//! Reload classification of properties: which ones do not update live through `/kdf reload` or hot
//! pack configuration. Source of truth is the plugin resource `Resources/Config/cold-properties.json`,
//! exported into datapacks as `catalog/cold_properties.json` (and per property as `kdf.reload`).

use crate::ReflectionIndex;
use serde::Serialize;
use serde_json::Value;
use std::sync::OnceLock;

/// Reload behaviour of a property.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum Reload {
    /// Applies live.
    Hot,
    /// Needs a restart (or never affects existing state).
    Cold,
    /// Class-default change of an actor or component: only actors spawned afterwards see it.
    NewActors,
}

impl Reload {
    pub fn as_str(self) -> &'static str {
        match self {
            Reload::Hot => "hot",
            Reload::Cold => "cold",
            Reload::NewActors => "newActors",
        }
    }
    pub fn parse(text: &str) -> Option<Reload> {
        match text {
            "hot" => Some(Reload::Hot),
            "cold" => Some(Reload::Cold),
            "newActors" => Some(Reload::NewActors),
            _ => None,
        }
    }
    /// `cold` and `newActors` do not apply live.
    pub fn is_not_live(self) -> bool {
        self != Reload::Hot
    }
}

/// Explanation used when the actor rule (not a catalog entry) classifies a property.
pub const NEW_ACTORS_REASON: &str =
    "Class defaults of actors and components only affect actors spawned afterwards; set applyToSpawnedActors: true to update existing ones.";

/// Catalog shipped with the editor: a copy of the plugin resource
/// `Resources/Config/cold-properties.json` (a test keeps both equal). It stands in for datapacks
/// that were exported before the catalog became part of the export; a datapack catalog replaces it.
pub fn builtin_catalog() -> &'static ColdCatalog {
    static CATALOG: OnceLock<ColdCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        ColdCatalog::parse(include_str!("../data/cold-properties.json")).unwrap_or_default()
    })
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ColdEntry {
    pub class: String,
    pub property: String,
    pub reload: Reload,
    pub reason: String,
    pub verified: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ColdCatalog {
    pub version: u64,
    pub entries: Vec<ColdEntry>,
}

impl ColdCatalog {
    /// Parses `{ "version": 1, "entries": [ { class, property, reload, reason, verified } ] }`.
    pub fn parse(text: &str) -> Result<ColdCatalog, String> {
        let value: Value =
            serde_json::from_str(text).map_err(|error| format!("invalid cold catalog: {error}"))?;
        let entries = value
            .get("entries")
            .and_then(Value::as_array)
            .ok_or("cold catalog has no entries array")?;
        let mut catalog = ColdCatalog {
            version: value.get("version").and_then(Value::as_u64).unwrap_or(1),
            entries: Vec::new(),
        };
        for entry in entries {
            let (Some(class), Some(property), Some(reload)) = (
                entry.get("class").and_then(Value::as_str),
                entry.get("property").and_then(Value::as_str),
                entry
                    .get("reload")
                    .and_then(Value::as_str)
                    .and_then(Reload::parse),
            ) else {
                continue;
            };
            catalog.entries.push(ColdEntry {
                class: class.to_owned(),
                property: property.to_owned(),
                reload,
                reason: entry
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                verified: entry
                    .get("verified")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
        }
        Ok(catalog)
    }

    /// Entry that applies to `property` (top-level name, case-insensitive) of `class`. Entries match
    /// the class and its subclasses; the closest ancestor wins, and at equal distance `hot` wins.
    pub fn lookup(
        &self,
        reflection: &ReflectionIndex,
        class: &str,
        property: &str,
    ) -> Option<&ColdEntry> {
        let chain = reflection.super_chain_inclusive(class);
        let mut best: Option<(usize, &ColdEntry)> = None;
        for entry in &self.entries {
            if !entry.property.eq_ignore_ascii_case(property) {
                continue;
            }
            let Some(distance) = chain.iter().position(|path| {
                reflection
                    .class_path(&entry.class)
                    .is_some_and(|wanted| wanted == *path)
            }) else {
                continue;
            };
            let better = match best {
                None => true,
                Some((current, existing)) => {
                    distance < current
                        || (distance == current
                            && entry.reload == Reload::Hot
                            && existing.reload != Reload::Hot)
                }
            };
            if better {
                best = Some((distance, entry));
            }
        }
        best.map(|(_, entry)| entry)
    }
}
