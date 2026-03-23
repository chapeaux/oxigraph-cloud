# Oxigraph Cloud-Native

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue.svg)](LICENSE)

Cloud-native distributed SPARQL + SHACL database built on Oxigraph, TiKV, and Rudof.

## Overview

Oxigraph Cloud-Native extends the [Oxigraph](https://github.com/oxigraph/oxigraph) RDF triplestore with a pluggable storage layer, SHACL shape validation, and Kubernetes-ready deployment. It introduces a `StorageBackend` trait that decouples the SPARQL engine from its storage, enabling both embedded RocksDB for single-node use and distributed TiKV for horizontally scalable deployments. SHACL validation is powered by the Rust-native [rudof](https://github.com/rudof-project/rudof) library and can be configured to reject, warn on, or skip non-conforming data at ingestion time. The project ships with Helm charts for production OpenShift clusters and resource-constrained Developer Sandbox environments.

## Features

- **SPARQL 1.1 query and update** -- full compliance inherited from upstream Oxigraph
- **Pluggable storage backends** -- RocksDB (embedded, single-node) and TiKV (distributed, Raft-replicated), selectable at startup via `--backend`
- **SHACL validation via rudof** -- three modes: Off (default), Warn (log but accept), Enforce (reject with HTTP 422)
- **Write authentication** -- API key protection for write operations via `--write-key` or `OXIGRAPH_WRITE_KEY`
- **TiKV Coprocessor pushdown** -- scan, filter, aggregate, and bloom filter semi-join operations pushed to Region-local execution
- **Cloud-native deployment** -- Helm chart with OpenShift Route support, health/readiness probes, network policies
- **HTTP transaction API** -- begin/commit/rollback transactions over HTTP with buffered multi-step writes
- **Changelog with undo** -- opt-in changelog recording of write operations, with `POST /changelog/{id}/undo` to revert
- **OpenTelemetry observability** -- Prometheus `/metrics` endpoint and OTLP distributed trace export (feature-gated via `--features otel`)
- **Structured JSON logging** -- machine-parseable log output via `tracing-subscriber`
- **Configurable query timeouts** -- per-query execution time limits via `--query-timeout`
- **Configurable upload size limits** -- control maximum request body size via `--max-upload-size`
- **CORS support** -- configurable allowed origins via `--cors-origins`
- **In-memory backend** -- always-available backend for testing and WASM targets

## Quick Start

### Prerequisites

- Rust 1.87+ and Cargo
- C/C++ compiler and CMake (for RocksDB, vendored via `oxrocksdb-sys`)

### Build

```bash
cargo build --release -p oxigraph-server
```

### Run

```bash
./target/release/oxigraph-cloud --bind 127.0.0.1:7878 --location /tmp/oxigraph-data
```

The server starts on `http://127.0.0.1:7878`. Without `--location`, data is stored in memory only.

### Test with curl

Insert RDF data:

```bash
curl -X POST http://127.0.0.1:7878/store \
  -H "Content-Type: text/turtle" \
  -d '
@prefix ex: <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .

ex:alice a foaf:Person ;
  foaf:name "Alice" ;
  foaf:knows ex:bob .

ex:bob a foaf:Person ;
  foaf:name "Bob" .
'
```

Query with SPARQL SELECT:

```bash
curl -s http://127.0.0.1:7878/query \
  -H "Accept: application/sparql-results+json" \
  --data-urlencode "query=
    PREFIX foaf: <http://xmlns.com/foaf/0.1/>
    SELECT ?name WHERE {
      ?person a foaf:Person ;
        foaf:name ?name .
    }
    ORDER BY ?name
  " | python3 -m json.tool
```

Insert via SPARQL UPDATE:

```bash
curl -X POST http://127.0.0.1:7878/update \
  -H "Content-Type: application/sparql-update" \
  -d '
    PREFIX ex: <http://example.org/>
    PREFIX foaf: <http://xmlns.com/foaf/0.1/>
    INSERT DATA {
      ex:carol a foaf:Person ;
        foaf:name "Carol" .
    }
  '
```

## Docker / Podman

Build the container image:

```bash
podman build -t oxigraph-cloud .
```

Run the container:

```bash
podman run -p 7878:7878 oxigraph-cloud
```

The Containerfile uses a multi-stage build with Red Hat UBI 9. Four image variants are available:

| Variant | Build command | Features | Base |
|---------|--------------|----------|------|
| Default | `podman build -t oxigraph-cloud .` | RocksDB + SHACL | ubi-micro |
| TiKV | `podman build -t oxigraph-cloud:tikv -f Containerfile.tikv .` | RocksDB + TiKV + SHACL | ubi-minimal |
| OTel | `podman build --build-arg EXTRA_FEATURES=otel -t oxigraph-cloud:otel .` | RocksDB + SHACL + OTel | ubi-micro |
| TiKV + OTel | `podman build --build-arg EXTRA_FEATURES=otel -t oxigraph-cloud:tikv-otel -f Containerfile.tikv .` | All features | ubi-minimal |

TiKV variants use `ubi-minimal` for full glibc DNS resolution (required by gRPC). Default variants use the smaller `ubi-micro`.

Pre-built images are available at `quay.io/ldary/oxigraph-cloud` with tag suffixes: `:latest`, `:-tikv`, `:-otel`, `:-tikv-otel`.

To persist data across container restarts:

```bash
podman run -p 7878:7878 -v oxigraph-data:/opt/oxigraph/data oxigraph-cloud
```

## Helm Deployment (OpenShift / Kubernetes)

### Production install

```bash
helm install oxigraph helm/oxigraph-cloud
```

### Developer Sandbox install

```bash
helm install oxigraph helm/oxigraph-cloud \
  -f helm/oxigraph-cloud/values-sandbox.yaml
```

### Key Helm values

| Value | Default | Description |
|-------|---------|-------------|
| `backend` | `rocksdb` | Storage backend: `rocksdb` or `tikv` |
| `tikv.pdEndpoints` | `[]` | TiKV PD endpoint addresses |
| `shacl.mode` | `off` | SHACL validation mode: `off`, `warn`, `enforce` |
| `replicas` | `1` | Number of Oxigraph pods |
| `storage.size` | `10Gi` | PVC size for RocksDB data |
| `route.enabled` | `false` | Create an OpenShift Route |
| `monitoring.enabled` | `false` | Enable Prometheus ServiceMonitor |
| `monitoring.otel.enabled` | `false` | Enable OpenTelemetry (adds `--otel` flag and env vars) |
| `monitoring.otel.endpoint` | `""` | OTLP exporter endpoint |
| `tls.enabled` | `false` | Enable mTLS for Oxigraph-to-TiKV communication |
| `resources.requests.cpu` | `100m` | CPU request |
| `resources.requests.memory` | `256Mi` | Memory request |

See `helm/oxigraph-cloud/values.yaml` for the full list. For a step-by-step sandbox walkthrough, see [docs/sandbox-quickstart.md](docs/sandbox-quickstart.md).

## TiKV Backend

### Kubernetes / Helm deployment

Deploy a TiKV cluster using the included Helm chart, then connect Oxigraph:

```bash
helm install tikv-cluster helm/tikv-cluster
helm install oxigraph helm/oxigraph-cloud -f helm/oxigraph-cloud/values-tikv.yaml
```

The `tikv-cluster` chart deploys PD and TiKV as StatefulSets with persistent storage. PD advertises its client URL via a ClusterIP service so that tikv-client can resolve member addresses from any pod.

### Local development (Docker Compose)

```bash
docker compose -f docker-compose.tikv.yml up -d
```

This starts 1 PD node and 3 TiKV nodes. The PD client endpoint is exposed at `127.0.0.1:2379`.

```bash
./target/release/oxigraph-cloud \
  --backend tikv \
  --pd-endpoints 127.0.0.1:2379 \
  --bind 127.0.0.1:7878
```

### Tear down

```bash
docker compose -f docker-compose.tikv.yml down      # stop, keep data
docker compose -f docker-compose.tikv.yml down -v    # stop and delete data
```

For cluster sizing, Region tuning, backup/restore, and monitoring, see [docs/tikv-operations-guide.md](docs/tikv-operations-guide.md).

## SHACL Validation

### Upload shapes

```bash
curl -X POST http://127.0.0.1:7878/shacl/shapes \
  -H "Content-Type: text/turtle" \
  -d '
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

<http://example.org/PersonShape> a sh:NodeShape ;
  sh:targetClass foaf:Person ;
  sh:property [
    sh:path foaf:name ;
    sh:minCount 1 ;
    sh:datatype xsd:string ;
  ] .
'
```

### Set validation mode

```bash
curl -X PUT http://127.0.0.1:7878/shacl/mode \
  -H "Content-Type: application/json" \
  -d '{"mode": "enforce"}'
```

Supported modes: `off`, `warn`, `enforce` (alias: `strict`).

### Trigger on-demand validation

```bash
curl -X POST http://127.0.0.1:7878/shacl/validate \
  -H "Accept: application/json"
```

When validation mode is `strict`/`enforce`, inserting data that violates shapes returns HTTP 422 with a SHACL validation report. The report format is content-negotiated: request `application/json` for a simplified JSON report, or `text/turtle` for the W3C SHACL Validation Report in RDF.

For the full API specification, see [docs/shacl-api-spec.md](docs/shacl-api-spec.md).

## HTTP Transactions

Multi-step write operations can be composed into atomic transactions:

```bash
# Begin a transaction
TXN_URL=$(curl -s -X POST http://127.0.0.1:7878/transactions \
  -H "Content-Type: application/json" | python3 -c "import sys,json; print(json.load(sys.stdin)['transaction_id'])")

# Add data within the transaction
curl -X PUT "http://127.0.0.1:7878/transactions/$TXN_URL/add" \
  -H "Content-Type: text/turtle" \
  -d '<http://example.org/x> <http://example.org/p> "hello" .'

# Query within the transaction (sees uncommitted data)
curl -X POST "http://127.0.0.1:7878/transactions/$TXN_URL/query" \
  -H "Content-Type: application/sparql-query" \
  -d 'SELECT * WHERE { ?s ?p ?o }'

# Commit (or DELETE to rollback)
curl -X PUT "http://127.0.0.1:7878/transactions/$TXN_URL/commit"
```

Transactions use a buffered operations pattern and are automatically cleaned up after `--transaction-timeout` seconds of inactivity (default: 60s).

## Changelog and Undo

Enable changelog recording to track and revert write operations:

```bash
# Start server with changelog enabled
./target/release/oxigraph-cloud --changelog --location /tmp/oxigraph-data

# After making writes, list changelog entries
curl http://127.0.0.1:7878/changelog

# Undo a specific transaction
curl -X POST http://127.0.0.1:7878/changelog/1/undo

# View details of a changelog entry
curl http://127.0.0.1:7878/changelog/1
```

Changelog entries are stored as RDF in the `<urn:oxigraph:changelog>` named graph and are retained up to `--changelog-retain` entries (default: 100).

## OpenTelemetry Observability

OpenTelemetry support is available behind the `otel` cargo feature:

```bash
cargo build --release -p oxigraph-server --features otel
```

### Enable metrics

```bash
./target/release/oxigraph-cloud --otel --location /tmp/oxigraph-data
```

Prometheus metrics are exposed at `GET /metrics`:

```bash
curl http://127.0.0.1:7878/metrics
```

Available metrics: `oxigraph_http_requests_total`, `oxigraph_sparql_queries_total`, `oxigraph_query_duration_seconds`, `oxigraph_active_transactions`, `oxigraph_shacl_validations_total`, `oxigraph_store_triple_count`.

### Enable distributed tracing

```bash
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
  ./target/release/oxigraph-cloud --otel --location /tmp/oxigraph-data
```

Traces are exported via OTLP gRPC to the configured endpoint (e.g., Jaeger, OpenTelemetry Collector).

## CLI Reference

| Flag | Default | Description |
|------|---------|-------------|
| `--backend` | `rocksdb` | Storage backend: `rocksdb` or `tikv` |
| `--pd-endpoints` | `127.0.0.1:2379` | TiKV PD endpoints (comma-separated) |
| `--bind` | `127.0.0.1:7878` | HTTP server bind address |
| `--location` | (none) | RocksDB data directory; omit for in-memory storage |
| `--shacl-mode` | `off` | SHACL validation mode: `off`, `enforce`, `warn` |
| `--write-key` | (none) | API key for write operations (env: `OXIGRAPH_WRITE_KEY`) |
| `--cors-origins` | (empty) | CORS allowed origins: `*` for wildcard, or comma-separated list |
| `--query-timeout` | `30` | Query execution timeout in seconds |
| `--max-upload-size` | `134217728` | Maximum upload body size in bytes (default 128 MB) |
| `--changelog` | `false` | Enable changelog recording for write operations |
| `--changelog-retain` | `100` | Maximum changelog entries to retain (0 = unlimited) |
| `--transaction-timeout` | `60` | Transaction idle timeout in seconds |
| `--otel` | `false` | Enable Prometheus `/metrics` endpoint (requires `otel` feature) |
| `--otel-endpoint` | (none) | OTLP endpoint for trace export (env: `OTEL_EXPORTER_OTLP_ENDPOINT`) |
| `--otel-service-name` | `oxigraph-cloud` | OpenTelemetry service name (env: `OTEL_SERVICE_NAME`) |

## Architecture

Oxigraph Cloud-Native is organized as a Cargo workspace with the forked Oxigraph source and additional crates for TiKV integration and SHACL validation.

```
oxigraph-cloud/
  oxigraph/                    # Forked Oxigraph (core SPARQL engine)
    lib/oxigraph/src/
      storage/
        backend_trait.rs       # StorageBackend trait hierarchy
        rocksdb.rs             # RocksDB backend
        tikv.rs                # TiKV backend (feature-gated)
        memory.rs              # In-memory backend
      store.rs                 # Public Store API
      sparql/                  # SPARQL evaluation
  crates/
    oxigraph-server/           # HTTP server binary (CLI)
    oxigraph-tikv/             # TiKV config types and integration tests
    oxigraph-shacl/            # SHACL validation via rudof
    oxigraph-coprocessor/      # TiKV Coprocessor plugin (cdylib)
  deploy/
    helm/oxigraph-cloud/       # Helm chart (default, tikv, sandbox values)
    k8s/                       # Raw Kubernetes manifests
    openshift/                 # OpenShift Kustomize overlay
    monitoring/                # Prometheus ServiceMonitor + Grafana dashboard
    docker-compose.yml         # Local PD + TiKV + Oxigraph stack
  tests/
    benchmark/                 # Criterion benchmarks
    chaos/                     # Chaos testing scripts
    data/                      # Sample RDF dataset + SHACL shapes
    integration/               # SPARQL roundtrip + bulk load tests
```

Key design decisions:

- **Enum dispatch** (not generics) keeps `Store` non-generic while supporting multiple backends at near-zero overhead (ADR-002)
- **Sync trait with `block_on` bridge** avoids async infection of the iterator-heavy SPARQL evaluator (ADR-003)
- **1-byte key prefix** maps Oxigraph's 11 column families to TiKV's flat key space, preserving lexicographic scan order

For the full architecture guide, see [docs/architecture-guide.md](docs/architecture-guide.md).

## Testing

Run all workspace tests:

```bash
cargo test --workspace
```

Run backend conformance tests with RocksDB:

```bash
cargo test -p oxigraph --features rocksdb -- backend_tests
```

Run SHACL tests:

```bash
cargo test -p oxigraph-shacl
```

Run TiKV integration tests (requires a running TiKV cluster):

```bash
TIKV_PD_ENDPOINTS=127.0.0.1:2379 cargo test -p oxigraph-tikv \
  --features integration-tests -- --test-threads=1
```

Run W3C SPARQL compliance tests:

```bash
cargo test -p oxigraph-testsuite --features rocksdb
```

Run benchmarks:

```bash
cargo bench -p oxigraph --features tikv
```

## Documentation

### Research documents

| Document | Description |
|----------|-------------|
| [docs/01-overview.md](docs/01-overview.md) | Project goals, high-level architecture, key decisions |
| [docs/02-oxigraph-storage-architecture.md](docs/02-oxigraph-storage-architecture.md) | KV tables, byte encoding, transactional guarantees |
| [docs/03-rudof-shacl-integration.md](docs/03-rudof-shacl-integration.md) | SRDF trait bridge, rudof crates, performance benchmarks |
| [docs/04-distributed-sparql-theory.md](docs/04-distributed-sparql-theory.md) | OLTP/OLAP theory, network bottleneck, semi-joins |
| [docs/05-tikv-backend.md](docs/05-tikv-backend.md) | TiKV architecture, Coprocessor pushdown, Region tuning |
| [docs/06-backend-alternatives-rejected.md](docs/06-backend-alternatives-rejected.md) | FoundationDB, DynamoDB, S3/Parquet -- why rejected |
| [docs/07-storage-trait-design.md](docs/07-storage-trait-design.md) | StorageBackend trait design, async considerations |
| [docs/08-references.md](docs/08-references.md) | All external references and links |

### Architecture and implementation

| Document | Description |
|----------|-------------|
| [docs/architecture-guide.md](docs/architecture-guide.md) | System overview, component roles, data flow |
| [docs/api-reference.md](docs/api-reference.md) | SPARQL, SHACL, health endpoint reference |
| [docs/tikv-key-encoding.md](docs/tikv-key-encoding.md) | Column family to key prefix mapping |
| [docs/tikv-dev-setup.md](docs/tikv-dev-setup.md) | Local TiKV cluster setup (tiup / Docker Compose) |
| [docs/tikv-sizing-guide.md](docs/tikv-sizing-guide.md) | Cluster sizing by dataset size |
| [docs/tikv-tuning.md](docs/tikv-tuning.md) | Region, write, read, and GC tuning |
| [docs/tikv-operations-guide.md](docs/tikv-operations-guide.md) | TiKV cluster operations guide |
| [docs/backup-restore.md](docs/backup-restore.md) | TiKV BR backup/restore procedures |
| [docs/shacl-api-spec.md](docs/shacl-api-spec.md) | SHACL REST API specification |
| [docs/coprocessor-pushdown-mapping.md](docs/coprocessor-pushdown-mapping.md) | SPARQL operator → Coprocessor mapping |
| [docs/sandbox-quickstart.md](docs/sandbox-quickstart.md) | Developer Sandbox deployment guide |
| [docs/security-deployment.md](docs/security-deployment.md) | TLS, auth, network policies, container security |
| [docs/operations-runbook.md](docs/operations-runbook.md) | Day-to-day operations and incident response |
| [docs/troubleshooting.md](docs/troubleshooting.md) | Common issues and solutions |
| [docs/test-plan.md](docs/test-plan.md) | Test strategy across all phases |
| [docs/conformance-report.md](docs/conformance-report.md) | W3C SPARQL/SHACL conformance results |
| [docs/benchmark-results.md](docs/benchmark-results.md) | TiKV vs RocksDB performance comparison |
| [docs/security-audit-report.md](docs/security-audit-report.md) | cargo audit and license check results |
| [docs/test-conformance-suite.md](docs/test-conformance-suite.md) | Backend conformance test specification |
| [docs/query-optimization-design.md](docs/query-optimization-design.md) | Coprocessor pushdown and query optimization |
| [docs/security-compliance-assessment.md](docs/security-compliance-assessment.md) | Security and compliance assessment |
| [docs/legal-license-audit.md](docs/legal-license-audit.md) | License compatibility audit |

### Architecture Decision Records

| ADR | Title |
|-----|-------|
| [ADR-001](docs/adr/001-fork-strategy.md) | Fork Strategy |
| [ADR-002](docs/adr/002-generics-vs-dyn-dispatch.md) | Generics vs Dynamic Dispatch |
| [ADR-003](docs/adr/003-async-strategy.md) | Async Strategy |
| [ADR-004](docs/adr/004-shacl-validation-report-format.md) | SHACL Validation Report Format |

## Contributing

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/my-feature`
3. Make your changes
4. Run the test suite: `cargo test --workspace`
5. Run clippy: `cargo clippy --workspace --all-features`
6. Commit and push your branch
7. Open a Pull Request against `main`

Please run the full test suite before submitting. If your change touches storage backends, verify the conformance tests pass for all affected backends.

All contributions are dual-licensed under MIT and Apache-2.0 (see below).

## License

This project is licensed under either of

- [Apache License, Version 2.0](http://www.apache.org/licenses/LICENSE-2.0)
- [MIT License](http://opensource.org/licenses/MIT)

at your option. See the [LICENSE](LICENSE) file for details.
