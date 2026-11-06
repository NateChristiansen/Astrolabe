# Install Astrolabe

Astrolabe needs two things on each machine: a small program downloaded for your system, and a plugin installed in your agent app. This guide takes you through both, then starts your first session.

Already installed? See [Using Astrolabe](../docs/usage.md), [updating](#update-astrolabe), or [troubleshooting](../docs/troubleshooting.md).

## Before you start

- Install [Git](https://git-scm.com/downloads) and configure a name and email for commits. Astrolabe commits changes to your notes.
- Install and sign in to Claude Code or Codex, with its command available in your terminal. Astrolabe requires plugin hooks; see [compatibility](../docs/versions.md).

Run the commands below in your own terminal. Use Bash or zsh on macOS/Linux, or PowerShell on Windows. You don't need Rust, Node.js, Python, or a source checkout to install a prebuilt release. Git is still needed for your notes and the plugin marketplace.

## 1. Download and install the program

**Release candidate:** prebuilt downloads are being prepared. Until `v0.5.0` appears on the [releases page](https://github.com/NateChristiansen/Astrolabe/releases), use [Build from source](#build-from-source). The commands below describe that release, not a download that is already available.

The first release targets Apple silicon and Intel Macs, x64 Windows, and x64 Linux with glibc. See [versions and compatibility](../docs/versions.md) for the tested systems. Install into your own account; administrator access is not needed.

### macOS / Linux

```sh
curl --fail --location --output install.sh https://github.com/NateChristiansen/Astrolabe/releases/download/v0.5.0/install.sh
sh install.sh
"$HOME/.local/bin/astrolabe" --version
```

### Windows (PowerShell)

```powershell
Invoke-WebRequest https://github.com/NateChristiansen/Astrolabe/releases/download/v0.5.0/install.ps1 -OutFile install.ps1
& .\install.ps1
& "$env:USERPROFILE/.local/bin/astrolabe.exe" --version
```

You can read the downloaded script before running it. It selects your system's archive, verifies its SHA-256 checksum before unpacking, and installs `astrolabe` in your user-local bin folder. It also installs the same build as `astrolabe-trial` so older cached plugins keep working during migration. It does not change PATH, your notes, or your agent app's settings.

If an older executable is already there, follow [Migrate an existing installation](#migrate-an-existing-installation). A failed download or unsupported system leaves your existing installation in place.

## 2. Install the plugin in your agent app

Choose the app you use, or install in both. These commands let the app fetch the plugin from the hosted marketplace; you don't clone the source yourself. The marketplace is pinned to the same release as the program.

### Claude Code

```sh
claude plugin marketplace add https://github.com/NateChristiansen/Astrolabe.git#v0.5.0
claude plugin install astrolabe@astrolabe
```

This installs at user scope. See the [Claude Code plugin guide](https://code.claude.com/docs/en/discover-plugins) for app-specific installation options.

### Codex

```sh
codex plugin marketplace add https://github.com/NateChristiansen/Astrolabe.git --ref v0.5.0
codex plugin add astrolabe@astrolabe
```

Review and trust Astrolabe's five hooks. In the CLI, open `/hooks`; in the desktop app, use **Review hooks** in the composer or **Settings → Hooks**. Installing the plugin alone does not trust its hooks. See the [Codex hook guide](https://learn.chatgpt.com/docs/hooks#plugin-bundled-hooks).

Already registered an Astrolabe marketplace from a local folder or older release? Follow [updating](#update-astrolabe) to replace that source first.

## 3. Set up your notes

Your notes live in a separate Git repo. Choose **new notes** for your first machine or **existing notes** if you already use Astrolabe.

### New notes

Choose a new or empty folder outside the Astrolabe source checkout and your project repos. For example:

macOS / Linux:

```sh
"$HOME/.local/bin/astrolabe" notes init "$HOME/Astrolabe-notes"
```

Windows:

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe.exe" notes init "$env:USERPROFILE/Astrolabe-notes"
```

This creates and registers your notes, makes their first commit, and adds an effort called **Fill in your notes**. An effort is a note about a piece of work: where it stands, what was decided, and what comes next.

Your notes stay on this machine until you [connect a private remote](../docs/usage.md#use-another-machine). You can do that after your first session. A fresh clone of an empty private Git repo also works as the folder for `notes init`.

### Existing notes

Clone your private notes repo onto this machine first, or use a checkout you already have. Register its location; don't run `notes init` on it.

macOS / Linux:

```sh
"$HOME/.local/bin/astrolabe" repo add agent "$HOME/Astrolabe-notes"
```

Windows:

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe.exe" repo add agent "$env:USERPROFILE/Astrolabe-notes"
```

Replace the example folder with yours. `agent` is Astrolabe's reserved name for the notes repo.

## 4. Turn it on

macOS / Linux:

```sh
"$HOME/.local/bin/astrolabe" on
```

Windows:

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe.exe" on
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
"$HOME/.local/bin/astrolabe" home --open
```

Windows:

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe.exe" home --open
```

Your browser opens the local board, normally at [localhost:4747](http://localhost:4747/). It shows your efforts, journal and notes. It starts automatically when needed.

Continue with [Using Astrolabe](../docs/usage.md) to start work in a project, choose when agents check in, and resume on another machine.

## Update Astrolabe

Update the program and plugin together. Existing sessions keep instructions they already received, so finish or pause current work and close your agent sessions before updating.

1. Download the new release's installer as in [step 1](#1-download-and-install-the-program) and run it. Use the same release tag for both downloads and the marketplace. Each successful update keeps the previous program files in the backup folder printed by the installer.
2. Remove the old plugin and marketplace registration in each app you use:

   ```sh
   # Claude Code
   claude plugin uninstall astrolabe@astrolabe
   claude plugin marketplace remove astrolabe

   # Codex
   codex plugin remove astrolabe@astrolabe
   codex plugin marketplace remove astrolabe
   ```

3. Repeat [step 2](#2-install-the-plugin-in-your-agent-app) using the new release tag. This also moves an existing local marketplace to the hosted source. Removing the old registration does not erase your notes.
4. Review and trust any changed hooks, restart your app, and repeat the [session check](#5-start-your-first-session).

Marketplace refresh alone does not update the program. A pinned marketplace also stays on its selected release until you change the registration. Astrolabe's installers do not enable background updates.

If Windows reports a file in use, close the named Astrolabe sessions and stop its board process in Task Manager, then retry. The installer does not kill processes. See [troubleshooting](../docs/troubleshooting.md) for conflicting or interrupted installations.

To roll back, run the previous release's installer and register that release's marketplace again. For the first migration from a source build, keep the printed backup folder and your old source checkout until the new session check passes. [Recovery instructions](../docs/releases.md#recover-an-interrupted-installation) explain how to restore those original files.

## Migrate an existing installation

Older versions used `astrolabe-trial`. Close your agent sessions and board before migrating. Download the installer in step 1, then explicitly allow it to adopt your old Astrolabe build:

```sh
sh install.sh --migrate-trial
```

On Windows:

```powershell
& .\install.ps1 -MigrateTrial
```

Migration checks any existing `astrolabe` and `astrolabe-trial` programs, backs them up, and installs both names using the new build. An unrelated executable, symlink, or a file changed since the last managed installation is refused; move the conflicting file aside yourself after checking what it is. Notes, repo bindings, and the machine's on/off switch stay in place. Finish the plugin steps under [updating](#update-astrolabe); don't run `notes init` again.

## Build from source

This is the development path and the fallback while prebuilt releases are not yet published. Install [Rust through rustup](https://rustup.rs/) and the build tools it requests, then:

```sh
git clone https://github.com/NateChristiansen/Astrolabe.git
cd Astrolabe
cargo build --release --locked -p astrolabe
./target/release/astrolabe install
```

On Windows, the last command is `& .\target\release\astrolabe.exe install`. Add `--migrate-trial` for an existing unmanaged Astrolabe installation.

Register `./plugin` instead of the hosted URL in step 2. Keep this checkout because your local marketplace refers to it. For later source updates, rebuild and run `install` again, then refresh the app's local plugin. Don't use a published plugin with an unrelated development build.

## Turn off or remove Astrolabe

To stop its hooks doing work, run the installed program with `off` in your terminal:

```sh
"$HOME/.local/bin/astrolabe" off
```

On Windows:

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe.exe" off
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
