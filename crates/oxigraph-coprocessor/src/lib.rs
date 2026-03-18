//! TiKV Coprocessor plugin for Oxigraph query pushdown.
//!
//! This crate compiles to a `cdylib` (.so) that TiKV loads at runtime as a
//! Coprocessor plugin. It executes scan, filter, and aggregation operations
//! directly on Region-local data, avoiding network transfer of intermediate
//! results back to the query coordinator.
//!
//! # Architecture
//!
//! The plugin receives binary-encoded request messages (see [`protocol`])
//! and dispatches to one of four operations:
//!
//! - [`scan`]: Index scan with optional bloom filter semi-join pruning
//! - [`filter`]: Filter expression evaluation on encoded key bytes
//! - [`aggregate`]: COUNT / MIN / MAX pushdown
//! - [`bloom`]: Bloom filter deserialization and membership testing
//!
//! All operations navigate Oxigraph's variable-length encoded keys at the byte
//! level using the term navigation helpers, without deserializing to `EncodedTerm`.
//!
//! # Key Prefix Constants
//!
//! These must match the constants in `oxigraph/lib/oxigraph/src/storage/tikv.rs`
//! exactly. Each constant is a 1-byte table prefix prepended to every key in the
//! corresponding index.

pub mod aggregate;
pub mod bloom;
pub mod filter;
pub mod plugin_api;
pub mod protocol;
pub mod scan;

// ---------------------------------------------------------------------------
// Key prefix constants — must match tikv.rs TABLE_* constants exactly
// ---------------------------------------------------------------------------

/// Default table (metadata, version key, etc.)
pub const TABLE_DEFAULT: u8 = 0x00;
/// ID-to-string mapping for term dictionary lookups.
pub const TABLE_ID2STR: u8 = 0x01;
/// Subject-Predicate-Object-Graph index.
pub const TABLE_SPOG: u8 = 0x02;
/// Predicate-Object-Subject-Graph index.
pub const TABLE_POSG: u8 = 0x03;
/// Object-Subject-Predicate-Graph index.
pub const TABLE_OSPG: u8 = 0x04;
/// Graph-Subject-Predicate-Object index (named graphs).
pub const TABLE_GSPO: u8 = 0x05;
/// Graph-Predicate-Object-Subject index (named graphs).
pub const TABLE_GPOS: u8 = 0x06;
/// Graph-Object-Subject-Predicate index (named graphs).
pub const TABLE_GOSP: u8 = 0x07;
/// Default-graph Subject-Predicate-Object index.
pub const TABLE_DSPO: u8 = 0x08;
/// Default-graph Predicate-Object-Subject index.
pub const TABLE_DPOS: u8 = 0x09;
/// Default-graph Object-Subject-Predicate index.
pub const TABLE_DOSP: u8 = 0x0A;
/// Named graph registry.
pub const TABLE_GRAPHS: u8 = 0x0B;

// ---------------------------------------------------------------------------
// Term navigation helpers
// ---------------------------------------------------------------------------

/// Returns the total byte length of an encoded term starting at `data[0]`,
/// including the 1-byte type prefix.
///
/// This matches `binary_encoder.rs` `write_term` / `read_term` exactly.
/// The plugin uses this to skip over terms in a key without deserializing them.
pub fn encoded_term_len(data: &[u8]) -> Result<usize, &'static str> {
    if data.is_empty() {
        return Err("empty term data");
    }
    let total = match data[0] {
        // NamedNode: 1 type + 16 hash
        1 => 17,
        // NumericalBlankNode: 1 type + 16 id
        8 => 17,
        // SmallBlankNode: 1 type + 16 small_string
        9 => 17,
        // BigBlankNode: 1 type + 16 hash
        10 => 17,
        // SmallStringLiteral: 1 type + 16 small_string
        16 => 17,
        // BigStringLiteral: 1 type + 16 hash
        17 => 17,
        // SmallSmallLangStringLiteral: 1 type + 16 lang + 16 value
        20 => 33,
        // SmallBigLangStringLiteral: 1 type + 16 lang_hash + 16 value
        21 => 33,
        // BigSmallLangStringLiteral: 1 type + 16 lang + 16 value_hash
        22 => 33,
        // BigBigLangStringLiteral: 1 type + 16 lang_hash + 16 value_hash
        23 => 33,
        // SmallTypedLiteral: 1 type + 16 datatype_hash + 16 value
        24 => 33,
        // BigTypedLiteral: 1 type + 16 datatype_hash + 16 value_hash
        25 => 33,
        // BooleanLiteral true/false: 1 type only
        28 | 29 => 1,
        // FloatLiteral: 1 type + 4 float
        30 => 5,
        // DoubleLiteral: 1 type + 8 double
        31 => 9,
        // IntegerLiteral: 1 type + 8 i64
        32 => 9,
        // DecimalLiteral: 1 type + 16 decimal
        33 => 17,
        // DateTime, Time, Date, GYearMonth, GYear, GMonthDay, GDay, GMonth:
        // 1 type + 18 value
        34..=41 => 19,
        // DurationLiteral: 1 type + 24 value
        42 => 25,
        // YearMonthDuration: 1 type + 8 value
        43 => 9,
        // DayTimeDuration: 1 type + 16 value
        44 => 17,
        // Triple/StarTriple: 1 type + 3 recursive terms
        48 | 49 => {
            let mut offset = 1;
            for _ in 0..3 {
                let child_len = encoded_term_len(&data[offset..])?;
                offset += child_len;
            }
            offset
        }
        // RDF-12 directional lang string literals: 1 type + 16 + 16
        56..=63 => 33,
        _ => return Err("unknown term type byte"),
    };
    Ok(total)
}

/// Extract the byte slice for the Nth term (0-based) from a key buffer.
///
/// The key buffer must NOT include the 1-byte table prefix — strip it before
/// calling this function.
pub fn extract_term_bytes(key: &[u8], position: usize) -> Result<&[u8], &'static str> {
    let mut offset = 0;
    for i in 0..=position {
        if offset >= key.len() {
            return Err("key too short for requested term position");
        }
        let len = encoded_term_len(&key[offset..])?;
        if i == position {
            return Ok(&key[offset..offset + len]);
        }
        offset += len;
    }
    Err("term position not found")
}

/// Extract and concatenate multiple term byte slices from a key.
///
/// Used for multi-variable bloom filter checks where the filter key is the
/// concatenation of multiple term encodings.
pub fn extract_concat_term_bytes(key: &[u8], positions: &[u32]) -> Result<Vec<u8>, &'static str> {
    let mut result = Vec::new();
    for &pos in positions {
        let term_bytes = extract_term_bytes(key, pos as usize)?;
        result.extend_from_slice(term_bytes);
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Plugin entry point
// ---------------------------------------------------------------------------

/// Unique identifier for this Coprocessor plugin.
/// The client sends this as the coprocessor request's `tp` field.
pub const OXIGRAPH_COPROCESSOR_ID: i64 = 10001;

use plugin_api::{
    CoprocessorPlugin, Key, PluginError, PluginResult, RawRequest, RawResponse, RawStorage,
};
use protocol::{
    OpType, decode_request, encode_count_response, encode_min_max_response, encode_scan_response,
};
use scan::{IndexScanParams, execute_index_scan};
use std::ops::Range;

/// The Oxigraph TiKV Coprocessor plugin.
///
/// Dispatches incoming binary requests to the scan, filter, aggregate, or
/// min/max modules operating on Region-local data via [`RawStorage`].
#[derive(Default)]
pub struct OxigraphCoprocessorPlugin;

impl CoprocessorPlugin for OxigraphCoprocessorPlugin {
    fn on_raw_coprocessor_request(
        &self,
        ranges: Vec<Range<Key>>,
        request: RawRequest,
        storage: &dyn RawStorage,
    ) -> PluginResult<RawResponse> {
        let req = decode_request(&request).map_err(|e| {
            PluginError::Other(
                format!("failed to decode request: {e}"),
                Box::new(()),
            )
        })?;

        // Collect all KV pairs from the requested ranges via RawStorage.
        let mut all_pairs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for range in &ranges {
            let pairs = storage.scan(range.clone())?;
            all_pairs.extend(pairs);
        }

        match req.op_type {
            OpType::IndexScan => {
                let params = IndexScanParams {
                    table_prefix: req.table_prefix,
                    key_prefix: req.key_prefix,
                    limit: 0,
                    bloom_filter: req.bloom_filter,
                    bloom_positions: vec![],
                };
                let result = execute_index_scan(
                    &params,
                    all_pairs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())),
                );
                Ok(encode_scan_response(result.scanned_keys, &result.pairs))
            }
            OpType::FilterScan => {
                // FilterScan uses the same index scan but applies byte-level
                // filter predicates. For now, delegate to IndexScan (filter
                // predicates would be encoded in an extended request format).
                let params = IndexScanParams {
                    table_prefix: req.table_prefix,
                    key_prefix: req.key_prefix,
                    limit: 0,
                    bloom_filter: req.bloom_filter,
                    bloom_positions: vec![],
                };
                let result = execute_index_scan(
                    &params,
                    all_pairs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())),
                );
                Ok(encode_scan_response(result.scanned_keys, &result.pairs))
            }
            OpType::CountScan => {
                let result = aggregate::execute_count(
                    req.table_prefix,
                    &req.key_prefix,
                    all_pairs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())),
                );
                Ok(encode_count_response(result.scanned_keys, result.count))
            }
            OpType::MinMaxScan => {
                let result = aggregate::execute_min_max(
                    req.table_prefix,
                    &req.key_prefix,
                    all_pairs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())),
                );
                Ok(encode_min_max_response(
                    result.scanned_keys,
                    result.min_key.as_deref(),
                    result.max_key.as_deref(),
                ))
            }
        }
    }
}

declare_plugin!(OxigraphCoprocessorPlugin::default());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_len_named_node() {
        // Type byte 1 (NamedNode) followed by 16 bytes of hash
        let mut data = vec![1u8];
        data.extend_from_slice(&[0u8; 16]);
        assert_eq!(encoded_term_len(&data), Ok(17));
    }

    #[test]
    fn term_len_boolean() {
        assert_eq!(encoded_term_len(&[28]), Ok(1)); // true
        assert_eq!(encoded_term_len(&[29]), Ok(1)); // false
    }

    #[test]
    fn term_len_integer() {
        let mut data = vec![32u8];
        data.extend_from_slice(&[0u8; 8]);
        assert_eq!(encoded_term_len(&data), Ok(9));
    }

    #[test]
    fn term_len_unknown_type_byte() {
        assert_eq!(encoded_term_len(&[255]), Err("unknown term type byte"));
    }

    #[test]
    fn term_len_empty() {
        assert_eq!(encoded_term_len(&[]), Err("empty term data"));
    }

    #[test]
    fn extract_term_at_position() {
        // Build a key with 3 terms: NamedNode(17) + Boolean(1) + Integer(9)
        let mut key = Vec::new();
        // Term 0: NamedNode (type=1, 16 bytes hash)
        key.push(1);
        key.extend_from_slice(&[0xAA; 16]);
        // Term 1: BooleanLiteral true (type=28)
        key.push(28);
        // Term 2: IntegerLiteral (type=32, 8 bytes)
        key.push(32);
        key.extend_from_slice(&[0xBB; 8]);

        let term0 = extract_term_bytes(&key, 0).expect("term 0");
        assert_eq!(term0.len(), 17);
        assert_eq!(term0[0], 1);

        let term1 = extract_term_bytes(&key, 1).expect("term 1");
        assert_eq!(term1, &[28]);

        let term2 = extract_term_bytes(&key, 2).expect("term 2");
        assert_eq!(term2.len(), 9);
        assert_eq!(term2[0], 32);
    }

    #[test]
    fn extract_term_out_of_bounds() {
        let key = vec![28u8]; // Single boolean term
        assert!(extract_term_bytes(&key, 1).is_err());
    }

    #[test]
    fn extract_concat_terms() {
        let mut key = Vec::new();
        // Term 0: BooleanLiteral true
        key.push(28);
        // Term 1: BooleanLiteral false
        key.push(29);
        // Term 2: NamedNode
        key.push(1);
        key.extend_from_slice(&[0xCC; 16]);

        let concat = extract_concat_term_bytes(&key, &[0, 2]).expect("concat");
        // Should be term0 (1 byte) + term2 (17 bytes) = 18 bytes
        assert_eq!(concat.len(), 18);
        assert_eq!(concat[0], 28);
        assert_eq!(concat[1], 1);
    }

    #[test]
    fn table_constants_match_tikv_rs() {
        // These values must match oxigraph/lib/oxigraph/src/storage/tikv.rs
        assert_eq!(TABLE_DEFAULT, 0x00);
        assert_eq!(TABLE_ID2STR, 0x01);
        assert_eq!(TABLE_SPOG, 0x02);
        assert_eq!(TABLE_POSG, 0x03);
        assert_eq!(TABLE_OSPG, 0x04);
        assert_eq!(TABLE_GSPO, 0x05);
        assert_eq!(TABLE_GPOS, 0x06);
        assert_eq!(TABLE_GOSP, 0x07);
        assert_eq!(TABLE_DSPO, 0x08);
        assert_eq!(TABLE_DPOS, 0x09);
        assert_eq!(TABLE_DOSP, 0x0A);
        assert_eq!(TABLE_GRAPHS, 0x0B);
    }
}
