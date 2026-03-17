# TiKV Coprocessor Plugin: Implementation Plan

> **Phase 4 Task 6** | **Status**: Ready for implementation | **Date**: 2026-03-17

This document provides everything a developer needs to implement the Oxigraph TiKV Coprocessor plugin, from protobuf schemas to deployment instructions.

---

## 1. Protobuf Schema

File: `oxigraph-tikv-coprocessor/proto/oxigraph_coprocessor.proto`

```protobuf
syntax = "proto3";

package oxigraph.coprocessor;

// ============================================================================
// REQUEST
// ============================================================================

message OxigraphCoprocessorRequest {
    // The operation to perform on this Region's data.
    OpType op_type = 1;

    // 1-byte table prefix (e.g., 0x08 for DSPO, 0x09 for DPOS) concatenated
    // with the encoded term prefix bytes that define the scan range's lower bound.
    // Example for "all triples with predicate P in default graph":
    //   [0x09] ++ encode_term(P)
    bytes scan_prefix = 2;

    // Explicit exclusive upper bound. If empty, derived from scan_prefix via
    // the increment-last-byte algorithm (prefix_upper_bound).
    bytes scan_upper_bound = 3;

    // Maximum number of results to return. 0 = unlimited.
    uint32 limit = 4;

    // Optional filter to apply during scan (for FILTER_SCAN).
    FilterExpr filter = 5;

    // For MIN_MAX_SCAN: true = MAX (reverse scan), false = MIN (forward scan).
    bool is_max = 6;

    // Positions of terms to extract from each scanned key and return.
    // Each value is 0-based: 0 = first term in key, 1 = second, 2 = third, etc.
    // For DSPO keys: 0=subject, 1=predicate, 2=object.
    // For DPOS keys: 0=predicate, 1=object, 2=subject.
    // If empty, full keys are returned.
    repeated uint32 return_term_positions = 7;

    // Optional bloom filter for semi-join pushdown (serialized bit vector).
    bytes bloom_filter = 8;

    // Number of hash functions used in the bloom filter.
    uint32 bloom_num_hashes = 9;

    // Which term position(s) in the scanned key to check against the bloom filter.
    // For a single-variable semi-join on ?person via DPOS index where ?person
    // is at position 2 (third term): bloom_check_positions = [2].
    // For multi-variable joins, the extracted terms are concatenated before checking.
    repeated uint32 bloom_check_positions = 10;

    enum OpType {
        // Prefix range scan, return matching keys (optionally projected via return_term_positions).
        INDEX_SCAN = 0;

        // Prefix range scan with filter predicate applied per key.
        FILTER_SCAN = 1;

        // Count matching keys in this Region. Returns count in response.count.
        COUNT_SCAN = 2;

        // Return the minimum or maximum key in the prefix range.
        MIN_MAX_SCAN = 3;
    }
}

// A filter expression tree evaluated against encoded key bytes.
// All comparisons operate directly on the binary-encoded term representation
// without deserializing to RDF terms.
message FilterExpr {
    FilterOp op = 1;

    // Which term position in the key this filter operates on (0-based).
    // For DPOS: 0=predicate, 1=object, 2=subject.
    uint32 term_position = 2;

    // Encoded term bytes to compare against (for EQ, NEQ, GT, GTE, LT, LTE).
    bytes constant_value = 3;

    // Child expressions for AND, OR, NOT.
    repeated FilterExpr children = 4;

    // For TYPE_CHECK: inclusive range of type bytes.
    // Named nodes: 1-7, blank nodes: 8-15, literals: 16-47, triples: 48-55.
    uint32 type_byte_min = 5;
    uint32 type_byte_max = 6;

    enum FilterOp {
        EQ = 0;
        NEQ = 1;
        GT = 2;
        GTE = 3;
        LT = 4;
        LTE = 5;
        AND = 6;
        OR = 7;
        NOT = 8;
        // Checks if the term's leading type byte falls within [type_byte_min, type_byte_max].
        TYPE_CHECK = 9;
        // Checks if the term slot is non-empty (always true for fixed-position keys).
        BOUND_CHECK = 10;
    }
}

// ============================================================================
// RESPONSE
// ============================================================================

message OxigraphCoprocessorResponse {
    // For INDEX_SCAN / FILTER_SCAN: each entry is a concatenation of the
    // requested return_term_positions' encoded bytes, or the full key if
    // return_term_positions was empty.
    repeated bytes results = 1;

    // For COUNT_SCAN: the number of matching keys in this Region.
    uint64 count = 2;

    // For MIN_MAX_SCAN: the encoded key bytes of the min or max entry.
    bytes min_max_value = 3;

    // Error message if processing failed within this Region.
    string error = 4;
}
```

Generate Rust code from this with `prost-build` in `build.rs`.

---

## 2. TiKV Plugin Crate Structure

### 2.1 Crate Layout

```
oxigraph-tikv-coprocessor/
  Cargo.toml
  build.rs                          # prost-build for .proto
  proto/
    oxigraph_coprocessor.proto
  src/
    lib.rs                          # Plugin entry point (CoprocessorPlugin impl)
    scan.rs                         # IndexScan and FilterScan execution
    aggregate.rs                    # CountScan, MinMaxScan execution
    bloom.rs                        # Bloom filter deserialization and checking
    term_nav.rs                     # Navigate encoded terms within key bytes
    proto_gen.rs                    # Re-export of generated protobuf types
```

### 2.2 Cargo.toml

```toml
[package]
name = "oxigraph-tikv-coprocessor"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]  # Produces .so for TiKV plugin loading

[dependencies]
tikv-client = { version = "0.3" }           # For coprocessor_plugin_api types
coprocessor_plugin_api = { version = "0.1" } # TiKV plugin trait
prost = "0.13"
bytes = "1"

[build-dependencies]
prost-build = "0.13"
```

**Note**: The exact `coprocessor_plugin_api` version must match the target TiKV release. As of TiKV 7.x, the plugin API is accessed via the `tikv_coprocessor_plugin_api` crate from the TiKV repository. Pin to the same TiKV commit used in deployment.

### 2.3 Plugin Entry Point (`src/lib.rs`)

```rust
use coprocessor_plugin_api::*;
use prost::Message;

mod scan;
mod aggregate;
mod bloom;
mod term_nav;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/oxigraph.coprocessor.rs"));
}

use proto::{OxigraphCoprocessorRequest, OxigraphCoprocessorResponse};
use proto::oxigraph_coprocessor_request::OpType;

/// Unique identifier for this coprocessor plugin.
/// The client sends this as the coprocessor request's `tp` field.
const OXIGRAPH_COPROCESSOR_ID: i64 = 10001;

#[derive(Default)]
pub struct OxigraphCoprocessorPlugin;

impl CoprocessorPlugin for OxigraphCoprocessorPlugin {
    fn on_raw_coprocessor_request(
        &self,
        ranges: Vec<Range>,
        request: RawRequest,
        storage: &dyn RawStorage,
    ) -> PluginResult<RawResponse> {
        // Decode the protobuf request
        let req = OxigraphCoprocessorRequest::decode(request.as_ref())
            .map_err(|e| PluginError::Other(format!("failed to decode request: {e}")))?;

        let response = match req.op_type() {
            OpType::IndexScan => scan::execute_index_scan(&req, &ranges, storage),
            OpType::FilterScan => scan::execute_filter_scan(&req, &ranges, storage),
            OpType::CountScan => aggregate::execute_count_scan(&req, &ranges, storage),
            OpType::MinMaxScan => aggregate::execute_min_max_scan(&req, &ranges, storage),
        }?;

        // Encode response
        let mut buf = Vec::with_capacity(response.encoded_len());
        response.encode(&mut buf)
            .map_err(|e| PluginError::Other(format!("failed to encode response: {e}")))?;
        Ok(buf)
    }
}

// TiKV plugin registration macro
declare_plugin!(OxigraphCoprocessorPlugin::default());
```

### 2.4 Term Navigation (`src/term_nav.rs`)

This module navigates Oxigraph's variable-length encoded terms within key bytes without deserializing them to `EncodedTerm`. This is the performance-critical path.

```rust
/// Encoded term byte sizes by type byte.
/// Matches binary_encoder.rs write_term / read_term exactly.
///
/// Returns the total byte length of an encoded term starting at `data[0]`,
/// including the 1-byte type prefix.
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
/// The key buffer must NOT include the 1-byte table prefix.
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
/// Used for multi-variable bloom filter checks.
pub fn extract_concat_term_bytes(key: &[u8], positions: &[u32]) -> Result<Vec<u8>, &'static str> {
    let mut result = Vec::new();
    for &pos in positions {
        let term_bytes = extract_term_bytes(key, pos as usize)?;
        result.extend_from_slice(term_bytes);
    }
    Ok(result)
}
```

### 2.5 Scan Execution (`src/scan.rs`)

```rust
use crate::proto::*;
use crate::bloom::BloomFilter;
use crate::term_nav::{extract_term_bytes, extract_concat_term_bytes};
use coprocessor_plugin_api::*;

pub fn execute_index_scan(
    req: &OxigraphCoprocessorRequest,
    ranges: &[Range],
    storage: &dyn RawStorage,
) -> PluginResult<OxigraphCoprocessorResponse> {
    let bloom = if !req.bloom_filter.is_empty() {
        Some(BloomFilter::from_bytes(&req.bloom_filter, req.bloom_num_hashes))
    } else {
        None
    };

    let limit = if req.limit > 0 { req.limit as usize } else { usize::MAX };
    let mut results = Vec::new();

    for range in ranges {
        // TiKV provides the range already clipped to this Region's boundaries
        let mut iter = storage.scan(range)?;
        while let Some((key, _value)) = iter.next()? {
            // Strip the 1-byte table prefix for term navigation
            let key_body = &key[1..];

            // Apply bloom filter check if present
            if let Some(ref bf) = bloom {
                let check_bytes = if req.bloom_check_positions.len() == 1 {
                    extract_term_bytes(key_body, req.bloom_check_positions[0] as usize)
                        .map_err(|e| PluginError::Other(e.to_string()))?
                        .to_vec()
                } else {
                    extract_concat_term_bytes(key_body, &req.bloom_check_positions)
                        .map_err(|e| PluginError::Other(e.to_string()))?
                };
                if !bf.check(&check_bytes) {
                    continue; // Bloom says definitely not in build side
                }
            }

            // Project requested term positions, or return full key
            let result_bytes = if req.return_term_positions.is_empty() {
                key.to_vec()
            } else {
                let mut projected = Vec::new();
                for &pos in &req.return_term_positions {
                    let term = extract_term_bytes(key_body, pos as usize)
                        .map_err(|e| PluginError::Other(e.to_string()))?;
                    projected.extend_from_slice(term);
                }
                projected
            };

            results.push(result_bytes);
            if results.len() >= limit {
                break;
            }
        }
        if results.len() >= limit {
            break;
        }
    }

    Ok(OxigraphCoprocessorResponse {
        results,
        ..Default::default()
    })
}

pub fn execute_filter_scan(
    req: &OxigraphCoprocessorRequest,
    ranges: &[Range],
    storage: &dyn RawStorage,
) -> PluginResult<OxigraphCoprocessorResponse> {
    let filter = req.filter.as_ref()
        .ok_or_else(|| PluginError::Other("FILTER_SCAN requires a filter expression".into()))?;
    let bloom = if !req.bloom_filter.is_empty() {
        Some(BloomFilter::from_bytes(&req.bloom_filter, req.bloom_num_hashes))
    } else {
        None
    };

    let limit = if req.limit > 0 { req.limit as usize } else { usize::MAX };
    let mut results = Vec::new();

    for range in ranges {
        let mut iter = storage.scan(range)?;
        while let Some((key, _value)) = iter.next()? {
            let key_body = &key[1..];

            // Bloom check
            if let Some(ref bf) = bloom {
                let check_bytes = extract_concat_term_bytes(key_body, &req.bloom_check_positions)
                    .map_err(|e| PluginError::Other(e.to_string()))?;
                if !bf.check(&check_bytes) {
                    continue;
                }
            }

            // Filter check
            if !evaluate_filter(filter, key_body)? {
                continue;
            }

            let result_bytes = if req.return_term_positions.is_empty() {
                key.to_vec()
            } else {
                let mut projected = Vec::new();
                for &pos in &req.return_term_positions {
                    let term = extract_term_bytes(key_body, pos as usize)
                        .map_err(|e| PluginError::Other(e.to_string()))?;
                    projected.extend_from_slice(term);
                }
                projected
            };

            results.push(result_bytes);
            if results.len() >= limit {
                break;
            }
        }
        if results.len() >= limit {
            break;
        }
    }

    Ok(OxigraphCoprocessorResponse {
        results,
        ..Default::default()
    })
}

fn evaluate_filter(expr: &FilterExpr, key_body: &[u8]) -> PluginResult<bool> {
    use crate::proto::filter_expr::FilterOp;
    use crate::term_nav::extract_term_bytes;

    match expr.op() {
        FilterOp::Eq => {
            let term = extract_term_bytes(key_body, expr.term_position as usize)
                .map_err(|e| PluginError::Other(e.to_string()))?;
            Ok(term == expr.constant_value.as_slice())
        }
        FilterOp::Neq => {
            let term = extract_term_bytes(key_body, expr.term_position as usize)
                .map_err(|e| PluginError::Other(e.to_string()))?;
            Ok(term != expr.constant_value.as_slice())
        }
        FilterOp::Gt => {
            let term = extract_term_bytes(key_body, expr.term_position as usize)
                .map_err(|e| PluginError::Other(e.to_string()))?;
            Ok(term > expr.constant_value.as_slice())
        }
        FilterOp::Gte => {
            let term = extract_term_bytes(key_body, expr.term_position as usize)
                .map_err(|e| PluginError::Other(e.to_string()))?;
            Ok(term >= expr.constant_value.as_slice())
        }
        FilterOp::Lt => {
            let term = extract_term_bytes(key_body, expr.term_position as usize)
                .map_err(|e| PluginError::Other(e.to_string()))?;
            Ok(term < expr.constant_value.as_slice())
        }
        FilterOp::Lte => {
            let term = extract_term_bytes(key_body, expr.term_position as usize)
                .map_err(|e| PluginError::Other(e.to_string()))?;
            Ok(term <= expr.constant_value.as_slice())
        }
        FilterOp::And => {
            for child in &expr.children {
                if !evaluate_filter(child, key_body)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        FilterOp::Or => {
            for child in &expr.children {
                if evaluate_filter(child, key_body)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        FilterOp::Not => {
            if expr.children.len() != 1 {
                return Err(PluginError::Other("NOT requires exactly one child".into()));
            }
            Ok(!evaluate_filter(&expr.children[0], key_body)?)
        }
        FilterOp::TypeCheck => {
            let term = extract_term_bytes(key_body, expr.term_position as usize)
                .map_err(|e| PluginError::Other(e.to_string()))?;
            let type_byte = term[0];
            Ok(type_byte >= expr.type_byte_min as u8 && type_byte <= expr.type_byte_max as u8)
        }
        FilterOp::BoundCheck => {
            // In fixed-layout keys, all positions are always bound.
            // This is included for completeness with OPTIONAL patterns.
            Ok(extract_term_bytes(key_body, expr.term_position as usize).is_ok())
        }
    }
}
```

### 2.6 Aggregation Execution (`src/aggregate.rs`)

```rust
use crate::proto::*;
use coprocessor_plugin_api::*;

pub fn execute_count_scan(
    req: &OxigraphCoprocessorRequest,
    ranges: &[Range],
    storage: &dyn RawStorage,
) -> PluginResult<OxigraphCoprocessorResponse> {
    let mut count: u64 = 0;

    for range in ranges {
        let mut iter = storage.scan(range)?;
        while let Some((_key, _value)) = iter.next()? {
            count += 1;
        }
    }

    Ok(OxigraphCoprocessorResponse {
        count,
        ..Default::default()
    })
}

pub fn execute_min_max_scan(
    req: &OxigraphCoprocessorRequest,
    ranges: &[Range],
    storage: &dyn RawStorage,
) -> PluginResult<OxigraphCoprocessorResponse> {
    // For MIN: take the first key in forward scan order.
    // For MAX: take the last key in the range (reverse iteration if supported,
    //          otherwise scan forward and keep the last).
    //
    // Oxigraph's term encoding preserves sort order within a type, so the
    // lexicographic first/last key in the prefix range corresponds to MIN/MAX.

    let mut result_key: Option<Vec<u8>> = None;

    for range in ranges {
        let mut iter = storage.scan(range)?;
        if req.is_max {
            // Scan to the end, keep last
            while let Some((key, _value)) = iter.next()? {
                result_key = Some(key.to_vec());
            }
        } else {
            // MIN: take the very first key
            if let Some((key, _value)) = iter.next()? {
                if result_key.is_none() {
                    result_key = Some(key.to_vec());
                }
                // No need to continue scanning for MIN
                break;
            }
        }
    }

    Ok(OxigraphCoprocessorResponse {
        min_max_value: result_key.unwrap_or_default(),
        ..Default::default()
    })
}
```

---

## 3. Client-Side Changes to `tikv.rs`

### 3.1 Coprocessor Capability Detection

Add a field to `TiKvStorageInner` to cache whether the plugin is available, and probe at connection time:

```rust
// In TiKvStorageInner:
struct TiKvStorageInner {
    client: TransactionClient,
    runtime: Runtime,
    scan_batch_size: usize,
    coprocessor_available: bool,  // NEW
}

// In TiKvStorage::connect_with_config, after ensure_version():
let coprocessor_available = storage.probe_coprocessor();
// ... store in inner

impl TiKvStorage {
    /// Send a no-op INDEX_SCAN with an empty prefix to check if the plugin responds.
    /// If TiKV returns a coprocessor-not-found error, mark as unavailable.
    fn probe_coprocessor(&self) -> bool {
        use prost::Message;
        let req = OxigraphCoprocessorRequest {
            op_type: OpType::CountScan as i32,
            scan_prefix: vec![TABLE_DEFAULT, 0xFF], // Prefix that matches nothing
            ..Default::default()
        };
        let mut buf = Vec::new();
        req.encode(&mut buf).ok();

        match self.inner.runtime.block_on(
            self.inner.client.raw_coprocessor(
                OXIGRAPH_COPROCESSOR_ID,
                vec![],  // empty ranges
                buf,
            )
        ) {
            Ok(_) => {
                log::info!("Oxigraph Coprocessor plugin detected on TiKV cluster");
                true
            }
            Err(e) => {
                log::warn!(
                    "Oxigraph Coprocessor plugin not available, \
                     falling back to raw scan: {e}"
                );
                false
            }
        }
    }

    pub fn coprocessor_available(&self) -> bool {
        self.inner.coprocessor_available
    }
}
```

### 3.2 New `coprocessor_scan` Method

Add to `TiKvStorageReader`:

```rust
impl<'a> TiKvStorageReader<'a> {
    /// Execute a Coprocessor request against TiKV. Falls back to raw prefix
    /// scan if the Coprocessor plugin is not available.
    pub fn coprocessor_scan(
        &self,
        table: u8,
        prefix: &[u8],
        encoding: QuadEncoding,
        filter: Option<FilterExpr>,
        bloom: Option<BloomFilterPayload>,
        limit: u32,
        return_positions: Vec<u32>,
    ) -> Result<Vec<Vec<u8>>, StorageError> {
        // Fallback: if coprocessor not available, do a raw scan and filter client-side
        if !self.storage.coprocessor_available() {
            return self.fallback_raw_scan(table, prefix, encoding, limit);
        }

        let full_prefix = prefixed_key(table, prefix);
        let upper = prefix_upper_bound(&full_prefix).unwrap_or_default();

        let mut req = OxigraphCoprocessorRequest {
            op_type: if filter.is_some() {
                OpType::FilterScan as i32
            } else {
                OpType::IndexScan as i32
            },
            scan_prefix: full_prefix.clone(),
            scan_upper_bound: upper.clone(),
            limit,
            filter,
            return_term_positions: return_positions,
            ..Default::default()
        };

        if let Some(bf) = bloom {
            req.bloom_filter = bf.bits;
            req.bloom_num_hashes = bf.num_hashes;
            req.bloom_check_positions = bf.check_positions;
        }

        let mut buf = Vec::new();
        req.encode(&mut buf).map_err(|e| StorageError::Other(Box::new(e)))?;

        // Build key ranges for the Coprocessor to process
        let range = (full_prefix.clone(), upper.clone());

        let response_bytes = self.storage.inner.runtime.block_on(
            self.storage.inner.client.raw_coprocessor(
                OXIGRAPH_COPROCESSOR_ID,
                vec![range],
                buf,
            )
        ).map_err(map_tikv_error)?;

        let resp = OxigraphCoprocessorResponse::decode(response_bytes.as_slice())
            .map_err(|e| StorageError::Other(Box::new(e)))?;

        if !resp.error.is_empty() {
            return Err(StorageError::Other(resp.error.into()));
        }

        Ok(resp.results)
    }

    /// Execute a COUNT_SCAN via Coprocessor.
    /// Returns the total count across all Regions.
    pub fn coprocessor_count(
        &self,
        table: u8,
        prefix: &[u8],
    ) -> Result<u64, StorageError> {
        if !self.storage.coprocessor_available() {
            // Fallback: count via raw scan
            return Ok(self.scan_prefix_keys(table, prefix)?.len() as u64);
        }

        let full_prefix = prefixed_key(table, prefix);
        let upper = prefix_upper_bound(&full_prefix).unwrap_or_default();

        let req = OxigraphCoprocessorRequest {
            op_type: OpType::CountScan as i32,
            scan_prefix: full_prefix.clone(),
            scan_upper_bound: upper.clone(),
            ..Default::default()
        };

        let mut buf = Vec::new();
        req.encode(&mut buf).map_err(|e| StorageError::Other(Box::new(e)))?;

        let range = (full_prefix, upper);

        // TiKV fans out to all Regions covering the range;
        // each Region returns its partial count.
        // The client library aggregates responses.
        let response_bytes = self.storage.inner.runtime.block_on(
            self.storage.inner.client.raw_coprocessor(
                OXIGRAPH_COPROCESSOR_ID,
                vec![range],
                buf,
            )
        ).map_err(map_tikv_error)?;

        let resp = OxigraphCoprocessorResponse::decode(response_bytes.as_slice())
            .map_err(|e| StorageError::Other(Box::new(e)))?;

        Ok(resp.count)
    }

    fn fallback_raw_scan(
        &self,
        table: u8,
        prefix: &[u8],
        _encoding: QuadEncoding,
        limit: u32,
    ) -> Result<Vec<Vec<u8>>, StorageError> {
        let keys = if limit > 0 {
            self.scan_prefix_keys_limit(table, prefix, limit)?
        } else {
            self.scan_prefix_keys(table, prefix)?
        };
        Ok(keys.into_iter().map(|k| {
            let v: Vec<u8> = k.into();
            v
        }).collect())
    }
}
```

### 3.3 Payload Types for Bloom Filter

```rust
/// Serialized bloom filter payload to attach to a Coprocessor request.
pub struct BloomFilterPayload {
    pub bits: Vec<u8>,
    pub num_hashes: u32,
    pub check_positions: Vec<u32>,
}
```

---

## 4. Bloom Filter Semi-Join

### 4.1 Bloom Filter Implementation (`src/bloom.rs` in the plugin crate; mirrored in oxigraph client)

A simple bloom filter using double hashing (two independent hash functions combined to simulate `k` hash functions).

```rust
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

pub struct BloomFilter {
    bits: Vec<u8>,
    num_hashes: u32,
    num_bits: u64,
}

impl BloomFilter {
    /// Create a new bloom filter sized for `capacity` items at the given
    /// false positive rate.
    pub fn with_capacity_and_fpr(capacity: usize, fpr: f64) -> Self {
        // Optimal number of bits: -n * ln(p) / (ln(2)^2)
        let num_bits = (-(capacity as f64) * fpr.ln() / (2.0_f64.ln().powi(2)))
            .ceil() as u64;
        let num_bits = num_bits.max(64); // minimum 8 bytes

        // Optimal number of hashes: (m/n) * ln(2)
        let num_hashes = ((num_bits as f64 / capacity as f64) * 2.0_f64.ln())
            .ceil() as u32;
        let num_hashes = num_hashes.max(1);

        let num_bytes = ((num_bits + 7) / 8) as usize;
        BloomFilter {
            bits: vec![0u8; num_bytes],
            num_hashes,
            num_bits,
        }
    }

    /// Insert an item into the bloom filter.
    pub fn insert(&mut self, item: &[u8]) {
        let (h1, h2) = self.hash_pair(item);
        for i in 0..self.num_hashes {
            let bit_index = self.bit_index(h1, h2, i);
            let byte_index = (bit_index / 8) as usize;
            let bit_offset = (bit_index % 8) as u8;
            self.bits[byte_index] |= 1 << bit_offset;
        }
    }

    /// Check if an item might be in the set. Returns false if definitely not present.
    pub fn check(&self, item: &[u8]) -> bool {
        let (h1, h2) = self.hash_pair(item);
        for i in 0..self.num_hashes {
            let bit_index = self.bit_index(h1, h2, i);
            let byte_index = (bit_index / 8) as usize;
            let bit_offset = (bit_index % 8) as u8;
            if self.bits[byte_index] & (1 << bit_offset) == 0 {
                return false;
            }
        }
        true
    }

    /// Serialize to bytes for transmission in the Coprocessor request.
    pub fn serialize(&self) -> Vec<u8> {
        self.bits.clone()
    }

    /// Reconstruct from serialized bytes (received by the Coprocessor plugin).
    pub fn from_bytes(data: &[u8], num_hashes: u32) -> Self {
        let num_bits = (data.len() as u64) * 8;
        BloomFilter {
            bits: data.to_vec(),
            num_hashes,
            num_bits,
        }
    }

    pub fn num_hashes(&self) -> u32 {
        self.num_hashes
    }

    fn hash_pair(&self, item: &[u8]) -> (u64, u64) {
        let mut h1 = DefaultHasher::new();
        item.hash(&mut h1);
        let hash1 = h1.finish();

        // Second hash: seed with a different initial state
        let mut h2 = DefaultHasher::new();
        hash1.hash(&mut h2);
        item.hash(&mut h2);
        let hash2 = h2.finish();

        (hash1, hash2)
    }

    fn bit_index(&self, h1: u64, h2: u64, i: u32) -> u64 {
        (h1.wrapping_add((i as u64).wrapping_mul(h2))) % self.num_bits
    }
}
```

**Critical note on hash portability**: Both client and server MUST use the same hash function. `DefaultHasher` is NOT stable across Rust versions. For production, replace with a deterministic hasher such as `xxhash-rust` (xxh3) or `siphasher` with fixed keys. Example:

```rust
// Use xxhash for deterministic, portable hashing:
// In Cargo.toml: xxhash-rust = { version = "0.8", features = ["xxh3"] }
fn hash_pair(&self, item: &[u8]) -> (u64, u64) {
    let hash1 = xxhash_rust::xxh3::xxh3_64_with_seed(item, 0);
    let hash2 = xxhash_rust::xxh3::xxh3_64_with_seed(item, 0x9E3779B97F4A7C15);
    (hash1, hash2)
}
```

### 4.2 Client-Side: Building and Sending the Bloom Filter

This code lives in the Oxigraph query evaluator, invoked when a `Join` with `HashBuildLeftProbeRight` is executed against TiKV.

```rust
use crate::storage::binary_encoder::write_term;

/// Build a bloom filter from the left-side join results for the given join key variables.
///
/// `left_results` - results from evaluating the build (left) side of the join
/// `join_key_indices` - indices into InternalTuple for the join key variables
///
/// Returns None if the cardinality is outside the [min, max] bloom filter thresholds.
fn build_semi_join_bloom(
    left_results: &[InternalTuple],
    join_key_indices: &[usize],
    min_bloom_cardinality: usize,  // default: 100
    max_bloom_cardinality: usize,  // default: 500_000
) -> Option<(BloomFilter, Vec<u32>)> {
    let cardinality = left_results.len();
    if cardinality < min_bloom_cardinality || cardinality > max_bloom_cardinality {
        return None;
    }

    let mut bloom = BloomFilter::with_capacity_and_fpr(cardinality, 0.01);

    for tuple in left_results {
        let mut key_bytes = Vec::with_capacity(join_key_indices.len() * WRITTEN_TERM_MAX_SIZE);
        let mut all_bound = true;

        for &var_idx in join_key_indices {
            if let Some(term) = tuple.get(var_idx) {
                write_term(&mut key_bytes, term);
            } else {
                all_bound = false;
                break;
            }
        }

        if all_bound {
            bloom.insert(&key_bytes);
        }
    }

    // Map join key variable indices to term positions in the probe-side TiKV key.
    // This mapping depends on which index table is used for the probe side.
    // The caller must compute bloom_check_positions based on the QuadPattern
    // and selected index.
    // Placeholder: actual positions set by the caller.
    None // Caller wraps with positions
}

/// Map a SPARQL variable's role (subject/predicate/object) to its position
/// in a specific index table's key layout.
///
/// Returns the 0-based term position within the key (after the table prefix byte).
fn quad_role_to_key_position(role: QuadRole, table: u8) -> u32 {
    match table {
        // DSPO: S=0, P=1, O=2
        TABLE_DSPO => match role {
            QuadRole::Subject => 0,
            QuadRole::Predicate => 1,
            QuadRole::Object => 2,
            QuadRole::GraphName => panic!("default graph tables have no graph position"),
        },
        // DPOS: P=0, O=1, S=2
        TABLE_DPOS => match role {
            QuadRole::Subject => 2,
            QuadRole::Predicate => 0,
            QuadRole::Object => 1,
            QuadRole::GraphName => panic!("default graph tables have no graph position"),
        },
        // DOSP: O=0, S=1, P=2
        TABLE_DOSP => match role {
            QuadRole::Subject => 1,
            QuadRole::Predicate => 2,
            QuadRole::Object => 0,
            QuadRole::GraphName => panic!("default graph tables have no graph position"),
        },
        // SPOG: S=0, P=1, O=2, G=3
        TABLE_SPOG => match role {
            QuadRole::Subject => 0,
            QuadRole::Predicate => 1,
            QuadRole::Object => 2,
            QuadRole::GraphName => 3,
        },
        // POSG: P=0, O=1, S=2, G=3
        TABLE_POSG => match role {
            QuadRole::Subject => 2,
            QuadRole::Predicate => 0,
            QuadRole::Object => 1,
            QuadRole::GraphName => 3,
        },
        // OSPG: O=0, S=1, P=2, G=3
        TABLE_OSPG => match role {
            QuadRole::Subject => 1,
            QuadRole::Predicate => 2,
            QuadRole::Object => 0,
            QuadRole::GraphName => 3,
        },
        // GSPO: G=0, S=1, P=2, O=3
        TABLE_GSPO => match role {
            QuadRole::Subject => 1,
            QuadRole::Predicate => 2,
            QuadRole::Object => 3,
            QuadRole::GraphName => 0,
        },
        // GPOS: G=0, P=1, O=2, S=3
        TABLE_GPOS => match role {
            QuadRole::Subject => 3,
            QuadRole::Predicate => 1,
            QuadRole::Object => 2,
            QuadRole::GraphName => 0,
        },
        // GOSP: G=0, O=1, S=2, P=3
        TABLE_GOSP => match role {
            QuadRole::Subject => 2,
            QuadRole::Predicate => 3,
            QuadRole::Object => 1,
            QuadRole::GraphName => 0,
        },
        _ => panic!("unknown table prefix"),
    }
}

enum QuadRole {
    Subject,
    Predicate,
    Object,
    GraphName,
}
```

### 4.3 Server-Side: Bloom Check During Scan

Already shown in `scan.rs` above (Section 2.5). The critical path:

1. For each scanned key, strip the 1-byte table prefix.
2. Use `extract_term_bytes` or `extract_concat_term_bytes` to pull the join-key term bytes at the positions specified in `bloom_check_positions`.
3. Call `bloom.check(&term_bytes)`. If false, skip the key entirely -- it cannot join with the build side.
4. If true (possible match), include the key in results. The client-side exact hash join will eliminate any false positives.

### 4.4 End-to-End Example

Query: `SELECT ?name ?birth WHERE { ?p foaf:name ?name . ?p dbpedia:birth ?birth }`

1. **Left side** (build): Scan DPOS with prefix `encode_term(foaf:name)`. Returns `(P=foaf:name, O=?name, S=?p)` keys.
2. **Build bloom**: Extract `?p` values (at position 2 in DPOS). Insert each encoded term into a bloom filter with 1% FPR.
3. **Right side** (probe): Send Coprocessor `FILTER_SCAN` to DPOS with prefix `encode_term(dbpedia:birth)`, bloom filter attached, `bloom_check_positions = [2]` (position of subject in DPOS key).
4. **TiKV plugin**: For each key matching prefix `dbpedia:birth`, extracts the subject term at position 2, checks bloom. Only returns keys where `?p` might be in the build side.
5. **Client**: Receives filtered results. Performs exact hash join on `?p`. False positives (~1%) are eliminated.

---

## 5. Build, Deploy, and Verify

### 5.1 Build the Plugin

```bash
# Clone and build from the project root
cd oxigraph-tikv-coprocessor

# Ensure the TiKV coprocessor plugin API version matches your TiKV deployment
# Check your TiKV version:
#   tikv-server --version

# Build the shared library
cargo build --release

# Output: target/release/liboxigraph_tikv_coprocessor.so
ls -la target/release/liboxigraph_tikv_coprocessor.so
```

### 5.2 Deploy to TiKV

#### Option A: Direct file deployment (development)

Copy the `.so` to each TiKV node and configure TiKV to load it:

```toml
# tikv.toml (add to each TiKV node's configuration)
[coprocessor]
  region-split-size = "96MiB"

[coprocessor-plugin]
  # Directory where TiKV looks for .so plugin files
  dir = "/opt/tikv/coprocessor-plugins"
```

```bash
# On each TiKV node:
mkdir -p /opt/tikv/coprocessor-plugins
cp liboxigraph_tikv_coprocessor.so /opt/tikv/coprocessor-plugins/

# Restart TiKV (rolling restart for zero-downtime)
systemctl restart tikv
```

#### Option B: Container image (production / OpenShift)

Embed the plugin in the TiKV container image:

```dockerfile
# Dockerfile.tikv-with-coprocessor
FROM pingcap/tikv:v7.5.0

COPY liboxigraph_tikv_coprocessor.so /opt/tikv/coprocessor-plugins/

# The TiKV Helm chart / operator config must set:
#   coprocessor-plugin.dir = "/opt/tikv/coprocessor-plugins"
```

For OpenShift deployments using the TiDB Operator:

```yaml
# In the TidbCluster CR spec:
spec:
  tikv:
    config: |
      [coprocessor-plugin]
      dir = "/opt/tikv/coprocessor-plugins"
    additionalVolumes:
      - name: coprocessor-plugins
        emptyDir: {}
    additionalVolumeMounts:
      - name: coprocessor-plugins
        mountPath: /opt/tikv/coprocessor-plugins
    # Use initContainer or custom image to inject the .so
```

### 5.3 Verify Plugin is Loaded

Check TiKV logs for plugin loading messages:

```bash
# On a TiKV node:
grep -i "coprocessor.*plugin" /var/log/tikv/tikv.log

# Expected output:
# [INFO] loaded coprocessor plugin: oxigraph_tikv_coprocessor
```

### 5.4 Integration Test

Run from the Oxigraph client to verify end-to-end functionality:

```rust
#[cfg(test)]
mod coprocessor_tests {
    use super::*;

    #[test]
    fn test_coprocessor_count_scan() {
        let storage = TiKvStorage::connect(&["127.0.0.1:2379".to_string()]).unwrap();
        assert!(storage.coprocessor_available(), "plugin must be loaded for this test");

        // Insert test data
        let mut txn = storage.start_transaction().unwrap();
        // ... insert 100 triples with predicate <http://example.org/name>

        let reader = storage.snapshot();
        let predicate_encoded = encode_term(&EncodedTerm::NamedNode {
            iri_id: StrHash::new("http://example.org/name"),
        });

        let count = reader.coprocessor_count(TABLE_DPOS, &predicate_encoded).unwrap();
        assert_eq!(count, 100);
    }

    #[test]
    fn test_coprocessor_bloom_filter_scan() {
        let storage = TiKvStorage::connect(&["127.0.0.1:2379".to_string()]).unwrap();
        assert!(storage.coprocessor_available());

        // Insert test data: 1000 triples with predicate P1, 1000 with P2.
        // Only 100 subjects overlap between P1 and P2.
        // ... (setup code)

        // Build bloom from P1 scan results (subjects at position 2 in DPOS)
        let p1_keys = reader.scan_prefix_keys(TABLE_DPOS, &encode_term(&p1_encoded)).unwrap();
        let mut bloom = BloomFilter::with_capacity_and_fpr(p1_keys.len(), 0.01);
        for key in &p1_keys {
            let key_bytes: Vec<u8> = key.clone().into();
            let subject_bytes = extract_term_bytes(&key_bytes[1..], 2).unwrap();
            bloom.insert(subject_bytes);
        }

        // Coprocessor scan with bloom filter
        let payload = BloomFilterPayload {
            bits: bloom.serialize(),
            num_hashes: bloom.num_hashes(),
            check_positions: vec![2], // subject position in DPOS
        };

        let filtered = reader.coprocessor_scan(
            TABLE_DPOS,
            &encode_term(&p2_encoded),
            QuadEncoding::Dpos,
            None,
            Some(payload),
            0,
            vec![],
        ).unwrap();

        // Should return ~100 results (the overlapping subjects) plus <=1% false positives
        assert!(filtered.len() >= 100);
        assert!(filtered.len() <= 110); // ~1% FPR on 1000 items = ~10 false positives max
    }
}
```

### 5.5 Smoke Test Script

```bash
#!/bin/bash
# smoke-test-coprocessor.sh
# Run after deploying the plugin to verify it works.

set -euo pipefail

OXIGRAPH_ENDPOINT="http://localhost:7878"

# Load test data
curl -s -X POST "${OXIGRAPH_ENDPOINT}/store" \
  -H "Content-Type: text/turtle" \
  --data-binary @- <<'EOF'
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix ex: <http://example.org/> .

ex:alice foaf:name "Alice" .
ex:bob foaf:name "Bob" .
ex:alice ex:birth "1990-01-01" .
ex:bob ex:birth "1985-06-15" .
ex:charlie ex:birth "2000-12-31" .
EOF

# Query that exercises the coprocessor (2-BGP join)
RESULT=$(curl -s "${OXIGRAPH_ENDPOINT}/query" \
  -H "Accept: application/sparql-results+json" \
  --data-urlencode "query=SELECT ?name ?birth WHERE { ?p <http://xmlns.com/foaf/0.1/name> ?name . ?p <http://example.org/birth> ?birth }")

echo "Query result: ${RESULT}"

# Verify: should return Alice+1990-01-01 and Bob+1985-06-15 (not Charlie)
COUNT=$(echo "${RESULT}" | jq '.results.bindings | length')
if [ "${COUNT}" -eq 2 ]; then
    echo "PASS: Coprocessor semi-join returned expected 2 results"
else
    echo "FAIL: Expected 2 results, got ${COUNT}"
    exit 1
fi

# COUNT pushdown test
COUNT_RESULT=$(curl -s "${OXIGRAPH_ENDPOINT}/query" \
  -H "Accept: application/sparql-results+json" \
  --data-urlencode "query=SELECT (COUNT(*) AS ?c) WHERE { ?s <http://example.org/birth> ?o }")

COUNT_VAL=$(echo "${COUNT_RESULT}" | jq -r '.results.bindings[0].c.value')
if [ "${COUNT_VAL}" -eq 3 ]; then
    echo "PASS: COUNT pushdown returned expected 3"
else
    echo "FAIL: Expected count 3, got ${COUNT_VAL}"
    exit 1
fi

echo "All coprocessor smoke tests passed."
```

---

## 6. Summary of Files to Create/Modify

| File | Action | Description |
|------|--------|-------------|
| `oxigraph-tikv-coprocessor/Cargo.toml` | **Create** | New crate for the TiKV plugin |
| `oxigraph-tikv-coprocessor/build.rs` | **Create** | prost-build for protobuf generation |
| `oxigraph-tikv-coprocessor/proto/oxigraph_coprocessor.proto` | **Create** | Protobuf schema (Section 1) |
| `oxigraph-tikv-coprocessor/src/lib.rs` | **Create** | Plugin entry point (Section 2.3) |
| `oxigraph-tikv-coprocessor/src/term_nav.rs` | **Create** | Encoded term byte navigation (Section 2.4) |
| `oxigraph-tikv-coprocessor/src/scan.rs` | **Create** | IndexScan/FilterScan execution (Section 2.5) |
| `oxigraph-tikv-coprocessor/src/aggregate.rs` | **Create** | CountScan/MinMaxScan execution (Section 2.6) |
| `oxigraph-tikv-coprocessor/src/bloom.rs` | **Create** | Bloom filter (Section 4.1) |
| `oxigraph/lib/oxigraph/src/storage/tikv.rs` | **Modify** | Add coprocessor_scan, coprocessor_count, probe logic (Section 3) |
| `oxigraph/lib/oxigraph/src/storage/bloom.rs` | **Create** | Client-side bloom filter (same impl as plugin) |

## 7. Implementation Order

1. **term_nav.rs** first -- it has zero dependencies and can be unit tested against `binary_encoder.rs` test vectors.
2. **bloom.rs** next -- pure data structure, unit testable in isolation.
3. **proto** + **build.rs** -- generate the protobuf types.
4. **lib.rs** + **scan.rs** + **aggregate.rs** -- the plugin itself. Test with a local TiKV dev cluster.
5. **tikv.rs modifications** -- client-side integration. Wire up `coprocessor_scan` and `coprocessor_count`.
6. **Integration tests** -- end-to-end with real TiKV + plugin loaded.
