//! Finding Node.js when `node` isn't on the PATH PerritoDeck started with.
//!
//! A program's PATH is fixed when it starts, so Node.js installed while PerritoDeck is
//! running isn't on it until PerritoDeck restarts. Before saying a plugin needs Node.js,
//! the host looks where a program started now would find it, and where Node.js's
//! installers put it.

use std::{ffi::OsString, path::PathBuf};

const EXE: &str = if cfg!(windows) { "node.exe" } else { "node" };

/// Node.js on the PATH as it is now, or in a folder its installers use.
pub fn find() -> Option<PathBuf> {
    candidates(&fresh_path(), &install_folders())
        .into_iter()
        .find(|p| p.is_file())
}

/// Where to look, in order: each folder of each `paths` value (lists like PATH), then
/// `folders`. Each place once.
fn candidates(paths: &[OsString], folders: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in paths
        .iter()
        .flat_map(std::env::split_paths)
        .chain(folders.iter().cloned())
    {
        let exe = dir.join(EXE);
        if !dir.as_os_str().is_empty() && !out.contains(&exe) {
            out.push(exe);
        }
    }
    out
}

/// The PATH a program started now would get: the machine's, then the user's, as
/// Windows joins them, read from the registry.
#[cfg(windows)]
fn fresh_path() -> Vec<OsString> {
    use windows_registry::{Key, CURRENT_USER, LOCAL_MACHINE};
    const MACHINE: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    const USER: &str = "Environment";
    let read =
        |root: &Key, key: &str, name: &str| root.open(key).and_then(|k| k.get_string(name)).ok();
    // A folder may be written with a variable added since PerritoDeck started (nvm
    // adds %NVM_SYMLINK%), so variables are looked up in the registry too.
    let var = |name: &str| {
        std::env::var(name)
            .ok()
            .or_else(|| read(CURRENT_USER, USER, name))
            .or_else(|| read(LOCAL_MACHINE, MACHINE, name))
    };
    [
        read(LOCAL_MACHINE, MACHINE, "Path"),
        read(CURRENT_USER, USER, "Path"),
    ]
    .into_iter()
    .flatten()
    .map(|p| expand(&p, var).into())
    .collect()
}

/// Elsewhere there's no saved PATH to read again; the install folders are the fallback.
#[cfg(not(windows))]
fn fresh_path() -> Vec<OsString> {
    Vec::new()
}

/// `s` with each `%NAME%` replaced by `var(NAME)`. Unknown names stay as they are, as
/// Windows leaves them.
#[cfg(any(windows, test))]
fn expand(s: &str, var: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let Some(len) = after.find('%') else {
            break;
        };
        let name = &after[..len];
        out.push_str(&rest[..start]);
        match var(name).filter(|_| !name.is_empty()) {
            Some(value) => out.push_str(&value),
            None => {
                out.push('%');
                out.push_str(name);
                out.push('%');
            }
        }
        rest = &after[len + 1..];
    }
    out.push_str(rest);
    out
}

/// Where Node.js's installers put it. On Windows that's the official installer's folder,
/// which winget and Chocolatey use too, and where nvm-windows links the version in use.
/// On macOS, Homebrew's folders and the official installer's, which an app opened from
/// the Finder doesn't have on its PATH.
fn install_folders() -> Vec<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("ProgramFiles")
            .map(|p| PathBuf::from(p).join("nodejs"))
            .into_iter()
            .collect()
    } else if cfg!(target_os = "macos") {
        vec!["/opt/homebrew/bin".into(), "/usr/local/bin".into()]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn looks_on_the_path_first_then_in_the_install_folders() {
        let machine = std::env::join_paths(["m1", "shared"]).unwrap();
        let user = std::env::join_paths(["u1", "", "shared"]).unwrap();
        let found = candidates(
            &[machine, user],
            &[PathBuf::from("installed"), PathBuf::from("m1")],
        );
        let want: Vec<PathBuf> = ["m1", "shared", "u1", "installed"]
            .iter()
            .map(|d| Path::new(d).join(EXE))
            .collect();
        assert_eq!(found, want, "in order, each place once, no empty entries");
    }

    #[test]
    fn expands_variables_and_leaves_unknown_ones() {
        let var = |name: &str| (name == "Root").then(|| r"C:\Root".to_owned());
        assert_eq!(
            expand(r"%Root%\bin;%Nope%\x;100%;%%", var),
            r"C:\Root\bin;%Nope%\x;100%;%%"
        );
        assert_eq!(expand("plain", var), "plain");
    }

    /// The machine's PATH names System32 through %SystemRoot%, so reading it from the
    /// registry and expanding it gives a real folder.
    #[cfg(windows)]
    #[test]
    fn reads_the_path_from_the_registry() {
        let dirs: Vec<PathBuf> = fresh_path()
            .iter()
            .flat_map(std::env::split_paths)
            .collect();
        assert!(
            dirs.iter().any(|d| d.join("cmd.exe").is_file()),
            "System32 is on the machine's PATH: {dirs:?}"
        );
    }
}
