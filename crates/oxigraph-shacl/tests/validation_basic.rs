//! Basic SHACL validation tests.
#![expect(clippy::tests_outside_test_module)]

use oxigraph::io::RdfFormat;
use oxigraph::store::Store;
use oxigraph_shacl::shapes::CompiledShapes;
use oxigraph_shacl::validator::{ShaclMode, ShaclValidator};

const PERSON_SHAPES: &str = "
    @prefix sh: <http://www.w3.org/ns/shacl#> .
    @prefix ex: <http://example.org/> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

    ex:PersonShape a sh:NodeShape ;
        sh:targetClass ex:Person ;
        sh:property [
            sh:path ex:name ;
            sh:datatype xsd:string ;
            sh:minCount 1 ;
            sh:maxCount 1 ;
        ] .
";

fn make_validator(mode: ShaclMode) -> ShaclValidator {
    let shapes = CompiledShapes::from_turtle(PERSON_SHAPES).unwrap();
    let mut v = ShaclValidator::new(mode);
    v.set_shapes(shapes);
    v
}

fn store_with_data(turtle: &str) -> Store {
    let store = Store::new().unwrap();
    store
        .load_from_reader(RdfFormat::Turtle, turtle.as_bytes())
        .unwrap();
    store
}

#[test]
fn valid_data_passes() {
    let v = make_validator(ShaclMode::Enforce);
    let store = store_with_data(
        r#"
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
        ex:alice a ex:Person ; ex:name "Alice"^^xsd:string .
    "#,
    );
    assert!(v.validate(&store).unwrap().is_passed());
}

#[test]
fn invalid_data_fails() {
    let v = make_validator(ShaclMode::Enforce);
    let store = store_with_data(
        "
        @prefix ex: <http://example.org/> .
        ex:bob a ex:Person .
    ",
    );
    assert!(v.validate(&store).unwrap().is_failed());
}

#[test]
fn empty_store_passes() {
    let v = make_validator(ShaclMode::Enforce);
    let store = Store::new().unwrap();
    assert!(v.validate(&store).unwrap().is_passed());
}

#[test]
fn off_mode_skips() {
    let v = make_validator(ShaclMode::Off);
    let store = store_with_data(
        "
        @prefix ex: <http://example.org/> .
        ex:bob a ex:Person .
    ",
    );
    assert!(v.validate(&store).unwrap().is_skipped());
}

#[test]
fn warn_mode_still_validates() {
    let v = make_validator(ShaclMode::Warn);
    let store = store_with_data(
        "
        @prefix ex: <http://example.org/> .
        ex:bob a ex:Person .
    ",
    );
    // Warn mode still runs validation; the caller decides what to do
    assert!(v.validate(&store).unwrap().is_failed());
}

#[test]
fn no_shapes_returns_error() {
    let v = ShaclValidator::new(ShaclMode::Enforce);
    let store = Store::new().unwrap();
    v.validate(&store).unwrap_err();
}
