//! Which app is in front, for switching profiles automatically.
//!
//! Windows: the foreground window's program. macOS: the owner of the frontmost normal
//! window. Linux: the active window's `WM_CLASS` on X11. Wayland has no general way to
//! ask, so auto-switching is unavailable there for now.

/// A short, stable name for an app: `obs64`, `OBS`, `firefox`. Compared case-insensitively.
pub fn foreground_app() -> Option<String> {
    imp::foreground_app()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// False where the desktop doesn't let us see the focused app (Wayland).
pub fn supported() -> bool {
    imp::supported()
}

/// Whether "no app" from [`foreground_app`] means "nothing worth switching for; keep
/// the last app". On Windows it does: the foreground window is NULL while focus moves
/// and while the UAC or lock screen is up, and the shell's own windows (taskbar, Start,
/// Search, Alt+Tab) are reported as no app too.
pub const NONE_KEEPS_LAST: bool = cfg!(windows);

/// Windows processes whose windows are the shell rather than an app. Their windows
/// being in front shouldn't switch profiles.
const SHELL_PROCESSES: [&str; 13] = [
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
    "consent", // UAC prompt
];

/// `explorer.exe` window classes that are the shell: the taskbars, the desktop, the
/// task switcher and the tray overflow. File Explorer windows (`CabinetWClass`) count as
/// an app.
const SHELL_EXPLORER_CLASSES: [&str; 8] = [
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "Progman",
    "WorkerW",
    "MultitaskingViewFrame",
    "TaskSwitcherWnd",
    "XamlExplorerHostIslandWindow",
    "TopLevelWindowForOverflowXamlIsland",
];

/// Whether a foreground window of `process` (exe name without `.exe`) with window class
/// `class` is part of the Windows shell.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_windows_shell(process: &str, class: &str) -> bool {
    SHELL_PROCESSES
        .iter()
        .any(|p| p.eq_ignore_ascii_case(process))
        || (process.eq_ignore_ascii_case("explorer") && SHELL_EXPLORER_CLASSES.contains(&class))
}

// Asking the OS which window is in front needs its C APIs.
#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use windows_sys::{
        core::BOOL,
        Win32::{
            Foundation::{CloseHandle, HWND, LPARAM},
            System::Threading::{
                OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
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

    pub fn foreground_app() -> Option<String> {
        // SAFETY: plain Win32 calls on a window handle Windows just gave us; a window
        // that has gone away since only makes the calls fail.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            return None;
        }
        let pid = window_pid(hwnd)?;
        let name = process_name(pid)?;
        if name.eq_ignore_ascii_case("ApplicationFrameHost") {
            // A Store app: the frame is ApplicationFrameHost's, and the app's own
            // CoreWindow is a child window owned by the app's process. A suspended
            // (minimised) app may have none; keep the frame host's name then.
            return Some(
                hosted_app_pid(hwnd, pid)
                    .and_then(process_name)
                    .unwrap_or(name),
            );
        }
        if super::is_windows_shell(&name, &class_name(hwnd)) {
            return None;
        }
        Some(name)
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
    fn process_name(pid: u32) -> Option<String> {
        // SAFETY: the handle is closed before returning and the buffer outlives the
        // call that fills it.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok =
                QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
            CloseHandle(process);
            if ok == 0 {
                return None;
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            std::path::Path::new(&path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        }
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
    pub fn foreground_app() -> Option<String> {
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
                return Some(name.to_string());
            }
        }
        None
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

    pub fn foreground_app() -> Option<String> {
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
        Some(String::from_utf8_lossy(class).into_owned())
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    pub fn supported() -> bool {
        false
    }
    pub fn foreground_app() -> Option<String> {
        None
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
        // File Explorer is an app someone may have a profile for.
        assert!(!is_windows_shell("explorer", "CabinetWClass"));
        assert!(!is_windows_shell("obs64", "Qt663QWindowIcon"));
    }
}
