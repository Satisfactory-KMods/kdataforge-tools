use kdf_validation::{Advisory, Diagnostic, LintResult, Position, Range, Severity};
use serde_json::{json, Value};
use std::process::Command;

#[test]
fn shared_reports_keep_location_evidence_advisory_and_exit_semantics() {
    let mut result = LintResult::default();
    result.diagnostics.push(
        Diagnostic::new(
            Severity::Warning,
            "property.cold",
            "Cold field",
            Some(std::env::temp_dir().join("quoted space.yml")),
            Some(1),
            "$.value",
            Range {
                start: Position {
                    line: 2,
                    character: 4,
                },
                end: Position {
                    line: 2,
                    character: 8,
                },
            },
            "property",
        )
        .with_advisory(Advisory::Hint),
    );
    result.finish();
    let json = kdf_cli::json_report(&result);
    let sarif = kdf_cli::sarif_report(&result);
    let why = kdf_cli::why_report(&result, None, None);
    let diagnostic = &json["diagnostics"][0];
    let finding = &sarif["runs"][0]["results"][0];
    assert_eq!(diagnostic["code"], finding["ruleId"]);
    assert_eq!(diagnostic["location"], finding["properties"]["location"]);
    assert_eq!(diagnostic["evidence"], finding["properties"]["evidence"]);
    assert_eq!(diagnostic["explanationId"], why["explanations"][0]["id"]);
    assert_eq!(finding["level"], "note");
    assert!(diagnostic["location"]["uri"]
        .as_str()
        .unwrap()
        .contains("quoted%20space.yml"));
    assert_eq!(
        finding["locations"][0]["physicalLocation"]["region"],
        json!({"startLine":3,"startColumn":5,"endLine":3,"endColumn":9})
    );
    assert_eq!(kdf_cli::lint_exit_code(&result, true), 0);
    result.diagnostics.push(Diagnostic::new(
        Severity::Warning,
        "semantic.example",
        "Warning",
        None,
        None,
        "$",
        Range::default(),
        "semantic",
    ));
    result.finish();
    assert_eq!(kdf_cli::lint_exit_code(&result, true), 1);
    assert_eq!(kdf_cli::lint_exit_code(&result, false), 0);
    assert!(
        kdf_cli::sarif_report(&result)["runs"][0]["results"][0]["locations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cli_why_has_identical_evidence_and_exit_behavior_to_lint() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("pack.yml");
    std::fs::write(&path, "ref: Test\nname: Test\nversion: wrong\n").unwrap();
    let run = |subcommand: &str| {
        Command::new(env!("CARGO_BIN_EXE_kdf"))
            .arg(subcommand)
            .arg(&path)
            .args(["--format", "json"])
            .env_remove("KDF_USER_SETTINGS_PATH")
            .output()
            .unwrap()
    };
    let lint = run("lint");
    let why = run("why");
    assert_eq!(lint.status.code(), Some(1));
    assert_eq!(why.status.code(), Some(1));
    let lint: Value = serde_json::from_slice(&lint.stdout).unwrap();
    let why: Value = serde_json::from_slice(&why.stdout).unwrap();
    assert_eq!(lint["schemaVersion"], 1);
    assert_eq!(
        lint["diagnostics"][0]["explanationId"],
        why["explanations"][0]["id"]
    );
    assert_eq!(
        lint["diagnostics"][0]["evidence"],
        why["explanations"][0]["evidence"]
    );
    let bad = Command::new(env!("CARGO_BIN_EXE_kdf"))
        .arg("why")
        .arg(&path)
        .args(["--line", "0"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
}
