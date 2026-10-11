// Mirrors the serde types in host/crates/host/src/lib.rs.

export interface HostStatus {
  hostName: string;
  version: string;
  fingerprint: string | null;
  addresses: string[];
  insecure: boolean;
  dataDir: string;
  profilePath: string;
  profilesDir: string;
  integrations: string[];
  paused: boolean;
  autoSwitchSupported: boolean;
  /** Serial numbers of tablets on a USB cable that can reach this computer. */
  usbDevices: string[];
  /** adb was found, so tablets on USB work. */
  usbAvailable: boolean;
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
  | ({ type: "obs" } & ObsAction)
  | { type: "homeAssistant"; service: string; entity?: string; data?: unknown }
  | { type: "plugin"; plugin: string; action: string; settings?: Record<string, unknown> }
  | { type: "none" };

// Mirrors perritodeck_actions::SliderSpec.
export type SliderSpec =
  | { type: "volume" }
  | { type: "homeAssistant"; entity: string }
  | { type: "obs"; input: string }
  | { type: "command"; program: string; args?: string[] }
  | { type: "plugin"; plugin: string; action: string; settings?: Record<string, unknown>; follow?: string };

export type NavSpec = "back" | "next" | "previous" | { page: string };

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

export type KeyKind = "action" | "page" | "back" | "next" | "previous" | "toggle" | "slider" | "label";

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
  | { kind: "foregroundApp"; app: string | null };

export type View = "buttons" | "tablets" | "pair" | "switching" | "plugins" | "settings";

// Mirrors perritodeck_plugins::manifest.
export interface PluginField {
  id: string;
  label: string;
  type: "text" | "number" | "checkbox" | "select";
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
  values: { id: string; name: string }[];
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

export interface Update {
  version: string;
  url: string;
}
