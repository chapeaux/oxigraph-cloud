# Backup and Restore

## TiKV Backup (tikv-br)

### Full Backup to S3

```bash
tiup br backup full \
  --pd "pd-host:2379" \
  --storage "s3://bucket-name/oxigraph-backup/$(date +%Y%m%d)" \
  --s3.region us-east-1
```

### Full Backup to Local Storage

```bash
tiup br backup full \
  --pd "pd-host:2379" \
  --storage "local:///backup/oxigraph/$(date +%Y%m%d)"
```

### Scheduled Backups (Kubernetes CronJob)

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: oxigraph-backup
spec:
  schedule: "0 2 * * *"  # Daily at 2 AM
  jobTemplate:
    spec:
      template:
        spec:
          containers:
            - name: backup
              image: pingcap/br:latest
              command:
                - /br
                - backup
                - full
                - --pd=tikv-pd:2379
                - --storage=s3://backup-bucket/oxigraph/daily
          restartPolicy: OnFailure
```

## Restore

### Restore to New Cluster

```bash
# 1. Deploy a fresh TiKV cluster
# 2. Restore from backup
tiup br restore full \
  --pd "new-pd-host:2379" \
  --storage "s3://bucket-name/oxigraph-backup/20260318"

# 3. Point Oxigraph to the new cluster
oxigraph-cloud --backend tikv --pd-endpoints new-pd-host:2379
```

### Restore to Existing Cluster (Overwrites Data)

```bash
tiup br restore full \
  --pd "pd-host:2379" \
  --storage "s3://bucket-name/oxigraph-backup/20260318"
```

## RocksDB Backend Backup

For single-node RocksDB deployments, back up the data directory:

```bash
# Stop the server first (or use a filesystem snapshot)
tar czf oxigraph-backup.tar.gz /opt/oxigraph/data/
```

## Verification

After restore, verify data integrity:

```bash
curl -s http://localhost:7878/query \
  -H 'Content-Type: application/sparql-query' \
  -d 'SELECT (COUNT(*) AS ?count) WHERE { ?s ?p ?o }'
```
