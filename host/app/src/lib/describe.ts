// Plain-language names for actions, shared by the editor and the mock host.

import type { ActionSpec, KeyFields, KeyKind, NavSpec, SliderSpec } from "./types";

export function describeSlider(s: SliderSpec): string {
  switch (s.type) {
    case "volume":
      return "volume";
    case "homeAssistant":
      return `brightness of ${s.entity}`;
    case "obs":
      return `OBS volume of ${s.input}`;
    case "command":
      return `runs ${s.program}`;
    case "plugin":
      return `${s.plugin}: ${s.action}`;
  }
}

export const DEFAULT_BG = "#1E1E2E";
export const DEFAULT_FG = "#CDD6F4";

export const mediaKeys = {
  playPause: "Play / pause",
  next: "Next track",
  previous: "Previous track",
  stop: "Stop",
  volumeUp: "Volume up",
  volumeDown: "Volume down",
  mute: "Mute",
} as const;

export const obsOps = {
  scene: "Switch scene",
  toggleRecord: "Start or stop recording",
  startRecord: "Start recording",
  stopRecord: "Stop recording",
  toggleStream: "Start or stop streaming",
  startStream: "Start streaming",
  stopStream: "Stop streaming",
  toggleMute: "Mute or unmute an audio source",
  toggleReplayBuffer: "Start or stop the replay buffer",
  saveReplayBuffer: "Save the replay",
} as const;

export function describeAction(a: ActionSpec): string {
  switch (a.type) {
    case "hotkey":
      return `hotkey ${a.keys}`;
    case "text":
      return `types "${a.text.slice(0, 24)}"`;
    case "media":
      return mediaKeys[a.key].toLowerCase();
    case "openUrl":
      return `opens ${a.url}`;
    case "openPath":
      return `opens ${a.path}`;
    case "launch":
      return `starts ${a.program}`;
    case "command":
      return `runs ${a.program}`;
    case "multi":
      return `${a.steps.length} steps`;
    case "obs":
      return `OBS: ${obsOps[a.op].toLowerCase()}${"scene" in a ? ` “${a.scene}”` : ""}${"input" in a ? ` “${a.input}”` : ""}`;
    case "homeAssistant":
      return `Home Assistant ${a.service}${a.entity ? ` ${a.entity}` : ""}`;
    case "plugin":
      return `${a.plugin}: ${a.action}`;
    case "sdPlugin":
      return `Stream Deck plugin: ${a.action}`;
    case "none":
      return "nothing";
  }
}

export function kindOf(f: KeyFields, states: number): { kind: KeyKind; detail: string } {
  if (f.slider) return { kind: "slider", detail: `slider: ${describeSlider(f.slider)}` };
  const nav: NavSpec | null = f.nav;
  if (nav && typeof nav === "object") return { kind: "page", detail: `opens ${nav.page}` };
  if (nav === "back") return { kind: "back", detail: "back" };
  if (nav === "next") return { kind: "next", detail: "next page" };
  if (nav === "previous") return { kind: "previous", detail: "previous page" };
  if (states >= 2) return { kind: "toggle", detail: `${states} states${f.action ? `, ${describeAction(f.action)}` : ""}` };
  if (f.action) return { kind: "action", detail: describeAction(f.action) };
  return { kind: "label", detail: "no action" };
}
