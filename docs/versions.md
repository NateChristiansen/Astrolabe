# Version pins

| Tool | Baseline | Enforcement |
| --- | --- | --- |
| Rust | 1.99.1, edition 2024 | rust-toolchain.toml; crate rust-version 1.99 |
| Node | 26.x | package engines and CI |
| pnpm | 11 (11.0.9) | app/package.json and CI |
| Tauri Rust | 2.12.5 | exact Cargo dependency and desktop lockfile |
| tauri-build | 2.7.4 | exact build dependency and desktop lockfile |
| Tauri CLI | 2.12.4 | exact package dependency and pnpm lockfile |
| @agentclientprotocol/claude-agent-acp | 0.99.1 | adapters/package.json and lockfile |
| @agentclientprotocol/codex-acp | 2.5.0 | adapters/package.json and lockfile |
| Claude Code | 2.1.314 | user-installed; doctor reports it (step 5) |
| codex-cli | 0.166.0 | user-installed; doctor reports it (step 5) |

Client update checks stay off in everything Astrolabe launches.

Rust and JavaScript transitive versions are recorded in their checked-in lockfiles.
The blank app does not need the Tauri JavaScript API package.
