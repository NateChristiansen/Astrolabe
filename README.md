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

Follow the [installation guide](plugin/README.md) to build Astrolabe, install it in your agent app, and check your first session.

Setup creates a notes repo and a first effort, “Fill in your notes,” to help you add your preferences and register a project. Already have Astrolabe notes? Register that checkout on the new machine instead.

Then read [Using Astrolabe](docs/usage.md) for starting work, choosing when agents check in, and picking up where you left off.

[Command reference](docs/commands.md) · [How it works](docs/architecture.md) · [Contributing](CONTRIBUTING.md)
