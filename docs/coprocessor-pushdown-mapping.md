# Coprocessor Pushdown Mapping

## Overview

This document maps Oxigraph's `sparopt` query plan operators to TiKV Coprocessor operations that can be pushed down to Region-local execution.

## Pushdown-Eligible Operations

| sparopt Operator | Coprocessor Op | Description |
|-----------------|---------------|-------------|
| `QuadPattern` (BGP) | `IndexScan` | Prefix scan on the appropriate index table (SPOG/POSG/etc.) based on bound variables |
| `Filter` (simple) | `FilterScan` | Apply predicate during scan; avoids transferring filtered-out rows |
| `Aggregate(COUNT)` | `CountScan` | Count matching keys without returning values |
| `Aggregate(MIN/MAX)` | `MinMaxScan` | Return first/last key in sorted range |

## Not Pushdown-Eligible (Process on Coordinator)

| Operator | Reason |
|----------|--------|
| `Join` (hash/merge) | Requires data from multiple Regions |
| `Union` | Can be parallelized but not pushed down per-Region |
| `Optional` (LEFT JOIN) | Requires coordinator-side null handling |
| `OrderBy` | Global ordering requires all results |
| `Distinct` | Requires global deduplication |
| `Group` (non-COUNT) | Complex aggregation needs full result set |
| `SubQuery` | Nested evaluation not supported in Coprocessor |

## Semi-Join Optimization

For 2-BGP join patterns like:
```sparql
SELECT ?x ?name WHERE {
  ?x ex:type ex:Person .   # BGP1
  ?x ex:name ?name .        # BGP2
}
```

1. Execute BGP1 scan, collect `?x` bindings
2. Build a Bloom filter over `?x` values
3. Send Bloom filter with BGP2 scan request
4. Region-local scan prunes non-matching `?x` before returning results

This reduces network transfer proportional to the selectivity of BGP1.

## Index Selection

| Bound Variables | Best Index | Table Prefix |
|----------------|-----------|-------------|
| S, P, O, G | SPOG | 0x02 |
| P, O | POSG | 0x03 |
| O, S | OSPG | 0x04 |
| G, S, P, O | GSPO | 0x05 |
| S, P, O (default graph) | DSPO | 0x08 |
| P, O (default graph) | DPOS | 0x09 |
| O (default graph) | DOSP | 0x0A |
