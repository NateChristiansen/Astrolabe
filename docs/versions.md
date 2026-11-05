# Version pins

Rust 1.99.1, edition 2024, is pinned by `rust-toolchain.toml` and installed in CI. The crate requires Rust 1.99. Rust dependencies are recorded in the workspace's `Cargo.lock`; checks use `--locked`.

Claude Code and Codex are installed and authenticated by the owner. The plugin does not install, pin or launch either client. Each client must support the plugin hooks in [the plugin manifest](../plugin/astrolabe/hooks/hooks.json).
