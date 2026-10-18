//! The PerritoDeck desktop app: runs the host in the tray and gives it a window.
//!
//! The window is a view onto [`perritodeck_host::Host`]; closing it leaves the host
//! running in the tray so the tablets keep working.

use std::{
    ffi::OsString,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use perritodeck_host::{
    logging, updates, ActionSpec, AppSettings, AutoSwitch, DeviceInfo, Edit, Host, HostEvent,
    HostOptions, HostStatus, McpSettings, McpView, Network, NetworkHint, OnlineEntry, PairingOffer,
    PluginsView, ProfileSummary, ProfileView, SdPluginsView,
};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, Wry,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_opener::OpenerExt;
use tracing::{error, info};

/// Passed when the app starts at login, so it goes straight to the tray.
const HIDDEN_ARG: &str = "--hidden";

/// The running host, or why it couldn't start (shown in the window).
struct App {
    host: Result<Arc<Host>, String>,
    pause_item: Mutex<Option<CheckMenuItem<Wry>>>,
    /// Set by "Restart now": start the app again once it has exited.
    relaunch: AtomicBool,
}

impl App {
    fn host(&self) -> Result<&Arc<Host>, String> {
        self.host.as_ref().map_err(Clone::clone)
    }
}

type Res<T> = Result<T, String>;

/// Run a host call that can take seconds (stopping a plugin waits for its process to
/// end) on a thread meant for blocking. A plain command runs on the UI thread, so the
/// window would freeze until it returned.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Res<T> + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_status(app: State<'_, App>) -> Res<HostStatus> {
    Ok(app.host()?.status())
}

/// Whether the firewall may be keeping tablets on Wi-Fi out (asks Windows about the
/// network, which takes a second).
#[tauri::command]
async fn network_hint(app: State<'_, App>) -> Res<Option<NetworkHint>> {
    let host = app.host()?.clone();
    blocking(move || Ok(host.network_hint())).await
}

#[tauri::command]
fn list_devices(app: State<'_, App>) -> Res<Vec<DeviceInfo>> {
    Ok(app.host()?.devices())
}

#[tauri::command]
fn revoke_device(app: State<'_, App>, device_id: String) -> Res<bool> {
    Ok(app.host()?.revoke(&device_id))
}

#[tauri::command]
fn rename_device(app: State<'_, App>, device_id: String, name: String) -> Res<()> {
    app.host()?.rename_device(&device_id, &name)
}

#[tauri::command]
fn assign_profile(app: State<'_, App>, device_id: String, profile_id: Option<String>) -> Res<()> {
    app.host()?
        .assign_profile(&device_id, profile_id.as_deref())
}

#[tauri::command]
fn start_pairing(app: State<'_, App>) -> Res<PairingOffer> {
    app.host()?.new_pairing_offer().ok_or_else(|| {
        "Pairing is off because the host is running in insecure development mode.".to_owned()
    })
}

#[tauri::command]
fn list_profiles(app: State<'_, App>) -> Res<Vec<ProfileSummary>> {
    Ok(app.host()?.profiles())
}

/// Off the UI thread: the editor asks again whenever a Stream Deck plugin redraws a key
/// (up to a few times a second), and each answer reads and encodes the key images.
#[tauri::command]
async fn get_profile(app: State<'_, App>, id: Option<String>) -> Res<ProfileView> {
    let host = app.host()?.clone();
    blocking(move || host.profile_view(id.as_deref())).await
}

/// Apply editor changes to a profile file; tablets update straight away.
#[tauri::command]
fn edit_profile(app: State<'_, App>, id: String, edits: Vec<Edit>) -> Res<ProfileView> {
    app.host()?.edit_profile(&id, &edits)
}

#[tauri::command]
fn create_profile(
    app: State<'_, App>,
    name: String,
    copy_of: Option<String>,
    companion: Option<bool>,
) -> Res<String> {
    let host = app.host()?;
    if companion == Some(true) {
        host.create_companion_profile(&name)
    } else {
        host.create_profile(&name, copy_of.as_deref())
    }
}

#[tauri::command]
fn delete_profile(app: State<'_, App>, id: String) -> Res<()> {
    app.host()?.delete_profile(&id)
}

#[tauri::command]
fn undo_edit(app: State<'_, App>) -> Res<ProfileView> {
    app.host()?.undo()
}

#[tauri::command]
fn redo_edit(app: State<'_, App>) -> Res<ProfileView> {
    app.host()?.redo()
}

/// Run an action now, from the editor's "Try it" button.
#[tauri::command]
async fn test_action(app: State<'_, App>, action: ActionSpec) -> Res<Option<String>> {
    let host = app.host()?.clone();
    host.test_action(&action).await
}

#[tauri::command]
fn set_paused(app: State<'_, App>, paused: bool) -> Res<()> {
    app.host()?.set_paused(paused);
    if let Some(item) = app.pause_item.lock().expect("pause item").as_ref() {
        let _ = item.set_checked(paused);
    }
    Ok(())
}

#[tauri::command]
fn get_auto_switch(app: State<'_, App>) -> Res<AutoSwitch> {
    Ok(app.host()?.auto_switch())
}

#[tauri::command]
fn set_auto_switch(app: State<'_, App>, auto: AutoSwitch) -> Res<()> {
    app.host()?.set_auto_switch(auto)
}

#[tauri::command]
fn recent_apps(app: State<'_, App>) -> Res<Vec<String>> {
    Ok(app.host()?.recent_apps())
}

#[tauri::command]
fn get_autostart(handle: AppHandle) -> Res<bool> {
    handle.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_autostart(handle: AppHandle, app: State<'_, App>, enabled: bool) -> Res<()> {
    let launcher = handle.autolaunch();
    if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    }
    .map_err(|e| e.to_string())?;
    app.host()?.set_start_at_login(enabled)
}

/// Keep "Start at login" as the person chose it in PerritoDeck. On Windows an update runs
/// the old version's uninstaller, which removes the login entry: put it back. If the
/// entry is there but turned off in Task Manager's Startup tab, that's the person's
/// choice too, so PerritoDeck follows it.
fn keep_autostart(handle: &AppHandle, host: &Host) {
    let launcher = handle.autolaunch();
    let Ok(on) = launcher.is_enabled() else {
        return;
    };
    let wanted = host.start_at_login();
    if on && !wanted {
        // Turned on before PerritoDeck remembered it.
        let _ = host.set_start_at_login(true);
    }
    #[cfg(windows)]
    if !on && wanted {
        let name = &handle.package_info().name;
        let entry = windows_registry::CURRENT_USER
            .open(r"Software\Microsoft\Windows\CurrentVersion\Run")
            .and_then(|key| key.get_string(name));
        if entry.is_ok() {
            let _ = host.set_start_at_login(false);
        } else if let Err(e) = launcher.enable() {
            tracing::warn!("couldn't put \"Start at login\" back: {e}");
        } else {
            info!("put \"Start at login\" back (an update or reinstall had removed it)");
        }
    }
}

#[tauri::command]
fn get_network(app: State<'_, App>) -> Res<Network> {
    Ok(app.host()?.network())
}

/// Saved for the next start; the window offers to restart.
#[tauri::command]
fn set_network(app: State<'_, App>, network: Network) -> Res<()> {
    app.host()?.set_network(network)
}

#[tauri::command]
fn get_check_updates(app: State<'_, App>) -> Res<bool> {
    Ok(app.host()?.check_updates_enabled())
}

#[tauri::command]
fn set_check_updates(app: State<'_, App>, enabled: bool) -> Res<()> {
    app.host()?.set_check_updates(enabled)
}

/// `None` when this is the latest version.
#[tauri::command]
async fn check_for_updates() -> Res<Option<updates::Update>> {
    blocking(|| updates::check(env!("CARGO_PKG_VERSION"))).await
}

/// Open a release page from [`check_for_updates`]; nothing else.
#[tauri::command]
fn open_release_page(handle: AppHandle, url: String) -> Res<()> {
    if !url.starts_with(updates::RELEASES) {
        return Err("Not a PerritoDeck release page.".into());
    }
    handle
        .opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

/// The licence texts online, for a build that doesn't carry the file (`tauri dev`, a
/// local `tauri build`).
const LICENCES_ONLINE: &str =
    "https://github.com/perritodeck/perritodeck/blob/main/THIRD_PARTY_NOTICES.md";
/// The licence texts' name in the resource folder (release.yml's `bundle.resources`).
/// host/app/tests/licences.spec.ts reads this line to check the two agree.
const LICENCES_FILE: &str = "third-party-licenses.txt";

/// Settings → "Open-source licences": `third-party-licenses.txt`, which release.yml puts
/// in the app's resource folder, or the notices on GitHub when this build doesn't have it.
/// The window adds "Couldn't open the licences" and the link to any error.
#[tauri::command]
fn open_licences(handle: AppHandle) -> Res<()> {
    let file = handle
        .path()
        .resource_dir()
        .ok()
        .map(|d| d.join(LICENCES_FILE))
        .filter(|f| f.is_file());
    let opened = match file {
        Some(file) => handle
            .opener()
            .open_path(file.to_string_lossy(), None::<&str>),
        None => handle.opener().open_url(LICENCES_ONLINE, None::<&str>),
    };
    opened.map_err(|e| e.to_string())
}

/// "Restart now", after a change that needs it (the port, the network). Quits like
/// Quit does; the exit handler in [`run`] then starts the app again.
#[tauri::command]
fn restart_app(app: State<'_, App>, handle: AppHandle) {
    app.relaunch.store(true, Ordering::Relaxed);
    handle.exit(0);
}

/// Start the app again, as `AppHandle::request_restart` does, but without
/// [`HIDDEN_ARG`]: the person just pressed "Restart now" in the window, so it should
/// come back even when this run was started at login. Call it while exiting, once the
/// plugins have stopped and the single-instance lock is gone.
fn relaunch(handle: &AppHandle) -> ! {
    handle.cleanup_before_exit();
    let mut env = handle.env();
    env.args_os = relaunch_args(env.args_os);
    tauri::process::restart(&env)
}

/// This run's arguments without [`HIDDEN_ARG`].
fn relaunch_args(mut args: Vec<OsString>) -> Vec<OsString> {
    args.retain(|a| a != HIDDEN_ARG);
    args
}

/// Let the person pick an image for a key; it's copied next to the profiles.
/// `None` if they cancelled.
#[tauri::command]
async fn choose_icon(handle: AppHandle, app: State<'_, App>) -> Res<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let host = app.host()?.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        handle
            .dialog()
            .file()
            .set_title("Choose an image for the key")
            .add_filter("Images", &["png", "jpg", "jpeg", "gif"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(file) = picked else { return Ok(None) };
    let path = file.into_path().map_err(|e| e.to_string())?;
    host.import_icon(&path).map(Some)
}

/// Save a built-in icon (drawn by the window as a PNG, base64) for use on a key.
#[tauri::command]
fn import_icon_data(app: State<'_, App>, png_base64: String) -> Res<String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png_base64.trim())
        .map_err(|_| "That image data is damaged.".to_owned())?;
    app.host()?.import_icon_bytes(&bytes)
}

/// Save a profile as a `.perritoprofile` file. False if the person cancelled.
#[tauri::command]
async fn export_profile(handle: AppHandle, app: State<'_, App>, id: String) -> Res<bool> {
    use tauri_plugin_dialog::DialogExt;
    let host = app.host()?.clone();
    let name = host.profile_view(Some(&id))?.name;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        handle
            .dialog()
            .file()
            .set_title("Export profile")
            .set_file_name(format!("{name}.perritoprofile"))
            .add_filter("PerritoDeck profile", &["perritoprofile"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(file) = picked else { return Ok(false) };
    let path = file.into_path().map_err(|e| e.to_string())?;
    host.export_profile(&id, &path).map(|()| true)
}

/// Add a profile from a `.perritoprofile` file. `None` if the person cancelled.
#[tauri::command]
async fn import_profile(handle: AppHandle, app: State<'_, App>) -> Res<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let host = app.host()?.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        handle
            .dialog()
            .file()
            .set_title("Import a profile")
            .add_filter("PerritoDeck profile", &["perritoprofile"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(file) = picked else { return Ok(None) };
    let path = file.into_path().map_err(|e| e.to_string())?;
    host.import_profile(&path).map(Some)
}

#[tauri::command]
fn list_plugins(app: State<'_, App>) -> Res<PluginsView> {
    Ok(app.host()?.plugins())
}

#[tauri::command]
async fn install_catalog_plugin(app: State<'_, App>, id: String) -> Res<String> {
    let host = app.host()?.clone();
    blocking(move || host.install_from_catalog(&id)).await
}

#[tauri::command]
fn get_plugin_settings(app: State<'_, App>, id: String) -> Res<serde_json::Value> {
    app.host()?.plugin_settings(&id)
}

#[tauri::command]
fn set_plugin_settings(app: State<'_, App>, id: String, settings: serde_json::Value) -> Res<()> {
    app.host()?.set_plugin_settings(&id, settings)
}

#[tauri::command]
fn list_sd_plugins(app: State<'_, App>) -> Res<SdPluginsView> {
    Ok(app.host()?.sd_plugins())
}

/// Install a Stream Deck plugin the person picks: a `.streamDeckPlugin` file, or (with
/// `folder`) a `.sdPlugin` folder. `None` if they cancelled.
#[tauri::command]
async fn install_sd_plugin(
    handle: AppHandle,
    app: State<'_, App>,
    folder: bool,
) -> Res<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let host = app.host()?.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let dialog = handle.dialog().file();
        if folder {
            dialog
                .set_title("Choose the plugin's .sdPlugin folder")
                .blocking_pick_folder()
        } else {
            dialog
                .set_title("Choose a Stream Deck plugin")
                .add_filter("Stream Deck plugin", &["streamDeckPlugin"])
                .blocking_pick_file()
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    blocking(move || host.install_sd_plugin(&path).map(Some)).await
}

#[tauri::command]
fn sd_settings_page(
    app: State<'_, App>,
    profile: String,
    page: String,
    row: u32,
    col: u32,
) -> Res<String> {
    app.host()?.sd_settings_page(&profile, &page, row, col)
}

#[tauri::command]
async fn remove_sd_plugin(app: State<'_, App>, uuid: String) -> Res<()> {
    let host = app.host()?.clone();
    blocking(move || host.remove_sd_plugin(&uuid)).await
}

#[tauri::command]
async fn restart_sd_plugin(app: State<'_, App>, uuid: String) -> Res<()> {
    let host = app.host()?.clone();
    blocking(move || host.restart_sd_plugin(&uuid)).await
}

/// What the online plugin catalogue offers (asks the network only when called).
#[tauri::command]
async fn list_online_plugins(app: State<'_, App>) -> Res<Vec<OnlineEntry>> {
    let host = app.host()?.clone();
    blocking(move || host.online_plugins()).await
}

/// Download (hash-checked) and install or update a plugin from the online catalogue.
#[tauri::command]
async fn install_online_plugin(app: State<'_, App>, id: String) -> Res<String> {
    let host = app.host()?.clone();
    blocking(move || host.install_online_plugin(&id)).await
}

/// Install a plugin from a folder the person picks. `None` if they cancelled.
#[tauri::command]
async fn install_plugin_folder(handle: AppHandle, app: State<'_, App>) -> Res<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let host = app.host()?.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        handle
            .dialog()
            .file()
            .set_title("Choose the plugin's folder")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(folder) = picked else {
        return Ok(None);
    };
    let path = folder.into_path().map_err(|e| e.to_string())?;
    blocking(move || host.install_plugin(&path).map(Some)).await
}

#[tauri::command]
async fn remove_plugin(app: State<'_, App>, id: String) -> Res<()> {
    let host = app.host()?.clone();
    blocking(move || host.remove_plugin(&id)).await
}

#[tauri::command]
async fn restart_plugin(app: State<'_, App>, id: String) -> Res<()> {
    let host = app.host()?.clone();
    blocking(move || host.restart_plugin(&id)).await
}

#[tauri::command]
fn open_plugins_folder(handle: AppHandle, app: State<'_, App>) -> Res<()> {
    let host = app.host()?;
    host.rescan_plugins();
    let dir = std::path::PathBuf::from(host.plugins().folder);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    handle
        .opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("Couldn't open {}: {e}", dir.display()))
}

/// macOS only: whether PerritoDeck may send key presses (System Settings → Privacy &
/// Security → Accessibility). `None` elsewhere.
#[tauri::command]
fn accessibility_allowed() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        // SAFETY: takes no arguments and only reads the process's permission.
        Some(unsafe { AXIsProcessTrusted() })
    }
    #[cfg(not(target_os = "macos"))]
    None
}

#[tauri::command]
fn open_accessibility_settings(handle: AppHandle) -> Res<()> {
    handle
        .opener()
        .open_url(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            None::<&str>,
        )
        .map_err(|e| e.to_string())
}

/// Open a profile in the user's text editor; `None` for the default.
#[tauri::command]
fn open_profile_file(handle: AppHandle, app: State<'_, App>, id: Option<String>) -> Res<()> {
    let host = app.host()?;
    let path = match id {
        Some(id) => host.profiles_dir.join(format!("{id}.toml")),
        None => host.profile_path.clone(),
    };
    handle
        .opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("Couldn't open {}: {e}", path.display()))
}

/// Show the log files' folder in the file manager.
#[tauri::command]
fn open_logs_folder(handle: AppHandle, app: State<'_, App>) -> Res<()> {
    let dir = match app.host() {
        Ok(h) => h.logs_dir(),
        Err(_) => logging::logs_dir(&data_dir()?),
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    handle
        .opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("Couldn't open {}: {e}", dir.display()))
}

/// The report for "Copy diagnostics" (the window copies it to the clipboard). It reads
/// the log files and, on Windows, asks PowerShell about the network, which can take a
/// few seconds, so it runs off the UI thread.
#[tauri::command]
async fn get_diagnostics(app: State<'_, App>) -> Res<String> {
    let version = env!("CARGO_PKG_VERSION");
    let host = app.host().cloned();
    blocking(move || {
        Ok(match host {
            Ok(h) => h.diagnostics(Some(version)),
            // Even when the host didn't start, the files and log can say why.
            Err(e) => {
                let dir = data_dir()?;
                let mut report = perritodeck_host::diagnostics::offline(
                    &dir,
                    &perritodeck_host::default_profile_path(&dir),
                );
                report.app_version = Some(version.to_owned());
                let mut why =
                    perritodeck_host::diagnostics::Section::new("PerritoDeck couldn't start");
                why.line(e);
                report.sections.insert(0, why);
                report.render()
            }
        })
    })
    .await
}

#[tauri::command]
fn get_detailed_logs(app: State<'_, App>) -> Res<bool> {
    Ok(app.host()?.detailed_logs())
}

#[tauri::command]
fn set_detailed_logs(app: State<'_, App>, enabled: bool) -> Res<()> {
    app.host()?.set_detailed_logs(enabled)
}

#[tauri::command]
fn get_mcp(app: State<'_, App>) -> Res<McpView> {
    Ok(app.host()?.mcp_view())
}

/// Save the AI assistant settings; the endpoint starts, restarts or stops to match.
#[tauri::command]
async fn set_mcp(app: State<'_, App>, mcp: McpSettings) -> Res<McpView> {
    let host = app.host()?.clone();
    host.set_mcp(mcp).await
}

#[tauri::command]
async fn new_mcp_token(app: State<'_, App>) -> Res<McpView> {
    let host = app.host()?.clone();
    host.new_mcp_token().await
}

/// Show the settings folder in the file manager.
#[tauri::command]
fn open_data_folder(handle: AppHandle, app: State<'_, App>) -> Res<()> {
    let dir = app.host()?.data_dir.clone();
    handle
        .opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("Couldn't open {}: {e}", dir.display()))
}

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// The host's data folder: `PERRITODECK_DATA_DIR`, or the usual per-user one.
fn data_dir() -> Result<std::path::PathBuf, String> {
    match std::env::var_os("PERRITODECK_DATA_DIR") {
        Some(d) => Ok(d.into()),
        None => perritodeck_host::default_data_dir().map_err(|e| e.to_string()),
    }
}

fn start_host(app: &tauri::App) -> Result<Host, String> {
    let mut opts = HostOptions::new(data_dir()?).with_saved_network();
    opts.profile = std::env::var_os("PERRITODECK_PROFILE").map(Into::into);
    // The bridge assistants start ships next to the app; the host copies it into the
    // data folder so they don't keep the app's own program in use.
    opts.mcp_bridge = perritodeck_host::mcp::shipped_bridge();
    // First-party plugins ship inside the app, ready to add from the Plugins page.
    opts.plugin_catalog = app
        .path()
        .resource_dir()
        .ok()
        .map(|d| d.join("plugins"))
        .filter(|d| d.is_dir());
    // After "Restart now" the old process may still hold the port for a moment.
    let mut tries = 0;
    loop {
        match tauri::async_runtime::block_on(Host::start(opts.clone())) {
            Ok(host) => return Ok(host),
            Err(e) if tries < 10 => {
                tries += 1;
                info!("host didn't start yet ({e:#}); trying again");
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            Err(e) => return Err(format!("{e:#}")),
        }
    }
}

fn build_tray(app: &tauri::App, paused: bool) -> tauri::Result<CheckMenuItem<Wry>> {
    // `&` marks each item's access key (Windows and Linux; macOS leaves it out).
    let open = MenuItem::with_id(app, "open", "&Open PerritoDeck", true, None::<&str>)?;
    let pair = MenuItem::with_id(app, "pair", "P&air a tablet…", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Pause &keys", true, paused, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "&Quit PerritoDeck", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &pair, &pause, &sep, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("PerritoDeck")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "pair" => {
                show_window(app);
                let _ = app.emit("navigate", "pair");
            }
            "pause" => {
                let state = app.state::<App>();
                if let Ok(host) = state.host() {
                    let now = !host.paused();
                    host.set_paused(now);
                    if let Some(item) = state.pause_item.lock().expect("pause item").as_ref() {
                        let _ = item.set_checked(now);
                    }
                }
            }
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(pause)
}

/// Forward host events to the window, and keep the tray tooltip current.
fn forward_events(handle: AppHandle, host: &Host) {
    let mut events = host.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if matches!(
                        event,
                        HostEvent::Connected { .. }
                            | HostEvent::Disconnected { .. }
                            | HostEvent::ProfileChanged
                    ) {
                        update_tooltip(&handle);
                    }
                    let _ = handle.emit("host-event", &event);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
}

fn update_tooltip(handle: &AppHandle) {
    let Some(app) = handle.try_state::<App>() else {
        return;
    };
    let Ok(host) = app.host() else { return };
    let n = host.devices().iter().filter(|d| d.connected).count();
    let mut text = match n {
        0 => "PerritoDeck — no tablets connected".to_owned(),
        1 => "PerritoDeck — 1 tablet connected".to_owned(),
        n => format!("PerritoDeck — {n} tablets connected"),
    };
    if host.paused() {
        text.push_str(" (keys paused)");
    }
    if let Some(tray) = handle.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(text));
    }
}

/// `perritodeck-app mcp [--data-dir <folder>] [--config]`: the app doubles as the stdio
/// bridge for AI assistants. That was the setup before `perritodeck-mcp` (which the app
/// ships and copies into the data folder); it keeps working for assistants set up then,
/// and Settings asks to set them up again, because while one runs the app's program is
/// in use and can't be updated. Handled before Tauri starts, so it never opens a window
/// or takes the single-instance lock. Returns the exit code.
pub fn run_mcp_bridge(args: &[String]) -> i32 {
    let mut config = false;
    let mut dir = None;
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--config" => config = true,
            "--data-dir" => dir = rest.next().map(std::path::PathBuf::from),
            other => {
                eprintln!("perritodeck-app mcp: unexpected argument {other:?}");
                return 2;
            }
        }
    }
    let dir = match dir.map(Ok).unwrap_or_else(data_dir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("perritodeck-app mcp: {e}");
            return 1;
        }
    };
    // stdout carries the protocol: problems go to stderr only.
    let mut log = logging::LogOptions::new(None);
    log.console = logging::Console::Stderr;
    log.level = "warn";
    logging::init(log);
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("perritodeck-app mcp: {e}");
            return 1;
        }
    };
    let agent = concat!("perritodeck-app/", env!("CARGO_PKG_VERSION"));
    match runtime.block_on(perritodeck_host::mcp::run_command(&dir, config, agent)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("perritodeck-app mcp: {e}");
            1
        }
    }
}

/// Log to the console and to files in the data folder's `logs` folder.
fn init_logging() {
    let logs = data_dir().ok();
    let mut log = logging::LogOptions::new(logs.as_deref().map(logging::logs_dir));
    log.detailed = logs
        .as_deref()
        .is_some_and(|d| AppSettings::load(&perritodeck_host::settings_path(d)).detailed_logs);
    if let Some(file) = logging::init(log) {
        info!(
            "PerritoDeck {} logging to {}",
            env!("CARGO_PKG_VERSION"),
            file.display()
        );
    }
}

pub fn run() {
    let result = tauri::Builder::default()
        // Must come first: a second launch focuses this one instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            info!("PerritoDeck was opened again: showing the window");
            show_window(app)
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_ARG]),
        ))
        .setup(|app| {
            // Not before: a second launch exits while the app is built (single-instance,
            // above), and would otherwise write a startup line into this one's log, or
            // even start a new file under it.
            init_logging();
            let host = start_host(app).map(Arc::new);
            match &host {
                Ok(h) => {
                    info!("host running on {}", h.addr);
                    tauri::async_runtime::block_on(h.apply_mcp());
                    keep_autostart(app.handle(), h);
                    forward_events(app.handle().clone(), h);
                }
                Err(e) => error!("host didn't start: {e}"),
            }
            let paused = host.as_ref().is_ok_and(|h| h.paused());
            app.manage(App {
                host,
                pause_item: Mutex::new(None),
                relaunch: AtomicBool::new(false),
            });
            let pause = build_tray(app, paused)?;
            *app.state::<App>().pause_item.lock().expect("pause item") = Some(pause);
            update_tooltip(app.handle());
            // Started at login: stay in the tray. Otherwise show the window.
            if !std::env::args().any(|a| a == HIDDEN_ARG) {
                show_window(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps the host running in the tray.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            network_hint,
            list_devices,
            revoke_device,
            rename_device,
            assign_profile,
            start_pairing,
            list_profiles,
            get_profile,
            edit_profile,
            create_profile,
            delete_profile,
            undo_edit,
            redo_edit,
            test_action,
            set_paused,
            get_auto_switch,
            set_auto_switch,
            recent_apps,
            get_autostart,
            set_autostart,
            open_profile_file,
            open_data_folder,
            open_logs_folder,
            get_diagnostics,
            get_detailed_logs,
            set_detailed_logs,
            get_mcp,
            set_mcp,
            new_mcp_token,
            choose_icon,
            import_icon_data,
            list_plugins,
            install_catalog_plugin,
            install_plugin_folder,
            remove_plugin,
            restart_plugin,
            open_plugins_folder,
            export_profile,
            import_profile,
            accessibility_allowed,
            open_accessibility_settings,
            get_network,
            set_network,
            get_check_updates,
            set_check_updates,
            check_for_updates,
            restart_app,
            open_release_page,
            open_licences,
            list_online_plugins,
            install_online_plugin,
            list_sd_plugins,
            get_plugin_settings,
            set_plugin_settings,
            install_sd_plugin,
            remove_sd_plugin,
            sd_settings_page,
            restart_sd_plugin,
        ])
        .build(tauri::generate_context!())
        .map(|app| {
            app.run(|handle, event| {
                // Quit (`app.exit`) ends the process right after this: stop the plugins
                // first, so they and the programs they started don't outlive the app.
                // Plugins' exit handlers (the single-instance lock) have run already.
                if let tauri::RunEvent::Exit = event {
                    let Some(state) = handle.try_state::<App>() else {
                        return;
                    };
                    if let Ok(host) = state.host() {
                        info!("stopping plugins before exit");
                        host.stop_plugins();
                    }
                    if state.relaunch.load(Ordering::Relaxed) {
                        relaunch(handle);
                    }
                }
            })
        });
    if let Err(e) = result {
        // Building failed, so setup never ran to start the log.
        init_logging();
        error!("PerritoDeck couldn't start: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_now_drops_the_started_at_login_flag() {
        let args = |a: &[&str]| a.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            relaunch_args(args(&["perritodeck-app", HIDDEN_ARG])),
            args(&["perritodeck-app"])
        );
        assert_eq!(
            relaunch_args(args(&["perritodeck-app", "--other"])),
            args(&["perritodeck-app", "--other"])
        );
    }
}
