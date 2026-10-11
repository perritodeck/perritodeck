// Talks to the Rust side through Tauri. Opened in a plain browser (`npm run dev`,
// Playwright), it uses the in-memory mock instead so the UI can be worked on alone.

import type { DeviceInfo, HostEvent, HostStatus, PairingOffer, ProfileView, View } from "./types";
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
  startPairing: () => call<PairingOffer>("start_pairing"),
  profile: () => call<ProfileView>("get_profile"),
  openProfileFile: () => call<void>("open_profile_file"),
  openDataFolder: () => call<void>("open_data_folder"),
  onHostEvent: (cb: (e: HostEvent) => void) => on<HostEvent>("host-event", cb),
  onNavigate: (cb: (v: View) => void) => on<View>("navigate", cb),
};
