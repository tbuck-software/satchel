#!/bin/sh
set -eu
umask 077
root=${SATCHEL_PUSH_HOME:-"$HOME/.satchel-push"}
mkdir -p "$root"
python3 -m venv "$root/venv"
"$root/venv/bin/python" -m pip --disable-pip-version-check install --quiet \
  'google-auth[requests]>=2.40,<3' 'cryptography>=45,<51' 'ifaddr==0.2.0' \
  'zeroconf>=0.146,<1' 'imapclient==4.3.0' 'watchdog>=6,<7' 'keyring>=25,<26'
file=$(mktemp "$root/device.py.XXXXXX")
trap 'rm -f "$file"' EXIT
curl --fail --silent --show-error --location --proto '=https' \
  https://raw.githubusercontent.com/tbuck-software/satchel/main/tooling/push-device/device.py -o "$file"
mv "$file" "$root/device.py"
if ( : </dev/tty ) 2>/dev/null; then
  exec "$root/venv/bin/python" "$root/device.py" pair "$@" </dev/tty
fi
exec "$root/venv/bin/python" "$root/device.py" pair "$@" </dev/null
