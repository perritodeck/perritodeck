// Talks to the Rust side through Tauri. Opened in a plain browser (`npm run dev`,
// Playwright), it uses the in-memory mock instead so the UI can be worked on alone.

import type { ActionSpec, AutoSwitch, DeviceInfo, Edit, ProfileSummary, HostEvent, HostStatus, PairingOffer, ProfileView, View } from "./types";
import { mock } from "./mock";

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri) return mock.invoke<T>(cmd, args);
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

async function on<T>(event: string, cb: (payload: T) => void): Promise<() => void> {
  if (!inTauri) return mock.listen(event, cb as (p: unknown) => void);
  const { listen } = await import("@tauri-apps/api/event");
  return listen<T>(event, (e) => cb(e.payload));
}

export const api = {
  status: () => call<HostStatus>("get_status"),
  devices: () => call<DeviceInfo[]>("list_devices"),
  revoke: (deviceId: string) => call<boolean>("revoke_device", { deviceId }),
  renameDevice: (deviceId: string, name: string) => call<void>("rename_device", { deviceId, name }),
  assignProfile: (deviceId: string, profileId: string | null) => call<void>("assign_profile", { deviceId, profileId }),
  startPairing: () => call<PairingOffer>("start_pairing"),
  profiles: () => call<ProfileSummary[]>("list_profiles"),
  profile: (id?: string) => call<ProfileView>("get_profile", { id: id ?? null }),
  edit: (id: string, edits: Edit[]) => call<ProfileView>("edit_profile", { id, edits }),
  createProfile: (name: string, copyOf?: string) => call<string>("create_profile", { name, copyOf: copyOf ?? null }),
  deleteProfile: (id: string) => call<void>("delete_profile", { id }),
  undo: () => call<ProfileView>("undo_edit"),
  redo: () => call<ProfileView>("redo_edit"),
  /** Runs the action now; resolves to a warning note, or null when it went fine. */
  testAction: (action: ActionSpec) => call<string | null>("test_action", { action }),
  setPaused: (paused: boolean) => call<void>("set_paused", { paused }),
  autoSwitch: () => call<AutoSwitch>("get_auto_switch"),
  setAutoSwitch: (auto: AutoSwitch) => call<void>("set_auto_switch", { auto }),
  recentApps: () => call<string[]>("recent_apps"),
  autostart: () => call<boolean>("get_autostart"),
  setAutostart: (enabled: boolean) => call<void>("set_autostart", { enabled }),
  openProfileFile: (id?: string) => call<void>("open_profile_file", { id: id ?? null }),
  openDataFolder: () => call<void>("open_data_folder"),
  onHostEvent: (cb: (e: HostEvent) => void) => on<HostEvent>("host-event", cb),
  onNavigate: (cb: (v: View) => void) => on<View>("navigate", cb),
};
