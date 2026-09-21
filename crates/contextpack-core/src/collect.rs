use std::collections::{HashMap, HashSet};

use crate::evidence::{deduplicate, reduce_to_budget, source_evidence};
use crate::git::collect_git;
use crate::ids::assign_pack_id;
use crate::path::{Repository, logical_lines};
use crate::plan::{CollectionPlan, FileQuery, Query, RangeQuery};
use crate::search::{collect_search, source_error_diagnostic};
use crate::symbol::collect_symbol;
use crate::{
    CONTRACT_VERSION, CollectionResult, Diagnostic, EvidenceRecord, Metrics, ProviderInfo,
    QueryResult, QueryType, Resolution, Severity, Status, TOOL_VERSION, ToolInfo, Truncation,
    TruncationKind,
};

struct QueryCollection {
    result: QueryResult,
    evidence: Vec<EvidenceRecord>,
}

pub struct Collector<'a> {
    plan: &'a CollectionPlan,
    repository: Repository,
}

impl<'a> Collector<'a> {
    pub fn new(plan: &'a CollectionPlan) -> Self {
        Self {
            plan,
            repository: Repository::new(plan.repository_root()),
        }
    }

    pub fn collect(&self) -> CollectionResult {
        let mut query_results = Vec::new();
        let mut evidence = Vec::new();
        for query in &self.plan.collect {
            let mut collection = self.collect_query(query);
            apply_query_budget(&mut collection, query.query_budget(&self.plan.limits));
            query_results.push(collection.result);
            evidence.extend(collection.evidence);
        }
        evidence = deduplicate(evidence);
        sort_evidence(&mut evidence, &self.plan.collect);
        // Deduplication keeps IDs because requested_by is excluded from identity.
        for result in &mut query_results {
            result.evidence_ids.sort_by_key(|id| {
                evidence
                    .iter()
                    .position(|record| &record.evidence_id == id)
                    .unwrap_or(usize::MAX)
            });
            result.evidence_ids.dedup();
        }

        let pack_truncations = apply_pack_budget(
            &mut query_results,
            &mut evidence,
            self.plan.limits.max_pack_evidence_bytes,
        );
        sort_evidence(&mut evidence, &self.plan.collect);
        refresh_payload_metrics(&mut query_results, &evidence);

        let mut diagnostics = query_results
            .iter()
            .flat_map(|query| query.diagnostics.clone())
            .chain(
                evidence
                    .iter()
                    .flat_map(|record| record.diagnostics.clone()),
            )
            .collect::<Vec<_>>();
        diagnostics.sort_by(|a, b| {
            (&a.severity, &a.code, &a.query_id, &a.path).cmp(&(
                &b.severity,
                &b.code,
                &b.query_id,
                &b.path,
            ))
        });
        diagnostics.dedup();

        let mut result = CollectionResult {
            contract_version: CONTRACT_VERSION,
            pack_id: String::new(),
            parent_pack: self.plan.parent_pack.clone(),
            tool: ToolInfo {
                name: "contextpack".into(),
                version: TOOL_VERSION.into(),
            },
            providers: providers(),
            normalized_plan: self.plan.normalized_json(),
            query_results,
            evidence,
            diagnostics,
            pack_truncations,
        };
        assign_pack_id(&mut result);
        result
    }

    fn collect_query(&self, query: &Query) -> QueryCollection {
        match query {
            Query::Search(query) => {
                let value = collect_search(self.plan, &self.repository, query);
                QueryCollection {
                    result: value.result,
                    evidence: value.evidence,
                }
            }
            Query::Range(query) => self.collect_range(query),
            Query::File(query) => self.collect_file(query),
            Query::Git(query) => {
                let value = collect_git(self.plan, &self.repository, query);
                QueryCollection {
                    result: value.result,
                    evidence: value.evidence,
                }
            }
            Query::Symbol(query) => {
                let value = collect_symbol(self.plan, &self.repository, query);
                QueryCollection {
                    result: value.result,
                    evidence: value.evidence,
                }
            }
        }
    }

    fn collect_file(&self, query: &FileQuery) -> QueryCollection {
        match self.repository.read_text(&query.path) {
            Ok(content) => {
                let lines = logical_lines(&content);
                self.source_success(
                    &query.id,
                    QueryType::File,
                    &query.path,
                    1,
                    content,
                    lines.len(),
                    false,
                    query
                        .limits
                        .max_evidence_lines
                        .unwrap_or(self.plan.limits.max_evidence_lines),
                    query
                        .limits
                        .max_evidence_bytes
                        .unwrap_or(self.plan.limits.max_evidence_bytes),
                )
            }
            Err(error) => source_failure(&query.id, QueryType::File, &query.path, error),
        }
    }

    fn collect_range(&self, query: &RangeQuery) -> QueryCollection {
        let content = match self.repository.read_text(&query.path) {
            Ok(content) => content,
            Err(error) => return source_failure(&query.id, QueryType::Range, &query.path, error),
        };
        let lines = logical_lines(&content);
        if query.start_line > lines.len() {
            let diagnostic = Diagnostic::query(
                "range_out_of_bounds",
                Severity::Error,
                format!(
                    "range starts at line {}, beyond end of `{}`",
                    query.start_line, query.path
                ),
                &query.id,
            )
            .with_path(&query.path);
            return QueryCollection {
                result: QueryResult {
                    query_id: query.id.clone(),
                    query_type: QueryType::Range,
                    status: Status::Failed,
                    evidence_ids: vec![],
                    candidates: vec![],
                    diagnostics: vec![diagnostic],
                    metrics: Metrics::Source {
                        source_lines: 0,
                        included_lines: 0,
                        source_bytes: 0,
                        included_bytes: 0,
                    },
                    truncations: vec![],
                },
                evidence: vec![],
            };
        }
        let actual_end = query.end_line.min(lines.len());
        let selected = lines[query.start_line - 1..actual_end].concat();
        self.source_success(
            &query.id,
            QueryType::Range,
            &query.path,
            query.start_line,
            selected,
            actual_end - query.start_line + 1,
            query.end_line > lines.len(),
            query
                .limits
                .max_evidence_lines
                .unwrap_or(self.plan.limits.max_evidence_lines),
            query
                .limits
                .max_evidence_bytes
                .unwrap_or(self.plan.limits.max_evidence_bytes),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn source_success(
        &self,
        id: &str,
        query_type: QueryType,
        path: &str,
        start_line: usize,
        content: String,
        source_lines: usize,
        out_of_bounds: bool,
        max_lines: usize,
        max_bytes: usize,
    ) -> QueryCollection {
        let source_bytes = content.len();
        let evidence = source_evidence(
            id,
            path,
            start_line,
            &content,
            Resolution::Exact,
            "filesystem",
            max_lines,
            max_bytes,
        );
        let included_lines = evidence
            .fragments
            .iter()
            .map(|fragment| logical_lines(&fragment.content).len())
            .sum();
        let included_bytes = evidence.payload_bytes();
        let mut diagnostics = Vec::new();
        if out_of_bounds {
            diagnostics.push(
                Diagnostic::query(
                    "range_out_of_bounds",
                    Severity::Warning,
                    format!("requested range extends beyond end of `{path}`"),
                    id,
                )
                .with_path(path),
            );
        }
        if !evidence.truncations.is_empty() {
            diagnostics.push(Diagnostic::query(
                "evidence_limit_reached",
                Severity::Warning,
                format!("source evidence from `{path}` exceeded its evidence limit"),
                id,
            ));
        }
        let status = if out_of_bounds || !evidence.truncations.is_empty() {
            Status::Partial
        } else {
            Status::Ok
        };
        let evidence_id = evidence.evidence_id.clone();
        QueryCollection {
            result: QueryResult {
                query_id: id.into(),
                query_type,
                status,
                evidence_ids: vec![evidence_id],
                candidates: vec![],
                diagnostics,
                metrics: Metrics::Source {
                    source_lines,
                    included_lines,
                    source_bytes,
                    included_bytes,
                },
                truncations: vec![],
            },
            evidence: vec![evidence],
        }
    }
}

pub fn collect_plan(plan: &CollectionPlan) -> CollectionResult {
    Collector::new(plan).collect()
}

fn source_failure(
    id: &str,
    query_type: QueryType,
    path: &str,
    error: crate::path::SourceError,
) -> QueryCollection {
    let unavailable = matches!(
        error,
        crate::path::SourceError::Binary | crate::path::SourceError::UnsupportedEncoding
    );
    QueryCollection {
        result: QueryResult {
            query_id: id.into(),
            query_type,
            status: if unavailable {
                Status::Unavailable
            } else {
                Status::Failed
            },
            evidence_ids: vec![],
            candidates: vec![],
            diagnostics: vec![source_error_diagnostic(id, path, error)],
            metrics: Metrics::Source {
                source_lines: 0,
                included_lines: 0,
                source_bytes: 0,
                included_bytes: 0,
            },
            truncations: vec![],
        },
        evidence: vec![],
    }
}

fn apply_query_budget(collection: &mut QueryCollection, limit: usize) {
    collection.evidence.sort_by(evidence_order);
    let total = collection
        .evidence
        .iter()
        .map(EvidenceRecord::payload_bytes)
        .sum::<usize>();
    if total <= limit {
        return;
    }
    let mut remaining = limit;
    let mut affected = Vec::new();
    for evidence in &mut collection.evidence {
        let before = evidence.payload_bytes();
        if before > remaining {
            let old = evidence.evidence_id.clone();
            reduce_to_budget(evidence, remaining, TruncationKind::QueryBudget);
            affected.push((old, evidence.evidence_id.clone()));
        }
        remaining = remaining.saturating_sub(evidence.payload_bytes());
    }
    let id_map = affected.into_iter().collect::<HashMap<_, _>>();
    for id in &mut collection.result.evidence_ids {
        if let Some(replacement) = id_map.get(id) {
            *id = replacement.clone();
        }
    }
    let included = collection
        .evidence
        .iter()
        .map(EvidenceRecord::payload_bytes)
        .sum();
    let truncation = Truncation {
        kind: TruncationKind::QueryBudget,
        unit: "bytes".into(),
        limit,
        total,
        included,
        omitted: total - included,
        strategy: "stable_evidence_order".into(),
        omitted_source_spans: vec![],
        affected_evidence_ids: collection
            .evidence
            .iter()
            .filter(|e| {
                e.truncations
                    .iter()
                    .any(|t| t.kind == TruncationKind::QueryBudget)
            })
            .map(|e| e.evidence_id.clone())
            .collect(),
    };
    collection.result.truncations.push(truncation);
    collection.result.diagnostics.push(Diagnostic::query(
        "query_budget_reached",
        Severity::Warning,
        format!("query evidence exceeded its {limit}-byte budget"),
        &collection.result.query_id,
    ));
    degrade_status(&mut collection.result.status);
    update_included_metrics(&mut collection.result, included);
}

fn apply_pack_budget(
    query_results: &mut [QueryResult],
    evidence: &mut Vec<EvidenceRecord>,
    limit: usize,
) -> Vec<Truncation> {
    let total = evidence
        .iter()
        .map(EvidenceRecord::payload_bytes)
        .sum::<usize>();
    if total <= limit {
        return vec![];
    }
    let mut by_id = evidence
        .iter()
        .enumerate()
        .map(|(index, record)| (record.evidence_id.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut visited = HashSet::new();
    let mut remaining = limit;
    let mut changed = HashMap::new();
    for result in query_results.iter() {
        for id in &result.evidence_ids {
            let Some(&index) = by_id.get(id) else {
                continue;
            };
            if !visited.insert(index) {
                continue;
            }
            let record = &mut evidence[index];
            if record.payload_bytes() > remaining {
                let old = record.evidence_id.clone();
                reduce_to_budget(record, remaining, TruncationKind::PackBudget);
                changed.insert(old, record.evidence_id.clone());
            }
            remaining = remaining.saturating_sub(record.payload_bytes());
        }
    }
    // Defensive handling for evidence not referenced due to an earlier invariant violation.
    for (index, record) in evidence.iter_mut().enumerate() {
        if !visited.contains(&index) && record.payload_bytes() > 0 {
            let old = record.evidence_id.clone();
            reduce_to_budget(record, 0, TruncationKind::PackBudget);
            changed.insert(old, record.evidence_id.clone());
        }
    }
    by_id.clear();
    for result in query_results.iter_mut() {
        let mut affected = Vec::new();
        for id in &mut result.evidence_ids {
            if let Some(new) = changed.get(id) {
                *id = new.clone();
                affected.push(new.clone());
            }
        }
        if !affected.is_empty() {
            degrade_status(&mut result.status);
            result.diagnostics.push(Diagnostic::query(
                "pack_budget_reached",
                Severity::Warning,
                "pack budget omitted part of this query's evidence",
                &result.query_id,
            ));
            result.truncations.push(Truncation {
                kind: TruncationKind::PackBudget,
                unit: "bytes".into(),
                limit,
                total,
                included: evidence.iter().map(EvidenceRecord::payload_bytes).sum(),
                omitted: total
                    - evidence
                        .iter()
                        .map(EvidenceRecord::payload_bytes)
                        .sum::<usize>(),
                strategy: "plan_order".into(),
                omitted_source_spans: vec![],
                affected_evidence_ids: affected,
            });
        }
    }
    *evidence = deduplicate(std::mem::take(evidence));
    let included = evidence
        .iter()
        .map(EvidenceRecord::payload_bytes)
        .sum::<usize>();
    vec![Truncation {
        kind: TruncationKind::PackBudget,
        unit: "bytes".into(),
        limit,
        total,
        included,
        omitted: total - included,
        strategy: "plan_order".into(),
        omitted_source_spans: vec![],
        affected_evidence_ids: evidence
            .iter()
            .filter(|record| {
                record
                    .truncations
                    .iter()
                    .any(|t| t.kind == TruncationKind::PackBudget)
            })
            .map(|record| record.evidence_id.clone())
            .collect(),
    }]
}

fn update_included_metrics(result: &mut QueryResult, included: usize) {
    match &mut result.metrics {
        Metrics::Source { included_bytes, .. } | Metrics::Git { included_bytes, .. } => {
            *included_bytes = included
        }
        _ => {}
    }
}

fn refresh_payload_metrics(results: &mut [QueryResult], evidence: &[EvidenceRecord]) {
    let by_id = evidence
        .iter()
        .map(|record| (record.evidence_id.as_str(), record))
        .collect::<HashMap<_, _>>();
    for result in results {
        let records = result
            .evidence_ids
            .iter()
            .filter_map(|id| by_id.get(id.as_str()).copied())
            .collect::<Vec<_>>();
        match &mut result.metrics {
            Metrics::Source {
                included_lines,
                included_bytes,
                ..
            } => {
                *included_lines = records
                    .iter()
                    .flat_map(|record| &record.fragments)
                    .map(|fragment| logical_lines(&fragment.content).len())
                    .sum();
                *included_bytes = records.iter().map(|record| record.payload_bytes()).sum();
            }
            Metrics::Git { included_bytes, .. } => {
                *included_bytes = records.iter().map(|record| record.payload_bytes()).sum();
            }
            _ => {}
        }
    }
}

fn degrade_status(status: &mut Status) {
    if matches!(status, Status::Ok | Status::Empty) {
        *status = Status::Partial;
    }
}

fn evidence_order(a: &EvidenceRecord, b: &EvidenceRecord) -> std::cmp::Ordering {
    evidence_sort_key(a).cmp(&evidence_sort_key(b))
}

fn evidence_sort_key(record: &EvidenceRecord) -> (String, usize, String, String) {
    match &record.origin {
        crate::EvidenceOrigin::Source {
            path, full_extent, ..
        } => (
            path.clone(),
            full_extent.start_line,
            "source".into(),
            record.evidence_id.clone(),
        ),
        crate::EvidenceOrigin::Git { .. } => {
            (String::new(), 0, "git".into(), record.evidence_id.clone())
        }
    }
}

fn sort_evidence(records: &mut [EvidenceRecord], queries: &[Query]) {
    let positions = queries
        .iter()
        .enumerate()
        .map(|(index, query)| (query.id(), index))
        .collect::<HashMap<_, _>>();
    records.sort_by(|a, b| {
        let a_query = a
            .requested_by
            .iter()
            .filter_map(|id| positions.get(id.as_str()))
            .min()
            .copied()
            .unwrap_or(usize::MAX);
        let b_query = b
            .requested_by
            .iter()
            .filter_map(|id| positions.get(id.as_str()))
            .min()
            .copied()
            .unwrap_or(usize::MAX);
        (a_query, evidence_sort_key(a)).cmp(&(b_query, evidence_sort_key(b)))
    });
    for record in records {
        record
            .requested_by
            .sort_by_key(|id| positions.get(id.as_str()).copied().unwrap_or(usize::MAX));
    }
}

fn providers() -> Vec<ProviderInfo> {
    vec![
        ProviderInfo {
            id: "filesystem".into(),
            kind: "source".into(),
            implementation: "contextpack-filesystem".into(),
            version: TOOL_VERSION.into(),
            tree_sitter_version: None,
            grammar_version: None,
        },
        ProviderInfo {
            id: "search".into(),
            kind: "search".into(),
            implementation: "grep-regex/ignore".into(),
            version: TOOL_VERSION.into(),
            tree_sitter_version: None,
            grammar_version: None,
        },
        ProviderInfo {
            id: "git-cli".into(),
            kind: "git".into(),
            implementation: "git".into(),
            version: git_version(),
            tree_sitter_version: None,
            grammar_version: None,
        },
        ProviderInfo {
            id: "cpp-tree-sitter".into(),
            kind: "symbol".into(),
            implementation: "tree_sitter_cpp".into(),
            version: TOOL_VERSION.into(),
            tree_sitter_version: Some("0.25.10".into()),
            grammar_version: Some("0.23.4".into()),
        },
    ]
}

fn git_version() -> String {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unavailable".into())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::parse_plan;

    #[test]
    fn file_range_and_search_collect_end_to_end() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join("a.cpp"), "one\ntwo needle\nthree\n").unwrap();
        let plan_path = directory.path().join("plan.yaml");
        let yaml = "version: 1\nrepository: .\ncollect:\n  - id: f\n    type: file\n    path: a.cpp\n  - id: r\n    type: range\n    path: a.cpp\n    start_line: 2\n    end_line: 9\n  - id: s\n    type: search\n    query: needle\n";
        let plan = parse_plan(yaml, &plan_path).unwrap();
        let result = collect_plan(&plan);
        assert_eq!(result.query_results[0].status, Status::Ok);
        assert_eq!(result.query_results[1].status, Status::Partial);
        assert_eq!(result.query_results[2].status, Status::Ok);
        assert!(result.pack_id.starts_with("cp1-"));
    }

    #[test]
    fn deterministic_repeated_collection() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join("a.txt"), "stable\n").unwrap();
        let plan = parse_plan(
            "version: 1\nrepository: .\ncollect:\n  - id: f\n    type: file\n    path: a.txt\n",
            directory.path().join("plan.yaml"),
        )
        .unwrap();
        assert_eq!(collect_plan(&plan), collect_plan(&plan));
    }
}
