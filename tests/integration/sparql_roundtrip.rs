//! SPARQL roundtrip integration tests.
//!
//! Tests that data inserted via SPARQL UPDATE can be queried back correctly.

use oxigraph::io::RdfFormat;
use oxigraph::sparql::{QueryResults, SparqlEvaluator};
use oxigraph::store::Store;

fn query_count(store: &Store, sparql: &str) -> usize {
    let evaluator = SparqlEvaluator::new();
    let prepared = evaluator.parse_query(sparql).unwrap();
    let results = prepared.on_store(store).execute().unwrap();
    match results {
        QueryResults::Solutions(solutions) => solutions.count(),
        _ => panic!("Expected solutions"),
    }
}

#[test]
fn insert_and_query_roundtrip() {
    let store = Store::new().unwrap();

    // Insert via SPARQL UPDATE
    let evaluator = SparqlEvaluator::new();
    let update = evaluator
        .parse_update(
            "INSERT DATA { <http://ex.org/s> <http://ex.org/p> <http://ex.org/o> }",
        )
        .unwrap();
    update.on_store(&store).execute().unwrap();

    // Query back
    let count = query_count(&store, "SELECT * WHERE { ?s ?p ?o }");
    assert_eq!(count, 1);
}

#[test]
fn load_turtle_and_query() {
    let store = Store::new().unwrap();
    let data = r#"
        @prefix ex: <http://example.org/> .
        ex:alice ex:knows ex:bob .
        ex:bob ex:knows ex:carol .
    "#;
    store
        .load_from_reader(RdfFormat::Turtle, data.as_bytes())
        .unwrap();

    let count = query_count(
        &store,
        "SELECT * WHERE { ?s <http://example.org/knows> ?o }",
    );
    assert_eq!(count, 2);
}

#[test]
fn delete_and_verify() {
    let store = Store::new().unwrap();

    let evaluator = SparqlEvaluator::new();
    let insert = evaluator
        .parse_update(
            "INSERT DATA {
            <http://ex.org/a> <http://ex.org/p> <http://ex.org/b> .
            <http://ex.org/c> <http://ex.org/p> <http://ex.org/d> .
        }",
        )
        .unwrap();
    insert.on_store(&store).execute().unwrap();
    assert_eq!(query_count(&store, "SELECT * WHERE { ?s ?p ?o }"), 2);

    let delete = evaluator
        .parse_update(
            "DELETE DATA { <http://ex.org/a> <http://ex.org/p> <http://ex.org/b> }",
        )
        .unwrap();
    delete.on_store(&store).execute().unwrap();
    assert_eq!(query_count(&store, "SELECT * WHERE { ?s ?p ?o }"), 1);
}

#[test]
fn filter_query() {
    let store = Store::new().unwrap();
    let data = r#"
        @prefix ex: <http://example.org/> .
        ex:a ex:value 10 .
        ex:b ex:value 20 .
        ex:c ex:value 30 .
    "#;
    store
        .load_from_reader(RdfFormat::Turtle, data.as_bytes())
        .unwrap();

    let count = query_count(
        &store,
        "SELECT * WHERE { ?s <http://example.org/value> ?v . FILTER(?v > 15) }",
    );
    assert_eq!(count, 2);
}
