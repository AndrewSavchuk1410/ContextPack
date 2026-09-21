use std::fs;

use contextpack_core::{Metrics, Status, TruncationKind, collect_plan, parse_plan};
use tempfile::tempdir;

#[test]
fn search_respects_gitignore_extensions_regex_context_and_match_limit() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join(".gitignore"), "src/ignored.cpp\n").unwrap();
    fs::write(
        dir.path().join("src/visible.cpp"),
        "before\nNeedle(1)\nmiddle\nNeedle(2)\nafter\n",
    )
    .unwrap();
    fs::write(dir.path().join("src/ignored.cpp"), "Needle(99)\n").unwrap();
    fs::write(dir.path().join("src/other.txt"), "Needle(100)\n").unwrap();
    fs::write(dir.path().join(".hidden.cpp"), "Needle(101)\n").unwrap();
    let plan = parse_plan(
        "version: 1\nrepository: .\ncollect:\n  - id: regex\n    type: search\n    query: 'Needle\\([0-9]+\\)'\n    mode: regex\n    paths: [src]\n    extensions: [.CPP]\n    context: { before: 1, after: 1 }\n    limits: { max_matches: 1 }\n",
        dir.path().join("plan.yaml"),
    )
    .unwrap();
    let result = collect_plan(&plan);
    let query = &result.query_results[0];
    assert_eq!(query.status, Status::Partial);
    let Metrics::Search {
        matches_found,
        matches_included,
        files_with_matches,
        ..
    } = query.metrics
    else {
        panic!()
    };
    assert_eq!(
        (matches_found, matches_included, files_with_matches),
        (2, 1, 1)
    );
    assert_eq!(result.evidence.len(), 1);
    assert_eq!(
        result.evidence[0].fragments[0].content,
        "before\nNeedle(1)\nmiddle\n"
    );
    assert!(
        query
            .truncations
            .iter()
            .any(|t| t.kind == TruncationKind::MatchLimit)
    );
}

#[test]
fn evidence_is_deduplicated_and_pack_budget_is_charged_once() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("same.txt"), "1111\n2222\n3333\n4444\n").unwrap();
    let plan = parse_plan(
        "version: 1\nrepository: .\nlimits:\n  max_pack_evidence_bytes: 10\ncollect:\n  - id: first\n    type: file\n    path: same.txt\n  - id: second\n    type: file\n    path: same.txt\n",
        dir.path().join("plan.yaml"),
    )
    .unwrap();
    let result = collect_plan(&plan);
    assert_eq!(result.evidence.len(), 1);
    assert_eq!(result.evidence[0].requested_by, ["first", "second"]);
    assert!(result.evidence[0].payload_bytes() <= 10);
    assert_eq!(
        result.query_results[0].evidence_ids,
        result.query_results[1].evidence_ids
    );
    assert_eq!(result.pack_truncations.len(), 1);
    assert_eq!(result.pack_truncations[0].kind, TruncationKind::PackBudget);
}

#[test]
fn query_budget_keeps_metadata_for_fully_omitted_later_evidence() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "hit aaaaaaaaaa\n").unwrap();
    fs::write(dir.path().join("b.txt"), "hit bbbbbbbbbb\n").unwrap();
    let plan = parse_plan(
        "version: 1\nrepository: .\ncollect:\n  - id: search\n    type: search\n    query: hit\n    context: { before: 0, after: 0 }\n    limits: { max_query_evidence_bytes: 5 }\n",
        dir.path().join("plan.yaml"),
    )
    .unwrap();
    let result = collect_plan(&plan);
    assert_eq!(result.query_results[0].status, Status::Partial);
    assert_eq!(result.evidence.len(), 2);
    assert!(
        result
            .evidence
            .iter()
            .all(|record| record.fragments.is_empty())
    );
    assert!(
        result.query_results[0]
            .truncations
            .iter()
            .any(|t| t.kind == TruncationKind::QueryBudget)
    );
}

#[test]
fn explicitly_named_hidden_search_path_is_deliberate() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join(".hidden.cpp"), "secret needle\n").unwrap();
    fs::write(dir.path().join("visible.cpp"), "no match\n").unwrap();
    let plan = parse_plan(
        "version: 1\nrepository: .\ncollect:\n  - id: hidden\n    type: search\n    query: needle\n    paths: [.hidden.cpp]\n",
        dir.path().join("plan.yaml"),
    )
    .unwrap();
    let result = collect_plan(&plan);
    assert_eq!(result.query_results[0].status, Status::Ok);
    assert_eq!(result.evidence.len(), 1);
}
