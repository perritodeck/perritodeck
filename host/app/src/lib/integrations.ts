// The form's checks on what's typed in Settings → Integrations, so a mistake shows as it's
// typed. The host checks again, and its word is final (host/crates/integrations/src/
// file.rs); tests/integrations.spec.ts runs both on the same cases.

import type { CompanionInput, HaInput, ObsInput, Secret } from "./types";

const OBS_EMPTY = "Type OBS's address, like ws://127.0.0.1:4455.";
const OBS_WSS = "OBS's WebSocket server uses ws://, not wss://. Change the address to start with ws://.";
const OBS_SCHEME = "The address must start with ws://, like ws://127.0.0.1:4455.";
const UNUSABLE = "That isn't an address PerritoDeck can use.";
const HA_EMPTY = "Type the address you open Home Assistant with, like http://homeassistant.local:8123.";
const HA_SCHEME = "The address must start with http:// or https://.";
const USERINFO = "Leave any user name and password out of the address.";
const NO_TOKEN = "Paste a long-lived access token.";
const SPACED_TOKEN = "That doesn't look like an access token: it has spaces in it.";
const COMPANION_EMPTY = "Type the name or address of the computer running Companion, like 127.0.0.1.";
const COMPANION_SCHEME = "Type just the name or address, without http:// or ws://.";
const COMPANION_PORT_IN_HOST = "Put the port in the Satellite port box.";
const PORT_RANGE = "Use a number from 1 to 65535.";
const RETYPE_TOKEN = "Type the token again to use it with a new address.";

/** What the window knows about what's saved: never the secret itself. */
export interface SavedSection {
  url: string;
  hasPassword?: boolean;
  hasToken?: boolean;
}

interface Address {
  scheme: string;
  host: string;
  port: number | null;
  path: string;
}

/** `scheme://host:port/rest`, or the reason it can't be used. */
export function splitAddress(text: string): Address | string {
  if (/\s/.test(text)) return UNUSABLE;
  const at = text.indexOf("://");
  if (at < 0) return UNUSABLE;
  const scheme = text.slice(0, at).toLowerCase();
  const rest = text.slice(at + 3);
  const authority = rest.split(/[/?#]/)[0];
  if (authority.includes("@")) return USERINFO;
  let host: string;
  let portText: string | null = null;
  if (authority.startsWith("[")) {
    const close = authority.indexOf("]");
    if (close < 0) return UNUSABLE;
    host = authority.slice(0, close + 1);
    const tail = authority.slice(close + 1);
    if (tail !== "") {
      if (!tail.startsWith(":")) return UNUSABLE;
      portText = tail.slice(1);
    }
  } else {
    const colon = authority.lastIndexOf(":");
    if (colon >= 0) {
      if (authority.indexOf(":") !== colon) return UNUSABLE;
      host = authority.slice(0, colon);
      portText = authority.slice(colon + 1);
    } else {
      host = authority;
    }
  }
  if (host === "" || host === "[]") return UNUSABLE;
  let port: number | null = null;
  if (portText !== null) {
    if (!/^\d+$/.test(portText)) return UNUSABLE;
    port = Number(portText);
    if (port < 1 || port > 65535) return UNUSABLE;
  }
  const path = rest.slice(authority.length);
  return { scheme, host: host.toLowerCase(), port, path: path === "/" ? "" : path };
}

function withScheme(text: string, scheme: string): string {
  const bareIpv6 = !text.includes("[") && (text.match(/:/g) ?? []).length >= 2;
  return bareIpv6 ? `${scheme}://[${text}]` : `${scheme}://${text}`;
}

function schemeOf(text: string): string | null {
  const at = text.indexOf("://");
  return at < 0 ? null : text.slice(0, at).toLowerCase();
}

/** OBS's address as the host saves it, or why it can't be. */
export function obsUrl(text: string): { url: string } | { message: string } {
  let t = text.trim();
  if (t.endsWith("/")) t = t.slice(0, -1);
  if (t === "") return { message: OBS_EMPTY };
  const scheme = schemeOf(t);
  if (scheme === "wss") return { message: OBS_WSS };
  if (scheme !== null && scheme !== "ws") return { message: OBS_SCHEME };
  const a = splitAddress(scheme === null ? withScheme(t, "ws") : t);
  if (typeof a === "string") return { message: a };
  return { url: `ws://${a.host}:${a.port ?? 4455}${a.path}` };
}

/** Home Assistant's address as the host saves it, or why it can't be. */
export function haUrl(text: string): { url: string } | { message: string } {
  const t = text.trim();
  if (t === "") return { message: HA_EMPTY };
  const scheme = schemeOf(t);
  if (scheme !== null && scheme !== "http" && scheme !== "https") return { message: HA_SCHEME };
  const a = splitAddress(scheme === null ? withScheme(t, "http") : t);
  if (typeof a === "string") return { message: a };
  const port = a.port ?? (scheme === null ? 8123 : null);
  return { url: `${a.scheme}://${a.host}${port === null ? "" : `:${port}`}` };
}

/** The host in an address, without an IPv6 address's brackets; null if unreadable. */
export function hostOf(url: string): string | null {
  const t = url.trim();
  const a = splitAddress(t.includes("://") ? t : withScheme(t, "http"));
  return typeof a === "string" ? null : a.host.replace(/^\[|\]$/g, "");
}

/** On the local network: as the host decides when a kept token may go to a new address. */
export function isLocal(host: string): boolean {
  const h = host.replace(/^\[|\]$/g, "").replace(/\.$/, "").toLowerCase();
  const v4 = h.match(/^(\d+)\.(\d+)\.(\d+)\.(\d+)$/);
  if (v4) {
    const [a, b] = [Number(v4[1]), Number(v4[2])];
    return a === 127 || a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) || (a === 169 && b === 254);
  }
  if (h.includes(":")) {
    const first = parseInt(h.split(":")[0] || "0", 16);
    return h === "::1" || (first & 0xffc0) === 0xfe80 || (first & 0xfe00) === 0xfc00;
  }
  return !h.includes(".") || [".local", ".lan", ".home.arpa", ".internal"].some((end) => h.endsWith(end));
}

function tokenProblem(token: Secret, saved: SavedSection | null, url: string): string | null {
  if (token.kind === "clear") return NO_TOKEN;
  if (token.kind === "set") {
    let t = token.value.trim();
    const m = t.match(/^bearer(\s+|$)/i);
    if (m) t = t.slice(m[0].length).trim();
    if (t === "") return NO_TOKEN;
    if (/\s/.test(t)) return SPACED_TOKEN;
    return null;
  }
  if (!saved?.hasToken) return NO_TOKEN;
  const a = splitAddress(url);
  if (typeof a === "string") return a;
  const host = a.host.replace(/^\[|\]$/g, "");
  if (hostOf(saved.url) !== host && a.scheme === "http" && !isLocal(host)) return RETYPE_TOKEN;
  return null;
}

/** What's wrong with a section as typed, as the host would say it; null if nothing. */
export function validationMessage(
  kind: "obs" | "homeAssistant" | "companion",
  input: ObsInput | HaInput | CompanionInput,
  saved: SavedSection | null,
): string | null {
  if (kind === "obs") {
    const r = obsUrl((input as ObsInput).url);
    return "message" in r ? r.message : null;
  }
  if (kind === "homeAssistant") {
    const i = input as HaInput;
    const r = haUrl(i.url);
    if ("message" in r) return r.message;
    return tokenProblem(i.token, saved, r.url);
  }
  const i = input as CompanionInput;
  let host = i.host.trim();
  if (host === "") return COMPANION_EMPTY;
  if (host.includes("://")) return COMPANION_SCHEME;
  if (host.startsWith("[") && host.endsWith("]")) host = host.slice(1, -1);
  if (host === "" || /[\s/?#@[\]]/.test(host)) return UNUSABLE;
  if ((host.match(/:/g) ?? []).length === 1) return COMPANION_PORT_IN_HOST;
  const port = i.port.trim();
  if (!/^\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535) return PORT_RANGE;
  return null;
}
