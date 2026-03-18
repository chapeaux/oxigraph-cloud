# Architecture Guide

## System Overview

```
                    ┌─────────────────────────┐
                    │   SPARQL/HTTP Clients    │
                    └────────────┬────────────┘
                                 │
                    ┌────────────▼────────────┐
                    │   oxigraph-server       │
                    │   (HTTP + SPARQL)       │
                    │                         │
                    │  ┌───────────────────┐  │
                    │  │ SHACL Validator   │  │
                    │  │ (oxigraph-shacl)  │  │
                    │  └───────────────────┘  │
                    └────────────┬────────────┘
                                 │
                    ┌────────────▼────────────┐
                    │   oxigraph (core)       │
                    │   SPARQL engine +       │
                    │   StorageBackend trait   │
                    └──────┬──────────┬───────┘
                           │          │
              ┌────────────▼──┐  ┌────▼───────────┐
              │   RocksDB     │  │   TiKV Client   │
              │   (embedded)  │  │   (distributed) │
              └───────────────┘  └────┬────────────┘
                                      │
                        ┌─────────────▼──────────────┐
                        │        TiKV Cluster        │
                        │  ┌─────┐ ┌─────┐ ┌─────┐  │
                        │  │TiKV │ │TiKV │ │TiKV │  │
                        │  └──┬──┘ └──┬──┘ └──┬──┘  │
                        │     └───────┼───────┘      │
                        │          ┌──▼──┐           │
                        │          │ PD  │           │
                        │          └─────┘           │
                        └────────────────────────────┘
```

## Component Roles

### oxigraph-server
HTTP server exposing SPARQL query/update, Graph Store Protocol, SHACL management API, and health/readiness probes.

### oxigraph (core library)
Forked Oxigraph with `StorageBackend` trait abstraction. Contains SPARQL 1.1 engine, query optimizer, and RDF I/O.

### StorageBackend Trait
Pluggable storage with implementations for RocksDB (embedded), Memory (testing), and TiKV (distributed).

### oxigraph-shacl
SHACL validation via rudof. Bridges Oxigraph Store to rudof's validation engine.

### oxigraph-coprocessor
TiKV Coprocessor plugin for query pushdown (scan, filter, aggregation on Region-local data).

## Key Encoding

| Prefix | Table | Purpose |
|--------|-------|---------|
| 0x00 | default | Metadata, version |
| 0x01 | id2str | Term dictionary |
| 0x02-0x04 | SPOG/POSG/OSPG | Named graph indexes |
| 0x05-0x07 | GSPO/GPOS/GOSP | Graph-first indexes |
| 0x08-0x0A | DSPO/DPOS/DOSP | Default graph indexes |
| 0x0B | graphs | Named graph registry |
