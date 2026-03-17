# TiKV Operations Guide for Oxigraph Cloud-Native

> **Version**: 1.0 | **Date**: 2026-03-17
> **Covers**: Tasks 6.2 (Cluster Sizing), 6.3 (Region Tuning), 6.4 (Monitoring), 6.6 (Backup & Restore)

This guide provides production-grade operational guidance for running TiKV as the storage backend for Oxigraph. All recommendations are specific to Oxigraph's workload characteristics:

- **12 table prefixes** (0x00-0x0B) mapped from former RocksDB column families
- **Many small KV pairs**: quad index keys are 52-69 bytes with empty values
- **9 index copies per triple**: 6 named-graph indexes + 3 default-graph indexes
- **Heavy range scans**: SPARQL queries translate to bounded prefix scans
- **Read-heavy with bursty writes**: bulk loads followed by sustained query traffic

---

## 1. Cluster Sizing (Task 6.2)

### 1.1 Storage Estimation

Each RDF triple stored in Oxigraph produces approximately 9 index entries (6 named-graph quad indexes + 3 default-graph triple indexes). The `id2str` dictionary adds one entry per unique RDF term.

**Per-triple storage formula:**

```
raw_key_bytes_per_triple = 9 entries * avg_key_size
  - Named graph keys (spog/posg/ospg/gspo/gpos/gosp): 1 prefix + 4*17 = 69 bytes
  - Default graph keys (dspo/dpos/dosp):               1 prefix + 3*17 = 52 bytes
  = (6 * 69) + (3 * 52)
  = 414 + 156
  = 570 bytes of key data per triple (values are empty for index entries)

id2str_overhead = ~3 unique terms per triple * (1 prefix + 16 hash + ~40 avg string)
  = ~171 bytes per triple

total_raw_per_triple ~= 741 bytes
```

With TiKV overhead (MVCC versioning, Raft log, RocksDB metadata), apply a **1.5x amplification factor**:

```
effective_bytes_per_triple ~= 741 * 1.5 ~= 1,112 bytes ~= 1.1 KB
```

**Quick reference table:**

| Triple Count | Raw Data  | With Overhead (1.5x) | Estimated Region Count (96 MB) |
|-------------|-----------|----------------------|-------------------------------|
| 1 million   | ~0.7 GB   | ~1.1 GB              | ~12 Regions                   |
| 10 million  | ~7 GB     | ~11 GB               | ~115 Regions                  |
| 100 million | ~70 GB    | ~110 GB              | ~1,150 Regions                |
| 1 billion   | ~700 GB   | ~1.1 TB              | ~11,500 Regions               |

All of these are well within TiKV's comfortable range. Problems begin above ~100,000 Regions.

### 1.2 Cluster Profiles

#### Small (Development / CI)

For local development, CI pipelines, and functional testing.

| Component | Instances | CPU | Memory | Storage |
|-----------|-----------|-----|--------|---------|
| PD        | 1         | 0.5 | 512 Mi | 1 Gi (ephemeral OK) |
| TiKV      | 1         | 2   | 4 Gi   | 20 Gi SSD |
| Oxigraph  | 1         | 1   | 2 Gi   | -- (stateless) |

**Notes:**
- Single PD and TiKV -- no fault tolerance; acceptable for dev
- Replication factor set to 1 (`max-replicas: 1` in PD config)
- Can use `tiup playground` for zero-config local deployment
- Suitable for datasets up to ~5 million triples

```yaml
# PD config overrides for dev
replication:
  max-replicas: 1
schedule:
  leader-schedule-limit: 4
  region-schedule-limit: 4
```

#### Medium (Staging / Small Production)

For staging environments, integration testing, and small production datasets.

| Component | Instances | CPU | Memory | Storage |
|-----------|-----------|-----|--------|---------|
| PD        | 3         | 2   | 4 Gi   | 20 Gi SSD |
| TiKV      | 3         | 4   | 16 Gi  | 200 Gi SSD |
| Oxigraph  | 2         | 2   | 4 Gi   | -- (stateless) |

**Notes:**
- 3 PD nodes for Raft quorum -- tolerates 1 PD failure
- 3 TiKV nodes with default replication factor 3 -- tolerates 1 TiKV failure
- Block cache: allocate ~40% of TiKV memory = ~6.4 Gi per node
- Suitable for datasets up to ~100 million triples
- Oxigraph stateless replicas behind a Service for query load balancing

#### Large (Production)

For production workloads with large datasets and high query throughput.

| Component | Instances | CPU   | Memory | Storage |
|-----------|-----------|-------|--------|---------|
| PD        | 3         | 4     | 8 Gi   | 50 Gi NVMe SSD |
| TiKV      | 5+        | 8     | 32 Gi  | 500 Gi - 2 Ti NVMe SSD |
| Oxigraph  | 3+        | 4     | 8 Gi   | -- (stateless) |

**Notes:**
- 5+ TiKV nodes for better Region distribution and parallelism
- NVMe SSD mandatory -- Raft WAL and RocksDB benefit enormously from low-latency I/O
- Block cache: ~12.8 Gi per node (40% of 32 Gi)
- Enable Titan for `id2str` entries where values exceed 1 KB (long IRIs/literals)
- Suitable for datasets of 100 million to 1+ billion triples
- Horizontal scaling: add TiKV nodes as needed; PD handles Region rebalancing automatically

### 1.3 Memory Sizing: Block Cache

The TiKV block cache should hold the **working set** -- the set of keys and values actively accessed by queries. For Oxigraph:

- The `id2str` table (prefix 0x01) is accessed on every query result to resolve hashes to strings
- The most-queried index table (often `dspo` or `dpos`) should fit in cache

**Block cache sizing formula:**

```
block_cache_size = tikv_memory * 0.40

# Verify working set fits:
working_set ~= id2str_size + primary_index_size
id2str_size ~= unique_terms * ~57 bytes  (1 prefix + 16 hash + ~40 string avg)
primary_index_size ~= triples * 52 bytes  (one index, e.g., dspo)
```

For 100M triples with ~50M unique terms:
- id2str: ~2.85 GB
- dspo: ~5.2 GB
- Working set: ~8 GB

With 3 TiKV nodes at 16 Gi each, total block cache = 3 * 6.4 Gi = 19.2 Gi -- working set fits comfortably.

**tikv.toml configuration:**

```toml
[storage.block-cache]
# Use shared block cache across all RocksDB column families
shared = true
# Set to 40% of total memory; adjust based on monitoring
capacity = "6.4GB"
```

---

## 2. Region Tuning (Task 6.3)

### 2.1 Default Region Size Analysis

TiKV's default Region size is **96 MB**. Each Region is a contiguous range of keys managed by a Raft group. For Oxigraph:

- A 69-byte quad key with empty value occupies ~69 bytes raw (plus MVCC overhead, ~100 bytes effective)
- At 96 MB per Region, each Region holds approximately **1 million quad index entries**
- Each table prefix defines its own key range, so Regions do not span tables

**Impact of 96 MB default:**

| Dataset Size | Total Index Entries | Estimated Regions | Assessment |
|-------------|--------------------|--------------------|------------|
| 1M triples  | ~9M entries        | ~12                | Very few Regions -- good |
| 100M triples| ~900M entries      | ~1,150             | Comfortable |
| 1B triples  | ~9B entries        | ~11,500            | Still fine |

**Recommendation:** Keep the default 96 MB Region size. Oxigraph's small keys mean each Region holds many entries, keeping Region count low. Changing Region size is disruptive (requires region split/merge operations across the cluster).

### 2.2 When to Consider Larger Regions

If the dataset exceeds **500 million triples** and Region count approaches 50,000+, consider increasing Region size to 192 MB or 256 MB to reduce Raftstore heartbeat overhead:

```toml
[coprocessor]
# Increase from default 96 MB to 192 MB
region-max-size = "192MB"
region-split-size = "144MB"
```

**Warning:** Larger Regions increase recovery time (each Region recovery replays more Raft log) and reduce parallelism (fewer Regions means fewer parallel scan targets). Only increase if Raftstore CPU is the bottleneck.

### 2.3 Hibernate Region

Hibernate Region suppresses Raft heartbeats for Regions with no recent read or write activity. This is critical for Oxigraph because:

- The `id2str` table (prefix 0x01) is written during ingestion but then becomes read-only
- The `graphs` table (prefix 0x0B) rarely changes after initial setup
- Named-graph indexes (prefixes 0x05-0x07) may be idle if queries primarily target the default graph

**Enable Hibernate Region:**

```toml
[raftstore]
hibernate-regions = true

# Time before a Region enters hibernation (default: 10 minutes)
peer-stale-state-check-interval = "10m"
```

**Expected impact:** For a 100M triple dataset with ~1,150 Regions, if 60% of Regions are idle (dictionary, unused graph indexes), heartbeat traffic drops from ~1,150 to ~460 active Region heartbeats. This directly reduces Raftstore CPU.

### 2.4 Region Merge

Small Regions waste resources (each Region has Raft overhead regardless of data size). After bulk deletes or for sparsely populated index tables, configure aggressive Region merging:

```toml
[schedule]
# Merge Regions smaller than this threshold (default: 20 MB)
max-merge-region-size = 20

# Maximum number of keys in a Region eligible for merge (0 = unlimited)
max-merge-region-keys = 200000

# How frequently to check for merge candidates
merge-schedule-limit = 8
```

**Oxigraph-specific consideration:** The `graphs` table (prefix 0x0B) and `default` table (prefix 0x00) are typically very small (a few KB). These will naturally merge with adjacent Regions. Since table prefixes ensure key isolation, cross-table merges do not cause correctness issues -- TiKV Regions are purely a physical partitioning concern.

### 2.5 Raftstore Concurrency

The Raftstore thread pool processes Raft messages (heartbeats, proposals, log replication). For Oxigraph's write-heavy bulk load followed by read-heavy queries:

```toml
[raftstore]
# Number of Raftstore threads (default: 2)
# Increase for nodes with many Regions or high write throughput
store-pool-size = 4

# Raft apply thread pool (applies committed log entries to state machine)
apply-pool-size = 4

# Limit concurrent Region splits to prevent thundering herd during bulk load
split-region-check-tick-interval = "10s"
region-split-check-diff = "48MB"
```

**Tuning guidance by cluster size:**

| TiKV CPUs | store-pool-size | apply-pool-size |
|-----------|-----------------|-----------------|
| 2 (dev)   | 2 (default)     | 2 (default)     |
| 4 (staging)| 3              | 3               |
| 8 (prod)  | 4               | 4               |
| 16+       | 6               | 6               |

### 2.6 Complete tikv.toml Reference

Below is a consolidated `tikv.toml` for the **medium (staging)** profile:

```toml
# ============================================================
# TiKV Configuration for Oxigraph Cloud-Native (Staging)
# ============================================================

[server]
# gRPC concurrency (number of threads handling RPCs)
grpc-concurrency = 4
# Maximum message size (important for large scan results)
grpc-raft-conn-num = 1
max-grpc-send-msg-len = 10485760   # 10 MB

[storage]
# Use the transactional API (required for Oxigraph's MVCC)
# Scheduler threads for processing write requests
scheduler-worker-pool-size = 4

[storage.block-cache]
shared = true
capacity = "6.4GB"

[raftstore]
hibernate-regions = true
store-pool-size = 3
apply-pool-size = 3
# Check for split less aggressively (Oxigraph keys are small)
split-region-check-tick-interval = "10s"
region-split-check-diff = "48MB"
peer-stale-state-check-interval = "10m"

[coprocessor]
# Keep default Region size for Oxigraph's many-small-keys pattern
region-max-size = "96MB"
region-split-size = "72MB"

[rocksdb]
# RocksDB write buffer for the local engine
max-background-jobs = 4
max-sub-compactions = 2

[rocksdb.defaultcf]
# Optimize for Oxigraph's small KV pairs
block-size = "16KB"
# Bloom filter for point lookups (id2str hash lookups)
bloom-filter-bits-per-key = 10
whole-key-filtering = true

[rocksdb.writecf]
block-size = "16KB"

[readpool.coprocessor]
# Thread pool for Coprocessor requests (range scans)
use-unified-pool = true

[readpool.unified]
max-thread-count = 4
```

---

## 3. Monitoring (Task 6.4)

### 3.1 Prometheus + Grafana Setup

TiKV exposes metrics on port **20180** by default (`/metrics` endpoint). PD exposes on port **2379** (`/metrics`).

**ServiceMonitor for OpenShift/Prometheus Operator:**

```yaml
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: tikv-monitor
  namespace: oxigraph
  labels:
    app: tikv
spec:
  selector:
    matchLabels:
      app: tikv
  endpoints:
    - port: metrics
      interval: 15s
      path: /metrics
---
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: pd-monitor
  namespace: oxigraph
  labels:
    app: pd
spec:
  selector:
    matchLabels:
      app: pd
  endpoints:
    - port: metrics
      interval: 15s
      path: /metrics
---
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: oxigraph-monitor
  namespace: oxigraph
  labels:
    app: oxigraph
spec:
  selector:
    matchLabels:
      app: oxigraph
  endpoints:
    - port: metrics
      interval: 15s
      path: /metrics
```

### 3.2 Key TiKV Metrics

#### Throughput & Latency

| Metric | PromQL | Description | Target |
|--------|--------|-------------|--------|
| TiKV QPS (read) | `rate(tikv_grpc_msg_duration_seconds_count{type="kv_get"}[5m])` | Point read operations per second | Baseline varies |
| TiKV QPS (scan) | `rate(tikv_grpc_msg_duration_seconds_count{type="coprocessor"}[5m])` | Coprocessor (range scan) ops/s | Monitor for saturation |
| Read latency P99 | `histogram_quantile(0.99, rate(tikv_grpc_msg_duration_seconds_bucket{type="kv_get"}[5m]))` | Point read P99 latency | < 10 ms |
| Scan latency P99 | `histogram_quantile(0.99, rate(tikv_grpc_msg_duration_seconds_bucket{type="coprocessor"}[5m]))` | Range scan P99 latency | < 50 ms |
| Write latency P99 | `histogram_quantile(0.99, rate(tikv_grpc_msg_duration_seconds_bucket{type="kv_prewrite"}[5m]))` | Transaction prewrite P99 | < 20 ms |

#### Raftstore Health

| Metric | PromQL | Description | Alert Threshold |
|--------|--------|-------------|-----------------|
| Raftstore CPU | `rate(tikv_thread_cpu_seconds_total{name=~"raftstore.*"}[1m])` | CPU used by Raft processing | > 80% of pool capacity |
| Raft proposals/s | `rate(tikv_raftstore_proposal_total[5m])` | Write proposals per second | Sudden spikes |
| Raft log lag | `tikv_raftstore_log_lag` | Log entries behind on followers | > 1000 entries |
| Apply wait | `histogram_quantile(0.99, rate(tikv_raftstore_apply_wait_time_duration_secs_bucket[5m]))` | Time waiting to apply committed entries | > 100 ms |

#### Region Health

| Metric | PromQL | Description | Alert Threshold |
|--------|--------|-------------|-----------------|
| Region count | `tikv_raftstore_region_count{type="region"}` | Total Regions per node | > 30,000 per node |
| Leader count | `tikv_raftstore_region_count{type="leader"}` | Leader Regions per node | Imbalance > 20% across nodes |
| Region size | `histogram_quantile(0.5, tikv_region_written_bytes_bucket)` | Median Region write throughput | Monitor for hot Regions |
| Approximate keys | `tikv_engine_estimate_num_keys{cf="default"}` | Estimated key count per CF | Capacity planning |

#### Storage & Resources

| Metric | PromQL | Description | Alert Threshold |
|--------|--------|-------------|-----------------|
| Disk usage | `tikv_engine_size_bytes` | Total engine data size | > 80% of volume capacity |
| Block cache hit rate | `tikv_engine_block_cache_hit / (tikv_engine_block_cache_hit + tikv_engine_block_cache_miss)` | Cache effectiveness | < 90% indicates undersized cache |
| Compaction pending bytes | `tikv_engine_pending_compaction_bytes` | Pending compaction work | > 1 GB sustained |
| Write stall | `tikv_engine_write_stall` | RocksDB write stalls | Any occurrence |

### 3.3 Custom Oxigraph Metrics

Instrument the Oxigraph server with the following application-level metrics (exposed at `/metrics` in Prometheus format):

```rust
use prometheus::{
    register_histogram_vec, register_gauge, register_counter_vec,
    register_histogram, HistogramVec, Gauge, CounterVec, Histogram,
};
use lazy_static::lazy_static;

lazy_static! {
    /// SPARQL query latency by operation type (SELECT, CONSTRUCT, ASK, DESCRIBE)
    pub static ref QUERY_DURATION: HistogramVec = register_histogram_vec!(
        "oxigraph_query_duration_seconds",
        "SPARQL query execution time in seconds",
        &["operation"],
        vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
    ).unwrap();

    /// Currently active TiKV transactions
    pub static ref ACTIVE_TRANSACTIONS: Gauge = register_gauge!(
        "oxigraph_active_transactions",
        "Number of currently active TiKV transactions"
    ).unwrap();

    /// SHACL validation operations by result (pass, fail, error)
    pub static ref SHACL_VALIDATIONS: CounterVec = register_counter_vec!(
        "oxigraph_shacl_validations_total",
        "Total SHACL validation operations",
        &["result"]
    ).unwrap();

    /// SHACL validation duration
    pub static ref SHACL_DURATION: Histogram = register_histogram!(
        "oxigraph_shacl_duration_seconds",
        "SHACL validation execution time in seconds",
        vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]
    ).unwrap();

    /// TiKV scan batch sizes (keys returned per scan request)
    pub static ref SCAN_BATCH_SIZE: HistogramVec = register_histogram_vec!(
        "oxigraph_tikv_scan_batch_size",
        "Number of keys returned per TiKV scan batch",
        &["table"],
        vec![1.0, 10.0, 50.0, 100.0, 500.0, 1000.0, 5000.0, 10000.0]
    ).unwrap();

    /// Triple count by operation (insert, delete)
    pub static ref TRIPLE_OPERATIONS: CounterVec = register_counter_vec!(
        "oxigraph_triple_operations_total",
        "Total triple insert/delete operations",
        &["operation"]
    ).unwrap();

    /// SPARQL UPDATE (write) latency
    pub static ref UPDATE_DURATION: Histogram = register_histogram!(
        "oxigraph_update_duration_seconds",
        "SPARQL UPDATE execution time in seconds",
        vec![0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0]
    ).unwrap();

    /// TiKV connection pool status
    pub static ref TIKV_POOL_ACTIVE: Gauge = register_gauge!(
        "oxigraph_tikv_pool_active_connections",
        "Number of active connections in the TiKV connection pool"
    ).unwrap();
}
```

### 3.4 Alert Rules

```yaml
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: oxigraph-tikv-alerts
  namespace: oxigraph
spec:
  groups:
    - name: tikv.critical
      rules:
        # Raftstore CPU saturation
        - alert: TiKVRaftstoreCPUHigh
          expr: >
            sum(rate(tikv_thread_cpu_seconds_total{name=~"raftstore.*"}[5m])) by (instance)
            > 0.8 * count(tikv_thread_cpu_seconds_total{name=~"raftstore.*"}) by (instance)
          for: 5m
          labels:
            severity: critical
          annotations:
            summary: "TiKV Raftstore CPU > 80% on {{ $labels.instance }}"
            description: >
              Raftstore threads are saturated. This causes Raft proposal delays
              and can lead to Region leader transfer failures. Consider increasing
              store-pool-size or adding TiKV nodes.

        # Region count explosion
        - alert: TiKVRegionCountHigh
          expr: tikv_raftstore_region_count{type="region"} > 30000
          for: 10m
          labels:
            severity: warning
          annotations:
            summary: "TiKV Region count > 30,000 on {{ $labels.instance }}"
            description: >
              High Region count increases heartbeat overhead. Consider increasing
              region-max-size or enabling more aggressive Region merge.

        # P99 read latency
        - alert: TiKVReadLatencyHigh
          expr: >
            histogram_quantile(0.99,
              rate(tikv_grpc_msg_duration_seconds_bucket{type="kv_get"}[5m])
            ) > 0.05
          for: 5m
          labels:
            severity: warning
          annotations:
            summary: "TiKV read P99 latency > 50ms on {{ $labels.instance }}"
            description: >
              Point read latency is elevated. Check block cache hit rate,
              disk I/O, and compaction status.

        # P99 scan latency
        - alert: TiKVScanLatencyHigh
          expr: >
            histogram_quantile(0.99,
              rate(tikv_grpc_msg_duration_seconds_bucket{type="coprocessor"}[5m])
            ) > 0.5
          for: 5m
          labels:
            severity: warning
          annotations:
            summary: "TiKV Coprocessor (scan) P99 latency > 500ms on {{ $labels.instance }}"
            description: >
              Range scan latency is high. This directly impacts SPARQL query
              performance. Check for hot Regions or insufficient Coprocessor threads.

        # Disk space
        - alert: TiKVDiskSpaceLow
          expr: >
            (tikv_engine_size_bytes / tikv_store_size_bytes) > 0.80
          for: 10m
          labels:
            severity: critical
          annotations:
            summary: "TiKV disk usage > 80% on {{ $labels.instance }}"
            description: >
              Disk space is running low. TiKV will refuse writes when disk is
              full. Expand storage or add nodes immediately.

        # Write stall
        - alert: TiKVWriteStall
          expr: increase(tikv_engine_write_stall[5m]) > 0
          for: 1m
          labels:
            severity: critical
          annotations:
            summary: "RocksDB write stall detected on {{ $labels.instance }}"
            description: >
              RocksDB is stalling writes due to compaction backlog. This blocks
              all write transactions. Check compaction pending bytes and I/O throughput.

        # Leader imbalance
        - alert: TiKVLeaderImbalance
          expr: >
            (max(tikv_raftstore_region_count{type="leader"})
             - min(tikv_raftstore_region_count{type="leader"}))
            / max(tikv_raftstore_region_count{type="leader"}) > 0.2
          for: 15m
          labels:
            severity: warning
          annotations:
            summary: "TiKV leader distribution imbalanced by > 20%"
            description: >
              Leaders are unevenly distributed across TiKV nodes. PD should
              rebalance automatically; check PD scheduler status.

    - name: oxigraph.application
      rules:
        # Oxigraph query latency
        - alert: OxigraphQueryLatencyHigh
          expr: >
            histogram_quantile(0.99,
              rate(oxigraph_query_duration_seconds_bucket[5m])
            ) > 5.0
          for: 5m
          labels:
            severity: warning
          annotations:
            summary: "Oxigraph SPARQL query P99 latency > 5s"

        # SHACL validation failures spike
        - alert: OxigraphSHACLFailureRate
          expr: >
            rate(oxigraph_shacl_validations_total{result="fail"}[5m])
            / rate(oxigraph_shacl_validations_total[5m]) > 0.5
          for: 10m
          labels:
            severity: warning
          annotations:
            summary: "SHACL validation failure rate > 50%"
            description: >
              More than half of incoming data is failing SHACL validation.
              Check if shapes are too restrictive or data quality has degraded.

        # Block cache hit rate
        - alert: TiKVBlockCacheHitRateLow
          expr: >
            tikv_engine_block_cache_hit
            / (tikv_engine_block_cache_hit + tikv_engine_block_cache_miss) < 0.90
          for: 15m
          labels:
            severity: warning
          annotations:
            summary: "TiKV block cache hit rate < 90%"
            description: >
              The working set does not fit in the block cache. Increase
              storage.block-cache.capacity or add more TiKV nodes.
```

### 3.5 Grafana Dashboard Layout

The Oxigraph-TiKV Grafana dashboard should be organized into the following rows:

**Row 1: Cluster Overview**
- Total triple count (from `oxigraph_triple_operations_total`)
- SPARQL QPS (from `oxigraph_query_duration_seconds_count`)
- Active transactions gauge
- Cluster health status (all nodes up/down)

**Row 2: Query Performance**
- SPARQL query latency heatmap (P50, P95, P99)
- Query rate by operation type (SELECT, CONSTRUCT, ASK, UPDATE)
- SHACL validation rate and duration
- Scan batch size distribution

**Row 3: TiKV Throughput**
- TiKV read/write QPS per node
- Coprocessor request rate
- gRPC message duration (P99) per type
- Transaction commit rate and latency

**Row 4: Raftstore & Regions**
- Raftstore CPU utilization per node
- Region count per node
- Leader distribution across nodes
- Raft proposal rate
- Region heartbeat rate (should decrease with Hibernate Region)

**Row 5: Storage Engine**
- Block cache hit rate
- Disk usage per node (used vs. capacity)
- Compaction pending bytes
- Write stall count
- RocksDB write throughput (bytes/s)

**Row 6: Resource Utilization**
- Node CPU utilization
- Node memory utilization
- Network I/O (inter-node Raft traffic)
- Disk I/O latency and throughput

**Importing TiKV dashboards:**
The TiKV project provides official Grafana dashboards. Import them from the TiKV repository:
- `tikv-summary` dashboard (Grafana ID: available from PingCAP)
- `tikv-details` dashboard
- `pd-summary` dashboard

Then add a custom "Oxigraph Application" dashboard for the application-level metrics.

---

## 4. Backup & Restore (Task 6.6)

### 4.1 TiKV BR (Backup & Restore) Overview

TiKV BR (`tikv-br` / `br`) performs distributed, consistent backups by:

1. Requesting a consistent snapshot timestamp from PD
2. Each TiKV node exports its Region data at that timestamp in parallel
3. SST files are written directly to the backup destination (S3, GCS, or local storage)
4. Metadata file records the backup timestamp and Region mapping

This produces a **point-in-time consistent snapshot** without stopping writes.

### 4.2 Backup to S3-Compatible Object Storage

**Prerequisites:**
- S3-compatible endpoint (AWS S3, MinIO, Ceph RGW, or OpenShift Data Foundation)
- A bucket dedicated to backups (e.g., `oxigraph-backups`)
- IAM credentials with `s3:PutObject`, `s3:GetObject`, `s3:ListBucket`, `s3:DeleteObject`

**Full backup command:**

```bash
tikv-br backup full \
  --pd "pd-0.pd:2379,pd-1.pd:2379,pd-2.pd:2379" \
  --storage "s3://oxigraph-backups/full/$(date +%Y%m%d-%H%M%S)" \
  --s3.endpoint "https://s3.example.com" \
  --s3.region "us-east-1" \
  --ratelimit 128        \
  --concurrency 4        \
  --log-file /var/log/tikv-br/backup.log
```

**Parameters explained:**
- `--ratelimit 128`: Limit to 128 MB/s per TiKV node to avoid impacting online queries
- `--concurrency 4`: Number of concurrent backup workers per TiKV node
- `--checksum=true` (default): Verifies data integrity after backup

**Backup with key range filtering (single table):**

To back up only specific Oxigraph tables (e.g., just the `id2str` dictionary):

```bash
tikv-br backup raw \
  --pd "pd-0.pd:2379" \
  --storage "s3://oxigraph-backups/id2str/$(date +%Y%m%d)" \
  --start "0x01" \
  --end "0x02" \
  --cf default
```

### 4.3 Scheduled Backup via CronJob

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: tikv-backup
  namespace: oxigraph
spec:
  schedule: "0 2 * * *"    # Daily at 02:00 UTC
  concurrencyPolicy: Forbid
  successfulJobsHistoryLimit: 7
  failedJobsHistoryLimit: 3
  jobTemplate:
    spec:
      backoffLimit: 2
      activeDeadlineSeconds: 7200    # 2 hour timeout
      template:
        metadata:
          labels:
            app: tikv-backup
        spec:
          restartPolicy: OnFailure
          containers:
            - name: tikv-br
              image: pingcap/br:v7.5.0
              command:
                - /bin/sh
                - -c
                - |
                  set -euo pipefail
                  BACKUP_TS=$(date +%Y%m%d-%H%M%S)
                  BACKUP_PATH="s3://oxigraph-backups/scheduled/${BACKUP_TS}"

                  echo "Starting backup to ${BACKUP_PATH}"
                  /br backup full \
                    --pd "${PD_ENDPOINTS}" \
                    --storage "${BACKUP_PATH}" \
                    --s3.endpoint "${S3_ENDPOINT}" \
                    --s3.region "${S3_REGION}" \
                    --ratelimit 128 \
                    --concurrency 4 \
                    --log-file /tmp/backup.log

                  echo "Backup completed successfully"

                  # Clean up backups older than 30 days
                  # (Implement via a separate cleanup job or S3 lifecycle policy)
              env:
                - name: PD_ENDPOINTS
                  value: "pd-0.pd:2379,pd-1.pd:2379,pd-2.pd:2379"
                - name: S3_ENDPOINT
                  valueFrom:
                    secretKeyRef:
                      name: tikv-backup-s3
                      key: endpoint
                - name: S3_REGION
                  valueFrom:
                    secretKeyRef:
                      name: tikv-backup-s3
                      key: region
                - name: AWS_ACCESS_KEY_ID
                  valueFrom:
                    secretKeyRef:
                      name: tikv-backup-s3
                      key: access-key
                - name: AWS_SECRET_ACCESS_KEY
                  valueFrom:
                    secretKeyRef:
                      name: tikv-backup-s3
                      key: secret-key
              resources:
                requests:
                  cpu: "500m"
                  memory: "1Gi"
                limits:
                  cpu: "2"
                  memory: "4Gi"
---
apiVersion: v1
kind: Secret
metadata:
  name: tikv-backup-s3
  namespace: oxigraph
type: Opaque
stringData:
  endpoint: "https://s3.example.com"
  region: "us-east-1"
  access-key: "REPLACE_WITH_ACCESS_KEY"
  secret-key: "REPLACE_WITH_SECRET_KEY"
```

**S3 lifecycle policy for automatic cleanup:**

Configure the S3 bucket with a lifecycle rule to expire old backups:

```json
{
  "Rules": [
    {
      "ID": "expire-old-backups",
      "Status": "Enabled",
      "Filter": { "Prefix": "scheduled/" },
      "Expiration": { "Days": 30 }
    }
  ]
}
```

### 4.4 Restore Procedure

#### Restore to a New Cluster

**Step 1: Deploy a fresh TiKV cluster** (same or compatible version) with no data.

**Step 2: Run the restore command:**

```bash
tikv-br restore full \
  --pd "new-pd-0.pd:2379,new-pd-1.pd:2379,new-pd-2.pd:2379" \
  --storage "s3://oxigraph-backups/scheduled/20260317-020000" \
  --s3.endpoint "https://s3.example.com" \
  --s3.region "us-east-1" \
  --ratelimit 256 \
  --concurrency 8 \
  --log-file /var/log/tikv-br/restore.log
```

**Step 3: Verify the restore:**

```bash
# Check cluster health
tikv-ctl --pd "new-pd-0.pd:2379" store

# Verify Region count matches backup
tikv-ctl --pd "new-pd-0.pd:2379" region

# Run Oxigraph health check
curl http://oxigraph:7878/health

# Run a SPARQL query to verify data
curl -X POST http://oxigraph:7878/query \
  -H "Content-Type: application/sparql-query" \
  -d "SELECT (COUNT(*) AS ?count) WHERE { ?s ?p ?o }"
```

**Step 4: Update Oxigraph configuration** to point to the new PD endpoints and restart.

#### Restore to the Same Cluster (Disaster Recovery)

If the existing cluster is corrupted or partially failed:

1. **Stop all Oxigraph instances** to prevent writes during recovery
2. **Scale TiKV StatefulSet to 0** to stop all TiKV nodes
3. **Delete PersistentVolumeClaims** for TiKV data (this destroys existing data)
4. **Scale TiKV StatefulSet back up** -- fresh nodes with empty storage
5. **Run the restore command** as above
6. **Wait for PD to rebalance Regions** across the restored cluster
7. **Restart Oxigraph instances**

### 4.5 Point-in-Time Recovery (PITR)

TiKV supports log backup for point-in-time recovery. This captures incremental changes (Raft log entries) between full backups:

**Start log backup (continuous):**

```bash
tikv-br log start \
  --pd "pd-0.pd:2379" \
  --storage "s3://oxigraph-backups/log-backup" \
  --s3.endpoint "https://s3.example.com" \
  --s3.region "us-east-1"
```

**Restore to a specific point in time:**

```bash
# First restore the most recent full backup before the target time
tikv-br restore full \
  --pd "new-pd-0.pd:2379" \
  --storage "s3://oxigraph-backups/scheduled/20260316-020000"

# Then apply log backup up to the target timestamp
tikv-br restore point \
  --pd "new-pd-0.pd:2379" \
  --full-backup-storage "s3://oxigraph-backups/scheduled/20260316-020000" \
  --storage "s3://oxigraph-backups/log-backup" \
  --restored-ts "2026-03-17 01:30:00+00:00" \
  --s3.endpoint "https://s3.example.com" \
  --s3.region "us-east-1"
```

**PITR CronJob for log backup health check:**

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: tikv-log-backup-check
  namespace: oxigraph
spec:
  schedule: "*/30 * * * *"    # Every 30 minutes
  jobTemplate:
    spec:
      template:
        spec:
          restartPolicy: OnFailure
          containers:
            - name: check
              image: pingcap/br:v7.5.0
              command:
                - /bin/sh
                - -c
                - |
                  /br log status \
                    --pd "${PD_ENDPOINTS}" \
                    --storage "s3://oxigraph-backups/log-backup"
              env:
                - name: PD_ENDPOINTS
                  value: "pd-0.pd:2379,pd-1.pd:2379,pd-2.pd:2379"
```

### 4.6 Backup Strategy Summary

| Backup Type | Frequency | Retention | RPO | RTO |
|-------------|-----------|-----------|-----|-----|
| Full backup | Daily at 02:00 UTC | 30 days | 24 hours (without PITR) | ~1 hour (depends on data size) |
| Log backup (PITR) | Continuous | 7 days | Minutes | ~1-2 hours |
| Pre-upgrade snapshot | Before each upgrade | Until next successful upgrade | N/A | ~30 minutes |

**RPO** (Recovery Point Objective): Maximum acceptable data loss.
**RTO** (Recovery Time Objective): Maximum acceptable downtime.

With PITR enabled, RPO drops to minutes (limited by log backup flush interval). Without PITR, RPO equals the interval between full backups (24 hours in the default schedule).

---

## 5. Security Configuration (SEC-03, SEC-13)

### 5.1 Transparent Data Encryption (TDE)

TiKV supports Transparent Data Encryption via RocksDB's encryption layer. TDE encrypts all data at rest -- SST files, WAL, and MANIFEST -- using AES-256-CTR. This addresses SEC-03 from the security compliance assessment.

#### 5.1.1 Generate the Master Key

The master key is a 256-bit (32-byte) cryptographic key used to encrypt the per-file data keys. Generate it securely:

```bash
# Generate a 32-byte random key (AES-256)
openssl rand -out master.key 32

# Verify the key length (must be exactly 32 bytes)
wc -c master.key
# Expected output: 32 master.key

# Set restrictive permissions
chmod 600 master.key
```

In Kubernetes, store the master key as a Secret:

```bash
kubectl create secret generic tikv-encryption-key \
  --namespace oxigraph \
  --from-file=master.key=master.key

# Verify
kubectl get secret tikv-encryption-key -n oxigraph -o jsonpath='{.data.master\.key}' | base64 -d | wc -c
```

Mount the Secret into each TiKV pod at `/etc/tikv/encryption/`:

```yaml
# In the TiKV StatefulSet spec
volumes:
  - name: encryption-key
    secret:
      secretName: tikv-encryption-key
      defaultMode: 0400
containers:
  - name: tikv
    volumeMounts:
      - name: encryption-key
        mountPath: /etc/tikv/encryption
        readOnly: true
```

#### 5.1.2 tikv.toml TDE Configuration

Add the following to `tikv.toml`:

```toml
[security.encryption]
# Encryption method for data files (SST, WAL, MANIFEST)
# Options: "plaintext" (disabled), "aes128-ctr", "aes192-ctr", "aes256-ctr"
data-encryption-method = "aes256-ctr"

# Interval for automatic data key rotation (default: 7 days)
# Each SST file uses its own data key; rotation applies to newly created files
data-key-rotation-period = "7d"

[security.encryption.master-key]
# Master key type: "file" (local file) or "kms" (cloud KMS)
type = "file"
path = "/etc/tikv/encryption/master.key"

# For AWS KMS (production alternative):
# type = "kms"
# key-id = "arn:aws:kms:us-east-1:123456789:key/abcd-1234-..."
# region = "us-east-1"
# endpoint = ""   # Leave empty for default AWS endpoint

[security.encryption.previous-master-key]
# Required only during master key rotation; otherwise leave commented out
# type = "file"
# path = "/etc/tikv/encryption/previous-master.key"
```

#### 5.1.3 Key Rotation Procedure

**Data key rotation** happens automatically based on `data-key-rotation-period`. New SST files use a fresh data key; old files retain their original data key until compaction rewrites them.

**Master key rotation** requires a rolling restart:

1. Copy the current master key to a backup location:
   ```bash
   cp master.key previous-master.key
   ```

2. Generate a new master key:
   ```bash
   openssl rand -out master.key 32
   ```

3. Update the Kubernetes Secret with both keys:
   ```bash
   kubectl create secret generic tikv-encryption-key \
     --namespace oxigraph \
     --from-file=master.key=master.key \
     --from-file=previous-master.key=previous-master.key \
     --dry-run=client -o yaml | kubectl apply -f -
   ```

4. Update `tikv.toml` to reference the previous master key:
   ```toml
   [security.encryption.master-key]
   type = "file"
   path = "/etc/tikv/encryption/master.key"

   [security.encryption.previous-master-key]
   type = "file"
   path = "/etc/tikv/encryption/previous-master.key"
   ```

5. Perform a rolling restart of TiKV pods:
   ```bash
   kubectl rollout restart statefulset tikv -n oxigraph
   ```

6. After all pods have restarted and re-encrypted with the new master key, remove the `previous-master-key` section from `tikv.toml` and delete the old key from the Secret.

#### 5.1.4 Verify TDE is Active

Check TiKV logs after startup for encryption initialization:

```bash
kubectl logs tikv-0 -n oxigraph | grep -i encryption
# Expected output includes:
#   "encryption is enabled" or "encryption method: aes256-ctr"
```

Query the TiKV encryption status via the status endpoint:

```bash
# Port-forward to a TiKV node
kubectl port-forward tikv-0 20180:20180 -n oxigraph

# Check encryption info (available in TiKV 6.1+)
curl -s http://localhost:20180/debug/pprof/tikv | grep encryption
```

Verify that SST files on disk are not readable as plaintext:

```bash
kubectl exec tikv-0 -n oxigraph -- \
  strings /var/lib/tikv/data/db/000042.sst | head -5
# Should show binary/encrypted data, not readable key-value content
```

### 5.2 mTLS Between Oxigraph and TiKV (SEC-13)

Mutual TLS ensures that both Oxigraph and TiKV authenticate each other and that all gRPC traffic is encrypted in transit. This addresses SEC-13 from the security compliance assessment.

#### 5.2.1 Certificate Architecture

The mTLS setup requires:
- A **CA certificate** (shared trust root)
- **TiKV server certificates** (one per TiKV node, or a wildcard for the StatefulSet)
- **PD server certificates** (one per PD node, or a wildcard)
- **Client certificate** for Oxigraph to authenticate to TiKV/PD

In Kubernetes with cert-manager, a single self-signed CA Issuer generates all certificates.

#### 5.2.2 TiKV Server TLS Configuration (tikv.toml)

```toml
[security]
# Path to the CA certificate (used to verify client certificates)
ca-path = "/etc/tikv/tls/ca.pem"
# TiKV server certificate and key
cert-path = "/etc/tikv/tls/server.pem"
key-path = "/etc/tikv/tls/server-key.pem"

# Optional: require client certificates (enables mutual TLS)
# When set, only clients presenting a certificate signed by ca-path are allowed
# cert-allowed-cn = ["oxigraph-client"]
```

#### 5.2.3 PD Server TLS Configuration (pd.toml)

```toml
[security]
# CA certificate for verifying TiKV and client certificates
cacert-path = "/etc/pd/tls/ca.pem"
# PD server certificate and key
cert-path = "/etc/pd/tls/server.pem"
key-path = "/etc/pd/tls/server-key.pem"

[security.encryption]
# PD also supports encryption for metadata stored in etcd
# data-encryption-method = "aes256-ctr"
```

#### 5.2.4 cert-manager Certificate Resources

The following cert-manager resources create the CA and issue certificates for all components. These are provided as a Helm template in `helm/oxigraph-cloud/templates/certificate.yaml` (gated by `tls.enabled`).

```yaml
# Self-signed root CA
apiVersion: cert-manager.io/v1
kind: Issuer
metadata:
  name: tikv-selfsigned-issuer
  namespace: oxigraph
spec:
  selfSigned: {}
---
# CA certificate
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: tikv-ca
  namespace: oxigraph
spec:
  isCA: true
  commonName: tikv-ca
  secretName: tikv-ca-secret
  duration: 87600h    # 10 years
  renewBefore: 8760h  # 1 year
  privateKey:
    algorithm: ECDSA
    size: 256
  issuerRef:
    name: tikv-selfsigned-issuer
    kind: Issuer
---
# CA Issuer (signs all component certificates)
apiVersion: cert-manager.io/v1
kind: Issuer
metadata:
  name: tikv-ca-issuer
  namespace: oxigraph
spec:
  ca:
    secretName: tikv-ca-secret
---
# TiKV server certificate
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: tikv-server-cert
  namespace: oxigraph
spec:
  secretName: tikv-server-tls
  duration: 8760h     # 1 year
  renewBefore: 720h   # 30 days
  commonName: tikv
  dnsNames:
    - "tikv"
    - "tikv.oxigraph.svc"
    - "tikv.oxigraph.svc.cluster.local"
    - "*.tikv.oxigraph.svc.cluster.local"
  privateKey:
    algorithm: ECDSA
    size: 256
  issuerRef:
    name: tikv-ca-issuer
    kind: Issuer
---
# PD server certificate
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: pd-server-cert
  namespace: oxigraph
spec:
  secretName: pd-server-tls
  duration: 8760h
  renewBefore: 720h
  commonName: pd
  dnsNames:
    - "pd"
    - "pd.oxigraph.svc"
    - "pd.oxigraph.svc.cluster.local"
    - "*.pd.oxigraph.svc.cluster.local"
  privateKey:
    algorithm: ECDSA
    size: 256
  issuerRef:
    name: tikv-ca-issuer
    kind: Issuer
---
# Oxigraph client certificate (for authenticating to TiKV/PD)
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: oxigraph-client-cert
  namespace: oxigraph
spec:
  secretName: oxigraph-client-tls
  duration: 8760h
  renewBefore: 720h
  commonName: oxigraph-client
  usages:
    - client auth
  privateKey:
    algorithm: ECDSA
    size: 256
  issuerRef:
    name: tikv-ca-issuer
    kind: Issuer
```

Mount the client TLS Secret into Oxigraph pods:

```yaml
# In the Oxigraph StatefulSet/Deployment spec
volumes:
  - name: tikv-client-tls
    secret:
      secretName: oxigraph-client-tls
      defaultMode: 0400
  - name: tikv-ca
    secret:
      secretName: tikv-ca-secret
      defaultMode: 0444
containers:
  - name: oxigraph
    volumeMounts:
      - name: tikv-client-tls
        mountPath: /etc/oxigraph/tls
        readOnly: true
      - name: tikv-ca
        mountPath: /etc/oxigraph/tls/ca
        readOnly: true
```

#### 5.2.5 Configuring tikv-client in Rust for TLS

The `tikv-client` Rust crate supports TLS via `Config::with_security()`. The `oxigraph-tikv` crate should wire this as follows:

```rust
use tikv_client::{Config, TransactionClient};
use std::path::PathBuf;

/// TLS configuration for connecting to TiKV
pub struct TlsConfig {
    /// Path to the CA certificate (PEM)
    pub ca_path: PathBuf,
    /// Path to the client certificate (PEM)
    pub cert_path: PathBuf,
    /// Path to the client private key (PEM)
    pub key_path: PathBuf,
}

/// Create a TiKV TransactionClient with optional mTLS
pub async fn create_tikv_client(
    pd_endpoints: Vec<String>,
    tls: Option<TlsConfig>,
) -> Result<TransactionClient, tikv_client::Error> {
    let config = if let Some(tls) = tls {
        Config::default().with_security(
            tls.ca_path.to_str().expect("valid CA path"),
            tls.cert_path.to_str().expect("valid cert path"),
            tls.key_path.to_str().expect("valid key path"),
        )
    } else {
        Config::default()
    };

    TransactionClient::new_with_config(pd_endpoints, config).await
}
```

The paths correspond to the Kubernetes Secret volume mounts:
- `ca_path`: `/etc/oxigraph/tls/ca/ca.crt`
- `cert_path`: `/etc/oxigraph/tls/tls.crt`
- `key_path`: `/etc/oxigraph/tls/tls.key`

These can be passed via environment variables or CLI flags:

```bash
oxigraph-cloud serve \
  --backend tikv \
  --tikv-pd-endpoints "pd-0.pd:2379,pd-1.pd:2379,pd-2.pd:2379" \
  --tikv-ca-path /etc/oxigraph/tls/ca/ca.crt \
  --tikv-cert-path /etc/oxigraph/tls/tls.crt \
  --tikv-key-path /etc/oxigraph/tls/tls.key
```

### 5.3 Combined Security Configuration Reference

Below is the complete `[security]` section for `tikv.toml` with both TDE and mTLS enabled:

```toml
# ============================================================
# TiKV Security Configuration (TDE + mTLS)
# ============================================================

[security]
# --- mTLS ---
ca-path = "/etc/tikv/tls/ca.pem"
cert-path = "/etc/tikv/tls/server.pem"
key-path = "/etc/tikv/tls/server-key.pem"

# --- Transparent Data Encryption ---
[security.encryption]
data-encryption-method = "aes256-ctr"
data-key-rotation-period = "7d"

[security.encryption.master-key]
type = "file"
path = "/etc/tikv/encryption/master.key"
```

---

## Appendix A: Troubleshooting Quick Reference

| Symptom | Likely Cause | Action |
|---------|-------------|--------|
| High Raftstore CPU | Too many Regions, or high write rate | Enable Hibernate Region; increase Region size; add TiKV nodes |
| Region count explosion | Bulk load causing rapid splits | Increase `region-split-check-diff`; increase Region size |
| High P99 read latency | Block cache miss; slow disk | Increase block cache; verify NVMe SSD; check compaction |
| Write stalls | Compaction backlog | Increase `max-background-jobs`; faster storage; reduce write rate |
| Leader imbalance | PD scheduler behind or misconfigured | Check PD logs; increase `leader-schedule-limit` |
| Scan returning empty results | Key prefix mismatch | Verify table prefix byte matches expected encoding |
| Transaction conflict | Hot key contention on same index entries | Use optimistic transactions; retry with backoff |
| Backup timeout | Large dataset, slow network to S3 | Increase `--concurrency`; raise `--ratelimit`; use closer S3 region |

## Appendix B: Useful tikv-ctl Commands

```bash
# Check cluster store status
tikv-ctl --pd "pd:2379" store

# List all Regions and their leaders
tikv-ctl --pd "pd:2379" region

# Scan keys in a Region (useful for debugging key encoding)
tikv-ctl --host "tikv:20160" scan --from "0x02" --to "0x03" --limit 10

# Check Region properties (size, key count)
tikv-ctl --host "tikv:20160" region-properties -r <region_id>

# Force Region merge (for stuck small Regions)
tikv-ctl --pd "pd:2379" operator add merge-region <source_region_id> <target_region_id>

# Compact a key range (force compaction for prefix 0x01 = id2str)
tikv-ctl --host "tikv:20160" compact --from "0x01" --to "0x02" --db kv
```
