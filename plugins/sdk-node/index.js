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
//
// Ordering: onSettings/onWatch callbacks run one after another in the order the host
// sent them; `run` messages wait until onSettings/onStart have finished after hello.
// An every() timer never overlaps itself: the next tick is scheduled `seconds` after
// the previous one settled. plugin.dataDir is a folder for the plugin's own files
// (PERRITODECK_PLUGIN_DATA from the host, or a per-user folder when run by hand),
// created the first time it is read.
//
// To open a web page, file or app for the person, use plugin.openForUser(target) or
// plugin.startForUser(program, args), not child_process: on Windows everything a plugin
// starts itself ends when the plugin stops, so the host opens these instead.
"use strict";

const { spawn } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const readline = require("node:readline");

const PROTOCOL = 1;
const DATA_ENV = "PERRITODECK_PLUGIN_DATA";
/** How long openForUser and startForUser wait for the host to say how it went. */
const OPEN_TIMEOUT_MS = 15000;

/** The plugin id from the manifest next to the main script, or the folder's name. */
function pluginName(main = process.argv[1]) {
  const folder = path.dirname(path.resolve(main || "."));
  try {
    const id = JSON.parse(fs.readFileSync(path.join(folder, "perritodeck-plugin.json"), "utf8")).id;
    if (typeof id === "string" && id) return id;
  } catch {
    // no manifest: use the folder's name
  }
  return path.basename(folder) || "plugin";
}

/** Where plugin files go when the host didn't say (a plugin run by hand). */
function defaultDataDir(name, { platform = process.platform, env = process.env, home = os.homedir() } = {}) {
  if (platform === "win32") {
    const base = env.LOCALAPPDATA || env.APPDATA || path.join(home, "AppData", "Local");
    return path.join(base, "PerritoDeck", "plugin-data", name);
  }
  if (platform === "darwin") return path.join(home, "Library", "Application Support", "PerritoDeck", "plugin-data", name);
  return path.join(env.XDG_DATA_HOME || path.join(home, ".local", "share"), "perritodeck", "plugin-data", name);
}

class Plugin {
  constructor({ input = process.stdin, output = process.stdout } = {}) {
    this._actions = new Map();
    this._onStart = [];
    this._onSettings = [];
    this._onWatch = [];
    this._timers = [];
    this._started = false;
    this._stopped = false;
    this._timeouts = new Set();
    this._ready = null; // resolves once hello's callbacks have run
    this._chain = Promise.resolve(); // hello/settings/watch callbacks, in order
    this._dataDir = null;
    this._input = input;
    this._output = output;
    this._opens = new Map(); // open requests waiting for the host's "opened"
    this._nextOpen = 1;
    this._hostOpens = false; // the host said hello with the "open" feature
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

  /** A folder for the plugin's own files, created if missing (see the top of this file). */
  get dataDir() {
    if (this._dataDir === null) {
      const folder = process.env[DATA_ENV] || defaultDataDir(pluginName());
      fs.mkdirSync(folder, { recursive: true });
      this._dataDir = folder;
    }
    return this._dataDir;
  }

  /** Run fn after the callbacks queued before it (they keep the host's order). */
  _enqueue(fn) {
    this._chain = this._chain.then(fn).catch((e) => console.error(e));
    return this._chain;
  }

  _startTimer({ seconds, fn, now }) {
    const schedule = (delay) => {
      if (this._stopped) return;
      const h = setTimeout(() => {
        this._timeouts.delete(h);
        tick();
      }, delay);
      h.unref?.();
      this._timeouts.add(h);
    };
    // The next tick is scheduled only after this one settled, so ticks never overlap.
    const tick = () =>
      Promise.resolve()
        .then(fn)
        .catch((e) => console.error(e))
        .finally(() => schedule(seconds * 1000));
    if (now) tick();
    else schedule(seconds * 1000);
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

  /**
   * Open a web address, file, folder or app link for the person, the way double-clicking
   * would. The host opens it, so it keeps running after the plugin stops. Resolves once
   * it's open; rejects with the reason when it couldn't be.
   */
  openForUser(target) {
    const opener = { win32: "explorer.exe", darwin: "open" }[process.platform] ?? "xdg-open";
    return this._open({ target: String(target) }, () => [opener, [String(target)]]);
  }

  /** Start a program for the person: startForUser("code", ["."]). Like openForUser, the
   * host starts it, so it keeps running after the plugin stops. */
  startForUser(program, args = []) {
    const argv = args.map(String);
    return this._open({ program: String(program), args: argv }, () => [String(program), argv]);
  }

  _open(fields, fallback) {
    if (!this._hostOpens) {
      // A host from before the "open" message: start it here, as plugins used to (on
      // Windows it then ends with the plugin).
      return new Promise((resolve, reject) => {
        const [cmd, args] = fallback();
        const child = spawn(cmd, args, { detached: true, stdio: "ignore", windowsHide: true });
        child.once("error", reject);
        child.once("spawn", () => {
          child.unref();
          resolve();
        });
      });
    }
    const id = this._nextOpen++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this._opens.delete(id);
        reject(new Error("PerritoDeck didn't say whether it could open it"));
      }, OPEN_TIMEOUT_MS);
      timer.unref?.();
      this._opens.set(id, { resolve, reject, timer });
      this._send({ event: "open", id, ...fields });
    });
  }

  _opened(m) {
    const waiting = this._opens.get(m.id);
    if (!waiting) return;
    this._opens.delete(m.id);
    clearTimeout(waiting.timer);
    if (m.ok) waiting.resolve();
    else waiting.reject(new Error(m.message || "it couldn't be opened"));
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
        if (!m || typeof m !== "object") return;
        if (m.event === "hello") {
          this.hostVersion = m.host;
          this._hostOpens = Array.isArray(m.features) && m.features.includes("open");
          this.settings = m.settings ?? {};
          const settings = this.settings;
          this._ready = this._enqueue(async () => {
            await this._call(this._onSettings, settings);
            await this._call(this._onStart);
            this._send({ event: "ready" });
            if (!this._started) {
              this._started = true;
              for (const t of this._timers) this._startTimer(t);
            }
          });
        } else if (m.event === "settings") {
          this.settings = m.settings ?? {};
          const settings = this.settings;
          this._enqueue(() => this._call(this._onSettings, settings));
        } else if (m.event === "watch") {
          this.watching = new Set(m.values ?? []);
          const values = [...this.watching].sort();
          this._enqueue(() => this._call(this._onWatch, values));
        } else if (m.event === "run") {
          // Actions wait for onStart (a host only sends them after ready, but be safe).
          Promise.resolve(this._ready).then(() => this._run(m.id, m.action, m.settings));
        } else if (m.event === "opened") {
          this._opened(m);
        } else if (m.event === "shutdown") {
          rl.close();
        }
      });
      rl.on("close", () => {
        this._stopped = true;
        for (const h of this._timeouts) clearTimeout(h);
        this._timeouts.clear();
        for (const { reject, timer } of this._opens.values()) {
          clearTimeout(timer);
          reject(new Error("PerritoDeck stopped the plugin"));
        }
        this._opens.clear();
        resolve();
      });
    });
  }
}

module.exports = { Plugin, PROTOCOL, DATA_ENV, defaultDataDir };
