use std::collections::BTreeMap;

use crate::ids::{assign_evidence_id, evidence_projection};
use crate::{
    EvidenceFragment, EvidenceKind, EvidenceOrigin, EvidenceRecord, Provenance, Resolution,
    SourceExtent, SourceSpan, Truncation, TruncationKind,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn source_evidence(
    query_id: &str,
    path: &str,
    start_line: usize,
    content: &str,
    resolution: Resolution,
    provider: &str,
    max_lines: usize,
    max_bytes: usize,
) -> EvidenceRecord {
    let lines = crate::path::logical_lines(content);
    let end_line = if lines.is_empty() {
        start_line.saturating_sub(1)
    } else {
        start_line + lines.len() - 1
    };
    let mut numbered = lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| (start_line + index, line))
        .collect::<Vec<_>>();
    let mut truncations = Vec::new();
    if numbered.len() > max_lines {
        let total = numbered.len();
        let head = max_lines.div_ceil(2);
        let tail = max_lines / 2;
        let omitted_start = numbered[head].0;
        let omitted_end = numbered[total - tail - 1].0;
        let mut selected = numbered[..head].to_vec();
        selected.extend_from_slice(&numbered[total - tail..]);
        numbered = selected;
        truncations.push(Truncation {
            kind: TruncationKind::LineLimit,
            unit: "lines".into(),
            limit: max_lines,
            total,
            included: max_lines,
            omitted: total - max_lines,
            strategy: "head_tail".into(),
            omitted_source_spans: vec![SourceSpan {
                path: path.into(),
                start_line: omitted_start,
                end_line: omitted_end,
            }],
            affected_evidence_ids: vec![],
        });
    }
    let selected_bytes = numbered.iter().map(|(_, line)| line.len()).sum::<usize>();
    if selected_bytes > max_bytes {
        let before = selected_bytes;
        trim_numbered_to_bytes(&mut numbered, max_bytes);
        let included = numbered.iter().map(|(_, line)| line.len()).sum::<usize>();
        truncations.push(Truncation {
            kind: TruncationKind::ByteLimit,
            unit: "bytes".into(),
            limit: max_bytes,
            total: before,
            included,
            omitted: before - included,
            strategy: "head_tail_line_aligned".into(),
            omitted_source_spans: complement_spans(path, start_line, end_line, &numbered),
            affected_evidence_ids: vec![],
        });
    }
    let fragments = fragments_from_numbered(path, &numbered);
    let mut evidence = EvidenceRecord {
        evidence_id: String::new(),
        kind: EvidenceKind::Source,
        requested_by: vec![query_id.into()],
        resolution,
        provenance: Provenance {
            provider_id: provider.into(),
        },
        origin: EvidenceOrigin::Source {
            path: path.into(),
            full_extent: SourceExtent {
                start_line,
                end_line,
            },
        },
        fragments,
        truncations,
        diagnostics: vec![],
    };
    assign_evidence_id(&mut evidence);
    evidence
}

pub(crate) fn git_evidence(
    query_id: &str,
    origin: EvidenceOrigin,
    content: &str,
    max_lines: usize,
    max_bytes: usize,
) -> EvidenceRecord {
    let mut lines = crate::path::logical_lines(content)
        .into_iter()
        .enumerate()
        .map(|(i, line)| (i + 1, line))
        .collect::<Vec<_>>();
    let mut truncations = Vec::new();
    if lines.len() > max_lines {
        let total = lines.len();
        let head = max_lines.div_ceil(2);
        let tail = max_lines / 2;
        let mut selected = lines[..head].to_vec();
        selected.extend_from_slice(&lines[total - tail..]);
        lines = selected;
        truncations.push(Truncation {
            kind: TruncationKind::LineLimit,
            unit: "lines".into(),
            limit: max_lines,
            total,
            included: max_lines,
            omitted: total - max_lines,
            strategy: "head_tail".into(),
            omitted_source_spans: vec![],
            affected_evidence_ids: vec![],
        });
    }
    let selected_bytes = lines.iter().map(|(_, line)| line.len()).sum::<usize>();
    if selected_bytes > max_bytes {
        trim_numbered_to_bytes(&mut lines, max_bytes);
        let included = lines.iter().map(|(_, line)| line.len()).sum::<usize>();
        truncations.push(Truncation {
            kind: TruncationKind::ByteLimit,
            unit: "bytes".into(),
            limit: max_bytes,
            total: selected_bytes,
            included,
            omitted: selected_bytes - included,
            strategy: "head_tail_line_aligned".into(),
            omitted_source_spans: vec![],
            affected_evidence_ids: vec![],
        });
    }
    let fragments = group_numbered(&lines)
        .into_iter()
        .map(|group| EvidenceFragment {
            span: None,
            content: group.into_iter().map(|(_, line)| line).collect(),
        })
        .collect();
    let mut evidence = EvidenceRecord {
        evidence_id: String::new(),
        kind: EvidenceKind::Git,
        requested_by: vec![query_id.into()],
        resolution: Resolution::Exact,
        provenance: Provenance {
            provider_id: "git-cli".into(),
        },
        origin,
        fragments,
        truncations,
        diagnostics: vec![],
    };
    assign_evidence_id(&mut evidence);
    evidence
}

fn trim_numbered_to_bytes(lines: &mut Vec<(usize, String)>, limit: usize) {
    if lines.is_empty() {
        return;
    }
    let split = lines.len().div_ceil(2);
    let mut head = lines[..split].to_vec();
    let mut tail = lines[split..].to_vec();
    while head
        .iter()
        .chain(&tail)
        .map(|(_, s)| s.len())
        .sum::<usize>()
        > limit
    {
        let head_bytes = head.iter().map(|(_, s)| s.len()).sum::<usize>();
        let tail_bytes = tail.iter().map(|(_, s)| s.len()).sum::<usize>();
        if head_bytes > tail_bytes {
            if head.pop().is_none() && !tail.is_empty() {
                tail.remove(0);
            }
        } else if !tail.is_empty() {
            tail.remove(0);
        } else if head.pop().is_none() {
            break;
        }
    }
    head.extend(tail);
    *lines = head;
}

fn fragments_from_numbered(path: &str, lines: &[(usize, String)]) -> Vec<EvidenceFragment> {
    group_numbered(lines)
        .into_iter()
        .map(|group| EvidenceFragment {
            span: Some(SourceSpan {
                path: path.into(),
                start_line: group.first().unwrap().0,
                end_line: group.last().unwrap().0,
            }),
            content: group.into_iter().map(|(_, line)| line).collect(),
        })
        .collect()
}

fn group_numbered(lines: &[(usize, String)]) -> Vec<Vec<(usize, String)>> {
    let mut groups: Vec<Vec<(usize, String)>> = Vec::new();
    for line in lines {
        if groups
            .last()
            .and_then(|g| g.last())
            .is_none_or(|previous| previous.0 + 1 != line.0)
        {
            groups.push(Vec::new());
        }
        groups.last_mut().unwrap().push(line.clone());
    }
    groups
}

fn complement_spans(
    path: &str,
    start: usize,
    end: usize,
    retained: &[(usize, String)],
) -> Vec<SourceSpan> {
    let retained = retained.iter().map(|(line, _)| *line).collect::<Vec<_>>();
    let mut spans = Vec::new();
    let mut omitted_start = None;
    for line in start..=end {
        if retained.binary_search(&line).is_err() {
            omitted_start.get_or_insert(line);
        } else if let Some(begin) = omitted_start.take() {
            spans.push(SourceSpan {
                path: path.into(),
                start_line: begin,
                end_line: line - 1,
            });
        }
    }
    if let Some(begin) = omitted_start {
        spans.push(SourceSpan {
            path: path.into(),
            start_line: begin,
            end_line: end,
        });
    }
    spans
}

pub(crate) fn reduce_to_budget(
    evidence: &mut EvidenceRecord,
    limit: usize,
    kind: TruncationKind,
) -> bool {
    let total = evidence.payload_bytes();
    if total <= limit {
        return false;
    }
    match evidence.kind {
        EvidenceKind::Source => {
            let mut lines = evidence
                .fragments
                .iter()
                .flat_map(|fragment| {
                    let start = fragment.span.as_ref().map_or(1, |s| s.start_line);
                    crate::path::logical_lines(&fragment.content)
                        .into_iter()
                        .enumerate()
                        .map(move |(i, line)| (start + i, line))
                })
                .collect::<Vec<_>>();
            trim_numbered_to_bytes(&mut lines, limit);
            let path = match &evidence.origin {
                EvidenceOrigin::Source { path, .. } => path.clone(),
                _ => unreachable!(),
            };
            evidence.fragments = fragments_from_numbered(&path, &lines);
        }
        EvidenceKind::Git => {
            let mut lines = evidence
                .fragments
                .iter()
                .flat_map(|f| crate::path::logical_lines(&f.content))
                .enumerate()
                .map(|(i, line)| (i + 1, line))
                .collect::<Vec<_>>();
            trim_numbered_to_bytes(&mut lines, limit);
            evidence.fragments = group_numbered(&lines)
                .into_iter()
                .map(|group| EvidenceFragment {
                    span: None,
                    content: group.into_iter().map(|(_, s)| s).collect(),
                })
                .collect();
        }
    }
    let included = evidence.payload_bytes();
    evidence.truncations.push(Truncation {
        kind: kind.clone(),
        unit: "bytes".into(),
        limit,
        total,
        included,
        omitted: total - included,
        strategy: match kind {
            TruncationKind::QueryBudget => "stable_evidence_order",
            TruncationKind::PackBudget => "plan_order",
            _ => "head_tail_line_aligned",
        }
        .into(),
        omitted_source_spans: vec![],
        affected_evidence_ids: vec![],
    });
    assign_evidence_id(evidence);
    true
}

pub(crate) fn deduplicate(records: Vec<EvidenceRecord>) -> Vec<EvidenceRecord> {
    let mut by_projection: BTreeMap<Vec<u8>, EvidenceRecord> = BTreeMap::new();
    for mut record in records {
        let key = serde_json::to_vec(&evidence_projection(&record)).unwrap();
        if let Some(existing) = by_projection.get_mut(&key) {
            existing.requested_by.append(&mut record.requested_by);
            existing.requested_by.sort();
            existing.requested_by.dedup();
            for diagnostic in record.diagnostics {
                if !existing.diagnostics.contains(&diagnostic) {
                    existing.diagnostics.push(diagnostic);
                }
            }
        } else {
            assign_evidence_id(&mut record);
            by_projection.insert(key, record);
        }
    }
    by_projection.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_limit_is_head_tail_without_synthetic_text() {
        let record = source_evidence(
            "q",
            "a.cpp",
            10,
            "a\nb\nc\nd\ne\n",
            Resolution::Exact,
            "filesystem",
            3,
            100,
        );
        assert_eq!(record.fragments.len(), 2);
        assert_eq!(record.fragments[0].content, "a\nb\n");
        assert_eq!(record.fragments[1].content, "e\n");
        assert_eq!(record.truncations[0].omitted, 2);
    }

    #[test]
    fn byte_limit_keeps_complete_outside_lines() {
        let record = source_evidence(
            "q",
            "a.cpp",
            1,
            "111\n222\n333\n444\n",
            Resolution::Exact,
            "filesystem",
            10,
            8,
        );
        assert!(record.payload_bytes() <= 8);
        assert!(record.fragments.iter().all(|f| f.content.ends_with('\n')));
    }
}
