# Host app behavior Astrolabe relies on

Astrolabe runs inside agent apps it doesn't control. This page records what those apps do at the points the program depends on, where each fact comes from, and what hasn't been checked. Cite it instead of assuming; recheck a fact when the app's version moves.

Checked on 2026-11-09 with Claude Code 2.1.316 and Codex CLI 0.166.0 on macOS. "Docs" means the app's published reference on that date. "Probe" means the behavior was run in a disposable config folder and observed.

## A hook that times out or fails

**Claude Code** (docs). A `PreToolUse` command hook that times out, crashes, fails to start or prints invalid JSON does not block the tool call: the call continues through the normal permission flow. Exit code 2, or a deny decision on exit 0, blocks. Setting `"onFailure": "block"` on a command hook makes a failed or timed-out hook block instead; it needs 2.1.295 or later. Plugin hooks are documented as the same shape as settings hooks, with no stated restriction on `onFailure` or `timeout`. Sources: [hooks reference](https://code.claude.com/docs/en/hooks), [plugin manifest reference](https://code.claude.com/docs/en/plugins/manifest-reference).

Command hooks default to a 600-second timeout. `UserPromptSubmit` defaults to 30 seconds, and `SessionEnd` hooks share 1.5 seconds unless a longer per-hook timeout is set.

**Codex** (docs and probe). Hooks default to 600 seconds; `SessionEnd` and `Interrupt` default to 1 second and allow at most 3. Exit code 2 or a deny decision blocks. The docs do not say what happens when a local command hook times out or fails. Source: [hooks](https://learn.chatgpt.com/docs/hooks).

Probed with a trusted `PreToolUse` command hook that had a 10-second timeout, in an unattended `codex exec` run:

- a hook that exits 2 at once blocks the command
- a hook that exits 1 lets the command run
- a hook that exits 0 after printing invalid JSON lets the command run
- a hook that sleeps 25 seconds and then exits 2 lets the command run; the run took about 26 seconds, so the command waited for the hook and its late denial was ignored

Not checked: `onFailure` set from a plugin's `hooks.json` in a live Claude Code session, and whether Codex has a setting that makes a failed hook block.

So in both apps a guard hook that runs out of time or fails lets the command through. The guard has to finish inside its own timeout, and nothing else stands behind it.

Astrolabe's hook answers inside that timeout once it's running, and denies when it fails partway. What's left is a program that's missing or dies as it starts: it gives no answer, so the command runs. `onFailure` is not set in the plugin's manifest, because with no matcher it would block every tool call in every session on a machine where the program is missing, and it is unprobed from a plugin.

## How much hook text reaches the model

**Claude Code** (docs). A hook's `additionalContext`, `systemMessage` and plain output are capped at 10,000 characters, with no setting to raise it. Past the cap the text goes to a file and the model gets the path and a preview of up to 2,000 characters.

**Codex** (docs). Each hook output message is limited to roughly 2,500 tokens by default. `additionalContextLimit` on the command handler sets another limit in approximate tokens, or `0` for the full text. Past the limit the full text is saved under the temp folder and the model gets a head-and-tail preview with the path.

The two limits use different units, so one byte budget does not describe both.

## What a hook receives for a shell command

Both apps document the command as the string `tool_input.command` and the session's working folder as top-level `cwd` (docs). Neither documents a command given as a list, a `cmd` field, or a working-folder field inside `tool_input`.

**Codex** (probe). A captured `PreToolUse` input for a shell command carried `tool_name` "Bash", `tool_input` holding only the string `command`, and top-level `session_id`, `turn_id`, `transcript_path`, `cwd`, `hook_event_name`, `model`, `permission_mode` and `tool_use_id`. When the model set the shell tool's own working folder, that folder arrived as the top-level `cwd`; `tool_input` still held only `command`.

Local evidence: of 226 denials in one Mac's guard log, the 214 that record a tool all came from a tool named `Bash` with a readable command. The `cmd`, `exec_command` and `workdir` shapes the tests model were not observed there or in the probe.

Not checked: the PowerShell tool's input on Windows, and Codex's desktop app.

Codex states that the transcript format is not a stable interface for hooks.

## Registering a marketplace at a different ref

**Claude Code** (probe). With the marketplace `astrolabe` added at `#v0.5.0` and its plugin installed:

- adding the same source again prints "already on disk" and exits 0
- adding the same repository at `#main` fails with exit 1: the source doesn't match the entry in user settings
- the ref is stored in `extraKnownMarketplaces` in the config folder's `settings.json`

Per the docs, `plugin marketplace remove` without `--scope` removes the declaration from every scope; removing the last one uninstalls the marketplace's plugins and deletes their saved data where it can. `plugin marketplace update` and `plugin update` refresh within the configured ref. No documented command moves a marketplace to another ref in place. Source: [plugins CLI reference](https://code.claude.com/docs/en/plugins/cli-reference).

**Codex** (probe). With the marketplace added at `--ref v0.5.0` and its plugin added:

- adding the same source and ref again reports it is already added and exits 0
- adding the same repository with `--ref main` fails: the marketplace "is already added from a different source; remove it before adding this source"
- `plugin marketplace upgrade` refreshes to the latest revision of the configured ref
- after `plugin marketplace remove`, the `[plugins."astrolabe@astrolabe"]` entry was still in `config.toml`

So in both apps a ref change means remove, then add. A same-ref refresh never needs a removal.

## A disposable config folder

**Codex** (probe). `CODEX_HOME` points the CLI at another folder. With only a copy of `auth.json` in it, `codex exec` ran a session and wrote its state there; the real `config.toml` was unchanged, by hash, and gained no entry for the probe folder. Plugin and marketplace commands wrote only to that folder. A `hooks.json` placed there did not run until it was trusted: an unattended run skips untrusted hooks without saying so. Trusting it once in an interactive session with `/hooks` recorded a hash per hook entry in that folder's `config.toml`, and unattended runs then ran it. The hash covers the entry in `hooks.json`, not the script it names: changing the script kept the trust. The CLI also has a flag that runs hooks without recorded trust for one invocation; it was not used.

The real `config.toml` can change during a run for unrelated reasons, such as the desktop app updating itself, so compare it for entries naming the run's folders, not by whole-file hash.

**Claude Code** (probe and docs). `CLAUDE_CONFIG_DIR` points the CLI at another folder. Plugin and marketplace commands wrote only to that folder, and the real marketplace and plugin files were unchanged, by hash. A fresh folder is not signed in (`claude auth status`), and on macOS the sign-in is not a file that can be copied in. The docs give a folder-trust field in the user config file and say an unattended run never shows the trust dialog; they don't say whether such a run records trust.

Not checked: a signed-in session in a disposable Claude Code folder.

## A bare argument to PowerShell

`pwsh` treats a first argument with no parameter name as a script file: `-File` "is the default parameter if no parameters are present but values are present in the command line" ([about_pwsh](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_pwsh)). The page for Windows `powershell.exe` does not state its default.

Not checked: `powershell.exe` on Windows.

## Agent instruction files

**Claude Code** (docs) reads a repository's `AGENTS.md` when no `CLAUDE.md`, `.claude/CLAUDE.md` or `CLAUDE.local.md` exists in the working folder or above; it needs 2.1.277 or later, and a setting can load both. Source: [memory](https://code.claude.com/docs/en/memory).

**Codex** (docs) reads one file per folder from the project root down to the working folder, preferring `AGENTS.override.md`, and stops adding files at 32 KiB combined by default. Source: [AGENTS.md](https://learn.chatgpt.com/docs/agent-configuration/agents-md).
