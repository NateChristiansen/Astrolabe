# Command reference

For a walkthrough, see [installation](../plugin/README.md) or [Using Astrolabe](usage.md).

## Run terminal commands

Terminal entries below show the arguments to pass to the installed program. For example, to check machine status:

macOS / Linux:

```sh
"$HOME/.local/bin/astrolabe-trial" status
```

Windows (PowerShell):

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe-trial.exe" status
```

Replace `status` with the arguments you need below. The installed name is `astrolabe-trial` because that's what the hooks currently expect; using its full path avoids depending on PATH. Agent-session commands use `astrolabe` and are handled by the plugin instead.

## Machine and repo setup

Run these through the installed program in your own terminal:

- `--version`: print the program version.
- `on`, `off`, `status`: enable Astrolabe for this machine, disable it, or inspect the switch. Turning it off doesn't erase instructions already delivered to a session.
- `notes init <folder>`: create and register notes, including the first effort, **Fill in your notes**. The folder must be new, empty, or a fresh clone of an empty repo. The machine must not already have registered notes.
- `repo add agent <checkout>`: register existing notes on this machine, replacing the previous registration if there is one.
- `repo add <name> [<checkout>]`: register a project. Omit the checkout to use the current folder. A project with a remote gets that remote recorded in its shared repo note; a project without one gets a local binding. Changes to repo notes are left for you to commit.

## Board and notes

These also run through the installed program in your terminal:

- `home --open`: ensure the local server is running and open the board.
- `home`: write the standalone `home.html`, ensure the server is running, and print its address.
- `home --serve`: run the server in the foreground.
- `home --port <number>`: set the server's port on this machine.
- `home --theme auto|light|dark`: set the appearance on this machine.
- `home --palette iris|ember|slate`: set the palette. Iris is the default.
- `efforts [--repo <name>] [--all]`: list the data the board reads as JSON.
- `notes status`: inspect notes sync state.
- `notes sync`: retry synchronization with the notes remote.

Combine `home` options as needed. For example, on macOS/Linux:

```sh
"$HOME/.local/bin/astrolabe-trial" home --open --theme dark --palette iris
```

A browser's saved appearance choice takes precedence in that browser. See [sharing notes between machines](usage.md#share-notes-between-machines) for the sync workflow.

## Work in an agent session

Normally, ask the agent to start work or **pick up 12**. It runs these commands for you:

- `astrolabe activate --effort <number> [--worker]`: load an effort's context. Workers leave notes reporting to the agent that dispatched them.
- `astrolabe activate --status`: inspect the loaded effort and this session's guard proof.
- `astrolabe activate --off`: unload the effort. The machine's guard stays on.
- `astrolabe effort new --title "<title>"`: create an effort, with its goal in the command's input. From the notes folder, include `--repo <name>`.
- `astrolabe effort save`: save the current state, with the new text in the command's input.
- `astrolabe journal "<sentence>"`: record a finding worth keeping without creating an effort. From the notes folder, include `--repo <name>`.

The plugin supplies the agent with the full recording format, including plans, decisions, checklists and journal updates. These commands must run alone so the pre-command hook can handle them. Running the program directly doesn't load or save an effort. See [recording internals](internals.md#recording-work).

## Hook entry point

The plugin runs the installed program with `hook <event> --if-active`. It does nothing while the machine switch is off.

The guard checks shell commands supplied to the hook; it can't see file edits or other tools. Each session proves its own hook with an allowed control and a denied probe. Effort settings guide agents and aren't enforced permissions. See [guard details](internals.md#the-shell-guard).

[Installation](../plugin/README.md) · [Troubleshooting](troubleshooting.md) · [Using Astrolabe](usage.md) · [Project overview](../README.md)
