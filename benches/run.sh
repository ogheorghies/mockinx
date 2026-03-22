#!/usr/bin/env bash
set -euo pipefail

# Performance benchmark: raw TCP vs axum baseline vs mockinx
# Requires: wrk (brew install wrk / apt install wrk)

RAW_PORT=9997
BASELINE_PORT=9998
MOCKINX_PORT=9999
WRK_THREADS=${WRK_THREADS:-4}
WRK_CONNECTIONS=${WRK_CONNECTIONS:-100}
WRK_DURATION=${WRK_DURATION:-10s}
ENDPOINT="/api/test"

BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

PIDS=()
cleanup() {
    echo -e "\n${DIM}cleaning up...${RESET}"
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
}
trap cleanup EXIT

if ! command -v wrk &>/dev/null; then
    echo "wrk not found. Install with:"
    echo "  macOS:  brew install wrk"
    echo "  Linux:  apt install wrk"
    exit 1
fi

echo -e "${BOLD}mockinx performance benchmark${RESET}"
echo -e "${DIM}wrk -t${WRK_THREADS} -c${WRK_CONNECTIONS} -d${WRK_DURATION}${RESET}"
echo

echo -e "${DIM}building release...${RESET}"
cargo build --release --bin mockinx --bin baseline-server --bin raw-tcp-server 2>&1 | tail -1

# Start all servers
./target/release/raw-tcp-server "$RAW_PORT" 2>/dev/null &
PIDS+=($!)
./target/release/baseline-server "$BASELINE_PORT" 2>/dev/null &
PIDS+=($!)
./target/release/mockinx "$MOCKINX_PORT" 2>/dev/null &
PIDS+=($!)

sleep 0.5

# Register mockinx rule
curl -s -X POST "http://localhost:${MOCKINX_PORT}/_mx" \
    -d '{"match": {"g": "/api/test"}, "reply": {"s": 200, "h": {"Content-Type": "application/json"}, "b": {"name": "Owl", "price": 5.99}}}' \
    >/dev/null

# Verify all respond
for port in $RAW_PORT $BASELINE_PORT $MOCKINX_PORT; do
    if ! curl -s "http://localhost:${port}${ENDPOINT}" >/dev/null; then
        echo "ERROR: server on port ${port} not responding"
        exit 1
    fi
done

# Warmup
echo -e "${DIM}warming up...${RESET}"
for port in $RAW_PORT $BASELINE_PORT $MOCKINX_PORT; do
    wrk -t2 -c10 -d2s "http://localhost:${port}${ENDPOINT}" >/dev/null 2>&1
done

# Benchmark
echo -e "\n${BOLD}=== RAW TCP (no HTTP parsing) ===${RESET}"
wrk -t"$WRK_THREADS" -c"$WRK_CONNECTIONS" -d"$WRK_DURATION" \
    "http://localhost:${RAW_PORT}${ENDPOINT}"

echo -e "\n${BOLD}=== BASELINE (axum) ===${RESET}"
wrk -t"$WRK_THREADS" -c"$WRK_CONNECTIONS" -d"$WRK_DURATION" \
    "http://localhost:${BASELINE_PORT}${ENDPOINT}"

echo -e "\n${BOLD}=== MOCKINX (rule match + reply) ===${RESET}"
wrk -t"$WRK_THREADS" -c"$WRK_CONNECTIONS" -d"$WRK_DURATION" \
    "http://localhost:${MOCKINX_PORT}${ENDPOINT}"

echo -e "\n${BOLD}done${RESET}"
