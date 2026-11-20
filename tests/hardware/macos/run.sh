#!/bin/bash
# One macOS v1 hardware run. One shared target directory.
# --build-only compiles and prints the build time, then stops.
# --ci skips lid, Touch ID, and lock exercises.
set -euo pipefail
root="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$root"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target/hardware-suite}"

build_one() {
  cargo build --locked --release --manifest-path "$1"
}

host_target() {
  local arch
  arch="$(uname -m)"
  case "$arch" in
    arm64) printf '%s\n' aarch64-apple-darwin ;;
    x86_64) printf '%s\n' x86_64-apple-darwin ;;
    *) printf '%s\n' "" ;;
  esac
}

other_target() {
  case "$(host_target)" in
    aarch64-apple-darwin) printf '%s\n' x86_64-apple-darwin ;;
    x86_64-apple-darwin) printf '%s\n' aarch64-apple-darwin ;;
    *) printf '%s\n' "" ;;
  esac
}

ensure_target() {
  local triple="$1"
  if rustup target list --installed | grep -qx "$triple"; then
    return 0
  fi
  rustup target add "$triple"
}

build_all() {
  local manifest
  for manifest in \
    spikes/macos-keep-awake/Cargo.toml \
    spikes/macos-physical-input/Cargo.toml \
    spikes/macos-lock-kit/Cargo.toml \
    spikes/macos-desktop-proof/Cargo.toml \
    tests/hardware/macos/Cargo.toml
  do
    build_one "$manifest"
  done
  local other
  other="$(other_target)"
  if [ -n "$other" ]; then
    ensure_target "$other"
    ensure_target "$(host_target)"
    cargo build --locked --release --manifest-path spikes/macos-desktop-proof/Cargo.toml --target "$other"
    cargo build --locked --release --manifest-path spikes/macos-desktop-proof/Cargo.toml --target "$(host_target)"
    local fat_dir="$CARGO_TARGET_DIR/fat"
    mkdir -p "$fat_dir"
    local host_bin="$CARGO_TARGET_DIR/$(host_target)/release/macos-desktop-proof"
    local other_bin="$CARGO_TARGET_DIR/$other/release/macos-desktop-proof"
    local fat="$fat_dir/macos-desktop-proof"
    lipo -create "$host_bin" "$other_bin" -output "$fat"
    chmod +x "$fat"
    codesign --force --sign - "$fat" >/dev/null
    export SUITE_SMOKE_BIN="$fat"
    export SUITE_DESKTOP_BIN="$host_bin"
  fi
}

start=$(date +%s)
build_all
build_end=$(date +%s)
build_s=$((build_end - start))
echo "build=${build_s}s target=${CARGO_TARGET_DIR}"

if [ "${1:-}" = "--build-only" ]; then
  echo "Build only. Next: ./tests/hardware/macos/run.sh"
  exit 0
fi

export SUITE_BUILD_MS=$((build_s * 1000))
export SUITE_ROOT="$root"
exec "$CARGO_TARGET_DIR/release/macos-hardware-suite" "$@"
