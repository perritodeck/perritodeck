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

// Asking the OS which window is in front needs its C APIs.
#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
    };

    pub fn supported() -> bool {
        true
    }

    pub fn foreground_app() -> Option<String> {
        // SAFETY: plain Win32 calls; the handle is closed before returning and the
        // buffer outlives the call that fills it.
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == 0 {
                return None;
            }
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
