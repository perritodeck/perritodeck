// End-to-end test of the real desktop app (Linux): drives the app's WebView through
// tauri-driver/WebKitWebDriver, pairs fakedeck with the host inside the app, edits a
// key in the editor, and checks the change lands in the profile file and on the
// "tablet". Run under a display (xvfb-run in CI):
//
//   APP=src-tauri/target/debug/perritodeck-app FAKEDECK=../target/debug/perritodeck-fakedeck node e2e/real-app.mjs

import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const app = resolve(process.env.APP ?? "src-tauri/target/debug/perritodeck-app");
const fakedeck = resolve(process.env.FAKEDECK ?? "../target/debug/perritodeck-fakedeck");
const dataDir = mkdtempSync(join(tmpdir(), "perritodeck-e2e-"));
const profilePath = join(dataDir, "profiles", "default.toml");
const fdState = join(dataDir, "fakedeck.json");
const W = "http://127.0.0.1:4444";

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
