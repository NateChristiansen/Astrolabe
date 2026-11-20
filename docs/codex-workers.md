# Codex workers in the app

An agent outside the Codex app, such as a Claude orchestrator, starts a Codex worker in four steps: a short CLI launch turn that only binds the effort, opening that thread in the app, queueing one sentence that points at the brief, and collecting the result. The app then runs the work where the owner can watch it and files the chat under its project. This is the one launch form; it passed first try twice on macOS and once on Windows on 2026-11-17, with CLI 0.166.0, and every time in six live launches on 2026-11-19 and 20, two each from zsh, Windows PowerShell 5.1 and Git Bash.

## Why it's split

`codex exec` runs a thread on a server of its own, and the desktop app doesn't show a thread while an exec run holds it. Nothing outside the app can start a turn on the app's server, and the app's new-chat link only fills in the composer. So the CLI runs one short turn, the app takes the thread when that turn ends, and every later turn runs in the app.

The launch turn only binds. The guard proof is made in the first queued turn, so it proves the app's hooks, not the CLI's: proof is recorded per session, and a proof from the launch turn would still read as proven in the app.

## Before you start

- Run `astrolabe codex-hooks`. Launch only when it exits 0; otherwise Codex would skip the hooks unseen ([when hooks need review](#when-hooks-need-review)).
- Write the brief under the notes' `.handoffs/<initiative>/` folder. Its path must have no spaces and no quote characters, because the queued sentence carries it unquoted inside single quotes. If it can't, copy the brief to such a path in the hand-off folder.
- Begin the brief with the app-side steps, then the work:

  > Run the allowed control and the denied probe that activation printed, each alone, then `astrolabe activate --status`. If it doesn't name your effort and say the guard is proven, stop and report that; do no other work. Set this chat's title to "<number and effort title>". Use `list_projects` to find "<project>" on this host and `move_thread_to_project` to file this chat there; report the exact error if the move fails. Then carry out the scope below, save your effort and write the requested report.

  The brief is a file the worker reads, so its quotes and newlines arrive as written in every shell.
- Use the [sandbox and notes setup](codex.md) for permissions.

## 1. Run the launch turn

Run it from the saved app project's registered repository folder with `--worktree`, so Codex makes the worktree and the app can file the chat under that project. Replace every placeholder. In Bash or zsh:

```sh
codex exec -m gpt-6.2-sol -c model_reasoning_effort=medium --worktree -C '<project-folder>' --add-dir '<notes-folder>' --approve-for-me --json 'Run astrolabe activate --effort <n> --worker --model gpt-6.2-sol --reasoning medium alone. Reply with the first line of its output and stop. Do not run the guard proof yet and do no other work.' < /dev/null
```

In PowerShell (5.1 or 7), the same command with stdin closed by an empty pipeline instead:

```powershell
'' | codex exec -m gpt-6.2-sol -c model_reasoning_effort=medium --worktree -C '<project-folder>' --add-dir '<notes-folder>' --approve-for-me --json 'Run astrolabe activate --effort <n> --worker --model gpt-6.2-sol --reasoning medium alone. Reply with the first line of its output and stop. Do not run the guard proof yet and do no other work.'
```

Keep stdin closed: an open stdin leaves `codex exec` waiting forever. It may still print "Reading additional input from stdin..."; that's harmless. Every argument is in single quotes and none holds a double quote, so no shell splits it.

GPT-6.2 Sol at medium is for routine work. For demanding work use `-m gpt-6.2-astra`, give the reason in the brief, and name the same model and effort in the activate line. The guard refuses a launch that doesn't name both, and binding is refused when Codex reports a different model ([worker model and effort](hosts.md#worker-model-and-effort)). A profile (`-p`) alone doesn't count. `--approve-for-me` selects automatic approval review with the workspace sandbox; use it only within the approved authority. If the dispatcher's own sandbox blocks the launch from starting its session storage, request host approval for that one launch.

With `--worktree` the turn can take a few minutes, mostly checkout time.

Check: the process exits 0, and the JSON stream holds `thread.started` with the thread id and a completed turn whose reply says the effort is bound. Record the thread id, effort and report location. If activation says the hook didn't take the command, stop: see [when hooks need review](#when-hooks-need-review).

## 2. Open it in the app

Only after the launch process has exited, since an exec run and the app must never write one thread at once. On macOS:

```sh
open 'codex://threads/<thread-id>'
```

In PowerShell:

```powershell
Start-Process 'codex://threads/<thread-id>'
```

In Git Bash on Windows:

```sh
start 'codex://threads/<thread-id>'
```

The [deep-link reference](https://learn.chatgpt.com/docs/reference/commands#deep-links) documents this link. An accepted open proves nothing; step 3's check covers it.

## 3. Queue the brief

The same line in every shell:

```sh
codex queue --thread '<thread-id>' --message 'Read <absolute-brief-path> and follow it.'
```

Don't put the brief's text in `--message`. The command takes no file or stdin, and Windows PowerShell 5.1 passes an argument's inner double quotes to a program unescaped, which split a brief on 2026-11-17.

Check: the thread's session log gains a turn start after the launch turn's end. Find it by thread id under the Codex home's `sessions` folder (`rollout-*-<thread-id>.jsonl`). If no turn starts within a few minutes, open the link once more. If still nothing starts, report it with the queued message's id. Don't queue again or start a replacement: the queued message can still run later. The session log is Codex's transcript, not a documented interface; read it for these two events only.

## 4. Collect the result

The worker's turn has ended when the log records a turn end after the queued turn's start. Then read its report and check the work itself; a finished turn proves the turn ran, not that the work is right.

## Follow-ups

Send them with `codex queue` to the same thread, in the form of step 3, and check them the same way. Don't add `--resume`, whose failure still exits 0, and never run `codex exec resume` against a thread the app owns: that gave an active-writer conflict on Windows. Report a conflict instead of starting another worker.

## When hooks need review

Activation and the guard proof are a gate before any task work. If activation says the plugin's hook didn't take the command, stop and report an unbound worker. An installed plugin or a returned thread id doesn't prove its hooks ran.

Codex ties hook trust to a fingerprint of the whole installed plugin folder, so a plugin refresh that changes any file there, a skill's text included, drops it, and an unattended run skips the hooks without a warning. The board says so under Needs you, and `astrolabe codex-hooks` exits nonzero. The owner reviews the current hooks in the app, or with `/hooks` in the CLI; don't rewrite trust entries or bypass review ([hook trust reference](https://learn.chatgpt.com/docs/hooks#review-and-trust-hooks)). Then queue to the same thread and require binding and the guard proof again. A failed activation isn't a reason to start a duplicate.

## Not checked

App restarts mid-task, several workers launched at once, and a missing or signed-out app. Workers filed under one project are independent sessions, not nested ones; don't edit app state to force grouping.
