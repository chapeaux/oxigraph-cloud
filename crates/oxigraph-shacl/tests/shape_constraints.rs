//! Tests for specific SHACL constraint types.

use oxigraph::io::RdfFormat;
use oxigraph::store::Store;
use oxigraph_shacl::shapes::CompiledShapes;
use oxigraph_shacl::validator::{ShaclMode, ShaclValidator, ValidationOutcome};

fn validate(shapes_turtle: &str, data_turtle: &str) -> ValidationOutcome {
    let shapes = CompiledShapes::from_turtle(shapes_turtle).unwrap();
    let mut v = ShaclValidator::new(ShaclMode::Enforce);
    v.set_shapes(shapes);

    let store = Store::new().unwrap();
    store
        .load_from_reader(RdfFormat::Turtle, data_turtle.as_bytes())
        .unwrap();
    v.validate(&store).unwrap()
}

#[test]
fn cardinality_min_count() {
    let shapes = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        ex:S a sh:NodeShape ; sh:targetClass ex:T ;
            sh:property [ sh:path ex:p ; sh:minCount 2 ] .
    "#;
    // Only 1 value — should fail
    let data = r#"
        @prefix ex: <http://example.org/> .
        ex:x a ex:T ; ex:p "one" .
    "#;
    assert!(validate(shapes, data).is_failed());
}

#[test]
fn cardinality_max_count() {
    let shapes = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        ex:S a sh:NodeShape ; sh:targetClass ex:T ;
            sh:property [ sh:path ex:p ; sh:maxCount 1 ] .
    "#;
    // 2 values — should fail
    let data = r#"
        @prefix ex: <http://example.org/> .
        ex:x a ex:T ; ex:p "one", "two" .
    "#;
    assert!(validate(shapes, data).is_failed());
}

#[test]
fn datatype_constraint() {
    let shapes = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
        ex:S a sh:NodeShape ; sh:targetClass ex:T ;
            sh:property [ sh:path ex:age ; sh:datatype xsd:integer ] .
    "#;
    // String instead of integer — should fail
    let data = r#"
        @prefix ex: <http://example.org/> .
        ex:x a ex:T ; ex:age "not a number" .
    "#;
    assert!(validate(shapes, data).is_failed());
}

#[test]
fn class_constraint_pass() {
    let shapes = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        ex:S a sh:NodeShape ; sh:targetClass ex:T ;
            sh:property [ sh:path ex:ref ; sh:class ex:Other ] .
    "#;
    let data = r#"
        @prefix ex: <http://example.org/> .
        ex:x a ex:T ; ex:ref ex:y .
        ex:y a ex:Other .
    "#;
    assert!(validate(shapes, data).is_passed());
}
