# TiKV Performance Tuning for Oxigraph

## Region Configuration

Oxigraph generates many small KV pairs. Tune Region settings accordingly:

```toml
# tikv.toml
[raftstore]
# Smaller regions for better prefix scan locality
region-max-size = "64MB"
region-split-size = "48MB"

# Merge small regions to reduce heartbeat overhead
merge-max-region-size = "32MB"
merge-check-tick-interval = "10s"

# Hibernate idle regions to save CPU
hibernate-regions = true
```

## Write Performance

```toml
[rocksdb.defaultcf]
# Oxigraph bulk loads benefit from larger write buffers
write-buffer-size = "128MB"
max-write-buffer-number = 5

[raftstore]
# Batch Raft proposals for bulk insert
raft-max-inflight-msgs = 256
store-batch-system.pool-size = 4
```

## Read Performance

```toml
[readpool.coprocessor]
# Increase for concurrent SPARQL queries
max-tasks-per-worker-normal = 2000

[storage.block-cache]
# Allocate more memory to block cache for read-heavy workloads
capacity = "8GB"
```

## GC and Compaction

```toml
[gc]
# Oxigraph rarely updates keys, so GC can be less aggressive
max-write-bytes-per-sec = "0"  # unlimited during GC

[rocksdb.defaultcf]
# Reduce write amplification
level0-file-num-compaction-trigger = 4
dynamic-level-bytes = true
```

## Monitoring Key Metrics

| Metric | Healthy Range | Action if Exceeded |
|--------|--------------|-------------------|
| Raftstore CPU | < 80% | Add nodes or increase pool size |
| Region heartbeat latency | < 100ms | Check network, reduce Region count |
| gRPC duration (P99) | < 50ms | Check disk I/O, increase cache |
| Pending compaction bytes | < 1GB | Increase compaction threads |
