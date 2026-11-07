# Command reference

Exact syntax and options for the installed program and agent-session commands. For everyday tasks, see [Using Astrolabe](usage.md); for setup, see [installation](install.md).

- [Machine and repo setup](#machine-and-repo-setup)
- [Install the program](#install-the-program)
- [Update](#update)
- [Board and notes](#board-and-notes)
- [Load an effort](#load-an-effort)
- [Create an effort](#create-an-effort)
- [Create an initiative](#create-an-initiative)
- [Save an effort](#save-an-effort)
- [Journal](#journal)
- [Hook entry point](#hook-entry-point)

## Run the installed program

Setup, board and notes commands run through the installed executable. For example, to check machine status:

macOS / Linux:

```sh
"$HOME/.local/bin/astrolabe" status
```

Windows (PowerShell):

```powershell
& "$env:USERPROFILE/.local/bin/astrolabe.exe" status
```

Replace `status` with the arguments below. The current installer and hooks use the name `astrolabe`; the full path avoids depending on PATH. Agents can run these commands too, except `on` and `off`, which belong in your own terminal.

[Agent-session commands](#load-an-effort) instead use the literal name `astrolabe`. The plugin intercepts them; they don't need an executable on PATH.

## Machine and repo setup

| Arguments | Result |
| --- | --- |
| `--version` | Print the program version. |
| `--help` or `-h` | Print top-level syntax. Running without arguments does the same. |
| `on` | Enable Astrolabe for the machine. Requires registered notes. |
| `off` | Disable the machine's hooks. Instructions already delivered to a session remain in its context. |
| `status` | Show the machine switch. For the loaded effort and guard proof, use `astrolabe activate --status`. |
| `notes init <folder>` | Create, commit and register notes, including the first effort, **Fill in your notes**. Requires Git commit identity, an unregistered machine, and a new or empty folder (a fresh clone of an empty repo also works). |
| `repo add agent <checkout>` | Register an existing notes Git checkout on this machine, replacing any previous registration. |
| `repo add <name> [<checkout>]` | Register a project; omit the checkout to use the current repo. |

Project names use ASCII letters, digits, `-`, `_` and `.`, and cannot start with `.`. `agent` is reserved for the notes checkout.

For a project with a remote, registration writes the remote into `knowledge/repos/<name>.md`, so other machines can recognize the project. That edit is left uncommitted. A project without a remote gets a local binding; it also needs a repo note before an effort can load. See [adding a project](usage.md#add-a-project).

## Install the program

`install [--dir <folder>]` installs the executable you invoked; it does not download a release or install the plugin. Normal installation uses your user-local bin folder, matching the hooks. It replaces only a program its receipt recorded, and moves the `astrolabe-trial` copy that 0.5.0 also installed into the backup folder. `--dir` is for isolated testing and does not change where the hooks look. See [installation](install.md#1-download-and-install-the-program).

## Update

`update [--channel release|nightly] [--dry-run] [--force]` moves the program and the plugin together. It works out the channel from where your plugin comes from now: a release tag such as `v0.5.0` means **release**, and anything else (`main`, `nightly`, a local folder) means **nightly**. `--channel` picks one and switches to it.

- **release** installs the newest published release and registers the plugin at its tag.
- **nightly** installs the rolling `nightly` prerelease, which every push to `main` replaces, and registers the plugin at the `nightly` tag, the same commit as the program.

It verifies the running installation against its receipt before downloading. A repo build hands off to the verified user-local installation; a receipted custom installation updates in place. Missing or damaged installations stop with a repair message. It runs the published installer, which checks the download, keeps a backup and refuses files it didn't write. Then, in each app whose command is on PATH (`claude`, `codex`), it removes and re-adds the plugin and its marketplace. It skips an app it can't find and says so. With neither app found, it updates only the program.

An app whose marketplace already follows the channel's tag keeps its registration. The command refreshes that marketplace, then compares the app's installed copy of the plugin with the marketplace's, file by file. If any file differs, it removes and re-adds the plugin. Both apps keep the installed copy by the plugin's version, and that version stays the same across nightlies, so the apps' own update commands report a changed nightly as current. The command says it's already up to date only when the program is the newest build and every app's copy matches.

`--dry-run` prints what it would do and changes nothing; it compares with each marketplace as it stands, without refreshing it. `--force` reinstalls even when everything already matches. Restart your apps afterwards; in Codex, review the changed hooks.

## Board and notes

### `home`

Write the standalone `home.html`, ensure the local board server is running, and print its path and address. The plugin starts the server automatically during normal use. When the program file is replaced, as by `update` or `install`, the running server starts the new program on the same port within a few seconds and exits, and an open board reloads itself.

| Option | Effect |
| --- | --- |
| `--open` | Open the board in your browser. |
| `--serve` | Run the server in the foreground. If one is already running, print its address and exit. |
| `--port <number>` | Save the port for this machine; default `4747`. |
| `--theme auto\|light\|dark` | Save the theme; default `auto` follows the system. |
| `--palette iris\|ember\|slate\|moss\|graphite` | Save the palette; default `iris`. |

Options combine. A browser's saved appearance choice takes precedence in that browser.

The home dashboard puts work needing attention and active efforts first. The sidebar
opens the overview, efforts, journal and searchable notes. Click the Astrolabe mark to
return home; a note's back link returns to the view that opened it. Earlier journal
updates and recently closed work expand in place.

The palette menu beside the light/dark toggle offers Iris, Ember, Slate, Moss and
Graphite. It also includes **Follow system appearance**. Palette and appearance are
remembered independently in each browser. Live updates retain the current view,
reading position and expanded history.

```sh
"$HOME/.local/bin/astrolabe" home --open --theme dark
```

### `efforts [--repo <name>] [--all]`

Print board data as JSON. By default, effort rows exclude `done` and `abandoned`; `--all` includes them. `--repo` filters effort rows only.

The output contains `efforts`, `today`, today's raw `journal` and parsed `journal_lines`, `notes` sync warnings, `lessons` and the `drafts` count. Journal entries, warnings, lessons and drafts are not filtered by repo.

### `notes status` and `notes sync`

`notes status` reports unpublished changes, divergence and unfinished writes. `notes sync` retries pending writes and synchronization with the notes remote. See [troubleshooting](troubleshooting.md) for failures and [using another machine](usage.md#use-another-machine) for setup.

Run `astrolabe notes sync` alone in an agent session: the plugin's hook recovers unfinished writes locally, starts background sync and reports whether it started. Use `astrolabe notes status` for the result. In a terminal without the hook it runs synchronization directly as before.

### `lesson close`

Run `astrolabe lesson close lessons/<name>.md --as "<disposition>"` alone in an agent session. The hook validates the request, pulls the notes, then appends the disposition as the refreshed lesson's last line, commits only that lesson, removes it in a second commit and requests background sync. Both commit subjects name the lesson; unrelated staged work stays staged. Interrupted closes resume through the notes recovery path without repeating the disposition.

The path is relative to the notes checkout and must name a Markdown file inside `lessons/`, without symbolic links. A missing file, an outside path, a non-Markdown file or an empty disposition is refused without changes. The disposition must be one line. Without hook interception, the command closes nothing.

## Load an effort

These commands run through the plugin in an agent session with Astrolabe on. Normally you ask the agent to **pick up 12** and it handles the command.

| Command | Result |
| --- | --- |
| `astrolabe activate --effort <number>` | Load the effort and bind it to this session. |
| `astrolabe activate --effort <number> --worker` | Load the effort for a dispatched worker. The worker saves that effort's state, decisions and pieces, and nothing else: no journal line, no new effort or initiative, no other effort's record, no lesson close. Its dispatcher does those, and, after `effort read <number>`, marks it done with `effort save --effort <number> --status done` and a Current state body. |
| `astrolabe activate --initiative <slug>` | Bind to an initiative page. A quoted title also resolves; workers and subagents cannot bind. Missing involvement and authority default to `check in at plan` and `edit`. |
| `astrolabe activate --status` | Show the loaded effort or initiative and this session's guard proof. |
| `astrolabe activate --off` | Unload the effort or initiative; the machine switch and guard stay on. |

Run each command alone. The notes checkout can load any effort or initiative; a project checkout loads records for that project. Running the executable outside the hook does not bind a record.

## Allow one approved command

Run `astrolabe allow --once "<exact command>" --said "<the owner's words>"` alone through the plugin's hook with an effort loaded. Use literal quoting for your shell. It records a five-minute lease for one force push or GitHub write that the guard would otherwise deny. Use a direct `git` or `gh` invocation; chained commands, shell wrappers, expansion and other guard reasons are refused. Running the executable outside the hook records nothing.

Run the approved command byte for byte in the same session and working directory. The hook consumes the lease before letting it through, even if execution then fails. Minting and using a lease each append a line with the command and quote to the existing denial log. The hook can't read chat: the quote records the agent's claim of the owner's words.

## Create an effort

`astrolabe effort new` creates and commits a note, assigns its number and binds it to the session. It requires a title and a goal in the body:

```sh
astrolabe effort new --title "Fix failed uploads" <<'ASTROLABE'
Let people retry failed uploads without losing their selection.
ASTROLABE
```

| Option | Values and default |
| --- | --- |
| `--title "<title>"` | Required, nonempty title. |
| `--repo <name>` | Required from the notes checkout. In a project checkout, omit it: the project is inferred and an override is refused. |
| `--status <state>` | `proposed`, `ready` or `active`; default `active`. |
| `--involvement "<value>"` | `each step`, `check in at plan` or `autonomous`; default `check in at plan`. |
| `--authority <value>` | `read`, `edit` or `push`; default `edit`. |
| `--initiative "<name>"` | Optional grouping; default empty. Existing page titles and aliases are stored as their canonical slug. A name without a page is refused; file it with `astrolabe initiative new` first. |
| `--priority <label>` | Default `none`. The board recognizes `low`, `high` and `urgent` (`top` also sorts as urgent); other labels sort normally. |
| `--issue <n>` | The GitHub issue the work answers, or a list such as `"5, 7"`; default none. The session context then says to end the landing commit with `Fixes #<n>`, because the landing script pushes straight to main with no pull request, and GitHub closes an issue only when a closing keyword in a commit reaches the default branch. |

Involvement and authority follow the owner's instructions; see [choosing how the agent works](usage.md#choose-how-the-agent-works).

## Create an initiative

`astrolabe initiative new` files a page alone or with its efforts in one commit. Omit `## Efforts` for a page only. An Efforts section (any heading starting with "Efforts") accepts either new efforts, with a title and goal per paragraph, or existing efforts, with each paragraph beginning exactly `effort <number>` and optional explanatory prose. Do not mix the two forms. New efforts start `proposed`; both forms write the canonical page slug as membership. Joins preserve every other part of each effort and allocate no numbers. Duplicate, missing or conflicting members refuse the entire operation.

Pages carry status, repo, involvement, authority, wait and save stamps, followed by Current state and Decisions, with Purpose when there is purpose text. The first save of a legacy page inserts missing managed sections and puts its unheaded purpose under `## Purpose`, preserving that text, links, fences and other sections. Existing Purpose headings are reused. Titles and slugs resolve ignoring case and surrounding `[[ ]]`; ambiguous titles or slugs are refused.

```sh
astrolabe initiative new --title "Saving and progression" <<'ASTROLABE'
A fresh profile plays from the first gallery to the final valley, and the hub remembers.

## Efforts and order
Progress facts file
The per-profile record: facts with reset generations and a union merge.

Route and locked levels
Which tool each level teaches, the route as data, locked cards with hints.

## What completion means
- A fresh profile reaches the valley and the ending records.
ASTROLABE
```

| Option | Values and default |
| --- | --- |
| `--title "<name>"` | Required. The initiative's title; its filename slug under `initiatives/` becomes canonical membership. |
| `--repo <name>` | As for `effort new`: required from the notes checkout, refused elsewhere. |
| `--involvement "<value>"` | Applied to the page and every new effort; default `check in at plan`. |
| `--authority <value>` | Applied to the page and every new effort; default `edit`. |

The page is the orchestrator's record: state, cross-effort decisions, owner questions and the daily journal line live there, never in a separate orchestrate effort. Membership comes from efforts whose `initiative` property names the page; progress is derived from their statuses, never a checklist of efforts. Worker briefs and reports live under `.agents/<initiative slug>/` in the notes.

The existing session binding stays unchanged. No journal line is written by filing. The reply gives the activation command and actual commit/sync outcome. A planning-only spike closes without `--initiative`; the page links it as provenance.

Bind with `astrolabe activate --initiative <slug>` and save with `astrolabe effort save`. That save replaces Current state, appends dated Decisions, and accepts `--status`, `--waits-for`, `--reason` and `--journal`. The page appears in Needs you from an `owner:` wait. Its journal line is keyed by slug in the distinct `initiative:<slug>` namespace and replaces the previous line for the day, including after a merge. Initiative statuses are `proposed`, `ready`, `active`, `blocked`, `done` and `abandoned`. Done needs a nonempty acceptance statement in the body and every member done or abandoned; unreadable member records prevent closure. Pieces, ticks, plan replacement and initiative membership are unavailable on the page itself.

## Save an effort

`astrolabe effort save` updates the effort or initiative bound to this session and commits the notes. It takes a replacement Current state, any options below, or both. Omit the body to keep the existing Current state, unless the save changes status to active, review, blocked, done or abandoned: those transitions require a Current state body. Repeating the status on disk or omitting `--status` requires none. The same rule applies to initiative bindings, which have no review status. With `--effort <number>` it changes another effort instead; see below.

Without a notes remote, a successful save reports `saved locally; sync unavailable: no remote configured`. With a remote, `background sync started` means the background process was launched, not that a push finished. A launch failure is reported separately; the local save still succeeds.

```sh
astrolabe effort save --status active --journal "Added upload retries that keep the original selection." <<'ASTROLABE'
Failed uploads can now be retried without choosing the files again.

I kept the selection until the upload succeeds so a retry takes one click.

Ready for a look. Slow connections still need checking.
ASTROLABE
```

Write Current state as three short paragraphs: where it stands, choices and reasons, then what's next and what's unchecked. Keep it under 150 words, without label lines such as `**Status:**`.

| Option | Effect |
| --- | --- |
| `--status <state>` | Set `proposed`, `ready`, `active`, `review`, `blocked`, `done` or `abandoned`. Review means built and checked, waiting for review, acceptance or landing. |
| `--waits-for "<condition>"` | Record what needs to happen next. Use `owner: <request>` when it needs the owner. Required in the same command as `--status blocked`. An empty value clears it. |
| `--reason "<reason>"` | Required with `--status abandoned`, and refused with any other status. Stored under Decisions as a dated `abandoned: <reason>` line, so it stays after later saves replace Current state. |
| `--decision "chose X over Y: reason"` | Append a dated decision. Repeat the flag for several; each is recorded, in the order given. |
| `--journal "<entry>"` | Update today's journal line for this effort. A later save from the same machine replaces its earlier line. See [journal format](#journal). |
| `--plan-file <path>` | Replace the saved plan with a UTF-8 text file. |
| `--pieces-file <path>` | Replace the whole Pieces checklist with a UTF-8 text file. Include existing checked items you want to retain. |
| `--tick "<piece>"` | Check a piece, ignoring case. An exact label wins; otherwise the prefix must match exactly one unfinished piece. Ambiguous prefixes are refused with the matching labels; missing prefixes list the available pieces. Repeat to check several. |
| `--untick "<piece>"` | Uncheck a piece by its label or a unique prefix, ignoring case. The value must uniquely match across both checked and unchecked pieces, including longer labels. Repeat as needed. |
| `--initiative "<name>"` | Set the effort's `initiative` property; an empty value clears it. Existing page names resolve to their slug; unknown names are refused with a pointer to `astrolabe initiative new`. Omitting this option leaves membership untouched. |
| `--effort <number>` | Save another effort. After `effort read <number>` in this session, a body replaces its Current state and `--tick` / `--untick` change its pieces. Status, wait, reason, decision and initiative flags remain available. Journal, plan replacement and pieces-file writes are refused. The target must share the session's repo or an existing initiative with the bound record, or belong to the bound initiative page; the notes checkout can reach any repo. The bound record is untouched. |

A done save lists any unticked pieces in its reply without refusing the save. Keep a dated explanation of deferred or dropped pieces in the note.

When `--status done` marks an effort with an issue done, the result says whether a commit on this checkout's `origin/main` (as last fetched) closes each issue. It names any issue no commit closes, so the issue doesn't stay open after the work lands. It doesn't contact GitHub.

A human reply in the bound conversation clears an active or blocked `owner:` wait at the next supported pre-command or Stop hook, moving the effort into In motion. This updates status only; the agent still interprets the reply and saves its progress. A reply does not grant approval. Review efforts and unsupported or ambiguous transcripts leave the wait alone.

A save without `--waits-for` clears the previous waiting condition unless the effort stays blocked or in review. Give `--waits-for` again while still waiting; use `--status active` after an answer when automatic detection is unavailable. For example:

```sh
astrolabe effort save --status blocked --waits-for "owner: try upload retries" <<'ASTROLABE'
Blocked, waiting for the upload retry check.
ASTROLABE
```

After the condition is resolved:

```sh
astrolabe effort save --status active <<'ASTROLABE'
Active again after the upload check.
ASTROLABE
```

Pieces come with an approved plan that has more than one landable step, one independently checkable outcome per line. A one-step fix or spike needs none. A tick means its outcome is met, which can be on a branch; if landing is a deliverable, record it as a piece. File paths are relative to the session's working directory unless absolute. A pieces file uses Markdown checkboxes:

```markdown
- [x] Keep the upload selection
- [ ] Check slow connections
```

Flag values refuse `$`, backticks and inner double quotes. For a piece such as ``First repo note: `astrolabe repo add <name>` from that repo``, use an unambiguous prefix without changing the piece's text:

```sh
astrolabe effort save --tick "First repo note"
```

Alternatively, put the plan below Current state after a line containing `## Plan`. Use that or `--plan-file`, not both. The plan's headings are adjusted to fit inside the effort note.

`save` has no flags for changing the title, repo, involvement, authority, priority or issue. Edit those in the effort note with the owner’s approval, then reload the effort to refresh its session context.

### Read another effort

Run `astrolabe effort read <number>` alone through the hook before writing another effort's Current state or ticks. It prints the full note and remembers its content hash for that number in this session, separately from the bound record. A worker can read only its own effort. Read and save use the same repository and initiative access rules.

```sh
astrolabe effort read 12
```

Then, as a separate command:

```sh
astrolabe effort save --effort 12 --status review --tick "Build and check" <<'ASTROLABE'
Built and checked, waiting for review.
ASTROLABE
```

If the note changed since delivery, the save refuses to overwrite it and prints the current note. Read that text before saving again; the refusal records the newly delivered version. A successful save advances that number's hash without changing the bound record's hash. `--journal` remains unavailable with `--effort`.

### Multiline input and quoting

The Bash/zsh examples above use the exact quoted delimiter `<<'ASTROLABE'`, with `ASTROLABE` alone on the closing line. In PowerShell, use a single-quoted here-string as the last argument:

```powershell
astrolabe effort save --status active @'
Failed uploads can now be retried without choosing the files again.

I kept the selection until the upload succeeds so a retry takes one click.

Ready for a look. Slow connections still need checking.
'@
```

A PowerShell here-string piped into `astrolabe effort save` also works. Use the same body formats for `effort new`.

Each recording command must run alone, without chaining other commands. Flag values can be bare words or double-quoted strings; they cannot contain `$`, backticks or inner double quotes. Put prose needing those characters in the body. The hook performs the write and replaces the command with a read of its result; direct executable calls don't record work. See [recording details](architecture.md#recording-work).

## Journal

With an effort loaded, use `astrolabe effort save --journal "<entry>"`. With no effort loaded, record a standalone finding:

```sh
astrolabe journal "Found that upload retries already keep the original file selection."
```

From the notes checkout, include `--repo <name>`; from a project checkout, omit it. A standalone entry is appended to today's journal and committed. `journal` is refused while an effort is loaded, so the entry can be attached to that effort instead.

Entries describe what changed or was learned, in past tense, usually about 25 words. The program rejects multiline entries, more than 40 words or two sentences, filenames and paths, references to “the owner”, and a closing status-only sentence such as “Merged.” Use “my” when writing in the owner's voice. The board already shows status.

Supply the entry as one double-quoted argument, or in a body using the [multiline input format](#multiline-input-and-quoting), never both. Even in a body, the journal entry must be one line.

## Hook entry point

`hook <event> --if-active` is called by the plugin, with the app's event JSON on stdin. Events are `session-start`, `prompt`, `pre-command`, `post-tool`, `stop` and `session-end`. Hooks do nothing while the machine switch is off. See [the hook manifest](../plugin/astrolabe/hooks/hooks.json) and [architecture](architecture.md#hooks-and-session-binding) for integration details.

[Installation](install.md) · [Troubleshooting](troubleshooting.md) · [Using Astrolabe](usage.md) · [Project overview](../README.md)
