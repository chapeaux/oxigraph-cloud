# ADR-002: Generics vs Dynamic Dispatch for StorageBackend

## Status: Accepted

## Context

Phase 1 requires wiring the `StorageBackend` trait into Oxigraph's `Store` struct. The dispatch mechanism determines how `Store` invokes backend operations and has cascading effects on the entire API surface, binary size, and runtime performance.

Oxigraph's `Store` is currently used directly by the SPARQL query engine (`spareval`), the HTTP server, and downstream consumers. Whichever dispatch mechanism we choose will ripple through all of these layers.

The hot path is key-value operations: `get`, `put`, `batch_scan`. These are called millions of times during query evaluation and bulk loading. The dispatch mechanism must not introduce unacceptable overhead on these operations.

## Options Considered

### (a) Full Generics: `Store<B: StorageBackend>`

Make `Store` generic over the backend type.

- **Pros**: Zero-cost abstraction. The compiler monomorphizes each backend, inlining `get`/`put`/`scan` calls. No vtable indirection on the hot path. Type safety ensures backend-specific capabilities can be expressed at the type level.
- **Cons**: Type parameter infection. `Store<B>` propagates to every struct and function that holds or uses a `Store`: the SPARQL evaluator, the HTTP handler, transaction wrappers, and the SRDF bridge for rudof. This is a massive API change touching dozens of files. It also means binary bloat -- each backend produces a separate monomorphization of the entire query engine. Testing and library ergonomics suffer when every function signature carries `<B: StorageBackend>`.

### (b) Dynamic Dispatch: `Box<dyn StorageBackend>`

Store holds a trait object.

- **Pros**: Simple API. `Store` remains a concrete type with no type parameters. Adding new backends requires no changes to downstream code. Clean library boundary -- consumers don't need to be generic.
- **Cons**: Vtable indirection on every KV operation. For `get`/`put` this is a single indirect call (nanoseconds), which is negligible compared to network round-trip time for TiKV (microseconds to milliseconds). However, for the RocksDB backend, where operations complete in microseconds, the vtable overhead is proportionally more significant. Also prevents inlining of hot-path backend calls for the embedded case. Iterator returns require `Box<dyn Iterator>`, adding allocation overhead per scan.

### (c) Enum Dispatch

Replace the trait object with an enum:
```rust
enum Backend {
    RocksDb(RocksDbBackend),
    InMemory(InMemoryBackend),
    TiKv(TiKvBackend),
}
```

- **Pros**: No vtable indirection -- the compiler can optimize match arms, especially when only one variant is used at runtime. No type parameter infection. Known set of backends allows exhaustive matching. The `enum_dispatch` crate can auto-generate the boilerplate. This is close to what Oxigraph already does internally (its current `StorageReader` is an enum over RocksDB and in-memory).
- **Cons**: Not truly open for extension -- adding a new backend requires modifying the enum (though this is acceptable since backends are a controlled set). Slightly larger binary than pure dynamic dispatch since all backend code is always compiled in. Cannot be used across crate boundaries without the enum living in a shared crate.

## Decision

**Enum dispatch (option c).**

This is the pragmatic middle ground for this project, and it aligns with Oxigraph's existing architectural pattern:

1. **Oxigraph already uses enum dispatch** between RocksDB and in-memory backends. Extending this pattern to include TiKV is the smallest conceptual change and minimizes disruption to the existing codebase.

2. **The set of backends is closed and controlled.** We are building three backends (RocksDB, in-memory, TiKV). There is no realistic need for arbitrary third-party backends -- this is not a general-purpose storage library. If a new backend is ever needed, adding an enum variant is trivial.

3. **No type parameter infection.** The `Store` struct, SPARQL evaluator, HTTP server, and SRDF bridge all remain concrete types. This avoids the massive API refactoring that generics would require.

4. **Performance is competitive with generics.** The compiler can optimize single-variant match arms to near-zero overhead. For the common case where a deployment uses exactly one backend, branch prediction makes the match effectively free. For the TiKV backend, the network round-trip dominates any dispatch overhead by 3-4 orders of magnitude.

5. **The `StorageBackend` trait still exists** as the contract that each backend must implement. The enum simply wraps implementations of this trait. If we ever need to move to generics (e.g., for a library use case), the trait is already defined and the refactoring is mechanical.

The implementation approach:

```rust
pub trait StorageBackend {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()>;
    // ... etc
}

pub(crate) enum BackendStore {
    RocksDb(RocksDbBackend),
    InMemory(InMemoryBackend),
    TiKv(TiKvBackend),
}

// Use enum_dispatch or manual impl forwarding
impl StorageBackend for BackendStore { ... }
```

## Consequences

- `Store` remains a concrete, non-generic struct. All existing Oxigraph API consumers are unaffected at the type level.
- Adding a new backend requires: (1) implementing `StorageBackend`, (2) adding an enum variant, (3) adding match arms (or using `enum_dispatch` to auto-generate them).
- The TiKV backend code is always compiled into the binary. If binary size is a concern for embedded use cases, we can gate variants behind cargo features (`#[cfg(feature = "tikv")]`).
- The `StorageBackend` trait serves as documentation and contract, even though dispatch goes through the enum. This keeps the door open for a future generics migration if the library is published for external consumption.
- Phase 1 task 1.6 is significantly simplified: we extend the existing enum pattern rather than rewriting `Store`'s type signature.
