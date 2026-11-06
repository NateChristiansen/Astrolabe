<p align="center">
  <img src="assets/branding/astrolabe.svg" width="112" height="112" alt="Astrolabe symbol">
</p>

<h1 align="center">Astrolabe</h1>

<p align="center">Your agents can pick up where you left off—even on another machine.</p>

Astrolabe is a plugin that gives your agents a shared set of notes: your preferences, decisions, and work in progress. The notes live in your own Git repo, so they can travel with you between sessions, repos, and supported agent apps.

## How it fits into your day

Ask an agent to work on something. Astrolabe gives that work a note called an **effort**, where the agent records what changed, what was decided, and what's next.

Later, ask an agent to “pick up 12,” using that effort's number. It gets the saved state and the context for that repo and machine. You can continue in a fresh session without reconstructing the last conversation.

- **Your preferences travel with the work.** Agents receive your working preferences and writing voice, with repo and machine notes when you load an effort.
- **You choose when to check in.** Each effort tells agents whether to work autonomously, check the plan with you, or pause at each step.
- **You can see where things stand.** A local board shows active work, what needs you, and a short journal of what changed. You can browse the notes alongside it.

The notes are plain Markdown, separate from your project code. You can read and edit them yourself.

## Get started

Astrolabe currently builds from source. Start with the [per-machine setup](plugin/README.md#setup-once-per-machine), then [install the plugin in your agent app](plugin/README.md#install).

Setup creates a notes repo and a first effort, “Fill in your notes,” to help you add your preferences and register a project. Already have Astrolabe notes? Register that checkout on the new machine instead.

Once you're set up, open the board with `astrolabe-trial home --open`.

## The command guard

Astrolabe's shell hook blocks a fixed set of destructive commands and checks that the hook works in each session. It is an accident guard, not a sandbox: it cannot see file edits or other tools. Effort settings guide agents; they aren't enforced permissions.

## Develop

Install the [pinned Rust toolchain](rust-toolchain.toml) and Node.js, then run:

```sh
scripts/check
cargo build --release --locked -p astrolabe
```

Run `scripts/check` before each commit. It checks formatting, lints, and all tests; CI runs the same checks on macOS and Windows. Building alone does not install the plugin.

[Command reference](docs/commands.md) · [How the plugin works](plugin/README.md) · [Icon and colors](assets/branding/README.md)
