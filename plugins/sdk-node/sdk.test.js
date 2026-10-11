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
