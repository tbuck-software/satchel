# Contributing

Satchel is a local-first mail client built with Vue 3, TypeScript, Rust, and
Tauri 2. Read [AGENTS.md](AGENTS.md) and [STYLEGUIDE.md](STYLEGUIDE.md) before
changing the app.

## Development

Use Node 22 or newer, pnpm 9.15.9, and rustup. The checkout pins the Rust
toolchain in `rust-toolchain.toml`. Install the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/),
including WebKitGTK 4.1 and appindicator development packages on Linux.

```sh
pnpm install --frozen-lockfile
pnpm run dev:mock
pnpm run tauri:dev
```

Copy `.env.example` to `.env` for your own OAuth registrations. Never commit
credentials. Browser mock mode needs no account or native backend.

## Checks

Before a commit, run:

```sh
make check
pnpm exec vue-tsc -b
cargo fmt --all --check
cargo clippy -p satchel -p satchel-core --all-targets -- -D warnings
```

CI also checks generated IPC contracts, migrations, dependency licenses, and
secrets. All pull requests run on GitHub-hosted runners. See [docs/CI.md](docs/CI.md).
Use Conventional Commits and describe the problem, resulting behavior, and
validation in your pull request.

## Test data and screenshots

Use invented names and reserved domains such as `example.com`, `.example`,
and `.test`. Do not copy real messages, meeting links, calendar identifiers,
contact lists, server names, or account details into fixtures or screenshots.
Keep private reproductions under the ignored `.private/` directory. Its contents
must never be added with `git add -f`.

Tests may contain explicitly documented fixture keys. Those keys must never
protect an actual account or message. Secret-scanner exceptions belong next
to the exact fixture and must explain its purpose.

## License

By contributing, you agree that your contribution is licensed under the
[MIT License](LICENSE). Third-party assets retain their own licenses; see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
