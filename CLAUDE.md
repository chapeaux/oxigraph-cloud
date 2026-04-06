# Oxigraph Cloud-Native: TiKV & Rudof Integration

Cloud-native distributed SPARQL + SHACL database. All 8 implementation phases complete.

## Project Structure

| Path | Purpose |
|------|---------|
| `oxigraph/` | Forked Oxigraph with StorageBackend trait (RocksDB, TiKV, Memory) |
| `crates/oxigraph-server/` | HTTP server binary (SPARQL, SHACL, transactions, changelog, health) |
| `crates/oxigraph-shacl/` | SHACL validation via rudof (validator, shapes, report) |
| `crates/oxigraph-tikv/` | TiKV config types and integration tests |
| `crates/oxigraph-coprocessor/` | TiKV Coprocessor plugin (scan, filter, aggregate, bloom) |
| `crates/oxigraph-cdc/` | W3C Solid Notifications CDC engine (WebSocket, SSE, AS2 JSON-LD) |
| `helm/oxigraph-cloud/` | Oxigraph Helm chart (values.yaml, values-tikv.yaml, values-sandbox.yaml) |
| `helm/tikv-cluster/` | TiKV cluster Helm chart (PD + TiKV StatefulSets, services) |
| `deploy/helm/oxigraph-cloud/` | Legacy Helm chart location |
| `deploy/k8s/` | Raw Kubernetes manifests |
| `deploy/openshift/` | OpenShift Kustomize overlay (Route, RBAC) |
| `deploy/monitoring/` | Prometheus ServiceMonitor + Grafana dashboard |
| `deploy/docker-compose.yml` | Local PD + TiKV + Oxigraph stack |
| `tests/benchmark/` | Criterion benchmarks |
| `tests/chaos/` | Chaos testing scripts (kill-pod, concurrent-load) |
| `PLAN.md` | Full 8-phase implementation plan |

## Key Implementation Details

- **StorageBackend trait**: `oxigraph/lib/oxigraph/src/storage/backend_trait.rs`
- **TiKV backend**: `oxigraph/lib/oxigraph/src/storage/tikv.rs` (~1570 lines)
  - ID2STR read cache (`RefCell<HashMap>` per reader, >90% hit rate)
  - Scan batch size 4096 (was 512)
  - Transaction reuse across scan batches (lazy init, single txn per iterator)
  - Read-only snapshot transactions via `TransactionOptions`
- **Server**: `crates/oxigraph-server/src/main.rs` — SHACL validation-on-ingest wired into `/update` and `/store POST`
- **SHACL mode flag**: `--shacl-mode off|warn|enforce` (server accepts `strict` as alias for `enforce`)
- **SHACL mode API**: `PUT /shacl/mode` expects JSON body `{"mode": "enforce"}`
- **Write auth**: `--write-key` or `OXIGRAPH_WRITE_KEY` env var, `Authorization: Bearer <key>` header
- **Transactions**: `crates/oxigraph-server/src/transactions.rs` — buffered ops replayed on commit (`Transaction<'a>` borrows Store, can't span HTTP requests)
- **Changelog**: `crates/oxigraph-server/src/changelog.rs` — stored in `<urn:oxigraph:changelog>` named graph, opt-in via `--changelog`. Records all write paths: transaction commits (operation `"transaction"`), direct SPARQL UPDATE (operation `"update"`), and `/store` POST (operation `"store"`)
- **CDC**: `crates/oxigraph-cdc/` — W3C Solid Notifications Protocol, feature-gated behind `cdc`
  - Separate axum server on dedicated tokio runtime (same process, different port)
  - `tokio::sync::broadcast` bridge from sync changelog to async CDC server
  - WebSocket (WebSocketChannel2023) and SSE (StreamingHTTPChannel2023)
  - AS2 JSON-LD notifications with RDF-star N-Quads deltas
  - Configurable batching window, backpressure via bounded channels
- **TiKV connection retry**: exponential backoff (5 attempts, 100ms→1.6s) in `TiKvStorage::connect_with_config`
- **Telemetry**: `crates/oxigraph-server/src/telemetry.rs` — Prometheus metrics via `prometheus` crate, OTLP traces via `tracing-opentelemetry` (feature-gated behind `otel`)

## Container Images

**Registry**: `quay.io/ldary/oxigraph-cloud`

| Tag suffix | Containerfile | Features | Base image |
|------------|--------------|----------|------------|
| (none) | `Containerfile` | rocksdb, shacl | ubi9/ubi-micro |
| `-tikv` | `Containerfile.tikv` | rocksdb, tikv, shacl | ubi9/ubi-minimal |
| `-otel` | `Containerfile` | rocksdb, shacl, otel | ubi9/ubi-micro |
| `-tikv-otel` | `Containerfile.tikv` | rocksdb, tikv, shacl, otel | ubi9/ubi-minimal |

- **TiKV variants** use `ubi-minimal` (full glibc NSS resolver for gRPC DNS; installs openssl-libs, zlib, libstdc++ via microdnf)
- **Default variants** use `ubi-micro` (near-zero CVEs, only libstdc++ copied from builder)
- **Build arg**: `EXTRA_FEATURES` controls additional cargo features (e.g., `otel`)
- **CI/CD**: GitHub Actions matrix builds all 4 variants, pushes to quay.io on tag releases only

## Cargo Features

| Feature | Default | Description |
|---------|---------|-------------|
| `rocksdb` | Yes | RocksDB embedded storage backend |
| `shacl` | Yes | SHACL validation via rudof |
| `tikv` | No | TiKV distributed storage backend |
| `otel` | No | OpenTelemetry metrics + traces (Prometheus `/metrics`, OTLP export) |
| `cdc` | No | W3C Solid Notifications CDC engine (WebSocket + SSE) |

## HTTP API Summary

### Main Server (default port 7878)

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/query` | No | SPARQL query (query param) |
| POST | `/query` | No | SPARQL query (body) |
| POST | `/update` | Write | SPARQL UPDATE |
| POST/GET | `/store` | Write/No | Load/dump RDF data |
| GET | `/health` | No | Liveness probe |
| GET | `/ready` | No | Readiness probe |
| GET | `/metrics` | No | Prometheus metrics (otel feature) |
| POST | `/transactions` | Write | Begin transaction |
| PUT | `/transactions/{id}/add` | Write | Add RDF to transaction |
| PUT | `/transactions/{id}/remove` | Write | Remove RDF from transaction |
| POST | `/transactions/{id}/query` | No | Query within transaction |
| POST | `/transactions/{id}/update` | Write | SPARQL UPDATE within transaction |
| PUT | `/transactions/{id}/commit` | Write | Commit transaction |
| DELETE | `/transactions/{id}` | Write | Rollback transaction |
| GET | `/changelog` | No | List changelog entries |
| GET | `/changelog/{id}` | No | Get changelog entry detail |
| POST | `/changelog/{id}/undo` | Write | Undo a transaction |
| DELETE | `/changelog` | Write | Purge changelog |
| POST | `/shacl/shapes` | Write | Upload SHACL shapes |
| GET | `/shacl/shapes` | No | Get loaded shapes info |
| DELETE | `/shacl/shapes` | Write | Delete shapes |
| POST | `/shacl/validate` | No | Trigger on-demand validation |
| GET | `/shacl/mode` | No | Get SHACL mode |
| PUT | `/shacl/mode` | Write | Set SHACL mode |

### CDC Server (configurable port, `--cdc-port`, requires `cdc` feature)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/.well-known/solid` | SNP discovery (JSON-LD description resource) |
| POST | `/subscription` | Create WebSocket or SSE subscription |
| GET | `/channel/ws/{id}` | WebSocket notification channel |
| GET | `/channel/sse/{id}` | Server-Sent Events notification channel |

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
