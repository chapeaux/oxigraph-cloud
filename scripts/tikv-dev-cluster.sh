#!/usr/bin/env bash
#
# tikv-dev-cluster.sh -- Start or stop a local TiKV development cluster
#
# Uses TiUP playground to run 1 PD + 3 TiKV nodes locally.
# PD client endpoint: 127.0.0.1:2379
#
# Usage:
#   ./tikv-dev-cluster.sh start   # Start the cluster (default if no arg)
#   ./tikv-dev-cluster.sh stop    # Stop the cluster
#   ./tikv-dev-cluster.sh status  # Check cluster health
#
# Requirements:
#   - TiUP (https://tiup.io/)
#   - Linux or macOS
#
# Environment variables:
#   TIKV_PD_PORT    PD client port (default: 2379)
#   TIKV_KV_COUNT   Number of TiKV nodes (default: 3)
#

set -euo pipefail

PD_PORT="${TIKV_PD_PORT:-2379}"
KV_COUNT="${TIKV_KV_COUNT:-3}"
PD_ENDPOINT="127.0.0.1:${PD_PORT}"

# Log a timestamped message.
log() {
    echo "[$(date '+%H:%M:%S')] $*"
}

# Check whether TiUP is installed and on the PATH.
check_tiup() {
    if command -v tiup &>/dev/null; then
        log "TiUP found: $(tiup --version 2>&1 | head -1)"
        return 0
    fi

    cat <<'INSTALL_MSG'

  TiUP is not installed.  Install it with:

      curl --proto '=https' --tlsv1.2 -sSf \
          https://tiup-mirrors.pingcap.com/install.sh | sh

  Then add it to your PATH (the installer prints the exact line) and re-run
  this script.

INSTALL_MSG
    return 1
}

# Start the playground cluster.
do_start() {
    check_tiup

    log "Starting TiKV playground: 1 PD + ${KV_COUNT} TiKV nodes ..."
    log "PD client endpoint will be: ${PD_ENDPOINT}"

    # Launch playground in the background. The --mode tikv-slim flag omits
    # TiDB (the SQL layer) since we only need the KV store.
    tiup playground --mode tikv-slim \
        --kv "${KV_COUNT}" \
        --pd 1 \
        --pd.port "${PD_PORT}" \
        --tag oxigraph-dev &

    PLAYGROUND_PID=$!

    log "Playground starting (PID ${PLAYGROUND_PID}). Waiting for PD to become healthy ..."

    # Wait up to 120 seconds for PD to accept connections.
    local retries=0
    local max_retries=60
    while (( retries < max_retries )); do
        if curl -s "http://${PD_ENDPOINT}/pd/api/v1/health" >/dev/null 2>&1; then
            log "PD is healthy."
            break
        fi
        retries=$((retries + 1))
        sleep 2
    done

    if (( retries >= max_retries )); then
        log "ERROR: PD did not become healthy within 120 seconds."
        log "Check 'tiup playground display --tag oxigraph-dev' for details."
        exit 1
    fi

    # Give TiKV nodes a few more seconds to register with PD.
    sleep 5

    # Verify store count.
    local store_count
    store_count=$(curl -s "http://${PD_ENDPOINT}/pd/api/v1/stores" \
        | grep -c '"state_name": *"Up"' || true)
    log "TiKV stores up: ${store_count} / ${KV_COUNT}"

    echo ""
    echo "============================================"
    echo "  TiKV dev cluster is running"
    echo "  PD endpoint:  ${PD_ENDPOINT}"
    echo ""
    echo "  Use with Oxigraph:"
    echo "    --pd-endpoints ${PD_ENDPOINT}"
    echo ""
    echo "  Rust client connection string:"
    echo "    TransactionClient::new(vec![\"${PD_ENDPOINT}\"])"
    echo ""
    echo "  Stop with:"
    echo "    $0 stop"
    echo "============================================"
    echo ""

    # Wait for the playground process so the script stays alive.
    wait "${PLAYGROUND_PID}" 2>/dev/null || true
}

# Stop the playground cluster.
do_stop() {
    check_tiup

    log "Stopping TiKV playground (tag: oxigraph-dev) ..."
    tiup clean oxigraph-dev 2>/dev/null || true
    log "Cluster stopped."
}

# Print cluster health status.
do_status() {
    if ! curl -s "http://${PD_ENDPOINT}/pd/api/v1/health" >/dev/null 2>&1; then
        log "PD is not reachable at ${PD_ENDPOINT}."
        exit 1
    fi

    log "PD is healthy at ${PD_ENDPOINT}."
    echo ""
    echo "Stores:"
    curl -s "http://${PD_ENDPOINT}/pd/api/v1/stores" \
        | python3 -m json.tool 2>/dev/null \
        || curl -s "http://${PD_ENDPOINT}/pd/api/v1/stores"
    echo ""
}

# ---- main ----

case "${1:-start}" in
    start)
        do_start
        ;;
    stop)
        do_stop
        ;;
    status)
        do_status
        ;;
    *)
        echo "Usage: $0 {start|stop|status}"
        exit 1
        ;;
esac
