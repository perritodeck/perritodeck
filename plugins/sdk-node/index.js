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
// The host talks to the plugin over stdin/stdout, one JSON object per line (see
// PROTOCOL.md). Don't write to stdout yourself: use plugin.log or console.error.
"use strict";

const readline = require("node:readline");

const PROTOCOL = 1;

class Plugin {
  constructor({ input = process.stdin, output = process.stdout } = {}) {
    this._actions = new Map();
    this._onStart = [];
    this._input = input;
    this._output = output;
    this.hostVersion = null;
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

  /** Publish a live value; key titles show it as {plugin:<plugin id>:<value id>}. */
  setValue(id, value) {
    this._send({ event: "value", id, value: String(value) });
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
          for (const fn of this._onStart) {
            try {
              await fn();
            } catch (e) {
              console.error(e);
            }
          }
          this._send({ event: "ready" });
        } else if (m.event === "run") {
          this._run(m.id, m.action, m.settings);
        } else if (m.event === "shutdown") {
          rl.close();
        }
      });
      rl.on("close", resolve);
    });
  }
}

module.exports = { Plugin, PROTOCOL };
