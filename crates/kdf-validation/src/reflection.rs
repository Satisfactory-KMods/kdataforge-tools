//! Bounded, read-only reader for the KDF reflection section of a datapack.
use serde::{
    de::{SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use serde_json::value::RawValue;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use zip::ZipArchive;

use crate::reflection_query::Lazy;

mod import_text;

const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_CLASSES: u64 = 256 * 1024 * 1024;
const MAX_FLAT_CLASSES: u64 = 384 * 1024 * 1024;
const MAX_CLASS_LINE: usize = 2 * 1024 * 1024;
const MAX_CLASS_COUNT: usize = 200_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReflectionStatus {
    Ready,
    Limited,
    MissingData,
    UnsupportedVersion,
    Invalid,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassSummary {
    pub path: String,
    pub name: String,
    pub source: Option<String>,
    pub kind: Option<String>,
    #[serde(rename = "abstract")]
    pub abstract_class: bool,
    pub parent: Option<String>,
    pub form: Option<String>,
    pub properties: usize,
    pub texture_path: Option<String>,
    pub remaps_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_descriptor_path: Option<String>,
}

/// Location of the archive section the optional sections (structs, settings, catalog) are read from.
#[derive(Clone, Debug)]
pub(crate) struct ArchiveRef {
    pub(crate) path: PathBuf,
    /// Entry prefix of the reflection section (`""`, `reflection/` or `kdf/reflection/`).
    pub(crate) prefix: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReflectionIndex {
    /// SHA-256 of the exact archive bytes loaded, absent when no archive was usable.
    pub digest: Option<String>,
    #[serde(skip)]
    pub(crate) replacements: Vec<Value>,
    pub status: ReflectionStatus,
    pub reason: Option<String>,
    pub schema_version: Option<u64>,
    pub engine_version: Option<String>,
    pub classes: Vec<ClassSummary>,
    #[serde(skip)]
    pub(crate) full: HashMap<String, Arc<ClassRecord>>,
    #[serde(skip)]
    property_owners: HashMap<String, HashMap<String, String>>,
    #[serde(skip)]
    icon_descriptors: HashMap<String, String>,
    #[serde(skip)]
    pub(crate) archive: Option<ArchiveRef>,
    #[serde(skip)]
    pub(crate) lazy: Lazy,
}

/// Keep serialized class records until requested. A flattened game snapshot repeats
/// hundreds of fields per class; eagerly expanding every serde_json tree consumes
/// gigabytes and makes concurrent index loads starve interactive requests.
#[derive(Debug)]
pub(crate) struct ClassRecord {
    raw: Box<str>,
    parent: Option<String>,
    parsed: OnceLock<Option<Value>>,
    defaults: OnceLock<HashMap<String, SnapshotDefault>>,
    properties_available: bool,
    pub(crate) functions_present: bool,
}

#[derive(Clone, Debug)]
struct SnapshotDefault {
    value: Option<Value>,
    empty_bool: bool,
}

#[derive(Deserialize)]
struct ClassMetadata {
    path: String,
    name: Option<String>,
    source: Option<Value>,
    kind: Option<String>,
    #[serde(rename = "super")]
    parent: Option<String>,
    flags: Option<Value>,
    functions: Option<serde::de::IgnoredAny>,
    #[serde(default, deserialize_with = "property_summary")]
    properties: PropertySummary,
}

#[derive(Default)]
struct PropertySummary {
    available: bool,
    count: usize,
    // Outer None: no mForm property. Some(None): explicit unknown/invalid form; stop inheritance.
    form: Option<Option<&'static str>>,
    signatures: Vec<(String, String, String)>,
    buildable_class: Option<Option<String>>,
}

#[derive(Deserialize)]
struct PropertyMetadata<'a> {
    #[serde(borrow)]
    name: Option<Cow<'a, str>>,
    #[serde(borrow, rename = "default")]
    value: Option<&'a RawValue>,
    #[serde(borrow, rename = "ownerPath")]
    owner: Option<&'a RawValue>,
    #[serde(borrow, rename = "cppType")]
    cpp_type: Option<&'a RawValue>,
}

fn resource_form(value: Option<&RawValue>) -> Option<&'static str> {
    let name: String = serde_json::from_str(value?.get()).ok()?;
    match name.rsplit("::").next()? {
        "RF_SOLID" => Some("solid"),
        "RF_LIQUID" => Some("liquid"),
        "RF_GAS" => Some("gas"),
        _ => None,
    }
}

fn buildable_class_default(value: Option<&RawValue>) -> Option<String> {
    let value = serde_json::from_str::<String>(value?.get()).ok()?;
    let path = [
        "/Script/Engine.BlueprintGeneratedClass'",
        "/Script/CoreUObject.Class'",
    ]
    .iter()
    .find_map(|prefix| {
        value
            .strip_prefix(prefix)
            .and_then(|path| path.strip_suffix('\''))
    })
    .unwrap_or(&value);
    (path.starts_with('/')
        && path.contains('.')
        && !path
            .chars()
            .any(|character| character.is_whitespace() || matches!(character, '\'' | '"')))
    .then(|| path.to_owned())
}

fn property_summary<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<PropertySummary, D::Error> {
    struct Count;
    impl<'de> Visitor<'de> for Count {
        type Value = PropertySummary;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a property array")
        }
        fn visit_seq<A: SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<PropertySummary, A::Error> {
            let mut summary = PropertySummary {
                available: true,
                ..PropertySummary::default()
            };
            while let Some(property) = sequence.next_element::<PropertyMetadata<'de>>()? {
                summary.count += 1;
                if property.name.as_deref() == Some("mForm") {
                    summary.form = Some(resource_form(property.value));
                }
                if property.name.as_deref() == Some("mBuildableClass") {
                    let path = buildable_class_default(property.value);
                    match &summary.buildable_class {
                        None => summary.buildable_class = Some(path),
                        Some(previous) if *previous != path => summary.buildable_class = Some(None),
                        _ => {}
                    }
                }
                if let (Some(name), Some(owner), Some(cpp_type)) = (
                    property.name,
                    property
                        .owner
                        .and_then(|value| serde_json::from_str::<String>(value.get()).ok()),
                    property
                        .cpp_type
                        .and_then(|value| serde_json::from_str::<String>(value.get()).ok()),
                ) {
                    if !name.is_empty() && owner.starts_with('/') && !cpp_type.is_empty() {
                        summary
                            .signatures
                            .push((name.into_owned(), owner, cpp_type));
                    }
                }
            }
            Ok(summary)
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<PropertySummary, E> {
            Ok(PropertySummary::default())
        }
    }
    // Retain classes with unavailable property lists, but never treat those lists as
    // evidence that every property was exported. RawValue also keeps deep metadata bounded.
    let raw = <&RawValue>::deserialize(deserializer)?;
    if !raw.get().trim_start().starts_with('[') {
        return Ok(PropertySummary::default());
    }
    let mut deserializer = serde_json::Deserializer::from_str(raw.get());
    deserializer
        .deserialize_any(Count)
        .map_err(serde::de::Error::custom)
}

impl ReflectionIndex {
    /// Explicit edit previews recheck bytes even when archive size/mtime are unchanged.
    pub fn archive_is_current(&self) -> bool {
        let Some(archive) = &self.archive else {
            return false;
        };
        let Ok(mut file) = File::open(&archive.path) else {
            return false;
        };
        archive_digest(&mut file).ok().as_ref() == self.digest.as_ref()
    }

    /// Compact typed defaults only. Selector previews must not expand every property's metadata.
    /// ImportText defaults are decoded once per class; volatile, undecodable or absent defaults
    /// are unknown (`None`).
    pub(crate) fn snapshot_default(&self, class: &str, name: &str) -> Option<&Value> {
        self.snapshot_property_default(class, name)?.value.as_ref()
    }

    /// Only boolean defaults are usable here. An exported, nonvolatile empty BoolProperty is
    /// false; missing or undecodable defaults remain unknown and still shadow parent defaults.
    pub(crate) fn snapshot_bool_default(&self, class: &str, name: &str) -> Option<bool> {
        let default = self.snapshot_property_default(class, name)?;
        if default.empty_bool {
            Some(false)
        } else {
            default.value.as_ref()?.as_bool()
        }
    }

    fn snapshot_property_default(&self, class: &str, name: &str) -> Option<&SnapshotDefault> {
        #[derive(Deserialize)]
        struct Defaults {
            #[serde(default)]
            properties: Vec<DefaultProperty>,
        }
        #[derive(Deserialize)]
        struct DefaultProperty {
            name: String,
            #[serde(rename = "authoredName")]
            authored_name: Option<String>,
            #[serde(default, deserialize_with = "present_default")]
            default: Option<Value>,
            #[serde(default, rename = "defaultVolatile")]
            volatile: bool,
            kind: Option<String>,
            inner: Option<ElementType>,
        }
        #[derive(Deserialize)]
        struct ElementType {
            kind: Option<String>,
        }
        impl DefaultProperty {
            fn typed_default(self) -> SnapshotDefault {
                let empty_bool = !self.volatile
                    && self.kind.as_deref() == Some("BoolProperty")
                    && self.default.as_ref().and_then(Value::as_str) == Some("");
                let inner_kind = self.inner.and_then(|inner| inner.kind);
                let value = self.default.filter(|_| !self.volatile).and_then(|raw| {
                    import_text::typed_default(self.kind.as_deref(), inner_kind.as_deref(), raw)
                });
                SnapshotDefault { value, empty_bool }
            }
        }
        fn present_default<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
            Value::deserialize(d).map(Some)
        }
        let key = name.to_lowercase();
        for path in self.super_chain_inclusive(class) {
            let record = self.full.get(path)?;
            if !record.properties_available {
                return None;
            }
            let defaults = record.defaults.get_or_init(|| {
                let Ok(decoded) = serde_json::from_str::<Defaults>(&record.raw) else {
                    return HashMap::new();
                };
                let mut values = HashMap::new();
                for mut property in decoded.properties {
                    let name = std::mem::take(&mut property.name).to_lowercase();
                    let authored = property.authored_name.take();
                    let value = property.typed_default();
                    if let Some(authored) = authored {
                        values.insert(authored.to_lowercase(), value.clone());
                    }
                    values.insert(name, value);
                }
                values
            });
            // A declared but unexported child default cannot be replaced by a parent's value.
            if let Some(value) = defaults.get(&key) {
                return Some(value);
            }
        }
        None
    }
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            digest: None,
            replacements: Vec::new(),
            status: ReflectionStatus::MissingData,
            reason: Some(reason.into()),
            schema_version: None,
            engine_version: None,
            classes: Vec::new(),
            full: HashMap::new(),
            property_owners: HashMap::new(),
            icon_descriptors: HashMap::new(),
            archive: None,
            lazy: Lazy::default(),
        }
    }

    pub fn load(path: &Path) -> Self {
        match Self::try_load(path) {
            Ok(index) => index,
            Err(reason) => Self {
                digest: None,
                replacements: Vec::new(),
                status: ReflectionStatus::Invalid,
                reason: Some(reason),
                schema_version: None,
                engine_version: None,
                classes: Vec::new(),
                full: HashMap::new(),
                property_owners: HashMap::new(),
                icon_descriptors: HashMap::new(),
                archive: None,
                lazy: Lazy::default(),
            },
        }
    }

    pub fn class_info(&self, path: &str) -> Option<&Value> {
        let record = self.full.get(path)?;
        record
            .parsed
            .get_or_init(|| {
                // Ignored metadata can exceed serde_json's Value recursion limit.
                // Cache unavailable full metadata rather than terminating the service.
                serde_json::from_str(&record.raw).ok()
            })
            .as_ref()
    }
    pub fn contains_class(&self, path: &str) -> bool {
        self.full.contains_key(path)
    }

    pub fn parent_path(&self, path: &str) -> Option<&str> {
        self.full.get(path)?.parent.as_deref()
    }

    /// Exact cached descriptor default relationship; no name matching or archive scan.
    pub fn icon_descriptor_path(&self, building: &str) -> Option<&str> {
        self.icon_descriptors.get(building).map(String::as_str)
    }

    /// Resolve only a unique declaring class demonstrated by exported signatures.
    /// Built while indexing; interactive lookup never scans/parses class records.
    /// Limited snapshots cannot prove uniqueness because competing owners may be omitted.
    pub fn infer_property_owner<'a>(&'a self, names: &[&str]) -> Option<&'a str> {
        if self.status != ReflectionStatus::Ready || names.len() > 64 {
            return None;
        }
        let signatures = names
            .iter()
            .filter_map(|name| {
                let name = name.split(['.', '[']).next().unwrap_or(name).to_lowercase();
                self.property_owners.get(&name)
            })
            .collect::<Vec<_>>();
        let first = signatures.first()?;
        let candidates = first
            .keys()
            .filter(|owner| {
                self.contains_class(owner)
                    && signatures.iter().all(|signatures| {
                        signatures
                            .get(*owner)
                            .is_some_and(|cpp_type| !cpp_type.is_empty())
                    })
            })
            .collect::<Vec<_>>();
        (candidates.len() == 1).then(|| candidates[0].as_str())
    }

    fn try_load(path: &Path) -> Result<Self, String> {
        if !path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("kdfdata"))
        {
            return Err("expected .kdfdata archive".into());
        }
        let metadata = path
            .metadata()
            .map_err(|e| format!("cannot read datapack: {e}"))?;
        if metadata.len() > MAX_ARCHIVE {
            return Err("datapack exceeds 512 MiB safety limit".into());
        }
        let mut file = File::open(path).map_err(|e| format!("cannot open datapack: {e}"))?;
        let digest = Some(archive_digest(&mut file)?);
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut archive =
            ZipArchive::new(file).map_err(|e| format!("invalid datapack ZIP: {e}"))?;
        let prefix = crate::reflection_archive::unique_reflection_prefix(&archive)?;
        let Some(prefix) = prefix else {
            return Ok(Self::unavailable(
                "bound datapack has no KDF reflection section",
            ));
        };
        let manifest = read_entry(
            &mut archive,
            &format!("{prefix}manifest.json"),
            MAX_MANIFEST,
        )?;
        let manifest: Value = serde_json::from_str(&manifest)
            .map_err(|e| format!("invalid reflection manifest: {e}"))?;
        let version = manifest.get("schemaVersion").and_then(Value::as_u64);
        if version != Some(1) {
            return Ok(Self {
                digest,
                replacements: Vec::new(),
                status: ReflectionStatus::UnsupportedVersion,
                reason: Some(format!("unsupported reflection schemaVersion: {version:?}")),
                schema_version: version,
                engine_version: manifest
                    .get("engineVersion")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                classes: Vec::new(),
                full: HashMap::new(),
                property_owners: HashMap::new(),
                icon_descriptors: HashMap::new(),
                archive: None,
                lazy: Lazy::default(),
            });
        }
        let count = manifest.pointer("/counts/classes").and_then(Value::as_u64);
        if count.is_some_and(|count| count > MAX_CLASS_COUNT as u64) {
            return Err("reflection class count exceeds safety limit".into());
        }
        let raw = read_entry(
            &mut archive,
            &format!("{prefix}classes.ndjson"),
            MAX_CLASSES,
        )?;
        let native_mapping = read_entry(
            &mut archive,
            &format!("{prefix}nativeMapping.json"),
            32 * 1024 * 1024,
        )
        .ok()
        .and_then(|data| serde_json::from_str::<Value>(&data).ok());
        let mut classes = Vec::new();
        let mut full = HashMap::new();
        let mut property_owners: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut building_defaults = HashMap::new();
        let mut forms = HashMap::new();
        let mut properties_reason = None;
        for (number, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            if line.len() > MAX_CLASS_LINE {
                return Err(format!(
                    "reflection class line {} exceeds safety limit",
                    number + 1
                ));
            }
            if classes.len() == MAX_CLASS_COUNT {
                return Err("reflection class count exceeds safety limit".into());
            }
            let record: ClassMetadata = serde_json::from_str(line)
                .map_err(|e| format!("invalid class record at line {}: {e}", number + 1))?;
            let path = record.path.clone();
            if !record.properties.available && properties_reason.is_none() {
                properties_reason = Some(format!(
                    "class {path} properties is missing or not an array; property coverage is unknown"
                ));
            }
            if path.is_empty() {
                return Err(format!("class line {} has no path", number + 1));
            }
            if let Some(form) = record.properties.form {
                forms.insert(path.clone(), form);
            }
            if let Some(buildable) = record.properties.buildable_class {
                building_defaults.insert(path.clone(), buildable);
            }
            for (name, owner, cpp_type) in record.properties.signatures {
                property_owners
                    .entry(name.to_lowercase())
                    .or_default()
                    .entry(owner)
                    .and_modify(|previous| {
                        if *previous != cpp_type {
                            previous.clear();
                        }
                    })
                    .or_insert(cpp_type);
            }
            let summary = ClassSummary {
                name: record
                    .name
                    .as_deref()
                    .unwrap_or_else(|| path.rsplit('.').next().unwrap_or(&path))
                    .to_owned(),
                source: record
                    .source
                    .as_ref()
                    .and_then(|source| source.get("ref"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                kind: record.kind.clone(),
                abstract_class: record
                    .flags
                    .as_ref()
                    .and_then(|flags| flags.get("names"))
                    .and_then(Value::as_array)
                    .is_some_and(|values| values.iter().any(|v| v.as_str() == Some("Abstract"))),
                parent: record.parent.clone(),
                form: None,
                properties: record.properties.count,
                texture_path: None,
                icon_descriptor_path: None,
                remaps_to: native_mapping
                    .as_ref()
                    .and_then(|mapping| mapping.get("natives"))
                    .and_then(|natives| natives.get(&path))
                    .and_then(|entry| entry.get("concreteBlueprint"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                path: path.clone(),
            };
            if full
                .insert(
                    path,
                    Arc::new(ClassRecord {
                        raw: line.into(),
                        parent: record.parent,
                        parsed: OnceLock::new(),
                        defaults: OnceLock::new(),
                        properties_available: record.properties.available,
                        functions_present: record.functions.is_some(),
                    }),
                )
                .is_some()
            {
                return Err(format!("duplicate class path at line {}", number + 1));
            }
            classes.push(summary);
        }
        // Flattened data adds inherited properties when the exporter was asked for it.
        let flat_name = format!("{prefix}classes.flat.ndjson");
        let flattened_expected = manifest
            .pointer("/options/flattened")
            .and_then(Value::as_bool)
            == Some(true);
        let flat_reason = if archive.file_names().any(|name| name == flat_name) {
            match load_flattened(
                &mut archive,
                &flat_name,
                &mut full,
                &mut forms,
                &mut building_defaults,
            ) {
                Ok(loaded) if flattened_expected && loaded != classes.len() => Some(format!(
                    "flattened archive has {loaded} known classes; expected {}",
                    classes.len()
                )),
                Ok(_) => None,
                Err(error) => Some(error),
            }
        } else if flattened_expected {
            Some("flattened class records are missing".into())
        } else {
            None
        };
        resolve_forms(&mut classes, &full, &forms);
        let icon_descriptors = building_icon_descriptors(&full, &building_defaults);
        for class in &mut classes {
            class.icon_descriptor_path = icon_descriptors.get(&class.path).cloned();
        }
        classes.sort_by(|a, b| a.path.cmp(&b.path));
        let count_reason = match count {
            Some(count) if count != classes.len() as u64 => Some(format!(
                "manifest declares {count} classes; archive contains {}",
                classes.len()
            )),
            None => Some("manifest counts.classes is missing or not a nonnegative integer; archive completeness is unknown".into()),
            _ => None,
        };
        let failures = manifest.get("failedLoads").and_then(Value::as_array);
        let failure_reason = match failures {
            None => Some("manifest failedLoads is missing or not an array; archive completeness is unknown".to_owned()),
            Some(failures) if failures.iter().any(|failure| {
                ["path", "reason"].iter().any(|key| {
                    !failure.get(key).and_then(Value::as_str).is_some_and(|value| !value.trim().is_empty())
                })
            }) => Some("manifest failedLoads contains malformed failure records; archive completeness is unknown".to_owned()),
            Some(failures) => manifest.pointer("/counts/failedLoads").and_then(|count| {
                (count.as_u64() != Some(failures.len() as u64)).then(||
                    format!("manifest counts.failedLoads is invalid or does not match {} failure records", failures.len()))
            }),
        };
        let failures = failures.map_or(0, Vec::len);
        let assets_omitted =
            manifest.pointer("/options/assets").and_then(Value::as_bool) == Some(false);
        // Schema 1 legacy exports omit metadataAvailable. An explicit value must
        // affirm availability; malformed values cannot establish completeness.
        let metadata_reason = match manifest.get("metadataAvailable") {
            None | Some(Value::Bool(true)) => None,
            Some(Value::Bool(false)) => Some("reflection metadata unavailable in this build"),
            Some(_) => Some(
                "manifest metadataAvailable is not a boolean; metadata availability is unknown",
            ),
        };
        let status = if failures > 0
            || count_reason.is_some()
            || assets_omitted
            || metadata_reason.is_some()
            || failure_reason.is_some()
            || properties_reason.is_some()
            || flat_reason.is_some()
        {
            ReflectionStatus::Limited
        } else {
            ReflectionStatus::Ready
        };
        let mut reasons = Vec::new();
        if let Some(reason) = count_reason {
            reasons.push(reason);
        }
        if let Some(reason) = failure_reason {
            reasons.push(reason);
        }
        if failures > 0 {
            reasons.push(format!(
                "export reported {failures} failed loads; omitted classes remain unknown"
            ));
        }
        if assets_omitted {
            reasons.push("asset catalog omitted by exporter option".into());
        }
        if let Some(reason) = metadata_reason {
            reasons.push(reason.into());
        }
        if let Some(reason) = properties_reason {
            reasons.push(reason);
        }
        if let Some(reason) = flat_reason {
            reasons.push(reason);
        }
        let reason = (!reasons.is_empty()).then(|| reasons.join("; "));
        Ok(Self {
            digest,
            replacements: manifest
                .get("replacementMetadata")
                .filter(|metadata| metadata.get("schemaVersion").and_then(Value::as_u64) == Some(1))
                .and_then(|metadata| metadata.get("replacements"))
                .and_then(Value::as_array)
                .filter(|records| records.len() <= 4096)
                .cloned()
                .unwrap_or_default(),
            status,
            reason,
            schema_version: version,
            engine_version: manifest
                .get("engineVersion")
                .and_then(Value::as_str)
                .map(str::to_owned),
            classes,
            full,
            property_owners,
            icon_descriptors,
            archive: Some(ArchiveRef {
                path: path.to_path_buf(),
                prefix: prefix.to_owned(),
            }),
            lazy: Lazy::default(),
        })
    }
}

fn archive_digest(file: &mut File) -> Result<String, String> {
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 65536];
    let mut total = 0u64;
    loop {
        let count = file
            .read(&mut bytes)
            .map_err(|e| format!("cannot hash datapack: {e}"))?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_ARCHIVE {
            return Err("datapack exceeds 512 MiB safety limit".into());
        }
        hash.update(&bytes[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn load_flattened(
    archive: &mut ZipArchive<File>,
    name: &str,
    classes: &mut HashMap<String, Arc<ClassRecord>>,
    forms: &mut HashMap<String, Option<&'static str>>,
    building_defaults: &mut HashMap<String, Option<String>>,
) -> Result<usize, String> {
    let entry = archive
        .by_name(name)
        .map_err(|error| format!("cannot read flattened classes: {error}"))?;
    if entry.size() == 0 || entry.size() > MAX_FLAT_CLASSES {
        return Err("flattened class entry is empty or exceeds 384 MiB safety limit".into());
    }
    let mut reader = BufReader::new(entry);
    let mut line = Vec::new();
    let mut seen = HashSet::new();
    let mut total = 0_u64;
    let mut loaded = 0_usize;
    loop {
        line.clear();
        let read = (&mut reader)
            .take((MAX_CLASS_LINE + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|error| format!("cannot read flattened class line: {error}"))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or("flattened class size overflow")?;
        if total > MAX_FLAT_CLASSES || line.len() > MAX_CLASS_LINE {
            return Err("flattened class line exceeds safety limit".into());
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let record: ClassMetadata = serde_json::from_slice(&line)
            .map_err(|error| format!("invalid flattened class line: {error}"))?;
        let path = record.path;
        if !record.properties.available {
            return Err(format!(
                "flattened class {path} properties is missing or not an array; property coverage is unknown"
            ));
        }
        if !seen.insert(path.clone()) {
            return Err(format!("duplicate flattened class path: {path}"));
        }
        if !classes.contains_key(&path) {
            return Err(format!("flattened class has no base record: {path}"));
        }
        if let Some(form) = record.properties.form {
            // Flattened records include per-class CDO overrides for inherited mForm.
            // Example: Desc_Water owns no field; its flattened default is RF_LIQUID.
            forms.insert(path.clone(), form);
        }
        if let Some(buildable) = record.properties.buildable_class {
            building_defaults.insert(path.clone(), buildable);
        }
        let raw = std::str::from_utf8(&line).map_err(|_| "flattened class record is not UTF-8")?;
        classes.insert(
            path,
            Arc::new(ClassRecord {
                raw: raw.into(),
                parent: record.parent,
                parsed: OnceLock::new(),
                defaults: OnceLock::new(),
                properties_available: record.properties.available,
                functions_present: record.functions.is_some(),
            }),
        );
        loaded += 1;
    }
    Ok(loaded)
}

fn building_icon_descriptors(
    classes: &HashMap<String, Arc<ClassRecord>>,
    defaults: &HashMap<String, Option<String>>,
) -> HashMap<String, String> {
    let inherits = |path: &str, base: &str| {
        let mut current = Some(path);
        for _ in 0..128 {
            let Some(path) = current else {
                return false;
            };
            if path == base {
                return true;
            }
            current = classes
                .get(path)
                .and_then(|record| record.parent.as_deref());
        }
        false
    };
    let mut candidates: HashMap<String, Option<String>> = HashMap::new();
    for (descriptor, building) in defaults {
        let Some(building) = building else {
            continue;
        };
        if !inherits(descriptor, "/Script/FactoryGame.FGBuildingDescriptor")
            || !classes.contains_key(building)
            || !inherits(building, "/Script/FactoryGame.FGBuildable")
        {
            continue;
        }
        candidates
            .entry(building.clone())
            .and_modify(|candidate| {
                if candidate.as_ref() != Some(descriptor) {
                    *candidate = None;
                }
            })
            .or_insert_with(|| Some(descriptor.clone()));
    }
    candidates
        .into_iter()
        .filter_map(|(building, descriptor)| descriptor.map(|descriptor| (building, descriptor)))
        .collect()
}

fn resolve_forms(
    classes: &mut [ClassSummary],
    records: &HashMap<String, Arc<ClassRecord>>,
    own: &HashMap<String, Option<&'static str>>,
) {
    let mut resolved: HashMap<&str, Option<&'static str>> = HashMap::new();
    for class in classes {
        let mut current = class.path.as_str();
        let mut visited = HashSet::new();
        let mut trail = Vec::new();
        let form = loop {
            if let Some(form) = resolved.get(current) {
                break *form;
            }
            if let Some(form) = own.get(current) {
                break *form;
            }
            if trail.len() >= 128 || !visited.insert(current) {
                break None;
            }
            trail.push(current);
            match records
                .get(current)
                .filter(|record| record.properties_available)
                .and_then(|record| record.parent.as_deref())
            {
                Some(parent) => current = parent,
                None => break None,
            }
        };
        for path in trail {
            resolved.insert(path, form);
        }
        class.form = form.map(str::to_owned);
    }
}

fn read_entry(archive: &mut ZipArchive<File>, name: &str, max_size: u64) -> Result<String, String> {
    let entry = archive
        .by_name(name)
        .map_err(|_| format!("missing reflection entry {name}"))?;
    if entry.size() > max_size {
        return Err(format!("reflection entry {name} exceeds safety limit"));
    }
    let mut bytes = Vec::new();
    entry
        .take(max_size + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read reflection entry {name}: {e}"))?;
    if bytes.len() as u64 > max_size {
        return Err(format!("reflection entry {name} exceeds safety limit"));
    }
    String::from_utf8(bytes).map_err(|e| format!("reflection entry {name} is not UTF-8: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn form_archive(records: &str) -> tempfile::NamedTempFile {
        archive_with_manifest(
            records,
            serde_json::json!({"schemaVersion": 1, "counts": {"classes": records.lines().filter(|line| !line.is_empty()).count()}, "failedLoads": [], "metadataAvailable": true, "options": {"assets": true}}),
        )
    }

    fn archive_with_manifest(records: &str, manifest: Value) -> tempfile::NamedTempFile {
        let file = tempfile::Builder::new()
            .suffix(".kdfdata")
            .tempfile()
            .unwrap();
        let mut zip = zip::ZipWriter::new(File::create(file.path()).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.start_file("classes.ndjson", options).unwrap();
        zip.write_all(records.as_bytes()).unwrap();
        zip.finish().unwrap();
        file
    }

    #[test]
    fn manifest_class_count_mismatch_including_zero_limits_loaded_records() {
        for count in [0, 2] {
            let file = archive_with_manifest(
                r#"{"path":"/Script/Test.Example","properties":[]}"#,
                serde_json::json!({"schemaVersion": 1, "counts": {"classes": count}, "failedLoads": []}),
            );
            let index = ReflectionIndex::load(file.path());
            assert_eq!(index.status, ReflectionStatus::Limited);
            assert_eq!(
                index.reason.as_deref(),
                Some(format!("manifest declares {count} classes; archive contains 1").as_str())
            );
            assert_eq!(index.classes.len(), 1);
            assert!(index.class_info("/Script/Test.Example").is_some());
        }
    }

    #[test]
    fn manifest_zero_class_count_accepts_empty_archive() {
        let file = archive_with_manifest(
            "\n  \n",
            serde_json::json!({"schemaVersion": 1, "counts": {"classes": 0}, "failedLoads": []}),
        );
        let index = ReflectionIndex::load(file.path());
        assert_eq!(index.status, ReflectionStatus::Ready);
        assert_eq!(index.reason, None);
        assert!(index.classes.is_empty());
    }

    #[test]
    fn missing_invalid_or_malformed_failed_loads_keep_positive_class_lookups() {
        let mut manifests = vec![serde_json::json!({"schemaVersion":1,"counts":{"classes":1}})];
        for failures in [
            Value::Null,
            serde_json::json!(false),
            serde_json::json!(0),
            serde_json::json!("none"),
            serde_json::json!({}),
            serde_json::json!([null]),
            serde_json::json!(["/Missing"]),
            serde_json::json!([{}]),
            serde_json::json!([{"path":"/Missing"}]),
            serde_json::json!([{"path":"/Missing","reason":false}]),
            serde_json::json!([{"path":" ","reason":"failed"}]),
            serde_json::json!([{"path":"/Missing","reason":""}]),
        ] {
            manifests.push(serde_json::json!({"schemaVersion":1,"counts":{"classes":1},"failedLoads":failures}));
        }
        for manifest in manifests {
            let file = archive_with_manifest(
                r#"{"path":"/Script/Test.Example","properties":[{"name":"mValue","cppType":"int32"}]}"#,
                manifest.clone(),
            );
            let index = ReflectionIndex::load(file.path());
            assert_eq!(index.status, ReflectionStatus::Limited, "{manifest}");
            assert!(index.reason.as_deref().unwrap().contains("failedLoads"));
            assert!(index.contains_class("/Script/Test.Example"));
            assert!(index
                .find_property("/Script/Test.Example", "mValue")
                .is_some());
            assert_eq!(index.schema_version, Some(1));
        }
    }

    #[test]
    fn optional_failure_count_must_match_failed_load_records() {
        for count in [
            Value::Null,
            serde_json::json!("0"),
            serde_json::json!(false),
            serde_json::json!(-1),
            serde_json::json!(0.0),
            serde_json::json!(1),
            serde_json::json!([]),
            serde_json::json!({}),
        ] {
            let file = archive_with_manifest(
                r#"{"path":"/Script/Test.Example","properties":[]}"#,
                serde_json::json!({"schemaVersion":1,"counts":{"classes":1,"failedLoads":count},"failedLoads":[]}),
            );
            let index = ReflectionIndex::load(file.path());
            assert_eq!(index.status, ReflectionStatus::Limited, "{count}");
            assert!(index
                .reason
                .as_deref()
                .unwrap()
                .contains("counts.failedLoads"));
            assert!(index.contains_class("/Script/Test.Example"));
        }
        let file = archive_with_manifest(
            r#"{"path":"/Script/Test.Example","properties":[]}"#,
            serde_json::json!({"schemaVersion":1,"counts":{"classes":1,"failedLoads":0},"failedLoads":[]}),
        );
        assert_eq!(
            ReflectionIndex::load(file.path()).status,
            ReflectionStatus::Ready
        );
    }

    #[test]
    fn explicit_metadata_unavailability_or_invalid_type_limits_legacy_schema() {
        for available in [
            Value::Null,
            serde_json::json!(false),
            serde_json::json!("true"),
            serde_json::json!(1),
            serde_json::json!([]),
            serde_json::json!({}),
        ] {
            let file = archive_with_manifest(
                r#"{"path":"/Script/Test.Example","properties":[]}"#,
                serde_json::json!({"schemaVersion":1,"counts":{"classes":1},"failedLoads":[],"metadataAvailable":available}),
            );
            let index = ReflectionIndex::load(file.path());
            assert_eq!(index.status, ReflectionStatus::Limited, "{available}");
            assert!(index.reason.as_deref().unwrap().contains("metadata"));
            assert!(index.contains_class("/Script/Test.Example"));
        }
        // Schema 1 accepted exports predate metadataAvailable and optional capabilities.
        let file = archive_with_manifest(
            r#"{"path":"/Script/Test.Example","properties":[]}"#,
            serde_json::json!({"schemaVersion":1,"counts":{"classes":1},"failedLoads":[]}),
        );
        assert_eq!(
            ReflectionIndex::load(file.path()).status,
            ReflectionStatus::Ready
        );
    }

    #[test]
    fn unavailable_class_properties_limit_without_discarding_other_records() {
        let mut records = vec![serde_json::json!({"path":"/Script/Test.Unknown"})];
        for properties in [
            Value::Null,
            serde_json::json!(false),
            serde_json::json!(0),
            serde_json::json!("[]"),
            serde_json::json!({}),
        ] {
            records
                .push(serde_json::json!({"path":"/Script/Test.Unknown","properties":properties}));
        }
        for record in records {
            let file = form_archive(&format!(
                "{record}\n{}",
                serde_json::json!({
                    "path":"/Script/Test.Known","properties":[{"name":"mValue","cppType":"int32","ownerPath":"/Script/Test.Known"}]
                })
            ));
            let index = ReflectionIndex::load(file.path());
            assert_eq!(index.status, ReflectionStatus::Limited, "{record}");
            assert!(index.reason.as_deref().unwrap().contains("properties"));
            assert!(index.contains_class("/Script/Test.Unknown"));
            assert!(index.class_info("/Script/Test.Unknown").is_some());
            assert!(index
                .find_property("/Script/Test.Known", "mValue")
                .is_some());
            assert_eq!(index.infer_property_owner(&["mValue"]), None);
        }
    }

    #[test]
    fn unknown_properties_stop_inherited_queries_and_defaults() {
        let base = serde_json::json!({"path":"/Script/Test.Base","properties":[
            {"name":"mValue","kind":"BoolProperty","cppType":"bool","default":"True"},
            {"name":"mForm","default":"RF_SOLID"}
        ]});
        let mut unknown =
            vec![serde_json::json!({"path":"/Script/Test.Child","super":"/Script/Test.Base"})];
        for properties in [
            Value::Null,
            serde_json::json!(false),
            serde_json::json!(0),
            serde_json::json!("[]"),
            serde_json::json!({}),
        ] {
            unknown.push(serde_json::json!({"path":"/Script/Test.Child","super":"/Script/Test.Base","properties":properties}));
        }
        // An explicit empty array still permits legacy schema-1 inheritance.
        unknown.push(serde_json::json!({"path":"/Script/Test.Child","super":"/Script/Test.Base","properties":[]}));
        for child in unknown {
            let available = child.get("properties").is_some_and(Value::is_array);
            let grandchild = serde_json::json!({"path":"/Script/Test.Grandchild","super":"/Script/Test.Child","properties":[
                {"name":"mLocal","kind":"IntProperty","cppType":"int32","default":"7"}
            ]});
            let file = form_archive(&format!("{base}\n{child}\n{grandchild}"));
            let index = ReflectionIndex::load(file.path());
            for path in ["/Script/Test.Child", "/Script/Test.Grandchild"] {
                assert_eq!(
                    index.find_property(path, "mValue").is_some(),
                    available,
                    "{child}"
                );
                assert_eq!(
                    index.resolve_property_path(path, "mValue").is_some(),
                    available
                );
                assert_eq!(
                    index.snapshot_bool_default(path, "mValue"),
                    available.then_some(true)
                );
                assert_eq!(
                    index.snapshot_default(path, "mValue"),
                    available.then_some(&Value::Bool(true))
                );
                let members = index.list_members(path, crate::reflection_query::TargetKind::Class);
                assert_eq!(
                    members.iter().any(|member| member.name == "mValue"),
                    available
                );
                assert_eq!(
                    index
                        .classes
                        .iter()
                        .find(|class| class.path == path)
                        .unwrap()
                        .form
                        .as_deref(),
                    available.then_some("solid")
                );
            }
            assert!(index.find_property("/Script/Test.Base", "mValue").is_some());
            assert!(index
                .find_property("/Script/Test.Grandchild", "mLocal")
                .is_some());
            assert_eq!(
                index.snapshot_default("/Script/Test.Grandchild", "mLocal"),
                Some(&serde_json::json!(7))
            );
        }
    }

    #[test]
    fn manifest_missing_or_invalid_class_counts_never_prove_completeness() {
        let mut manifests = vec![
            serde_json::json!({"schemaVersion": 1, "failedLoads": []}),
            serde_json::json!({"schemaVersion": 1, "counts": {}, "failedLoads": []}),
        ];
        for count in [
            Value::Null,
            serde_json::json!("0"),
            serde_json::json!(false),
            serde_json::json!(-1),
            serde_json::json!(0.0),
            serde_json::json!(1.5),
            serde_json::json!([]),
            serde_json::json!({}),
        ] {
            manifests.push(serde_json::json!({"schemaVersion": 1, "counts": {"classes": count}, "failedLoads": []}));
        }
        for manifest in manifests {
            for records in ["", r#"{"path":"/Script/Test.Example","properties":[]}"#] {
                let file = archive_with_manifest(records, manifest.clone());
                let index = ReflectionIndex::load(file.path());
                assert_eq!(index.status, ReflectionStatus::Limited, "{manifest}");
                assert_eq!(
                    index.reason.as_deref(),
                    Some("manifest counts.classes is missing or not a nonnegative integer; archive completeness is unknown")
                );
                assert_eq!(index.classes.len(), usize::from(!records.is_empty()));
            }
        }
    }

    #[test]
    fn query_warmup_keeps_class_records_lazy_and_short_names_unambiguous() {
        let records = concat!(
            "{\"path\":\"/Script/Test.Base\",\"flags\":{\"names\":[\"Abstract\"]},\"properties\":[],\"functions\":[]}\n",
            "{\"path\":\"/Game/One.Duplicate\",\"super\":\"/Script/Test.Base\",\"properties\":[{\"name\":\"mCount\",\"cppType\":\"int32\",\"kind\":\"IntProperty\"}]}\n",
            "{\"path\":\"/Game/Two.Duplicate\",\"super\":\"/Script/Test.Base\",\"properties\":[]}\n",
        );
        let file = form_archive(records);
        let index = ReflectionIndex::load(file.path());
        index.warm();
        assert!(index.has_function_data());
        assert!(index.is_abstract("/script/test.base"));
        assert_eq!(index.descendants("/Script/Test.Base", true).len(), 2);
        assert_eq!(
            index.super_chain_inclusive("/game/one.duplicate"),
            ["/Game/One.Duplicate", "/Script/Test.Base"]
        );
        assert!(
            index.class_path("Duplicate").is_none(),
            "ambiguous short name must not choose one class"
        );
        assert!(
            index
                .full
                .values()
                .all(|record| record.parsed.get().is_none()),
            "warming and metadata queries must not expand every flattened record"
        );
        assert_eq!(
            index
                .find_property("/Game/One.Duplicate", "mCount")
                .unwrap()
                .cpp_type,
            "int32"
        );
        assert_eq!(
            index
                .full
                .values()
                .filter(|record| record.parsed.get().is_some())
                .count(),
            1
        );
    }

    #[test]
    fn summary_form_uses_actual_defaults_and_inheritance_without_expanding_records() {
        let records = concat!(
            "{\"path\":\"/Script/Test.Base\",\"properties\":[{\"default\":\"RF_SOLID\",\"name\":\"mForm\"}]}\n",
            "{\"path\":\"/Game/Test.Child\",\"super\":\"/Script/Test.Base\",\"properties\":[{\"name\":\"mName\"}]}\n",
            "{\"path\":\"/Game/Test.Override\",\"super\":\"/Game/Test.Child\",\"properties\":[{\"default\":\"RF_LIQUID\",\"name\":\"mForm\"}]}\n",
            "{\"path\":\"/Game/Test.Invalid\",\"super\":\"/Script/Test.Base\",\"properties\":[{\"default\":\"RF_INVALID\",\"name\":\"mForm\"}]}\n",
            "{\"path\":\"/Game/Water.GuessedWater\",\"super\":\"/Script/Test.Missing\",\"properties\":[]}\n",
            "{\"path\":\"/Game/Test.CycleA\",\"super\":\"/Game/Test.CycleB\",\"properties\":[]}\n",
            "{\"path\":\"/Game/Test.CycleB\",\"super\":\"/Game/Test.CycleA\",\"properties\":[]}\n",
        );
        let file = form_archive(records);
        let index = ReflectionIndex::load(file.path());
        assert!(
            matches!(index.status, ReflectionStatus::Ready),
            "{:?}",
            index.reason
        );
        let form = |path: &str| {
            index
                .classes
                .iter()
                .find(|entry| entry.path == path)
                .unwrap()
                .form
                .as_deref()
        };
        assert_eq!(form("/Script/Test.Base"), Some("solid"));
        assert_eq!(form("/Game/Test.Child"), Some("solid"));
        assert_eq!(form("/Game/Test.Override"), Some("liquid"));
        assert_eq!(form("/Game/Test.Invalid"), None);
        assert_eq!(form("/Game/Water.GuessedWater"), None);
        assert_eq!(form("/Game/Test.CycleA"), None);
        assert!(index
            .full
            .values()
            .all(|record| record.parsed.get().is_none()));
    }

    #[test]
    fn summary_form_default_before_name_skips_deep_metadata_and_retains_property_count() {
        let deep = format!("{}0{}", "[".repeat(256), "]".repeat(256));
        let records = format!(
            r#"{{"path":"/Script/Test.Gas","properties":[{{"default":{deep},"name":"mDeep"}},{{"default":"EResourceForm::RF_GAS","metadata":{deep},"name":"mForm"}}]}}"#
        );
        let file = form_archive(&records);
        let index = ReflectionIndex::load(file.path());
        assert!(
            matches!(index.status, ReflectionStatus::Ready),
            "{:?}",
            index.reason
        );
        assert_eq!(index.classes[0].form.as_deref(), Some("gas"));
        assert_eq!(index.classes[0].properties, 2);
        assert!(index.full["/Script/Test.Gas"].parsed.get().is_none());
        assert!(index.class_info("/Script/Test.Gas").is_none());
        assert!(
            index.class_info("/Script/Test.Gas").is_none(),
            "failed deep expansion cached safely"
        );
    }

    #[test]
    fn bool_snapshot_distinguishes_empty_defaults_without_changing_generic_snapshots() {
        let base = "/Script/Test.Base";
        let records = [
            serde_json::json!({"path":base,"properties":[
                {"name":"mEmpty","authoredName":"EmptyAlias","kind":"BoolProperty","default":""},
                {"name":"mTrue","kind":"BoolProperty","default":"True"},
                {"name":"mFalse","kind":"BoolProperty","default":"False"},
                {"name":"mUnknown","kind":"BoolProperty","default":"unknown"},
                {"name":"mVolatile","kind":"BoolProperty","default":"","defaultVolatile":true},
                {"name":"mAbsent","kind":"BoolProperty"},
                {"name":"mNull","kind":"BoolProperty","default":null},
                {"name":"mString","kind":"StrProperty","default":""},
                {"name":"mUntyped","default":""}
            ]}),
            serde_json::json!({"path":"/Script/Test.Child","super":base,"properties":[]}),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        let file = form_archive(&records);
        let index = ReflectionIndex::load(file.path());
        for class in [base, "/Script/Test.Child"] {
            for name in ["mEmpty", "emptyalias"] {
                assert_eq!(index.snapshot_bool_default(class, name), Some(false));
                assert_eq!(index.snapshot_default(class, name), None);
            }
            assert_eq!(index.snapshot_bool_default(class, "mTrue"), Some(true));
            assert_eq!(
                index.snapshot_default(class, "mFalse"),
                Some(&Value::Bool(false))
            );
            for name in [
                "mUnknown",
                "mVolatile",
                "mAbsent",
                "mNull",
                "mMissing",
                "mString",
                "mUntyped",
            ] {
                assert_eq!(
                    index.snapshot_bool_default(class, name),
                    None,
                    "{class}.{name}"
                );
            }
        }
        assert!(index
            .full
            .values()
            .all(|record| record.parsed.get().is_none()));
    }

    #[test]
    fn unknown_child_bool_defaults_shadow_known_parent_defaults() {
        let base = serde_json::json!({"path":"/Script/Test.Base","properties":[
            {"name":"mOverride","kind":"BoolProperty","default":"False"}
        ]});
        for property in [
            serde_json::json!({"name":"mOverride","kind":"BoolProperty"}),
            serde_json::json!({"name":"mOverride","kind":"BoolProperty","default":"Unknown"}),
            serde_json::json!({"name":"mOverride","kind":"BoolProperty","default":"","defaultVolatile":true}),
            serde_json::json!({"name":"mOverride","kind":"BoolProperty","default":null}),
        ] {
            let generic_default = property
                .get("default")
                .filter(|value| value.is_null())
                .cloned();
            let child = serde_json::json!({"path":"/Script/Test.Child","super":"/Script/Test.Base","properties":[property]});
            let file = form_archive(&format!("{base}\n{child}"));
            let index = ReflectionIndex::load(file.path());
            assert_eq!(
                index.snapshot_bool_default("/Script/Test.Child", "mOverride"),
                None
            );
            assert_eq!(
                index.snapshot_default("/Script/Test.Child", "mOverride"),
                generic_default.as_ref()
            );
        }
    }

    #[test]
    fn summary_form_uses_flattened_cdo_override_before_parent_default() {
        let base = concat!(
            "{\"path\":\"/Script/FactoryGame.FGItemDescriptor\",\"properties\":[{\"default\":\"RF_SOLID\",\"name\":\"mForm\"}]}\n",
            "{\"path\":\"/Script/FactoryGame.FGResourceDescriptor\",\"super\":\"/Script/FactoryGame.FGItemDescriptor\",\"properties\":[]}\n",
            "{\"path\":\"/Game/Water.Desc_Water_C\",\"super\":\"/Script/FactoryGame.FGResourceDescriptor\",\"properties\":[]}\n",
        );
        let file = form_archive(base);
        let handle = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(file.path())
            .unwrap();
        let mut zip = zip::ZipWriter::new_append(handle).unwrap();
        zip.start_file(
            "classes.flat.ndjson",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(br#"{"path":"/Game/Water.Desc_Water_C","super":"/Script/FactoryGame.FGResourceDescriptor","properties":[{"default":"RF_LIQUID","name":"mForm","ownerPath":"/Script/FactoryGame.FGItemDescriptor"}]}"#).unwrap();
        zip.finish().unwrap();
        let index = ReflectionIndex::load(file.path());
        assert!(
            matches!(index.status, ReflectionStatus::Ready),
            "{:?}",
            index.reason
        );
        assert_eq!(
            index
                .classes
                .iter()
                .find(|class| class.path == "/Game/Water.Desc_Water_C")
                .unwrap()
                .form
                .as_deref(),
            Some("liquid")
        );
        assert!(index
            .full
            .values()
            .all(|record| record.parsed.get().is_none()));
    }

    fn archive(version: u64, prefix: &str, failures: usize) -> tempfile::NamedTempFile {
        let file = tempfile::Builder::new()
            .suffix(".kdfdata")
            .tempfile()
            .unwrap();
        let mut zip = zip::ZipWriter::new(File::create(file.path()).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file(format!("{prefix}manifest.json"), options)
            .unwrap();
        zip.write_all(serde_json::json!({
            "schemaVersion": version,
            "engineVersion": "fixture-engine",
            "counts": {"classes": 1},
            "failedLoads": vec![serde_json::json!({"path":"/Missing", "reason":"not loaded"}); failures],
        }).to_string().as_bytes()).unwrap();
        zip.start_file(format!("{prefix}classes.ndjson"), options)
            .unwrap();
        zip.write_all(b"{\"path\":\"/Script/Test.Example\",\"name\":\"Example\",\"kind\":\"native\",\"super\":\"/Script/CoreUObject.Object\",\"source\":{\"ref\":\"Test\"},\"flags\":{\"names\":[\"Abstract\"]},\"properties\":[{\"name\":\"mValue\"}]}\n").unwrap();
        zip.finish().unwrap();
        file
    }

    fn archive_with_prefixes(prefixes: &[&str]) -> tempfile::NamedTempFile {
        let file = tempfile::Builder::new()
            .suffix(".kdfdata")
            .tempfile()
            .unwrap();
        let mut zip = zip::ZipWriter::new(File::create(file.path()).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for prefix in prefixes {
            zip.start_file(format!("{prefix}manifest.json"), options)
                .unwrap();
            zip.write_all(br#"{"schemaVersion":1,"counts":{"classes":0},"failedLoads":[]}"#)
                .unwrap();
            zip.start_file(format!("{prefix}classes.ndjson"), options)
                .unwrap();
            zip.write_all(b"").unwrap();
        }
        zip.finish().unwrap();
        file
    }

    #[test]
    fn rejects_archives_with_competing_reflection_manifests() {
        for prefixes in [
            &["", "reflection/"][..],
            &["", "kdf/reflection/"][..],
            &["reflection/", "kdf/reflection/"][..],
        ] {
            let file = archive_with_prefixes(prefixes);
            let index = ReflectionIndex::load(file.path());
            assert_eq!(index.status, ReflectionStatus::Invalid, "{prefixes:?}");
            assert_eq!(
                index.reason.as_deref(),
                Some("datapack has multiple KDF reflection manifests"),
                "{prefixes:?}"
            );
        }
    }

    #[test]
    fn preserves_single_supported_reflection_roots_and_missing_section_status() {
        for prefix in ["", "reflection/", "kdf/reflection/"] {
            let file = archive_with_prefixes(&[prefix]);
            assert_eq!(
                ReflectionIndex::load(file.path()).status,
                ReflectionStatus::Ready,
                "{prefix}"
            );
        }

        let file = archive_with_prefixes(&[]);
        let index = ReflectionIndex::load(file.path());
        assert_eq!(index.status, ReflectionStatus::MissingData);
        assert_eq!(
            index.reason.as_deref(),
            Some("bound datapack has no KDF reflection section")
        );
    }

    #[test]
    fn loads_verified_export_and_preserves_missing_metadata() {
        let file = archive(1, "", 0);
        let index = ReflectionIndex::load(file.path());
        assert!(
            matches!(index.status, ReflectionStatus::Ready),
            "{:?}",
            index.reason
        );
        assert_eq!(index.classes.len(), 1);
        assert_eq!(index.classes[0].source.as_deref(), Some("Test"));
        assert!(index.classes[0].abstract_class);
        assert!(index.classes[0].texture_path.is_none());
        assert_eq!(
            index.class_info("/Script/Test.Example").unwrap()["properties"][0]["name"],
            "mValue"
        );
    }

    #[test]
    fn partial_and_future_exports_are_explicit() {
        let partial = archive(1, "reflection/", 1);
        assert!(matches!(
            ReflectionIndex::load(partial.path()).status,
            ReflectionStatus::Limited
        ));
        let future = archive(2, "", 0);
        assert!(matches!(
            ReflectionIndex::load(future.path()).status,
            ReflectionStatus::UnsupportedVersion
        ));
    }

    #[test]
    fn rejects_kdata_extension() {
        let file = tempfile::Builder::new()
            .suffix(".kdata")
            .tempfile()
            .unwrap();
        let mut zip = zip::ZipWriter::new(File::create(file.path()).unwrap());
        zip.start_file(
            "catalog/parts.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"[]").unwrap();
        zip.finish().unwrap();
        let index = ReflectionIndex::load(file.path());
        assert!(matches!(index.status, ReflectionStatus::Invalid));
        assert!(index.reason.unwrap().contains(".kdfdata"));
        assert!(index.classes.is_empty());
    }

    #[test]
    fn loads_large_runtime_class_record_within_bounded_line_limit() {
        let file = tempfile::Builder::new()
            .suffix(".kdfdata")
            .tempfile()
            .unwrap();
        let mut zip = zip::ZipWriter::new(File::create(file.path()).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(br#"{"schemaVersion":1,"counts":{"classes":1},"failedLoads":[]}"#)
            .unwrap();
        zip.start_file("classes.ndjson", options).unwrap();
        let record = serde_json::json!({
            "path": "/Script/Test.Large",
            "name": "Large",
            "properties": [{"name": "mLarge", "value": "x".repeat(1_500_000)}]
        });
        zip.write_all(record.to_string().as_bytes()).unwrap();
        zip.write_all(b"\n").unwrap();
        zip.finish().unwrap();

        let index = ReflectionIndex::load(file.path());
        assert!(
            matches!(index.status, ReflectionStatus::Ready),
            "{:?}",
            index.reason
        );
        assert_eq!(index.classes.len(), 1);
        assert_eq!(
            index.class_info("/Script/Test.Large").unwrap()["properties"][0]["value"]
                .as_str()
                .unwrap()
                .len(),
            1_500_000
        );
    }

    #[test]
    fn flattened_properties_replace_base_records_and_invalid_flat_data_is_limited() {
        fn with_flat(flat: &[u8]) -> tempfile::NamedTempFile {
            let file = tempfile::Builder::new()
                .suffix(".kdfdata")
                .tempfile()
                .unwrap();
            let mut zip = zip::ZipWriter::new(File::create(file.path()).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("manifest.json", options).unwrap();
            zip.write_all(br#"{"schemaVersion":1,"counts":{"classes":1},"failedLoads":[],"metadataAvailable":true,"options":{"assets":true,"flattened":true}}"#)
                .unwrap();
            zip.start_file("classes.ndjson", options).unwrap();
            zip.write_all(br#"{"path":"/Script/Test.Child","name":"Child","properties":[]}"#)
                .unwrap();
            zip.write_all(b"\n").unwrap();
            zip.start_file("classes.flat.ndjson", options).unwrap();
            zip.write_all(flat).unwrap();
            zip.finish().unwrap();
            file
        }

        let valid = with_flat(
            br#"{"path":"/Script/Test.Child","name":"Child","properties":[{"name":"mInherited"}]}"#,
        );
        let loaded = ReflectionIndex::load(valid.path());
        assert!(matches!(loaded.status, ReflectionStatus::Ready));
        assert_eq!(
            loaded.class_info("/Script/Test.Child").unwrap()["properties"][0]["name"],
            "mInherited"
        );

        let invalid = with_flat(b"not-json\n");
        let loaded = ReflectionIndex::load(invalid.path());
        assert!(matches!(loaded.status, ReflectionStatus::Limited));
        assert!(loaded
            .reason
            .unwrap()
            .contains("invalid flattened class line"));
        assert_eq!(loaded.classes.len(), 1);

        let mut unavailable = vec![serde_json::json!({"path":"/Script/Test.Child"})];
        for properties in [
            Value::Null,
            serde_json::json!(false),
            serde_json::json!(0),
            serde_json::json!("[]"),
            serde_json::json!({}),
        ] {
            unavailable
                .push(serde_json::json!({"path":"/Script/Test.Child","properties":properties}));
        }
        for record in unavailable {
            let file = with_flat(record.to_string().as_bytes());
            let index = ReflectionIndex::load(file.path());
            assert_eq!(index.status, ReflectionStatus::Limited, "{record}");
            assert!(index.reason.as_deref().unwrap().contains("properties"));
            assert!(index.contains_class("/Script/Test.Child"));
            assert_eq!(
                index.class_info("/Script/Test.Child").unwrap()["properties"],
                serde_json::json!([]),
                "unavailable flattened properties must preserve usable base record"
            );
        }
    }
}
