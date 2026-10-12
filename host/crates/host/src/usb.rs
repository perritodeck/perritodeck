//! Tablets on a USB cable: when Android's `adb` tool is installed, forward the host's
//! port to every connected tablet (`adb reverse`), so Perrito can reach this computer
//! at 127.0.0.1 without Wi-Fi.

use std::{
    collections::BTreeSet,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use tracing::{debug, info};

/// Serial numbers of tablets currently forwarded.
pub type UsbDevices = Arc<Mutex<BTreeSet<String>>>;

/// `adb` from PATH, or from the usual Android SDK locations.
pub fn find_adb() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "adb.exe" } else { "adb" };
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(exe)).collect())
        .unwrap_or_default();
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(sdk) = std::env::var_os(var) {
            candidates.push(PathBuf::from(sdk).join("platform-tools").join(exe));
        }
    }
    if let Some(home) = directories::BaseDirs::new() {
        let h = home.home_dir();
        candidates.push(h.join("AppData/Local/Android/Sdk/platform-tools").join(exe));
        candidates.push(h.join("Library/Android/sdk/platform-tools").join(exe));
        candidates.push(h.join("Android/Sdk/platform-tools").join(exe));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Parse `adb devices` output: serials of devices that are ready.
pub fn ready_devices(output: &str) -> BTreeSet<String> {
    output
        .lines()
        .skip(1)
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let (serial, state) = (parts.next()?, parts.next()?);
            (state == "device").then(|| serial.to_owned())
        })
        .collect()
}

/// An `adb` command that never opens a console window. The installed app is a GUI
/// process with no console, so without `CREATE_NO_WINDOW` Windows would open a new
/// console (a Terminal window on Windows 11) for every poll.
fn adb_command(adb: &std::path::Path) -> Command {
    let mut c = Command::new(adb);
    c.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

pub fn spawn(port: u16, devices: UsbDevices) {
    let Some(adb) = find_adb() else {
        debug!("adb not found; USB tablets need Wi-Fi");
        return;
    };
    info!(
        "adb found at {}; USB tablets will be connected automatically",
        adb.display()
    );
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(4));
        loop {
            every.tick().await;
            let adb = adb.clone();
            let known = devices.lock().expect("usb poisoned").clone();
            let Ok(now) = tokio::task::spawn_blocking(move || {
                let out = adb_command(&adb).arg("devices").output().ok()?;
                let ready = ready_devices(&String::from_utf8_lossy(&out.stdout));
                for serial in ready.difference(&known) {
                    let spec = format!("tcp:{port}");
                    let ok = adb_command(&adb)
                        .args(["-s", serial, "reverse", &spec, &spec])
                        .output()
                        .is_ok_and(|o| o.status.success());
                    if ok {
                        info!("USB tablet {serial} connected (adb reverse {spec})");
                    }
                }
                Some(ready)
            })
            .await
            else {
                continue;
            };
            if let Some(ready) = now {
                *devices.lock().expect("usb poisoned") = ready;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_adb_devices() {
        let out = "List of devices attached\nG000L40XXXXX\tdevice\nemulator-5554\toffline\nABC\tunauthorized\n\n";
        assert_eq!(
            ready_devices(out).into_iter().collect::<Vec<_>>(),
            ["G000L40XXXXX"]
        );
        assert!(ready_devices("List of devices attached\n").is_empty());
    }
}
