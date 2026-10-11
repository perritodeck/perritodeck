//! One-time pairing codes and the list of paired devices.
//!
//! A pairing offer is a random 6-digit code valid for a few minutes. Redeeming it
//! issues the device a 256-bit secret; the host keeps only its SHA-256. Every failed
//! attempt burns one of a small number of tries on every open offer, so guessing a
//! code over the network is hopeless (5 tries against a million codes).

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::identity::write_private;

pub const DEFAULT_OFFER_TTL: Duration = Duration::from_secs(5 * 60);
pub const MAX_ATTEMPTS: u8 = 5;
const MAX_NAME_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedDevice {
    pub name: String,
    secret_sha256: String,
    pub paired_at: u64,
    #[serde(default)]
    pub last_seen: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PairError {
    #[error("that code is wrong or has expired; start pairing again on the computer")]
    InvalidCode,
    #[error("deviceId must be 1–128 characters")]
    BadDeviceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfferInfo {
    pub expires_in: Duration,
}

struct Offer {
    code: String,
    expires: Instant,
    attempts_left: u8,
}

#[derive(Default, Serialize, Deserialize)]
struct DeviceFile {
    #[serde(default)]
    devices: BTreeMap<String, PairedDevice>,
}

struct Inner {
    devices: BTreeMap<String, PairedDevice>,
    offers: Vec<Offer>,
}

pub struct PairingStore {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hash_secret(secret: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(secret.as_bytes()))
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl PairingStore {
    /// Devices are kept only in memory (tests, `--insecure` runs).
    pub fn in_memory() -> Self {
        Self {
            path: None,
            inner: Mutex::new(Inner {
                devices: BTreeMap::new(),
                offers: Vec::new(),
            }),
        }
    }

    /// Load paired devices from `path` (created on first pairing).
    pub fn open(path: &Path) -> io::Result<Self> {
        let devices = match std::fs::read(path) {
            Ok(bytes) => {
                serde_json::from_slice::<DeviceFile>(&bytes)
                    .map_err(|e| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("{}: {e}", path.display()),
                        )
                    })?
                    .devices
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            path: Some(path.to_owned()),
            inner: Mutex::new(Inner {
                devices,
                offers: Vec::new(),
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("pairing store poisoned")
    }

    fn save(&self, inner: &Inner) {
        let Some(path) = &self.path else { return };
        let file = DeviceFile {
            devices: inner.devices.clone(),
        };
        let json = serde_json::to_vec_pretty(&file).expect("serialisable");
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = write_private(path, &json) {
            tracing::error!("could not save paired devices to {}: {e}", path.display());
        }
    }

    /// Open a new pairing window and return its 6-digit code.
    pub fn new_offer(&self, ttl: Duration) -> String {
        let code = format!("{:06}", rand::rng().random_range(0..1_000_000u32));
        self.add_offer(code.clone(), ttl);
        code
    }

    /// Open a pairing window with a chosen code (CI and automated tests only).
    pub fn add_offer(&self, code: String, ttl: Duration) {
        let mut inner = self.lock();
        inner.offers.retain(|o| o.expires > Instant::now());
        inner.offers.push(Offer {
            code,
            expires: Instant::now() + ttl,
            attempts_left: MAX_ATTEMPTS,
        });
    }

    /// Whether any pairing window is open, and for how long.
    pub fn open_offer(&self) -> Option<OfferInfo> {
        let now = Instant::now();
        let inner = self.lock();
        inner
            .offers
            .iter()
            .filter(|o| o.expires > now)
            .map(|o| OfferInfo {
                expires_in: o.expires - now,
            })
            .max_by_key(|o| o.expires_in)
    }

    /// Redeem a code. On success the offer is consumed and the device's secret returned.
    pub fn redeem(
        &self,
        code: &str,
        device_id: &str,
        device_name: &str,
    ) -> Result<String, PairError> {
        if device_id.is_empty() || device_id.len() > MAX_NAME_LEN {
            return Err(PairError::BadDeviceId);
        }
        let mut inner = self.lock();
        let now = Instant::now();
        inner
            .offers
            .retain(|o| o.expires > now && o.attempts_left > 0);
        let hit = inner
            .offers
            .iter()
            .position(|o| ct_eq(o.code.as_bytes(), code.as_bytes()));
        let Some(i) = hit else {
            for o in &mut inner.offers {
                o.attempts_left = o.attempts_left.saturating_sub(1);
            }
            inner.offers.retain(|o| o.attempts_left > 0);
            return Err(PairError::InvalidCode);
        };
        inner.offers.remove(i);

        let mut raw = [0u8; 32];
        rand::rng().fill(&mut raw);
        let secret = URL_SAFE_NO_PAD.encode(raw);
        let name: String = device_name.chars().take(MAX_NAME_LEN).collect();
        inner.devices.insert(
            device_id.to_owned(),
            PairedDevice {
                name,
                secret_sha256: hash_secret(&secret),
                paired_at: now_unix(),
                last_seen: None,
            },
        );
        self.save(&inner);
        Ok(secret)
    }

    /// Check a device's secret. Updates its last-seen time on success.
    pub fn verify(&self, device_id: &str, secret: &str) -> bool {
        let mut inner = self.lock();
        let Some(d) = inner.devices.get_mut(device_id) else {
            return false;
        };
        if !ct_eq(d.secret_sha256.as_bytes(), hash_secret(secret).as_bytes()) {
            return false;
        }
        d.last_seen = Some(now_unix());
        self.save(&inner);
        true
    }

    pub fn devices(&self) -> Vec<(String, PairedDevice)> {
        self.lock()
            .devices
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Give a paired device a new name (shown in the app). False if it isn't paired.
    pub fn rename(&self, device_id: &str, name: &str) -> bool {
        let mut inner = self.lock();
        let Some(d) = inner.devices.get_mut(device_id) else {
            return false;
        };
        d.name = name.to_owned();
        self.save(&inner);
        true
    }

    /// Forget a device. Returns false if it wasn't paired.
    pub fn revoke(&self, device_id: &str) -> bool {
        let mut inner = self.lock();
        let removed = inner.devices.remove(device_id).is_some();
        if removed {
            self.save(&inner);
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_verify_revoke() {
        let s = PairingStore::in_memory();
        let code = s.new_offer(DEFAULT_OFFER_TTL);
        assert_eq!(code.len(), 6);
        assert!(s.open_offer().is_some());
        let secret = s.redeem(&code, "dev", "Kindle").unwrap();
        assert!(s.open_offer().is_none(), "offers are single use");
        assert_eq!(s.redeem(&code, "dev2", "x"), Err(PairError::InvalidCode));
        assert!(s.verify("dev", &secret));
        assert!(!s.verify("dev", "wrong"));
        assert!(!s.verify("other", &secret));
        assert!(s.devices()[0].1.last_seen.is_some());
        assert!(s.rename("dev", "Desk tablet"));
        assert_eq!(s.devices()[0].1.name, "Desk tablet");
        assert!(!s.rename("nobody", "x"));
        assert!(s.revoke("dev"));
        assert!(!s.verify("dev", &secret));
        assert!(!s.revoke("dev"));
    }

    #[test]
    fn wrong_guesses_burn_the_offer() {
        let s = PairingStore::in_memory();
        s.add_offer("123456".into(), DEFAULT_OFFER_TTL);
        for _ in 0..MAX_ATTEMPTS {
            assert_eq!(s.redeem("000000", "dev", "x"), Err(PairError::InvalidCode));
        }
        assert_eq!(
            s.redeem("123456", "dev", "x"),
            Err(PairError::InvalidCode),
            "offer is gone after too many guesses"
        );
    }

    #[test]
    fn expired_offers_do_not_work() {
        let s = PairingStore::in_memory();
        s.add_offer("123456".into(), Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(s.redeem("123456", "dev", "x"), Err(PairError::InvalidCode));
    }

    #[test]
    fn devices_persist_without_plain_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        let s = PairingStore::open(&path).unwrap();
        s.add_offer("654321".into(), DEFAULT_OFFER_TTL);
        let secret = s.redeem("654321", "tab", "Tablet").unwrap();
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains(&secret), "only the hash is stored");
        let reopened = PairingStore::open(&path).unwrap();
        assert!(reopened.verify("tab", &secret));
        assert_eq!(reopened.devices()[0].1.name, "Tablet");
    }
}
