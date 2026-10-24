//! Files only this user account may read: written atomically, and owner-only on every
//! system. On Unix that's mode 0600; on Windows a protected DACL that lets in this
//! account, SYSTEM and the Administrators group, and nobody else.
//!
//! Used for `integrations.toml`, which holds passwords and tokens.

use std::{fs, io, path::Path};

/// Who can read a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Privacy {
    /// Only this account (and, on Windows, SYSTEM and Administrators).
    Private,
    /// Other accounts on this computer can read it.
    ReadableByOthers,
    /// It can't be told: the drive keeps no permissions (FAT, exFAT, some shares), or
    /// asking failed.
    Unknown,
}

/// Write `data` to `path` in one go, owner-only. `name` is the file's name for messages
/// ("integrations.toml").
///
/// The data goes to `<file name>.tmp` beside it, made private before anything is written
/// to it, then renamed over `path`. If the new file couldn't be made private and other
/// accounts could read it, nothing is saved. A symlink is written through, to the file
/// it points at.
pub fn write_atomic(path: &Path, data: &[u8], name: &str) -> Result<(), String> {
    write_atomic_with(path, data, name, restrict)
}

/// Make an existing file owner-only.
pub fn restrict(path: &Path) -> io::Result<()> {
    imp::restrict(path)
}

/// Who can read `path`. Never fails: what can't be told is [`Privacy::Unknown`].
pub fn privacy(path: &Path) -> Privacy {
    imp::privacy(path)
}

fn write_atomic_with(
    path: &Path,
    data: &[u8],
    name: &str,
    restrict: impl Fn(&Path) -> io::Result<()>,
) -> Result<(), String> {
    let saving = |e: io::Error| format!("Couldn't save {name}: {e}.");
    // Write through a symlink to the file it points at, rather than replacing the link.
    let target = match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => fs::canonicalize(path).map_err(saving)?,
        _ => path.to_owned(),
    };
    let file_name = target
        .file_name()
        .ok_or_else(|| format!("Couldn't save {name}: it has no file name."))?;
    let mut tmp_name = file_name.to_owned();
    tmp_name.push(".tmp");
    let tmp = target.with_file_name(tmp_name);
    // A leftover from an attempt that failed half way would keep its permissions.
    match fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(saving(e)),
    }
    let mut file = {
        let mut open = fs::OpenOptions::new();
        open.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
        open.open(&tmp).map_err(saving)?
    };
    let gone = |tmp: &Path| {
        let _ = fs::remove_file(tmp);
    };
    // Private before a single byte of the secret is in it.
    if let Err(e) = restrict(&tmp) {
        match privacy(&tmp) {
            Privacy::ReadableByOthers => {
                drop(file);
                gone(&tmp);
                return Err(format!(
                    "Couldn't make {name} private ({e}), so nothing was saved. Keep \
                     PerritoDeck's settings folder on a drive that supports permissions."
                ));
            }
            Privacy::Private | Privacy::Unknown => {
                tracing::warn!("couldn't make {name} private: {e}");
            }
        }
    }
    let written = io::Write::write_all(&mut file, data).and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = written {
        gone(&tmp);
        return Err(saving(e));
    }
    rename(&tmp, &target).map_err(|e| {
        gone(&tmp);
        let in_use = cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32));
        let hint = if in_use {
            " Is it open in another program?"
        } else {
            ""
        };
        format!("Couldn't save {name}: {e}.{hint}")
    })
}

/// `fs::rename`, tried again for a second on Windows while another program (a virus
/// scanner, an editor, the search indexer) holds the file: access denied (5) or a
/// sharing violation (32). A rename within a folder keeps the file's security.
fn rename(from: &Path, to: &Path) -> io::Result<()> {
    let mut tries = 0;
    loop {
        match fs::rename(from, to) {
            Err(e) if cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32)) && tries < 20 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            other => return other,
        }
    }
}

/// Whether a DACL in SDDL form (`D:P(A;;FA;;;SY)…`) lets anyone but `me` (a SID string),
/// SYSTEM, Administrators and the file's owner read the file's contents.
///
/// An entry counts only if it allows (`A`, `OA`, `XA`), applies to the file itself (not
/// inherit-only, `IO`) and grants reading data: FILE_READ_DATA (0x1), GENERIC_ALL,
/// GENERIC_READ or MAXIMUM_ALLOWED. READ_CONTROL or SYNCHRONIZE alone don't. AppContainer
/// and package SIDs (`AC`, `S-1-15-…`) don't count: Windows adds them on AppData that a
/// packaged app redirects, and they aren't other people. No DACL at all
/// (`D:NO_ACCESS_CONTROL`) lets everyone in.
#[allow(dead_code)] // used on Windows, tested everywhere
pub(crate) fn others_allowed(sddl: &str, me: &str) -> bool {
    let Some(at) = sddl.find("D:") else {
        return false;
    };
    let dacl = &sddl[at + 2..];
    // The DACL runs until a SACL (`S:`) that follows it, outside any entry.
    let mut depth = 0;
    let mut end = dacl.len();
    for (i, c) in dacl.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            'S' if depth == 0 && dacl[i..].starts_with("S:") => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    let dacl = &dacl[..end];
    if dacl.starts_with("NO_ACCESS_CONTROL") {
        return true;
    }
    let me_is_admin = me.rsplit('-').next() == Some("500");
    dacl.split('(').skip(1).any(|ace| {
        let fields: Vec<&str> = ace.trim_end_matches(')').split(';').collect();
        let [kind, flags, rights, _, _, sid, ..] = fields[..] else {
            return false;
        };
        let allows = matches!(kind, "A" | "OA" | "XA");
        let inherit_only = flags.as_bytes().chunks(2).any(|f| f == b"IO");
        let exempt = sid == me
            || matches!(
                sid,
                "SY" | "BA" | "OW" | "CO" | "S-1-5-18" | "S-1-5-32-544" | "S-1-3-4" | "AC"
            )
            || sid.starts_with("S-1-15-")
            || (sid == "LA" && me_is_admin);
        allows && !inherit_only && !exempt && reads(rights)
    })
}

/// Whether an SDDL rights field grants reading a file's contents.
fn reads(rights: &str) -> bool {
    const READS: u32 = 0x1 | 0x1000_0000 | 0x8000_0000 | 0x0200_0000;
    let mask = match rights
        .strip_prefix("0x")
        .or_else(|| rights.strip_prefix("0X"))
    {
        // Unreadable hex counts as reading: better a needless warning than a missed one.
        Some(hex) => u32::from_str_radix(hex, 16).unwrap_or(u32::MAX),
        None => rights
            .as_bytes()
            .chunks(2)
            .map(|code| match code {
                b"GA" => 0x1000_0000,
                b"GR" => 0x8000_0000,
                b"FA" => 0x001F_01FF,
                b"FR" => 0x0012_0089,
                b"CC" => 0x1,
                b"MA" => 0x0200_0000,
                _ => 0,
            })
            .fold(0, |a, b| a | b),
    };
    mask & READS != 0
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    pub fn restrict(path: &Path) -> io::Result<()> {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
    }

    pub fn privacy(path: &Path) -> Privacy {
        match fs::metadata(path) {
            Ok(m) if m.permissions().mode() & 0o077 == 0 => Privacy::Private,
            Ok(_) => Privacy::ReadableByOthers,
            Err(_) => Privacy::Unknown,
        }
    }
}

/// The Win32 calls: the current user's SID, a DACL set from SDDL, and a file's DACL read
/// back as SDDL.
#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use super::*;
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree, ERROR_SUCCESS, HANDLE},
        Security::{
            Authorization::{
                ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
                ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
                SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
            },
            GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACL,
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
            TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    pub fn restrict(path: &Path) -> io::Result<()> {
        let me = current_user_sid()?;
        apply_sddl(path, &format!("D:P(A;;FA;;;{me})(A;;FA;;;SY)(A;;FA;;;BA)"))
    }

    pub fn privacy(path: &Path) -> Privacy {
        match (dacl_sddl(path), current_user_sid()) {
            (Ok(Some(sddl)), Ok(me)) if others_allowed(&sddl, &me) => Privacy::ReadableByOthers,
            (Ok(Some(_)), Ok(_)) => Privacy::Private,
            // No DACL: a drive that keeps no permissions (FAT, exFAT, some shares).
            _ => Privacy::Unknown,
        }
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain([0]).collect()
    }

    /// Frees, when dropped, what Windows allocated with LocalAlloc.
    struct Local(*mut core::ffi::c_void);

    impl Drop for Local {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the pointer came from an API documented to be freed with LocalFree.
                unsafe { LocalFree(self.0) };
            }
        }
    }

    /// A NUL-terminated UTF-16 string Windows allocated.
    ///
    /// # Safety
    /// `p` must point at a NUL-terminated UTF-16 string.
    unsafe fn from_wide(p: *const u16) -> String {
        let mut len = 0;
        // SAFETY: the caller promises a NUL terminator.
        while unsafe { *p.add(len) } != 0 {
            len += 1;
        }
        // SAFETY: the `len` u16s before the NUL are readable.
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
    }

    /// This process's user, as `S-1-5-21-…`.
    pub fn current_user_sid() -> io::Result<String> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: GetCurrentProcess returns a pseudo-handle; `token` is written on success.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let sid = user_sid(token);
        // SAFETY: `token` was opened above and is closed once.
        unsafe { CloseHandle(token) };
        sid
    }

    fn user_sid(token: HANDLE) -> io::Result<String> {
        let mut len = 0u32;
        // SAFETY: asks for the size only (it fails with ERROR_INSUFFICIENT_BUFFER).
        unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut len) };
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        // u64s keep TOKEN_USER's pointer aligned.
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        // SAFETY: `buf` holds at least `len` bytes.
        let ok = unsafe {
            GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len)
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: on success the buffer starts with a TOKEN_USER.
        let user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
        let mut text: *mut u16 = ptr::null_mut();
        // SAFETY: the SID points into `buf`, alive for this call.
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let text = Local(text.cast());
        // SAFETY: ConvertSidToStringSidW returns a NUL-terminated string.
        Ok(unsafe { from_wide(text.0 as *const u16) })
    }

    /// Replace `path`'s DACL with the one in `sddl`; `D:P(…)` also stops it inheriting
    /// its folder's entries.
    pub fn apply_sddl(path: &Path, sddl: &str) -> io::Result<()> {
        let text: Vec<u16> = sddl.encode_utf16().chain([0]).collect();
        let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `text` is NUL-terminated; `sd` is written on success.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let sd = Local(sd);
        let (mut present, mut defaulted) = (0, 0);
        let mut dacl: *mut ACL = ptr::null_mut();
        // SAFETY: `sd` is a valid descriptor until dropped; the DACL points into it.
        let ok =
            unsafe { GetSecurityDescriptorDacl(sd.0, &mut present, &mut dacl, &mut defaulted) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let protected = if sddl.starts_with("D:P") {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            0
        };
        let name = wide(path);
        // SAFETY: `name` is NUL-terminated and `dacl` lives in `sd`, alive for this call.
        let e = unsafe {
            SetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | protected,
                ptr::null_mut(),
                ptr::null_mut(),
                dacl,
                ptr::null(),
            )
        };
        if e != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(e as i32));
        }
        Ok(())
    }

    /// `path`'s DACL as SDDL, or `None` when it has none.
    pub fn dacl_sddl(path: &Path) -> io::Result<Option<String>> {
        let name = wide(path);
        let mut dacl: *mut ACL = ptr::null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `name` is NUL-terminated; `dacl` and `sd` are written on success.
        let e = unsafe {
            GetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut sd,
            )
        };
        if e != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(e as i32));
        }
        let sd = Local(sd);
        if dacl.is_null() {
            return Ok(None);
        }
        let mut text: *mut u16 = ptr::null_mut();
        // SAFETY: `sd` is valid until dropped; `text` is written on success.
        let ok = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                sd.0,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let text = Local(text.cast());
        // SAFETY: the conversion returns a NUL-terminated string.
        Ok(Some(unsafe { from_wide(text.0 as *const u16) }))
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::*;

    pub fn restrict(_: &Path) -> io::Result<()> {
        Ok(())
    }

    pub fn privacy(_: &Path) -> Privacy {
        Privacy::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "S-1-5-21-1004336348-1177238915-682003330-1001";

    #[test]
    fn who_else_a_dacl_lets_read() {
        let others = |sddl: &str| others_allowed(sddl, ME);
        // What a file made in %APPDATA% inherits: this account, SYSTEM, Administrators.
        assert!(!others(&format!(
            "D:AI(A;ID;FA;;;{ME})(A;ID;FA;;;SY)(A;ID;FA;;;BA)"
        )));
        // What restrict sets.
        assert!(!others(&format!(
            "D:P(A;;FA;;;{ME})(A;;FA;;;SY)(A;;FA;;;BA)"
        )));
        // Users, Authenticated Users or Everyone reading it, by any spelling of read.
        for sid in ["BU", "AU", "WD"] {
            for rights in ["FR", "0x1200a9", "GR", "FA", "GA", "CC", "0x1"] {
                let sddl = format!("D:(A;;FA;;;{ME})(A;;{rights};;;{sid})");
                assert!(others(&sddl), "{sddl}");
            }
        }
        // Another account.
        assert!(others(&format!(
            "D:(A;;FA;;;{ME})(A;;FA;;;S-1-5-21-1004336348-1177238915-682003330-1002)"
        )));
        // Rights that don't read the contents: SYNCHRONIZE, READ_CONTROL.
        assert!(!others(&format!("D:(A;;FA;;;{ME})(A;;0x100000;;;BU)")));
        assert!(!others(&format!("D:(A;;FA;;;{ME})(A;;RC;;;BU)")));
        // Only for what's made inside a folder, not the file itself.
        assert!(!others(&format!("D:(A;;FA;;;{ME})(A;OICIIO;FA;;;BU)")));
        // A deny entry lets nobody in.
        assert!(!others(&format!("D:(D;;FA;;;BU)(A;;FA;;;{ME})")));
        // The owner, Creator Owner, and the long forms of SYSTEM and Administrators.
        for sid in ["OW", "CO", "S-1-5-18", "S-1-5-32-544", "S-1-3-4"] {
            assert!(!others(&format!("D:(A;;FA;;;{sid})")), "{sid}");
        }
        // AppContainer and package SIDs, as on AppData a packaged app redirects.
        assert!(!others("D:(A;;FA;;;AC)"));
        assert!(!others(
            "D:(A;;FR;;;S-1-15-2-1430448594-2639229838-973813799-439329657-1197984847)"
        ));
        // The built-in Administrator account is "me" only when I am it.
        assert!(others(&format!("D:(A;;FA;;;{ME})(A;;FA;;;LA)")));
        assert!(!others_allowed(
            "D:(A;;FA;;;LA)",
            "S-1-5-21-1004336348-1177238915-682003330-500"
        ));
        // No DACL at all lets everyone in; an object ACE that reads counts too.
        assert!(others("D:NO_ACCESS_CONTROL"));
        assert!(others(&format!(
            "D:(A;;FA;;;{ME})(OA;;CCDC;bf967aba-0de6-11d0-a285-00aa003049e2;;BU)"
        )));
        // An O:/G:/S: part around the DACL doesn't confuse it.
        assert!(others(&format!("O:{ME}G:{ME}D:(A;;FR;;;BU)S:AI")));
        assert!(!others(&format!("O:BUG:BUD:(A;;FA;;;{ME})")));
    }

    #[test]
    fn write_atomic_replaces_and_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.toml");
        write_atomic(&path, b"one", "integrations.toml").unwrap();
        write_atomic(&path, b"two", "integrations.toml").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["integrations.toml"]);
        assert_eq!(privacy(&path), Privacy::Private);
    }

    #[test]
    fn a_leftover_tmp_file_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.toml");
        let tmp = dir.path().join("integrations.toml.tmp");
        fs::write(&tmp, "an older attempt, much longer than what's saved").unwrap();
        make_readable(&tmp);
        write_atomic(&path, b"new", "integrations.toml").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert!(!tmp.exists());
        assert_eq!(privacy(&path), Privacy::Private);
    }

    #[test]
    fn restrict_makes_a_readable_file_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.toml");
        fs::write(&path, "x").unwrap();
        make_readable(&path);
        assert_eq!(privacy(&path), Privacy::ReadableByOthers);
        restrict(&path).unwrap();
        assert_eq!(privacy(&path), Privacy::Private);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "x",
            "still mine to read"
        );
    }

    #[test]
    fn a_failed_restrict_on_a_readable_tmp_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.toml");
        fs::write(&path, "before").unwrap();
        let e = write_atomic_with(&path, b"after", "integrations.toml", |tmp| {
            make_readable(tmp);
            Err(io::Error::other("not supported"))
        })
        .unwrap_err();
        assert_eq!(
            e,
            "Couldn't make integrations.toml private (not supported), so nothing was saved. \
             Keep PerritoDeck's settings folder on a drive that supports permissions."
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "before");
        assert!(!dir.path().join("integrations.toml.tmp").exists());
    }

    #[test]
    fn a_failed_restrict_on_a_private_tmp_still_saves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.toml");
        write_atomic_with(&path, b"after", "integrations.toml", |tmp| {
            restrict(tmp)?;
            Err(io::Error::other("not supported"))
        })
        .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "after");
    }

    #[test]
    fn privacy_of_a_missing_file_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(privacy(&dir.path().join("nope")), Privacy::Unknown);
    }

    /// Let other accounts read `path`, as a file made with the wrong permissions.
    fn make_readable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        }
        #[cfg(windows)]
        {
            let me = imp::current_user_sid().unwrap();
            imp::apply_sddl(path, &format!("D:P(A;;FR;;;BU)(A;;FA;;;{me})")).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn new_and_replaced_files_are_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.toml");
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        write_atomic(&path, b"one", "integrations.toml").unwrap();
        assert_eq!(mode(&path), 0o600);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(privacy(&path), Privacy::ReadableByOthers);
        write_atomic(&path, b"two", "integrations.toml").unwrap();
        assert_eq!(mode(&path), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn writes_through_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.toml");
        let link = dir.path().join("integrations.toml");
        fs::write(&real, "old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_atomic(&link, b"new", "integrations.toml").unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "new");
    }

    #[cfg(windows)]
    #[test]
    fn write_atomic_sets_a_protected_dacl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.toml");
        write_atomic(&path, b"x", "integrations.toml").unwrap();
        let sddl = imp::dacl_sddl(&path).unwrap().expect("a DACL");
        let me = imp::current_user_sid().unwrap();
        assert!(sddl.starts_with("D:P"), "{sddl}");
        let mut sids: Vec<&str> = sddl
            .split('(')
            .skip(1)
            .map(|ace| ace.trim_end_matches(')').rsplit(';').next().unwrap())
            .collect();
        sids.sort();
        let mut want = vec!["BA", "SY", me.as_str()];
        want.sort();
        assert_eq!(sids, want, "{sddl}");
    }

    #[cfg(windows)]
    #[test]
    fn save_reports_a_locked_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.toml");
        fs::write(&path, "before").unwrap();
        let _held = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let e = write_atomic(&path, b"after", "integrations.toml").unwrap_err();
        assert!(e.starts_with("Couldn't save integrations.toml: "), "{e}");
        assert!(e.ends_with(" Is it open in another program?"), "{e}");
        assert!(!dir.path().join("integrations.toml.tmp").exists());
    }
}
