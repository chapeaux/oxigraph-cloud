# Cloud-Native Graph Storage and Distributed SPARQL Theory

## Workload Classification

| Paradigm | Description | SPARQL Example |
|----------|-------------|----------------|
| **OLTP** | High-volume, short, concurrent transactions; point lookups/small updates | Fetch a specific node, update a single edge |
| **OLAP** | Complex, long-running aggregations across entire dataset | Multi-hop path traversal for fraud detection |

Oxigraph aims to balance both OLTP and OLAP within a unified engine.

## The Network Bottleneck Problem

When migrating from local RocksDB to distributed storage:
- Primary bottleneck shifts from **disk I/O** to **network communication**
- In shared-nothing architecture, RDF graph is partitioned across many storage nodes
- Naive approach: storage nodes transmit gigabytes of raw, unfiltered triples to compute node for hash joins
- This is the **primary bottleneck** in distributed SPARQL engines

## Requirements for Optimal Storage Backend

1. Durable, highly available, strongly consistent storage
2. **Compute pushdown** — filter, aggregate, and join at storage nodes to minimize network transfer
3. Ordered, lexicographical range scans (for SPO/POS/OSP prefix matching)
4. Distributed MVCC for concurrent writes with ACID guarantees

## Network Optimization Strategies

### Extended Vertical Partitioning (ExtVP) and Semi-Joins

- Pre-filter data before network transmission using algebraic join decomposition
- Structural correlations: Subject-Subject (SS), Object-Subject (OS), Subject-Object (SO)
- Example: For `?person foaf:name ?name` JOIN `?person dbpedia:birth ?birthdate`, an OS semi-join filter ensures storage nodes only transmit `?person` records if the corresponding birthdate exists

### Implementation Requirements
- Enhance Oxigraph's `sparopt` (SPARQL Optimizer) crate
- Generate execution plans that pass bloom filters / semi-join conditions to remote KV store before requesting data

### Coprocessor Pushdown vs. Scatter-Gather

| Approach | Backend | Behavior |
|----------|---------|----------|
| **Coprocessor Pushdown** | TiKV | Translate sparopt AST into TiKV Protobuf Coprocessor requests; storage nodes filter/aggregate locally |
| **Scatter-Gather** | FoundationDB | Fetch batches from FDB, filter/join entirely in compute node |

Coprocessor pushdown is dramatically more efficient for graph workloads.

## Key References
- Distributed SPARQL survey: https://www.vldb.org/pvldb/vol10/p2049-abdelaziz.pdf
- S2RDF (ExtVP): http://www.vldb.org/pvldb/vol9/p804-schaetzle.pdf
- Distributed semi-joins: https://ceur-ws.org/Vol-401/iswc2008pd_submission_69.pdf
