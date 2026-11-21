PYTHON ?= python3

.PHONY: nightly stable help check test-push-device icons dev dev-android dev-ios daily install install-android install-all all _install-android-after-mac release import-daily-data render-fidelity

help:
	@printf '%s\n' \
		'Satchel commands:' \
		'  make nightly  Download, verify, install, and open the latest nightly' \
		'  make stable   Download, verify, install, and open the latest stable release' \
		'  make daily    Build, check, install, and open Satchel Daily.app' \
		'  make install  Build, check, install, and open stable Satchel.app' \
		'  make install-android  Build, check, install, and open Satchel on a connected Android device' \
		'  make install-all  Build, check, install, and open Satchel on macOS and Android' \
		'  make import-daily-data  Copy stable DB/settings into Daily channel' \
		'  make render-fidelity    Render known real-mail cases from the Daily DB and check fidelity' \
		'  make release  Run release checks and build signed/notarized release' \
		'  make dev          Run the desktop app with hot reload' \
		'  make dev-android  Run the Android app with hot reload' \
		'  make dev-ios      Run the iOS app with hot reload' \
		'  make check    Run frontend, Rust, and push-device tests' \
		'  make test-push-device    Run push-device tests in a temporary Python environment' \
		'  make icons    Render every app, tray, notification and web icon from public/branding'

check: test-push-device
	pnpm run check

test-push-device:
	@set -eu; \
	venv=$$(mktemp -d); \
	trap 'rm -rf "$$venv"' EXIT; \
	$(PYTHON) -m venv "$$venv"; \
	"$$venv/bin/python" -m pip --disable-pip-version-check install --quiet \
		--require-hashes --only-binary=:all: -r tooling/push-device/requirements-test.txt; \
	"$$venv/bin/python" -B -m unittest discover -v -s tooling/push-device

icons:
	pnpm run icons

dev:
	pnpm run tauri:dev

dev-android:
	pnpm run tauri:android:dev

dev-ios:
	pnpm run tauri:ios:dev

daily:
	pnpm run install:daily:mac

import-daily-data:
	pnpm run import:daily:data

render-fidelity:
	pnpm run render:fidelity -- --cache-inline-images

install:
	pnpm run install:mac

install-android:
	pnpm run install:android

all: install-all

install-all: _install-android-after-mac

_install-android-after-mac: install
	pnpm run plugins:build
	node ./scripts/install-android.mjs --open

release:
	pnpm run release:mac

nightly:
	pnpm run install:nightly

stable:
	pnpm run install:stable
