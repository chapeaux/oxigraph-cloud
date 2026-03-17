# Overview: Cloud-Native Distributed SPARQL and SHACL Database

## Project Goal

Transform Oxigraph from a high-performance embedded RDF database into a cloud-native, distributed SPARQL and SHACL platform by:

1. **Integrating SHACL validation** via the Rust-native `rudof` crate
2. **Decoupling the storage layer** from RocksDB to support distributed backends (TiKV selected)
3. **Deploying on Kubernetes/OpenShift** with a Developer Sandbox variation

## Why Oxigraph

- Written entirely in Rust — memory-safe, no data races
- Highly compliant SPARQL 1.1 implementation
- Current persistence bound to embedded RocksDB (single-node LSM-tree engine)
- Lacks distributed consensus, geographic replication, or decoupled storage/compute

## Two Mandatory Architectural Evolutions

### 1. SHACL Validation (Rudof Integration)
- Enforce schema compliance and data quality at ingestion time
- Use the `rudof` crate's `shacl_validation` module
- Bridge via the SRDF trait abstraction

### 2. Distributed Storage Backend (TiKV)
- Decouple Oxigraph's compute layer from RocksDB
- Introduce a `StorageBackend` trait for pluggable backends
- TiKV selected as the optimal backend (Rust-native, Raft consensus, range-based partitioning, Coprocessor pushdown)

## Key References

- Oxigraph: https://github.com/oxigraph/oxigraph
- Rudof: https://github.com/rudof-project/rudof
- TiKV: https://tikv.org
- Architecture discussion: https://github.com/oxigraph/oxigraph/discussions/1487
