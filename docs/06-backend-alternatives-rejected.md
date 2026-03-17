# Rejected Backend Alternatives

## FoundationDB — Rejected (5-Second Transaction Limit)

### Strengths
- Deterministic simulation testing framework — mathematically proven ACID
- Unified ordered lexicographic key-value space
- "Record Layer" pattern: stateless compute layers on top of raw KV store
- Mature Rust bindings (`foundationdb-rs`, 5M+ downloads)

### Why Rejected

**5-second transaction time limit**: Any transaction exceeding 5s is unilaterally aborted. Complex SPARQL OLAP queries across large graphs will exceed this.

**Workaround complexity**: Would require continuation tokens (like Apple's RecordScanLimiter), partitioning queries into sub-5s chunks, invalidating true snapshot isolation for analytics.

**No Coprocessor pushdown**: Strictly scatter-gather execution model. All filtering, aggregation, and joins must occur in compute node, amplifying network bottleneck.

### Latency Profile

| Range Size (KV pairs) | Batched (ms) | Sequential Async (ms) |
|----------------------|--------------|----------------------|
| 10 | 1.237 | 1.920 |
| 100 | 1.515 | 2.947 |
| 1,000 | 3.316 | 6.013 |

If used, Oxigraph iterators would need aggressive prefetching/bulk-batching.

## Amazon DynamoDB — Rejected (Impedance Mismatch)

### Why Rejected

**Partition Key requirement**: DynamoDB requires exact Partition Key value for efficient queries. SPARQL queries have variables in any position.

**Full Scan problem**: Queries not aligned with Partition Key trigger full table scans, consuming RCUs for every item read (even if 99.9% are discarded by filters).

**Cost**: Complex SPARQL queries → astronomical, prohibitive RCU costs.

**Mitigation gaps**: DAX only caches reads; doesn't fix the inability to perform arbitrary multi-dimensional range scans.

## S3-Native Columnar Storage (Parquet/Arrow) — Rejected (OLTP Deficit)

### Strengths
- Excellent for OLAP: stateless compute, Parquet columnar pruning, bloom filters
- Elastic scaling: spin up hundreds of compute nodes against S3

### Why Rejected

**Latency**: S3 HTTP overhead = tens to hundreds of milliseconds per request. Incompatible with OLTP point queries and real-time SHACL validation.

**Immutability**: S3 objects are immutable. SPARQL UPDATE requires complex append-only metadata layer (Apache Iceberg-like) with background compaction.

**Verdict**: Viable as a tertiary archival/warehousing mechanism, not as primary operational backend.
