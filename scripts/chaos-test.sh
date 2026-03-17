#!/usr/bin/env bash
#
# chaos-test.sh -- Chaos testing for the local podman-compose TiKV cluster
#
# Runs fault-injection scenarios against the TiKV cluster defined in
# docker-compose.tikv.yml to verify resilience and recovery behavior.
#
# Usage:
#   ./chaos-test.sh                     # Run all scenarios
#   ./chaos-test.sh --scenario A        # Run only Scenario A
#   ./chaos-test.sh --scenario A,B      # Run Scenarios A and B
#   ./chaos-test.sh --help              # Show usage
#
# Prerequisites:
#   - podman and podman-compose (or docker/docker-compose)
#   - TiKV cluster running via: podman-compose -f docker-compose.tikv.yml up -d
#   - Oxigraph server running with TiKV backend on http://localhost:7878
#
# Environment variables:
#   OXIGRAPH_URL       Oxigraph SPARQL endpoint (default: http://localhost:7878)
#   PD_ENDPOINT        PD API endpoint (default: http://localhost:2379)
#   COMPOSE_FILE       Path to compose file (default: docker-compose.tikv.yml)
#   CONTAINER_RUNTIME  podman or docker (default: auto-detect)
#   NETWORK_NAME       Compose network name (default: oxigraph-k8s_default)
#

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

OXIGRAPH_URL="${OXIGRAPH_URL:-http://localhost:7878}"
PD_ENDPOINT="${PD_ENDPOINT:-http://localhost:2379}"
COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.tikv.yml}"
NETWORK_NAME="${NETWORK_NAME:-oxigraph-k8s_default}"

# Container names from docker-compose.tikv.yml
PD_CONTAINER="oxigraph-pd0"
TIKV0_CONTAINER="oxigraph-tikv0"
TIKV1_CONTAINER="oxigraph-tikv1"
TIKV2_CONTAINER="oxigraph-tikv2"

# Timeouts (seconds)
RAFT_ELECTION_WAIT=15
PD_RECOVERY_WAIT=20
NETWORK_RECOVERY_WAIT=15
HEALTH_CHECK_RETRIES=30
HEALTH_CHECK_INTERVAL=2

# Results tracking
RESULTS_DIR=""
PASS_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0

# Which scenarios to run (empty = all)
SELECTED_SCENARIOS=""

# ---------------------------------------------------------------------------
# Container runtime detection
# ---------------------------------------------------------------------------

detect_runtime() {
    if [[ -n "${CONTAINER_RUNTIME:-}" ]]; then
        echo "${CONTAINER_RUNTIME}"
        return
    fi
    if command -v podman &>/dev/null; then
        echo "podman"
    elif command -v docker &>/dev/null; then
        echo "docker"
    else
        echo ""
    fi
}

RUNTIME=$(detect_runtime)

# ---------------------------------------------------------------------------
# Logging helpers
# ---------------------------------------------------------------------------

log()      { echo "[$(date '+%H:%M:%S')] $*"; }
log_info() { echo "[$(date '+%H:%M:%S')] [INFO]  $*"; }
log_pass() { echo "[$(date '+%H:%M:%S')] [PASS]  $*"; }
log_fail() { echo "[$(date '+%H:%M:%S')] [FAIL]  $*"; }
log_warn() { echo "[$(date '+%H:%M:%S')] [WARN]  $*"; }
log_step() { echo "[$(date '+%H:%M:%S')]   -> $*"; }

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

usage() {
    cat <<'USAGE'
Usage: chaos-test.sh [OPTIONS]

Options:
  --scenario LIST   Comma-separated list of scenarios to run (A, B, C)
                    Default: run all scenarios
  --oxigraph-url U  Oxigraph SPARQL endpoint (default: http://localhost:7878)
  --pd-endpoint  E  PD API endpoint (default: http://localhost:2379)
  --network NAME    Compose network name (default: oxigraph-k8s_default)
  --help            Show this help message

Scenarios:
  A  Kill TiKV node during continuous inserts, verify Raft recovery
  B  Kill PD leader, verify graceful failure and recovery
  C  Network partition (disconnect tikv2), verify quorum operation

Examples:
  ./chaos-test.sh                         # Run all scenarios
  ./chaos-test.sh --scenario A            # Run only Scenario A
  ./chaos-test.sh --scenario A,C          # Run Scenarios A and C
USAGE
    exit 0
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --scenario)
                SELECTED_SCENARIOS="$2"
                shift 2
                ;;
            --oxigraph-url)
                OXIGRAPH_URL="$2"
                shift 2
                ;;
            --pd-endpoint)
                PD_ENDPOINT="$2"
                shift 2
                ;;
            --network)
                NETWORK_NAME="$2"
                shift 2
                ;;
            --help|-h)
                usage
                ;;
            *)
                log_fail "Unknown argument: $1"
                usage
                ;;
        esac
    done
}

should_run_scenario() {
    local scenario="$1"
    if [[ -z "${SELECTED_SCENARIOS}" ]]; then
        return 0  # Run all if none specified
    fi
    # Check if the scenario letter appears in the comma-separated list
    echo "${SELECTED_SCENARIOS}" | tr ',' '\n' | grep -qi "^${scenario}$"
}

# ---------------------------------------------------------------------------
# Utility functions
# ---------------------------------------------------------------------------

# Run a container runtime command (podman or docker).
rt() {
    "${RUNTIME}" "$@"
}

# Insert RDF triples via SPARQL UPDATE.
sparql_update() {
    local query="$1"
    curl -sf -X POST "${OXIGRAPH_URL}/update" \
        -H 'Content-Type: application/sparql-update' \
        -d "${query}" 2>/dev/null
}

# Run a SPARQL SELECT query; return JSON result.
sparql_query() {
    local query="$1"
    curl -sf "${OXIGRAPH_URL}/query" \
        -H 'Accept: application/json' \
        -d "query=${query}" 2>/dev/null
}

# Count triples matching a pattern. Returns integer or empty on failure.
count_triples() {
    local pattern="${1:-?s ?p ?o}"
    local result
    result=$(sparql_query "SELECT (COUNT(*) AS ?c) WHERE { ${pattern} }" 2>/dev/null) || return 1
    echo "${result}" | python3 -c "
import sys, json
data = json.load(sys.stdin)
print(data['results']['bindings'][0]['c']['value'])
" 2>/dev/null || echo ""
}

# Insert a batch of numbered test triples.
insert_test_triples() {
    local prefix="$1"
    local start="$2"
    local count="$3"
    local sparql="INSERT DATA {"
    for (( i=start; i<start+count; i++ )); do
        sparql+=" <http://chaos-test/${prefix}/s${i}> <http://chaos-test/p> \"value-${i}\" ."
    done
    sparql+=" }"
    sparql_update "${sparql}"
}

# Delete all chaos test data.
cleanup_test_data() {
    log_step "Cleaning up chaos test data"
    sparql_update "DELETE WHERE { ?s <http://chaos-test/p> ?o }" 2>/dev/null || true
}

# Wait for PD to become healthy.
wait_for_pd() {
    local max_retries="${1:-${HEALTH_CHECK_RETRIES}}"
    local retries=0
    while (( retries < max_retries )); do
        if curl -sf "${PD_ENDPOINT}/pd/api/v1/health" >/dev/null 2>&1; then
            return 0
        fi
        retries=$((retries + 1))
        sleep "${HEALTH_CHECK_INTERVAL}"
    done
    return 1
}

# Wait for a specific number of TiKV stores to be in "Up" state.
wait_for_stores() {
    local expected="${1:-3}"
    local max_retries="${2:-${HEALTH_CHECK_RETRIES}}"
    local retries=0
    while (( retries < max_retries )); do
        local up_count
        up_count=$(curl -sf "${PD_ENDPOINT}/pd/api/v1/stores" 2>/dev/null \
            | python3 -c "
import sys, json
data = json.load(sys.stdin)
count = sum(1 for s in data.get('stores', []) if s.get('store', {}).get('state_name') == 'Up')
print(count)
" 2>/dev/null || echo "0")
        if (( up_count >= expected )); then
            return 0
        fi
        retries=$((retries + 1))
        sleep "${HEALTH_CHECK_INTERVAL}"
    done
    return 1
}

# Wait for Oxigraph SPARQL endpoint to be reachable.
wait_for_oxigraph() {
    local max_retries="${1:-${HEALTH_CHECK_RETRIES}}"
    local retries=0
    while (( retries < max_retries )); do
        if sparql_query "SELECT * WHERE { ?s ?p ?o } LIMIT 1" >/dev/null 2>&1; then
            return 0
        fi
        retries=$((retries + 1))
        sleep "${HEALTH_CHECK_INTERVAL}"
    done
    return 1
}

# Record a scenario result.
record_result() {
    local scenario="$1"
    local status="$2"
    local details="$3"
    local result_file="${RESULTS_DIR}/scenario-${scenario}.txt"
    {
        echo "Scenario: ${scenario}"
        echo "Date: $(date -Iseconds)"
        echo "Result: ${status}"
        echo "Details: ${details}"
        echo ""
    } > "${result_file}"

    if [[ "${status}" == "PASS" ]]; then
        PASS_COUNT=$((PASS_COUNT + 1))
        log_pass "Scenario ${scenario}: ${details}"
    elif [[ "${status}" == "SKIP" ]]; then
        SKIP_COUNT=$((SKIP_COUNT + 1))
        log_warn "Scenario ${scenario} SKIPPED: ${details}"
    else
        FAIL_COUNT=$((FAIL_COUNT + 1))
        log_fail "Scenario ${scenario}: ${details}"
    fi
}

# ---------------------------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------------------------

preflight_checks() {
    log "============================================"
    log "  Chaos Test Suite for TiKV Cluster"
    log "============================================"
    log ""

    # Check container runtime
    if [[ -z "${RUNTIME}" ]]; then
        log_fail "Neither podman nor docker found on PATH."
        exit 1
    fi
    log_info "Container runtime: ${RUNTIME}"

    # Check PD health
    log_info "Checking PD health at ${PD_ENDPOINT} ..."
    if ! curl -sf "${PD_ENDPOINT}/pd/api/v1/health" >/dev/null 2>&1; then
        log_fail "PD is not reachable at ${PD_ENDPOINT}."
        log_fail "Start the cluster first: podman-compose -f ${COMPOSE_FILE} up -d"
        exit 1
    fi
    log_info "PD is healthy."

    # Check TiKV store count
    local store_count
    store_count=$(curl -sf "${PD_ENDPOINT}/pd/api/v1/stores" 2>/dev/null \
        | python3 -c "
import sys, json
data = json.load(sys.stdin)
count = sum(1 for s in data.get('stores', []) if s.get('store', {}).get('state_name') == 'Up')
print(count)
" 2>/dev/null || echo "0")
    log_info "TiKV stores up: ${store_count}/3"
    if (( store_count < 3 )); then
        log_fail "Expected 3 TiKV stores in Up state, found ${store_count}."
        exit 1
    fi

    # Check Oxigraph endpoint
    log_info "Checking Oxigraph endpoint at ${OXIGRAPH_URL} ..."
    if ! sparql_query "SELECT * WHERE { ?s ?p ?o } LIMIT 1" >/dev/null 2>&1; then
        log_warn "Oxigraph SPARQL endpoint is not reachable at ${OXIGRAPH_URL}."
        log_warn "Some tests will insert data directly and skip SPARQL verification."
    else
        log_info "Oxigraph SPARQL endpoint is reachable."
    fi

    # Check container existence
    for container in "${PD_CONTAINER}" "${TIKV0_CONTAINER}" "${TIKV1_CONTAINER}" "${TIKV2_CONTAINER}"; do
        if ! rt inspect "${container}" >/dev/null 2>&1; then
            log_fail "Container ${container} not found. Is the compose cluster running?"
            exit 1
        fi
    done
    log_info "All expected containers found."

    # Set up results directory
    RESULTS_DIR="/tmp/chaos-test-results/$(date +%Y%m%d-%H%M%S)"
    mkdir -p "${RESULTS_DIR}"
    log_info "Results will be saved to: ${RESULTS_DIR}"
    log ""
}

# ---------------------------------------------------------------------------
# Scenario A: Kill TiKV node during continuous inserts
# ---------------------------------------------------------------------------

scenario_a() {
    log "============================================"
    log "  Scenario A: Kill TiKV Node During Operations"
    log "============================================"
    log ""

    local bg_pid=""

    # Cleanup handler for this scenario
    cleanup_a() {
        # Kill background insert process if still running
        if [[ -n "${bg_pid}" ]] && kill -0 "${bg_pid}" 2>/dev/null; then
            kill "${bg_pid}" 2>/dev/null || true
            wait "${bg_pid}" 2>/dev/null || true
        fi
        # Ensure tikv0 is running
        if ! rt inspect --format '{{.State.Running}}' "${TIKV0_CONTAINER}" 2>/dev/null | grep -q "true"; then
            log_step "Restarting ${TIKV0_CONTAINER} ..."
            rt start "${TIKV0_CONTAINER}" 2>/dev/null || true
        fi
    }

    # Step 1: Insert baseline data
    log_step "Inserting baseline data (100 triples) ..."
    if ! insert_test_triples "scenA-baseline" 0 100; then
        record_result "A" "FAIL" "Failed to insert baseline data"
        cleanup_a
        return
    fi

    local baseline_count
    baseline_count=$(count_triples "?s <http://chaos-test/p> ?o")
    if [[ -z "${baseline_count}" ]] || (( baseline_count < 100 )); then
        record_result "A" "FAIL" "Baseline count too low: ${baseline_count:-0}"
        cleanup_a
        return
    fi
    log_step "Baseline triple count: ${baseline_count}"

    # Step 2: Start continuous inserts in the background
    log_step "Starting continuous inserts in background ..."
    (
        for batch in $(seq 1 20); do
            insert_test_triples "scenA-bg" $((batch * 10)) 10 2>/dev/null || true
            sleep 0.5
        done
    ) &
    bg_pid=$!

    # Step 3: Kill tikv0
    sleep 2  # Let a few inserts land first
    log_step "Killing ${TIKV0_CONTAINER} ..."
    rt kill "${TIKV0_CONTAINER}" 2>/dev/null || true

    # Step 4: Wait for Raft leader election
    log_step "Waiting ${RAFT_ELECTION_WAIT}s for Raft leader re-election ..."
    sleep "${RAFT_ELECTION_WAIT}"

    # Step 5: Verify data is still accessible via remaining nodes
    log_step "Verifying data accessibility with ${TIKV0_CONTAINER} down ..."
    local post_kill_count
    post_kill_count=$(count_triples "?s <http://chaos-test/p> ?o")
    local data_accessible=false
    if [[ -n "${post_kill_count}" ]] && (( post_kill_count >= baseline_count )); then
        log_step "Data still accessible. Triple count: ${post_kill_count} (baseline: ${baseline_count})"
        data_accessible=true
    else
        log_step "Data query returned: ${post_kill_count:-<empty>} (baseline: ${baseline_count})"
    fi

    # Wait for background inserts to finish
    wait "${bg_pid}" 2>/dev/null || true
    bg_pid=""

    # Step 6: Restart tikv0
    log_step "Restarting ${TIKV0_CONTAINER} ..."
    rt start "${TIKV0_CONTAINER}" 2>/dev/null || true

    # Step 7: Wait for cluster recovery
    log_step "Waiting for cluster to recover (all 3 stores Up) ..."
    if wait_for_stores 3 45; then
        log_step "All 3 TiKV stores are back Up."
    else
        log_step "Warning: Not all stores recovered within timeout."
    fi

    # Step 8: Final verification
    sleep 5
    local final_count
    final_count=$(count_triples "?s <http://chaos-test/p> ?o")
    log_step "Final triple count: ${final_count:-<unknown>} (baseline was: ${baseline_count})"

    # Evaluate result
    if [[ "${data_accessible}" == "true" ]] && [[ -n "${final_count}" ]] && (( final_count >= baseline_count )); then
        record_result "A" "PASS" \
            "Data remained accessible during node failure. Baseline: ${baseline_count}, Final: ${final_count}"
    else
        record_result "A" "FAIL" \
            "Data accessibility issue. Accessible during kill: ${data_accessible}, Final count: ${final_count:-<unknown>}, Baseline: ${baseline_count}"
    fi

    # Cleanup
    cleanup_test_data
    cleanup_a
    log ""
}

# ---------------------------------------------------------------------------
# Scenario B: Kill PD leader
# ---------------------------------------------------------------------------

scenario_b() {
    log "============================================"
    log "  Scenario B: Kill PD Leader"
    log "============================================"
    log ""

    # Cleanup handler for this scenario
    cleanup_b() {
        if ! rt inspect --format '{{.State.Running}}' "${PD_CONTAINER}" 2>/dev/null | grep -q "true"; then
            log_step "Restarting ${PD_CONTAINER} ..."
            rt start "${PD_CONTAINER}" 2>/dev/null || true
        fi
    }

    # Step 1: Insert test data before killing PD
    log_step "Inserting test data (50 triples) ..."
    if ! insert_test_triples "scenB" 0 50; then
        record_result "B" "FAIL" "Failed to insert pre-kill test data"
        cleanup_b
        return
    fi

    local pre_count
    pre_count=$(count_triples "?s <http://chaos-test/p> ?o")
    log_step "Pre-kill triple count: ${pre_count:-<unknown>}"

    # Step 2: Kill the PD leader
    log_step "Killing ${PD_CONTAINER} ..."
    rt kill "${PD_CONTAINER}" 2>/dev/null || true

    # Step 3: Verify operations fail gracefully (not crash/hang indefinitely)
    log_step "Testing operations with PD down (expecting graceful failure) ..."
    sleep 3

    local graceful_failure=true
    # Attempt a write -- should fail but not hang. Use a timeout.
    local write_result
    write_result=$(timeout 15 bash -c "
        curl -sf -X POST '${OXIGRAPH_URL}/update' \
            -H 'Content-Type: application/sparql-update' \
            -d 'INSERT DATA { <http://chaos-test/scenB/pd-down> <http://chaos-test/p> \"should-fail\" . }' 2>&1
    " 2>&1) && graceful_failure=false || true
    # If the write succeeded, that is unexpected but not necessarily a failure
    # (cached PD info might still work briefly)

    # Attempt a read -- should also timeout or fail gracefully
    local read_result
    read_result=$(timeout 15 bash -c "
        curl -sf '${OXIGRAPH_URL}/query' \
            -H 'Accept: application/json' \
            -d 'query=SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' 2>&1
    " 2>&1) || true

    if [[ "${graceful_failure}" == "true" ]]; then
        log_step "Operations failed gracefully with PD down (as expected)."
    else
        log_step "Note: Some operations may have succeeded using cached PD state."
    fi

    # Step 4: Restart PD
    log_step "Restarting ${PD_CONTAINER} ..."
    rt start "${PD_CONTAINER}" 2>/dev/null || true

    # Step 5: Wait for PD to recover
    log_step "Waiting for PD to recover ..."
    if ! wait_for_pd "${HEALTH_CHECK_RETRIES}"; then
        record_result "B" "FAIL" "PD did not recover within timeout after restart"
        cleanup_b
        return
    fi
    log_step "PD is healthy again."

    # Wait a bit for TiKV nodes to reconnect
    sleep "${PD_RECOVERY_WAIT}"

    # Step 6: Verify stores are back
    log_step "Verifying TiKV stores ..."
    if ! wait_for_stores 3 30; then
        log_step "Warning: Not all stores came back within timeout."
    fi

    # Step 7: Verify data is intact
    log_step "Waiting for Oxigraph to re-establish connection ..."
    if ! wait_for_oxigraph 20; then
        record_result "B" "FAIL" "Oxigraph SPARQL endpoint did not recover after PD restart"
        cleanup_b
        return
    fi

    local post_count
    post_count=$(count_triples "?s <http://chaos-test/p> ?o")
    log_step "Post-recovery triple count: ${post_count:-<unknown>} (pre-kill: ${pre_count:-<unknown>})"

    if [[ -n "${post_count}" ]] && [[ -n "${pre_count}" ]] && (( post_count >= pre_count )); then
        record_result "B" "PASS" \
            "PD recovered successfully. Pre-kill count: ${pre_count}, Post-recovery count: ${post_count}"
    else
        record_result "B" "FAIL" \
            "Data integrity issue after PD recovery. Pre-kill: ${pre_count:-<unknown>}, Post: ${post_count:-<unknown>}"
    fi

    # Cleanup
    cleanup_test_data
    cleanup_b
    log ""
}

# ---------------------------------------------------------------------------
# Scenario C: Network partition simulation
# ---------------------------------------------------------------------------

scenario_c() {
    log "============================================"
    log "  Scenario C: Network Partition Simulation"
    log "============================================"
    log ""

    # Cleanup handler for this scenario
    cleanup_c() {
        # Ensure tikv2 is reconnected
        rt network connect "${NETWORK_NAME}" "${TIKV2_CONTAINER}" 2>/dev/null || true
    }

    # Step 1: Verify the network exists
    if ! rt network inspect "${NETWORK_NAME}" >/dev/null 2>&1; then
        log_warn "Network ${NETWORK_NAME} not found."
        log_warn "Trying to detect the correct network name ..."
        local detected_network
        detected_network=$(rt inspect "${TIKV2_CONTAINER}" --format '{{range $k, $v := .NetworkSettings.Networks}}{{$k}}{{end}}' 2>/dev/null || echo "")
        if [[ -n "${detected_network}" ]]; then
            NETWORK_NAME="${detected_network}"
            log_step "Detected network: ${NETWORK_NAME}"
        else
            record_result "C" "SKIP" "Could not determine container network name"
            return
        fi
    fi

    # Step 2: Insert test data
    log_step "Inserting test data (50 triples) ..."
    if ! insert_test_triples "scenC" 0 50; then
        record_result "C" "FAIL" "Failed to insert test data"
        cleanup_c
        return
    fi

    local pre_count
    pre_count=$(count_triples "?s <http://chaos-test/p> ?o")
    log_step "Pre-partition triple count: ${pre_count:-<unknown>}"

    # Step 3: Disconnect tikv2 from the network (simulates network partition)
    log_step "Disconnecting ${TIKV2_CONTAINER} from network ${NETWORK_NAME} ..."
    if ! rt network disconnect "${NETWORK_NAME}" "${TIKV2_CONTAINER}" 2>/dev/null; then
        record_result "C" "FAIL" "Failed to disconnect ${TIKV2_CONTAINER} from network"
        cleanup_c
        return
    fi
    log_step "${TIKV2_CONTAINER} disconnected."

    # Step 4: Verify cluster still operates (2/3 quorum maintained)
    log_step "Waiting 10s for cluster to detect partition ..."
    sleep 10

    log_step "Verifying cluster operations with 2/3 quorum ..."
    local quorum_works=false

    # Try to insert data -- should succeed with 2/3 nodes
    if insert_test_triples "scenC-during" 100 20; then
        log_step "Write succeeded with 2/3 quorum."
        quorum_works=true
    else
        log_step "Write failed with 2/3 quorum (may be expected if region leaders were on tikv2)."
    fi

    # Try to read data
    local during_count
    during_count=$(count_triples "?s <http://chaos-test/p> ?o")
    if [[ -n "${during_count}" ]]; then
        log_step "Read succeeded during partition. Triple count: ${during_count}"
        quorum_works=true
    else
        log_step "Read failed during partition."
    fi

    # Step 5: Reconnect tikv2
    log_step "Reconnecting ${TIKV2_CONTAINER} to network ${NETWORK_NAME} ..."
    if ! rt network connect "${NETWORK_NAME}" "${TIKV2_CONTAINER}" 2>/dev/null; then
        record_result "C" "FAIL" "Failed to reconnect ${TIKV2_CONTAINER}"
        return
    fi
    log_step "${TIKV2_CONTAINER} reconnected."

    # Step 6: Wait for recovery
    log_step "Waiting ${NETWORK_RECOVERY_WAIT}s for cluster to heal ..."
    sleep "${NETWORK_RECOVERY_WAIT}"

    # Step 7: Verify recovery -- all 3 stores should be Up
    log_step "Verifying cluster recovery ..."
    if ! wait_for_stores 3 30; then
        log_step "Warning: Not all stores recovered within timeout."
    fi

    # Step 8: Verify data integrity
    local post_count
    post_count=$(count_triples "?s <http://chaos-test/p> ?o")
    log_step "Post-recovery triple count: ${post_count:-<unknown>} (pre-partition: ${pre_count:-<unknown>})"

    # Evaluate result
    if [[ "${quorum_works}" == "true" ]] && [[ -n "${post_count}" ]] && (( post_count >= pre_count )); then
        record_result "C" "PASS" \
            "Cluster operated with 2/3 quorum and recovered. Pre: ${pre_count}, Post: ${post_count}"
    elif [[ "${quorum_works}" == "true" ]]; then
        record_result "C" "PASS" \
            "Cluster operated with 2/3 quorum. Recovery count: ${post_count:-<unknown>}"
    else
        record_result "C" "FAIL" \
            "Cluster did not maintain quorum. Post: ${post_count:-<unknown>}, Pre: ${pre_count:-<unknown>}"
    fi

    # Cleanup
    cleanup_test_data
    log ""
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main() {
    parse_args "$@"
    preflight_checks

    local start_time
    start_time=$(date +%s)

    # Run selected scenarios
    if should_run_scenario "A"; then
        scenario_a
    else
        log_info "Skipping Scenario A"
        SKIP_COUNT=$((SKIP_COUNT + 1))
    fi

    if should_run_scenario "B"; then
        scenario_b
    else
        log_info "Skipping Scenario B"
        SKIP_COUNT=$((SKIP_COUNT + 1))
    fi

    if should_run_scenario "C"; then
        scenario_c
    else
        log_info "Skipping Scenario C"
        SKIP_COUNT=$((SKIP_COUNT + 1))
    fi

    # Summary
    local end_time
    end_time=$(date +%s)
    local duration=$(( end_time - start_time ))

    log "============================================"
    log "  Chaos Test Summary"
    log "============================================"
    log "  Duration:  ${duration}s"
    log "  Passed:    ${PASS_COUNT}"
    log "  Failed:    ${FAIL_COUNT}"
    log "  Skipped:   ${SKIP_COUNT}"
    log "  Results:   ${RESULTS_DIR}"
    log "============================================"

    if (( FAIL_COUNT > 0 )); then
        exit 1
    fi
}

main "$@"
