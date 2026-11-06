# Astrolabe

A plugin for Claude Code and Codex that carries an owner's notes between sessions and machines. It delivers owner and voice files, loads effort context when asked, records work in a separate notes repo, and serves the owner's home on the machine: the board, the notes tree and each note as a page. Its shell hook guards against a fixed set of destructive commands.

The repo contains the [plugin](plugin/README.md) and the Rust program its hooks call. Personal notes and machine bindings stay outside this repo.

## What it's for

One place to run agents across an owner's repos, where their notes are the memory. The owner offloads what's in flight into durable notes and the note taking is automated: each piece of work is an effort with a state and a current state a fresh session can pick up on any machine, and a one-line journal says what changed. A board shows what's in flight, what's next and when the owner is needed.

It stays thin: context, hooks and guardrails around the models, nothing that scripts how a model works, no gates, no UI the apps already have. What it carries is what no model can learn on its own: the owner's preferences and voice, decisions and why, the state of each piece of work, facts about their machines. Rules carry their reason, so a dead one is easy to spot, and pain points are recorded so the setup improves from evidence.

It works the same with every supported agent app, so the owner can switch providers or pair them without changing how they work, and it never forks an app's tools. The owner stays in the loop as much as they choose: check-ins at logical points or a fully autonomous run, verification sized to the change, and nothing posted in their name.

## Build and check

Install the Rust toolchain pinned in `rust-toolchain.toml`, then run:

```sh
scripts/check
cargo build --release --locked -p astrolabe
```

`scripts/check` runs formatting, clippy with warnings denied, and every test suite. It uses Node.js for the Git landing tests. Run it before each commit. CI runs the same checks on macOS and Windows.

## Setup

See the [plugin instructions](plugin/README.md) for installation and the binary location the hooks expect. Make a new notes checkout with `notes init <folder>`, or register an existing one as `agent`, register target repos under the names in their notes, then switch the plugin on from a terminal. Installing a plugin writes the client's configuration; building this repo does not.

## Commands

- `hook <event> --if-active`: the plugin's hook entry point; does nothing when the machine switch is off.
- `on`, `off`, `status`: the machine switch, operated by the owner.
- `activate --effort <number> [--worker]`, `activate --status`, `activate --off`: load effort context for this session, inspect guard proof, or unload the effort.
- `effort new`, `effort save`, `journal`: record work through the plugin's pre-command hook.
- `efforts [--repo <name>] [--all]`: list what the home reads as JSON.
- `home [--open] [--serve] [--port <number>] [--theme auto|light|dark] [--palette ember|iris|slate]`: draw the file, make sure the server is up and print its address; `--serve` runs the server in the foreground.
- `notes init <folder>`: make and register a new owner's notes checkout, with effort 1, "Fill in your notes", active.
- `notes sync`, `notes status`: publish notes or inspect their sync state.
- `repo add <name> [<checkout>]`: register the notes checkout, record a target repo's remote in its note, or register a local repo without a remote.

The guard is an accident guard, not a sandbox. It sees shell commands supplied to the hook, not file edits or other tools. Each session proves its own hook with an allowed control and a denied probe; `activate --status` reports whether that proof holds.
