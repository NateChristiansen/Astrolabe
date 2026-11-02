// The test control's tool list is fixed, and is there with no app running.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const server = fileURLToPath(new URL("../../scripts/test-control.mjs", import.meta.url));

test("the tools show with the app down, and a call without start says so", async () => {
  const child = spawn(process.execPath, [server], { stdio: ["pipe", "pipe", "inherit"] });
  const waiting = new Map();
  createInterface({ input: child.stdout }).on("line", (line) => {
    const message = JSON.parse(line);
    waiting.get(message.id)(message);
  });
  let id = 0;
  const call = (method, params) =>
    new Promise((done) => {
      waiting.set(++id, done);
      child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    });
  const hello = await call("initialize", { protocolVersion: "2025-06-18", capabilities: {} });
  assert.equal(hello.result.protocolVersion, "2025-06-18");
  const { result } = await call("tools/list", {});
  assert.deepEqual(result.tools.map((tool) => tool.name), ["start", "stop", "read", "click", "type", "screenshot", "run_steps"]);
  for (const name of ["read", "stop"]) {
    const said = await call("tools/call", { name, arguments: {} });
    assert.equal(said.result.isError, true);
    assert.match(said.result.content[0].text, /Nothing is started here/);
  }
  const closed = new Promise((done) => child.on("exit", done));
  child.stdin.end();
  assert.equal(await closed, 0);
});
