//! Criterion benchmarks comparing TiKV vs RocksDB storage backends.
//!
//! Requires `--features tikv` and the `TIKV_PD_ENDPOINTS` env var pointing at
//! a live PD cluster (comma-separated, e.g. "pd0:2379,pd1:2379,pd2:2379").
//!
//! Benchmarks are silently skipped when TiKV is unreachable so that
//! `cargo bench` never fails in environments without a cluster.

#![allow(clippy::panic)]

use codspeed_criterion_compat::{Criterion, criterion_group, criterion_main};
use oxigraph::io::{RdfFormat, RdfParser};
use oxigraph::sparql::{QueryResults, SparqlEvaluator};
use oxigraph::store::Store;
use spargebra::Query;
use std::str::FromStr;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Try to open a TiKV-backed store. Returns `None` when the env var is missing
/// or the cluster is unreachable.
fn try_tikv_store() -> Option<Store> {
    let endpoints_str = std::env::var("TIKV_PD_ENDPOINTS").ok()?;
    let endpoints: Vec<String> = endpoints_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if endpoints.is_empty() {
        return None;
    }
    Store::open_tikv(&endpoints).ok()
}

fn rocksdb_store() -> (Store, TempDir) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir).unwrap();
    (store, dir)
}

/// Generate N triples as NTriples text, each with a unique subject.
fn generate_ntriples(n: usize, prefix: &str) -> String {
    let mut buf = String::new();
    for i in 0..n {
        buf.push_str(&format!(
            "<http://bench.example.com/{prefix}/s{i}> <http://bench.example.com/p> <http://bench.example.com/o> .\n"
        ));
    }
    buf
}

/// Load NTriples data into a store via bulk loader.
fn bulk_load(store: &Store, data: &[u8]) {
    let mut loader = store.bulk_loader();
    loader
        .load_from_slice(RdfParser::from_format(RdfFormat::NTriples).lenient(), data)
        .unwrap();
    loader.commit().unwrap();
}

/// Execute a SPARQL SELECT and consume all results.
fn run_select(store: &Store, sparql: &str) -> usize {
    let query = Query::from_str(sparql).unwrap();
    match SparqlEvaluator::new()
        .for_query(query)
        .on_store(store)
        .execute()
        .unwrap()
    {
        QueryResults::Solutions(s) => s.map(|r| r.unwrap()).count(),
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Benchmark groups
// ---------------------------------------------------------------------------

fn bench_single_insert(c: &mut Criterion) {
    let tikv = try_tikv_store();
    let mut group = c.benchmark_group("single_triple_insert");
    group.sample_size(20);

    // RocksDB
    {
        let (store, _dir) = rocksdb_store();
        let mut idx = 0u64;
        group.bench_function("rocksdb", |b| {
            b.iter(|| {
                let data = format!(
                    "<http://bench.example.com/insert/s{idx}> <http://bench.example.com/p> <http://bench.example.com/o> .\n",
                );
                store
                    .load_from_slice(RdfFormat::NTriples, data.as_bytes())
                    .unwrap();
                idx += 1;
            })
        });
    }

    // TiKV
    if let Some(store) = &tikv {
        let mut idx = 0u64;
        group.bench_function("tikv", |b| {
            b.iter(|| {
                let data = format!(
                    "<http://bench.example.com/tikv-insert/s{idx}> <http://bench.example.com/p> <http://bench.example.com/o> .\n",
                );
                store
                    .load_from_slice(RdfFormat::NTriples, data.as_bytes())
                    .unwrap();
                idx += 1;
            })
        });
    } else {
        eprintln!("TiKV unavailable — skipping tikv single insert benchmark");
    }

    group.finish();
}

fn bench_point_query(c: &mut Criterion) {
    let tikv = try_tikv_store();
    let mut group = c.benchmark_group("point_query_latency");
    group.sample_size(50);

    let sparql =
        "SELECT ?o WHERE { <http://bench.example.com/point/s42> <http://bench.example.com/p> ?o }";

    // RocksDB
    {
        let (store, _dir) = rocksdb_store();
        let data = generate_ntriples(100, "point");
        bulk_load(&store, data.as_bytes());

        group.bench_function("rocksdb", |b| {
            b.iter(|| {
                run_select(&store, sparql);
            })
        });
    }

    // TiKV
    if let Some(store) = &tikv {
        // Clear and load
        store.clear().unwrap();
        let data = generate_ntriples(100, "point");
        bulk_load(store, data.as_bytes());

        group.bench_function("tikv", |b| {
            b.iter(|| {
                run_select(store, sparql);
            })
        });
    } else {
        eprintln!("TiKV unavailable — skipping tikv point query benchmark");
    }

    group.finish();
}

fn bench_range_scan(c: &mut Criterion) {
    let tikv = try_tikv_store();
    let mut group = c.benchmark_group("range_scan_100");
    group.sample_size(30);

    let sparql = "SELECT ?s ?o WHERE { ?s <http://bench.example.com/p> ?o } LIMIT 100";

    // RocksDB
    {
        let (store, _dir) = rocksdb_store();
        let data = generate_ntriples(500, "range");
        bulk_load(&store, data.as_bytes());

        group.bench_function("rocksdb", |b| {
            b.iter(|| {
                let count = run_select(&store, sparql);
                assert_eq!(count, 100);
            })
        });
    }

    // TiKV
    if let Some(store) = &tikv {
        store.clear().unwrap();
        let data = generate_ntriples(500, "range");
        bulk_load(store, data.as_bytes());

        group.bench_function("tikv", |b| {
            b.iter(|| {
                let count = run_select(store, sparql);
                assert_eq!(count, 100);
            })
        });
    } else {
        eprintln!("TiKV unavailable — skipping tikv range scan benchmark");
    }

    group.finish();
}

fn bench_two_bgp_join(c: &mut Criterion) {
    let tikv = try_tikv_store();
    let mut group = c.benchmark_group("sparql_2bgp_join");
    group.sample_size(20);

    // Build data with a join pattern: s -p1-> mid -p2-> o
    let mut data = String::new();
    for i in 0..200 {
        let mid = i / 2; // creates fan-in so join produces results
        data.push_str(&format!(
            "<http://bench.example.com/join/s{i}> <http://bench.example.com/p1> <http://bench.example.com/join/m{mid}> .\n"
        ));
        data.push_str(&format!(
            "<http://bench.example.com/join/m{mid}> <http://bench.example.com/p2> <http://bench.example.com/join/o{i}> .\n"
        ));
    }

    let sparql = "SELECT ?s ?o WHERE { ?s <http://bench.example.com/p1> ?m . ?m <http://bench.example.com/p2> ?o }";

    // RocksDB
    {
        let (store, _dir) = rocksdb_store();
        bulk_load(&store, data.as_bytes());

        group.bench_function("rocksdb", |b| {
            b.iter(|| {
                run_select(&store, sparql);
            })
        });
    }

    // TiKV
    if let Some(store) = &tikv {
        store.clear().unwrap();
        bulk_load(store, data.as_bytes());

        group.bench_function("tikv", |b| {
            b.iter(|| {
                run_select(store, sparql);
            })
        });
    } else {
        eprintln!("TiKV unavailable — skipping tikv 2-BGP join benchmark");
    }

    group.finish();
}

fn bench_bulk_load_1000(c: &mut Criterion) {
    let tikv = try_tikv_store();
    let mut group = c.benchmark_group("bulk_load_1000");
    group.sample_size(10);

    let data = generate_ntriples(1000, "bulk");

    // RocksDB
    group.bench_function("rocksdb", |b| {
        b.iter(|| {
            let (store, _dir) = rocksdb_store();
            bulk_load(&store, data.as_bytes());
        })
    });

    // TiKV
    if let Some(store) = &tikv {
        group.bench_function("tikv", |b| {
            b.iter(|| {
                store.clear().unwrap();
                bulk_load(store, data.as_bytes());
            })
        });
    } else {
        eprintln!("TiKV unavailable — skipping tikv bulk load benchmark");
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion harness
// ---------------------------------------------------------------------------

criterion_group!(
    tikv_benches,
    bench_single_insert,
    bench_point_query,
    bench_range_scan,
    bench_two_bgp_join,
    bench_bulk_load_1000,
);

criterion_main!(tikv_benches);
