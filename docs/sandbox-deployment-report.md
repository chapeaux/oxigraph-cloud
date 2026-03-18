# Developer Sandbox Deployment Report

**Date**: 2026-03-18
**Cluster**: `api.rm3.7wse.p1.openshiftapps.com`
**Namespace**: `ldary-dev`

## Deployed Resources

| Resource | Name | Status |
|----------|------|--------|
| StatefulSet | `oxigraph` | 1/1 Running |
| Service | `oxigraph` | ClusterIP 172.30.34.126:7878 |
| Route | `oxigraph` | `oxigraph-ldary-dev.apps.rm3.7wse.p1.openshiftapps.com` (edge TLS) |
| PVC | `oxigraph-storage-oxigraph-0` | 1Gi RWO (gp3) |

**Image**: `quay.io/ldary/oxigraph-cloud:0.5.6`
**Backend**: RocksDB (embedded)
**SHACL**: Disabled (built without `shacl` feature for UBI 9 compat)
**Write Auth**: Enabled (`--write-key`)

## End-to-End Validation Results

| Test | Result |
|------|--------|
| `GET /health` | PASS — returned `OK` |
| `GET /ready` | PASS — returned `READY` |
| `POST /store` (insert triple) | PASS — HTTP 204 |
| `POST /query` (SELECT) | PASS — returned inserted triple |
| `POST /update` (SPARQL UPDATE) | PASS — HTTP 204 |
| Query verify after UPDATE | PASS — data persisted |
| Triple count | 32,459 triples loaded |
| Unauthorized write (no key) | PASS — HTTP 401 |
| SHACL endpoints | N/A — feature not compiled in |

## Resource Usage

```
Namespace quota: 3 CPU / 30Gi memory
StatefulSet:     1 replica, 1Gi PVC
Container:       Default limits (10m CPU / 64Mi memory request, 1 CPU / 1000Mi limit)
```

## Helm Chart Validation

```
helm lint ./deploy/helm/oxigraph-cloud — PASSED
helm lint -f values-sandbox.yaml    — PASSED
helm lint -f values-tikv.yaml       — PASSED
```

## Known Issues

1. **SHACL disabled**: The `shacl` feature requires rudof crates that need nightly features on UBI 9's older toolchain. A newer Rust toolchain (1.87+) in the Containerfile resolves this for non-UBI builds.
2. **TiKV not deployed**: Developer Sandbox resources are too constrained for TiKV (needs 3 nodes). RocksDB backend is the correct choice for sandbox.

## OpenShift-Specific Notes

- Route uses `edge` TLS termination (automatic cert)
- Pod runs as UID assigned by OpenShift (not fixed UID 1001)
- `gp3` StorageClass used for PVC
- CronJob `oxigraph-sync` runs periodically (data sync from external source)
