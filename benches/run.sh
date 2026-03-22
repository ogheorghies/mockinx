#!/usr/bin/env bash
set -euo pipefail

# Performance benchmark: baseline axum vs mockinx
# Requires: wrk (brew install wrk / apt install wrk)

BASELINE_PORT=9998
MOCKINX_PORT=9999
WRK_THREADS=${WRK_THREADS:-4}
WRK_CONNECTIONS=${WRK_CONNECTIONS:-100}
WRK_DURATION=${WRK_DURATION:-10s}
ENDPOINT="/api/test"

# Colors
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

cleanup() {
    echo -e "\n${DIM}cleaning up...${RESET}"
    kill "$BASELINE_PID" 2>/dev/null || true
    kill "$MOCKINX_PID" 2>/dev/null || true
    wait "$BASELINE_PID" 2>/dev/null || true
    wait "$MOCKINX_PID" 2>/dev/null || true
}
trap cleanup EXIT

# Check wrk
if ! command -v wrk &>/dev/null; then
    echo "wrk not found. Install with:"
    echo "  macOS:  brew install wrk"
    echo "  Linux:  apt install wrk"
    exit 1
fi

echo -e "${BOLD}mockinx performance benchmark${RESET}"
echo -e "${DIM}wrk -t${WRK_THREADS} -c${WRK_CONNECTIONS} -d${WRK_DURATION}${RESET}"
echo

# Build release
echo -e "${DIM}building release...${RESET}"
cargo build --release --bin mockinx --bin baseline-server 2>&1 | tail -1

# Start baseline server
./target/release/baseline-server "$BASELINE_PORT" 2>/dev/null &
BASELINE_PID=$!

# Start mockinx and register a rule
./target/release/mockinx "$MOCKINX_PORT" 2>/dev/null &
MOCKINX_PID=$!

# Wait for servers to be ready
sleep 0.5

# Register mockinx rule (same response as baseline)
curl -s -X POST "http://localhost:${MOCKINX_PORT}/_mx" \
    -d '{"match": {"g": "/api/test"}, "reply": {"s": 200, "h": {"Content-Type": "application/json"}, "b": {"name": "Owl", "price": 5.99}}}' \
    >/dev/null

# Verify both respond
BASELINE_CHECK=$(curl -s "http://localhost:${BASELINE_PORT}${ENDPOINT}")
MOCKINX_CHECK=$(curl -s "http://localhost:${MOCKINX_PORT}${ENDPOINT}")

if [ -z "$BASELINE_CHECK" ] || [ -z "$MOCKINX_CHECK" ]; then
    echo "ERROR: servers not responding"
    exit 1
fi

# Warmup
echo -e "${DIM}warming up...${RESET}"
wrk -t2 -c10 -d2s "http://localhost:${BASELINE_PORT}${ENDPOINT}" >/dev/null 2>&1
wrk -t2 -c10 -d2s "http://localhost:${MOCKINX_PORT}${ENDPOINT}" >/dev/null 2>&1

# Benchmark baseline
echo -e "\n${BOLD}=== BASELINE (raw axum) ===${RESET}"
wrk -t"$WRK_THREADS" -c"$WRK_CONNECTIONS" -d"$WRK_DURATION" \
    "http://localhost:${BASELINE_PORT}${ENDPOINT}"

# Benchmark mockinx
echo -e "\n${BOLD}=== MOCKINX (rule match + reply) ===${RESET}"
wrk -t"$WRK_THREADS" -c"$WRK_CONNECTIONS" -d"$WRK_DURATION" \
    "http://localhost:${MOCKINX_PORT}${ENDPOINT}"

echo -e "\n${BOLD}done${RESET}"
