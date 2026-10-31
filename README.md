# Astrolabe

Astrolabe is an MIT-licensed desktop shell and Rust core for running agents
through an adapter protocol (ACP), with owner notes and memory in a separate
private agent repo.

Status: **phase 0**. This repository contains contracts, a CLI stub, a blank
Tauri window showing the version, and a private agent-repo seed. It does not yet
start agents, provide an MCP server, pair machines, or manage notes.

See [the design overview](docs/overview.md), [decisions](docs/decisions.md),
[version pins](docs/versions.md), and [protocol contract](crates/protocol/README.md).

## Build

Install Rust 1.99.1 (rustfmt and clippy), Node 26.x, and pnpm 11.0.9.
Install the [Tauri native prerequisites](https://v2.tauri.app/start/prerequisites/):
Xcode tooling on macOS; MSVC C++ Build Tools and WebView2 on Windows.
The commands below work in a terminal or native PowerShell, from the repo root:

```text
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo run -p protocol --bin export-schemas
pnpm --dir app install --frozen-lockfile
pnpm --dir app generate:protocol
pnpm --dir app typecheck
pnpm --dir app build
cargo check --manifest-path app/src-tauri/Cargo.toml --locked
cargo fmt --manifest-path app/src-tauri/Cargo.toml --check
node --test scripts/seed.test.cjs
```

Re-exporting schemas and regenerating types must leave no diff. The desktop
crate has a separate lockfile and is excluded from the core workspace so core
checks do not require native GUI libraries. Build the web assets before checking
the desktop crate. Run the blank desktop window with `pnpm --dir app tauri dev`.
CLI examples: `cargo run -p astrolabe -- --version`, `cargo run -p astrolabe -- doctor`,
and `cargo run -p astrolabe -- init`.

The app version comes from `app/package.json`, also read by Tauri. Keep the root
workspace version aligned when releasing. Generated TypeScript is checked in at
`app/src/protocol/`; Rust types and exported JSON Schemas are the contract source.

## Private agent repo

Copy `seed/` (including dotfiles) to a separate private repository and fill its
placeholders there. Keep owner context, voice, and journals out of this public
code repository. Seed MCP commands are deliberately placeholders;
the seeded agent client server is disabled. `init` does not copy files yet.

Development uses a dev clone through the client's `--plugin-dir` or equivalent
override. Client-specific wiring and verification belong to phase 1a; phase 0
does not install a plugin or modify client configuration.

## Secrets

No secrets belong in this repository. Leak protection relies on the forge's
built-in secret scanning and push protection, not on a scanner of our own.
