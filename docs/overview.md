# Overview

Three programs, one machine.

- **The app** is a Tauri window. It lists sessions, shows a plain chat and raises cards. It holds no state of its own.
- **The core** is `astrolabe serve`, a separate process that outlives the window. It owns the SQLite database and every running agent, and talks to windows over a local WebSocket with a secret made fresh at each start. If it dies a window restarts it, and sessions whose process went are marked interrupted. Quit ends the core within 30 seconds, whatever the sessions are doing. A session it hasn't reached by then may outlive it.
- **The agents** are the real clients, each started through its pinned ACP adapter, over pipes, with update checks off.

## What a launch does

1. Finds the repo and syncs it, carrying on from your local copy if that fails.
2. Makes a worktree, unless this is your agent repo, which runs in place.
3. Composes the layer: instructions, hooks, tool server, deny rules, native memory off. On your agent repo it adds nothing the repo already supplies.
4. Asks the guard to deny a force push and pass a plain command. If it doesn't, nothing starts.
5. Waits for the session-start hook's receipt, up to 30 seconds. With no receipt the session is ended and the window says why. Codex only fires that hook after the first prompt, so its first turn can run during the wait.

## The guard

`astrolabe hook pre-command` reads each command before it runs and denies a fixed list of destructive ones, with one layer of wrapping seen through. Denials go in `guard-denials.log` in the data folder. It stops a careless command, not a determined one: the known limits are the rows in `crates/astrolabe/tests/fixtures/guard-bypass.tsv`.

## The agent repo

A private repo of your own, made by `astrolabe init`, holding plain Markdown: who you are, how you write, notes. The session-start hook reads the owner and voice files fresh for each session, so there's no copy to drift. A plain client in that repo gets the same layer from the repo's own config once you've trusted it. Nothing private goes in this public repo: the seed has placeholders only.

## Where things live

- Data folder: `astrolabe where`. `ASTROLABE_DATA_DIR` moves it.
- Adapters: `adapters` in the data folder. `ASTROLABE_ADAPTERS` moves them.
- User config is written in one place only: `doctor apply` adds a marked block to Codex's config, after showing it and asking. `doctor remove` takes it out again.

## Limits today

- One machine. Reaching work on another is future design.
- A plain client doesn't show in the app until it has reported in, and only as a row: the app can't talk to it.
- A row, card or quit is shown to windows that are connected. A window not reading at the moment of a quit can start a new core.
- A stuck session takes 2 to 4 seconds to stop, so past 7 to 14 of them the 30-second bound cuts the rest off and their worktrees stay.
