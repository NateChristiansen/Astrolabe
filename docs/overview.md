# Astrolabe overview

Astrolabe is a small coordination layer around agent client sessions. A Rust
core owns shared contracts and will manage sessions through an adapter protocol
(ACP). A Tauri desktop app presents the work.
The clients and adapters remain external tools with their own authentication
and execution environments.

An effort is a bounded piece of work: a question to resolve, a change to make,
or an investigation to finish. Runs belong to efforts and refer to concrete
revisions. Handoffs carry readable prose plus a small typed payload so software
can connect a finding or question to the work that produced it. Software assigns
identifiers, timestamps, and revisions; agents supply claims and evidence.

The agent repo is a separate private repository owned by the person using the
tool. It holds owner context, voice preferences, and notes as plain Markdown.
Effort notes describe bounded work, journals capture dated observations, and
initiatives connect longer-running directions. These files remain readable and
editable without Astrolabe. Private content does not belong in the public tool's
source tree. The public seed contains placeholders only.

A needs-you view will collect questions that require an owner decision. Each
request asks one self-contained question, supplies context, recommends an action
with a reason, and explains what waits. An answer refers to that request. Agents
ask the next question only after receiving an answer. Review findings distinguish
executed evidence from inference; optional confidence is an uncalibrated report,
not an approval signal.

Cross-machine pairing will let a desktop view work running on another machine.
The core should keep session identity separate from the presentation layer, so a
change in where a window runs does not change which effort or revision a handoff
concerns. Pairing, transport, trust, and reconnection behavior remain future design
work; phase 0 supplies no network listener or pairing implementation.

Phase 0 establishes the workspace, versioned handoff contract, desktop shell,
agent-repo seed, and basic public-content checks. The CLI has only version output
and stub commands. It does not yet orchestrate agents, persist sessions, route
questions, or synchronize notes. Later phases can build those behaviors against
named consumers and evaluation questions instead of expanding the contract
speculatively.
