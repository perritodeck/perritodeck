//! The online plugin catalogue: an `index.json` listing plugins as zip files, each with
//! its SHA-256. The app fetches it only when someone asks to look online.
//!
//! ```json
//! { "version": 1,
//!   "plugins": [ { "id": "dev.example.weather", "name": "Weather", "version": "1.2.0",
//!                  "description": "…", "author": "…",
//!                  "url": "dev.example.weather-1.2.0.zip", "sha256": "…" } ] }
//! ```
//!
//! `url` may be relative to the index. A download whose hash doesn't match is refused.

use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Where the app looks unless told otherwise.
pub const DEFAULT_INDEX: &str = "https://perritodeck.dev/plugins/index.json";
/// Biggest plugin zip the app downloads.
pub const MAX_ZIP_BYTES: u64 = 25 * 1024 * 1024;
const MAX_INDEX_BYTES: u64 = 2 * 1024 * 1024;
/// Biggest a plugin may get once unpacked, and how many files it may have.
const MAX_UNPACKED_BYTES: u64 = 100 * 1024 * 1024;
const MAX_FILES: usize = 5_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlinePlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    pub url: String,
    pub sha256: String,
}

#[derive(Deserialize)]
struct Index {
    version: u32,
    plugins: Vec<OnlinePlugin>,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// Only HTTPS, except plain HTTP to this computer (for testing a catalogue locally).
fn allowed(url: &str) -> bool {
    url.starts_with("https://")
        || ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"]
            .iter()
            .any(|p| url.starts_with(p))
}

fn get(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    if !allowed(url) {
        return Err(format!("{url} isn't an https:// address."));
    }
    let mut resp = agent()
        .get(url)
        .header("User-Agent", "PerritoDeck")
        .call()
        .map_err(|e| format!("Couldn't reach {url}: {e}"))?;
    let code = resp.status().as_u16();
    if code != 200 {
        return Err(format!("{url} answered {code}."));
    }
    let mut body = Vec::new();
    resp.body_mut()
        .as_reader()
        .take(limit + 1)
        .read_to_end(&mut body)
        .map_err(|e| format!("Download from {url} failed: {e}"))?;
    if body.len() as u64 > limit {
        return Err(format!("{url} is too big."));
    }
    Ok(body)
}

/// Read the catalogue at `index_url`.
pub fn fetch_index(index_url: &str) -> Result<Vec<OnlinePlugin>, String> {
    let body = get(index_url, MAX_INDEX_BYTES)?;
    parse_index(&body)
}

fn parse_index(body: &[u8]) -> Result<Vec<OnlinePlugin>, String> {
    let index: Index = serde_json::from_slice(body)
        .map_err(|e| format!("The plugin catalogue couldn't be read: {e}"))?;
    if index.version != 1 {
        return Err("The plugin catalogue needs a newer PerritoDeck.".into());
    }
    Ok(index.plugins)
}

/// `url` relative to the index's address (or absolute).
pub fn resolve(index_url: &str, url: &str) -> String {
    if url.contains("://") {
        return url.to_owned();
    }
    let base = index_url.rsplit_once('/').map_or(index_url, |(b, _)| b);
    format!("{base}/{}", url.trim_start_matches('/'))
}

/// Download a plugin's zip and check its hash.
pub fn download(index_url: &str, entry: &OnlinePlugin) -> Result<Vec<u8>, String> {
    let bytes = get(&resolve(index_url, &entry.url), MAX_ZIP_BYTES)?;
    verify(&bytes, &entry.sha256)?;
    Ok(bytes)
}

fn verify(bytes: &[u8], sha256: &str) -> Result<(), String> {
    let got: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if !got.eq_ignore_ascii_case(sha256.trim()) {
        return Err("The download doesn't match the catalogue (its SHA-256 differs), so it wasn't installed.".into());
    }
    Ok(())
}

/// Unpack a plugin zip into `dest` (which must be empty or missing) and return the folder
/// that holds its manifest: `dest` itself, or the zip's single top folder.
pub fn unpack(zip_bytes: &[u8], dest: &Path) -> Result<PathBuf, String> {
    let bad = |e: zip::result::ZipError| format!("The plugin download isn't a valid zip: {e}");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(bad)?;
    if zip.len() > MAX_FILES {
        return Err("The plugin has too many files.".into());
    }
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(bad)?;
        // `enclosed_name` rejects absolute paths and `..`, so nothing lands outside `dest`.
        let Some(rel) = file.enclosed_name() else {
            return Err(format!(
                "The plugin contains an unsafe path: {}",
                file.name()
            ));
        };
        let out = dest.join(rel);
        if file.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if file.is_symlink() {
            return Err("The plugin contains a symbolic link, which isn't allowed.".into());
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut f = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        let left = MAX_UNPACKED_BYTES.saturating_sub(total);
        let n =
            std::io::copy(&mut (&mut file).take(left + 1), &mut f).map_err(|e| e.to_string())?;
        total += n;
        if total > MAX_UNPACKED_BYTES {
            return Err("The plugin is too big once unpacked.".into());
        }
        #[cfg(unix)]
        if let Some(mode) = file.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            // Keep "executable" for exec plugins; nothing else (no setuid and the like).
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o755));
        }
    }
    if dest.join(perritodeck_plugins::MANIFEST).is_file() {
        return Ok(dest.to_owned());
    }
    let mut dirs = std::fs::read_dir(dest)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir());
    match (dirs.next(), dirs.next()) {
        (Some(only), None) if only.path().join(perritodeck_plugins::MANIFEST).is_file() => {
            Ok(only.path())
        }
        _ => Err(format!(
            "The download has no {}; it isn't a PerritoDeck plugin.",
            perritodeck_plugins::MANIFEST
        )),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    pub(crate) fn plugin_zip(top: &str, id: &str) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut out);
            let opts = zip::write::SimpleFileOptions::default();
            z.start_file(format!("{top}perritodeck-plugin.json"), opts)
                .unwrap();
            write!(
                z,
                r#"{{"id":"{id}","name":"T","version":"2.0.0","runtime":"python","main":"plugin.py"}}"#
            )
            .unwrap();
            z.start_file(format!("{top}plugin.py"), opts).unwrap();
            z.write_all(b"print('hi')\n").unwrap();
            z.finish().unwrap();
        }
        out.into_inner()
    }

    #[test]
    fn index_and_urls() {
        let plugins = parse_index(
            br#"{"version":1,"plugins":[{"id":"a.b","name":"A","version":"1.0.0","url":"a.b-1.0.0.zip","sha256":"00"}]}"#,
        )
        .unwrap();
        assert_eq!(plugins[0].id, "a.b");
        assert!(parse_index(br#"{"version":2,"plugins":[]}"#).is_err());
        assert_eq!(
            resolve("https://perritodeck.dev/plugins/index.json", "a.zip"),
            "https://perritodeck.dev/plugins/a.zip"
        );
        assert_eq!(
            resolve("https://x/i.json", "https://y/a.zip"),
            "https://y/a.zip"
        );
        assert!(allowed("https://x") && allowed("http://127.0.0.1:8000/i.json"));
        assert!(!allowed("http://example.com/i.json") && !allowed("file:///etc/passwd"));
    }

    #[test]
    fn hashes_are_checked() {
        let bytes = b"zip";
        let good: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert!(verify(bytes, &good.to_uppercase()).is_ok());
        assert!(verify(bytes, "00").is_err());
    }

    #[test]
    fn unpacks_flat_or_in_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let flat = unpack(&plugin_zip("", "a.b"), &dir.path().join("1")).unwrap();
        assert!(flat.join("plugin.py").is_file());
        let nested = unpack(&plugin_zip("weather/", "a.b"), &dir.path().join("2")).unwrap();
        assert!(nested.ends_with("weather"));
        assert!(unpack(b"not a zip", &dir.path().join("3")).is_err());
    }

    #[test]
    fn refuses_paths_outside() {
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut out);
            z.start_file("../evil.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(b"x").unwrap();
            z.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let err = unpack(&out.into_inner(), &dir.path().join("p")).unwrap_err();
        assert!(err.contains("unsafe path"), "{err}");
        assert!(!dir.path().join("evil.txt").exists());
    }
}
