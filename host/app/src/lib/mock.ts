// Stand-in for the Rust host when the UI runs in a plain browser. Playwright drives it
// through `window.__perritoMock`.

import { DEFAULT_BG, DEFAULT_FG, kindOf } from "./describe";
import type { McpSettings, McpView, PluginManifest, PluginsView, SdPlugin, AutoSwitch, DeviceInfo, Edit, HostEvent, HostStatus, KeyFields, KeyView, PairingOffer, ProfileSummary, ProfileView } from "./types";

type Listener = (payload: unknown) => void;

const status: HostStatus = {
  hostName: "studio-desktop",
  version: "0.1.0",
  fingerprint: "3FA9 07C2 D41E",
  addresses: ["192.168.1.20:46710"],
  insecure: false,
  dataDir: "/home/me/.config/PerritoDeck",
  logsDir: "/home/me/.config/PerritoDeck/logs",
  profilePath: "/home/me/.config/PerritoDeck/profiles/default.toml",
  profilesDir: "/home/me/.config/PerritoDeck/profiles",
  integrations: ["OBS"],
  paused: false,
  autoSwitchSupported: true,
  usbDevices: [],
  usbAvailable: true,
  companion: null,
};

let devices: DeviceInfo[] = [
  { id: "a1b2c3", name: "Fire HD 10", lastSeen: Math.floor(Date.now() / 1000), connected: true, profile: "default", assignedProfile: null },
  {
    id: "d4e5f6",
    name: "Pixel Tablet",
    lastSeen: Math.floor(Date.now() / 1000) - 3 * 86400,
    connected: false,
    profile: "default",
    assignedProfile: null,
  },
];

function key(row: number, col: number, f: Partial<KeyFields>, states = 0, shown?: Partial<KeyView>): KeyView {
  const fields: KeyFields = {
    title: null,
    bg: null,
    fg: null,
    action: null,
    nav: null,
    icon: null,
    slider: null,
    states: [],
    follow: null,
    ...f,
  };
  return {
    row,
    col,
    title: fields.title ?? "",
    bg: fields.bg ?? DEFAULT_BG,
    fg: fields.fg ?? DEFAULT_FG,
    ...kindOf(fields, states),
    fields,
    states,
    hasIcon: !!fields.icon,
    iconData: fields.icon ? MOCK_ICON : null,
    ...shown,
  };
}

const dark = "#11111B";
// A small orange circle.
const MOCK_ICON =
  "data:image/svg+xml;utf8," +
  encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><circle cx="5" cy="5" r="4" fill="%23FAB387"/></svg>').replace("%2523", "%23");

const starter: ProfileView = {
  id: "default",
  name: "Starter",
  cols: 5,
  rows: 3,
  fixedGrid: true,
  companion: false,
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
        key(
          1,
          0,
          {
            states: [
              { title: "Mic\nlive", bg: "#A6E3A1", fg: dark, action: { type: "hotkey", keys: "Ctrl+Shift+M" } },
              { title: "Mic\nmuted", bg: "#F38BA8", fg: dark, action: { type: "hotkey", keys: "Ctrl+Shift+M" } },
            ],
          },
          2,
          { title: "Mic\nlive", bg: "#A6E3A1", fg: dark, detail: "2 states, hotkey Ctrl+Shift+M" },
        ),
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
        key(0, 4, { title: "Volume", bg: "#89B4FA", fg: dark, slider: { type: "volume" } }),
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

const streaming: ProfileView = {
  id: "streaming",
  name: "Streaming",
  cols: 5,
  rows: 3,
  fixedGrid: true,
  companion: false,
  reloadError: null,
  canUndo: false,
  canRedo: false,
  pages: [
    {
      name: "Main",
      keys: [
        key(0, 0, { title: "Rec", bg: "#F38BA8", fg: dark, action: { type: "obs", op: "toggleRecord" } }),
        key(0, 1, { title: "Live", bg: "#F38BA8", fg: dark, action: { type: "obs", op: "toggleStream" } }),
        key(0, 2, {
          title: "Any\nemote",
          action: { type: "random", actions: [{ type: "text", text: "GG" }, { type: "text", text: "Nice!" }] },
        }),
        key(2, 0, { title: "‹ Home", bg: "#CBA6F7", fg: dark, nav: { profile: "default" } }),
      ],
    },
  ],
};

let profiles: Record<string, ProfileView> = { default: starter, streaming };
let autoSwitch: AutoSwitch = { enabled: false, rules: [] };

const httpPlugin: PluginManifest = {
  id: "dev.perritodeck.http",
  name: "HTTP requests",
  version: "1.0.0",
  description: "Call a web address when a key is pressed.",
  author: "PerritoDeck contributors",
  runtime: "python",
  main: "plugin.py",
  actions: [
    {
      id: "request",
      name: "Send a request",
      description: "",
      fields: [
        { id: "url", label: "Web address", type: "text", placeholder: "https://example.com/hook" },
        {
          id: "method",
          label: "Method",
          type: "select",
          default: "POST",
          options: ["GET", "POST", "PUT"].map((m) => ({ value: m, label: m })),
        },
        { id: "insecure", label: "Skip the check", type: "checkbox" },
      ],
    },
    {
      id: "send-value",
      name: "Send the slider's value",
      description: "",
      slider: true,
      fields: [{ id: "url", label: "Web address", type: "text", placeholder: "https://example.com/dim?level={value}" }],
    },
  ],
  values: [
    { id: "last-status", name: "Status of the last request" },
    { id: "up", name: "Address answers", arg: "Web address" },
  ],
  settings: [
    { id: "base", label: "Base address", type: "text", placeholder: "https://example.com" },
    { id: "token", label: "Token", type: "password" },
    { id: "insecure", label: "Skip certificate check", type: "checkbox" },
  ],
};
const mqttPlugin: PluginManifest = {
  ...httpPlugin,
  id: "dev.perritodeck.mqtt",
  name: "MQTT",
  description: "Publish an MQTT message.",
  actions: [{ id: "publish", name: "Publish a message", description: "", fields: [{ id: "topic", label: "Topic", type: "text" }] }],
  values: [],
};
const sdCounter: SdPlugin = {
  uuid: "com.example.counter",
  name: "Counter",
  version: "1.0.0.0",
  author: "Example",
  description: "Counts presses.",
  folder: "/x",
  status: { state: "running" },
  icon: null,
  actions: [{ uuid: "com.example.counter.up", name: "Count up", tooltip: "Adds one", icon: null, hasSettingsPage: true }],
};
let sdPlugins: SdPlugin[] = [];
let installedPlugins: PluginsView["installed"] = [
  { manifest: httpPlugin, status: { state: "running" }, folder: "/home/me/.config/PerritoDeck/plugins/dev.perritodeck.http" },
];
function pluginsView(): PluginsView {
  return {
    installed: installedPlugins,
    broken: [],
    folder: "/home/me/.config/PerritoDeck/plugins",
    catalog: [httpPlugin, mqttPlugin].map((m) => ({ manifest: m, installed: installedPlugins.some((p) => p.manifest.id === m.id) })),
  };
}

type Snapshot = { id: string; before: ProfileView | null; after: ProfileView | null };
const undoStack: Snapshot[] = [];
const redoStack: Snapshot[] = [];

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
    case "setName":
      if (!e.name.trim()) throw "a profile needs a name";
      p.name = e.name.trim();
      return;
    case "setKey": {
      if (e.key.bg && !/^#[0-9a-fA-F]{6}$/.test(e.key.bg)) throw "that change would break the profile:\n  - colours look like #1E1E2E";
      const pg = page(p, e.page);
      const old = pg.keys.find((k) => same(k, e.at));
      const s0 = e.key.states[0];
      const next = key(e.at[0], e.at[1], e.key, e.key.states.length, e.key.states.length >= 2 ? { title: s0?.title ?? e.key.title ?? "", bg: s0?.bg ?? e.key.bg ?? DEFAULT_BG, fg: s0?.fg ?? e.key.fg ?? DEFAULT_FG } : {});
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
          k.fields.nav && typeof k.fields.nav === "object" && !("profile" in k.fields.nav) && k.fields.nav.page === e.from
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

function view(id: string): ProfileView {
  const p = profiles[id];
  if (!p) throw `There's no profile called ${id}.`;
  return structuredClone({ ...p, canUndo: undoStack.length > 0, canRedo: redoStack.length > 0 });
}

function summaries(): ProfileSummary[] {
  return Object.values(profiles)
    .map((p) => ({ id: p.id, name: p.name, pages: p.pages.length, isDefault: p.id === "default", hasError: !!p.reloadError }))
    .sort((a, b) => Number(b.isDefault) - Number(a.isDefault) || a.name.localeCompare(b.name));
}

function slug(name: string) {
  const s = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  return s || "profile";
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
      case "list_profiles":
        return out(summaries());
      case "get_profile":
        return out(view((args?.id as string | null) ?? "default"));
      case "edit_profile": {
        const id = args?.id as string;
        const edits = args?.edits as Edit[];
        const draft = structuredClone(profiles[id]);
        if (!draft) throw `There's no profile called ${id}.`;
        for (const e of edits) applyEdit(draft, e);
        this.edits.push(edits);
        undoStack.push({ id, before: profiles[id], after: draft });
        redoStack.length = 0;
        profiles[id] = draft;
        return out(view(id));
      }
      case "undo_edit":
      case "redo_edit": {
        const back = cmd === "undo_edit";
        const [from, to] = back ? [undoStack, redoStack] : [redoStack, undoStack];
        const step = from.pop();
        if (!step) throw `Nothing to ${back ? "undo" : "redo"}.`;
        to.push(step);
        const target = back ? step.before : step.after;
        if (target) profiles[step.id] = target;
        return out(view(step.id));
      }
      case "create_profile": {
        const name = (args?.name as string).trim();
        if (!name) throw "A profile needs a name.";
        let id = slug(name);
        for (let n = 2; profiles[id]; n++) id = `${slug(name)}-${n}`;
        const from = args?.copyOf as string | null;
        const companion = !!args?.companion;
        profiles[id] = from
          ? { ...structuredClone(profiles[from]), id, name }
          : {
              id,
              name,
              cols: 5,
              rows: 3,
              fixedGrid: false,
              companion,
              reloadError: null,
              canUndo: false,
              canRedo: false,
              pages: companion ? [] : [{ name: "Main", keys: [] }],
            };
        return out(id);
      }
      case "delete_profile": {
        const id = args?.id as string;
        if (id === "default") throw "The default profile can't be deleted.";
        delete profiles[id];
        devices = devices.map((d) => (d.assignedProfile === id ? { ...d, assignedProfile: null, profile: "default" } : d));
        autoSwitch = { ...autoSwitch, rules: autoSwitch.rules.filter((r) => r.profile !== id) };
        return out(undefined);
      }
      case "assign_profile": {
        const pid = (args?.profileId as string | null) ?? null;
        if (pid && !profiles[pid]) throw `There's no profile called ${pid}.`;
        devices = devices.map((d) =>
          d.id === args?.deviceId ? { ...d, assignedProfile: pid === "default" ? null : pid, profile: pid ?? "default" } : d,
        );
        return out(undefined);
      }
      case "rename_device": {
        const name = (args?.name as string).trim();
        if (!name) throw "A tablet needs a name.";
        devices = devices.map((d) => (d.id === args?.deviceId ? { ...d, name } : d));
        return out(undefined);
      }
      case "set_paused":
        status.paused = !!args?.paused;
        this.emit("host-event", { kind: "profileChanged" });
        return out(undefined);
      case "get_auto_switch":
        return out(autoSwitch);
      case "set_auto_switch":
        autoSwitch = structuredClone(args?.auto as AutoSwitch);
        return out(undefined);
      case "recent_apps":
        return out(["obs64", "Code", "firefox"]);
      case "export_profile":
        this.exported.push(args?.id as string);
        return out(true);
      case "import_profile": {
        let id = "imported";
        for (let n = 2; profiles[id]; n++) id = `imported-${n}`;
        profiles[id] = { ...structuredClone(streaming), id, name: "Imported" };
        return out(id);
      }
      case "list_plugins":
        return out(pluginsView());
      case "install_catalog_plugin": {
        const m = [httpPlugin, mqttPlugin].find((p) => p.id === args?.id);
        if (!m) throw "That plugin doesn't ship with this version.";
        installedPlugins = [...installedPlugins.filter((p) => p.manifest.id !== m.id), { manifest: m, status: { state: "running" }, folder: "/x" }];
        return out(m.id);
      }
      case "install_plugin_folder":
        return out(null);
      case "remove_plugin":
        installedPlugins = installedPlugins.filter((p) => p.manifest.id !== args?.id);
        return out(undefined);
      case "restart_plugin":
      case "open_plugins_folder":
        return out(undefined);
      case "import_icon_data":
        return out("icons/builtin.png");
      case "choose_icon":
        return out("icons/mock.png");
      case "accessibility_allowed":
        return out(this.accessibility);
      case "open_accessibility_settings":
        this.accessibility = true;
        return out(undefined);
      case "get_autostart":
        return out(this.autostart);
      case "set_autostart":
        this.autostart = !!args?.enabled;
        return out(undefined);
      case "get_network":
        return out({ ...this.network });
      case "set_network": {
        const n = args?.network as { port: number; lan: boolean };
        if (n.port < 1024) throw "Use a port from 1024 up.";
        this.network = { ...n };
        return out(undefined);
      }
      case "get_check_updates":
        return out(this.checkUpdates);
      case "set_check_updates":
        this.checkUpdates = !!args?.enabled;
        return out(undefined);
      case "check_for_updates":
        if (typeof this.latest === "string") throw this.latest;
        return out(this.latest);
      case "get_plugin_settings":
        return out({ ...(this.pluginSettings[String(args?.id)] ?? {}) });
      case "set_plugin_settings":
        this.pluginSettings[String(args?.id)] = { ...(args?.settings as Record<string, unknown>) };
        return out(undefined);
      case "list_sd_plugins":
        return out({ installed: sdPlugins, broken: [], folder: "/home/me/.config/PerritoDeck/sdplugins" });
      case "install_sd_plugin": {
        if (sdPlugins.some((p) => p.uuid === sdCounter.uuid)) return out(null);
        sdPlugins = [...sdPlugins, sdCounter];
        return out(sdCounter.uuid);
      }
      case "sd_settings_page":
        return out("about:blank#sd-settings");
      case "remove_sd_plugin":
        sdPlugins = sdPlugins.filter((p) => p.uuid !== args?.uuid);
        return out(undefined);
      case "restart_sd_plugin":
        return out(undefined);
      case "list_online_plugins": {
        if (typeof this.onlineIndex === "string") throw this.onlineIndex;
        return out(
          this.onlineIndex.map((o) => {
            const have = installedPlugins.find((p) => p.manifest.id === o.id)?.manifest.version ?? null;
            return { ...o, installed: have, update: !!have && have !== o.version };
          }),
        );
      }
      case "install_online_plugin": {
        if (typeof this.onlineIndex === "string") throw this.onlineIndex;
        const o = this.onlineIndex.find((x) => x.id === args?.id);
        if (!o) throw `${args?.id} isn't in the plugin catalogue.`;
        const m: PluginManifest = { ...httpPlugin, id: o.id, name: o.name, version: o.version, description: o.description, actions: [], values: [] };
        installedPlugins = [...installedPlugins.filter((p) => p.manifest.id !== o.id), { manifest: m, status: { state: "running" }, folder: "/x" }];
        return out(o.id);
      }
      case "open_release_page":
        this.opened.push(String(args?.url));
        return out(undefined);
      case "restart_app":
        this.restarted = true;
        return out(undefined);
      case "test_action": {
        const a = args?.action as { type: string; program?: string };
        this.tested.push(args?.action);
        if (a.type === "command" && a.program === "false") return out("exited with status 1");
        return out(null);
      }
      case "open_profile_file":
      case "open_data_folder":
        return out(undefined);
      case "open_logs_folder":
        this.opened.push("logs");
        return out(undefined);
      case "get_diagnostics":
        return out(
          `PerritoDeck diagnostics\n=======================\n\n## Versions\napp:            ${status.version}\n\n## Last 1 log lines\nINFO host running\n`,
        );
      case "get_detailed_logs":
        return out(this.detailedLogs);
      case "set_detailed_logs":
        this.detailedLogs = !!args?.enabled;
        return out(undefined);
      case "get_mcp":
        return out(mcpView(this));
      case "set_mcp": {
        const m = args?.mcp as McpSettings;
        if (m.port < 1024) throw "Use a port from 1024 up.";
        if (m.port === 46710) throw `Port ${m.port} is the one tablets use; pick another.`;
        this.mcpSettings = { ...m };
        return out(mcpView(this));
      }
      case "new_mcp_token":
        this.mcpToken = `mocktoken${String(Number(this.mcpToken.slice(9)) + 1).padStart(4, "0")}`;
        return out(mcpView(this));
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
      devices.push({ id: e.deviceId, name: e.deviceName, lastSeen: null, connected: false, profile: "default", assignedProfile: null });
    }
    if (e.kind === "connected" || e.kind === "disconnected") {
      devices = devices.map((d) => (d.id === e.deviceId ? { ...d, connected: e.kind === "connected" } : d));
    }
    this.emit("host-event", e);
  },
  autostart: false,
  /** The status the host reports; tests may change it. */
  hostStatus: status,
  network: { port: 46710, lan: true },
  // Tests can set these before the page loads, through window.__perritoMockInit.
  checkUpdates: !!init().checkUpdates,
  /** What check_for_updates answers; a string is an error. */
  latest: (init().latest ?? null) as { version: string; url: string } | string | null,
  restarted: false,
  detailedLogs: false,
  mcpSettings: { enabled: false, press: false, edit: false, run: false, port: 46711 } as McpSettings,
  mcpToken: "mocktoken0001",
  /** A port "another program" holds, so turning MCP on there fails. */
  busyPort: 46799,
  pluginSettings: {} as Record<string, Record<string, unknown>>,
  /** The online catalogue; a string is an error. */
  onlineIndex: [
    { id: "dev.example.weather", name: "Weather", version: "1.2.0", description: "Today's weather on a key.", author: "Example", url: "w.zip", sha256: "00" },
    { id: "dev.perritodeck.http", name: "HTTP requests", version: "1.1.0", description: "Call a web address.", author: "PerritoDeck contributors", url: "h.zip", sha256: "00" },
  ] as { id: string; name: string; version: string; description: string; author: string; url: string; sha256: string }[] | string,
  opened: [] as string[],
  exported: [] as string[],
  accessibility: null as boolean | null,
  setReloadError(msg: string | null) {
    profiles.default.reloadError = msg;
    this.emit("host-event", { kind: "profileChanged" });
  },
  failToStart(msg: string) {
    failStart = msg;
  },
};

function mcpView(m: { mcpSettings: McpSettings; mcpToken: string; busyPort: number }): McpView {
  const settings = { ...m.mcpSettings };
  const url = `http://127.0.0.1:${settings.port}/mcp`;
  const command = "/usr/bin/perritodeck-app";
  const state: McpView["state"] = !settings.enabled
    ? { state: "off" }
    : settings.port === m.busyPort
      ? { state: "error", message: `port ${settings.port} is in use by another program` }
      : { state: "running", url };
  return {
    settings,
    state,
    url,
    snippets: {
      command,
      args: ["mcp"],
      stdioConfig: JSON.stringify({ mcpServers: { perritodeck: { command, args: ["mcp"] } } }, null, 2),
      httpConfig: JSON.stringify(
        { mcpServers: { perritodeck: { type: "http", url, headers: { Authorization: `Bearer ${m.mcpToken}` } } } },
        null,
        2,
      ),
      claudeCodeStdio: `claude mcp add perritodeck -- ${command} mcp`,
      claudeCodeHttp: `claude mcp add --transport http perritodeck ${url} --header "Authorization: Bearer ${m.mcpToken}"`,
    },
  };
}

function init(): { checkUpdates?: boolean; latest?: { version: string; url: string } | string | null } {
  return typeof window !== "undefined" ? ((window as unknown as { __perritoMockInit?: object }).__perritoMockInit ?? {}) : {};
}

if (typeof window !== "undefined") (window as unknown as { __perritoMock: typeof mock }).__perritoMock = mock;
