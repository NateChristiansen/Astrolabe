# Architecture

This guide explains how the plugin, notes and local board work together. For normal use, start with [installation](../plugin/README.md) or [Using Astrolabe](usage.md).

## Machine and session state

The installed program's `on` and `off` commands control one machine switch. When off, plugin hooks do nothing. When on, new sessions receive owner and voice notes, recording instructions, and a suggestion of the current repo's active efforts. Shell commands are guarded even before an effort is loaded.

Loading an effort adds its settings and current state, the repo note and machine notes. The agent loads it only when the owner asks to start, resume or switch to that effort. `ASTROLABE_EFFORT=<number>` at launch can select it for a worker.

Subagents receive the shell guard, but don't inherit the parent session's context or authority automatically. A dispatched worker runs `astrolabe activate --effort <number> --worker` to receive context. Workers leave notes reporting to the agent that dispatched them.

Effort involvement and authority are context supplied to agents. They aren't enforced permissions.

## Hooks and session binding

The [hook manifest](../plugin/astrolabe/hooks/hooks.json) calls `~/.local/bin/astrolabe-trial` at session start, before shell commands, after supported edits, at stop, and at session end. The fixed location avoids depending on the PATH inherited by a desktop app.

The plugin uses a shared folder with a marketplace file for each supported app. Claude Code uses its plugin layout; Codex accepts that compatibility layout. On Windows the hook command is expanded by PowerShell in Codex and by Git Bash or sh in Claude Code. These app-specific installation details are in the [setup guide](../plugin/README.md#2-install-the-plugin-in-your-agent-app).

The agent runs `astrolabe activate --effort <number>` as a standalone shell command. The pre-command hook receives the app's session identifier and binds the effort there. It then replaces the command with a plain read of the result file: `cat` in a POSIX shell or `Get-Content -Raw` in PowerShell.

The hook performs the write before the app runs that read. This lets effort commands work during planning, including when the app permits only read commands. A command that the hook didn't intercept doesn't load an effort or claim guard protection.

## Recording work

For a change, plan or investigation that needs a lasting record, the agent runs `astrolabe effort new` with a title and a goal. The program creates and commits the note, assigns its number and binds it to the session. New work defaults to `check in at plan` involvement and `edit` authority unless the owner specifies otherwise.

The agent saves a short current state, decisions, an optional plan and a checklist of pieces. It is instructed to save before waiting on the owner, before long runs and when a piece is done. The program adds machine and session facts; the agent writes the meaning.

Recording commands also run through the pre-command hook. The hook can write to the separate notes checkout even when the app's tool sandbox cannot. Without interception, the executable reports an already-recorded result, an unfinished write, or that nothing was recorded; running it directly doesn't create or save an effort.

The Stop hook compares the repo's HEAD, diff and untracked contents with the last saved state. Edit and commit events can also mark a session as changed. It nudges once per change and avoids a repeated stop loop. A nudge is a reminder, not a mandatory save gate. Findings left only in chat can go undetected.

## Synchronization and recovery

Notes writes share an advisory lock on each machine. Each complete write is first stored as an intent in the local state folder, then applied so a retry won't commit the same operation twice. Interrupted writes can be finished from those intents.

Before writing, Astrolabe attempts to pull notes with a bounded wait. A pull timeout backs off further attempts for five minutes. A save is refused if the effort changed since the session received it; the reply includes the current text so the agent can reconcile its next save.

If a pull brings in two efforts with the same number, the newer one is renumbered, its Decisions record the change, and its journal entries follow it.

Pushes run in the background after state changes, periodically during activity, at session end, and at the next session start for pending work. `notes status` reports unpublished or diverged state; `notes sync` retries synchronization explicitly. These commands use the installed executable, as shown in the [command reference](commands.md#board-and-notes).

The journal uses Git's union merge driver. Hidden entry metadata identifies the effort and writing machine, allowing a writer to replace its own entry and the board to collapse duplicate effort entries. This reduces conflicts when machines add to the same day; it doesn't remove the need to check sync errors.

When working in the notes checkout, a pull stashes hand edits around the rebase. If they can't be reapplied, they stay in the stash and the next reply reports it. Manual edits and repo registration changes are the owner's to commit.

## Files and local state

Personal content lives in the notes checkout and is read at runtime:

- `knowledge/owner.md` and `knowledge/voice.md`: preferences delivered to new sessions.
- `knowledge/machines.md` and `knowledge/repos/<name>.md`: context delivered with an effort.
- `efforts/<year>/<month>/NNNN-slug.md`: effort records, stored by creation date so status changes don't move files or break links.
- `journal/<year>/<month>/<day>.md`: the owner's history, written by agents rather than generated from tool activity.
- `lessons/`: an inbox of friction waiting for triage. The board lists it; sessions don't automatically receive it, so unreviewed observations don't become standing instructions. Triaged entries are deleted.
- `initiatives/`: notes for groups of related efforts.
- `.agents/`: material agents write for each other, hidden behind the board's dot-folder toggle.

The effort format is fixed by the program; there is no user-editable effort template.

Local state defaults to `~/.local/state/astrolabe-layer/`, or `%USERPROFILE%\.local\state\astrolabe-layer\` on Windows. It holds machine settings, repo bindings, session state, write intents, result files and guard denials. `ASTROLABE_LAYER_STATE` overrides the location for isolated tests.

The notes checkout is registered locally as `agent`. Target repos with remotes are matched against their shared repo notes; repos without remotes need local bindings. Loading an effort also requires its repo note, including for a repo without a remote. The notes checkout and its worktrees can load any effort. Creating an effort or journal entry there requires `--repo` because the folder doesn't identify the target project.

## Serving the board

Session starts and notes changes ensure a local server is running. Starting it from the hooks saves the owner from configuring a separate startup service. A lock keeps one server per machine. It normally listens at [localhost:4747](http://localhost:4747/), and stops after a day without a request. The `home` command can set the port, open the browser or run the server in the foreground.

The page receives change notifications and refreshes the board, tree or open note in place. It preserves the reader's scroll and navigation. The notes tree groups efforts and journal entries by month, newest first.

Each update also writes `home.html` at the notes root, excluded from Git through that checkout's local exclude file. This standalone page reloads once a minute when used without the server.

Appearance settings are saved per machine; a browser's own choice takes precedence there. Iris is the default palette, with Ember and Slate also available. The page and browser icon share the [project artwork](../assets/branding/README.md).

## The shell guard

The pre-command hook checks commands submitted through the app's shell tools. It denies machine-wide kills, force pushes, hard resets, posting through `gh`, and recursive deletes outside the worktree.

Each session must prove interception with both an allowed control and a denied probe. `astrolabe activate --status` checks that the control ran, the probe was denied and its marker wasn't created. A subagent's proof doesn't prove the parent session's hook.

The guard doesn't see file edits, MCP calls, input written into an existing shell, or commands an app runs without invoking the hook. It's an accident guard, not a security boundary.

## Other limits

- A direct command run within a minute of another session's identical command in the same folder can report that session's result. The reply identifies that case.
- Instructions already delivered to a running session don't update automatically. Tell the agent about the change or start a fresh session.
- Synchronizing notes doesn't synchronize project files or reconstruct an unsaved conversation.

[Contributing](../CONTRIBUTING.md) · [Command reference](commands.md)
