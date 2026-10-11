//! mDNS advertisement (`_perritodeck._tcp`) so tablets can find the host without
//! typing an address, plus the pairing link shown as a QR code.

use std::net::IpAddr;

use mdns_sd::{ServiceDaemon, ServiceInfo};

pub const SERVICE_TYPE: &str = "_perritodeck._tcp.local.";

/// Keeps the advertisement alive; dropping it withdraws the service.
pub struct Advertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Drop for Advertisement {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// Advertise the host on every LAN interface. TXT: protocol version, host id, name.
pub fn advertise(
    host_name: &str,
    host_id: &str,
    port: u16,
) -> Result<Advertisement, mdns_sd::Error> {
    let daemon = ServiceDaemon::new()?;
    let instance = format!("PerritoDeck on {host_name}");
    let mdns_host = format!("{}.local.", sanitize_label(host_name));
    let props = [("v", "1"), ("id", host_id), ("n", host_name)];
    let info = ServiceInfo::new(SERVICE_TYPE, &instance, &mdns_host, "", port, &props[..])?
        .enable_addr_auto();
    let fullname = info.get_fullname().to_owned();
    daemon.register(info)?;
    Ok(Advertisement { daemon, fullname })
}

fn sanitize_label(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(63)
        .collect();
    if s.is_empty() {
        "perritodeck".into()
    } else {
        s
    }
}

/// Non-loopback IPv4 addresses of this machine, for the pairing link.
pub fn lan_addresses() -> Vec<IpAddr> {
    let mut ips: Vec<IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .map(|i| i.ip())
        .filter(|ip| ip.is_ipv4())
        .collect();
    ips.sort();
    ips.dedup();
    ips
}

/// `perritodeck://pair?v=1&n=<name>&a=<ip:port>…&fp=<fingerprint>&c=<code>`
pub fn pairing_uri(
    host_name: &str,
    addrs: &[IpAddr],
    port: u16,
    fingerprint: &str,
    code: &str,
) -> String {
    let mut uri = format!("perritodeck://pair?v=1&n={}", percent_encode(host_name));
    for a in addrs {
        let host = match a {
            IpAddr::V6(v6) => format!("[{v6}]"),
            IpAddr::V4(v4) => v4.to_string(),
        };
        uri.push_str(&format!("&a={}", percent_encode(&format!("{host}:{port}"))));
    }
    uri.push_str(&format!("&fp={fingerprint}&c={code}"));
    uri
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_format() {
        let uri = pairing_uri(
            "Dev Laptop",
            &["192.168.1.20".parse().unwrap(), "10.0.0.5".parse().unwrap()],
            46710,
            "abc_-123",
            "042913",
        );
        assert_eq!(
            uri,
            "perritodeck://pair?v=1&n=Dev%20Laptop&a=192.168.1.20%3A46710&a=10.0.0.5%3A46710&fp=abc_-123&c=042913"
        );
    }

    #[test]
    fn labels() {
        assert_eq!(sanitize_label("my_pc.lan"), "my-pc-lan");
        assert_eq!(sanitize_label(""), "perritodeck");
    }
}
