# Claude adapter recordings

Raw newline-delimited JSON-RPC from a real adapter, replayed in `cargo test` by `examples/replay_agent.rs`. Each line is `{"dir","t","msg"}`: `in` is what the client wrote, `out` what the adapter wrote, `t` milliseconds from start. A last `exit` line holds the exit code as the recording tap saw it.

Recorded on Mac, 2026-10-31: claude-agent-acp 0.99.1, Claude Code 2.1.312 (the copy bundled with the agent SDK), Node 26.9.0, the cheapest model. `crash` killed the adapter under the tap, so its code is the tap's.

To re-record, run `examples/record_claude` in a throwaway clone with a clean environment. The output is not fit to commit until scrubbed.

## Scrub

The scrub is not in this repo. Apply all of these, then run `recordings_name_no_machine_or_account`:

- Renumber session, tool, message and request ids, consistently across the files of one scenario.
- Rewrite the working directory to `/private/tmp/astrolabe-fixture`.
- Remove anywhere: `agentId`, `runId`, `outputFile`, `toolResponse`, `rawOutput`.
- Remove `content` from `tool_call` and `tool_call_update`.
- Replace the params of `_auth/status_update`, which name the account.
- Empty `availableCommands`, which lists installed tools.
- Remove `_claude/rateLimit` and the prompt result's `quota`.
- Keep only the `mode` entry of `configOptions`.
- Read what is left. The test catches known shapes, not new ones.
