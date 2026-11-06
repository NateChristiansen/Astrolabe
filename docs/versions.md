# Versions and compatibility

Rust 1.99.1, edition 2024, is pinned by `rust-toolchain.toml` and installed in CI. The crate requires Rust 1.99. Rust dependencies are recorded in the workspace's `Cargo.lock`; checks use `--locked`.

Astrolabe builds from source. Node.js is needed for the development checks, but not for the program or its hooks. CI checks macOS and Windows; Linux is not currently covered by CI.

## Agent apps

Install and sign in to Claude Code or Codex separately. Astrolabe does not install, pin or launch either app. Each app must load the events in the [hook manifest](../plugin/astrolabe/hooks/hooks.json), including pre-command interception that can update the shell tool input.

The [installation guide](../plugin/README.md#2-install-the-plugin-in-your-agent-app) uses each app's plugin CLI. Codex also requires review and trust of the installed hooks. A successful installation or machine `on` command doesn't prove the hooks run: complete the [first-session check](../plugin/README.md#5-start-your-first-session).

There is no established minimum app version. Installation command syntax was checked with Codex CLI 0.166.0 and Claude Code 2.1.316 on macOS; these are not minimum supported versions. See the [Codex plugin documentation](https://developers.openai.com/plugins/build/plugins) and [Claude Code plugin documentation](https://code.claude.com/docs/en/discover-plugins) for host behavior, and use the session check to verify your actual app and shell.

[Installation](../plugin/README.md) · [Contributing](../CONTRIBUTING.md)
