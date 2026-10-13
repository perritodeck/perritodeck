// Mirrors the serde types in host/crates/host/src/lib.rs.

export interface HostStatus {
  hostName: string;
  version: string;
  fingerprint: string | null;
  addresses: string[];
  insecure: boolean;
  dataDir: string;
  /** Where the log files are. */
  logsDir: string;
  profilePath: string;
  profilesDir: string;
  integrations: string[];
  /** The mistake in integrations.toml, which is then ignored; null if there's none. */
  integrationsError: string | null;
  paused: boolean;
  autoSwitchSupported: boolean;
  /** Serial numbers of tablets on a USB cable that can reach this computer. */
  usbDevices: string[];
  /** Tablets on a cable that can't connect yet and why, or adb not answering; one line each. */
  usbProblems: string[];
  /** adb was found, so tablets on USB work. */
  usbAvailable: boolean;
  /** How the connection to Bitfocus Companion is doing; null if it isn't set up. */
  companion: string | null;
}

/** Why tablets may not get through over Wi-Fi (most often the firewall). */
export interface NetworkHint {
  /** Tablets that tried the network, got no answer and came over the USB cable instead. */
  fellBack: string[];
  /** Windows Firewall blocks PerritoDeck on this network, or has no rule letting it in. */
  firewallBlocks: boolean;
  /** A network in use is one Windows calls Public. */
  publicNetwork: boolean;
  /** The port tablets connect to. */
  port: number;
  /** Whose firewall to explain. */
  os: "windows" | "macos" | "linux" | string;
}

export interface DeviceInfo {
  id: string;
  name: string;
  /** Unix seconds. */
  lastSeen: number | null;
  connected: boolean;
  /** The profile it shows now. */
  profile: string;
  /** The profile chosen for it; null means the default. */
  assignedProfile: string | null;
}

export interface ProfileSummary {
  id: string;
  name: string;
  pages: number;
  isDefault: boolean;
  hasError: boolean;
}

export interface SwitchRule {
  app: string;
  profile: string;
  device?: string | null;
}

export interface AutoSwitch {
  enabled: boolean;
  rules: SwitchRule[];
}

export interface PairingOffer {
  code: string;
  uri: string;
  fingerprint: string;
  expiresInSecs: number;
  qrSvg: string;
}

export type MediaKey = "playPause" | "next" | "previous" | "stop" | "volumeUp" | "volumeDown" | "mute";

export type ObsAction =
  | { op: "scene"; scene: string }
  | { op: "toggleMute"; input: string }
  | {
      op:
        | "toggleRecord"
        | "startRecord"
        | "stopRecord"
        | "toggleStream"
        | "startStream"
        | "stopStream"
        | "toggleReplayBuffer"
        | "saveReplayBuffer";
    };

// Mirrors perritodeck_actions::ActionSpec.
export type ActionSpec =
  | { type: "hotkey"; keys: string }
  | { type: "text"; text: string }
  | { type: "media"; key: MediaKey }
  | { type: "openUrl"; url: string }
  | { type: "openPath"; path: string }
  | { type: "launch"; program: string; args?: string[] }
  | { type: "command"; program: string; args?: string[]; cwd?: string; timeoutMs?: number }
  | { type: "multi"; steps: { action: ActionSpec; delayMs?: number }[] }
  | { type: "random"; actions: ActionSpec[] }
  | ({ type: "obs" } & ObsAction)
  | { type: "homeAssistant"; service: string; entity?: string; data?: unknown }
  | { type: "plugin"; plugin: string; action: string; settings?: Record<string, unknown> }
  | { type: "sdPlugin"; plugin: string; action: string; settings?: Record<string, unknown> }
  | { type: "none" };

// Mirrors perritodeck_actions::SliderSpec.
export type SliderSpec =
  | { type: "volume" }
  | { type: "homeAssistant"; entity: string }
  | { type: "obs"; input: string }
  | { type: "command"; program: string; args?: string[] }
  | { type: "plugin"; plugin: string; action: string; settings?: Record<string, unknown>; follow?: string };

// Mirrors perritodeck_profile::NavSpec. `{ profile }` switches the tablet it's pressed on.
export type NavSpec = "back" | "next" | "previous" | { page: string } | { profile: string; page?: string };

/** What the editor changes on a key. `null` means "not set" (defaults apply). */
export interface KeyFields {
  title: string | null;
  bg: string | null;
  fg: string | null;
  action: ActionSpec | null;
  nav: NavSpec | null;
  /** Image file relative to the profiles folder, e.g. "icons/3fa907c2d41e.png". */
  icon: string | null;
  /** Makes the key a slider. */
  slider: SliderSpec | null;
  /** Two or more make the key a toggle; empty for an ordinary key. */
  states: StateFields[];
  /** For a two-state toggle: the live value it follows, e.g. "obs:recording". */
  follow: string | null;
}

/** One state of a toggle (fields fall back to the key's). */
export interface StateFields {
  title?: string | null;
  icon?: string | null;
  bg?: string | null;
  fg?: string | null;
  action?: ActionSpec | null;
}

export type Pos = [number, number];

// Mirrors perritodeck_profile::edit::Edit.
export type Edit =
  | { op: "setKey"; page: string; at: Pos; key: KeyFields }
  | { op: "clearKey"; page: string; at: Pos }
  | { op: "moveKey"; page: string; from: Pos; to: Pos }
  | { op: "moveKeyToPage"; fromPage: string; at: Pos; toPage: string; to: Pos }
  | { op: "setName"; name: string }
  | { op: "addPage"; name: string }
  | { op: "renamePage"; from: string; to: string }
  | { op: "removePage"; name: string }
  | { op: "movePage"; name: string; index: number };

export type KeyKind = "action" | "page" | "profile" | "back" | "next" | "previous" | "toggle" | "slider" | "label";

export interface KeyView {
  row: number;
  col: number;
  title: string;
  bg: string;
  fg: string;
  kind: KeyKind;
  detail: string;
  fields: KeyFields;
  /** Toggle states; 0 for an ordinary key. */
  states: number;
  hasIcon: boolean;
  /** PNG data URL of the key's image, for the preview. */
  iconData: string | null;
  /**
   * What a Stream Deck plugin drew on the key (PNG data URL, filling the key), shown
   * instead of the background and image. Display only. Plugins draw only keys a tablet
   * shows, so it's null for the others.
   */
  liveImage: string | null;
  /** The title a Stream Deck plugin gave the key, shown instead of `title`. */
  liveTitle: string | null;
}

export interface PageView {
  name: string;
  keys: KeyView[];
}

export interface ProfileView {
  id: string;
  name: string;
  cols: number;
  rows: number;
  fixedGrid: boolean;
  /** Shows Bitfocus Companion's buttons; `pages` is then unused. */
  companion: boolean;
  pages: PageView[];
  reloadError: string | null;
  canUndo: boolean;
  canRedo: boolean;
}

export type HostEvent =
  | { kind: "connected"; deviceId: string; deviceName: string }
  | { kind: "disconnected"; deviceId: string }
  | { kind: "paired"; deviceId: string; deviceName: string }
  | { kind: "keyPressed"; deviceId: string; keyIndex: number }
  | { kind: "profileChanged" }
  /** A Stream Deck plugin drew keys of the profile differently. */
  | { kind: "keyLooksChanged"; profile: string }
  | { kind: "foregroundApp"; app: string | null };

export type View = "buttons" | "tablets" | "pair" | "switching" | "plugins" | "settings";

// Mirrors perritodeck_plugins::manifest.
export interface PluginField {
  id: string;
  label: string;
  type: "text" | "number" | "checkbox" | "select" | "password";
  placeholder?: string;
  options?: { value: string; label: string }[];
  default?: unknown;
}

export interface PluginManifest {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string;
  runtime: "node" | "python" | "exec";
  main: string;
  /** `slider`: the action takes a value (0–100) and goes on a slider, not a key. */
  actions: { id: string; name: string; description: string; fields: PluginField[]; slider?: boolean }[];
  /** `arg`: the value needs an argument (a host, a VM id); this is its label. */
  values: { id: string; name: string; arg?: string }[];
  /** Settings for the whole plugin, filled in on the Plugins page. */
  settings?: PluginField[];
}

export type PluginStatus =
  | { state: "starting" }
  | { state: "running" }
  | { state: "missingRuntime"; runtime: string }
  | { state: "crashed"; reason: string }
  | { state: "stopped" };

export interface PluginsView {
  installed: { manifest: PluginManifest; status: PluginStatus; folder: string }[];
  broken: { folder: string; problem: string }[];
  folder: string;
  catalog: { manifest: PluginManifest; installed: boolean }[];
}

/** How tablets reach this computer; applies after a restart. */
export interface Network {
  port: number;
  /** Accept tablets over Wi-Fi/Ethernet; off means USB (and this computer) only. */
  lan: boolean;
}

/** The local MCP server for AI assistants. Everything is off by default. */
export interface McpSettings {
  enabled: boolean;
  /** Press existing keys and switch the profile a tablet shows. */
  press: boolean;
  /** Create and change keys, pages and profiles. */
  edit: boolean;
  /** Run any action, including programs and commands. */
  run: boolean;
  port: number;
}

export type McpState = { state: "off" } | { state: "running"; url: string } | { state: "error"; message: string };

export interface McpSnippets {
  command: string;
  args: string[];
  stdioConfig: string;
  httpConfig: string;
  claudeCodeStdio: string;
  claudeCodeHttp: string;
}

export interface McpView {
  settings: McpSettings;
  state: McpState;
  url: string;
  snippets: McpSnippets;
  /** An assistant connected through the app's own program (`perritodeck-app mcp`), the
   * setup from before the separate bridge. It works, but keeps the app from updating. */
  appBridgeUsed: boolean;
}

export interface Update {
  version: string;
  url: string;
}

/** A plugin in the online catalogue. */
export interface OnlineEntry {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string;
  url: string;
  sha256: string;
  /** The installed version, if any. */
  installed: string | null;
  /** The catalogue has a newer version. */
  update: boolean;
}

/** A Stream Deck plugin installed on this computer. */
export interface SdPlugin {
  uuid: string;
  name: string;
  version: string;
  author: string;
  description: string;
  folder: string;
  status:
    | { state: "starting" }
    | { state: "running" }
    | { state: "needsNode" }
    | { state: "unsupported"; reason: string }
    | { state: "crashed"; reason: string };
  icon: string | null;
  actions: { uuid: string; name: string; tooltip: string; icon: string | null; hasSettingsPage: boolean }[];
}

export interface SdPluginsView {
  installed: SdPlugin[];
  broken: { folder: string; problem: string }[];
  folder: string;
}
