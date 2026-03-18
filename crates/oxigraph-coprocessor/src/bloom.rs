//! Standalone bloom filter using the double-hashing technique.
//!
//! Uses two SipHash-derived hashes to simulate `k` independent hash functions:
//!   h_i(x) = h1(x) + i * h2(x)  (mod bit_count)
//!
//! This is the Kirsch-Mitzenmacher optimization — two real hashes are sufficient
//! to simulate any number of hash functions without loss of asymptotic false
//! positive rate.
//!
//! The same implementation is used on both sides:
//! - **Client side** (`oxigraph-tikv`): builds the filter from the build-side
//!   hash join keys, serializes it, and sends it in the Coprocessor request.
//! - **Plugin side** (this crate): deserializes and checks probe-side keys
//!   against it to skip non-matching rows at the TiKV Region level.

use std::hash::{Hash, Hasher};

/// A simple bloom filter using double hashing (Kirsch-Mitzenmacher technique).
pub struct BloomFilter {
    /// The bit vector, stored as bytes (little-endian bit ordering within each byte).
    bits: Vec<u8>,
    /// Total number of bits in the filter (may not be a multiple of 8).
    bit_count: u64,
    /// Number of hash functions to simulate.
    num_hashes: u32,
}

impl BloomFilter {
    /// Create a new bloom filter sized for the expected number of items and
    /// desired false positive rate.
    ///
    /// # Panics
    ///
    /// Panics if `expected_items` is 0 or `false_positive_rate` is not in (0, 1).
    #[must_use]
    pub fn new(expected_items: usize, false_positive_rate: f64) -> Self {
        assert!(expected_items > 0, "expected_items must be > 0");
        assert!(
            false_positive_rate > 0.0 && false_positive_rate < 1.0,
            "false_positive_rate must be in (0, 1)"
        );

        // Optimal bit count: m = -n * ln(p) / (ln(2)^2)
        let n = expected_items as f64;
        let m = (-n * false_positive_rate.ln() / (std::f64::consts::LN_2.powi(2))).ceil() as u64;
        let m = m.max(8); // At least 8 bits

        // Optimal number of hash functions: k = (m/n) * ln(2)
        #[expect(
            clippy::cast_possible_truncation,
            reason = "hash count is always small"
        )]
        let k = ((m as f64 / n) * std::f64::consts::LN_2).ceil() as u32;
        let k = k.max(1);

        let byte_count = ((m + 7) / 8) as usize;

        Self {
            bits: vec![0u8; byte_count],
            bit_count: m,
            num_hashes: k,
        }
    }

    /// Create a bloom filter from pre-existing parameters (for deserialization
    /// on the plugin side, where the client already computed optimal sizing).
    #[must_use]
    pub fn from_raw(bits: Vec<u8>, num_hashes: u32) -> Self {
        let bit_count = (bits.len() as u64) * 8;
        Self {
            bits,
            bit_count,
            num_hashes,
        }
    }

    /// Insert an item into the bloom filter.
    pub fn insert(&mut self, item: &[u8]) {
        let (h1, h2) = self.hash_pair(item);
        for i in 0..self.num_hashes {
            let bit_idx = self.get_bit_index(h1, h2, i);
            self.set_bit(bit_idx);
        }
    }

    /// Check if an item might be in the set.
    ///
    /// Returns `true` if the item is possibly present (may be a false positive),
    /// or `false` if the item is definitely not present.
    #[must_use]
    pub fn check(&self, item: &[u8]) -> bool {
        let (h1, h2) = self.hash_pair(item);
        for i in 0..self.num_hashes {
            let bit_idx = self.get_bit_index(h1, h2, i);
            if !self.get_bit(bit_idx) {
                return false;
            }
        }
        true
    }

    /// Serialize the bloom filter to bytes.
    ///
    /// Format: [4 bytes num_hashes (LE)] [8 bytes bit_count (LE)] [bits...]
    #[must_use]
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + 8 + self.bits.len());
        out.extend_from_slice(&self.num_hashes.to_le_bytes());
        out.extend_from_slice(&self.bit_count.to_le_bytes());
        out.extend_from_slice(&self.bits);
        out
    }

    /// Deserialize a bloom filter from bytes produced by [`serialize`](Self::serialize).
    ///
    /// Returns `None` if the data is too short or malformed.
    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 12 {
            return None;
        }
        let num_hashes = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let bit_count = u64::from_le_bytes([
            data[4], data[5], data[6], data[7], data[8], data[9], data[10], data[11],
        ]);
        let bits = data[12..].to_vec();
        let expected_bytes = ((bit_count + 7) / 8) as usize;
        if bits.len() < expected_bytes {
            return None;
        }
        Some(Self {
            bits,
            bit_count,
            num_hashes,
        })
    }

    /// Compute two independent SipHash values for double hashing.
    fn hash_pair(&self, item: &[u8]) -> (u64, u64) {
        // Use SipHash with two different keys for independence.
        let h1 = {
            let mut hasher = siphasher::sip::SipHasher13::new_with_keys(0, 0);
            item.hash(&mut hasher);
            hasher.finish()
        };
        let h2 = {
            let mut hasher = siphasher::sip::SipHasher13::new_with_keys(1, 1);
            item.hash(&mut hasher);
            hasher.finish()
        };
        (h1, h2)
    }

    /// Compute the bit index for the i-th hash function using double hashing.
    fn get_bit_index(&self, h1: u64, h2: u64, i: u32) -> u64 {
        h1.wrapping_add(h2.wrapping_mul(u64::from(i))) % self.bit_count
    }

    fn set_bit(&mut self, bit_idx: u64) {
        let byte_idx = (bit_idx / 8) as usize;
        let bit_offset = (bit_idx % 8) as u8;
        self.bits[byte_idx] |= 1 << bit_offset;
    }

    fn get_bit(&self, bit_idx: u64) -> bool {
        let byte_idx = (bit_idx / 8) as usize;
        let bit_offset = (bit_idx % 8) as u8;
        (self.bits[byte_idx] & (1 << bit_offset)) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_check() {
        let mut bf = BloomFilter::new(100, 0.01);
        bf.insert(b"hello");
        bf.insert(b"world");

        assert!(bf.check(b"hello"));
        assert!(bf.check(b"world"));
        // "foobar" was never inserted — should almost certainly not match
        // (could be a false positive, but extremely unlikely with 100 items / 0.01 FP rate)
        assert!(!bf.check(b"foobar"));
    }

    #[test]
    fn serialize_roundtrip() {
        let mut bf = BloomFilter::new(50, 0.001);
        bf.insert(b"test_item_1");
        bf.insert(b"test_item_2");

        let serialized = bf.serialize();
        let bf2 = BloomFilter::from_bytes(&serialized).expect("deserialization should succeed");

        assert!(bf2.check(b"test_item_1"));
        assert!(bf2.check(b"test_item_2"));
        assert!(!bf2.check(b"never_inserted"));
    }

    #[test]
    fn from_raw_works() {
        let mut bf = BloomFilter::new(10, 0.1);
        bf.insert(b"abc");

        let raw_bits = bf.bits.clone();
        let num_hashes = bf.num_hashes;

        let bf2 = BloomFilter::from_raw(raw_bits, num_hashes);
        assert!(bf2.check(b"abc"));
    }

    #[test]
    fn empty_filter_checks_false() {
        let bf = BloomFilter::new(100, 0.01);
        assert!(!bf.check(b"anything"));
    }

    #[test]
    #[should_panic(expected = "expected_items must be > 0")]
    fn zero_items_panics() {
        drop(BloomFilter::new(0, 0.01));
    }

    #[test]
    fn from_bytes_too_short_returns_none() {
        assert!(BloomFilter::from_bytes(&[0u8; 5]).is_none());
    }
}
