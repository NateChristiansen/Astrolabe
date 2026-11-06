# Contributing

## Build and check

Install Rust through rustup and Node.js. The repo selects its [pinned Rust toolchain](rust-toolchain.toml). Run from the repo root, using Bash or zsh on macOS/Linux and Git Bash on Windows:

```sh
scripts/check
cargo build --release --locked -p astrolabe
```

Run `scripts/check` before each commit. It runs formatting checks, clippy with warnings denied, all Rust tests, and the Git landing tests. CI runs the same checks on macOS and Windows.

Building alone does not install the plugin. Follow the [installation guide](plugin/README.md) to try your build in an agent app. After changing the program, rebuild and copy it to the installed location; after changing the plugin, refresh the app's installed copy and start a fresh session.

## Project documentation

- [Using Astrolabe](docs/usage.md): the owner's everyday workflow.
- [Architecture](docs/architecture.md): hooks, session context, notes, and the local board.
- [Command reference](docs/commands.md): terminal and agent-session commands.
- [Versions and compatibility](docs/versions.md): build pins and required app capabilities.
- [Icon and colors](assets/branding/README.md): the shared mark and its appearance.

Personal notes and machine bindings stay outside this repository.

When changing behavior, update the guide where a reader uses it. Keep setup steps in the installation guide, everyday tasks in the usage guide, and implementation details in the architecture guide.
