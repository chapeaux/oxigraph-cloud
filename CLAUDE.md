# Oxigraph Cloud-Native: TiKV & Rudof Integration

Cloud-native distributed SPARQL + SHACL database. All 8 implementation phases complete.

## Project Structure

| Path | Purpose |
|------|---------|
| `oxigraph/` | Forked Oxigraph with StorageBackend trait (RocksDB, TiKV, Memory) |
| `crates/oxigraph-server/` | HTTP server binary (SPARQL, SHACL, health endpoints) |
| `crates/oxigraph-shacl/` | SHACL validation via rudof (validator, shapes, report) |
| `crates/oxigraph-tikv/` | TiKV config types and integration tests |
| `crates/oxigraph-coprocessor/` | TiKV Coprocessor plugin (scan, filter, aggregate, bloom) |
| `deploy/helm/oxigraph-cloud/` | Helm chart (values.yaml, values-tikv.yaml, values-sandbox.yaml) |
| `deploy/k8s/` | Raw Kubernetes manifests |
| `deploy/openshift/` | OpenShift Kustomize overlay (Route, RBAC) |
| `deploy/docker-compose.yml` | Local PD + TiKV + Oxigraph stack |
| `tests/benchmark/` | Criterion benchmarks |
| `tests/chaos/` | Chaos testing scripts (kill-pod, concurrent-load) |
| `PLAN.md` | Full 8-phase implementation plan |

## Key Implementation Details

- **StorageBackend trait**: `oxigraph/lib/oxigraph/src/storage/backend_trait.rs`
- **TiKV backend**: `oxigraph/lib/oxigraph/src/storage/tikv.rs` (~1520 lines)
- **Server**: `crates/oxigraph-server/src/main.rs` — SHACL validation-on-ingest wired into `/update` and `/store POST`
- **SHACL mode flag**: `--shacl-mode off|warn|enforce` (server accepts `strict` as alias for `enforce`)
- **SHACL mode API**: `PUT /shacl/mode` expects JSON body `{"mode": "enforce"}`
- **Write auth**: `--write-key` or `OXIGRAPH_WRITE_KEY` env var, `Authorization: Bearer <key>` header
- **Transactions**: `POST /transactions` to begin, `PUT .../commit` to commit, `DELETE` to rollback
- **Changelog**: `GET /changelog`, `POST /changelog/{id}/undo` — opt-in via `--changelog`
- **OpenTelemetry**: `--features otel` enables Prometheus `/metrics` endpoint and OTLP trace export (`--otel`, `--otel-endpoint`)
- **Telemetry module**: `crates/oxigraph-server/src/telemetry.rs` — metrics, tracing init, `/metrics` handler
- **Container images**: `quay.io/ldary/oxigraph-cloud:0.7.2` (RocksDB + SHACL), plus `-tikv`, `-otel`, `-tikv-otel` variants
- **CI/CD**: GitHub Actions matrix builds all 4 image variants, pushes to quay.io on main/tags
- **Base image**: `ubi9/ubi-micro` (near-zero CVEs), stable Rust toolchain

## Reference Documentation

| File | Content |
|------|---------|
| `docs/01-overview.md` | Project goals, high-level architecture, key decisions |
| `docs/02-oxigraph-storage-architecture.md` | Current KV tables, byte encoding, transactional guarantees |
| `docs/03-rudof-shacl-integration.md` | SRDF trait bridge, rudof crates, performance benchmarks |
| `docs/04-distributed-sparql-theory.md` | OLTP/OLAP theory, network bottleneck, ExtVP/semi-joins |
| `docs/05-tikv-backend.md` | TiKV architecture, Coprocessor pushdown, Region tuning |
| `docs/06-backend-alternatives-rejected.md` | FoundationDB, DynamoDB, S3/Parquet — why rejected |
| `docs/07-storage-trait-design.md` | StorageBackend trait design, async considerations |
| `docs/08-references.md` | All external references and links |
