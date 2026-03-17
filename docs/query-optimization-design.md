# Query Optimization Design: Coprocessor Pushdown & Semi-Join Filters

> **Phase 4 Tasks 4.1 + 4.4** | **Status**: Design | **Date**: 2026-03-17

This document maps Oxigraph's `sparopt` query plan representation to TiKV Coprocessor capabilities and defines a concrete semi-join filter strategy for distributed join optimization.

---

## 1. sparopt Plan Node Taxonomy

The `sparopt` crate (`lib/sparopt/src/algebra.rs`) defines `GraphPattern` as the core plan node enum. Every SPARQL query is lowered to a tree of these nodes by the `Optimizer`, which performs normalization, join reordering, and filter pushdown (within the logical plan).

### 1.1 Complete Node Inventory

| Node | Description | Pushdown Candidate? |
|------|-------------|---------------------|
| `QuadPattern { subject, predicate, object, graph_name }` | Single triple/quad pattern (BGP leaf). Resolved via `internal_quads_for_pattern` on the dataset. | **Yes** -- primary pushdown target. Maps directly to TiKV prefix scan on SPO/POS/OSP index. |
| `Path { subject, path, object, graph_name }` | Property path evaluation (transitive closure, alternatives, etc.) | **No** -- requires iterative expansion. Must remain in compute node. |
| `Graph { graph_name }` | Named graph existence check or enumeration | **Partial** -- existence check can be a point lookup on TABLE_GRAPHS |
| `Join { left, right, algorithm: JoinAlgorithm }` | Inner join. Currently only `HashBuildLeftProbeRight { keys }`. | **Indirect** -- semi-join filter target (see Section 4) |
| `LeftJoin { left, right, expression, algorithm }` | Left outer join with optional filter | **No** -- complex semantics; keep in compute node |
| `Lateral { left, right }` | Correlated subquery (sep-0006) | **No** |
| `Filter { expression, inner }` | Predicate filter over child results | **Yes** -- simple filters on encoded term values can be pushed into TiKV scan |
| `Union { inner: Vec }` | Bag union of alternatives | **Partial** -- translates to parallel scan of multiple key ranges |
| `Extend { inner, variable, expression }` | Computed column (BIND) | **No** -- expression evaluation stays in compute node |
| `Minus { left, right, algorithm }` | Anti-join | **No** |
| `Values { variables, bindings }` | Inline data table | **No** -- already materialized |
| `OrderBy { inner, expression }` | Sort | **No** -- requires full materialization |
| `Project { inner, variables }` | Column projection | **Partial** -- can reduce bytes returned from TiKV by selecting only needed term components |
| `Distinct { inner }` | Duplicate elimination | **No** |
| `Reduced { inner }` | Consecutive deduplication | **No** |
| `Slice { inner, start, length }` | OFFSET/LIMIT | **Yes** -- `LIMIT` can be pushed as `scan_limit` parameter to TiKV |
| `Group { inner, variables, aggregates }` | GROUP BY + aggregation. `AggregateExpression` supports `CountSolutions` and `FunctionCall(Sum/Min/Max/Avg/etc.)` | **Yes** -- COUNT, SUM, MIN, MAX can be partially computed at storage nodes |
| `Service { name, inner, silent }` | Federated SPARQL | **No** -- delegated to remote endpoint |

### 1.2 Join Algorithm Detail

The optimizer currently selects a single join strategy:

```rust
pub enum JoinAlgorithm {
    HashBuildLeftProbeRight { keys: Vec<Variable> },
}
```

The `keys` vector contains the shared variables between left and right sides. When empty, the evaluator falls back to a Cartesian product. This `keys` field is the critical input for our semi-join filter design -- it tells us exactly which variable bindings to include in the bloom filter.

### 1.3 Evaluation Architecture

`spareval` (`lib/spareval/src/eval.rs`) compiles the `GraphPattern` tree into a tree of closures (`InternalTupleEvaluator`), where each closure takes an `InternalTuple` and returns an iterator of `InternalTuple` results. The `QueryableDataset` trait provides the storage access point via `internal_quads_for_pattern(subject, predicate, object, graph_name)`.

Key insight: the current evaluator is **fully pull-based and synchronous**. Each node calls its children lazily. For TiKV pushdown, we need to introduce **push-based batch execution** for eligible subtrees while keeping the pull-based interface at the boundary.

---

## 2. Coprocessor Pushdown Mapping

### 2.1 Architecture

TiKV's Coprocessor framework accepts a DAG of execution operators sent as protobuf messages over gRPC. Each TiKV storage node executes the DAG locally against its Region data and returns only the results. This eliminates the need to transfer raw key-value pairs to the compute node for filtering.

Our approach: **custom Coprocessor plugin** deployed alongside TiKV, rather than reusing TiDB's SQL-oriented built-in operators. Rationale:
- TiDB's built-in Coprocessor speaks in terms of SQL table schemas, column IDs, and datum encoding -- none of which match Oxigraph's byte-encoded RDF terms
- A custom plugin can operate directly on Oxigraph's `EncodedTerm` byte layout
- TiKV's plugin framework (`coprocessor_plugin_api`) allows loading `.so` plugins without rebuilding TiKV

### 2.2 Pushdown Operator Catalog

#### Operator 1: IndexScan

Maps `QuadPattern` to a prefix range scan on the appropriate TiKV table.

```
Input:  QuadPattern { subject: ?s, predicate: <foaf:name>, object: ?o, graph_name: None }
Output: Coprocessor IndexScan {
            table: TABLE_DPOS,
            prefix: [0x09, encode_term(<foaf:name>)],
            returns: [subject_offset, object_offset]
        }
```

Index selection logic (same as existing Oxigraph, based on which positions are bound):
| Bound positions | Default graph table | Named graph table |
|----------------|--------------------|--------------------|
| S,P,O | DSPO | SPOG |
| S,P | DSPO | SPOG |
| S,O | DOSP | OSPG |
| S | DSPO | SPOG (or GSPO with G bound) |
| P,O | DPOS | POSG |
| P | DPOS | POSG |
| O | DOSP | OSPG |
| none | DSPO | GSPO |

#### Operator 2: FilterScan

Combines `Filter { expression, inner: QuadPattern }` into a single Coprocessor request that scans and filters in-place.

Pushable filter expressions (operating on `EncodedTerm` byte representation):
- **Equality**: `?var = <constant>` -- compare encoded bytes directly
- **Type check**: `isIRI(?var)`, `isLiteral(?var)` -- check leading type byte (1-7 for named nodes, 16-47 for literals, etc.)
- **Bound check**: `BOUND(?var)` -- term slot is non-empty
- **Numeric comparison**: `?var > 42` -- compare inline-encoded integer/float/decimal bytes
- **Boolean AND/OR** of the above

Non-pushable expressions (require full term deserialization or external state):
- `REGEX`, `CONTAINS`, `STRSTARTS` on big string literals (require id2str dictionary lookup)
- `LANG()`, `DATATYPE()` comparisons on big literals
- `EXISTS` subqueries
- Custom function calls

#### Operator 3: CountAggregation

Maps `Group { inner: QuadPattern, variables: [], aggregates: [(_, CountSolutions)] }` to:

```
Coprocessor CountScan {
    table: TABLE_DPOS,
    prefix: [0x09, encode_term(<foaf:name>)],
}
-> Returns: u64 count per Region
```

The compute node sums partial counts from all Regions. This is the simplest and highest-impact aggregation pushdown.

#### Operator 4: MinMaxAggregation

For `MIN(?x)` or `MAX(?x)` over a scan, the Coprocessor can:
- For MAX: scan the prefix range in reverse, return the first key
- For MIN: return the first key in forward scan
- Works because Oxigraph's term encoding preserves sort order within a type

#### Operator 5: LimitScan

Maps `Slice { inner: QuadPattern, start: 0, length: Some(n) }` to a scan with `limit: n`. Each Region returns at most `n` results; the compute node merges and applies the final limit.

### 2.3 Coprocessor Request Protobuf Schema

```protobuf
message OxigraphCoprocessorRequest {
    enum OpType {
        INDEX_SCAN = 0;
        FILTER_SCAN = 1;
        COUNT_SCAN = 2;
        MIN_MAX_SCAN = 3;
    }
    OpType op_type = 1;
    bytes table_prefix = 2;      // 1-byte table ID + encoded term prefix
    bytes upper_bound = 3;       // exclusive upper bound for range
    uint32 limit = 4;            // 0 = unlimited
    FilterExpr filter = 5;       // optional filter expression
    bool is_max = 6;             // for MIN_MAX_SCAN: true=MAX, false=MIN
    repeated uint32 return_offsets = 7;  // byte offsets of terms to return
    bytes bloom_filter = 8;      // optional semi-join bloom filter (Section 4)
    uint32 bloom_term_offset = 9; // byte offset of term to check against bloom
}

message FilterExpr {
    enum FilterOp {
        EQ = 0; NEQ = 1; GT = 2; GTE = 3; LT = 4; LTE = 5;
        AND = 6; OR = 7; NOT = 8;
        TYPE_CHECK = 9; BOUND_CHECK = 10;
    }
    FilterOp op = 1;
    uint32 term_offset = 2;      // byte offset within the scanned key
    bytes constant_value = 3;    // encoded term to compare against
    repeated FilterExpr children = 4; // for AND/OR/NOT
    uint32 type_byte_min = 5;    // for TYPE_CHECK: min type byte (inclusive)
    uint32 type_byte_max = 6;    // for TYPE_CHECK: max type byte (inclusive)
}

message OxigraphCoprocessorResponse {
    repeated bytes results = 1;  // encoded key fragments or full keys
    uint64 count = 2;            // for COUNT_SCAN
    bytes min_max_value = 3;     // for MIN_MAX_SCAN
}
```

### 2.4 Pushdown Decision Logic

The query planner walks the `GraphPattern` tree bottom-up. At each node, it checks whether the subtree is "pushdown-eligible":

```
fn is_pushdown_eligible(pattern: &GraphPattern) -> PushdownPlan {
    match pattern {
        QuadPattern { .. } => PushdownPlan::IndexScan,

        Filter { inner, expression } if is_pushdown_eligible(inner).is_some()
            && is_pushable_expression(expression) =>
            PushdownPlan::FilterScan,

        Slice { inner, start: 0, length: Some(n) }
            if is_pushdown_eligible(inner).is_some() =>
            PushdownPlan::WithLimit(inner_plan, n),

        Group { inner, variables: [], aggregates }
            if is_pushdown_eligible(inner).is_some()
            && aggregates.len() == 1
            && is_pushable_aggregate(&aggregates[0]) =>
            PushdownPlan::Aggregation,

        _ => PushdownPlan::None,
    }
}
```

---

## 3. Batch Prefetching Strategy

### 3.1 Problem

Without prefetching, evaluating a multi-BGP query against TiKV proceeds as:
1. Scan first QuadPattern -- network round-trip
2. For each result, scan second QuadPattern -- N more round-trips
3. Total: O(N) sequential network round-trips

### 3.2 Design

**Predictive batch prefetching** issues parallel `batch_scan` requests to TiKV based on the query plan structure.

#### Step 1: Plan Analysis

When the evaluator encounters a `Join { left: QuadPattern(A), right: QuadPattern(B), algorithm: HashBuildLeftProbeRight { keys } }`:

1. Execute the left scan (QuadPattern A) -- this is the "build" side
2. Collect results into batches of size `prefetch_batch_size` (configurable, default 256)
3. For each batch, compute the set of bound values for the join variables
4. Construct prefetch key ranges for QuadPattern B using those bound values
5. Issue a single `batch_scan` to TiKV with all key ranges in parallel

#### Step 2: Key Range Construction

For a join `?person foaf:name ?name . ?person dbpedia:birth ?birthdate`:
- Left scan on DPOS with prefix `encode(foaf:name)` yields `{?person}` bindings
- For each `?person` value, construct a DPOS prefix: `encode(dbpedia:birth) + encode(?person)` ... wait, that's POS ordering. We need DSPO: `encode(?person) + encode(dbpedia:birth)`.
- Issue `batch_scan([DSPO_prefix(?person_1, dbpedia:birth), DSPO_prefix(?person_2, dbpedia:birth), ...])`.

#### Step 3: Parallel Execution

```rust
/// Prefetch key ranges for the probe side of a hash join.
async fn batch_prefetch(
    client: &TransactionClient,
    table: u8,
    prefixes: Vec<Vec<u8>>,
    limit_per_prefix: u32,
) -> Result<Vec<Vec<KvPair>>, StorageError> {
    let ranges: Vec<BoundRange> = prefixes.iter()
        .map(|p| prefix_range(&prefixed_key(table, p)))
        .collect();

    // TiKV client fans out to all relevant Regions in parallel
    let snapshot = client.snapshot().await?;
    let mut results = Vec::with_capacity(ranges.len());
    for range in ranges {
        results.push(snapshot.scan(range, limit_per_prefix).await?);
    }
    results
}
```

The existing `TiKvConfig::scan_batch_size` (default 512) controls how many keys are fetched per individual scan. The new `prefetch_batch_size` controls how many probe-side prefixes are batched into one parallel operation.

#### Step 4: Integration with Evaluator

Wrap the batch prefetch results in a `PrefetchCache` that the right-side evaluator consults before issuing individual scans:

```rust
struct PrefetchCache {
    cache: HashMap<Vec<u8>, Vec<KvPair>>,  // prefix -> results
}

impl PrefetchCache {
    fn get_or_scan(&self, prefix: &[u8]) -> Option<&[KvPair]> {
        self.cache.get(prefix).map(|v| v.as_slice())
    }
}
```

When the right-side `QuadPattern` evaluator fires for a specific binding, it first checks the `PrefetchCache`. Cache miss triggers a normal single-prefix scan (fallback).

### 3.3 Applicability

Batch prefetching applies when:
- The join's right child is a `QuadPattern` (not a complex subgraph)
- The join keys correspond to a bound position in the right-side pattern
- The left side produces a bounded number of results (not a full table scan)

Heuristic: skip prefetching when the left side's estimated cardinality exceeds `max_prefetch_cardinality` (default 10,000) to avoid memory pressure.

---

## 4. Semi-Join Filter Design

### 4.1 Motivation

Even with batch prefetching, the right-side scan may return many results that don't join with the left side. A **bloom filter semi-join** eliminates non-matching rows at the storage node before they cross the network.

### 4.2 Algorithm

For a 2-BGP join:
```sparql
SELECT ?person ?name ?birthdate WHERE {
    ?person foaf:name ?name .          -- BGP_A (left/build)
    ?person dbpedia:birth ?birthdate . -- BGP_B (right/probe)
}
```

**Step 1: Execute BGP_A and build bloom filter**

```rust
// Execute left-side scan
let left_results: Vec<InternalTuple> = evaluate(bgp_a);

// Extract join key values (encoded term bytes for ?person)
let join_key_index = variable_position("?person", &encoded_variables);
let mut bloom = BloomFilter::with_capacity_and_fpr(left_results.len(), 0.01);

for tuple in &left_results {
    if let Some(term) = tuple.get(join_key_index) {
        // Serialize the InternalTerm to its encoded byte representation
        let mut key_bytes = Vec::with_capacity(WRITTEN_TERM_MAX_SIZE);
        write_term(&mut key_bytes, &term);
        bloom.insert(&key_bytes);
    }
}
```

**Step 2: Send bloom filter to TiKV with BGP_B scan**

The bloom filter is serialized and attached to the Coprocessor request:

```rust
let request = OxigraphCoprocessorRequest {
    op_type: OpType::FILTER_SCAN,
    table_prefix: prefixed_key(TABLE_DPOS, &encode_term(dbpedia_birth)),
    bloom_filter: bloom.serialize(),
    bloom_term_offset: subject_byte_offset_in_dpos_key,  // offset where ?person bytes begin
    ..default()
};
```

**Step 3: TiKV Coprocessor applies bloom filter during scan**

Inside the Coprocessor plugin, for each scanned key:

```rust
fn process_key(key: &[u8], request: &OxigraphCoprocessorRequest) -> bool {
    if request.bloom_filter.is_empty() {
        return true;  // no bloom filter, accept all
    }
    let term_start = request.bloom_term_offset as usize;
    let term_end = term_start + encoded_term_length(&key[term_start..]);
    let term_bytes = &key[term_start..term_end];
    bloom_check(&request.bloom_filter, term_bytes)
}
```

**Step 4: Compute node receives filtered results, performs exact join**

The bloom filter may produce false positives (at the configured 1% FPR). The compute node performs the final exact hash join on the filtered results, eliminating any false positives.

### 4.3 Key Serialization for Bloom Filter

The bloom filter operates on **encoded term bytes** -- the same byte representation used in TiKV keys. This is critical: the Coprocessor extracts a byte slice from the scanned key at a known offset and checks it against the bloom filter directly, with zero deserialization.

Term byte layout (from `binary_encoder.rs`):
```
[type_byte: u8] [value: up to 32 bytes]
```

For the bloom filter, we include the full encoded term (type byte + value). This ensures that two terms with the same hash but different types (e.g., a named node and a literal with the same hash) are correctly distinguished.

The term offset within a key depends on the index table:

| Table | Key layout | Subject offset | Predicate offset | Object offset |
|-------|-----------|----------------|------------------|---------------|
| DSPO  | S \| P \| O | 0 | sizeof(S) | sizeof(S) + sizeof(P) |
| DPOS  | P \| O \| S | sizeof(P) + sizeof(O) | 0 | sizeof(P) |
| DOSP  | O \| S \| P | sizeof(O) | sizeof(O) + sizeof(S) | 0 |

Since encoded terms have variable length (1-33 bytes), the Coprocessor must parse terms sequentially from the key start to find the target offset. This is fast: read the type byte, determine the value length from the type, skip forward.

### 4.4 Bloom Filter Sizing

| Left-side cardinality | FPR | Bloom filter size | Network overhead |
|-----------------------|-----|-------------------|------------------|
| 1,000 | 1% | ~1.2 KB | Negligible |
| 10,000 | 1% | ~12 KB | Negligible |
| 100,000 | 1% | ~120 KB | Acceptable |
| 1,000,000 | 1% | ~1.2 MB | Marginal |

For cardinalities above 100K, consider reducing FPR to 5% (halves filter size) or falling back to batch prefetching without bloom filter.

### 4.5 When to Apply Semi-Join Filters

The optimizer applies a semi-join filter when ALL of the following hold:

1. The pattern is a `Join` with `HashBuildLeftProbeRight` algorithm
2. The join `keys` vector is non-empty (not a Cartesian product)
3. Both children are `QuadPattern` nodes (or `Filter { QuadPattern }`)
4. The TiKV Coprocessor plugin is available (capability check at startup)
5. The left-side estimated cardinality is between `min_bloom_cardinality` (default 100) and `max_bloom_cardinality` (default 500,000)

Below `min_bloom_cardinality`, the overhead of building and transmitting the bloom filter exceeds the savings. Above `max_bloom_cardinality`, the bloom filter becomes too large; fall back to plain batch prefetching.

### 4.6 Multi-Variable Semi-Joins

When the join has multiple shared variables (e.g., `keys: [?person, ?city]`), the bloom filter hashes the **concatenation** of all join key encoded terms:

```rust
let mut key_bytes = Vec::new();
for &var_idx in &join_key_indices {
    if let Some(term) = tuple.get(var_idx) {
        write_term(&mut key_bytes, &term);
    } else {
        return; // skip tuples with unbound join keys
    }
}
bloom.insert(&key_bytes);
```

The Coprocessor extracts and concatenates the corresponding byte ranges from the scanned key in the same order.

### 4.7 Cascading Semi-Joins (3+ BGP Patterns)

For queries with 3+ BGP patterns joined in sequence:

```sparql
?person foaf:name ?name .
?person dbpedia:birth ?birthdate .
?person dbpedia:employer ?employer .
```

The optimizer produces a left-deep join tree:
```
Join(
  Join(BGP_A, BGP_B),
  BGP_C
)
```

Semi-join filters cascade:
1. Execute BGP_A, build bloom_AB for `?person`
2. Execute BGP_B with bloom_AB, build bloom_BC from the join result's `?person` values
3. Execute BGP_C with bloom_BC

Each successive bloom filter is tighter because it only contains values that survived the previous join. This progressively reduces data transfer at each stage.

---

## 5. Implementation Roadmap

### Step 1: Bloom Filter Library (1-2 days)

**Owner**: `/rust-dev`

- Add `oxigraph-bloom` utility module (or use the `bloomfilter` crate)
- API: `BloomFilter::new(capacity, fpr)`, `insert(&[u8])`, `check(&[u8]) -> bool`, `serialize() -> Vec<u8>`, `deserialize(&[u8]) -> BloomFilter`
- Unit tests for false positive rate validation

### Step 2: Coprocessor Plugin Skeleton (3-5 days)

**Owner**: `/rust-dev` + `/tikv-ops`

- Create `oxigraph-coprocessor` crate implementing TiKV's `CoprocessorPlugin` trait
- Implement `OxigraphCoprocessorRequest` / `OxigraphCoprocessorResponse` protobuf messages (use `prost`)
- Implement `IndexScan` operator: prefix range scan, decode keys, return matching encoded terms
- Build and deploy plugin `.so` to TiKV dev cluster
- Integration test: send raw Coprocessor request, verify response

### Step 3: Pushdown Decision Layer (2-3 days)

**Owner**: `/rust-dev`

- Add `PushdownAnalyzer` that walks a `GraphPattern` tree and annotates pushdown-eligible subtrees
- Output: `PushdownPlan` enum per node (None, IndexScan, FilterScan, CountScan, etc.)
- Wire into `SimpleEvaluator::build_graph_pattern_evaluator`: when a node has a `PushdownPlan`, generate a Coprocessor request instead of the normal closure chain
- Unit tests with mock Coprocessor responses

### Step 4: FilterScan Pushdown (2-3 days)

**Owner**: `/rust-dev`

- Implement `is_pushable_expression` for `Expression` variants
- Translate pushable expressions to `FilterExpr` protobuf
- Implement `FilterExpr` evaluation in the Coprocessor plugin
- Integration tests: `SELECT ?s WHERE { ?s <p> ?o . FILTER(?o > 42) }`

### Step 5: Semi-Join Bloom Filter (3-5 days)

**Owner**: `/rust-dev`

- After left-side evaluation in a `Join`, build bloom filter from join key bindings
- Attach serialized bloom filter to right-side Coprocessor request
- Implement bloom filter check in Coprocessor plugin's scan loop
- Add `bloom_term_offset` computation based on index table and pattern structure
- Integration tests: 2-BGP join queries with and without bloom filter, verify same results
- Benchmark: measure bytes transferred with and without bloom filter

### Step 6: Batch Prefetching (2-3 days)

**Owner**: `/rust-dev`

- Implement `PrefetchCache` struct
- Add batch prefix construction logic for probe-side QuadPatterns
- Wire into join evaluator: after building left side, prefetch right-side key ranges
- Combine with bloom filter: prefetched ranges carry the bloom filter to Coprocessor
- Benchmark: measure latency improvement on multi-BGP queries

### Step 7: COUNT/MIN/MAX Aggregation Pushdown (2-3 days)

**Owner**: `/rust-dev`

- Implement `CountScan` and `MinMaxScan` operators in Coprocessor plugin
- Wire `Group` node detection into pushdown analyzer
- Compute node merges partial aggregates from all Regions
- Integration tests: `SELECT COUNT(*) WHERE { ?s <p> ?o }`, `SELECT MIN(?o) WHERE { <s> <p> ?o }`

### Step 8: Benchmarking & Tuning (2-3 days)

**Owner**: `/test-qa`

- Benchmark suite covering:
  - Single BGP scan (baseline)
  - Multi-BGP join without optimization
  - Multi-BGP join with batch prefetching only
  - Multi-BGP join with bloom filter semi-join
  - COUNT aggregation pushdown vs. compute-side count
- Tune defaults: `prefetch_batch_size`, `min_bloom_cardinality`, `max_bloom_cardinality`, bloom FPR
- Document results and update configuration recommendations

### Total Estimated Effort: 17-27 days

### Dependency Chain

```
Step 1 (Bloom) ──────────────────────────┐
Step 2 (Coprocessor skeleton) ───┐       │
                                 v       v
Step 3 (Decision layer) ──> Step 4 (FilterScan) ──> Step 5 (Semi-join)
                                 │                        │
                                 v                        v
                            Step 7 (Aggregation)    Step 6 (Prefetch)
                                 │                        │
                                 └────────┬───────────────┘
                                          v
                                    Step 8 (Benchmark)
```

Steps 1 and 2 can proceed in parallel. Steps 3-4 are sequential. Steps 5, 6, and 7 can proceed in parallel after Step 4. Step 8 requires all prior steps.

---

## 6. Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Custom Coprocessor plugin vs. built-in operators | Custom plugin | TiDB's built-in operators use SQL datum encoding incompatible with Oxigraph's `EncodedTerm` byte layout |
| Bloom filter vs. hash-based semi-join | Bloom filter | Compact serialization (KB not MB); false positives acceptable since exact join follows; no need to transmit actual key values |
| Where bloom filter check runs | TiKV Coprocessor (storage node) | Eliminates non-matching rows before network transfer -- the entire point |
| Bloom filter FPR | 1% default, configurable | 1% provides good filtering with compact size; adjustable for large cardinalities |
| Sync wrapper for Coprocessor calls | `runtime.block_on()` per ADR-003 | Consistent with existing TiKV backend approach; avoids async infection of evaluator |
| Aggregation pushdown scope | COUNT, MIN, MAX only (Phase 4) | SUM and AVG require type-aware arithmetic on encoded terms; defer to Phase 4+ |

---

## 7. Risk Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| TiKV Coprocessor plugin API instability | Plugin breaks on TiKV upgrade | Pin TiKV version; wrap plugin API in thin compatibility layer; CI tests against target TiKV version |
| Bloom filter overhead exceeds savings for small result sets | Slower queries | Cardinality threshold (`min_bloom_cardinality = 100`) bypasses bloom for small results |
| Variable-length encoded terms complicate offset computation | Incorrect bloom checks | Sequential term parsing in Coprocessor (read type byte -> determine length -> skip); thorough fuzz testing |
| Coprocessor plugin deployment complexity | Ops burden | Provide pre-built plugin binary in container image; document TiKV `coprocessor.region-split-size` and plugin loading config |
| `block_on()` in evaluator blocks tokio runtime | Deadlock under concurrent queries | Use dedicated `Runtime` for Coprocessor calls (separate from server's async runtime), matching existing TiKV backend pattern |
