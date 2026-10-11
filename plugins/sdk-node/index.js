// PerritoDeck plugin SDK for Node.js (no dependencies). MIT licensed.
//
//   const { Plugin } = require("./perritodeck-plugin");
//   const plugin = new Plugin();
//   plugin.action("say-hello", async (settings) => {
//     plugin.log(`hello, ${settings.name ?? "world"}`);
//     // return a string to flash a warning on the key; throw to flash an error
//   });
//   plugin.start();
//
//   plugin.onSettings((s) => { /* the plugin's own settings (Plugins page) */ });
//   plugin.every(5, () => {
//     for (const host of plugin.watched("up")) plugin.setValue("up", check(host), host);
//   });
//
// The host talks to the plugin over stdin/stdout, one JSON object per line (see
// PROTOCOL.md). Don't write to stdout yourself: use plugin.log or console.error.
"use strict";

const readline = require("node:readline");

const PROTOCOL = 1;

class Plugin {
  constructor({ input = process.stdin, output = process.stdout } = {}) {
    this._actions = new Map();
    this._onStart = [];
    this._onSettings = [];
    this._onWatch = [];
    this._timers = [];
    this._started = false;
    this._input = input;
    this._output = output;
    this.hostVersion = null;
    /** The plugin's own settings, as saved on the Plugins page. */
    this.settings = {};
    /** Values with an argument that keys show, e.g. "up:192.168.1.10". */
    this.watching = new Set();
  }

  /** Run fn(settings) whenever the plugin's settings arrive or change. */
  onSettings(fn) {
    this._onSettings.push(fn);
    return this;
  }

  /** Run fn(values) when the set of watched values changes. */
  onWatch(fn) {
    this._onWatch.push(fn);
    return this;
  }

  /** Run fn() every `seconds` once the host said hello (errors are logged). */
  every(seconds, fn, { now = true } = {}) {
    this._timers.push({ seconds, fn, now });
    return this;
  }

  /** The arguments keys use with a value: ["192.168.1.10", …] for up:<host>. */
  watched(id) {
    const prefix = id + ":";
    return [...this.watching].filter((v) => v.startsWith(prefix)).map((v) => v.slice(prefix.length)).sort();
  }

  async _call(fns, ...args) {
    for (const fn of fns) {
      try {
        await fn(...args);
      } catch (e) {
        console.error(e);
      }
    }
  }

  /** Register a handler (settings) => void | string | Promise<void | string>. */
  action(id, handler) {
    this._actions.set(id, handler);
    return this;
  }

  /** Run fn() once the host has said hello (before any action). */
  onStart(fn) {
    this._onStart.push(fn);
    return this;
  }

  /** Publish a live value: {plugin:<plugin id>:<value id>}, or with `arg`,
   * {plugin:<plugin id>:<value id>:<arg>}. */
  setValue(id, value, arg) {
    const full = arg === undefined || arg === null ? id : `${id}:${arg}`;
    this._send({ event: "value", id: full, value: String(value) });
  }

  log(message, level = "info") {
    this._send({ event: "log", level, message: String(message) });
  }

  _send(message) {
    this._output.write(JSON.stringify(message) + "\n");
  }

  async _run(id, actionId, settings) {
    const handler = this._actions.get(actionId);
    if (!handler) {
      this._send({ event: "result", id, ok: false, message: `unknown action ${actionId}` });
      return;
    }
    try {
      const warning = await handler(settings ?? {});
      const result = { event: "result", id, ok: true };
      if (typeof warning === "string" && warning) result.message = warning;
      this._send(result);
    } catch (e) {
      console.error(e);
      this._send({ event: "result", id, ok: false, message: (e && e.message) || String(e) });
    }
  }

  /** Talk to the host until it says goodbye. Resolves when the host closes. */
  start() {
    const rl = readline.createInterface({ input: this._input, crlfDelay: Infinity });
    return new Promise((resolve) => {
      rl.on("line", async (line) => {
        let m;
        try {
          m = JSON.parse(line);
        } catch {
          return;
        }
        if (m.event === "hello") {
          this.hostVersion = m.host;
          this.settings = m.settings ?? {};
          await this._call(this._onSettings, this.settings);
          await this._call(this._onStart);
          this._send({ event: "ready" });
          if (!this._started) {
            this._started = true;
            for (const t of this._timers) {
              const tick = () => Promise.resolve().then(t.fn).catch((e) => console.error(e));
              if (t.now) tick();
              const h = setInterval(tick, t.seconds * 1000);
              h.unref?.();
              this._intervals = [...(this._intervals ?? []), h];
            }
          }
        } else if (m.event === "settings") {
          this.settings = m.settings ?? {};
          this._call(this._onSettings, this.settings);
        } else if (m.event === "watch") {
          this.watching = new Set(m.values ?? []);
          this._call(this._onWatch, [...this.watching].sort());
        } else if (m.event === "run") {
          this._run(m.id, m.action, m.settings);
        } else if (m.event === "shutdown") {
          rl.close();
        }
      });
      rl.on("close", () => {
        for (const h of this._intervals ?? []) clearInterval(h);
        resolve();
      });
    });
  }
}

module.exports = { Plugin, PROTOCOL };
