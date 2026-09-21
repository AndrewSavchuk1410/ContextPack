//! Deterministic Markdown renderer for ContextPack V1.

use std::fmt::Write;

use contextpack_core::{
    CollectionResult, Diagnostic, EvidenceKind, EvidenceOrigin, EvidenceRecord, Metrics,
    QueryResult, Truncation,
};
use serde_json::Value;

pub fn render_markdown(result: &CollectionResult) -> String {
    let mut out = String::new();
    writeln!(out, "# ContextPack\n").unwrap();
    render_manifest(&mut out, result);
    render_summary(&mut out, result);
    writeln!(out, "## Queries\n").unwrap();
    for query in &result.query_results {
        render_query(&mut out, result, query);
    }
    render_evidence(&mut out, result);
    render_diagnostics(&mut out, &result.diagnostics);
    render_truncation_summary(&mut out, result);
    out
}

fn render_manifest(out: &mut String, result: &CollectionResult) {
    writeln!(out, "## Manifest\n").unwrap();
    writeln!(out, "- Format: `{}`", result.contract_version).unwrap();
    writeln!(out, "- Pack ID: `{}`", result.pack_id).unwrap();
    writeln!(
        out,
        "- Parent Pack: {}",
        result
            .parent_pack
            .as_deref()
            .map(|id| format!("`{id}`"))
            .unwrap_or_else(|| "None.".into())
    )
    .unwrap();
    writeln!(out, "- ContextPack Version: `{}`", result.tool.version).unwrap();
    writeln!(out, "- Repository: `.`").unwrap();
    writeln!(out, "- Providers:").unwrap();
    for provider in &result.providers {
        let extra = match (&provider.tree_sitter_version, &provider.grammar_version) {
            (Some(ts), Some(grammar)) => format!("; Tree-sitter `{ts}`; grammar `{grammar}`"),
            _ => String::new(),
        };
        writeln!(
            out,
            "  - `{}` — {} `{}`{}",
            provider.id, provider.implementation, provider.version, extra
        )
        .unwrap();
    }
    writeln!(out).unwrap();
}

fn render_summary(out: &mut String, result: &CollectionResult) {
    writeln!(out, "## Collection Summary\n").unwrap();
    writeln!(
        out,
        "| Query | Type | Status | Evidence | Candidates | Diagnostics |"
    )
    .unwrap();
    writeln!(out, "|---|---|---|---:|---:|---:|").unwrap();
    for query in &result.query_results {
        writeln!(
            out,
            "| `{}` | {} | {} | {} | {} | {} |",
            md_cell(&query.query_id),
            enum_text(&query.query_type),
            enum_text(&query.status),
            query.evidence_ids.len(),
            query.candidates.len(),
            query.diagnostics.len()
        )
        .unwrap();
    }
    writeln!(out).unwrap();
}

fn render_query(out: &mut String, result: &CollectionResult, query: &QueryResult) {
    writeln!(out, "### Query `{}`\n", query.query_id).unwrap();
    writeln!(out, "#### Request\n").unwrap();
    if let Some(request) = find_request(result, &query.query_id) {
        render_request(out, request);
    } else {
        writeln!(out, "None.").unwrap();
    }
    writeln!(out).unwrap();
    writeln!(out, "#### Result\n").unwrap();
    writeln!(out, "- Status: `{}`", enum_text(&query.status)).unwrap();
    render_metrics(out, &query.metrics);
    writeln!(out).unwrap();
    writeln!(out, "#### Candidates\n").unwrap();
    if query.candidates.is_empty() {
        writeln!(out, "None.\n").unwrap();
    } else {
        writeln!(
            out,
            "| Candidate | Kind | Role | Match | Qualified name | Signature | Location | Parse |"
        )
        .unwrap();
        writeln!(out, "|---|---|---|---|---|---|---|---|").unwrap();
        for candidate in &query.candidates {
            writeln!(
                out,
                "| `{}` | {} | {} | {} | `{}` | `{}` | `{}:{}:{}` | {} |",
                candidate.candidate_id,
                enum_text(&candidate.kind),
                enum_text(&candidate.role),
                enum_text(&candidate.match_kind),
                md_cell(&candidate.name.qualified),
                md_cell(candidate.signature_text.as_deref().unwrap_or("")),
                md_cell(&candidate.name_location.path),
                candidate.name_location.line,
                candidate.name_location.column,
                enum_text(&candidate.parse_quality),
            )
            .unwrap();
        }
        writeln!(out).unwrap();
    }
    writeln!(out, "#### Evidence References\n").unwrap();
    if query.evidence_ids.is_empty() {
        writeln!(out, "None.\n").unwrap();
    } else {
        for id in &query.evidence_ids {
            writeln!(out, "- `{id}`").unwrap();
        }
        writeln!(out).unwrap();
    }
    writeln!(out, "#### Query Diagnostics\n").unwrap();
    render_diagnostic_list(out, &query.diagnostics);
    writeln!(out).unwrap();
    writeln!(out, "#### Query Truncation\n").unwrap();
    render_truncation_list(out, &query.truncations);
    writeln!(out).unwrap();
}

fn find_request<'a>(result: &'a CollectionResult, id: &str) -> Option<&'a Value> {
    result
        .normalized_plan
        .get("collect")?
        .as_array()?
        .iter()
        .find(|query| query.get("id").and_then(Value::as_str) == Some(id))
}

fn render_request(out: &mut String, request: &Value) {
    let Some(object) = request.as_object() else {
        writeln!(out, "None.").unwrap();
        return;
    };
    let preferred = [
        "type",
        "operation",
        "query",
        "mode",
        "name",
        "language",
        "roles",
        "kinds",
        "path",
        "paths",
        "target",
        "base",
        "head",
        "revision",
        "left",
        "right",
        "start_line",
        "end_line",
        "extensions",
        "fallback",
        "case_sensitive",
        "context",
        "max_entries",
        "limits",
        "exclude",
    ];
    for key in preferred {
        if let Some(value) = object.get(key)
            && !value.is_null()
            && !is_empty(value)
        {
            writeln!(out, "- {}: {}", title(key), inline_json(value)).unwrap();
        }
    }
}

fn render_metrics(out: &mut String, metrics: &Metrics) {
    let Value::Object(values) = serde_json::to_value(metrics).unwrap() else {
        return;
    };
    for (key, value) in values {
        if key != "kind" {
            writeln!(out, "- {}: `{}`", title(&key), value).unwrap();
        }
    }
}

fn render_evidence(out: &mut String, result: &CollectionResult) {
    writeln!(out, "## Evidence\n").unwrap();
    if result.evidence.is_empty() {
        writeln!(out, "None.\n").unwrap();
        return;
    }
    for record in &result.evidence {
        render_record(out, record);
    }
}

fn render_record(out: &mut String, record: &EvidenceRecord) {
    writeln!(out, "### Evidence `{}`\n", record.evidence_id).unwrap();
    writeln!(
        out,
        "- Requested by: {}",
        record
            .requested_by
            .iter()
            .map(|id| format!("`{id}`"))
            .collect::<Vec<_>>()
            .join(", ")
    )
    .unwrap();
    writeln!(out, "- Kind: `{}`", enum_text(&record.kind)).unwrap();
    writeln!(out, "- Resolution: `{}`", enum_text(&record.resolution)).unwrap();
    writeln!(out, "- Provider: `{}`", record.provenance.provider_id).unwrap();
    match &record.origin {
        EvidenceOrigin::Source { path, full_extent } => {
            writeln!(
                out,
                "- Full source extent: `{}:{}-{}`",
                path, full_extent.start_line, full_extent.end_line
            )
            .unwrap();
        }
        EvidenceOrigin::Git {
            operation,
            target,
            base,
            head,
            paths,
        } => {
            writeln!(out, "- Origin: Git `{operation}`").unwrap();
            if let Some(target) = target {
                writeln!(out, "- Target: `{target}`").unwrap();
            }
            if let Some(base) = base {
                writeln!(out, "- Base: `{}`", md_cell(base)).unwrap();
            }
            if let Some(head) = head {
                writeln!(out, "- Head: `{}`", md_cell(head)).unwrap();
            }
            if !paths.is_empty() {
                writeln!(out, "- Paths: `{}`", md_cell(&paths.join("`, `"))).unwrap();
            }
        }
    }
    writeln!(out).unwrap();
    if record.fragments.is_empty() {
        writeln!(out, "No payload retained.\n").unwrap();
    }
    for (index, fragment) in record.fragments.iter().enumerate() {
        if record.fragments.len() > 1 {
            match &fragment.span {
                Some(span) => writeln!(
                    out,
                    "#### Fragment {} — `{}:{}-{}`\n",
                    index + 1,
                    span.path,
                    span.start_line,
                    span.end_line
                )
                .unwrap(),
                None => writeln!(out, "#### Fragment {}\n", index + 1).unwrap(),
            }
        }
        write_fenced(out, fence_language(record), &fragment.content);
        writeln!(out).unwrap();
    }
    for truncation in &record.truncations {
        for span in &truncation.omitted_source_spans {
            writeln!(
                out,
                "Omitted: `{}:{}-{}` — {} lines (`{}`).",
                span.path,
                span.start_line,
                span.end_line,
                span.end_line.saturating_sub(span.start_line) + 1,
                enum_text(&truncation.kind),
            )
            .unwrap();
        }
        if truncation.omitted_source_spans.is_empty() {
            writeln!(
                out,
                "Omitted: {} {} (`{}`).",
                truncation.omitted,
                truncation.unit,
                enum_text(&truncation.kind)
            )
            .unwrap();
        }
    }
    if !record.truncations.is_empty() {
        writeln!(out).unwrap();
    }
}

fn fence_language(record: &EvidenceRecord) -> &'static str {
    match (&record.kind, &record.origin) {
        (EvidenceKind::Source, EvidenceOrigin::Source { path, .. })
            if [
                ".cpp", ".cc", ".cxx", ".c++", ".h", ".hpp", ".hh", ".hxx", ".inl", ".ipp", ".tpp",
            ]
            .iter()
            .any(|ext| path.to_ascii_lowercase().ends_with(ext)) =>
        {
            "cpp"
        }
        (EvidenceKind::Git, EvidenceOrigin::Git { operation, .. })
            if matches!(operation.as_str(), "diff" | "show") =>
        {
            "diff"
        }
        _ => "text",
    }
}

fn write_fenced(out: &mut String, language: &str, content: &str) {
    let longest = content
        .split(|ch| ch != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat((longest + 1).max(3));
    writeln!(out, "{fence}{language}").unwrap();
    write!(out, "{content}").unwrap();
    if !content.ends_with('\n') {
        writeln!(out).unwrap();
    }
    writeln!(out, "{fence}").unwrap();
}

fn render_diagnostics(out: &mut String, diagnostics: &[Diagnostic]) {
    writeln!(out, "## Diagnostics\n").unwrap();
    if diagnostics.is_empty() {
        writeln!(out, "None.\n").unwrap();
        return;
    }
    writeln!(out, "| Severity | Code | Query | Message |").unwrap();
    writeln!(out, "|---|---|---|---|").unwrap();
    for diagnostic in diagnostics {
        writeln!(
            out,
            "| {} | `{}` | {} | {} |",
            enum_text(&diagnostic.severity),
            diagnostic.code,
            diagnostic
                .query_id
                .as_deref()
                .map(|id| format!("`{}`", md_cell(id)))
                .unwrap_or_else(|| "—".into()),
            md_cell(&diagnostic.message)
        )
        .unwrap();
    }
    writeln!(out).unwrap();
}

fn render_diagnostic_list(out: &mut String, diagnostics: &[Diagnostic]) {
    if diagnostics.is_empty() {
        writeln!(out, "None.").unwrap();
        return;
    }
    for diagnostic in diagnostics {
        writeln!(
            out,
            "- [{}] `{}` — {}",
            enum_text(&diagnostic.severity),
            diagnostic.code,
            diagnostic.message
        )
        .unwrap();
    }
}

fn render_truncation_list(out: &mut String, truncations: &[Truncation]) {
    if truncations.is_empty() {
        writeln!(out, "None.").unwrap();
        return;
    }
    for truncation in truncations {
        writeln!(
            out,
            "- `{}`: total {}, included {}, omitted {} {} (`{}`).",
            enum_text(&truncation.kind),
            truncation.total,
            truncation.included,
            truncation.omitted,
            truncation.unit,
            truncation.strategy
        )
        .unwrap();
    }
}

fn render_truncation_summary(out: &mut String, result: &CollectionResult) {
    writeln!(out, "## Truncation Summary\n").unwrap();
    let mut rows: Vec<(String, &Truncation)> = Vec::new();
    for query in &result.query_results {
        for truncation in &query.truncations {
            rows.push((format!("query `{}`", query.query_id), truncation));
        }
    }
    for record in &result.evidence {
        for truncation in &record.truncations {
            rows.push((format!("evidence `{}`", record.evidence_id), truncation));
        }
    }
    for truncation in &result.pack_truncations {
        rows.push(("pack".into(), truncation));
    }
    if rows.is_empty() {
        writeln!(out, "None.\n").unwrap();
        return;
    }
    writeln!(out, "| Scope | Kind | Total | Included | Omitted |").unwrap();
    writeln!(out, "|---|---|---:|---:|---:|").unwrap();
    for (scope, truncation) in rows {
        writeln!(
            out,
            "| {} | `{}` | {} {} | {} | {} |",
            scope,
            enum_text(&truncation.kind),
            truncation.total,
            truncation.unit,
            truncation.included,
            truncation.omitted
        )
        .unwrap();
    }
    writeln!(out).unwrap();
}

fn enum_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn inline_json(value: &Value) -> String {
    match value {
        Value::String(value) => format!("`{}`", md_cell(value)),
        Value::Array(values) => values
            .iter()
            .map(inline_json)
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(_) => format!("`{}`", md_cell(&serde_json::to_string(value).unwrap())),
        _ => format!("`{value}`"),
    }
}

fn is_empty(value: &Value) -> bool {
    matches!(value, Value::Array(values) if values.is_empty())
        || matches!(value, Value::Object(values) if values.is_empty())
}

fn title(value: &str) -> String {
    let mut result = value.replace('_', " ");
    if let Some(first) = result.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    result
}

fn md_cell(value: &str) -> String {
    value.replace('|', "\\|").replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use contextpack_core::{collect_plan, parse_plan};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn renders_fixed_sections_without_absolute_repository() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.cpp"), "int main() {}\n").unwrap();
        let plan = parse_plan(
            "version: 1\nrepository: .\ncollect:\n  - id: source\n    type: file\n    path: a.cpp\n",
            dir.path().join("plan.yaml"),
        )
        .unwrap();
        let markdown = render_markdown(&collect_plan(&plan));
        for heading in [
            "# ContextPack",
            "## Manifest",
            "## Collection Summary",
            "## Queries",
            "## Evidence",
            "## Diagnostics",
            "## Truncation Summary",
        ] {
            assert!(markdown.contains(heading));
        }
        assert!(!markdown.contains(&dir.path().display().to_string()));
        assert!(markdown.contains("```cpp\nint main() {}\n```"));
    }

    #[test]
    fn chooses_a_fence_longer_than_evidence_backticks() {
        let mut output = String::new();
        write_fenced(&mut output, "text", "a ``` b");
        assert!(output.starts_with("````text\n"));
    }
}
