//! Aggregation pushdown execution (COUNT, MIN, MAX).
//!
//! These functions run inside TiKV's Coprocessor plugin and return scalar
//! aggregation results per Region, which the client merges:
//!
//! - **`COUNT_SCAN`**: Counts all keys matching the scan prefix within this Region.
//!   The client sums partial counts from all Regions covering the prefix range.
//!
//! - **`MIN_MAX_SCAN`**: Returns the lexicographically smallest (MIN) or largest
//!   (MAX) key within the scan prefix range in this Region. The client then takes
//!   the global min/max across Region-level results.
//!
//! Since Oxigraph's term encoding preserves sort order for numeric and temporal
//! types, lexicographic MIN/MAX on encoded keys corresponds to semantic MIN/MAX
//! for those types.

// TODO: Implement when TiKV coprocessor_plugin_api dependency is available.
// See docs/coprocessor-implementation-plan.md Section 2.6 for full implementation.
//
// pub fn execute_count_scan(
//     req: &OxigraphCoprocessorRequest,
//     ranges: &[Range],
//     storage: &dyn RawStorage,
// ) -> PluginResult<OxigraphCoprocessorResponse> { ... }
//
// pub fn execute_min_max_scan(
//     req: &OxigraphCoprocessorRequest,
//     ranges: &[Range],
//     storage: &dyn RawStorage,
// ) -> PluginResult<OxigraphCoprocessorResponse> { ... }
