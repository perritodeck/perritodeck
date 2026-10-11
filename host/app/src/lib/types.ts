// Mirrors the serde types in host/crates/host/src/lib.rs.

export interface HostStatus {
  hostName: string;
  version: string;
  fingerprint: string | null;
  addresses: string[];
  insecure: boolean;
  dataDir: string;
  profilePath: string;
  integrations: string[];
}

export interface DeviceInfo {
  id: string;
  name: string;
  /** Unix seconds. */
  lastSeen: number | null;
  connected: boolean;
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
  | { type: "none" };

export type NavSpec = "back" | "next" | "previous" | { page: string };

/** What the editor changes on a key. `null` means "not set" (defaults apply). */
export interface KeyFields {
  title: string | null;
  bg: string | null;
  fg: string | null;
  action: ActionSpec | null;
  nav: NavSpec | null;
}

export type Pos = [number, number];

// Mirrors perritodeck_profile::edit::Edit.
export type Edit =
  | { op: "setKey"; page: string; at: Pos; key: KeyFields }
  | { op: "clearKey"; page: string; at: Pos }
  | { op: "moveKey"; page: string; from: Pos; to: Pos }
  | { op: "moveKeyToPage"; fromPage: string; at: Pos; toPage: string; to: Pos }
  | { op: "addPage"; name: string }
  | { op: "renamePage"; from: string; to: string }
  | { op: "removePage"; name: string }
  | { op: "movePage"; name: string; index: number };

export type KeyKind = "action" | "page" | "back" | "next" | "previous" | "toggle" | "label";

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
}

export interface PageView {
  name: string;
  keys: KeyView[];
}

export interface ProfileView {
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
  | { kind: "profileChanged" };

export type View = "buttons" | "tablets" | "pair" | "settings";
