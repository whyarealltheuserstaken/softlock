#!/usr/bin/env bash
# Fail-closed PR path classification for CI and Universal workflows.
set -euo pipefail

starts_with() {
  [[ "$1" == "$2"* ]]
}

platform_of() {
  local file="$1"
  if [[ "$file" == *.md ]]; then
    echo docs
    return
  fi
  if starts_with "$file" "crates/softlock-macos/" \
    || starts_with "$file" "spikes/macos-" \
    || starts_with "$file" "tests/hardware/macos/"; then
    echo macos
    return
  fi
  if starts_with "$file" "crates/softlock-windows/" \
    || starts_with "$file" "crates/softlock-seam3/" \
    || starts_with "$file" "spikes/desktop-proof/" \
    || starts_with "$file" "spikes/input-hook/" \
    || starts_with "$file" "spikes/os-lock/" \
    || starts_with "$file" "tests/hardware/windows/"; then
    echo windows
    return
  fi
  if starts_with "$file" "crates/softlock-linux/" \
    || starts_with "$file" "tests/hardware/linux/"; then
    echo linux
    return
  fi
  echo unknown
}

classify_file_list() {
  SHARED=false
  LINUX=false
  WINDOWS=false
  MACOS=false
  DOCS_ONLY=true
  local file platform saw_file=false

  for file in "$@"; do
    [[ -z "$file" ]] && continue
    saw_file=true
    platform="$(platform_of "$file")"
    case "$platform" in
      docs) ;;
      macos)
        DOCS_ONLY=false
        MACOS=true
        ;;
      windows)
        DOCS_ONLY=false
        WINDOWS=true
        ;;
      linux)
        DOCS_ONLY=false
        LINUX=true
        ;;
      unknown)
        DOCS_ONLY=false
        SHARED=true
        ;;
    esac
  done

  if ! $saw_file; then
    SHARED=true
    DOCS_ONLY=false
  fi
}

list_changed_files() {
  if [[ "${GITHUB_EVENT_NAME:-}" == "pull_request" ]]; then
    local base="${GITHUB_EVENT_PULL_REQUEST_BASE_SHA:-}"
    if [[ -z "$base" ]]; then
      echo "missing GITHUB_EVENT_PULL_REQUEST_BASE_SHA" >&2
      exit 1
    fi
    git diff --name-only "${base}...HEAD"
    return
  fi
  if [[ "${GITHUB_REF:-}" == "refs/heads/main" ]]; then
    return
  fi
  echo "unsupported event for path classification: ${GITHUB_EVENT_NAME:-} ${GITHUB_REF:-}" >&2
  exit 1
}

emit_outputs() {
  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    {
      echo "shared=${SHARED}"
      echo "linux=${LINUX}"
      echo "windows=${WINDOWS}"
      echo "macos=${MACOS}"
      echo "docs_only=${DOCS_ONLY}"
    } >>"$GITHUB_OUTPUT"
  fi
}

run_self_check() {
  expect() {
    local label="$1"
    shift
    local want_shared="$1" want_linux="$2" want_windows="$3" want_macos="$4" want_docs="$5"
    shift 5
    classify_file_list "$@"
    if [[ "$want_shared" != "$SHARED" || "$want_linux" != "$LINUX" || "$want_windows" != "$WINDOWS" || "$want_macos" != "$MACOS" || "$want_docs" != "$DOCS_ONLY" ]]; then
      echo "${label}: wanted shared=${want_shared} linux=${want_linux} windows=${want_windows} macos=${want_macos} docs_only=${want_docs}" >&2
      echo "  got shared=${SHARED} linux=${LINUX} windows=${WINDOWS} macos=${MACOS} docs_only=${DOCS_ONLY} files=$*" >&2
      exit 1
    fi
  }

  expect new-crate true false false false false crates/softlock-new/src/lib.rs
  expect macos-spike false false false true false spikes/macos-lock-kit/src/foo.rs
  expect windows-hw false false true false false tests/hardware/windows/run.ps1
  expect linux-hw false true false false false tests/hardware/linux/run.sh
  expect readme false false false false true README.md
  expect hw-schema true false false false false tests/hardware/check-schema.py
  expect desktop-spike false false true false false spikes/desktop-proof/src/main.rs
  expect os-lock-spike false false true false false spikes/os-lock/src/main.rs
  expect overlay-ws true false false false false spikes/overlay/src/main.rs
  expect mixed-macos false false false true false docs/guide.md crates/softlock-macos/src/lib.rs

  echo "ci-path-outputs self-check ok"
}

main() {
  if [[ "${1:-}" == "--self-check" ]]; then
    run_self_check
    return
  fi

  if [[ "${GITHUB_REF:-}" == "refs/heads/main" ]]; then
    SHARED=true
    LINUX=true
    WINDOWS=true
    MACOS=true
    DOCS_ONLY=false
    emit_outputs
    return
  fi

  mapfile -t changed < <(list_changed_files)
  classify_file_list "${changed[@]}"
  emit_outputs
}

main "$@"