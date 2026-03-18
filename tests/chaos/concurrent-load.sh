#!/bin/bash
# Chaos test: Concurrent readers and writers
set -euo pipefail

ENDPOINT="${1:-https://oxigraph-ldary-dev.apps.rm3.7wse.p1.openshiftapps.com}"
KEY="${2:-changeme-dev-key}"
WRITERS="${3:-5}"
READERS="${4:-10}"
ITERS="${5:-20}"

echo "Concurrent load: $WRITERS writers x $READERS readers x $ITERS iterations"

for w in $(seq 1 "$WRITERS"); do
  (for i in $(seq 1 "$ITERS"); do
    code=$(curl -sf -o /dev/null -w "%{http_code}" -X POST "$ENDPOINT/store" \
      -H "Authorization: Bearer $KEY" -H 'Content-Type: application/n-triples' \
      --data-raw "<http://load/$w-$i> <http://load/p> \"v$w-$i\" ." \
      --connect-timeout 10 --max-time 30 2>/dev/null || echo "000")
    [ "$code" != "204" ] && echo "W$w/$i: FAIL $code" >&2
  done) &
done

for r in $(seq 1 "$READERS"); do
  (for i in $(seq 1 "$ITERS"); do
    code=$(curl -sf -o /dev/null -w "%{http_code}" "$ENDPOINT/query" \
      -H 'Content-Type: application/sparql-query' \
      --data-raw 'SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' \
      --connect-timeout 10 --max-time 30 2>/dev/null || echo "000")
    [ "$code" != "200" ] && echo "R$r/$i: FAIL $code" >&2
  done) &
done

wait
curl -sf "$ENDPOINT/health" && echo "Final health: OK" || echo "Final health: FAIL"
curl -sf "$ENDPOINT/query" -H 'Content-Type: application/sparql-query' \
  --data-raw 'SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }'
echo ""
