#!/usr/bin/env bash
#
# Move a local installation from the FlowMail identity to the Satchel identity.
#
# The app derives every storage location from its bundle identifier, so renaming
# the app without moving the data leaves it starting on an empty directory: the
# mailbox database, the .env holding the OAuth client IDs and the master key all
# live under the old identifier.
#
# This exists instead of an in-app migration because there are no external
# installations to upgrade — only the developer's own machines. Run it once per
# machine, with the app closed, before installing a build that carries the new
# identifier.
#
# The daily channel was meant to be a disposable copy of the stable one (see
# `make import-daily-data`), but the roles inverted in practice: the stable
# profile went stale while the real mailbox grew on the daily identifier.
# --promote-daily restores the intended split by turning the daily profile into
# the stable one; the daily channel then starts empty and is refilled on demand.
#
# Usage:
#   scripts/migrate-app-identity.sh                  # dry run, changes nothing
#   scripts/migrate-app-identity.sh --apply          # stable profile -> Satchel
#   scripts/migrate-app-identity.sh --promote-daily  # daily profile  -> Satchel
#   scripts/migrate-app-identity.sh --rollback       # undo it (new -> old)

set -euo pipefail

OLD_ID="de.tbuck.flowmail"
NEW_ID="de.tbuck.satchel"
OLD_INNER="flowmail"
NEW_INNER="satchel"
OLD_DB="flowmail.db"
NEW_DB="satchel.db"
OLD_KEYRING_SERVICE="flowmail"
NEW_KEYRING_SERVICE="satchel"
KEYRING_ACCOUNT="pgp-master-key-v1"
OLD_ENV_PREFIX="FLOWMAIL"
NEW_ENV_PREFIX="SATCHEL"

SUPPORT="$HOME/Library/Application Support"
LOGS="$HOME/Library/Logs"

APPLY=0
ROLLBACK=0
PROMOTE_DAILY=0
for arg in "$@"; do
  case "$arg" in
    --apply) APPLY=1 ;;
    --rollback) ROLLBACK=1 ;;
    --promote-daily) PROMOTE_DAILY=1 ;;
    -h|--help) sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

if [ "$PROMOTE_DAILY" = 1 ]; then
  OLD_ID="de.tbuck.flowmail.daily"
fi

if [ "$ROLLBACK" = 1 ]; then
  tmp="$OLD_ID"; OLD_ID="$NEW_ID"; NEW_ID="$tmp"
  tmp="$OLD_INNER"; OLD_INNER="$NEW_INNER"; NEW_INNER="$tmp"
  tmp="$OLD_DB"; OLD_DB="$NEW_DB"; NEW_DB="$tmp"
  tmp="$OLD_KEYRING_SERVICE"; OLD_KEYRING_SERVICE="$NEW_KEYRING_SERVICE"; NEW_KEYRING_SERVICE="$tmp"
  tmp="$OLD_ENV_PREFIX"; OLD_ENV_PREFIX="$NEW_ENV_PREFIX"; NEW_ENV_PREFIX="$tmp"
fi

steps=0
run() {
  steps=$((steps + 1))
  if [ "$APPLY" = 1 ]; then
    printf '  [%d] %s\n' "$steps" "$1"
    shift
    "$@"
  else
    printf '  [%d] %s\n' "$steps" "$1"
  fi
}

fail() { printf 'Abbruch: %s\n' "$1" >&2; exit 1; }

# ---------------------------------------------------------------- Vorbedingungen

if [ "$APPLY" = 1 ]; then
  # A live process holds the SQLite WAL open. Moving the directory underneath it
  # is how you get a torn database rather than a moved one.
  if pgrep -x flowmail >/dev/null 2>&1 || pgrep -x satchel >/dev/null 2>&1; then
    fail "Die App läuft noch. Erst beenden, dann erneut ausführen."
  fi
fi

src_dir="$SUPPORT/$OLD_ID"
dst_dir="$SUPPORT/$NEW_ID"

[ -d "$src_dir" ] || fail "Quellverzeichnis fehlt: $src_dir"
[ -e "$dst_dir" ] && fail "Zielverzeichnis existiert bereits: $dst_dir"

if [ "$APPLY" = 1 ]; then
  echo "Migration $OLD_ID -> $NEW_ID"
else
  echo "PROBELAUF — es wird nichts verändert. Mit --apply ausführen."
  echo "Geplant: $OLD_ID -> $NEW_ID"
fi
echo

# ------------------------------------------------------------------ Datenbestand

size=$(du -sh "$src_dir" 2>/dev/null | cut -f1)
echo "Datenverzeichnis ($size):"
run "Verschiebe $src_dir -> $dst_dir" mv "$src_dir" "$dst_dir"

inner_old="$dst_dir/$OLD_INNER"
inner_new="$dst_dir/$NEW_INNER"
if [ -d "$inner_old" ] || [ "$APPLY" = 0 ]; then
  run "Benenne inneres Verzeichnis $OLD_INNER/ -> $NEW_INNER/ um" mv "$inner_old" "$inner_new"
fi

# The database, its WAL and its shared-memory file must keep the same stem or
# SQLite treats them as belonging to a different database and discards them.
echo
echo "Datenbankdateien:"
for suffix in "" "-wal" "-shm"; do
  from="$inner_new/${OLD_DB}${suffix}"
  to="$inner_new/${NEW_DB}${suffix}"
  if [ -f "$from" ] || [ "$APPLY" = 0 ]; then
    run "${OLD_DB}${suffix} -> ${NEW_DB}${suffix}" mv "$from" "$to"
  fi
done

# Older ad-hoc backups sit next to the database and would otherwise keep the old
# name forever. They are not load-bearing, so a failure here is not fatal.
if [ "$APPLY" = 1 ] && [ -d "$inner_new" ]; then
  while IFS= read -r backup; do
    base=$(basename "$backup")
    run "Sicherung $base -> ${base/$OLD_DB/$NEW_DB}" \
      mv "$backup" "$inner_new/${base/$OLD_DB/$NEW_DB}"
  done < <(find "$inner_new" -maxdepth 1 -name "${OLD_DB}.backup-*" 2>/dev/null || true)
fi

# -------------------------------------------------------------------------- .env

# The OAuth client IDs live here under a FLOWMAIL_ prefix that the renamed build
# no longer reads. Without this the app starts fine and only fails at the point
# where you try to add an account, with a misleading "configuration missing".
echo
echo "OAuth-Konfiguration:"
env_file="$inner_new/.env"
if [ -f "$env_file" ] || [ "$APPLY" = 0 ]; then
  if [ "$APPLY" = 1 ]; then
    keys=$(grep -cE "^${OLD_ENV_PREFIX}_" "$env_file" 2>/dev/null || echo 0)
    if [ "$keys" -gt 0 ]; then
      cp "$env_file" "${env_file}.pre-rename"
      # Only the key half of each line is touched; secrets are never echoed.
      perl -pi -e "s/^${OLD_ENV_PREFIX}_/${NEW_ENV_PREFIX}_/" "$env_file"
      steps=$((steps + 1))
      printf '  [%d] %s Schlüssel %s_* -> %s_* umbenannt (Sicherung: .env.pre-rename)\n' \
        "$steps" "$keys" "$OLD_ENV_PREFIX" "$NEW_ENV_PREFIX"
    else
      echo "  (keine ${OLD_ENV_PREFIX}_-Schlüssel gefunden)"
    fi
  else
    keys=$(grep -cE "^${OLD_ENV_PREFIX}_" "$SUPPORT/$OLD_ID/$OLD_INNER/.env" 2>/dev/null || echo 0)
    steps=$((steps + 1))
    printf '  [%d] %s Schlüssel %s_* -> %s_* umbenennen\n' \
      "$steps" "$keys" "$OLD_ENV_PREFIX" "$NEW_ENV_PREFIX"
  fi
else
  echo "  (keine .env vorhanden)"
fi

# ------------------------------------------------------------------------- Logs

echo
echo "Logs:"
if [ -d "$LOGS/$OLD_ID" ]; then
  run "Verschiebe $LOGS/$OLD_ID -> $LOGS/$NEW_ID" mv "$LOGS/$OLD_ID" "$LOGS/$NEW_ID"
  if [ -d "$LOGS/$NEW_ID/$OLD_INNER" ] || [ "$APPLY" = 0 ]; then
    run "Benenne $OLD_INNER/ -> $NEW_INNER/ um" mv "$LOGS/$NEW_ID/$OLD_INNER" "$LOGS/$NEW_ID/$NEW_INNER"
  fi
else
  echo "  (kein Log-Verzeichnis unter $OLD_ID)"
fi

# --------------------------------------------------------------------- Keychain

# The master key only reaches the keychain on machines where a build with the
# keychain path actually ran; the file fallback under the data directory moves
# with it either way. Both cases are handled, neither is required.
echo
echo "Schlüsselbund:"
if security find-generic-password -s "$OLD_KEYRING_SERVICE" -a "$KEYRING_ACCOUNT" >/dev/null 2>&1; then
  if [ "$APPLY" = 1 ]; then
    secret=$(security find-generic-password -s "$OLD_KEYRING_SERVICE" -a "$KEYRING_ACCOUNT" -w)
    security add-generic-password -s "$NEW_KEYRING_SERVICE" -a "$KEYRING_ACCOUNT" -w "$secret" -U
    unset secret
    steps=$((steps + 1))
    printf '  [%d] Master-Key nach Service "%s" kopiert\n' "$steps" "$NEW_KEYRING_SERVICE"
    echo "      Der alte Eintrag bleibt bestehen — nach erfolgreichem Start löschen mit:"
    echo "      security delete-generic-password -s $OLD_KEYRING_SERVICE -a $KEYRING_ACCOUNT"
  else
    steps=$((steps + 1))
    printf '  [%d] Master-Key von Service "%s" nach "%s" kopieren\n' \
      "$steps" "$OLD_KEYRING_SERVICE" "$NEW_KEYRING_SERVICE"
  fi
else
  echo "  (kein Master-Key im Schlüsselbund — nichts zu tun)"
fi

# Per-account entries use the "<service>::<account-id>" convention.
matches=$(security dump-keychain 2>/dev/null | grep -c "\"${OLD_KEYRING_SERVICE}::" || true)
if [ "${matches:-0}" -gt 0 ]; then
  echo "  Achtung: $matches Konto-Einträge \"${OLD_KEYRING_SERVICE}::*\" gefunden."
  echo "  Diese trägt die App beim nächsten Anmelden neu an; ein Kopieren lohnt nicht."
fi

# ------------------------------------------------------------------------ Fazit

echo
# Echo back the flags this run was invoked with; dropping --promote-daily from
# the follow-up command would silently migrate the other profile.
mode=""
[ "$PROMOTE_DAILY" = 1 ] && mode=" --promote-daily"
[ "$ROLLBACK" = 1 ] && mode="$mode --rollback"

if [ "$APPLY" = 1 ]; then
  echo "Fertig — $steps Schritte."
  if [ "$ROLLBACK" = 1 ]; then
    echo "Erneut vorwärts mit: $0${mode/ --rollback/} --apply"
  else
    echo "Zurückrollen mit: $0$mode --rollback --apply"
  fi
else
  echo "$steps Schritte geplant. Ausführen mit: $0$mode --apply"
fi
