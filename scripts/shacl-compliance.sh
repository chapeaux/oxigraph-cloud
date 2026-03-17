#!/usr/bin/env bash
#
# shacl-compliance.sh -- Run SHACL compliance tests through the Oxigraph
#                        validation pipeline (rudof integration).
#
# Usage:
#   ./scripts/shacl-compliance.sh                  # Run all SHACL tests
#   ./scripts/shacl-compliance.sh --rudof-baseline  # Also run rudof's own test suite
#   ./scripts/shacl-compliance.sh --tikv            # Include TiKV backend tests
#   ./scripts/shacl-compliance.sh --help            # Show this help
#
# Environment variables:
#   TIKV_PD_ENDPOINTS   PD endpoints for TiKV tests (default: 127.0.0.1:2379)
#   OXIGRAPH_DIR        Path to oxigraph source root (default: auto-detected)
#   RUDOF_DIR           Path to rudof source checkout (default: /tmp/rudof)
#
# This script tests SHACL Core constraint validation through three layers:
#
#   1. Rudof baseline      -- rudof's own shacl_testsuite (proves the engine works)
#   2. SRDF bridge         -- oxigraph-shacl's W3C tests via the SRDF trait bridge
#                             against the default (RocksDB/in-memory) backend
#   3. TiKV backend        -- same tests with the TiKV storage backend
#
# Output:
#   Results are written to /tmp/shacl-compliance-{rudof,bridge,tikv}.log
#   A pass/fail summary is printed to stdout.
#

set -euo pipefail

# --- Configuration -----------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OXIGRAPH_DIR="${OXIGRAPH_DIR:-$(cd "${SCRIPT_DIR}/../oxigraph" && pwd)}"
RUDOF_DIR="${RUDOF_DIR:-/tmp/rudof}"
TIKV_PD_ENDPOINTS="${TIKV_PD_ENDPOINTS:-127.0.0.1:2379}"

RUDOF_LOG="/tmp/shacl-compliance-rudof.log"
BRIDGE_LOG="/tmp/shacl-compliance-bridge.log"
TIKV_LOG="/tmp/shacl-compliance-tikv.log"

RUN_RUDOF_BASELINE=false
RUN_TIKV=false

# --- Helpers ------------------------------------------------------------------

log() {
    echo "[$(date '+%H:%M:%S')] $*"
}

usage() {
    sed -n '2,/^$/{ s/^# \?//; p }' "$0"
    exit 0
}

# Extract pass/fail counts from cargo test output.
# cargo test prints a summary line like: "test result: ok. 42 passed; 0 failed; 0 ignored"
parse_cargo_summary() {
    local logfile="$1"
    if [[ ! -f "$logfile" ]]; then
        echo "-- --"
        return
    fi
    if grep -q "^SKIPPED" "$logfile" 2>/dev/null; then
        echo "SKIP SKIP"
        return
    fi
    local passed failed
    passed=$(grep -oP '\d+ passed' "$logfile" | tail -1 | grep -oP '\d+' || echo 0)
    failed=$(grep -oP '\d+ failed' "$logfile" | tail -1 | grep -oP '\d+' || echo 0)
    echo "$passed $failed"
}

# Check TiKV availability
tikv_available() {
    if [[ -z "$TIKV_PD_ENDPOINTS" ]]; then
        return 1
    fi
    local host port
    host="${TIKV_PD_ENDPOINTS%%:*}"
    port="${TIKV_PD_ENDPOINTS##*:}"
    if command -v nc &>/dev/null; then
        nc -z -w 2 "$host" "$port" 2>/dev/null
    elif command -v bash &>/dev/null; then
        (echo > "/dev/tcp/$host/$port") 2>/dev/null
    else
        return 0
    fi
}

# --- Argument parsing ---------------------------------------------------------

while [[ $# -gt 0 ]]; do
    case "$1" in
        --rudof-baseline)
            RUN_RUDOF_BASELINE=true
            shift
            ;;
        --tikv)
            RUN_TIKV=true
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

# --- Step 1: Rudof baseline (optional) ----------------------------------------

if $RUN_RUDOF_BASELINE; then
    log "============================================================"
    log "  Step 1: Rudof SHACL Test Suite (baseline)"
    log "============================================================"
    log ""

    if [[ ! -d "$RUDOF_DIR" ]]; then
        log "Cloning rudof to $RUDOF_DIR..."
        git clone --depth 1 https://github.com/rudof-project/rudof.git "$RUDOF_DIR"
    else
        log "Using existing rudof checkout at $RUDOF_DIR"
    fi

    log "Running rudof's shacl_testsuite..."
    log "Output: $RUDOF_LOG"

    cd "$RUDOF_DIR"
    if cargo test -p shacl_testsuite 2>&1 | tee "$RUDOF_LOG"; then
        log "Rudof baseline: tests completed successfully."
    else
        log "Rudof baseline: some tests failed (see $RUDOF_LOG)."
    fi
    log ""
fi

# --- Step 2: SRDF bridge tests (RocksDB/in-memory backend) -------------------

log "============================================================"
log "  Step 2: SHACL via SRDF Bridge (default backend)"
log "============================================================"
log ""

cd "$OXIGRAPH_DIR"

# Check if oxigraph-shacl crate exists yet
if [[ -d "$OXIGRAPH_DIR/../oxigraph-shacl" ]] || \
   cargo metadata --no-deps --format-version=1 2>/dev/null | grep -q '"name":"oxigraph-shacl"'; then
    log "Running oxigraph-shacl W3C SHACL tests..."
    log "Output: $BRIDGE_LOG"

    if cargo test -p oxigraph-shacl --test shacl_w3c 2>&1 | tee "$BRIDGE_LOG"; then
        log "SRDF bridge (default backend): tests completed successfully."
    else
        log "SRDF bridge (default backend): some tests failed (see $BRIDGE_LOG)."
    fi
else
    log "NOTE: oxigraph-shacl crate not found."
    log ""
    log "The oxigraph-shacl crate (with W3C SHACL test harness) has not been"
    log "created yet. This is expected during early development phases."
    log ""
    log "To create it, implement Task 3.1 (SRDF trait bridge) which will produce:"
    log "  - oxigraph-shacl/Cargo.toml"
    log "  - oxigraph-shacl/src/lib.rs       (SRDF trait impl for Store<B>)"
    log "  - oxigraph-shacl/tests/shacl_w3c.rs  (W3C SHACL test harness)"
    log ""
    log "The test harness should:"
    log "  1. Load W3C SHACL Core test suite manifest (tests/w3c-shacl/core/manifest.ttl)"
    log "  2. For each test case:"
    log "     a. Load the data graph into an Oxigraph Store"
    log "     b. Parse the shapes graph via rudof's ShaclParser"
    log "     c. Validate via rudof's shacl_validation crate through the SRDF bridge"
    log "     d. Compare the validation report's sh:conforms to expected result"
    log "  3. Report pass/fail per SHACL Core constraint type"
    log ""
    echo "SKIPPED: oxigraph-shacl crate not yet available" > "$BRIDGE_LOG"
fi

log ""

# --- Step 3: TiKV backend SHACL tests (optional) -----------------------------

if $RUN_TIKV; then
    log "============================================================"
    log "  Step 3: SHACL via SRDF Bridge (TiKV backend)"
    log "============================================================"
    log ""

    if ! tikv_available; then
        log "SKIP: TiKV not available at $TIKV_PD_ENDPOINTS"
        log "  Start a TiKV cluster first: ./scripts/tikv-dev-cluster.sh start"
        echo "SKIPPED: TiKV not available at $TIKV_PD_ENDPOINTS" > "$TIKV_LOG"
    elif [[ ! -d "$OXIGRAPH_DIR/../oxigraph-shacl" ]] && \
         ! cargo metadata --no-deps --format-version=1 2>/dev/null | grep -q '"name":"oxigraph-shacl"'; then
        log "SKIP: oxigraph-shacl crate not yet available (see Step 2 notes)."
        echo "SKIPPED: oxigraph-shacl crate not yet available" > "$TIKV_LOG"
    else
        log "Running oxigraph-shacl W3C SHACL tests with TiKV backend..."
        log "  PD endpoints: $TIKV_PD_ENDPOINTS"
        log "Output: $TIKV_LOG"

        cd "$OXIGRAPH_DIR"
        if TIKV_PD_ENDPOINTS="$TIKV_PD_ENDPOINTS" \
            cargo test -p oxigraph-shacl --features tikv --test shacl_w3c 2>&1 | tee "$TIKV_LOG"; then
            log "TiKV SHACL: tests completed successfully."
        else
            log "TiKV SHACL: some tests failed (see $TIKV_LOG)."
        fi
    fi
    log ""
fi

# --- Summary ------------------------------------------------------------------

log "============================================================"
log "  SHACL Compliance Summary"
log "============================================================"
log ""

printf "%-40s  %10s  %10s\n" "Test Layer" "Passed" "Failed"
printf "%-40s  %10s  %10s\n" \
    "$(printf '%0.s-' {1..40})" \
    "$(printf '%0.s-' {1..10})" \
    "$(printf '%0.s-' {1..10})"

# Rudof baseline
if $RUN_RUDOF_BASELINE; then
    read -r rp rf <<< "$(parse_cargo_summary "$RUDOF_LOG")"
    printf "%-40s  %10s  %10s\n" "Rudof shacl_testsuite (baseline)" "$rp" "$rf"
fi

# SRDF bridge (default backend)
read -r bp bf <<< "$(parse_cargo_summary "$BRIDGE_LOG")"
printf "%-40s  %10s  %10s\n" "oxigraph-shacl (default backend)" "$bp" "$bf"

# TiKV backend
if $RUN_TIKV; then
    read -r tp tf <<< "$(parse_cargo_summary "$TIKV_LOG")"
    printf "%-40s  %10s  %10s\n" "oxigraph-shacl (TiKV backend)" "$tp" "$tf"
fi

printf "%-40s  %10s  %10s\n" \
    "$(printf '%0.s-' {1..40})" \
    "$(printf '%0.s-' {1..10})" \
    "$(printf '%0.s-' {1..10})"

log ""
log "Log files:"
$RUN_RUDOF_BASELINE && [[ -f "$RUDOF_LOG" ]] && log "  Rudof baseline: $RUDOF_LOG"
[[ -f "$BRIDGE_LOG" ]] && log "  SRDF bridge:    $BRIDGE_LOG"
$RUN_TIKV && [[ -f "$TIKV_LOG" ]] && log "  TiKV backend:   $TIKV_LOG"
log ""

# Detect failures
has_failures=false
if [[ -f "$BRIDGE_LOG" ]] && ! grep -q "^SKIPPED" "$BRIDGE_LOG" 2>/dev/null; then
    if grep -qP '\d+ failed' "$BRIDGE_LOG" 2>/dev/null; then
        fail_count=$(grep -oP '\d+ failed' "$BRIDGE_LOG" | tail -1 | grep -oP '\d+')
        if [[ "$fail_count" -gt 0 ]]; then
            has_failures=true
        fi
    fi
fi

if $has_failures; then
    log "WARNING: SHACL compliance tests have failures. Review logs above."
    exit 1
fi

log "SHACL compliance check complete."
exit 0
