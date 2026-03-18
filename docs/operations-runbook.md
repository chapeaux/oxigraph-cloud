# Operations Runbook

## Day-to-Day Operations

### Health Check

```bash
# Liveness
curl -f http://oxigraph:7878/health

# Readiness (checks backend connectivity)
curl -f http://oxigraph:7878/ready
```

### Check Store Size

```bash
curl -s http://oxigraph:7878/query \
  -H 'Content-Type: application/sparql-query' \
  -d 'SELECT (COUNT(*) AS ?triples) WHERE { ?s ?p ?o }'
```

### Load Data

```bash
curl -X POST http://oxigraph:7878/store \
  -H 'Content-Type: text/turtle' \
  -H 'Authorization: Bearer <write-key>' \
  --data-binary @data.ttl
```

## Scaling

### Horizontal (TiKV backend)
Add TiKV nodes — data rebalances automatically via PD:
```bash
tiup cluster scale-out <cluster-name> scale-out.yaml
```

### Vertical
Increase resource limits in Helm values and upgrade:
```bash
helm upgrade oxigraph-cloud ./deploy/helm/oxigraph-cloud \
  --set resources.limits.memory=4Gi
```

## Incident Response

### Oxigraph Pod CrashLooping
1. Check logs: `kubectl logs -f deploy/oxigraph-cloud`
2. Common causes: TiKV unreachable, disk full (RocksDB), OOM
3. Fix: check PD endpoints, increase PVC size, increase memory limits

### TiKV Node Down
1. Check: `tiup ctl pd store -u http://pd:2379`
2. TiKV tolerates minority failure (1 of 3 nodes)
3. Replace failed node; data replicates automatically

### High Query Latency
1. Check Grafana dashboard for query P99
2. Common causes: full table scans, missing index, Region hotspot
3. Fix: optimize SPARQL query, check TiKV Region distribution

### SHACL Validation Blocking Inserts
1. Check mode: `curl http://oxigraph:7878/shacl/mode`
2. Temporarily switch to warn: `curl -X PUT http://oxigraph:7878/shacl/mode -d '{"mode":"warn"}'`
3. Investigate validation report for the failing data
