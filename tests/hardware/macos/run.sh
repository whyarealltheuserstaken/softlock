#!/bin/bash
# One macOS v1 hardware run. --ci skips lid, Touch ID, and lock exercises.
set -euo pipefail
root="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$root"

build() {
  cargo build --locked --release --manifest-path "$1"
}

build spikes/macos-keep-awake/Cargo.toml &
keep=$!
build spikes/macos-lock-kit/Cargo.toml &
lock=$!
build spikes/macos-desktop-proof/Cargo.toml &
desk=$!
build tests/hardware/macos/Cargo.toml &
suite=$!
wait "$keep" "$lock" "$desk" "$suite"

export SUITE_ROOT="$root"
exec "$root/tests/hardware/macos/target/release/macos-hardware-suite" "$@"
