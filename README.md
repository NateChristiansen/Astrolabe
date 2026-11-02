# Astrolabe

A desktop app that runs Claude Code and Codex sessions in your repos, each in its own git worktree, with the same safety layer on both. MIT licensed.

You pick a repo and an agent, then chat in the window. Astrolabe talks to the clients through their ACP adapters. It doesn't replace them: you install the clients and they keep their own logins.

## What works today

- Sessions on either client from the window: chat, tool lines, Stop, Quit. A session keeps running with the window closed.
- A git worktree per session on a product repo. It is removed at the end only if nothing in it would be lost.
- A guard that blocks commands like `git push --force`, `git reset --hard` and `pkill` before they run. It catches accidents. It is not a sandbox.
- A launch that fails loudly if the guard fails its self-test or the session-start hook doesn't report. On Claude Code that doesn't prove the pre-command hook is in force.
- Approval cards: Allow, Reject, or Reject and say what instead.
- A "may be stuck" card when a tool runs past 10 minutes.
- `astrolabe init`: a private agent repo for your own notes, whose owner and voice files reach each session.
- A plain client started in that repo gets the same layer once you trust it, and shows in the app as a row.

macOS and Windows. More in [the overview](docs/overview.md).

## Install

There is no release yet, and builds are unsigned. Build from source, with Rust 1.99.1, Node 26, pnpm 11.0.9 and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/). Install and log in to Claude Code and Codex yourself; [the versions tested](docs/versions.md) are listed.

macOS, from the repo root:

```bash
pnpm --dir app install --frozen-lockfile
cargo build --release --locked -p astrolabe
mkdir -p app/src-tauri/binaries
cp target/release/astrolabe "app/src-tauri/binaries/astrolabe-$(rustc -vV | sed -n 's/^host: //p')"
pnpm --dir app tauri build --config '{"bundle":{"active":true,"targets":["app"],"externalBin":["binaries/astrolabe"]}}'
```

Windows is the same with `.exe` on both file names and `"targets":["nsis"]`, run from Git Bash.

Then, using the `astrolabe` beside the app (on macOS, in `Astrolabe.app/Contents/MacOS`):

```bash
astrolabe doctor adapters        # downloads the pinned adapters; asks first
astrolabe doctor inspect         # what it found; writes nothing
astrolabe repo add myrepo /path/to/repo
```

Open the app and start a session.

For an agent repo, run `astrolabe init`, then `astrolabe doctor install`, which copies `astrolabe` to `.local/bin` in your home. That folder has to be on your clients' PATH, as the repo's hooks call `astrolabe` by name. Then `astrolabe doctor apply` shows you the lines it adds to Codex's config and asks first. Claude's folder trust stays its own dialog.

On Windows, run setup from a plain terminal tab, not from a shell inside a packaged app, which can write to a private copy of your app data.

## What's not done

- Signed builds, installers you can trust at first sight, and updates.
- Reaching work on another machine.
- Tools for agents beyond a single layer check.
- A dials control: every session gets the same defaults.
- Owner and voice text in child sessions on Claude Code.

## Develop

`cargo test`, and [CI](.github/workflows/system.yml) has the full list of checks. [Test control](docs/test-control.md) drives a debug build from an agent. See also [size](docs/size.md), [decisions](docs/decisions.md) and [the protocol contract](crates/protocol/README.md).

Your notes belong in your agent repo, never here. No secrets belong in this repo.
