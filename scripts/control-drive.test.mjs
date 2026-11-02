// Drives a real test build through its channel, with no agent login. Needs a window and the
// result of `node scripts/test-control.mjs build`, so it runs in CI's control job, not the gates.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { after, test } from "node:test";
import { fileURLToPath } from "node:url";
import { Control, MARK } from "./test-control.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const EXE = process.platform === "win32" ? ".exe" : "";
const APP = path.join(ROOT, "app/src-tauri/target/debug", `astrolabe-desktop${EXE}`);
const CORE = path.join(path.dirname(APP), `astrolabe${EXE}`);
const base = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "astrolabe-drive-")));
// A stand-in home, so the "default" folder here is never the machine's real one.
const home = path.join(base, "home");
const homes = { HOME: home, USERPROFILE: home, LOCALAPPDATA: path.join(home, "local") };
const standIn = process.platform === "win32" ? path.join(homes.LOCALAPPDATA, "Astrolabe") : path.join(home, "Library/Application Support/Astrolabe");
const refused = (env) => {
  const clean = { ...process.env, ...homes, ...env };
  if (!env.ASTROLABE_DATA_DIR) delete clean.ASTROLABE_DATA_DIR;
  return spawnSync(APP, [], { env: clean, encoding: "utf8", timeout: 60000 });
};
const repo = path.join(base, "repo");
fs.mkdirSync(repo, { recursive: true });
spawnSync("git", ["init", "-q", "-b", "main", repo]);
spawnSync("git", ["-C", repo, "-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-q", "--allow-empty", "-m", "init"]);
after(() => fs.rmSync(base, { recursive: true, force: true, maxRetries: 5 }));

/** A socket's whole answer to what is written on it. */
const heard = (port, text) =>
  new Promise((done, fail) => {
    const socket = net.connect({ host: "127.0.0.1", port }, () => socket.write(text));
    let all = "";
    socket.on("data", (data) => (all += data));
    socket.on("close", () => done(all));
    socket.on("error", fail);
  });

test("no data folder set: refused, and says why", () => {
  const run = refused({});
  assert.equal(run.status, 3);
  assert.match(run.stderr, /ASTROLABE_DATA_DIR is not set/);
});

test("the default data folder, or inside it, or by another spelling: refused", () => {
  for (const folder of [standIn, path.join(standIn, "x"), path.join(path.dirname(standIn), "astrolabe"), [path.dirname(standIn), "x", "..", "Astrolabe"].join(path.sep)]) {
    const run = refused({ ASTROLABE_DATA_DIR: folder });
    assert.equal(run.status, 3, folder);
    assert.match(run.stderr, /default data folder|plain full path/);
  }
  if (process.platform !== "win32") {
    fs.mkdirSync(standIn, { recursive: true });
    const link = path.join(base, "link");
    fs.symlinkSync(standIn, link);
    assert.equal(refused({ ASTROLABE_DATA_DIR: link }).status, 3, "through a link");
    fs.rmSync(link);
    fs.rmSync(standIn, { recursive: true });
  }
  assert.ok(!fs.existsSync(standIn), "a refused run made the default folder");
});

test("start refuses a normal build, and a core already in the folder, launching nothing", async () => {
  const fake = path.join(base, `plain${EXE}`);
  fs.writeFileSync(fake, "no marker here");
  await assert.rejects(new Control().start({ app: fake }), /has no test control/);
  const busy = path.join(base, "busy");
  fs.mkdirSync(busy);
  fs.writeFileSync(path.join(busy, "bridge.txt"), "1\nx\n");
  await assert.rejects(new Control().start({ data_folder: busy, env: homes }), /has a core/);
});

test("beside a live core: the window drives its own, and the live one is untouched", async () => {
  const liveEnv = { ...process.env, ...homes };
  delete liveEnv.ASTROLABE_DATA_DIR;
  const stand = spawn(CORE, ["serve"], { env: liveEnv, stdio: "ignore" });
  const bridge = path.join(standIn, "bridge.txt");
  for (let i = 0; i < 300 && !fs.existsSync(bridge); i++) await new Promise((done) => setTimeout(done, 100));
  const before = fs.readFileSync(bridge, "utf8");
  const control = new Control();
  try {
    const data = path.join(base, "data");
    const started = await control.start({ data_folder: data, repos: { throwaway: repo }, env: homes });
    assert.equal(started.data_folder, data);
    const { run } = control;

    // The channel is private.
    const file = fs.readFileSync(path.join(data, `${MARK}.txt`), "utf8");
    if (process.platform !== "win32") assert.equal(fs.statSync(path.join(data, `${MARK}.txt`)).mode & 0o777, 0o600);
    assert.ok(file.includes(run.secret));
    assert.match(await heard(run.port, "\n"), /refused/);
    assert.match(await heard(run.port, "wrong\npage {\"do\":\"read\"}\n"), /refused/);
    assert.match(await heard(run.port, `${run.secret.slice(0, 63)}\ncore\n`), /refused/);
    assert.doesNotMatch(await heard(run.port, "GET / HTTP/1.1\r\nHost: x\r\n\r\n"), /serving|stopped/);
    const outside = Object.values(os.networkInterfaces()).flat().find((one) => !one.internal && one.family === "IPv4");
    if (outside) {
      const reached = await new Promise((done) => {
        const socket = net.connect({ host: outside.address, port: run.port, timeout: 2000 });
        socket.on("connect", () => (socket.destroy(), done(true)));
        socket.on("error", () => done(false));
        socket.on("timeout", () => (socket.destroy(), done(false)));
      });
      assert.equal(reached, false, "reachable from outside loopback");
    }
    assert.ok(!run.said.includes(run.secret), "the secret reached the window's output");
    await assert.rejects(control.start({ env: homes }), /already started/);

    // Real clicks, read back from the page.
    const steps = await control.runSteps({
      steps: [
        { do: "wait_for", name: "Quit Astrolabe", timeout_ms: 60000 },
        { do: "wait_for", core: "serving" },
        { do: "read" },
        { do: "screenshot" },
        { do: "click", name: "Quit Astrolabe" },
        { do: "wait_for", text: "Astrolabe has stopped", timeout_ms: 60000 },
        { do: "wait_for", core: "stopped", timeout_ms: 30000 },
        { do: "click", name: "Start" },
        { do: "wait_for", name: "Quit Astrolabe", timeout_ms: 60000 },
      ],
    });
    assert.equal(steps.ok, true, JSON.stringify(steps));
    const repos = steps.results[2].controls.find((one) => one.name === "Repo");
    assert.deepEqual(repos.options, ["throwaway"]);
    const picture = fs.readFileSync(steps.results[3].file);
    assert.equal(picture.subarray(1, 4).toString(), "PNG");
    assert.ok(picture.length > 1000);
    fs.mkdirSync(path.join(ROOT, "control-shots"), { recursive: true });
    fs.copyFileSync(steps.results[3].file, path.join(ROOT, "control-shots", "window.png"));

    // Fails on what is not there, and what can't be pressed, and stops at the first failure.
    const missing = await control.runSteps({ steps: [{ do: "click", name: "No such button" }, { do: "read" }] });
    assert.equal(missing.ok, false);
    assert.equal(missing.results.length, 1);
    assert.match(missing.results[0].why, /nothing named/);
    assert.match((await control.type({ name: "Quit Astrolabe", text: "x" })).why, /takes no text/);
    const late = await control.runSteps({ steps: [{ do: "wait_for", text: "never shown", timeout_ms: 300 }] });
    assert.ok(late.results[0].saw.includes("Quit Astrolabe"), "a timeout reports what it saw");

    const stopped = await control.stop();
    assert.equal(stopped.data_folder, data);
    assert.ok(!fs.existsSync(path.join(data, "bridge.txt")), "its core's file should be gone");
    await assert.rejects(control.stop(), /Nothing is started here/);

    assert.equal(fs.readFileSync(bridge, "utf8"), before, "the live core's file changed");
    assert.equal(stand.exitCode, null, "the live core stopped");
  } finally {
    if (control.run) control.run.child.kill();
    stand.kill();
  }
});
