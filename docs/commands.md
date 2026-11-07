# Command reference

Exact syntax and options for the installed program and agent-session commands. For everyday tasks, see [Using Astrolabe](usage.md); for setup, see [installation](install.md).

- [Machine and repo setup](#machine-and-repo-setup)
- [Install the program](#install-the-program)
- [Update](#update)
- [Board and notes](#board-and-notes)
- [Load an effort](#load-an-effort)
- [Create an effort](#create-an-effort)
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

It runs the published installer, which checks the download, keeps a backup and refuses files it didn't write. Then, in each app whose command is on PATH (`claude`, `codex`), it removes and re-adds the plugin and its marketplace. It skips an app it can't find and says so. With neither app found, it updates only the program. `--dry-run` prints what it would do and changes nothing. `--force` reinstalls even when everything already matches. Restart your apps afterwards; in Codex, review the changed hooks.

## Board and notes

### `home`

Write the standalone `home.html`, ensure the local board server is running, and print its path and address. The plugin starts the server automatically during normal use.

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

## Load an effort

These commands run through the plugin in an agent session with Astrolabe on. Normally you ask the agent to **pick up 12** and it handles the command.

| Command | Result |
| --- | --- |
| `astrolabe activate --effort <number>` | Load the effort and bind it to this session. |
| `astrolabe activate --effort <number> --worker` | Load context for a dispatched worker. The worker leaves notes reporting to its dispatcher. |
| `astrolabe activate --status` | Show the loaded effort and this session's guard proof. |
| `astrolabe activate --off` | Unload the effort; the machine switch and guard stay on. |

Run each command alone. The notes checkout can load any effort; a project checkout loads efforts for that project. Running the executable outside the hook doesn't load an effort.

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
| `--initiative "<name>"` | Optional grouping; default empty. |
| `--priority <label>` | Default `none`. The board recognizes `low`, `high` and `urgent` (`top` also sorts as urgent); other labels sort normally. |

Involvement and authority follow the owner's instructions; see [choosing how the agent works](usage.md#choose-how-the-agent-works).

## Save an effort

`astrolabe effort save` updates the effort bound to this session and commits the notes. It takes a replacement Current state, any options below, or both. Omit the body to keep the existing Current state.

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
| `--status <state>` | Set `proposed`, `ready`, `active`, `blocked`, `done` or `abandoned`. |
| `--waits-for "<condition>"` | Record what needs to happen next. Use `owner: <request>` when it needs the owner. Required in the same command as `--status blocked`. |
| `--reason "<reason>"` | Required with `--status abandoned`, and refused with any other status. Stored under Decisions as a dated `abandoned: <reason>` line, so it stays after later saves replace Current state. |
| `--decision "chose X over Y: reason"` | Append a dated decision. |
| `--journal "<entry>"` | Update today's journal line for this effort. A later save from the same machine replaces its earlier line. See [journal format](#journal). |
| `--plan-file <path>` | Replace the saved plan with a UTF-8 text file. |
| `--pieces-file <path>` | Replace the whole Pieces checklist with a UTF-8 text file. Include existing checked items you want to retain. |
| `--tick "<piece>"` | Check a piece by its label or a unique prefix, ignoring case. Repeat to check several. |
| `--untick "<piece>"` | Uncheck a piece using the same matching rules. Repeat as needed. |

Setting a nonblocked status without `--waits-for` clears the previous waiting condition. For example:

```sh
astrolabe effort save --status blocked --waits-for "owner: try upload retries"
```

After the condition is resolved:

```sh
astrolabe effort save --status active
```

File paths are relative to the session's working directory unless absolute. A pieces file uses Markdown checkboxes:

```markdown
- [x] Keep the upload selection
- [ ] Check slow connections
```

Alternatively, put the plan below Current state after a line containing `## Plan`. Use that or `--plan-file`, not both. The plan's headings are adjusted to fit inside the effort note.

`save` has no flags for changing the title, repo, involvement, authority, initiative or priority. Edit those in the effort note with the owner’s approval, then reload the effort to refresh its session context.

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

`hook <event> --if-active` is called by the plugin, with the app's event JSON on stdin. Events are `session-start`, `pre-command`, `post-tool`, `stop` and `session-end`. Hooks do nothing while the machine switch is off. See [the hook manifest](../plugin/astrolabe/hooks/hooks.json) and [architecture](architecture.md#hooks-and-session-binding) for integration details.

[Installation](install.md) · [Troubleshooting](troubleshooting.md) · [Using Astrolabe](usage.md) · [Project overview](../README.md)
