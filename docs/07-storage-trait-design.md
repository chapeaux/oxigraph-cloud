# Storage Trait Design: Engineering the Abstraction Layer

## Current State

Oxigraph isolates persistence behind a `Store` struct with hardcoded enum dispatch between in-memory and RocksDB backends. This rigid coupling prevents pluggable external backends.

Reference: https://github.com/oxigraph/oxigraph/discussions/1487

## Required Refactoring

### StorageBackend Trait

Introduce a generic `StorageBackend` trait with:

**Standard CRUD**:
- `get(key) -> Option<Value>`
- `put(key, value)`
- `delete(key)`

**Batched Variants** (critical for distributed backends to minimize network round-trips):
- `batch_put(entries)`
- `batch_scan(prefix) -> Iterator`

**Transaction Support**:
- Snapshot-based reads
- Atomic batch commits

### Async I/O Considerations

Distributed backends require async I/O. The trait will need:
- **Generic Associated Types (GATs)** for async iterators
- `Pin<Box<dyn Future>>` for managing transaction lifetimes across threads
- Careful design to satisfy Rust's borrow checker

### Design Precedents

| Project | Pattern |
|---------|---------|
| `agentdb` | Traits abstracting SQL, KV, and Graph backends behind unified API |
| `confidentialcontainers` | Generic `StorageBackend` trait for secure storage |

## Backend Implementations

The trait enables multiple backend implementations:

| Backend | Use Case |
|---------|----------|
| **RocksDB** | Embedded, single-node, edge computing (preserved as default) |
| **TiKV** | Distributed, cloud-native, production Kubernetes deployments |
| **In-Memory** | Testing, ephemeral workloads |

## Implementation Strategy

1. Define the `StorageBackend` trait with sync + async variants
2. Refactor existing RocksDB code to implement the trait
3. Implement TiKV backend using `tikv-client` crate
4. Wire the trait into `Store` struct using generics or dynamic dispatch
5. Implement SRDF trait for rudof integration against the abstracted Store
