#!/usr/bin/env bash
# Dev loop: throwaway server + client against it, both debug builds.
# Usage: scripts/dev.sh            (server in background, client foreground)
#        scripts/dev.sh server     (server only, foreground, debug logs)
set -euo pipefail
cd "$(dirname "$0")/.."

DEV_DIR="${TMPDIR:-/tmp}/teresa-dev"
mkdir -p "$DEV_DIR"

if [[ "${1:-}" == "server" ]]; then
    exec env RUST_LOG="${RUST_LOG:-teresa_server=debug}" \
        cargo run -p teresa-server -- --config <(printf 'database = "%s/dev.db"\n' "$DEV_DIR")
fi

cargo build -p teresa-server -p teresa-client

printf 'database = "%s/dev.db"\n' "$DEV_DIR" > "$DEV_DIR/server.toml"
./target/debug/teresad --config "$DEV_DIR/server.toml" &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT
sleep 0.5

# Isolated client state so your real identity is untouched.
HOME_DIR="$DEV_DIR/home"
mkdir -p "$HOME_DIR"
HOME="$HOME_DIR" USERPROFILE="$HOME_DIR" ./target/debug/teresa "$@"
