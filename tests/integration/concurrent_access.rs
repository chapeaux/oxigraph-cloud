//! Concurrent access tests.

use oxigraph::io::RdfFormat;
use oxigraph::sparql::{QueryResults, SparqlEvaluator};
use oxigraph::store::Store;
use std::sync::Arc;
use std::thread;

#[test]
fn concurrent_readers() {
    let store = Store::new().unwrap();
    let data = r#"
        @prefix ex: <http://example.org/> .
        ex:a ex:p ex:b .
        ex:c ex:p ex:d .
    "#;
    store
        .load_from_reader(RdfFormat::Turtle, data.as_bytes())
        .unwrap();

    let store = Arc::new(store);
    let mut handles = vec![];

    for _ in 0..8 {
        let s = store.clone();
        handles.push(thread::spawn(move || {
            let evaluator = SparqlEvaluator::new();
            let prepared = evaluator
                .parse_query("SELECT * WHERE { ?s ?p ?o }")
                .unwrap();
            match prepared.on_store(&s).execute().unwrap() {
                QueryResults::Solutions(solutions) => {
                    let count = solutions.count();
                    assert_eq!(count, 2);
                }
                _ => panic!("Expected solutions"),
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn concurrent_readers_and_writer() {
    let store = Arc::new(Store::new().unwrap());

    // Writer thread
    let writer_store = store.clone();
    let writer = thread::spawn(move || {
        for i in 0..100 {
            let nt = format!(
                "<http://ex.org/s{i}> <http://ex.org/p> <http://ex.org/o{i}> .\n"
            );
            writer_store
                .load_from_reader(RdfFormat::NTriples, nt.as_bytes())
                .unwrap();
        }
    });

    // Reader threads (may see partial writes, but should not crash)
    let mut readers = vec![];
    for _ in 0..4 {
        let s = store.clone();
        readers.push(thread::spawn(move || {
            for _ in 0..10 {
                let evaluator = SparqlEvaluator::new();
                let prepared = evaluator
                    .parse_query("SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }")
                    .unwrap();
                let _ = prepared.on_store(&s).execute().unwrap();
            }
        }));
    }

    writer.join().unwrap();
    for r in readers {
        r.join().unwrap();
    }
}
