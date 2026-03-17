//! Formal `StorageBackend` trait defining the contract for pluggable storage backends.
//!
//! Per ADR-002, Oxigraph uses enum dispatch (not dynamic dispatch) for backend selection.
//! This trait exists to document the required interface and ensure new backends
//! (e.g., TiKV) implement the complete contract.
//!
//! Per ADR-003, all trait methods are synchronous. Backends backed by async I/O
//! (e.g., TiKV via `tikv-client`) must use `tokio::Runtime::block_on()` internally.
//!
//! # Architecture
//!
//! The storage layer has four main components:
//!
//! - **Storage** (factory): Creates readers, transactions, and bulk loaders
//! - **StorageReader** (read-only snapshot): Point-in-time consistent reads
//! - **StorageTransaction** (write-only): Buffered writes with atomic commit
//! - **StorageBulkLoader**: High-throughput batch ingestion
//!
//! Each backend implements these four components. The `StorageKind` enum in `mod.rs`
//! dispatches to the appropriate backend at runtime.

use crate::model::{GraphNameRef, NamedOrBlankNodeRef, QuadRef};
use crate::storage::error::StorageError;
use crate::storage::numeric_encoder::{EncodedQuad, EncodedTerm, StrHash, StrLookup};
use oxrdf::Quad;

/// Trait for the top-level storage factory.
///
/// Implementations must be `Clone + Send + Sync` (storage is shared across threads).
pub trait StorageBackend: Clone + Send + Sync + 'static {
    /// The reader type returned by `snapshot()`.
    type Reader<'a>: StorageBackendReader<'a>
    where
        Self: 'a;

    /// The write-only transaction type.
    type Transaction<'a>: StorageBackendTransaction<'a>
    where
        Self: 'a;

    /// The read+write transaction type.
    type ReadableTransaction<'a>: StorageBackendReadableTransaction<'a>
    where
        Self: 'a;

    /// The bulk loader type.
    type BulkLoader<'a>: StorageBackendBulkLoader<'a>
    where
        Self: 'a;

    /// Create a point-in-time snapshot reader.
    fn snapshot(&self) -> Self::Reader<'static>;

    /// Start a write-only transaction.
    fn start_transaction(&self) -> Result<Self::Transaction<'_>, StorageError>;

    /// Start a read+write transaction (supports both reads and writes within the same txn).
    fn start_readable_transaction(&self) -> Result<Self::ReadableTransaction<'_>, StorageError>;

    /// Create a bulk loader for high-throughput ingestion.
    fn bulk_loader(&self) -> Self::BulkLoader<'_>;
}

/// Trait for read-only snapshot access to the storage.
///
/// All reads reflect the state at the time the snapshot was taken.
/// Must implement `StrLookup` for dictionary (id2str) lookups.
pub trait StorageBackendReader<'a>: StrLookup + Sized {
    /// Iterator type for quad results.
    type QuadIter: Iterator<Item = Result<EncodedQuad, StorageError>>;

    /// Iterator type for graph name results.
    type GraphIter: Iterator<Item = Result<EncodedTerm, StorageError>>;

    /// Total number of quads in the store.
    fn len(&self) -> Result<usize, StorageError>;

    /// Whether the store contains any quads.
    fn is_empty(&self) -> Result<bool, StorageError>;

    /// Check if a specific quad exists.
    fn contains(&self, quad: &EncodedQuad) -> Result<bool, StorageError>;

    /// Query quads matching a pattern. `None` means "any value" (wildcard).
    ///
    /// This is the core query primitive. The backend must select the optimal
    /// index (SPO, POS, OSP, etc.) based on which components are bound.
    fn quads_for_pattern(
        &self,
        subject: Option<&EncodedTerm>,
        predicate: Option<&EncodedTerm>,
        object: Option<&EncodedTerm>,
        graph_name: Option<&EncodedTerm>,
    ) -> Self::QuadIter;

    /// Iterate over all named graphs.
    fn named_graphs(&self) -> Self::GraphIter;

    /// Check if a named graph exists.
    fn contains_named_graph(&self, graph_name: &EncodedTerm) -> Result<bool, StorageError>;

    /// Check if a string hash exists in the dictionary.
    fn contains_str(&self, key: &StrHash) -> Result<bool, StorageError>;

    /// Validate storage invariants (for debugging/integrity checks).
    fn validate(&self) -> Result<(), StorageError>;
}

/// Trait for write-only transactions.
///
/// Writes are buffered and applied atomically on `commit()`.
/// Dropping without calling `commit()` rolls back all changes.
pub trait StorageBackendTransaction<'a> {
    /// Insert a quad.
    fn insert(&mut self, quad: QuadRef<'_>);

    /// Insert a named graph (without any quads).
    fn insert_named_graph(&mut self, graph_name: NamedOrBlankNodeRef<'_>);

    /// Remove a quad.
    fn remove(&mut self, quad: QuadRef<'_>);

    /// Remove all quads from the default graph (keep the graph itself).
    fn clear_default_graph(&mut self);

    /// Remove all quads from all named graphs (keep the graphs themselves).
    fn clear_all_named_graphs(&mut self);

    /// Remove all quads from all graphs (default + named).
    fn clear_all_graphs(&mut self);

    /// Remove all named graphs and their quads.
    fn remove_all_named_graphs(&mut self);

    /// Remove everything (all quads and all named graphs).
    fn clear(&mut self);

    /// Atomically commit all buffered writes.
    fn commit(self) -> Result<(), StorageError>;
}

/// Trait for readable transactions (read + write within the same transaction).
///
/// Extends `StorageBackendTransaction` with the ability to read the current
/// state (including uncommitted writes from this transaction).
pub trait StorageBackendReadableTransaction<'a>: StorageBackendTransaction<'a> {
    /// The reader type for this transaction.
    type Reader<'b>: StorageBackendReader<'b>
    where
        Self: 'b;

    /// Get a reader that sees both committed state and this transaction's uncommitted writes.
    fn reader(&self) -> Self::Reader<'_>;

    /// Clear a specific graph (default or named).
    fn clear_graph(&mut self, graph_name: GraphNameRef<'_>) -> Result<(), StorageError>;

    /// Remove a specific named graph and its quads.
    fn remove_named_graph(
        &mut self,
        graph_name: NamedOrBlankNodeRef<'_>,
    ) -> Result<(), StorageError>;
}

/// Trait for high-throughput bulk loading.
///
/// Bulk loaders may bypass some transactional guarantees for performance
/// (e.g., writing directly to SST files in RocksDB).
pub trait StorageBackendBulkLoader<'a> {
    /// Register a progress callback (called periodically with count of loaded quads).
    fn on_progress(self, callback: impl Fn(u64) + Send + Sync + 'static) -> Self;

    /// Disable atomicity guarantees for higher throughput.
    fn without_atomicity(self) -> Self;

    /// Load a batch of quads.
    fn load_batch(
        &mut self,
        quads: Vec<Quad>,
        max_num_threads: usize,
    ) -> Result<(), StorageError>;

    /// Commit all loaded data.
    fn commit(self) -> Result<(), StorageError>;
}
