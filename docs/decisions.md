# Decisions

- Keep phase 0 to contracts and scaffolding; adapters, persistence, MCP, pairing, and orchestration wait for phase 1a or later.
- Use a Rust 2024 workspace with serde/serde_json for the shared wire format and schemars for JSON Schema derived from Rust types.
- Use a tagged envelope enum to bind each kind to its payload; BTreeMap extension fields retain unknown fields with deterministic ordering.
- Version the envelope and each payload independently, using the explicit string `"1"`; reject unknown kinds/versions rather than invent forwarding behavior.
- Keep questions as one string and treat the one-question rule as a producer obligation; punctuation counting cannot validate prose intent.
- Use no CLI dependency: two stub commands and version output fit a standard-library argument match.
- Use Tauri 2, React, TypeScript, and Vite as requested; no UI kit, router, state library, or Tauri plugin is needed for a blank window.
- Use json-schema-to-typescript only at build time to keep checked-in TypeScript aligned with the JSON Schema contract.
- Keep the native Tauri crate outside the core workspace, with its own lockfile, so CLI/contract checks do not require native GUI libraries.
- Use Node's standard library for the scanner and seed tests: Node is already required by the app, works on native Windows, and avoids a shell/runtime dependency.
- Scan Git objects rather than working-tree files; push scanning covers introduced history so deleting a secret in a later commit does not hide it.
- Use heuristic entropy checks for general assignments and sensitive JSON/YAML keys; recognize Cargo checksum fields without exempting lockfiles, and never report matching secret values.
- Keep explicit notes in a separate private agent repo; disable client automatic memory generation/use in the seed, and leave MCP placeholders unwired.

The seed uses client configuration locations to disable automatic memory and
leave the agent client server disabled. It is a template, not a client install
or a validated running integration. Client and adapter integration belongs to
phase 1a.
