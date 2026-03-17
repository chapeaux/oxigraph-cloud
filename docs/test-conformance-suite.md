# StorageBackend Conformance Test Suite

> **Status**: Design v1 | **Date**: 2026-03-17
> **Task**: Phase 1, Task 1.5
> **Owner**: `/test-qa`
> **Consumers**: `/rust-dev` (implementation), `/tikv-ops` (CI TiKV cluster)

---

## Overview

This document specifies a backend-agnostic conformance test suite that validates any `StorageBackend` implementation. Every test is parameterized over a generic `B: StorageBackend`, so the same test logic runs against RocksDB, in-memory (BTreeMap), and TiKV backends without modification.

A backend that passes all tests in this suite is considered conformant and safe to wire into `oxigraph::Store<B>`.

---

## Test Infrastructure

### Parameterization Strategy

Use a declarative macro that expands a full test module for each backend. This avoids code duplication and ensures every backend runs identical assertions.

```rust
/// Macro that generates a conformance test module for a given backend factory.
///
/// `$mod_name` - unique module name (e.g., `rocksdb_conformance`)
/// `$factory`  - expression returning `impl StorageBackend` (called once per test)
macro_rules! conformance_tests {
    ($mod_name:ident, $factory:expr) => {
        mod $mod_name {
            use super::*;

            // -- Basic CRUD --
            #[test]
            fn test_crud_put_get() {
                let backend = $factory;
                crud::test_put_get(&backend);
            }

            #[test]
            fn test_crud_delete() {
                let backend = $factory;
                crud::test_delete(&backend);
            }

            // ... all other test functions follow the same pattern
        }
    };
}

// Instantiate for each backend:
conformance_tests!(inmemory, InMemoryBackend::new());
conformance_tests!(rocksdb, RocksDbBackend::open_temp().unwrap());

// TiKV tests gated behind feature flag + environment variable:
#[cfg(feature = "tikv")]
conformance_tests!(tikv, TikvBackend::connect_from_env().unwrap());
```

Each test function lives in a helper module (e.g., `crud`, `range_scan`, `txn`) and takes `&impl StorageBackend` as its argument. The macro simply dispatches.

### Async Backend Handling

For backends that are internally async (TiKV), the `StorageBackend` trait methods are sync on the surface but may internally call `tokio::runtime::Handle::block_on()`. Tests run on a standard `#[test]` harness.

If the project adopts a fully async trait, wrap tests with:

```rust
#[tokio::test]
async fn test_crud_put_get() {
    let backend = TikvBackend::connect_from_env().await.unwrap();
    crud::test_put_get(&backend).await;
}
```

The macro can be extended with a `$async_marker` variant to generate `#[tokio::test]` instead of `#[test]`.

### CI Considerations

| Backend | CI Setup | Gate |
|---------|----------|------|
| In-memory | None required | Always runs |
| RocksDB | Linked at build time (vendored via `rocksdb` crate) | Always runs |
| TiKV | Service container: `pingcap/tidb:latest` or `tiup playground` in a sidecar | Feature flag `tikv` + env var `TIKV_PD_ENDPOINTS` |

**GitHub Actions example for TiKV service container:**

```yaml
jobs:
  tikv-integration:
    runs-on: ubuntu-latest
    services:
      pd:
        image: pingcap/pd:latest
        ports: ["2379:2379"]
        options: >-
          --name pd
      tikv:
        image: pingcap/tikv:latest
        ports: ["20160:20160"]
        options: >-
          --name tikv
          --link pd
    env:
      TIKV_PD_ENDPOINTS: "127.0.0.1:2379"
    steps:
      - uses: actions/checkout@v4
      - run: cargo test --features tikv -- --test-threads=1
```

Note: TiKV integration tests should run with `--test-threads=1` to avoid cross-test interference on shared cluster state, unless each test uses an isolated key prefix.

### Test Key Prefix Isolation

To allow parallel test execution on a shared backend, every test function should use a unique key prefix derived from the test name:

```rust
fn test_prefix(test_name: &str) -> Vec<u8> {
    let mut prefix = b"test/".to_vec();
    prefix.extend_from_slice(test_name.as_bytes());
    prefix.push(b'/');
    prefix
}
```

### Cleanup

Each test should clean up its keys after execution via `batch_delete`, or the backend factory should provide a fresh/isolated instance (temp directory for RocksDB, key prefix for TiKV).

---

## Test Categories

### 1. Basic CRUD

#### T1.1 `test_put_get`

- **Description**: Put a key-value pair, then get it back.
- **Setup**: Empty backend.
- **Steps**:
  1. `put(b"key1", b"value1")`
  2. `result = get(b"key1")`
- **Expected**: `result == Some(b"value1")`
- **Methods exercised**: `put`, `get`

#### T1.2 `test_get_missing_key`

- **Description**: Get a key that does not exist.
- **Setup**: Empty backend.
- **Steps**:
  1. `result = get(b"nonexistent")`
- **Expected**: `result == None`
- **Methods exercised**: `get`

#### T1.3 `test_put_overwrite`

- **Description**: Overwriting an existing key replaces its value.
- **Setup**: Empty backend.
- **Steps**:
  1. `put(b"key1", b"value_a")`
  2. `put(b"key1", b"value_b")`
  3. `result = get(b"key1")`
- **Expected**: `result == Some(b"value_b")`
- **Methods exercised**: `put`, `get`

#### T1.4 `test_delete`

- **Description**: Delete a key, then confirm it is gone.
- **Setup**: `put(b"key1", b"value1")`
- **Steps**:
  1. `delete(b"key1")`
  2. `result = get(b"key1")`
- **Expected**: `result == None`
- **Methods exercised**: `put`, `delete`, `get`

#### T1.5 `test_delete_nonexistent`

- **Description**: Deleting a key that does not exist should not error.
- **Setup**: Empty backend.
- **Steps**:
  1. `delete(b"no_such_key")` -- should not panic or return error
- **Expected**: No error. Subsequent `get(b"no_such_key")` returns `None`.
- **Methods exercised**: `delete`, `get`

#### T1.6 `test_put_empty_value`

- **Description**: Values may be empty byte slices (used by Oxigraph for existence-only index entries).
- **Setup**: Empty backend.
- **Steps**:
  1. `put(b"key1", b"")`
  2. `result = get(b"key1")`
- **Expected**: `result == Some(b"")` (not `None`)
- **Methods exercised**: `put`, `get`

---

### 2. Batch Operations

#### T2.1 `test_batch_put`

- **Description**: Insert multiple key-value pairs in a single batch call.
- **Setup**: Empty backend.
- **Steps**:
  1. `batch_put([(b"a", b"1"), (b"b", b"2"), (b"c", b"3")])`
  2. Verify each key individually via `get`.
- **Expected**: All three keys retrievable with correct values.
- **Methods exercised**: `batch_put`, `get`

#### T2.2 `test_batch_put_overwrite`

- **Description**: Batch put overwrites existing keys.
- **Setup**: `put(b"a", b"old")`
- **Steps**:
  1. `batch_put([(b"a", b"new"), (b"b", b"val")])`
  2. `get(b"a")`
- **Expected**: `get(b"a") == Some(b"new")`
- **Methods exercised**: `batch_put`, `get`

#### T2.3 `test_batch_put_large`

- **Description**: Batch put with 1,000 entries to verify no truncation or partial writes.
- **Setup**: Empty backend.
- **Steps**:
  1. Generate 1,000 entries: key = `format!("key_{:05}", i)`, value = `format!("val_{}", i)`
  2. `batch_put(entries)`
  3. Verify 10 randomly sampled keys.
  4. Verify total count via prefix scan.
- **Expected**: All 1,000 entries present and correct.
- **Methods exercised**: `batch_put`, `get`, `batch_scan`

#### T2.4 `test_batch_delete`

- **Description**: Delete multiple keys in a single batch call.
- **Setup**: `batch_put([(b"a", b"1"), (b"b", b"2"), (b"c", b"3")])`
- **Steps**:
  1. `batch_delete([b"a", b"c"])`
  2. Verify: `get(b"a") == None`, `get(b"b") == Some(b"2")`, `get(b"c") == None`
- **Expected**: Only deleted keys are removed; others remain.
- **Methods exercised**: `batch_put`, `batch_delete`, `get`

#### T2.5 `test_batch_empty`

- **Description**: Batch put/delete with an empty list should be a no-op.
- **Setup**: `put(b"x", b"y")`
- **Steps**:
  1. `batch_put([])`
  2. `batch_delete([])`
  3. `get(b"x")`
- **Expected**: `get(b"x") == Some(b"y")`. No errors.
- **Methods exercised**: `batch_put`, `batch_delete`, `get`

---

### 3. Range Scans

#### T3.1 `test_prefix_scan`

- **Description**: Scan all keys with a given prefix, returned in lexicographic order.
- **Setup**:
  ```
  batch_put([
    (b"ns/alpha", b"1"),
    (b"ns/beta",  b"2"),
    (b"ns/gamma", b"3"),
    (b"other/x",  b"4"),
  ])
  ```
- **Steps**:
  1. `results = batch_scan(b"ns/")`
  2. Collect all (key, value) pairs.
- **Expected**: Exactly 3 results: `(ns/alpha, 1), (ns/beta, 2), (ns/gamma, 3)` in that order. `other/x` excluded.
- **Methods exercised**: `batch_scan`

#### T3.2 `test_prefix_scan_empty_result`

- **Description**: Prefix scan with no matching keys returns an empty iterator.
- **Setup**: `put(b"abc", b"1")`
- **Steps**:
  1. `results = batch_scan(b"xyz/")`
- **Expected**: Empty iterator, no error.
- **Methods exercised**: `batch_scan`

#### T3.3 `test_bounded_range_scan`

- **Description**: Scan keys within an inclusive start and exclusive end range.
- **Setup**:
  ```
  batch_put([
    (b"a", b"1"), (b"b", b"2"), (b"c", b"3"),
    (b"d", b"4"), (b"e", b"5"),
  ])
  ```
- **Steps**:
  1. `results = range_scan(b"b"..b"d")` (if supported by trait)
  2. Alternatively, test via prefix scan + manual bounds.
- **Expected**: Results: `(b, 2), (c, 3)`. Keys `a`, `d`, `e` excluded.
- **Methods exercised**: `range_scan` or `batch_scan` with bounds

#### T3.4 `test_full_scan`

- **Description**: Scan with empty prefix returns all keys in the backend (within the test prefix namespace).
- **Setup**: Insert 5 keys with the test prefix.
- **Steps**:
  1. `results = batch_scan(test_prefix)` -- scan with the test-specific prefix
- **Expected**: All 5 keys returned in lexicographic order.
- **Methods exercised**: `batch_scan`

#### T3.5 `test_scan_after_delete`

- **Description**: Deleted keys do not appear in subsequent scans.
- **Setup**: `batch_put([(b"p/a", b"1"), (b"p/b", b"2"), (b"p/c", b"3")])`
- **Steps**:
  1. `delete(b"p/b")`
  2. `results = batch_scan(b"p/")`
- **Expected**: Results: `(p/a, 1), (p/c, 3)`. Deleted key `p/b` absent.
- **Methods exercised**: `batch_scan`, `delete`

#### T3.6 `test_scan_single_result`

- **Description**: Prefix scan matching exactly one key.
- **Setup**: `batch_put([(b"unique_prefix/only", b"val"), (b"other/x", b"y")])`
- **Steps**:
  1. `results = batch_scan(b"unique_prefix/")`
- **Expected**: Exactly one result: `(unique_prefix/only, val)`.
- **Methods exercised**: `batch_scan`

---

### 4. Lexicographic Ordering

#### T4.1 `test_byte_order_simple`

- **Description**: Keys are returned in unsigned byte-lexicographic order, not string/locale order.
- **Setup**:
  ```
  batch_put([
    (b"key_c", b"3"),
    (b"key_a", b"1"),
    (b"key_b", b"2"),
  ])
  ```
- **Steps**:
  1. `results = batch_scan(b"key_")`
- **Expected**: Order: `key_a`, `key_b`, `key_c`.
- **Methods exercised**: `batch_scan`

#### T4.2 `test_byte_order_binary_keys`

- **Description**: Ordering is correct for non-UTF8 binary keys (as used by Oxigraph's encoded term keys).
- **Setup**:
  ```
  batch_put([
    ([0x01, 0xFF], b"first"),
    ([0x01, 0x00], b"second"),
    ([0x02, 0x00], b"third"),
  ])
  ```
- **Steps**:
  1. `results = batch_scan(&[0x01])`
- **Expected**: Order: `[0x01, 0x00]` before `[0x01, 0xFF]` (unsigned byte comparison). Key `[0x02, 0x00]` excluded by prefix.
- **Methods exercised**: `batch_scan`

#### T4.3 `test_byte_order_varying_key_lengths`

- **Description**: Keys of different lengths with the same prefix are ordered correctly (shorter key first if it is a prefix of the longer key).
- **Setup**:
  ```
  batch_put([
    (b"ab",  b"1"),
    (b"abc", b"2"),
    (b"a",   b"3"),
  ])
  ```
- **Steps**:
  1. `results = batch_scan(b"a")`
- **Expected**: Order: `a`, `ab`, `abc`.
- **Methods exercised**: `batch_scan`

#### T4.4 `test_byte_order_oxigraph_term_encoding`

- **Description**: Simulate Oxigraph's 33-byte encoded term keys (1 type byte + 32 value bytes). Verify ordering matches what Oxigraph expects for index lookups.
- **Setup**: Create 4 keys mimicking SPO index entries:
  ```
  // Type byte 0x04 (NamedNode) + 16-byte hash for S + 16-byte hash for P + 16-byte hash for O
  // Use controlled hash values to verify ordering
  key_a = [0x04] ++ [0x00; 16] ++ [0x00; 16] ++ [0x00; 16]  // smallest
  key_b = [0x04] ++ [0x00; 16] ++ [0x00; 16] ++ [0xFF; 16]  // same S,P, larger O
  key_c = [0x04] ++ [0x00; 16] ++ [0xFF; 16] ++ [0x00; 16]  // same S, larger P
  key_d = [0x04] ++ [0xFF; 16] ++ [0x00; 16] ++ [0x00; 16]  // larger S
  ```
- **Steps**:
  1. Insert all four keys (out of order).
  2. `results = batch_scan(&[0x04])`
- **Expected**: Order: `key_a`, `key_b`, `key_c`, `key_d`. This confirms the backend sorts multi-component keys correctly for Oxigraph's index access patterns.
- **Methods exercised**: `batch_scan`

---

### 5. Transaction Atomicity

#### T5.1 `test_transaction_commit`

- **Description**: All writes within a committed transaction are visible afterward.
- **Setup**: Empty backend.
- **Steps**:
  1. `txn = backend.transaction()`
  2. `txn.put(b"t1", b"v1")`
  3. `txn.put(b"t2", b"v2")`
  4. `txn.commit()`
  5. `get(b"t1")`, `get(b"t2")`
- **Expected**: Both keys visible after commit.
- **Methods exercised**: `transaction`, `Transaction::put`, `Transaction::commit`, `get`

#### T5.2 `test_transaction_rollback`

- **Description**: Writes in a rolled-back (or dropped) transaction are not visible.
- **Setup**: Empty backend.
- **Steps**:
  1. `txn = backend.transaction()`
  2. `txn.put(b"t1", b"v1")`
  3. `txn.put(b"t2", b"v2")`
  4. `txn.rollback()` (or `drop(txn)` without commit)
  5. `get(b"t1")`, `get(b"t2")`
- **Expected**: Both keys return `None`.
- **Methods exercised**: `transaction`, `Transaction::put`, `Transaction::rollback`, `get`

#### T5.3 `test_transaction_all_or_nothing`

- **Description**: If a transaction fails mid-way (simulated error), none of the writes persist.
- **Setup**: Empty backend.
- **Steps**:
  1. `txn = backend.transaction()`
  2. `txn.put(b"t1", b"v1")`
  3. `txn.put(b"t2", b"v2")`
  4. Simulate failure: drop the transaction without committing (or, for backends that support it, inject a conflict).
  5. `get(b"t1")`, `get(b"t2")`
- **Expected**: Both keys return `None`. No partial state.
- **Methods exercised**: `transaction`, `Transaction::put`, `get`

#### T5.4 `test_transaction_read_own_writes`

- **Description**: Within an open transaction, reads reflect writes made in the same transaction.
- **Setup**: Empty backend.
- **Steps**:
  1. `txn = backend.transaction()`
  2. `txn.put(b"t1", b"v1")`
  3. `result = txn.get(b"t1")`
- **Expected**: `result == Some(b"v1")` (read-own-writes semantics).
- **Methods exercised**: `transaction`, `Transaction::put`, `Transaction::get`

#### T5.5 `test_transaction_delete_within_txn`

- **Description**: Deletes within a transaction are respected on commit.
- **Setup**: `put(b"existing", b"val")`
- **Steps**:
  1. `txn = backend.transaction()`
  2. `txn.delete(b"existing")`
  3. `txn.put(b"new_key", b"new_val")`
  4. `txn.commit()`
  5. `get(b"existing")`, `get(b"new_key")`
- **Expected**: `existing` returns `None`, `new_key` returns `Some(b"new_val")`.
- **Methods exercised**: `transaction`, `Transaction::delete`, `Transaction::put`, `Transaction::commit`, `get`

#### T5.6 `test_transaction_batch_operations`

- **Description**: Batch put and batch delete work within a transaction.
- **Setup**: Empty backend.
- **Steps**:
  1. `txn = backend.transaction()`
  2. `txn.batch_put([(b"a", b"1"), (b"b", b"2"), (b"c", b"3")])`
  3. `txn.batch_delete([b"b"])`
  4. `txn.commit()`
  5. Verify: `a` present, `b` absent, `c` present.
- **Expected**: Batch operations within a transaction commit atomically.
- **Methods exercised**: `transaction`, `Transaction::batch_put`, `Transaction::batch_delete`, `Transaction::commit`, `get`

---

### 6. Snapshot Isolation

#### T6.1 `test_snapshot_point_in_time`

- **Description**: A snapshot sees data as of the moment it was taken, unaffected by subsequent writes.
- **Setup**: `put(b"s1", b"original")`
- **Steps**:
  1. `snap = backend.snapshot()`
  2. `put(b"s1", b"modified")`
  3. `put(b"s2", b"new")`
  4. `snap.get(b"s1")`
  5. `snap.get(b"s2")`
- **Expected**: `snap.get(b"s1") == Some(b"original")`, `snap.get(b"s2") == None`.
- **Methods exercised**: `snapshot`, `Snapshot::get`, `put`

#### T6.2 `test_snapshot_scan_consistency`

- **Description**: A snapshot's prefix scan reflects the state at snapshot time, not current state.
- **Setup**: `batch_put([(b"ns/a", b"1"), (b"ns/b", b"2")])`
- **Steps**:
  1. `snap = backend.snapshot()`
  2. `put(b"ns/c", b"3")` (after snapshot)
  3. `delete(b"ns/a")` (after snapshot)
  4. `results = snap.batch_scan(b"ns/")`
- **Expected**: Snapshot scan returns `(ns/a, 1), (ns/b, 2)`. Key `ns/c` absent; key `ns/a` still visible.
- **Methods exercised**: `snapshot`, `Snapshot::batch_scan`, `put`, `delete`

#### T6.3 `test_snapshot_does_not_block_writes`

- **Description**: Holding an open snapshot does not prevent new writes from committing.
- **Setup**: Empty backend.
- **Steps**:
  1. `snap = backend.snapshot()`
  2. `put(b"k1", b"v1")` -- must succeed, not block
  3. `get(b"k1")` -- via backend, not snapshot
  4. Drop snapshot.
- **Expected**: Write succeeds. `get(b"k1") == Some(b"v1")`. No deadlock.
- **Methods exercised**: `snapshot`, `put`, `get`

#### T6.4 `test_multiple_snapshots`

- **Description**: Multiple snapshots can coexist, each reflecting its own point in time.
- **Setup**: `put(b"k", b"v1")`
- **Steps**:
  1. `snap1 = backend.snapshot()`
  2. `put(b"k", b"v2")`
  3. `snap2 = backend.snapshot()`
  4. `put(b"k", b"v3")`
  5. Check: `snap1.get(b"k")`, `snap2.get(b"k")`, `backend.get(b"k")`
- **Expected**: `snap1` sees `v1`, `snap2` sees `v2`, current state is `v3`.
- **Methods exercised**: `snapshot`, `Snapshot::get`, `put`, `get`

---

### 7. Concurrent Access

#### T7.1 `test_concurrent_readers`

- **Description**: Multiple threads can read concurrently without interference.
- **Setup**: `batch_put([(b"k1", b"v1"), (b"k2", b"v2"), ..., (b"k10", b"v10")])`
- **Steps**:
  1. Spawn 10 threads, each reading all 10 keys.
  2. Join all threads.
- **Expected**: All threads see all 10 keys with correct values. No panics.
- **Methods exercised**: `get` (concurrent)

#### T7.2 `test_reader_writer_isolation`

- **Description**: A reader does not see partial state from an in-progress write transaction.
- **Setup**: `put(b"k", b"before")`
- **Steps**:
  1. Thread A: start transaction, `txn.put(b"k", b"during")`, sleep briefly, commit.
  2. Thread B: take a snapshot before thread A commits, read `b"k"`.
- **Expected**: Thread B's snapshot sees `b"before"`, not `b"during"`.
- **Methods exercised**: `transaction`, `snapshot`, `Snapshot::get`, `Transaction::put`, `Transaction::commit`

#### T7.3 `test_concurrent_write_transactions`

- **Description**: Two concurrent transactions writing to disjoint keys both succeed.
- **Setup**: Empty backend.
- **Steps**:
  1. Thread A: transaction, `put(b"a", b"1")`, commit.
  2. Thread B: transaction, `put(b"b", b"2")`, commit.
  3. (Threads run concurrently.)
  4. Verify both keys present.
- **Expected**: Both commits succeed; both keys visible.
- **Methods exercised**: `transaction`, `Transaction::put`, `Transaction::commit`, `get`

#### T7.4 `test_write_conflict_detection`

- **Description**: Two concurrent transactions writing to the same key result in at most one succeeding (optimistic conflict detection).
- **Setup**: `put(b"k", b"initial")`
- **Steps**:
  1. `txn1 = backend.transaction()`, `txn1.get(b"k")`
  2. `txn2 = backend.transaction()`, `txn2.get(b"k")`
  3. `txn1.put(b"k", b"from_txn1")`
  4. `txn2.put(b"k", b"from_txn2")`
  5. `txn1.commit()` -- should succeed
  6. `txn2.commit()` -- may fail with conflict error
- **Expected**: At least one commit succeeds. If both succeed, the final value is deterministic (last committer wins). The backend must not corrupt data. This test verifies the backend's conflict resolution behavior and documents it.
- **Methods exercised**: `transaction`, `Transaction::get`, `Transaction::put`, `Transaction::commit`

---

### 8. Edge Cases

#### T8.1 `test_empty_key`

- **Description**: The backend must handle empty keys gracefully (accept or reject with clear error -- both are conformant as long as behavior is documented).
- **Setup**: Empty backend.
- **Steps**:
  1. Attempt `put(b"", b"value")`
  2. If accepted, `get(b"")` should return `Some(b"value")`.
  3. If rejected, error should be explicit (not a panic).
- **Expected**: Defined behavior, no panic.
- **Methods exercised**: `put`, `get`

#### T8.2 `test_large_value`

- **Description**: Values up to 1 MB should be storable (Oxigraph's dictionary may store large literals).
- **Setup**: Empty backend.
- **Steps**:
  1. `value = vec![0xAB; 1_048_576]` (1 MB)
  2. `put(b"large_val_key", &value)`
  3. `result = get(b"large_val_key")`
- **Expected**: `result == Some(value)` -- exact byte-for-byte match.
- **Methods exercised**: `put`, `get`

#### T8.3 `test_maximum_key_size`

- **Description**: Keys up to 256 bytes should be storable (Oxigraph's index keys are ~100 bytes for quad entries: 4 x 33 bytes = 132 bytes, plus table prefix).
- **Setup**: Empty backend.
- **Steps**:
  1. `key = vec![0xFF; 256]`
  2. `put(&key, b"val")`
  3. `result = get(&key)`
- **Expected**: `result == Some(b"val")`.
- **Methods exercised**: `put`, `get`

#### T8.4 `test_binary_key_all_byte_values`

- **Description**: Keys containing all 256 possible byte values are handled correctly (no null-termination bugs, no encoding issues).
- **Setup**: Empty backend.
- **Steps**:
  1. `key = (0u8..=255u8).collect::<Vec<u8>>()`
  2. `put(&key, b"all_bytes")`
  3. `result = get(&key)`
- **Expected**: `result == Some(b"all_bytes")`.
- **Methods exercised**: `put`, `get`

#### T8.5 `test_repeated_put_delete_cycles`

- **Description**: Repeatedly writing and deleting the same key does not leak state.
- **Setup**: Empty backend.
- **Steps**:
  1. For `i` in `0..100`:
     - `put(b"cycle_key", format!("v{}", i).as_bytes())`
     - `delete(b"cycle_key")`
  2. `result = get(b"cycle_key")`
- **Expected**: `result == None` after final delete.
- **Methods exercised**: `put`, `delete`, `get`

#### T8.6 `test_scan_with_key_at_prefix_boundary`

- **Description**: A key that equals exactly the prefix (no trailing bytes) is included or excluded consistently.
- **Setup**: `batch_put([(b"prefix", b"exact"), (b"prefix/child", b"child")])`
- **Steps**:
  1. `results = batch_scan(b"prefix")`
- **Expected**: Behavior depends on whether `batch_scan` is prefix-match (include `b"prefix"` because it starts with the prefix) or strictly-after. Document the expected behavior and assert consistently. Typically, prefix scan includes the exact-match key.
- **Methods exercised**: `batch_scan`

---

### 9. Dictionary Table (`id2str`) Round-Trip

These tests verify the encoding/decoding layer that sits above the raw KV backend. They exercise `StorageBackend` through the dictionary abstraction.

#### T9.1 `test_id2str_named_node`

- **Description**: Store a NamedNode IRI, retrieve it by hash ID.
- **Setup**: Empty backend.
- **Steps**:
  1. Encode `<http://example.org/resource>` to its 128-bit hash ID.
  2. `put(id2str_key(hash), serialized_named_node)`
  3. `result = get(id2str_key(hash))`
  4. Decode result back to NamedNode.
- **Expected**: Round-trip produces identical IRI string.
- **Methods exercised**: `put`, `get`

#### T9.2 `test_id2str_literal_types`

- **Description**: Round-trip all literal types: small string, big string, integer, float, boolean, date.
- **Setup**: Empty backend.
- **Steps**:
  1. For each literal type, encode, store via `put`, retrieve via `get`, decode.
- **Expected**: All literals round-trip correctly.
- **Methods exercised**: `put`, `get`

#### T9.3 `test_id2str_blank_node_variants`

- **Description**: Round-trip blank node variants: NumericalBlankNode, SmallBlankNode, BigBlankNode.
- **Setup**: Empty backend.
- **Steps**:
  1. Create one of each blank node type, encode, store, retrieve, decode.
- **Expected**: All three variants round-trip correctly.
- **Methods exercised**: `put`, `get`

#### T9.4 `test_id2str_hash_collision_guard`

- **Description**: Two different IRIs producing (hypothetically) the same hash prefix are distinguishable. In practice, verify that two distinct IRIs get distinct hash keys.
- **Setup**: Empty backend.
- **Steps**:
  1. Store two distinct NamedNodes.
  2. Retrieve both.
- **Expected**: Each retrieves its own value, not the other's.
- **Methods exercised**: `put`, `get`

---

### 10. Index Table Prefix Scan Behavior

These tests verify that each of Oxigraph's index permutation tables supports the correct prefix scan patterns. They use synthetic encoded keys mimicking Oxigraph's format.

For each index table, the key structure is: `[table_prefix_byte] ++ [component1] ++ [component2] ++ [component3] ++ [optional component4]`

Each component is a 33-byte encoded term (1 type byte + 32 value bytes).

#### T10.1 `test_spo_index_subject_prefix_scan`

- **Description**: SPO index supports scanning by subject prefix (find all triples for a given subject).
- **Setup**: Insert 5 SPO entries with 2 distinct subjects (S1 with 3 triples, S2 with 2 triples).
- **Steps**:
  1. `results = batch_scan(spo_prefix ++ S1_encoded)`
- **Expected**: Exactly 3 results, all with subject S1, in P-then-O order.
- **Methods exercised**: `batch_put`, `batch_scan`

#### T10.2 `test_spo_index_subject_predicate_prefix_scan`

- **Description**: SPO index supports scanning by (subject, predicate) prefix.
- **Setup**: Insert entries: (S1,P1,O1), (S1,P1,O2), (S1,P2,O3).
- **Steps**:
  1. `results = batch_scan(spo_prefix ++ S1_encoded ++ P1_encoded)`
- **Expected**: 2 results: (S1,P1,O1) and (S1,P1,O2).
- **Methods exercised**: `batch_put`, `batch_scan`

#### T10.3 `test_pos_index_predicate_prefix_scan`

- **Description**: POS index supports scanning by predicate prefix (find all subjects/objects for a given predicate).
- **Setup**: Insert POS entries for predicates P1 (3 entries) and P2 (2 entries).
- **Steps**:
  1. `results = batch_scan(pos_prefix ++ P1_encoded)`
- **Expected**: Exactly 3 results, all for predicate P1.
- **Methods exercised**: `batch_put`, `batch_scan`

#### T10.4 `test_osp_index_object_prefix_scan`

- **Description**: OSP index supports scanning by object prefix.
- **Setup**: Insert OSP entries for objects O1 (2 entries) and O2 (3 entries).
- **Steps**:
  1. `results = batch_scan(osp_prefix ++ O1_encoded)`
- **Expected**: Exactly 2 results, all for object O1.
- **Methods exercised**: `batch_put`, `batch_scan`

#### T10.5 `test_gspo_index_graph_prefix_scan`

- **Description**: GSPO index supports scanning by graph prefix (all quads in a named graph).
- **Setup**: Insert GSPO entries in 2 graphs (G1 with 3 quads, G2 with 2 quads).
- **Steps**:
  1. `results = batch_scan(gspo_prefix ++ G1_encoded)`
- **Expected**: Exactly 3 results, all in graph G1, ordered by S, then P, then O.
- **Methods exercised**: `batch_put`, `batch_scan`

#### T10.6 `test_spog_index_full_key_exact_match`

- **Description**: An exact SPOG key lookup returns the entry (existence check for a specific quad).
- **Setup**: Insert one SPOG entry for (S1, P1, O1, G1).
- **Steps**:
  1. `result = get(spog_prefix ++ S1 ++ P1 ++ O1 ++ G1)`
- **Expected**: `result == Some(b"")` (empty value; the key's existence is the information).
- **Methods exercised**: `put`, `get`

#### T10.7 `test_cross_index_consistency`

- **Description**: After inserting a quad into all index tables (SPO, POS, OSP, SPOG, POSG, OSPG, GSPO), each index returns the quad via its respective prefix scan.
- **Setup**: Insert one quad (S, P, O, G) into all 7 index tables.
- **Steps**:
  1. Verify `batch_scan(spo_prefix ++ S)` includes (S,P,O).
  2. Verify `batch_scan(pos_prefix ++ P)` includes (P,O,S).
  3. Verify `batch_scan(osp_prefix ++ O)` includes (O,S,P).
  4. Verify `batch_scan(gspo_prefix ++ G)` includes (G,S,P,O).
  5. Verify `batch_scan(spog_prefix ++ S)` includes (S,P,O,G).
  6. Verify `batch_scan(posg_prefix ++ P)` includes (P,O,S,G).
  7. Verify `batch_scan(ospg_prefix ++ O)` includes (O,S,P,G).
- **Expected**: The quad is discoverable from every index. This is the fundamental invariant that Oxigraph's query engine relies on.
- **Methods exercised**: `batch_put`, `batch_scan`

#### T10.8 `test_graph_directory_table`

- **Description**: The graph directory table lists all named graphs.
- **Setup**: Insert entries into the graph directory table for graphs G1, G2, G3.
- **Steps**:
  1. `results = batch_scan(graph_dir_prefix)`
- **Expected**: 3 entries, one per graph, in lexicographic order by encoded graph ID.
- **Methods exercised**: `batch_put`, `batch_scan`

---

## Test Summary Matrix

| Category | Test IDs | Methods Exercised | Priority |
|----------|----------|-------------------|----------|
| Basic CRUD | T1.1--T1.6 | `get`, `put`, `delete` | P0 (must pass) |
| Batch Operations | T2.1--T2.5 | `batch_put`, `batch_delete`, `get` | P0 |
| Range Scans | T3.1--T3.6 | `batch_scan` | P0 |
| Lexicographic Ordering | T4.1--T4.4 | `batch_scan` | P0 |
| Transaction Atomicity | T5.1--T5.6 | `transaction`, `commit`, `rollback` | P0 |
| Snapshot Isolation | T6.1--T6.4 | `snapshot`, `Snapshot::get`, `Snapshot::batch_scan` | P1 |
| Concurrent Access | T7.1--T7.4 | All (multi-threaded) | P1 |
| Edge Cases | T8.1--T8.6 | `put`, `get`, `delete`, `batch_scan` | P1 |
| Dictionary (id2str) | T9.1--T9.4 | `put`, `get` | P1 |
| Index Tables | T10.1--T10.8 | `batch_put`, `batch_scan`, `get` | P0 |

**P0** = must pass before a backend is considered conformant.
**P1** = should pass; failures require documented justification and tracking issue.

---

## Implementation Notes for `/rust-dev`

1. **File location**: `crates/oxigraph-tikv/tests/conformance/` (or `tests/backend_conformance/` at workspace root).

2. **Test helpers crate**: Consider a `oxigraph-test-helpers` crate in the workspace that provides:
   - The `conformance_tests!` macro
   - Key encoding helpers (synthetic SPO/POS/OSP keys)
   - Backend factory functions for each implementation
   - The `test_prefix()` function for key isolation

3. **Feature gating**: TiKV tests behind `#[cfg(feature = "tikv")]` and require `TIKV_PD_ENDPOINTS` env var. Tests should skip gracefully (not fail) if the env var is missing:
   ```rust
   fn tikv_backend_or_skip() -> Option<TikvBackend> {
       let endpoints = std::env::var("TIKV_PD_ENDPOINTS").ok()?;
       Some(TikvBackend::connect(&endpoints).expect("TiKV connection failed"))
   }
   ```

4. **Concurrency tests (T7.x)**: Use `std::thread::scope` (Rust 1.63+) for clean thread management. For async backends, use `tokio::spawn` within a `#[tokio::test]` harness.

5. **Timing sensitivity**: T7.2 (reader-writer isolation) uses a barrier or channel to coordinate threads, not `sleep`. Use `std::sync::Barrier` to ensure the snapshot is taken before the write commits.

6. **Index table tests (T10.x)**: Define constants for table prefix bytes matching Oxigraph's encoding:
   ```rust
   const SPO_PREFIX: u8 = 0x01;  // actual values from Oxigraph source
   const POS_PREFIX: u8 = 0x02;
   const OSP_PREFIX: u8 = 0x03;
   // ... etc
   ```
   Extract exact values from `oxigraph/src/storage/` during Phase 1.3.

7. **Assertion messages**: Every assertion should include the test ID and a human-readable failure message:
   ```rust
   assert_eq!(result, Some(b"v1".to_vec()), "[T1.1] get after put should return the stored value");
   ```
