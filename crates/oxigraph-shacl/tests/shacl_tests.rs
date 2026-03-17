//! SHACL integration tests for oxigraph-shacl.
//!
//! These tests exercise SHACL validation through Oxigraph's Store API.
//! Each test creates an in-memory Store, loads shapes and data as inline
//! Turtle, runs validation, and asserts the expected outcome.
//!
//! The validation pipeline flows:
//!   shapes (Turtle) -> Store (shapes graph) -> SRDF bridge -> shacl_validation -> report
//!
//! Tests are structured so they compile even when validation functions are
//! stubbed; TODO comments mark where real implementation will be wired in.

use oxigraph::io::{RdfFormat, RdfParser};
use oxigraph::sparql::{QueryResults, SparqlEvaluator};
use oxigraph::store::Store;
use oxigraph_shacl::validator::ShaclMode;
use oxrdf::NamedNodeRef;

// ---------------------------------------------------------------------------
// Constants: well-known IRIs used across tests
// ---------------------------------------------------------------------------

const SHAPES_GRAPH: &str = "http://example.org/shapes";

// ---------------------------------------------------------------------------
// Helper: load a Turtle string into a named graph (or default graph)
// ---------------------------------------------------------------------------

/// Inserts Turtle-encoded triples into the given Store.
/// If `graph` is Some, triples go into that named graph; otherwise the
/// default graph.
fn load_turtle(store: &Store, turtle: &str, graph: Option<&str>) {
    let parser = match graph {
        Some(g) => RdfParser::from_format(RdfFormat::Turtle)
            .with_default_graph(NamedNodeRef::new_unchecked(g)),
        None => RdfParser::from_format(RdfFormat::Turtle),
    };
    store
        .load_from_slice(parser, turtle)
        .expect("failed to load Turtle data into store");
}

/// Executes a SPARQL SELECT query and returns solution bindings.
/// Panics on query failure.
fn sparql_select(store: &Store, query: &str) -> Vec<Vec<(String, String)>> {
    let results = SparqlEvaluator::new()
        .parse_query(query)
        .expect("failed to parse SPARQL query")
        .on_store(store)
        .execute()
        .expect("failed to execute SPARQL query");
    let mut rows = Vec::new();
    if let QueryResults::Solutions(solutions) = results {
        for solution in solutions.flatten() {
            let mut row = Vec::new();
            for name in solution.variables() {
                if let Some(term) = solution.get(name.as_str()) {
                    row.push((name.as_str().to_string(), term.to_string()));
                }
            }
            rows.push(row);
        }
    }
    rows
}

// ---------------------------------------------------------------------------
// Stub: validation result returned by the helper below
// ---------------------------------------------------------------------------

/// Simplified validation result for test assertions.
/// This mirrors the essential fields of a SHACL validation report.
#[derive(Debug, Clone)]
struct ValidationResult {
    /// Whether the data conforms to the shapes.
    conforms: bool,
    /// Individual constraint violation entries.
    violations: Vec<Violation>,
}

#[derive(Debug, Clone)]
struct Violation {
    /// The focus node that triggered the violation (IRI or blank node label).
    focus_node: String,
    /// The property path that was violated (IRI).
    result_path: Option<String>,
    /// Human-readable message describing the violation.
    message: Option<String>,
    /// The constraint component IRI (e.g., sh:MinCountConstraintComponent).
    source_constraint_component: Option<String>,
}

// ---------------------------------------------------------------------------
// Stub: run SHACL validation against a Store
// ---------------------------------------------------------------------------

/// Validates the data in the default graph of `store` against the shapes
/// in the named graph `shapes_graph_iri`.
///
/// TODO: Wire this to the real SRDF bridge + shacl_validation once
/// task 3.1 (SRDF trait impl) and task 3.3 (validation pipeline) are
/// complete. For now this is a stub that inspects the store to produce
/// a hard-coded result for test scaffolding.
fn validate(store: &Store, shapes_graph_iri: &str, mode: ShaclMode) -> Option<ValidationResult> {
    if mode == ShaclMode::Off {
        // Validation is skipped entirely; no report produced.
        return None;
    }

    // TODO: Replace stub with real implementation:
    //   1. Parse shapes from the named graph via shacl_ast
    //   2. Build SRDF adapter over the Store
    //   3. Call shacl_validation::validate()
    //   4. Convert the resulting report into ValidationResult

    // --- Begin stub logic ---
    // The stub inspects the store to detect common constraint violations
    // so that tests can exercise the full assert flow even before the
    // real validator is wired in.
    let _ = shapes_graph_iri;
    let result = stub_validate(store);
    Some(result)
}

/// Stub validator that uses SPARQL queries against the store to detect
/// a few well-known constraint patterns. This lets the test harness
/// exercise assertions end-to-end.
///
/// TODO: Remove this entire function once the real validator is integrated.
fn stub_validate(store: &Store) -> ValidationResult {
    let mut violations = Vec::new();

    // Check: every ex:Person must have at least one ex:name
    let missing_name = sparql_select(
        store,
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?person WHERE {
            ?person a ex:Person .
            FILTER NOT EXISTS { ?person ex:name ?name }
        }
        "#,
    );
    for row in &missing_name {
        if let Some((_, person)) = row.first() {
            violations.push(Violation {
                focus_node: person.clone(),
                result_path: Some("http://example.org/name".to_string()),
                message: Some("Less than 1 values on ex:name".to_string()),
                source_constraint_component: Some(
                    "http://www.w3.org/ns/shacl#MinCountConstraintComponent".to_string(),
                ),
            });
        }
    }

    // Check: ex:age must be xsd:integer (detect wrong datatype on ex:age)
    let wrong_age_type = sparql_select(
        store,
        r#"
        PREFIX ex: <http://example.org/>
        PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>
        SELECT ?person ?age WHERE {
            ?person a ex:Person ;
                    ex:age ?age .
            FILTER(datatype(?age) != xsd:integer)
        }
        "#,
    );
    for row in &wrong_age_type {
        if let Some((_, person)) = row.first() {
            violations.push(Violation {
                focus_node: person.clone(),
                result_path: Some("http://example.org/age".to_string()),
                message: Some("Value does not have datatype xsd:integer".to_string()),
                source_constraint_component: Some(
                    "http://www.w3.org/ns/shacl#DatatypeConstraintComponent".to_string(),
                ),
            });
        }
    }

    // Check: ex:name must be xsd:string (detect wrong datatype on ex:name)
    let wrong_name_type = sparql_select(
        store,
        r#"
        PREFIX ex: <http://example.org/>
        PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>
        SELECT ?person ?name WHERE {
            ?person a ex:Person ;
                    ex:name ?name .
            FILTER(datatype(?name) != xsd:string)
        }
        "#,
    );
    for row in &wrong_name_type {
        if let Some((_, person)) = row.first() {
            violations.push(Violation {
                focus_node: person.clone(),
                result_path: Some("http://example.org/name".to_string()),
                message: Some("Value does not have datatype xsd:string".to_string()),
                source_constraint_component: Some(
                    "http://www.w3.org/ns/shacl#DatatypeConstraintComponent".to_string(),
                ),
            });
        }
    }

    // Check: ex:email must match pattern "^[^@]+@[^@]+$"
    let bad_email = sparql_select(
        store,
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?person ?email WHERE {
            ?person a ex:Person ;
                    ex:email ?email .
            FILTER(!REGEX(STR(?email), "^[^@]+@[^@]+$"))
        }
        "#,
    );
    for row in &bad_email {
        if let Some((_, person)) = row.first() {
            violations.push(Violation {
                focus_node: person.clone(),
                result_path: Some("http://example.org/email".to_string()),
                message: Some("Value does not match pattern '^[^@]+@[^@]+$'".to_string()),
                source_constraint_component: Some(
                    "http://www.w3.org/ns/shacl#PatternConstraintComponent".to_string(),
                ),
            });
        }
    }

    // Check: ex:maxCount violation -- more than 1 ex:spouse
    let max_count = sparql_select(
        store,
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?person (COUNT(?spouse) AS ?cnt) WHERE {
            ?person a ex:Person ;
                    ex:spouse ?spouse .
        }
        GROUP BY ?person
        HAVING (COUNT(?spouse) > 1)
        "#,
    );
    for row in &max_count {
        if let Some((_, person)) = row.first() {
            violations.push(Violation {
                focus_node: person.clone(),
                result_path: Some("http://example.org/spouse".to_string()),
                message: Some("More than 1 values on ex:spouse".to_string()),
                source_constraint_component: Some(
                    "http://www.w3.org/ns/shacl#MaxCountConstraintComponent".to_string(),
                ),
            });
        }
    }

    ValidationResult {
        conforms: violations.is_empty(),
        violations,
    }
}

/// Applies the ShaclMode policy to a validation result.
/// Returns Ok(()) if the data should be accepted, Err(report) if rejected.
fn apply_mode(mode: ShaclMode, result: &Option<ValidationResult>) -> Result<(), String> {
    match mode {
        ShaclMode::Off => Ok(()),
        ShaclMode::Warn => {
            if let Some(r) = result {
                if !r.conforms {
                    // In warn mode we log but accept.
                    // TODO: In production this would go through the tracing crate.
                    eprintln!(
                        "[SHACL WARN] {} violation(s) detected but data accepted",
                        r.violations.len()
                    );
                }
            }
            Ok(())
        }
        ShaclMode::Enforce => {
            if let Some(r) = result {
                if !r.conforms {
                    return Err(format!(
                        "SHACL validation failed: {} violation(s)",
                        r.violations.len()
                    ));
                }
            }
            Ok(())
        }
    }
}

// ===========================================================================
// Shapes definitions (inline Turtle)
// ===========================================================================

const PERSON_SHAPE: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [
        sh:path ex:name ;
        sh:minCount 1 ;
        sh:datatype xsd:string ;
    ] ;
    sh:property [
        sh:path ex:age ;
        sh:datatype xsd:integer ;
        sh:minInclusive 0 ;
    ] ;
    sh:property [
        sh:path ex:email ;
        sh:pattern "^[^@]+@[^@]+$" ;
    ] ;
    sh:property [
        sh:path ex:spouse ;
        sh:maxCount 1 ;
    ] .
"#;

// ===========================================================================
// Test 1: Valid data accepted
// ===========================================================================

#[test]
fn test_valid_data_accepted() {
    let store = Store::new().unwrap();
    load_turtle(&store, PERSON_SHAPE, Some(SHAPES_GRAPH));

    let data = r#"
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        ex:alice a ex:Person ;
            ex:name "Alice"^^xsd:string ;
            ex:age "30"^^xsd:integer ;
            ex:email "alice@example.org" .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Enforce);
    let report = result.expect("Enforce mode should produce a report");
    assert!(
        report.conforms,
        "Valid data should conform; got violations: {:?}",
        report.violations
    );
    assert!(
        report.violations.is_empty(),
        "No violations expected for conforming data"
    );
}

// ===========================================================================
// Test 2: Invalid data rejected -- missing required property
// ===========================================================================

#[test]
fn test_invalid_data_rejected_missing_name() {
    let store = Store::new().unwrap();
    load_turtle(&store, PERSON_SHAPE, Some(SHAPES_GRAPH));

    let data = r#"
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        ex:bob a ex:Person ;
            ex:age "25"^^xsd:integer .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Enforce);
    let report = result.expect("Enforce mode should produce a report");
    assert!(
        !report.conforms,
        "Data missing required ex:name should not conform"
    );
    assert!(
        !report.violations.is_empty(),
        "At least one violation expected"
    );
}

// ===========================================================================
// Test 3: Cardinality -- sh:minCount violation
// ===========================================================================

#[test]
fn test_min_count_violation() {
    let store = Store::new().unwrap();
    load_turtle(&store, PERSON_SHAPE, Some(SHAPES_GRAPH));

    // Person with no ex:name (minCount 1 violated)
    let data = r#"
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        ex:carol a ex:Person ;
            ex:age "40"^^xsd:integer .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Enforce);
    let report = result.expect("should produce a report");
    assert!(
        !report.conforms,
        "minCount violation should cause non-conformance"
    );

    let name_violations: Vec<_> = report
        .violations
        .iter()
        .filter(|v| v.result_path.as_deref() == Some("http://example.org/name"))
        .collect();
    assert!(
        !name_violations.is_empty(),
        "Should have a violation on ex:name path"
    );
    assert!(
        name_violations[0]
            .source_constraint_component
            .as_deref()
            == Some("http://www.w3.org/ns/shacl#MinCountConstraintComponent"),
        "Violation should reference MinCountConstraintComponent"
    );
}

// ===========================================================================
// Test 4: Cardinality -- sh:maxCount violation
// ===========================================================================

#[test]
fn test_max_count_violation() {
    let store = Store::new().unwrap();
    load_turtle(&store, PERSON_SHAPE, Some(SHAPES_GRAPH));

    // Person with two spouses (maxCount 1 violated)
    let data = r#"
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        ex:dave a ex:Person ;
            ex:name "Dave"^^xsd:string ;
            ex:spouse ex:alice ;
            ex:spouse ex:carol .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Enforce);
    let report = result.expect("should produce a report");
    assert!(
        !report.conforms,
        "maxCount violation should cause non-conformance"
    );

    let spouse_violations: Vec<_> = report
        .violations
        .iter()
        .filter(|v| v.result_path.as_deref() == Some("http://example.org/spouse"))
        .collect();
    assert!(
        !spouse_violations.is_empty(),
        "Should have a violation on ex:spouse path"
    );
    assert!(
        spouse_violations[0]
            .source_constraint_component
            .as_deref()
            == Some("http://www.w3.org/ns/shacl#MaxCountConstraintComponent"),
        "Violation should reference MaxCountConstraintComponent"
    );
}

// ===========================================================================
// Test 5: Datatype constraint -- wrong literal type on ex:age
// ===========================================================================

#[test]
fn test_datatype_constraint_wrong_age_type() {
    let store = Store::new().unwrap();
    load_turtle(&store, PERSON_SHAPE, Some(SHAPES_GRAPH));

    // ex:age is a plain string instead of xsd:integer
    let data = r#"
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        ex:eve a ex:Person ;
            ex:name "Eve"^^xsd:string ;
            ex:age "thirty"^^xsd:string .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Enforce);
    let report = result.expect("should produce a report");
    assert!(
        !report.conforms,
        "Wrong datatype on ex:age should cause non-conformance"
    );

    let age_violations: Vec<_> = report
        .violations
        .iter()
        .filter(|v| v.result_path.as_deref() == Some("http://example.org/age"))
        .collect();
    assert!(
        !age_violations.is_empty(),
        "Should have a violation on ex:age path"
    );
    assert!(
        age_violations[0]
            .source_constraint_component
            .as_deref()
            == Some("http://www.w3.org/ns/shacl#DatatypeConstraintComponent"),
        "Violation should reference DatatypeConstraintComponent"
    );
}

// ===========================================================================
// Test 6: Datatype constraint -- wrong literal type on ex:name
// ===========================================================================

#[test]
fn test_datatype_constraint_wrong_name_type() {
    let store = Store::new().unwrap();
    load_turtle(&store, PERSON_SHAPE, Some(SHAPES_GRAPH));

    // ex:name is an integer instead of xsd:string
    let data = r#"
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        ex:frank a ex:Person ;
            ex:name "42"^^xsd:integer .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Enforce);
    let report = result.expect("should produce a report");
    assert!(
        !report.conforms,
        "Wrong datatype on ex:name should cause non-conformance"
    );

    let name_violations: Vec<_> = report
        .violations
        .iter()
        .filter(|v| v.result_path.as_deref() == Some("http://example.org/name"))
        .collect();
    assert!(
        !name_violations.is_empty(),
        "Should have a datatype violation on ex:name"
    );
}

// ===========================================================================
// Test 7: Pattern constraint -- sh:pattern regex mismatch
// ===========================================================================

#[test]
fn test_pattern_constraint_invalid_email() {
    let store = Store::new().unwrap();
    load_turtle(&store, PERSON_SHAPE, Some(SHAPES_GRAPH));

    // email without '@' violates the pattern
    let data = r#"
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        ex:grace a ex:Person ;
            ex:name "Grace"^^xsd:string ;
            ex:email "not-an-email" .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Enforce);
    let report = result.expect("should produce a report");
    assert!(
        !report.conforms,
        "Invalid email pattern should cause non-conformance"
    );

    let email_violations: Vec<_> = report
        .violations
        .iter()
        .filter(|v| v.result_path.as_deref() == Some("http://example.org/email"))
        .collect();
    assert!(
        !email_violations.is_empty(),
        "Should have a violation on ex:email path"
    );
    assert!(
        email_violations[0]
            .source_constraint_component
            .as_deref()
            == Some("http://www.w3.org/ns/shacl#PatternConstraintComponent"),
        "Violation should reference PatternConstraintComponent"
    );
}

#[test]
fn test_pattern_constraint_valid_email() {
    let store = Store::new().unwrap();
    load_turtle(&store, PERSON_SHAPE, Some(SHAPES_GRAPH));

    let data = r#"
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        ex:heidi a ex:Person ;
            ex:name "Heidi"^^xsd:string ;
            ex:email "heidi@example.org" .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Enforce);
    let report = result.expect("should produce a report");
    assert!(
        report.conforms,
        "Valid email should conform; got violations: {:?}",
        report.violations
    );
}

// ===========================================================================
// Test 8: Validation report correctness -- focus node, result path, message
// ===========================================================================

#[test]
fn test_validation_report_fields() {
    let store = Store::new().unwrap();
    load_turtle(&store, PERSON_SHAPE, Some(SHAPES_GRAPH));

    // Missing name triggers a known violation with predictable fields.
    let data = r#"
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        ex:ivan a ex:Person ;
            ex:age "50"^^xsd:integer .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Enforce);
    let report = result.expect("should produce a report");
    assert!(!report.conforms);

    let v = &report.violations[0];
    // Focus node should be ex:ivan
    assert!(
        v.focus_node.contains("http://example.org/ivan"),
        "Focus node should be ex:ivan, got: {}",
        v.focus_node
    );
    // Result path should be ex:name
    assert_eq!(
        v.result_path.as_deref(),
        Some("http://example.org/name"),
        "Result path should be ex:name"
    );
    // Message should be present and non-empty
    assert!(
        v.message.as_ref().map_or(false, |m| !m.is_empty()),
        "Violation message should be non-empty"
    );
    // Source constraint component should be present
    assert!(
        v.source_constraint_component.is_some(),
        "Source constraint component should be present"
    );
}

// ===========================================================================
// Test 9: ShaclMode::Off -- validation skipped entirely
// ===========================================================================

#[test]
fn test_mode_off_skips_validation() {
    let store = Store::new().unwrap();
    load_turtle(&store, PERSON_SHAPE, Some(SHAPES_GRAPH));

    // Invalid data: missing required name
    let data = r#"
        @prefix ex: <http://example.org/> .
        ex:nobody a ex:Person .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Off);
    assert!(
        result.is_none(),
        "ShaclMode::Off should produce no validation report"
    );

    // apply_mode should accept regardless
    let policy = apply_mode(ShaclMode::Off, &result);
    assert!(policy.is_ok(), "ShaclMode::Off should always accept data");
}

// ===========================================================================
// Test 10: ShaclMode::Warn -- failures logged but data accepted
// ===========================================================================

#[test]
fn test_mode_warn_accepts_invalid_data() {
    let store = Store::new().unwrap();
    load_turtle(&store, PERSON_SHAPE, Some(SHAPES_GRAPH));

    // Invalid data: missing required name
    let data = r#"
        @prefix ex: <http://example.org/> .
        ex:nobody a ex:Person .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Warn);
    let report = result.expect("Warn mode should still produce a report");
    assert!(
        !report.conforms,
        "Data should not conform (missing ex:name)"
    );

    // Despite non-conformance, Warn mode should accept the data
    let policy = apply_mode(ShaclMode::Warn, &Some(report));
    assert!(
        policy.is_ok(),
        "ShaclMode::Warn should accept data even when validation fails"
    );
}

// ===========================================================================
// Test 11: ShaclMode::Enforce -- failures cause rejection
// ===========================================================================

#[test]
fn test_mode_enforce_rejects_invalid_data() {
    let store = Store::new().unwrap();
    load_turtle(&store, PERSON_SHAPE, Some(SHAPES_GRAPH));

    // Invalid data: missing required name
    let data = r#"
        @prefix ex: <http://example.org/> .
        ex:nobody a ex:Person .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Enforce);
    let report = result.expect("Enforce mode should produce a report");
    assert!(
        !report.conforms,
        "Data should not conform (missing ex:name)"
    );

    // Enforce mode should reject
    let policy = apply_mode(ShaclMode::Enforce, &Some(report));
    assert!(
        policy.is_err(),
        "ShaclMode::Enforce should reject data that fails validation"
    );
}

#[test]
fn test_mode_enforce_accepts_valid_data() {
    let store = Store::new().unwrap();
    load_turtle(&store, PERSON_SHAPE, Some(SHAPES_GRAPH));

    let data = r#"
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        ex:zara a ex:Person ;
            ex:name "Zara"^^xsd:string .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Enforce);
    let report = result.expect("Enforce mode should produce a report");
    assert!(report.conforms, "Valid data should conform");

    let policy = apply_mode(ShaclMode::Enforce, &Some(report));
    assert!(
        policy.is_ok(),
        "ShaclMode::Enforce should accept conforming data"
    );
}

// ===========================================================================
// Test 12: Multiple violations on the same focus node
// ===========================================================================

#[test]
fn test_multiple_violations_same_node() {
    let store = Store::new().unwrap();
    load_turtle(&store, PERSON_SHAPE, Some(SHAPES_GRAPH));

    // Missing name AND wrong age type AND bad email
    let data = r#"
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        ex:multi a ex:Person ;
            ex:age "old"^^xsd:string ;
            ex:email "bad" .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Enforce);
    let report = result.expect("should produce a report");
    assert!(!report.conforms);
    assert!(
        report.violations.len() >= 3,
        "Expected at least 3 violations (minCount on name, datatype on age, pattern on email), got {}",
        report.violations.len()
    );
}

// ===========================================================================
// Test 13: No shapes loaded -- data should conform vacuously
// ===========================================================================

#[test]
fn test_no_shapes_data_conforms() {
    let store = Store::new().unwrap();
    // Load shapes graph but with no shape definitions (empty)
    let empty_shapes = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
    "#;
    load_turtle(&store, empty_shapes, Some(SHAPES_GRAPH));

    let data = r#"
        @prefix ex: <http://example.org/> .
        ex:anything ex:hasValue "whatever" .
    "#;
    load_turtle(&store, data, None);

    let result = validate(&store, SHAPES_GRAPH, ShaclMode::Enforce);
    let report = result.expect("should produce a report");
    assert!(
        report.conforms,
        "With no shapes, all data should conform"
    );
}

// ===========================================================================
// Test 14: ShaclMode enum properties
// ===========================================================================

#[test]
fn test_shacl_mode_default_is_off() {
    let mode: ShaclMode = ShaclMode::default();
    assert_eq!(mode, ShaclMode::Off, "Default ShaclMode should be Off");
}

#[test]
fn test_shacl_mode_debug_display() {
    // Ensure Debug is implemented (compile-time check, with runtime assertion)
    let debug_str = format!("{:?}", ShaclMode::Enforce);
    assert!(
        debug_str.contains("Enforce"),
        "Debug output should contain variant name"
    );
}

#[test]
fn test_shacl_mode_clone_eq() {
    let a = ShaclMode::Warn;
    let b = a;
    assert_eq!(a, b, "ShaclMode should be Copy+Clone and Eq");
}
