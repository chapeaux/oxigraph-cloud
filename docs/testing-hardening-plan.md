# Phase 8: Testing, Hardening & Release Plan

> **Tasks**: 8.1 (W3C SPARQL), 8.2 (W3C SHACL), 8.3 (Chaos Testing), 8.4 (Security Audit)
> **Also covers**: Task 2.7 (Performance Benchmarks)
> **Status**: Draft | **Date**: 2026-03-17

---

## Table of Contents

1. [W3C SPARQL 1.1 Compliance (Task 8.1)](#1-w3c-sparql-11-compliance-task-81)
2. [W3C SHACL Compliance (Task 8.2)](#2-w3c-shacl-compliance-task-82)
3. [Chaos Testing (Task 8.3)](#3-chaos-testing-task-83)
4. [Security Audit (Task 8.4)](#4-security-audit-task-84)
5. [Performance Benchmarks (Task 2.7)](#5-performance-benchmarks-task-27)

---

## 1. W3C SPARQL 1.1 Compliance (Task 8.1)

### 1.1 Background

Oxigraph ships a comprehensive W3C test runner in `oxigraph/testsuite/`. The test harness
(`oxigraph_testsuite::check_testsuite`) loads W3C manifest files and runs registered evaluators
for SPARQL syntax, query evaluation, update evaluation, federation, and result format tests.

The existing test file at `oxigraph/testsuite/tests/sparql.rs` exercises the following W3C
test suites against the default RocksDB-backed `Store`:

| Test Function | Manifest | Known Ignores |
|---------------|----------|---------------|
| `sparql10_w3c_query_syntax_testsuite` | `sparql10/manifest-syntax.ttl` | 1 (tokenizer) |
| `sparql10_w3c_query_evaluation_testsuite` | `sparql10/manifest-evaluation.ttl` | 11 (RDF 1.1 normalization, graph scoping) |
| `sparql11_query_w3c_evaluation_testsuite` | `sparql11/manifest-sparql11-query.ttl` | 7 (graph scoping, property paths) |
| `sparql11_federation_w3c_evaluation_testsuite` | `sparql11/manifest-sparql11-fed.ttl` | 1 (service eval order) |
| `sparql11_update_w3c_evaluation_testsuite` | `sparql11/manifest-sparql11-update.ttl` | 0 |
| `sparql11_json_w3c_evaluation_testsuite` | `sparql11/json-res/manifest.ttl` | 0 |
| `sparql11_tsv_w3c_evaluation_testsuite` | `sparql11/csv-tsv-res/manifest.ttl` | 3 (CSV format tests) |
| `sparql12_w3c_testsuite` | `sparql12/manifest.ttl` | 3 (triple terms, nested aggregates) |

**Acceptance criterion**: The TiKV backend must produce the exact same pass/fail matrix as
the RocksDB backend. Any new failure is a storage-abstraction regression.

### 1.2 Running Tests Against the TiKV Backend

#### Step 1: Ensure TiKV dev cluster is running

```bash
# Option A: tiup playground (3 TiKV + 1 PD)
tiup playground --mode tikv-slim --kv 3

# Option B: docker-compose
docker compose -f deploy/docker-compose-tikv.yaml up -d

# Verify cluster health
tikv-ctl --pd 127.0.0.1:2379 store
```

#### Step 2: Run the W3C SPARQL test suite with TiKV feature

The `Store` type is parameterized by `StorageBackend`. The test suite must be compiled with
the `tikv` feature flag to select `TikvBackend`:

```bash
# Run all SPARQL W3C tests against TiKV
cd /home/ldary/rh/oxigraph-k8s/oxigraph
TIKV_PD_ENDPOINTS="127.0.0.1:2379" \
  cargo test -p oxigraph-testsuite --features tikv -- sparql \
  2>&1 | tee w3c-sparql-tikv-results.log
```

#### Step 3: Run baseline comparison against RocksDB

```bash
# Run the same tests against RocksDB (default, no tikv feature)
cargo test -p oxigraph-testsuite -- sparql \
  2>&1 | tee w3c-sparql-rocksdb-results.log
```

#### Step 4: Diff the results

```bash
# Extract PASS/FAIL lines and compare
diff <(grep -E '(PASS|FAIL|ok|FAILED)' w3c-sparql-rocksdb-results.log | sort) \
     <(grep -E '(PASS|FAIL|ok|FAILED)' w3c-sparql-tikv-results.log | sort)
```

Any line present in the TiKV output but not in RocksDB output is a regression.

### 1.3 Identifying Storage-Abstraction Regressions

Regressions from the storage abstraction will manifest in specific categories:

| Symptom | Likely Cause | Investigation |
|---------|-------------|---------------|
| Transaction atomicity test fails | TiKV 2PC vs RocksDB WriteBatch semantics differ | Check `StorageBackend::transaction()` commit path |
| Range scan returns wrong order | Key encoding prefix mismatch | Verify key prefix scheme (task 2.2) for the failing index |
| Snapshot isolation test fails | TiKV MVCC timestamp vs RocksDB snapshot semantics | Check `StorageBackend::snapshot()` implementation |
| Timeout during evaluation test | Network latency to TiKV cluster | Increase test timeout; check connection pool config |
| Named graph test fails | GSPO/SPOG/POSG/OSPG key mapping error | Audit quad index key encoding |

### 1.4 CI Integration

```yaml
# .github/workflows/w3c-sparql.yaml
name: W3C SPARQL Compliance
on: [push, pull_request]

jobs:
  sparql-compliance:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        backend: [rocksdb, tikv]
    services:
      tikv:
        image: pingcap/tikv:latest
        # only started when backend == tikv
    steps:
      - uses: actions/checkout@v4
      - name: Run W3C SPARQL test suite
        env:
          TIKV_PD_ENDPOINTS: ${{ matrix.backend == 'tikv' && '127.0.0.1:2379' || '' }}
        run: |
          FEATURES=""
          if [ "${{ matrix.backend }}" = "tikv" ]; then
            FEATURES="--features tikv"
          fi
          cargo test -p oxigraph-testsuite $FEATURES -- sparql
      - name: Upload results
        uses: actions/upload-artifact@v4
        with:
          name: w3c-sparql-${{ matrix.backend }}
          path: w3c-sparql-*.log
```

---

## 2. W3C SHACL Compliance (Task 8.2)

### 2.1 Background

SHACL validation is provided by the `rudof` project's `shacl_validation` crate. The rudof
project includes its own W3C SHACL test suite runner in the `shacl_testsuite` crate, which
exercises conformance against the official W3C SHACL test suite manifests.

Our integration bridges rudof to Oxigraph via the SRDF trait implementation (task 3.1),
meaning that test failures can originate from three layers:

1. **Rudof bug**: The `shacl_validation` engine itself produces wrong results
2. **SRDF bridge bug**: Our `SRDF` trait impl for `Store<B>` returns incorrect neighborhoods
3. **Storage bug**: The underlying TiKV or RocksDB backend returns wrong data

### 2.2 Running the W3C SHACL Test Suite

#### Step 1: Run rudof's own SHACL test suite (baseline)

```bash
# Clone rudof and run its internal SHACL test suite
cd /tmp
git clone https://github.com/rudof-project/rudof.git
cd rudof
cargo test -p shacl_testsuite
```

This establishes the baseline of what rudof itself passes/fails, independent of our
integration.

#### Step 2: Run SHACL tests via our SRDF bridge (in-memory backend)

```bash
cd /home/ldary/rh/oxigraph-k8s/oxigraph
cargo test -p oxigraph-shacl --test shacl_w3c -- \
  2>&1 | tee w3c-shacl-inmemory-results.log
```

#### Step 3: Run SHACL tests via our SRDF bridge (TiKV backend)

```bash
TIKV_PD_ENDPOINTS="127.0.0.1:2379" \
  cargo test -p oxigraph-shacl --features tikv --test shacl_w3c -- \
  2>&1 | tee w3c-shacl-tikv-results.log
```

### 2.3 SHACL Test Harness Design

Create `oxigraph-shacl/tests/shacl_w3c.rs`:

```rust
//! W3C SHACL compliance tests run against Oxigraph's Store via the SRDF bridge.

use oxigraph::store::Store;
use oxigraph::io::RdfParser;
use shacl_ast::ShaclParser;
use shacl_validation::validate;

/// Load the W3C SHACL test suite manifest and run each test case.
///
/// For each test:
/// 1. Load the data graph into an Oxigraph Store
/// 2. Load the shapes graph
/// 3. Run validation via rudof's shacl_validation
/// 4. Compare the validation report to the expected result
#[test]
fn w3c_shacl_core_tests() {
    let manifest_path = "tests/w3c-shacl/core/manifest.ttl";
    let test_cases = parse_shacl_manifest(manifest_path);

    let mut failures = Vec::new();
    let mut passes = 0;

    for test in &test_cases {
        let store = Store::new().unwrap();
        // Load data graph
        store.load_from_reader(
            RdfParser::from_format(test.data_format),
            test.data_reader(),
        ).unwrap();

        // Load shapes and validate
        let shapes = ShaclParser::parse(&test.shapes_content).unwrap();
        let srdf_store = OxigraphSrdf::new(&store);
        let report = validate(&shapes, &srdf_store);

        // Compare report conformance to expected
        if report.conforms() != test.expected_conformance {
            failures.push(format!(
                "{}: expected conforms={}, got conforms={}",
                test.id, test.expected_conformance, report.conforms()
            ));
        } else {
            passes += 1;
        }
    }

    println!("{passes} passed, {} failed", failures.len());
    assert!(
        failures.is_empty(),
        "SHACL test failures:\n{}",
        failures.join("\n")
    );
}
```

### 2.4 Mapping Test Results: Rudof vs Integration Bugs

Use a three-way comparison to attribute failures:

```
For each failing test T:
  1. Does T fail in rudof's own shacl_testsuite?
     YES -> rudof bug. File issue upstream.
     NO  -> Continue to step 2.

  2. Does T fail with in-memory backend but pass with RocksDB?
     YES -> SRDF bridge bug in our implementation.
     NO  -> Continue to step 3.

  3. Does T fail only with TiKV backend?
     YES -> Storage-layer bug in TiKV backend (key encoding, range scan, etc.)
     NO  -> T fails with all backends -> likely an SRDF bridge bug.
```

### 2.5 Known Failure Categories

Document known failures in `docs/shacl-compliance-report.md`:

| Category | Example Constraints | Expected Status |
|----------|-------------------|-----------------|
| Core constraints (sh:minCount, sh:datatype, sh:class) | Must pass | Critical |
| SPARQL-based constraints (sh:sparql) | May fail if rudof doesn't support | Document as rudof limitation |
| Property pair constraints (sh:equals, sh:disjoint) | Should pass | High priority |
| Qualified value shapes (sh:qualifiedMinCount) | May have edge cases | Medium priority |
| Advanced: sh:rule, sh:entailment | Not required for SHACL Core | Low priority / out of scope |

### 2.6 CI Integration

```yaml
# .github/workflows/w3c-shacl.yaml
name: W3C SHACL Compliance
on: [push, pull_request]

jobs:
  shacl-compliance:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          submodules: true
      - name: Download W3C SHACL test suite
        run: |
          git clone https://github.com/w3c/data-shapes.git tests/w3c-shacl
      - name: Run SHACL compliance tests
        run: cargo test -p oxigraph-shacl --test shacl_w3c
      - name: Generate compliance report
        run: cargo test -p oxigraph-shacl --test shacl_w3c -- --report > shacl-report.txt
      - uses: actions/upload-artifact@v4
        with:
          name: shacl-compliance-report
          path: shacl-report.txt
```

---

## 3. Chaos Testing (Task 8.3)

### 3.1 Prerequisites

- Kubernetes cluster with TiKV deployed (3 TiKV nodes, 3 PD nodes, 2+ Oxigraph pods)
- `kubectl` access to the namespace
- Optional: [Chaos Mesh](https://chaos-mesh.org/) operator installed for advanced scenarios
- Test data: pre-loaded dataset of 100K triples for verification

### 3.2 Verification Baseline

Before running any chaos scenario, establish a verification baseline:

```bash
# Count all triples
curl -s 'http://oxigraph:7878/query' \
  -H 'Accept: application/json' \
  -d 'query=SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' \
  | jq '.results.bindings[0].c.value'

# Compute a checksum over sorted triples
curl -s 'http://oxigraph:7878/query' \
  -H 'Accept: text/plain' \
  -d 'query=SELECT ?s ?p ?o WHERE { ?s ?p ?o } ORDER BY ?s ?p ?o' \
  | sha256sum > baseline-checksum.txt
```

### 3.3 Scenario A: Kill 1 TiKV Node During Write Transaction

**Purpose**: Verify that an in-flight write transaction either commits fully or fails
cleanly when a TiKV node dies mid-transaction.

**Setup**:
```bash
# Identify TiKV pods
kubectl get pods -l app.kubernetes.io/component=tikv -n oxigraph

# Prepare a large write payload (10K triples)
cat > insert-payload.rq <<'EOF'
INSERT DATA {
  # 10K generated triples
  <http://example.org/s1> <http://example.org/p> "value1" .
  # ... (generated via script)
}
EOF
```

**Action**:
```bash
# In terminal 1: start the write
curl -X POST 'http://oxigraph:7878/update' \
  -H 'Content-Type: application/sparql-update' \
  -d @insert-payload.rq &
WRITE_PID=$!

# In terminal 2: kill a TiKV pod after 500ms
sleep 0.5
kubectl delete pod tikv-2 -n oxigraph --grace-period=0 --force

# Wait for write to complete (or fail)
wait $WRITE_PID
WRITE_EXIT=$?
```

**Verification**:
```bash
# 1. Check write result: should be either success (200) or explicit error (5xx)
#    NEVER partial data.
echo "Write exit code: $WRITE_EXIT"

# 2. Wait for TiKV pod to recover
kubectl wait --for=condition=ready pod -l app.kubernetes.io/component=tikv \
  -n oxigraph --timeout=120s

# 3. Verify data integrity
curl -s 'http://oxigraph:7878/query' \
  -H 'Accept: application/json' \
  -d 'query=SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' \
  | jq '.results.bindings[0].c.value'

# Count should be either:
#   - baseline + 10K (write succeeded before kill)
#   - baseline + 0   (write failed/rolled back)
# NEVER baseline + N where 0 < N < 10K (partial write = BUG)
```

**Expected behavior**: TiKV's Raft replication ensures that committed writes survive
a single node failure. If the write was in-flight during the kill, TiKV's 2PC will
either complete (if quorum was reached) or abort (client receives an error). Oxigraph
should surface the error as an HTTP 500 and the client retries.

---

### 3.4 Scenario B: Kill PD Leader During Operation

**Purpose**: Verify that PD leader election completes and the cluster recovers without
data loss or stale timestamps.

**Setup**:
```bash
# Identify the PD leader
kubectl exec -n oxigraph pd-0 -- pd-ctl member leader show
# Note: output shows current leader name and ID
```

**Action**:
```bash
# Start a continuous read/write workload
./scripts/sparql-load-generator.sh --qps 50 --duration 60s &
LOAD_PID=$!

# Kill the PD leader
PD_LEADER_POD=$(kubectl exec -n oxigraph pd-0 -- \
  pd-ctl member leader show | jq -r '.name')
kubectl delete pod "$PD_LEADER_POD" -n oxigraph --grace-period=0 --force

# Monitor for errors in the load generator
wait $LOAD_PID
```

**Verification**:
```bash
# 1. Verify new PD leader was elected
kubectl exec -n oxigraph pd-0 -- pd-ctl member leader show

# 2. Check for transaction errors during the failover window
grep -c 'ERROR\|TIMEOUT\|RETRY' load-generator.log

# 3. Verify data integrity (compare to baseline + expected inserts)
curl -s 'http://oxigraph:7878/query' \
  -H 'Accept: application/json' \
  -d 'query=SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }'
```

**Expected behavior**: PD leader election takes 1-3 seconds. During this window,
timestamp allocation stalls, so new transactions cannot start. In-flight transactions
that already have timestamps may complete. Oxigraph should retry TSO requests with
backoff. After new leader election, all operations resume normally.

---

### 3.5 Scenario C: Network Partition Between Oxigraph and TiKV

**Purpose**: Verify that Oxigraph handles network timeouts gracefully with proper
error messages and automatic recovery when connectivity is restored.

**Setup (using tc for traffic control)**:
```bash
# If using Chaos Mesh:
cat > network-partition.yaml <<'EOF'
apiVersion: chaos-mesh.org/v1alpha1
kind: NetworkChaos
metadata:
  name: oxigraph-tikv-partition
  namespace: oxigraph
spec:
  action: partition
  mode: all
  selector:
    namespaces: [oxigraph]
    labelSelectors:
      app: oxigraph-server
  direction: both
  target:
    mode: all
    selector:
      namespaces: [oxigraph]
      labelSelectors:
        app.kubernetes.io/component: tikv
  duration: "30s"
EOF

# If using tc directly (on Oxigraph pod):
kubectl exec -n oxigraph oxigraph-0 -- \
  tc qdisc add dev eth0 root netem loss 100%
```

**Action**:
```bash
# Apply network partition
kubectl apply -f network-partition.yaml

# Attempt queries during partition
for i in $(seq 1 10); do
  curl -s -o /dev/null -w "%{http_code} %{time_total}s\n" \
    'http://oxigraph:7878/query' \
    -d 'query=SELECT * WHERE { ?s ?p ?o } LIMIT 1'
  sleep 2
done | tee partition-responses.log

# Wait for partition to heal (Chaos Mesh auto-removes after duration)
sleep 35
```

**Verification**:
```bash
# 1. During partition: expect HTTP 503 or 504 (timeout) responses
grep -c '503\|504\|500' partition-responses.log
# Should be > 0 (partition causes errors)

# 2. After partition heals: expect HTTP 200
curl -s -o /dev/null -w "%{http_code}\n" \
  'http://oxigraph:7878/query' \
  -d 'query=SELECT * WHERE { ?s ?p ?o } LIMIT 1'
# Should be 200

# 3. Verify readiness probe recovers
kubectl get pods -n oxigraph -l app=oxigraph-server -o wide
# Pods should be Ready

# 4. Data integrity check
curl -s 'http://oxigraph:7878/query' \
  -H 'Accept: application/json' \
  -d 'query=SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }'
# Should match baseline
```

**Expected behavior**: Oxigraph's TiKV client has configurable timeouts (default: 20s
for RPC). During partition, all operations fail with timeout errors. The `/ready` endpoint
reports not-ready, so Kubernetes stops routing traffic. After partition heals, the gRPC
connection pool reconnects and operations resume. No data corruption occurs.

---

### 3.6 Scenario D: Kill TiKV Node During Bulk Load

**Purpose**: Verify that a bulk load operation can be safely retried after partial
failure, without duplicate data.

**Setup**:
```bash
# Prepare a 100K triple N-Triples file
./scripts/generate-lubm.sh --triples 100000 > bulk-load.nt
wc -l bulk-load.nt  # verify count
```

**Action**:
```bash
# Start bulk load via HTTP POST
curl -X POST 'http://oxigraph:7878/store' \
  -H 'Content-Type: application/n-triples' \
  --data-binary @bulk-load.nt &
LOAD_PID=$!

# Kill a TiKV node after 2 seconds
sleep 2
kubectl delete pod tikv-1 -n oxigraph --grace-period=0 --force

# Wait for load to complete/fail
wait $LOAD_PID
LOAD_EXIT=$?
echo "Bulk load exit code: $LOAD_EXIT"
```

**Verification**:
```bash
# Wait for cluster recovery
kubectl wait --for=condition=ready pod -l app.kubernetes.io/component=tikv \
  -n oxigraph --timeout=120s

# Check how many triples were loaded
LOADED=$(curl -s 'http://oxigraph:7878/query' \
  -H 'Accept: application/json' \
  -d 'query=SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' \
  | jq -r '.results.bindings[0].c.value')
echo "Triples loaded: $LOADED"

# If load failed, retry
if [ "$LOAD_EXIT" -ne 0 ]; then
  echo "Retrying bulk load..."
  curl -X POST 'http://oxigraph:7878/store' \
    -H 'Content-Type: application/n-triples' \
    --data-binary @bulk-load.nt
fi

# Final count should be exactly 100K (not 100K + partial from first attempt)
FINAL=$(curl -s 'http://oxigraph:7878/query' \
  -H 'Accept: application/json' \
  -d 'query=SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' \
  | jq -r '.results.bindings[0].c.value')
echo "Final triple count: $FINAL (expected: 100000)"
```

**Expected behavior**: Bulk load batches writes into transactions. If a TiKV node dies
mid-batch, uncommitted transactions are rolled back. The bulk load endpoint returns an
error. On retry, previously committed batches are idempotent (inserting the same triple
is a no-op in a set-based store). The final count should be exactly 100K.

---

### 3.7 Scenario E: Simultaneous Writes from Multiple Oxigraph Instances

**Purpose**: Verify that concurrent writes from multiple Oxigraph pods do not cause
data corruption, lost updates, or deadlocks.

**Setup**:
```bash
# Scale Oxigraph to 3 replicas
kubectl scale deployment oxigraph-server -n oxigraph --replicas=3
kubectl wait --for=condition=ready pod -l app=oxigraph-server \
  -n oxigraph --timeout=60s

# Get all Oxigraph pod IPs
PODS=$(kubectl get pods -n oxigraph -l app=oxigraph-server \
  -o jsonpath='{.items[*].status.podIP}')
```

**Action**:
```bash
# Each pod inserts a unique set of 1000 triples concurrently
POD_NUM=0
for POD_IP in $PODS; do
  POD_NUM=$((POD_NUM + 1))
  (
    for i in $(seq 1 1000); do
      curl -s -X POST "http://${POD_IP}:7878/update" \
        -H 'Content-Type: application/sparql-update' \
        -d "INSERT DATA { <http://example.org/pod${POD_NUM}/s${i}> <http://example.org/p> \"v${i}\" . }"
    done
  ) &
done
wait
```

**Verification**:
```bash
# Total triples should be exactly 3 * 1000 = 3000 (plus baseline)
TOTAL=$(curl -s 'http://oxigraph:7878/query' \
  -H 'Accept: application/json' \
  -d 'query=SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' \
  | jq -r '.results.bindings[0].c.value')
EXPECTED=$((BASELINE + 3000))
echo "Total: $TOTAL (expected: $EXPECTED)"

# Verify each pod's data is present
for POD_NUM in 1 2 3; do
  COUNT=$(curl -s 'http://oxigraph:7878/query' \
    -H 'Accept: application/json' \
    -d "query=SELECT (COUNT(*) AS ?c) WHERE { ?s <http://example.org/p> ?o . FILTER(STRSTARTS(STR(?s), \"http://example.org/pod${POD_NUM}/\")) }" \
    | jq -r '.results.bindings[0].c.value')
  echo "Pod $POD_NUM triples: $COUNT (expected: 1000)"
done

# Check for write conflicts in Oxigraph logs
kubectl logs -l app=oxigraph-server -n oxigraph | grep -c 'WriteConflict\|deadlock'
```

**Expected behavior**: TiKV's optimistic transactions may encounter write conflicts
when two transactions touch the same key. Since each pod writes to a unique key range
(different subject URIs), conflicts should be rare. If conflicts occur, the TiKV client
retries automatically. Final triple count must be exactly 3000. Write conflicts are
logged but not considered failures.

---

### 3.8 Chaos Testing Tools Summary

| Tool | Use Case | Installation |
|------|----------|-------------|
| `kubectl delete pod --force` | Kill individual pods | Built-in |
| `tc` (traffic control) | Network delay/loss/partition | `iproute2` in pod |
| [Chaos Mesh](https://chaos-mesh.org/) | Declarative fault injection | `helm install chaos-mesh` |
| Custom load generator | Concurrent read/write workload | `scripts/sparql-load-generator.sh` |

### 3.9 Chaos Test Results Template

For each scenario, record:

```
Scenario: [A/B/C/D/E]
Date: YYYY-MM-DD
Cluster: [config details]
Result: PASS / FAIL
Data integrity: VERIFIED / COMPROMISED
Recovery time: Xs
Notes: [any observations]
```

---

## 4. Security Audit (Task 8.4)

### 4.1 Dependency Vulnerability Scanning

#### cargo audit

```bash
# Install cargo-audit
cargo install cargo-audit

# Run against workspace
cd /home/ldary/rh/oxigraph-k8s/oxigraph
cargo audit

# Generate JSON report for CI
cargo audit --json > audit-report.json

# Check for critical/high severity
cargo audit --deny warnings
```

**CI integration**:
```yaml
- name: Cargo audit
  run: |
    cargo install cargo-audit
    cargo audit --deny warnings
```

**Frequency**: Run on every PR and weekly scheduled scan.

#### cargo-deny (license + advisory + ban check)

```bash
cargo install cargo-deny
cargo deny check advisories
cargo deny check licenses
cargo deny check bans
```

### 4.2 Container Image Scanning

#### Trivy

```bash
# Scan the built container image
trivy image --severity HIGH,CRITICAL \
  --exit-code 1 \
  quay.io/oxigraph-cloud/oxigraph-server:latest

# Generate SARIF report for GitHub Security tab
trivy image --format sarif \
  --output trivy-results.sarif \
  quay.io/oxigraph-cloud/oxigraph-server:latest
```

#### Grype

```bash
# Alternative scanner
grype quay.io/oxigraph-cloud/oxigraph-server:latest \
  --fail-on high
```

**CI integration**:
```yaml
- name: Trivy scan
  uses: aquasecurity/trivy-action@master
  with:
    image-ref: quay.io/oxigraph-cloud/oxigraph-server:latest
    format: sarif
    output: trivy-results.sarif
    severity: CRITICAL,HIGH
    exit-code: 1
- name: Upload Trivy results
  uses: github/codeql-action/upload-sarif@v3
  with:
    sarif_file: trivy-results.sarif
```

### 4.3 Container Hardening

#### Non-root user verification

The Containerfile must run the process as a non-root user:

```dockerfile
# In the Containerfile (runtime stage)
RUN groupadd -r oxigraph && useradd -r -g oxigraph -s /sbin/nologin oxigraph
USER oxigraph:oxigraph
```

Verify:
```bash
# Check that the container runs as non-root
podman run --rm quay.io/oxigraph-cloud/oxigraph-server:latest id
# Expected: uid=1000(oxigraph) gid=1000(oxigraph)

# Check no SUID/SGID binaries
podman run --rm quay.io/oxigraph-cloud/oxigraph-server:latest \
  find / -perm /6000 -type f 2>/dev/null
# Expected: empty output (or only harmless system binaries)
```

#### Read-only filesystem

```yaml
# In Kubernetes deployment spec
securityContext:
  readOnlyRootFilesystem: true
  runAsNonRoot: true
  runAsUser: 1000
  allowPrivilegeEscalation: false
  capabilities:
    drop: [ALL]
volumeMounts:
  - name: tmp
    mountPath: /tmp
volumes:
  - name: tmp
    emptyDir:
      sizeLimit: 64Mi
```

### 4.4 RBAC Review for OpenShift

Audit the following resources for least-privilege compliance:

```bash
# List all ClusterRoles and RoleBindings in the oxigraph namespace
oc get rolebindings -n oxigraph -o yaml
oc get clusterrolebindings | grep oxigraph

# Check service account permissions
oc auth can-i --list --as=system:serviceaccount:oxigraph:oxigraph-sa
```

**Required RBAC checklist**:

| Resource | Oxigraph SA | TiKV Operator SA | Notes |
|----------|-------------|-------------------|-------|
| Pods | get, list | get, list, create, delete | TiKV operator manages pods |
| Services | get, list | get, list, create | TiKV operator creates services |
| PVCs | - | get, list, create | Storage for TiKV data |
| Secrets | get (TLS certs only) | get, create | mTLS certificates |
| ConfigMaps | get | get, create, update | TiKV configuration |
| StatefulSets | - | get, create, update, delete | TiKV node management |
| ClusterRole | NONE | NONE | No cluster-wide access |

### 4.5 Network Policy

Restrict network communication to only necessary paths:

```yaml
# network-policy.yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: oxigraph-network-policy
  namespace: oxigraph
spec:
  podSelector:
    matchLabels:
      app: oxigraph-server
  policyTypes: [Ingress, Egress]
  ingress:
    # Allow SPARQL endpoint access from ingress controller only
    - from:
        - namespaceSelector:
            matchLabels:
              network.openshift.io/policy-group: ingress
      ports:
        - port: 7878
          protocol: TCP
  egress:
    # Allow connection to TiKV PD
    - to:
        - podSelector:
            matchLabels:
              app.kubernetes.io/component: pd
      ports:
        - port: 2379
          protocol: TCP
    # Allow connection to TiKV storage nodes
    - to:
        - podSelector:
            matchLabels:
              app.kubernetes.io/component: tikv
      ports:
        - port: 20160
          protocol: TCP
    # Allow DNS resolution
    - to:
        - namespaceSelector: {}
      ports:
        - port: 53
          protocol: UDP
        - port: 53
          protocol: TCP
---
# TiKV internal network policy
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: tikv-internal-policy
  namespace: oxigraph
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/component: tikv
  policyTypes: [Ingress]
  ingress:
    # Allow from Oxigraph pods
    - from:
        - podSelector:
            matchLabels:
              app: oxigraph-server
      ports:
        - port: 20160
          protocol: TCP
    # Allow from other TiKV pods (Raft replication)
    - from:
        - podSelector:
            matchLabels:
              app.kubernetes.io/component: tikv
      ports:
        - port: 20160
          protocol: TCP
    # Allow from PD
    - from:
        - podSelector:
            matchLabels:
              app.kubernetes.io/component: pd
      ports:
        - port: 20160
          protocol: TCP
```

### 4.6 TLS Configuration for gRPC (Oxigraph <-> TiKV)

```toml
# tikv.toml - TiKV server TLS config
[security]
ca-path = "/etc/tikv/tls/ca.crt"
cert-path = "/etc/tikv/tls/server.crt"
key-path = "/etc/tikv/tls/server.key"
```

Oxigraph TiKV client TLS configuration:

```rust
// In oxigraph-tikv connection setup
use tikv_client::{Config, TransactionClient};

let config = Config::default()
    .with_security(
        "/etc/oxigraph/tls/ca.crt",
        "/etc/oxigraph/tls/client.crt",
        "/etc/oxigraph/tls/client.key",
    );

let client = TransactionClient::new_with_config(
    vec!["pd-0:2379", "pd-1:2379", "pd-2:2379"],
    config,
).await?;
```

Certificate management via cert-manager:

```yaml
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: tikv-server-cert
  namespace: oxigraph
spec:
  secretName: tikv-server-tls
  issuerRef:
    name: oxigraph-ca-issuer
    kind: Issuer
  commonName: tikv
  dnsNames:
    - "*.tikv-peer.oxigraph.svc.cluster.local"
    - "*.pd.oxigraph.svc.cluster.local"
  usages:
    - server auth
    - client auth
```

### 4.7 SPARQL Injection Prevention

Oxigraph parses SPARQL queries via `spargebra::SparqlParser`, which produces a typed AST.
This inherently prevents injection attacks similar to SQL injection, because:

1. The query string is fully parsed before evaluation -- no string concatenation with data
2. SPARQL UPDATE operations go through the same parser
3. Parameter binding (if supported) uses typed values, not string interpolation

**Verification**:
```bash
# Attempt SPARQL injection via query parameter
curl -s 'http://oxigraph:7878/query' \
  -d 'query=SELECT * WHERE { ?s ?p ?o } ; DROP ALL'
# Expected: parse error (SPARQL does not support semicolons between operations)

# Attempt via Content-Type confusion
curl -s -X POST 'http://oxigraph:7878/update' \
  -H 'Content-Type: application/sparql-query' \
  -d 'DROP ALL'
# Expected: rejected (wrong Content-Type for update operations)
```

**Recommendations**:
- Ensure the SPARQL endpoint validates `Content-Type` headers strictly
- In `enforce` SHACL mode, all writes are validated before commit
- Log all SPARQL UPDATE operations for audit trails
- Consider read-only mode for public-facing deployments (`--read-only` flag)

### 4.8 Rate Limiting on the SPARQL Endpoint

Implement at the Kubernetes ingress/route level:

```yaml
# OpenShift Route with rate limiting via HAProxy annotations
apiVersion: route.openshift.io/v1
kind: Route
metadata:
  name: oxigraph-sparql
  namespace: oxigraph
  annotations:
    haproxy.router.openshift.io/rate-limit-connections: "true"
    haproxy.router.openshift.io/rate-limit-connections.concurrent-tcp: "20"
    haproxy.router.openshift.io/rate-limit-connections.rate-http: "100"
    haproxy.router.openshift.io/rate-limit-connections.rate-tcp: "50"
    haproxy.router.openshift.io/timeout: "60s"
spec:
  to:
    kind: Service
    name: oxigraph-server
  port:
    targetPort: 7878
  tls:
    termination: edge
```

For more granular control, consider an application-level rate limiter:

```rust
// In oxigraph-server, using tower middleware
use tower::limit::RateLimitLayer;
use std::time::Duration;

let rate_limit = RateLimitLayer::new(100, Duration::from_secs(1)); // 100 req/s
```

### 4.9 Security Audit Checklist

| Item | Tool/Method | Pass Criteria | Status |
|------|-------------|--------------|--------|
| No critical/high CVEs in Rust deps | `cargo audit` | Exit code 0 | [ ] |
| No critical/high CVEs in container | Trivy / Grype | Exit code 0 | [ ] |
| Container runs as non-root | `podman run ... id` | uid != 0 | [ ] |
| Read-only root filesystem | K8s securityContext | No write errors in normal operation | [ ] |
| No unnecessary capabilities | `capabilities.drop: [ALL]` | Verified in pod spec | [ ] |
| RBAC least-privilege | `oc auth can-i --list` | No cluster-admin bindings | [ ] |
| Network policies applied | `kubectl get networkpolicy` | Policies exist and tested | [ ] |
| mTLS between Oxigraph and TiKV | TLS config verification | `openssl s_client` succeeds | [ ] |
| SPARQL injection tested | Manual + automated tests | All injection attempts rejected | [ ] |
| Rate limiting configured | Route annotations / middleware | Burst traffic throttled | [ ] |
| Secrets not in container image | `trivy image --secret` | No embedded secrets | [ ] |
| Container image signed | cosign / sigstore | Signature verifiable | [ ] |

---

## 5. Performance Benchmarks (Task 2.7)

### 5.1 Benchmark Suite Design

All benchmarks use the [Criterion](https://docs.rs/criterion) framework for statistical
rigor (warm-up, multiple iterations, outlier detection).

Create `oxigraph-tikv/benches/storage_bench.rs`:

```rust
use codspeed_criterion_compat::{
    criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use oxigraph::store::Store;
use oxigraph::io::RdfParser;
use oxigraph::model::*;
use std::time::Duration;

/// Benchmark 1: Single-triple insert latency
fn bench_single_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_insert");
    group.measurement_time(Duration::from_secs(10));

    // Test with both backends
    for backend_name in &["rocksdb", "tikv"] {
        let store = create_store(backend_name);
        let mut counter = 0u64;

        group.bench_function(*backend_name, |b| {
            b.iter(|| {
                counter += 1;
                let quad = Quad::new(
                    NamedNode::new(format!("http://example.org/s{counter}")).unwrap(),
                    NamedNode::new("http://example.org/p").unwrap(),
                    Literal::new_simple_literal(format!("value{counter}")),
                    GraphName::DefaultGraph,
                );
                store.insert(&quad).unwrap();
            });
        });
    }
    group.finish();
}

/// Benchmark 2: Bulk load throughput
fn bench_bulk_load(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulk_load");

    for size in &[1_000, 10_000, 100_000] {
        let data = generate_ntriples(*size);
        group.throughput(Throughput::Elements(*size as u64));

        for backend_name in &["rocksdb", "tikv"] {
            group.bench_with_input(
                BenchmarkId::new(*backend_name, size),
                &data,
                |b, data| {
                    b.iter(|| {
                        let store = create_store(backend_name);
                        store.bulk_loader().load_from_reader(
                            RdfParser::from_format(RdfFormat::NTriples),
                            data.as_slice(),
                        ).unwrap();
                    });
                },
            );
        }
    }
    group.finish();
}

/// Benchmark 3: Point query latency (single triple lookup)
fn bench_point_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("point_query");

    for backend_name in &["rocksdb", "tikv"] {
        let store = create_store(backend_name);
        load_test_data(&store, 100_000);
        let subject = NamedNode::new("http://example.org/s50000").unwrap();

        group.bench_function(*backend_name, |b| {
            b.iter(|| {
                let results: Vec<_> = store
                    .quads_for_pattern(
                        Some(subject.as_ref().into()),
                        None, None, None,
                    )
                    .collect();
                assert!(!results.is_empty());
            });
        });
    }
    group.finish();
}

/// Benchmark 4: Range scan throughput
fn bench_range_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("range_scan");

    for result_count in &[1_000, 10_000] {
        for backend_name in &["rocksdb", "tikv"] {
            let store = create_store(backend_name);
            load_test_data(&store, 100_000);
            let predicate = NamedNode::new("http://example.org/p").unwrap();

            group.throughput(Throughput::Elements(*result_count as u64));
            group.bench_with_input(
                BenchmarkId::new(*backend_name, result_count),
                result_count,
                |b, _| {
                    b.iter(|| {
                        let results: Vec<_> = store
                            .quads_for_pattern(
                                None,
                                Some(predicate.as_ref().into()),
                                None, None,
                            )
                            .take(*result_count)
                            .collect();
                        assert_eq!(results.len(), *result_count);
                    });
                },
            );
        }
    }
    group.finish();
}

/// Benchmark 5: Complex SPARQL query (multi-join BGP)
fn bench_complex_sparql(c: &mut Criterion) {
    let mut group = c.benchmark_group("complex_sparql");

    let query = r#"
        SELECT ?name ?dept ?manager WHERE {
            ?person <http://example.org/name> ?name .
            ?person <http://example.org/department> ?dept .
            ?dept <http://example.org/managedBy> ?manager .
        }
    "#;

    for backend_name in &["rocksdb", "tikv"] {
        let store = create_store(backend_name);
        load_lubm_data(&store);

        group.bench_function(*backend_name, |b| {
            b.iter(|| {
                let results = store.query(query).unwrap();
                // consume results
                if let QueryResults::Solutions(solutions) = results {
                    let _: Vec<_> = solutions.collect();
                }
            });
        });
    }
    group.finish();
}

/// Benchmark 6: SHACL validation overhead on ingestion
fn bench_shacl_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("shacl_overhead");

    let shapes = load_shacl_shapes("tests/fixtures/simple-shapes.ttl");

    for mode in &["off", "simple_shapes", "complex_shapes"] {
        group.bench_function(*mode, |b| {
            b.iter(|| {
                let store = create_store("rocksdb");
                if *mode != "off" {
                    store.set_shacl_shapes(&shapes);
                    store.set_shacl_mode(ShaclMode::Enforce);
                }
                load_test_data(&store, 10_000);
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_single_insert,
    bench_bulk_load,
    bench_point_query,
    bench_range_scan,
    bench_complex_sparql,
    bench_shacl_validation,
);
criterion_main!(benches);
```

### 5.2 Running Benchmarks

```bash
# Run all storage benchmarks (RocksDB only - no TiKV needed)
cargo bench -p oxigraph-tikv --bench storage_bench

# Run with TiKV backend
TIKV_PD_ENDPOINTS="127.0.0.1:2379" \
  cargo bench -p oxigraph-tikv --bench storage_bench --features tikv

# Run specific benchmark group
cargo bench -p oxigraph-tikv --bench storage_bench -- "bulk_load"

# Generate HTML report
cargo bench -p oxigraph-tikv --bench storage_bench -- --output-format html
# Results in target/criterion/report/index.html
```

### 5.3 LUBM Dataset for Realistic Benchmarks

The [Lehigh University Benchmark (LUBM)](http://swat.cse.lehigh.edu/projects/lubm/) is
the standard benchmark for RDF stores.

```bash
# Generate LUBM datasets of varying sizes
# Using the UBA data generator
java -jar uba.jar -univ 1 -onto http://swat.cse.lehigh.edu/onto/univ-bench.owl
# Produces ~100K triples for 1 university

# Or use a pre-generated dataset
curl -L -o lubm-1.nt.gz \
  https://github.com/rvesse/lubm-uba/releases/download/v1.0/lubm-1-university.nt.gz
gunzip lubm-1.nt.gz
```

### 5.4 Expected Results: RocksDB vs TiKV Comparison Matrix

| Benchmark | RocksDB (target) | TiKV (acceptable) | TiKV (concerning) |
|-----------|------------------|-------------------|-------------------|
| Single insert latency | < 0.1 ms | < 5 ms (50x) | > 10 ms |
| Bulk load 100K triples | < 2s | < 15s (7.5x) | > 30s |
| Point query latency | < 0.05 ms | < 2 ms (40x) | > 5 ms |
| Range scan 10K results | < 50 ms | < 250 ms (5x) | > 500 ms |
| Complex SPARQL (multi-join) | < 100 ms | < 500 ms (5x) | > 2s |
| SHACL validation overhead | < 20% | < 30% | > 50% |

**Rationale for acceptable thresholds**: TiKV adds network round-trip (typically 0.1-2 ms),
Raft consensus (1 RTT for reads, 2 RTTs for writes), and serialization overhead. For
single-key operations, the overhead is dominated by network latency. For bulk operations,
batching amortizes the per-key overhead, so the ratio should be lower.

### 5.5 Performance Regression CI

```yaml
# .github/workflows/benchmarks.yaml
name: Performance Benchmarks
on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Run benchmarks
        run: cargo bench -p oxigraph-tikv --bench storage_bench -- --output-format bencher | tee bench-output.txt
      - name: Store benchmark result
        uses: benchmark-action/github-action-benchmark@v1
        with:
          tool: cargo
          output-file-path: bench-output.txt
          alert-threshold: "150%"
          comment-on-alert: true
          fail-on-alert: true
          github-token: ${{ secrets.GITHUB_TOKEN }}
```

### 5.6 1M Triple Bulk Load Test (Extended)

For the 1M triple test, use a dedicated benchmark script rather than Criterion (which
would take too long for iterative measurement):

```bash
#!/bin/bash
# scripts/bench-bulk-load-1M.sh
set -euo pipefail

TRIPLES=1000000
DATA_FILE="/tmp/lubm-1M.nt"

echo "Generating $TRIPLES triples..."
./scripts/generate-lubm.sh --triples $TRIPLES > "$DATA_FILE"
SIZE=$(du -h "$DATA_FILE" | cut -f1)
echo "Data file: $SIZE"

for BACKEND in rocksdb tikv; do
    echo "=== Benchmarking $BACKEND ==="
    if [ "$BACKEND" = "tikv" ]; then
        export TIKV_PD_ENDPOINTS="127.0.0.1:2379"
    fi

    # Start a fresh store
    STORE_DIR=$(mktemp -d)
    START=$(date +%s%N)

    oxigraph serve --location "$STORE_DIR" --backend "$BACKEND" &
    SERVER_PID=$!
    sleep 2

    # Bulk load
    curl -s -X POST 'http://localhost:7878/store' \
        -H 'Content-Type: application/n-triples' \
        --data-binary "@$DATA_FILE"

    END=$(date +%s%N)
    ELAPSED=$(( (END - START) / 1000000 ))

    # Verify count
    COUNT=$(curl -s 'http://localhost:7878/query' \
        -H 'Accept: application/json' \
        -d 'query=SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' \
        | jq -r '.results.bindings[0].c.value')

    echo "$BACKEND: Loaded $COUNT triples in ${ELAPSED}ms ($(( TRIPLES * 1000 / ELAPSED )) triples/sec)"

    kill $SERVER_PID
    rm -rf "$STORE_DIR"
done
```

---

## Appendix A: Test Execution Summary Script

```bash
#!/bin/bash
# scripts/run-all-phase8-tests.sh
set -euo pipefail

echo "=========================================="
echo "Phase 8: Testing & Hardening - Full Run"
echo "=========================================="

echo ""
echo "[1/5] W3C SPARQL 1.1 Compliance (RocksDB)"
cargo test -p oxigraph-testsuite -- sparql 2>&1 | tail -5

echo ""
echo "[2/5] W3C SPARQL 1.1 Compliance (TiKV)"
TIKV_PD_ENDPOINTS="127.0.0.1:2379" \
  cargo test -p oxigraph-testsuite --features tikv -- sparql 2>&1 | tail -5

echo ""
echo "[3/5] W3C SHACL Compliance"
cargo test -p oxigraph-shacl --test shacl_w3c 2>&1 | tail -5

echo ""
echo "[4/5] Security Audit"
cargo audit --deny warnings
trivy image --severity HIGH,CRITICAL --exit-code 1 \
  quay.io/oxigraph-cloud/oxigraph-server:latest

echo ""
echo "[5/5] Performance Benchmarks"
cargo bench -p oxigraph-tikv --bench storage_bench -- --output-format bencher | tail -20

echo ""
echo "=========================================="
echo "Phase 8 complete. Review results above."
echo "=========================================="
```

## Appendix B: Chaos Test Automation Script

```bash
#!/bin/bash
# scripts/run-chaos-tests.sh
set -euo pipefail

NAMESPACE="oxigraph"
RESULTS_DIR="chaos-results/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$RESULTS_DIR"

# Capture baseline
echo "Capturing baseline..."
BASELINE=$(curl -s 'http://oxigraph:7878/query' \
  -H 'Accept: application/json' \
  -d 'query=SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' \
  | jq -r '.results.bindings[0].c.value')
echo "Baseline triple count: $BASELINE" | tee "$RESULTS_DIR/baseline.txt"

run_scenario() {
    local NAME=$1
    local DESCRIPTION=$2
    echo ""
    echo "=== Scenario $NAME: $DESCRIPTION ==="
    echo "Start: $(date -Iseconds)"
}

verify_integrity() {
    local EXPECTED=$1
    local ACTUAL=$(curl -s 'http://oxigraph:7878/query' \
      -H 'Accept: application/json' \
      -d 'query=SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' \
      | jq -r '.results.bindings[0].c.value')

    if [ "$ACTUAL" = "$EXPECTED" ]; then
        echo "INTEGRITY: PASS (count=$ACTUAL, expected=$EXPECTED)"
        return 0
    else
        echo "INTEGRITY: FAIL (count=$ACTUAL, expected=$EXPECTED)"
        return 1
    fi
}

# Scenario A: Kill TiKV node during write
run_scenario "A" "Kill TiKV node during write"
# ... (implementation as described in section 3.3)

# Scenario B: Kill PD leader
run_scenario "B" "Kill PD leader"
# ... (implementation as described in section 3.4)

# Scenario C: Network partition
run_scenario "C" "Network partition"
# ... (implementation as described in section 3.5)

# Scenario D: Kill during bulk load
run_scenario "D" "Kill during bulk load"
# ... (implementation as described in section 3.6)

# Scenario E: Concurrent writes
run_scenario "E" "Concurrent writes from multiple instances"
# ... (implementation as described in section 3.7)

echo ""
echo "=== All chaos scenarios complete ==="
echo "Results saved to: $RESULTS_DIR"
```
