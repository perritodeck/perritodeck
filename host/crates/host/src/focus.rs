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
    /// Where the OS says (Windows), its process and the window that was in front, to
    /// notice the app going away.
    seen: Option<Seen>,
}

/// An app's process, and its window that was in front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
struct Seen {
    pid: u32,
    /// The window's handle, as a number so it can be passed to another thread.
    window: usize,
}

impl Foreground {
    /// Whether the app has gone since it was in front: it has quit, or closed or hidden
    /// (in the tray, say) its last window. False where we can't tell.
    pub fn has_left(&self) -> bool {
        self.seen.is_some_and(|seen| imp::has_left(seen, &self.app))
    }
}

/// The app in front, if there's one to name.
pub fn foreground() -> Option<Foreground> {
    let (app, seen) = imp::foreground()?;
    let app = app.trim();
    (!app.is_empty()).then(|| Foreground {
        app: app.to_owned(),
        seen,
    })
}

/// False where the desktop doesn't let us see the focused app (Wayland).
pub fn supported() -> bool {
    imp::supported()
}

/// Whether "no app" from [`foreground`] means "nothing worth switching for; keep the
/// last app". On Windows it does: the foreground window is NULL while focus moves and
/// while the UAC or lock screen is up, and the shell's own windows (taskbar, Start,
/// Search, Alt+Tab) are reported as no app too. Once the last app has gone
/// ([`Foreground::has_left`]), as when it's closed or hidden in the tray with only the
/// desktop behind it, no app means no app again.
pub const NONE_KEEPS_LAST: bool = cfg!(windows);

/// Decides, one look at a time, when the app in front has changed enough to switch
/// profiles for.
#[derive(Debug)]
pub struct Watch {
    /// The app the profiles were last switched for.
    last: Option<String>,
    /// The last app's newest window seen in front, to notice it going away while no app
    /// is.
    last_seen: Option<Foreground>,
    /// Whether the last look found the last app gone too.
    leaving: bool,
    /// [`NONE_KEEPS_LAST`], except in tests.
    none_keeps_last: bool,
}

impl Default for Watch {
    fn default() -> Self {
        Self {
            last: None,
            last_seen: None,
            leaving: false,
            none_keeps_last: NONE_KEEPS_LAST,
        }
    }
}

impl Watch {
    /// The app last seen in front, to ask whether it has left while no app is.
    pub fn last_seen(&self) -> Option<&Foreground> {
        self.last_seen.as_ref()
    }

    /// Takes a look at what's in front (`front`), and whether the app last seen has left
    /// since (`left`, asked only while no app is in front). Gives the app to switch
    /// profiles for when that has changed, `Some(None)` being no app.
    pub fn change(&mut self, front: Option<Foreground>, left: bool) -> Option<Option<String>> {
        if front.is_none() && self.none_keeps_last {
            // On Windows a moment with no window in front, or the shell, keeps the last
            // app, unless it has left. It has to look gone twice in a row, so an app
            // replacing its window (a splash screen, a game changing display modes)
            // isn't taken for the app closing.
            let leaving = std::mem::replace(&mut self.leaving, left);
            if !(left && leaving) {
                return None;
            }
        }
        self.leaving = false;
        let app = front.as_ref().map(|f| f.app.clone());
        // The newest window seen counts even when the app hasn't changed, so closing an
        // older one isn't taken for the app going away.
        self.last_seen = front;
        if app == self.last {
            return None;
        }
        self.last.clone_from(&app);
        Some(app)
    }
}

/// Windows processes whose windows are the shell rather than an app. Their windows
/// being in front shouldn't switch profiles.
const SHELL_PROCESSES: [&str; 17] = [
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
    // The Win+Shift+S overlay once Snipping Tool is updated, as on current Windows 11.
    // Its own window, for marking up a snip, keeps the last app too.
    "SnippingTool",
    "TextInputHost", // emoji panel, clipboard history, touch keyboard
    "ClickToDo",     // Copilot+ PCs' Click to Do overlay
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
    use super::Seen;
    use windows_sys::{
        core::BOOL,
        Win32::{
            Foundation::{CloseHandle, HANDLE, HWND, LPARAM, STILL_ACTIVE},
            System::Threading::{
                GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
            UI::WindowsAndMessaging::{
                EnumChildWindows, EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowLongW,
                GetWindowThreadProcessId, IsHungAppWindow, IsWindowVisible, GWL_EXSTYLE,
                WS_EX_TOOLWINDOW,
            },
        },
    };

    pub fn supported() -> bool {
        true
    }

    pub fn foreground() -> Option<(String, Option<Seen>)> {
        // SAFETY: plain Win32 calls on a window handle Windows just gave us; a window
        // that has gone away since only makes the calls fail.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            return None;
        }
        let class = class_name(hwnd);
        let pid = app_pid(hwnd, &class)?;
        let name = process_name(pid)?;
        if super::is_windows_shell(&name, &class) {
            return None;
        }
        let window = hwnd as usize;
        Some((name, Some(Seen { pid, window })))
    }

    /// The process of the app that `window`, of class `class`, belongs to.
    fn app_pid(window: HWND, class: &str) -> Option<u32> {
        let pid = window_pid(window)?;
        if class == "ApplicationFrameWindow" {
            // A Store app: the frame is ApplicationFrameHost's (explorer has frames too
            // on Windows 11), and the app's own CoreWindow is a child window owned by
            // the app's process. While the app starts or closes there is none: that's
            // no app, like the shell, rather than the frame's owner.
            return hosted_app_pid(window, pid);
        }
        Some(pid)
    }

    /// Whether the app `app`, `seen` in front, has gone: it has quit, or closed or hidden
    /// its last window. A minimized window, or one on another virtual desktop, is still
    /// there.
    pub fn has_left(seen: Seen, app: &str) -> bool {
        has_quit(seen.pid, app) || !(is_still_there(seen) || has_window(seen.pid, app))
    }

    /// Whether the window `seen` in front is still there and still the app's: shown, or
    /// hung (whether or not Windows has swapped it for its "Not Responding" copy yet).
    fn is_still_there(seen: Seen) -> bool {
        let window = seen.window as HWND;
        // SAFETY: plain Win32 calls; a window that has gone is neither visible nor hung.
        let there = unsafe { IsWindowVisible(window) != 0 || IsHungAppWindow(window) != 0 };
        if !there {
            return false;
        }
        let class = class_name(window);
        match app_pid(window, &class) {
            Some(pid) => pid == seen.pid,
            // A Store app's frame stays while the app is minimized, but the app's own
            // window is taken out of it then.
            None => class == "ApplicationFrameWindow",
        }
    }

    /// Whether process `pid`, the app `app`, has a window someone could go back to.
    fn has_window(pid: u32, app: &str) -> bool {
        struct Search<'a> {
            pid: u32,
            app: &'a str,
            found: bool,
        }
        unsafe extern "system" fn visit(window: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: `lparam` is the `Search` below, alive for the whole enumeration.
            let search = unsafe { &mut *(lparam as *mut Search<'_>) };
            if is_app_window(window, search.pid, search.app) {
                search.found = true;
                0 // stop
            } else {
                1 // keep going
            }
        }
        let mut search = Search {
            pid,
            app,
            found: false,
        };
        // SAFETY: the callback only runs during this call, while `search` is alive.
        unsafe {
            EnumWindows(Some(visit), &mut search as *mut Search<'_> as LPARAM);
        }
        search.found
    }

    /// Whether the top-level window `window` is one of the app `app`'s (process `pid`)
    /// that someone could go back to: shown (minimized is fine), not a tool window (the
    /// small helper windows some programs keep shown), and one [`foreground`] would name
    /// as the app.
    fn is_app_window(window: HWND, pid: u32, app: &str) -> bool {
        // SAFETY: plain Win32 calls on a window Windows has just listed.
        let shown = unsafe {
            IsWindowVisible(window) != 0
                && (GetWindowLongW(window, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW) == 0
        };
        if !shown {
            return false;
        }
        let class = class_name(window);
        app_pid(window, &class) == Some(pid) && !super::is_windows_shell(app, &class)
    }

    /// Whether process `pid`, last seen as `app`, has exited, or its id now belongs to
    /// another program.
    pub(super) fn has_quit(pid: u32, app: &str) -> bool {
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
    pub fn foreground() -> Option<(String, Option<super::Seen>)> {
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

    /// No processes or windows are reported here, so this is never asked.
    pub fn has_left(_seen: super::Seen, _app: &str) -> bool {
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

    pub fn foreground() -> Option<(String, Option<super::Seen>)> {
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

    /// No processes or windows are reported here, so this is never asked.
    pub fn has_left(_seen: super::Seen, _app: &str) -> bool {
        false
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    pub fn supported() -> bool {
        false
    }
    pub fn foreground() -> Option<(String, Option<super::Seen>)> {
        None
    }
    pub fn has_left(_seen: super::Seen, _app: &str) -> bool {
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
        assert!(is_windows_shell("SnippingTool", ""));
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

    fn in_front(app: &str, window: usize) -> Option<Foreground> {
        Some(Foreground {
            app: app.into(),
            seen: Some(Seen { pid: 8, window }),
        })
    }

    #[test]
    fn the_watch_switches_when_the_app_in_front_changes() {
        let mut watch = Watch {
            none_keeps_last: true,
            ..Watch::default()
        };
        assert_eq!(
            watch.change(in_front("obs64", 10), false),
            Some(Some("obs64".into()))
        );
        // Another window of the same app isn't a change, but it's the one to watch now.
        assert_eq!(watch.change(in_front("obs64", 11), false), None);
        assert_eq!(watch.last_seen(), in_front("obs64", 11).as_ref());
        // No app in front (the desktop, the shell, a moment between windows) keeps the
        // last app...
        assert_eq!(watch.change(None, false), None);
        assert_eq!(watch.last_seen(), in_front("obs64", 11).as_ref());
        // ...until it has left, as seen on two looks in a row: here it came back...
        assert_eq!(watch.change(None, true), None);
        assert_eq!(watch.change(in_front("obs64", 11), false), None);
        assert_eq!(watch.change(None, true), None);
        assert_eq!(watch.change(None, false), None);
        // ...and here it's gone.
        assert_eq!(watch.change(None, true), None);
        assert_eq!(watch.change(None, true), Some(None));
        assert_eq!(watch.last_seen(), None);
        assert_eq!(watch.change(None, false), None);
        assert_eq!(
            watch.change(in_front("firefox", 20), false),
            Some(Some("firefox".into()))
        );

        // Where no app means just that (macOS, Linux), it switches straight away.
        let mut watch = Watch {
            none_keeps_last: false,
            ..Watch::default()
        };
        assert_eq!(
            watch.change(in_front("firefox", 20), false),
            Some(Some("firefox".into()))
        );
        assert_eq!(watch.change(None, false), Some(None));
        assert_eq!(watch.change(None, false), None);
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
        let me = me.file_stem().unwrap().to_string_lossy();
        let pid = std::process::id();
        assert!(!imp::has_quit(pid, &me));
        // The same process id, now another program's.
        assert!(imp::has_quit(pid, "obs64"));

        // `set /p` waits for a line on stdin.
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "set /p line="])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let cmd = child.id();
        assert!(!imp::has_quit(cmd, "cmd"));
        child.stdin.take().unwrap().write_all(b"bye\n").unwrap();
        child.wait().unwrap();
        // Ended, though `child` still holds a handle that keeps the process around.
        assert!(imp::has_quit(cmd, "cmd"));
        drop(child);
        assert!(imp::has_quit(cmd, "cmd"));
    }

    #[cfg(windows)]
    #[test]
    #[allow(unsafe_code)]
    fn an_app_has_left_once_its_last_window_is_closed_or_hidden() {
        use windows_sys::Win32::{
            Foundation::HWND,
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, ShowWindow, SHOW_WINDOW_CMD, SW_HIDE,
                SW_SHOWNOACTIVATE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
            },
        };

        // Plain "Static" windows: 1x1 pixel, far off the screen and never activated.
        // Those shown are owned by a hidden window, which keeps them off the taskbar.
        let class: Vec<u16> = "Static\0".encode_utf16().collect();
        let create = |owner: HWND, ex_style: u32| {
            // SAFETY: the class name is NUL-terminated and outlives the call.
            let window = unsafe {
                CreateWindowExW(
                    WS_EX_NOACTIVATE | ex_style,
                    class.as_ptr(),
                    std::ptr::null(),
                    WS_POPUP,
                    -32000,
                    -32000,
                    1,
                    1,
                    owner,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            assert!(!window.is_null());
            window
        };
        // SAFETY: the windows are this thread's own, made above.
        let show = |window: HWND, how: SHOW_WINDOW_CMD| unsafe {
            ShowWindow(window, how);
        };
        let close = |window: HWND| unsafe {
            DestroyWindow(window);
        };
        let owner = create(std::ptr::null_mut(), 0);
        let first = create(owner, 0);
        let second = create(owner, 0);
        let tool = create(owner, WS_EX_TOOLWINDOW);

        let me = std::env::current_exe().unwrap();
        let app = Foreground {
            app: me.file_stem().unwrap().to_string_lossy().into_owned(),
            seen: Some(Seen {
                pid: std::process::id(),
                window: first as usize,
            }),
        };
        // Nothing shown yet (the test process has no windows of its own).
        assert!(app.has_left());
        show(first, SW_SHOWNOACTIVATE);
        assert!(!app.has_left());
        // Its window that was in front is hidden (as in the tray), but there's another.
        show(second, SW_SHOWNOACTIVATE);
        show(first, SW_HIDE);
        assert!(!app.has_left());
        // That one is closed. A tool window still shown doesn't count.
        show(tool, SW_SHOWNOACTIVATE);
        close(second);
        assert!(app.has_left());
        // Its window that was in front is shown again, then closed.
        show(first, SW_SHOWNOACTIVATE);
        assert!(!app.has_left());
        close(first);
        assert!(app.has_left());
        close(tool);
        close(owner);

        // Without a process or a window there's no telling.
        let unknown = Foreground {
            app: "obs64".into(),
            seen: None,
        };
        assert!(!unknown.has_left());
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
            // The app and its process; another window of the same app isn't news here.
            let app = now.as_ref().map(|f| (f.app.clone(), f.seen.map(|s| s.pid)));
            if app != last {
                println!("{:6.1}s {now:?}", start.elapsed().as_secs_f32());
                last = app;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
