//! The host's long-lived TLS identity. Clients pin the SHA-256 fingerprint of this
//! certificate when they pair, so it must survive restarts.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};
use tokio_rustls::TlsAcceptor;

const CERT_FILE: &str = "host-cert.der";
const KEY_FILE: &str = "host-key.der";

#[derive(Clone)]
pub struct HostIdentity {
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
    fingerprint: String,
}

impl HostIdentity {
    /// A new self-signed ECDSA P-256 certificate (not saved).
    pub fn generate() -> Self {
        let key = rcgen::KeyPair::generate().expect("P-256 key generation");
        let params = rcgen::CertificateParams::new(vec!["perritodeck-host".to_owned()])
            .expect("static subject name is valid");
        let cert = params
            .self_signed(&key)
            .expect("self-signing a fresh certificate");
        Self::from_parts(cert.der().to_vec(), key.serialize_der())
    }

    /// Load from `dir`, or create and save a new identity there.
    pub fn load_or_create(dir: &Path) -> io::Result<Self> {
        let (cert_path, key_path) = (dir.join(CERT_FILE), dir.join(KEY_FILE));
        if cert_path.exists() && key_path.exists() {
            let id = Self::from_parts(fs::read(&cert_path)?, fs::read(&key_path)?);
            // Fail early on a corrupt file rather than on the first connection.
            id.server_config().map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: {e}", cert_path.display()),
                )
            })?;
            return Ok(id);
        }
        fs::create_dir_all(dir)?;
        let id = Self::generate();
        write_private(&key_path, &id.key_der)?;
        fs::write(&cert_path, &id.cert_der)?;
        Ok(id)
    }

    fn from_parts(cert_der: Vec<u8>, key_der: Vec<u8>) -> Self {
        let fingerprint = fingerprint_of(&cert_der);
        Self {
            cert_der,
            key_der,
            fingerprint,
        }
    }

    /// base64url (no padding) SHA-256 of the certificate DER. This is the host id.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn cert_der(&self) -> &[u8] {
        &self.cert_der
    }

    fn server_config(&self) -> Result<rustls::ServerConfig, rustls::Error> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(self.cert_der.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_der.clone())),
            )
    }

    pub fn tls_acceptor(&self) -> TlsAcceptor {
        TlsAcceptor::from(Arc::new(
            self.server_config().expect("identity validated on load"),
        ))
    }

    pub fn paths(dir: &Path) -> (PathBuf, PathBuf) {
        (dir.join(CERT_FILE), dir.join(KEY_FILE))
    }
}

pub fn fingerprint_of(cert_der: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(cert_der))
}

/// Short form people compare by eye during manual pairing: first 6 bytes as hex,
/// e.g. `3FA9 07C2 D41E`. The Android client computes the same string.
pub fn display_fingerprint(fingerprint: &str) -> String {
    let bytes = URL_SAFE_NO_PAD.decode(fingerprint).unwrap_or_default();
    let hex: String = bytes.iter().take(6).map(|b| format!("{b:02X}")).collect();
    hex.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Write a secret file readable only by the current user (on Unix).
pub(crate) fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        #[cfg(unix)]
        let mut f = {
            use std::os::unix::fs::OpenOptionsExt;
            fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?
        };
        #[cfg(not(unix))]
        let mut f = fs::File::create(&tmp)?;
        io::Write::write_all(&mut f, data)?;
        f.sync_all()?;
    }
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_and_reloads_same_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let a = HostIdentity::load_or_create(dir.path()).unwrap();
        let b = HostIdentity::load_or_create(dir.path()).unwrap();
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_eq!(a.fingerprint().len(), 43);
        assert_ne!(a.fingerprint(), HostIdentity::generate().fingerprint());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o077,
                0,
                "private key must not be group/world readable"
            );
        }
    }

    #[test]
    fn corrupt_identity_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(CERT_FILE), b"nope").unwrap();
        fs::write(dir.path().join(KEY_FILE), b"nope").unwrap();
        assert!(HostIdentity::load_or_create(dir.path()).is_err());
    }

    #[test]
    fn display_form() {
        // SHA-256 of nothing starts E3B0C442 98FC.
        assert_eq!(display_fingerprint(&fingerprint_of(b"")), "E3B0 C442 98FC");
    }
}
