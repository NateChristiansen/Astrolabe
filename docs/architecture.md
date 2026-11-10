# Architecture

This guide explains how the plugin, notes and local board work together. For normal use, start with [installation](install.md) or [Using Astrolabe](usage.md).

## Machine and session state

The installed program's `on` and `off` commands control one machine switch. When off, plugin hooks do nothing. When on, new sessions receive the owner note, the Rules section of the voice note, recording instructions, a suggestion of the current repo's active efforts, and that repo's note, which holds how work lands there. A repo note too long for the hook's cut goes as a file for the agent to read first. Shell commands are guarded even before an effort is loaded.

Loading an effort adds its settings, current state, Pieces and linked initiative title when present, the whole voice note, the repo note and machine notes. The agent loads it only when the owner asks to start, resume or switch to that effort. `ASTROLABE_EFFORT=<number>` at launch binds a new session to that effort the same way, with no worker restrictions.

The rules delivered with a binding depend on its role. An owner's effort gets every rule: saving, Current state, decisions, plans and Pieces, states, the journal, lessons, initiative filing and how orchestration works. A worker gets the same saving rules without the journal, lessons, filing or orchestration, then its restrictions last. An initiative binding gets saving, decisions, states without review, the initiative's journal line, lessons and the orchestrator's steps.

Subagents receive the shell guard, but don't inherit the parent session's context or authority automatically, and can't bind to an effort. So each worker is a separate session, whose first command is `astrolabe activate --effort <number> --worker` to receive its effort's context. A worker saves that effort's own record and nothing else; the dispatcher files efforts, writes the journal and marks each done.

Effort involvement and authority are context supplied to agents. They aren't enforced permissions.

## Hooks and session binding

The [hook manifest](../plugin/astrolabe/hooks/hooks.json) calls `~/.local/bin/astrolabe` at session start, when a prompt is sent, before shell commands, after supported edits, at stop, and at session end. The fixed location avoids depending on the PATH inherited by a desktop app.

The plugin uses a shared folder with a marketplace file for each supported app. Claude Code uses its plugin layout; Codex accepts that compatibility layout. On Windows the hook command is expanded by PowerShell in Codex and by Git Bash or sh in Claude Code. These app-specific installation details are in the [setup guide](install.md#2-install-the-plugin-in-your-agent-app).

The agent runs `astrolabe activate --effort <number>` as a standalone shell command. The pre-command hook receives the app's session identifier and binds the effort there. It then replaces the command with a plain read of the result file: `cat` in a POSIX shell or `Get-Content -Raw` in PowerShell.

The hook performs the write before the app runs that read. This lets effort commands work during planning, including when the app permits only read commands. A command that the hook didn't intercept doesn't load an effort or claim guard protection.

A session on an effort is named after it, as the effort's number and title ("83 Name sessions after their effort"), so the owner can see which agent is on what. Only taking up the work names it: loading an effort, or filing one that starts active. An effort filed for later, proposed or ready, is bound to the session that filed it but leaves its name as it was. Claude Code takes the name from a hook: at session start for a session already on an effort, or at the next prompt for one that picked an effort up mid-session, since no hook before a command can set it. So the name shows at once, a Claude agent that picks up an effort is also told to rename its session with the Claude desktop app's own tool; the hook still names it where there's no such tool. Each name is given once, and a rename the owner makes after it is kept, as is one made after the agent was told to rename the session. A session already on an effort at its start has no such tell, and there the hook can't tell the owner's name from the app's own title, so it replaces either. Codex hooks can't name a thread, so a Codex agent that picks up an effort is told to rename its thread with the Codex app's own tool; a thread started outside the app has no such tool and isn't listed there.

## Recording work

For a change, plan or investigation that needs a lasting record, the agent runs `astrolabe effort new` with a title and a goal. The program creates and commits the note, assigns its number and binds it to the session. New work defaults to `check in at plan` involvement and `edit` authority unless the owner specifies otherwise.

The agent saves a short current state, decisions and an optional plan. Work with several landable steps keeps a Pieces checklist of independently checkable outcomes, written at the start as a best guess and revised as the work goes; a one-step fix or spike needs none. A tick stamps its day, a dropped line stays struck, and a save can't lose a ticked line. It is instructed to save before waiting on the owner, before long runs and when a piece is done. The program adds machine and session facts; the agent writes the meaning. Journal entries are validated before writing: they must be short, describe the change, and omit filenames and closing status-only sentences. See the [journal format](commands.md#journal).

A plan that names several efforts ends in an initiative: `astrolabe initiative new` files the page alone, with new proposed efforts, or around existing efforts. Membership is an effort's `initiative` property naming the page; a planning-only spike stays outside and is linked as provenance. Progress is derived from member statuses, never a checklist of efforts. The orchestrator binds with `activate --initiative <slug>` and saves its state, decisions, owner questions and daily journal line on that page. Workers save only their effort and journal nothing; their briefs and reports live under `.handoffs/<initiative slug>/` in the notes.

Efforts can be proposed, ready, active, review, blocked, done or abandoned. Review means built and checked, waiting for review, acceptance or landing; blocked means an impediment. Status changes to active, review, blocked, done or abandoned require Current state. After `effort read <number>`, a dispatcher can save that effort's body and ticks by number with stale writes refused, including across repos sharing an initiative. Initiatives have no review status and close explicitly with an acceptance sentence and no open members.

Recording commands also run through the pre-command hook. The hook can write to the separate notes checkout even when the app's tool sandbox cannot. Without interception, the executable reports an already-recorded result, an unfinished write, or that nothing was recorded; running it directly doesn't create or save an effort.

The Stop hook compares the repo's HEAD, diff and untracked contents with the last saved state. Edit and commit events can also mark a session as changed. It nudges once per change and avoids a repeated stop loop. A nudge is a reminder, not a mandatory save gate. Findings left only in chat can go undetected.

The pre-command and Stop hooks silently check for a new human reply when their bound effort has an owner wait. They read at most the last 64 KiB of the provider transcript (plus a bounded 64 KiB session header for Codex), requiring explicit human provenance and the original message time after delivery of the current note version. Claude queued replies use their original timestamp. Background messages, stale replies, other sessions and workers cannot clear the wait. Missing or changed transcript formats leave it unchanged; the next ordinary save remains the fallback. No transcript text enters model context.

The automatic transition clears only `waits_for`, changes active/blocked to active (review efforts stay untouched), and updates the note timestamp. It preserves progress text, journal and save-nudge bookkeeping. It uses the existing local write lock and durable intent, checks the delivered note hash, refreshes the board, and syncs in the background. It skips busy locks and pending recovery rather than delaying a tool. Unlike explicit saves it does not fetch before writing; remote conflicts still use the normal sync protections. A reply is activity, not approval.

## Synchronization and recovery

Notes writes share an advisory lock on each machine. Each complete write is first stored as an intent in the local state folder, then applied so a retry won't commit the same operation twice. Interrupted writes can be finished from those intents. An unreadable intent is preserved and reported by `notes status`; new writes wait until it is repaired or moved aside.

Before writing, Astrolabe attempts to pull notes with a bounded wait. A pull timeout backs off further attempts for five minutes. A save is refused if the effort changed since the session received it; the reply includes the current text so the agent can reconcile its next save.

If a pull brings in two efforts with the same number, the newer one is renumbered, its Decisions record the change, and its journal entries follow it.

Pushes run in the background after every save, and again at a turn end, session end or the next session start for anything still unpublished. While a board page is open, its server pulls once a minute and when a page loads, so a machine that's only being looked at still follows the others. That pull is skipped while a save holds the lock, and it only rebases when origin has new commits, so hand edits in the checkout aren't stashed when nothing came in. The server never pushes. `notes status` reports unpublished or diverged state; `notes sync` retries synchronization explicitly. These commands use the installed executable, as shown in the [command reference](commands.md#board-and-notes).

The journal uses Git's union merge driver. Hidden entry metadata identifies the effort or initiative and writing machine, allowing a writer to replace its own entry and the board to collapse duplicate entries. This reduces conflicts when machines add to the same day; it doesn't remove the need to check sync errors.

When working in the notes checkout, a pull stashes hand edits around the rebase. If they can't be reapplied, they stay in the stash and the next reply reports it. Recording commits include changes under `efforts/`, `journal/`, `initiatives/`, `lessons/` and `drafts/`, plus `.gitattributes`, including hand edits there. Lessons and drafts are written as plain files, so the next save, journal entry or filing on that machine is what commits one, as it stands, under that write's own subject; a sync then carries it. `lesson close` is the exception: its two commits hold only the lesson it closes. Other staged files stay staged and out of those commits; knowledge notes and repo registration changes need a separate commit.

## Files and local state

Personal content lives in the notes checkout and is read at runtime:

- `knowledge/owner.md` and `knowledge/voice.md`: preferences and writing style. New sessions receive the owner note and voice rules; effort activation adds the full voice note.
- `knowledge/repos/<name>.md`: each repo's rules, delivered to every session in that repo.
- `knowledge/machines.md`: context delivered with an effort.
- `efforts/<year>/<month>/NNNN-slug.md`: effort records, stored by creation date so status changes don't move files or break links.
- `journal/<year>/<month>/<day>.md`: the owner's history, written by agents rather than generated from tool activity.
- `lessons/`: an inbox of friction waiting for triage. The board lists it; sessions don't automatically receive it, so unreviewed observations don't become standing instructions. Triaged entries are deleted.
- `drafts/`: text waiting for the owner to post or decide on. The board counts it; nothing delivers it to a session. Closed drafts are deleted.

  The board lists both from disk, so it shows what this machine has. A lesson or draft that Git reports as untracked, changed or staged is marked "not committed": no other machine can have it until a notes write here commits it. That fact is read from `git status` when the board's data is collected; rendering runs no Git.
- `initiatives/`: initiative records; the board reads their members from effort frontmatter.
- `.handoffs/`: material agents write for each other, hidden behind the board's dot-folder toggle.

The effort format is fixed by the program; there is no user-editable effort template.

Local state defaults to `~/.local/state/astrolabe-layer/`, or `%USERPROFILE%\.local\state\astrolabe-layer\` on Windows. It holds machine settings, repo bindings, session state, write intents, result files and guard denials. `ASTROLABE_LAYER_STATE` overrides the location for isolated tests.

The notes checkout is registered locally as `agent`. Target repos with remotes are matched against their shared repo notes; repos without remotes need local bindings. Loading an effort also requires its repo note, including for a repo without a remote. The notes checkout and its worktrees can load any effort. Creating an effort or journal entry there requires `--repo` because the folder doesn't identify the target project.

## Serving the board

Session starts and notes changes ensure a local server is running. Starting it from the hooks saves the owner from configuring a separate startup service. A lock keeps one server per machine. It normally listens at [localhost:4747](http://localhost:4747/), and stops after a day without a request. It also watches the program file it started from: once a new one sits there, it starts that program on the same port and exits, so an update reaches the board without a manual restart. The `home` command can set the port, open the browser or run the server in the foreground.

The page receives change notifications and refreshes the board, tree or open note in place. It preserves the reader's scroll and navigation. The sidebar's status line says "Reconnecting…" while the stream is down. A refresh that fails while it stays up, from an error response, a network failure or a page that won't parse, leaves the older content in place and says so; only a successful refresh clears that, and the next change or reconnect retries. There is no retry timer. The notes tree groups efforts and journal entries by month, newest first.

Each update also writes `home.html` at the notes root, excluded from Git through that checkout's local exclude file. This standalone page reloads once a minute when used without the server.

Appearance settings are saved per machine; a browser's own choice takes precedence there. Iris is the default palette, with Ember, Slate, Moss and Graphite also available; see the [command reference](commands.md) for setting one. The page and browser icon share the [project artwork](../assets/branding/README.md).

## The shell guard

The pre-command hook checks commands submitted through the app's shell tools. It denies machine-wide kills, force pushes, hard resets, posting through `gh`, and recursive deletes outside the worktree.

`astrolabe allow --once "<exact command>" --said "<the owner's words>"` follows the activation routing: the hook writes and the app reads its result file. With an effort loaded, it records the command, quote, session, working directory, mint time and five-minute expiry in a separate lease file beside the session state, under a per-session lease lock. Session-state writers never rewrite leases, so concurrent hooks cannot erase or restore approvals through stale session state. Only force pushes and GitHub writes qualify. After an ordinary denial, the hook checks for an unexpired byte-for-byte match in that session and directory, durably consumes it before returning no denial, and logs minting and use in `guard-denials.log`. Missing, corrupt or unwritable state keeps the command denied. Classification in `guard.rs` remains pure. The hook can't read chat, so the quote records the agent's claim of the owner's words.

Loading or creating an effort asks the session to prove interception with both an allowed control and a denied probe. A new session has no proven protection until that check succeeds. `astrolabe activate --status` checks that the control ran, the probe was denied and its marker wasn't created. A subagent's proof doesn't prove the parent session's hook.

The guard doesn't see file edits, MCP calls, input written into an existing shell, or commands an app runs without invoking the hook. It's an accident guard, not a security boundary.

## Other limits

- A direct command run within a minute of another session's identical command in the same folder can report that session's result. The reply identifies that case.
- Instructions already delivered to a running session don't update automatically. Tell the agent about the change or start a fresh session.
- Synchronizing notes doesn't synchronize project files or reconstruct an unsaved conversation.

## Rust implementation

The crate stays small enough to build as one package. Modules separate responsibilities rather than imposing a file-length target. The command grammar, saved formats and hook responses remain compatible.

### Where changes belong

- `effort/command.rs` parses shell commands and bodies. `effort/file.rs` interprets note properties, sections and checklists. Neither starts processes or writes files. `effort/initiative.rs` reads and writes an initiative page and resolves which page an effort's `initiative` property names. `effort/roster.rs` builds what a reply shows about an initiative's members. `effort.rs` exposes them and holds the allowed involvement and authority values.
- `record.rs` coordinates new efforts, saves and journal entries: the lock, recovery, pull, stale checks and the write stay there, in order. `record/save.rs` holds a save's checked request, its pure record edits, stamps and commit subject, and its reply; it reads no notes and writes no session state. `record/validation.rs` checks prose, settings and plan sections before a write is attempted. `record/lesson.rs` parses and applies `lesson close`.
- `notes/storage.rs` owns recoverable operations and commit scope. `notes/sync.rs` owns network Git calls, backoff, merge recovery and synchronization warnings. `notes/journal.rs` owns journal formats; `notes/facts.rs` observes a product checkout; `notes/process.rs` owns detached children and platform handle behavior. `notes.rs` holds the checkout, lock, numbering and shared filesystem primitives.
- `session.rs` owns session records and the save and acknowledgement transitions. `nudge.rs` decides when work needs a save reminder. `reply.rs` clears an owner wait when the bound conversation shows a new human reply.
- `activate/repos.rs` matches checkouts to repository notes. `activate/context.rs` loads effort context. `activate/proof.rs` evaluates guard evidence. `activate/tickets.rs` persists hook replies. `activate/routing.rs` handles provider inputs and applies activation transitions. `activate/allow.rs` records and consumes a one-time approval for a guarded command. `activate/housekeeping.rs` runs `notes sync` and `lesson close` inside the hook. `activate.rs` exposes the facade and machine switch.
- `guard.rs` classifies a shell command as allowed or denied; it is pure, and reads what a command says rather than running anything. `hook.rs` reads a hook's input and writes its denial output.
- `board.rs` collects a typed snapshot for both the board and `efforts --json`. Collection may read Git status; rendering does not run Git.
- `home/server.rs` handles HTTP and live event streams. `home/render.rs`, `home/markdown.rs` and `home/nav.rs` render the board, notes and navigation. `home/search.rs` answers the notes view's search, inside the notes checkout only. `home/settings.rs` holds appearance and port settings. `home.rs` exposes them and writes the standalone board file. CSS and JavaScript live beside the renderer and are embedded at build time.
- `application.rs` connects completed note operations to presentation. `home_effects.rs` writes the ignored board file and ensures the local server is running. Storage has no dependency on either rendering or server startup.
- `main.rs` parses the program's arguments and calls the module that owns each command; `lib.rs` lists the modules.
- `install.rs` installs the executable it was invoked from, replacing only a program the installer's receipt recorded and keeping a backup. `update.rs` moves the program and each app's plugin to the newest release or nightly. `codex_setup.rs` is the opt-in `setup codex` command; no hook or installer calls it.
- `seed.rs` creates a new notes checkout for `notes init`. `registry.rs` holds the local repo registration helpers. `worktree.rs` runs Git for the modules that read a checkout.

### Trace a save

1. The provider hook reaches `activate::gate`; command recognition and parsing use `EffortCommand`.
2. `record::save` reads the typed session record and checks the request with `SaveRequest::parse`, which refuses anything that needs no note before any lock or write.
3. It takes the notes lock, reconciles pending operations, answers a retry of the same tool call, then pulls and resolves initiative membership. A save by number goes to `save_target` from here.
4. It checks the delivered note hash, then the status transition and an initiative's open members, and edits the record with the pure helpers in `record/save.rs`. It plans an `Op` containing the complete files, journal lines and session state to write.
5. `Notes::apply` persists the intent, writes files, commits only the intended note paths, records that the commit completed, then writes the session state. A retry finishes remaining steps without overwriting a newer committed note.
6. It starts the background sync, builds the reply with `saved_reply`, and `application::finish` records the reply and refreshes the board after the operation. Storage reports changes; orchestration consumes them while holding the notes lock. Several recovered operations can share one refresh. A board failure is reported separately from a successful notes save.

The existing deliberate aggregation of hand edits inside effort and journal paths remains. Unrelated staged files stay out of automatic note commits.

### Trace a resume

The session-start route reads `SessionState`, refreshes notes and reads the record again because synchronization may renumber the effort. Context loading matches the repository, reads the effort through the shared note parser and prepares the session text. Only a successful transition replaces the session record. A failed context reload retains guard protection and records the existing shell-command block behavior.

`SessionState` keeps JSON at the storage boundary. Its `Field<T>` represents a known value, an unexpected legacy value or an absent field. Null and absent fields remain distinct; unsupported optional values retain their old fallback behavior. Unknown fields survive saving and resuming the same effort. A deliberate switch to another effort starts a new record, as before. Resume still refreshes derived context and clears the fields the previous implementation cleared.

Pending operation state payloads remain JSON because they are durable replay instructions, including records produced by older builds. Their persistence format is unchanged. Provider hook input and output also remain JSON at the external boundary.

### Verification

Run `scripts/check` before committing. Binary integration fixtures remain in `tests/activate.rs` and `tests/record.rs`; their subdirectories group scenarios by behavior. Recovery, two-machine synchronization and live HTTP flows still use real child processes and disposable Git repositories. Small parser and rendering tests remain beside the implementation.

For state changes, verify legacy optional fields and unknown data through a save or resume. For rendering or asset moves, compare collected JSON and rendered HTML with the previous build, then inspect both a normal and narrow viewport. The recorded-backoff test verifies an existing backoff record; it does not simulate a network timeout.

The same checks run in macOS, Windows and Linux CI. A local macOS pass does not establish Windows, Linux or live agent-client compatibility.

[Contributing](../CONTRIBUTING.md) · [Command reference](commands.md)
