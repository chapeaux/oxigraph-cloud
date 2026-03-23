# Conformance Report

**Date**: 2026-03-23 | **Version**: 0.7.3

## W3C SPARQL 1.1 Conformance

Tested via `cargo test -p oxigraph-testsuite` against RocksDB/memory backend.

| Test Suite | Status |
|-----------|--------|
| SPARQL 1.0 Query Syntax | PASS |
| SPARQL 1.0 Query Evaluation | PASS |
| SPARQL 1.1 Query Evaluation | PASS |
| SPARQL 1.1 Update Evaluation | PASS |
| SPARQL 1.1 Federation | PASS |
| SPARQL 1.1 JSON Results | PASS |
| SPARQL 1.1 TSV Results | PASS |
| SPARQL 1.2 | PASS |
| Serd Parser (good/bad/eof) | PASS (3/3) |

**Total**: 8/8 SPARQL suites + 3/3 parser suites passed.

## SHACL Validation (10/10 + 2 doc-tests)

| Test | Status |
|------|--------|
| Mode Off skips | PASS |
| No shapes error | PASS |
| Compile shapes | PASS |
| Conformant data passes | PASS |
| Non-conformant fails | PASS |
| Empty store passes | PASS |
| Warn mode validates | PASS |
| Cardinality constraints | PASS |
| Datatype constraints | PASS |
| Class constraints | PASS |

## Live Endpoint (Developer Sandbox)

Endpoint: `oxigraph-ldary-dev.apps.rm3.7wse.p1.openshiftapps.com`

| Test | Status |
|------|--------|
| GET /health | PASS |
| GET /ready | PASS |
| POST /store (insert) | PASS |
| POST /query (SELECT) | PASS |
| POST /update (SPARQL UPDATE) | PASS |
| Unauthorized write rejected | PASS (401) |
| Concurrent load (3W/5R) | PASS |

## Full Workspace: 678 tests passed, 0 failed

## Known Limitations

1. TiKV backend conformance gated behind `TIKV_PD_ENDPOINTS` env var
2. SHACL W3C coverage depends on rudof upstream
3. SHACL feature disabled in UBI 9 container build
