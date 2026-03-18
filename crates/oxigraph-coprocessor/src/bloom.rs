//! Bloom filter for semi-join pushdown.

use siphasher::sip::SipHasher;
use std::hash::{Hash, Hasher};

#[derive(Clone, Debug)]
pub struct BloomFilter {
    bits: Vec<u8>,
    num_bits: usize,
    num_hashes: u8,
}

impl BloomFilter {
    pub fn new(expected_elements: usize, fp_rate: f64) -> Self {
        let num_bits = optimal_num_bits(expected_elements, fp_rate);
        let num_hashes = optimal_num_hashes(expected_elements, num_bits);
        Self {
            bits: vec![0; num_bits.div_ceil(8)],
            num_bits,
            num_hashes,
        }
    }

    pub fn from_bytes(bytes: Vec<u8>, num_hashes: u8) -> Self {
        let num_bits = bytes.len() * 8;
        Self {
            bits: bytes,
            num_bits,
            num_hashes,
        }
    }

    pub fn insert(&mut self, key: &[u8]) {
        let (h1, h2) = hash_pair(key);
        for i in 0..usize::from(self.num_hashes) {
            let bit_pos = (h1.wrapping_add(i.wrapping_mul(h2))) % self.num_bits;
            self.bits[bit_pos / 8] |= 1 << (bit_pos % 8);
        }
    }

    pub fn may_contain(&self, key: &[u8]) -> bool {
        let (h1, h2) = hash_pair(key);
        for i in 0..usize::from(self.num_hashes) {
            let bit_pos = (h1.wrapping_add(i.wrapping_mul(h2))) % self.num_bits;
            if self.bits[bit_pos / 8] & (1 << (bit_pos % 8)) == 0 {
                return false;
            }
        }
        true
    }

    pub fn to_bytes(&self) -> &[u8] {
        &self.bits
    }
    pub fn num_hashes(&self) -> u8 {
        self.num_hashes
    }
}

fn hash_pair(key: &[u8]) -> (usize, usize) {
    let mut h1 = SipHasher::new_with_keys(0, 0);
    key.hash(&mut h1);
    let mut h2 = SipHasher::new_with_keys(1, 1);
    key.hash(&mut h2);
    #[expect(clippy::cast_possible_truncation)]
    let pair = (h1.finish() as usize, h2.finish() as usize);
    pair
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn optimal_num_bits(n: usize, fp: f64) -> usize {
    (-(n as f64 * fp.ln()) / (2.0_f64.ln().powi(2))).ceil() as usize
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn optimal_num_hashes(n: usize, m: usize) -> u8 {
    ((m as f64 / n as f64) * 2.0_f64.ln())
        .ceil()
        .clamp(1.0, 16.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bloom_insert_and_check() {
        let mut bf = BloomFilter::new(100, 0.01);
        bf.insert(b"hello");
        bf.insert(b"world");
        assert!(bf.may_contain(b"hello"));
        assert!(bf.may_contain(b"world"));
        assert!(!bf.may_contain(b"unknown"));
    }

    #[test]
    fn test_bloom_roundtrip() {
        let mut bf = BloomFilter::new(50, 0.01);
        bf.insert(b"key1");
        let bytes = bf.to_bytes().to_vec();
        let bf2 = BloomFilter::from_bytes(bytes, bf.num_hashes());
        assert!(bf2.may_contain(b"key1"));
    }

    #[test]
    fn test_bloom_fp_rate() {
        let n = 1000;
        let mut bf = BloomFilter::new(n, 0.01);
        for i in 0..n {
            bf.insert(format!("key-{i}").as_bytes());
        }
        let mut fps = 0;
        for i in n..n + 10000 {
            if bf.may_contain(format!("other-{i}").as_bytes()) {
                fps += 1;
            }
        }
        assert!((f64::from(fps) / 10000.0) < 0.03);
    }
}
