# Contributing

## Build and check

Install Rust through rustup, Node.js, and Python 3.13 or newer. The repo selects its [pinned Rust toolchain](rust-toolchain.toml). Run from the repo root, using Bash or zsh on macOS/Linux and Git Bash on Windows:

```sh
scripts/check
cargo build --release --locked -p astrolabe
```

Run `scripts/check` before each commit. It runs formatting checks, clippy with warnings denied, all Rust tests, the Git landing tests, and release packaging checks. CI runs the same checks on macOS, Windows, and Linux.

Building alone does not install the plugin. Follow the [installation guide](docs/install.md) to try your build in an agent app. After changing the program, rebuild and run the new executable with `install`; after changing the plugin, refresh the app's installed copy and start a fresh session.

To see a home change without replacing the one you use, run `target/debug/astrolabe home --preview --port 4799`. It serves your notes read-only from that build, beside the installed home, and saves nothing: `--theme` and `--palette` apply to the preview alone, and it leaves the machine's port and server as they are. Without `--port` it picks a free one and prints the address. It runs until you stop it, so restart it after a rebuild.

## Project documentation

- [Installation](docs/install.md): setup, updates, and migration.
- [Using Astrolabe](docs/usage.md): the owner's everyday workflow.
- [Architecture](docs/architecture.md): hooks, session context, notes, and the local board.
- [Command reference](docs/commands.md): terminal and agent-session commands.
- [Versions and compatibility](docs/versions.md): build pins and required app capabilities.
- [Host app behavior](docs/hosts.md): what each agent app does at the points the program depends on, with sources.
- [Icon and colors](assets/branding/README.md): the shared mark and its appearance.

Personal notes and machine bindings stay outside this repository.

When changing behavior, update the guide where a reader uses it. Keep setup steps in the installation guide, everyday tasks in the usage guide, and implementation details in the architecture guide.

## Releases

See [Preparing a release](docs/releases.md) for artifact generation, migration tests, and the review required before publication. The release workflow's build jobs only upload CI artifacts. Its `publish-nightly` job replaces the rolling [nightly prerelease](docs/releases.md#nightly) on each push to `main`; versioned releases are still published by hand after review.
