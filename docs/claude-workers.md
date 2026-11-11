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
- The plugin's hook names the session after its effort when the worker binds. `--name <name>` sets a name at launch.
- Start the brief with `astrolabe activate --effort <n> --worker` as its own first command, then have the worker prove its own guard with the probe that activation prints. The proof is per session: the dispatcher's proof doesn't cover the worker, and the worker's doesn't cover a subagent.
- Remember the folder you launched from. Sessions are stored per project folder, and every later resume or handoff runs from that same folder.

The worker saves only its own effort and writes its report where the brief says. The dispatcher reviews, journals and closes, as in [usage](usage.md).

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
- On success it prints that it's opening the session in the desktop app, and the app continues the same session, not a copy.
- It needs a Claude subscription sign-in and a supported desktop platform. It isn't available over SSH, in WSL or for cloud sessions. See the [desktop guide](https://code.claude.com/docs/en/desktop#coming-from-the-cli) and the [CLI reference](https://code.claude.com/docs/en/cli-reference).

After the handoff, the dispatcher's follow-ups use the same resume command, one at a time. Sequential follow-ups to a handed-off session completed under the same id. Don't send one while the owner is sending a turn from the app: two writers on one session weren't tested. Whether the app shows those CLI turns without a refresh wasn't checked either.

## When the handoff isn't available

Keep the worker in the CLI. Everything above the handoff works without the app: the session id, its effort binding, its report and resumed follow-ups. Say in the result that the worker wasn't handed to the app, and why.

## What's been checked

Observed on Windows with Claude Code 2.1.319: a worker launched by a Codex dispatcher persisted, bound its own effort and proved its own guard. After it exited, the handoff command failed with captured output and succeeded in a pseudo-terminal. Afterwards `claude agents --json --all` showed the same session id, folder and name, idle. Two sequential resumed follow-ups from the dispatcher then completed in that session: a short reply, and a documentation task with a worktree added. Each result reported the original session id and success.

Not checked: the app's sidebar showing the session, the app showing CLI turns without a refresh, the effort binding in turns sent from the app, a CLI follow-up while the app is sending a turn, a Claude Code dispatcher, macOS, restarting the app, several workers handed off at once, and a missing or signed-out app.
