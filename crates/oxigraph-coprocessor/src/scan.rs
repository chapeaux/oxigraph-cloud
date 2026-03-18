//! Index scan and filter scan pushdown execution.
//!
//! These functions run inside TiKV's Coprocessor plugin framework, operating
//! on the local Region's RocksDB data. They navigate Oxigraph's variable-length
//! encoded keys to extract, filter, and project terms without full deserialization.
//!
//! # Operations
//!
//! - **`INDEX_SCAN`**: Prefix scan with optional bloom filter pruning and term projection.
//! - **`FILTER_SCAN`**: Like INDEX_SCAN but also evaluates a pushed-down filter expression
//!   (equality, range, type checks, boolean combinators) on encoded term bytes.

// TODO: Implement when TiKV coprocessor_plugin_api dependency is available.
// See docs/coprocessor-implementation-plan.md Section 2.5 for full implementation.
//
// pub fn execute_index_scan(
//     req: &OxigraphCoprocessorRequest,
//     ranges: &[Range],
//     storage: &dyn RawStorage,
// ) -> PluginResult<OxigraphCoprocessorResponse> { ... }
//
// pub fn execute_filter_scan(
//     req: &OxigraphCoprocessorRequest,
//     ranges: &[Range],
//     storage: &dyn RawStorage,
// ) -> PluginResult<OxigraphCoprocessorResponse> { ... }
//
// fn evaluate_filter(expr: &FilterExpr, key_body: &[u8]) -> PluginResult<bool> { ... }
