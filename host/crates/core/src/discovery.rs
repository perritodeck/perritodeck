//! mDNS advertisement (`_perritodeck._tcp`) so tablets can find the host without
//! typing an address, plus the pairing link shown as a QR code.

use std::net::IpAddr;

use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};

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
    // The host listens on IPv4 only, so don't hand out IPv6 addresses. Android before
    // 14 (and Fire OS) gives the app a single address per host, and when that's an
    // IPv6 one the tablet can't connect.
    daemon.disable_interface(IfKind::IPv6)?;
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

/// The category Windows gave each connected network, like `Ethernet 12: Public`, for
/// the diagnostics report: on a Public network the firewall turns tablets away unless
/// its prompt was answered for Public networks. The network's own name (often the
/// Wi-Fi's) is left out. Empty on other systems, or when Windows doesn't say.
pub fn network_categories() -> Vec<String> {
    #[cfg(windows)]
    {
        windows_network_categories().unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// What Windows Firewall does with tablets connecting to a program on the network types
/// in use now, read from its rules (nothing is changed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Firewall {
    /// It's off, or a rule lets the program (or its port) in. Otherwise a rule blocks
    /// it, or no rule mentions it and Windows turns it away by default.
    pub lets_in: bool,
    /// A network in use is one Windows calls Public.
    pub public: bool,
}

/// Whether Windows Firewall lets tablets reach `exe` on TCP `port`. `None` on other
/// systems, or when Windows doesn't say (another firewall may be in charge then). Takes
/// about a second.
pub fn windows_firewall(exe: &std::path::Path, port: u16) -> Option<Firewall> {
    #[cfg(windows)]
    {
        // Rule fields: Direction 1 = in, Action 0 = block, Protocol 6 = TCP and 256 = any,
        // Profiles and CurrentProfileTypes 1 = domain, 2 = private, 4 = public.
        const SCRIPT: &str = r#"
$fw = New-Object -ComObject HNetCfg.FwPolicy2
$cur = $fw.CurrentProfileTypes
$on = $false
foreach ($p in 1, 2, 4) { if (($cur -band $p) -and $fw.FirewallEnabled($p)) { $on = $true } }
$exe = [Environment]::ExpandEnvironmentVariables($env:PERRITODECK_FW_EXE)
$port = $env:PERRITODECK_FW_PORT
$v = 'none'
if (-not $on) { $v = 'off' } else {
  foreach ($r in $fw.Rules) {
    if (-not $r.Enabled -or $r.Direction -ne 1 -or -not ($r.Profiles -band $cur)) { continue }
    if ($r.Protocol -ne 6 -and $r.Protocol -ne 256) { continue }
    $app = $r.ApplicationName
    if ($app -and [Environment]::ExpandEnvironmentVariables($app) -ne $exe) { continue }
    if ($r.LocalPorts -ne '*' -and ($r.LocalPorts -split ',' | ForEach-Object { $_.Trim() }) -notcontains $port) { continue }
    if (-not $app -and $r.LocalPorts -eq '*') { continue }
    if ($r.Action -eq 0) { $v = 'block'; break }
    $v = 'allow'
  }
}
'{0} {1}' -f $v, $cur
"#;
        let exe = exe.display().to_string();
        let port = port.to_string();
        parse_firewall(&powershell(
            SCRIPT,
            &[("PERRITODECK_FW_EXE", &exe), ("PERRITODECK_FW_PORT", &port)],
        )?)
    }
    #[cfg(not(windows))]
    {
        let _ = (exe, port);
        None
    }
}

#[cfg(any(windows, test))]
fn parse_firewall(out: &str) -> Option<Firewall> {
    let (verdict, profiles) = out
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())?
        .split_once(' ')?;
    let profiles: u32 = profiles.trim().parse().ok()?;
    let lets_in = match verdict {
        "allow" | "off" => true,
        "none" | "block" => false,
        _ => return None,
    };
    Some(Firewall {
        lets_in,
        public: profiles & 4 != 0,
    })
}

#[cfg(windows)]
fn windows_network_categories() -> Option<Vec<String>> {
    let out = powershell(
        "Get-NetConnectionProfile | ForEach-Object { '{0}: {1}' -f $_.InterfaceAlias, $_.NetworkCategory }",
        &[],
    )?;
    Some(parse_categories(&out))
}

/// Runs a Windows PowerShell command and returns what it printed, or `None` if it
/// couldn't start or didn't finish within a few seconds. Values go in through `env`,
/// never quoted into the command.
///
/// Windows PowerShell writes to a pipe in the console's old code page (437, 850, ...),
/// so one adapter called, say, "Conexión de área local" isn't valid UTF-8. The command
/// asks for UTF-8 first, and anything that still isn't is replaced rather than losing
/// the whole answer.
#[cfg(windows)]
fn powershell(command: &str, env: &[(&str, &str)]) -> Option<String> {
    // It normally answers within a second; a report shouldn't wait on it for long.
    powershell_within(command, env, std::time::Duration::from_secs(5))
}

/// [`powershell`], giving up after `timeout`.
#[cfg(windows)]
fn powershell_within(
    command: &str,
    env: &[(&str, &str)],
    timeout: std::time::Duration,
) -> Option<String> {
    use std::io::Read as _;
    use std::os::windows::process::CommandExt as _;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // By full path: a `powershell` earlier on PATH could be anything.
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let exe = std::path::Path::new(&root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let command = format!("[Console]::OutputEncoding = [Text.Encoding]::UTF8; {command}");
    let mut child = Command::new(exe)
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    while child.try_wait().ok()?.is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut out = Vec::new();
    child.stdout.take()?.read_to_end(&mut out).ok()?;
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// Keeps the `<adapter>: <category>` lines, in case anything else was printed.
#[cfg(any(windows, test))]
fn parse_categories(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| {
            ["Public", "Private", "DomainAuthenticated"]
                .iter()
                .any(|c| l.strip_suffix(c).is_some_and(|rest| rest.ends_with(": ")))
        })
        .map(str::to_owned)
        .collect()
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
    fn firewall_answers() {
        let fw = |lets_in, public| Some(Firewall { lets_in, public });
        assert_eq!(parse_firewall("allow 4\r\n"), fw(true, true));
        assert_eq!(parse_firewall("none 4"), fw(false, true));
        assert_eq!(parse_firewall("block 2"), fw(false, false));
        assert_eq!(parse_firewall("off 6"), fw(true, true));
        assert_eq!(parse_firewall(""), None);
        assert_eq!(
            parse_firewall("New-Object : Retrieving the COM class factory failed"),
            None
        );
    }

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
    fn network_categories_are_parsed() {
        let out = "Ethernet 12: Public\r\nWi-Fi: Private\r\n\r\nCorp: DomainAuthenticated\r\nWARNING: something\r\n";
        assert_eq!(
            parse_categories(out),
            [
                "Ethernet 12: Public",
                "Wi-Fi: Private",
                "Corp: DomainAuthenticated"
            ]
        );
        // A name in the console's old code page (here 437's "ó") costs only that letter.
        let out = String::from_utf8_lossy(b"Conexi\xa2n: Public\r\nWi-Fi: Private\r\n");
        assert_eq!(
            parse_categories(&out),
            ["Conexi\u{fffd}n: Public", "Wi-Fi: Private"]
        );
    }

    /// Windows PowerShell's output arrives as UTF-8 whatever the console's code page. It
    /// waits longer than the app does: a cold PowerShell start on a busy CI runner, with the
    /// other tests running, can take more than the app's 5 seconds, and this tests the text.
    #[cfg(windows)]
    #[test]
    fn powershell_output_keeps_accented_letters() {
        let out = powershell_within(
            "'Conexión de área local: Public'",
            &[],
            std::time::Duration::from_secs(60),
        )
        .unwrap();
        assert_eq!(out.trim(), "Conexión de área local: Public");
    }

    /// Asks Windows for real (read-only): whatever it says is in the expected shape.
    #[cfg(windows)]
    #[test]
    fn network_categories_on_this_machine() {
        for line in network_categories() {
            assert_eq!(parse_categories(&line), std::slice::from_ref(&line));
        }
    }

    #[test]
    fn labels() {
        assert_eq!(sanitize_label("my_pc.lan"), "my-pc-lan");
        assert_eq!(sanitize_label(""), "perritodeck");
    }
}
