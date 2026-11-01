# Codex ACP replay fixtures

Recorded on macOS, 2026-10-31, using codex-acp 2.5.0, codex-cli 0.166.0
selected through `CODEX_PATH`, Node v26.9.0 and gpt-6.1-terra with low reasoning effort.
Six successful manual recorder scenarios; no provider is used during replay.
History: the earlier bundled 0.165.0 recordings were initially mislabeled 0.166.0 because the recorder probed PATH instead of the adapter's selected executable.

Each JSONL row names a zero-based process (resume uses two). Wire rows carry
`direction`, `message` and the original process-relative `elapsed_ms`. The fake
agent checks every client message before sending subsequent recorded messages.
Only cwd is normalized at runtime. Replay preserves order, without sleeping for
recorded delays. `exit_code` rows are observed proxy exits, not recorder exits.

These are deliberately trimmed protocol fixtures, not raw logs:

- Session and tool ids are deterministic placeholders; process ids are ordinals.
- Cwd becomes `/fixture/project`; prompts become `fixture prompt N`.
- Only the scenario's short answer chunks remain as text. Tool titles and option
  names are the protocol option kinds; option ids retain their wire values. Tool commands, raw inputs/outputs, terminal payloads,
  message ids, session titles and metadata are removed.
- Authentication notification parameters, model/mode/config catalogs, command
  catalogs, auth methods and usage details are removed or replaced with neutral
  shape values. Capability structure is preserved.
- Recorder events, config hashes, owner/account information, lossless `bytes`,
  stack traces, repo contents and instructions are omitted.
- Stderr retains only one path-free AppServerUnavailableError summary per process
  where observed. Cancel had no stderr.

Full-turn, allow, reject and the second resume process have no recorded final exit
row. The fake returns normally at trace end for those segments; its code 0 is
harness behavior, not measured real-exit evidence. Two-turns, cancel and the first
resume process replay observed code 0. All six recorder commands exited 0 and
reported config unchanged; recorder success is separate from proxy exit evidence.
The recorder explicitly shut down the adapters after successful turns; these are
not spontaneous crash recordings. Shutdown did not log ClientEvent::Exited.

No history updates preceded the real resume response. This recording therefore
checks route selection, session identity and continuation, not handling of
history replay (covered separately by the shared client's synthetic tests).
The resume response contains model/mode/config catalogs without `sessionId` and
is projected to `{}`. The same response shape occurs in the earlier 0.165.0 log.
No tool status update emitted `pending`; permission requests contain a `pending`
tool snapshot, answered within 0–1 ms in these approval traces. These short traces
cannot establish long pending behavior. Idle launch and child-session scenarios
are outside these fixtures.

Compared with the earlier 0.165.0 traces, protocol shapes, capabilities, answers,
permission outcomes and stop reasons are unchanged. Beyond timing, reject swaps
the failed tool update and a session-info update; the resumed turn swaps command
and session-info updates. Full-turn and allow no longer have proxy exit rows.
Raw usage counts and generated ids differ, and reject's command catalog grows
from 22 to 52 entries (catalog contents are omitted). These are observed run
differences, not established version-specific behavior changes.

Regenerating fixtures (Node, no Python):

1. Outside the builder sandbox, set `CODEX_PATH` to the exact user-installed
   executable selected by the lead. The driver accepts `Some(client_path)` and
   sets that variable; step 5's doctor will supply the path. `None` leaves the
   adapter's inherited/bundled selection. The recorder uses `CODEX_PATH --version`
   on that exact path. When unset it records `bundled` and the installed adapter's
   resolved `@openai/codex` dependency version, never PATH codex.
2. Run each recorder scenario with a fresh disposable clone under `/private/tmp`,
   stdin redirected from `/dev/null`, and a new private output filename:

   ```sh
   CODEX_PATH=/absolute/path/to/codex cargo run --offline --locked -p astrolabe --example record_codex -- full-turn /private/tmp/disposable-clone /private/tmp/full-turn.private.jsonl < /dev/null
   ```

3. Scrub each scenario to a new staging filename (the script refuses overwrite):

   ```sh
   node scripts/scrub-codex.cjs full-turn /private/tmp/full-turn.private.jsonl /private/tmp/full-turn.scrubbed.jsonl
   ```

   Repeat for `approval-allow`, `approval-reject`, `two-turns`, `resume`, and
   `cancel-tool`. The scrubber projects only supported protocol fields, assigns
   deterministic session/tool/process ids, replaces cwd/prompts and catalogs,
   validates answer text, and drops recorder/config/byte payloads and raw stderr.
   Unknown message shapes stop the scrub for review. It does not copy provenance
   headers: verify the private `before.codex` record and update this README with
   the actual runtime version before replacing public fixtures.
4. Review each staged JSONL and its diff for private data and expected protocol
   shape, then copy it over the corresponding fixture. Scrubbing is a projection,
   not a substitute for that review. Keep raw logs private. Build the subprocess
   helpers before running the targeted replay tests:

   ```sh
   cargo build --offline --locked -p astrolabe --examples
   cargo test --offline --locked -p astrolabe --test driver_codex
   node --test scripts/scrub-codex.test.cjs
   ```

Replay canonicalizes both cwd paths, including on Windows (checked by CI only).
At trace end, including recorded exit rows, the fake checks for trailing client
messages until EOF or 250 ms of quiet; stdin stays open while the client awaits
exit, so this is a bounded check, not proof against arbitrarily delayed writes.
Exit codes are compared to evidence only when a recorded exit row exists.
`check_for_update_on_startup = false` is requested in per-session CODEX_CONFIG;
its effect on app-server startup update checks remains unverified.
