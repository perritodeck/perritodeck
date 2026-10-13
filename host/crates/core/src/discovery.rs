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

/// This machine's IPv4 addresses a tablet could use, for the pairing link. The address
/// of the default-route interface comes first: the tablet tries them in order with a
/// timeout each, and on Windows the WSL and Hyper-V adapters (172.16–31.x) would
/// otherwise sort before a 192.168.x Wi-Fi address and cost seconds per pairing.
///
/// Cheap enough to call whenever the addresses are shown: they change when a laptop
/// moves to another network or Wi-Fi comes up after the host started.
pub fn lan_addresses() -> Vec<IpAddr> {
    let ips = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .map(|i| i.ip())
        .collect();
    order_addresses(ips, default_route_ipv4())
}

/// Keeps the addresses a tablet could use: IPv4, and neither loopback nor link-local
/// (169.254.x, which Windows gives an adapter that found no DHCP server). Sorted and
/// deduplicated, with `first` moved to the front when it's in the list.
fn order_addresses(ips: Vec<IpAddr>, first: Option<IpAddr>) -> Vec<IpAddr> {
    let mut ips: Vec<IpAddr> = ips
        .into_iter()
        .filter(|ip| matches!(ip, IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_link_local()))
        .collect();
    ips.sort();
    ips.dedup();
    if let Some(pos) = first.and_then(|f| ips.iter().position(|ip| *ip == f)) {
        let ip = ips.remove(pos);
        ips.insert(0, ip);
    }
    ips
}

/// The local address the OS would use to reach the internet. "Connecting" a UDP socket
/// only picks a route and a source address; no packet is sent. 192.0.2.1 is a
/// documentation address (RFC 5737), so it never belongs to anyone's server, and it
/// goes through the default route like any public address.
fn default_route_ipv4() -> Option<IpAddr> {
    let sock = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    sock.connect(("192.0.2.1", 9)).ok()?;
    let ip = sock.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
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
    fn default_route_address_comes_first() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let all = vec![ip("192.168.1.20"), ip("172.24.0.1"), ip("172.24.0.1")];
        assert_eq!(
            order_addresses(all.clone(), Some(ip("192.168.1.20"))),
            [ip("192.168.1.20"), ip("172.24.0.1")]
        );
        assert_eq!(
            order_addresses(all.clone(), None),
            [ip("172.24.0.1"), ip("192.168.1.20")]
        );
        // Not one of ours (say, a VPN with no listed adapter): plain order.
        assert_eq!(
            order_addresses(all, Some(ip("10.0.0.5"))),
            [ip("172.24.0.1"), ip("192.168.1.20")]
        );
    }

    #[test]
    fn only_addresses_a_tablet_can_use() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let all = vec![
            ip("169.254.221.130"),
            ip("192.168.1.60"),
            ip("127.0.0.1"),
            ip("fd24:3c13:226:491f::5"),
            ip("fe80::1"),
            ip("172.17.0.1"),
        ];
        assert_eq!(
            order_addresses(all, Some(ip("192.168.1.60"))),
            [ip("192.168.1.60"), ip("172.17.0.1")]
        );
        // A link-local default route (an adapter with no DHCP) isn't put first or at all.
        assert_eq!(
            order_addresses(
                vec![ip("169.254.1.2"), ip("172.17.0.1")],
                Some(ip("169.254.1.2"))
            ),
            [ip("172.17.0.1")]
        );
    }

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
