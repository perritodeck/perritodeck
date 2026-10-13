// End-to-end test of the real desktop app (Linux): drives the app's WebView through
// tauri-driver/WebKitWebDriver, pairs fakedeck with the host inside the app, edits a
// key in the editor, and checks the change lands in the profile file and on the
// "tablet". Run under a display (xvfb-run in CI):
//
//   APP=src-tauri/target/debug/perritodeck-app FAKEDECK=../target/debug/perritodeck-fakedeck node e2e/real-app.mjs

import { spawn, execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, existsSync, cpSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const app = resolve(process.env.APP ?? "src-tauri/target/debug/perritodeck-app");
const fakedeck = resolve(process.env.FAKEDECK ?? "../target/debug/perritodeck-fakedeck");
const dataDir = mkdtempSync(join(tmpdir(), "perritodeck-e2e-"));
const profilePath = join(dataDir, "profiles", "default.toml");
const fdState = join(dataDir, "fakedeck.json");
const W = "http://127.0.0.1:4444";
// A Stream Deck plugin with a settings page, preinstalled.
const sdFixture = resolve("../crates/sdcompat/tests/fixtures/com.perritodeck.test-counter.sdPlugin");
cpSync(sdFixture, join(dataDir, "sdplugins", "com.perritodeck.test-counter.sdPlugin"), { recursive: true });
// The MCP server for AI assistants, on (it's off by default), on a port of its own.
const mcpPort = 46911;
writeFileSync(join(dataDir, "app-settings.json"), JSON.stringify({ mcp: { enabled: true, port: mcpPort } }));

const children = [];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
function log(...a) {
  console.log("[e2e]", ...a);
}

async function waitFor(what, fn, ms = 20000) {
  const until = Date.now() + ms;
  let last;
  while (Date.now() < until) {
    try {
      const v = await fn();
      if (v) return v;
    } catch (e) {
      last = e;
    }
    await sleep(200);
  }
  throw new Error(`timed out waiting for ${what}${last ? `: ${last.message}` : ""}`);
}

async function wd(method, path, body) {
  const res = await fetch(W + path, {
    method,
    headers: { "content-type": "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
  const json = await res.json();
  if (!res.ok || json.value?.error) throw new Error(`${method} ${path}: ${JSON.stringify(json.value)}`);
  return json.value;
}

let session;
const el = async (css) => {
  const v = await wd("POST", `/session/${session}/element`, { using: "css selector", value: css });
  return Object.values(v)[0];
};
const js = (script, ...args) => wd("POST", `/session/${session}/execute/sync`, { script, args });
const click = async (css) => wd("POST", `/session/${session}/element/${await waitFor(css, () => el(css))}/click`, {});
const text = async (css) => wd("GET", `/session/${session}/element/${await el(css)}/text`);

function start(cmd, args, env = {}) {
  const p = spawn(cmd, args, { env: { ...process.env, ...env }, stdio: ["ignore", "pipe", "pipe"] });
  p.out = "";
  p.stdout.on("data", (d) => (p.out += d));
  p.stderr.on("data", (d) => (p.out += d));
  children.push(p);
  return p;
}

let failed = false;
try {
  const driver = start("tauri-driver", [], { PERRITODECK_DATA_DIR: dataDir, RUST_LOG: "info" });
  await waitFor("tauri-driver", () => fetch(W + "/status").then((r) => r.ok));
  session = (
    await wd("POST", "/session", {
      capabilities: { alwaysMatch: { "tauri:options": { application: app }, "webkitgtk:browserOptions": {} } },
    })
  ).sessionId;
  log("app started; data in", dataDir);

  // First run: nothing paired, so the app opens on pairing, with a real code and QR.
  await waitFor("pairing view", () => js("return document.querySelector('h1')?.textContent === 'Pair a tablet'"));
  const code = await waitFor("pairing code", async () => {
    const label = await js("return document.querySelector('.code')?.getAttribute('aria-label')");
    return label?.replace(/\D/g, "");
  });
  if (!(await js("return !!document.querySelector('.qr svg')"))) throw new Error("no QR code drawn");
  log("pairing code", code);

  execFileSync(fakedeck, ["--state", fdState, "--device-id", "e2e-tablet", "pair", code], { stdio: "inherit" });
  await waitFor("the app to say it paired", () =>
    js("return document.querySelector('h2')?.textContent?.startsWith('Paired with')"),
  );
  const watch = start(fakedeck, ["--state", fdState, "--device-id", "e2e-tablet", "watch"]);
  await waitFor("fakedeck to get the layout", () => watch.out.includes("Copy"));

  // Edit a key in the editor.
  await click("nav li:first-child button"); // Buttons
  await click('[aria-label^="Copy:"]');
  const title = await waitFor("title field", () => el("#f1"));
  await wd("POST", `/session/${session}/element/${title}/clear`, {});
  await wd("POST", `/session/${session}/element/${title}/value`, { text: "Copied" });

  await waitFor("the profile file to change", () => readFileSync(profilePath, "utf8").includes('title = "Copied"'));
  const saved = readFileSync(profilePath, "utf8");
  if (!saved.includes("# ")) throw new Error("comments were lost from the profile");
  log("saved to the profile file, comments kept");
  await waitFor("the tablet to show the new title", () => watch.out.includes("Copied"), 5000);
  log("the tablet shows it");

  // Undo restores the file.
  await click("body"); // leave the text field so Ctrl+Z isn't the field's own undo
  await waitFor("undo enabled", () => js("return [...document.querySelectorAll('button')].some(b => b.textContent.trim() === 'Undo' && !b.disabled)"));
  await js("[...document.querySelectorAll('button')].find(b => b.textContent.trim() === 'Undo').click()");
  await waitFor("undo to restore the file", () => !readFileSync(profilePath, "utf8").includes('"Copied"'));
  log("undo restored the file");

  // Tablets view lists the paired fakedeck as connected.
  await click("nav li:nth-child(2) button");
  await waitFor("tablet listed", async () => (await text("main")).includes("Connected now"));
  log("tablet listed as connected");

  // The plugins that ship with the app are offered, and one can be added and used.
  await click("nav li:nth-child(5) button"); // Plugins
  await waitFor("plugin catalog", async () => (await text("main")).includes("HTTP requests"));
  await js("[...document.querySelectorAll('li')].find(l => l.textContent.includes('HTTP requests'))?.querySelector('button')?.click()");
  await waitFor("plugin running", async () => (await text("main")).includes("Running"), 30000);
  log("bundled HTTP plugin installed and running");
  await waitFor("Stream Deck plugin host", async () => (await text("main")).includes("Stream Deck plugins"));
  log("Stream Deck plugin host is up");

  // A Stream Deck plugin's own settings page opens in the editor and talks to the host.
  mkdirSync(join(dataDir, "profiles"), { recursive: true });
  writeFileSync(
    join(dataDir, "profiles", "sd.toml"),
    `schema_version = 1\nname = "SD"\n[[pages]]\nname = "Main"\n[[pages.keys]]\nat = [0, 0]\ntitle = "Count"\n` +
      `action = { type = "sdPlugin", plugin = "com.perritodeck.test-counter", action = "com.perritodeck.test-counter.up" }\n`,
  );
  await click("nav li:first-child button"); // Buttons
  await waitFor("the SD profile to load", () => js("return [...document.querySelectorAll('#profile-select option')].some(o => o.value === 'sd')"));
  await js("const s = document.querySelector('#profile-select'); s.value = 'sd'; s.dispatchEvent(new Event('change', { bubbles: true }));");
  await waitFor("the SD profile's key", () => js("return [...document.querySelectorAll('[aria-label]')].some(e => e.getAttribute('aria-label').startsWith('Count'))"), 10000).catch(async (e) => {
    console.error(await js("return document.querySelector('#profile-select')?.value + ' | ' + [...document.querySelectorAll('[aria-label]')].map(e => e.getAttribute('aria-label')).slice(0, 12).join(' / ')"));
    throw e;
  });
  await click('[aria-label^="Count"]');
  await waitFor("settings button", () => js("return [...document.querySelectorAll('button')].some(b => b.textContent.trim() === 'Open its settings')"));
  await js("[...document.querySelectorAll('button')].find(b => b.textContent.trim() === 'Open its settings').click()");
  const frame = await waitFor("settings frame", () => el("iframe.sd-page"));
  void frame;
  await wd("POST", `/session/${session}/frame`, { id: 0 }); // the only frame on the page
  await waitFor("the page's field", () => el("#step"));
  // Give the page a moment to connect, then change its field as a person would.
  await sleep(1000);
  await js("const i = document.getElementById('step'); i.value = '7'; i.dispatchEvent(new Event('change', { bubbles: true }));");
  await wd("POST", `/session/${session}/frame/parent`, {});
  await waitFor("the plugin's settings to be saved", () => {
    const f = join(dataDir, "sdplugins", "settings.json");
    return existsSync(f) && readFileSync(f, "utf8").includes('"step": 7');
  });
  log("Stream Deck settings page works in the real app");

  // Settings: the MCP server runs, and assistants start the bridge the app copied into
  // its data folder (perritodeck-mcp), so they never keep the app's own program in use.
  const settingsTab = "[...document.querySelectorAll('nav button')].find(b => b.textContent.includes('Settings')).click()";
  await js(settingsTab);
  await waitFor("MCP running", async () => (await text("main")).includes(`Running at http://127.0.0.1:${mcpPort}/mcp`));
  const bridgeCopy = join(dataDir, "bin", process.platform === "win32" ? "perritodeck-mcp.exe" : "perritodeck-mcp");
  if (!existsSync(bridgeCopy)) throw new Error(`the app didn't copy its bridge to ${bridgeCopy}`);
  const claudeCode = await js("return document.querySelector('[aria-label=\"Claude Code command\"]').textContent");
  if (!claudeCode.includes(bridgeCopy)) throw new Error(`the Claude Code command doesn't start the copied bridge: ${claudeCode}`);
  const rpc = [
    { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-11-25", capabilities: {}, clientInfo: { name: "e2e", version: "1" } } },
    { jsonrpc: "2.0", method: "notifications/initialized" },
    { jsonrpc: "2.0", id: 2, method: "tools/list" },
    { jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "list_profiles", arguments: {} } },
  ];
  const throughBridge = (command, args) => {
    const bridged = spawnSync(command, args, {
      input: rpc.map((m) => JSON.stringify(m)).join("\n") + "\n",
      encoding: "utf8",
      timeout: 20000,
    });
    const answers = bridged.stdout.trim().split("\n").filter(Boolean).map((l) => JSON.parse(l));
    if (answers.length !== 3) throw new Error(`${command} answered ${answers.length} of 3: ${bridged.stdout} ${bridged.stderr}`);
    if (answers[0].result?.protocolVersion !== "2025-11-25") throw new Error(`initialize: ${JSON.stringify(answers[0])}`);
    if (!answers[1].result.tools.some((t) => t.name === "list_profiles")) throw new Error("no list_profiles tool");
    if (answers[1].result.tools.some((t) => t.name === "press_key")) throw new Error("press_key offered without permission");
    if (!answers[2].result.content[0].text.includes("SD")) throw new Error(`list_profiles: ${JSON.stringify(answers[2])}`);
  };
  throughBridge(bridgeCopy, ["--data-dir", dataDir]);
  log("MCP over the copied perritodeck-mcp bridge works");
  // Assistants set up before the bridge start the app itself; that still works, and
  // Settings asks to set them up again.
  throughBridge(app, ["mcp", "--data-dir", dataDir]);
  await js("[...document.querySelectorAll('nav button')].find(b => b.textContent.includes('Buttons')).click()");
  await js(settingsTab);
  await waitFor("the note about the older setup", async () => (await text("main")).includes("older command that starts PerritoDeck itself"));
  log("MCP over the app's own bridge still works, and Settings asks to set it up again");

  // Logs are written to files next to the data, with the secrets left out.
  const logFile = join(dataDir, "logs", "perritodeck.log");
  await waitFor("the log file", () => existsSync(logFile) && readFileSync(logFile, "utf8").includes("host running"));
  const token = readFileSync(join(dataDir, "mcp-token"), "utf8").trim();
  if (readFileSync(logFile, "utf8").includes(token)) throw new Error("the MCP token is in the log");
  log("log file written, no token in it");
  log("PASS");
} catch (e) {
  failed = true;
  console.error("[e2e] FAIL:", e.message);
  for (const c of children) console.error(`--- ${c.spawnargs.slice(0, 2).join(" ")} output ---\n${(c.out.match(/thread .* panicked at[^\n]*\n[^\n]*/)?.[0] ?? "") + "\n" + c.out.slice(-3000)}`);
  if (existsSync(profilePath)) console.error("--- profile ---\n" + readFileSync(profilePath, "utf8").slice(0, 3000));
} finally {
  if (session) await wd("DELETE", `/session/${session}`).catch(() => {});
  for (const c of children) c.kill();
}
process.exit(failed ? 1 : 0);
