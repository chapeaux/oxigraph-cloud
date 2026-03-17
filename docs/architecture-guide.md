# Oxigraph Cloud-Native: Architecture Guide

> A comprehensive guide for new contributors to the cloud-native Oxigraph project with TiKV storage and Rudof-based SHACL validation.

---

## Table of Contents

1. [Project Overview and Goals](#project-overview-and-goals)
2. [Repository Structure](#repository-structure)
3. [Storage Architecture](#storage-architecture)
4. [Key Encoding Scheme](#key-encoding-scheme)
5. [SHACL Validation Pipeline](#shacl-validation-pipeline)
6. [Deployment Topology](#deployment-topology)
7. [How to Add a New Storage Backend](#how-to-add-a-new-storage-backend)
8. [Development Workflow](#development-workflow)
9. [Design Documents and ADRs](#design-documents-and-adrs)

---

## Project Overview and Goals

This project transforms Oxigraph from a high-performance embedded RDF database into a cloud-native, distributed SPARQL and SHACL platform. The three core objectives are:

1. **Pluggable storage layer** -- Decouple Oxigraph from its hardcoded RocksDB dependency by introducing a `StorageBackend` trait, enabling distributed backends.

2. **SHACL validation at ingestion** -- Integrate the Rust-native `rudof` crate to enforce SHACL shape constraints on incoming RDF data, with configurable modes (off, warn, enforce).

3. **Kubernetes/OpenShift deployment** -- Package as OCI container images with Helm charts for both production OpenShift clusters (TiKV-backed) and resource-constrained Developer Sandbox environments (RocksDB fallback).

**Why Oxigraph**: Written entirely in Rust, highly compliant SPARQL 1.1 implementation, clean internal architecture with storage already partially abstracted behind enum dispatch.

**Why TiKV**: Rust-native, Raft consensus, range-based partitioning (aligns with Oxigraph's lexicographic key encoding), Coprocessor pushdown framework, CNCF Graduated project.

**Why Rudof**: Rust-native SHACL/ShEx validation library, 10.8x faster than TopQuadrant, bridges via the SRDF trait abstraction.

---

## Repository Structure

```
oxigraph-k8s/
|-- CLAUDE.md                          # Project instructions
|-- PLAN.md                            # Phased implementation plan
|-- docs/                              # Architecture research and ADRs
|   |-- 01-overview.md                 # Project goals, high-level architecture
|   |-- 02-oxigraph-storage-architecture.md  # KV tables, byte encoding
|   |-- 03-rudof-shacl-integration.md  # SRDF trait bridge, rudof crates
|   |-- 04-distributed-sparql-theory.md # OLTP/OLAP theory, network bottleneck
|   |-- 05-tikv-backend.md             # TiKV architecture, Coprocessor pushdown
|   |-- 06-backend-alternatives-rejected.md  # FoundationDB, DynamoDB, S3
|   |-- 07-storage-trait-design.md     # StorageBackend trait design
|   |-- 08-references.md              # All external references
|   |-- adr/
|   |   |-- 001-fork-strategy.md       # Full fork (accepted)
|   |   |-- 002-generics-vs-dyn-dispatch.md  # Enum dispatch (accepted)
|   |   |-- 003-async-strategy.md      # Sync + block_on (accepted)
|   |   |-- 004-shacl-validation-report-format.md  # Content-negotiated (accepted)
|   |-- audit-oxigraph-storage.md      # Storage layer coupling assessment
|   |-- test-conformance-suite.md      # Backend conformance test spec
|   |-- tikv-key-encoding.md           # Column family to key prefix mapping
|   |-- tikv-client-compatibility.md   # tikv-client crate compatibility report
|   |-- shacl-api-spec.md             # SHACL REST API specification
|   |-- project-status.md             # Status dashboard
|   |-- architecture-guide.md         # This document
|
|-- crates/
|   |-- oxigraph-tikv/                 # TiKV backend crate (external wrapper)
|   |   |-- src/
|   |   |   |-- lib.rs
|   |   |   |-- backend.rs            # TiKvConfig struct
|   |   |-- tests/
|   |       |-- integration.rs         # SPARQL round-trip integration tests
|   |-- oxigraph-shacl/               # SHACL validation crate
|       |-- src/
|       |   |-- lib.rs                 # Crate root, architecture docs
|       |   |-- validator.rs           # ShaclValidator, ShaclMode, ValidationOutcome
|       |   |-- shapes.rs             # CompiledShapes (wraps rudof SchemaIR)
|       |   |-- error.rs              # ShaclError enum
|       |-- tests/
|           |-- shacl_tests.rs         # 18 integration tests
|
|-- oxigraph/                          # Forked Oxigraph (full source)
    |-- lib/
    |   |-- oxigraph/                  # Core library
    |   |   |-- src/
    |   |       |-- storage/
    |   |       |   |-- mod.rs         # Storage + StorageKind enum dispatch
    |   |       |   |-- backend_trait.rs  # StorageBackend trait definitions
    |   |       |   |-- backend_tests.rs  # Conformance test macro
    |   |       |   |-- memory.rs      # In-memory MVCC backend
    |   |       |   |-- rocksdb.rs     # RocksDB backend
    |   |       |   |-- tikv.rs        # TiKV backend (feature-gated)
    |   |       |   |-- binary_encoder.rs  # Byte encoding for RDF terms
    |   |       |   |-- numeric_encoder.rs # Numeric datatype encoding
    |   |       |   |-- small_string.rs    # Small string optimization
    |   |       |   |-- error.rs       # StorageError, CorruptionError
    |   |       |-- store.rs           # Public Store API
    |   |       |-- sparql/            # SPARQL evaluation
    |   |-- oxrdf/                     # RDF data model (storage-agnostic)
    |   |-- spargebra/                 # SPARQL algebra parser
    |   |-- sparopt/                   # SPARQL query optimizer
    |   |-- spareval/                  # SPARQL evaluator
    |   |-- sparesults/                # Query result serialization
    |   |-- oxttl/                     # Turtle/TriG parser
    |   |-- oxrdfxml/                  # RDF/XML parser
    |   |-- oxrdfio/                   # RDF I/O facade
    |   |-- oxsdatatypes/              # XSD datatype implementations
    |   |-- oxjsonld/                  # JSON-LD processor
    |   |-- spargeo/                   # GeoSPARQL support
    |   |-- sparql-smith/              # SPARQL fuzzer
    |-- cli/                           # CLI binary
    |-- testsuite/                     # W3C compliance test runner
    |-- python/                        # Python bindings (PyO3)
    |-- js/                            # JavaScript/WASM bindings
    |-- fuzz/                          # Fuzz targets
```

### Crate Responsibilities

| Crate | Purpose | Storage Dependency |
|-------|---------|-------------------|
| `oxigraph` (lib) | Core: storage, SPARQL parsing/evaluation, RDF model | **Contains all storage code** |
| `oxrdf` | RDF data model types (NamedNode, Literal, Quad) | None |
| `spargebra` | SPARQL 1.1 algebra parser | None |
| `sparopt` | SPARQL query optimizer | None |
| `spareval` | SPARQL evaluator | Uses `StorageReader` iterators (no direct backend dependency) |
| `oxigraph-shacl` | SHACL validation via rudof | Depends on `oxigraph::Store` public API |
| `oxigraph-tikv` | TiKV configuration and integration tests | Depends on `oxigraph` with `tikv` feature |

---

## Storage Architecture

### The StorageBackend Trait

Defined in `oxigraph/lib/oxigraph/src/storage/backend_trait.rs`, the trait hierarchy consists of four components:

```
StorageBackend (factory)
  |-- snapshot() -> Reader       (point-in-time read-only access)
  |-- start_transaction() -> Transaction  (write-only, atomic commit)
  |-- start_readable_transaction() -> ReadableTransaction  (read + write)
  |-- bulk_loader() -> BulkLoader  (high-throughput ingestion)
```

**StorageBackendReader** provides:
- `quads_for_pattern(s, p, o, g)` -- Core quad pattern matching (selects optimal index)
- `contains(quad)` -- Existence check
- `named_graphs()` -- Graph directory iteration
- `contains_str(hash)` / `get_str(hash)` -- Dictionary lookups (via `StrLookup` trait)
- `len()` / `is_empty()` -- Cardinality

**StorageBackendTransaction** provides:
- `insert(quad)` / `remove(quad)` -- Quad manipulation
- `insert_named_graph(name)` -- Graph directory management
- `clear_default_graph()` / `clear_all_graphs()` / `clear()` -- Bulk operations
- `commit()` -- Atomic commit (drop without commit = rollback)

**StorageBackendBulkLoader** provides:
- `load_batch(quads, threads)` -- Batch ingestion
- `on_progress(callback)` -- Progress monitoring
- `without_atomicity()` -- Trade ACID for throughput

### Enum Dispatch (ADR-002)

Per ADR-002, the `Store` struct remains non-generic. Backend selection is handled via a `StorageKind` enum:

```rust
// In storage/mod.rs
enum StorageKind {
    #[cfg(feature = "rocksdb")]
    RocksDb(RocksDbStorage),
    #[cfg(feature = "tikv")]
    TiKv(TiKvStorage),
    Memory(MemoryStorage),
}
```

Each enum variant is feature-gated. At runtime, exactly one variant is active. The compiler optimizes single-variant match arms to near-zero overhead.

Corresponding reader, transaction, and bulk loader enums follow the same pattern (`StorageReaderKind`, `StorageTransactionKind`, etc.).

### Sync Trait with `block_on` Bridge (ADR-003)

All trait methods are synchronous. The TiKV backend wraps async `tikv-client` calls via `tokio::Runtime::block_on()`:

```rust
impl StorageBackendReader for TiKvStorageReader {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.runtime.block_on(self.snapshot.get(key.to_vec()))
    }
}
```

SPARQL query evaluation threads must run outside the tokio runtime (via `spawn_blocking` or a separate thread pool) to avoid nested-runtime panics.

Scan performance is maintained via prefetch buffers: `scan_prefix` fetches batches of entries in a single async RPC, serving subsequent `next()` calls from the buffer.

### Backend Implementations

| Backend | File | Use Case | Feature Flag |
|---------|------|----------|-------------|
| **Memory** | `storage/memory.rs` | Testing, WASM, ephemeral | Always compiled |
| **RocksDB** | `storage/rocksdb.rs` | Embedded, single-node | `rocksdb` |
| **TiKV** | `storage/tikv.rs` | Distributed, cloud-native | `tikv` |

---

## Key Encoding Scheme

### RDF Term Encoding

Oxigraph encodes RDF terms as fixed-length byte sequences:
- **1 type byte** defining the term kind (NamedNode, BlankNode, Literal variants)
- **Up to 32 value bytes** (128-bit hash for NamedNodes, inline for small strings/numerics)

### 11 KV Tables (Column Families)

Oxigraph maintains 11 key-value tables for indexing:

| Table | Purpose | Key Structure |
|-------|---------|--------------|
| `id2str` | Dictionary: StrHash -> String | 16-byte hash |
| `dspo` | Default graph: Subject-Predicate-Object | `[S][P][O]` |
| `dpos` | Default graph: Predicate-Object-Subject | `[P][O][S]` |
| `dosp` | Default graph: Object-Subject-Predicate | `[O][S][P]` |
| `spog` | Named graph: Subject-Predicate-Object-Graph | `[S][P][O][G]` |
| `posg` | Named graph: Predicate-Object-Subject-Graph | `[P][O][S][G]` |
| `ospg` | Named graph: Object-Subject-Predicate-Graph | `[O][S][P][G]` |
| `gspo` | Named graph: Graph-Subject-Predicate-Object | `[G][S][P][O]` |
| `gpos` | Named graph: Graph-Predicate-Object-Subject | `[G][P][O][S]` |
| `gosp` | Named graph: Graph-Object-Subject-Predicate | `[G][O][S][P]` |
| `graphs` | Named graph directory | `[G]` |

### TiKV Key Prefix Mapping

RocksDB uses column families to separate these tables. TiKV has a flat key space, so each table gets a unique 1-byte prefix:

| Prefix | Table | Description |
|--------|-------|-------------|
| `0x00` | default | Metadata (storage version) |
| `0x01` | id2str | Dictionary |
| `0x02` | spog | Named graph S-P-O-G index |
| `0x03` | posg | Named graph P-O-S-G index |
| `0x04` | ospg | Named graph O-S-P-G index |
| `0x05` | gspo | Named graph G-S-P-O index |
| `0x06` | gpos | Named graph G-P-O-S index |
| `0x07` | gosp | Named graph G-O-S-P index |
| `0x08` | dspo | Default graph S-P-O index |
| `0x09` | dpos | Default graph P-O-S index |
| `0x0A` | dosp | Default graph O-S-P index |
| `0x0B` | graphs | Named graph directory |

Every TiKV key has the format: `[1 byte: table prefix][N bytes: original key from Oxigraph's encoder]`.

### Prefix Scan Translation

RocksDB column family prefix scan:
```
cf("spog").prefix_iterator(subject_prefix)
```

Equivalent TiKV bounded range scan:
```
start_key = [0x02] ++ subject_prefix
end_key   = [0x02] ++ successor(subject_prefix)
tikv_client.scan(start_key..end_key)
```

Full table scan:
```
start_key = [0x02]
end_key   = [0x03]   // next table prefix
```

The prefix scheme preserves lexicographic ordering within each table, which is essential for SPARQL Basic Graph Pattern evaluation.

### Region Alignment

Each table prefix naturally falls into its own TiKV Region range. For a 100M triple dataset (~900M index keys, ~50-60 GB), this yields ~600-700 Regions -- well within TiKV's comfortable operating range.

---

## SHACL Validation Pipeline

### Architecture

The validation pipeline flows through four stages:

```
1. Shapes (Turtle/RDF)
       |
       v
2. CompiledShapes (rudof SchemaIR)
       |
       v
3. ShaclValidator.validate(store)
       |  - Serializes store data to NTriples
       |  - Loads into rudof's RdfData
       |  - Runs shacl_validation::validate()
       v
4. ValidationOutcome
       |-- Skipped (mode=Off)
       |-- Passed  (all shapes conform)
       |-- Failed(ValidationReport)
```

### Key Types

**`CompiledShapes`** (`crates/oxigraph-shacl/src/shapes.rs`):
- Wraps rudof's `SchemaIR`
- Constructed via `CompiledShapes::from_turtle(turtle_string)`
- Provides `target_shape_count()` and `shape_count()` for introspection

**`ShaclValidator`** (`crates/oxigraph-shacl/src/validator.rs`):
- Holds a `ShaclMode` and optional `CompiledShapes`
- `validate(&self, store: &Store) -> Result<ValidationOutcome, ShaclError>`
- Internally serializes store to NTriples, loads into rudof's `RdfData`, runs validation

**`ShaclMode`**:
- `Off` (default) -- No validation
- `Warn` -- Log failures, accept data
- `Enforce` -- Reject non-conforming data with HTTP 422

**`ValidationOutcome`**:
- `Skipped` -- Mode was Off
- `Passed` -- All shapes conform
- `Failed(ValidationReport)` -- Contains rudof's `ValidationReport` with focus nodes, result paths, constraint components

### REST API (Planned)

Endpoints under `/shacl`:
- `POST /shacl/shapes` -- Upload shapes graph
- `GET /shacl/shapes` -- List active shapes
- `GET /shacl/shapes/{id}` -- Retrieve specific shapes (content-negotiated)
- `DELETE /shacl/shapes/{id}` -- Remove shapes
- `POST /shacl/validate` -- Trigger on-demand validation
- `GET /shacl/mode` / `PUT /shacl/mode` -- Query/set validation mode

### Validation Report Format (ADR-004)

On validation failure:
- HTTP status: `422 Unprocessable Entity`
- `Accept: application/json` returns simplified JSON with `conforms`, `results[]` (focusNode, resultPath, value, sourceShape, sourceConstraintComponent, resultSeverity, resultMessage)
- `Accept: text/turtle` (or other RDF types) returns W3C SHACL Validation Report as RDF
- `Link: </shacl>; rel="describedby"` header points to shapes graph

---

## Deployment Topology

### Production: OpenShift with TiKV (Phase 6)

```
                   +------------------+
                   |   OpenShift      |
                   |   Route / LB     |
                   +--------+---------+
                            |
              +-------------+-------------+
              |                           |
    +---------+---------+    +-----------+---------+
    | Oxigraph Pod      |    | Oxigraph Pod        |
    | (--backend tikv)  |    | (--backend tikv)    |
    | --pd-endpoints    |    | --pd-endpoints      |
    | --shacl-mode=...  |    | --shacl-mode=...    |
    +--------+----------+    +----------+----------+
             |                          |
             +----------+---+----------+
                        |   |
              +---------+---+---------+
              |    TiKV Cluster       |
              |  (via TiDB Operator)  |
              |  +---+  +---+  +---+ |
              |  |PD |  |PD |  |PD | |
              |  +---+  +---+  +---+ |
              |  +----+ +----+ +----+|
              |  |TiKV| |TiKV| |TiKV||
              |  +----+ +----+ +----+|
              +-----------------------+
```

- Oxigraph pods are stateless (storage in TiKV)
- TiKV cluster managed by TiDB Operator (StatefulSet with PVCs)
- PD (Placement Driver) handles Region management and timestamp allocation
- Horizontal scaling: add Oxigraph pods for compute, TiKV nodes for storage

### Developer Sandbox: RocksDB Fallback (Phase 7)

```
    +------------------+
    | Developer Sandbox|
    | (limited CPU/RAM)|
    +--------+---------+
             |
    +--------+---------+
    | Oxigraph Pod     |
    | (--backend rocks) |
    | + PVC (ephemeral)|
    +------------------+
```

- Single Oxigraph pod with embedded RocksDB
- No TiKV cluster required (too resource-intensive for sandbox)
- Demonstrates the `StorageBackend` trait abstraction
- Same SHACL validation capabilities
- Helm values overlay: `values-sandbox.yaml`

---

## How to Add a New Storage Backend

Follow these steps to add a new backend (e.g., FoundationDB, SQLite):

### Step 1: Implement the Backend

Create a new file at `oxigraph/lib/oxigraph/src/storage/your_backend.rs`.

Implement the full storage contract by providing types that satisfy the traits in `backend_trait.rs`:

```rust
// 1. Storage factory (Clone + Send + Sync)
pub struct YourStorage { /* connection, config */ }

// 2. Read-only snapshot
pub struct YourStorageReader { /* snapshot handle */ }

// 3. Write-only transaction
pub struct YourStorageTransaction { /* write buffer */ }

// 4. Read+write transaction
pub struct YourStorageReadableTransaction { /* both */ }

// 5. Bulk loader
pub struct YourStorageBulkLoader { /* batch buffer */ }
```

Each type must implement the corresponding trait from `backend_trait.rs`. Key requirements:

- `StorageBackendReader`: Must implement `StrLookup` for dictionary lookups. `quads_for_pattern()` must select the optimal index (SPO, POS, OSP, etc.) based on which components are bound.
- `StorageBackendTransaction`: `commit()` must be atomic. Dropping without `commit()` must roll back.
- Iterator types for quads and graphs must return `Result<EncodedQuad/EncodedTerm, StorageError>`.

### Step 2: Add the Enum Variant

In `storage/mod.rs`, add your backend to the dispatch enums:

```rust
enum StorageKind {
    #[cfg(feature = "rocksdb")]
    RocksDb(RocksDbStorage),
    #[cfg(feature = "tikv")]
    TiKv(TiKvStorage),
    #[cfg(feature = "your_backend")]
    YourBackend(YourStorage),       // <-- add this
    Memory(MemoryStorage),
}
```

Add corresponding variants to `StorageReaderKind`, `StorageTransactionKind`, `StorageReadableTransactionKind`, `StorageBulkLoaderKind`, and their iterator types.

Add match arms in all `impl` blocks that dispatch to the enum variants.

### Step 3: Add a Constructor

```rust
impl Storage {
    #[cfg(feature = "your_backend")]
    pub fn open_your_backend(config: &YourConfig) -> Result<Self, StorageError> {
        Ok(Self {
            kind: StorageKind::YourBackend(YourStorage::connect(config)?),
        })
    }
}
```

### Step 4: Feature-Gate in Cargo.toml

In `oxigraph/lib/oxigraph/Cargo.toml`:

```toml
[features]
your_backend = ["your-client-crate"]

[dependencies]
your-client-crate = { version = "...", optional = true }
```

### Step 5: Run the Conformance Test Suite

The conformance tests in `backend_tests.rs` use a macro to generate identical tests for any backend. Add your backend:

```rust
#[cfg(feature = "your_backend")]
conformance_tests!(your_backend_conformance, Storage::open_your_backend(&test_config()).unwrap());
```

All 18 conformance tests must pass:
- T1: Basic CRUD (insert, delete, query, overwrite)
- T3: Pattern queries (by subject, full scan)
- T5: Transaction atomicity (commit visibility, rollback on drop)
- T6: Named graph lifecycle (create, list, clear)
- T7: Readable transactions (read-own-writes)
- T8: Bulk loader
- T9: Storage validation

### Step 6: Wire into the Server CLI

Add a CLI flag in the server binary for backend selection:

```
oxigraph-server --backend your_backend --your-config-flag value
```

---

## Development Workflow

### Prerequisites

- Rust stable toolchain (edition 2021)
- For RocksDB: C/C++ compiler, CMake (vendored via `oxrocksdb-sys`)
- For TiKV: TiUP playground or Docker Compose for local cluster
- For SHACL: No additional dependencies (rudof is a Rust crate)

### Build

```bash
# Build all crates (memory backend only, fastest)
cargo build --workspace

# Build with RocksDB support
cargo build --workspace --features rocksdb

# Build with TiKV support
cargo build --workspace --features tikv

# Build with all features
cargo build --workspace --all-features
```

### Test

```bash
# Run all tests (memory + RocksDB backends)
cargo test --workspace --features rocksdb

# Run backend conformance tests only
cargo test -p oxigraph --features rocksdb -- backend_tests

# Run SHACL tests
cargo test -p oxigraph-shacl

# Run TiKV integration tests (requires running TiKV cluster)
TIKV_PD_ENDPOINTS=127.0.0.1:2379 cargo test -p oxigraph-tikv --features integration-tests -- --test-threads=1

# Run W3C compliance tests
cargo test -p oxigraph-testsuite --features rocksdb
```

### Local TiKV Cluster

**Option A: TiUP (recommended for development)**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://tiup-mirrors.pingcap.com/install.sh | sh
tiup playground --mode tikv-slim
# PD endpoint: 127.0.0.1:2379
```

**Option B: Docker Compose (recommended for CI)**
```bash
docker-compose -f docker-compose-tikv.yml up -d
# See docs/tikv-client-compatibility.md for compose file
```

### Code Organization Conventions

- Storage backends are internal to `lib/oxigraph/src/storage/` (not separate crates)
- The `oxigraph-tikv` crate under `crates/` is for external configuration types and integration tests
- The `oxigraph-shacl` crate under `crates/` is independent of storage backends (works through the public `Store` API)
- Feature flags gate backend compilation: `rocksdb`, `tikv`
- All encoding logic is in `binary_encoder.rs` and `numeric_encoder.rs` (storage-agnostic)
- SPARQL evaluation (`spareval`) never touches storage backends directly -- only `StorageReader`

---

## Design Documents and ADRs

### Architecture Decision Records

| ADR | Title | File |
|-----|-------|------|
| ADR-001 | Fork Strategy | `docs/adr/001-fork-strategy.md` |
| ADR-002 | Generics vs Dynamic Dispatch | `docs/adr/002-generics-vs-dyn-dispatch.md` |
| ADR-003 | Async Strategy | `docs/adr/003-async-strategy.md` |
| ADR-004 | SHACL Validation Report Format | `docs/adr/004-shacl-validation-report-format.md` |

### Research Documents

| Document | File |
|----------|------|
| Project Overview | `docs/01-overview.md` |
| Oxigraph Storage Architecture | `docs/02-oxigraph-storage-architecture.md` |
| Rudof SHACL Integration | `docs/03-rudof-shacl-integration.md` |
| Distributed SPARQL Theory | `docs/04-distributed-sparql-theory.md` |
| TiKV Backend Analysis | `docs/05-tikv-backend.md` |
| Rejected Backend Alternatives | `docs/06-backend-alternatives-rejected.md` |
| Storage Trait Design | `docs/07-storage-trait-design.md` |
| References | `docs/08-references.md` |

### Implementation Documents

| Document | File |
|----------|------|
| Storage Layer Audit | `docs/audit-oxigraph-storage.md` |
| Backend Conformance Test Spec | `docs/test-conformance-suite.md` |
| TiKV Key Encoding Design | `docs/tikv-key-encoding.md` |
| TiKV Client Compatibility | `docs/tikv-client-compatibility.md` |
| SHACL REST API Specification | `docs/shacl-api-spec.md` |
| Project Status Dashboard | `docs/project-status.md` |
