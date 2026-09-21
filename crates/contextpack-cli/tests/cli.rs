use std::fs;
use std::process::Command;

use tempfile::tempdir;

#[test]
fn validates_and_collects_a_fixture_repository() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("sample.cpp"),
        "namespace Demo { int value(); }\n",
    )
    .unwrap();
    let plan = dir.path().join("plan.yaml");
    fs::write(
        &plan,
        "version: 1\nrepository: .\ncollect:\n  - id: file\n    type: file\n    path: sample.cpp\n  - id: symbol\n    type: symbol\n    name: Demo::value\n    roles: [declaration]\n    kinds: [function]\n",
    )
    .unwrap();
    let binary = env!("CARGO_BIN_EXE_contextpack");
    let validated = Command::new(binary)
        .args(["validate", plan.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        validated.status.success(),
        "{}",
        String::from_utf8_lossy(&validated.stderr)
    );
    let output = dir.path().join("pack.md");
    let json = dir.path().join("pack.json");
    let collected = Command::new(binary)
        .args([
            "collect",
            plan.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--json",
            json.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        collected.status.success(),
        "{}",
        String::from_utf8_lossy(&collected.stderr)
    );
    let markdown = fs::read_to_string(output).unwrap();
    assert!(markdown.contains("# ContextPack"));
    assert!(markdown.contains("### Query `symbol`"));
    assert!(serde_json::from_slice::<serde_json::Value>(&fs::read(json).unwrap()).is_ok());
}

#[test]
fn invalid_plan_returns_validation_exit_code() {
    let dir = tempdir().unwrap();
    let plan = dir.path().join("bad.yaml");
    fs::write(&plan, "version: 9\ncollect: []\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_contextpack"))
        .args(["validate", plan.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported_plan_version"));
}
