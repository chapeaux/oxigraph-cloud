# TiKV Client Rust Crate: Compatibility Report

**Date:** 2026-03-17
**Agent:** tikv-ops
**Status:** Based on known crate state; live verification recommended before implementation.

> **Note:** This report is based on the author's knowledge of the `tikv-client` crate as of
> early-to-mid 2025. The crate.io API, GitHub repo, and web searches were unavailable during
> this analysis. All version numbers, API details, and setup instructions should be verified
> against live sources before making implementation decisions.

---

## 1. Crate Overview

| Field | Value |
|-------|-------|
| Crate name | `tikv-client` |
| Repository | https://github.com/tikv/client-rust |
| crates.io | https://crates.io/crates/tikv-client |
| License | Apache-2.0 |
| Docs | https://docs.rs/tikv-client |

### Latest Version and Release History

The `tikv-client` crate has historically had a slow release cadence. Key releases:

| Version | Approximate Date | Notes |
|---------|------------------|-------|
| 0.3.0 | ~2024 | Major API stabilization, async-only |
| 0.2.0 | ~2023 | Transactional + Raw API split |
| 0.1.0 | ~2022 | Initial public release |

**Important:** The crate has remained at 0.x semver, indicating the API is not yet considered
stable. Breaking changes between minor versions are possible.

### Download Statistics

The crate has modest download numbers (low thousands), reflecting that it is a specialized
client for TiKV rather than a general-purpose library. This is expected and not a concern --
TiKV itself is a mature CNCF Graduated project.

---

## 2. Async Runtime: Tokio Version

The `tikv-client` crate is built on **Tokio** as its async runtime.

### Key Dependencies

- **tokio**: The crate requires Tokio 1.x (typically `tokio = "1"` with features
  `rt-multi-thread`, `macros`, `time`, `sync`, `net`).
- **grpcio** or **tonic**: The gRPC layer uses either `grpcio` (C-based gRPC bindings) or
  has been migrating toward `tonic` (pure-Rust gRPC). Check the latest `Cargo.toml` to
  confirm which is in use.
  - If `grpcio` is still used: requires CMake and a C/C++ toolchain at build time, plus
    the gRPC C core library. This adds complexity to container builds.
  - If `tonic` is used: pure Rust, much simpler build story.

### Oxigraph Compatibility

Oxigraph currently uses synchronous storage operations. The `tikv-client` crate is
async-only. This means:

1. The `StorageBackend` trait for TiKV must be async, OR
2. We must use `tokio::runtime::Runtime::block_on()` to bridge sync-to-async, OR
3. We make the entire storage layer async (preferred long-term approach).

**Recommendation:** Design the `StorageBackend` trait with async variants from the start.
Use `#[tokio::main]` or a shared `Runtime` handle in the Oxigraph server process.

---

## 3. Key API Surface

### TransactionClient

The primary entry point for transactional operations:

```rust
use tikv_client::TransactionClient;

// Connect to PD endpoints
let client = TransactionClient::new(vec!["127.0.0.1:2379"]).await?;

// Begin a transaction
let mut txn = client.begin_optimistic().await?;
// or
let mut txn = client.begin_pessimistic().await?;
```

### Transaction Methods

The `Transaction` struct exposes these key methods relevant to Oxigraph:

```rust
impl Transaction {
    // Point operations
    pub async fn get(&mut self, key: impl Into<Key>) -> Result<Option<Value>>;
    pub async fn put(&mut self, key: impl Into<Key>, value: impl Into<Value>) -> Result<()>;
    pub async fn delete(&mut self, key: impl Into<Key>) -> Result<()>;
    pub async fn key_exists(&mut self, key: impl Into<Key>) -> Result<bool>;

    // Batch operations
    pub async fn batch_get(&mut self, keys: impl IntoIterator<Item = impl Into<Key>>)
        -> Result<Vec<KvPair>>;

    // Range scan -- critical for Oxigraph index traversal
    pub async fn scan(
        &mut self,
        range: impl Into<BoundRange>,
        limit: u32,
    ) -> Result<Vec<KvPair>>;

    pub async fn scan_keys(
        &mut self,
        range: impl Into<BoundRange>,
        limit: u32,
    ) -> Result<Vec<Key>>;

    // Commit / rollback
    pub async fn commit(&mut self) -> Result<Option<Timestamp>>;
    pub async fn rollback(&mut self) -> Result<()>;
}
```

### RawClient (alternative, non-transactional)

```rust
use tikv_client::RawClient;

let client = RawClient::new(vec!["127.0.0.1:2379"]).await?;
client.put("key".to_owned(), "value".to_owned()).await?;
let value = client.get("key".to_owned()).await?;
```

### Key Types

- `Key`: wraps `Vec<u8>` -- directly compatible with Oxigraph's byte-encoded keys.
- `Value`: wraps `Vec<u8>` -- directly compatible with Oxigraph's encoded values.
- `BoundRange`: constructed from Rust range syntax (`key1..key2`, `key1..=key2`, `key1..`),
  which maps cleanly to Oxigraph's prefix-based index scans.

### Mapping to Oxigraph Operations

| Oxigraph Operation | tikv-client API | Notes |
|--------------------|-----------------|-------|
| Point lookup (SPO) | `txn.get(key)` | Direct mapping |
| Index scan (prefix) | `txn.scan(prefix..end, limit)` | Use prefix + increment for end bound |
| Insert triple | `txn.put(key, value)` per index | 3-6 puts per triple (SPO, POS, OSP + GSPO, GPOS, GOSP) |
| Delete triple | `txn.delete(key)` per index | Mirror of insert |
| Batch load | `txn.put()` in loop, then `txn.commit()` | Consider chunking for large batches |
| Transaction | `begin_optimistic()` / `commit()` | Optimistic preferred for read-heavy SPARQL workloads |

---

## 4. Compatibility Concerns

### Rust Edition and MSRV

- **tikv-client**: Likely uses Rust 2021 edition. MSRV is typically recent (1.70+).
- **Oxigraph**: Uses Rust 2021 edition with a relatively current MSRV.
- **Assessment:** No edition conflict expected. Both projects target modern Rust.

### grpcio Build Complexity

If `tikv-client` still depends on `grpcio` (C gRPC bindings):
- Requires `cmake`, `gcc`/`clang`, `libz-dev`, `libssl-dev` at build time.
- Significantly increases container image build time and size.
- **Mitigation:** Check if `tikv-client` supports a `tonic` feature flag to use pure-Rust gRPC.
  If not, ensure the Containerfile includes the necessary C build tools.

### Scan Pagination

The `scan()` method takes a `limit` parameter. For Oxigraph's iterator-based access patterns,
we need to implement cursor-based pagination:
- Scan with limit N, take last key, scan again from last_key+1.
- This is a standard pattern but adds complexity to the iterator adapter.

### Optimistic vs. Pessimistic Transactions

- **Optimistic** (default): Lower latency for read-heavy workloads; retries on conflict.
  Best for SPARQL queries.
- **Pessimistic**: Acquires locks upfront; better for write-heavy workloads.
  Consider for bulk SPARQL UPDATE operations.

### No Coprocessor in Client Crate

The `tikv-client` crate provides standard KV operations only. The Coprocessor pushdown
framework (mentioned in `05-tikv-backend.md` as the "paramount advantage") requires:
- Custom Coprocessor plugins written in Rust, compiled as shared libraries.
- Deployed to TiKV nodes via the Coprocessor plugin API.
- The client sends Coprocessor requests via raw gRPC, not through `tikv-client`.

**This is a significant implementation effort** and should be a Phase 2+ goal. Phase 1
should use standard KV operations through `tikv-client`.

---

## 5. Local Development Cluster Setup

### Option A: TiUP Playground (Recommended for Dev)

TiUP is PingCAP's official component manager. The playground command starts a local
TiKV + PD cluster with zero configuration:

```bash
# Install TiUP
curl --proto '=https' --tlsv1.2 -sSf https://tiup-mirrors.pingcap.com/install.sh | sh

# Start a minimal cluster (1 PD + 1 TiKV)
tiup playground --mode tikv-slim

# Start with specific component counts
tiup playground --mode tikv-slim --kv 3 --pd 3

# PD endpoint will be at 127.0.0.1:2379 by default
```

**Pros:** Fastest path to a running cluster, official tooling, handles binary downloads.
**Cons:** Linux/macOS only, no container isolation, not reproducible across team.

### Option B: Docker Compose (Recommended for CI)

A `docker-compose.yml` for a 3-node TiKV cluster:

```yaml
version: "3.8"

services:
  pd0:
    image: pingcap/pd:latest
    ports:
      - "2379:2379"
    command:
      - --name=pd0
      - --client-urls=http://0.0.0.0:2379
      - --peer-urls=http://0.0.0.0:2380
      - --advertise-client-urls=http://pd0:2379
      - --advertise-peer-urls=http://pd0:2380
      - --initial-cluster=pd0=http://pd0:2380

  tikv0:
    image: pingcap/tikv:latest
    depends_on:
      - pd0
    command:
      - --addr=0.0.0.0:20160
      - --advertise-addr=tikv0:20160
      - --pd-endpoints=pd0:2379

  tikv1:
    image: pingcap/tikv:latest
    depends_on:
      - pd0
    command:
      - --addr=0.0.0.0:20160
      - --advertise-addr=tikv1:20160
      - --pd-endpoints=pd0:2379

  tikv2:
    image: pingcap/tikv:latest
    depends_on:
      - pd0
    command:
      - --addr=0.0.0.0:20160
      - --advertise-addr=tikv2:20160
      - --pd-endpoints=pd0:2379
```

**Pros:** Reproducible, works in CI, close to production topology.
**Cons:** Heavier resource usage, images are large (~500 MB each).

### Option C: TiKV Operator on Kubernetes (Production / OpenShift)

The **TiDB Operator** (which manages TiKV as a component) is available for Kubernetes:

```bash
# Add PingCAP Helm repo
helm repo add pingcap https://charts.pingcap.org

# Install the operator
helm install tidb-operator pingcap/tidb-operator \
  --namespace tidb-admin --create-namespace

# Deploy a TiKV-only cluster (no TiDB SQL layer needed)
# Use a custom TidbCluster CR with tidb.replicas=0
kubectl apply -f tikv-cluster.yaml
```

A minimal TiKV-only `TidbCluster` custom resource:

```yaml
apiVersion: pingcap.com/v1alpha1
kind: TidbCluster
metadata:
  name: oxigraph-tikv
spec:
  version: v7.5.0
  pd:
    replicas: 3
    requests:
      storage: 10Gi
    storageClassName: gp3-csi  # adjust for your cluster
  tikv:
    replicas: 3
    requests:
      storage: 50Gi
    storageClassName: gp3-csi
    config:
      storage:
        block-cache:
          capacity: "4GB"
  tidb:
    replicas: 0  # We don't need the SQL layer
```

**Pros:** Production-grade, handles rolling upgrades, backup/restore, auto-scaling.
**Cons:** Requires a running K8s cluster, operator adds complexity.

**OpenShift note:** The TiDB Operator works on OpenShift but may require SecurityContextConstraints
(SCC) adjustments since TiKV containers may expect to run as specific UIDs.

### Option D: kind + TiKV Operator (Local K8s Dev)

```bash
# Create a kind cluster with sufficient resources
kind create cluster --name tikv-dev --config kind-config.yaml

# Install TiDB Operator + TiKV cluster as above
```

**Pros:** Tests the full K8s deployment path locally.
**Cons:** Resource-intensive (recommend 16 GB+ RAM), slow startup.

---

## 6. Recommendations

### Phase 1: MVP Integration

1. **Add `tikv-client` to Cargo.toml** with the latest 0.x version.
2. **Use `TransactionClient` with optimistic transactions** for all read/write operations.
3. **Implement scan pagination** as an async `Stream` adapter over `txn.scan()`.
4. **Use TiUP playground** for local development, Docker Compose for CI.
5. **Verify grpcio vs tonic** dependency -- if grpcio, add C build deps to Containerfile.

### Phase 2: Optimization

1. Implement Coprocessor plugins for prefix scan pushdown.
2. Evaluate pessimistic transactions for bulk write workloads.
3. Deploy TiKV Operator on OpenShift with production tuning.
4. Implement Region-aware scan batching.

### Potential Blockers

| Risk | Severity | Mitigation |
|------|----------|------------|
| `tikv-client` 0.x API instability | Medium | Pin exact version, wrap in adapter layer |
| grpcio C dependency build complexity | Medium | Check for tonic migration; add build deps |
| No streaming scan API (only limit-based) | Low | Implement cursor pagination adapter |
| Coprocessor plugin development complexity | High | Defer to Phase 2; standard KV is sufficient for MVP |
| TiKV Operator SCC issues on OpenShift | Medium | Pre-test on OpenShift sandbox; create custom SCCs |

### Action Items

- [ ] Verify exact latest version on crates.io
- [ ] Confirm tokio version compatibility with Oxigraph's dependency tree
- [ ] Confirm grpcio vs tonic status in latest release
- [ ] Write a minimal "hello TiKV" integration test using TiUP playground
- [ ] Create Docker Compose file for CI pipeline
- [ ] Test TiDB Operator on OpenShift Developer Sandbox
