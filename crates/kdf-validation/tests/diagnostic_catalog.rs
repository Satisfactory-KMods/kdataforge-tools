//! The diagnostic catalogue (`tests/fixtures/diagnostic-codes.json`) and the codes the crate can emit
//! must agree, in both directions.
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn catalog() -> Vec<(String, String)> {
    let value: Value =
        serde_json::from_str(include_str!("fixtures/diagnostic-codes.json")).unwrap();
    value["codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["code"].as_str().unwrap().to_owned(),
                entry["severity"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn emitted_codes() -> BTreeSet<String> {
    let pattern = Regex::new(
        r#""((?:config|template|setting|property|pack)\.[a-z]+(?:-[a-z]+)*|datapack\.unknown-class)""#,
    )
    .unwrap();
    let mut found = BTreeSet::new();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in fs::read_dir(source).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        // Skip test modules: only production code counts.
        let production = text.split("#[cfg(test)]").next().unwrap();
        for capture in pattern.captures_iter(production) {
            // `pack.yml` is the manifest file name, not a diagnostic code.
            if &capture[1] != "pack.yml" {
                found.insert(capture[1].to_owned());
            }
        }
    }
    found
}

#[test]
fn every_catalogued_code_is_emitted_and_every_emitted_code_is_catalogued() {
    let catalogued = catalog()
        .into_iter()
        .map(|(code, _)| code)
        .collect::<BTreeSet<_>>();
    let emitted = emitted_codes();
    let missing = catalogued.difference(&emitted).collect::<Vec<_>>();
    let unknown = emitted.difference(&catalogued).collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "catalogued but never emitted: {missing:?}"
    );
    assert!(
        unknown.is_empty(),
        "emitted but not catalogued: {unknown:?}"
    );
}

#[test]
fn catalogue_severities_are_known_letters() {
    for (code, severity) in catalog() {
        assert!(
            matches!(severity.as_str(), "E" | "W" | "I"),
            "{code}: {severity}"
        );
    }
}
