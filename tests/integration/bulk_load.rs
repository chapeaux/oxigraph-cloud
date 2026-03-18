//! Bulk load integration tests.

use oxigraph::io::RdfFormat;
use oxigraph::sparql::{QueryResults, SparqlEvaluator};
use oxigraph::store::Store;

fn count_triples(store: &Store) -> usize {
    let evaluator = SparqlEvaluator::new();
    let prepared = evaluator
        .parse_query("SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }")
        .unwrap();
    match prepared.on_store(store).execute().unwrap() {
        QueryResults::Solutions(mut solutions) => {
            let solution = solutions.next().unwrap().unwrap();
            solution[0]
                .as_ref()
                .unwrap()
                .to_string()
                .trim_matches('"')
                .split("^^")
                .next()
                .unwrap()
                .trim_matches('"')
                .parse::<usize>()
                .unwrap()
        }
        _ => panic!("Expected solutions"),
    }
}

#[test]
fn load_sample_dataset() {
    let store = Store::new().unwrap();
    let data = include_str!("../data/sample-dataset.ttl");
    store
        .load_from_reader(RdfFormat::Turtle, data.as_bytes())
        .unwrap();
    let count = count_triples(&store);
    assert!(count > 20, "Expected at least 20 triples, got {count}");
}

#[test]
fn load_ntriples_format() {
    let store = Store::new().unwrap();
    let nt = "<http://ex.org/a> <http://ex.org/p> <http://ex.org/b> .\n\
              <http://ex.org/c> <http://ex.org/p> <http://ex.org/d> .\n";
    store
        .load_from_reader(RdfFormat::NTriples, nt.as_bytes())
        .unwrap();
    assert_eq!(count_triples(&store), 2);
}

#[test]
fn load_1k_triples() {
    let store = Store::new().unwrap();
    let mut nt = String::new();
    for i in 0..1000 {
        nt.push_str(&format!(
            "<http://ex.org/s{i}> <http://ex.org/p> <http://ex.org/o{i}> .\n"
        ));
    }
    store
        .load_from_reader(RdfFormat::NTriples, nt.as_bytes())
        .unwrap();
    assert_eq!(count_triples(&store), 1000);
}
