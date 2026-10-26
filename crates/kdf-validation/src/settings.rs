//! Lint configuration shared by CLI and LSP. Project keys override user keys.

use crate::project::{ColdPropertyLevel, LintOptions, Profile};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
pub enum LintTrigger {
    #[serde(rename = "onType")]
    OnType,
    #[serde(rename = "onSave")]
    OnSave,
    #[serde(rename = "manual")]
    Manual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Text,
    Json,
    Github,
    Sarif,
}

impl OutputFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
            Self::Github => "github",
            Self::Sarif => "sarif",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LintSettings {
    pub datapack_validation: DatapackValidation,
    pub trigger: LintTrigger,
    pub debounce_ms: u64,
    pub schema_dirs: Vec<PathBuf>,
    pub allow_schema_override: bool,
    pub allow_unknown_types: bool,
    pub warnings_as_errors: bool,
    pub merge_duplicate_diagnostics: bool,
    pub output_format: OutputFormat,
    /// `[lint] coldProperties = "info" | "hint" | "off"`.
    pub cold_properties: ColdPropertyLevel,
}

impl Default for LintSettings {
    fn default() -> Self {
        Self {
            datapack_validation: DatapackValidation::default(),
            trigger: LintTrigger::OnType,
            debounce_ms: 300,
            schema_dirs: Vec::new(),
            allow_schema_override: false,
            allow_unknown_types: false,
            warnings_as_errors: false,
            merge_duplicate_diagnostics: true,
            output_format: OutputFormat::Text,
            cold_properties: ColdPropertyLevel::Info,
        }
    }
}

impl LintSettings {
    pub fn load(root: &Path, user_settings: Option<&Path>) -> Result<Self, String> {
        if !root.exists() {
            return Ok(Self::default());
        }
        let root = dunce::canonicalize(root)
            .map_err(|error| format!("cannot resolve project root {}: {error}", root.display()))?;
        let project_root = if root.is_file() {
            let parent = root.parent().unwrap_or(&root);
            parent
                .ancestors()
                .find(|directory| directory.join(".kdf-editor.toml").is_file())
                .unwrap_or(parent)
        } else if root.file_name().and_then(|value| value.to_str()) == Some("DataForge") {
            root.parent().unwrap_or(&root)
        } else if root.join("pack.yml").is_file() {
            // A selected nested pack belongs to DataForge's project. Search for
            // an explicit nearer override, stopping at that project boundary.
            // Standalone packs have no authoritative ancestor project marker.
            if let Some(project) = root
                .ancestors()
                .find(|directory| {
                    directory.file_name().and_then(|name| name.to_str()) == Some("DataForge")
                })
                .and_then(Path::parent)
            {
                root.ancestors()
                    .take_while(|directory| directory.starts_with(project))
                    .find(|directory| directory.join(".kdf-editor.toml").is_file())
                    .unwrap_or(project)
            } else {
                &root
            }
        } else {
            &root
        };
        let user_settings = user_settings
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("KDF_USER_SETTINGS_PATH").map(PathBuf::from));
        let mut settings = Self::default();
        if let Some(user_path) = user_settings {
            if let Some(patch) = read_patch(&user_path)? {
                settings.apply(patch, user_path.parent().unwrap_or(project_root))?;
            }
        }
        let project_path = project_root.join(".kdf-editor.toml");
        if let Some(patch) = read_patch(&project_path)? {
            settings.apply(patch, project_root)?;
        }
        Ok(settings)
    }

    pub fn lint_options(&self, profile: Profile) -> LintOptions {
        LintOptions {
            datapack_validation: self.datapack_validation.clone(),
            schema_dirs: self.schema_dirs.clone(),
            allow_schema_override: self.allow_schema_override,
            allow_unknown_types: self.allow_unknown_types,
            merge_duplicate_diagnostics: self.merge_duplicate_diagnostics
                && profile != Profile::Legacy,
            profile,
            cold_properties: self.cold_properties,
            ..Default::default()
        }
    }

    fn apply(&mut self, patch: LintPatch, base: &Path) -> Result<(), String> {
        if let Some(validation) = patch.validation {
            for (key, target) in [
                ("check_classes", &mut self.datapack_validation.check_classes),
                (
                    "check_properties",
                    &mut self.datapack_validation.check_properties,
                ),
                ("inlay_hints", &mut self.datapack_validation.inlay_hints),
            ] {
                if let Some(value) = validation.get(key).and_then(toml::Value::as_bool) {
                    *target = value;
                }
            }
            if let Some(value) = validation
                .get("unknown_class_severity")
                .and_then(toml::Value::as_str)
            {
                if let Some(severity) = UnknownClassSeverity::parse(value) {
                    self.datapack_validation.unknown_class_severity = severity;
                }
            }
        }
        if let Some(value) = patch.trigger {
            self.trigger = value;
        }
        if let Some(value) = patch.debounce_ms {
            if value > 60_000 {
                return Err("lint.debounce_ms must be at most 60000".into());
            }
            self.debounce_ms = value;
        }
        if let Some(directories) = patch.schema_dirs {
            self.schema_dirs = directories
                .into_iter()
                .filter_map(|directory| {
                    let (path, enabled) = match directory {
                        SchemaDirectory::Path(path) => (path, true),
                        SchemaDirectory::Entry { path, enabled } => (path, enabled),
                    };
                    if !enabled {
                        return None;
                    }
                    Some(if path.is_absolute() {
                        path
                    } else {
                        base.join(path)
                    })
                })
                .collect();
        }
        if let Some(value) = patch.allow_schema_override {
            self.allow_schema_override = value;
        }
        if let Some(value) = patch.allow_unknown_types {
            self.allow_unknown_types = value;
        }
        if let Some(value) = patch.warnings_as_errors {
            self.warnings_as_errors = value;
        }
        if let Some(value) = patch.merge_duplicate_diagnostics {
            self.merge_duplicate_diagnostics = value;
        }
        if let Some(value) = patch.output_format {
            self.output_format = value;
        }
        if let Some(value) = patch.cold_properties {
            self.cold_properties = match value.to_ascii_lowercase().as_str() {
                "info" | "information" => ColdPropertyLevel::Info,
                "hint" => ColdPropertyLevel::Hint,
                "off" | "none" => ColdPropertyLevel::Off,
                other => {
                    return Err(format!(
                        "lint.coldProperties must be \"info\", \"hint\" or \"off\", got {other:?}"
                    ))
                }
            };
        }
        Ok(())
    }
}

#[derive(Default, Deserialize)]
struct SettingsFile {
    lint: Option<LintPatch>,
    datapack: Option<DatapackPatch>,
}

#[derive(Default, Deserialize)]
struct DatapackPatch {
    validation: Option<toml::Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnknownClassSeverity {
    Error,
    Warning,
    Information,
    Off,
}
impl UnknownClassSeverity {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "error" => Some(Self::Error),
            "warning" => Some(Self::Warning),
            "information" => Some(Self::Information),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatapackValidation {
    pub check_classes: bool,
    pub check_properties: bool,
    pub inlay_hints: bool,
    pub unknown_class_severity: UnknownClassSeverity,
}
impl Default for DatapackValidation {
    fn default() -> Self {
        Self {
            check_classes: true,
            check_properties: true,
            inlay_hints: true,
            unknown_class_severity: UnknownClassSeverity::Warning,
        }
    }
}

#[derive(Default, Deserialize)]
struct LintPatch {
    #[serde(skip)]
    validation: Option<toml::Value>,
    trigger: Option<LintTrigger>,
    debounce_ms: Option<u64>,
    schema_dirs: Option<Vec<SchemaDirectory>>,
    allow_schema_override: Option<bool>,
    allow_unknown_types: Option<bool>,
    warnings_as_errors: Option<bool>,
    merge_duplicate_diagnostics: Option<bool>,
    output_format: Option<OutputFormat>,
    #[serde(rename = "coldProperties", alias = "cold_properties")]
    cold_properties: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SchemaDirectory {
    Path(PathBuf),
    Entry {
        path: PathBuf,
        #[serde(default = "enabled_by_default")]
        enabled: bool,
    },
}

fn enabled_by_default() -> bool {
    true
}

fn read_patch(path: &Path) -> Result<Option<LintPatch>, String> {
    let body = match fs::read_to_string(path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read settings {}: {error}", path.display())),
    };
    let settings: SettingsFile = toml::from_str(&body)
        .map_err(|error| format!("invalid settings {}: {error}", path.display()))?;
    let mut patch = settings.lint.unwrap_or_default();
    patch.validation = settings.datapack.and_then(|value| value.validation);
    Ok(Some(patch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint_path;

    #[test]
    fn project_overrides_user_per_key_and_disabled_dirs_do_not_lint() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("Project");
        let dataforge = project.join("DataForge");
        fs::create_dir_all(&dataforge).unwrap();
        let user = root.path().join("settings.toml");
        fs::write(&user, "[lint]\ntrigger = 'onSave'\ndebounce_ms = 500\nschema_dirs = ['user-schemas']\nwarnings_as_errors = true\noutput_format = 'github'\n").unwrap();
        fs::write(project.join(".kdf-editor.toml"), "[lint]\ntrigger = 'manual'\nschema_dirs = [{path = 'ci/schemas', enabled = false}, {path = 'schemas', enabled = true}]\nallow_unknown_types = true\n").unwrap();
        let settings = LintSettings::load(&dataforge, Some(&user)).unwrap();
        assert_eq!(settings.trigger, LintTrigger::Manual);
        assert_eq!(settings.debounce_ms, 500);
        assert_eq!(
            settings.schema_dirs,
            vec![dunce::canonicalize(&project).unwrap().join("schemas")]
        );
        assert!(settings.allow_unknown_types);
        assert!(settings.warnings_as_errors);
        assert_eq!(settings.output_format, OutputFormat::Github);
    }

    #[test]
    fn cold_properties_setting_accepts_info_hint_off() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".kdf-editor.toml");
        for (text, expected) in [
            ("info", ColdPropertyLevel::Info),
            ("hint", ColdPropertyLevel::Hint),
            ("off", ColdPropertyLevel::Off),
        ] {
            fs::write(
                &path,
                format!(
                    "[lint]
coldProperties = \"{text}\"
"
                ),
            )
            .unwrap();
            let settings = LintSettings::load(root.path(), None).unwrap();
            assert_eq!(settings.cold_properties, expected);
            assert_eq!(
                settings.lint_options(Profile::Runtime).cold_properties,
                expected
            );
        }
        fs::write(
            &path,
            "[lint]
cold_properties = \"hint\"
",
        )
        .unwrap();
        assert_eq!(
            LintSettings::load(root.path(), None)
                .unwrap()
                .cold_properties,
            ColdPropertyLevel::Hint
        );
        fs::write(
            &path,
            "[lint]
coldProperties = \"loud\"
",
        )
        .unwrap();
        assert!(LintSettings::load(root.path(), None).is_err());
    }

    #[test]
    fn rejects_invalid_lint_policy_before_running() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join(".kdf-editor.toml"),
            "[lint]\ntrigger = 'sometimes'\n",
        )
        .unwrap();
        assert!(LintSettings::load(root.path(), None)
            .unwrap_err()
            .contains("invalid settings"));
    }

    #[test]
    fn enabled_override_directory_changes_schema_diagnostics() {
        let project = tempfile::tempdir().unwrap();
        fs::write(
            project.path().join("pack.yml"),
            "ref: Pack\nname: Pack\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(project.path().join("balance.cdo.yml"), "type: cdo\npriority: 100\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mValue\n        value: 1\n").unwrap();
        let schemas = project.path().join("schemas");
        fs::create_dir(&schemas).unwrap();
        fs::write(schemas.join("cdo.schema.yml"), "$schema: https://json-schema.org/draft/2020-12/schema\n$id: https://raw.githubusercontent.com/Satisfactory-KMods/KDataForge-Linter/main/src/kdataforge_linter/schemas/cdo.schema.yml\nx-kdf-type: cdo\ntype: object\nrequired: [type, patches]\nadditionalProperties: false\nproperties:\n  type: {const: cdo}\n  priority: {type: integer}\n  patches: {type: array}\n").unwrap();
        let config = project.path().join(".kdf-editor.toml");
        fs::write(&config, "[lint]\nschema_dirs = [{path = 'schemas', enabled = false}]\nallow_schema_override = true\n").unwrap();
        let disabled = LintSettings::load(project.path(), None).unwrap();
        assert!(
            lint_path(project.path(), &disabled.lint_options(Profile::Runtime))
                .unwrap()
                .diagnostics
                .iter()
                .any(|item| item.code == "schema.unevaluatedProperties")
        );
        fs::write(&config, "[lint]\nschema_dirs = [{path = 'schemas', enabled = true}]\nallow_schema_override = true\n").unwrap();
        let enabled = LintSettings::load(project.path(), None).unwrap();
        let result = lint_path(project.path(), &enabled.lint_options(Profile::Runtime)).unwrap();
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|item| item.code == "schema.unevaluatedProperties"),
            "{:?}",
            result.diagnostics
        );
    }
}
