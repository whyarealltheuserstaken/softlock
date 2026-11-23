# Helpers for the macOS universal job.
#
# Ad-hoc codesign in the workflow is Decision D2 (unsigned or ad-hoc).

both_archs() {
  label="$1"
  value="$2"
  case "${value}" in
    *x86_64*) ;;
    *)
      echo "${label} missing x86_64: ${value}"
      exit 1
      ;;
  esac
  case "${value}" in
    *arm64*) ;;
    *)
      echo "${label} missing arm64: ${value}"
      exit 1
      ;;
  esac
}

# Require macOS 11.0 on the x86_64 slice and on the arm64 slice separately.
# `otool -arch all` is printed for display and is not the check.
# LC_BUILD_VERSION uses `minos`. LC_VERSION_MIN_MACOSX uses `version`.
require_minos() {
  bin="$1"
  echo "=== otool minos ${bin} ==="
  otool -arch all -l "${bin}" | grep 'minos ' || true
  for slice_arch in x86_64 arm64; do
    slice_lines="$(otool -arch "${slice_arch}" -l "${bin}" | grep 'minos ' || true)"
    if [ -z "${slice_lines}" ]; then
      slice_lines="$(vtool -arch "${slice_arch}" -show-build "${bin}" | awk '
        /LC_VERSION_MIN_MACOSX/ { grab = 1; next }
        /cmd LC_/ { grab = 0 }
        grab && $1 == "version" { print $1 " " $2; grab = 0 }
      ')"
    fi
    if [ -z "${slice_lines}" ]; then
      echo "${slice_arch}: no minos or version in ${bin}"
      exit 1
    fi
    while IFS= read -r slice_line; do
      [ -z "${slice_line}" ] && continue
      case "${slice_line}" in
        *"minos 11.0"*)
          echo "${slice_arch}: minos 11.0"
          ;;
        *"version 11.0"*)
          echo "${slice_arch}: version 11.0"
          ;;
        *)
          echo "${slice_arch}: deployment target is not 11.0: ${slice_line}"
          exit 1
          ;;
      esac
    done <<EOF
${slice_lines}
EOF
  done
}

scan_release() {
  dir="$1"
  log="$2"
  echo "=== release scan ${dir} ==="
  SOFTLOCK_RELEASE_SCAN_DIR="${PWD}/${dir}" \
    cargo test --locked -p softlock-lockhost --test release_binary -- --nocapture product_release_binaries \
    2>&1 | tee "${log}"
  grep -F "scanned release binaries" "${log}"
}

# Copy a fat Mach-O, append the release-scan marker bytes, and require the
# scan to fail. The copy is not a product binary. The scanner does not parse
# Mach-O; the appended bytes are still inside the file it reads.
scan_fails_when_fat_binary_has_markers() {
  src="$1"
  dir="target/universal/negative"
  mkdir -p "${dir}"
  dest="${dir}/$(basename "${src}")"
  cp "${src}" "${dest}"
  chmod +x "${dest}"
  printf '%s' 'softlock-simulated-physical-marker' >> "${dest}"
  printf '%s' 'softlock_simulated_physical_marker' >> "${dest}"
  printf '%s' 'softlock-dev-launch-marker' >> "${dest}"
  printf '%s' 'softlock_dev_launch_marker' >> "${dest}"
  printf '%s' 'softlock-live-fail-closed-marker' >> "${dest}"
  printf '%s' 'softlock_live_fail_closed_marker' >> "${dest}"
  printf '%s' 'softlock-present-physical-pin' >> "${dest}"
  echo "=== negative control ${dest} ==="
  echo "appended softlock-simulated-physical-marker"
  echo "appended softlock_simulated_physical_marker"
  echo "appended softlock-dev-launch-marker"
  echo "appended softlock_dev_launch_marker"
  log="target/universal/negative-scan.txt"
  set +e
  SOFTLOCK_RELEASE_SCAN_DIR="${PWD}/${dir}" \
    cargo test --locked -p softlock-lockhost --test release_binary -- --nocapture product_release_binaries \
    > "${log}" 2>&1
  status=$?
  set -e
  cat "${log}"
  if [ "${status}" -eq 0 ]; then
    echo "negative control: scan accepted a fat Mach-O that contains release markers"
    exit 1
  fi
  grep -F "test-only markers in release binaries" "${log}"
  grep -F "string=true symbol=true dev_string=true dev_symbol=true" "${log}"
  grep -F "pin_string=true" "${log}"
  echo "negative control: scan failed on a fat Mach-O with release markers"
}
