# Contributing

## Build and check

Install the [pinned Rust toolchain](rust-toolchain.toml) and Node.js, then run from the repo root:

```sh
scripts/check
cargo build --release --locked -p astrolabe
```

Run `scripts/check` before each commit. It runs formatting checks, clippy with warnings denied, all Rust tests, and the Git landing tests. CI runs the same checks on macOS and Windows.

Building alone does not install the plugin. See the [per-machine setup](plugin/README.md#setup-once-per-machine) and [plugin installation](plugin/README.md#install) to try your build in an agent app.

## Project documentation

- [Plugin internals](plugin/README.md): hooks, session context, notes, and the local board.
- [Command reference](docs/commands.md): terminal and agent-session commands.
- [Icon and colors](assets/branding/README.md): the shared mark and its appearance.

Personal notes and machine bindings stay outside this repository.
