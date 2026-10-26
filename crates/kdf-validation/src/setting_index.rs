//! Settings index (`indexes/settings.json` of a datapack): session settings, advanced game
//! settings and local options that `session:`, `ags:` and `option:` references can read.

use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingProvider {
    Session,
    Ags,
    Option,
    Other,
}

impl SettingProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            SettingProvider::Session => "session",
            SettingProvider::Ags => "ags",
            SettingProvider::Option => "option",
            SettingProvider::Other => "other",
        }
    }
    pub fn parse(text: &str) -> SettingProvider {
        match text {
            "session" => SettingProvider::Session,
            "ags" => SettingProvider::Ags,
            "option" => SettingProvider::Option,
            _ => SettingProvider::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingValueType {
    Bool,
    Int,
    Float,
    String,
    Color,
    Other,
}

impl SettingValueType {
    pub fn as_str(self) -> &'static str {
        match self {
            SettingValueType::Bool => "bool",
            SettingValueType::Int => "int",
            SettingValueType::Float => "float",
            SettingValueType::String => "string",
            SettingValueType::Color => "color",
            SettingValueType::Other => "other",
        }
    }
    pub fn parse(text: &str) -> SettingValueType {
        match text {
            "bool" => SettingValueType::Bool,
            "int" => SettingValueType::Int,
            "float" => SettingValueType::Float,
            "string" => SettingValueType::String,
            "color" => SettingValueType::Color,
            _ => SettingValueType::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SettingCategory {
    pub class: String,
    pub display_name: String,
    pub menu_priority: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SettingPlacement {
    pub category: SettingCategory,
    pub sub_category: Option<SettingCategory>,
    pub menu_priority: f64,
}

impl SettingPlacement {
    pub fn display_path(&self) -> String {
        let category = category_label(&self.category);
        match &self.sub_category {
            Some(sub_category) => format!("{category} → {}", category_label(sub_category)),
            None => category.to_owned(),
        }
    }
}

fn category_label(category: &SettingCategory) -> &str {
    if category.display_name.is_empty() {
        &category.class
    } else {
        &category.display_name
    }
}

fn parse_category(value: &Value) -> Option<SettingCategory> {
    let class = value.get("class")?.as_str()?.to_owned();
    if class.is_empty() {
        return None;
    }
    Some(SettingCategory {
        class,
        display_name: value
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        menu_priority: value
            .get("menuPriority")
            .and_then(Value::as_f64)
            .unwrap_or_default(),
    })
}

fn parse_placement(value: &Value) -> Option<SettingPlacement> {
    Some(SettingPlacement {
        category: parse_category(value.get("category")?)?,
        sub_category: value.get("subCategory").and_then(parse_category),
        menu_priority: value
            .get("menuPriority")
            .and_then(Value::as_f64)
            .unwrap_or_default(),
    })
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SettingRecord {
    pub id: String,
    pub provider: SettingProvider,
    pub path: String,
    pub class: String,
    pub display_name: String,
    pub tooltip: String,
    pub value_type: SettingValueType,
    pub default: Option<Value>,
    pub range: Option<(f64, f64)>,
    /// `(value, label)` pairs of selectors.
    pub options: Vec<(Value, String)>,
    pub placements: Vec<SettingPlacement>,
    pub session_wide: bool,
    pub dedicated_server_relevant: bool,
    pub source_ref: Option<String>,
    pub manager_class: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SettingsIndex {
    pub engine_version: Option<String>,
    pub settings: Vec<SettingRecord>,
    /// Settings the exporter could not resolve (`unresolved` list length).
    pub unresolved: usize,
}

impl SettingsIndex {
    pub fn parse(text: &str) -> Result<SettingsIndex, String> {
        let value: Value = serde_json::from_str(text)
            .map_err(|error| format!("invalid settings index: {error}"))?;
        if value.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
            return Err("unsupported settings index schemaVersion".into());
        }
        let text_of = |record: &Value, key: &str| {
            record
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let mut index = SettingsIndex {
            engine_version: value
                .get("engineVersion")
                .and_then(Value::as_str)
                .map(str::to_owned),
            settings: Vec::new(),
            unresolved: value
                .get("unresolved")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
        };
        for record in value
            .get("settings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let id = text_of(record, "id");
            if id.is_empty() {
                continue;
            }
            index.settings.push(SettingRecord {
                id,
                provider: SettingProvider::parse(&text_of(record, "provider")),
                path: text_of(record, "path"),
                class: text_of(record, "class"),
                display_name: text_of(record, "displayName"),
                tooltip: text_of(record, "tooltip"),
                value_type: SettingValueType::parse(&text_of(record, "valueType")),
                default: record.get("default").cloned(),
                range: record.get("range").and_then(|range| {
                    Some((range.get("min")?.as_f64()?, range.get("max")?.as_f64()?))
                }),
                options: record
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|options| {
                        options
                            .iter()
                            .filter_map(|option| {
                                Some((
                                    option.get("value")?.clone(),
                                    option
                                        .get("label")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_owned(),
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                placements: record
                    .get("placements")
                    .and_then(Value::as_array)
                    .map(|placements| placements.iter().filter_map(parse_placement).collect())
                    .unwrap_or_default(),
                session_wide: record
                    .get("sessionWide")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                dedicated_server_relevant: record
                    .get("dedicatedServerRelevant")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                source_ref: record
                    .pointer("/source/ref")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                manager_class: record
                    .get("managerClass")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
        index.settings.sort_by(|a, b| {
            (a.provider.as_str(), a.id.to_lowercase(), &a.path).cmp(&(
                b.provider.as_str(),
                b.id.to_lowercase(),
                &b.path,
            ))
        });
        Ok(index)
    }

    /// Setting a reference reads. `session:` falls back to advanced game settings like the runtime.
    /// The id match is exact first, then case-insensitive.
    pub fn find(&self, provider: &str, id: &str) -> Option<&SettingRecord> {
        let wanted: &[SettingProvider] = match provider {
            "session" => &[SettingProvider::Session, SettingProvider::Ags],
            "ags" => &[SettingProvider::Ags],
            "option" => &[SettingProvider::Option],
            _ => return None,
        };
        for kind in wanted {
            let candidates = self
                .settings
                .iter()
                .filter(|setting| setting.provider == *kind);
            let mut ci = None;
            for setting in candidates {
                if setting.id == id {
                    return Some(setting);
                }
                if ci.is_none() && setting.id.eq_ignore_ascii_case(id) {
                    ci = Some(setting);
                }
            }
            if ci.is_some() {
                return ci;
            }
        }
        None
    }

    /// Settings a provider prefix can read (`session` includes advanced game settings).
    pub fn of_provider(&self, provider: &str) -> Vec<&SettingRecord> {
        let wanted: &[SettingProvider] = match provider {
            "session" => &[SettingProvider::Session, SettingProvider::Ags],
            "ags" => &[SettingProvider::Ags],
            "option" => &[SettingProvider::Option],
            _ => &[],
        };
        self.settings
            .iter()
            .filter(|setting| wanted.contains(&setting.provider))
            .collect()
    }
}
