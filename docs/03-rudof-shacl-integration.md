# Integrating SHACL Validation via the Rudof Crate

## Rudof Architecture

`rudof` is a modular Rust library for parsing, validating, and managing RDF data shapes. Supports ShEx, SHACL, and DCTAP with conversion between them.

### Relevant Crates

| Crate | Purpose |
|-------|---------|
| `iri_s` | IRI handling structures |
| `srdf` | Simple RDF model — foundational abstraction for validation |
| `prefixmap` | Prefix map resolution (Turtle/TriG parsing) |
| `shacl_ast` | SHACL shapes Abstract Syntax Tree |
| `shacl_validation` | Validation algorithms against `shacl_ast` definitions |

### Platform Support
- Linux, Windows, macOS, Docker
- Python bindings (`pyrudof`)
- WebAssembly compilation

## The SRDF Trait: Bridge Between Rudof and Oxigraph

### Design
- Defines the **minimum RDF subset** needed for shape validation
- Primary capability: efficiently access the **neighborhood** of a node (outbound predicates/objects for a subject)
- Existing implementations: in-memory parsed RDF files, remote SPARQL endpoints

### Custom Implementation for Oxigraph

**Task**: Implement the SRDF trait for Oxigraph's `Store` struct.

**Approach**:
- Map neighborhood retrieval to `quads_for_pattern(Some(subject), None, None, None)`
- Bypasses `spareval` (evaluator) and `spargebra` (parser) entirely
- Routes directly to raw lexicographical range scans on SPO index
- Minimizes CPU overhead — operates at near bare-metal storage speeds

### Integration Flow
```
SHACL Shape Definition
    ↓
shacl_validation module
    ↓
SRDF trait (custom impl)
    ↓
Oxigraph Store::quads_for_pattern()
    ↓
Underlying KV storage (RocksDB or TiKV)
```

## Performance Benchmarks (10-LUBM Dataset)

| Engine | Language | Time (ms) |
|--------|----------|-----------|
| rdf4j | Java | 1.64 |
| **rudof** | **Rust** | **7.90** |
| Apache Jena | Java | 60.36 |
| TopQuadrant | Java | 85.74 |
| pyrudof | Python/Rust | 39,364.28 |
| pySHACL | Python | 72,227.29 |

- Rudof is **10.8x faster** than TopQuadrant
- pyrudof is **~2x faster** than pySHACL
- SHACL validation will not be a computational bottleneck

## Key References
- Rudof paper: https://ceur-ws.org/Vol-3828/paper32.pdf
- Rudof repo: https://github.com/rudof-project/rudof
- SRDF demo: https://labra.weso.es/pdf/2024_rudof_demo.pdf
- Refactoring discussion: https://github.com/rudof-project/rudof/discussions/212
