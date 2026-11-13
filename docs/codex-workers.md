# Independent Codex workers in the app

A worker dispatched from outside the Codex app, such as by a Claude orchestrator, can run in the app from its first real turn and sit under the project it works in. The dispatcher starts it with a short CLI launch turn, opens that thread in the app, and sends the brief with `codex queue`. The app then runs the work where the owner can watch it. This uses existing commands; it needs no Astrolabe service or RPC adapter.

## Why the launch is split

`codex exec` runs the thread on a server of its own. On Windows the desktop app only uses its built-in server, so it doesn't show a thread while an exec run still holds it, and nothing outside the app can start a turn on the app's server. The app's new-chat link only fills in the composer; it doesn't send. So the CLI runs one short turn, the app takes the thread once that turn ends, and every later turn runs in the app.

The launch turn is the step a worker does first anyway: bind to its effort. The guard proof and the brief move to the queued turn, so the proof is made by the app's runtime.

## What was checked

On Windows on 2026-11-12, with CLI 0.166.0 and app package 26.1106.6628.0:

- A launch turn in a plain repository ended in 9 seconds. Opening its link made the app take the thread at once, and a queued task started 10 seconds after it was sent. The queued turn kept the launch's workspace sandbox, write folders, `on-request` approval and automatic review.
- A launch with `--worktree` from the folder of a saved app project made a worktree managed by Codex in about 100 seconds, mostly checkout time. The app recorded the project folder it came from. It did not file the chat in the project by itself. The queued turn named the chat and moved it into the project, and the sidebar then listed it under that project.
- Both chats showed in the app while their queued turns ran, and the end of each turn appeared in the thread's saved session log.

On Windows on 2026-11-11, an earlier worker was opened only after its exec run ended. The app named and pinned it and completed a follow-up sent by `codex queue` without `--resume`. A later `codex exec resume` against the app-owned thread failed with an active-writer conflict.

On macOS on 2026-11-11, with CLI 0.166.0 and app 26.1103.60933 (bundled runtime 0.167.0-alpha.3.2), launch and same-thread queue completion worked. Hooks initially did not run. After the owner reviewed the refreshed plugin's hooks, the first app-originated turn bound the same thread to its effort and proved its guard; a later queued turn kept the binding and proof.

Not checked: the split launch on macOS, a real effort's binding and guard proof inside the app's turn on Windows, app restarts mid-task, several workers launched at once, and a missing or signed-out app. An installed app package version does not establish which runtime served every turn.

## Start a worker from the Codex app

When dispatching a new chat from the app, use `list_projects` to match the effort's repository to its saved project on the execution host. Check the project's folder before starting work. Pass that project and the effort's number and title to `create_thread`, with the model and effort set as for a CLI launch where the tool takes them. Whether it does, and whether the guard sees the call, hasn't been checked, so this path isn't enforced. Once creation finishes, check the title, project and working folder of the new chat. Select the worktree environment when the assignment calls for a new managed worktree. That path needs no launch turn.

On pickup in an existing desktop chat, activation asks the agent to call `set_thread_title`. Make the call on first pickup even when the effort is still being planned. The app can shorten long titles. A later pickup of the same effort preserves a manual rename.

## Start a worker from another agent

Each step below has a check. Wait for the check, not for a fixed time; the timings above are one machine's.

### 1. Run the launch turn

Start from the saved project's registered repository folder and let Codex make the worktree with `--worktree`, so the app can file the chat under that project. Put the brief under the notes' `.handoffs/<initiative>/` folder first. Use the [sandbox and notes setup](codex.md) for permissions. In PowerShell, replace every placeholder:

```powershell
$launch = 'Run astrolabe activate --effort <effort-number> --worker --model gpt-6.1-sol --reasoning medium alone. Reply with the first line of its output and stop. Do not run the guard proof yet and do no other work.'
'' | codex exec -m gpt-6.1-sol -c model_reasoning_effort=medium --worktree -C '<saved-project-repository-path>' --add-dir '<absolute-notes-path>' --approve-for-me --json $launch
```

Every launch names its model and reasoning effort: Sol at medium (`-m gpt-6.1-sol -c model_reasoning_effort=medium`) unless the work needs more, and then the reason goes in the brief. A launch that names neither inherits `config.toml`'s, which can be the costliest model at a high effort, so Astrolabe's guard refuses it ([worker model and effort](hosts.md#worker-model-and-effort)). A profile alone (`-p`) doesn't count, since what it sets can't be read from the command. The activate line names the same two, and binding is refused when Codex reports a different model.

Not checked: whether turns the app runs after `codex queue` keep the launch's model and effort, or take the app's own selection. Until that's checked, look at the thread's model in the app after the first queued turn.

The empty pipeline closes stdin; in Bash or zsh use `< /dev/null`. `--approve-for-me` selects automatic approval review with the workspace sandbox; use it only within the approved authority. It does not bypass approvals or hook trust. See the official [CLI reference](https://learn.chatgpt.com/docs/developer-commands?surface=cli).

Check: the process exits 0 and the JSON stream holds `thread.started` with the thread identifier and a completed turn whose reply says the effort is bound. Keep the identifier, effort and report location in the dispatch record. If activation says the hook didn't take the command, stop: see [when hooks need review](#when-hooks-need-review).

If the dispatcher's sandbox prevents initialization of normal session storage, request host approval for the launch itself. Keep the worker's workspace sandbox and normal approval review; do not bypass them to make the launch succeed.

A worker that needs an already prepared checkout can take `-C <prepared-worktree>` without `--worktree`. It keeps its branch and files and shows in the app the same way, but the app can't place it under a project: a move returns `The task's source project could not be safely determined.` In the PC check a chat outside any project showed under the app's recent chats; a prepared worktree's chat is expected to do the same, but that wasn't checked. The owner can move it by hand. Prefer doing the preparation inside a worktree that Codex made. Don't run in the main checkout just to get the grouping.

### 2. Open it in the app

```powershell
Start-Process -FilePath 'codex://threads/<real-thread-id>'
```

The official [deep-link reference](https://learn.chatgpt.com/docs/reference/commands#deep-links) documents this link for an existing local chat. Open it only after the launch process has exited; an exec run and the app must never write the same thread at once. An accepted opening request alone proves nothing. The next step's check covers it.

### 3. Queue the brief

```sh
codex queue --thread <real-thread-id> --message "<queued brief>"
```

Start the queued brief with the app-side steps, then the work:

> Run the allowed control and the denied probe that activation printed, each alone, then astrolabe activate --status. If it doesn't name your effort and say the guard is proven, stop and report that; do no other work. Set this chat's title to "<number and effort title>". Use `list_projects` to find "<project>" on this host and `move_thread_to_project` to file this chat there; report the exact error if the move fails. Then read <absolute-brief-path> and carry out its approved scope. Save your effort and write the requested report.

Proof is recorded per session, so a proof made in the launch turn would still read as proven in the app and wouldn't test the app's hooks. That's why the launch turn only binds.

Check: the thread's session log gains a turn start after the launch turn's end. Find the log by the thread identifier under the Codex home's `sessions` folder (`rollout-*-<thread-id>.jsonl`). If no new turn starts within a few minutes, open the link once more. If still nothing starts, report it with the queued message's identifier; the app may not own the thread or the queue may have stalled. Don't queue the brief again or start a replacement worker, since the queued message can still run later.

The session log is Codex's saved transcript, not a documented interface for other tools. Read it for these two events only, and treat a format change as a reason to fall back to the report.

### 4. Collect the result

The worker's turn has ended when the log records a turn end after the queued turn's start; the launch turn's end comes earlier and doesn't count. Then read the worker's report and check the work itself. A completed turn proves the turn ran, not that the work is right.

## When hooks need review

Treat activation and guard proof as a gate before ordinary task work. If activation says the plugin's hook did not take the command, stop and report an unbound worker. An installed, enabled plugin or a returned thread identifier is not proof that its hooks ran.

A plugin refresh can invalidate hook content trust. In the Mac check, unattended execution supplied no warning while hooks were skipped, despite saved trust entries. The same thread recovered after the owner reviewed the current hooks in the app. Use the normal review flow, including `/hooks` in the CLI; do not rewrite trust entries or bypass review. See the official [hook trust reference](https://learn.chatgpt.com/docs/hooks#review-and-trust-hooks).

After review, continue the retained thread and require binding and guard proof again before work. A failed activation is not a reason to duplicate the task.

## Follow up

While the app owns the worker, send follow-ups to its identifier with `codex queue`, as in step 3, and check them the same way. The official [queue reference](https://learn.chatgpt.com/docs/developer-commands?surface=cli#codex-queue) distinguishes queueing from optional `--resume`, whose failure can still exit successfully. Queue success proves acceptance only.

Do not run `codex exec resume` against an app-owned thread. That produced an active-writer conflict in the PC check. Report the conflict and use the queue; do not retry by starting another worker.

If the app isn't available, the worker can stay CLI-only: send the brief in the launch run itself and follow up with sequential resume from its original worktree, as in [launch and resume](codex.md#launch-and-resume). It then shows in the app only after it exits and its link is opened. For example, in PowerShell:

```powershell
Set-Location '<original-absolute-worktree-path>'
'' | codex exec resume -m gpt-6.1-sol -c model_reasoning_effort=medium '<real-thread-id>' 'Run the remaining approved checks and update your report.' --json
```

These are independent sessions. Filing under a project or pinning doesn't establish native parent-child nesting. Do not fabricate source, origin or parent metadata, or edit provider databases or app state to force grouping.
