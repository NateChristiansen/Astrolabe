# Protocol

One envelope carries a prose `body` and a typed `payload` selected by `kind`.
`schema_version` versions the envelope; `payload_version` independently versions
each payload kind. Both currently accept the string `"1"`. Unknown fields on
known object types survive round trips, including nested objects. Unknown kinds
or versions are rejected; forwarding those requires a future explicit consumer.

Software supplies ids, timestamps, and revisions; agents supply claims.
Add fields only against a named consumer or eval question. Phase-0 consumers are
the desktop handoff renderer (envelope/body), review display (finding), and
needs-you display (request/answer). No agent transport is implemented yet.

The request contains exactly one self-contained question, context, a
recommendation with its reason, and what waits. Ask the next question only after
an answer. The single `question` string models this contract; prose semantics
are a producer responsibility, not a question-mark counter. `request_id` joins
an answer to its request; `reply_to` optionally links envelopes.

`evidence` distinguishes executed checks from inference. Confidence is optional,
uncalibrated self-report, with value, method, and model; it is not a probability
or a decision threshold. Line numbers are one-based. Timestamps are RFC 3339 UTC.
Phase 0 models these conventions without runtime format validation.

Run `cargo run -p protocol --bin export-schemas` from any working directory.
Schemas are checked in under `protocol/schemas`; regenerate TypeScript with
`pnpm --dir app generate:protocol`. Commit schemas, types, and fixtures together.
