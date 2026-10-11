// Stand-in for the Rust host when the UI runs in a plain browser. Playwright drives it
// through `window.__perritoMock`.

import { DEFAULT_BG, DEFAULT_FG, kindOf } from "./describe";
import type { DeviceInfo, Edit, HostEvent, HostStatus, KeyFields, KeyView, PairingOffer, ProfileView } from "./types";

type Listener = (payload: unknown) => void;

const status: HostStatus = {
  hostName: "studio-desktop",
  version: "0.1.0",
  fingerprint: "3FA9 07C2 D41E",
  addresses: ["192.168.1.20:46710"],
  insecure: false,
  dataDir: "/home/me/.config/PerritoDeck",
  profilePath: "/home/me/.config/PerritoDeck/profiles/default.toml",
  integrations: ["OBS"],
};

let devices: DeviceInfo[] = [
  { id: "a1b2c3", name: "Fire HD 10", lastSeen: Math.floor(Date.now() / 1000), connected: true },
  { id: "d4e5f6", name: "Pixel Tablet", lastSeen: Math.floor(Date.now() / 1000) - 3 * 86400, connected: false },
];

function key(row: number, col: number, f: Partial<KeyFields>, states = 0, shown?: Partial<KeyView>): KeyView {
  const fields: KeyFields = { title: null, bg: null, fg: null, action: null, nav: null, ...f };
  return {
    row,
    col,
    title: fields.title ?? "",
    bg: fields.bg ?? DEFAULT_BG,
    fg: fields.fg ?? DEFAULT_FG,
    ...kindOf(fields, states),
    fields,
    states,
    hasIcon: false,
    ...shown,
  };
}

const dark = "#11111B";

let profile: ProfileView = {
  name: "Starter",
  cols: 5,
  rows: 3,
  fixedGrid: true,
  reloadError: null,
  canUndo: false,
  canRedo: false,
  pages: [
    {
      name: "Main",
      keys: [
        key(0, 0, { title: "{time}" }),
        key(0, 1, { title: "Play /\nPause", bg: "#A6E3A1", fg: dark, action: { type: "media", key: "playPause" } }),
        key(0, 2, { title: "Vol +", bg: "#89B4FA", fg: dark, action: { type: "media", key: "volumeUp" } }),
        key(0, 3, { title: "Vol −", bg: "#89B4FA", fg: dark, action: { type: "media", key: "volumeDown" } }),
        key(0, 4, { title: "Mute", bg: "#89B4FA", fg: dark, action: { type: "media", key: "mute" } }),
        key(1, 0, {}, 2, { title: "Mic\nlive", bg: "#A6E3A1", fg: dark, detail: "2 states, hotkey Ctrl+Shift+M" }),
        key(1, 1, { title: "Copy", action: { type: "hotkey", keys: "Ctrl+C" } }),
        key(1, 2, { title: "Paste", action: { type: "hotkey", keys: "Ctrl+V" } }),
        key(1, 3, { title: "Type\nhello", action: { type: "text", text: "Hello from Perrito! " } }),
        key(1, 4, { title: "Perrito\nwebsite", bg: "#FAB387", fg: dark, action: { type: "openUrl", url: "https://perritodeck.dev" } }),
        key(2, 0, { title: "More ›", bg: "#CBA6F7", fg: dark, nav: { page: "More" } }),
        key(2, 4, { title: "{date}", fg: "#9399B2" }),
      ],
    },
    {
      name: "More",
      keys: [
        key(0, 0, { title: "‹ Back", bg: "#CBA6F7", fg: dark, nav: "back" }),
        key(0, 1, { title: "Run a\ncommand", action: { type: "command", program: "hostname" } }),
        key(0, 2, {
          title: "Select all\n+ copy",
          action: {
            type: "multi",
            steps: [{ action: { type: "hotkey", keys: "Ctrl+A" }, delayMs: 100 }, { action: { type: "hotkey", keys: "Ctrl+C" } }],
          },
        }),
      ],
    },
  ],
};

const undoStack: ProfileView[] = [];
const redoStack: ProfileView[] = [];

const listeners = new Map<string, Set<Listener>>();
let failStart: string | null = null;

function qr(): string {
  // A stand-in pattern, not a scannable code.
  const cells: string[] = [];
  let seed = 7;
  for (let y = 0; y < 25; y++)
    for (let x = 0; x < 25; x++) {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      const finder = (x < 7 && y < 7) || (x > 17 && y < 7) || (x < 7 && y > 17);
      const on = finder ? x % 6 === 0 || y % 6 === 0 || (x % 6 > 1 && x % 6 < 5 && y % 6 > 1 && y % 6 < 5) : seed % 3 === 0;
      if (on) cells.push(`<rect x="${x + 2}" y="${y + 2}" width="1" height="1"/>`);
    }
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 29 29"><rect width="29" height="29" fill="#fff"/><g fill="#000">${cells.join("")}</g></svg>`;
}

function page(p: ProfileView, name: string) {
  const pg = p.pages.find((x) => x.name === name);
  if (!pg) throw `there's no page called “${name}”`;
  return pg;
}

const same = (k: KeyView, at: [number, number]) => k.row === at[0] && k.col === at[1];

/** The same rules as the Rust side, closely enough for UI work and tests. */
function applyEdit(p: ProfileView, e: Edit) {
  switch (e.op) {
    case "setKey": {
      if (e.key.bg && !/^#[0-9a-fA-F]{6}$/.test(e.key.bg)) throw "that change would break the profile:\n  - colours look like #1E1E2E";
      const pg = page(p, e.page);
      const old = pg.keys.find((k) => same(k, e.at));
      const next = key(e.at[0], e.at[1], e.key, old?.states ?? 0);
      pg.keys = old ? pg.keys.map((k) => (k === old ? next : k)) : [...pg.keys, next];
      return;
    }
    case "clearKey": {
      const pg = page(p, e.page);
      if (!pg.keys.some((k) => same(k, e.at))) throw "there's no key there";
      pg.keys = pg.keys.filter((k) => !same(k, e.at));
      return;
    }
    case "moveKey": {
      const pg = page(p, e.page);
      const a = pg.keys.find((k) => same(k, e.from));
      if (!a) throw "there's no key there";
      const b = pg.keys.find((k) => same(k, e.to));
      if (b) [b.row, b.col] = e.from;
      [a.row, a.col] = e.to;
      return;
    }
    case "moveKeyToPage": {
      if (e.fromPage === e.toPage) return applyEdit(p, { op: "moveKey", page: e.fromPage, from: e.at, to: e.to });
      const src = page(p, e.fromPage);
      const dst = page(p, e.toPage);
      const a = src.keys.find((k) => same(k, e.at));
      if (!a) throw "there's no key there";
      const b = dst.keys.find((k) => same(k, e.to));
      src.keys = src.keys.filter((k) => k !== a);
      dst.keys = dst.keys.filter((k) => k !== b);
      [a.row, a.col] = e.to;
      dst.keys.push(a);
      if (b) {
        [b.row, b.col] = e.at;
        src.keys.push(b);
      }
      return;
    }
    case "addPage":
      if (p.pages.some((x) => x.name === e.name)) throw `there's already a page called “${e.name}”`;
      p.pages.push({ name: e.name, keys: [] });
      return;
    case "renamePage": {
      if (e.from !== e.to && p.pages.some((x) => x.name === e.to)) throw `there's already a page called “${e.to}”`;
      page(p, e.from).name = e.to;
      for (const pg of p.pages)
        pg.keys = pg.keys.map((k) =>
          k.fields.nav && typeof k.fields.nav === "object" && k.fields.nav.page === e.from
            ? key(k.row, k.col, { ...k.fields, nav: { page: e.to } }, k.states)
            : k,
        );
      return;
    }
    case "removePage": {
      page(p, e.name);
      const opener = p.pages.some((pg) => pg.keys.some((k) => k.kind === "page" && k.detail === `opens ${e.name}`));
      if (opener) throw `that change would break the profile:\n  - a key opens “${e.name}”, which doesn't exist`;
      if (p.pages.length === 1) throw "that change would break the profile:\n  - a profile needs at least one page";
      p.pages = p.pages.filter((x) => x.name !== e.name);
      return;
    }
    case "movePage": {
      const pg = page(p, e.name);
      p.pages = p.pages.filter((x) => x !== pg);
      p.pages.splice(Math.min(e.index, p.pages.length), 0, pg);
      return;
    }
  }
}

function view(): ProfileView {
  return structuredClone({ ...profile, canUndo: undoStack.length > 0, canRedo: redoStack.length > 0 });
}

export const mock = {
  /** Every edit batch the UI sent, for tests. */
  edits: [] as Edit[][],
  /** Actions run with "Try it". */
  tested: [] as unknown[],

  async invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    await new Promise((r) => setTimeout(r, 20));
    if (failStart) throw failStart;
    const out = (v: unknown) => structuredClone(v) as T;
    switch (cmd) {
      case "get_status":
        return out(status);
      case "list_devices":
        return out(devices);
      case "revoke_device": {
        const before = devices.length;
        devices = devices.filter((d) => d.id !== args?.deviceId);
        return out(devices.length < before);
      }
      case "start_pairing": {
        const code = String(100000 + Math.floor(Math.random() * 900000));
        const offer: PairingOffer = {
          code,
          uri: `perritodeck://pair?v=1&n=studio-desktop&a=192.168.1.20:46710&fp=x&c=${code}`,
          fingerprint: status.fingerprint!,
          expiresInSecs: 300,
          qrSvg: qr(),
        };
        return out(offer);
      }
      case "get_profile":
        return out(view());
      case "edit_profile": {
        const edits = args?.edits as Edit[];
        const draft = structuredClone(profile);
        for (const e of edits) applyEdit(draft, e);
        this.edits.push(edits);
        undoStack.push(profile);
        redoStack.length = 0;
        profile = draft;
        return out(view());
      }
      case "undo_edit":
      case "redo_edit": {
        const [from, to] = cmd === "undo_edit" ? [undoStack, redoStack] : [redoStack, undoStack];
        const prev = from.pop();
        if (!prev) throw `Nothing to ${cmd === "undo_edit" ? "undo" : "redo"}.`;
        to.push(profile);
        profile = prev;
        return out(view());
      }
      case "test_action": {
        const a = args?.action as { type: string; program?: string };
        this.tested.push(args?.action);
        if (a.type === "command" && a.program === "false") return out("exited with status 1");
        return out(null);
      }
      case "open_profile_file":
      case "open_data_folder":
        return out(undefined);
    }
    throw `unknown command ${cmd}`;
  },

  listen(event: string, cb: Listener): () => void {
    if (!listeners.has(event)) listeners.set(event, new Set());
    listeners.get(event)!.add(cb);
    return () => listeners.get(event)?.delete(cb);
  },

  emit(event: string, payload: unknown) {
    listeners.get(event)?.forEach((cb) => cb(payload));
  },

  /** Test hooks. */
  hostEvent(e: HostEvent) {
    if (e.kind === "paired") {
      devices.push({ id: e.deviceId, name: e.deviceName, lastSeen: null, connected: false });
    }
    if (e.kind === "connected" || e.kind === "disconnected") {
      devices = devices.map((d) => (d.id === e.deviceId ? { ...d, connected: e.kind === "connected" } : d));
    }
    this.emit("host-event", e);
  },
  setReloadError(msg: string | null) {
    profile.reloadError = msg;
    this.emit("host-event", { kind: "profileChanged" });
  },
  failToStart(msg: string) {
    failStart = msg;
  },
};

if (typeof window !== "undefined") (window as unknown as { __perritoMock: typeof mock }).__perritoMock = mock;
