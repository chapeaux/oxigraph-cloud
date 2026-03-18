# Troubleshooting Guide

## Connection Issues

### "TiKV support requires the 'tikv' feature"
Build with the tikv feature: `cargo build -p oxigraph-server --features tikv`

### "Connection refused" to PD
- Verify PD is running: `curl http://pd-host:2379/pd/api/v1/health`
- Check `--pd-endpoints` matches actual PD address
- In Kubernetes: ensure Service and network policies allow traffic

### "Region not available"
- TiKV nodes may still be bootstrapping (~10-30 seconds after start)
- Check TiKV logs for Region split/merge activity
- Verify all TiKV nodes are registered with PD

## Performance Issues

### Slow SPARQL Queries
1. Check if the query requires full table scan (no bound variables)
2. Use `LIMIT` to reduce result size
3. Check TiKV Region distribution for hotspots
4. Consider increasing `--query-timeout`

### Slow Bulk Loading
1. Use Turtle or N-Triples format (faster parsing than RDF/XML)
2. For large loads, split into batches of ~10K triples
3. Disable SHACL validation during bulk load (`--shacl-mode=off`)

### High Memory Usage
- RocksDB: tune block cache size via RocksDB options
- TiKV: check `storage.block-cache.capacity` in tikv.toml
- Server: reduce `--max-upload-size` to limit memory per request

## SHACL Issues

### "No SHACL shapes loaded"
Upload shapes first: `curl -X POST http://host:7878/shacl/shapes -d @shapes.ttl`

### Validation Failures on Valid Data
- Check shapes for overly strict constraints
- Verify data types match exactly (e.g., `xsd:string` vs plain literals)
- Use `/shacl/validate` endpoint to get detailed report

## Container Issues

### Pod Fails to Start on OpenShift
- Check SecurityContextConstraints: pod runs as UID 1001
- Verify image pull from container registry
- Check PVC binding if using RocksDB backend

### "Permission denied" on Data Directory
- Ensure `/opt/oxigraph/data` is owned by UID 1001
- In OpenShift, arbitrary UIDs are used — ensure `fsGroup: 0` is set
