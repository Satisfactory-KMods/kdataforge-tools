//! Stable machine-readable reports. CLI and other Rust consumers share one evidence mapping.
use kdf_validation::{
    evidence::{explain, CONTRACT_VERSION},
    Diagnostic, LintResult, Severity,
};
use serde_json::{json, Value};

fn uri(item: &Diagnostic) -> Option<String> {
    item.file.as_ref().map(|path| {
        let absolute = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        };
        url::Url::from_file_path(&absolute)
            .map(|url| url.to_string())
            .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"))
    })
}

pub fn diagnostic_record(item: &Diagnostic) -> Value {
    let explanation = explain(item);
    let mut value = serde_json::to_value(item).expect("diagnostic serialization");
    value["ruleId"] = json!(item.code);
    value["location"] = json!({"uri": uri(item), "document": item.document, "yamlPath": item.yaml_path, "range": item.range});
    value["evidence"] = json!(explanation.evidence);
    value["explanationId"] = json!(explanation.id);
    value
}

pub fn json_report(result: &LintResult) -> Value {
    let mut report = serde_json::to_value(result).expect("lint serialization");
    report["schemaVersion"] = json!(CONTRACT_VERSION);
    report["positionEncoding"] = json!("utf-16");
    report["diagnostics"] = result.diagnostics.iter().map(diagnostic_record).collect();
    report
}

pub fn sarif_report(result: &LintResult) -> Value {
    let mut codes: Vec<_> = result
        .diagnostics
        .iter()
        .map(|item| item.code.as_str())
        .collect();
    codes.sort_unstable();
    codes.dedup();
    let rules: Vec<_> = codes
        .iter()
        .map(|code| json!({"id": code, "shortDescription": {"text": code}}))
        .collect();
    let results: Vec<_> = result.diagnostics.iter().map(|item| {
        let record = diagnostic_record(item);
        let mut region = json!({"startLine": item.range.start.line + 1, "startColumn": item.range.start.character + 1});
        if item.range.end > item.range.start {
            region["endLine"] = json!(item.range.end.line + 1);
            region["endColumn"] = json!(item.range.end.character + 1);
        }
        let locations: Vec<_> = uri(item).into_iter().map(|uri| json!({
            "physicalLocation": {"artifactLocation": {"uri": uri}, "region": region},
        })).collect();
        json!({
            "ruleId": item.code, "ruleIndex": codes.binary_search(&item.code.as_str()).unwrap(),
            "level": match (item.advisory, item.severity) {
                (Some(_), _) | (_, Severity::Information) => "note", (_, Severity::Warning) => "warning", (_, Severity::Error) => "error",
            },
            "message": {"text": item.message}, "locations": locations,
            "partialFingerprints": {"kdfEvidence/v1": record["explanationId"]},
            "properties": {"schemaVersion": CONTRACT_VERSION, "yamlPath": item.yaml_path, "document": item.document,
                "source": item.source, "location": record["location"], "evidence": record["evidence"], "explanationId": record["explanationId"]},
        })
    }).collect();
    json!({"version": "2.1.0", "$schema": "https://json.schemastore.org/sarif-2.1.0.json", "runs": [{
        "tool": {"driver": {"name": "kdf", "version": env!("CARGO_PKG_VERSION"), "rules": rules}},
        "columnKind": "utf16CodeUnits", "results": results,
        "properties": {"schemaVersion": CONTRACT_VERSION, "ok": result.ok, "errors": result.errors, "warnings": result.warnings},
    }]})
}

pub fn why_report(result: &LintResult, rule: Option<&str>, line: Option<u32>) -> Value {
    let explanations: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|item| {
            rule.is_none_or(|rule| item.code == rule)
                && line.is_none_or(|line| item.range.start.line + 1 == line)
        })
        .map(explain)
        .collect();
    json!({"schemaVersion": CONTRACT_VERSION, "positionEncoding": "utf-16", "explanations": explanations})
}

pub fn lint_exit_code(result: &LintResult, warnings_as_errors: bool) -> i32 {
    i32::from(result.errors > 0 || (warnings_as_errors && result.warnings > 0))
}
