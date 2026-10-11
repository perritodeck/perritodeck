//! The PerritoDeck desktop app: runs the host in the tray and gives it a window.
//!
//! The window is a view onto [`perritodeck_host::Host`]; closing it leaves the host
//! running in the tray so the tablets keep working.

use std::sync::{Arc, Mutex};

use perritodeck_host::{
    updates, ActionSpec, AutoSwitch, DeviceInfo, Edit, Host, HostEvent, HostOptions, HostStatus,
    Network, OnlineEntry, PairingOffer, PluginsView, ProfileSummary, ProfileView, SdPluginsView,
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
}

impl App {
    fn host(&self) -> Result<&Arc<Host>, String> {
        self.host.as_ref().map_err(Clone::clone)
    }
}

type Res<T> = Result<T, String>;

#[tauri::command]
fn get_status(app: State<'_, App>) -> Res<HostStatus> {
    Ok(app.host()?.status())
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

#[tauri::command]
fn get_profile(app: State<'_, App>, id: Option<String>) -> Res<ProfileView> {
    app.host()?.profile_view(id.as_deref())
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
fn set_autostart(handle: AppHandle, enabled: bool) -> Res<()> {
    let launcher = handle.autolaunch();
    if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    }
    .map_err(|e| e.to_string())
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
    tauri::async_runtime::spawn_blocking(|| updates::check(env!("CARGO_PKG_VERSION")))
        .await
        .map_err(|e| e.to_string())?
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

#[tauri::command]
fn restart_app(handle: AppHandle) {
    handle.request_restart();
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
fn install_catalog_plugin(app: State<'_, App>, id: String) -> Res<String> {
    app.host()?.install_from_catalog(&id)
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
    tauri::async_runtime::spawn_blocking(move || host.install_sd_plugin(&path).map(Some))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn remove_sd_plugin(app: State<'_, App>, uuid: String) -> Res<()> {
    app.host()?.remove_sd_plugin(&uuid)
}

#[tauri::command]
fn restart_sd_plugin(app: State<'_, App>, uuid: String) -> Res<()> {
    app.host()?.restart_sd_plugin(&uuid)
}

/// What the online plugin catalogue offers (asks the network only when called).
#[tauri::command]
async fn list_online_plugins(app: State<'_, App>) -> Res<Vec<OnlineEntry>> {
    let host = app.host()?.clone();
    tauri::async_runtime::spawn_blocking(move || host.online_plugins())
        .await
        .map_err(|e| e.to_string())?
}

/// Download (hash-checked) and install or update a plugin from the online catalogue.
#[tauri::command]
async fn install_online_plugin(app: State<'_, App>, id: String) -> Res<String> {
    let host = app.host()?.clone();
    tauri::async_runtime::spawn_blocking(move || host.install_online_plugin(&id))
        .await
        .map_err(|e| e.to_string())?
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
    host.install_plugin(&path).map(Some)
}

#[tauri::command]
fn remove_plugin(app: State<'_, App>, id: String) -> Res<()> {
    app.host()?.remove_plugin(&id)
}

#[tauri::command]
fn restart_plugin(app: State<'_, App>, id: String) -> Res<()> {
    app.host()?.restart_plugin(&id)
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

fn start_host(app: &tauri::App) -> Result<Host, String> {
    let data_dir = match std::env::var_os("PERRITODECK_DATA_DIR") {
        Some(d) => d.into(),
        None => perritodeck_host::default_data_dir().map_err(|e| e.to_string())?,
    };
    let mut opts = HostOptions::new(data_dir).with_saved_network();
    opts.profile = std::env::var_os("PERRITODECK_PROFILE").map(Into::into);
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
    let open = MenuItem::with_id(app, "open", "Open PerritoDeck", true, None::<&str>)?;
    let pair = MenuItem::with_id(app, "pair", "Pair a tablet…", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Pause keys", true, paused, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit PerritoDeck", true, None::<&str>)?;
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

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let result = tauri::Builder::default()
        // Must come first: a second launch focuses this one instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app)
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_ARG]),
        ))
        .setup(|app| {
            let host = start_host(app).map(Arc::new);
            match &host {
                Ok(h) => {
                    info!("host running on {}", h.addr);
                    forward_events(app.handle().clone(), h);
                }
                Err(e) => error!("host didn't start: {e}"),
            }
            let paused = host.as_ref().is_ok_and(|h| h.paused());
            app.manage(App {
                host,
                pause_item: Mutex::new(None),
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
            list_online_plugins,
            install_online_plugin,
            list_sd_plugins,
            install_sd_plugin,
            remove_sd_plugin,
            restart_sd_plugin,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        error!("PerritoDeck couldn't start: {e}");
        std::process::exit(1);
    }
}
