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
///
/// One writer per file at a time: two saves of the same path at once share the `.tmp`
/// name and can fail or clobber each other. Callers serialise their saves (the host
/// holds its integrations lock).
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
    restrict: impl FnOnce(&Path) -> io::Result<()>,
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
        // Until restrict, the file has its folder's inherited DACL, and access is checked
        // only when a file is opened: shared with nobody, no one can open it meanwhile
        // and keep reading after the secret is in.
        #[cfg(windows)]
        std::os::windows::fs::OpenOptionsExt::share_mode(&mut open, 0);
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

/// One entry of a file's DACL, as Windows holds it (read with GetAce, not from SDDL,
/// whose spelling of the same rights varies).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // built on Windows, tested everywhere
pub(crate) struct Ace {
    /// An allow entry (deny entries let nobody in).
    pub allow: bool,
    /// Only for what's made inside a folder, not the file itself.
    pub inherit_only: bool,
    pub mask: u32,
    /// Who it's for, as `S-1-…`; `None` where it can't be told (an object entry), which
    /// counts as someone else.
    pub sid: Option<String>,
}

/// Rights that read a file's contents: FILE_READ_DATA, GENERIC_ALL, GENERIC_READ and
/// MAXIMUM_ALLOWED. READ_CONTROL or SYNCHRONIZE alone don't.
const READS: u32 = 0x1 | 0x1000_0000 | 0x8000_0000 | 0x0200_0000;

/// This account, SYSTEM, Administrators, Creator Owner, Owner Rights, and AppContainer or
/// package SIDs (Windows adds them on AppData a packaged app redirects; they aren't
/// other people).
fn exempt(sid: &str, me: &str) -> bool {
    sid == me
        || matches!(sid, "S-1-5-18" | "S-1-5-32-544" | "S-1-3-0" | "S-1-3-4")
        || sid.starts_with("S-1-15-")
}

/// Whether a DACL lets anyone but `me` (a SID string), SYSTEM and Administrators read
/// the file's contents. A file owned by someone else counts too: its owner can always
/// change the DACL.
#[allow(dead_code)] // used on Windows, tested everywhere
pub(crate) fn lets_others_read(aces: &[Ace], me: &str, owner: Option<&str>) -> bool {
    owner.is_some_and(|o| !exempt(o, me))
        || aces.iter().any(|a| {
            a.allow
                && !a.inherit_only
                && a.mask & READS != 0
                && a.sid.as_deref().map_or(true, |s| !exempt(s, me))
        })
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
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                GetNamedSecurityInfoW, SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
            },
            GetAce, GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE,
            ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, INHERIT_ONLY_ACE,
            OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
            PSID, TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    pub fn restrict(path: &Path) -> io::Result<()> {
        let me = current_user_sid()?;
        apply_sddl(path, &format!("D:P(A;;FA;;;{me})(A;;FA;;;SY)(A;;FA;;;BA)"))
    }

    pub fn privacy(path: &Path) -> Privacy {
        match (read_access(path), current_user_sid()) {
            (Ok(Some((aces, owner))), Ok(me)) if lets_others_read(&aces, &me, owner.as_deref()) => {
                Privacy::ReadableByOthers
            }
            (Ok(Some(_)), Ok(_)) => Privacy::Private,
            // No DACL: a drive that keeps no permissions (FAT, exFAT, some shares).
            _ => Privacy::Unknown,
        }
    }

    // ACE types (winnt.h) whose entries allow access.
    const ACCESS_ALLOWED: u8 = 0;
    const ACCESS_ALLOWED_OBJECT: u8 = 5;
    const ACCESS_ALLOWED_CALLBACK: u8 = 9;
    const ACCESS_ALLOWED_CALLBACK_OBJECT: u8 = 11;

    /// The entries of `path`'s DACL and its owner, or `None` when it has no DACL.
    pub fn read_access(path: &Path) -> io::Result<Option<(Vec<Ace>, Option<String>)>> {
        let name = wide(path);
        let mut dacl: *mut ACL = ptr::null_mut();
        let mut owner: PSID = ptr::null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `name` is NUL-terminated; the out pointers are written on success and
        // point into `sd`.
        let e = unsafe {
            GetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
                &mut owner,
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut sd,
            )
        };
        if e != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(e as i32));
        }
        let _sd = Local(sd);
        if dacl.is_null() {
            return Ok(None);
        }
        let owner = if owner.is_null() {
            None
        } else {
            Some(sid_string(owner)?)
        };
        // SAFETY: `dacl` points at a valid ACL inside `sd`.
        let count = unsafe { (*dacl).AceCount };
        let mut aces = Vec::with_capacity(count as usize);
        for i in 0..u32::from(count) {
            let mut ace: *mut core::ffi::c_void = ptr::null_mut();
            // SAFETY: `i` is below the ACL's entry count; `ace` points into the ACL.
            if unsafe { GetAce(dacl, i, &mut ace) } == 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: every ACE starts with an ACE_HEADER.
            let header = unsafe { &*(ace as *const ACE_HEADER) };
            let kind = header.AceType;
            let allow = matches!(
                kind,
                ACCESS_ALLOWED
                    | ACCESS_ALLOWED_OBJECT
                    | ACCESS_ALLOWED_CALLBACK
                    | ACCESS_ALLOWED_CALLBACK_OBJECT
            );
            let inherit_only = u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0;
            if !allow {
                aces.push(Ace {
                    allow,
                    inherit_only,
                    mask: 0,
                    sid: None,
                });
                continue;
            }
            // SAFETY: allow entries of every kind put the mask right after the header.
            let entry = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
            // The SID follows the mask in plain and callback entries; object entries put
            // object types first, so their SID isn't read and they count as others.
            let sid = if matches!(kind, ACCESS_ALLOWED | ACCESS_ALLOWED_CALLBACK) {
                Some(sid_string(&entry.SidStart as *const u32 as PSID)?)
            } else {
                None
            };
            aces.push(Ace {
                allow,
                inherit_only,
                mask: entry.Mask,
                sid,
            });
        }
        Ok(Some((aces, owner)))
    }

    /// A SID as `S-1-…`.
    fn sid_string(sid: PSID) -> io::Result<String> {
        let mut text: *mut u16 = ptr::null_mut();
        // SAFETY: the caller passes a valid SID; `text` is written on success.
        if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let text = Local(text.cast());
        // SAFETY: ConvertSidToStringSidW returns a NUL-terminated string.
        Ok(unsafe { from_wide(text.0 as *const u16) })
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
        // The SID points into `buf`, alive for this call.
        sid_string(user.User.Sid)
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
    #[cfg(test)]
    pub fn dacl_sddl(path: &Path) -> io::Result<Option<String>> {
        use windows_sys::Win32::Security::Authorization::ConvertSecurityDescriptorToStringSecurityDescriptorW;
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

    const USERS: &str = "S-1-5-32-545";

    fn allow(mask: u32, sid: &str) -> Ace {
        Ace {
            allow: true,
            inherit_only: false,
            mask,
            sid: Some(sid.into()),
        }
    }

    #[test]
    fn who_else_an_acl_lets_read() {
        let others = |aces: &[Ace]| lets_others_read(aces, ME, Some(ME));
        let mine = allow(0x1F01FF, ME);
        // What a file made in %APPDATA% inherits, and what restrict sets: this account,
        // SYSTEM, Administrators.
        assert!(!others(&[
            mine.clone(),
            allow(0x1F01FF, "S-1-5-18"),
            allow(0x1F01FF, "S-1-5-32-544")
        ]));
        // Users, Authenticated Users or Everyone, with any mask that reads the data:
        // FILE_READ_DATA however it's combined (0x20019 prints as KR, 0xF003F as KA),
        // GENERIC_READ, GENERIC_ALL, MAXIMUM_ALLOWED.
        for sid in [USERS, "S-1-5-11", "S-1-1-0"] {
            for mask in [
                0x120089,
                0x1200a9,
                0x20019,
                0xF003F,
                0x1,
                0x8000_0000,
                0x1000_0000,
                0x0200_0000,
            ] {
                assert!(others(&[mine.clone(), allow(mask, sid)]), "{sid} {mask:#x}");
            }
        }
        // Another account.
        let other = "S-1-5-21-1004336348-1177238915-682003330-1002";
        assert!(others(&[mine.clone(), allow(0x1F01FF, other)]));
        // Rights that don't read the contents: SYNCHRONIZE, READ_CONTROL, writing.
        for mask in [0x100000, 0x20000, 0x120116] {
            assert!(!others(&[mine.clone(), allow(mask, USERS)]), "{mask:#x}");
        }
        // Only for what's made inside a folder, not the file itself.
        let inherit_only = Ace {
            inherit_only: true,
            ..allow(0x1F01FF, USERS)
        };
        assert!(!others(&[mine.clone(), inherit_only]));
        // A deny entry lets nobody in.
        let deny = Ace {
            allow: false,
            ..allow(0x1F01FF, USERS)
        };
        assert!(!others(&[deny, mine.clone()]));
        // Creator Owner, Owner Rights, AppContainer and package SIDs.
        for sid in [
            "S-1-3-0",
            "S-1-3-4",
            "S-1-15-2-1",
            "S-1-15-2-1430448594-2639229838",
        ] {
            assert!(!others(&[allow(0x1F01FF, sid)]), "{sid}");
        }
        // An entry whose SID can't be told (an object entry) counts as someone else.
        let unknown = Ace {
            sid: None,
            ..allow(0x1, USERS)
        };
        assert!(others(&[mine.clone(), unknown]));
        // A file someone else owns isn't private, whatever its DACL: the owner can
        // change it. SYSTEM or Administrators owning it is fine.
        let only_mine = std::slice::from_ref(&mine);
        assert!(lets_others_read(only_mine, ME, Some(other)));
        assert!(!lets_others_read(only_mine, ME, Some("S-1-5-32-544")));
        assert!(!lets_others_read(only_mine, ME, None));
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
        assert!(sddl.starts_with("D:P"), "{sddl}");
        // Who the entries are for comes from the ACL itself, not the SDDL, which spells
        // some accounts as two-letter aliases: the built-in Administrator (RID 500, what
        // GitHub's Windows runners log in as) prints as `LA`, not its `S-1-5-21-…-500`.
        let (aces, _) = imp::read_access(&path).unwrap().expect("a DACL");
        let me = imp::current_user_sid().unwrap();
        // FILE_ALL_ACCESS, which SDDL's `FA` stands for.
        const FILE_ALL_ACCESS: u32 = 0x001F_01FF;
        for ace in &aces {
            assert!(ace.allow && !ace.inherit_only, "{ace:?} in {sddl}");
            assert_eq!(ace.mask, FILE_ALL_ACCESS, "{ace:?} in {sddl}");
        }
        let mut sids: Vec<&str> = aces.iter().map(|a| a.sid.as_deref().unwrap()).collect();
        sids.sort();
        let mut want = vec!["S-1-5-18", "S-1-5-32-544", me.as_str()];
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

    /// Read access Windows prints as a registry-style code (Users with 0x20019 reads back
    /// as `KR`) still counts as other accounts reading.
    #[cfg(windows)]
    #[test]
    fn privacy_sees_read_access_however_windows_prints_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.toml");
        fs::write(&path, "secret").unwrap();
        let me = imp::current_user_sid().unwrap();
        for rights in ["0x20019", "0xF003F", "0x1", "0x80000000"] {
            imp::apply_sddl(&path, &format!("D:P(A;;FA;;;{me})(A;;{rights};;;BU)")).unwrap();
            assert_eq!(privacy(&path), Privacy::ReadableByOthers, "{rights}");
        }
        // Rights that don't read the contents.
        imp::apply_sddl(&path, &format!("D:P(A;;FA;;;{me})(A;;0x120000;;;BU)")).unwrap();
        assert_eq!(privacy(&path), Privacy::Private);
    }

    /// While the temporary file is being made private and filled, nobody else can open
    /// it: a handle opened before restrict could read the secret afterwards.
    #[cfg(windows)]
    #[test]
    fn nobody_else_can_open_the_tmp_while_its_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.toml");
        let mut opened = None;
        write_atomic_with(&path, b"secret", "integrations.toml", |tmp| {
            opened = Some(fs::File::open(tmp).map(drop).map_err(|e| e.raw_os_error()));
            restrict(tmp)
        })
        .unwrap();
        assert_eq!(opened, Some(Err(Some(32))), "a sharing violation");
        assert_eq!(fs::read_to_string(&path).unwrap(), "secret");
        assert_eq!(privacy(&path), Privacy::Private);
    }
}
