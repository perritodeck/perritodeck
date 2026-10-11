// Talks to the Rust side through Tauri. Opened in a plain browser (`npm run dev`,
// Playwright), it uses the in-memory mock instead so the UI can be worked on alone.

import type { ActionSpec, AutoSwitch, DeviceInfo, Edit, Network, PluginsView, Update, ProfileSummary, HostEvent, HostStatus, PairingOffer, ProfileView, View } from "./types";
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
  network: () => call<Network>("get_network"),
  setNetwork: (network: Network) => call<void>("set_network", { network }),
  checkUpdatesEnabled: () => call<boolean>("get_check_updates"),
  setCheckUpdates: (enabled: boolean) => call<void>("set_check_updates", { enabled }),
  checkForUpdates: () => call<Update | null>("check_for_updates"),
  restart: () => call<void>("restart_app"),
  openReleasePage: (url: string) => call<void>("open_release_page", { url }),
  openProfileFile: (id?: string) => call<void>("open_profile_file", { id: id ?? null }),
  openDataFolder: () => call<void>("open_data_folder"),
  plugins: () => call<PluginsView>("list_plugins"),
  installCatalogPlugin: (id: string) => call<string>("install_catalog_plugin", { id }),
  /** Pick a plugin folder; resolves to the plugin's id, or null if cancelled. */
  installPluginFolder: () => call<string | null>("install_plugin_folder"),
  removePlugin: (id: string) => call<void>("remove_plugin", { id }),
  restartPlugin: (id: string) => call<void>("restart_plugin", { id }),
  openPluginsFolder: () => call<void>("open_plugins_folder"),
  /** Pick an image file; resolves to its path for the key, or null if cancelled. */
  chooseIcon: () => call<string | null>("choose_icon"),
  /** Save a PNG (base64) as a key image; resolves to its path for the key. */
  importIconData: (pngBase64: string) => call<string>("import_icon_data", { pngBase64 }),
  /** Save a profile to a file the person picks; false if they cancelled. */
  exportProfile: (id: string) => call<boolean>("export_profile", { id }),
  /** Add a profile from a file; resolves to its id, or null if cancelled. */
  importProfile: () => call<string | null>("import_profile"),
  /** macOS: may PerritoDeck send key presses? null on other systems. */
  accessibilityAllowed: () => call<boolean | null>("accessibility_allowed"),
  openAccessibilitySettings: () => call<void>("open_accessibility_settings"),
  onHostEvent: (cb: (e: HostEvent) => void) => on<HostEvent>("host-event", cb),
  onNavigate: (cb: (v: View) => void) => on<View>("navigate", cb),
};
