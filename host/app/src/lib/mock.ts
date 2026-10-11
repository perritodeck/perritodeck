// Stand-in for the Rust host when the UI runs in a plain browser. Playwright drives it
// through `window.__perritoMock`.

import type { DeviceInfo, HostEvent, HostStatus, PairingOffer, ProfileView } from "./types";

type Listener = (payload: unknown) => void;

const D = { bg: "#1E1E2E", fg: "#CDD6F4" };
const dark = "#11111B";

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

const profile: ProfileView = {
  name: "Starter",
  cols: 5,
  rows: 3,
  reloadError: null,
  pages: [
    {
      name: "Main",
      keys: [
        { row: 0, col: 0, title: "{time}", ...D, kind: "label", detail: "no action" },
        { row: 0, col: 1, title: "Play /\nPause", bg: "#A6E3A1", fg: dark, kind: "action", detail: "media PlayPause" },
        { row: 0, col: 2, title: "Vol +", bg: "#89B4FA", fg: dark, kind: "action", detail: "media VolumeUp" },
        { row: 0, col: 3, title: "Vol −", bg: "#89B4FA", fg: dark, kind: "action", detail: "media VolumeDown" },
        { row: 0, col: 4, title: "Mute", bg: "#89B4FA", fg: dark, kind: "action", detail: "media Mute" },
        { row: 1, col: 0, title: "Mic\nlive", bg: "#A6E3A1", fg: dark, kind: "toggle", detail: "2 states, hotkey Ctrl+Shift+M" },
        { row: 1, col: 1, title: "Copy", ...D, kind: "action", detail: "hotkey Ctrl+C" },
        { row: 1, col: 2, title: "Paste", ...D, kind: "action", detail: "hotkey Ctrl+V" },
        { row: 1, col: 3, title: "Type\nhello", ...D, kind: "action", detail: 'types "Hello from Perrito! "' },
        { row: 1, col: 4, title: "Perrito\nwebsite", bg: "#FAB387", fg: dark, kind: "action", detail: "opens https://perritodeck.dev" },
        { row: 2, col: 0, title: "More ›", bg: "#CBA6F7", fg: dark, kind: "page", detail: "opens More" },
        { row: 2, col: 4, title: "{date}", bg: D.bg, fg: "#9399B2", kind: "label", detail: "no action" },
      ],
    },
    {
      name: "More",
      keys: [
        { row: 0, col: 0, title: "‹ Back", bg: "#CBA6F7", fg: dark, kind: "back", detail: "back" },
        { row: 0, col: 1, title: "Run a\ncommand", ...D, kind: "action", detail: "runs hostname" },
        { row: 0, col: 2, title: "Select all\n+ copy", ...D, kind: "action", detail: "2 steps" },
      ],
    },
  ],
};

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

export const mock = {
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
        return out(profile);
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
