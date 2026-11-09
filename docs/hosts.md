# Host app behavior Astrolabe relies on

Astrolabe runs inside agent apps it doesn't control. This page records what those apps do at the points the program depends on, where each fact comes from, and what hasn't been checked. Cite it instead of assuming; recheck a fact when the app's version moves.

Checked on 2026-11-09 with Claude Code 2.1.316 and Codex CLI 0.166.0 on macOS. "Docs" means the app's published reference on that date. "Probe" means the behavior was run in a disposable config folder and observed.

## A hook that times out or fails

**Claude Code** (docs). A `PreToolUse` command hook that times out, crashes, fails to start or prints invalid JSON does not block the tool call: the call continues through the normal permission flow. Exit code 2, or a deny decision on exit 0, blocks. Setting `"onFailure": "block"` on a command hook makes a failed or timed-out hook block instead; it needs 2.1.295 or later. Plugin hooks are documented as the same shape as settings hooks, with no stated restriction on `onFailure` or `timeout`. Sources: [hooks reference](https://code.claude.com/docs/en/hooks), [plugin manifest reference](https://code.claude.com/docs/en/plugins/manifest-reference).

Command hooks default to a 600-second timeout. `UserPromptSubmit` defaults to 30 seconds, and `SessionEnd` hooks share 1.5 seconds unless a longer per-hook timeout is set.

**Codex** (docs). Hooks default to 600 seconds; `SessionEnd` and `Interrupt` default to 1 second and allow at most 3. Exit code 2 or a deny decision blocks. The docs do not say whether the tool call proceeds when a local command hook times out or fails. Source: [hooks](https://learn.chatgpt.com/docs/hooks).

Not checked: `onFailure` set from a plugin's `hooks.json` in a live Claude Code session, and any Codex timeout or failure behavior.

So the guard must not rely on either app to block when the hook runs out of time. It has to finish inside its own timeout.

## How much hook text reaches the model

**Claude Code** (docs). A hook's `additionalContext`, `systemMessage` and plain output are capped at 10,000 characters, with no setting to raise it. Past the cap the text goes to a file and the model gets the path and a preview of up to 2,000 characters.

**Codex** (docs). Each hook output message is limited to roughly 2,500 tokens by default. `additionalContextLimit` on the command handler sets another limit in approximate tokens, or `0` for the full text. Past the limit the full text is saved under the temp folder and the model gets a head-and-tail preview with the path.

The two limits use different units, so one byte budget does not describe both.

## What a hook receives for a shell command

Both apps document the command as the string `tool_input.command` and the session's working folder as top-level `cwd` (docs). Neither documents a command given as a list, a `cmd` field, or a working-folder field inside `tool_input`.

Local evidence: of 226 denials in one Mac's guard log, the 214 that record a tool all came from a tool named `Bash` with a readable command. The `cmd`, `exec_command` and `workdir` shapes the tests model were not observed there.

Not checked: a captured hook input from a live Codex session, and the PowerShell tool's input on Windows.

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

**Codex** (probe). `CODEX_HOME` points the CLI at another folder. With only a copy of `auth.json` in it, `codex exec` ran a session and wrote its state there; the real `config.toml` was unchanged, by hash, and gained no entry for the probe folder. Plugin and marketplace commands wrote only to that folder. A `hooks.json` placed there did not run: hooks need trust recorded in that folder's `config.toml`, and an unattended run skips untrusted hooks without saying so. The CLI has a flag that runs hooks without recorded trust for one invocation; it was not used.

**Claude Code** (probe and docs). `CLAUDE_CONFIG_DIR` points the CLI at another folder. Plugin and marketplace commands wrote only to that folder, and the real marketplace and plugin files were unchanged, by hash. A fresh folder is not signed in (`claude auth status`), and on macOS the sign-in is not a file that can be copied in. The docs give a folder-trust field in the user config file and say an unattended run never shows the trust dialog; they don't say whether such a run records trust.

Not checked: a signed-in session in a disposable Claude Code folder, and trusted hooks in a disposable Codex folder.

## A bare argument to PowerShell

`pwsh` treats a first argument with no parameter name as a script file: `-File` "is the default parameter if no parameters are present but values are present in the command line" ([about_pwsh](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_pwsh)). The page for Windows `powershell.exe` does not state its default.

Not checked: `powershell.exe` on Windows.

## Agent instruction files

**Claude Code** (docs) reads a repository's `AGENTS.md` when no `CLAUDE.md`, `.claude/CLAUDE.md` or `CLAUDE.local.md` exists in the working folder or above; it needs 2.1.277 or later, and a setting can load both. Source: [memory](https://code.claude.com/docs/en/memory).

**Codex** (docs) reads one file per folder from the project root down to the working folder, preferring `AGENTS.override.md`, and stops adding files at 32 KiB combined by default. Source: [AGENTS.md](https://learn.chatgpt.com/docs/agent-configuration/agents-md).
