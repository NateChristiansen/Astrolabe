# Decisions

The plugin replaced the previous product on 2026-11-03. Its choices come first; the product's decisions follow, kept as records.

- Keep the owner's notes in a separate private Git repo, read at run time, so sessions and machines share the same record without personal content in this repo.
- Use the plugin's machine state folder for repo bindings and guard denials, so hooks and terminal commands read the same records across clients.
- Use standard-library command routing for the small CLI, serde/serde_json for hook input and state, and Markdown files for effort records.
- Keep only the gated hook entry point. The owner's machine switch decides whether hooks do anything; loading an effort supplies context and settings to that session.
- Treat the command guard as an accident guard. Prove interception with an allowed control and a denied probe when a session loads or creates an effort, not at every session start (2026-11-06): a session without one only reads and is told it holds no proven protection, and the probe cost every session about two tool calls. Never claim that the guard covers file edits or other tools.
- Keep the existing privacy policy: no string-matching content scrubbers in this repo. Private content stays in the separate notes checkout; secrets rely on the forge's secret scanning and push protection.
- File efforts and journal days on month shelves by creation date, over flat folders or folders by repo: the creation date never changes, so a file never moves and every link holds, and the board, not the folder, shows status.
- Refuse a journal line at write time for the shapes sessions got wrong with the rule alone (a third sentence, a closing status word, a file name, "the owner"), rather than add start text: on 2026-11-05 nine of seventeen lines broke rules every session had been given, and the label-line refusal on Current state is what held there.
- Serve the owner's home as a page on localhost rather than a Markdown note or an Obsidian base: the owner saw a Markdown sample and found it too plain, and columns, colour and ticking ages need a page; Obsidian stays optional.
- Start the server lazily from the hooks, one per machine by a lock, stopping when idle, over a login item or a scheduled task: the hooks already run at every change, and a login item is one more thing per machine to install and forget.
- Have the page re-read itself when the server says the notes changed, rather than push the new board down the stream: the page already draws itself, and one path is less to keep right.
- Count the notes' lessons on the board and never deliver them to sessions (2026-11-05). Stored experience helps when the store is small, curated and retired one entry at a time, and hurts as an always-loaded archive: instruction following falls with the count of rules, and growing lessons archives go stale and unread. A lesson is an inbox entry that ends as a short rule with its reason, a fix, or nothing, then is deleted; the folder's size is the backlog.
- Draw the board as one calm page rather than four fixed columns: the counts as chips under the date, a colour rail on each card for its state, ready, blocked and ideas beside the active grid only when they have cards, the journal under the work, closed work as a list. The owner picked it from three mockups on 2026-11-05: empty columns and uniform type had hidden what mattered.

## Superseded 2026-11-05 by the plugin

The product these served (the Tauri app, the core, the ACP adapters, the per-machine store) was removed in effort 7; each is superseded by the plugin's choices above.

- Keep phase 0 to contracts and scaffolding; adapters, persistence, MCP, pairing, and orchestration wait for phase 1a or later.
- Use a Rust 2024 workspace with serde/serde_json for the shared wire format and schemars for JSON Schema derived from Rust types.
- Use a tagged envelope enum to bind each kind to its payload; BTreeMap extension fields retain unknown fields with deterministic ordering.
- Version the envelope and each payload independently, using the explicit string `"1"`; reject unknown kinds/versions rather than invent forwarding behavior.
- Keep questions as one string and treat the one-question rule as a producer obligation; punctuation counting cannot validate prose intent.
- Use no CLI dependency: two stub commands and version output fit a standard-library argument match.
- Use Tauri 2, React, TypeScript, and Vite as requested; no UI kit, router, state library, or Tauri plugin is needed for a blank window.
- Use json-schema-to-typescript only at build time to keep checked-in TypeScript aligned with the JSON Schema contract.
- Keep the native Tauri crate outside the core workspace, with its own lockfile, so CLI/contract checks do not require native GUI libraries.
- Use Node's standard library for the seed test: Node is already required by the app, works on native Windows, and avoids a shell/runtime dependency.
- No in-repo content or secret scanner: no string-matching scrubbers for privacy; private data stays off the machines that build this repo, and secrets rely on the forge's built-in secret scanning and push protection.
- Use rusqlite with bundled SQLite for the per-machine store: one embedded file, no system library to differ between Mac and Windows; the existing no-CLI-dependency decision stands, and arguments remain a standard-library match.
- Use serde_json in the core because agent updates arrive as JSON-RPC and only some fields matter; ACP types stay untyped until a driver consumes them.
- Use no app-dirs, signal or process crates: the OS data dir is two environment variables, every state change commits so `serve` needs no shutdown handler, the standard library's file lock excludes a second core, and liveness comes from `ps` or `tasklist`.
