//! Core domain model and read-only repository collectors for ContextPack V1.

mod collect;
mod evidence;
mod git;
mod ids;
mod path;
mod plan;
mod search;
mod symbol;

pub use collect::{Collector, collect_plan};
pub use ids::{canonical_json_bytes, content_id};
pub use plan::{
    CollectionPlan, ContextSpec, EvidenceLimits, FileQuery, GitOperation, GitQuery, GitTarget,
    Limits, PlanError, Query, QueryLimits, RangeQuery, SearchLimits, SearchMode, SearchQuery,
    SymbolFallback, SymbolLanguage, SymbolLimits, SymbolQuery, Traversal, load_plan, parse_plan,
};

use serde::{Deserialize, Serialize};

pub const CONTRACT_VERSION: u32 = 1;
pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueryType {
    Search,
    Range,
    File,
    Git,
    Symbol,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Empty,
    Partial,
    Ambiguous,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl Diagnostic {
    pub fn query(code: &str, severity: Severity, message: impl Into<String>, id: &str) -> Self {
        Self {
            code: code.into(),
            severity,
            message: message.into(),
            query_id: Some(id.into()),
            evidence_id: None,
            candidate_id: None,
            path: None,
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceSpan {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NameLocation {
    pub path: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Source,
    Git,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    Exact,
    Syntactic,
    Textual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Provenance {
    pub provider_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvidenceOrigin {
    Source {
        path: String,
        full_extent: SourceExtent,
    },
    Git {
        operation: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        base: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        head: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        paths: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceExtent {
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceFragment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<SourceSpan>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TruncationKind {
    MatchLimit,
    CandidateLimit,
    LineLimit,
    ByteLimit,
    QueryBudget,
    PackBudget,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Truncation {
    pub kind: TruncationKind,
    pub unit: String,
    pub limit: usize,
    pub total: usize,
    pub included: usize,
    pub omitted: usize,
    pub strategy: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub omitted_source_spans: Vec<SourceSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceRecord {
    pub evidence_id: String,
    pub kind: EvidenceKind,
    pub requested_by: Vec<String>,
    pub resolution: Resolution,
    pub provenance: Provenance,
    pub origin: EvidenceOrigin,
    pub fragments: Vec<EvidenceFragment>,
    pub truncations: Vec<Truncation>,
    pub diagnostics: Vec<Diagnostic>,
}

impl EvidenceRecord {
    pub fn payload_bytes(&self) -> usize {
        self.fragments.iter().map(|f| f.content.len()).sum()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Constructor,
    Destructor,
    Operator,
    Class,
    Struct,
    Enum,
    Namespace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SymbolRole {
    Definition,
    Declaration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    ExactQualified,
    SuffixQualified,
    TemplateElided,
    Unqualified,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredName {
    pub leaf: String,
    pub spelled: String,
    pub qualified: String,
    pub match_aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    Namespace,
    Class,
    Struct,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScopeEntry {
    pub kind: ScopeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub anonymous: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TemplateInfo {
    pub is_template: bool,
    pub layers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParseQuality {
    Clean,
    Recovered,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolCandidate {
    pub candidate_id: String,
    pub language: String,
    pub kind: SymbolKind,
    pub role: SymbolRole,
    pub name: StructuredName,
    pub lexical_scope: Vec<ScopeEntry>,
    pub match_kind: MatchKind,
    pub name_location: NameLocation,
    pub source_extent: SourceSpan,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_location: Option<SourceSpan>,
    pub template: TemplateInfo,
    pub modifiers: Vec<String>,
    pub parse_quality: ParseQuality,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Metrics {
    Search {
        matches_found: usize,
        matches_included: usize,
        matches_omitted: usize,
        files_with_matches: usize,
    },
    Symbol {
        candidates_found: usize,
        candidates_included: usize,
        candidates_omitted: usize,
        textual_fallback_matches: usize,
    },
    Source {
        source_lines: usize,
        included_lines: usize,
        source_bytes: usize,
        included_bytes: usize,
    },
    Git {
        output_bytes: usize,
        included_bytes: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueryResult {
    pub query_id: String,
    pub query_type: QueryType,
    pub status: Status,
    pub evidence_ids: Vec<String>,
    pub candidates: Vec<SymbolCandidate>,
    pub diagnostics: Vec<Diagnostic>,
    pub metrics: Metrics,
    pub truncations: Vec<Truncation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderInfo {
    pub id: String,
    pub kind: String,
    pub implementation: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree_sitter_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grammar_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CollectionResult {
    pub contract_version: u32,
    pub pack_id: String,
    pub parent_pack: Option<String>,
    pub tool: ToolInfo,
    pub providers: Vec<ProviderInfo>,
    pub normalized_plan: serde_json::Value,
    pub query_results: Vec<QueryResult>,
    pub evidence: Vec<EvidenceRecord>,
    pub diagnostics: Vec<Diagnostic>,
    pub pack_truncations: Vec<Truncation>,
}
