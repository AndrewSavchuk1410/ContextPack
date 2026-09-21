use std::collections::{BTreeMap, BTreeSet};

use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};

use crate::evidence::source_evidence;
use crate::path::{Repository, SourceError, logical_lines};
use crate::plan::{CollectionPlan, SearchMode, SearchQuery};
use crate::{
    Diagnostic, EvidenceRecord, Metrics, QueryResult, QueryType, Resolution, Severity, Status,
    Truncation, TruncationKind,
};

pub(crate) struct SearchCollection {
    pub result: QueryResult,
    pub evidence: Vec<EvidenceRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Hit {
    path: String,
    line: usize,
    start: usize,
}

pub(crate) fn collect_search(
    plan: &CollectionPlan,
    repository: &Repository,
    query: &SearchQuery,
) -> SearchCollection {
    let max_matches = query.limits.max_matches.unwrap_or(plan.limits.max_matches);
    let max_lines = query
        .limits
        .max_evidence_lines
        .unwrap_or(plan.limits.max_evidence_lines);
    let max_bytes = query
        .limits
        .max_evidence_bytes
        .unwrap_or(plan.limits.max_evidence_bytes);
    let matcher = build_matcher(query);
    let mut excludes = plan.traversal.exclude.clone();
    excludes.extend(query.exclude.clone());
    let files =
        match repository.discover(&query.paths, &query.extensions, &excludes, &plan.traversal) {
            Ok(files) => files,
            Err(error) => return failed_discovery(query, error),
        };
    let mut hits = Vec::new();
    let mut contents = BTreeMap::new();
    let mut diagnostics = Vec::new();
    for path in files {
        match repository.read_text(&path) {
            Ok(content) => {
                for (line_index, line) in logical_lines(&content).iter().enumerate() {
                    let _ = matcher.find_iter(line.as_bytes(), |m| {
                        hits.push(Hit {
                            path: path.clone(),
                            line: line_index + 1,
                            start: m.start(),
                        });
                        true
                    });
                }
                contents.insert(path, content);
            }
            Err(SourceError::Binary | SourceError::UnsupportedEncoding) => {}
            Err(error) => diagnostics.push(source_error_diagnostic(&query.id, &path, error)),
        }
    }
    hits.sort();
    let total = hits.len();
    let files_with_matches = hits.iter().map(|h| &h.path).collect::<BTreeSet<_>>().len();
    let retained = hits.into_iter().take(max_matches).collect::<Vec<_>>();
    let mut truncations = Vec::new();
    if total > retained.len() {
        truncations.push(Truncation {
            kind: TruncationKind::MatchLimit,
            unit: "matches".into(),
            limit: max_matches,
            total,
            included: retained.len(),
            omitted: total - retained.len(),
            strategy: "stable_prefix".into(),
            omitted_source_spans: vec![],
            affected_evidence_ids: vec![],
        });
        diagnostics.push(Diagnostic::query(
            "match_limit_reached",
            Severity::Warning,
            format!(
                "{total} matches were found; {} were included",
                retained.len()
            ),
            &query.id,
        ));
    }

    let mut windows: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for hit in &retained {
        let line_count = contents
            .get(&hit.path)
            .map(|text| logical_lines(text).len())
            .unwrap_or(0);
        let start = hit.line.saturating_sub(query.context.before).max(1);
        let end = (hit.line + query.context.after).min(line_count);
        windows
            .entry(hit.path.clone())
            .or_default()
            .push((start, end));
    }
    for ranges in windows.values_mut() {
        ranges.sort();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for &(start, end) in ranges.iter() {
            if let Some(last) = merged.last_mut()
                && start <= last.1 + 1
            {
                last.1 = last.1.max(end);
                continue;
            }
            merged.push((start, end));
        }
        *ranges = merged;
    }

    let mut evidence = Vec::new();
    for (path, ranges) in windows {
        let lines = logical_lines(&contents[&path]);
        for (start, end) in ranges {
            let content = lines[start - 1..end].concat();
            let record = source_evidence(
                &query.id,
                &path,
                start,
                &content,
                Resolution::Textual,
                "search",
                max_lines,
                max_bytes,
            );
            if !record.truncations.is_empty() {
                diagnostics.push(Diagnostic::query(
                    "evidence_limit_reached",
                    Severity::Warning,
                    format!("search evidence from `{path}` exceeded its evidence limit"),
                    &query.id,
                ));
            }
            evidence.push(record);
        }
    }
    let has_error = diagnostics.iter().any(|d| d.severity == Severity::Error);
    let status = if total == 0 && has_error {
        Status::Failed
    } else if total == 0 {
        Status::Empty
    } else if !truncations.is_empty()
        || evidence.iter().any(|e| !e.truncations.is_empty())
        || has_error
    {
        Status::Partial
    } else {
        Status::Ok
    };
    let evidence_ids = evidence.iter().map(|e| e.evidence_id.clone()).collect();
    SearchCollection {
        result: QueryResult {
            query_id: query.id.clone(),
            query_type: QueryType::Search,
            status,
            evidence_ids,
            candidates: vec![],
            diagnostics,
            metrics: Metrics::Search {
                matches_found: total,
                matches_included: retained.len(),
                matches_omitted: total - retained.len(),
                files_with_matches,
            },
            truncations,
        },
        evidence,
    }
}

fn build_matcher(query: &SearchQuery) -> RegexMatcher {
    let expression = match query.mode {
        SearchMode::Literal => regex::escape(&query.query),
        SearchMode::Regex => query.query.clone(),
    };
    RegexMatcherBuilder::new()
        .case_insensitive(!query.case_sensitive)
        .build(&expression)
        .expect("regex validated with the same engine")
}

fn failed_discovery(query: &SearchQuery, error: SourceError) -> SearchCollection {
    let diagnostic = source_error_diagnostic(&query.id, ".", error);
    SearchCollection {
        result: QueryResult {
            query_id: query.id.clone(),
            query_type: QueryType::Search,
            status: Status::Failed,
            evidence_ids: vec![],
            candidates: vec![],
            diagnostics: vec![diagnostic],
            metrics: Metrics::Search {
                matches_found: 0,
                matches_included: 0,
                matches_omitted: 0,
                files_with_matches: 0,
            },
            truncations: vec![],
        },
        evidence: vec![],
    }
}

pub(crate) fn source_error_diagnostic(id: &str, path: &str, error: SourceError) -> Diagnostic {
    let (code, message) = match error {
        SourceError::NotFound => ("file_not_found", format!("path was not found: `{path}`")),
        SourceError::PermissionDenied => (
            "permission_denied",
            format!("permission was denied reading `{path}`"),
        ),
        SourceError::Io(message) => ("io_error", message),
        SourceError::Outside => (
            "path_outside_repository",
            format!("path is outside the repository: `{path}`"),
        ),
        SourceError::SymlinkEscape => (
            "symlink_escape",
            format!("symlink resolves outside the repository: `{path}`"),
        ),
        SourceError::Binary => ("binary_file", format!("file is binary: `{path}`")),
        SourceError::UnsupportedEncoding => (
            "unsupported_text_encoding",
            format!("file is not valid UTF-8: `{path}`"),
        ),
    };
    Diagnostic::query(code, Severity::Error, message, id).with_path(path)
}
