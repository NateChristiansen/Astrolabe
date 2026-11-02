# Test control

A small tool for agents to drive a debug build of the app and read the answer from the live page. It is in no normal build.

## What it is

- `test-control`, a Cargo feature of `app/src-tauri`, off by default. It adds a private local channel to the window, and sends the page script in `app/src-tauri/src/control.js` for each ask. That script isn't in the page's bundle.
- `scripts/test-control.mjs`, an MCP server over stdio with no dependencies, and the same steps from a shell.

## Build

```
pnpm --dir app install --frozen-lockfile
node scripts/test-control.mjs build
```

`start` builds nothing. It opens this build, with the core beside it, and refuses any program without the control in it.

## Turn it on for one session

Nothing is added to any client's config.

```
claude --mcp-config '{"mcpServers":{"astrolabe-test":{"command":"node","args":["scripts/test-control.mjs"]}}}'
codex -c 'mcp_servers.astrolabe_test.command="node"' -c 'mcp_servers.astrolabe_test.args=["scripts/test-control.mjs"]'
```

Run these from the repo root. Tools: `start`, `stop`, `read`, `click`, `type`, `screenshot`, `run_steps`.

## From a shell

```
node scripts/test-control.mjs run steps.json --repo throwaway=/path/to/a/repo
```

`steps.json` is the list `run_steps` takes:

```json
[
  { "do": "wait_for", "name": "Quit Astrolabe", "timeout_ms": 30000 },
  { "do": "click", "name": "Quit Astrolabe" },
  { "do": "wait_for", "text": "Astrolabe has stopped" },
  { "do": "wait_for", "core": "stopped" },
  { "do": "screenshot" }
]
```

## What holds it apart

- A test build won't start unless `ASTROLABE_DATA_DIR` names a full path apart from the default data folder, so it has its own core, lock, `bridge.txt` and database.
- The channel listens on loopback only, and wants the fresh secret from an owner-only file in that folder. Each ask is a new connection with the secret first.
- CI builds the packaged apps without the feature and fails if the marker string is in them. It also checks that the scan finds it in a test build.

## What it can't check

Clicks are events dispatched in the page, not input from the system. So it doesn't show that a window is on screen, focus or keyboard behaviour, drag, menus, the title bar's close button, or anything outside the page. The picture is the web view's, without the title bar.

Agents still run as their real clients with the real login and home folder. Use a throwaway repo.
