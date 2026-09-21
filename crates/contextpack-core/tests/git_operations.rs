use std::fs;
use std::path::Path;
use std::process::Command;

use contextpack_core::{Status, collect_plan, parse_plan};
use tempfile::tempdir;

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .env("GIT_AUTHOR_DATE", "2020-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2020-01-01T00:00:00Z")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn all_v1_git_operations_execute_without_a_shell() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let dir = tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.name", "Fixture"]);
    git(
        dir.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();
    git(dir.path(), &["add", "tracked.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "initial"]);
    fs::write(dir.path().join("tracked.txt"), "one\ntwo changed\n").unwrap();
    fs::write(dir.path().join("staged.txt"), "staged\n").unwrap();
    git(dir.path(), &["add", "staged.txt"]);

    let plan = parse_plan(
        "version: 1\nrepository: .\ncollect:\n  - { id: status, type: git, operation: status }\n  - { id: branch, type: git, operation: branch }\n  - { id: log, type: git, operation: log, max_entries: 2 }\n  - { id: diff, type: git, operation: diff, target: working_tree }\n  - { id: staged, type: git, operation: diff, target: staged }\n  - { id: stat, type: git, operation: diff_stat, target: baseline, base: HEAD }\n  - { id: revisions, type: git, operation: diff, target: revisions, base: HEAD, head: HEAD }\n  - { id: show, type: git, operation: show, revision: HEAD, paths: [tracked.txt] }\n  - { id: blame, type: git, operation: blame, path: tracked.txt, revision: HEAD, start_line: 1, end_line: 2 }\n  - { id: base, type: git, operation: merge_base, left: HEAD, right: HEAD }\n  - { id: changed, type: git, operation: changed_files, target: working_tree }\n  - { id: check, type: git, operation: diff_check, target: working_tree }\n",
        dir.path().join("plan.yaml"),
    )
    .unwrap();
    let result = collect_plan(&plan);
    assert_eq!(result.query_results.len(), 12);
    for query in &result.query_results {
        assert!(
            matches!(query.status, Status::Ok | Status::Empty),
            "{}: {:?}",
            query.query_id,
            query.diagnostics
        );
    }
    assert_eq!(result.query_results[6].status, Status::Empty);
    assert_eq!(result.query_results[11].status, Status::Empty);
}
