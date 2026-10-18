#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo 'Usage: build-notarized-macos.sh bundle-folder [Tauri options...]' >&2
  exit 2
fi
bundle_folder=$1
shift
packaging_log=$(mktemp)
stapler_log=$(mktemp)
trap 'rm -f "$packaging_log" "$stapler_log"' EXIT
export SATCHEL_REAL_XCRUN
SATCHEL_REAL_XCRUN=$(command -v xcrun)
export SATCHEL_STAPLER_RETRY_LOG="$stapler_log"
tool_directory=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/macos-tools" && pwd)
export PATH="$tool_directory:$PATH"

run_packager() {
  local status
  pnpm exec tauri "$@" 2>&1 | tee "$packaging_log"
  status=${PIPESTATUS[0]}
  cat "$stapler_log"
  : > "$stapler_log"
  return "$status"
}

package_with_retries() {
  local command=$1
  shift
  local attempt status
  for attempt in 1 2 3; do
    if run_packager "$command" "$@"; then
      return 0
    else
      status=$?
    fi
    # These errors occur after compilation. Recreate the bundles rather than
    # rebuilding Rust, and retain the failure if signing remains unavailable.
    if [ "$attempt" -eq 3 ] ||
      ! grep -Fq 'failed to bundle project:' "$packaging_log" ||
      ! grep -Eq 'A timestamp was expected but was not found|hdiutil: create failed - Resource busy' "$packaging_log"; then
      return "$status"
    fi
    echo "Temporary macOS packaging failure; recreating bundles (attempt $((attempt + 1))/3)." >&2
    sleep "$((attempt * 10))"
    command=bundle
  done
}

package_with_retries build "$@"

# Tauri CLI 2.12.3 ignores stapler's exit status. Rebuild all bundles on a
# missing ticket, so the DMG and updater archive contain the verified app too.
# https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.3/crates/tauri-macos-sign/src/lib.rs
for attempt in 1 2 3; do
  shopt -s nullglob
  apps=("$bundle_folder"/*.app)
  if [ "${#apps[@]}" -ne 1 ]; then
    echo 'Expected exactly one macOS app bundle.' >&2
    exit 1
  fi
  if xcrun stapler validate "${apps[0]}"; then
    exit 0
  fi
  if [ "$attempt" -lt 3 ]; then
    echo "Notarization ticket missing; recreating the bundles (attempt $attempt/3)." >&2
    sleep "$((attempt * 10))"
    package_with_retries bundle "$@"
  fi
done
echo 'No valid notarization ticket after three bundle attempts; refusing to publish.' >&2
exit 65
