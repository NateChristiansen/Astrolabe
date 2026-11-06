# Working on the Rust code

The crate stays small enough to build as one package. Modules separate responsibilities rather than imposing a file-length target. The command grammar, saved formats and hook responses remain compatible.

## Where changes belong

- `effort/command.rs` parses shell commands and bodies. `effort/file.rs` interprets note properties, sections and checklists. Neither starts processes or writes files.
- `record.rs` coordinates new efforts, saves and journal entries. `record/validation.rs` checks prose, settings and plan sections before a write is attempted.
- `notes/storage.rs` owns recoverable operations and commit scope. `notes/sync.rs` owns network Git calls, backoff, merge recovery and synchronization warnings. `notes/journal.rs` owns journal formats; `notes/facts.rs` observes a product checkout; `notes/process.rs` owns detached children and platform handle behavior. `notes.rs` holds the checkout, lock, numbering and shared filesystem primitives.
- `session.rs` owns session records and the save and acknowledgement transitions. `nudge.rs` decides when work needs a save reminder.
- `activate/repos.rs` matches checkouts to repository notes. `activate/context.rs` loads effort context. `activate/proof.rs` evaluates guard evidence. `activate/tickets.rs` persists hook replies. `activate/routing.rs` handles provider inputs and applies activation transitions. `activate.rs` exposes the facade and machine switch.
- `board.rs` collects a typed snapshot for both the board and `efforts --json`. Collection may read Git status; rendering does not run Git.
- `home/server.rs` handles HTTP and live event streams. `home/render.rs`, `home/markdown.rs` and `home/nav.rs` render the board, notes and navigation. `home/settings.rs` holds appearance and port settings. CSS and JavaScript live beside the renderer and are embedded at build time.
- `application.rs` connects completed note operations to presentation. `home_effects.rs` writes the ignored board file and ensures the local server is running. Storage has no dependency on either rendering or server startup.

## Trace a save

1. The provider hook reaches `activate::gate`; command recognition and parsing use `EffortCommand`.
2. `record::save` reads the typed session record, validates the request, takes the notes lock and reconciles pending operations.
3. It checks the delivered note hash and plans an `Op` containing the complete files, journal lines and session state to write.
4. `Notes::apply` persists the intent, writes files, commits only the intended note paths, records that the commit completed, then writes the session state. A retry finishes remaining steps without overwriting a newer committed note.
5. `application::finish` records the reply and refreshes the board after the operation. Storage reports changes; orchestration consumes them while holding the notes lock. Several recovered operations can share one refresh. A board failure is reported separately from a successful notes save.

The existing deliberate aggregation of hand edits inside effort and journal paths remains. Unrelated staged files stay out of automatic note commits.

## Trace a resume

The session-start route reads `SessionState`, refreshes notes and reads the record again because synchronization may renumber the effort. Context loading matches the repository, reads the effort through the shared note parser and prepares the session text. Only a successful transition replaces the session record. A failed context reload retains guard protection and records the existing shell-command block behavior.

`SessionState` keeps JSON at the storage boundary. Its `Field<T>` represents a known value, an unexpected legacy value or an absent field. Null and absent fields remain distinct; unsupported optional values retain their old fallback behavior. Unknown fields survive saving and resuming the same effort. A deliberate switch to another effort starts a new record, as before. Resume still refreshes derived context and clears the fields the previous implementation cleared.

Pending operation state payloads remain JSON because they are durable replay instructions, including records produced by older builds. Their persistence format is unchanged. Provider hook input and output also remain JSON at the external boundary.

## Verification

Run `scripts/check` before committing. Binary integration fixtures remain in `tests/activate.rs` and `tests/record.rs`; their subdirectories group scenarios by behavior. Recovery, two-machine synchronization and live HTTP flows still use real child processes and disposable Git repositories. Small parser and rendering tests remain beside the implementation.

For state changes, verify legacy optional fields and unknown data through a save or resume. For rendering or asset moves, compare collected JSON and rendered HTML with the previous build, then inspect both a normal and narrow viewport. The recorded-backoff test verifies an existing backoff record; it does not simulate a network timeout.

The same checks run in macOS and Windows CI. A local macOS pass does not establish Windows, Linux or live agent-client compatibility.
