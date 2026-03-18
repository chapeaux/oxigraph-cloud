//! Filter expression evaluation for Coprocessor pushdown.
//!
//! Evaluates `FilterExpr` trees against encoded key bytes at the TiKV Region level.
//! Supported filter operations:
//!
//! - **EQ / NEQ**: Exact byte-level equality on an encoded term at a given position.
//! - **GT / GTE / LT / LTE**: Lexicographic range comparison on encoded term bytes.
//!   Valid for types whose encoding preserves sort order (numerics, datetime).
//! - **TYPE_CHECK**: Verify the leading type-discriminant byte falls within a range
//!   (e.g., "is this term a NamedNode?" checks type byte == 1).
//! - **BOUND_CHECK**: Check if a term slot is non-empty (always true for fixed keys).
//! - **AND / OR / NOT**: Boolean combinators over child `FilterExpr` nodes.
//!
//! All comparisons operate on raw encoded bytes, avoiding deserialization to
//! `EncodedTerm` for maximum throughput in the Coprocessor hot path.

// TODO: Implement when TiKV coprocessor_plugin_api dependency is available.
// See docs/coprocessor-implementation-plan.md Section 2.5 (evaluate_filter) for
// the full implementation. The filter evaluator is called from scan::execute_filter_scan.
