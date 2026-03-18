# TiKV Local Development Setup

## Option 1: tiup playground (Recommended)

```bash
# Install tiup
curl --proto '=https' --tlsv1.2 -sSf https://tiup-mirrors.pingcap.com/install.sh | sh
source ~/.bashrc

# Start a local cluster (1 PD + 3 TiKV)
tiup playground --tag oxigraph-dev --pd 1 --kv 3

# PD will be available at 127.0.0.1:2379
```

### Verify the cluster

```bash
# Check cluster status
tiup ctl:v8.5.0 pd -u http://127.0.0.1:2379 store

# Test with oxigraph-cloud
cargo run -p oxigraph-server --features tikv -- \
  --backend tikv --pd-endpoints 127.0.0.1:2379 --bind 127.0.0.1:7878
```

## Option 2: Docker Compose

```bash
docker compose -f deploy/docker-compose.yml up
```

This starts PD + 3 TiKV nodes + Oxigraph Cloud server. The SPARQL endpoint is at `http://localhost:7878`.

## Verification

```bash
# Insert a triple
curl -X POST http://localhost:7878/store \
  -H 'Content-Type: text/turtle' \
  -d '@prefix ex: <http://example.org/> . ex:s ex:p ex:o .'

# Query
curl -X POST http://localhost:7878/query \
  -H 'Content-Type: application/sparql-query' \
  -d 'SELECT * WHERE { ?s ?p ?o }'
```

## Troubleshooting

| Issue | Solution |
|-------|----------|
| `tikv-client` connection refused | Ensure PD is running on the configured port |
| Region not available | Wait for TiKV nodes to finish bootstrapping (~10s) |
| "key not in region" errors | Cluster may need more time to split Regions |
| `tiup playground` crashes | Check disk space; TiKV needs at least 1GB free |
