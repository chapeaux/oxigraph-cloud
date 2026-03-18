//! Index scan operations for Coprocessor pushdown.

use crate::extract_concat_term_bytes;

pub struct ScanResult {
    pub pairs: Vec<(Vec<u8>, Vec<u8>)>,
    pub scanned_keys: u64,
}

pub struct IndexScanParams {
    pub table_prefix: u8,
    pub key_prefix: Vec<u8>,
    pub limit: u64,
    pub bloom_filter: Option<Vec<u8>>,
    pub bloom_positions: Vec<u32>,
}

pub fn execute_index_scan<'a>(
    params: &IndexScanParams,
    pairs: impl Iterator<Item = (&'a [u8], &'a [u8])>,
) -> ScanResult {
    let mut result = ScanResult { pairs: Vec::new(), scanned_keys: 0 };
    let full_prefix = {
        let mut p = vec![params.table_prefix];
        p.extend_from_slice(&params.key_prefix);
        p
    };

    for (key, value) in pairs {
        if !key.starts_with(&full_prefix) { continue; }
        result.scanned_keys += 1;

        if let Some(ref bloom_bytes) = params.bloom_filter {
            let key_without_prefix = &key[1..];
            if let Ok(concat) = extract_concat_term_bytes(key_without_prefix, &params.bloom_positions) {
                if !bloom_check(bloom_bytes, &concat) { continue; }
            }
        }

        result.pairs.push((key.to_vec(), value.to_vec()));
        if params.limit > 0 && result.pairs.len() as u64 >= params.limit { break; }
    }
    result
}

fn bloom_check(filter: &[u8], key: &[u8]) -> bool {
    if filter.is_empty() { return true; }
    let bits = filter.len() * 8;
    let h = siphasher_hash(key);
    let h1 = h as usize;
    let h2 = (h >> 32) as usize;
    for i in 0usize..3 {
        let bit_pos = (h1.wrapping_add(i.wrapping_mul(h2))) % bits;
        if filter[bit_pos / 8] & (1 << (bit_pos % 8)) == 0 { return false; }
    }
    true
}

fn siphasher_hash(key: &[u8]) -> u64 {
    use siphasher::sip::SipHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = SipHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_index_scan() {
        let data: Vec<(Vec<u8>, Vec<u8>)> = vec![
            (vec![0x02, 1, 0, 0], b"val1".to_vec()),
            (vec![0x02, 1, 0, 1], b"val2".to_vec()),
            (vec![0x03, 1, 0, 0], b"other".to_vec()),
        ];
        let params = IndexScanParams {
            table_prefix: 0x02, key_prefix: vec![], limit: 0,
            bloom_filter: None, bloom_positions: vec![],
        };
        let result = execute_index_scan(&params, data.iter().map(|(k, v)| (k.as_slice(), v.as_slice())));
        assert_eq!(result.pairs.len(), 2);
    }

    #[test]
    fn test_scan_with_limit() {
        let data: Vec<(Vec<u8>, Vec<u8>)> = vec![
            (vec![0x02, 1], b"a".to_vec()),
            (vec![0x02, 2], b"b".to_vec()),
            (vec![0x02, 3], b"c".to_vec()),
        ];
        let params = IndexScanParams {
            table_prefix: 0x02, key_prefix: vec![], limit: 2,
            bloom_filter: None, bloom_positions: vec![],
        };
        let result = execute_index_scan(&params, data.iter().map(|(k, v)| (k.as_slice(), v.as_slice())));
        assert_eq!(result.pairs.len(), 2);
    }
}
