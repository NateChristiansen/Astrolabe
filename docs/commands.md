# Command reference

For installation, start with the [setup instructions](../plugin/README.md#setup-once-per-machine).

The build produces `astrolabe`, but the current setup installs it as `astrolabe-trial` (`astrolabe-trial.exe` on Windows), the filename the hooks expect. The terminal examples below use that installed name. Inside an agent session, the plugin intercepts `astrolabe activate`, `astrolabe effort`, and `astrolabe journal` commands to load context and record work.

## Machine and repo setup

Run these in a terminal:

- `astrolabe-trial on`, `off`, `status`: turn the plugin on or off for this machine, or inspect its state.
- `astrolabe-trial notes init <folder>`: create and register a notes checkout, with the first effort, “Fill in your notes.” The folder must be new or empty, and the machine must not already have a registered notes checkout.
- `astrolabe-trial repo add agent <checkout>`: register an existing notes checkout on this machine.
- `astrolabe-trial repo add <name> [<checkout>]`: register a target repo under the name used in its repo note. Omit the checkout to use the current folder. A repo with a remote gets that remote recorded in its note; a repo without one gets a local binding.

## Work in an agent session

Normally, ask the agent to start work or “pick up 12.” It runs these commands for you:

- `astrolabe activate --effort <number> [--worker]`: load an effort's context for this session. Workers get the context but leave reporting to the agent that dispatched them.
- `astrolabe activate --status`: inspect the current effort and this session's guard proof.
- `astrolabe activate --off`: unload the effort. The machine's guard stays on.
- `astrolabe effort new --title "<title>"`: create an effort, with its goal in the command's input. From the notes checkout, include `--repo <name>`.
- `astrolabe effort save`: save the current state, with the new text in the command's input.
- `astrolabe journal "<sentence>"`: record a finding worth keeping without creating an effort. From the notes checkout, include `--repo <name>`.

The plugin supplies the agent with the full recording instructions. These commands depend on its pre-command hook; running the program directly does not record the work. See [how notes get written](../plugin/README.md#how-notes-get-written).

## Board and notes

- `astrolabe-trial home --open`: start the local server if needed and open the board in your browser.
- `astrolabe-trial home`: update the standalone `home.html`, ensure the server is running, and print its address.
- `astrolabe-trial home --serve`: run the server in the foreground.
- `astrolabe-trial home --port <number>`: set the server's port on this machine.
- `astrolabe-trial home --theme auto|light|dark`: set the appearance on this machine.
- `astrolabe-trial home --palette iris|ember|slate`: set the palette. Iris is the default. The [shared icon](../assets/branding/README.md) follows the board's appearance and palette.
- `astrolabe-trial efforts [--repo <name>] [--all]`: list the data the board reads as JSON.
- `astrolabe-trial notes status`: inspect notes sync state.
- `astrolabe-trial notes sync`: sync the notes with their remote.

The `home` options can be combined, for example `astrolabe-trial home --open --theme dark --palette iris`. A browser's saved appearance choice takes precedence in that browser.

## Hook entry point

`astrolabe-trial hook <event> --if-active` is called by the plugin. It does nothing when the machine switch is off.

The guard blocks a fixed set of destructive shell commands supplied to the hook. It is an accident guard, not a sandbox: it cannot see file edits or other tools. Effort settings guide agents; they aren't enforced permissions. Each session proves its own hook with an allowed control and a denied probe; `astrolabe activate --status` reports whether that proof holds.

[Back to the README](../README.md)
