//! Read-only, bounded comparison of two KDF reflection exports.

use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_CLASS_BYTES: u64 = 256 * 1024 * 1024;
const MAX_STRUCT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ENUM_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RECORDS: usize = 200_000;
const MAX_RECORD_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReflectionDiffStatus {
    Ready,
    Limited,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReflectionChangeKind {
    Added,
    Removed,
    Changed,
    /// One side may have omitted this record, so presence cannot be interpreted as a change.
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReflectionChange {
    pub path: String,
    pub change: ReflectionChangeKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReflectionPropertyChange {
    pub owner_path: String,
    pub name: String,
    pub change: ReflectionChangeKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReflectionDiff {
    pub status: ReflectionDiffStatus,
    pub reasons: Vec<String>,
    pub classes: Vec<ReflectionChange>,
    pub properties: Vec<ReflectionPropertyChange>,
    pub structs: Vec<ReflectionChange>,
    pub enums: Vec<ReflectionChange>,
}

#[derive(Default)]
struct RecordSet {
    records: BTreeMap<String, Value>,
    complete: bool,
}

struct Snapshot {
    classes: RecordSet,
    structs: RecordSet,
    enums: RecordSet,
    reasons: Vec<String>,
}

/// Compare paths and JSON reflection records without modifying either archive.
///
/// Known records are compared semantically. A one-sided record is `Unknown` when
/// the absent side has failed loads, incomplete options/metadata, a count
/// mismatch, or a missing NDJSON section. `Changed` means both records were
/// observed and their JSON values differ; callers can inspect `status` and
/// `reasons` before presenting a result as a complete game-update diff.
/// Property changes use declared properties in `classes.ndjson` and
/// `structs.ndjson`; inherited properties are reported at their owning type.
pub fn diff_reflection_archives(
    baseline: &Path,
    candidate: &Path,
) -> Result<ReflectionDiff, String> {
    let baseline = Snapshot::load(baseline)?;
    let candidate = Snapshot::load(candidate)?;
    let mut reasons = baseline
        .reasons
        .iter()
        .map(|reason| format!("baseline: {reason}"))
        .chain(
            candidate
                .reasons
                .iter()
                .map(|reason| format!("candidate: {reason}")),
        )
        .collect::<Vec<_>>();
    reasons.sort();

    Ok(ReflectionDiff {
        status: if reasons.is_empty() {
            ReflectionDiffStatus::Ready
        } else {
            ReflectionDiffStatus::Limited
        },
        reasons,
        classes: compare_records(&baseline.classes, &candidate.classes),
        properties: compare_properties(&baseline, &candidate)?,
        structs: compare_records(&baseline.structs, &candidate.structs),
        enums: compare_records(&baseline.enums, &candidate.enums),
    })
}

impl Snapshot {
    fn load(path: &Path) -> Result<Self, String> {
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("kdfdata"))
        {
            return Err("expected .kdfdata reflection archive".into());
        }
        let metadata = path
            .metadata()
            .map_err(|error| format!("cannot read datapack: {error}"))?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_ARCHIVE_BYTES {
            return Err("datapack is empty, not a file, or exceeds 512 MiB safety limit".into());
        }
        let mut archive = ZipArchive::new(
            File::open(path).map_err(|error| format!("cannot open datapack: {error}"))?,
        )
        .map_err(|error| format!("invalid datapack ZIP: {error}"))?;
        inspect_archive(&mut archive)?;
        let prefix = crate::reflection_archive::unique_reflection_prefix(&archive)?
            .ok_or("datapack has no KDF reflection manifest")?;
        let manifest: Value = serde_json::from_slice(&read_entry(
            &mut archive,
            &format!("{prefix}manifest.json"),
            MAX_MANIFEST_BYTES,
        )?)
        .map_err(|error| format!("invalid reflection manifest: {error}"))?;
        let version = manifest.get("schemaVersion").and_then(Value::as_u64);
        if version != Some(1) {
            return Err(format!("unsupported reflection schemaVersion: {version:?}"));
        }

        let mut reasons = Vec::new();
        let failed_loads = manifest.get("failedLoads").and_then(Value::as_array);
        match failed_loads {
            Some(failures) if !failures.is_empty() => reasons.push(format!(
                "failedLoads contains {} entries; omitted paths remain unknown",
                failures.len()
            )),
            None => reasons.push("failedLoads list is absent or invalid".into()),
            _ => {}
        }
        if manifest.get("metadataAvailable").and_then(Value::as_bool) != Some(true) {
            reasons.push("reflection metadata is incomplete or unavailable".into());
        }
        for option in ["assets", "functions", "flattened"] {
            if manifest.pointer(&format!("/options/{option}")) != Some(&Value::Bool(true)) {
                reasons.push(format!("export option {option} is not true"));
            }
        }
        if manifest.pointer("/counts/classes").and_then(Value::as_u64) == Some(0) {
            reasons.push("manifest reports zero classes".into());
        }
        let globally_complete = reasons.is_empty();
        let mut classes = load_record_set(
            &mut archive,
            &format!("{prefix}classes.ndjson"),
            MAX_CLASS_BYTES,
            manifest.pointer("/counts/classes").and_then(Value::as_u64),
            globally_complete,
            &mut reasons,
        )?;
        let mut structs = load_record_set(
            &mut archive,
            &format!("{prefix}structs.ndjson"),
            MAX_STRUCT_BYTES,
            manifest.pointer("/counts/structs").and_then(Value::as_u64),
            globally_complete,
            &mut reasons,
        )?;
        let enums = load_record_set(
            &mut archive,
            &format!("{prefix}enums.ndjson"),
            MAX_ENUM_BYTES,
            manifest.pointer("/counts/enums").and_then(Value::as_u64),
            globally_complete,
            &mut reasons,
        )?;
        if classes.records.is_empty() {
            reasons.push("class catalog is empty".into());
            classes.complete = false;
        }
        let class_properties_missing = missing_property_lists(&classes);
        if class_properties_missing > 0 {
            reasons.push(format!(
                "{class_properties_missing} classes records have no property list"
            ));
            classes.complete = false;
        }
        let struct_properties_missing = missing_property_lists(&structs);
        if struct_properties_missing > 0 {
            reasons.push(format!(
                "{struct_properties_missing} structs records have no property list"
            ));
            structs.complete = false;
        }
        Ok(Self {
            classes,
            structs,
            enums,
            reasons,
        })
    }
}

fn missing_property_lists(records: &RecordSet) -> usize {
    records
        .records
        .values()
        .filter(|record| record.get("properties").and_then(Value::as_array).is_none())
        .count()
}

fn inspect_archive(archive: &mut ZipArchive<File>) -> Result<(), String> {
    if archive.len() > MAX_ENTRIES {
        return Err("datapack has too many entries".into());
    }
    let mut names = HashSet::new();
    let mut expanded_bytes = 0_u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("cannot inspect datapack entry: {error}"))?;
        let name = entry.name().trim_end_matches('/');
        if name.is_empty()
            || name.contains('\\')
            || name.starts_with('/')
            || name
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || name.contains(':')
        {
            return Err(format!("unsafe datapack entry: {}", entry.name()));
        }
        if !names.insert(entry.name().to_ascii_lowercase()) {
            return Err(format!("duplicate datapack entry: {}", entry.name()));
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("datapack contains a symbolic link".into());
        }
        expanded_bytes = expanded_bytes
            .checked_add(entry.size())
            .ok_or("datapack expanded size overflow")?;
        if expanded_bytes > MAX_EXPANDED_BYTES {
            return Err("datapack expanded size exceeds 4 GiB safety limit".into());
        }
    }
    Ok(())
}

fn read_entry(archive: &mut ZipArchive<File>, name: &str, limit: u64) -> Result<Vec<u8>, String> {
    let entry = archive
        .by_name(name)
        .map_err(|_| format!("missing reflection entry {name}"))?;
    if entry.size() > limit {
        return Err(format!("reflection entry {name} exceeds safety limit"));
    }
    let mut bytes = Vec::new();
    entry
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read reflection entry {name}: {error}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("reflection entry {name} exceeds safety limit"));
    }
    Ok(bytes)
}

fn load_record_set(
    archive: &mut ZipArchive<File>,
    name: &str,
    limit: u64,
    count: Option<u64>,
    globally_complete: bool,
    reasons: &mut Vec<String>,
) -> Result<RecordSet, String> {
    if !archive.file_names().any(|entry| entry == name) {
        reasons.push(format!("{name} section is missing"));
        return Ok(RecordSet::default());
    }
    let bytes = read_entry(archive, name, limit)?;
    let mut records = BTreeMap::new();
    for (number, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        if line.len() > MAX_RECORD_BYTES {
            return Err(format!("{name} record {} exceeds 2 MiB limit", number + 1));
        }
        if records.len() == MAX_RECORDS {
            return Err(format!("{name} exceeds {MAX_RECORDS} records"));
        }
        let record: Value = serde_json::from_slice(line)
            .map_err(|error| format!("invalid {name} record {}: {error}", number + 1))?;
        let path = record
            .get("path")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| format!("{name} record {} has no path", number + 1))?
            .to_owned();
        if records.insert(path.clone(), record).is_some() {
            return Err(format!("duplicate {name} path: {path}"));
        }
    }
    let matches_count = count == Some(records.len() as u64);
    if !matches_count {
        reasons.push(format!(
            "{name} count differs from manifest ({count:?} versus {})",
            records.len()
        ));
    }
    Ok(RecordSet {
        records,
        complete: globally_complete && matches_count,
    })
}

fn compare_records(baseline: &RecordSet, candidate: &RecordSet) -> Vec<ReflectionChange> {
    baseline
        .records
        .keys()
        .chain(candidate.records.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|path| {
            let change = match (baseline.records.get(path), candidate.records.get(path)) {
                (Some(before), Some(after)) if before != after => ReflectionChangeKind::Changed,
                (Some(_), None) if candidate.complete => ReflectionChangeKind::Removed,
                (Some(_), None) => ReflectionChangeKind::Unknown,
                (None, Some(_)) if baseline.complete => ReflectionChangeKind::Added,
                (None, Some(_)) => ReflectionChangeKind::Unknown,
                _ => return None,
            };
            Some(ReflectionChange {
                path: (*path).clone(),
                change,
            })
        })
        .collect()
}

struct PropertySet {
    values: BTreeMap<(String, String), Value>,
    incomplete_owners: BTreeSet<String>,
}

fn collect_properties(snapshot: &Snapshot) -> Result<PropertySet, String> {
    let mut values = BTreeMap::new();
    let mut incomplete_owners = BTreeSet::new();
    for records in [&snapshot.classes.records, &snapshot.structs.records] {
        for (owner_path, record) in records {
            let Some(properties) = record.get("properties").and_then(Value::as_array) else {
                incomplete_owners.insert((*owner_path).clone());
                continue;
            };
            for property in properties {
                let name = property
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| format!("property in {owner_path} has no name"))?
                    .to_owned();
                let key = ((*owner_path).clone(), name);
                if values.insert(key.clone(), property.clone()).is_some() {
                    return Err(format!("duplicate property {} in {owner_path}", key.1));
                }
            }
        }
    }
    Ok(PropertySet {
        values,
        incomplete_owners,
    })
}

fn compare_properties(
    baseline: &Snapshot,
    candidate: &Snapshot,
) -> Result<Vec<ReflectionPropertyChange>, String> {
    let before = collect_properties(baseline)?;
    let after = collect_properties(candidate)?;
    let mut changes = Vec::new();
    for key in before
        .values
        .keys()
        .chain(after.values.keys())
        .collect::<BTreeSet<_>>()
    {
        let (owner_path, name) = key;
        let candidate_complete = (if candidate.classes.records.contains_key(owner_path) {
            candidate.classes.complete
        } else if candidate.structs.records.contains_key(owner_path) {
            candidate.structs.complete
        } else {
            candidate.classes.complete && candidate.structs.complete
        }) && !after.incomplete_owners.contains(owner_path);
        let baseline_complete = (if baseline.classes.records.contains_key(owner_path) {
            baseline.classes.complete
        } else if baseline.structs.records.contains_key(owner_path) {
            baseline.structs.complete
        } else {
            baseline.classes.complete && baseline.structs.complete
        }) && !before.incomplete_owners.contains(owner_path);
        let change = match (before.values.get(key), after.values.get(key)) {
            (Some(old), Some(new)) if old != new => ReflectionChangeKind::Changed,
            (Some(_), None) if candidate_complete => ReflectionChangeKind::Removed,
            (Some(_), None) => ReflectionChangeKind::Unknown,
            (None, Some(_)) if baseline_complete => ReflectionChangeKind::Added,
            (None, Some(_)) => ReflectionChangeKind::Unknown,
            _ => continue,
        };
        changes.push(ReflectionPropertyChange {
            owner_path: (*owner_path).clone(),
            name: (*name).clone(),
            change,
        });
    }
    Ok(changes)
}
