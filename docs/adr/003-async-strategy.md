# ADR-003: Async Strategy for StorageBackend

## Status: Accepted

## Context

The TiKV client (`tikv-client` crate) is fully async, built on tokio. Every `get`, `put`, `scan`, and transaction operation returns a `Future` that must be `.await`-ed within a tokio runtime.

Oxigraph's query engine (`spareval`) is fundamentally synchronous and iterator-based. SPARQL evaluation proceeds by calling `next()` on iterators that chain through BGP evaluation, joins, filters, and projections. This iterator pipeline assumes each `next()` call can synchronously fetch the next key-value pair from storage.

These two models are in direct tension. The async strategy determines how we bridge them without either (a) rewriting Oxigraph's entire query engine to be async, or (b) sacrificing TiKV's async performance.

## Options Considered

### (a) Fully Async Trait with GATs

Define `StorageBackend` as an async trait using Generic Associated Types for async iterators:

```rust
trait StorageBackend {
    type ScanStream<'a>: Stream<Item = Result<(Vec<u8>, Vec<u8>)>> + 'a;
    async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    async fn scan(&self, prefix: &[u8]) -> Self::ScanStream<'_>;
}
```

- **Pros**: Native async from storage through to the network layer. No `block_on` overhead. Enables true concurrent I/O when multiple BGPs can be evaluated in parallel. Future-proof for async Rust ecosystem evolution.
- **Cons**: **Catastrophic async infection.** Every caller of `StorageBackend` must become async. This cascades through Oxigraph's iterator-based query evaluator, which calls storage in `next()` implementations on `Iterator` trait impls. Converting these to `Stream` would require rewriting the entire `spareval` crate -- hundreds of iterator adapters, join implementations, and aggregation pipelines. This is effectively a rewrite of Oxigraph's query engine, estimated at months of work with high regression risk. The RocksDB and in-memory backends gain nothing from being async (they complete in microseconds). GATs for async iterators, while stabilized, still have rough edges in practice (lifetime inference, trait object compatibility).

### (b) Sync Trait + `block_on` Bridge for TiKV

Define `StorageBackend` as a synchronous trait. The TiKV implementation internally uses `tokio::runtime::Handle::block_on()` to bridge async TiKV calls into sync returns:

```rust
trait StorageBackend {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    fn scan_prefix(&self, prefix: &[u8]) -> Box<dyn Iterator<Item = Result<...>>>;
}

impl StorageBackend for TiKvBackend {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.runtime.block_on(self.client.get(key.to_vec()))
    }
}
```

- **Pros**: Zero changes to Oxigraph's query engine. The iterator pipeline works unchanged. RocksDB and in-memory backends are naturally sync. Simple to implement -- the TiKV backend holds a `tokio::Runtime` handle and blocks on each async call. This is the approach used by many projects that integrate async clients into sync codebases (e.g., `reqwest::blocking`).
- **Cons**: `block_on` inside an async context panics (cannot nest runtimes without `spawn_blocking`). Each `block_on` call occupies a thread while waiting for the network response, limiting concurrency. For sequential iterator `next()` calls, each call incurs a full network round-trip with no pipelining -- this is the key performance concern. Batch operations (`batch_scan`, `batch_put`) amortize this but individual `next()` calls on scan iterators cannot.

### (c) Dual Sync/Async Traits

Define both a sync and async version of the trait:

```rust
trait StorageBackend { fn get(...) -> Result<...>; }
trait AsyncStorageBackend { async fn get(...) -> Result<...>; }
```

- **Pros**: Each consumer picks the appropriate trait. The query engine uses sync; the HTTP layer or batch loader could use async directly.
- **Cons**: Double the API surface. Every backend must implement both traits (or one wraps the other, adding complexity). The sync trait still needs `block_on` for TiKV, so this doesn't solve the core problem -- it just adds an async fast path for callers that can use it. The query engine, which is the primary consumer, still uses the sync path. Significant maintenance burden for marginal benefit.

## Decision

**Sync trait + `block_on` bridge (option b), with buffered prefetching to mitigate the round-trip penalty.**

The fundamental constraint is Oxigraph's synchronous, iterator-based query engine. Rewriting it to be async (option a) is a multi-month effort that would diverge so far from upstream Oxigraph that merges become impractical -- directly conflicting with our fork strategy (ADR-001). Dual traits (option c) add complexity without solving the core problem since the query engine is the primary storage consumer.

The `block_on` approach works because:

1. **The query engine runs on dedicated threads, not inside tokio tasks.** We control the server architecture. SPARQL query evaluation runs on a thread pool (e.g., `rayon` or a dedicated `tokio::spawn_blocking` pool), not on tokio worker threads. This avoids the nested-runtime panic.

2. **The real performance concern is iterator `next()` latency, and we mitigate it with prefetch buffers.** The TiKV backend's `scan_prefix` implementation will prefetch a batch of key-value pairs (e.g., 256 or 1024 at a time) in a single async `batch_scan` call, buffer them locally, and serve subsequent `next()` calls from the buffer. This converts N sequential round-trips into N/batch_size round-trips. This is the same pattern TiDB uses internally.

3. **Point operations (`get`, `put`) are already batched by Oxigraph's transaction model.** Writes are buffered and committed as a batch. Reads during query evaluation are predominantly scans, not individual gets, so the prefetch buffer covers the dominant case.

4. **This preserves the path to Coprocessor pushdown (Phase 4).** Coprocessor pushdown replaces scan-iterate patterns with single RPC calls that return aggregated results. As we implement pushdown, the iterator-level performance concern diminishes because fewer individual KV operations are needed.

Implementation details:

- The TiKV backend holds a `tokio::Runtime` (created at startup, shared across connections).
- `StorageBackend::scan_prefix` returns a `PrefetchIterator` that holds an internal buffer and refills it via `block_on(client.batch_scan(...))` when exhausted.
- Prefetch buffer size is configurable (default 512 entries, tunable per workload).
- `StorageBackend::get` uses `block_on(client.get(...))` directly -- single point gets have acceptable latency (sub-millisecond on local TiKV, single-digit ms on networked TiKV).
- Batch writes use `block_on(client.batch_put(...))` -- already naturally batched.

## Consequences

- Oxigraph's query engine (`spareval`, `sparopt`) requires **zero modifications** for the storage abstraction. All iterator-based evaluation works unchanged.
- The TiKV backend's scan performance depends heavily on prefetch buffer tuning. Phase 2 benchmarks (task 2.7) must measure the impact of different buffer sizes.
- SPARQL query evaluation threads must not run inside the tokio runtime's async context. The server architecture must ensure query processing happens on blocking threads (e.g., `actix_web::web::block`, `tokio::spawn_blocking`, or a separate `rayon` thread pool).
- If Oxigraph upstream ever adopts async iterators (unlikely in near term), we can migrate the trait. The sync-to-async direction is easier than async-to-sync.
- The Coprocessor pushdown work in Phase 4 will further reduce the importance of iterator-level latency, as whole BGP evaluations become single RPCs rather than iterated scans.
- Risk R4 from the PLAN (async conversion cascading through the codebase) is fully mitigated by this decision.
