# Codex ACP replay fixtures

Recorded on macOS, 2026-10-31, using codex-acp 2.5.0, codex-cli
0.166.0, Node v26.9.0 and gpt-6.1-terra with low reasoning effort.
Six successful manual recorder scenarios; no provider is used during replay.

Each JSONL row names a zero-based process (resume uses two). Wire rows carry
`direction`, `message` and the original process-relative `elapsed_ms`. The fake
agent checks every client message before sending subsequent recorded messages.
Only cwd is normalized at runtime. Replay preserves order, without sleeping for
recorded delays. `exit_code` rows are observed proxy exits, not recorder exits.

These are deliberately trimmed protocol fixtures, not raw logs:

- Session and tool ids are deterministic placeholders; process ids are ordinals.
- Cwd becomes `/fixture/project`; prompts become `fixture prompt N`.
- Only the scenario's short answer chunks remain as text. Tool titles and option
  names are placeholders. Tool commands, raw inputs/outputs, terminal payloads,
  message ids, session titles and metadata are removed.
- Authentication notification parameters, model/mode/config catalogs, command
  catalogs, auth methods and usage details are removed or replaced with neutral
  shape values. Capability structure is preserved.
- Recorder events, config hashes, owner/account information, lossless `bytes`,
  stack traces, repo contents and instructions are omitted.
- Stderr retains only one path-free AppServerUnavailableError summary per process
  where observed. Cancel had no stderr.

The reject process and second resume process have no recorded final exit row.
The fake returns normally at EOF for those two segments; its code 0 is harness
behavior, not measured real-exit evidence. Other segments replay observed code 0.
The recorder explicitly shut down the adapters after successful turns; these are
not spontaneous crash recordings. Shutdown did not log ClientEvent::Exited.

No history updates preceded the real resume response. This recording therefore
checks route selection, session identity and continuation, not handling of
history replay (covered separately by the shared client's synthetic tests).
No tool status update emitted `pending`; permission requests contain a `pending`
tool snapshot, answered after 1 ms in each approval trace. These short traces
cannot establish long pending behavior. Idle launch and child-session scenarios
are outside these fixtures.
