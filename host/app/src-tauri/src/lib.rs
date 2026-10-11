//! The PerritoDeck desktop app: runs the host in the tray and gives it a window.
//!
//! The window is a view onto [`perritodeck_host::Host`]; closing it leaves the host
//! running in the tray so the tablets keep working.

use std::sync::Arc;

use perritodeck_host::{
    DeviceInfo, Edit, Host, HostEvent, HostOptions, HostStatus, PairingOffer, ProfileView,
};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WindowEvent,
};
use tauri_plugin_opener::OpenerExt;
use tracing::{error, info};

/// The running host, or why it couldn't start (shown in the window).
struct App {
    host: Result<Arc<Host>, String>,
}

impl App {
    fn host(&self) -> Result<&Arc<Host>, String> {
        self.host.as_ref().map_err(Clone::clone)
    }
}

#[tauri::command]
fn get_status(app: State<'_, App>) -> Result<HostStatus, String> {
    Ok(app.host()?.status())
}

#[tauri::command]
fn list_devices(app: State<'_, App>) -> Result<Vec<DeviceInfo>, String> {
    Ok(app.host()?.devices())
}

#[tauri::command]
fn revoke_device(app: State<'_, App>, device_id: String) -> Result<bool, String> {
    Ok(app.host()?.revoke(&device_id))
}

#[tauri::command]
fn start_pairing(app: State<'_, App>) -> Result<PairingOffer, String> {
    app.host()?.new_pairing_offer().ok_or_else(|| {
        "Pairing is off because the host is running in insecure development mode.".to_owned()
    })
}

#[tauri::command]
fn get_profile(app: State<'_, App>) -> Result<ProfileView, String> {
    Ok(app.host()?.profile_view())
}

/// Apply editor changes to the profile file; tablets update straight away.
#[tauri::command]
fn edit_profile(app: State<'_, App>, edits: Vec<Edit>) -> Result<ProfileView, String> {
    app.host()?.edit_profile(&edits)
}

#[tauri::command]
fn undo_edit(app: State<'_, App>) -> Result<ProfileView, String> {
    app.host()?.undo()
}

#[tauri::command]
fn redo_edit(app: State<'_, App>) -> Result<ProfileView, String> {
    app.host()?.redo()
}

/// Open the profile in the user's text editor.
#[tauri::command]
fn open_profile_file(handle: AppHandle, app: State<'_, App>) -> Result<(), String> {
    let path = app.host()?.profile_path.clone();
    handle
        .opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("Couldn't open {}: {e}", path.display()))
}

/// Show the settings folder in the file manager.
#[tauri::command]
fn open_data_folder(handle: AppHandle, app: State<'_, App>) -> Result<(), String> {
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

fn start_host() -> Result<Host, String> {
    let data_dir = match std::env::var_os("PERRITODECK_DATA_DIR") {
        Some(d) => d.into(),
        None => perritodeck_host::default_data_dir().map_err(|e| e.to_string())?,
    };
    let mut opts = HostOptions::new(data_dir);
    opts.profile = std::env::var_os("PERRITODECK_PROFILE").map(Into::into);
    tauri::async_runtime::block_on(Host::start(opts)).map_err(|e| format!("{e:#}"))
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open PerritoDeck", true, None::<&str>)?;
    let pair = MenuItem::with_id(app, "pair", "Pair a tablet…", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit PerritoDeck", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &pair, &sep, &quit])?;
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
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
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
                        HostEvent::Connected { .. } | HostEvent::Disconnected { .. }
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
    let text = match n {
        0 => "PerritoDeck — no tablets connected".to_owned(),
        1 => "PerritoDeck — 1 tablet connected".to_owned(),
        n => format!("PerritoDeck — {n} tablets connected"),
    };
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
        .setup(|app| {
            let host = start_host().map(Arc::new);
            match &host {
                Ok(h) => {
                    info!("host running on {}", h.addr);
                    forward_events(app.handle().clone(), h);
                }
                Err(e) => error!("host didn't start: {e}"),
            }
            app.manage(App { host });
            build_tray(app)?;
            update_tooltip(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps the host running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            list_devices,
            revoke_device,
            start_pairing,
            get_profile,
            edit_profile,
            undo_edit,
            redo_edit,
            open_profile_file,
            open_data_folder,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        error!("PerritoDeck couldn't start: {e}");
        std::process::exit(1);
    }
}
