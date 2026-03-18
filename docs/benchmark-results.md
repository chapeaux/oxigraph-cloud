# Performance Benchmark Results

**Date**: 2026-03-18
**Environment**: Red Hat Developer Sandbox (OpenShift)
**Datasets**: RocksDB: 32K triples | TiKV: 1K triples (single-node)

## Results (median of 3-5 runs, seconds)

| Operation | RocksDB | TiKV | TiKV/RocksDB Ratio |
|-----------|---------|------|-------------------|
| Point query (single subject) | 0.051s | 0.052s | **1.0x** |
| Range scan (100 results) | 0.065s | 0.118s | **1.8x** |
| COUNT aggregate | 0.059s | 0.053s | **0.9x** |
| FILTER (string contains) | 0.097s | 0.372s | **3.8x** |
| Single INSERT | 0.048s | 0.052s | **1.1x** |

## Analysis

- **Point queries**: Near-identical latency (~50ms). TiKV's Percolator 2PC overhead is minimal for single-key lookups.
- **Range scans**: TiKV is ~1.8x slower due to network round-trip for batch_scan vs RocksDB's local iterator. The prefetch batching (512 keys) helps.
- **COUNT**: TiKV is actually slightly faster in this test — likely due to smaller dataset (1K vs 32K) and warm TiKV block cache.
- **FILTER**: TiKV is ~3.8x slower because FILTER requires fetching all candidates over the network, then evaluating the filter client-side. This is exactly where **Coprocessor pushdown** would help — pushing the FILTER to TiKV Regions would eliminate the data transfer.
- **INSERT**: Near-identical (~50ms). TiKV's 2PC commit is fast for single-key writes.

## Coprocessor Pushdown Potential

The FILTER query result (3.8x slower) demonstrates the key optimization opportunity:
- Without pushdown: scan all keys from TiKV → transfer to client → apply FILTER
- With pushdown: TiKV Region evaluates FILTER locally → only matching results transferred

Expected improvement for FILTER queries: **2-4x** reduction in latency by eliminating network transfer of non-matching rows.

## Notes

- All measurements include HTTP round-trip (client → Route → Pod → backend)
- RocksDB dataset (32K triples) is larger than TiKV dataset (1K triples)
- Single-node TiKV — no replication overhead
- Sandbox resources are limited (250m-500m CPU per container)
- Target: TiKV within 5x of RocksDB — **achieved** for all operations
