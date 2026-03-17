# Oxigraph Cloud-Native: Developer Sandbox Quick-Start

Deploy Oxigraph on the Red Hat Developer Sandbox in under 15 minutes.

## Prerequisites

- A [Red Hat Developer Sandbox](https://developers.redhat.com/developer-sandbox) account (free)
- The `oc` CLI installed: [Install OpenShift CLI](https://docs.openshift.com/container-platform/latest/cli_reference/openshift_cli/getting-started-cli.html)
- The `helm` CLI installed (v3.x): [Install Helm](https://helm.sh/docs/intro/install/)

## Step 1: Log in to the Developer Sandbox

1. Open your Developer Sandbox console at https://console.redhat.com/openshift/sandbox
2. Click the username dropdown in the top-right corner and select **Copy login command**
3. Click **Display Token** and copy the `oc login` command
4. Run it in your terminal:

```bash
oc login --token=<your-token> --server=https://<your-cluster-api>
```

Verify you are connected:

```bash
oc whoami
oc project
```

## Step 2: Deploy Oxigraph with Helm

Clone the repository and install using the sandbox values overlay:

```bash
git clone https://github.com/oxigraph/oxigraph-cloud-native.git
cd oxigraph-cloud-native

helm install oxigraph ./helm/oxigraph-cloud \
  -f ./helm/oxigraph-cloud/values-sandbox.yaml
```

This deploys Oxigraph with:
- RocksDB backend (embedded, no external dependencies)
- 50m/128Mi CPU/memory requests, 500m/512Mi limits
- 1Gi persistent volume
- An OpenShift Route for external access

## Step 3: Verify the Deployment

Wait for the pod to become ready:

```bash
oc get pods -w
```

You should see output like:

```
NAME          READY   STATUS    RESTARTS   AGE
oxigraph-0    1/1     Running   0          45s
```

Get the Route URL:

```bash
export OXIGRAPH_URL=https://$(oc get route oxigraph-oxigraph-cloud -o jsonpath='{.spec.host}')
echo "Oxigraph endpoint: ${OXIGRAPH_URL}"
```

Test connectivity:

```bash
curl -s "${OXIGRAPH_URL}"
```

You should see the Oxigraph web interface HTML response.

## Step 4: Run Your First SPARQL Query

### Insert Data

Insert some RDF triples using the Turtle format:

```bash
curl -X POST "${OXIGRAPH_URL}/store" \
  -H "Content-Type: text/turtle" \
  -d '
@prefix ex: <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a foaf:Person ;
  foaf:name "Alice" ;
  foaf:age 30 ;
  foaf:knows ex:bob .

ex:bob a foaf:Person ;
  foaf:name "Bob" ;
  foaf:age 25 ;
  foaf:knows ex:alice .

ex:carol a foaf:Person ;
  foaf:name "Carol" ;
  foaf:age 35 .
'
```

A successful insert returns HTTP 204 (No Content).

### Query Data with SPARQL SELECT

```bash
curl -s "${OXIGRAPH_URL}/query" \
  -H "Accept: application/sparql-results+json" \
  --data-urlencode "query=
    PREFIX foaf: <http://xmlns.com/foaf/0.1/>
    SELECT ?name ?age WHERE {
      ?person a foaf:Person ;
        foaf:name ?name ;
        foaf:age ?age .
    }
    ORDER BY ?name
  " | python3 -m json.tool
```

Expected output:

```json
{
    "results": {
        "bindings": [
            { "name": { "type": "literal", "value": "Alice" }, "age": { "type": "literal", "value": "30", "datatype": "http://www.w3.org/2001/XMLSchema#integer" } },
            { "name": { "type": "literal", "value": "Bob" }, "age": { "type": "literal", "value": "25", "datatype": "http://www.w3.org/2001/XMLSchema#integer" } },
            { "name": { "type": "literal", "value": "Carol" }, "age": { "type": "literal", "value": "35", "datatype": "http://www.w3.org/2001/XMLSchema#integer" } }
        ]
    }
}
```

### Query with SPARQL UPDATE (INSERT)

```bash
curl -X POST "${OXIGRAPH_URL}/update" \
  -H "Content-Type: application/sparql-update" \
  -d '
    PREFIX ex: <http://example.org/>
    PREFIX foaf: <http://xmlns.com/foaf/0.1/>

    INSERT DATA {
      ex:dave a foaf:Person ;
        foaf:name "Dave" ;
        foaf:age 28 ;
        foaf:knows ex:alice .
    }
  '
```

### Count All Triples

```bash
curl -s "${OXIGRAPH_URL}/query" \
  -H "Accept: application/sparql-results+json" \
  --data-urlencode "query=SELECT (COUNT(*) AS ?count) WHERE { ?s ?p ?o }"
```

## Step 5: Upload SHACL Shapes (Optional)

SHACL validation is off by default in the sandbox deployment. To enable it, upgrade the release with validation turned on:

```bash
helm upgrade oxigraph ./helm/oxigraph-cloud \
  -f ./helm/oxigraph-cloud/values-sandbox.yaml \
  --set shacl.mode=enforce
```

Then upload a shapes graph:

```bash
curl -X POST "${OXIGRAPH_URL}/shacl" \
  -H "Content-Type: text/turtle" \
  -d '
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

<http://example.org/PersonShape> a sh:NodeShape ;
  sh:targetClass foaf:Person ;
  sh:property [
    sh:path foaf:name ;
    sh:minCount 1 ;
    sh:maxCount 1 ;
    sh:datatype xsd:string ;
  ] ;
  sh:property [
    sh:path foaf:age ;
    sh:minCount 1 ;
    sh:datatype xsd:integer ;
    sh:minInclusive 0 ;
  ] .
'
```

With SHACL enforcement active, inserting data that violates the shapes will return HTTP 422 with a validation report.

Test with invalid data (missing required `foaf:name`):

```bash
curl -v -X POST "${OXIGRAPH_URL}/store" \
  -H "Content-Type: text/turtle" \
  -d '
@prefix ex: <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .

ex:invalid a foaf:Person ;
  foaf:age "not-a-number" .
'
```

This should be rejected because `foaf:name` is missing and `foaf:age` has the wrong datatype.

## Cleanup

To remove the deployment:

```bash
helm uninstall oxigraph
```

To also remove the persistent volume claim:

```bash
oc delete pvc -l app.kubernetes.io/name=oxigraph-cloud
```

## Troubleshooting

**Pod stuck in Pending**: Check if the PVC can be bound. Developer Sandbox provides limited storage.

```bash
oc get pvc
oc describe pod oxigraph-0
```

**Route not accessible**: Verify the Route was created and has a host assigned.

```bash
oc get route
```

**Pod crash-looping**: Check logs for startup errors.

```bash
oc logs oxigraph-0
```

**Resource quota exceeded**: The sandbox has strict CPU and memory limits. The sandbox values file is tuned for these constraints, but if other workloads are using resources, you may need to scale down other deployments first.

```bash
oc describe resourcequota
```
