# TiKV Cluster Sizing Guide

## Key Factors

Each RDF triple generates entries in multiple indexes (SPOG, POSG, OSPG + default-graph variants), plus dictionary entries (id2str). Expect **~6-8 KV pairs per triple** with an average of **~100-200 bytes per pair**.

## Sizing by Dataset

| Dataset Size | Triples | Est. KV Pairs | Est. Storage | TiKV Nodes | PD Nodes | CPU/Node | RAM/Node |
|-------------|---------|---------------|-------------|------------|----------|----------|----------|
| Small       | < 1M    | ~8M           | ~2 GB       | 1          | 1        | 2 cores  | 4 GB     |
| Medium      | 1-100M  | ~800M         | ~200 GB     | 3          | 3        | 4 cores  | 16 GB    |
| Large       | 100M+   | ~8B+          | ~2 TB+      | 5+         | 3        | 8 cores  | 32 GB    |

## Region Count Estimation

TiKV splits data into Regions (default 96 MB each):
- **Small**: ~20 Regions
- **Medium**: ~2,000 Regions
- **Large**: ~20,000+ Regions

Oxigraph's key pattern creates many small values, so actual Region count may be higher than pure size estimates.

## Storage Recommendations

- Use **SSD/NVMe** for TiKV data directories — rotational disks are not recommended
- Provision **2x** the estimated storage for compaction headroom
- Use a dedicated **StorageClass** in Kubernetes with `volumeBindingMode: WaitForFirstConsumer`

## Memory Sizing

- TiKV's block cache defaults to 45% of total memory
- PD requires minimal memory (~512 MB for small clusters)
- For Oxigraph's pattern (many prefix scans), increase `readpool.coprocessor.max-tasks-per-worker-normal`
