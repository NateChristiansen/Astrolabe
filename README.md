# Astrolabe

A plugin for Claude Code and Codex that carries an owner's notes between sessions and machines. It delivers owner and voice files, loads effort context when asked, records work in a separate notes repo, and draws a local board. Its shell hook guards against a fixed set of destructive commands.

The repo contains the [plugin](plugin/README.md) and the Rust program its hooks call. Personal notes and machine bindings stay outside this repo.

## Build and check

Install the Rust toolchain pinned in `rust-toolchain.toml`, then run:

```sh
scripts/check
cargo build --release --locked -p astrolabe
```

`scripts/check` runs formatting, clippy with warnings denied, and every test suite. It uses Node.js for the Git landing tests. Run it before each commit. CI runs the same checks on macOS and Windows.

## Setup

See the [plugin instructions](plugin/README.md) for installation and the binary location the hooks expect. Register an existing notes checkout as `agent`, register target repos under the names in their notes, then switch the plugin on from a terminal. Installing a plugin writes the client's configuration; building this repo does not.

## Commands

- `hook <event> --if-active`: the plugin's hook entry point; does nothing when the machine switch is off.
- `on`, `off`, `status`: the machine switch, operated by the owner.
- `activate --effort <number> [--worker]`, `activate --status`, `activate --off`: load effort context for this session, inspect guard proof, or unload the effort.
- `effort new`, `effort save`, `journal`: record work through the plugin's pre-command hook.
- `efforts [--repo <name>] [--all]`: list what the board reads as JSON.
- `home [--open] [--serve [--port <number>]] [--theme auto|light|dark]`: draw or serve the board.
- `notes sync`, `notes status`: publish notes or inspect their sync state.
- `repo add <name> [<checkout>]`: register the notes checkout, record a target repo's remote in its note, or register a local repo without a remote.

The guard is an accident guard, not a sandbox. It sees shell commands supplied to the hook, not file edits or other tools. Each session proves its own hook with an allowed control and a denied probe; `activate --status` reports whether that proof holds.
