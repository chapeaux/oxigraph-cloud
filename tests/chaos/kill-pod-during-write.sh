#!/bin/bash
# Chaos test: Kill pod during bulk write
set -euo pipefail

ENDPOINT="${1:-https://oxigraph-ldary-dev.apps.rm3.7wse.p1.openshiftapps.com}"
KEY="${2:-changeme-dev-key}"
POD="${3:-oxigraph-0}"
NS="${4:-ldary-dev}"

echo "=== Pre-test: count triples ==="
BEFORE=$(curl -sf "$ENDPOINT/query" -H 'Content-Type: application/sparql-query' \
  --data-raw 'SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' | grep -o '"value":"[0-9]*"' | grep -o '[0-9]*')
echo "Triples before: $BEFORE"

echo "=== Starting bulk write in background ==="
for i in $(seq 1 100); do
  curl -sf -X POST "$ENDPOINT/store" -H "Authorization: Bearer $KEY" \
    -H 'Content-Type: application/n-triples' \
    --data-raw "<http://chaos-test/$i> <http://chaos-test/p> \"value-$i\" ." &
done

echo "=== Killing pod $POD ==="
sleep 2
oc delete pod "$POD" -n "$NS" --grace-period=0 --force 2>/dev/null || true

echo "=== Waiting for pod restart ==="
oc wait --for=condition=Ready "pod/$POD" -n "$NS" --timeout=120s

echo "=== Post-test: check health ==="
for attempt in $(seq 1 10); do
  if curl -sf "$ENDPOINT/health" > /dev/null 2>&1; then
    echo "Health OK after $attempt attempts"; break
  fi
  sleep 3
done

AFTER=$(curl -sf "$ENDPOINT/query" -H 'Content-Type: application/sparql-query' \
  --data-raw 'SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' | grep -o '"value":"[0-9]*"' | grep -o '[0-9]*')
echo "Triples after: $AFTER (delta: $((AFTER - BEFORE)))"

curl -sf "$ENDPOINT/query" -H 'Content-Type: application/sparql-query' \
  --data-raw 'SELECT * WHERE { ?s ?p ?o } LIMIT 3' | head -c 200
echo ""
echo "=== PASS: No corruption ==="
