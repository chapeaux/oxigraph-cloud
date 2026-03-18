# Security Deployment Guide

## Authentication

### Write Key Authentication
All write operations (SPARQL UPDATE, data upload, SHACL management) require a Bearer token:

```bash
# Set via environment variable
export OXIGRAPH_WRITE_KEY="your-secret-key"

# Or via CLI flag
oxigraph-cloud --write-key "your-secret-key"

# Client usage
curl -X POST http://host:7878/update \
  -H 'Authorization: Bearer your-secret-key' \
  -H 'Content-Type: application/sparql-update' \
  -d 'INSERT DATA { <s> <p> <o> }'
```

### Kubernetes Secrets
```yaml
apiVersion: v1
kind: Secret
metadata:
  name: oxigraph-cloud-secrets
type: Opaque
stringData:
  write-key: "generate-a-strong-random-key"
```

## TLS Configuration

### mTLS Between Oxigraph and TiKV
TiKV supports TLS for all communication channels:

```toml
# tikv.toml
[security]
ca-path = "/etc/tikv/tls/ca.pem"
cert-path = "/etc/tikv/tls/server.pem"
key-path = "/etc/tikv/tls/server-key.pem"
```

Configure the Oxigraph TiKV client with matching certificates via environment variables or configuration.

### SPARQL Endpoint TLS
Use OpenShift Routes or Kubernetes Ingress with TLS termination:

```yaml
# OpenShift Route with edge TLS
spec:
  tls:
    termination: edge
    insecureEdgeTerminationPolicy: Redirect
```

## Network Policies

Restrict traffic to only necessary paths:

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: oxigraph-cloud
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/name: oxigraph-cloud
  policyTypes: [Ingress, Egress]
  ingress:
    - ports:
        - port: 7878
  egress:
    - to:
        - podSelector:
            matchLabels:
              app: tikv-pd
      ports:
        - port: 2379
    - to:
        - podSelector:
            matchLabels:
              app: tikv
      ports:
        - port: 20160
```

## Container Security

- Runs as non-root user (UID 1001)
- Based on UBI 9 minimal (reduced attack surface)
- No shell access in production image
- Read-only root filesystem recommended

## Security Scanning

```bash
# Rust dependency audit
cargo audit

# Container image CVE scan
podman scan oxigraph-cloud:latest

# SBOM generation
syft oxigraph-cloud:latest -o spdx-json > sbom.json
```
