use std::collections::BTreeSet;

use tree_sitter::{Node, Parser};

use crate::evidence::source_evidence;
use crate::ids::assign_candidate_id;
use crate::path::{Repository, SourceError, logical_lines};
use crate::plan::{
    CollectionPlan, ContextSpec, SearchLimits, SearchMode, SearchQuery, SymbolFallback,
    SymbolLanguage, SymbolQuery,
};
use crate::search::{collect_search, source_error_diagnostic};
use crate::{
    Diagnostic, EvidenceRecord, MatchKind, Metrics, NameLocation, ParseQuality, Provenance,
    QueryResult, QueryType, Resolution, ScopeEntry, ScopeKind, Severity, SourceSpan, Status,
    StructuredName, SymbolCandidate, SymbolKind, SymbolRole, TemplateInfo, Truncation,
    TruncationKind,
};

const CPP_EXTENSIONS: &[&str] = &[
    ".cpp", ".cc", ".cxx", ".c++", ".h", ".hpp", ".hh", ".hxx", ".inl", ".ipp", ".tpp",
];

pub(crate) struct SymbolCollection {
    pub result: QueryResult,
    pub evidence: Vec<EvidenceRecord>,
}

pub(crate) fn collect_symbol(
    plan: &CollectionPlan,
    repository: &Repository,
    query: &SymbolQuery,
) -> SymbolCollection {
    let mut extensions = if query.extensions.is_empty() {
        CPP_EXTENSIONS.iter().map(|v| (*v).to_owned()).collect()
    } else {
        query.extensions.clone()
    };
    if matches!(query.language, SymbolLanguage::Auto) {
        extensions.retain(|extension| CPP_EXTENSIONS.contains(&extension.as_str()));
        if extensions.is_empty() {
            return symbol_terminal(
                query,
                Status::Unavailable,
                Diagnostic::query(
                    "language_provider_unavailable",
                    Severity::Error,
                    "no V1 language provider is registered for the requested extensions",
                    &query.id,
                ),
            );
        }
    }
    let mut excludes = plan.traversal.exclude.clone();
    excludes.extend(query.exclude.clone());
    let files = match repository.discover(&query.paths, &extensions, &excludes, &plan.traversal) {
        Ok(files) => files,
        Err(error) => {
            return symbol_terminal(
                query,
                Status::Failed,
                source_error_diagnostic(&query.id, ".", error),
            );
        }
    };
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_cpp::LANGUAGE.into())
        .is_err()
    {
        return symbol_terminal(
            query,
            Status::Unavailable,
            Diagnostic::query(
                "language_provider_unavailable",
                Severity::Error,
                "Tree-sitter C++ provider is unavailable",
                &query.id,
            ),
        );
    }

    let mut all = Vec::<(SymbolCandidate, String)>::new();
    let mut diagnostics = Vec::new();
    for path in files {
        let source = match repository.read_text(&path) {
            Ok(source) => source,
            Err(SourceError::Binary | SourceError::UnsupportedEncoding) => continue,
            Err(error) => {
                diagnostics.push(source_error_diagnostic(&query.id, &path, error));
                continue;
            }
        };
        let Some(tree) = parser.parse(&source, None) else {
            diagnostics.push(Diagnostic::query(
                "cpp_symbol_syntactic_resolution_failed",
                Severity::Warning,
                format!("C++ parsing failed for `{path}`"),
                &query.id,
            ));
            continue;
        };
        let mut candidates = Vec::new();
        walk_cpp(
            tree.root_node(),
            &source,
            &path,
            &[],
            0,
            None,
            false,
            &mut candidates,
        );
        for mut candidate in candidates {
            if !query.roles.contains(&candidate.role) || !query.kinds.contains(&candidate.kind) {
                continue;
            }
            if let Some(match_kind) = match_candidate(&query.name, &candidate) {
                candidate.match_kind = match_kind;
                assign_candidate_id(&mut candidate);
                all.push((candidate, source.clone()));
            }
        }
    }
    all.sort_by(|(a, _), (b, _)| {
        (
            &a.name_location.path,
            a.name_location.line,
            a.name_location.column,
            &a.kind,
            &a.signature_text,
            &a.candidate_id,
        )
            .cmp(&(
                &b.name_location.path,
                b.name_location.line,
                b.name_location.column,
                &b.kind,
                &b.signature_text,
                &b.candidate_id,
            ))
    });

    if all.is_empty() && matches!(query.fallback, SymbolFallback::Textual) {
        let mut fallback = textual_fallback(plan, repository, query, extensions);
        fallback.result.diagnostics.extend(diagnostics);
        if fallback.result.status == Status::Empty
            && fallback
                .result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
        {
            fallback.result.status = Status::Failed;
        }
        return fallback;
    }
    if all.is_empty() {
        let status = if diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
        {
            Status::Failed
        } else {
            Status::Empty
        };
        return SymbolCollection {
            result: QueryResult {
                query_id: query.id.clone(),
                query_type: QueryType::Symbol,
                status,
                evidence_ids: vec![],
                candidates: vec![],
                diagnostics,
                metrics: Metrics::Symbol {
                    candidates_found: 0,
                    candidates_included: 0,
                    candidates_omitted: 0,
                    textual_fallback_matches: 0,
                },
                truncations: vec![],
            },
            evidence: vec![],
        };
    }

    let found = all.len();
    let max_candidates = query
        .limits
        .max_candidates
        .unwrap_or(plan.limits.max_candidates);
    all.truncate(max_candidates);
    let mut truncations = Vec::new();
    if found > all.len() {
        truncations.push(Truncation {
            kind: TruncationKind::CandidateLimit,
            unit: "candidates".into(),
            limit: max_candidates,
            total: found,
            included: all.len(),
            omitted: found - all.len(),
            strategy: "stable_prefix".into(),
            omitted_source_spans: vec![],
            affected_evidence_ids: vec![],
        });
        diagnostics.push(Diagnostic::query(
            "candidate_limit_reached",
            Severity::Warning,
            format!("{found} candidates were found; {} were included", all.len()),
            &query.id,
        ));
    }
    if found > 1 {
        diagnostics.push(Diagnostic::query(
            "symbol_ambiguous",
            Severity::Warning,
            format!("{found} syntactic candidates match `{}`", query.name),
            &query.id,
        ));
    }
    let max_lines = query
        .limits
        .max_evidence_lines
        .unwrap_or(plan.limits.max_evidence_lines);
    let max_bytes = query
        .limits
        .max_evidence_bytes
        .unwrap_or(plan.limits.max_evidence_bytes);
    let mut evidence = Vec::new();
    let mut candidates = Vec::new();
    for (candidate, source) in all {
        let lines = logical_lines(&source);
        let start = candidate.source_extent.start_line;
        let end = candidate.source_extent.end_line.min(lines.len());
        let content = if start <= end {
            lines[start - 1..end].concat()
        } else {
            String::new()
        };
        let record = source_evidence(
            &query.id,
            &candidate.source_extent.path,
            start,
            &content,
            Resolution::Syntactic,
            "cpp-tree-sitter",
            max_lines,
            max_bytes,
        );
        if !record.truncations.is_empty() {
            diagnostics.push(Diagnostic::query(
                "evidence_limit_reached",
                Severity::Warning,
                format!(
                    "symbol evidence for `{}` exceeded its evidence limit",
                    candidate.name.qualified
                ),
                &query.id,
            ));
        }
        if candidate.parse_quality == ParseQuality::Recovered {
            let mut diagnostic = Diagnostic::query(
                "cpp_parse_recovered",
                Severity::Warning,
                format!(
                    "Tree-sitter recovery affected `{}`",
                    candidate.name.qualified
                ),
                &query.id,
            );
            diagnostic.candidate_id = Some(candidate.candidate_id.clone());
            diagnostic.path = Some(candidate.source_extent.path.clone());
            diagnostics.push(diagnostic);
        }
        candidates.push(candidate);
        evidence.push(record);
    }
    let status = if found > 1 {
        Status::Ambiguous
    } else if !truncations.is_empty()
        || evidence.iter().any(|e| !e.truncations.is_empty())
        || diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
        || candidates
            .iter()
            .any(|c| c.parse_quality == ParseQuality::Recovered)
    {
        Status::Partial
    } else {
        Status::Ok
    };
    let evidence_ids = evidence.iter().map(|e| e.evidence_id.clone()).collect();
    SymbolCollection {
        result: QueryResult {
            query_id: query.id.clone(),
            query_type: QueryType::Symbol,
            status,
            evidence_ids,
            candidates,
            diagnostics,
            metrics: Metrics::Symbol {
                candidates_found: found,
                candidates_included: evidence.len(),
                candidates_omitted: found - evidence.len(),
                textual_fallback_matches: 0,
            },
            truncations,
        },
        evidence,
    }
}

fn textual_fallback(
    plan: &CollectionPlan,
    repository: &Repository,
    query: &SymbolQuery,
    extensions: Vec<String>,
) -> SymbolCollection {
    let leaf = query
        .name
        .rsplit("::")
        .next()
        .unwrap_or(&query.name)
        .to_string();
    let search_query = SearchQuery {
        id: query.id.clone(),
        query: leaf,
        mode: SearchMode::Literal,
        paths: query.paths.clone(),
        exclude: query.exclude.clone(),
        extensions,
        case_sensitive: true,
        context: ContextSpec::default(),
        limits: SearchLimits {
            max_matches: Some(
                query
                    .limits
                    .max_candidates
                    .unwrap_or(plan.limits.max_candidates),
            ),
            max_evidence_lines: query.limits.max_evidence_lines,
            max_evidence_bytes: query.limits.max_evidence_bytes,
            max_query_evidence_bytes: query.limits.max_query_evidence_bytes,
        },
    };
    let search = collect_search(plan, repository, &search_query);
    let matches = match search.result.metrics {
        Metrics::Search {
            matches_included, ..
        } => matches_included,
        _ => 0,
    };
    let mut diagnostics = search.result.diagnostics;
    diagnostics.push(Diagnostic::query(
        "cpp_symbol_syntactic_resolution_failed",
        Severity::Warning,
        format!(
            "no syntactic candidate matched `{}`; textual fallback was used",
            query.name
        ),
        &query.id,
    ));
    SymbolCollection {
        result: QueryResult {
            query_id: query.id.clone(),
            query_type: QueryType::Symbol,
            status: if matches == 0 {
                Status::Empty
            } else {
                Status::Partial
            },
            evidence_ids: search.result.evidence_ids,
            candidates: vec![],
            diagnostics,
            metrics: Metrics::Symbol {
                candidates_found: 0,
                candidates_included: 0,
                candidates_omitted: 0,
                textual_fallback_matches: matches,
            },
            truncations: search.result.truncations,
        },
        evidence: search.evidence,
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_cpp(
    node: Node<'_>,
    source: &str,
    path: &str,
    scope: &[ScopeEntry],
    template_layers: usize,
    outer_start: Option<usize>,
    inherited_recovery: bool,
    output: &mut Vec<SymbolCandidate>,
) {
    match node.kind() {
        "template_declaration" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "template_parameter_list" {
                    walk_cpp(
                        child,
                        source,
                        path,
                        scope,
                        template_layers + 1,
                        outer_start.or(Some(node.start_byte())),
                        inherited_recovery || node.has_error(),
                        output,
                    );
                }
            }
            return;
        }
        "function_definition" => {
            if let Some(candidate) = callable_candidate(
                node,
                source,
                path,
                scope,
                template_layers,
                outer_start,
                inherited_recovery,
            ) {
                output.push(candidate);
                return;
            }
            if let Some((entry, body)) = recovered_class_body(node, source) {
                let mut next = scope.to_vec();
                next.push(entry);
                walk_children(body, source, path, &next, 0, None, true, output);
            }
            return;
        }
        "declaration" | "field_declaration" => {
            if owned_function_declarator(node).is_some() {
                if let Some(candidate) = callable_candidate(
                    node,
                    source,
                    path,
                    scope,
                    template_layers,
                    outer_start,
                    inherited_recovery,
                ) {
                    output.push(candidate);
                }
                return;
            }
        }
        "namespace_definition" => {
            let name_node = node.child_by_field_name("name");
            let name = name_node.map(|n| compact_name(text(n, source)));
            if let Some(name_node) = name_node {
                output.push(non_callable_candidate(
                    node,
                    name_node,
                    SymbolKind::Namespace,
                    SymbolRole::Definition,
                    source,
                    path,
                    scope,
                    template_layers,
                    outer_start,
                ));
            }
            let mut next = scope.to_vec();
            if let Some(name) = name {
                for component in split_qualified(&name) {
                    next.push(ScopeEntry {
                        kind: ScopeKind::Namespace,
                        name: Some(component),
                        anonymous: false,
                    });
                }
            } else {
                next.push(ScopeEntry {
                    kind: ScopeKind::Namespace,
                    name: None,
                    anonymous: true,
                });
            }
            if let Some(body) = node.child_by_field_name("body") {
                walk_children(
                    body,
                    source,
                    path,
                    &next,
                    0,
                    None,
                    inherited_recovery || node.has_error(),
                    output,
                );
            }
            return;
        }
        "class_specifier" | "struct_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let kind = if node.kind() == "class_specifier" {
                    SymbolKind::Class
                } else {
                    SymbolKind::Struct
                };
                let role = if node.child_by_field_name("body").is_some() {
                    SymbolRole::Definition
                } else {
                    SymbolRole::Declaration
                };
                output.push(non_callable_candidate(
                    node,
                    name_node,
                    kind.clone(),
                    role,
                    source,
                    path,
                    scope,
                    template_layers,
                    outer_start,
                ));
                if let Some(body) = node.child_by_field_name("body") {
                    let mut next = scope.to_vec();
                    next.push(ScopeEntry {
                        kind: if kind == SymbolKind::Class {
                            ScopeKind::Class
                        } else {
                            ScopeKind::Struct
                        },
                        name: Some(compact_name(text(name_node, source))),
                        anonymous: false,
                    });
                    walk_children(
                        body,
                        source,
                        path,
                        &next,
                        0,
                        None,
                        inherited_recovery || node.has_error(),
                        output,
                    );
                }
                return;
            }
        }
        "enum_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let role = if node.child_by_field_name("body").is_some() {
                    SymbolRole::Definition
                } else {
                    SymbolRole::Declaration
                };
                output.push(non_callable_candidate(
                    node,
                    name_node,
                    SymbolKind::Enum,
                    role,
                    source,
                    path,
                    scope,
                    template_layers,
                    outer_start,
                ));
                return;
            }
        }
        _ => {}
    }
    walk_children(
        node,
        source,
        path,
        scope,
        template_layers,
        outer_start,
        inherited_recovery,
        output,
    );
}

#[allow(clippy::too_many_arguments)]
fn walk_children(
    node: Node<'_>,
    source: &str,
    path: &str,
    scope: &[ScopeEntry],
    template_layers: usize,
    outer_start: Option<usize>,
    inherited_recovery: bool,
    output: &mut Vec<SymbolCandidate>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_cpp(
            child,
            source,
            path,
            scope,
            template_layers,
            outer_start,
            inherited_recovery,
            output,
        );
    }
}

fn callable_candidate(
    node: Node<'_>,
    source: &str,
    path: &str,
    scope: &[ScopeEntry],
    template_layers: usize,
    outer_start: Option<usize>,
    inherited_recovery: bool,
) -> Option<SymbolCandidate> {
    let role = callable_role(node, source)?;
    let function_declarator = owned_function_declarator(node)?;
    let declarator = function_declarator
        .child_by_field_name("declarator")
        .or_else(|| first_name_descendant(function_declarator))?;
    let name_node = name_expression(declarator).unwrap_or(declarator);
    let leaf_node = leaf_name_node(name_node);
    let spelled = compact_name(text(name_node, source));
    let elided_spelled = compact_name(&text_eliding_templates(name_node, source));
    let leaf = compact_name(text(leaf_node, source));
    let structured = structured_name(&leaf, &spelled, &elided_spelled, scope);
    let class_name = scope.iter().rev().find_map(|entry| match entry.kind {
        ScopeKind::Class | ScopeKind::Struct => entry.name.as_deref(),
        ScopeKind::Namespace => None,
    });
    let owner = split_qualified(&elided_spelled)
        .into_iter()
        .rev()
        .nth(1)
        .map(|v| elide_text_templates(&v));
    let plain_leaf = elide_text_templates(&leaf);
    let start_byte = outer_start.unwrap_or(node.start_byte());
    let end_byte = node.end_byte();
    let signature_end = if role == SymbolRole::Definition {
        node.child_by_field_name("body")
            .map(|body| body.start_byte())
            .unwrap_or(end_byte)
    } else {
        end_byte
    };
    let signature_raw = source
        .get(start_byte..signature_end)?
        .trim()
        .trim_end_matches(';');
    let candidate_modifiers = modifiers(signature_raw);
    let kind = if leaf.starts_with('~') {
        SymbolKind::Destructor
    } else if leaf.starts_with("operator") {
        SymbolKind::Operator
    } else if class_name.is_some_and(|name| elide_text_templates(name) == plain_leaf)
        || owner.is_some_and(|name| name == plain_leaf)
    {
        SymbolKind::Constructor
    } else if (class_name.is_some() || spelled.contains("::"))
        && !candidate_modifiers
            .iter()
            .any(|modifier| modifier == "friend")
    {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };
    let extent = source_span(path, source, start_byte, end_byte);
    let signature_text = signature_raw
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let signature_location = source_span(path, source, start_byte, signature_end);
    let parse_quality = if inherited_recovery || node.has_error() {
        ParseQuality::Recovered
    } else {
        ParseQuality::Clean
    };
    let mut candidate = SymbolCandidate {
        candidate_id: String::new(),
        language: "cpp".into(),
        kind,
        role,
        name: structured,
        lexical_scope: scope.to_vec(),
        match_kind: MatchKind::Unqualified,
        name_location: NameLocation {
            path: path.into(),
            line: leaf_node.start_position().row + 1,
            column: leaf_node.start_position().column + 1,
        },
        source_extent: extent,
        signature_text: Some(signature_text),
        signature_location: Some(signature_location),
        template: TemplateInfo {
            is_template: template_layers > 0,
            layers: template_layers,
        },
        modifiers: candidate_modifiers,
        parse_quality,
        provenance: Provenance {
            provider_id: "cpp-tree-sitter".into(),
        },
    };
    assign_candidate_id(&mut candidate);
    Some(candidate)
}

/// Returns the function declarator structurally owned by a declaration or
/// definition. Deliberately do not search arbitrary descendants: a recovered
/// node can contain an entire class body, including unrelated methods.
fn owned_function_declarator(node: Node<'_>) -> Option<Node<'_>> {
    let mut declarator = node.child_by_field_name("declarator")?;
    loop {
        if declarator.kind() == "function_declarator" {
            return Some(declarator);
        }
        declarator = declarator.child_by_field_name("declarator")?;
    }
}

/// Classify a callable only when its own syntax establishes the role. Parser
/// recovery affects parse quality, but never changes declaration/definition
/// invariants.
fn callable_role(node: Node<'_>, source: &str) -> Option<SymbolRole> {
    match node.kind() {
        "declaration" | "field_declaration" => text(node, source)
            .trim_end()
            .ends_with(';')
            .then_some(SymbolRole::Declaration),
        "function_definition" => {
            if node
                .child_by_field_name("body")
                .is_some_and(|body| matches!(body.kind(), "compound_statement" | "try_statement"))
            {
                return Some(SymbolRole::Definition);
            }

            let mut cursor = node.walk();
            let mut role = None;
            for child in node.named_children(&mut cursor) {
                match child.kind() {
                    "default_method_clause" | "delete_method_clause" => {
                        role = Some(SymbolRole::Definition);
                        break;
                    }
                    "pure_virtual_clause" => role = Some(SymbolRole::Declaration),
                    _ => {}
                }
            }
            role.or_else(|| {
                text(node, source)
                    .trim_end()
                    .ends_with(';')
                    .then_some(SymbolRole::Declaration)
            })
        }
        _ => None,
    }
}

/// Unreal export macros can make Tree-sitter recover a class as a
/// `function_definition` whose return type is a class specifier and whose
/// declarator is actually the class name. Preserve the class scope so its
/// members can still be considered, but never emit the wrapper as a callable.
fn recovered_class_body<'a>(node: Node<'a>, source: &str) -> Option<(ScopeEntry, Node<'a>)> {
    if node.kind() != "function_definition" || !node.has_error() {
        return None;
    }
    let type_node = node.child_by_field_name("type")?;
    let scope_kind = match type_node.kind() {
        "class_specifier" => ScopeKind::Class,
        "struct_specifier" => ScopeKind::Struct,
        _ => return None,
    };
    let declarator = node.child_by_field_name("declarator")?;
    if declarator.kind() == "function_declarator" || owned_function_declarator(node).is_some() {
        return None;
    }
    let name_node = name_expression(declarator)?;
    let name = compact_name(text(name_node, source));
    if name.is_empty() || name.contains("::") || name.chars().any(char::is_whitespace) {
        return None;
    }
    let body = node.child_by_field_name("body")?;
    if body.kind() != "compound_statement" {
        return None;
    }
    Some((
        ScopeEntry {
            kind: scope_kind,
            name: Some(name),
            anonymous: false,
        },
        body,
    ))
}

#[allow(clippy::too_many_arguments)]
fn non_callable_candidate(
    node: Node<'_>,
    name_node: Node<'_>,
    kind: SymbolKind,
    role: SymbolRole,
    source: &str,
    path: &str,
    scope: &[ScopeEntry],
    template_layers: usize,
    outer_start: Option<usize>,
) -> SymbolCandidate {
    let spelled = compact_name(text(name_node, source));
    let elided = compact_name(&text_eliding_templates(name_node, source));
    let leaf_node = leaf_name_node(name_node);
    let leaf = compact_name(text(leaf_node, source));
    let name = structured_name(&leaf, &spelled, &elided, scope);
    let start = outer_start.unwrap_or(node.start_byte());
    let mut candidate = SymbolCandidate {
        candidate_id: String::new(),
        language: "cpp".into(),
        kind,
        role,
        name,
        lexical_scope: scope.to_vec(),
        match_kind: MatchKind::Unqualified,
        name_location: NameLocation {
            path: path.into(),
            line: leaf_node.start_position().row + 1,
            column: leaf_node.start_position().column + 1,
        },
        source_extent: source_span(path, source, start, node.end_byte()),
        signature_text: None,
        signature_location: None,
        template: TemplateInfo {
            is_template: template_layers > 0,
            layers: template_layers,
        },
        modifiers: vec![],
        parse_quality: if node.has_error() {
            ParseQuality::Recovered
        } else {
            ParseQuality::Clean
        },
        provenance: Provenance {
            provider_id: "cpp-tree-sitter".into(),
        },
    };
    assign_candidate_id(&mut candidate);
    candidate
}

fn structured_name(
    leaf: &str,
    spelled: &str,
    elided_spelled: &str,
    scope: &[ScopeEntry],
) -> StructuredName {
    let lexical = scope
        .iter()
        .filter_map(|entry| entry.name.clone())
        .collect::<Vec<_>>();
    let spelled_parts = split_qualified(spelled);
    let elided_parts = split_qualified(elided_spelled);
    let mut overlap = 0;
    for count in 1..=lexical.len().min(spelled_parts.len()) {
        let left = &lexical[lexical.len() - count..];
        let right = &spelled_parts[..count];
        if left
            .iter()
            .map(|s| elide_text_templates(s))
            .eq(right.iter().map(|s| elide_text_templates(s)))
        {
            overlap = count;
        }
    }
    let mut qualified_parts = lexical[..lexical.len() - overlap].to_vec();
    qualified_parts.extend(spelled_parts.clone());
    let qualified = qualified_parts.join("::");
    let mut aliases = BTreeSet::new();
    aliases.insert(spelled.into());
    for index in 1..qualified_parts.len().saturating_sub(1) {
        aliases.insert(qualified_parts[index..].join("::"));
    }
    let mut elided_qualified_parts = lexical
        .iter()
        .take(lexical.len() - overlap)
        .map(|s| elide_text_templates(s))
        .collect::<Vec<_>>();
    elided_qualified_parts.extend(elided_parts);
    let elided_qualified = elided_qualified_parts.join("::");
    aliases.insert(elided_spelled.into());
    aliases.insert(elided_qualified.clone());
    for index in 1..elided_qualified_parts.len().saturating_sub(1) {
        aliases.insert(elided_qualified_parts[index..].join("::"));
    }
    aliases.remove(&qualified);
    aliases.remove(leaf);
    aliases.remove("");
    StructuredName {
        leaf: leaf.into(),
        spelled: spelled.into(),
        qualified,
        match_aliases: aliases.into_iter().collect(),
    }
}

fn match_candidate(query: &str, candidate: &SymbolCandidate) -> Option<MatchKind> {
    if query == candidate.name.qualified {
        return Some(MatchKind::ExactQualified);
    }
    if query.contains("::")
        && (candidate.name.qualified.ends_with(&format!("::{query}"))
            || (query == candidate.name.spelled && candidate.name.qualified != query))
    {
        return Some(MatchKind::SuffixQualified);
    }
    if candidate
        .name
        .match_aliases
        .iter()
        .any(|alias| alias == query)
    {
        return Some(MatchKind::TemplateElided);
    }
    if !query.contains("::") && query == candidate.name.leaf {
        return Some(MatchKind::Unqualified);
    }
    None
}

fn first_name_descendant(node: Node<'_>) -> Option<Node<'_>> {
    if is_name_kind(node.kind()) {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = first_name_descendant(child) {
            return Some(found);
        }
    }
    None
}

fn name_expression(node: Node<'_>) -> Option<Node<'_>> {
    if is_name_kind(node.kind()) {
        return Some(node);
    }
    node.child_by_field_name("declarator")
        .and_then(name_expression)
        .or_else(|| node.child_by_field_name("name").and_then(name_expression))
        .or_else(|| first_name_descendant(node))
}

fn is_name_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "field_identifier"
            | "type_identifier"
            | "qualified_identifier"
            | "destructor_name"
            | "operator_name"
            | "operator_cast"
            | "template_function"
    )
}

fn leaf_name_node(node: Node<'_>) -> Node<'_> {
    if let Some(name) = node.child_by_field_name("name") {
        return leaf_name_node(name);
    }
    if node.kind() == "qualified_identifier" {
        let mut cursor = node.walk();
        if let Some(last) = node.named_children(&mut cursor).last() {
            return leaf_name_node(last);
        }
    }
    node
}

fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or("")
}

fn text_eliding_templates(node: Node<'_>, source: &str) -> String {
    let mut ranges = Vec::new();
    collect_template_ranges(node, &mut ranges);
    ranges.sort();
    let mut result = String::new();
    let mut cursor = node.start_byte();
    for (start, end) in ranges {
        if start >= cursor {
            result.push_str(source.get(cursor..start).unwrap_or(""));
            cursor = end;
        }
    }
    result.push_str(source.get(cursor..node.end_byte()).unwrap_or(""));
    result
}

fn collect_template_ranges(node: Node<'_>, ranges: &mut Vec<(usize, usize)>) {
    if node.kind() == "template_argument_list" {
        ranges.push((node.start_byte(), node.end_byte()));
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_template_ranges(child, ranges);
    }
}

fn compact_name(value: &str) -> String {
    value
        .split("::")
        .map(|part| part.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("::")
}

fn split_qualified(value: &str) -> Vec<String> {
    let chars = value.chars().collect::<Vec<_>>();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut i = 0;
    while i + 1 < chars.len() {
        match chars[i] {
            '<' => depth += 1,
            '>' => depth = (depth - 1).max(0),
            ':' if chars[i + 1] == ':' && depth == 0 => {
                parts.push(
                    chars[start..i]
                        .iter()
                        .collect::<String>()
                        .trim()
                        .to_string(),
                );
                i += 1;
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(chars[start..].iter().collect::<String>().trim().to_string());
    parts
}

fn elide_text_templates(value: &str) -> String {
    let mut result = String::new();
    let mut depth = 0usize;
    for ch in value.chars() {
        match ch {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => result.push(ch),
            _ => {}
        }
    }
    result
}

fn source_span(path: &str, source: &str, start: usize, end: usize) -> SourceSpan {
    SourceSpan {
        path: path.into(),
        start_line: byte_line(source, start),
        end_line: byte_end_line(source, end),
    }
}

fn byte_line(source: &str, byte: usize) -> usize {
    source.as_bytes()[..byte.min(source.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

fn byte_end_line(source: &str, byte: usize) -> usize {
    let capped = byte.min(source.len());
    if capped > 0 && source.as_bytes().get(capped - 1) == Some(&b'\n') {
        byte_line(source, capped - 1)
    } else {
        byte_line(source, capped)
    }
}

fn modifiers(signature: &str) -> Vec<String> {
    const MODIFIERS: &[&str] = &[
        "static",
        "inline",
        "virtual",
        "constexpr",
        "consteval",
        "explicit",
        "friend",
        "override",
        "final",
    ];
    let words = signature
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .collect::<BTreeSet<_>>();
    MODIFIERS
        .iter()
        .filter(|modifier| words.contains(**modifier))
        .map(|value| (*value).to_owned())
        .collect()
}

fn symbol_terminal(
    query: &SymbolQuery,
    status: Status,
    diagnostic: Diagnostic,
) -> SymbolCollection {
    SymbolCollection {
        result: QueryResult {
            query_id: query.id.clone(),
            query_type: QueryType::Symbol,
            status,
            evidence_ids: vec![],
            candidates: vec![],
            diagnostics: vec![diagnostic],
            metrics: Metrics::Symbol {
                candidates_found: 0,
                candidates_included: 0,
                candidates_omitted: 0,
                textual_fallback_matches: 0,
            },
            truncations: vec![],
        },
        evidence: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(source: &str) -> Vec<SymbolCandidate> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_cpp::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let mut output = Vec::new();
        walk_cpp(
            tree.root_node(),
            source,
            "fixture.cpp",
            &[],
            0,
            None,
            false,
            &mut output,
        );
        output
    }

    #[test]
    fn extracts_overloads_and_qualified_methods() {
        let candidates = extract(
            "namespace A { class Foo { public: void bar(int); }; }\nvoid A::Foo::bar(int x) {}\nvoid A::Foo::bar(double x) {}\n",
        );
        let bars = candidates
            .iter()
            .filter(|c| c.name.leaf == "bar")
            .collect::<Vec<_>>();
        assert_eq!(bars.len(), 3);
        assert!(bars.iter().any(|c| c.role == SymbolRole::Declaration));
        assert!(bars.iter().any(|c| c.name.qualified == "A::Foo::bar"));
    }

    #[test]
    fn extracts_templates_ctors_dtors_and_operators() {
        let candidates = extract(
            "template<class T> struct Box { Box(); ~Box(); bool operator==(const Box&) const; void put(T); };\ntemplate<class T> void Box<T>::put(T value) {}\n",
        );
        assert!(candidates.iter().any(|c| c.kind == SymbolKind::Constructor));
        assert!(candidates.iter().any(|c| c.kind == SymbolKind::Destructor));
        assert!(candidates.iter().any(|c| c.kind == SymbolKind::Operator));
        let put = candidates
            .iter()
            .find(|c| c.name.spelled.contains("Box<T>::put"))
            .unwrap();
        assert!(put.name.match_aliases.iter().any(|a| a == "Box::put"));
        assert!(put.template.is_template);
    }

    #[test]
    fn suffix_and_template_matching_are_explicit() {
        let candidate = extract(
            "template<class T> struct Foo { void bar(); }; template<class T> void Foo<T>::bar() {}",
        )
        .into_iter()
        .find(|c| c.name.spelled.contains("Foo<T>::bar"))
        .unwrap();
        assert_eq!(
            match_candidate("Foo::bar", &candidate),
            Some(MatchKind::TemplateElided)
        );
        assert_eq!(
            match_candidate("bar", &candidate),
            Some(MatchKind::Unqualified)
        );
    }
}
