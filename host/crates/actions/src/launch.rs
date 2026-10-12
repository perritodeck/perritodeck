//! Finding and starting programs on Windows, for Launch and Command keys.
//!
//! Rust's own program search on Windows only appends `.exe`, so `code` (`code.cmd`),
//! `npm` and `az` aren't found. When a plain spawn reports NotFound, the callers retry
//! with [`search_path`], which looks for the PATHEXT names that `CreateProcess` can run.
//! Launch also falls back to `ShellExecuteW`, which knows App Paths names (`obs64`,
//! `chrome`), shortcuts and documents.
//!
//! The helpers that don't call Windows are compiled everywhere so the tests run on Linux.
#![cfg_attr(not(windows), allow(dead_code))]

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

/// PATHEXT when the variable is missing (the Windows default, minus script hosts).
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// Extensions `CreateProcess` starts directly. Rust's std runs `.bat` and `.cmd`
/// through `cmd.exe` with its own safe argument escaping. Others in PATHEXT (`.vbs`,
/// `.js`, `.msc`) need the shell, so the search skips them.
const RUNNABLE: [&str; 4] = [".com", ".exe", ".bat", ".cmd"];

/// Whether `program` names a file by path (`C:\x\y.exe`, `.\y`, `C:y`) rather than
/// a bare name to look up.
pub(crate) fn has_path(program: &str) -> bool {
    program.contains(['/', '\\', ':'])
}

/// The folder a launched program should start in: its own, as Explorer's "Start in"
/// does for shortcuts. OBS, for one, can't find its locale files otherwise. `None` for
/// a bare name.
pub(crate) fn program_dir(program: &str) -> Option<&Path> {
    if !has_path(program) {
        return None;
    }
    Path::new(program)
        .parent()
        .filter(|d| !d.as_os_str().is_empty() && d.is_dir())
}

/// Find a program as `cmd.exe` would, minus the current folder: for a bare name, each
/// PATH folder in turn with each runnable PATHEXT extension; for a path without an
/// extension, that path with each extension. `None` when nothing matches or the name
/// already ends in a runnable extension (the plain spawn already tried that).
pub(crate) fn search_path(
    program: &str,
    path: Option<&OsStr>,
    pathext: Option<&OsStr>,
) -> Option<PathBuf> {
    let lower = program.to_ascii_lowercase();
    if program.is_empty() || RUNNABLE.iter().any(|e| lower.ends_with(e)) {
        return None;
    }
    let pathext = pathext
        .and_then(OsStr::to_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(DEFAULT_PATHEXT);
    let exts: Vec<String> = pathext
        .split(';')
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| RUNNABLE.contains(&e.as_str()))
        .collect();
    let dirs: Vec<PathBuf> = if has_path(program) {
        vec![PathBuf::new()]
    } else {
        path.map(|p| std::env::split_paths(p).collect())
            .unwrap_or_default()
    };
    dirs.iter()
        .filter(|d| d.as_os_str().is_empty() || d.is_absolute())
        .flat_map(|d| exts.iter().map(move |e| d.join(format!("{program}{e}"))))
        .find(|p| p.is_file())
}

/// Join arguments into one command line the way the Microsoft C runtime parses it
/// back ("Parsing C++ command-line arguments"): quote arguments with spaces, tabs or
/// quotes, escape quotes, and double the backslashes that come before a quote.
pub(crate) fn quote_args(args: &[String]) -> String {
    let mut out = String::new();
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if !a.is_empty() && !a.contains([' ', '\t', '\n', '\u{0b}', '"']) {
            out.push_str(a);
            continue;
        }
        out.push('"');
        let mut slashes = 0;
        for c in a.chars() {
            match c {
                '\\' => slashes += 1,
                '"' => {
                    out.push_str(&"\\".repeat(slashes * 2 + 1));
                    out.push('"');
                    slashes = 0;
                }
                _ => {
                    out.push_str(&"\\".repeat(slashes));
                    out.push(c);
                    slashes = 0;
                }
            }
        }
        out.push_str(&"\\".repeat(slashes * 2));
        out.push('"');
    }
    out
}

/// Whether a failed spawn is worth handing to the shell: the file wasn't found, or it
/// isn't a program (`ERROR_BAD_EXE_FORMAT`, for shortcuts and documents).
#[cfg(all(windows, feature = "system"))]
pub(crate) fn shell_can_try(e: &std::io::Error) -> bool {
    const ERROR_BAD_EXE_FORMAT: i32 = 193;
    e.kind() == std::io::ErrorKind::NotFound || e.raw_os_error() == Some(ERROR_BAD_EXE_FORMAT)
}

/// Start `program` through `ShellExecuteW`, on a thread of its own with COM initialised
/// as Microsoft's docs ask (shell extensions may need an STA). `Err` carries the
/// ShellExecute result code (32 or less) when it fails.
#[cfg(all(windows, feature = "system"))]
#[allow(unsafe_code)]
pub(crate) async fn shell_execute(program: &str, args: &[String]) -> Result<(), isize> {
    use windows::{
        core::{w, HSTRING, PCWSTR},
        Win32::{
            System::Com::{
                CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
            },
            UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        },
    };

    let file = HSTRING::from(program);
    let params = HSTRING::from(quote_args(args));
    let dir = program_dir(program).map(|d| HSTRING::from(d.as_os_str()));
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        // SAFETY: plain Win32 calls with valid, NUL-terminated strings that outlive them.
        // COM is initialised on this new thread and uninitialised only if that worked.
        let code = unsafe {
            let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
            let r = ShellExecuteW(
                None,
                w!("open"),
                &file,
                &params,
                dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
                SW_SHOWNORMAL,
            );
            if com.is_ok() {
                CoUninitialize();
            }
            r.0 as isize
        };
        let _ = tx.send(code);
    });
    match rx.await {
        Ok(code) if code > 32 => Ok(()),
        Ok(code) => Err(code),
        Err(_) => Err(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_like_the_c_runtime() {
        let q = |a: &[&str]| quote_args(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(q(&[]), "");
        assert_eq!(q(&["a", "b"]), "a b");
        assert_eq!(q(&["two words"]), "\"two words\"");
        assert_eq!(q(&[""]), "\"\"");
        assert_eq!(q(&["say \"hi\""]), r#""say \"hi\"""#);
        assert_eq!(q(&[r"C:\a b\"]), r#""C:\a b\\""#);
        assert_eq!(q(&[r"C:\plain\"]), r"C:\plain\");
        assert_eq!(q(&[r#"a\"b"#]), r#""a\\\"b""#);
    }

    #[test]
    fn paths_and_their_folders() {
        assert!(!has_path("notepad"));
        assert!(has_path(r"C:\x\obs64.exe"));
        assert!(has_path("C:obs64.exe"));
        assert!(has_path("./tool"));
        assert_eq!(program_dir("notepad"), None);
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("obs64.exe");
        assert_eq!(program_dir(exe.to_str().unwrap()), Some(dir.path()));
        assert_eq!(program_dir("/no/such/folder/x"), None);
    }

    #[test]
    fn finds_pathext_names() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(b.path().join("code.cmd"), "").unwrap();
        std::fs::write(b.path().join("tool.vbs"), "").unwrap();
        std::fs::write(a.path().join("both.bat"), "").unwrap();
        std::fs::write(b.path().join("both.exe"), "").unwrap();
        let path = std::env::join_paths([a.path(), b.path()]).unwrap();
        let ext = OsStr::new(".COM;.EXE;.BAT;.CMD;.VBS");
        let find = |p: &str| search_path(p, Some(&path), Some(ext));
        assert_eq!(find("code"), Some(b.path().join("code.cmd")));
        // Folders come first, then extensions, as in cmd.exe.
        assert_eq!(find("both"), Some(a.path().join("both.bat")));
        // Script hosts need the shell, so they're not "found" here.
        assert_eq!(find("tool"), None);
        assert_eq!(find("code.cmd"), None);
        assert_eq!(find("missing"), None);
        // A missing PATHEXT means the default list.
        assert_eq!(
            search_path("code", Some(&path), None),
            Some(b.path().join("code.cmd"))
        );
        // A path with no extension gets the extensions too.
        let full = b.path().join("code");
        assert_eq!(
            search_path(full.to_str().unwrap(), None, Some(ext)),
            Some(b.path().join("code.cmd"))
        );
    }
}
