use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use grep_regex::RegexMatcherBuilder;
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Diagnostic, Severity, SymbolKind, SymbolRole};

fn default_repository() -> String {
    ".".into()
}
fn default_true() -> bool {
    true
}
fn default_max_matches() -> usize {
    50
}
fn default_max_candidates() -> usize {
    50
}
fn default_max_evidence_lines() -> usize {
    400
}
fn default_max_evidence_bytes() -> usize {
    131_072
}
fn default_max_query_evidence_bytes() -> usize {
    524_288
}
fn default_max_pack_evidence_bytes() -> usize {
    2_097_152
}
fn default_paths() -> Vec<String> {
    vec![".".into()]
}
fn default_before_after() -> usize {
    3
}
fn default_roles() -> Vec<SymbolRole> {
    vec![SymbolRole::Definition]
}
fn default_kinds() -> Vec<SymbolKind> {
    vec![
        SymbolKind::Function,
        SymbolKind::Method,
        SymbolKind::Constructor,
        SymbolKind::Destructor,
        SymbolKind::Operator,
        SymbolKind::Class,
        SymbolKind::Struct,
        SymbolKind::Enum,
        SymbolKind::Namespace,
    ]
}
fn default_max_entries() -> usize {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectionPlan {
    pub version: u32,
    #[serde(default = "default_repository")]
    pub repository: String,
    #[serde(default)]
    pub parent_pack: Option<String>,
    #[serde(default)]
    pub traversal: Traversal,
    #[serde(default)]
    pub limits: Limits,
    pub collect: Vec<Query>,
    #[serde(skip)]
    pub(crate) source_dir: PathBuf,
    #[serde(skip)]
    pub(crate) resolved_repository: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Traversal {
    #[serde(default = "default_true")]
    pub respect_gitignore: bool,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default)]
    pub follow_symlinks: bool,
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl Default for Traversal {
    fn default() -> Self {
        Self {
            respect_gitignore: true,
            include_hidden: false,
            follow_symlinks: false,
            exclude: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    #[serde(default = "default_max_matches")]
    pub max_matches: usize,
    #[serde(default = "default_max_candidates")]
    pub max_candidates: usize,
    #[serde(default = "default_max_evidence_lines")]
    pub max_evidence_lines: usize,
    #[serde(default = "default_max_evidence_bytes")]
    pub max_evidence_bytes: usize,
    #[serde(default = "default_max_query_evidence_bytes")]
    pub max_query_evidence_bytes: usize,
    #[serde(default = "default_max_pack_evidence_bytes")]
    pub max_pack_evidence_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_matches: default_max_matches(),
            max_candidates: default_max_candidates(),
            max_evidence_lines: default_max_evidence_lines(),
            max_evidence_bytes: default_max_evidence_bytes(),
            max_query_evidence_bytes: default_max_query_evidence_bytes(),
            max_pack_evidence_bytes: default_max_pack_evidence_bytes(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EvidenceLimits {
    pub max_evidence_lines: Option<usize>,
    pub max_evidence_bytes: Option<usize>,
    pub max_query_evidence_bytes: Option<usize>,
}

pub type QueryLimits = EvidenceLimits;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SearchLimits {
    pub max_matches: Option<usize>,
    pub max_evidence_lines: Option<usize>,
    pub max_evidence_bytes: Option<usize>,
    pub max_query_evidence_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SymbolLimits {
    pub max_candidates: Option<usize>,
    pub max_evidence_lines: Option<usize>,
    pub max_evidence_bytes: Option<usize>,
    pub max_query_evidence_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Query {
    Search(SearchQuery),
    Range(RangeQuery),
    File(FileQuery),
    Git(GitQuery),
    Symbol(SymbolQuery),
}

impl Query {
    pub fn id(&self) -> &str {
        match self {
            Self::Search(q) => &q.id,
            Self::Range(q) => &q.id,
            Self::File(q) => &q.id,
            Self::Git(q) => &q.id,
            Self::Symbol(q) => &q.id,
        }
    }

    pub fn query_budget(&self, global: &Limits) -> usize {
        match self {
            Self::Search(q) => q.limits.max_query_evidence_bytes,
            Self::Range(q) => q.limits.max_query_evidence_bytes,
            Self::File(q) => q.limits.max_query_evidence_bytes,
            Self::Git(q) => q.limits.max_query_evidence_bytes,
            Self::Symbol(q) => q.limits.max_query_evidence_bytes,
        }
        .unwrap_or(global.max_query_evidence_bytes)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchQuery {
    pub id: String,
    pub query: String,
    #[serde(default)]
    pub mode: SearchMode,
    #[serde(default = "default_paths")]
    pub paths: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default = "default_true")]
    pub case_sensitive: bool,
    #[serde(default)]
    pub context: ContextSpec,
    #[serde(default)]
    pub limits: SearchLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    #[default]
    Literal,
    Regex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextSpec {
    #[serde(default = "default_before_after")]
    pub before: usize,
    #[serde(default = "default_before_after")]
    pub after: usize,
}

impl Default for ContextSpec {
    fn default() -> Self {
        Self {
            before: 3,
            after: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeQuery {
    pub id: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    #[serde(default)]
    pub limits: EvidenceLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileQuery {
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub limits: EvidenceLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolQuery {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub language: SymbolLanguage,
    #[serde(default = "default_roles")]
    pub roles: Vec<SymbolRole>,
    #[serde(default = "default_kinds")]
    pub kinds: Vec<SymbolKind>,
    #[serde(default = "default_paths")]
    pub paths: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub fallback: SymbolFallback,
    #[serde(default)]
    pub limits: SymbolLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SymbolLanguage {
    #[default]
    Auto,
    Cpp,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SymbolFallback {
    #[default]
    Textual,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitQuery {
    pub id: String,
    pub operation: GitOperation,
    #[serde(default)]
    pub target: Option<GitTarget>,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_entries: Option<usize>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub start_line: Option<usize>,
    #[serde(default)]
    pub end_line: Option<usize>,
    #[serde(default)]
    pub left: Option<String>,
    #[serde(default)]
    pub right: Option<String>,
    #[serde(default)]
    pub limits: EvidenceLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitOperation {
    Status,
    Branch,
    Log,
    Diff,
    DiffStat,
    Show,
    Blame,
    MergeBase,
    ChangedFiles,
    DiffCheck,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitTarget {
    WorkingTree,
    Staged,
    Baseline,
    Revisions,
}

#[derive(Debug, Error)]
#[error("collection plan validation failed")]
pub struct PlanError {
    pub diagnostics: Vec<Diagnostic>,
}

impl PlanError {
    fn one(code: &str, message: impl Into<String>) -> Self {
        Self {
            diagnostics: vec![Diagnostic {
                code: code.into(),
                severity: Severity::Error,
                message: message.into(),
                query_id: None,
                evidence_id: None,
                candidate_id: None,
                path: None,
            }],
        }
    }
}

pub fn load_plan(path: impl AsRef<Path>) -> Result<CollectionPlan, PlanError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|e| {
        PlanError::one(
            "malformed_plan",
            format!("could not read plan {}: {e}", path.display()),
        )
    })?;
    parse_plan(&text, path)
}

pub fn parse_plan(text: &str, plan_path: impl AsRef<Path>) -> Result<CollectionPlan, PlanError> {
    let mut plan: CollectionPlan = serde_yaml::from_str(text).map_err(|e| {
        let msg = e.to_string();
        let code = if msg.contains("unknown field") {
            "unknown_field"
        } else {
            "malformed_plan"
        };
        PlanError::one(code, msg)
    })?;
    plan.source_dir = plan_path
        .as_ref()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    validate_and_normalize(&mut plan)?;
    Ok(plan)
}

fn validate_and_normalize(plan: &mut CollectionPlan) -> Result<(), PlanError> {
    let mut errors = Vec::new();
    if plan.version != 1 {
        errors.push(plan_diag(
            "unsupported_plan_version",
            format!("plan version must be 1, got {}", plan.version),
        ));
    }
    if plan.collect.is_empty() {
        errors.push(plan_diag(
            "malformed_plan",
            "collect must contain at least one query",
        ));
    }
    validate_positive_limits(&plan.limits, &mut errors);
    if let Some(parent) = &plan.parent_pack
        && !Regex::new(r"^cp1-[0-9a-f]{32}$").unwrap().is_match(parent)
    {
        errors.push(plan_diag(
            "invalid_query_combination",
            "parent_pack must be a cp1- ID",
        ));
    }
    let id_re = Regex::new(r"^[A-Za-z][A-Za-z0-9._-]{0,63}$").unwrap();
    let mut ids = HashSet::new();
    for query in &mut plan.collect {
        let id = query.id().to_string();
        if !id_re.is_match(&id) {
            errors.push(query_diag(
                "invalid_query_id",
                format!("invalid query id `{id}`"),
                &id,
            ));
        } else if !ids.insert(id.clone()) {
            errors.push(query_diag(
                "duplicate_query_id",
                format!("duplicate query id `{id}`"),
                &id,
            ));
        }
        validate_query(query, &mut errors);
    }

    let repo = plan.source_dir.join(&plan.repository);
    match repo.canonicalize() {
        Ok(path) if path.is_dir() => plan.resolved_repository = path,
        Ok(_) => errors.push(plan_diag(
            "repository_unavailable",
            format!("repository is not a directory: {}", repo.display()),
        )),
        Err(e) => errors.push(plan_diag(
            "repository_unavailable",
            format!("repository is unavailable: {}: {e}", repo.display()),
        )),
    }
    if !errors.is_empty() {
        return Err(PlanError {
            diagnostics: errors,
        });
    }

    plan.repository = plan.repository.replace('\\', "/");
    normalize_plan(plan);
    Ok(())
}

fn validate_query(query: &Query, errors: &mut Vec<Diagnostic>) {
    let id = query.id();
    match query {
        Query::Search(q) => {
            if q.query.is_empty() || q.query.contains(['\r', '\n']) {
                errors.push(query_diag(
                    "invalid_query_combination",
                    "search query must be non-empty and single-line",
                    id,
                ));
            }
            if matches!(q.mode, SearchMode::Regex)
                && RegexMatcherBuilder::new().build(&q.query).is_err()
            {
                errors.push(query_diag("invalid_regex", "invalid search regex", id));
            }
            validate_paths(&q.paths, id, errors);
            validate_extensions(&q.extensions, id, errors);
            validate_optional_limits(
                [
                    q.limits.max_matches,
                    q.limits.max_evidence_lines,
                    q.limits.max_evidence_bytes,
                    q.limits.max_query_evidence_bytes,
                ],
                id,
                errors,
            );
        }
        Query::Range(q) => {
            validate_one_path(&q.path, id, errors);
            if q.start_line == 0 || q.end_line == 0 || q.end_line < q.start_line {
                errors.push(query_diag(
                    "invalid_query_combination",
                    "range lines must be >= 1 and end_line >= start_line",
                    id,
                ));
            }
            validate_evidence_limits(&q.limits, id, errors);
        }
        Query::File(q) => {
            validate_one_path(&q.path, id, errors);
            validate_evidence_limits(&q.limits, id, errors);
        }
        Query::Symbol(q) => {
            if q.name.is_empty() || q.name.contains(['\r', '\n']) {
                errors.push(query_diag(
                    "invalid_query_combination",
                    "symbol name must be non-empty and single-line",
                    id,
                ));
            }
            if q.roles.is_empty() || q.kinds.is_empty() {
                errors.push(query_diag(
                    "invalid_query_combination",
                    "symbol roles and kinds may not be empty",
                    id,
                ));
            }
            validate_paths(&q.paths, id, errors);
            validate_extensions(&q.extensions, id, errors);
            validate_optional_limits(
                [
                    q.limits.max_candidates,
                    q.limits.max_evidence_lines,
                    q.limits.max_evidence_bytes,
                    q.limits.max_query_evidence_bytes,
                ],
                id,
                errors,
            );
        }
        Query::Git(q) => validate_git(q, errors),
    }
}

fn validate_git(q: &GitQuery, errors: &mut Vec<Diagnostic>) {
    let id = &q.id;
    validate_paths(&q.paths, id, errors);
    if let Some(path) = &q.path {
        validate_one_path(path, id, errors);
    }
    validate_evidence_limits(&q.limits, id, errors);
    if q.max_entries == Some(0) {
        errors.push(query_diag(
            "invalid_limit",
            "max_entries must be positive",
            id,
        ));
    }
    for revision in [
        q.base.as_deref(),
        q.head.as_deref(),
        q.revision.as_deref(),
        q.left.as_deref(),
        q.right.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if revision.starts_with('-') || revision.contains(['\r', '\n', '\0']) {
            errors.push(query_diag(
                "invalid_query_combination",
                "Git revisions may not begin with '-' or contain control line breaks",
                id,
            ));
        }
    }
    let invalid = |message: &str, errors: &mut Vec<Diagnostic>| {
        errors.push(query_diag("invalid_query_combination", message, id));
    };
    match q.operation {
        GitOperation::Status | GitOperation::Branch => {
            if q.target.is_some()
                || q.base.is_some()
                || q.head.is_some()
                || !q.paths.is_empty()
                || q.path.is_some()
                || q.left.is_some()
                || q.right.is_some()
                || q.start_line.is_some()
                || q.end_line.is_some()
                || q.revision.is_some()
                || q.max_entries.is_some()
            {
                invalid("status/branch accept no operation fields", errors);
            }
        }
        GitOperation::Log => {
            if q.target.is_some()
                || q.base.is_some()
                || q.head.is_some()
                || q.path.is_some()
                || !q.paths.is_empty()
                || q.left.is_some()
                || q.right.is_some()
                || q.start_line.is_some()
                || q.end_line.is_some()
            {
                invalid("log accepts only revision and max_entries", errors);
            }
        }
        GitOperation::Diff
        | GitOperation::DiffStat
        | GitOperation::ChangedFiles
        | GitOperation::DiffCheck => {
            if q.revision.is_some()
                || q.max_entries.is_some()
                || q.path.is_some()
                || q.start_line.is_some()
                || q.end_line.is_some()
                || q.left.is_some()
                || q.right.is_some()
            {
                invalid("diff-family operation contains unrelated fields", errors);
            }
            match q.target {
                Some(GitTarget::WorkingTree | GitTarget::Staged) => {
                    if q.base.is_some() || q.head.is_some() {
                        invalid("working_tree/staged forbid base and head", errors);
                    }
                }
                Some(GitTarget::Baseline) => {
                    if q.base.is_none() || q.head.is_some() {
                        invalid("baseline requires base and forbids head", errors);
                    }
                }
                Some(GitTarget::Revisions) => {
                    if q.base.is_none() {
                        invalid("revisions requires base", errors);
                    }
                }
                None => invalid("diff-family operations require target", errors),
            }
        }
        GitOperation::Show => {
            if q.revision.is_none() {
                invalid("show requires revision", errors);
            }
            if q.target.is_some()
                || q.base.is_some()
                || q.head.is_some()
                || q.path.is_some()
                || q.start_line.is_some()
                || q.end_line.is_some()
                || q.left.is_some()
                || q.right.is_some()
                || q.max_entries.is_some()
            {
                invalid("show contains unrelated fields", errors);
            }
        }
        GitOperation::Blame => {
            if q.path.is_none() || q.start_line.is_some() != q.end_line.is_some() {
                invalid(
                    "blame requires path and either both line bounds or neither",
                    errors,
                );
            }
            if matches!(q.start_line, Some(0))
                || matches!(q.end_line, Some(0))
                || q.start_line.zip(q.end_line).is_some_and(|(a, b)| b < a)
            {
                invalid("blame line bounds are invalid", errors);
            }
            if q.target.is_some()
                || q.base.is_some()
                || q.head.is_some()
                || !q.paths.is_empty()
                || q.left.is_some()
                || q.right.is_some()
                || q.max_entries.is_some()
            {
                invalid("blame contains unrelated fields", errors);
            }
        }
        GitOperation::MergeBase => {
            if q.right.is_none() {
                invalid("merge_base requires right", errors);
            }
            if q.target.is_some()
                || q.base.is_some()
                || q.head.is_some()
                || q.revision.is_some()
                || q.max_entries.is_some()
                || !q.paths.is_empty()
                || q.path.is_some()
                || q.start_line.is_some()
                || q.end_line.is_some()
            {
                invalid("merge_base contains unrelated fields", errors);
            }
        }
    }
}

fn validate_positive_limits(l: &Limits, errors: &mut Vec<Diagnostic>) {
    if [
        l.max_matches,
        l.max_candidates,
        l.max_evidence_lines,
        l.max_evidence_bytes,
        l.max_query_evidence_bytes,
        l.max_pack_evidence_bytes,
    ]
    .contains(&0)
    {
        errors.push(plan_diag("invalid_limit", "all limits must be positive"));
    }
}

fn validate_evidence_limits(l: &EvidenceLimits, id: &str, errors: &mut Vec<Diagnostic>) {
    validate_optional_limits(
        [
            l.max_evidence_lines,
            l.max_evidence_bytes,
            l.max_query_evidence_bytes,
        ],
        id,
        errors,
    );
}

fn validate_optional_limits<const N: usize>(
    values: [Option<usize>; N],
    id: &str,
    errors: &mut Vec<Diagnostic>,
) {
    if values.into_iter().flatten().any(|v| v == 0) {
        errors.push(query_diag("invalid_limit", "limits must be positive", id));
    }
}

fn validate_extensions(exts: &[String], id: &str, errors: &mut Vec<Diagnostic>) {
    for ext in exts {
        if !ext.starts_with('.') || ext.len() < 2 || ext.contains(['/', '\\']) {
            errors.push(query_diag(
                "invalid_query_combination",
                format!("invalid extension `{ext}`"),
                id,
            ));
        }
    }
}

fn validate_paths(paths: &[String], id: &str, errors: &mut Vec<Diagnostic>) {
    for path in paths {
        validate_one_path(path, id, errors);
    }
}

fn validate_one_path(path: &str, id: &str, errors: &mut Vec<Diagnostic>) {
    if let Err(message) = validate_relative(path, false) {
        errors.push(query_diag("path_outside_repository", message, id));
    }
}

fn validate_relative(value: &str, _repository_field: bool) -> Result<(), String> {
    if value.is_empty() {
        return Err("path may not be empty".into());
    }
    let value = value.replace('\\', "/");
    if value.starts_with('/') || value.starts_with("//") {
        return Err(format!("absolute path is forbidden: `{value}`"));
    }
    if value.len() >= 2 && value.as_bytes()[1] == b':' {
        return Err(format!(
            "drive-qualified query path is forbidden: `{value}`"
        ));
    }
    let mut depth = 0usize;
    for component in Path::new(&value).components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::ParentDir if depth == 0 => {
                return Err(format!("path escapes repository: `{value}`"));
            }
            Component::ParentDir => depth -= 1,
            Component::Prefix(_) | Component::RootDir => {
                return Err(format!("absolute path is forbidden: `{value}`"));
            }
            Component::CurDir => {}
        }
    }
    Ok(())
}

fn normalize_plan(plan: &mut CollectionPlan) {
    for query in &mut plan.collect {
        match query {
            Query::Search(q) => {
                normalize_paths(&mut q.paths);
                normalize_extensions(&mut q.extensions);
            }
            Query::Range(q) => q.path = normalize_path(&q.path),
            Query::File(q) => q.path = normalize_path(&q.path),
            Query::Git(q) => {
                normalize_paths(&mut q.paths);
                if let Some(path) = &mut q.path {
                    *path = normalize_path(path);
                }
                if matches!(q.operation, GitOperation::MergeBase) && q.left.is_none() {
                    q.left = Some("HEAD".into());
                }
                if matches!(q.operation, GitOperation::Log | GitOperation::Blame)
                    && q.revision.is_none()
                {
                    q.revision = Some("HEAD".into());
                }
                if matches!(q.operation, GitOperation::Log) && q.max_entries.is_none() {
                    q.max_entries = Some(default_max_entries());
                }
                if matches!(
                    q.operation,
                    GitOperation::Diff
                        | GitOperation::DiffStat
                        | GitOperation::ChangedFiles
                        | GitOperation::DiffCheck
                ) && matches!(q.target, Some(GitTarget::Revisions))
                    && q.head.is_none()
                {
                    q.head = Some("HEAD".into());
                }
            }
            Query::Symbol(q) => {
                normalize_paths(&mut q.paths);
                if q.extensions.is_empty() {
                    q.extensions = [
                        ".cpp", ".cc", ".cxx", ".c++", ".h", ".hpp", ".hh", ".hxx", ".inl", ".ipp",
                        ".tpp",
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
                }
                normalize_extensions(&mut q.extensions);
                q.roles.sort();
                q.roles.dedup();
                q.kinds.sort();
                q.kinds.dedup();
                q.name = normalize_symbol_name(&q.name);
            }
        }
    }
}

pub(crate) fn normalize_symbol_name(value: &str) -> String {
    value
        .split("::")
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("::")
}

fn normalize_paths(paths: &mut Vec<String>) {
    for path in paths.iter_mut() {
        *path = normalize_path(path);
    }
    paths.sort();
    paths.dedup();
}

fn normalize_extensions(exts: &mut Vec<String>) {
    for ext in exts.iter_mut() {
        *ext = ext.to_ascii_lowercase();
    }
    exts.sort();
    exts.dedup();
}

fn normalize_path(value: &str) -> String {
    let mut parts = Vec::new();
    let normalized = value.replace('\\', "/");
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

fn plan_diag(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        severity: Severity::Error,
        message: message.into(),
        query_id: None,
        evidence_id: None,
        candidate_id: None,
        path: None,
    }
}

fn query_diag(code: &str, message: impl Into<String>, id: &str) -> Diagnostic {
    Diagnostic::query(code, Severity::Error, message, id)
}

impl CollectionPlan {
    pub fn repository_root(&self) -> &Path {
        &self.resolved_repository
    }

    pub fn normalized_json(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).expect("serializable plan");
        value["repository"] = serde_json::Value::String(".".into());
        let global = &self.limits;
        if let Some(queries) = value
            .get_mut("collect")
            .and_then(serde_json::Value::as_array_mut)
        {
            for (index, query_value) in queries.iter_mut().enumerate() {
                let query = &self.collect[index];
                let Some(object) = query_value.as_object_mut() else {
                    continue;
                };
                let mut limits = serde_json::Map::new();
                let (max_lines, max_bytes, max_query) = match query {
                    Query::Search(q) => {
                        limits.insert(
                            "max_matches".into(),
                            q.limits.max_matches.unwrap_or(global.max_matches).into(),
                        );
                        (
                            q.limits
                                .max_evidence_lines
                                .unwrap_or(global.max_evidence_lines),
                            q.limits
                                .max_evidence_bytes
                                .unwrap_or(global.max_evidence_bytes),
                            q.limits
                                .max_query_evidence_bytes
                                .unwrap_or(global.max_query_evidence_bytes),
                        )
                    }
                    Query::Symbol(q) => {
                        limits.insert(
                            "max_candidates".into(),
                            q.limits
                                .max_candidates
                                .unwrap_or(global.max_candidates)
                                .into(),
                        );
                        (
                            q.limits
                                .max_evidence_lines
                                .unwrap_or(global.max_evidence_lines),
                            q.limits
                                .max_evidence_bytes
                                .unwrap_or(global.max_evidence_bytes),
                            q.limits
                                .max_query_evidence_bytes
                                .unwrap_or(global.max_query_evidence_bytes),
                        )
                    }
                    Query::Range(q) => (
                        q.limits
                            .max_evidence_lines
                            .unwrap_or(global.max_evidence_lines),
                        q.limits
                            .max_evidence_bytes
                            .unwrap_or(global.max_evidence_bytes),
                        q.limits
                            .max_query_evidence_bytes
                            .unwrap_or(global.max_query_evidence_bytes),
                    ),
                    Query::File(q) => (
                        q.limits
                            .max_evidence_lines
                            .unwrap_or(global.max_evidence_lines),
                        q.limits
                            .max_evidence_bytes
                            .unwrap_or(global.max_evidence_bytes),
                        q.limits
                            .max_query_evidence_bytes
                            .unwrap_or(global.max_query_evidence_bytes),
                    ),
                    Query::Git(q) => (
                        q.limits
                            .max_evidence_lines
                            .unwrap_or(global.max_evidence_lines),
                        q.limits
                            .max_evidence_bytes
                            .unwrap_or(global.max_evidence_bytes),
                        q.limits
                            .max_query_evidence_bytes
                            .unwrap_or(global.max_query_evidence_bytes),
                    ),
                };
                limits.insert("max_evidence_lines".into(), max_lines.into());
                limits.insert("max_evidence_bytes".into(), max_bytes.into());
                limits.insert("max_query_evidence_bytes".into(), max_query.into());
                object.insert("limits".into(), serde_json::Value::Object(limits));
                object.retain(|_, field| !field.is_null());
                if let Query::Git(git) = query {
                    let allowed: &[&str] = match git.operation {
                        GitOperation::Status | GitOperation::Branch => {
                            &["id", "type", "operation", "limits"]
                        }
                        GitOperation::Log => &[
                            "id",
                            "type",
                            "operation",
                            "revision",
                            "max_entries",
                            "limits",
                        ],
                        GitOperation::Diff
                        | GitOperation::DiffStat
                        | GitOperation::ChangedFiles
                        | GitOperation::DiffCheck => &[
                            "id",
                            "type",
                            "operation",
                            "target",
                            "base",
                            "head",
                            "paths",
                            "limits",
                        ],
                        GitOperation::Show => {
                            &["id", "type", "operation", "revision", "paths", "limits"]
                        }
                        GitOperation::Blame => &[
                            "id",
                            "type",
                            "operation",
                            "path",
                            "revision",
                            "start_line",
                            "end_line",
                            "limits",
                        ],
                        GitOperation::MergeBase => {
                            &["id", "type", "operation", "left", "right", "limits"]
                        }
                    };
                    object.retain(|key, _| allowed.contains(&key.as_str()));
                }
            }
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn parse(body: &str) -> Result<CollectionPlan, PlanError> {
        let dir = tempdir().unwrap();
        let path = dir.path().join("plan.yaml");
        let text = format!("version: 1\nrepository: .\ncollect:\n{body}");
        // Keep the temporary repository alive for canonicalization.
        let result = parse_plan(&text, &path);
        std::mem::forget(dir);
        result
    }

    #[test]
    fn defaults_and_normalizes_sets() {
        let plan = parse(
            "  - id: find\n    type: search\n    query: hello\n    extensions: [.HPP, .cpp, .CPP]\n",
        )
        .unwrap();
        let Query::Search(q) = &plan.collect[0] else {
            panic!()
        };
        assert_eq!(q.extensions, [".cpp", ".hpp"]);
        assert_eq!(plan.limits.max_matches, 50);
    }

    #[test]
    fn rejects_unknown_fields() {
        let error =
            parse("  - id: f\n    type: file\n    path: a\n    mystery: true\n").unwrap_err();
        assert_eq!(error.diagnostics[0].code, "unknown_field");
    }

    #[test]
    fn requires_the_contract_version() {
        let dir = tempdir().unwrap();
        let error = parse_plan(
            "repository: .\ncollect:\n  - id: f\n    type: file\n    path: a\n",
            dir.path().join("plan.yaml"),
        )
        .unwrap_err();
        assert_eq!(error.diagnostics[0].code, "malformed_plan");
    }

    #[test]
    fn rejects_duplicate_ids_and_bad_regex() {
        let error = parse(
            "  - id: same\n    type: search\n    query: '[x'\n    mode: regex\n  - id: same\n    type: file\n    path: x\n",
        )
        .unwrap_err();
        assert!(error.diagnostics.iter().any(|d| d.code == "invalid_regex"));
        assert!(
            error
                .diagnostics
                .iter()
                .any(|d| d.code == "duplicate_query_id")
        );
    }
}
