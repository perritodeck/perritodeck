#!/usr/bin/env node
// Screenshots of the PerritoDeck desktop app for the website and stores.
//
// The app's UI runs in a plain browser against the mock host (host/app/src/lib/mock.ts),
// which this script fills with a realistic setup through `window.__perritoMock`. Every
// view is saved at 2x device scale, light and dark, to docs/assets/screenshot-<name>[-dark].png.
//
//   cd host/app && npm ci            # once; Playwright comes with the dev dependencies
//   node scripts/screenshots.mjs     # from the repo root
//
// It reuses a dev server already on port 5174 (`npx vite --port 5174`) or starts one.
// PW_CHROMIUM=/path/to/chromium picks the browser; by default Playwright's own is used,
// or /opt/pw-browsers/chromium when it exists. `--only buttons,plugins` limits the views.

import { spawn } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const appDir = join(root, "host/app");
const outDir = join(root, "docs/assets");
const require = createRequire(join(appDir, "package.json"));
const { chromium } = require("playwright");

const PORT = 5174;
const BASE = `http://localhost:${PORT}/`;
const VIEWPORT = { width: 1280, height: 800 };

const onlyArg = process.argv.indexOf("--only");
const only = onlyArg > 0 ? new Set(process.argv[onlyArg + 1].split(",")) : null;
const setArg = process.argv.indexOf("--set");
/** "desktop", "store" or "all". */
const set = setArg > 0 ? process.argv[setArg + 1] : "all";

// ---- dev server -------------------------------------------------------------------

async function up() {
  try {
    const r = await fetch(BASE);
    return r.ok;
  } catch {
    return false;
  }
}

async function devServer() {
  if (await up()) return null;
  const child = spawn("npx", ["vite", "--port", String(PORT), "--strictPort"], {
    cwd: appDir,
    stdio: "ignore",
    shell: process.platform === "win32",
  });
  for (let i = 0; i < 120; i++) {
    if (await up()) return child;
    await new Promise((r) => setTimeout(r, 250));
  }
  child.kill();
  throw new Error("the dev server didn't start on port " + PORT);
}

// ---- the setup the mock is given ---------------------------------------------------

// The first-party plugins' real manifests, so the Plugins page and pickers look real.
const manifests = Object.fromEntries(
  readdirSync(join(root, "plugins"))
    .filter((d) => existsSync(join(root, "plugins", d, "perritodeck-plugin.json")))
    .map((d) => {
      const m = JSON.parse(readFileSync(join(root, "plugins", d, "perritodeck-plugin.json"), "utf8"));
      return [m.id, m];
    }),
);

const INSTALLED = ["dev.perritodeck.obs", "dev.perritodeck.proxmox", "dev.perritodeck.truenas", "dev.perritodeck.uptime", "dev.perritodeck.weather"];

/** Runs in the page before the app starts: wraps the mock's answers. */
function installMock({ manifests, installed }) {
  const now = Math.floor(Date.now() / 1000);
  const wait = setInterval(() => {
    const m = window.__perritoMock;
    if (!m) return;
    clearInterval(wait);
    const base = m.invoke.bind(m);
    const folder = "/home/me/.config/PerritoDeck/plugins/";
    let devices = [
      { id: "a1b2c3", name: "Fire HD 10", lastSeen: now, connected: true, profile: "streaming", assignedProfile: "streaming" },
      { id: "b2c3d4", name: "Galaxy Tab A8", lastSeen: now - 40, connected: true, profile: "homelab", assignedProfile: "homelab" },
      { id: "d4e5f6", name: "Pixel Tablet", lastSeen: now - 3 * 86400, connected: false, profile: "default", assignedProfile: null },
    ];
    const sdPlugins = [
      {
        uuid: "org.example.counter",
        name: "Counter",
        version: "1.2.0",
        author: "Example",
        description: "Counts presses.",
        folder: "/x",
        status: { state: "running" },
        icon: null,
        actions: [{ uuid: "org.example.counter.up", name: "Count up", tooltip: "Adds one", icon: null, hasSettingsPage: true }],
      },
      {
        uuid: "org.example.pomodoro",
        name: "Pomodoro timer",
        version: "2.0.1",
        author: "Example",
        description: "Focus timer on a key.",
        folder: "/y",
        status: { state: "running" },
        icon: null,
        actions: [{ uuid: "org.example.pomodoro.start", name: "Start a timer", tooltip: "", icon: null, hasSettingsPage: false }],
      },
    ];
    m.pluginSettings["dev.perritodeck.obs"] = { url: "ws://127.0.0.1:4455", password: "hunter2hunter2" };
    m.pluginSettings["dev.perritodeck.proxmox"] = { url: "https://pve.home.arpa:8006", token_id: "deck@pve!perrito", token_secret: "0000", insecure: true };
    m.checkUpdates = true;
    m.autostart = true;
    m.invoke = async (cmd, args) => {
      switch (cmd) {
        case "list_devices":
          await base("get_status");
          return structuredClone(devices);
        case "assign_profile":
          devices = devices.map((d) => (d.id === args.deviceId ? { ...d, assignedProfile: args.profileId, profile: args.profileId ?? "default" } : d));
          return undefined;
        case "list_plugins": {
          await base("get_status");
          const all = Object.values(manifests);
          return {
            installed: installed.map((id) => ({ manifest: manifests[id], status: { state: "running" }, folder: folder + id })),
            broken: [],
            folder: "/home/me/.config/PerritoDeck/plugins",
            catalog: all.map((mf) => ({ manifest: mf, installed: installed.includes(mf.id) })),
          };
        }
        case "list_sd_plugins":
          return { installed: sdPlugins, broken: [], folder: "/home/me/.config/PerritoDeck/sdplugins" };
      }
      return base(cmd, args);
    };
  }, 1);
}

const k = (title, extra = {}) => ({ title, bg: null, fg: null, action: null, nav: null, icon: null, slider: null, states: [], follow: null, ...extra });
const dark = "#11111B";
const obs = (action, settings = {}) => ({ type: "plugin", plugin: "dev.perritodeck.obs", action, settings });

/** Profiles built through the mock's own edit commands. */
const PROFILES = {
  streaming: {
    name: "Streaming",
    pages: ["Main", "SFX"],
    keys: [
      ["Main", [0, 0], k("Starting\nsoon", { bg: "#89B4FA", fg: dark, action: { type: "obs", op: "scene", scene: "Starting soon" } })],
      ["Main", [0, 1], k("Gameplay", { bg: "#89B4FA", fg: dark, action: { type: "obs", op: "scene", scene: "Gameplay" } })],
      ["Main", [0, 2], k("Just\nChatting", { bg: "#89B4FA", fg: dark, action: { type: "obs", op: "scene", scene: "Just Chatting" } })],
      ["Main", [0, 3], k("Be right\nback", { bg: "#89B4FA", fg: dark, action: { type: "obs", op: "scene", scene: "Be right back" } })],
      ["Main", [0, 4], k("Go live", { bg: "#F38BA8", fg: dark, action: { type: "obs", op: "toggleStream" } })],
      [
        "Main",
        [1, 0],
        k(null, {
          follow: "plugin:dev.perritodeck.obs:muted:Mic/Aux",
          states: [
            { title: "Mic\nlive", bg: "#A6E3A1", fg: dark, action: { type: "obs", op: "toggleMute", input: "Mic/Aux" } },
            { title: "Mic\nmuted", bg: "#F38BA8", fg: dark, action: { type: "obs", op: "toggleMute", input: "Mic/Aux" } },
          ],
        }),
      ],
      ["Main", [1, 1], k("Mic", { bg: "#94E2D5", fg: dark, slider: { type: "obs", input: "Mic/Aux" } })],
      ["Main", [1, 2], k("Save\nreplay", { bg: "#F9E2AF", fg: dark, action: { type: "obs", op: "saveReplayBuffer" } })],
      ["Main", [1, 3], k("Mark", { bg: "#FAB387", fg: dark, action: obs("record-chapter") })],
      [
        "Main",
        [1, 4],
        k(null, {
          follow: "obs:recording",
          states: [
            { title: "REC", action: { type: "obs", op: "startRecord" } },
            { title: "STOP", bg: "#F38BA8", fg: dark, action: { type: "obs", op: "stopRecord" } },
          ],
        }),
      ],
      ["Main", [2, 0], k("SFX ›", { bg: "#CBA6F7", fg: dark, nav: { page: "SFX" } })],
      ["Main", [2, 1], k("Desk\nlight", { bg: "#F9E2AF", fg: dark, action: { type: "homeAssistant", service: "light.toggle", entity: "light.desk" } })],
      ["Main", [2, 2], k("Camera", { action: obs("source-visibility", { item: "Gameplay/Camera" }) })],
      ["Main", [2, 3], k("On air")],
      ["Main", [2, 4], k("{time}", { fg: "#9399B2" })],
      ["SFX", [0, 0], k("‹ Back", { bg: "#CBA6F7", fg: dark, nav: "back" })],
    ],
  },
  homelab: {
    name: "Homelab",
    pages: ["Main"],
    keys: [
      ["Main", [0, 0], k("Proxmox", { bg: "#89B4FA", fg: dark, action: { type: "openUrl", url: "https://pve.home.arpa:8006" } })],
      ["Main", [0, 1], k("Plex VM", { bg: "#A6E3A1", fg: dark, action: { type: "plugin", plugin: "dev.perritodeck.proxmox", action: "toggle", settings: { vmid: "101" } } })],
      ["Main", [0, 2], k("NAS pool", { bg: "#A6E3A1", fg: dark })],
      ["Main", [0, 3], k("Wake\ndesktop", { bg: "#FAB387", fg: dark })],
    ],
  },
};

async function buildProfiles(page) {
  await page.evaluate(async (profiles) => {
    const m = window.__perritoMock;
    for (const [id, p] of Object.entries(profiles)) {
      if (id !== "streaming") await m.invoke("create_profile", { name: p.name, copyOf: null, companion: false });
      const edits = p.pages.slice(1).map((name) => ({ op: "addPage", name }));
      for (const [pg, at, key] of p.keys) edits.push({ op: "setKey", page: pg, at, key });
      await m.invoke("edit_profile", { id, edits });
    }
    await m.invoke("set_auto_switch", {
      auto: {
        enabled: true,
        rules: [
          { app: "obs64", profile: "streaming", device: null },
          { app: "Code", profile: "homelab", device: "b2c3d4" },
          { app: "firefox", profile: "default", device: null },
        ],
      },
    });
    m.emit("host-event", { kind: "profileChanged" });
  }, PROFILES);
}

// ---- the shots ---------------------------------------------------------------------

const nav = (page, name) => page.locator("nav.rail").getByRole("button", { name: new RegExp(`^${name}`) }).click();

async function openStreaming(page) {
  await page.getByLabel("Profile", { exact: true }).selectOption("streaming");
  await page.getByRole("button", { name: /^Gameplay/ }).waitFor();
}

/** Views with little on them get a shorter window, so the shots aren't mostly empty. */
const HEIGHT = { pair: 600, tablets: 560, switching: 600 };

const SHOTS = {
  async buttons(page) {
    await openStreaming(page);
    await page.getByRole("button", { name: /^Gameplay/ }).click();
    await page.getByLabel("Title").waitFor();
  },
  async "live-values"(page) {
    await openStreaming(page);
    await page.getByRole("button", { name: /^On air/ }).click();
    await page.getByRole("button", { name: "Insert a live value…" }).click();
    await page.getByLabel("Live value", { exact: true }).selectOption("plugin:dev.perritodeck.obs:scene");
  },
  async plugins(page) {
    await nav(page, "Plugins");
    await page.getByRole("heading", { name: "Plugins" }).waitFor();
    await page.getByRole("button", { name: "Settings for OBS Studio" }).click();
    await page.getByRole("button", { name: "Save settings" }).waitFor();
  },
  async "stream-deck-plugins"(page) {
    await nav(page, "Plugins");
    const sd = page.getByRole("heading", { name: "Stream Deck plugins" });
    await sd.waitFor();
    // Scroll so the heading and its explanation sit near the top, whichever element scrolls.
    await sd.evaluate((el) => {
      el.scrollIntoView({ block: "start" });
      for (let n = el.parentElement; n; n = n.parentElement) if (n.scrollTop > 0) n.scrollTop -= 40;
      if (window.scrollY > 0) window.scrollBy(0, -40);
    });
  },
  async pair(page) {
    await nav(page, "Pair a tablet");
    await page.getByRole("heading", { name: "Pair a tablet" }).waitFor();
    // A fixed code, so the shots don't change from run to run.
    await page.getByLabel(/^Pairing code/).evaluate((el) => {
      const digits = "482915";
      const cells = el.children.length ? [...el.children] : [];
      if (cells.length === 6) cells.forEach((c, i) => (c.textContent = digits[i]));
      else el.textContent = digits.split("").join(" ");
    });
  },
  async tablets(page) {
    await nav(page, "Tablets");
    await page.getByText("Galaxy Tab A8").first().waitFor();
  },
  async switching(page) {
    await nav(page, "Switching");
    await page.getByRole("heading", { name: "Switching" }).waitFor();
  },
  async settings(page) {
    await nav(page, "Settings");
    await page.getByRole("heading", { name: "Settings" }).waitFor();
  },
};

async function desktopShots(browser, saved) {
  const server = await devServer();
  try {
    for (const scheme of ["light", "dark"]) {
      const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 2, colorScheme: scheme, reducedMotion: "reduce", locale: "en-US" });
      for (const [name, shoot] of Object.entries(SHOTS)) {
        if (only && !only.has(name)) continue;
        const page = await context.newPage();
        const viewport = { ...VIEWPORT, height: HEIGHT[name] ?? VIEWPORT.height };
        await page.setViewportSize(viewport);
        await page.clock.setFixedTime(new Date("2026-10-11T20:07:00"));
        await page.addInitScript(installMock, { manifests, installed: INSTALLED });
        await page.goto(BASE);
        await page.getByRole("heading", { name: "Buttons" }).waitFor();
        await buildProfiles(page);
        await shoot(page);
        await page.evaluate(() => document.fonts.ready);
        await page.mouse.move(0, viewport.height - 1);
        await page.waitForTimeout(250);
        const file = join(outDir, `screenshot-${name}${scheme === "dark" ? "-dark" : ""}.png`);
        await page.screenshot({ path: file });
        saved.push(file);
        await page.close();
      }
      await context.close();
    }
  } finally {
    server?.kill();
  }
}

// =====================================================================================
// Store and website images: what Perrito shows, drawn the way the host draws keys.
//
// Keys follow host/crates/render (square image, 8% margin, Inter SemiBold title centred
// and shrunk until it fits, icon in the top 55%) and the Android client's layout
// (8dp padding and gaps, square cells, corners 14% of the key, sliders filled from the
// bottom over a 35% tint). Live values ({plugin:…}) appear as the text they'd show.
// =====================================================================================

const fontData = (p) => readFileSync(join(root, p)).toString("base64");
const FONTS = {
  inter: fontData("host/crates/render/assets/Inter-SemiBold.otf"),
  bricolage: fontData("host/app/node_modules/@fontsource-variable/bricolage-grotesque/files/bricolage-grotesque-latin-wght-normal.woff2"),
};
const TABLER = JSON.parse(readFileSync(join(appDir, "node_modules/@tabler/icons/tabler-nodes-outline.json"), "utf8"));
const iconBody = (name) => {
  const nodes = TABLER[name];
  if (!nodes) throw new Error(`no Tabler icon ${name}`);
  return nodes.map(([tag, attrs]) => `<${tag} ${Object.entries(attrs).map(([a, v]) => `${a}="${v}"`).join(" ")}/>`).join("");
};

// Catppuccin-like key tones, as in starter.toml.
const C = { red: "#F38BA8", apricot: "#FAB387", yellow: "#F9E2AF", green: "#A6E3A1", teal: "#94E2D5", blue: "#89B4FA", mauve: "#CBA6F7", ink: "#11111B", muted: "#9399B2", up: "#A6E3A1" };

/** A key: [row, col, title, bg, fg, icon]; colours default to the host's. */
const K = (row, col, title, bg = null, icon = null, fg = null) => ({ row, col, title, bg: bg ?? "#1E1E2E", fg: fg ?? (bg ? C.ink : "#CDD6F4"), icon });
/** A slider showing `value`. */
const S = (row, col, title, bg, value, fg = "#CDD6F4") => ({ row, col, title, bg, fg, slider: value });

const DECKS = {
  streaming: {
    tablet: [
      K(0, 0, "Starting\nsoon", C.blue), K(0, 1, "Gameplay", C.apricot), K(0, 2, "Just\nChatting", C.blue),
      K(0, 3, "Be right\nback", C.blue), K(0, 4, "Ending", C.blue), K(0, 5, "01:24:07", C.red, "broadcast"),
      K(1, 0, "Mic live", C.green, "microphone"), S(1, 1, "Mic", C.teal, 44), S(1, 2, "Desktop", C.blue, 36),
      S(1, 3, "Music", C.mauve, 22), K(1, 4, "Camera", C.green, "video"), K(1, 5, "00:41:12", C.red, "player-record"),
      K(2, 0, "Clip it", C.yellow, "cut"), K(2, 1, "Chapter", null, "flag"), K(2, 2, "60 fps\n0% drop"),
      K(2, 3, "6,012\nkbps"), K(2, 4, "Key light", C.yellow, "bulb"), K(2, 5, "20:07", null, null, C.muted),
      K(3, 0, "SFX ›", C.mauve), K(3, 1, "Chat", null, "message-circle"), K(3, 2, "Deaths\n12"),
      K(3, 3, "Break", null, "coffee"), K(3, 4, "Hide chat", null, "eye-off"), K(3, 5, "More ›", C.mauve),
    ],
    phone: [
      K(0, 0, "Gameplay", C.apricot), K(0, 1, "Just\nChatting", C.blue), K(0, 2, "Be right\nback", C.blue),
      K(1, 0, "Mic live", C.green, "microphone"), S(1, 1, "Mic", C.teal, 44), K(1, 2, "01:24:07", C.red, "broadcast"),
      K(2, 0, "Camera", C.green, "video"), K(2, 1, "Clip it", C.yellow, "cut"), K(2, 2, "00:41:12", C.red, "player-record"),
      K(3, 0, "60 fps\n0% drop"), K(3, 1, "Key light", C.yellow, "bulb"), K(3, 2, "Chapter", null, "flag"),
      K(4, 0, "SFX ›", C.mauve), K(4, 1, "Deaths\n12"), K(4, 2, "20:07", null, null, C.muted),
    ],
  },
  homelab: {
    tablet: [
      K(0, 0, "pve-01", C.green, "server"), K(0, 1, "CPU\n23%"), K(0, 2, "RAM\n61%"),
      K(0, 3, "Guests\n14 up"), K(0, 4, "tank", C.green, "database"), K(0, 5, "Pool\n68% used"),
      K(1, 0, "Plex", C.green, "power"), K(1, 1, "Home Asst", C.green, "power"), K(1, 2, "Win 11", null, "power", C.muted),
      K(1, 3, "k3s", C.green, "power"), K(1, 4, "Scrub", C.yellow, "refresh"), K(1, 5, "1 alert", C.yellow, "alert-triangle"),
      K(2, 0, "Nextcloud\n142 ms", null, null, C.up), K(2, 1, "Jellyfin\n38 ms", null, null, C.up), K(2, 2, "Gitea\n61 ms", null, null, C.up),
      K(2, 3, "Backups\nDOWN", C.red), K(2, 4, "Router\n2 ms", null, null, C.up), K(2, 5, "NAS\n47 °C"),
      K(3, 0, "Wake PC", C.apricot, "bolt"), K(3, 1, "SSH", null, "terminal"), K(3, 2, "Deploy", C.blue, "rocket"),
      K(3, 3, "Ollama", C.mauve, "robot"), K(3, 4, "12°\nRain"), K(3, 5, "20:07", null, null, C.muted),
    ],
    phone: [
      K(0, 0, "pve-01", C.green, "server"), K(0, 1, "CPU\n23%"), K(0, 2, "RAM\n61%"),
      K(1, 0, "Plex", C.green, "power"), K(1, 1, "Win 11", null, "power", C.muted), K(1, 2, "tank", C.green, "database"),
      K(2, 0, "Nextcloud\n142 ms", null, null, C.up), K(2, 1, "Backups\nDOWN", C.red), K(2, 2, "Router\n2 ms", null, null, C.up),
      K(3, 0, "1 alert", C.yellow, "alert-triangle"), K(3, 1, "Wake PC", C.apricot, "bolt"), K(3, 2, "Deploy", C.blue, "rocket"),
      K(4, 0, "Pool\n68% used"), K(4, 1, "Ollama", C.mauve, "robot"), K(4, 2, "20:07", null, null, C.muted),
    ],
  },
  desk: {
    tablet: [
      K(0, 0, "Prev", null, "player-skip-back"), K(0, 1, "Pause", C.green, "player-pause"), K(0, 2, "Next", null, "player-skip-forward"),
      S(0, 3, "Volume", C.blue, 42), K(0, 4, "Lumen Drift\nNight Bus"), K(0, 5, "Sun 11 Oct\n20:07", null, null, C.muted),
      K(1, 0, "Desk", C.yellow, "bulb"), K(1, 1, "Lamp", C.yellow, "lamp"), S(1, 2, "Desk light", C.yellow, 34),
      K(1, 3, "Cosy", C.mauve, "home"), K(1, 4, "21.5 °C", null, "temperature"), K(1, 5, "12°\nRain"),
      K(2, 0, "Muted", C.red, "microphone-off"), K(2, 1, "Cam off", null, "video-off"), K(2, 2, "Hand", null, "hand-stop"),
      K(2, 3, "Share", C.blue, "screen-share"), K(2, 4, "Leave", C.red, "phone"), K(2, 5, "CPU\n18%"),
      K(3, 0, "Copy", null, "copy"), K(3, 1, "Paste", null, "clipboard"), K(3, 2, "Terminal", null, "terminal-2"),
      K(3, 3, "Browser", null, "browser"), K(3, 4, "Mail", null, "mail"), K(3, 5, "Lock", C.apricot, "lock"),
    ],
  },
};

/** Screens Perrito runs on: pixels, density, grid. */
const SCREENS = {
  tablet: { width: 1920, height: 1200, dpi: 224, cols: 6, rows: 4 },
  phone: { width: 1080, height: 1920, dpi: 420, cols: 3, rows: 5 },
};

const fontFaces = `
@font-face { font-family: "Inter"; font-weight: 600; src: url(data:font/otf;base64,${FONTS.inter}) format("opentype"); }
@font-face { font-family: "Bricolage"; font-weight: 200 800; src: url(data:font/woff2;base64,${FONTS.bricolage}) format("woff2"); }`;

/** Runs in the page: draws every `canvas[data-key]` the way the host renders a key. */
function drawKeys(icons) {
  const ASC = 0.96875, DESC = 0.2421875; // Inter's hhea metrics, in em
  const lineHeight = (px) => (ASC + DESC) * px * 1.05;
  function wrap(ctx, text, maxW) {
    const out = [];
    for (const para of text.split("\n")) {
      let line = "";
      for (const word of para.split(/\s+/).filter(Boolean)) {
        const cand = line ? `${line} ${word}` : word;
        if (ctx.measureText(cand).width <= maxW || !line) line = cand;
        else {
          out.push(line);
          line = word;
        }
      }
      out.push(line);
    }
    return out;
  }
  function title(ctx, text, color, x0, y0, w, h) {
    let px = Math.max(w * 0.24, 8);
    let lines;
    for (;;) {
      ctx.font = `600 ${px}px Inter`;
      lines = wrap(ctx, text, w);
      const height = lines.length * lineHeight(px);
      const widest = Math.max(...lines.map((l) => ctx.measureText(l).width));
      if ((height <= h && widest <= w) || px <= 8) break;
      px *= 0.92;
    }
    const lh = lineHeight(px);
    let base = y0 + Math.max(0, h - lines.length * lh) / 2 + ASC * px;
    ctx.fillStyle = color;
    ctx.textBaseline = "alphabetic";
    for (const l of lines) {
      ctx.fillText(l, x0 + Math.max(0, w - ctx.measureText(l).width) / 2, base);
      base += lh;
    }
  }
  const load = (src) =>
    new Promise((ok, fail) => {
      const img = new Image();
      img.onload = () => ok(img);
      img.onerror = fail;
      img.src = src;
    });
  return Promise.all(
    [...document.querySelectorAll("canvas[data-key]")].map(async (cv) => {
      const k = JSON.parse(cv.dataset.key);
      const size = cv.width;
      const ctx = cv.getContext("2d");
      ctx.fillStyle = k.bg;
      ctx.fillRect(0, 0, size, size);
      const margin = size * 0.08;
      const inner = size - 2 * margin;
      const hasTitle = !!k.title.trim();
      let top = margin;
      if (k.icon) {
        const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="192" height="192" viewBox="0 0 24 24" fill="none" stroke="${k.fg}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${icons[k.icon]}</svg>`;
        const img = await load("data:image/svg+xml;charset=utf-8," + encodeURIComponent(svg));
        const boxH = hasTitle ? inner * 0.55 : inner;
        const s = Math.min(inner, boxH);
        const y = hasTitle ? margin : (size - s) / 2;
        ctx.drawImage(img, (size - s) / 2, y, s, s);
        top = y + s + margin * 0.5;
      }
      if (hasTitle) title(ctx, k.title, k.fg, margin, top, inner, size - margin - top);
    }),
  );
}

function deckHtml(screen, keys, { host = "studio-desktop", rtt = "3.8", status = true } = {}) {
  const d = screen.dpi / 160;
  const pad = 8 * d;
  const gap = 8 * d;
  const cell = Math.floor(Math.min((screen.width - 2 * pad - gap * (screen.cols - 1)) / screen.cols, (screen.height - 2 * pad - gap * (screen.rows - 1)) / screen.rows));
  const byPos = new Map(keys.map((k) => [`${k.row},${k.col}`, k]));
  let cells = "";
  for (let r = 0; r < screen.rows; r++)
    for (let c = 0; c < screen.cols; c++) {
      const k = byPos.get(`${r},${c}`);
      if (!k) cells += `<div class="key empty"></div>`;
      else if (k.slider != null)
        cells += `<div class="key slider" style="--bg:${k.bg};--fg:${k.fg};--v:${k.slider}%"><div class="fill"></div><div class="lbl"><span>${k.title}</span><b>${k.slider}</b></div></div>`;
      else cells += `<canvas class="key" width="${cell}" height="${cell}" data-key='${JSON.stringify(k).replace(/'/g, "&#39;")}'></canvas>`;
    }
  const fs = cell * 0.14; // the slider's text, as in DeckSlider
  return `<!doctype html><meta charset="utf-8"><style>${fontFaces}
  html, body { margin: 0; width: ${screen.width}px; height: ${screen.height}px; background: #11111B; overflow: hidden; }
  .grid { position: absolute; inset: ${pad}px; display: grid; place-content: center; grid-template-columns: repeat(${screen.cols}, ${cell}px); grid-auto-rows: ${cell}px; gap: ${gap}px; }
  .key { width: ${cell}px; height: ${cell}px; border-radius: ${cell * 0.14}px; display: block; }
  .empty { background: #1E1E2E; }
  .slider { position: relative; overflow: hidden; background: color-mix(in srgb, var(--bg) 35%, #11111B); color: var(--fg); font-family: Inter; font-weight: 600; }
  .slider .fill { position: absolute; left: 0; right: 0; bottom: 0; height: var(--v); background: var(--bg); }
  .slider .lbl { position: absolute; inset: ${cell * 0.08}px 0 auto; display: flex; flex-direction: column; align-items: center; line-height: 1.25; }
  .slider span { font-size: ${fs}px; }
  .slider b { font-size: ${fs * 1.4}px; font-weight: 600; }
  .status { position: absolute; top: ${2 * d}px; right: ${8 * d}px; font: 600 ${11 * d}px/${16 * d}px Inter; color: #9399B2; letter-spacing: 0.02em; }
  </style><div class="grid">${cells}</div>${status ? `<div class="status">${host} · ${rtt} ms</div>` : ""}`;
}

const dataUrl = (file) => "data:image/png;base64," + readFileSync(file).toString("base64");

/** A plain, brandless device around a screen image, on a transparent background. */
function frameHtml(kind, screenFile, scale) {
  const s = SCREENS[kind];
  const w = s.width * scale;
  const h = s.height * scale;
  const bezel = kind === "tablet" ? 44 : 30;
  const radius = kind === "tablet" ? 70 : 96;
  const m = 90; // room for the shadow
  return `<!doctype html><style>
  html, body { margin: 0; background: transparent; }
  .dev { position: absolute; left: ${m}px; top: ${m * 0.6}px; padding: ${bezel}px; border-radius: ${radius}px;
    background: linear-gradient(160deg, #3A4256, #232838 40%, #1A1E2A);
    box-shadow: inset 0 0 0 2px #4A5368, inset 0 0 0 7px #161A24, 0 50px 90px -40px #11111B99, 0 20px 40px -24px #11111B80; }
  .dev img { display: block; width: ${w}px; height: ${h}px; border-radius: ${kind === "tablet" ? 16 : 40}px; }
  .cam { position: absolute; width: 12px; height: 12px; border-radius: 50%; background: #0B0D13; box-shadow: inset 0 0 0 2px #2B3142;
    ${kind === "tablet" ? `top: ${bezel / 2 - 6}px; left: calc(50% - 6px);` : `top: ${bezel + 26}px; left: calc(50% - 6px); background: #05060A;`} }
  </style><div class="dev"><img src="${dataUrl(screenFile)}"><div class="cam"></div></div>`;
}
const frameSize = (kind, scale) => {
  const s = SCREENS[kind];
  const bezel = kind === "tablet" ? 44 : 30;
  return { width: Math.round(s.width * scale + 2 * bezel + 180), height: Math.round(s.height * scale + 2 * bezel + 90 * 0.6 + 120) };
};

/** Tablet with a phone in front of it, for the website. */
function heroHtml(tabletFile, phoneFile) {
  return `<!doctype html><style>
  html, body { margin: 0; background: transparent; }
  .dev { position: absolute; border-radius: 60px; padding: 36px;
    background: linear-gradient(160deg, #3A4256, #232838 40%, #1A1E2A);
    box-shadow: inset 0 0 0 2px #4A5368, inset 0 0 0 6px #161A24, 0 50px 90px -40px #11111BAA, 0 20px 40px -24px #11111B80; }
  .dev img { display: block; }
  .tab { left: 40px; top: 30px; }
  .tab img { width: 1440px; height: 900px; border-radius: 14px; }
  .ph { left: 1250px; top: 330px; border-radius: 74px; padding: 22px; }
  .ph img { width: 405px; height: 720px; border-radius: 34px; }
  </style><div class="dev tab"><img src="${dataUrl(tabletFile)}"></div><div class="dev ph"><img src="${dataUrl(phoneFile)}"></div>`;
}

function featureHtml() {
  const icon = readFileSync(join(outDir, "icon-master-rounded.svg"), "utf8");
  const keys = [
    ["Go live", C.red], ["Scene 2", C.blue], ["", C.apricot, "paw"], ["Mic", C.green], ["Clip", C.yellow],
    ["20:07", "#1E1E2E", null, "#CDD6F4"], ["Lights", C.mauve], ["REC", C.red], ["Wake PC", C.apricot], ["Lamp", C.yellow],
    ["Scene 1", C.blue], ["Uptime", C.teal], ["Pause", C.green], ["Next", "#1E1E2E", null, "#CDD6F4"], ["Mute", C.blue],
  ];
  // The paw in ink on an apricot key: the brand mark as one of the keys.
  const paw = `<svg viewBox="190 240 644 540">${icon.match(/<g transform[^>]*>([\s\S]*?)<\/g>/)[1].replace(/fill="#FAB387"/g, 'fill="#11111B"').replace(/fill="#D9875B"/g, 'fill="#11111B"')}</svg>`;
  return `<!doctype html><style>${fontFaces}
  html, body { margin: 0; width: 1024px; height: 500px; overflow: hidden; }
  body { background: radial-gradient(90% 130% at 88% 40%, #2E3548 0%, #1E1E2E 50%, #161620 100%); position: relative; }
  .brand { position: absolute; left: 64px; top: 0; bottom: 0; display: flex; flex-direction: column; justify-content: center; gap: 26px; }
  .brand svg { display: block; filter: drop-shadow(0 12px 20px #0008); }
  .brand h1 { margin: 0; font: 800 70px/0.95 Bricolage; letter-spacing: -0.03em; color: #EEF1F6; }
  .brand h1 span { color: #FAB387; }
  .deck { position: absolute; left: 548px; top: 50%; display: grid; grid-template-columns: repeat(5, 104px); gap: 14px;
    transform: translateY(-50%) perspective(1200px) rotateY(22deg) rotateX(8deg) rotateZ(-3deg); transform-origin: 0 50%; padding: 20px; border-radius: 30px;
    background: #2A3142; box-shadow: inset 0 0 0 2px #39415580, 0 40px 60px -30px #000c; }
  .k { height: 104px; border-radius: 16px; display: grid; place-items: center; font: 600 18px/1.15 Inter; text-align: center; box-shadow: inset 0 -6px 0 #0000002e; }
  .k svg { width: 64%; height: 64%; }
  </style>
  <div class="brand">${icon.replace(/<svg[^>]*>/, '<svg xmlns="http://www.w3.org/2000/svg" viewBox="72 72 880 880" width="120" height="120">').replace('fill="#1E1E2E"', 'fill="#2A3142"')}<h1>Perrito<span>Deck</span></h1></div>
  <div class="deck">${keys
    .map(([t, bg, special, fg]) => `<div class="k" style="background:${bg};color:${fg ?? C.ink}">${special === "paw" ? paw : t}</div>`)
    .join("")}</div>`;
}

async function storeImages(browser, saved) {
  const meta = join(root, "fastlane/metadata/android/en-US/images");
  const web = join(root, "website/img");
  for (const d of [join(meta, "phoneScreenshots"), join(meta, "tenInchScreenshots"), web]) mkdirSync(d, { recursive: true });
  const shoot = async (html, size, file, { transparent = false, draw = false } = {}) => {
    const page = await browser.newPage({ viewport: size, deviceScaleFactor: 1 });
    await page.setContent(html);
    await page.evaluate(() => document.fonts.ready);
    await page.evaluate(() => Promise.all([...document.fonts].map((f) => f.load())));
    if (draw) await page.evaluate(drawKeys, ICONS_FOR_PAGE);
    await page.screenshot({ path: file, omitBackground: transparent });
    await page.close();
    saved.push(file);
    return file;
  };
  const ICONS_FOR_PAGE = Object.fromEntries(
    [...new Set(Object.values(DECKS).flatMap((d) => Object.values(d).flat().map((k) => k.icon).filter(Boolean)))].map((n) => [n, iconBody(n)]),
  );

  // Raw screens: Google Play/F-Droid screenshots (24-bit PNG, no alpha) and the website.
  const order = { tablet: ["streaming", "homelab", "desk"], phone: ["streaming", "homelab"] };
  const raw = {};
  for (const [kind, names] of Object.entries(order)) {
    const s = SCREENS[kind];
    for (const [i, name] of names.entries()) {
      const file = join(meta, kind === "tablet" ? "tenInchScreenshots" : "phoneScreenshots", `${i + 1}.png`);
      // The latency line would sit on the top-right key of a full-height tablet grid; leave it out there.
      await shoot(deckHtml(s, DECKS[name][kind], { status: kind === "phone" }), { width: s.width, height: s.height }, file, { draw: true });
      raw[`${kind}-${name}`] = file;
      copyFileSync(file, join(web, `deck-${kind}-${name}.png`));
      saved.push(join(web, `deck-${kind}-${name}.png`));
    }
  }

  // Device mockups for the website.
  for (const name of order.tablet) {
    const scale = 0.75;
    await shoot(frameHtml("tablet", raw[`tablet-${name}`], scale), frameSize("tablet", scale), join(web, `tablet-${name}.png`), { transparent: true });
  }
  for (const name of order.phone) {
    const scale = 0.5;
    await shoot(frameHtml("phone", raw[`phone-${name}`], scale), frameSize("phone", scale), join(web, `phone-${name}.png`), { transparent: true });
  }
  await shoot(heroHtml(raw["tablet-streaming"], raw["phone-homelab"]), { width: 1740, height: 1140 }, join(web, "devices-hero.png"), { transparent: true });

  // Google Play: feature graphic (no alpha) and the 512 px hi-res icon (full square; Play rounds it).
  await shoot(featureHtml(), { width: 1024, height: 500 }, join(meta, "featureGraphic.png"));
  const master = readFileSync(join(outDir, "icon-master.svg"), "utf8");
  const rounded = readFileSync(join(outDir, "icon-master-rounded.svg"), "utf8");
  const svgPage = (svg, px) => `<!doctype html><style>html,body{margin:0;background:transparent}svg{display:block;width:${px}px;height:${px}px}</style>${svg}`;
  // Play wants a 32-bit PNG (with an alpha channel), even though this one is fully opaque.
  // Chromium drops the alpha channel of a fully opaque image, so nick the top-left corner
  // pixel (Play's own rounded mask cuts it off anyway).
  const nicked = svgPage(master, 512).replace("</style>", "svg{clip-path:polygon(1px 0,100% 0,100% 100%,0 100%,0 1px)}</style>");
  await shoot(nicked, { width: 512, height: 512 }, join(meta, "icon.png"), { transparent: true });
  await shoot(svgPage(master, 1024), { width: 1024, height: 1024 }, join(outDir, "icon-1024.png"));
  await shoot(svgPage(rounded, 1024), { width: 1024, height: 1024 }, join(outDir, "icon-rounded-1024.png"), { transparent: true });
}

async function main() {
  mkdirSync(outDir, { recursive: true });
  const executablePath = process.env.PW_CHROMIUM ?? (existsSync("/opt/pw-browsers/chromium") ? "/opt/pw-browsers/chromium" : undefined);
  const browser = await chromium.launch({ executablePath });
  const saved = [];
  try {
    if (set !== "store") await desktopShots(browser, saved);
    if (set !== "desktop") await storeImages(browser, saved);
  } finally {
    await browser.close();
  }
  for (const f of [...new Set(saved)]) console.log("saved", f.slice(root.length + 1));
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
