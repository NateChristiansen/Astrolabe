# Versions and compatibility

Rust 1.99.1, edition 2024, is pinned by `rust-toolchain.toml` and installed in CI. The crate requires Rust 1.99. Rust dependencies are recorded in the workspace's `Cargo.lock`; checks use `--locked`.

Prebuilt releases need Git and a supported agent app, but no Rust toolchain. Node.js and Python 3.13 are used for development checks and release preparation, not by the installed program or hooks. Normal CI checks macOS, Windows, and Linux.

The first prebuilt release is a candidate until its [release checks](releases.md#review-before-publication) pass. The release workflow builds and tests Apple silicon macOS 15, Intel macOS 15, x64 Windows Server 2022, and x64 Ubuntu 22.04. Linux requires glibc 2.35 or newer; Alpine/musl and other architectures are not included. These runner baselines are not a promise that every older or newer OS or app combination works. Test normal downloaded-file protections and a fresh agent session on the supported desktop systems before publication.

The release tool is cargo-dist 0.34.0, pinned in `Cargo.toml` and verified by hash when downloaded. The crate, plugin, and release tag share one version. Installers keep a compatibility copy named `astrolabe-trial`; new hooks call `astrolabe`.

## Agent apps

Install and sign in to Claude Code or Codex separately. Astrolabe does not install, pin or launch either app. Each app must load the events in the [hook manifest](../plugin/astrolabe/hooks/hooks.json), including pre-command interception that can update the shell tool input.

The [installation guide](../plugin/README.md#2-install-the-plugin-in-your-agent-app) uses each app's plugin CLI. Codex also requires review and trust of the installed hooks. A successful installation or machine `on` command doesn't prove the hooks run: complete the [first-session check](../plugin/README.md#5-start-your-first-session).

There is no established minimum app version. Installation command syntax was checked with Codex CLI 0.166.0 and Claude Code 2.1.316 on macOS; these are not minimum supported versions. See the [Codex plugin documentation](https://developers.openai.com/plugins/build/plugins) and [Claude Code plugin documentation](https://code.claude.com/docs/en/discover-plugins) for host behavior, and use the session check to verify your actual app and shell.

[Installation](../plugin/README.md) · [Contributing](../CONTRIBUTING.md)
