//! TiKV smoke test — standalone binary.
//! Compile with: cargo build --example tikv_smoke -p oxigraph --features tikv
//! Run inside the podman network where tikv0/tikv1/tikv2 are resolvable.

fn main() {
    let endpoints: Vec<String> = std::env::var("TIKV_PD_ENDPOINTS")
        .unwrap_or_else(|_| "pd0:2379".to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    eprintln!("=== TiKV Smoke Test ===");
    eprintln!("PD endpoints: {endpoints:?}");

    eprintln!("\n--- Test 1: Connect ---");
    let store = oxigraph::store::Store::open_tikv(&endpoints).expect("Failed to connect to TiKV");
    eprintln!("Connected successfully");

    eprintln!("\n--- Test 2: Insert ---");
    let quad = oxigraph::model::Quad::new(
        oxigraph::model::NamedNode::new("http://example.org/smoke/s1").unwrap(),
        oxigraph::model::NamedNode::new("http://example.org/smoke/p1").unwrap(),
        oxigraph::model::Literal::new_simple_literal("hello tikv"),
        oxigraph::model::GraphName::DefaultGraph,
    );
    store.insert(&quad).expect("Insert failed");
    eprintln!("Insert OK");

    eprintln!("\n--- Test 3: Query ---");
    let results = oxigraph::sparql::SparqlEvaluator::new()
        .parse_query("SELECT ?o WHERE { <http://example.org/smoke/s1> <http://example.org/smoke/p1> ?o }")
        .unwrap()
        .on_store(&store)
        .execute()
        .expect("Query failed");

    match results {
        oxigraph::sparql::QueryResults::Solutions(mut solutions) => {
            let solution = solutions.next().expect("No results").expect("Error reading solution");
            let value = solution.get("o").expect("No ?o binding");
            eprintln!("Got value: {value}");
            assert_eq!(value.to_string(), "\"hello tikv\"");
            eprintln!("Query OK — value matches");
        }
        _ => panic!("Expected Solutions"),
    }

    eprintln!("\n--- Test 4: Count ---");
    let results = oxigraph::sparql::SparqlEvaluator::new()
        .parse_query("SELECT (COUNT(*) AS ?count) WHERE { ?s ?p ?o }")
        .unwrap()
        .on_store(&store)
        .execute()
        .expect("Count failed");
    match results {
        oxigraph::sparql::QueryResults::Solutions(mut solutions) => {
            let solution = solutions.next().unwrap().unwrap();
            let count = solution.get("count").unwrap();
            eprintln!("Total triples: {count}");
        }
        _ => panic!("Expected Solutions"),
    }

    eprintln!("\n--- Test 5: Delete ---");
    store.remove(&quad).expect("Remove failed");
    let results = oxigraph::sparql::SparqlEvaluator::new()
        .parse_query("SELECT ?o WHERE { <http://example.org/smoke/s1> <http://example.org/smoke/p1> ?o }")
        .unwrap()
        .on_store(&store)
        .execute()
        .expect("Post-delete query failed");
    match results {
        oxigraph::sparql::QueryResults::Solutions(mut solutions) => {
            assert!(solutions.next().is_none(), "Triple should be deleted");
            eprintln!("Delete OK — triple removed");
        }
        _ => panic!("Expected Solutions"),
    }

    eprintln!("\n=== All smoke tests passed ===");
}
