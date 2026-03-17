#!/usr/bin/env bash
#
# w3c-compliance.sh -- Run W3C SPARQL 1.1 compliance tests against RocksDB
#                      and TiKV backends, then compare results.
#
# Usage:
#   ./scripts/w3c-compliance.sh                  # Run both backends (TiKV skipped if unavailable)
#   ./scripts/w3c-compliance.sh --rocksdb-only   # Run RocksDB baseline only
#   ./scripts/w3c-compliance.sh --tikv-only      # Run TiKV only (requires TIKV_PD_ENDPOINTS)
#   ./scripts/w3c-compliance.sh --help           # Show this help
#
# Environment variables:
#   TIKV_PD_ENDPOINTS   Comma-separated PD endpoints (default: 127.0.0.1:2379)
#                       Required for TiKV tests. Set to empty string to skip TiKV.
#   OXIGRAPH_DIR        Path to oxigraph source root (default: auto-detected)
#
# Network note:
#   When running TiKV tests against a containerized TiKV cluster (e.g., via Podman),
#   the test process must be able to reach the PD and TiKV endpoints. Options:
#     1. Run inside the same Podman network:
#        podman run --network=tikv-net ... cargo test ...
#     2. Use host-mapped ports with --publish on the TiKV containers
#     3. Add /etc/hosts entries if TiKV nodes advertise container hostnames
#
# Output:
#   Results are written to /tmp/w3c-compliance-{rocksdb,tikv}.log
#   A summary comparison table is printed to stdout.
#

set -euo pipefail

# --- Configuration -----------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OXIGRAPH_DIR="${OXIGRAPH_DIR:-$(cd "${SCRIPT_DIR}/../oxigraph" && pwd)}"
TIKV_PD_ENDPOINTS="${TIKV_PD_ENDPOINTS:-127.0.0.1:2379}"

ROCKSDB_LOG="/tmp/w3c-compliance-rocksdb.log"
TIKV_LOG="/tmp/w3c-compliance-tikv.log"

RUN_ROCKSDB=true
RUN_TIKV=true

# --- Helpers ------------------------------------------------------------------

log() {
    echo "[$(date '+%H:%M:%S')] $*"
}

usage() {
    sed -n '2,/^$/{ s/^# \?//; p }' "$0"
    exit 0
}

# Parse a cargo test log and extract per-test-function pass/fail counts.
# Cargo test output lines look like:
#   test sparql10_w3c_query_syntax_testsuite ... ok
#   test sparql10_w3c_query_syntax_testsuite ... FAILED
#
# For our testsuite, each #[test] fn either passes (all sub-tests ok) or
# panics with a count of failing sub-tests. We parse both the cargo-level
# result and the panic message for detail.
parse_results() {
    local logfile="$1"
    local label="$2"

    if [[ ! -f "$logfile" ]]; then
        echo "  (no results -- $label was not run)"
        return
    fi

    # Extract individual test results from cargo output
    # Format: "test <name> ... ok" or "test <name> ... FAILED"
    grep -E '^test .+ \.\.\. (ok|FAILED)' "$logfile" 2>/dev/null || true
}

# Count passed/failed from a cargo test log
count_results() {
    local logfile="$1"
    if [[ ! -f "$logfile" ]]; then
        echo "0 0"
        return
    fi
    local passed failed
    passed=$(grep -cE '^test .+ \.\.\. ok$' "$logfile" 2>/dev/null || echo 0)
    failed=$(grep -cE '^test .+ \.\.\. FAILED$' "$logfile" 2>/dev/null || echo 0)
    echo "$passed $failed"
}

# Extract the test function names from cargo output
extract_test_names() {
    local logfile="$1"
    if [[ ! -f "$logfile" ]]; then
        return
    fi
    grep -oE '^test [^ ]+ ' "$logfile" 2>/dev/null | sed 's/^test //; s/ $//' | sort
}

# --- Argument parsing ---------------------------------------------------------

while [[ $# -gt 0 ]]; do
    case "$1" in
        --rocksdb-only)
            RUN_TIKV=false
            shift
            ;;
        --tikv-only)
            RUN_ROCKSDB=false
            shift
            ;;
        --help|-h)
            usage
            ;;
        *)
            echo "Unknown argument: $1" >&2
            usage
            ;;
    esac
done

# --- Pre-flight checks -------------------------------------------------------

if [[ ! -d "$OXIGRAPH_DIR" ]]; then
    echo "ERROR: Oxigraph directory not found at $OXIGRAPH_DIR" >&2
    echo "Set OXIGRAPH_DIR to the oxigraph source root." >&2
    exit 1
fi

if [[ ! -f "$OXIGRAPH_DIR/testsuite/Cargo.toml" ]]; then
    echo "ERROR: testsuite/Cargo.toml not found in $OXIGRAPH_DIR" >&2
    exit 1
fi

# Check TiKV availability
tikv_available() {
    if [[ -z "$TIKV_PD_ENDPOINTS" ]]; then
        return 1
    fi
    local host port
    host="${TIKV_PD_ENDPOINTS%%:*}"
    port="${TIKV_PD_ENDPOINTS##*:}"
    # Quick TCP check
    if command -v nc &>/dev/null; then
        nc -z -w 2 "$host" "$port" 2>/dev/null
    elif command -v bash &>/dev/null; then
        (echo > "/dev/tcp/$host/$port") 2>/dev/null
    else
        # Assume available if we cannot check
        return 0
    fi
}

# --- W3C test suite names (for the summary table) ----------------------------

# These correspond to the #[test] functions in testsuite/tests/sparql.rs
TEST_SUITES=(
    "sparql10_w3c_query_syntax_testsuite"
    "sparql10_w3c_query_evaluation_testsuite"
    "sparql11_query_w3c_evaluation_testsuite"
    "sparql11_federation_w3c_evaluation_testsuite"
    "sparql11_update_w3c_evaluation_testsuite"
    "sparql11_json_w3c_evaluation_testsuite"
    "sparql11_tsv_w3c_evaluation_testsuite"
    "sparql12_w3c_testsuite"
)

# --- Run RocksDB baseline ----------------------------------------------------

if $RUN_ROCKSDB; then
    log "Running W3C SPARQL test suite against RocksDB backend..."
    log "Output: $ROCKSDB_LOG"

    cd "$OXIGRAPH_DIR"
    if cargo test -p oxigraph-testsuite --test sparql -- --test-threads=1 \
        2>&1 | tee "$ROCKSDB_LOG"; then
        log "RocksDB: all tests passed."
    else
        log "RocksDB: some tests failed (see $ROCKSDB_LOG for details)."
    fi
    log ""
fi

# --- Run TiKV backend --------------------------------------------------------

if $RUN_TIKV; then
    if tikv_available; then
        log "Running W3C SPARQL test suite against TiKV backend..."
        log "  PD endpoints: $TIKV_PD_ENDPOINTS"
        log "Output: $TIKV_LOG"
        log ""
        log "NOTE: The TiKV backend test runner requires the 'tikv' feature on the"
        log "oxigraph crate and that the test binary can reach all TiKV nodes."
        log "If running against a Podman/Docker TiKV cluster, ensure either:"
        log "  - Tests run inside the same container network, or"
        log "  - TiKV ports are published to the host, or"
        log "  - /etc/hosts maps TiKV container names to reachable IPs."
        log ""

        cd "$OXIGRAPH_DIR"
        if TIKV_PD_ENDPOINTS="$TIKV_PD_ENDPOINTS" \
            cargo test -p oxigraph-testsuite --features tikv --test sparql -- --test-threads=1 \
            2>&1 | tee "$TIKV_LOG"; then
            log "TiKV: all tests passed."
        else
            log "TiKV: some tests failed (see $TIKV_LOG for details)."
        fi
    else
        log "SKIP: TiKV is not available at $TIKV_PD_ENDPOINTS"
        log "  To run TiKV tests, start a TiKV cluster and set TIKV_PD_ENDPOINTS."
        log "  Example: ./scripts/tikv-dev-cluster.sh start"
        log ""
        # Create a marker file so the comparison knows TiKV was skipped
        echo "SKIPPED: TiKV not available at $TIKV_PD_ENDPOINTS" > "$TIKV_LOG"
    fi
    log ""
fi

# --- Comparison and summary table --------------------------------------------

log "============================================================"
log "  W3C SPARQL Compliance Summary"
log "============================================================"
log ""

# Print header
printf "%-50s  %12s  %12s  %8s\n" "Test Suite" "RocksDB" "TiKV" "Delta"
printf "%-50s  %12s  %12s  %8s\n" \
    "$(printf '%0.s-' {1..50})" \
    "$(printf '%0.s-' {1..12})" \
    "$(printf '%0.s-' {1..12})" \
    "$(printf '%0.s-' {1..8})"

total_rocks_pass=0
total_rocks_fail=0
total_tikv_pass=0
total_tikv_fail=0

for suite in "${TEST_SUITES[@]}"; do
    # Check RocksDB result
    if [[ -f "$ROCKSDB_LOG" ]] && ! grep -q "^SKIPPED" "$ROCKSDB_LOG" 2>/dev/null; then
        if grep -qE "^test ${suite} \.\.\. ok$" "$ROCKSDB_LOG" 2>/dev/null; then
            rocks_status="PASS"
            ((total_rocks_pass++)) || true
        elif grep -qE "^test ${suite} \.\.\. FAILED$" "$ROCKSDB_LOG" 2>/dev/null; then
            rocks_status="FAIL"
            ((total_rocks_fail++)) || true
        else
            rocks_status="--"
        fi
    else
        rocks_status="--"
    fi

    # Check TiKV result
    if [[ -f "$TIKV_LOG" ]] && ! grep -q "^SKIPPED" "$TIKV_LOG" 2>/dev/null; then
        if grep -qE "^test ${suite} \.\.\. ok$" "$TIKV_LOG" 2>/dev/null; then
            tikv_status="PASS"
            ((total_tikv_pass++)) || true
        elif grep -qE "^test ${suite} \.\.\. FAILED$" "$TIKV_LOG" 2>/dev/null; then
            tikv_status="FAIL"
            ((total_tikv_fail++)) || true
        else
            tikv_status="--"
        fi
    else
        tikv_status="SKIPPED"
    fi

    # Compute delta
    if [[ "$rocks_status" == "PASS" && "$tikv_status" == "PASS" ]]; then
        delta="OK"
    elif [[ "$rocks_status" == "PASS" && "$tikv_status" == "FAIL" ]]; then
        delta="REGRESS"
    elif [[ "$rocks_status" == "FAIL" && "$tikv_status" == "PASS" ]]; then
        delta="FIXED"
    elif [[ "$tikv_status" == "SKIPPED" || "$rocks_status" == "--" ]]; then
        delta="--"
    else
        delta="BOTH-FAIL"
    fi

    printf "%-50s  %12s  %12s  %8s\n" "$suite" "$rocks_status" "$tikv_status" "$delta"
done

printf "%-50s  %12s  %12s  %8s\n" \
    "$(printf '%0.s-' {1..50})" \
    "$(printf '%0.s-' {1..12})" \
    "$(printf '%0.s-' {1..12})" \
    "$(printf '%0.s-' {1..8})"
printf "%-50s  %5d / %-4d  %5d / %-4d\n" \
    "Totals (pass / fail)" \
    "$total_rocks_pass" "$total_rocks_fail" \
    "$total_tikv_pass" "$total_tikv_fail"

log ""
log "Log files:"
[[ -f "$ROCKSDB_LOG" ]] && log "  RocksDB: $ROCKSDB_LOG"
[[ -f "$TIKV_LOG" ]]    && log "  TiKV:    $TIKV_LOG"
log ""

# Exit with error if there are regressions
if grep -q "REGRESS" <<< "$(for suite in "${TEST_SUITES[@]}"; do
    rocks_ok=false; tikv_ok=false
    [[ -f "$ROCKSDB_LOG" ]] && grep -qE "^test ${suite} \.\.\. ok$" "$ROCKSDB_LOG" 2>/dev/null && rocks_ok=true
    [[ -f "$TIKV_LOG" ]] && ! grep -q "^SKIPPED" "$TIKV_LOG" 2>/dev/null && grep -qE "^test ${suite} \.\.\. FAILED$" "$TIKV_LOG" 2>/dev/null && tikv_ok=false
    if $rocks_ok && ! $tikv_ok && [[ -f "$TIKV_LOG" ]] && ! grep -q "^SKIPPED" "$TIKV_LOG" 2>/dev/null; then
        echo "REGRESS"
    fi
done)"; then
    log "WARNING: TiKV backend has regressions compared to RocksDB baseline!"
    exit 1
fi

log "No regressions detected."
exit 0
