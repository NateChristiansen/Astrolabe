# Astrolabe plugin

One plugin folder for Claude Code and Codex, in Claude's layout, with one marketplace file per app. It's idle in every session until that session switches on for an effort: `/astrolabe:effort 70` on Claude, `$effort 70` on Codex, or `ASTROLABE_EFFORT=70` at launch for a worker. Then that session alone gets the owner and voice notes, the effort's settings and current state, the repo note, the machine notes, and the guard on shell commands.

Everything personal is read from the notes checkout at run time; nothing here holds it.

## How a session is bound

The skill runs `astrolabe activate --effort <n>` as a plain shell command. The plugin's pre-command hook sees it with the session id the app supplies, does the work there, and rewrites the command to `astrolabe activate --show <ticket>`, which only prints what the hook left. A command no hook took prints `NOT switched on` and claims no protection, which is what an untrusted Codex hook looks like.

## Setup, once per machine

1. `astrolabe` on the `PATH` the apps give their hooks.
2. The notes checkout: `astrolabe repo add agent <notes checkout>`.
3. Each target repo, under the name its note has in `knowledge/repos/<name>.md`: `astrolabe repo add <name> <checkout>`.
4. Each effort note says, on a line under its title: `repo: <name> · involvement: <autonomous | check in at plan | each step> · authority: <read | edit | push>`. Missing or unknown values refuse the switch, with the field named; nothing fills them in.

## Install

These write each app's home config.

- Claude: `claude plugin marketplace add <this folder>`, then `claude plugin install astrolabe@astrolabe`.
- Codex: `codex plugin marketplace add <this folder>`, then `codex plugin add astrolabe@astrolabe`, then review and trust both hooks with `/hooks`. Codex marks the hooks modified after any plugin version change, and they don't run until trusted again.

Remove: `claude plugin uninstall astrolabe@astrolabe` and `claude plugin marketplace remove astrolabe`; `codex plugin remove astrolabe@astrolabe` and `codex plugin marketplace remove astrolabe`. That leaves the program, its machine config and any activation state in `~/.local/state/astrolabe-layer/`.
