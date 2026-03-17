# Oxigraph Cloud-Native: TiKV & Rudof Integration

Using a team of agents, plan, implement, test, and distribute (via OpenShift and a separate Developer Sandbox variation) a cloud-native, distributed version of Oxigraph with TiKV storage and Rudof-based SHACL validation.

## Reference Documentation

The architecture research is split into focused documents under `docs/`:

| File | Content |
|------|---------|
| `docs/01-overview.md` | Project goals, high-level architecture, key decisions |
| `docs/02-oxigraph-storage-architecture.md` | Current KV tables, byte encoding, transactional guarantees |
| `docs/03-rudof-shacl-integration.md` | SRDF trait bridge, rudof crates, performance benchmarks |
| `docs/04-distributed-sparql-theory.md` | OLTP/OLAP theory, network bottleneck, ExtVP/semi-joins |
| `docs/05-tikv-backend.md` | TiKV architecture, Coprocessor pushdown, Region tuning |
| `docs/06-backend-alternatives-rejected.md` | FoundationDB, DynamoDB, S3/Parquet — why rejected |
| `docs/07-storage-trait-design.md` | StorageBackend trait design, async considerations |
| `docs/08-references.md` | All external references and links |

The original monolithic document is preserved in `cloud-native-oxigraph.md`.
