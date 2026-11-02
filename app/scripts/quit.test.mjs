// Told to quit, a window starts no core until the owner says. Anything else, it comes back.
import assert from "node:assert/strict";
import { test } from "node:test";
import { connect, retries } from "../src/core.ts";

test("told to quit means no restart", () => {
  assert.equal(retries({ told: true, closed: false }), false);
});

test("a dropped socket with no such message means restart", () => {
  assert.equal(retries({ told: false, closed: false }), true);
});

test("a window closed by its owner starts nothing", () => {
  assert.equal(retries({ told: false, closed: true }), false);
});

/** A window with a stand-in socket, counting how often it asks for a core. */
function aWindow() {
  const sockets = [];
  const links = [];
  let asked = 0;
  globalThis.window = {
    __TAURI__: { core: { invoke: async () => (asked++, [1, "secret"]) } },
  };
  globalThis.WebSocket = class {
    static OPEN = 1;
    send() {}
    close() {}
    constructor() {
      sockets.push(this);
    }
  };
  const core = connect(() => {}, (link) => links.push(link));
  return { core, sockets, links, asked: () => asked };
}
const turn = (ms = 0) => new Promise((done) => setTimeout(done, ms));

test("a real connection told to quit stays down until Start, then asks once", async () => {
  const { core, sockets, links, asked } = aWindow();
  await turn();
  sockets[0].onopen();
  sockets[0].onmessage({ data: JSON.stringify({ type: "quitting" }) });
  assert.equal(links.at(-1), "stopping");
  sockets[0].onclose();
  assert.equal(links.at(-1), "stopped");
  await turn(1200);
  assert.equal(asked(), 1, "no core was asked for");
  core.start();
  await turn();
  assert.equal(asked(), 2);
  core.close();
});

test("a real connection that just drops asks again", async () => {
  const { core, sockets, asked } = aWindow();
  await turn();
  sockets[0].onopen();
  sockets[0].onclose();
  await turn(1200);
  assert.equal(asked(), 2);
  core.close();
});
