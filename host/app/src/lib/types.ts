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

export type KeyKind = "action" | "page" | "back" | "next" | "previous" | "toggle" | "label";

export interface KeyView {
  row: number;
  col: number;
  title: string;
  bg: string;
  fg: string;
  kind: KeyKind;
  detail: string;
}

export interface PageView {
  name: string;
  keys: KeyView[];
}

export interface ProfileView {
  name: string;
  cols: number;
  rows: number;
  pages: PageView[];
  reloadError: string | null;
}

export type HostEvent =
  | { kind: "connected"; deviceId: string; deviceName: string }
  | { kind: "disconnected"; deviceId: string }
  | { kind: "paired"; deviceId: string; deviceName: string }
  | { kind: "keyPressed"; deviceId: string; keyIndex: number }
  | { kind: "profileChanged" };

export type View = "buttons" | "tablets" | "pair" | "settings";
