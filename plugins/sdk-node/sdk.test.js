"use strict";
const test = require("node:test");
const assert = require("node:assert");
const { PassThrough } = require("node:stream");
const { Plugin } = require("./index.js");

test("round trip with the host", async () => {
  const input = new PassThrough();
  const output = new PassThrough();
  const lines = [];
  let waiting = [];
  output.on("data", (d) => {
    for (const l of d.toString().split("\n").filter(Boolean)) {
      lines.push(JSON.parse(l));
      waiting.forEach((w) => w());
    }
  });
  const next = async () => {
    while (!lines.length) await new Promise((r) => waiting.push(r));
    waiting = [];
    return lines.shift();
  };
  const send = (m) => input.write(JSON.stringify(m) + "\n");

  const p = new Plugin({ input, output });
  p.onStart(() => p.setValue("ready", "yes"));
  p.action("ok", (s) => s.warn);
  p.action("boom", async () => {
    throw new Error("it broke");
  });
  const done = p.start();

  send({ event: "hello", protocol: 1, host: "test" });
  assert.deepStrictEqual(await next(), { event: "value", id: "ready", value: "yes" });
  assert.deepStrictEqual(await next(), { event: "ready" });
  send({ event: "run", id: 1, action: "ok", settings: {} });
  assert.deepStrictEqual(await next(), { event: "result", id: 1, ok: true });
  send({ event: "run", id: 2, action: "ok", settings: { warn: "careful" } });
  assert.deepStrictEqual(await next(), { event: "result", id: 2, ok: true, message: "careful" });
  const origError = console.error;
  console.error = () => {};
  send({ event: "run", id: 3, action: "boom", settings: {} });
  assert.deepStrictEqual(await next(), { event: "result", id: 3, ok: false, message: "it broke" });
  console.error = origError;
  send({ event: "run", id: 4, action: "nope" });
  assert.strictEqual((await next()).ok, false);
  send({ event: "shutdown" });
  await done;
});

test("settings, watched values and timers", async () => {
  const input = new PassThrough();
  const output = new PassThrough();
  const lines = [];
  output.on("data", (d) => {
    for (const l of d.toString().split("\n").filter(Boolean)) lines.push(JSON.parse(l));
  });
  const until = async (pred) => {
    for (let i = 0; i < 200; i++) {
      const hit = lines.find(pred);
      if (hit) return hit;
      await new Promise((r) => setTimeout(r, 10));
    }
    throw new Error("timed out: " + JSON.stringify(lines));
  };
  const send = (m) => input.write(JSON.stringify(m) + "\n");
  const p = new Plugin({ input, output });
  let ticks = 0;
  p.onSettings((s) => p.setValue("server", s.url ?? "none"));
  p.onWatch(() => p.watched("up").forEach((h) => p.setValue("up", "yes", h)));
  p.every(0.02, () => ticks++);
  const done = p.start();
  send({ event: "hello", protocol: 1, host: "t", settings: { url: "https://pve" } });
  await until((m) => m.id === "server" && m.value === "https://pve");
  send({ event: "watch", values: ["up:nas", "x:y"] });
  await until((m) => m.id === "up:nas");
  assert.deepStrictEqual(p.watched("up"), ["nas"]);
  await new Promise((r) => setTimeout(r, 80));
  assert.ok(ticks >= 2, `timer ran ${ticks} times`);
  send({ event: "shutdown" });
  await done;
});

function harness() {
  const input = new PassThrough();
  const output = new PassThrough();
  const lines = [];
  output.on("data", (d) => {
    for (const l of d.toString().split("\n").filter(Boolean)) lines.push(JSON.parse(l));
  });
  const until = async (pred, ms = 2000) => {
    for (let i = 0; i < ms / 10; i++) {
      const hit = lines.find(pred);
      if (hit) return hit;
      await new Promise((r) => setTimeout(r, 10));
    }
    throw new Error("timed out: " + JSON.stringify(lines));
  };
  const send = (m) => input.write(JSON.stringify(m) + "\n");
  return { input, output, lines, until, send };
}

test("every() never overlaps a slow tick", async () => {
  const h = harness();
  const p = new Plugin({ input: h.input, output: h.output });
  let running = 0;
  let most = 0;
  let ticks = 0;
  p.every(0.01, async () => {
    running++;
    most = Math.max(most, running);
    await new Promise((r) => setTimeout(r, 40));
    running--;
    ticks++;
  });
  const done = p.start();
  h.send({ event: "hello", protocol: 1, host: "t" });
  await new Promise((r) => setTimeout(r, 200));
  assert.strictEqual(most, 1, "ticks overlapped");
  assert.ok(ticks >= 2, `timer ran ${ticks} times`);
  h.send({ event: "shutdown" });
  await done;
  const after = ticks;
  await new Promise((r) => setTimeout(r, 100));
  assert.ok(ticks <= after + 1, "timer kept going after shutdown");
});

test("run waits for onStart, and callbacks keep their order", async () => {
  const h = harness();
  const p = new Plugin({ input: h.input, output: h.output });
  let started = false;
  const seen = [];
  p.onStart(async () => {
    await new Promise((r) => setTimeout(r, 50));
    started = true;
  });
  p.onSettings(async (s) => {
    if (s.slow) await new Promise((r) => setTimeout(r, 50));
    seen.push(`settings ${s.n}`);
  });
  p.onWatch((v) => seen.push(`watch ${v.join(",")}`));
  p.action("check", () => (started ? "started" : "too early"));
  const done = p.start();
  h.send({ event: "hello", protocol: 1, host: "t", settings: { n: 0 } });
  h.send({ event: "run", id: 1, action: "check", settings: {} });
  assert.deepStrictEqual(await h.until((m) => m.id === 1), { event: "result", id: 1, ok: true, message: "started" });
  const readyAt = h.lines.findIndex((m) => m.event === "ready");
  assert.ok(readyAt >= 0 && readyAt < h.lines.findIndex((m) => m.id === 1));
  h.send({ event: "settings", settings: { n: 1, slow: true } });
  h.send({ event: "watch", values: ["a:1"] });
  h.send({ event: "settings", settings: { n: 2 } });
  await h.until(() => seen.length === 4);
  assert.deepStrictEqual(seen, ["settings 0", "settings 1", "watch a:1", "settings 2"]);
  assert.strictEqual(p.settings.n, 2);
  h.send({ event: "shutdown" });
  await done;
});

test("dataDir uses the host's folder and creates it", async () => {
  const fs = require("node:fs");
  const os = require("node:os");
  const path = require("node:path");
  const { DATA_ENV, defaultDataDir } = require("./index.js");
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "perritodeck-"));
  const folder = path.join(tmp, "plugin-data", "dev.example");
  const old = process.env[DATA_ENV];
  process.env[DATA_ENV] = folder;
  try {
    assert.strictEqual(new Plugin().dataDir, folder);
    assert.ok(fs.statSync(folder).isDirectory());
  } finally {
    if (old === undefined) delete process.env[DATA_ENV];
    else process.env[DATA_ENV] = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  assert.strictEqual(
    defaultDataDir("x", { platform: "linux", env: {}, home: "/home/u" }),
    path.join("/home/u", ".local", "share", "perritodeck", "plugin-data", "x"),
  );
  assert.strictEqual(
    defaultDataDir("x", { platform: "win32", env: { LOCALAPPDATA: "C:\\L" }, home: "C:\\U" }),
    path.join("C:\\L", "PerritoDeck", "plugin-data", "x"),
  );
});
