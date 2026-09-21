use std::path::Path;

use contextpack_core::{
    MatchKind, ParseQuality, Status, SymbolKind, SymbolRole, collect_plan, parse_plan,
};

fn collect(query: &str) -> contextpack_core::CollectionResult {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cpp_symbols");
    let yaml = format!(
        "version: 1\nrepository: '{}'\ncollect:\n{}",
        fixture.display().to_string().replace('\\', "/"),
        query
    );
    parse_plan(&yaml, fixture.join("plan.yaml"))
        .map(|plan| collect_plan(&plan))
        .unwrap()
}

#[test]
fn overloads_are_ambiguous_and_none_is_selected() {
    let result = collect(
        "  - id: overloads\n    type: symbol\n    name: Acme::Model::overloaded\n    roles: [definition]\n    kinds: [function]\n    fallback: none\n",
    );
    let query = &result.query_results[0];
    assert_eq!(query.status, Status::Ambiguous);
    assert_eq!(query.candidates.len(), 2);
    assert!(
        query
            .candidates
            .iter()
            .all(|c| c.role == SymbolRole::Definition)
    );
    assert_ne!(
        query.candidates[0].candidate_id,
        query.candidates[1].candidate_id
    );
}

#[test]
fn declaration_and_definition_roles_are_distinct() {
    let result = collect(
        "  - id: declarations\n    type: symbol\n    name: Acme::Model::overloaded\n    roles: [declaration]\n    kinds: [function]\n    fallback: none\n  - id: record-definition\n    type: symbol\n    name: Acme::Model::Record\n    roles: [definition]\n    kinds: [struct]\n    fallback: none\n  - id: record-declaration\n    type: symbol\n    name: Acme::Model::Record\n    roles: [declaration]\n    kinds: [struct]\n    fallback: none\n",
    );
    assert_eq!(result.query_results[0].status, Status::Ambiguous);
    assert_eq!(result.query_results[0].candidates.len(), 2);
    assert_eq!(result.query_results[1].status, Status::Ok);
    assert_eq!(
        result.query_results[1].candidates[0].kind,
        SymbolKind::Struct
    );
    assert_eq!(result.query_results[2].status, Status::Ok);
    assert_eq!(
        result.query_results[2].candidates[0].role,
        SymbolRole::Declaration
    );
}

#[test]
fn template_elision_constructors_destructors_and_operators() {
    let result = collect(
        "  - id: put\n    type: symbol\n    name: Box::put\n    roles: [definition]\n    kinds: [method]\n    fallback: none\n  - id: ctor\n    type: symbol\n    name: Box::Box\n    roles: [definition]\n    kinds: [constructor]\n    fallback: none\n  - id: dtor\n    type: symbol\n    name: Box::~Box\n    roles: [definition]\n    kinds: [destructor]\n    fallback: none\n  - id: op\n    type: symbol\n    name: Box::operator==\n    roles: [definition]\n    kinds: [operator]\n    fallback: none\n",
    );
    for query in &result.query_results {
        assert_eq!(
            query.status,
            Status::Ok,
            "{}: {:?}",
            query.query_id,
            query.diagnostics
        );
        assert_eq!(query.candidates.len(), 1);
        assert_eq!(query.candidates[0].match_kind, MatchKind::TemplateElided);
        assert!(query.candidates[0].template.is_template);
    }
}

#[test]
fn nested_namespace_scope_and_anonymous_namespace_are_reconstructed() {
    let result = collect(
        "  - id: state\n    type: symbol\n    name: Acme::Model::State\n    roles: [definition]\n    kinds: [enum]\n    fallback: none\n  - id: hidden\n    type: symbol\n    name: hidden_helper\n    roles: [definition]\n    kinds: [function]\n    fallback: none\n",
    );
    let state = &result.query_results[0].candidates[0];
    assert_eq!(state.name.qualified, "Acme::Model::State");
    assert_eq!(state.lexical_scope.len(), 2);
    let hidden = &result.query_results[1].candidates[0];
    assert!(hidden.lexical_scope.iter().any(|scope| scope.anonymous));
    assert!(hidden.modifiers.iter().any(|modifier| modifier == "inline"));
}

#[test]
fn recovered_parse_is_explicit_and_textual_fallback_is_not_a_candidate() {
    let result = collect(
        "  - id: recovered\n    type: symbol\n    name: Broken::still_visible\n    roles: [definition]\n    kinds: [function]\n    fallback: none\n  - id: textual\n    type: symbol\n    name: definitely_not_a_symbol\n    fallback: textual\n",
    );
    let recovered = &result.query_results[0];
    assert!(
        matches!(recovered.status, Status::Ok | Status::Partial),
        "{recovered:#?}"
    );
    if recovered.status == Status::Partial {
        assert_eq!(
            recovered.candidates[0].parse_quality,
            ParseQuality::Recovered
        );
    }
    let textual = &result.query_results[1];
    assert_eq!(textual.status, Status::Empty);
    assert!(textual.candidates.is_empty());
    assert!(
        textual
            .diagnostics
            .iter()
            .any(|d| d.code == "cpp_symbol_syntactic_resolution_failed")
    );
}
