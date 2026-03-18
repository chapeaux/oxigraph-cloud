#![allow(clippy::panic, clippy::expect_used)]

use criterion::{Criterion, criterion_group, criterion_main};
use oxigraph::io::{RdfFormat, RdfParser};
use oxigraph::model::*;
use oxigraph::sparql::{QueryResults, SparqlEvaluator};
use oxigraph::store::Store;
use oxigraph_shacl::shapes::CompiledShapes;
use oxigraph_shacl::validator::{ShaclMode, ShaclValidator};

// ---------------------------------------------------------------------------
// Helper: generate N-Triples data for a given number of triples
// ---------------------------------------------------------------------------
fn generate_ntriples(count: usize) -> Vec<u8> {
    let mut buf = String::new();
    for i in 0..count {
        buf.push_str(&format!(
            "<http://example.org/s{i}> <http://example.org/p> <http://example.org/o{i}> .\n"
        ));
    }
    buf.into_bytes()
}

/// Create a fresh in-memory store pre-loaded with `count` triples.
fn populated_store(count: usize) -> Store {
    let store = Store::new().expect("failed to create store");
    let data = generate_ntriples(count);
    store
        .load_from_slice(RdfFormat::NTriples, &data)
        .expect("failed to load data");
    store
}

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

/// Measure latency of inserting a single triple into an empty store.
fn single_insert(c: &mut Criterion) {
    c.bench_function("single_insert", |b| {
        b.iter_with_setup(
            || Store::new().expect("failed to create store"),
            |store| {
                let s = NamedNodeRef::new("http://example.org/s").expect("invalid IRI");
                let p = NamedNodeRef::new("http://example.org/p").expect("invalid IRI");
                let o = NamedNodeRef::new("http://example.org/o").expect("invalid IRI");
                store
                    .insert(QuadRef::new(s, p, o, GraphNameRef::DefaultGraph))
                    .expect("insert failed");
            },
        );
    });
}

/// Bulk-load 1 000 triples from N-Triples into an in-memory store.
fn bulk_load_1k(c: &mut Criterion) {
    let data = generate_ntriples(1_000);
    c.bench_function("bulk_load_1k", |b| {
        b.iter_with_setup(
            || Store::new().expect("failed to create store"),
            |store| {
                store
                    .load_from_slice(RdfFormat::NTriples, &data)
                    .expect("load failed");
            },
        );
    });
}

/// Bulk-load 10 000 triples from N-Triples into an in-memory store.
fn bulk_load_10k(c: &mut Criterion) {
    let data = generate_ntriples(10_000);
    let mut group = c.benchmark_group("bulk_load_10k");
    group.sample_size(20);
    group.bench_function("load_from_slice", |b| {
        b.iter_with_setup(
            || Store::new().expect("failed to create store"),
            |store| {
                store
                    .load_from_slice(RdfFormat::NTriples, &data)
                    .expect("load failed");
            },
        );
    });
    group.bench_function("bulk_loader", |b| {
        b.iter_with_setup(
            || Store::new().expect("failed to create store"),
            |store| {
                let mut loader = store.bulk_loader();
                loader
                    .load_from_slice(
                        RdfParser::from_format(RdfFormat::NTriples).lenient(),
                        &data,
                    )
                    .expect("bulk load failed");
                loader.commit().expect("commit failed");
            },
        );
    });
    group.finish();
}

/// Point query: look up a specific triple by subject in a 10 000-triple store.
fn point_query(c: &mut Criterion) {
    let store = populated_store(10_000);
    let subject =
        NamedNodeRef::new("http://example.org/s5000").expect("invalid IRI");
    c.bench_function("point_query", |b| {
        b.iter(|| {
            let count = store
                .quads_for_pattern(
                    Some(subject.into()),
                    None,
                    None,
                    Some(GraphNameRef::DefaultGraph),
                )
                .count();
            assert!(count > 0, "expected at least one result");
        });
    });
}

/// Range scan: prefix scan returning the first 100 results from a 10 000-triple store.
fn range_scan(c: &mut Criterion) {
    let store = populated_store(10_000);
    c.bench_function("range_scan", |b| {
        b.iter(|| {
            let results: Vec<_> = store
                .quads_for_pattern(
                    None,
                    None,
                    None,
                    Some(GraphNameRef::DefaultGraph),
                )
                .take(100)
                .collect();
            assert_eq!(results.len(), 100, "expected 100 results");
        });
    });
}

/// SPARQL SELECT: run a simple SELECT query over a 10 000-triple store.
fn sparql_select(c: &mut Criterion) {
    let store = populated_store(10_000);
    c.bench_function("sparql_select", |b| {
        b.iter(|| {
            let results = SparqlEvaluator::new()
                .parse_query("SELECT ?s ?o WHERE { ?s <http://example.org/p> ?o } LIMIT 100")
                .expect("parse failed")
                .on_store(&store)
                .execute()
                .expect("query failed");
            if let QueryResults::Solutions(solutions) = results {
                let count = solutions.count();
                assert!(count > 0, "expected results");
            } else {
                panic!("expected solutions");
            }
        });
    });
}

/// SPARQL SELECT with FILTER: run a filtered query over a 10 000-triple store.
fn sparql_filter(c: &mut Criterion) {
    let store = populated_store(10_000);
    c.bench_function("sparql_filter", |b| {
        b.iter(|| {
            let results = SparqlEvaluator::new()
                .parse_query(
                    "SELECT ?s ?o WHERE { \
                        ?s <http://example.org/p> ?o . \
                        FILTER(?o = <http://example.org/o42>) \
                    }",
                )
                .expect("parse failed")
                .on_store(&store)
                .execute()
                .expect("query failed");
            if let QueryResults::Solutions(solutions) = results {
                let count = solutions.count();
                assert_eq!(count, 1, "expected exactly one result");
            } else {
                panic!("expected solutions");
            }
        });
    });
}

/// SHACL validation: validate store data against a simple shape.
fn shacl_validation(c: &mut Criterion) {
    // Build a store with typed data so the shape has something to validate.
    let store = Store::new().expect("failed to create store");
    let nt_data = r#"
<http://example.org/alice> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/Person> .
<http://example.org/alice> <http://example.org/name> "Alice" .
<http://example.org/alice> <http://example.org/age> "30"^^<http://www.w3.org/2001/XMLSchema#integer> .
<http://example.org/bob> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/Person> .
<http://example.org/bob> <http://example.org/name> "Bob" .
<http://example.org/bob> <http://example.org/age> "25"^^<http://www.w3.org/2001/XMLSchema#integer> .
"#;
    store
        .load_from_slice(RdfFormat::NTriples, nt_data)
        .expect("failed to load shacl test data");

    let shapes_turtle = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:PersonShape
    a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [
        sh:path ex:name ;
        sh:minCount 1 ;
        sh:datatype xsd:string ;
    ] ;
    sh:property [
        sh:path ex:age ;
        sh:minCount 1 ;
        sh:datatype xsd:integer ;
    ] .
"#;
    let shapes =
        CompiledShapes::from_turtle(shapes_turtle).expect("failed to compile shapes");

    c.bench_function("shacl_validation", |b| {
        b.iter_with_setup(
            || {
                let mut validator = ShaclValidator::new(ShaclMode::Enforce);
                validator.set_shapes(shapes.clone());
                validator
            },
            |validator| {
                let outcome = validator.validate(&store).expect("validation failed");
                assert!(
                    outcome.is_passed(),
                    "expected validation to pass"
                );
            },
        );
    });
}

criterion_group!(
    benches,
    single_insert,
    bulk_load_1k,
    bulk_load_10k,
    point_query,
    range_scan,
    sparql_select,
    sparql_filter,
    shacl_validation,
);
criterion_main!(benches);
