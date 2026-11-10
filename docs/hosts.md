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

### Replacing an installed copy at the same version

**Claude Code** (probe, 2.1.316). With the plugin installed and its cached copy made to differ from the marketplace's at the same version, and the install record's commit changed to an older one:

- `plugin update astrolabe@astrolabe` prints that it is already at the latest version, exits 0 and leaves the copy as it was
- `plugin install astrolabe@astrolabe` prints that it is already installed, exits 0 and leaves the copy as it was
- `plugin uninstall` then `plugin install` replaces the copy

**Codex** (probe, 0.166.0). `plugin add astrolabe@astrolabe` over an installed copy that differs replaces it and exits 0, with no removal first.

So a stale copy at the same version needs an uninstall first in Claude Code only.

### What a removal keeps

**Claude Code** (probe, 2.1.316).

- `plugin uninstall --keep-data` leaves the plugin's folder under `plugins/data/`, clears its `enabledPlugins` entry and keeps the marketplace; a later `plugin install` at user scope sets it enabled again
- `plugin marketplace remove astrolabe --scope user` with the plugin still installed also uninstalls the plugin and deletes its data folder
- the same removal after `plugin uninstall --keep-data` leaves the data folder in place
- removing a marketplace that isn't registered exits 1
- `plugin marketplace list --json` gives a Git source as `url` and `ref`, a GitHub shorthand as `repo`, and a folder as `path`
- a `file://` URL is refused as a marketplace source

**Codex** (probe, 0.166.0).

- `plugin marketplace remove` leaves the installed copy under `plugins/cache` as well as the `[plugins."astrolabe@astrolabe"]` entry with its `enabled` value; adding the marketplace at another ref and `plugin add` then works without `plugin remove`
- adding at a ref that doesn't exist fails with Git's checkout error and leaves no marketplace entry
- a folder source is recorded as `source_type = "local"` with the folder as `source`

Not checked: whether Codex still runs the plugin's hooks from the installed copy while its marketplace is removed, and Claude Code's removal when the marketplace is declared in project or local scope.

### Sign-in

Plugin and marketplace commands ran in a fresh `CLAUDE_CONFIG_DIR` that was not signed in (probe, 2.1.316), and in a `CODEX_HOME` with no `auth.json` (probe, 0.166.0).

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
