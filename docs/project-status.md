# Oxigraph Cloud-Native: Project Status Dashboard

> **Last Updated**: 2026-03-17
> **Plan Version**: Draft v1
> **Critical Path**: Phase 0 -> Phase 1 -> Phase 2 -> Phase 4 -> Phase 8

---

## Phase Status Overview

| Phase | Name | Status | Progress |
|-------|------|--------|----------|
| 0 | Project Bootstrap | Done | 100% |
| 1 | StorageBackend Trait Abstraction | Done | 100% |
| 2 | TiKV Backend Implementation | In Progress | ~70% |
| 3 | SHACL Validation via Rudof | In Progress | ~65% |
| 4 | Query Optimization & Coprocessor Pushdown | Not Started | 0% |
| 5 | Containerization & Kubernetes Manifests | Not Started | 0% |
| 6 | OpenShift Production Deployment | Not Started | 0% |
| 7 | Developer Sandbox Variant | Not Started | 0% |
| 8 | Testing, Hardening & Release | Not Started | 0% |

---

## Phase 0: Project Bootstrap -- Done

| Task | Status | Deliverable |
|------|--------|-------------|
| 0.1 Fork & vendor Oxigraph | Done | `/home/ldary/rh/oxigraph-k8s/oxigraph/` (full fork per ADR-001) |
| 0.2 Workspace Cargo.toml | Done | Workspace with `oxigraph-tikv`, `oxigraph-shacl` crates under `/home/ldary/rh/oxigraph-k8s/crates/` |
| 0.3 CI pipeline skeleton | Not Started | -- |
| 0.4 Dev environment docs | Done | Architecture docs in `/home/ldary/rh/oxigraph-k8s/docs/` |

**Decisions resolved**:
- Fork strategy: Full fork (ADR-001)
- Workspace layout: Monorepo with workspace members

---

## Phase 1: StorageBackend Trait Abstraction -- Done

| Task | Status | Deliverable |
|------|--------|-------------|
| 1.1 Define `StorageBackend` trait | Done | `/home/ldary/rh/oxigraph-k8s/oxigraph/lib/oxigraph/src/storage/backend_trait.rs` |
| 1.2 Async strategy decision | Done | ADR-003: Sync trait + `block_on` bridge |
| 1.3 RocksDB backend impl | Done | `/home/ldary/rh/oxigraph-k8s/oxigraph/lib/oxigraph/src/storage/rocksdb.rs` (implements trait contract) |
| 1.4 In-memory backend impl | Done | `/home/ldary/rh/oxigraph-k8s/oxigraph/lib/oxigraph/src/storage/memory.rs` (MVCC-based) |
| 1.5 Backend conformance test suite | Done | `/home/ldary/rh/oxigraph-k8s/oxigraph/lib/oxigraph/src/storage/backend_tests.rs` (macro-parameterized) |
| 1.6 Wire trait into `Store` | Done | `StorageKind` enum in `/home/ldary/rh/oxigraph-k8s/oxigraph/lib/oxigraph/src/storage/mod.rs` |

**Decisions resolved**:
- Generics vs. dynamic dispatch: Enum dispatch (ADR-002)
- Async strategy: Sync trait + `block_on` bridge (ADR-003)
- Error type: Unified `StorageError` enum

**Key deliverables**:
- `StorageBackend`, `StorageBackendReader`, `StorageBackendTransaction`, `StorageBackendReadableTransaction`, `StorageBackendBulkLoader` traits
- `StorageKind` enum with `RocksDb`, `TiKv`, `Memory` variants (feature-gated)
- Conformance test macro (`conformance_tests!`) generating 18 test functions per backend
- Memory + RocksDB conformance tests passing

---

## Phase 2: TiKV Backend Implementation -- In Progress (~70%)

| Task | Status | Deliverable |
|------|--------|-------------|
| 2.1 TiKV dev cluster setup | Done | Docker Compose config documented in `/home/ldary/rh/oxigraph-k8s/docs/tikv-client-compatibility.md` |
| 2.2 Key encoding mapping | Done | `/home/ldary/rh/oxigraph-k8s/docs/tikv-key-encoding.md` (12 prefix bytes, scan bounds helpers) |
| 2.3 TiKV backend core impl | Done | `/home/ldary/rh/oxigraph-k8s/oxigraph/lib/oxigraph/src/storage/tikv.rs` (~1300+ lines, full `StorageBackend` impl) |
| 2.4 Transaction support | Done | Optimistic 2PC via `tikv-client` `TransactionClient`, integrated in `tikv.rs` |
| 2.5 Connection pooling & config | In Progress | `TiKvConfig` struct in `/home/ldary/rh/oxigraph-k8s/crates/oxigraph-tikv/src/backend.rs` (basic config, pool not yet configurable) |
| 2.6 Integration test suite | Done | `/home/ldary/rh/oxigraph-k8s/crates/oxigraph-tikv/tests/integration.rs` (8 tests, currently using in-memory store as stub until TiKV constructor is finalized) |
| 2.7 Basic performance benchmarks | Not Started | -- |

**Key implementation details**:
- `TiKvStorage` holds `Arc<TransactionClient>` + `Arc<Runtime>` (shared tokio runtime)
- `TiKvStorageReader` wraps `tikv_client::Snapshot` with `block_on` for sync reads
- `TiKvStorageTransaction` wraps `tikv_client::Transaction` with buffered writes
- `TiKvStorageBulkLoader` batches writes in chunks of 100,000 entries
- `Storage::open_tikv(pd_endpoints)` constructor gated behind `#[cfg(feature = "tikv")]`

---

## Phase 3: SHACL Validation via Rudof -- In Progress (~65%)

| Task | Status | Deliverable |
|------|--------|-------------|
| 3.1 SRDF trait implementation | Partial | Validation uses NTriples serialization bridge (not direct SRDF trait impl); see `validator.rs` |
| 3.2 SHACL shape management API | Done (spec) | `/home/ldary/rh/oxigraph-k8s/docs/shacl-api-spec.md` (REST API spec, not yet implemented in server) |
| 3.3 Validation-on-ingest pipeline | In Progress | `ShaclValidator::validate()` works end-to-end; integration with HTTP server pending |
| 3.4 Validation report format | Done | ADR-004: Content-negotiated (JSON + W3C RDF), HTTP 422 |
| 3.5 Opt-in validation mode | Done | `ShaclMode` enum (Off/Warn/Enforce) in `/home/ldary/rh/oxigraph-k8s/crates/oxigraph-shacl/src/validator.rs` |
| 3.6 SHACL integration tests | Done | `/home/ldary/rh/oxigraph-k8s/crates/oxigraph-shacl/tests/shacl_tests.rs` (18 tests) + unit tests in `validator.rs` (6 tests) |
| 3.7 Performance impact benchmark | Not Started | -- |

**Key deliverables**:
- `CompiledShapes` -- wraps rudof's `SchemaIR`, compiles from Turtle/RDF
- `ShaclValidator` -- configurable mode, validates `Store` data against compiled shapes
- `ValidationOutcome` -- `Skipped` / `Passed` / `Failed(ValidationReport)`
- `ShaclError` -- typed error enum (ShapeCompilation, StoreSerialization, DataLoading, ValidationEngine)

---

## Test Count Summary

| Crate / Module | Test Count | Notes |
|----------------|------------|-------|
| `oxigraph` core (storage, store, sparql) | ~100+ | Includes existing upstream tests |
| `backend_tests.rs` conformance | 18 | Macro-expanded for Memory; 4 manual for RocksDB |
| `oxigraph-shacl` unit tests | 6 | In `validator.rs` |
| `oxigraph-shacl` integration tests | 18 | In `shacl_tests.rs` |
| `oxigraph-tikv` integration tests | 8 | In `integration.rs` (stub store until backend finalized) |
| Upstream oxigraph tests (testsuite, libs) | ~160+ | W3C SPARQL, parser, datatypes, serialization |
| **Total across workspace** | **~312** | From `#[test]` annotations |

---

## Architecture Decision Records (ADR Summary)

| ADR | Title | Decision | Impact |
|-----|-------|----------|--------|
| [ADR-001](adr/001-fork-strategy.md) | Fork Strategy | Full fork with upstream remote | Enables deep storage refactoring; quarterly merge cadence |
| [ADR-002](adr/002-generics-vs-dyn-dispatch.md) | Generics vs Dynamic Dispatch | Enum dispatch | Zero type parameter infection; extends Oxigraph's existing pattern |
| [ADR-003](adr/003-async-strategy.md) | Async Strategy | Sync trait + `block_on` bridge | Zero changes to SPARQL query engine; prefetch buffer for scan performance |
| [ADR-004](adr/004-shacl-validation-report-format.md) | Validation Report Format | Content-negotiated (JSON + W3C RDF) | HTTP 422 for rejection; `Link` header to shapes graph |

---

## Risks Materialized and Mitigations

| Risk ID | Risk | Materialized? | Mitigation Applied |
|---------|------|---------------|---------------------|
| R1 | Oxigraph storage too coupled to RocksDB | Partially | Storage audit (`audit-oxigraph-storage.md`) found coupling is layered and well-contained. Existing `Reader`/`Writer` enum pattern provided natural seam. Refactoring difficulty was MEDIUM-HIGH as predicted. |
| R4 | Async conversion cascades through codebase | Mitigated | ADR-003 chose sync trait + `block_on`, avoiding async infection entirely. SPARQL query engine unchanged. |
| R5 | rudof SRDF trait requires unsupported methods | Partially | Current implementation bypasses SRDF trait entirely, using NTriples serialization as bridge. Works but adds overhead. Direct SRDF impl remains future work. |
| R6 | Developer Sandbox too constrained for TiKV | Not yet tested | RocksDB fallback via `StorageKind` enum is ready; sandbox deployment not started. |
| R2 | `tikv-client` crate incompatibility | Not yet tested | Compatibility report written; live verification pending. |
| R3 | TiKV Coprocessor too SQL-specific | Not yet tested | Phase 4 not started. |
| R7 | TiKV Operator requires cluster-admin | Not yet tested | Phase 6 not started. |

---

## Remaining Work Items

### High Priority (Critical Path)

| Item | Phase | Blocking | Estimated Effort |
|------|-------|----------|-----------------|
| Finalize TiKV `Store` constructor and remove integration test stubs | 2 | Phase 2 completion | 1-2 days |
| TiKV connection pool configuration (env vars, retry policy) | 2 | Phase 2 completion | 2-3 days |
| Performance benchmarks: RocksDB vs TiKV (point query, scan, bulk load) | 2 | Phase 4 scoping | 1 week |
| CI pipeline skeleton (GitHub Actions / Tekton) | 0 | All subsequent CI | 2-3 days |
| Wire SHACL validator into HTTP server (validation-on-ingest) | 3 | Phase 3 completion | 1 week |
| Implement SHACL REST API endpoints in server | 3 | Phase 3 completion | 1 week |

### Medium Priority (Parallel Tracks)

| Item | Phase | Estimated Effort |
|------|-------|-----------------|
| Direct SRDF trait implementation (replace NTriples bridge) | 3 | 1-2 weeks |
| Multi-stage Containerfile (UBI-minimal base) | 5 | 2-3 days |
| Kubernetes manifests and Helm chart | 5 | 1 week |
| Health/readiness probes (`/health`, `/ready`) | 5 | 1-2 days |
| SHACL performance benchmark (validation overhead) | 3 | 3-5 days |

### Lower Priority (Future Phases)

| Item | Phase | Estimated Effort |
|------|-------|-----------------|
| Query plan analysis for Coprocessor pushdown | 4 | 2-3 weeks |
| Coprocessor DAG builder | 4 | 3-4 weeks |
| OpenShift adaptation (SCCs, Routes) | 6 | 1 week |
| TiKV cluster sizing and Region tuning | 6 | 1-2 weeks |
| Monitoring stack (Prometheus + Grafana) | 6 | 1 week |
| Developer Sandbox Helm values and quick-start guide | 7 | 1 week |
| W3C SPARQL 1.1 compliance testing on TiKV | 8 | 1 week |
| W3C SHACL compliance testing | 8 | 1 week |
| Chaos testing (kill TiKV nodes during transactions) | 8 | 1-2 weeks |
| Security audit and container CVE scan | 8 | 1 week |

---

## Recommended Priority for Remaining Tasks

1. **CI pipeline** (Phase 0.3) -- Unblocks automated testing for all work; should have been done first. Set up GitHub Actions with RocksDB build, memory tests, and linting.

2. **Finalize TiKV integration tests** (Phase 2.6) -- Remove the in-memory stub in `try_open_tikv_store()` and connect to real TiKV via Docker Compose in CI.

3. **TiKV performance benchmarks** (Phase 2.7) -- Critical for validating the architecture. Must confirm TiKV is within 5x of RocksDB for point queries before investing in Phase 4.

4. **Wire SHACL into HTTP server** (Phase 3.2, 3.3) -- The validator crate works standalone. Integrating it into the server unlocks the full validation-on-ingest pipeline.

5. **Containerization** (Phase 5.1) -- Can start immediately since the server binary exists. Multi-stage Containerfile is a prerequisite for all deployment phases.

6. **Direct SRDF trait implementation** (Phase 3.1) -- Current NTriples serialization bridge works but is inefficient for large stores. Replace with direct `quads_for_pattern` access.

7. **Kubernetes manifests + Helm** (Phase 5.3, 5.4) -- Unlocks Phase 6 and 7 in parallel.

8. **Coprocessor pushdown** (Phase 4) -- Research-heavy; defer until TiKV benchmarks confirm the optimization is needed.
