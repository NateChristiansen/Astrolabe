# Independent Codex workers in the app

An independently dispatched CLI worker can be opened as its actual thread in the Codex app. Keep its real thread identifier and worktree, then use queue for external follow-ups while the app owns it. This workflow uses existing commands; it needs no Astrolabe service or RPC adapter.

## What was checked

On Windows on 2026-11-11, with CLI 0.166.0 and installed app package 26.1106.6628.0, an independent worker activated its own effort and proved its guard. The app opened its actual transcript, named and pinned it, and completed a follow-up sent by external `codex queue` without `--resume`. The default app history listing had omitted it before opening and pinning.

Mac behavior, app restart, concurrent independent effort separation and missing-app or launch-failure acceptance remain unverified. The installed app package version does not establish which runtime served the app. Treat the steps below as the observed PC workflow, not cross-platform acceptance.

## Launch and retain the worker

Prepare a branch worktree and a brief under the notes' `.handoffs/<initiative>/` folder. Use the [sandbox and notes setup](codex.md) for permissions. For example, in PowerShell, replace every placeholder before running:

```powershell
$brief = 'First run astrolabe activate --effort <effort-number> --worker alone. Then read <absolute-brief-path> and carry out its approved scope. Save your effort and write the requested report.'
'' | codex exec -C '<absolute-worktree-path>' --add-dir '<absolute-notes-path>' --approve-for-me --json $brief
```

The empty pipeline closes stdin. In Bash or zsh, use the equivalent command with `< /dev/null`. The example selects automatic approval review with the workspace sandbox; use it only within the approved authority. It does not bypass approvals or hook trust. See the official [CLI reference](https://learn.chatgpt.com/docs/developer-commands?surface=cli) for these options.

If the dispatcher's sandbox prevents initialization of normal session storage, request host approval for the launch itself. That was required in the PC check. Keep the worker's workspace sandbox and normal approval review; do not bypass them to make the launch succeed. If approval is unavailable, report the launch limitation.

Retain the actual thread identifier returned in the JSON event stream, original absolute worktree, effort and report location in the dispatch record. Capture the final result and process outcome too. Use a persisted run, not `--ephemeral`; do not reconstruct identity from a display name or start a replacement because the app did not list it.

The worker's first command must be this command alone:

```sh
astrolabe activate --effort <effort-number> --worker
```

It then runs the exact allowed control and denied probe printed by activation, each alone, followed by `astrolabe activate --status`. Claim guard protection only when that worker's status says proven. The dispatcher's proof does not cover it. The worker saves its own effort; the dispatcher handles the journal, review and closing. Notes write access permits the brief and report, not project Git operations.

## Open the actual thread

The official [deep-link reference](https://learn.chatgpt.com/docs/reference/commands#deep-links) documents `codex://threads/<thread-id>` for an existing local chat. On Windows:

```powershell
Start-Process -FilePath 'codex://threads/<real-thread-id>' -WindowStyle Hidden
```

An accepted OS opening request alone is not a pass. Inspect the app's transcript and confirm the dispatched task, worktree and bound effort belong to the same identifier. An app-hosted dispatcher can use its native thread navigation when available.

Use app-native naming and pinning where available to make that same thread discoverable. App-hosted tools `set_thread_title` and `move_thread_to_sidebar_section` worked in the PC check, and a later app listing included the pinned worker. These tools are capabilities of that app session, not shell commands or a proved external pinning API.

App project membership and execution folder are separate. Opening the worker did not assign it to an app project. Keep its actual worktree even if an app project's saved folder differs. An external app-server could not read the desktop's existing project identifier in this check; do not assume its experimental project and section methods can organize the desktop sidebar.

These are independent sessions. Pinning does not establish native parent-child nesting, and no independently bound native nesting was proved. Do not fabricate source, origin or parent metadata, or edit provider databases to force grouping.

## Follow up and collect results

While the app owns the opened worker, send external follow-ups to its retained identifier:

```sh
codex queue --thread <real-thread-id> --message "Run the remaining approved checks and update your report."
```

The official [queue reference](https://learn.chatgpt.com/docs/developer-commands?surface=cli#codex-queue) distinguishes queueing from optional `--resume`, whose failure can still exit successfully. The PC app-owned check used queue without `--resume`. Queue success proves acceptance only: observe the same worker's completed turn and inspect its report or result. The PC check collected the exact completed response through the app's native wait tool.

Do not run sequential `exec resume` against an app-owned writer. That produced an active-writer conflict in the PC check. Report the conflict and use the app or its queue path; do not retry by starting another worker.

For an idle worker kept CLI-only, sequential resume remains the explicit fallback. Invoke from its original worktree and retain its permission settings, as described in [launch and resume](codex.md#launch-and-resume). For example, in PowerShell:

```powershell
Set-Location '<original-absolute-worktree-path>'
'' | codex exec resume '<real-thread-id>' 'Run the remaining approved checks and update your report.' --json
```

Use this only when no app or other process owns the worker. A separate app-server's local idle status does not establish global writer availability. If app opening is unavailable or fails, record that result and keep the same identifier and CLI-only path where usable; do not claim app visibility or automatically duplicate the task.
