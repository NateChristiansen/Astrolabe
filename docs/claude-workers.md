# Claude Code workers

A dispatcher can run a Claude Code worker as a persisted CLI session, then hand that same session to the Claude desktop app once its run ends. The handoff moves an idle session into the app. It doesn't mirror a session that's still working, so the app shows the worker after its run, not during it.

This was checked with a Codex dispatcher. A Claude Code dispatcher's shell passes its own session environment, including its session id, to a CLI it starts ([host behavior](hosts.md#what-a-session-passes-to-a-session-it-starts)), and a worker launched that way hasn't been tested.

## Launch

Create the worker's branch worktree first. Launch from that worktree, with the notes checkout added for briefs, reports and handoffs:

```sh
claude --print --permission-mode auto --add-dir /path/to/notes --output-format json "<brief>" < /dev/null
```

Replace the paths with yours. In PowerShell, pipe an empty string in (`'' | claude --print ...`) to close stdin. Keep the app's normal permission checks; don't use the bypass mode for a worker.

- The session persists by default. Don't pass `--no-session-persistence`, or the session can't be resumed or handed off.
- Record the session id. The JSON result carries it as `session_id`, and `claude agents --json` lists running sessions with their id, folder, name and status. To know it before launch, pass your own UUID with `--session-id <uuid>`.
- The plugin's hook names the session after its effort when the worker binds. That doesn't control the title the desktop app shows: on macOS the app kept the launch name after a handoff. Pass a clear name with `--name <name>` at launch, or rename the session in the app where that's allowed. The name doesn't affect the effort binding, which stays correct either way.
- Start the brief with `astrolabe activate --effort <n> --worker` as its own first command, then have the worker prove its own guard with the probe that activation prints. The proof is per session: the dispatcher's proof doesn't cover the worker, and the worker's doesn't cover a subagent.
- Remember the folder you launched from. Sessions are stored per project folder, and every later resume or handoff runs from that same folder.

The worker saves only its own effort and writes its report where the brief says. The dispatcher reviews, journals and closes, as in [usage](usage.md).

### When the launch says it isn't logged in

A dispatcher whose shell runs in a sandbox can start a worker that exits at once with `Not logged in · Please run /login`. That doesn't by itself mean the account is signed out. In the one macOS test, the stored sign-in was present and the launch still failed inside the sandbox.

- Check whether the account is available without changing anything. Run `claude auth status` from a terminal outside the sandbox. A sign-in there means the account is available. It doesn't prove what blocked the sandboxed launch.
- If the account is available, the dispatcher may request its normal host approval for that one launch, without changing the dispatcher's sandbox settings. If review denies it, stop and report the reason. In the macOS test, approval let the launch succeed.
- In that test the handoff needed approval too. Sandboxed, `claude --desktop --resume` failed with a message that `--desktop` requires signing in with a Claude account, even though the worker had run. Other hosts may differ.
- Don't log in or out, copy credentials, change settings or the environment, bypass trust, or turn the sandbox off to get past it.
- Before authorizing a fresh launch, check that the failed one left no running worker and no transcript under its id. In the macOS test it left neither. Nothing starts a replacement automatically. If you start one, record its new id.

## Follow-ups

To send a worker a follow-up, wait until its process has exited, then resume it from its launch folder with the same permission mode and added folders:

```sh
claude --print --permission-mode auto --resume <session-id> --add-dir /path/to/notes --output-format json "<follow-up>" < /dev/null
```

Add `--add-dir` for any other folder the follow-up needs, such as a worktree the worker is told to edit. Check that the result's `session_id` is the same id and that it reports success. One process writes a session at a time. A session marked idle in `claude agents --json` isn't proof that no other process holds it, so don't start a second writer because of that label alone. Don't rerun the task under a new id because a resume or handoff failed.

## Hand it to the desktop app

After the worker's process has exited, run this from the worker's launch folder, in a real terminal:

```sh
claude --desktop --resume <session-id>
```

- It needs an interactive terminal. With captured or redirected output it refuses to run (observed on Windows). An agent's shell tool has to give it a pseudo-terminal.
- It takes no prompt and no other session flags. Don't add `--print`, a prompt or permission options.
- It refuses a session that's still running in a terminal or in the background. Wait for the run to end; it won't move an active session.
- On success it prints that it's opening the session in the desktop app. The docs say the app continues the same session, not a copy. The printed line alone doesn't prove the app shows it. Before calling the worker visible, get evidence the app owns it, such as its own session list showing that id, or the owner's confirmation. The row can take a few minutes to appear.
- It needs a Claude subscription sign-in and a supported desktop platform. It isn't available over SSH, in WSL or for cloud sessions. See the [desktop guide](https://code.claude.com/docs/en/desktop#coming-from-the-cli) and the [CLI reference](https://code.claude.com/docs/en/cli-reference).

After the handoff, the dispatcher's follow-ups use the same resume command, one at a time. Sequential follow-ups to a handed-off session completed under the same id. Don't send one while the owner is sending a turn from the app: two writers on one session weren't tested. Whether the app shows those CLI turns without a refresh wasn't checked either.

## Messaging across machines

Finding sessions on other machines by name, and getting replies from them, needs the sending session connected to Remote Control. The [cross-session messaging docs](https://code.claude.com/docs/en/cross-session-messaging) cover the routes, including a one-way send to a known remote address without a usable reply address.

- Check a handed-off worker's Remote Control state before relying on it to reach other machines. Startup defaults differ. In the tests, both handed-off workers had it off, one on Windows and one on macOS.
- On Windows, while it was off, the worker's agent list showed only that machine's sessions, and a send by name to a macOS session failed as unreachable. Once the owner turned it on in the app, macOS sessions listed at once, and messages went both ways.
- Turning Remote Control on is the owner's call, per session. Don't change startup defaults or other settings to get it.
- A one-shot CLI process, such as a fresh `--print` resume, didn't carry the app session's connection. Replies to it failed once it had exited. To relay between machines, use the session already connected in the app, and have it record replies in a handoff file. Don't resume that session from the CLI as a second writer.

## When the handoff isn't available

Keep the worker in the CLI. These work without the app: dispatching it, its effort binding, its report, and sequential resumed follow-ups under the same id. Say in the result that the worker wasn't handed to the app, and why.

## What's been checked

Observed on Windows with Claude Code 2.1.319: a worker launched by a Codex dispatcher persisted, bound its own effort and proved its own guard. After it exited, the handoff command failed with captured output and succeeded in a pseudo-terminal. Afterwards `claude agents --json --all` showed the same session id, folder and name, idle. Two sequential resumed follow-ups from the dispatcher then completed in that session: a short reply, and a documentation task with a worktree added. Each result reported the original session id and success.

Also observed on Windows: the handed-off worker had Remote Control off and saw only that machine's sessions. With Remote Control turned on in the app, it listed a session on macOS and exchanged several messages with it both ways. Messages back to one-shot CLI processes failed once those processes had exited.

Observed on macOS with Claude Code 2.1.316, from a sandboxed Codex dispatcher:

- Without host approval, a worker launch exited with `Not logged in` and wrote no transcript, though the stored sign-in was present.
- With host approval granted through normal review, a fresh launch succeeded and persisted its transcript. The worker bound its own effort and proved its own guard.
- The handoff failed sandboxed on sign-in. With host approval it printed that it was opening the session in the desktop app.
- Right after the handoff, `claude agents --json --all` didn't list that session id. A few minutes later it listed it, same id and folder, idle. The app's own session list had it too, under its launch name and not renamed after its effort, with Remote Control off and no parent session. That proves the app registered it, not what the sidebar draws on screen. The test did not inspect sidebar rendering or send a new turn to the session. The handed-off worker isn't nested under its dispatcher in the app.
- Later, the session's idle process had ended. The app still listed the session, not running, but it no longer appeared in `claude agents --json --all` or the agent list, so a message to it couldn't be sent. Why the process ended wasn't checked.
- With no live process holding it, one sequential `--print --resume` follow-up from the dispatcher completed under the same id, with host approval granted through normal review. The worker was still bound to its effort with its guard proven, and no fork or new id appeared.

Not checked: a message sent through the app reaching a handed-off worker, whether opening a listed but stopped session in the app makes it reachable again, the app's sidebar as drawn on screen, the app showing CLI turns without a refresh, the effort binding in turns sent from the app, a CLI follow-up while the app is sending a turn, a Claude Code dispatcher, the app showing a CLI follow-up's turn on macOS, restarting the app, several workers handed off at once, and a missing or signed-out app.
