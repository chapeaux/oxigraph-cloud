# Oxigraph's Current Storage Architecture and Encoding

## Key-Value Table Indexing

Oxigraph implements persistent storage using **eleven distinct key-value tables** within RocksDB, using dictionary encoding and multiple index permutations.

### Table Structure

1. **Dictionary Table (`id2str`)** — Maps compressed string identifiers (hashes) back to full string representations. Reduces storage footprint for repeated URIs/IRIs/literals.

2. **Default Graph Quad Tables** (3 tables):
   - **SPO** (Subject → Predicate → Object) — Queries where subject is known
   - **POS** (Predicate → Object → Subject) — Inbound link traversals
   - **OSP** (Object → Subject → Predicate) — Queries where only object is known

3. **Named Graph Tables** (4 tables) — Incorporate Graph component (G) into sorting permutations: SPOG, POSG, OSPG, GSPO

4. **Graph Directory Table** — Definitive list of all existing named graphs

## Byte-Level Term Encoding

Oxigraph encodes RDF terms with:
- A **leading type byte** defining the term kind
- A **fixed-length value** of at most **32 bytes**

### Encoding Formats

| Term Type | Encoding Strategy |
|-----------|------------------|
| NamedNode | 128-bit cryptographic hash of full string |
| NumericalBlankNode | Direct u128 numerical value |
| SmallBlankNode | Inline storage (< 16 bytes) |
| BigBlankNode | Hashed |
| SmallStringLiteral | Inline |
| BigStringLiteral | Hashed |
| Numeric types | Type-specific inline encoding |

### Implications for Distributed Storage

- Keys are 32-byte structures forming SPO/POS/OSP index entries
- Any distributed backend **must** support ordered, lexicographical range scans
- Hash-based partition distribution (Memcached, default DynamoDB) will **fail** — sequential prefix scans are mandatory for resolving SPARQL Basic Graph Patterns (BGPs)

## Transactional Guarantees and Concurrency Control

### Current RocksDB Behavior
- **Writes**: Buffered and executed as atomic batch at transaction end
- **Reads**: Snapshot taken at transaction start → "repeatable read" isolation
- Only exposes fully committed changes

### Concurrency Limitations
- In-memory mode: MVCC with **single concurrent write transaction** (full serializability but severe write throughput bottleneck)
- Cloud-native backend must provide distributed MVCC with concurrent write support while maintaining ACID guarantees

## Key Source References
- Architecture wiki: https://github.com/oxigraph/oxigraph/wiki/Architecture
- Store API: https://docs.rs/oxigraph/latest/oxigraph/store/struct.Store.html
