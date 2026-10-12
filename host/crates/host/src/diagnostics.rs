//! "Copy diagnostics" (the app) and `perritodeck-host diagnostics` (the command line):
//! a plain-text report to paste into a bug report. It never includes keys,
//! certificates, tokens or passwords, and the log lines in it are redacted.

use std::{fmt::Write as _, path::Path};

use crate::{logging, mcp, settings::AppSettings};

/// How many log lines a report ends with.
pub const LOG_LINES: usize = 200;

/// What goes into a report.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// The desktop app's version, when the report comes from the app.
    pub app_version: Option<String>,
    pub host_version: String,
    pub sections: Vec<Section>,
    pub log: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Section {
    pub title: String,
    pub lines: Vec<String>,
}

impl Section {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            lines: Vec::new(),
        }
    }

    pub fn line(&mut self, text: impl Into<String>) -> &mut Self {
        self.lines.push(text.into());
        self
    }

    /// `label: value`, aligned.
    pub fn field(&mut self, label: &str, value: impl std::fmt::Display) -> &mut Self {
        self.lines.push(field(label, value));
        self
    }
}

/// `label: value` with the values lined up.
pub fn field(label: &str, value: impl std::fmt::Display) -> String {
    format!("{:<16}{value}", format!("{label}:"))
}

impl Report {
    /// The report as text. Every line is redacted, not just the log.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "PerritoDeck diagnostics");
        let _ = writeln!(out, "=======================");
        for section in &self.sections {
            let _ = writeln!(out);
            let _ = writeln!(out, "## {}", section.title);
            if section.lines.is_empty() {
                let _ = writeln!(out, "(none)");
            }
            for line in &section.lines {
                let _ = writeln!(out, "{}", logging::redact(line));
            }
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "## Last {} log lines", self.log.len());
        if self.log.is_empty() {
            let _ = writeln!(out, "(no log yet)");
        }
        for line in &self.log {
            let _ = writeln!(out, "{}", logging::redact(line));
        }
        out
    }
}

/// The "Versions" section: app, host, protocols, OS and desktop session.
pub fn versions(app_version: Option<&str>) -> Section {
    let mut s = Section::new("Versions");
    if let Some(v) = app_version {
        s.field("app", v);
    }
    s.field("host", env!("CARGO_PKG_VERSION"));
    s.field(
        "protocol",
        format!("tablet protocol {}", perritodeck_protocol::PROTOCOL_VERSION),
    );
    s.field(
        "mcp protocol",
        perritodeck_mcp::MODERN_VERSIONS
            .iter()
            .chain(perritodeck_mcp::LEGACY_VERSIONS)
            .copied()
            .collect::<Vec<_>>()
            .join(", "),
    );
    s.field(
        "os",
        sysinfo::System::long_os_version().unwrap_or_else(|| std::env::consts::OS.to_owned()),
    );
    s.field("arch", std::env::consts::ARCH);
    if cfg!(target_os = "linux") {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let session = var("XDG_SESSION_TYPE").unwrap_or_else(|| {
            if var("WAYLAND_DISPLAY").is_some() {
                "wayland".into()
            } else if var("DISPLAY").is_some() {
                "x11".into()
            } else {
                "none".into()
            }
        });
        let desktop = var("XDG_CURRENT_DESKTOP").unwrap_or_else(|| "unknown".into());
        s.field("session", format!("{session} ({desktop})"));
    }
    s
}

/// The "Paths" section.
pub fn paths(data_dir: &Path, profile: &Path) -> Section {
    let mut s = Section::new("Paths");
    s.field("data folder", data_dir.display());
    s.field("profile", profile.display());
    s.field("logs", logging::logs_dir(data_dir).display());
    s.field("plugins", data_dir.join("plugins").display());
    s.field("sd plugins", data_dir.join("sdplugins").display());
    s
}

/// The "Network" section from the saved settings (and the address in use, if known).
pub fn network(settings: &AppSettings, listening: Option<&str>, usb: &str) -> Section {
    let mut s = Section::new("Network");
    s.field("saved port", settings.network.port);
    s.field(
        "wi-fi",
        if settings.network.lan {
            "tablets may connect over the network"
        } else {
            "USB and this computer only"
        },
    );
    if let Some(l) = listening {
        s.field("listening", l);
    }
    s.field("usb", usb);
    s
}

/// The "AI assistants (MCP)" section. Never includes the token.
pub fn mcp_section(settings: &AppSettings, state: &str, has_token: bool) -> Section {
    let m = &settings.mcp;
    let on = |b: bool| if b { "on" } else { "off" };
    let mut s = Section::new("AI assistants (MCP)");
    s.field("enabled", on(m.enabled));
    s.field("state", state);
    s.field("port", m.port);
    s.field(
        "permissions",
        format!(
            "press {}, edit {}, run {}",
            on(m.press),
            on(m.edit),
            on(m.run)
        ),
    );
    s.field(
        "token file",
        if has_token { "exists" } else { "not made yet" },
    );
    s
}

/// The "Integrations" section from `integrations.toml`: what's set up, never the
/// addresses or secrets.
pub fn integrations_from_file(data_dir: &Path) -> Section {
    let mut s = Section::new("Integrations");
    match perritodeck_integrations::Config::load(&crate::integrations_path(data_dir)) {
        Ok(c) => {
            let set = |b: bool| if b { "set up" } else { "not set up" };
            s.field("OBS", set(c.obs.is_some()));
            s.field("Home Assistant", set(c.home_assistant.is_some()));
            s.field("Companion", set(c.companion.is_some()));
        }
        Err(_) => {
            s.line("integrations.toml has a mistake and is ignored");
        }
    }
    s
}

/// A report made without a running host (the command line): what the files say.
pub fn offline(data_dir: &Path, profile: &Path) -> Report {
    let settings = AppSettings::load(&crate::settings_path(data_dir));
    let mut sections = vec![versions(None), paths(data_dir, profile)];

    let mut tablets = Section::new("Tablets (paired)");
    match perritodeck_core::PairingStore::open(&crate::devices_path(data_dir)) {
        Ok(store) => {
            for (id, d) in store.devices() {
                let profile = settings
                    .device_profiles
                    .get(&id)
                    .map(String::as_str)
                    .unwrap_or("default");
                tablets.line(format!(
                    "{} ({}), profile {profile}, last seen {}",
                    d.name,
                    short_id(&id),
                    d.last_seen
                        .map(|s| format!("{s} (unix)"))
                        .unwrap_or_else(|| "never".into())
                ));
            }
        }
        Err(e) => {
            tablets.line(format!("can't read the paired tablets: {e}"));
        }
    }
    tablets.line(
        "Connection details are known only to the running host (use Copy diagnostics in the app).",
    );
    sections.push(tablets);

    let mut plugins = Section::new("Plugins (installed)");
    for (id, version) in plugin_manifests(&data_dir.join("plugins")) {
        plugins.line(format!("{id} {version}"));
    }
    sections.push(plugins);
    let mut sd = Section::new("Stream Deck plugins (installed)");
    for (uuid, version) in sd_manifests(&data_dir.join("sdplugins")) {
        sd.line(format!("{uuid} {version}"));
    }
    sections.push(sd);
    sections.push(integrations_from_file(data_dir));
    sections.push(network(&settings, None, "see the running host"));
    let running = std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], settings.mcp.port)),
        std::time::Duration::from_millis(300),
    )
    .is_ok();
    let state = match (settings.mcp.enabled, running) {
        (false, _) => "off",
        (true, true) => "something answers on its port (the host, probably)",
        (true, false) => "on, but nothing answers on its port (is the host running?)",
    };
    sections.push(mcp_section(
        &settings,
        state,
        mcp::read_token(data_dir).is_some(),
    ));
    Report {
        app_version: None,
        host_version: env!("CARGO_PKG_VERSION").to_owned(),
        sections,
        log: logging::tail(&logging::logs_dir(data_dir), LOG_LINES),
    }
}

/// The first characters of a device id, enough to tell tablets apart.
pub fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn plugin_manifests(dir: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let text =
                std::fs::read_to_string(e.path().join(perritodeck_plugins::MANIFEST)).ok()?;
            let m = perritodeck_plugins::Manifest::parse(&text).ok()?;
            Some((m.id, m.version))
        })
        .collect();
    out.sort();
    out
}

fn sd_manifests(dir: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let m = perritodeck_sdcompat::Manifest::read(&e.path()).ok()?;
            Some((m.uuid, m.version))
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_have_sections_and_redact_everything() {
        let mut s = Section::new("Tablets");
        s.field("Studio", "connected");
        s.line("a url http://me:pw@example.com and token=abc");
        let r = Report {
            app_version: Some("0.1.0".into()),
            host_version: "0.1.0".into(),
            sections: vec![versions(Some("0.1.0")), s, Section::new("Plugins")],
            log: vec![
                "INFO started".into(),
                "WARN sent Authorization: Bearer 0123456789abcdef0123456789abcdef".into(),
            ],
        };
        let text = r.render();
        assert!(text.starts_with("PerritoDeck diagnostics\n"));
        assert!(text.contains(&format!("## Versions\n{}\nhost:", field("app", "0.1.0"))));
        assert!(text.contains(&field("arch", std::env::consts::ARCH)));
        assert!(text.contains("tablet protocol 1"));
        assert!(text.contains("2026-07-28"));
        assert!(text.contains(&field("Studio", "connected")));
        assert!(text.contains("## Plugins\n(none)"));
        assert!(text.contains("## Last 2 log lines\nINFO started\n"));
        assert!(!text.contains("pw@"), "{text}");
        assert!(!text.contains("abc\n"), "{text}");
        assert!(!text.contains("0123456789abcdef"), "{text}");
        assert!(text.contains("[redacted]"));
    }

    #[test]
    fn offline_reports_read_the_files_without_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        std::fs::write(
            crate::integrations_path(data),
            "[home_assistant]\nurl = \"http://ha.local:8123\"\ntoken = \"sekrit-ha-token\"\n",
        )
        .unwrap();
        let mut settings = AppSettings::default();
        settings.mcp.enabled = true;
        settings.mcp.port = 1; // nothing listens there
        settings.save(&crate::settings_path(data)).unwrap();
        let token = mcp::new_token(data).unwrap();
        std::fs::create_dir_all(logging::logs_dir(data)).unwrap();
        std::fs::write(
            logging::logs_dir(data).join(logging::LOG_FILE),
            format!("INFO hello\nDEBUG token={token}\n"),
        )
        .unwrap();

        let text = offline(data, &crate::default_profile_path(data)).render();
        assert!(text.contains(&field("Home Assistant", "set up")), "{text}");
        assert!(text.contains(&field("OBS", "not set up")), "{text}");
        assert!(text.contains("nothing answers on its port"), "{text}");
        assert!(text.contains(&field("token file", "exists")), "{text}");
        assert!(text.contains("INFO hello"), "{text}");
        assert!(!text.contains(&token), "{text}");
        assert!(!text.contains("sekrit"), "{text}");
        assert!(!text.contains("ha.local"), "{text}");
        assert!(text.contains(&logging::logs_dir(data).display().to_string()));
    }
}
