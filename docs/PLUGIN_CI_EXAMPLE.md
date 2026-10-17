# Plugin CI Example

```yaml
name: plugin

on:
  pull_request:
  push:
    branches: [main]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-wasip2
      - run: pnpm install --frozen-lockfile
      - run: pnpm run plugins:build
      - run: pnpm run plugin -- list
```

The host validates `[package.metadata.satchel]` for API version, capabilities,
trust level, UI slots, and component signatures at discovery time.

Signatures come in two forms:

- `ed25519:<hex>` — a real signature over the component artifact, verified
  against the trusted keys embedded in
  `src-tauri/src/plugins/registry.rs` (extend via
  `SATCHEL_PLUGIN_TRUSTED_KEYS`). Only this form earns `trust = "verified"`
  and sets `signature_verified`.
- `sha256:<hex>` — an integrity digest. It detects accidental corruption but
  never grants trust, since anyone who modifies the artifact can recompute it.

Sign a component with `pnpm run plugin:sign -- <path-to-component.wasm>`
(signing key defaults to `.private/plugin-signing-key.pem`, override with
`SATCHEL_PLUGIN_SIGNING_KEY`).

Unsigned plugins are allowed in debug builds only; release builds reject them
unless `SATCHEL_ALLOW_UNSIGNED_PLUGINS=1` is set explicitly.
