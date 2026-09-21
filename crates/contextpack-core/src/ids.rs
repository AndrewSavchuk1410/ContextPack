use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{CollectionResult, Diagnostic, EvidenceRecord, SymbolCandidate};

/// Serialize a typed canonical projection using deterministic JSON object ordering.
pub fn canonical_json_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    let value = serde_json::to_value(value).expect("canonical projection must serialize");
    serde_json::to_vec(&value).expect("canonical value must serialize")
}

pub fn content_id<T: Serialize>(prefix: &str, value: &T) -> String {
    let digest = Sha256::digest(canonical_json_bytes(value));
    format!("{prefix}{}", hex::encode(&digest[..16]))
}

pub(crate) fn candidate_projection(candidate: &SymbolCandidate) -> Value {
    json!({
        "schema": 1,
        "language": candidate.language,
        "kind": candidate.kind,
        "role": candidate.role,
        "name": {
            "leaf": candidate.name.leaf,
            "spelled": candidate.name.spelled,
            "qualified": candidate.name.qualified,
        },
        "name_location": candidate.name_location,
        "source_extent": candidate.source_extent,
        "signature_text": candidate.signature_text,
    })
}

pub(crate) fn assign_candidate_id(candidate: &mut SymbolCandidate) {
    candidate.candidate_id = content_id("c1-", &candidate_projection(candidate));
}

pub(crate) fn evidence_projection(evidence: &EvidenceRecord) -> Value {
    json!({
        "schema": 1,
        "kind": evidence.kind,
        "resolution": evidence.resolution,
        "provenance": evidence.provenance,
        "origin": evidence.origin,
        "fragments": evidence.fragments,
        "truncations": evidence.truncations,
    })
}

pub(crate) fn assign_evidence_id(evidence: &mut EvidenceRecord) {
    evidence.evidence_id = content_id("e1-", &evidence_projection(evidence));
}

fn diagnostic_projection(diagnostic: &Diagnostic) -> Value {
    json!({
        "code": diagnostic.code,
        "severity": diagnostic.severity,
        "query_id": diagnostic.query_id,
        "evidence_id": diagnostic.evidence_id,
        "candidate_id": diagnostic.candidate_id,
        "path": diagnostic.path,
    })
}

pub(crate) fn assign_pack_id(result: &mut CollectionResult) {
    let query_results = result
        .query_results
        .iter()
        .map(|query| {
            let mut value = serde_json::to_value(query).expect("query result serializes");
            value["diagnostics"] = Value::Array(
                query
                    .diagnostics
                    .iter()
                    .map(diagnostic_projection)
                    .collect(),
            );
            value
        })
        .collect::<Vec<_>>();
    let evidence = result
        .evidence
        .iter()
        .map(|record| {
            let mut value = serde_json::to_value(record).expect("evidence serializes");
            value["diagnostics"] = Value::Array(
                record
                    .diagnostics
                    .iter()
                    .map(diagnostic_projection)
                    .collect(),
            );
            value
        })
        .collect::<Vec<_>>();
    let projection = json!({
        "contract_version": result.contract_version,
        "tool": result.tool,
        "providers": result.providers,
        "parent_pack": result.parent_pack,
        "normalized_plan": result.normalized_plan,
        "query_results": query_results,
        "evidence": evidence,
        "diagnostics": result.diagnostics.iter().map(diagnostic_projection).collect::<Vec<_>>(),
        "pack_truncations": result.pack_truncations,
    });
    result.pack_id = content_id("cp1-", &projection);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn stable_known_sha_prefix() {
        assert_eq!(
            content_id("x-", &json!({"a": 1, "b": 2})),
            "x-43258cff783fe7036d8a43033f830adf"
        );
    }
}
