#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo 'Usage: build-notarized-macos.sh bundle-folder [Tauri options...]' >&2
  exit 2
fi
bundle_folder=$1
shift
pnpm exec tauri build "$@"

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
    pnpm exec tauri bundle "$@"
  fi
done
echo 'No valid notarization ticket after three bundle attempts; refusing to publish.' >&2
exit 65
