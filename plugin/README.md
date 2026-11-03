# Astrolabe plugin

One plugin folder for Claude Code and Codex, in Claude's layout, with one marketplace file per app. A machine switch, `astrolabe on` and `astrolabe off`, run by the owner in a terminal, decides whether it does anything. Off, its hooks do nothing. On, every session and its subagents get the guard on shell commands, and every session gets the owner and voice notes, plus its repo's active efforts as a suggestion. No effort is loaded until the owner asks for one in plain words ("pick up 70", "work on effort 70", or `/astrolabe:effort 70`), and `ASTROLABE_EFFORT=70` at launch sets it for a worker. With an effort, that session also gets the effort's settings and current state, the repo note and the machine notes.

Everything personal is read from the notes checkout at run time; nothing here holds it.

## How an effort is bound to a session

The skill runs `astrolabe activate --effort <n>` as a plain shell command. The plugin's pre-command hook sees it with the session id the app supplies, does the work there, and rewrites the command to `astrolabe activate --show <ticket>`, which only prints what the hook left. A command no hook took prints `NOT switched on` and claims no protection, which is what an untrusted Codex hook looks like.

## Setup, once per machine

1. The program at `~/.local/bin/astrolabe-trial` (`astrolabe-trial.exe` on Windows), so the build in use elsewhere stays as it is. The hooks call it as `~/.local/bin/astrolabe-trial`, unquoted: Codex on Windows runs hooks in PowerShell and Claude in Git Bash or sh, all of them expand that `~`, and none needs PATH, which an app started from the Mac Dock may lack. A trial run points `~` at a scratch folder instead: HOME for Git Bash and sh, USERPROFILE for PowerShell.
2. The notes checkout: `astrolabe-trial repo add agent <notes checkout>`.
3. Each target repo, under the name its note has in `knowledge/repos/<name>.md`: `astrolabe-trial repo add <name> <checkout>`. The plugin reads these from `repos.json` in its state folder, `~/.local/state/astrolabe-layer/` (`%USERPROFILE%\.local\state\astrolabe-layer\` on Windows), not from the app-data database, which a packaged Windows app reads from its own private copy. Its denial log is there too. `repo add` still writes the database for the notes repo's own layer. An install from before `repos.json` has to run `repo add` again for `agent` and each target repo; until then sessions are told no notes checkout is set, and the plugin doesn't know to stay out of the notes checkout.
4. `astrolabe-trial on`.
5. Each effort note has, in its properties (YAML frontmatter, as Obsidian shows them): `status: active` to be suggested, `repo: <name>` (or a list), `involvement: <autonomous | check in at plan | each step>` and `authority: <read | edit | push>`. Missing or unknown values refuse the switch, with the field named; nothing fills them in.

## In the notes checkout

With the switch on, the plugin stays out of the registered notes checkout (`agent` in the registry) and its worktrees, found by git's common directory, because that repo runs its own layer. A trial exception to global coverage.

## Install

These write each app's home config.

- Claude: `claude plugin marketplace add <this folder>`, then `claude plugin install astrolabe@astrolabe`.
- Codex: `codex plugin marketplace add <this folder>`, then `codex plugin add astrolabe@astrolabe`, then review and trust both hooks with `/hooks`. Codex marks the hooks modified after any plugin version change, and they don't run until trusted again.

Remove: `claude plugin uninstall astrolabe@astrolabe` and `claude plugin marketplace remove astrolabe`; `codex plugin remove astrolabe@astrolabe` and `codex plugin marketplace remove astrolabe`. That leaves the program, its machine config, and the repo list, denial log and any activation state in `~/.local/state/astrolabe-layer/`.
