#!/usr/bin/env bash
# Smoke test for a live Oxigraph SPARQL endpoint
#
# Usage:
#   ENDPOINT=https://oxigraph-ldary-dev.apps.rm3.7wse.p1.openshiftapps.com \
#   ./endpoint-smoke-test.sh

set -euo pipefail

ENDPOINT="${ENDPOINT:-https://oxigraph-ldary-dev.apps.rm3.7wse.p1.openshiftapps.com}"
TIMEOUT=30
PASS=0
FAIL=0
SKIP=0

run_test() {
    local name="$1"
    local expected_pattern="$2"
    shift 2
    echo -n "  $name ... "
    local output
    if output=$("$@" 2>&1); then
        if echo "$output" | grep -qE "$expected_pattern"; then
            echo "PASS"
            PASS=$((PASS + 1))
        else
            echo "FAIL (unexpected response)"
            echo "    Response: $(echo "$output" | head -3)"
            FAIL=$((FAIL + 1))
        fi
    else
        echo "FAIL (request failed)"
        echo "    Error: $(echo "$output" | head -3)"
        FAIL=$((FAIL + 1))
    fi
}

run_test_http_code() {
    local name="$1"
    local expected_code="$2"
    shift 2
    echo -n "  $name ... "
    local code
    code=$(curl -s -o /dev/null -w "%{http_code}" "$@" 2>/dev/null || echo "000")
    if [ "$code" = "$expected_code" ]; then
        echo "PASS (HTTP $code)"
        PASS=$((PASS + 1))
    else
        echo "FAIL (HTTP $code, expected $expected_code)"
        FAIL=$((FAIL + 1))
    fi
}

echo "============================================"
echo "Endpoint Smoke Test"
echo "============================================"
echo "Endpoint: $ENDPOINT"
echo ""

echo "[Health & Readiness]"
run_test_http_code "GET /health" "200" \
    --connect-timeout 10 --max-time "$TIMEOUT" "$ENDPOINT/health"

echo ""
echo "[Data Operations]"
run_test_http_code "POST /store (insert turtle)" "200|204" \
    --connect-timeout 10 --max-time "$TIMEOUT" \
    -X POST "$ENDPOINT/store" \
    -H 'Content-Type: text/turtle' \
    -d '@prefix ex: <http://example.org/smoke-test/> . ex:s1 ex:p "hello" .'

run_test_http_code "POST /update (SPARQL INSERT DATA)" "200|204" \
    --connect-timeout 10 --max-time "$TIMEOUT" \
    -X POST "$ENDPOINT/update" \
    -H 'Content-Type: application/sparql-update' \
    -d 'INSERT DATA { <http://example.org/smoke-test/s2> <http://example.org/smoke-test/p> "world" }'

echo ""
echo "[Query Operations]"
run_test "SELECT COUNT" '"value"' \
    curl -sf --connect-timeout 10 --max-time "$TIMEOUT" \
    "$ENDPOINT/query" \
    -H 'Content-Type: application/sparql-query' \
    -H 'Accept: application/sparql-results+json' \
    -d 'SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }'

run_test "ASK query" '"boolean"' \
    curl -sf --connect-timeout 10 --max-time "$TIMEOUT" \
    "$ENDPOINT/query" \
    -H 'Content-Type: application/sparql-query' \
    -H 'Accept: application/sparql-results+json' \
    -d 'ASK { <http://example.org/smoke-test/s1> ?p ?o }'

run_test "SELECT with FILTER" '"results"' \
    curl -sf --connect-timeout 10 --max-time "$TIMEOUT" \
    "$ENDPOINT/query" \
    -H 'Content-Type: application/sparql-query' \
    -H 'Accept: application/sparql-results+json' \
    -d 'SELECT ?o WHERE { <http://example.org/smoke-test/s1> ?p ?o } LIMIT 5'

echo ""
echo "[Named Graphs]"
run_test_http_code "POST /store?graph=... (named graph insert)" "200|204" \
    --connect-timeout 10 --max-time "$TIMEOUT" \
    -X POST "$ENDPOINT/store?graph=http://example.org/smoke-test/g1" \
    -H 'Content-Type: text/turtle' \
    -d '@prefix ex: <http://example.org/smoke-test/> . ex:s3 ex:p "in-graph" .'

run_test "Query named graph" '"results"' \
    curl -sf --connect-timeout 10 --max-time "$TIMEOUT" \
    "$ENDPOINT/query" \
    -H 'Content-Type: application/sparql-query' \
    -H 'Accept: application/sparql-results+json' \
    -d 'SELECT ?o WHERE { GRAPH <http://example.org/smoke-test/g1> { ?s ?p ?o } }'

echo ""
echo "[Cleanup]"
run_test_http_code "DELETE test data" "200|204" \
    --connect-timeout 10 --max-time "$TIMEOUT" \
    -X POST "$ENDPOINT/update" \
    -H 'Content-Type: application/sparql-update' \
    -d 'DELETE WHERE { <http://example.org/smoke-test/s1> ?p ?o } ; DELETE WHERE { <http://example.org/smoke-test/s2> ?p ?o } ; DROP GRAPH <http://example.org/smoke-test/g1>'

echo ""
echo "============================================"
echo "Results: $PASS passed, $FAIL failed, $SKIP skipped"
echo "============================================"

if (( FAIL > 0 )); then
    exit 1
fi
