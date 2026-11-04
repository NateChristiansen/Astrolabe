# Astrolabe plugin

One plugin folder for Claude Code and Codex, in Claude's layout, with one marketplace file per app. A machine switch, `astrolabe on` and `astrolabe off`, run by the owner in a terminal, decides whether it does anything. Off, its hooks do nothing. On, every session and its subagents get the guard on shell commands, and every session gets the owner and voice notes, plus its repo's active efforts as a suggestion. No effort is loaded until the owner asks for one in plain words ("pick up 70", "work on effort 70", or `/astrolabe:effort 70`), and `ASTROLABE_EFFORT=70` at launch sets it for a worker. With an effort, that session also gets the effort's settings and current state, the repo note and the machine notes.

Every session also records its work: when the owner's request means a change, findings or a plan, the session runs `astrolabe effort new` with a title and a one-line goal, which numbers and writes `efforts/NNNN-slug.md` in the notes, journals a `started` line in `journal/YYYY-MM-DD.md`, commits, and binds the session to that effort. It saves with `astrolabe effort save`, the new Current state in a heredoc, at real moments: before waiting on the owner, before a long run, when a piece is done, and when the Stop hook says work moved since the last save. A question answered in chat records nothing. Agents write the meaning; the program writes the stamp (time, machine, provider, session, branch, head, unpushed, dirty), the journal line, the commit and the push.

Everything personal is read from the notes checkout at run time; nothing here holds it.

## How an effort is bound to a session

The skill runs `astrolabe activate --effort <n>` as a plain shell command. The plugin's pre-command hook sees it with the session id the app supplies, does the work there, and rewrites the command to `astrolabe activate --show <ticket>`, which only prints what the hook left. A command no hook took prints `NOT switched on` and claims no protection, which is what an untrusted Codex hook looks like.

## How notes get written

The agent's own tools can't write the notes checkout from a product repo (Codex's sandbox confines writes to the workspace; Claude prompts outside its folders), so the pre-command hook does it: it reads the whole `astrolabe effort …` command, heredoc body included, writes the notes as the user under one advisory lock per machine, and rewrites the command to a print of the result. The plain command, run with no hook, only explains itself: a recent ticket for the same command means it was done, a leftover intent means it's still finishing, otherwise "NOT recorded".

Writes are planned whole as an intent file in the state folder and applied idempotently, so a crash leaves what to finish and a retried tool call doesn't commit twice. Before each write the notes are pulled (bounded, 5 s); a save over a file that changed since it was delivered is refused with the current text. Two efforts with one number after a pull renumber the newer, with a `renumbered` journal line. Pushes run detached: at a state change, every 10 minutes, at session end, and at the next session start for anything left; `astrolabe notes status` shows what's unpublished or diverged, `astrolabe notes sync` runs a push by hand. The journal day file merges by union (`.gitattributes`, written on first use), so two machines appending the same day never conflict; the home view sorts lines by time and drops a doubled id.

The Stop hook nudges once per change when the repo's fingerprint (HEAD, the diff, untracked contents) differs from the last save's, or an edit or commit marked the session: Claude gets it as hook feedback, Codex as a block whose reason is the text, and `stop_hook_active` stops a second one. No save is ever required to finish. `astrolabe efforts [--repo <name>] [--all]` prints what the home view reads as JSON.

## Setup, once per machine

1. The program at `~/.local/bin/astrolabe-trial` (`astrolabe-trial.exe` on Windows), so the build in use elsewhere stays as it is. The hooks call it as `~/.local/bin/astrolabe-trial`, unquoted: Codex on Windows runs hooks in PowerShell and Claude in Git Bash or sh, all of them expand that `~`, and none needs PATH, which an app started from the Mac Dock may lack. A trial run points `~` at a scratch folder instead: HOME for Git Bash and sh, USERPROFILE for PowerShell.
2. The notes checkout: `astrolabe-trial repo add agent <notes checkout>`.
3. Each target repo, under the name its note has in `knowledge/repos/<name>.md`: `astrolabe-trial repo add <name> <checkout>`. The plugin reads these from its own records, one file per repo in `repos/` in its state folder, `~/.local/state/astrolabe-layer/` (`%USERPROFILE%\.local\state\astrolabe-layer\` on Windows), not from the app-data database, which a packaged Windows app reads from its own private copy. Its denial log is there too. `repo add` still writes the database for the notes repo's own layer; if the plugin's record fails it says so, and rerunning fixes it. `astrolabe on` refuses until `agent` is recorded there, since that's what keeps the plugin out of the notes checkout. Upgrading from before these records: switch off, rerun `repo add` for `agent` and each target repo, then switch on.
4. `astrolabe-trial on`.
5. Each effort note has, in its properties (YAML frontmatter, as Obsidian shows them): `status: active` to be suggested, `repo: <name>` (or a list), `involvement: <autonomous | check in at plan | each step>` and `authority: <read | edit | push>`. Missing or unknown values refuse the switch, with the field named; nothing fills them in.

## In the notes checkout

With the switch on, the plugin stays out of the registered notes checkout (`agent` in the registry) and its worktrees, found by git's common directory, because that repo runs its own layer. A trial exception to global coverage.

## Install

These write each app's home config.

- Claude: `claude plugin marketplace add <this folder>`, then `claude plugin install astrolabe@astrolabe`.
- Codex: `codex plugin marketplace add <this folder>`, then `codex plugin add astrolabe@astrolabe`, then review and trust all five hooks with `/hooks`. Codex marks the hooks modified after any plugin version change, and they don't run until trusted again.
- Upgrading: Claude keeps a cached copy of the version it installed, so after a plugin change run `claude plugin update astrolabe@astrolabe` (or uninstall and install); Codex: `codex plugin add astrolabe@astrolabe` again, then `/hooks` to trust the changed hooks.

Remove: `claude plugin uninstall astrolabe@astrolabe` and `claude plugin marketplace remove astrolabe`; `codex plugin remove astrolabe@astrolabe` and `codex plugin marketplace remove astrolabe`. That leaves the program, its machine config, and the repo list, denial log and any activation state in `~/.local/state/astrolabe-layer/`.
