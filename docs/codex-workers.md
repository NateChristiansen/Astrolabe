# Independent Codex workers in the app

An independently dispatched CLI worker can be opened as its actual thread in the Codex app. Keep its real thread identifier and worktree, then use queue for external follow-ups while the app owns it. This workflow uses existing commands; it needs no Astrolabe service or RPC adapter.

## What was checked

On Windows on 2026-11-11, with CLI 0.166.0 and installed app package 26.1106.6628.0, an independent worker activated its own effort and proved its guard. The app opened its actual transcript, named and pinned it, and completed a follow-up sent by external `codex queue` without `--resume`. The default app history listing had omitted it before opening and pinning.

On macOS on the same day, with CLI 0.166.0 and app 26.1103.60933 (bundled runtime 0.167.0-alpha.3.2), launch and same-thread queue completion worked. Hooks initially did not run. After the owner reviewed the refreshed plugin's hooks, the first app-originated turn bound that same thread to its effort and proved its guard; a later queued turn retained the binding and proof. No replacement thread was created.

Mac sidebar display, app restarts, broader concurrent-worker behavior and missing-app acceptance remain unverified. An installed app package version does not establish which runtime served every turn. The Mac result includes a recovered hook-trust failure, not a promise that every initial CLI launch is already guarded.

## Start a worker from the Codex app

When dispatching a new chat from the app, use `list_projects` to match the effort’s repository to its saved project on the execution host. Check the project’s folder before starting work. Pass that project and the effort’s number and title to `create_thread`. Once creation finishes, check the title, project and working folder of the new chat. Select the worktree environment when the assignment calls for a new managed worktree. For a worker that needs an already prepared checkout, use the CLI handoff below.

On pickup in an existing desktop chat, activation asks the agent to call `set_thread_title`. Make the call on first pickup even when the effort is still being planned. The app can shorten long titles. A later pickup of the same effort preserves a manual rename.

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

## When hooks need review

Treat activation and guard proof as a gate before ordinary task work. If activation says the plugin's hook did not take the command, stop and report an unbound worker. An installed, enabled plugin or a returned thread identifier is not proof that its hooks ran.

A plugin refresh can invalidate hook content trust. In the Mac check, unattended execution supplied no warning while hooks were skipped, despite saved trust entries. The same thread recovered after the owner reviewed the current hooks in the app. Use the normal review flow, including `/hooks` in the CLI; do not rewrite trust entries or bypass review. See the official [hook trust reference](https://learn.chatgpt.com/docs/hooks#review-and-trust-hooks).

After review, continue the retained thread and require binding and guard proof again before work. In the Mac check, the first bound turn came from the app; queue control was verified afterwards, not as the first post-review turn. App-owned threads use queue; idle CLI-only threads use sequential resume. A failed activation is not a reason to duplicate the task.

## Open the actual thread

The official [deep-link reference](https://learn.chatgpt.com/docs/reference/commands#deep-links) documents `codex://threads/<thread-id>` for an existing local chat. On Windows:

```powershell
Start-Process -FilePath 'codex://threads/<real-thread-id>' -WindowStyle Hidden
```

An accepted OS opening request alone is not a pass. Inspect the app's transcript and confirm the dispatched task, worktree and bound effort belong to the same identifier. An app-hosted dispatcher can use its native thread navigation when available.

### Name and file the opened worker

A CLI worker can receive the rename instruction before it has desktop tools. At the first handoff, the dispatcher checks the actual chat. An app-hosted dispatcher can use the tools directly. A Claude dispatcher can wait for `codex exec` to exit, open the existing chat, then send one message to that chat with `codex queue --thread <real-thread-id> --message <request>`. Include the real effort name and intended project in the request. For example:

> Check your bound effort and working folder. Set this chat’s initial title to “<number and effort title>” if it still shows the launch prompt. Use `list_projects` to find “<project>” on this host and `move_thread_to_project` to file this chat there. Read back the title, project membership and working folder. Report the exact error if the move fails.

Collect the completed reply. Check that the same chat is listed under the destination project and still runs in the assigned worktree. Preserve a title the owner set. Pinning is a separate sidebar action.

On Windows, a real Claude dispatch completed that queued turn with the desktop tools and named its original worker. Its project move returned `The task's source project could not be safely determined.` The worker ran in a separate Git worktree and the destination project's saved folder pointed to another checkout. The same error occurred with an existing CLI worker in a sibling Git worktree of a repository whose main checkout was registered with the correct project. The original worktree and effort binding survived. Report unresolved placement in this case. If the app specifically requires additional folder access, the owner can move the chat in the sidebar. On macOS, same-chat queue delivery has passed, but naming and project placement from the queued turn still need verification.

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
