# TiKV: Selected Distributed Storage Backend

TiKV is the **selected backend** for cloud-native Oxigraph. It is the most architecturally aligned candidate.

## Why TiKV

- Open-source, Apache 2.0 licensed
- **Written in Rust** (same as Oxigraph)
- Designed as the storage layer for TiDB (distributed HTAP SQL database)
- Range-based partitioning aligns with Oxigraph's lexicographic key encoding

## Architecture

### Consensus: Raft
- Every write recorded as Raft log
- Synchronously replicated to quorum before commit acknowledgement

### Partitioning: Regions
- Key space sharded into **Regions** (~96 MB each)
- Each Region stores a **contiguous, lexicographically sorted range** of keys
- Oxigraph's SPO/POS/OSP indexes naturally group related data within the same Region
- Region management: splitting, merging, load balancing handled by **Placement Driver (PD)**

### Local Storage
- Each TiKV node uses a local RocksDB (or TitanDB) instance for physical writes to NVMe storage

## Transactional Guarantees

- Strictly serializable distributed transactions (ACID)
- Two-phase commit protocol (Google Percolator model)
- PD provides centralized timestamp allocation for conflict detection
- Aligns with Oxigraph's "repeatable read" and atomic commit requirements

## Performance (YCSB Benchmarks)

| Configuration | Workload | OPS | P99 Latency |
|--------------|----------|-----|-------------|
| 3-Node (40 vCPUs, NVMe SSD) | 100% Read (Point Get) | 212,000 | < 10 ms |
| 3-Node (40 vCPUs, NVMe SSD) | 50% Read / 50% Update | 43,200 | < 10 ms |

## Coprocessor Framework

The **paramount advantage** for Oxigraph.

### How It Works
1. Oxigraph's `spareval` engine constructs a DAG of execution plans
2. DAG pushed to TiKV storage nodes via gRPC
3. Storage nodes execute scans, filters, and aggregations **locally**
4. Only final results returned to compute node

### Example
```
SPARQL: SELECT COUNT(?o) WHERE { <SubjectA> <PredicateB> ?o }
    ↓
Coprocessor DAG: IndexScan(POS keys) → Aggregation(COUNT)
    ↓
TiKV node returns: single integer (partial sum)
```

### TiGraph Precedent
- PingCAP's internal project mapping graph data to TiKV
- Achieved **8,700x** query performance improvement via Coprocessor pushdown

### Coprocessor Cache
- Caches pushdown results at Region level in compute instance memory
- Serves identical BGP patterns instantly if Region has no mutations

## Operational Considerations

### Complexity
- Must manage TiKV storage nodes + Placement Driver (PD) quorum
- Oxigraph's small, fragmented KV pairs → millions of Regions for large graphs

### Region Tuning Required
- Excessive Regions saturate Raftstore (heartbeat processing overhead)
- Mitigations:
  - **Hibernate Region**: Suppress heartbeats for idle data
  - **Region Merge**: Aggressively merge small regions
  - Tune Region size for Oxigraph's access patterns

## Rust Client
- Crate: `tikv-client`
- Docs: https://tikv.org/docs/7.1/develop/clients/rust/

## Key References
- TiKV architecture: https://tikv.org/docs/5.1/reference/architecture/storage/
- Data sharding: https://tikv.org/deep-dive/scalability/data-sharding/
- Coprocessor guide: https://tikv.github.io/tikv-dev-guide/understanding-tikv/coprocessor/intro.html
- Distributed SQL pushdown: https://tikv.org/deep-dive/distributed-sql/dist-sql/
- Region tuning: https://tikv.org/blog/tune-with-massive-regions-in-tikv/
- TiGraph: https://pingcap.co.jp/blog/tigraph-8700x-computing-performance-achieved-by-combining-graphs-rdbms-syntax/
- Performance: https://tikv.org/docs/6.1/deploy/performance/overview/
