# Codex sessions with a workspace sandbox

Astrolabe's effort settings describe what an agent may do. Codex's sandbox separately controls which files a command can write. An effort with edit authority still needs host permission for a local Git commit.

## Set up once

After registering your notes, preview the host configuration:

```sh
astrolabe setup codex
```

The preview names the config file, notes checkout, approval settings and managed permission profile. It writes nothing. To save it and check the notes grant from a project checkout:

```sh
astrolabe setup codex --apply --check
```

Setup uses the registered notes folder and keeps the workspace sandbox's Git protection. It preserves an existing approval reviewer; otherwise requests go to you. For unattended workers, explicitly choose automatic review:

```sh
astrolabe setup codex --reviewer auto_review
astrolabe setup codex --reviewer auto_review --apply --check
```

Automatic review may deny an action and uses additional model calls. It does not extend an effort's authority.

Rerunning a correct setup leaves the config, ownership record and backups unchanged. If your registered notes move, rerun setup: it replaces only the notes root it previously added. It preserves unrelated settings and comments, backs up a changed existing config, and refuses a managed profile edited outside setup. Supported legacy workspace settings are migrated into the profile, including extra writable roots, network access and temporary-folder exclusions. Other sandbox modes or settings that cannot be safely preserved are reported without replacing the config.

Start a fresh session after applying. In the desktop app select **Custom (config.toml)**. A chat's selected permission mode, project configuration or managed requirements can override the saved defaults. Setup doesn't change a running chat or add project trust entries. Review and trust Astrolabe's installed hooks yourself as described in [installation](install.md#2-install-the-plugin-in-your-agent-app).

`--check` uses the installed CLI's native sandbox to create and remove a handoff probe and confirm that the current project's Git metadata stays protected. Run it from a project checkout; outside one, it reports that Git protection wasn't checked. It tests the generated named profile with managed requirements included, not the desktop app's selected mode or a real approval request. `--config <file>` selects another config destination; it is mainly for isolated tests with a separate Astrolabe state folder.

## Launch and resume

Create a worker's branch worktree before starting it. With setup selected, launch from that worktree as usual. For a one-off CLI launch without persistent setup:

```sh
codex exec -m gpt-6.1-sol -c model_reasoning_effort=medium --approve-for-me --cd /path/to/worktree --add-dir /path/to/notes "<task>"
```

Replace the paths with yours. The model and effort are required: the guard refuses a launch that names neither, as it would inherit `config.toml`'s ([worker model and effort](hosts.md#worker-model-and-effort)). The same flags work in PowerShell with Windows paths. `--approve-for-me` selects workspace-write and automatic approval review; don't combine it with `--sandbox`. Close stdin when dispatching from a shell tool (`< /dev/null` in Bash or zsh).

Resume from the same worktree and retain its permission settings. A different working directory can leave the worker's files outside its writable workspace. An explicit `--sandbox` selects the legacy sandbox settings instead of the named profile, so use `--add-dir` as well if overriding it.

## Briefs, reports and Git

Write briefs and reports at the requested location under the notes' `.handoffs/<initiative>/` folder. The notes grant covers these files. Use Astrolabe commands for effort records and the journal; their hook already saves and syncs notes without a writable notes grant. Workers still save only their own effort and leave the journal and closing to their dispatcher. The dispatcher also commits and syncs directly written handoff files in the notes checkout, with host approval when needed.

Git metadata stays protected even in a writable checkout. For an authorized branch, commit or local integration, request host approval for that command when needed. Making the notes folder writable doesn't grant project Git access. Adding `.git` as a writable root isn't the workflow.

If approvals are unavailable or denied, finish the allowed edits and checks. Report the worktree, branch, changed files, checks and the exact Git step left for the dispatcher. Save built and checked work in review waiting for that step; use blocked when the restriction prevents the build or checks themselves. Don't mark a worker done or claim a commit or integration that didn't happen. An ordinary autonomous effort can finish only when its required work is actually complete.

If the handoff folder isn't writable, return the report in chat with its intended destination for the dispatcher to file. Don't silently move it to the notes root. The dispatcher must have permission to perform any remaining step; dispatch itself doesn't authorize a push or landing.

## Check the setup

Before dispatching real work, use a disposable repo and notes checkout. Confirm that a session can write a brief and report under `.handoffs`, commit an authorized change through host approval, and integrate a prepared worker branch. Check the files and Git history from outside the session. A run with approvals disabled tests the fallback, not autonomous Git completion.

The unattended workflow and setup sandbox check were checked with Codex CLI 0.166.0 on macOS. The desktop mode and Windows runtime have not been exercised. Other hosts may apply additional restrictions. See the official [CLI reference](https://learn.chatgpt.com/docs/cli/reference) and [approvals and security guide](https://learn.chatgpt.com/docs/agent-approvals-security).
