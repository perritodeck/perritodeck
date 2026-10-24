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
const RETYPE_PASSWORD = "Type the password again to use it with a new address.";

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

/**
 * Whether a secret saved for `saved` may go to `now` (both as the host saves them) without
 * being typed again: the same scheme, host and port, or the same host and port moved
 * from http:// to https://. As the host decides (may_keep in file.rs).
 */
export function mayKeep(saved: string, now: string, normalize: (t: string) => { url: string } | { message: string }): boolean {
  const s = normalize(saved);
  if (!("url" in s)) return false;
  const was = splitAddress(s.url);
  const is = splitAddress(now);
  if (typeof was === "string" || typeof is === "string") return false;
  const port = (a: Address) => a.port ?? (a.scheme === "https" ? 443 : a.scheme === "ws" ? 4455 : 80);
  const schemeOk = was.scheme === is.scheme || (was.scheme === "http" && is.scheme === "https");
  return schemeOk && was.host === is.host && port(was) === port(is);
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
  return mayKeep(saved.url, url, haUrl) ? null : RETYPE_TOKEN;
}

/** What's wrong with a section as typed, as the host would say it; null if nothing. */
export function validationMessage(
  kind: "obs" | "homeAssistant" | "companion",
  input: ObsInput | HaInput | CompanionInput,
  saved: SavedSection | null,
): string | null {
  if (kind === "obs") {
    const i = input as ObsInput;
    const r = obsUrl(i.url);
    if ("message" in r) return r.message;
    if (i.password.kind === "keep" && saved?.hasPassword && !mayKeep(saved.url, r.url, obsUrl)) return RETYPE_PASSWORD;
    return null;
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
