#!/usr/bin/env node
// Drives a test build of the app through its own private channel. No dependencies. As an MCP
// server over stdio with no arguments. Also, from a shell:
//   node scripts/test-control.mjs build
//   node scripts/test-control.mjs run <steps.json> [--data <folder>] [--repo <name>=<folder>]
//   node scripts/test-control.mjs absent|present <file or folder>...
// It only ever connects to the window it launched. See docs/test-control.md.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

export const MARK = "astrolabe-test-control";
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const EXE = process.platform === "win32" ? ".exe" : "";
const APP = path.join(ROOT, "app/src-tauri/target/debug", `astrolabe-desktop${EXE}`);
const BUILD = [
  "pnpm --dir app build",
  "cargo build --manifest-path app/src-tauri/Cargo.toml --locked --features test-control",
  "cargo build --locked -p astrolabe",
];
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));
const string = { type: "string" };
const target = {
  type: "object",
  properties: { name: { ...string, description: "What the control shows: its text, label or placeholder." }, role: { ...string, description: "Optional: button, textbox, combobox, radio." } },
  required: ["name"],
};
const blank = { type: "object", properties: {} };

/** Fixed, and the same with the app down, so a session always knows what it can do. */
export const TOOLS = [
  { name: "start", description: "Open the already built test build on a data folder of its own. Builds nothing. Refuses the default data folder.", inputSchema: { type: "object", properties: { data_folder: { ...string, description: "Optional: full path. Omitted, a throwaway folder is made." }, repos: { type: "object", description: "Optional: repo name to folder, added before the window opens.", additionalProperties: string } } } },
  { name: "stop", description: "Quit the core and window that start opened, through the window's own Quit. Refuses anything else.", inputSchema: blank },
  { name: "read", description: "The window's visible text, and its controls with enabled state.", inputSchema: blank },
  { name: "click", description: "Click a control found by what it shows. Fails if it is missing, ambiguous, disabled or covered.", inputSchema: target },
  { name: "type", description: "Put text in a field, or pick a choice in a list, found by what it shows.", inputSchema: { ...target, properties: { ...target.properties, text: string }, required: ["name", "text"] } },
  { name: "screenshot", description: "A picture of the page inside the window, kept in the data folder.", inputSchema: blank },
  { name: "run_steps", description: 'Run steps in order in one call and return each result, stopping at the first failure. Each step is {"do": read | click | type | screenshot | wait_for | sleep, ...}. click and type take the arguments of those tools. sleep takes ms. wait_for polls until it holds, and takes timeout_ms (default 15000) and one of: text (is in the visible text), line (is a whole visible line), gone (is not in the visible text), name with optional role (that control is showing and enabled), core ("serving" or "stopped").', inputSchema: { type: "object", properties: { steps: { type: "array", items: { type: "object" } } }, required: ["steps"] } },
];

/** One ask on the channel, with the secret first. A fresh connection each time. */
export function ask(run, line) {
  return new Promise((done, fail) => {
    const socket = net.connect({ host: "127.0.0.1", port: run.port });
    let heard = "";
    socket.setTimeout(30000, () => socket.destroy(new Error("the window gave no answer")));
    socket.on("connect", () => socket.write(`${run.secret}\n${line}\n`));
    socket.on("data", (data) => {
      heard += data;
      if (heard.endsWith("\n")) {
        socket.end();
        done(JSON.parse(heard));
      }
    });
    socket.on("error", fail);
    socket.on("close", () => fail(new Error("the window closed the connection")));
  });
}

const page = (run, what) => ask(run, `page ${JSON.stringify(what)}`);

/** The window and core one server or command opened. It holds nothing else. */
export class Control {
  run = null;
  need() {
    if (!this.run) throw new Error("Nothing is started here. Call start first.");
    if (this.run.exited) throw new Error(`The window this started has gone: ${this.run.said.trim() || "no words"}`);
    return this.run;
  }

  start(options) {
    if (this.run || this.starting) return Promise.reject(new Error("One is already started. Stop it first."));
    this.starting = this.open(options).finally(() => (this.starting = null));
    return this.starting;
  }

  async open({ data_folder, repos = {}, app = APP, env = {}, wait_ms = 60000 } = {}) {
    if (!fs.existsSync(app)) throw new Error(`No test build at ${app}. Nothing was built. Run: node scripts/test-control.mjs build`);
    if (!fs.readFileSync(app).includes(MARK)) throw new Error(`${app} has no test control, so it was not opened. It may be a normal build. Run: node scripts/test-control.mjs build`);
    const core = path.join(path.dirname(app), `astrolabe${EXE}`);
    if (!fs.existsSync(core)) throw new Error(`No core beside the test build, at ${core}. Run: node scripts/test-control.mjs build`);
    const made = !data_folder;
    const folder = data_folder ?? fs.mkdtempSync(path.join(os.tmpdir(), "astrolabe-test-"));
    if (!path.isAbsolute(folder)) throw new Error(`The data folder must be a full path, not ${folder}.`);
    // A core already there is not this server's to stop, so it won't open a window onto one.
    if (fs.existsSync(path.join(folder, "bridge.txt"))) throw new Error(`${folder} has a core, or one that did not stop cleanly. Use another folder.`);
    const file = path.join(folder, `${MARK}.txt`);
    fs.rmSync(file, { force: true });
    const childEnv = { ...process.env, ...env, ASTROLABE_DATA_DIR: folder };
    // The build's own rule, asked with no window, before the core is run on that folder at all.
    const checked = spawnSync(app, ["--check-folder"], { env: childEnv, encoding: "utf8" });
    if (checked.status !== 0) throw new Error(checked.stderr.trim() || "The test build refused its data folder.");
    for (const [name, checkout] of Object.entries(repos)) {
      const repo = spawnSync(core, ["repo", "add", name, checkout], { env: childEnv, encoding: "utf8" });
      if (repo.status !== 0) throw new Error(`repo add ${name}: ${repo.stderr.trim()}`);
    }
    const child = spawn(app, [], { env: childEnv, stdio: ["ignore", "ignore", "pipe"] });
    const run = { child, folder, made, app, file, said: "", shots: 0, exited: false };
    child.stderr.on("data", (data) => (run.said += data));
    child.on("exit", () => (run.exited = true));
    child.on("error", (why) => ((run.said += why.message), (run.exited = true)));
    const deadline = Date.now() + wait_ms;
    while (!run.exited && Date.now() < deadline) {
      // Not there yet, or still held open while it is written. Only this child's own will do.
      let said = "";
      try {
        said = fs.readFileSync(file, "utf8");
      } catch {}
      const [port, secret, pid] = said.split("\n");
      if (secret?.length === 64 && Number(pid) === child.pid) {
        Object.assign(run, { port: Number(port), secret });
        this.run = run;
        return { launched: app, pid: child.pid, data_folder: folder, made_folder: made };
      }
      await sleep(100);
    }
    if (run.exited) throw new Error(run.said.trim() || "The window ended before opening its channel.");
    child.kill();
    throw new Error("The window opened no channel in time, so it was closed.");
  }

  /** Quit the way the owner does, wait for the lock to go, then close this window. */
  async stop() {
    const run = this.need();
    const steps = [];
    if ((await ask(run, "core")).core === "serving") {
      const shown = await this.waitFor({ name: "Quit Astrolabe", role: "button", timeout_ms: 30000 });
      steps.push(shown, await page(run, { do: "click", name: "Quit Astrolabe", role: "button" }));
      await sleep(300);
      const { controls = [] } = await page(run, { do: "read" });
      if (controls.some((one) => one.name === "Stop and quit")) steps.push(await page(run, { do: "click", name: "Stop and quit" }));
      steps.push(await this.waitFor({ text: "Astrolabe has stopped", timeout_ms: 60000 }));
      steps.push(await this.waitFor({ core: "stopped", timeout_ms: 30000 }));
    }
    const failed = steps.find((step) => !step.ok);
    if (failed) throw new Error(`Quit did not finish, so the window stays: ${failed.why}`);
    await ask(run, "exit").catch(() => {});
    for (let i = 0; i < 100 && !run.exited; i++) await sleep(100);
    if (!run.exited) run.child.kill();
    this.run = null;
    return { stopped: run.app, pid: run.child.pid, data_folder: run.folder };
  }

  read() { return page(this.need(), { do: "read" }); }
  click({ name, role }) { return page(this.need(), { do: "click", name, role }); }
  type({ name, role, text }) { return page(this.need(), { do: "type", name, role, text }); }

  async screenshot() {
    const run = this.need();
    const said = await ask(run, "screenshot");
    if (!said.ok) return said;
    const file = path.join(run.folder, "shots", `${++run.shots}.png`);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, Buffer.from(said.png, "base64"));
    return { ok: true, file, png: said.png };
  }

  /** Reports what it last saw on a timeout, not just that time ran out. */
  async waitFor({ text, line, gone, name, role, core, timeout_ms = 15000 }) {
    const run = this.need();
    const deadline = Date.now() + timeout_ms;
    let seen = "";
    while (true) {
      if (core) {
        seen = (await ask(run, "core")).core;
        if (seen === core) return { ok: true, core };
      } else {
        const now = await page(run, { do: "read" });
        seen = now.text;
        const lines = (seen ?? "").split("\n").map((one) => one.trim());
        const control = name && now.controls?.some((one) => one.enabled && (one.name === name || one.name.includes(name)) && (!role || one.role === role));
        if (now.ok && (text === undefined || seen.includes(text)) && (line === undefined || lines.includes(line)) && (gone === undefined || !seen.includes(gone)) && (!name || control)) return { ok: true };
      }
      if (Date.now() > deadline) return { ok: false, why: `not seen within ${timeout_ms} ms`, saw: seen };
      await sleep(200);
    }
  }

  async runSteps({ steps }) {
    const results = [];
    for (const [index, step] of steps.entries()) {
      const began = Date.now();
      const { do: kind, ...args } = step;
      let said;
      try {
        if (kind === "read") said = await this.read();
        else if (kind === "click") said = await this.click(args);
        else if (kind === "type") said = await this.type(args);
        else if (kind === "wait_for") said = await this.waitFor(args);
        else if (kind === "sleep") said = (await sleep(args.ms ?? 0), { ok: true });
        else if (kind === "screenshot") {
          const { png, ...shot } = await this.screenshot();
          said = shot;
        } else said = { ok: false, why: `no such step: ${kind}` };
      } catch (why) {
        said = { ok: false, why: why.message };
      }
      results.push({ step: index + 1, do: kind, ms: Date.now() - began, ...said });
      if (!said.ok) return { ok: false, failed_at: index + 1, results };
    }
    return { ok: true, results };
  }
}

/** For the packaging check: the marker, in every file under these paths. */
export function scan(paths) {
  const found = [];
  let app = 0;
  const walk = (one) => {
    if (fs.statSync(one).isDirectory()) return fs.readdirSync(one).forEach((name) => walk(path.join(one, name)));
    if (path.basename(one).replace(/\.exe$/, "") === "astrolabe-desktop") app++;
    if (fs.readFileSync(one).includes(MARK)) found.push(one);
  };
  paths.forEach(walk);
  return { found, app };
}

function mcp() {
  const control = new Control();
  const send = (message) => process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
  const call = async (name, args) => {
    if (name === "start") return control.start({ data_folder: args.data_folder, repos: args.repos });
    if (name === "stop") return control.stop();
    if (name === "read") return control.read();
    if (name === "click") return control.click(args);
    if (name === "type") return control.type(args);
    if (name === "screenshot") return control.screenshot();
    if (name === "run_steps") return control.runSteps(args);
    throw new Error(`No such tool: ${name}`);
  };
  const lines = createInterface({ input: process.stdin });
  lines.on("line", async (line) => {
    const { id, method, params } = JSON.parse(line);
    if (id === undefined) return;
    if (method === "initialize") {
      send({ id, result: { protocolVersion: params.protocolVersion, capabilities: { tools: {} }, serverInfo: { name: "astrolabe-test-control", version: "0.0.0" } } });
    } else if (method === "ping") send({ id, result: {} });
    else if (method === "tools/list") send({ id, result: { tools: TOOLS } });
    else if (method === "tools/call") {
      try {
        const { png, ...said } = await call(params.name, params.arguments ?? {});
        const content = [{ type: "text", text: JSON.stringify(said, null, 2) }];
        if (png) content.push({ type: "image", data: png, mimeType: "image/png" });
        send({ id, result: { content, isError: said.ok === false } });
      } catch (why) {
        send({ id, result: { content: [{ type: "text", text: why.message }], isError: true } });
      }
    } else send({ id, error: { code: -32601, message: `No such method: ${method}` } });
  });
  // The session that wanted it has gone: leave nothing of this server's running.
  lines.on("close", async () => {
    await control.starting?.catch(() => {});
    await control.stop().catch(() => control.run?.child.kill());
    process.exit(0);
  });
}

function build() {
  for (const command of BUILD) {
    if (spawnSync(command, { cwd: ROOT, shell: true, stdio: "inherit" }).status !== 0) process.exit(1);
  }
  fs.copyFileSync(path.join(ROOT, "target/debug", `astrolabe${EXE}`), path.join(path.dirname(APP), `astrolabe${EXE}`));
}

async function command([verb, ...rest]) {
  if (verb === "build") return build();
  if (verb === "absent" || verb === "present") {
    const { found, app } = scan(rest);
    console.log(JSON.stringify({ found, app_programs: app }));
    const clean = found.length === 0;
    if (app === 0) console.error("No app program was in those paths, so this proves nothing.");
    process.exit(app > 0 && clean === (verb === "absent") ? 0 : 1);
  }
  if (verb !== "run" || !rest[0]) throw new Error("Usage: test-control.mjs [build | run <steps.json> [--data <folder>] [--repo <name>=<folder>] | absent|present <path>...]");
  const options = { repos: {} };
  rest.slice(1).forEach((word, i, all) => {
    if (word === "--data") options.data_folder = all[i + 1];
    if (word === "--repo") options.repos[all[i + 1].split("=")[0]] = all[i + 1].split("=").slice(1).join("=");
  });
  const control = new Control();
  const started = await control.start(options);
  const steps = await control.runSteps({ steps: JSON.parse(fs.readFileSync(rest[0], "utf8")) });
  // A stop that fails ends this window, as nothing is left to drive it. Its core is reported.
  const stopped = await control.stop().catch((why) => (control.run.child.kill(), { failed: why.message, core_may_remain_in: started.data_folder }));
  console.log(JSON.stringify({ started, ...steps, stopped }, null, 2));
  process.exit(steps.ok && !stopped.failed ? 0 : 1);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length > 2) command(process.argv.slice(2)).catch((why) => (console.error(why.message), process.exit(1)));
  else mcp();
}
