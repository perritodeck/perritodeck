//! Which app is in front, for switching profiles automatically.
//!
//! Windows: the foreground window's program. macOS: the owner of the frontmost normal
//! window. Linux: the active window's `WM_CLASS` on X11. Wayland has no general way to
//! ask, so auto-switching is unavailable there for now.

/// The app in front.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Foreground {
    /// A short, stable name for it: `obs64`, `OBS`, `firefox`. Compared case-insensitively.
    pub app: String,
    /// Its process, where the OS says (Windows), to notice the app quitting.
    pid: Option<u32>,
}

impl Foreground {
    /// Whether the app has quit since it was in front. False where we can't tell.
    pub fn has_quit(&self) -> bool {
        self.pid.is_some_and(|pid| imp::has_quit(pid, &self.app))
    }
}

/// The app in front, if there's one to name.
pub fn foreground() -> Option<Foreground> {
    let (app, pid) = imp::foreground()?;
    let app = app.trim();
    (!app.is_empty()).then(|| Foreground {
        app: app.to_owned(),
        pid,
    })
}

/// False where the desktop doesn't let us see the focused app (Wayland).
pub fn supported() -> bool {
    imp::supported()
}

/// Whether "no app" from [`foreground`] means "nothing worth switching for; keep the
/// last app". On Windows it does: the foreground window is NULL while focus moves and
/// while the UAC or lock screen is up, and the shell's own windows (taskbar, Start,
/// Search, Alt+Tab) are reported as no app too. Once the last app has quit
/// ([`Foreground::has_quit`]), as when it's closed with only the desktop behind it, no
/// app means no app again.
pub const NONE_KEEPS_LAST: bool = cfg!(windows);

/// Windows processes whose windows are the shell rather than an app. Their windows
/// being in front shouldn't switch profiles.
const SHELL_PROCESSES: [&str; 16] = [
    // Stands in for a hung app: Windows swaps a window that stops responding for a
    // "Not Responding" copy of it, which the Desktop Window Manager owns.
    "dwm",
    // Windows' own error boxes ("There is no disk in the drive", an app that can't
    // start), and the hung apps' copies on older Windows.
    "csrss",
    "ShellExperienceHost", // notifications and calendar; Windows 10's Action Center too
    "ShellHost",           // Windows 11 24H2+: Quick Settings, the volume mixer, Cast
    "StartMenuExperienceHost",
    "SearchHost", // Windows 11 Search
    "SearchApp",  // Windows 10 Search
    "SearchUI",   // Windows 10 Search before 2004
    "WidgetBoard",
    "Widgets",            // the widget board in earlier Windows 11 releases
    "ScreenClippingHost", // the Win+Shift+S overlay
    "TextInputHost",      // emoji panel, clipboard history, touch keyboard
    "ClickToDo",          // Copilot+ PCs' Click to Do overlay
    "LockApp",
    "consent",              // UAC prompt
    "ApplicationFrameHost", // Store apps' frames; the app in one is looked up
];

/// `explorer.exe` window classes that are File Explorer (Control Panel too), an app
/// someone may have a profile for. Every other window explorer owns is the shell: the
/// taskbars, the desktop, Alt+Tab, Task View, the tray overflow, menus, the Run box, and
/// the hidden windows it hands the foreground to while an app starts.
const FILE_EXPLORER_CLASSES: [&str; 2] = ["CabinetWClass", "ExploreWClass"];

/// Whether a foreground window of `process` (exe name without `.exe`) with window class
/// `class` is part of the Windows shell.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_windows_shell(process: &str, class: &str) -> bool {
    SHELL_PROCESSES
        .iter()
        .any(|p| p.eq_ignore_ascii_case(process))
        || (process.eq_ignore_ascii_case("explorer") && !FILE_EXPLORER_CLASSES.contains(&class))
}

// Asking the OS which window is in front needs its C APIs.
#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use windows_sys::{
        core::BOOL,
        Win32::{
            Foundation::{CloseHandle, HANDLE, HWND, LPARAM, STILL_ACTIVE},
            System::Threading::{
                GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
            UI::WindowsAndMessaging::{
                EnumChildWindows, GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId,
            },
        },
    };

    pub fn supported() -> bool {
        true
    }

    pub fn foreground() -> Option<(String, Option<u32>)> {
        // SAFETY: plain Win32 calls on a window handle Windows just gave us; a window
        // that has gone away since only makes the calls fail.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            return None;
        }
        let mut pid = window_pid(hwnd)?;
        let class = class_name(hwnd);
        if class == "ApplicationFrameWindow" {
            // A Store app: the frame is ApplicationFrameHost's (explorer has frames too
            // on Windows 11), and the app's own CoreWindow is a child window owned by
            // the app's process. While the app starts or closes there is none: that's
            // no app, like the shell, rather than the frame's owner.
            pid = hosted_app_pid(hwnd, pid)?;
        }
        let name = process_name(pid)?;
        if super::is_windows_shell(&name, &class) {
            return None;
        }
        Some((name, Some(pid)))
    }

    /// Whether process `pid`, last seen as `app`, has exited, or its id now belongs to
    /// another program.
    pub fn has_quit(pid: u32, app: &str) -> bool {
        // An exited process lingers while anything holds a handle to it (whatever
        // launched it, say), so ask for its exit code.
        // SAFETY: the handle is closed before returning, and `code` outlives the call
        // that fills it.
        let exited = unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                false
            } else {
                let mut code = 0;
                let ok = GetExitCodeProcess(process, &mut code);
                CloseHandle(process);
                ok != 0 && code != STILL_ACTIVE as u32
            }
        };
        exited || !process_name(pid).is_some_and(|name| name.eq_ignore_ascii_case(app))
    }

    fn window_pid(hwnd: HWND) -> Option<u32> {
        let mut pid = 0u32;
        // SAFETY: `pid` outlives the call that fills it.
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        (pid != 0).then_some(pid)
    }

    fn class_name(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        // SAFETY: the length passed is the buffer's, in characters.
        let len = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        String::from_utf16_lossy(&buf[..len.max(0) as usize])
    }

    /// The exe name, without `.exe`, of process `pid`.
    pub(super) fn process_name(pid: u32) -> Option<String> {
        // SAFETY: the handle is closed before returning.
        let named = unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                None
            } else {
                let name = image_name(process);
                CloseHandle(process);
                name
            }
        };
        // Another account's processes can't be opened (an app run as administrator
        // with an admin's password, as work PCs do), nor games under some anti-cheat.
        // The system's process list still names them. Windows' own processes can't be
        // opened either; those with windows that come to the front are shell.
        named.or_else(|| listed_name(pid))
    }

    /// The exe name, without `.exe`, of the process `process` is a handle to.
    fn image_name(process: HANDLE) -> Option<String> {
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        // SAFETY: the length passed is the buffer's, in characters, and the buffer
        // outlives the call that fills it.
        let ok = unsafe {
            QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
        };
        if ok == 0 {
            return None;
        }
        exe_stem(&String::from_utf16_lossy(&buf[..len as usize]))
    }

    /// Process `pid`'s exe name from the system's process list (a Toolhelp snapshot),
    /// which needs no access to the process itself.
    pub(super) fn listed_name(pid: u32) -> Option<String> {
        let pid = sysinfo::Pid::from_u32(pid);
        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            false,
            sysinfo::ProcessRefreshKind::nothing(),
        );
        exe_stem(&system.process(pid)?.name().to_string_lossy())
    }

    fn exe_stem(path: &str) -> Option<String> {
        std::path::Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
    }

    /// The first child window of `frame` that belongs to another process than the
    /// frame host's: the hosted Store app.
    fn hosted_app_pid(frame: HWND, host_pid: u32) -> Option<u32> {
        struct Search {
            host_pid: u32,
            found: u32,
        }
        unsafe extern "system" fn visit(child: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: `lparam` is the `Search` below, alive for the whole enumeration.
            let search = unsafe { &mut *(lparam as *mut Search) };
            match window_pid(child) {
                Some(pid) if pid != search.host_pid => {
                    search.found = pid;
                    0 // stop
                }
                _ => 1, // keep going
            }
        }
        let mut search = Search { host_pid, found: 0 };
        // SAFETY: the callback only runs during this call, while `search` is alive.
        unsafe {
            EnumChildWindows(frame, Some(visit), &mut search as *mut Search as LPARAM);
        }
        (search.found != 0).then_some(search.found)
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod imp {
    use core_foundation::{
        array::CFArray,
        base::{CFType, TCFType},
        dictionary::CFDictionary,
        number::CFNumber,
        string::CFString,
    };
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowOwnerName,
    };

    pub fn supported() -> bool {
        true
    }

    /// The window list is ordered front to back; the first normal-layer window belongs
    /// to the frontmost app. Owner names don't need the screen recording permission.
    pub fn foreground() -> Option<(String, Option<u32>)> {
        let list: CFArray = copy_window_info(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID,
        )?;
        // SAFETY: the keys are CoreGraphics' constant CFStrings.
        let (layer_key, owner_key) = unsafe {
            (
                CFString::wrap_under_get_rule(kCGWindowLayer),
                CFString::wrap_under_get_rule(kCGWindowOwnerName),
            )
        };
        for item in list.iter() {
            // SAFETY: each entry of the window list is a CFDictionary.
            let dict: CFDictionary<CFString, CFType> = unsafe {
                CFDictionary::wrap_under_get_rule(
                    *item as core_foundation::dictionary::CFDictionaryRef,
                )
            };
            let layer = dict
                .find(&layer_key)
                .and_then(|v| v.downcast::<CFNumber>())
                .and_then(|n| n.to_i64());
            if layer != Some(0) {
                continue;
            }
            if let Some(name) = dict.find(&owner_key).and_then(|v| v.downcast::<CFString>()) {
                return Some((name.to_string(), None));
            }
        }
        None
    }

    /// No process ids are reported here, so this is never asked.
    pub fn has_quit(_pid: u32, _app: &str) -> bool {
        false
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use x11rb::{
        connection::Connection,
        protocol::xproto::{AtomEnum, ConnectionExt},
    };

    pub fn supported() -> bool {
        std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some()
    }

    pub fn foreground() -> Option<(String, Option<u32>)> {
        if !supported() {
            return None;
        }
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let active = conn
            .intern_atom(false, b"_NET_ACTIVE_WINDOW")
            .ok()?
            .reply()
            .ok()?
            .atom;
        let window = conn
            .get_property(false, root, active, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?
            .value32()?
            .next()?;
        if window == 0 {
            return None;
        }
        // WM_CLASS is "instance\0class\0"; the class ("firefox", "obs") is the app.
        let class = conn
            .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
            .ok()?
            .reply()
            .ok()?
            .value;
        let mut parts = class.split(|b| *b == 0).filter(|p| !p.is_empty());
        let instance = parts.next();
        let class = parts.next().or(instance)?;
        Some((String::from_utf8_lossy(class).into_owned(), None))
    }

    /// No process ids are reported here, so this is never asked.
    pub fn has_quit(_pid: u32, _app: &str) -> bool {
        false
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    pub fn supported() -> bool {
        false
    }
    pub fn foreground() -> Option<(String, Option<u32>)> {
        None
    }
    pub fn has_quit(_pid: u32, _app: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_shell_windows_are_not_apps() {
        assert!(is_windows_shell("explorer", "Shell_TrayWnd"));
        assert!(is_windows_shell("explorer", "WorkerW"));
        assert!(is_windows_shell("SearchHost", "Windows.UI.Core.CoreWindow"));
        assert!(is_windows_shell("consent", "anything"));
        assert!(is_windows_shell("startmenuexperiencehost", ""));
        // Windows 11 24H2+ Quick Settings and volume mixer, and the widget board.
        assert!(is_windows_shell("ShellHost", "ControlCenterWindow"));
        assert!(is_windows_shell("WidgetBoard", ""));
        assert!(is_windows_shell("Widgets", ""));
        assert!(is_windows_shell("ScreenClippingHost", ""));
        assert!(is_windows_shell("TextInputHost", ""));
        // The Store apps' frame host is never the app itself.
        assert!(is_windows_shell("ApplicationFrameHost", ""));
        // A hung app's stand-in window, and Windows' error boxes. Neither process can be
        // opened, so they're named from the process list.
        assert!(is_windows_shell("dwm", "Ghost"));
        assert!(is_windows_shell("csrss", "#32770"));
        // Explorer's hidden window while an app starts, its XAML surfaces and the Run box.
        assert!(is_windows_shell("explorer", "ForegroundStaging"));
        assert!(is_windows_shell("explorer", "Windows.UI.Core.CoreWindow"));
        assert!(is_windows_shell("explorer", "XamlExplorerHostIslandWindow"));
        assert!(is_windows_shell("Explorer", "#32770"));
        // File Explorer is an app someone may have a profile for.
        assert!(!is_windows_shell("explorer", "CabinetWClass"));
        assert!(!is_windows_shell("EXPLORER", "ExploreWClass"));
        assert!(!is_windows_shell("obs64", "Qt663QWindowIcon"));
    }

    #[cfg(windows)]
    #[test]
    fn processes_are_named_even_when_they_cant_be_opened() {
        let me = std::env::current_exe().unwrap();
        let me = me.file_stem().unwrap().to_str().unwrap();
        let pid = std::process::id();
        assert_eq!(imp::process_name(pid).as_deref(), Some(me));
        assert_eq!(imp::listed_name(pid).as_deref(), Some(me));
        // The System process has no image path (and usually can't be opened), so its
        // name comes from the process list.
        assert_eq!(imp::process_name(4).as_deref(), Some("System"));
        // Process ids are multiples of 4, so there's never a 4001.
        assert_eq!(imp::listed_name(4001), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_own_processes_with_windows_in_front_are_shell() {
        use sysinfo::{ProcessRefreshKind, RefreshKind, System};

        let system = System::new_with_specifics(
            RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
        );
        let mut checked = 0;
        for (pid, process) in system.processes() {
            let exe = process.name().to_string_lossy();
            if exe.eq_ignore_ascii_case("dwm.exe") || exe.eq_ignore_ascii_case("csrss.exe") {
                // They can't be opened, so this is the name from the process list.
                let name = imp::process_name(pid.as_u32()).unwrap();
                assert!(is_windows_shell(&name, ""), "{name}");
                checked += 1;
            }
        }
        // There's a csrss for every session, and one for the services.
        assert!(checked >= 2);
    }

    #[cfg(windows)]
    #[test]
    fn an_app_has_quit_once_its_process_has_ended() {
        use std::{io::Write, process::Stdio};

        let me = std::env::current_exe().unwrap();
        let me = Foreground {
            app: me.file_stem().unwrap().to_string_lossy().into_owned(),
            pid: Some(std::process::id()),
        };
        assert!(!me.has_quit());
        // The same process id, now another program's.
        let reused = Foreground {
            app: "obs64".into(),
            ..me.clone()
        };
        assert!(reused.has_quit());

        // `set /p` waits for a line on stdin.
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "set /p line="])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let cmd = Foreground {
            app: "cmd".into(),
            pid: Some(child.id()),
        };
        assert!(!cmd.has_quit());
        child.stdin.take().unwrap().write_all(b"bye\n").unwrap();
        child.wait().unwrap();
        // Ended, though `child` still holds a handle that keeps the process around.
        assert!(cmd.has_quit());
        drop(child);
        assert!(cmd.has_quit());

        // Without a process id there's no telling.
        let unknown = Foreground {
            app: "obs64".into(),
            pid: None,
        };
        assert!(!unknown.has_quit());
    }

    /// What auto-switch sees in front, printed as it changes for a minute, to check
    /// shell windows by hand:
    /// `cargo test -p perritodeck-host --lib focus::tests::watch -- --ignored --nocapture`
    #[test]
    #[ignore = "watches the desktop for a minute"]
    fn watch() {
        use std::time::{Duration, Instant};

        let start = Instant::now();
        let mut last = None;
        while start.elapsed() < Duration::from_secs(60) {
            let now = foreground();
            if now != last {
                println!("{:6.1}s {now:?}", start.elapsed().as_secs_f32());
                last = now;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
