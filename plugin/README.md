# Astrolabe plugin

One plugin folder for Claude Code and Codex, in Claude's layout, with one marketplace file per app. A machine switch, `astrolabe on` and `astrolabe off`, run by the owner in a terminal, decides whether it does anything. Off, its hooks do nothing. On, every session and its subagents get the guard on shell commands, and every session gets the owner and voice notes. A session's effort is picked for it when its repo has exactly one active effort; otherwise the owner names one in plain words ("pick up 70", "work on effort 70", or `/astrolabe:effort 70`), and `ASTROLABE_EFFORT=70` at launch sets it for a worker. With an effort, that session also gets the effort's settings and current state, the repo note and the machine notes.

Everything personal is read from the notes checkout at run time; nothing here holds it.

## How an effort is bound to a session

The skill runs `astrolabe activate --effort <n>` as a plain shell command. The plugin's pre-command hook sees it with the session id the app supplies, does the work there, and rewrites the command to `astrolabe activate --show <ticket>`, which only prints what the hook left. A command no hook took prints `NOT switched on` and claims no protection, which is what an untrusted Codex hook looks like.

## Setup, once per machine

1. The program at `~/.local/bin/astrolabe-trial`, which the hooks call by that path for the trial so the build in use elsewhere stays as it is. `ASTROLABE_TRIAL_BIN` overrides it, for tests.
2. The notes checkout: `astrolabe repo add agent <notes checkout>`.
3. Each target repo, under the name its note has in `knowledge/repos/<name>.md`: `astrolabe repo add <name> <checkout>`.
4. `astrolabe-trial on`.
5. Each effort note has, in its properties (YAML frontmatter, as Obsidian shows them): `status: active` to be picked automatically, `repo: <name>` (or a list), `involvement: <autonomous | check in at plan | each step>` and `authority: <read | edit | push>`. Missing or unknown values refuse the switch, with the field named; nothing fills them in.

## Install

These write each app's home config.

- Claude: `claude plugin marketplace add <this folder>`, then `claude plugin install astrolabe@astrolabe`.
- Codex: `codex plugin marketplace add <this folder>`, then `codex plugin add astrolabe@astrolabe`, then review and trust both hooks with `/hooks`. Codex marks the hooks modified after any plugin version change, and they don't run until trusted again.

Remove: `claude plugin uninstall astrolabe@astrolabe` and `claude plugin marketplace remove astrolabe`; `codex plugin remove astrolabe@astrolabe` and `codex plugin marketplace remove astrolabe`. That leaves the program, its machine config and any activation state in `~/.local/state/astrolabe-layer/`.
