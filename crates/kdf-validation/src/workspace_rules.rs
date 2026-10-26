use crate::document::emit;
use crate::project::ParsedFile;
use crate::rules::{runtime_identity, GeneratedNamespace};
use crate::{LintResult, Severity};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

pub(crate) fn validate_dependencies(
    refs: &BTreeMap<String, PathBuf>,
    dependencies: &BTreeMap<String, Vec<String>>,
    parsed: &[ParsedFile],
    result: &mut LintResult,
) {
    for (reference, deps) in dependencies {
        let Some(path) = refs.get(reference) else {
            continue;
        };
        let Some(file) = parsed.iter().find(|f| &f.path == path) else {
            continue;
        };
        for dep in deps {
            if dep == reference || !refs.contains_key(dep) {
                emit(
                    file,
                    0,
                    "/dependencies",
                    Severity::Error,
                    "semantic.dependency-missing",
                    format!("dependency {dep:?} is missing, invalid, or self-referential"),
                    result,
                );
            }
        }
    }
    let mut visited = HashSet::new();
    let mut stack = Vec::new();
    fn visit(
        reference: &str,
        deps: &BTreeMap<String, Vec<String>>,
        refs: &BTreeMap<String, PathBuf>,
        parsed: &[ParsedFile],
        visited: &mut HashSet<String>,
        stack: &mut Vec<String>,
        result: &mut LintResult,
    ) {
        if let Some(start) = stack.iter().position(|r| r == reference) {
            let cycle = stack[start..]
                .iter()
                .chain(std::iter::once(&reference.to_owned()))
                .cloned()
                .collect::<Vec<_>>()
                .join(" -> ");
            if let Some(path) = refs.get(reference) {
                if let Some(file) = parsed.iter().find(|f| &f.path == path) {
                    emit(
                        file,
                        0,
                        "/dependencies",
                        Severity::Error,
                        "semantic.dependency-cycle",
                        format!("dependency cycle detected: {cycle}"),
                        result,
                    );
                }
            }
            return;
        }
        if !visited.insert(reference.to_owned()) {
            return;
        }
        stack.push(reference.to_owned());
        if let Some(next) = deps.get(reference) {
            for dependency in next {
                if refs.contains_key(dependency) {
                    visit(dependency, deps, refs, parsed, visited, stack, result);
                }
            }
        }
        stack.pop();
    }
    for reference in refs.keys() {
        visit(
            reference,
            dependencies,
            refs,
            parsed,
            &mut visited,
            &mut stack,
            result,
        );
    }
}

pub(crate) fn validate_redirects(
    manifests: &[&ParsedFile],
    generated: &HashMap<(PathBuf, GeneratedNamespace, String), PathBuf>,
    result: &mut LintResult,
) {
    for file in manifests {
        let Some(redirects) = file
            .documents
            .first()
            .and_then(|document| document.get("redirects"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        let Some(pack_root) = file.path.parent() else {
            continue;
        };
        for (old, new) in redirects {
            let Some(new) = new.as_str() else {
                continue;
            };
            let pointer = format!("/redirects/{}", old.replace('~', "~0").replace('/', "~1"));
            if old.eq_ignore_ascii_case(new) {
                emit(
                    file,
                    0,
                    &pointer,
                    Severity::Error,
                    "semantic.redirect-self",
                    format!("redirect {old:?} points to itself"),
                    result,
                );
                continue;
            }
            if generated.contains_key(&(
                pack_root.to_path_buf(),
                GeneratedNamespace::Class,
                runtime_identity(old),
            )) {
                emit(
                    file,
                    0,
                    &pointer,
                    Severity::Error,
                    "semantic.redirect-source-present",
                    format!("redirect source {old:?} is still a generated id in this pack"),
                    result,
                );
            }
            let mut chain = vec![old.to_owned()];
            let mut next = new;
            while let Some(target) = redirects.get(next).and_then(Value::as_str) {
                if chain.iter().any(|entry| entry.eq_ignore_ascii_case(next)) {
                    emit(
                        file,
                        0,
                        &pointer,
                        Severity::Error,
                        "semantic.redirect-cycle",
                        format!("redirect cycle detected from {old:?}"),
                        result,
                    );
                    break;
                }
                chain.push(next.to_owned());
                next = target;
                if chain.len() > redirects.len() + 1 {
                    break;
                }
            }
        }
    }
}
