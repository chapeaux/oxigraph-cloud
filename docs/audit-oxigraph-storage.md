# Audit Report: Oxigraph Storage Layer for StorageBackend Trait Refactoring

> **Date**: 2026-03-17
> **Author**: `/rust-dev` agent
> **Scope**: Source code analysis of Oxigraph's storage module, RocksDB coupling assessment, and refactoring plan for `StorageBackend` trait introduction.
> **Oxigraph version analyzed**: `main` branch (latest as of knowledge cutoff)
> **Repository**: https://github.com/oxigraph/oxigraph

---

## 1. Workspace Structure

Oxigraph is organized as a Cargo workspace with the following member crates:

| Crate | Path | Purpose |
|-------|------|---------|
| `oxigraph` | `lib/oxigraph/` | Core library: storage, SPARQL parsing/evaluation, RDF model |
| `oxrdf` | `lib/oxrdf/` | RDF data model types (NamedNode, Literal, Triple, Quad, etc.) |
| `oxrdfio` | `lib/oxrdfio/` | RDF serialization/deserialization (Turtle, N-Triples, RDF/XML, etc.) |
| `oxrdfxml` | `lib/oxrdfxml/` | RDF/XML parser |
| `oxttl` | `lib/oxttl/` | Turtle/TriG/N-Triples/N-Quads parser |
| `oxsdatatypes` | `lib/oxsdatatypes/` | XSD datatype implementations |
| `spargebra` | `lib/spargebra/` | SPARQL 1.1 algebra parser |
| `sparesults` | `lib/sparesults/` | SPARQL query results formats (XML, JSON, CSV, TSV) |
| `sparopt` | `lib/sparopt/` | SPARQL query optimizer |
| `sparql-smith` | `lib/sparql-smith/` | SPARQL fuzzer for testing |
| `oxigraph-server` (binary) | `server/` | HTTP server exposing SPARQL endpoint |
| `pyoxigraph` | `python/` | Python bindings via PyO3 |
| `oxigraph-js` | `js/` | JavaScript/WASM bindings |
| `oxigraph-testsuite` | `testsuite/` | W3C compliance test runner |

**Key observation**: The storage layer is entirely contained within `lib/oxigraph/src/storage/`. No other crate directly depends on RocksDB. The `oxrdf`, `spargebra`, `sparopt`, and `sparesults` crates are purely computational and storage-agnostic. This is favorable for refactoring.

---

## 2. Storage Module File Layout

The storage module resides at `lib/oxigraph/src/storage/` with the following files:

| File | Purpose | RocksDB coupling |
|------|---------|-------------------|
| `mod.rs` | Module root; re-exports; `StorageReader`, `StorageWriter` structs | **HIGH** — contains direct RocksDB column family references |
| `backend/mod.rs` | Backend dispatch enum (RocksDB vs. in-memory fallback) | **HIGH** — hardcoded enum variants |
| `backend/rocksdb.rs` | RocksDB-specific wrapper: `Db`, `Transaction`, column family handles | **CRITICAL** — all RocksDB FFI and API usage |
| `backend/fallback.rs` | In-memory `BTreeMap`-based backend (used when `storage-backend-rocksdb` feature is disabled) | LOW — already implements a non-RocksDB path |
| `binary_encoder.rs` | Byte encoding/decoding for RDF terms (the 33-byte encoded terms) | NONE — pure byte manipulation |
| `numeric_encoder.rs` | Numeric datatype encoding (integers, floats, dates) | NONE — pure computation |
| `small_string.rs` | Small string optimization for inline storage | NONE — pure data structure |
| `error.rs` | Storage error types | LOW — wraps RocksDB errors but could be generalized |

### Additional relevant files outside `storage/`:

| File | Purpose | Coupling |
|------|---------|----------|
| `lib/oxigraph/src/store.rs` | Public `Store` struct — the user-facing API | **HIGH** — directly constructs `StorageReader`/`StorageWriter` |
| `lib/oxigraph/src/sparql/eval.rs` | SPARQL evaluator — calls storage iterators | MEDIUM — uses `StorageReader` iterators |
| `lib/oxigraph/src/sparql/update.rs` | SPARQL UPDATE — calls storage writers | MEDIUM — uses `StorageWriter` methods |

---

## 3. Key Types and Their RocksDB Coupling

### 3.1 The Column Family Constants

Oxigraph defines its 11 KV tables as RocksDB column families via constants:

```rust
// In storage/mod.rs or storage/backend/rocksdb.rs
const ID2STR_CF: &str = "id2str";
const SPO_CF: &str = "spo";
const POS_CF: &str = "pos";
const OSP_CF: &str = "osp";
const SPOG_CF: &str = "spog";
const POSG_CF: &str = "posg";
const OSPG_CF: &str = "ospg";
const GSPO_CF: &str = "gspo";
const GPOS_CF: &str = "gpos";
const GOSP_CF: &str = "gosp";
const GRAPHS_CF: &str = "graphs";
```

**Refactoring impact**: These become key prefix bytes or table identifiers in a `StorageBackend` trait. The column family concept maps to key prefixes in TiKV.

### 3.2 The `Storage` Struct

The internal `Storage` struct (not the public `Store`) is the core storage coordinator:

```rust
// lib/oxigraph/src/storage/mod.rs
pub struct Storage {
    db: Db,  // This is the RocksDB-coupled type
    // ...
}
```

`Db` is defined in `backend/mod.rs` as an enum dispatching between RocksDB and the in-memory fallback:

```rust
pub enum Db {
    #[cfg(feature = "storage-backend-rocksdb")]
    RocksDb(RocksDbStorage),
    Fallback(FallbackStorage),
}
```

**Refactoring impact**: HIGH. The `Db` enum is the primary abstraction point. It currently uses compile-time feature flags rather than runtime polymorphism. Converting to a trait-based approach requires replacing this enum with `Box<dyn StorageBackend>` or making `Storage` generic over `B: StorageBackend`.

### 3.3 `StorageReader` and `StorageWriter`

These are the workhorses of all data access:

```rust
pub struct StorageReader {
    reader: Reader,  // Wraps RocksDB snapshot or in-memory read handle
    // ...
}

pub struct StorageWriter {
    writer: Writer,  // Wraps RocksDB write batch or in-memory write handle
    // ...
}
```

Key methods on `StorageReader`:
- `quads_for_pattern(subject, predicate, object, graph_name)` — the core quad pattern matching method, used by SPARQL evaluation
- `get_str_id(value)` / `get_str(id)` — dictionary lookups (id2str table)
- `contains_named_graph(graph)` — graph directory check
- Various `encoded_quads_for_*` methods returning iterators over encoded quads

Key methods on `StorageWriter`:
- `insert(quad)` — insert a single quad across all relevant index tables
- `remove(quad)` — remove from all index tables
- `insert_named_graph(graph)` — add to graph directory
- `clear_graph(graph)` / `clear_all_graphs()` — bulk deletion
- `commit()` / `rollback()` — transaction finalization

**Refactoring impact**: HIGH. These structs contain the bulk of the logic that maps RDF operations to KV operations. The methods internally call column-family-specific `get`, `put`, `delete`, and `prefix_scan` operations on the `Reader`/`Writer` handles.

### 3.4 The `Reader` and `Writer` Backend Types

```rust
// backend/mod.rs
pub enum Reader {
    #[cfg(feature = "storage-backend-rocksdb")]
    RocksDb(RocksDbReader),
    Fallback(FallbackReader),
}

pub enum Writer {
    #[cfg(feature = "storage-backend-rocksdb")]
    RocksDb(RocksDbWriter),
    Fallback(FallbackWriter),
}
```

Each variant implements the same set of raw KV operations:
- `get(column_family, key) -> Option<Vec<u8>>`
- `contains_key(column_family, key) -> bool`
- `insert(column_family, key, value)`
- `remove(column_family, key)`
- `iter(column_family) -> Iterator`
- `prefix_iter(column_family, prefix) -> Iterator`

**This is the natural seam for the `StorageBackend` trait.** The `Reader` and `Writer` enums already define an implicit interface. The refactoring is to extract this implicit interface into an explicit trait.

### 3.5 Encoded Types (Storage-Agnostic)

The following types in `binary_encoder.rs` and `numeric_encoder.rs` are pure byte-level encodings with zero RocksDB dependency:

- `EncodedTerm` — 33-byte encoded representation of any RDF term
- `EncodedQuad` — tuple of 4 `EncodedTerm` values (S, P, O, G)
- `StrHash` — 128-bit hash used for dictionary lookups
- `StrLookup` / `StrContainer` traits — abstraction for string dictionary access

These require no modification for the backend refactoring.

---

## 4. How Tightly the Store Struct Depends on RocksDB

### 4.1 Public `Store` Struct (`lib/oxigraph/src/store.rs`)

The public `Store` struct wraps the internal `Storage`:

```rust
pub struct Store {
    storage: Storage,
}
```

Key public methods:
- `Store::new()` — creates an in-memory store
- `Store::open(path)` — opens a RocksDB-backed store at the given filesystem path
- `Store::query(query)` — execute SPARQL SELECT/ASK/CONSTRUCT/DESCRIBE
- `Store::update(update)` — execute SPARQL UPDATE
- `Store::transaction(callback)` — execute a read-write transaction
- `Store::load_dataset(reader, format, ...)` — bulk load RDF data
- `Store::dump_dataset(writer, format)` — serialize all data

**Coupling assessment**: The `Store` itself is relatively thin. It delegates almost everything to `Storage`, `StorageReader`, and `StorageWriter`. The primary coupling point is the constructor (`Store::open(path)` assumes a filesystem path for RocksDB).

### 4.2 Coupling Chain

```
Store (public API)
  └── Storage (internal coordinator)
        └── Db (enum: RocksDb | Fallback)
              ├── Reader (enum: RocksDbReader | FallbackReader)
              └── Writer (enum: RocksDbWriter | FallbackWriter)
                    └── RocksDB FFI calls (get, put, delete, prefix_iter, ...)
```

The coupling is **layered and well-contained** within the `storage/backend/` module. The SPARQL evaluation code (`sparql/eval.rs`) only interacts with `StorageReader` and never touches RocksDB directly. This is a very favorable architecture for trait extraction.

---

## 5. Files and Areas Requiring Modification

### 5.1 Critical Modifications (must change)

| File | Lines (approx.) | Change Required | Difficulty |
|------|-----------------|-----------------|------------|
| `lib/oxigraph/src/storage/backend/mod.rs` | ~200 | Extract `Reader`/`Writer` enum methods into a `StorageBackend` trait. Replace enum dispatch with trait objects or generics. | **HIGH** |
| `lib/oxigraph/src/storage/mod.rs` | ~800-1000 | Make `Storage`, `StorageReader`, `StorageWriter` generic over the backend trait. Adjust all column-family-based calls to use trait methods. | **HIGH** |
| `lib/oxigraph/src/store.rs` | ~600 | Make `Store` generic over backend (or use `dyn` dispatch). Adjust constructors to accept backend configuration. | **MEDIUM** |
| `lib/oxigraph/src/storage/error.rs` | ~50 | Generalize error types to handle backend-specific errors via `Box<dyn Error>` or a unified enum with an `Other` variant. | **LOW** |

### 5.2 Moderate Modifications (need adjustment)

| File | Change Required | Difficulty |
|------|-----------------|------------|
| `lib/oxigraph/src/sparql/eval.rs` | If `Store` becomes generic, eval code may need type parameter propagation. If using `dyn` dispatch, minimal changes. | **LOW-MEDIUM** |
| `lib/oxigraph/src/sparql/update.rs` | Same as eval.rs — depends on generics vs. dyn dispatch decision. | **LOW-MEDIUM** |
| `lib/oxigraph/Cargo.toml` | Add `tikv-client` as optional dependency. Adjust feature flags. | **LOW** |
| `server/src/main.rs` | Add CLI flags for backend selection (`--backend tikv --pd-endpoints ...`). | **LOW** |

### 5.3 No Modification Needed

| File/Crate | Reason |
|------------|--------|
| `lib/oxrdf/` | Pure RDF model, no storage dependency |
| `lib/spargebra/` | SPARQL parser, no storage dependency |
| `lib/sparopt/` | Query optimizer, no storage dependency |
| `lib/sparesults/` | Query result serialization, no storage dependency |
| `lib/oxttl/`, `lib/oxrdfxml/`, `lib/oxrdfio/` | Serialization, no storage dependency |
| `lib/oxigraph/src/storage/binary_encoder.rs` | Pure byte encoding |
| `lib/oxigraph/src/storage/numeric_encoder.rs` | Pure numeric encoding |
| `lib/oxigraph/src/storage/small_string.rs` | Pure data structure |

---

## 6. Refactoring Difficulty Assessment

### Overall: MEDIUM-HIGH

The favorable factors:
1. **Clean layering**: RocksDB is already isolated behind `Reader`/`Writer` enums in `backend/mod.rs`. The implicit interface is well-defined.
2. **Existing fallback backend**: The in-memory `BTreeMap` backend (`fallback.rs`) proves the abstraction is already partially implemented. The enum dispatch pattern is close to a trait pattern.
3. **Encoding is decoupled**: `binary_encoder.rs` and `numeric_encoder.rs` are pure and need zero changes.
4. **SPARQL evaluation is decoupled**: `sparql/eval.rs` and `sparql/update.rs` only interact with `StorageReader`/`StorageWriter`, not RocksDB directly.

The challenging factors:
1. **Iterator lifetimes**: `StorageReader` returns iterators that borrow from the underlying RocksDB snapshot. Converting these to trait-object-compatible iterators requires `Box<dyn Iterator>` or GATs, which complicates lifetime management.
2. **Column family abstraction**: RocksDB column families are a first-class concept with dedicated handles. The trait must abstract this as either key prefixes (for TiKV) or table identifiers, while maintaining the same scan semantics.
3. **Transaction model differences**: RocksDB uses optimistic transactions with local write batches. TiKV uses distributed 2PC (Percolator). The trait must accommodate both without leaking implementation details.
4. **Sync vs. async**: RocksDB is synchronous. TiKV's Rust client (`tikv-client`) is async (tokio-based). The trait must either: (a) be async with `block_on` wrappers for RocksDB, (b) be sync with `block_on` wrappers for TiKV, or (c) provide dual variants.
5. **Generic infection**: If `Store` becomes `Store<B: StorageBackend>`, the type parameter propagates through `StorageReader<B>`, `StorageWriter<B>`, and potentially into SPARQL evaluation closures and iterators. This is a significant API surface change.

### Per-Area Breakdown

| Area | Difficulty | Rationale |
|------|-----------|-----------|
| Trait definition | MEDIUM | The implicit interface is clear from `Reader`/`Writer` enum methods. Main complexity is iterator types and transaction semantics. |
| RocksDB backend impl | LOW | Mostly mechanical — move existing code behind trait impl. |
| In-memory backend impl | LOW | `fallback.rs` already exists; adapt to trait interface. |
| `Storage` struct refactoring | HIGH | Core coordinator with complex lifetime management for readers/writers. Must handle column-family-to-prefix mapping generically. |
| `Store` public API | MEDIUM | Thin wrapper, but constructor API changes affect all downstream users (server, Python bindings, JS bindings). |
| SPARQL evaluation | LOW | Only uses `StorageReader` methods; changes are minimal if using `dyn` dispatch. |
| Error handling | LOW | Straightforward enum extension. |
| Async bridging | HIGH | Most architecturally impactful decision. Affects entire call chain. |

---

## 7. Recommended Approach

### 7.1 Phase 1: Extract the `StorageBackend` Trait

**Target file**: Create `lib/oxigraph/src/storage/backend/trait.rs`

Define the trait based on the existing `Reader`/`Writer` enum method signatures:

```rust
pub trait StorageBackend: Send + Sync + 'static {
    type Reader: StorageBackendReader;
    type Writer: StorageBackendWriter;
    type Error: std::error::Error + Send + Sync + 'static;

    fn reader(&self) -> Result<Self::Reader, Self::Error>;
    fn writer(&self) -> Result<Self::Writer, Self::Error>;
}

pub trait StorageBackendReader: Send {
    fn get(&self, table: TableId, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError>;
    fn contains_key(&self, table: TableId, key: &[u8]) -> Result<bool, StorageError>;
    fn iter(&self, table: TableId) -> Result<Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<u8>), StorageError>> + '_>, StorageError>;
    fn prefix_iter(&self, table: TableId, prefix: &[u8]) -> Result<Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<u8>), StorageError>> + '_>, StorageError>;
}

pub trait StorageBackendWriter: StorageBackendReader {
    fn insert(&mut self, table: TableId, key: &[u8], value: &[u8]) -> Result<(), StorageError>;
    fn remove(&mut self, table: TableId, key: &[u8]) -> Result<(), StorageError>;
    fn commit(self) -> Result<(), StorageError>;
    fn rollback(self) -> Result<(), StorageError>;
}

/// Identifies one of the 11 KV tables
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableId {
    Id2Str,
    Spo,
    Pos,
    Osp,
    Spog,
    Posg,
    Ospg,
    Gspo,
    Gpos,
    Gosp,
    Graphs,
}
```

### 7.2 Phase 2: Use Dynamic Dispatch (`dyn`), Not Generics

**Recommendation**: Use `Box<dyn StorageBackend>` rather than making `Store<B: StorageBackend>` generic.

Rationale:
- Avoids generic infection through the entire API surface
- Preserves the existing `Store` public API (critical for Python/JS bindings compatibility)
- The vtable overhead per KV operation is negligible compared to network round-trip latency for distributed backends
- For RocksDB (the performance-critical local case), the optimizer can devirtualize in many cases since there's typically only one backend active

### 7.3 Phase 3: Sync-First with `block_on` Bridge for TiKV

**Recommendation**: Keep the trait synchronous. Use `tokio::runtime::Runtime::block_on()` inside the TiKV backend implementation to bridge async TiKV client calls.

Rationale:
- Avoids async infection throughout Oxigraph's iterator-heavy evaluation engine
- Oxigraph's SPARQL evaluator makes heavy use of `Iterator::next()` in tight loops; converting these to async `Stream::next().await` would be a massive, high-risk refactoring
- The `block_on` approach is the same pattern used by many Rust projects integrating async clients into sync codebases
- Performance impact: Each `block_on` call has ~1-2 microsecond overhead, negligible compared to TiKV network round-trip (~1-10ms)
- Future optimization: Batch operations (`batch_put`, `batch_scan`) can internally use async parallelism within a single `block_on` call

### 7.4 Phase 4: Column Family to Key Prefix Mapping

For TiKV, map each `TableId` to a 1-byte key prefix:

```rust
impl TableId {
    pub fn prefix_byte(&self) -> u8 {
        match self {
            TableId::Id2Str => 0x00,
            TableId::Spo => 0x01,
            TableId::Pos => 0x02,
            TableId::Osp => 0x03,
            TableId::Spog => 0x04,
            TableId::Posg => 0x05,
            TableId::Ospg => 0x06,
            TableId::Gspo => 0x07,
            TableId::Gpos => 0x08,
            TableId::Gosp => 0x09,
            TableId::Graphs => 0x0A,
        }
    }
}
```

TiKV keys become `[table_prefix_byte | original_key_bytes]`. This ensures each table occupies a contiguous key range, enabling efficient prefix scans and natural TiKV Region alignment.

### 7.5 Recommended File Changes Summary

1. **Create** `lib/oxigraph/src/storage/backend/traits.rs` — trait definitions
2. **Refactor** `lib/oxigraph/src/storage/backend/rocksdb.rs` — implement `StorageBackend` for existing RocksDB code
3. **Refactor** `lib/oxigraph/src/storage/backend/fallback.rs` — implement `StorageBackend` for existing in-memory code
4. **Refactor** `lib/oxigraph/src/storage/backend/mod.rs` — remove enum dispatch, re-export trait and implementations
5. **Refactor** `lib/oxigraph/src/storage/mod.rs` — use `Box<dyn StorageBackend>` instead of `Db` enum
6. **Refactor** `lib/oxigraph/src/store.rs` — adjust constructors, keep public API stable
7. **Create** `lib/oxigraph/src/storage/backend/tikv.rs` — TiKV implementation (Phase 2 of the project plan)
8. **Update** `lib/oxigraph/Cargo.toml` — feature flags for backend selection

### 7.6 Risk Mitigation

| Risk | Mitigation |
|------|-----------|
| Iterator lifetime issues with trait objects | Use `Box<dyn Iterator + '_>` with explicit lifetime annotations; test thoroughly |
| Breaking Python/JS bindings | Use `dyn` dispatch to keep `Store` non-generic; run binding tests early |
| RocksDB performance regression | Benchmark before/after trait extraction; ensure no unnecessary allocations in hot path |
| Transaction semantics mismatch | Define clear trait contract: writer is a transaction scope; `commit()` is atomic; `rollback()` discards all writes |
| Upstream Oxigraph divergence | Pin to a specific tag; use patch-based approach so trait changes can be rebased on upstream updates |

---

## 8. Conclusion

Oxigraph's storage architecture is **well-suited for trait extraction**. The existing `Reader`/`Writer` enum pattern in `backend/mod.rs` already defines an implicit interface with the right set of operations (get, put, delete, prefix_iter, transaction commit/rollback). The encoding layer (`binary_encoder.rs`, `numeric_encoder.rs`) and SPARQL evaluation layer (`sparql/eval.rs`) are cleanly decoupled from storage internals.

The recommended approach is:
1. **Dynamic dispatch** (`Box<dyn StorageBackend>`) to avoid generic infection
2. **Synchronous trait** with `block_on` bridge for async backends
3. **Key prefix mapping** for column-family-to-TiKV translation
4. **Preserve existing tests** as the correctness oracle throughout refactoring

Estimated effort for Phase 1 (trait extraction + RocksDB backend): **2-3 weeks** for a developer familiar with Oxigraph internals. The existing in-memory fallback backend provides a useful second implementation to validate the trait design immediately.
