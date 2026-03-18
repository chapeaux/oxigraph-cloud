# Test Plan

## 1. Backend Conformance Tests (Phase 1.5)

**Location**: `crates/oxigraph-tikv/tests/conformance.rs`

Tests parameterized over `StorageBackend`:
- CRUD: put, get, delete, batch_put
- Range scans: prefix scan, range scan, ordering
- Transactions: atomicity, rollback, snapshot isolation
- Concurrent reads during write transactions
- Edge cases: empty keys/values, large values, missing keys

**Backends tested**: Memory (always), RocksDB (always), TiKV (when `TIKV_PD_ENDPOINTS` is set)

## 2. SPARQL Integration Tests (Phase 2.6)

**Location**: `tests/integration/`

- Insert triples via SPARQL UPDATE → query via SELECT → verify
- Bulk load from Turtle/N-Triples files
- Concurrent readers and writers
- Server HTTP API (health, ready, query, update, store)

## 3. SHACL Validation Tests (Phase 3.6)

**Location**: `crates/oxigraph-shacl/tests/`

- Conformant data passes validation
- Non-conformant data fails with correct report
- Shape constraints: cardinality, datatype, pattern, class
- Validation modes: off (skip), warn (log), enforce (reject)

## 4. W3C Compliance (Phase 8.1-8.2)

### SPARQL 1.1
- Run upstream Oxigraph test suite against TiKV backend
- Target: same pass rate as RocksDB backend
- Location: `oxigraph/testsuite/`

### SHACL
- Run W3C SHACL test suite via rudof
- Document any failures (rudof vs integration bugs)

## 5. Chaos Testing (Phase 8.3)

Scenarios:
- Kill 1 of 3 TiKV nodes during write transaction → verify retry/failure
- Kill PD leader → verify re-election and continued operation
- Network partition between Oxigraph and TiKV → verify timeout handling
- Kill Oxigraph pod during bulk load → verify no data corruption

## 6. Performance Benchmarks (Phase 2.7)

**Location**: `tests/benchmark/`

| Benchmark | Metric |
|-----------|--------|
| Single triple insert | Latency (P50/P99) |
| Bulk load (1K/10K/100K) | Throughput (triples/sec) |
| Point query | Latency (P50/P99) |
| Range scan | Throughput (results/sec) |
| SHACL validation overhead | % slowdown vs no validation |

Compare: RocksDB vs TiKV. Target: TiKV within 5x of RocksDB for point queries.

## 7. Security Audit (Phase 8.4)

- `cargo audit` — no critical/high vulnerabilities
- Container image CVE scan — no critical findings
- RBAC review — minimal permissions
- Network policy review — no unnecessary exposure
