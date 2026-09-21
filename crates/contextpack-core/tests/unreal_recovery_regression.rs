use std::path::Path;

use contextpack_core::{ParseQuality, Status, SymbolRole, collect_plan, parse_plan};

fn collect(name: &str, role: &str) -> contextpack_core::CollectionResult {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cpp_symbols");
    let yaml = format!(
        "version: 1\nrepository: '{}'\ncollect:\n  - id: unreal-symbol\n    type: symbol\n    name: {name}\n    language: cpp\n    roles: [{role}]\n    kinds: [function, method]\n    paths: [include, src]\n    fallback: none\n",
        fixture.display().to_string().replace('\\', "/"),
    );
    let plan = parse_plan(&yaml, fixture.join("plan.yaml")).unwrap();
    collect_plan(&plan)
}

#[test]
fn definition_query_returns_only_the_out_of_line_implementation() {
    let result = collect("BeginPlay", "definition");
    let query = &result.query_results[0];
    assert_eq!(query.status, Status::Ok, "{query:#?}");
    assert_eq!(query.candidates.len(), 1, "{query:#?}");
    let candidate = &query.candidates[0];
    assert_eq!(candidate.role, SymbolRole::Definition);
    assert_eq!(candidate.source_extent.path, "src/UnrealActor.cpp");
    assert_eq!(candidate.source_extent.start_line, 3);
    assert_eq!(candidate.source_extent.end_line, 6);
    assert_eq!(
        candidate.signature_text.as_deref(),
        Some("void AStaticMeshAnimationActor::BeginPlay()")
    );
}

#[test]
fn declaration_query_returns_the_header_declaration() {
    let result = collect("BeginPlay", "declaration");
    let query = &result.query_results[0];
    assert_eq!(query.status, Status::Partial, "{query:#?}");
    assert_eq!(query.candidates.len(), 1, "{query:#?}");
    let candidate = &query.candidates[0];
    assert_eq!(candidate.role, SymbolRole::Declaration);
    assert_eq!(candidate.source_extent.path, "include/UnrealActor.h");
    assert_eq!(candidate.source_extent.start_line, 18);
    assert_eq!(candidate.source_extent.end_line, 18);
    assert_eq!(
        candidate.signature_text.as_deref(),
        Some("virtual void BeginPlay() override")
    );
    assert_eq!(candidate.parse_quality, ParseQuality::Recovered);
    let recovery = query
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "cpp_parse_recovered")
        .expect("recovered candidate should have a diagnostic");
    assert_eq!(
        recovery.candidate_id.as_deref(),
        Some(&*candidate.candidate_id)
    );
    assert_eq!(recovery.path.as_deref(), Some("include/UnrealActor.h"));
}

#[test]
fn recovered_inline_definition_keeps_callable_extent_and_signature() {
    let result = collect("GetFrameCount", "definition");
    let query = &result.query_results[0];
    assert_eq!(query.status, Status::Partial, "{query:#?}");
    assert_eq!(query.candidates.len(), 1, "{query:#?}");
    let candidate = &query.candidates[0];
    assert_eq!(candidate.role, SymbolRole::Definition);
    assert_eq!(candidate.source_extent.path, "include/UnrealActor.h");
    assert_eq!(candidate.source_extent.start_line, 20);
    assert_eq!(candidate.source_extent.end_line, 23);
    assert_eq!(
        candidate.signature_text.as_deref(),
        Some("FORCEINLINE int32 GetFrameCount() const")
    );
    assert_eq!(candidate.parse_quality, ParseQuality::Recovered);
}

#[test]
fn ufunction_declaration_and_definition_keep_distinct_roles() {
    let declaration = collect("StartAnimation", "declaration");
    let declaration_query = &declaration.query_results[0];
    assert_eq!(
        declaration_query.candidates.len(),
        1,
        "{declaration_query:#?}"
    );
    assert_eq!(
        declaration_query.candidates[0].source_extent.path,
        "include/UnrealActor.h"
    );
    assert_eq!(
        declaration_query.candidates[0].role,
        SymbolRole::Declaration
    );

    let definition = collect("StartAnimation", "definition");
    let definition_query = &definition.query_results[0];
    assert_eq!(definition_query.status, Status::Ok, "{definition_query:#?}");
    assert_eq!(
        definition_query.candidates.len(),
        1,
        "{definition_query:#?}"
    );
    assert_eq!(
        definition_query.candidates[0].source_extent.path,
        "src/UnrealActor.cpp"
    );
    assert_eq!(definition_query.candidates[0].role, SymbolRole::Definition);
}
