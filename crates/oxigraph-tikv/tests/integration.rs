//! Integration tests for the TiKV storage backend.
//!
//! These tests exercise the FULL SPARQL round-trip through Oxigraph's public
//! `Store` API, backed by a live TiKV cluster. They verify that the TiKV
//! backend correctly supports insert, query, delete, named graphs, bulk load,
//! transaction rollback, and concurrent reads.
//!
//! # Prerequisites
//!
//! - A running TiKV cluster (PD endpoint accessible)
//! - Feature flag `integration-tests` enabled
//! - Optionally set `TIKV_PD_ENDPOINTS` env var (defaults to "127.0.0.1:2379")
//!
//! # Running
//!
//! ```sh
//! cargo test -p oxigraph-tikv --features integration-tests -- --test-threads=1
//! ```

#![cfg(feature = "integration-tests")]

use oxigraph::model::*;
use oxigraph::sparql::{QueryResults, SparqlEvaluator};
use oxigraph::store::Store;
use std::sync::atomic::{AtomicU64, Ordering};

// ============================================================================
// Helper module
// ============================================================================

mod helpers {
    use oxigraph::sparql::SparqlEvaluator;
    use oxigraph::store::Store;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Global counter to generate unique test prefixes, ensuring no
    /// interference between tests running on a shared TiKV cluster.
    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Read the TiKV PD endpoints from the environment, falling back to localhost.
    pub fn pd_endpoints() -> Vec<String> {
        let raw =
            std::env::var("TIKV_PD_ENDPOINTS").unwrap_or_else(|_| "127.0.0.1:2379".to_string());
        raw.split(',').map(|s| s.trim().to_string()).collect()
    }

    /// Generate a unique IRI namespace for this test invocation.
    ///
    /// Each call returns a namespace like `http://test-<timestamp>-<counter>.example.org/`
    /// so that tests inserting triples into the same TiKV cluster do not
    /// collide with each other.
    pub fn unique_ns() -> String {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("http://test-{ts}-{id}.example.org/")
    }

    /// Attempt to open a TiKV-backed Store.
    ///
    /// Returns `None` if the TiKV cluster is unreachable (connection refused),
    /// allowing tests to skip gracefully rather than fail.
    ///
    /// TODO(tikv-backend): Replace the body of this function with the actual
    /// TiKV Store constructor once the backend is implemented. The expected
    /// API shape is one of:
    ///
    /// ```ignore
    /// Store::open_tikv(&pd_endpoints())
    /// ```
    ///
    /// or:
    ///
    /// ```ignore
    /// use oxigraph_tikv::TiKvConfig;
    /// Store::open_with_backend(TiKvConfig::new(pd_endpoints()))
    /// ```
    pub fn try_open_tikv_store() -> Option<Store> {
        let _endpoints = pd_endpoints();

        // TODO(tikv-backend): Uncomment and adapt once the TiKV constructor
        // exists. The real implementation should look like:
        //
        // match Store::open_tikv(&_endpoints) {
        //     Ok(store) => Some(store),
        //     Err(e) => {
        //         let msg = e.to_string();
        //         if msg.contains("connection refused")
        //             || msg.contains("Connection refused")
        //             || msg.contains("timed out")
        //             || msg.contains("Unavailable")
        //         {
        //             eprintln!("TiKV not available, skipping test: {msg}");
        //             None
        //         } else {
        //             panic!("Unexpected TiKV connection error: {e}");
        //         }
        //     }
        // }

        // Temporary: use in-memory store so the test file compiles and the
        // test structure can be validated. Once the TiKV backend lands, switch
        // to the real constructor above.
        match Store::new() {
            Ok(store) => Some(store),
            Err(e) => {
                eprintln!("Failed to create store: {e}");
                None
            }
        }
    }

    /// Execute a SPARQL UPDATE on a Store using the non-deprecated API.
    pub fn sparql_update(store: &Store, update: &str) {
        SparqlEvaluator::new()
            .parse_update(update)
            .expect("Failed to parse SPARQL UPDATE")
            .on_store(store)
            .execute()
            .expect("SPARQL UPDATE execution failed");
    }

    /// Open a TiKV-backed Store, skipping the test if TiKV is unavailable.
    ///
    /// Usage:
    /// ```ignore
    /// let store = require_tikv_store!();
    /// ```
    macro_rules! require_tikv_store {
        () => {
            match $crate::helpers::try_open_tikv_store() {
                Some(store) => store,
                None => {
                    eprintln!("SKIPPED: TiKV not available");
                    return;
                }
            }
        };
    }
    pub(crate) use require_tikv_store;
}

// ============================================================================
// Tests
// ============================================================================

/// Insert triples via SPARQL UPDATE and verify them via SPARQL SELECT.
#[test]
fn test_sparql_insert_and_select() {
    let store = helpers::require_tikv_store!();
    let ns = helpers::unique_ns();

    // Insert two triples into the default graph.
    helpers::sparql_update(
        &store,
        &format!(
            r#"INSERT DATA {{
                <{ns}alice> <{ns}name> "Alice" .
                <{ns}bob>   <{ns}name> "Bob" .
            }}"#
        ),
    );

    // Query them back.
    let query = format!(
        r#"SELECT ?s ?name WHERE {{
            ?s <{ns}name> ?name .
        }} ORDER BY ?name"#
    );
    let results = SparqlEvaluator::new()
        .parse_query(&query)
        .expect("Query parse failed")
        .on_store(&store)
        .execute()
        .expect("Query execution failed");

    if let QueryResults::Solutions(solutions) = results {
        let rows: Vec<_> = solutions.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(rows.len(), 2, "Expected 2 results from SELECT");

        let name0 = rows[0].get("name").expect("Missing ?name binding");
        let name1 = rows[1].get("name").expect("Missing ?name binding");

        assert_eq!(
            name0.to_string(),
            "\"Alice\"",
            "First result should be Alice"
        );
        assert_eq!(name1.to_string(), "\"Bob\"", "Second result should be Bob");
    } else {
        panic!("Expected QueryResults::Solutions");
    }
}

/// Test operations on named graphs: insert into named graphs, query specific
/// graphs, list named graphs.
#[test]
fn test_named_graph_operations() {
    let store = helpers::require_tikv_store!();
    let ns = helpers::unique_ns();

    let graph_a = format!("{ns}graph-a");
    let graph_b = format!("{ns}graph-b");

    // Insert data into two named graphs.
    helpers::sparql_update(
        &store,
        &format!(
            r#"INSERT DATA {{
                GRAPH <{graph_a}> {{
                    <{ns}s1> <{ns}p1> <{ns}o1> .
                    <{ns}s2> <{ns}p2> <{ns}o2> .
                }}
                GRAPH <{graph_b}> {{
                    <{ns}s3> <{ns}p3> <{ns}o3> .
                }}
            }}"#
        ),
    );

    // Query graph_a specifically.
    let query_a = format!(
        r#"SELECT (COUNT(*) AS ?cnt) WHERE {{
            GRAPH <{graph_a}> {{ ?s ?p ?o }}
        }}"#
    );
    let count_a = sparql_count(&store, &query_a);
    assert_eq!(count_a, 2, "graph_a should contain 2 triples");

    // Query graph_b specifically.
    let query_b = format!(
        r#"SELECT (COUNT(*) AS ?cnt) WHERE {{
            GRAPH <{graph_b}> {{ ?s ?p ?o }}
        }}"#
    );
    let count_b = sparql_count(&store, &query_b);
    assert_eq!(count_b, 1, "graph_b should contain 1 triple");

    // Verify both named graphs are listed.
    let named_graphs: Vec<NamedOrBlankNode> = store
        .named_graphs()
        .collect::<Result<Vec<_>, _>>()
        .expect("Failed to list named graphs");

    let graph_iris: Vec<String> = named_graphs.iter().map(|g| g.to_string()).collect();
    assert!(
        graph_iris.contains(&format!("<{graph_a}>")),
        "graph_a should be in named graphs list"
    );
    assert!(
        graph_iris.contains(&format!("<{graph_b}>")),
        "graph_b should be in named graphs list"
    );

    // Query across all named graphs using GRAPH ?g pattern.
    let query_all = format!(
        r#"SELECT (COUNT(*) AS ?cnt) WHERE {{
            GRAPH ?g {{ ?s ?p ?o }}
            FILTER(STRSTARTS(STR(?g), "{ns}"))
        }}"#
    );
    let count_all = sparql_count(&store, &query_all);
    assert_eq!(
        count_all, 3,
        "All named graphs should contain 3 triples total"
    );
}

/// Bulk load quads via the Store API and verify via SPARQL.
#[test]
fn test_bulk_load_and_query() {
    let store = helpers::require_tikv_store!();
    let ns = helpers::unique_ns();

    // Build a batch of triples.
    let triple_count = 100;
    let mut quads = Vec::with_capacity(triple_count);
    for i in 0..triple_count {
        let s = NamedNode::new(format!("{ns}item/{i}")).unwrap();
        let p = NamedNode::new(format!("{ns}index")).unwrap();
        let o = Literal::new_typed_literal(
            i.to_string(),
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        );
        quads.push(Quad::new(s, p, o, GraphName::DefaultGraph));
    }

    // Use the Store's extend method to bulk insert.
    store.extend(quads).expect("Bulk insert failed");

    // Verify count.
    let query = format!(
        r#"SELECT (COUNT(*) AS ?cnt) WHERE {{
            ?s <{ns}index> ?o .
        }}"#
    );
    let count = sparql_count(&store, &query);
    assert_eq!(
        count, triple_count as i64,
        "Bulk load should have inserted {triple_count} triples"
    );

    // Verify a specific triple via point query.
    let query_specific = format!(
        r#"SELECT ?val WHERE {{
            <{ns}item/42> <{ns}index> ?val .
        }}"#
    );
    let results = SparqlEvaluator::new()
        .parse_query(&query_specific)
        .unwrap()
        .on_store(&store)
        .execute()
        .unwrap();

    if let QueryResults::Solutions(solutions) = results {
        let rows: Vec<_> = solutions.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(rows.len(), 1, "Should find exactly one result for item/42");
    } else {
        panic!("Expected Solutions");
    }
}

/// Verify that a transaction that is not committed (dropped) does not
/// persist its changes (implicit rollback).
#[test]
fn test_transaction_rollback() {
    let store = helpers::require_tikv_store!();
    let ns = helpers::unique_ns();

    // Insert a triple so we have a baseline.
    helpers::sparql_update(
        &store,
        &format!(r#"INSERT DATA {{ <{ns}baseline> <{ns}exists> "true" . }}"#),
    );

    // Start a transaction, insert data, but do NOT commit.
    {
        let mut transaction = store.start_transaction().expect("start_transaction failed");
        SparqlEvaluator::new()
            .parse_update(&format!(
                r#"INSERT DATA {{ <{ns}phantom> <{ns}status> "should-not-persist" . }}"#
            ))
            .expect("Failed to parse update")
            .on_transaction(&mut transaction)
            .execute()
            .expect("Transaction update failed");
        // Explicitly drop without commit -- implicit rollback.
        // transaction.commit() is NOT called.
    }

    // Verify the phantom triple does not exist.
    let query = format!(
        r#"SELECT ?status WHERE {{
            <{ns}phantom> <{ns}status> ?status .
        }}"#
    );
    let results = SparqlEvaluator::new()
        .parse_query(&query)
        .unwrap()
        .on_store(&store)
        .execute()
        .unwrap();

    if let QueryResults::Solutions(solutions) = results {
        let rows: Vec<_> = solutions.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(
            rows.len(),
            0,
            "Rolled-back transaction data should not be visible"
        );
    } else {
        panic!("Expected Solutions");
    }

    // Verify baseline is still there.
    let baseline_query = format!(r#"SELECT ?val WHERE {{ <{ns}baseline> <{ns}exists> ?val . }}"#);
    let results = SparqlEvaluator::new()
        .parse_query(&baseline_query)
        .unwrap()
        .on_store(&store)
        .execute()
        .unwrap();

    if let QueryResults::Solutions(solutions) = results {
        let rows: Vec<_> = solutions.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(
            rows.len(),
            1,
            "Baseline triple should still exist after rollback"
        );
    } else {
        panic!("Expected Solutions");
    }
}

/// Test DELETE operations via SPARQL UPDATE: DELETE DATA, DELETE WHERE,
/// and combined INSERT/DELETE (MODIFY).
#[test]
fn test_sparql_delete_operations() {
    let store = helpers::require_tikv_store!();
    let ns = helpers::unique_ns();

    // Insert three triples.
    helpers::sparql_update(
        &store,
        &format!(
            r#"INSERT DATA {{
                <{ns}a> <{ns}type> <{ns}Widget> .
                <{ns}b> <{ns}type> <{ns}Widget> .
                <{ns}c> <{ns}type> <{ns}Gadget> .
            }}"#
        ),
    );

    // Delete one specific triple.
    helpers::sparql_update(
        &store,
        &format!(r#"DELETE DATA {{ <{ns}a> <{ns}type> <{ns}Widget> . }}"#),
    );

    // Verify only 2 remain.
    let count_query = format!(
        r#"SELECT (COUNT(*) AS ?cnt) WHERE {{
            ?s <{ns}type> ?o .
        }}"#
    );
    let count = sparql_count(&store, &count_query);
    assert_eq!(count, 2, "One triple should have been deleted, leaving 2");

    // Delete with WHERE pattern (all Widgets).
    helpers::sparql_update(
        &store,
        &format!(
            r#"DELETE {{ ?s <{ns}type> <{ns}Widget> }}
               WHERE  {{ ?s <{ns}type> <{ns}Widget> }}"#
        ),
    );

    // Verify only the Gadget remains.
    let remaining_query = format!(
        r#"SELECT ?s ?type WHERE {{
            ?s <{ns}type> ?type .
        }}"#
    );
    let results = SparqlEvaluator::new()
        .parse_query(&remaining_query)
        .unwrap()
        .on_store(&store)
        .execute()
        .unwrap();

    if let QueryResults::Solutions(solutions) = results {
        let rows: Vec<_> = solutions.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(rows.len(), 1, "Only the Gadget triple should remain");
        let type_val = rows[0].get("type").unwrap().to_string();
        assert!(
            type_val.contains("Gadget"),
            "Remaining triple should be a Gadget, got: {type_val}"
        );
    } else {
        panic!("Expected Solutions");
    }
}

/// Multiple threads reading concurrently from the same Store should all
/// see consistent data and not interfere with each other.
#[test]
fn test_concurrent_reads() {
    let store = helpers::require_tikv_store!();
    let ns = helpers::unique_ns();

    // Insert test data.
    let triple_count: usize = 50;
    let mut quads = Vec::with_capacity(triple_count);
    for i in 0..triple_count {
        let s = NamedNode::new(format!("{ns}entity/{i}")).unwrap();
        let p = NamedNode::new(format!("{ns}value")).unwrap();
        let o = Literal::new_typed_literal(
            i.to_string(),
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        );
        quads.push(Quad::new(s, p, o, GraphName::DefaultGraph));
    }
    store.extend(quads).expect("Bulk insert failed");

    // Spawn multiple reader threads.
    let reader_count = 8;
    let errors = AtomicU64::new(0);

    std::thread::scope(|scope| {
        let store_ref = &store;
        let ns_ref = &ns;
        let errors_ref = &errors;

        for thread_id in 0..reader_count {
            scope.spawn(move || {
                // Each thread runs multiple queries to stress concurrency.
                for _ in 0..5 {
                    let count_query = format!(
                        r#"SELECT (COUNT(*) AS ?cnt) WHERE {{
                            ?s <{ns_ref}value> ?o .
                        }}"#
                    );
                    let count = sparql_count(store_ref, &count_query);
                    if count != triple_count as i64 {
                        eprintln!(
                            "Thread {thread_id}: expected {triple_count} triples, got {count}"
                        );
                        errors_ref.fetch_add(1, Ordering::SeqCst);
                    }

                    // Also do a point query to verify specific data.
                    let point_query = format!(
                        r#"SELECT ?val WHERE {{
                            <{ns_ref}entity/{thread_id}> <{ns_ref}value> ?val .
                        }}"#
                    );
                    let results = SparqlEvaluator::new()
                        .parse_query(&point_query)
                        .unwrap()
                        .on_store(store_ref)
                        .execute()
                        .unwrap();

                    if let QueryResults::Solutions(solutions) = results {
                        let rows: Vec<_> = solutions.collect::<Result<Vec<_>, _>>().unwrap();
                        if rows.len() != 1 {
                            eprintln!(
                                "Thread {thread_id}: point query returned {} rows",
                                rows.len()
                            );
                            errors_ref.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                }
            });
        }
    });

    assert_eq!(
        errors.load(Ordering::SeqCst),
        0,
        "Concurrent reads produced incorrect results"
    );
}

/// Test DROP GRAPH and CLEAR GRAPH operations.
#[test]
fn test_graph_drop_and_clear() {
    let store = helpers::require_tikv_store!();
    let ns = helpers::unique_ns();
    let graph = format!("{ns}temp-graph");

    // Insert data into a named graph.
    let insert = format!(
        r#"INSERT DATA {{
            GRAPH <{graph}> {{
                <{ns}x> <{ns}y> <{ns}z> .
                <{ns}a> <{ns}b> <{ns}c> .
            }}
        }}"#
    );
    helpers::sparql_update(&store, &insert);

    // CLEAR the graph (removes triples but the graph may still exist).
    helpers::sparql_update(&store, &format!("CLEAR GRAPH <{graph}>"));

    let count_query = format!(
        r#"SELECT (COUNT(*) AS ?cnt) WHERE {{
            GRAPH <{graph}> {{ ?s ?p ?o }}
        }}"#
    );
    let count = sparql_count(&store, &count_query);
    assert_eq!(
        count, 0,
        "CLEAR GRAPH should remove all triples from the graph"
    );

    // Re-insert and then DROP the graph entirely.
    helpers::sparql_update(&store, &insert);
    helpers::sparql_update(&store, &format!("DROP GRAPH <{graph}>"));

    let count_after_drop = sparql_count(&store, &count_query);
    assert_eq!(
        count_after_drop, 0,
        "DROP GRAPH should remove all triples from the graph"
    );
}

/// Test INSERT/DELETE in a single SPARQL UPDATE (MODIFY operation).
#[test]
fn test_sparql_modify_operation() {
    let store = helpers::require_tikv_store!();
    let ns = helpers::unique_ns();

    // Insert initial data.
    helpers::sparql_update(
        &store,
        &format!(
            r#"INSERT DATA {{
                <{ns}item1> <{ns}status> "draft" .
                <{ns}item2> <{ns}status> "draft" .
            }}"#
        ),
    );

    // Modify: change all "draft" to "published" atomically.
    helpers::sparql_update(
        &store,
        &format!(
            r#"DELETE {{ ?item <{ns}status> "draft" }}
               INSERT {{ ?item <{ns}status> "published" }}
               WHERE  {{ ?item <{ns}status> "draft" }}"#
        ),
    );

    // Verify no drafts remain.
    let draft_query = format!(
        r#"SELECT (COUNT(*) AS ?cnt) WHERE {{
            ?s <{ns}status> "draft" .
        }}"#
    );
    let draft_count = sparql_count(&store, &draft_query);
    assert_eq!(draft_count, 0, "No drafts should remain after modify");

    // Verify two published items.
    let pub_query = format!(
        r#"SELECT (COUNT(*) AS ?cnt) WHERE {{
            ?s <{ns}status> "published" .
        }}"#
    );
    let pub_count = sparql_count(&store, &pub_query);
    assert_eq!(pub_count, 2, "Both items should be published");
}

// ============================================================================
// Shared helpers (outside module, used by test functions)
// ============================================================================

/// Execute a SPARQL SELECT that returns a single `?cnt` integer binding,
/// and return it as i64. Panics on unexpected result shapes.
fn sparql_count(store: &Store, query: &str) -> i64 {
    let results = SparqlEvaluator::new()
        .parse_query(query)
        .unwrap()
        .on_store(store)
        .execute()
        .expect("Count query failed");

    if let QueryResults::Solutions(solutions) = results {
        let rows: Vec<_> = solutions.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(rows.len(), 1, "COUNT query should return exactly one row");
        let cnt = rows[0].get("cnt").expect("Missing ?cnt binding");
        match cnt {
            Term::Literal(lit) => lit
                .value()
                .parse::<i64>()
                .unwrap_or_else(|e| panic!("Cannot parse count value '{}': {e}", lit.value())),
            other => panic!("Expected literal for ?cnt, got: {other}"),
        }
    } else {
        panic!("Expected QueryResults::Solutions for count query");
    }
}
