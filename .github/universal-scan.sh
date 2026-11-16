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

# Print every minimum-macOS line, then require macOS 11.0.
require_minos() {
  bin="$1"
  echo "=== otool minos ${bin} ==="
  lines="$(otool -l "${bin}" | grep 'minos ' || true)"
  printf '%s\n' "${lines}"
  if [ -z "${lines}" ]; then
    echo "no minos in ${bin}"
    exit 1
  fi
  bad="$(printf '%s\n' "${lines}" | grep -v 'minos 11.0' || true)"
  if [ -n "${bad}" ]; then
    echo "deployment target is not 11.0:"
    printf '%s\n' "${bad}"
    exit 1
  fi
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
  echo "negative control: scan failed on a fat Mach-O with release markers"
}
