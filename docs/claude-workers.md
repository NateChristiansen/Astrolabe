# Claude Code workers

A Claude Code worker has to be a session the owner can see and steer in the Claude desktop app's sidebar, never a run only its dispatcher sees. A run the owner can't see, they can't stop or redirect, and they learn what it did only from the dispatcher's summary. On one project a dispatcher ran a worker all day as background CLI runs and never handed it to the app, so the owner couldn't follow it.

The route depends on where the dispatcher runs:

- **A dispatcher in the Claude desktop app** starts the worker with the app's own session tool ([from the desktop app](#from-the-desktop-app)). The worker is in the sidebar from its first turn, with no CLI run and no handoff.
- **Any other dispatcher**, such as a Codex dispatcher or Claude Code in a terminal, launches a persisted CLI session and hands it to the desktop app as soon as its first run ends ([from anywhere else](#from-anywhere-else)).

Either way the worker's first turn starts with `astrolabe activate --effort <n> --worker --model <model> --reasoning <effort>` as its own first command, naming the model and effort it was started with, then has the worker prove its own guard with the probe that activation prints. The proof is per session: the dispatcher's proof doesn't cover the worker, and the worker's doesn't cover a subagent. The worker saves only its own effort and writes its report where the brief says. The dispatcher reviews and journals, as in [usage](usage.md). Whoever sees the effort's finish line met closes it, the worker included, never before that event.

## From the desktop app

1. Create the worker's branch worktree.
2. Start the worker with the app's session tool (`start_session`), with that worktree as its folder (`cwd`), a fresh context, and its model and effort: Opus at medium unless the work needs more, and then the reason goes in the brief. The guard refuses a start that names no model or effort, and a fork, which keeps the dispatcher's ([worker model and effort](hosts.md#worker-model-and-effort)). The app starts a session at or below the dispatcher's own model and effort; for more, use the [CLI route](#from-anywhere-else), which isn't capped.
3. Wait for its arrival: its first save on its effort, or its first report for a worker told not to save ([asking once](hosts.md#asking-once-and-seeing-each-worker-arrive)). The app relays each turn's report to the dispatcher on its own.
4. Send follow-ups through the app's messaging to that session's id. Never resume it with `claude --print --resume`: the app holds the session, and a second writer on one session wasn't tested.

The app starts a session only on the owner's own words. Its one documented exception is a turn it opens when sessions this one started have reported, right after the owner's own turn or another such turn: there, a step the owner already asked to have in its own session may start, in the same project ([asking once](hosts.md#asking-once-and-seeing-each-worker-arrive)). That exception is unverified. On Windows on 2026-11-24 a start was refused in a turn opened by a task notification from the dispatcher's own read-only subagent, which isn't a report from a worker session it started; later that night, after the owner's own message, starts worked. So ask the owner once at plan approval for every start the plan needs, try a start in a worker-report turn only for a step the owner already asked for, and when the app refuses one, tell the owner which worker waits and start it in their next turn. A proposal card, which the owner starts with a click, can't carry a folder and starts in the dispatcher's own, so propose one only when that folder is in the worker's repo and the brief has the worker make its worktree before it edits.

The worker renames its own session after its effort with the app's rename tool when it binds.

## From anywhere else

This route was checked with a Codex dispatcher. A Claude Code dispatcher's shell passes its own session environment, including its session id, to a CLI it starts ([host behavior](hosts.md#what-a-session-passes-to-a-session-it-starts)), and a worker launched that way hasn't been tested. A dispatcher in the desktop app uses the app's tool instead.

1. Create the worker's branch worktree.
2. Launch from that worktree, as below, with a short first turn and not the brief: activate, prove the guard, report that, and stop. A launch that carried the whole brief could finish the work before the owner ever saw it. Record its session id and remember the folder.
3. Wait until its process has exited.
4. [Hand it to the desktop app](#hand-it-to-the-desktop-app) before anything else, and confirm the app has it. The handoff moves only an idle session, so this is the first moment it can, and every follow-up sent before it is another run the owner can't see. If it fails, tell the owner with the reason, and keep the worker in the CLI ([when the handoff isn't available](#when-the-handoff-isnt-available)).
5. Check the first turn's report: it bound the intended effort and its guard is proven. If either is missing, hold the brief back and find out why. Otherwise send the brief as the first [follow-up](#follow-ups), once the app has the worker or the owner has been told the handoff failed. Send later follow-ups one at a time.

### Launch

Launch from the worker's worktree, with the notes checkout added for briefs, reports and handoffs:

```sh
claude --print --model opus --effort medium --permission-mode auto --add-dir /path/to/notes --output-format json "<first turn>" < /dev/null
```

Replace the paths with yours. Every launch names its model and effort: Opus at medium unless the work needs more, and then the reason goes in the brief. A launch that names neither inherits the dispatcher's settings, and Astrolabe's guard refuses it ([worker model and effort](hosts.md#worker-model-and-effort)). `--model default` doesn't count, as it restores the inherited model. In PowerShell, pipe an empty string in (`'' | claude --print ...`) to close stdin. Keep the app's normal permission checks; don't use the bypass mode for a worker.

- The session persists by default. Don't pass `--no-session-persistence`, or the session can't be resumed or handed off.
- Record the session id. The JSON result carries it as `session_id`, and `claude agents --json` lists running sessions with their id, folder, name and status. To know it before launch, pass your own UUID with `--session-id <uuid>`.
- The plugin's hook names the session after its effort when the worker binds. That doesn't control the title the desktop app shows: on macOS the app kept the launch name after a handoff. Pass a clear name with `--name <name>` at launch, or rename the session in the app where that's allowed. The name doesn't affect the effort binding, which stays correct either way.
- Remember the folder you launched from. Sessions are stored per project folder, and every later resume or handoff runs from that same folder.

### When the launch says it isn't logged in

A dispatcher whose shell runs in a sandbox can start a worker that exits at once with `Not logged in · Please run /login`. That doesn't by itself mean the account is signed out. In the one macOS test, the stored sign-in was present and the launch still failed inside the sandbox.

- Check whether the account is available without changing anything. Run `claude auth status` from a terminal outside the sandbox. A sign-in there means the account is available. It doesn't prove what blocked the sandboxed launch.
- If the account is available, the dispatcher may request its normal host approval for that one launch, without changing the dispatcher's sandbox settings. If review denies it, stop and report the reason. In the macOS test, approval let the launch succeed.
- In that test the handoff needed approval too. Sandboxed, `claude --desktop --resume` failed with a message that `--desktop` requires signing in with a Claude account, even though the worker had run. Other hosts may differ.
- Don't log in or out, copy credentials, change settings or the environment, bypass trust, or turn the sandbox off to get past it.
- Before authorizing a fresh launch, check that the failed one left no running worker and no transcript under its id. In the macOS test it left neither. Nothing starts a replacement automatically. If you start one, record its new id.

### Hand it to the desktop app

After the worker's process has exited, run this from the worker's launch folder, in a real terminal:

```sh
claude --desktop --resume <session-id>
```

- It needs an interactive terminal. With captured or redirected output it refuses to run (observed on Windows). An agent's shell tool has to give it a pseudo-terminal.
- It takes no prompt and no other session flags. Don't add `--print`, a prompt, a model, an effort or permission options. The guard lets this one form through without a model and effort, because it moves a session that already ran rather than starting one.
- It refuses a session that's still running in a terminal or in the background. Wait for the run to end; it won't move an active session.
- On success it prints that it's opening the session in the desktop app. The docs say the app continues the same session, not a copy. The printed line alone doesn't prove the app shows it. Before calling the worker visible, get evidence the app owns it, such as its own session list showing that id, or the owner's confirmation. The row can take a few minutes to appear.
- It needs a Claude subscription sign-in and a supported desktop platform. It isn't available over SSH, in WSL or for cloud sessions. See the [desktop guide](https://code.claude.com/docs/en/desktop#coming-from-the-cli) and the [CLI reference](https://code.claude.com/docs/en/cli-reference).

### Follow-ups

To send a worker a follow-up, wait until its process has exited, then resume it from its launch folder with the same permission mode and added folders:

```sh
claude --print --model opus --effort medium --permission-mode auto --resume <session-id> --add-dir /path/to/notes --output-format json "<follow-up>" < /dev/null
```

Add `--add-dir` for any other folder the follow-up needs, such as a worktree the worker is told to edit. Check that the result's `session_id` is the same id and that it reports success. One process writes a session at a time. A session marked idle in `claude agents --json` isn't proof that no other process holds it, so don't start a second writer because of that label alone. Don't rerun the task under a new id because a resume or handoff failed.

After the handoff, the dispatcher's follow-ups use the same resume command, one at a time. Sequential follow-ups to a handed-off session completed under the same id. Don't send one while the owner is sending a turn from the app: two writers on one session weren't tested. Whether the app shows those CLI turns without a refresh wasn't checked either.

### When the handoff isn't available

Keep the worker in the CLI. These work without the app: dispatching it, its effort binding, its report, and sequential resumed follow-ups under the same id. Tell the owner the worker wasn't handed to the app, and why, as soon as the handoff fails, not only in the result.

## Messaging across machines

Finding sessions on other machines by name, and getting replies from them, needs the sending session connected to Remote Control. The [cross-session messaging docs](https://code.claude.com/docs/en/cross-session-messaging) cover the routes, including a one-way send to a known remote address without a usable reply address.

- Check a handed-off worker's Remote Control state before relying on it to reach other machines. Startup defaults differ. In the tests, both handed-off workers had it off, one on Windows and one on macOS.
- On Windows, while it was off, the worker's agent list showed only that machine's sessions, and a send by name to a macOS session failed as unreachable. Once the owner turned it on in the app, macOS sessions listed at once, and messages went both ways.
- Turning Remote Control on is the owner's call, per session. Don't change startup defaults or other settings to get it.
- A one-shot CLI process, such as a fresh `--print` resume, didn't carry the app session's connection. Replies to it failed once it had exited. To relay between machines, use the session already connected in the app, and have it record replies in a handoff file. Don't resume that session from the CLI as a second writer.

## What's been checked

Observed on Windows on 2026-11-24, from a Claude Code dispatcher in the desktop app: the app's session tool started a worker with a folder, a model and an effort, and the worker was in the owner's sidebar from its first turn, with no CLI run and no handoff. It bound its effort, proved its own guard and renamed its session with the app's tool. The guard had refused an earlier start that named no model. The app refused a start in a turn opened by a task notification from the dispatcher's own read-only subagent; starts worked later, after the owner's own message. A proposal card offered no folder. Not checked on this route: a start in a turn opened by a worker session's report, a follow-up sent through the app's messaging and seen in the worker, the owner steering the worker from the sidebar, macOS, and several workers started at once.

Observed on Windows with Claude Code 2.1.319: a worker launched by a Codex dispatcher persisted, bound its own effort and proved its own guard. After it exited, the handoff command failed with captured output and succeeded in a pseudo-terminal. Afterwards `claude agents --json --all` showed the same session id, folder and name, idle. Two sequential resumed follow-ups from the dispatcher then completed in that session: a short reply, and a documentation task with a worktree added. Each result reported the original session id and success.

Also observed on Windows: the handed-off worker had Remote Control off and saw only that machine's sessions. With Remote Control turned on in the app, it listed a session on macOS and exchanged several messages with it both ways. Messages back to one-shot CLI processes failed once those processes had exited.

Observed on macOS with Claude Code 2.1.316, from a sandboxed Codex dispatcher:

- Without host approval, a worker launch exited with `Not logged in` and wrote no transcript, though the stored sign-in was present.
- With host approval granted through normal review, a fresh launch succeeded and persisted its transcript. The worker bound its own effort and proved its own guard.
- The handoff failed sandboxed on sign-in. With host approval it printed that it was opening the session in the desktop app.
- Right after the handoff, `claude agents --json --all` didn't list that session id. A few minutes later it listed it, same id and folder, idle. The app's own session list had it too, under its launch name and not renamed after its effort, with Remote Control off and no parent session. That proves the app registered it, not what the sidebar draws on screen. The test did not inspect sidebar rendering or send a new turn to the session. The handed-off worker isn't nested under its dispatcher in the app.
- Later, the session's idle process had ended. The app still listed the session, not running, but it no longer appeared in `claude agents --json --all` or the agent list, so a message to it couldn't be sent. Why the process ended wasn't checked.
- With no live process holding it, one sequential `--print --resume` follow-up from the dispatcher completed under the same id, with host approval granted through normal review. The worker was still bound to its effort with its guard proven, and no fork or new id appeared.

Not checked on the CLI route: a message sent through the app reaching a handed-off worker, whether opening a listed but stopped session in the app makes it reachable again, the app's sidebar as drawn on screen, the app showing CLI turns without a refresh, the effort binding in turns sent from the app, a CLI follow-up while the app is sending a turn, a Claude Code dispatcher, the app showing a CLI follow-up's turn on macOS, restarting the app, several workers handed off at once, and a missing or signed-out app.
