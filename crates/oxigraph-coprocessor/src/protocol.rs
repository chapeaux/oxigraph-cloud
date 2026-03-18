//! Binary request/response encoding for the Oxigraph coprocessor plugin.
//!
//! We use a simple tag-length-value format instead of protobuf to keep the
//! crate self-contained (no prost / proto file dependency).
//!
//! # Request format
//!
//! ```text
//! Byte 0:       operation type
//!                 0 = IndexScan
//!                 1 = FilterScan
//!                 2 = CountScan
//!                 3 = MinMaxScan
//! Byte 1:       table prefix (e.g. TABLE_SPOG = 0x02)
//! Bytes 2..4:   key prefix length as u16 big-endian
//! Bytes 4..4+N: key prefix bytes
//! Remaining:    optional bloom filter bytes (may be empty)
//! ```
//!
//! # Response format
//!
//! ```text
//! Bytes 0..8:     scanned_keys as u64 big-endian
//! Bytes 8..16:    result_count as u64 big-endian (number of KV pairs, or
//!                 count for CountScan)
//! For IndexScan / FilterScan, followed by repeated:
//!   2 bytes:  key length as u16 big-endian
//!   N bytes:  key
//!   4 bytes:  value length as u32 big-endian
//!   M bytes:  value
//! For MinMaxScan, followed by:
//!   2 bytes:  min_key length (0 if absent)
//!   N bytes:  min_key
//!   2 bytes:  max_key length (0 if absent)
//!   M bytes:  max_key
//! ```

/// Operation types that can be requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpType {
    IndexScan = 0,
    FilterScan = 1,
    CountScan = 2,
    MinMaxScan = 3,
}

impl OpType {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(Self::IndexScan),
            1 => Some(Self::FilterScan),
            2 => Some(Self::CountScan),
            3 => Some(Self::MinMaxScan),
            _ => None,
        }
    }
}

/// Decoded coprocessor request.
#[derive(Debug, Clone)]
pub struct CoprocessorRequest {
    pub op_type: OpType,
    pub table_prefix: u8,
    pub key_prefix: Vec<u8>,
    pub bloom_filter: Option<Vec<u8>>,
}

/// Decode a raw request from the binary format.
pub fn decode_request(data: &[u8]) -> Result<CoprocessorRequest, String> {
    if data.len() < 4 {
        return Err("request too short: need at least 4 bytes".into());
    }

    let op_type =
        OpType::from_byte(data[0]).ok_or_else(|| format!("unknown op type: {}", data[0]))?;
    let table_prefix = data[1];
    let key_prefix_len = u16::from_be_bytes([data[2], data[3]]) as usize;

    if data.len() < 4 + key_prefix_len {
        return Err(format!(
            "request truncated: declared key prefix length {key_prefix_len} but only {} bytes remain",
            data.len() - 4
        ));
    }

    let key_prefix = data[4..4 + key_prefix_len].to_vec();
    let bloom_filter = if data.len() > 4 + key_prefix_len {
        Some(data[4 + key_prefix_len..].to_vec())
    } else {
        None
    };

    Ok(CoprocessorRequest {
        op_type,
        table_prefix,
        key_prefix,
        bloom_filter,
    })
}

/// Encode a raw request into the binary format.
pub fn encode_request(req: &CoprocessorRequest) -> Vec<u8> {
    let key_prefix_len = req.key_prefix.len();
    let bloom_len = req.bloom_filter.as_ref().map_or(0, Vec::len);
    let mut buf = Vec::with_capacity(4 + key_prefix_len + bloom_len);

    buf.push(req.op_type as u8);
    buf.push(req.table_prefix);
    #[allow(clippy::cast_possible_truncation)]
    let len_bytes = (key_prefix_len as u16).to_be_bytes();
    buf.extend_from_slice(&len_bytes);
    buf.extend_from_slice(&req.key_prefix);
    if let Some(ref bloom) = req.bloom_filter {
        buf.extend_from_slice(bloom);
    }
    buf
}

// ---------------------------------------------------------------------------
// Response encoding helpers
// ---------------------------------------------------------------------------

/// Encode a scan response (IndexScan / FilterScan): pairs of (key, value).
pub fn encode_scan_response(scanned_keys: u64, pairs: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&scanned_keys.to_be_bytes());
    #[allow(clippy::cast_possible_truncation)]
    let count_bytes = (pairs.len() as u64).to_be_bytes();
    buf.extend_from_slice(&count_bytes);
    for (key, value) in pairs {
        #[allow(clippy::cast_possible_truncation)]
        let key_len = (key.len() as u16).to_be_bytes();
        buf.extend_from_slice(&key_len);
        buf.extend_from_slice(key);
        #[allow(clippy::cast_possible_truncation)]
        let val_len = (value.len() as u32).to_be_bytes();
        buf.extend_from_slice(&val_len);
        buf.extend_from_slice(value);
    }
    buf
}

/// Encode a count response (CountScan).
pub fn encode_count_response(scanned_keys: u64, count: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16);
    buf.extend_from_slice(&scanned_keys.to_be_bytes());
    buf.extend_from_slice(&count.to_be_bytes());
    buf
}

/// Encode a min/max response (MinMaxScan).
pub fn encode_min_max_response(
    scanned_keys: u64,
    min_key: Option<&[u8]>,
    max_key: Option<&[u8]>,
) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&scanned_keys.to_be_bytes());
    // result_count = 1 if we have any result, 0 otherwise
    let has_result: u64 = if min_key.is_some() || max_key.is_some() {
        1
    } else {
        0
    };
    buf.extend_from_slice(&has_result.to_be_bytes());
    // min_key
    if let Some(k) = min_key {
        #[allow(clippy::cast_possible_truncation)]
        let len = (k.len() as u16).to_be_bytes();
        buf.extend_from_slice(&len);
        buf.extend_from_slice(k);
    } else {
        buf.extend_from_slice(&0u16.to_be_bytes());
    }
    // max_key
    if let Some(k) = max_key {
        #[allow(clippy::cast_possible_truncation)]
        let len = (k.len() as u16).to_be_bytes();
        buf.extend_from_slice(&len);
        buf.extend_from_slice(k);
    } else {
        buf.extend_from_slice(&0u16.to_be_bytes());
    }
    buf
}

/// Decode a scan response back into (scanned_keys, pairs).
pub fn decode_scan_response(data: &[u8]) -> Result<(u64, Vec<(Vec<u8>, Vec<u8>)>), String> {
    if data.len() < 16 {
        return Err("scan response too short".into());
    }
    let scanned_keys = u64::from_be_bytes(data[0..8].try_into().unwrap());
    let count = u64::from_be_bytes(data[8..16].try_into().unwrap());
    let mut offset = 16;
    let mut pairs = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if offset + 2 > data.len() {
            return Err("truncated key length".into());
        }
        let key_len = u16::from_be_bytes(data[offset..offset + 2].try_into().unwrap()) as usize;
        offset += 2;
        if offset + key_len > data.len() {
            return Err("truncated key data".into());
        }
        let key = data[offset..offset + key_len].to_vec();
        offset += key_len;
        if offset + 4 > data.len() {
            return Err("truncated value length".into());
        }
        let val_len = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        if offset + val_len > data.len() {
            return Err("truncated value data".into());
        }
        let value = data[offset..offset + val_len].to_vec();
        offset += val_len;
        pairs.push((key, value));
    }
    Ok((scanned_keys, pairs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let req = CoprocessorRequest {
            op_type: OpType::IndexScan,
            table_prefix: 0x02,
            key_prefix: vec![0xAA, 0xBB],
            bloom_filter: Some(vec![0xFF, 0x01]),
        };
        let encoded = encode_request(&req);
        let decoded = decode_request(&encoded).unwrap();
        assert_eq!(decoded.op_type, OpType::IndexScan);
        assert_eq!(decoded.table_prefix, 0x02);
        assert_eq!(decoded.key_prefix, vec![0xAA, 0xBB]);
        assert_eq!(decoded.bloom_filter, Some(vec![0xFF, 0x01]));
    }

    #[test]
    fn request_no_bloom() {
        let req = CoprocessorRequest {
            op_type: OpType::CountScan,
            table_prefix: 0x05,
            key_prefix: vec![],
            bloom_filter: None,
        };
        let encoded = encode_request(&req);
        let decoded = decode_request(&encoded).unwrap();
        assert_eq!(decoded.op_type, OpType::CountScan);
        assert!(decoded.bloom_filter.is_none());
    }

    #[test]
    fn scan_response_roundtrip() {
        let pairs = vec![
            (vec![0x02, 0x01], b"val1".to_vec()),
            (vec![0x02, 0x02], b"val2".to_vec()),
        ];
        let encoded = encode_scan_response(42, &pairs);
        let (scanned, decoded_pairs) = decode_scan_response(&encoded).unwrap();
        assert_eq!(scanned, 42);
        assert_eq!(decoded_pairs, pairs);
    }

    #[test]
    fn count_response_encoding() {
        let encoded = encode_count_response(100, 50);
        assert_eq!(encoded.len(), 16);
        let scanned = u64::from_be_bytes(encoded[0..8].try_into().unwrap());
        let count = u64::from_be_bytes(encoded[8..16].try_into().unwrap());
        assert_eq!(scanned, 100);
        assert_eq!(count, 50);
    }

    #[test]
    fn min_max_response_encoding() {
        let encoded = encode_min_max_response(10, Some(&[0x02, 0x01]), Some(&[0x02, 0xFF]));
        // scanned_keys(8) + result_count(8) + min_len(2) + min(2) + max_len(2) + max(2)
        assert_eq!(encoded.len(), 24);
        let scanned = u64::from_be_bytes(encoded[0..8].try_into().unwrap());
        assert_eq!(scanned, 10);
    }

    #[test]
    fn decode_request_too_short() {
        assert!(decode_request(&[0, 1, 2]).is_err());
    }

    #[test]
    fn decode_request_bad_op() {
        assert!(decode_request(&[99, 0, 0, 0]).is_err());
    }
}
