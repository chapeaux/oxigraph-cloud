# TiKV Key Encoding: Column Family to Flat Key Space Mapping

## Problem

RocksDB provides **column families** -- logically separate key spaces within a single database. Oxigraph uses 12 column families. TiKV exposes a single flat key space. We need a prefix scheme that:

1. Keeps each logical table isolated (no key collisions)
2. Preserves lexicographic ordering within each table
3. Distributes well across TiKV Regions

## Prefix Byte Mapping

Each former column family gets a unique single-byte prefix. The prefix is the **first byte** of every key written to TiKV.

| Prefix | Hex  | Table   | Description                            | Key Suffix                |
|--------|------|---------|----------------------------------------|---------------------------|
| `0x00` | `00` | default | Metadata (storage version)             | `b"version"`              |
| `0x01` | `01` | id2str  | Dictionary: StrHash -> String          | `[16 bytes: StrHash]`     |
| `0x02` | `02` | spog    | Subject-Predicate-Object-Graph         | `[S][P][O][G]` encoded    |
| `0x03` | `03` | posg    | Predicate-Object-Subject-Graph         | `[P][O][S][G]` encoded    |
| `0x04` | `04` | ospg    | Object-Subject-Predicate-Graph         | `[O][S][P][G]` encoded    |
| `0x05` | `05` | gspo    | Graph-Subject-Predicate-Object         | `[G][S][P][O]` encoded    |
| `0x06` | `06` | gpos    | Graph-Predicate-Object-Subject         | `[G][P][O][S]` encoded    |
| `0x07` | `07` | gosp    | Graph-Object-Subject-Predicate         | `[G][O][S][P]` encoded    |
| `0x08` | `08` | dspo    | Default graph: Subject-Predicate-Object| `[S][P][O]` encoded       |
| `0x09` | `09` | dpos    | Default graph: Predicate-Object-Subject| `[P][O][S]` encoded       |
| `0x0A` | `0A` | dosp    | Default graph: Object-Subject-Predicate| `[O][S][P]` encoded       |
| `0x0B` | `0B` | graphs  | Named graph directory                  | `[G]` encoded             |

Prefixes `0x0C`--`0xFF` are reserved for future tables.

## Key Format

Every TiKV key is:

```
[1 byte: table prefix][N bytes: original key from Oxigraph's encoder]
```

The value bytes are unchanged from RocksDB -- they are stored as-is.

### Concrete Examples

Each encoded RDF term is 1 type byte + up to 32 value bytes (variable length, self-describing via the type byte). A quad index key concatenates 3 or 4 terms.

**Example 1: id2str entry**

A `StrHash` is 16 bytes (u128). To store the mapping for `<http://example.org/Alice>`:

```
Key:   [0x01][a3 b2 c1 ... 16 bytes of StrHash]
Value: b"http://example.org/Alice"
```

**Example 2: spog quad entry**

For the quad `<Alice> <knows> <Bob> <graph1>` where each term is a NamedNode (type byte `0x01` + 16-byte hash):

```
Key:   [0x02][0x01][hash_Alice: 16B][0x01][hash_knows: 16B][0x01][hash_Bob: 16B][0x01][hash_graph1: 16B]
        ^prefix    ^--- S: 17B ---  ^--- P: 17B ---       ^--- O: 17B ---      ^--- G: 17B ---
Total key size: 1 + 4*17 = 69 bytes
Value: [] (empty -- presence of key is the index entry)
```

**Example 3: dspo (default graph) entry**

Same quad but in the default graph (3 terms, no G component):

```
Key:   [0x08][0x01][hash_Alice: 16B][0x01][hash_knows: 16B][0x01][hash_Bob: 16B]
Total key size: 1 + 3*17 = 52 bytes
Value: []
```

**Example 4: Metadata (storage version)**

```
Key:   [0x00]b"version"     (8 bytes total)
Value: [0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]   (u64 LE = 2)
```

## Prefix Scan Translation

Oxigraph's query engine performs prefix scans within column families. In TiKV, these translate to bounded range scans.

### RocksDB scan within column family

```
cf("spog").prefix_iterator(subject_prefix)
```

### Equivalent TiKV bounded scan

```
start_key = [0x02] ++ subject_prefix
end_key   = [0x02] ++ successor(subject_prefix)
tikv_client.scan(start_key..end_key)
```

### Full table scan

To scan all entries in a table (e.g., all spog entries):

```
start_key = [0x02]
end_key   = [0x03]    // next table prefix
tikv_client.scan(start_key..end_key)
```

## Region Alignment

TiKV partitions the key space into **Regions** (~96 MB default). The prefix scheme has these effects:

### Natural table isolation

Each prefix byte defines a contiguous key range. With reasonable data sizes, each table will occupy one or more Regions exclusively. The six quad index tables (spog, posg, ospg, gspo, gpos, gosp) will each have their own Regions -- enabling parallel scans across different index orderings.

### Parallelism benefits

- A SPARQL query using the `spog` index and another using `posg` will hit different Regions on potentially different TiKV nodes
- The `id2str` dictionary (prefix `0x01`) is isolated from index data, so dictionary lookups don't contend with index scans

### Region count considerations

For a dataset with N triples, Oxigraph stores roughly `6*N` quad index entries plus `3*N` default-graph entries (9 index copies total). Each key is ~52-69 bytes. For 100M triples:

- ~900M keys, ~50-60 GB of key data
- At 96 MB per Region: ~600-700 Regions
- Well within TiKV's comfortable operating range (problems start at 100K+ Regions)

### Tuning recommendations

- **Region size**: Keep the default 96 MB. Oxigraph's keys are small and access patterns benefit from moderate Region counts.
- **Hibernate Region**: Enable for the `id2str` (0x01) and `graphs` (0x0B) ranges, which are read-heavy and rarely mutated after bulk load.

## Implementation Notes

### Rust constants and helpers

```rust
// Table prefix constants
pub mod table {
    pub const DEFAULT:  u8 = 0x00;
    pub const ID2STR:   u8 = 0x01;
    pub const SPOG:     u8 = 0x02;
    pub const POSG:     u8 = 0x03;
    pub const OSPG:     u8 = 0x04;
    pub const GSPO:     u8 = 0x05;
    pub const GPOS:     u8 = 0x06;
    pub const GOSP:     u8 = 0x07;
    pub const DSPO:     u8 = 0x08;
    pub const DPOS:     u8 = 0x09;
    pub const DOSP:     u8 = 0x0A;
    pub const GRAPHS:   u8 = 0x0B;
}

/// Prepend a table prefix byte to a key.
#[inline]
pub fn prefixed_key(table: u8, key: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + key.len());
    out.push(table);
    out.extend_from_slice(key);
    out
}

/// Strip the table prefix from a TiKV key, returning the original key bytes.
/// Panics if `raw_key` is empty or has a mismatched prefix.
#[inline]
pub fn strip_prefix(table: u8, raw_key: &[u8]) -> &[u8] {
    debug_assert!(!raw_key.is_empty() && raw_key[0] == table);
    &raw_key[1..]
}

/// Compute the scan bounds for a prefix scan within a table.
///
/// Returns `(start_key_inclusive, end_key_exclusive)`.
///
/// - If `prefix` is empty, scans the entire table: `[table] .. [table+1]`
/// - Otherwise: `[table ++ prefix] .. [table ++ successor(prefix)]`
///
/// `successor(prefix)` is the lexicographically next prefix: increment the
/// last byte, or if it is 0xFF, truncate and carry. If all bytes are 0xFF,
/// fall back to `[table+1]` (scan to end of table).
pub fn scan_bounds(table: u8, prefix: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let start = prefixed_key(table, prefix);
    let end = match next_prefix(table, prefix) {
        Some(end) => end,
        None => vec![table + 1], // entire rest of table
    };
    (start, end)
}

/// Compute the exclusive upper bound for a prefix scan.
fn next_prefix(table: u8, prefix: &[u8]) -> Option<Vec<u8>> {
    if prefix.is_empty() {
        return None;
    }
    // Find the rightmost byte that is not 0xFF and increment it
    let mut end = prefixed_key(table, prefix);
    while let Some(&last) = end.last() {
        if last < 0xFF {
            *end.last_mut().unwrap() += 1;
            return Some(end);
        }
        end.pop(); // drop trailing 0xFF
    }
    None // all bytes were 0xFF; caller uses table+1
}
```

### Mapping QuadEncoding to table prefix

```rust
impl QuadEncoding {
    pub fn table_prefix(self) -> u8 {
        match self {
            Self::Spog => table::SPOG,
            Self::Posg => table::POSG,
            Self::Ospg => table::OSPG,
            Self::Gspo => table::GSPO,
            Self::Gpos => table::GPOS,
            Self::Gosp => table::GOSP,
            Self::Dspo => table::DSPO,
            Self::Dpos => table::DPOS,
            Self::Dosp => table::DOSP,
        }
    }
}
```

### Batch write pattern

When inserting a quad, Oxigraph writes to all relevant index tables atomically. In TiKV, this becomes a single transaction with multiple `put` calls:

```rust
async fn insert_quad(txn: &mut Transaction, quad: &EncodedQuad) {
    let mut buf = Vec::with_capacity(WRITTEN_TERM_MAX_SIZE * 4);

    // Named graph indexes (4-term keys)
    write_spog_quad(&mut buf, quad);
    txn.put(prefixed_key(table::SPOG, &buf), vec![]).await?;
    buf.clear();

    write_posg_quad(&mut buf, quad);
    txn.put(prefixed_key(table::POSG, &buf), vec![]).await?;
    buf.clear();

    // ... repeat for ospg, gspo, gpos, gosp

    // Default graph indexes (3-term keys, if graph is default)
    write_spo_quad(&mut buf, quad);
    txn.put(prefixed_key(table::DSPO, &buf), vec![]).await?;
    buf.clear();

    // ... repeat for dpos, dosp
}
```

### Storage version check on startup

```rust
async fn check_version(client: &TransactionClient) -> Result<(), StorageError> {
    let key = prefixed_key(table::DEFAULT, b"version");
    let snapshot = client.snapshot(client.current_timestamp().await?);
    match snapshot.get(key).await? {
        Some(value) => {
            let version = u64::from_le_bytes(value.try_into().unwrap());
            assert_eq!(version, LATEST_STORAGE_VERSION);
        }
        None => {
            // Fresh database -- write version
            let mut txn = client.begin_optimistic().await?;
            txn.put(
                prefixed_key(table::DEFAULT, b"version"),
                LATEST_STORAGE_VERSION.to_le_bytes().to_vec(),
            ).await?;
            txn.commit().await?;
        }
    }
    Ok(())
}
```

## Summary

| Aspect | Design Decision |
|--------|----------------|
| Prefix size | 1 byte (supports up to 256 tables) |
| Key format | `[prefix][original_key_bytes]` -- zero transformation of existing encoding |
| Ordering | Preserved -- prefix is constant within a table, so intra-table lex order is unchanged |
| Region alignment | Each table naturally falls into its own Region range |
| Scan translation | `cf.prefix_iterator(p)` becomes `tikv.scan(scan_bounds(table, p))` |
| Value format | Unchanged from RocksDB |
