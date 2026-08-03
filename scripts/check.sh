#!/usr/bin/env bash
# The green gate: formatting, pedantic lints, every test. CI runs exactly this.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "── rustfmt ──────────────────────────────"
cargo fmt --all -- --check

echo "── clippy (warnings are errors) ─────────"
cargo clippy --workspace --all-targets -- -D warnings

echo "── tests ────────────────────────────────"
cargo test --workspace

echo "── all green ────────────────────────────"
