# Install Astrolabe

Astrolabe needs two things on each machine: a small program built from this repo, and a plugin installed in your agent app. This guide takes you through both, then starts your first session.

Already installed? See [Using Astrolabe](../docs/usage.md), [updating](#update-astrolabe), or [troubleshooting](../docs/troubleshooting.md).

## Before you start

- Install [Git](https://git-scm.com/downloads) and configure a name and email for commits. Astrolabe commits changes to your notes.
- Install [Rust through rustup](https://rustup.rs/), including the build tools it requests for your operating system. The repo selects its [pinned toolchain](../rust-toolchain.toml) when you build.
- Install and sign in to Claude Code or Codex, with its command available in your terminal. Astrolabe requires plugin hooks; see [compatibility](../docs/versions.md).

Run the commands below in your own terminal. Use Bash or zsh on macOS/Linux, or PowerShell on Windows. You don't need Node.js just to install Astrolabe; it's used by the development checks.

## 1. Build and install the program

From the folder where you keep source code:

```sh
git clone https://github.com/NateChristiansen/Astrolabe.git
cd Astrolabe
cargo build --release --locked -p astrolabe
```

The build produces `astrolabe`, but the plugin currently expects it to be installed as **`astrolabe-trial`**. Copy it to that location using the commands for your system.

### macOS / Linux

```sh
mkdir -p "$HOME/.local/bin"
cp target/release/astrolabe "$HOME/.local/bin/astrolabe-trial"
"$HOME/.local/bin/astrolabe-trial" --version
```

### Windows (PowerShell)

```powershell
New-Item -ItemType Directory -Force "$env:USERPROFILE/.local/bin" | Out-Null
Copy-Item target/release/astrolabe.exe "$env:USERPROFILE/.local/bin/astrolabe-trial.exe"
& "$env:USERPROFILE/.local/bin/astrolabe-trial.exe" --version
```

The last command should print an Astrolabe version. These examples use the full path, so you don't need to change PATH or create an alias. Keep this source checkout: the local plugin installation below refers to it.

## 2. Install the plugin in your agent app

Stay in the **Astrolabe repo root**. Choose the app you use, or install in both. `./plugin` is the marketplace folder in this repo.

### Claude Code

```sh
claude plugin marketplace add ./plugin
claude plugin install astrolabe@astrolabe
```

This installs at user scope. See the [Claude Code plugin guide](https://code.claude.com/docs/en/discover-plugins) for app-specific installation options.

### Codex

```sh
codex plugin marketplace add ./plugin
codex plugin add astrolabe@astrolabe
```

Review and trust Astrolabe's five hooks. In the CLI, open `/hooks`; in the desktop app, use **Review hooks** in the composer or **Settings → Hooks**. Installing the plugin alone does not trust its hooks. See the [Codex hook guide](https://learn.chatgpt.com/docs/hooks#plugin-bundled-hooks).

## 3. Set up your notes

Your notes live in a separate Git repo. Choose **new notes** for your first machine or **existing notes** if you already use Astrolabe.

### New notes

Choose a new or empty folder outside the Astrolabe source checkout and your project repos. For example:

macOS / Linux:

```sh
"$HOME/.local/bin/astrolabe-trial" notes init "$HOME/Astrolabe-notes"
```

Windows:

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe-trial.exe" notes init "$env:USERPROFILE/Astrolabe-notes"
```

This creates and registers your notes, makes their first commit, and adds an effort called **Fill in your notes**. An effort is a note about a piece of work: where it stands, what was decided, and what comes next.

Your notes stay on this machine until you [connect a private remote](../docs/usage.md#use-another-machine). You can do that after your first session. A fresh clone of an empty private Git repo also works as the folder for `notes init`.

### Existing notes

Clone your private notes repo onto this machine first, or use a checkout you already have. Register its location; don't run `notes init` on it.

macOS / Linux:

```sh
"$HOME/.local/bin/astrolabe-trial" repo add agent "$HOME/Astrolabe-notes"
```

Windows:

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe-trial.exe" repo add agent "$env:USERPROFILE/Astrolabe-notes"
```

Replace the example folder with yours. `agent` is Astrolabe's reserved name for the notes repo.

## 4. Turn it on

macOS / Linux:

```sh
"$HOME/.local/bin/astrolabe-trial" on
```

Windows:

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe-trial.exe" on
```

The reply should say **Astrolabe is on for this machine**. This enables Astrolabe across sessions on this machine; you don't turn it on separately for each project. The app still needs to load and run its hooks.

## 5. Start your first session

Restart your agent app, then open a new session in **your notes folder**.

For new notes, say:

> Pick up 1.

The agent should load **Fill in your notes** and walk you through your preferences, writing voice, machine details, and first project. It should also check that the shell guard runs and report the result. Let the agent handle those checks.

For existing notes, ask the agent to pick up an effort by its number. The [board](#6-open-the-board) shows your saved work.

You're ready when the agent can name the loaded effort, read its saved state, and report that the guard is proven for the session. If it can't, use [troubleshooting](../docs/troubleshooting.md) before relying on it to record work.

## 6. Open the board

macOS / Linux:

```sh
"$HOME/.local/bin/astrolabe-trial" home --open
```

Windows:

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe-trial.exe" home --open
```

Your browser opens the local board, normally at [localhost:4747](http://localhost:4747/). It shows your efforts, journal and notes. It starts automatically when needed.

Continue with [Using Astrolabe](../docs/usage.md) to start work in a project, choose when agents check in, and resume on another machine.

## Update Astrolabe

Update both the program and the plugin:

1. Finish or pause your current agent work. In your Astrolabe source checkout, run `git pull --ff-only`.
2. Repeat the build and copy commands in [step 1](#1-build-and-install-the-program).
3. Refresh the plugin in each app you use:

   ```sh
   # Claude Code
   claude plugin update astrolabe@astrolabe

   # Codex
   codex plugin add astrolabe@astrolabe
   ```

4. Review and trust any changed hooks in Codex, then restart your agent app and repeat the [session check](#5-start-your-first-session).

Existing sessions keep instructions they already received. A fresh session gets the new ones. Updating the plugin alone does not replace the program you copied earlier.

If Windows reports that the executable is in use, close your agent sessions, stop Astrolabe's board process (`astrolabe-trial.exe`) in Task Manager, and retry the copy.

## Turn off or remove Astrolabe

To stop its hooks doing work, run the installed program with `off` in your terminal:

```sh
"$HOME/.local/bin/astrolabe-trial" off
```

On Windows:

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe-trial.exe" off
```

To uninstall from an app, run its pair of commands:

```sh
# Claude Code
claude plugin uninstall astrolabe@astrolabe
claude plugin marketplace remove astrolabe

# Codex
codex plugin remove astrolabe@astrolabe
codex plugin marketplace remove astrolabe
```

Uninstalling leaves your notes, the installed program, and [local machine state](../docs/internals.md#files-and-local-state) in place. Turning it off doesn't erase context already delivered to a session.

[Using Astrolabe](../docs/usage.md) · [Command reference](../docs/commands.md) · [Troubleshooting](../docs/troubleshooting.md) · [Internals](../docs/internals.md) · [Project overview](../README.md)
