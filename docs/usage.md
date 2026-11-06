# Using Astrolabe

Once you've [installed Astrolabe](../plugin/README.md), you work by talking to your agent. It keeps the notes needed to continue later.

## Start work

Open a session in a registered project and ask for what you want:

> Review how this project handles failed uploads. Show me a plan before changing anything.

The agent creates an **effort** for work that needs a lasting record. It saves where things stand, decisions and their reasons, and what comes next. Work with several parts uses a checklist inside that effort.

A question answered on the spot doesn't need an effort. A useful finding can go in the journal instead.

Before you close a session, you can say:

> Save where we are so I can pick this up later.

Agents are instructed to save at meaningful points, including before waiting on you. A closed window can't save a conversation that exists only in chat.

## Pick up saved work

Find the effort's number on the board, then tell an agent:

> Pick up 12.

It loads the saved state, the project's notes and your machine notes. You can do this in a fresh session or on another configured machine. Astrolabe carries the written record; it doesn't copy the previous chat or your project's uncommitted files.

Open the session in that project when you want to work on its code. You can also load any effort from your notes folder to discuss or plan it. When starting new work from the notes folder, tell the agent which project it's for.

## Choose how the agent works

Each effort has two settings. You can state them when asking for work; the agent records them in the note's properties.

**Involvement** controls when the agent checks in:

- `check in at plan`: show the plan and wait for your approval before changing project files. This is the default for new work.
- `each step`: report after each step and wait for you before continuing.
- `autonomous`: finish the work within the allowed scope, then report.

**Authority** sets the allowed kind of work:

- `read`: inspect and report without changing project files.
- `edit`: change files and make local commits on a branch. This is the default; it doesn't allow pushing.
- `push`: also push the effort's branch where the project's rules allow it.

For example:

> Fix the broken links. Work autonomously, but keep the changes local.

These settings apply to project work; Astrolabe still syncs the private notes repo you've connected. They are instructions to agents, not enforced permissions, and don't override your boundaries or authorize posting as you.

If you change an effort's settings in its Markdown properties, ask the agent to load it again so it receives them. The initial **Fill in your notes** effort uses `each step` so you can review your preferences as they're written.

## Read the board and journal

Open the board from your terminal:

```sh
"$HOME/.local/bin/astrolabe-trial" home --open
```

On Windows, use:

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe-trial.exe" home --open
```

The board shows work that needs you, active efforts, ready work, blocked work and ideas. Completed and abandoned efforts appear in the recent closed work. Open a card to read its note; use the notes tree to browse the rest.

The journal is a short history of what changed, rather than a transcript of agent activity. Each effort gets a line for the day, updated as its work progresses. Lessons are a separate inbox of friction to review; the board shows what still needs triage.

Use the appearance controls on the page to choose a palette and light or dark mode. The board updates as notes change. A standalone `home.html` is also written into your notes folder for use without the server.

## Add a project

Choose a short name for the project, such as `my-app`. From that project's Git checkout, run:

```sh
"$HOME/.local/bin/astrolabe-trial" repo add my-app
```

On Windows:

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe-trial.exe" repo add my-app
```

If the project has a Git remote, Astrolabe records it in `knowledge/repos/my-app.md` in your notes. Check and commit that note, then sync your notes to make the registration available elsewhere. Other checkouts and worktrees with the same remote are recognized by it.

If the project has no remote, registration is local to this machine. Repeat it on each machine using the same name. Also create `knowledge/repos/my-app.md` in your notes: it is required to load efforts, and `repo add` doesn't create it for a project without a remote. A heading and a short description are enough to start. Commit this note too.

Start a fresh session in the project after registering it. The agent should know which project it's in and list its active efforts, if any.

## Keep your preferences current

Your notes folder contains:

- `knowledge/owner.md`: your working preferences and boundaries.
- `knowledge/voice.md`: how you write, with examples in your own words.
- `knowledge/machines.md`: machine details agents can't discover for themselves.
- `knowledge/repos/`: instructions and useful context for each project.

Edit these as plain Markdown, or ask an agent to help while working in the notes folder. Review and commit hand edits so they can travel with the notes. Owner and voice notes reach every new session; project and machine notes arrive when an effort loads.

An existing session keeps instructions it already received. Tell it what changed or start a new one.

## Share notes between machines

Use a **private Git remote** for your notes. In your notes folder, connect an empty private repo you created on your Git host:

```sh
git remote add origin <your-private-repo-url>
git push -u origin HEAD
```

Replace the URL placeholder with your repo's address. If this notes checkout already has a remote, keep that connection instead. Git must be able to authenticate from this machine.

Astrolabe commits its note updates and attempts to sync them in the background. Before switching machines, check:

```sh
"$HOME/.local/bin/astrolabe-trial" notes status
"$HOME/.local/bin/astrolabe-trial" notes sync
```

On Windows:

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe-trial.exe" notes status
& "$env:USERPROFILE/.local/bin/astrolabe-trial.exe" notes sync
```

Resolve any reported unpublished changes, authentication errors or divergence before expecting the other machine to see the latest state. Hand edits still need to be committed.

On the other machine, follow [installation](../plugin/README.md), choosing **existing notes** after cloning your private repo. Open the project's own checkout and ask the agent to pick up the effort. Project code travels through that project's Git workflow, separately from Astrolabe notes.

## Understand the guard

While Astrolabe is on, its shell hook checks for a fixed set of risky commands: machine-wide kills, force pushes, hard resets, posting through `gh`, and recursive deletion outside the worktree.

The agent checks that the hook actually runs in its session and reports whether the guard is proven. It's an accident guard, not a sandbox: it can't see file edits, other tools, or input sent into an already-running shell. See [guard details](internals.md#the-shell-guard) if you need to understand its limits.

[Installation](../plugin/README.md) · [Troubleshooting](troubleshooting.md) · [Command reference](commands.md) · [Project overview](../README.md)
