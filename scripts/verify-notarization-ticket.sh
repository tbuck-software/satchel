#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo 'Usage: verify-notarization-ticket.sh app-bundle' >&2
  exit 2
fi

# stapler validates with Apple's service and can lose the connection even when
# notarization was accepted. Never publish unless validation actually succeeds.
for attempt in 1 2 3; do
  if xcrun stapler validate "$1"; then
    exit 0
  else
    status=$?
  fi
  if [ "$attempt" -lt 3 ]; then
    echo "Ticket validation failed; retrying (attempt $((attempt + 1))/3)." >&2
    sleep "$((attempt * 10))"
  fi
done
exit "$status"
