# Oxigraph Cloud-Native: Phased Implementation Plan

> **Status**: Draft v1 | **Date**: 2026-03-17
> **Scope**: TiKV storage backend, Rudof SHACL validation, OpenShift deployment

---

## Phase 0: Project Bootstrap

**Objective**: Establish the development workspace, CI pipeline, and project scaffolding.

**Blockers**: None (starting phase)

| # | Task | Owner | Description | Acceptance Criteria |
|---|------|-------|-------------|---------------------|
| 0.1 | Fork & vendor Oxigraph | `/rust-dev` | Fork oxigraph/oxigraph; add as git submodule or vendored dependency. Pin to a stable release tag. | Oxigraph builds locally with `cargo build`; all existing tests pass |
| 0.2 | Workspace Cargo.toml | `/rust-dev` | Create a Cargo workspace with member crates: `oxigraph-tikv`, `oxigraph-shacl`, `oxigraph-server` (thin wrapper) | `cargo check --workspace` succeeds |
| 0.3 | CI pipeline skeleton | `/k8s-deploy` | GitHub Actions (or Tekton) pipeline: lint, build, test, container build | PRs trigger CI; badge on README |
| 0.4 | Dev environment docs | `/k8s-deploy` | Document local dev setup: Rust toolchain, TiKV via `tiup playground`, rudof crate deps | New contributor can build and run tests from README instructions |

**Decisions needed**:
- [ ] `/architect`: Fork strategy — git submodule, patch crate, or full fork? Impacts how we track upstream Oxigraph changes.
- [ ] `/architect`: Workspace layout — monorepo with workspace members, or separate repos?

---

## Phase 1: StorageBackend Trait Abstraction

**Objective**: Define the pluggable storage trait and refactor Oxigraph's existing RocksDB code behind it.

**Blockers**: Phase 0 complete (workspace builds)

| # | Task | Owner | Description | Acceptance Criteria |
|---|------|-------|-------------|---------------------|
| 1.1 | Define `StorageBackend` trait | `/architect` + `/rust-dev` | Design trait with: `get`, `put`, `delete`, `batch_put`, `batch_scan(prefix)`, `snapshot`, `transaction` (atomic commit). Both sync and async variants. Handle GATs for async iterators. | Trait compiles; documented with rustdoc; reviewed by architect |
| 1.2 | Async strategy decision | `/architect` | Decide: (a) fully async trait with `async-trait` / GATs, (b) sync trait + `block_on` bridge for TiKV, (c) dual sync/async traits. Document tradeoffs. | ADR (Architecture Decision Record) written and committed |
| 1.3 | RocksDB backend impl | `/rust-dev` | Refactor Oxigraph's existing `storage/` module to implement `StorageBackend`. This is the compatibility layer — existing behavior must be preserved exactly. | All existing Oxigraph unit tests pass against the refactored RocksDB backend |
| 1.4 | In-memory backend impl | `/rust-dev` | Implement `StorageBackend` for `BTreeMap`-based in-memory storage. Used for fast testing. | In-memory backend passes the same test suite as RocksDB |
| 1.5 | Backend conformance test suite | `/test-qa` | Write backend-agnostic tests: CRUD, range scans, prefix scans, transaction atomicity, snapshot isolation, concurrent reads. Tests parameterized over `impl StorageBackend`. | Test suite runs against both RocksDB and in-memory backends |
| 1.6 | Wire trait into `Store` | `/rust-dev` | Make `oxigraph::Store` generic over `StorageBackend` (or use dynamic dispatch via `Box<dyn StorageBackend>`). SPARQL query and update paths must work unchanged. | `Store<RocksDbBackend>` passes full Oxigraph test suite including SPARQL compliance |

**Decisions needed**:
- [ ] `/architect`: Generics vs. dynamic dispatch for `Store<B: StorageBackend>` — generics give zero-cost abstraction but infect the entire API with type parameters; `dyn` is simpler but adds vtable overhead per KV op.
- [ ] `/architect`: Transaction model — should the trait expose optimistic transactions (TiKV-native) or pessimistic? Or both?
- [ ] `/architect`: Error type design — unified error enum or backend-specific errors behind `Box<dyn Error>`?

**Risks**:
- Oxigraph's internal storage code is tightly coupled to RocksDB column families. Refactoring may require significant surgery on `oxigraph/src/storage/`.
- Async conversion may cascade into Oxigraph's iterator-heavy query engine, which currently assumes synchronous `next()` calls.

---

## Phase 2: TiKV Backend Implementation

**Objective**: Implement `StorageBackend` for TiKV, enabling distributed storage with basic CRUD and range scans.

**Blockers**: Phase 1 tasks 1.1, 1.5 complete (trait defined, conformance tests exist)

| # | Task | Owner | Description | Acceptance Criteria |
|---|------|-------|-------------|---------------------|
| 2.1 | TiKV dev cluster setup | `/tikv-ops` | Document and script local TiKV cluster via `tiup playground` (PD + 3 TiKV nodes). Provide docker-compose alternative. | `tikv-ctl` confirms cluster healthy; Rust client connects and performs basic put/get |
| 2.2 | Key encoding mapping | `/architect` + `/rust-dev` | Map Oxigraph's 11 KV tables (id2str, SPO, POS, OSP, SPOG, POSG, OSPG, GSPO, graphs) to TiKV key space. Design key prefix scheme to keep each "table" in its own key range. | Document committed; key layout reviewed; prefix scans return correct results |
| 2.3 | TiKV backend core impl | `/rust-dev` | Implement `StorageBackend` for TiKV using `tikv-client` crate. Cover: `get`, `put`, `delete`, `batch_put`, `batch_scan`. Use TiKV's transactional API (not raw API) for MVCC. | Passes Phase 1 conformance test suite (task 1.5) against live TiKV cluster |
| 2.4 | Transaction support | `/rust-dev` | Implement snapshot reads and atomic batch commits using TiKV's 2PC (Percolator model). Map Oxigraph's transaction lifecycle to TiKV transactions. | Conformance tests for transaction atomicity and snapshot isolation pass |
| 2.5 | Connection pooling & config | `/rust-dev` | Configurable PD endpoint(s), connection pool size, timeouts, retry policy. Environment variable and config file support. | Server starts with `--backend tikv --pd-endpoints host:port` |
| 2.6 | Integration test suite | `/test-qa` | End-to-end tests: insert triples via SPARQL UPDATE, query via SPARQL SELECT, verify correctness. Run against TiKV cluster (CI needs TiKV service container). | SPARQL INSERT + SELECT round-trip tests pass; CI runs them |
| 2.7 | Basic performance benchmarks | `/test-qa` | Benchmark: single-triple insert, bulk load (1K, 10K, 100K triples), point query, range scan. Compare RocksDB vs TiKV latency/throughput. | Benchmark results documented; no catastrophic regression (TiKV within 5x of RocksDB for point queries) |

**Decisions needed**:
- [ ] `/architect`: Raw API vs. Transactional API — raw API is faster but loses MVCC. Recommend transactional, but need to confirm for read-heavy workloads.
- [ ] `/tikv-ops`: Region size tuning — default 96MB may be too large or small for Oxigraph's key patterns. Need empirical testing.

**Risks**:
- `tikv-client` crate maturity — check latest version, API stability, async runtime compatibility (tokio version).
- TiKV in CI — need containerized TiKV cluster in GitHub Actions. May require custom service container or `tiup` in CI.

---

## Phase 3: SHACL Validation via Rudof

**Objective**: Integrate SHACL validation into the ingestion pipeline using the `rudof` crate's `shacl_validation` module.

**Blockers**: Phase 1 task 1.6 complete (Store is generic over backend). Can run in parallel with Phase 2 using the in-memory or RocksDB backend.

| # | Task | Owner | Description | Acceptance Criteria |
|---|------|-------|-------------|---------------------|
| 3.1 | SRDF trait implementation | `/rust-dev` | Implement rudof's `SRDF` trait for `oxigraph::Store<B>`. Map neighborhood retrieval to `quads_for_pattern(Some(subject), None, None, None)`. This bridges rudof's validator to Oxigraph's storage layer. | `SRDFBasic` and `SRDF` traits compile against Oxigraph Store; unit tests pass |
| 3.2 | SHACL shape management API | `/rust-dev` | REST API endpoints: `POST /shacl` (upload shapes graph), `GET /shacl` (list active shapes), `DELETE /shacl/{id}` (remove). Shapes stored in a dedicated named graph. | curl commands upload, list, and delete SHACL shapes |
| 3.3 | Validation-on-ingest pipeline | `/rust-dev` | Hook into SPARQL UPDATE / bulk load path. Before committing a transaction, run `shacl_validation::validate()` against the candidate data. Reject transaction if validation fails. | Insert of non-conforming data returns HTTP 422 with SHACL validation report; conforming data succeeds |
| 3.4 | Validation report format | `/architect` | Define response format for validation failures. Options: SHACL validation report as RDF (Turtle/JSON-LD), or simplified JSON error. | ADR committed; format documented in API docs |
| 3.5 | Opt-in validation mode | `/rust-dev` | Validation should be configurable: off (default for backward compat), warn (log but accept), enforce (reject on failure). Per-graph or global. | Server flag `--shacl-mode=off|warn|enforce` works correctly |
| 3.6 | SHACL integration tests | `/test-qa` | Test suite: valid data accepted, invalid data rejected, complex shape constraints (cardinality, datatype, pattern), validation report correctness. Use W3C SHACL test suite where applicable. | All tests pass; W3C SHACL conformance subset passes |
| 3.7 | Performance impact benchmark | `/test-qa` | Measure overhead of SHACL validation on ingestion throughput. Compare: no validation vs. simple shapes vs. complex shapes. | Benchmark results documented; validation overhead < 20% for simple shapes on bulk load |

**Decisions needed**:
- [ ] `/architect`: Validate entire transaction or only changed triples? Full validation is safer but slower. Incremental validation (only new/modified triples) requires tracking deltas.
- [ ] `/architect`: Should SHACL validation work with TiKV backend specifically, or should we validate against a local snapshot for performance?

**Risks**:
- `rudof` crate API stability — pin version, monitor for breaking changes.
- SRDF trait may require methods beyond what Oxigraph's `quads_for_pattern` directly supports. Need to audit the full trait surface.

---

## Phase 4: Query Optimization & Coprocessor Pushdown

**Objective**: Optimize distributed query performance via TiKV Coprocessor pushdown, semi-join filters, and batch prefetching.

**Blockers**: Phase 2 complete (TiKV backend works end-to-end)

| # | Task | Owner | Description | Acceptance Criteria |
|---|------|-------|-------------|---------------------|
| 4.1 | Query plan analysis | `/architect` | Analyze Oxigraph's `sparopt` crate to identify which plan operators can be pushed to TiKV Coprocessor. Map BGP evaluation, FILTER, COUNT, and simple aggregations. | Document mapping sparopt operators to Coprocessor DAGs |
| 4.2 | Coprocessor DAG builder | `/rust-dev` | Translate eligible `sparopt` plan nodes into TiKV Coprocessor protobuf requests. Start with: IndexScan + filter pushdown. | Pushed-down scans with filters return correct results; verified by tests |
| 4.3 | Batch prefetching | `/rust-dev` | For non-pushdown queries, implement batch prefetching: predict next key ranges from query plan, issue parallel `batch_scan` requests to TiKV. | Measurable latency improvement on multi-BGP queries vs. sequential scans |
| 4.4 | Semi-join filter prototype | `/rust-dev` | Implement bloom-filter-based semi-join for 2-BGP join patterns. Compute bloom filter for first BGP results, send to TiKV for second BGP filtering. | Correct results on join queries; reduced data transfer measured |
| 4.5 | Coprocessor cache integration | `/rust-dev` + `/tikv-ops` | Enable and configure TiKV's Coprocessor cache. Ensure cache invalidation on Region mutations. | Repeated identical queries served from cache; mutation invalidates correctly |
| 4.6 | Optimization benchmarks | `/test-qa` | Benchmark suite: simple BGP, multi-join BGP, aggregation, FILTER queries. Compare: naive TiKV vs. Coprocessor pushdown vs. RocksDB baseline. | Pushdown queries show >= 2x improvement over naive distributed execution |

**Decisions needed**:
- [ ] `/architect`: Which Coprocessor API version to target? TiKV's Coprocessor API is internal and less stable than the client API.
- [ ] `/architect`: Custom Coprocessor plugin vs. built-in operators? Custom plugin gives more control but requires building/deploying alongside TiKV.

**Risks**:
- TiKV Coprocessor API is primarily designed for TiDB's SQL pushdown. Adapting it for SPARQL patterns may require non-trivial protobuf schema work.
- This is the most research-heavy phase. Budget extra time for prototyping and validation.
- Semi-join filters add query planning complexity. Start with a simple heuristic (2-BGP joins only).

---

## Phase 5: Containerization & Kubernetes Manifests

**Objective**: Package the application as OCI container images and create Kubernetes deployment manifests.

**Blockers**: Phase 2 task 2.5 complete (configurable TiKV connection). Can start container work as soon as the server binary exists.

| # | Task | Owner | Description | Acceptance Criteria |
|---|------|-------|-------------|---------------------|
| 5.1 | Multi-stage Containerfile | `/k8s-deploy` | Rust builder stage (cargo build --release) + minimal runtime stage (distroless or UBI-minimal). Target: < 50MB image. | `podman build` succeeds; container starts and serves SPARQL endpoint |
| 5.2 | TiKV operator evaluation | `/tikv-ops` + `/k8s-deploy` | Evaluate TiKV Operator for Kubernetes. Test: deploy TiKV cluster via operator, verify PD + TiKV pods, storage persistence. | TiKV cluster deployed via operator; survives pod restarts |
| 5.3 | Kubernetes manifests | `/k8s-deploy` | Deployment, Service, ConfigMap, PersistentVolumeClaim for Oxigraph server. Separate namespace. | `kubectl apply` deploys working Oxigraph + TiKV stack; SPARQL queries work |
| 5.4 | Helm chart | `/k8s-deploy` | Parameterized Helm chart: replica count, TiKV endpoints, resource limits, SHACL mode, storage class. | `helm install` deploys full stack; `helm upgrade` with changed values works |
| 5.5 | Health & readiness probes | `/rust-dev` + `/k8s-deploy` | `/health` (liveness) and `/ready` (checks TiKV connectivity) endpoints in the server. Wire into Kubernetes probes. | Pods restart on health failure; traffic only routes to ready pods |
| 5.6 | Container CI pipeline | `/k8s-deploy` | Build and push container images on tag/release. Multi-arch (amd64 + arm64) if feasible. | Tagged releases produce container images in registry |

**Decisions needed**:
- [ ] `/k8s-deploy`: Container registry — quay.io, ghcr.io, or internal registry?
- [ ] `/k8s-deploy`: Base image — Red Hat UBI for OpenShift compatibility, or distroless for minimal size?

---

## Phase 6: OpenShift Production Deployment

**Objective**: Deploy the full stack on OpenShift with production-grade configuration, monitoring, and security.

**Blockers**: Phase 5 complete (Helm chart works on vanilla Kubernetes)

| # | Task | Owner | Description | Acceptance Criteria |
|---|------|-------|-------------|---------------------|
| 6.1 | OpenShift adaptation | `/k8s-deploy` | Adapt manifests for OpenShift: SecurityContextConstraints, Routes (instead of Ingress), image pull policies, service accounts. | `oc apply` deploys on OpenShift; Route exposes SPARQL endpoint |
| 6.2 | TiKV cluster sizing | `/tikv-ops` | Size TiKV cluster for target dataset. Define: node count, CPU/memory per node, storage class (SSD), Region size tuning. Document rationale. | Sizing guide committed; cluster handles target dataset without Region explosion |
| 6.3 | Region tuning | `/tikv-ops` | Configure Hibernate Region, Region Merge thresholds, Raftstore concurrency. Tune for Oxigraph's many-small-keys pattern. | Raftstore CPU usage stable under load; heartbeat overhead < 5% |
| 6.4 | Monitoring stack | `/tikv-ops` + `/k8s-deploy` | Deploy Prometheus + Grafana. Import TiKV dashboards. Add Oxigraph custom metrics: query latency, active transactions, SHACL validation rate. | Grafana dashboards show TiKV and Oxigraph metrics; alerts configured |
| 6.5 | TLS & authentication | `/k8s-deploy` | mTLS between Oxigraph and TiKV. SPARQL endpoint behind OAuth/OIDC or API key auth. | All traffic encrypted; unauthenticated requests rejected |
| 6.6 | Backup & restore | `/tikv-ops` | TiKV backup strategy using `tikv-br` (Backup & Restore). Scheduled backups to object storage. Documented restore procedure. | Backup runs on schedule; restore to new cluster verified |
| 6.7 | Load testing | `/test-qa` | Sustained load test: concurrent SPARQL queries + writes under production-like conditions. Identify breaking points. | System stable under target QPS; P99 latency documented |

**Risks**:
- OpenShift security policies may restrict TiKV operator behavior (privileged containers, host networking).
- TiKV's PD requires stable pod identity (StatefulSet) — verify with OpenShift's scheduler.

---

## Phase 7: Developer Sandbox Variant

**Objective**: Create a resource-constrained deployment suitable for Red Hat Developer Sandbox (limited CPU, memory, no persistent storage guarantees).

**Blockers**: Phase 5 task 5.4 complete (Helm chart exists). Can run in parallel with Phase 6.

| # | Task | Owner | Description | Acceptance Criteria |
|---|------|-------|-------------|---------------------|
| 7.1 | Sandbox constraints analysis | `/k8s-deploy` | Document Developer Sandbox resource limits: CPU, memory, storage, namespace restrictions, no cluster-admin. | Constraints documented |
| 7.2 | Single-node TiKV mode | `/tikv-ops` + `/k8s-deploy` | TiKV single-node configuration (1 PD + 1 TiKV) with reduced resource requests. Alternatively, fall back to embedded RocksDB for sandbox. | Deployment fits within sandbox resource limits |
| 7.3 | Sandbox Helm values | `/k8s-deploy` | `values-sandbox.yaml` overlay: minimal replicas, reduced resource requests/limits, ephemeral storage option, pre-loaded sample data. | `helm install -f values-sandbox.yaml` deploys in sandbox |
| 7.4 | Quick-start guide | `/k8s-deploy` | Step-by-step guide: sign up for Developer Sandbox, deploy via Helm, run first SPARQL query, upload SHACL shapes. | New user can follow guide and have working instance in < 15 minutes |
| 7.5 | Sample dataset & shapes | `/test-qa` | Curated small dataset (e.g., subset of LUBM or DBpedia) + SHACL shapes for demo purposes. | Dataset loads in < 30 seconds; SHACL validation demo works |

**Decisions needed**:
- [ ] `/architect`: For sandbox, is TiKV worth the overhead, or should we default to RocksDB backend (which still showcases the StorageBackend trait)?
- [ ] `/k8s-deploy`: Can TiKV Operator run without cluster-admin in Developer Sandbox?

---

## Phase 8: Testing, Hardening & Release

**Objective**: Comprehensive testing, W3C compliance verification, and preparation for public release.

**Blockers**: Phases 2, 3, 5 complete (core functionality works)

| # | Task | Owner | Description | Acceptance Criteria |
|---|------|-------|-------------|---------------------|
| 8.1 | W3C SPARQL 1.1 compliance | `/test-qa` | Run W3C SPARQL 1.1 test suite against TiKV backend. Identify and fix any regressions from the storage abstraction. | Same pass rate as upstream Oxigraph on RocksDB |
| 8.2 | W3C SHACL compliance | `/test-qa` | Run W3C SHACL test suite via rudof integration. Document any failures and whether they're rudof or integration bugs. | Compliance report committed; critical failures fixed |
| 8.3 | Chaos testing | `/test-qa` + `/tikv-ops` | Kill TiKV nodes during transactions. Verify: no data loss, no corruption, transactions properly fail and can be retried. | All chaos scenarios pass; data integrity verified post-recovery |
| 8.4 | Security audit | `/k8s-deploy` | Container image CVE scan, dependency audit (`cargo audit`), RBAC review, network policy review. | No critical/high CVEs; all findings documented and mitigated |
| 8.5 | Documentation | All agents | API documentation, architecture guide, operations runbook, troubleshooting guide. | Docs reviewed and committed |
| 8.6 | Release packaging | `/k8s-deploy` | Versioned release: tagged container images, Helm chart in OCI registry, GitHub release with changelog. | `helm install` from registry works; container image pullable |

---

## Dependency Graph

```
Phase 0 (Bootstrap)
    │
    ▼
Phase 1 (StorageBackend Trait)
    │
    ├──────────────────────┐
    ▼                      ▼
Phase 2 (TiKV Backend)    Phase 3 (SHACL/Rudof)
    │                      │
    ├──────────┬───────────┘
    ▼          │
Phase 4       │    Phase 5 (Containers/K8s)
(Query Opt)   │        │
    │         │    ┌───┴────────────┐
    │         │    ▼                ▼
    │         │  Phase 6          Phase 7
    │         │  (OpenShift)      (Sandbox)
    │         │    │                │
    └─────────┴────┴────────────────┘
                   │
                   ▼
             Phase 8 (Hardening & Release)
```

**Critical path**: 0 → 1 → 2 → 4 → 8

**Parallel tracks**:
- Phase 3 (SHACL) can start as soon as Phase 1 is done, runs parallel to Phase 2
- Phase 5 (Containers) can start container work during Phase 2, full integration after
- Phase 7 (Sandbox) runs parallel to Phase 6

---

## Risk Register

| ID | Risk | Impact | Likelihood | Mitigation |
|----|------|--------|------------|------------|
| R1 | Oxigraph storage layer too tightly coupled to RocksDB | Phase 1 takes 2-3x longer | Medium | Start with code audit; engage upstream maintainer via discussion #1487 |
| R2 | `tikv-client` crate incompatible with current tokio version | Blocks Phase 2 | Low | Pin compatible versions; worst case, use raw gRPC client |
| R3 | TiKV Coprocessor API too SQL-specific for SPARQL | Phase 4 delivers limited value | Medium | Start with simple filter pushdown; defer complex pushdown to later iteration |
| R4 | Async conversion cascades through entire Oxigraph codebase | Massive refactoring scope | High | Consider sync wrapper (`block_on`) for TiKV calls to avoid async infection |
| R5 | `rudof` SRDF trait requires methods Oxigraph can't efficiently support | SHACL integration incomplete | Low | Audit full trait surface early; collaborate with rudof maintainers |
| R6 | Developer Sandbox resources too constrained for TiKV | Sandbox uses RocksDB fallback | Medium | Acceptable fallback; still demonstrates StorageBackend abstraction |
| R7 | TiKV Operator requires cluster-admin on OpenShift | Deployment complexity increases | Medium | Test early; manual StatefulSet deployment as fallback |

---

## Getting Started: Recommended First Actions

1. **`/architect`**: Make the three key decisions from Phase 1 (generics vs dyn, async strategy, fork strategy)
2. **`/rust-dev`**: Fork Oxigraph, set up workspace, audit `oxigraph/src/storage/` for refactoring scope
3. **`/tikv-ops`**: Set up local TiKV dev cluster, verify `tikv-client` crate works with current Rust toolchain
4. **`/test-qa`**: Begin designing the backend conformance test suite (task 1.5) — this unblocks all backend work
5. **`/k8s-deploy`**: Set up CI pipeline skeleton (task 0.3) — early CI prevents integration pain later
