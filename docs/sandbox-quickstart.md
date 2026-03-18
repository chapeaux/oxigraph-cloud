# Developer Sandbox Quick Start

Deploy Oxigraph Cloud on the Red Hat Developer Sandbox in under 15 minutes.

## Prerequisites

1. Sign up at [Red Hat Developer Sandbox](https://developers.redhat.com/developer-sandbox)
2. Install the `oc` CLI or use the web terminal

## Step 1: Deploy

```bash
oc login --token=<your-token> --server=<your-server>

helm install oxigraph-cloud ./deploy/helm/oxigraph-cloud \
  -f deploy/helm/oxigraph-cloud/values-sandbox.yaml
```

Or with raw manifests:
```bash
oc apply -k deploy/openshift/
```

## Step 2: Access the Endpoint

```bash
ENDPOINT=$(oc get route oxigraph-cloud -o jsonpath='{.spec.host}')
curl https://$ENDPOINT/health
```

## Step 3: Load Sample Data

```bash
curl -X POST https://$ENDPOINT/store \
  -H 'Content-Type: text/turtle' \
  --data-binary @tests/data/sample-dataset.ttl
```

## Step 4: Run Your First Query

```bash
curl -s https://$ENDPOINT/query \
  -H 'Content-Type: application/sparql-query' \
  -H 'Accept: application/sparql-results+json' \
  -d 'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
      SELECT ?name WHERE { ?p a foaf:Person ; foaf:name ?name }'
```

## Step 5: Try SHACL Validation

```bash
# Upload shapes
curl -X POST https://$ENDPOINT/shacl/shapes \
  -H 'Content-Type: text/turtle' \
  --data-binary @tests/data/sample-shapes.ttl

# Enable validation
curl -X PUT https://$ENDPOINT/shacl/mode \
  -H 'Content-Type: application/json' \
  -d '{"mode": "enforce"}'

# Try invalid data (should fail)
curl -X POST https://$ENDPOINT/store \
  -H 'Content-Type: text/turtle' \
  --data-binary @tests/data/invalid-data.ttl
```

## Resource Usage

CPU: 250m/500m | Memory: 256Mi/512Mi | Storage: 1Gi | Backend: RocksDB
