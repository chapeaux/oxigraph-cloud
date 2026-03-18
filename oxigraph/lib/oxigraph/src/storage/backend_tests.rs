//! Backend conformance test suite.
//!
//! Tests are parameterized via the `conformance_tests!` macro so they can
//! run against any storage backend (Memory, RocksDB, TiKV).
//!
//! Usage:
//! ```ignore
//! conformance_tests!(memory_tests, || MemoryStorage::new());
//! conformance_tests!(rocksdb_tests, || { /* create RocksDB instance */ });
//! ```

/// Macro that generates a complete conformance test module for a storage backend.
///
/// The `$factory` expression must return a `Storage` instance.
#[cfg(test)]
macro_rules! conformance_tests {
    ($mod_name:ident, $factory:expr) => {
        #[cfg(test)]
        mod $mod_name {
            use crate::model::{GraphNameRef, NamedOrBlankNodeRef, QuadRef};
            use crate::storage::Storage;
            use crate::storage::numeric_encoder::{EncodedQuad, EncodedTerm};
            use oxrdf::NamedNodeRef;

            fn make_storage() -> Storage {
                $factory
            }

            fn example_node(suffix: &str) -> NamedNodeRef<'_> {
                // Use a static-like pattern; NamedNodeRef borrows from the str
                match suffix {
                    "1" => NamedNodeRef::new_unchecked("http://example.com/1"),
                    "2" => NamedNodeRef::new_unchecked("http://example.com/2"),
                    "3" => NamedNodeRef::new_unchecked("http://example.com/3"),
                    "4" => NamedNodeRef::new_unchecked("http://example.com/4"),
                    _ => NamedNodeRef::new_unchecked("http://example.com/other"),
                }
            }

            // ==================== T1: Basic CRUD ====================

            #[test]
            fn t1_1_insert_and_query() {
                let storage = make_storage();
                let s = example_node("1");
                let p = example_node("2");
                let o = example_node("3");
                let quad = QuadRef::new(s, p, o, GraphNameRef::DefaultGraph);
                let encoded = EncodedQuad::from(quad);

                // Insert
                let mut txn = storage.start_transaction().unwrap();
                txn.insert(quad);
                txn.commit().unwrap();

                // Query
                let reader = storage.snapshot();
                assert!(reader.contains(&encoded).unwrap());
                assert_eq!(reader.len().unwrap(), 1);
                assert!(!reader.is_empty().unwrap());
            }

            #[test]
            fn t1_2_delete() {
                let storage = make_storage();
                let s = example_node("1");
                let p = example_node("2");
                let o = example_node("3");
                let quad = QuadRef::new(s, p, o, GraphNameRef::DefaultGraph);
                let encoded = EncodedQuad::from(quad);

                // Insert then delete
                let mut txn = storage.start_transaction().unwrap();
                txn.insert(quad);
                txn.commit().unwrap();

                let mut txn = storage.start_transaction().unwrap();
                txn.remove(quad);
                txn.commit().unwrap();

                let reader = storage.snapshot();
                assert!(!reader.contains(&encoded).unwrap());
                assert_eq!(reader.len().unwrap(), 0);
            }

            #[test]
            fn t1_3_missing_key_returns_empty() {
                let storage = make_storage();
                let s = example_node("1");
                let encoded = EncodedQuad::from(QuadRef::new(s, s, s, GraphNameRef::DefaultGraph));

                let reader = storage.snapshot();
                assert!(!reader.contains(&encoded).unwrap());
            }

            #[test]
            fn t1_4_overwrite_is_idempotent() {
                let storage = make_storage();
                let s = example_node("1");
                let quad = QuadRef::new(s, s, s, GraphNameRef::DefaultGraph);
                let encoded = EncodedQuad::from(quad);

                // Insert same quad twice
                let mut txn = storage.start_transaction().unwrap();
                txn.insert(quad);
                txn.commit().unwrap();

                let mut txn = storage.start_transaction().unwrap();
                txn.insert(quad);
                txn.commit().unwrap();

                let reader = storage.snapshot();
                assert!(reader.contains(&encoded).unwrap());
                assert_eq!(reader.len().unwrap(), 1);
            }

            // ==================== T3: Pattern Queries ====================

            #[test]
            fn t3_1_query_by_subject() {
                let storage = make_storage();
                let s1 = example_node("1");
                let s2 = example_node("2");
                let p = example_node("3");
                let o = example_node("4");

                let mut txn = storage.start_transaction().unwrap();
                txn.insert(QuadRef::new(s1, p, o, GraphNameRef::DefaultGraph));
                txn.insert(QuadRef::new(s2, p, o, GraphNameRef::DefaultGraph));
                txn.commit().unwrap();

                let reader = storage.snapshot();
                let encoded_s1 = EncodedTerm::from(s1);
                let results: Vec<_> = reader
                    .quads_for_pattern(Some(&encoded_s1), None, None, None)
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].subject, encoded_s1);
            }

            #[test]
            fn t3_2_query_all() {
                let storage = make_storage();
                let s = example_node("1");
                let p = example_node("2");
                let o1 = example_node("3");
                let o2 = example_node("4");

                let mut txn = storage.start_transaction().unwrap();
                txn.insert(QuadRef::new(s, p, o1, GraphNameRef::DefaultGraph));
                txn.insert(QuadRef::new(s, p, o2, GraphNameRef::DefaultGraph));
                txn.commit().unwrap();

                let reader = storage.snapshot();
                let results: Vec<_> = reader
                    .quads_for_pattern(None, None, None, None)
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                assert_eq!(results.len(), 2);
            }

            // ==================== T5: Transaction Atomicity ====================

            #[test]
            fn t5_1_commit_makes_writes_visible() {
                let storage = make_storage();
                let s = example_node("1");
                let quad = QuadRef::new(s, s, s, GraphNameRef::DefaultGraph);
                let encoded = EncodedQuad::from(quad);

                // Before commit: not visible
                let pre_snapshot = storage.snapshot();

                let mut txn = storage.start_transaction().unwrap();
                txn.insert(quad);
                txn.commit().unwrap();

                // After commit: visible in new snapshot
                assert!(storage.snapshot().contains(&encoded).unwrap());
                // Old snapshot: not visible
                assert!(!pre_snapshot.contains(&encoded).unwrap());
            }

            #[test]
            fn t5_2_rollback_on_drop() {
                let storage = make_storage();
                let s = example_node("1");
                let quad = QuadRef::new(s, s, s, GraphNameRef::DefaultGraph);
                let encoded = EncodedQuad::from(quad);

                // Transaction that is dropped without commit
                {
                    let mut txn = storage.start_transaction().unwrap();
                    txn.insert(quad);
                    // txn dropped here — implicit rollback
                }

                let reader = storage.snapshot();
                assert!(!reader.contains(&encoded).unwrap());
            }

            // ==================== T6: Named Graphs ====================

            #[test]
            fn t6_1_named_graph_lifecycle() {
                let storage = make_storage();
                let graph = example_node("1");
                let encoded_graph = EncodedTerm::from(graph);

                // Insert named graph
                let mut txn = storage.start_transaction().unwrap();
                txn.insert_named_graph(graph.into());
                txn.commit().unwrap();

                let reader = storage.snapshot();
                assert!(reader.contains_named_graph(&encoded_graph).unwrap());

                let graphs: Vec<_> = reader
                    .named_graphs()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                assert_eq!(graphs.len(), 1);
            }

            #[test]
            fn t6_2_clear_all() {
                let storage = make_storage();
                let s = example_node("1");
                let p = example_node("2");
                let o = example_node("3");

                let mut txn = storage.start_transaction().unwrap();
                txn.insert(QuadRef::new(s, p, o, GraphNameRef::DefaultGraph));
                txn.insert(QuadRef::new(s, p, o, s));
                txn.insert_named_graph(s.into());
                txn.commit().unwrap();

                let mut txn = storage.start_transaction().unwrap();
                txn.clear();
                txn.commit().unwrap();

                let reader = storage.snapshot();
                assert!(reader.is_empty().unwrap());
            }

            // ==================== T7: Readable Transactions ====================

            #[test]
            fn t7_1_readable_transaction_sees_own_writes() {
                let storage = make_storage();
                let s = example_node("1");
                let quad = QuadRef::new(s, s, s, GraphNameRef::DefaultGraph);
                let encoded = EncodedQuad::from(quad);

                let mut txn = storage.start_readable_transaction().unwrap();
                txn.insert(quad);

                // Reader within the transaction should see the write
                {
                    let reader = txn.reader();
                    assert!(reader.contains(&encoded).unwrap());
                }

                txn.commit().unwrap();
            }

            // ==================== T8: Bulk Loader ====================

            #[test]
            fn t8_1_bulk_load() {
                let storage = make_storage();
                let s = example_node("1");
                let p = example_node("2");
                let o = example_node("3");

                let quads = vec![oxrdf::Quad::new(
                    s.into_owned(),
                    p.into_owned(),
                    oxrdf::Term::from(o.into_owned()),
                    oxrdf::GraphName::DefaultGraph,
                )];

                let mut loader = storage.bulk_loader();
                loader.load_batch(quads, 1).unwrap();
                loader.commit().unwrap();

                let reader = storage.snapshot();
                assert_eq!(reader.len().unwrap(), 1);
            }

            // ==================== T9: Validation ====================

            #[test]
            fn t9_1_validate_empty() {
                let storage = make_storage();
                storage.snapshot().validate().unwrap();
            }

            #[test]
            fn t9_2_validate_after_operations() {
                let storage = make_storage();
                let s = example_node("1");
                let p = example_node("2");
                let o = example_node("3");

                let mut txn = storage.start_transaction().unwrap();
                txn.insert(QuadRef::new(s, p, o, GraphNameRef::DefaultGraph));
                txn.insert(QuadRef::new(s, p, o, s));
                txn.insert_named_graph(s.into());
                txn.commit().unwrap();

                storage.snapshot().validate().unwrap();

                // Remove and validate again
                let mut txn = storage.start_transaction().unwrap();
                txn.remove(QuadRef::new(s, p, o, GraphNameRef::DefaultGraph));
                txn.commit().unwrap();

                storage.snapshot().validate().unwrap();
            }
        }
    };
}

#[cfg(test)]
conformance_tests!(memory_conformance, Storage::new().unwrap());

#[cfg(test)]
#[cfg(all(not(target_family = "wasm"), feature = "tikv"))]
mod tikv_conformance {
    use crate::model::{GraphNameRef, QuadRef};
    use crate::storage::Storage;
    use crate::storage::numeric_encoder::{EncodedQuad, EncodedTerm};
    use oxrdf::NamedNodeRef;

    /// Returns `Some(Storage)` if TiKV is available, `None` otherwise.
    /// Reads `TIKV_PD_ENDPOINTS` (comma-separated) from the environment.
    fn try_make_tikv_storage() -> Option<Storage> {
        let endpoints_str = std::env::var("TIKV_PD_ENDPOINTS").ok()?;
        let endpoints: Vec<String> = endpoints_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if endpoints.is_empty() {
            return None;
        }
        match Storage::open_tikv(&endpoints) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("TiKV connection failed, skipping test: {e}");
                None
            }
        }
    }

    /// Helper: skip test if TiKV is not reachable.
    macro_rules! tikv_storage_or_skip {
        () => {
            match try_make_tikv_storage() {
                Some(s) => s,
                None => {
                    eprintln!("TIKV_PD_ENDPOINTS not set or TiKV unreachable — skipping");
                    return;
                }
            }
        };
    }

    fn nn(iri: &str) -> NamedNodeRef<'_> {
        NamedNodeRef::new_unchecked(iri)
    }

    // ==================== Insert & Query ====================

    #[test]
    fn tikv_insert_and_query() {
        let storage = tikv_storage_or_skip!();
        let s = nn("http://example.com/tikv-test/insert_query/s");
        let p = nn("http://example.com/tikv-test/insert_query/p");
        let o = nn("http://example.com/tikv-test/insert_query/o");
        let quad = QuadRef::new(s, p, o, GraphNameRef::DefaultGraph);
        let encoded = EncodedQuad::from(quad);

        // Clean up from any previous run
        let mut txn = storage.start_transaction().unwrap();
        txn.remove(quad);
        txn.commit().unwrap();

        let mut txn = storage.start_transaction().unwrap();
        txn.insert(quad);
        txn.commit().unwrap();

        let reader = storage.snapshot();
        assert!(reader.contains(&encoded).unwrap());
    }

    // ==================== Delete ====================

    #[test]
    fn tikv_delete() {
        let storage = tikv_storage_or_skip!();
        let s = nn("http://example.com/tikv-test/delete/s");
        let p = nn("http://example.com/tikv-test/delete/p");
        let o = nn("http://example.com/tikv-test/delete/o");
        let quad = QuadRef::new(s, p, o, GraphNameRef::DefaultGraph);
        let encoded = EncodedQuad::from(quad);

        let mut txn = storage.start_transaction().unwrap();
        txn.insert(quad);
        txn.commit().unwrap();

        let mut txn = storage.start_transaction().unwrap();
        txn.remove(quad);
        txn.commit().unwrap();

        let reader = storage.snapshot();
        assert!(!reader.contains(&encoded).unwrap());
    }

    // ==================== Snapshot Isolation ====================

    #[test]
    fn tikv_snapshot_isolation() {
        let storage = tikv_storage_or_skip!();
        let s = nn("http://example.com/tikv-test/snapshot/s");
        let quad = QuadRef::new(s, s, s, GraphNameRef::DefaultGraph);
        let encoded = EncodedQuad::from(quad);

        // Ensure clean state
        let mut txn = storage.start_transaction().unwrap();
        txn.remove(quad);
        txn.commit().unwrap();

        let pre_snapshot = storage.snapshot();

        let mut txn = storage.start_transaction().unwrap();
        txn.insert(quad);
        txn.commit().unwrap();

        assert!(storage.snapshot().contains(&encoded).unwrap());
        assert!(!pre_snapshot.contains(&encoded).unwrap());
    }

    // ==================== Named Graphs ====================

    #[test]
    fn tikv_named_graph_lifecycle() {
        let storage = tikv_storage_or_skip!();
        let graph = nn("http://example.com/tikv-test/named_graph/g1");
        let encoded_graph = EncodedTerm::from(graph);

        let mut txn = storage.start_transaction().unwrap();
        txn.insert_named_graph(graph.into());
        txn.commit().unwrap();

        let reader = storage.snapshot();
        assert!(reader.contains_named_graph(&encoded_graph).unwrap());
    }

    // ==================== Clear ====================

    #[test]
    fn tikv_clear() {
        let storage = tikv_storage_or_skip!();
        let s = nn("http://example.com/tikv-test/clear/s");
        let p = nn("http://example.com/tikv-test/clear/p");
        let o = nn("http://example.com/tikv-test/clear/o");

        let mut txn = storage.start_transaction().unwrap();
        txn.insert(QuadRef::new(s, p, o, GraphNameRef::DefaultGraph));
        txn.insert(QuadRef::new(s, p, o, s));
        txn.insert_named_graph(s.into());
        txn.commit().unwrap();

        let mut txn = storage.start_transaction().unwrap();
        txn.clear();
        txn.commit().unwrap();

        let reader = storage.snapshot();
        assert!(reader.is_empty().unwrap());
    }

    // ==================== Rollback on Drop ====================

    #[test]
    fn tikv_rollback_on_drop() {
        let storage = tikv_storage_or_skip!();
        let s = nn("http://example.com/tikv-test/rollback/s");
        let quad = QuadRef::new(s, s, s, GraphNameRef::DefaultGraph);
        let encoded = EncodedQuad::from(quad);

        // Ensure clean state
        let mut txn = storage.start_transaction().unwrap();
        txn.remove(quad);
        txn.commit().unwrap();

        {
            let mut txn = storage.start_transaction().unwrap();
            txn.insert(quad);
            // dropped without commit
        }

        let reader = storage.snapshot();
        assert!(!reader.contains(&encoded).unwrap());
    }

    // ==================== Validate ====================

    #[test]
    fn tikv_validate_empty() {
        let storage = tikv_storage_or_skip!();

        // Clear first to get a clean slate
        let mut txn = storage.start_transaction().unwrap();
        txn.clear();
        txn.commit().unwrap();

        storage.snapshot().validate().unwrap();
    }

    #[test]
    fn tikv_validate_after_operations() {
        let storage = tikv_storage_or_skip!();
        let s = nn("http://example.com/tikv-test/validate/s");
        let p = nn("http://example.com/tikv-test/validate/p");
        let o = nn("http://example.com/tikv-test/validate/o");

        // Clear and re-populate
        let mut txn = storage.start_transaction().unwrap();
        txn.clear();
        txn.commit().unwrap();

        let mut txn = storage.start_transaction().unwrap();
        txn.insert(QuadRef::new(s, p, o, GraphNameRef::DefaultGraph));
        txn.insert(QuadRef::new(s, p, o, s));
        txn.insert_named_graph(s.into());
        txn.commit().unwrap();

        storage.snapshot().validate().unwrap();

        let mut txn = storage.start_transaction().unwrap();
        txn.remove(QuadRef::new(s, p, o, GraphNameRef::DefaultGraph));
        txn.commit().unwrap();

        storage.snapshot().validate().unwrap();
    }

    // ==================== Bulk Load ====================

    #[test]
    fn tikv_bulk_load() {
        let storage = tikv_storage_or_skip!();
        let s = nn("http://example.com/tikv-test/bulk/s");
        let p = nn("http://example.com/tikv-test/bulk/p");
        let o = nn("http://example.com/tikv-test/bulk/o");

        // Clear first
        let mut txn = storage.start_transaction().unwrap();
        txn.clear();
        txn.commit().unwrap();

        let quads = vec![oxrdf::Quad::new(
            s.into_owned(),
            p.into_owned(),
            oxrdf::Term::from(o.into_owned()),
            oxrdf::GraphName::DefaultGraph,
        )];

        let mut loader = storage.bulk_loader();
        loader.load_batch(quads, 1).unwrap();
        loader.commit().unwrap();

        let reader = storage.snapshot();
        assert_eq!(reader.len().unwrap(), 1);
    }

    // ==================== Pattern Query ====================

    #[test]
    fn tikv_query_by_subject() {
        let storage = tikv_storage_or_skip!();
        let s1 = nn("http://example.com/tikv-test/pattern/s1");
        let s2 = nn("http://example.com/tikv-test/pattern/s2");
        let p = nn("http://example.com/tikv-test/pattern/p");
        let o = nn("http://example.com/tikv-test/pattern/o");

        // Clear and insert
        let mut txn = storage.start_transaction().unwrap();
        txn.clear();
        txn.commit().unwrap();

        let mut txn = storage.start_transaction().unwrap();
        txn.insert(QuadRef::new(s1, p, o, GraphNameRef::DefaultGraph));
        txn.insert(QuadRef::new(s2, p, o, GraphNameRef::DefaultGraph));
        txn.commit().unwrap();

        let reader = storage.snapshot();
        let encoded_s1 = EncodedTerm::from(s1);
        let results: Vec<_> = reader
            .quads_for_pattern(Some(&encoded_s1), None, None, None)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].subject, encoded_s1);
    }

    // ==================== Readable Transaction ====================

    #[test]
    fn tikv_readable_transaction_sees_own_writes() {
        let storage = tikv_storage_or_skip!();
        let s = nn("http://example.com/tikv-test/readable_txn/s");
        let quad = QuadRef::new(s, s, s, GraphNameRef::DefaultGraph);
        let encoded = EncodedQuad::from(quad);

        // Clean state
        let mut txn = storage.start_transaction().unwrap();
        txn.remove(quad);
        txn.commit().unwrap();

        let mut txn = storage.start_readable_transaction().unwrap();
        txn.insert(quad);

        {
            let reader = txn.reader();
            assert!(reader.contains(&encoded).unwrap());
        }

        txn.commit().unwrap();
    }
}

#[cfg(test)]
#[cfg(all(not(target_family = "wasm"), feature = "rocksdb"))]
mod rocksdb_conformance {
    use crate::model::{GraphNameRef, QuadRef};
    use crate::storage::Storage;
    use crate::storage::numeric_encoder::EncodedQuad;
    use tempfile::TempDir;

    fn make_rocksdb_storage() -> (Storage, TempDir) {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(dir.path()).unwrap();
        (storage, dir)
    }

    // RocksDB tests can't use the macro directly because we need to keep TempDir alive.
    // Instead, replicate the key tests with the TempDir pattern.

    #[test]
    fn rocksdb_insert_and_query() {
        let (storage, _dir) = make_rocksdb_storage();
        let s = oxrdf::NamedNodeRef::new_unchecked("http://example.com/1");
        let p = oxrdf::NamedNodeRef::new_unchecked("http://example.com/2");
        let o = oxrdf::NamedNodeRef::new_unchecked("http://example.com/3");
        let quad = QuadRef::new(s, p, o, GraphNameRef::DefaultGraph);
        let encoded = EncodedQuad::from(quad);

        let mut txn = storage.start_transaction().unwrap();
        txn.insert(quad);
        txn.commit().unwrap();

        let reader = storage.snapshot();
        assert!(reader.contains(&encoded).unwrap());
        assert_eq!(reader.len().unwrap(), 1);
    }

    #[test]
    fn rocksdb_snapshot_isolation() {
        let (storage, _dir) = make_rocksdb_storage();
        let s = oxrdf::NamedNodeRef::new_unchecked("http://example.com/1");
        let quad = QuadRef::new(s, s, s, GraphNameRef::DefaultGraph);
        let encoded = EncodedQuad::from(quad);

        let pre_snapshot = storage.snapshot();

        let mut txn = storage.start_transaction().unwrap();
        txn.insert(quad);
        txn.commit().unwrap();

        assert!(storage.snapshot().contains(&encoded).unwrap());
        assert!(!pre_snapshot.contains(&encoded).unwrap());
    }

    #[test]
    fn rocksdb_rollback_on_drop() {
        let (storage, _dir) = make_rocksdb_storage();
        let s = oxrdf::NamedNodeRef::new_unchecked("http://example.com/1");
        let quad = QuadRef::new(s, s, s, GraphNameRef::DefaultGraph);
        let encoded = EncodedQuad::from(quad);

        {
            let mut txn = storage.start_transaction().unwrap();
            txn.insert(quad);
        }

        assert!(!storage.snapshot().contains(&encoded).unwrap());
    }

    #[test]
    fn rocksdb_validate() {
        let (storage, _dir) = make_rocksdb_storage();
        let s = oxrdf::NamedNodeRef::new_unchecked("http://example.com/1");
        let p = oxrdf::NamedNodeRef::new_unchecked("http://example.com/2");
        let o = oxrdf::NamedNodeRef::new_unchecked("http://example.com/3");

        let mut txn = storage.start_transaction().unwrap();
        txn.insert(QuadRef::new(s, p, o, GraphNameRef::DefaultGraph));
        txn.insert(QuadRef::new(s, p, o, s));
        txn.insert_named_graph(s.into());
        txn.commit().unwrap();

        storage.snapshot().validate().unwrap();
    }
}
