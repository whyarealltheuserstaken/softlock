#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"

cargo test --release --locked --manifest-path "$HERE/Cargo.toml"
cargo run --release --locked --manifest-path "$HERE/Cargo.toml" -- "$@"
