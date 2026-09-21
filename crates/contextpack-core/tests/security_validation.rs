use std::fs;

use contextpack_core::{Status, collect_plan, parse_plan};
use tempfile::tempdir;

fn errors(yaml: &str) -> Vec<String> {
    let dir = tempdir().unwrap();
    parse_plan(yaml, dir.path().join("plan.yaml"))
        .unwrap_err()
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn rejects_repository_escape_and_absolute_query_paths() {
    let escaped = errors(
        "version: 1\nrepository: .\ncollect:\n  - id: bad\n    type: file\n    path: ../secret\n",
    );
    assert!(escaped.contains(&"path_outside_repository".into()));
    let absolute = errors(
        "version: 1\nrepository: .\ncollect:\n  - id: bad\n    type: file\n    path: C:/secret\n",
    );
    assert!(absolute.contains(&"path_outside_repository".into()));
}

#[test]
fn rejects_git_option_injection_and_invalid_combinations() {
    let revision = errors(
        "version: 1\nrepository: .\ncollect:\n  - id: git\n    type: git\n    operation: show\n    revision: --help\n",
    );
    assert!(revision.contains(&"invalid_query_combination".into()));
    let status = errors(
        "version: 1\nrepository: .\ncollect:\n  - id: git\n    type: git\n    operation: status\n    revision: HEAD\n",
    );
    assert!(status.contains(&"invalid_query_combination".into()));
}

#[test]
fn normalized_plan_expands_limits_and_removes_runtime_repository() {
    let dir = tempdir().unwrap();
    let yaml = format!(
        "version: 1\nrepository: '{}'\ncollect:\n  - id: f\n    type: file\n    path: x\n",
        dir.path().display().to_string().replace('\\', "/")
    );
    let plan = parse_plan(&yaml, dir.path().join("plan.yaml")).unwrap();
    let normalized = plan.normalized_json();
    assert_eq!(normalized["repository"], ".");
    assert_eq!(
        normalized["collect"][0]["limits"]["max_evidence_lines"],
        400
    );
    assert!(
        !normalized
            .to_string()
            .contains(&dir.path().display().to_string())
    );
}

#[test]
fn auto_language_does_not_invent_a_non_cpp_provider() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("module.py"), "def answer(): return 42\n").unwrap();
    let plan = parse_plan(
        "version: 1\nrepository: .\ncollect:\n  - id: python\n    type: symbol\n    name: answer\n    language: auto\n    extensions: [.py]\n",
        dir.path().join("plan.yaml"),
    )
    .unwrap();
    let result = collect_plan(&plan);
    assert_eq!(result.query_results[0].status, Status::Unavailable);
    assert_eq!(
        result.query_results[0].diagnostics[0].code,
        "language_provider_unavailable"
    );
}
