//! Live values in key titles: `{time}`, `{time_s}`, `{date}`, and values the host
//! keeps up to date such as `{cpu}`, `{mem}`, `{ha:sensor.outside}` or
//! `{obs:recording}`. A value that something declares, or one of the host's own, that
//! hasn't been reported yet shows as "–"; unknown names are left as written, so a typo
//! stays visible.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, TimeZone};

/// What stands for a declared value that hasn't been reported yet.
pub const PENDING: &str = "–";

/// Whether `name` is one of the values the host itself keeps up to date: `cpu`, `mem`,
/// `ha:<entity>` and `obs:<what>`. Nothing declares them, but they all get a value
/// (or "–") on the host's first round.
fn is_builtin(name: &str) -> bool {
    matches!(name, "cpu" | "mem") || name.starts_with("ha:") || name.starts_with("obs:")
}

/// Whether `name` is one of the `declared` live values: `plugin:<id>:<value>`, or for a
/// value that takes an argument, `plugin:<id>:<value>:<anything>` when `declared` holds
/// `plugin:<id>:<value>:`.
pub fn is_declared(declared: &HashSet<String>, name: &str) -> bool {
    if declared.contains(name) {
        return true;
    }
    // The prefix up to the third `:` (`plugin`, the plugin id and the value id).
    name.match_indices(':')
        .nth(2)
        .is_some_and(|(i, _)| declared.contains(&name[..=i]))
}

pub fn render_title<Tz: TimeZone>(
    title: &str,
    now: DateTime<Tz>,
    live: &HashMap<String, String>,
    declared: &HashSet<String>,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if !title.contains('{') {
        return title.to_owned();
    }
    let mut out = String::with_capacity(title.len());
    let mut rest = title;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let name = &after[..close];
        match name {
            "time_s" => out.push_str(&now.format("%H:%M:%S").to_string()),
            "time" => out.push_str(&now.format("%H:%M").to_string()),
            "date" => out.push_str(&now.format("%a %-d %b").to_string()),
            _ => match live.get(name) {
                Some(v) => out.push_str(v),
                None if is_builtin(name) || is_declared(declared, name) => out.push_str(PENDING),
                None => {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                }
            },
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// The `{name}`s used in a title (besides the clock ones).
pub fn tokens(title: &str) -> impl Iterator<Item = &str> {
    title
        .split('{')
        .skip(1)
        .filter_map(|s| s.split_once('}').map(|(name, _)| name))
        .filter(|n| !n.is_empty() && !matches!(*n, "time" | "time_s" | "date"))
}

/// A value to follow, as typed: the braces a title puts around it are left off, so
/// `{obs:recording}` follows `obs:recording`.
pub fn follow_name(typed: &str) -> &str {
    let t = typed.trim();
    t.strip_prefix('{')
        .and_then(|t| t.strip_suffix('}'))
        .map_or(t, str::trim)
}

/// Does a live value mean "on"? For toggles that follow something.
pub fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "on" | "true"
            | "yes"
            | "1"
            | "playing"
            | "open"
            | "home"
            | "active"
            | "recording"
            | "streaming"
            | "unlocked"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn follow_names_lose_their_braces() {
        assert_eq!(follow_name("obs:recording"), "obs:recording");
        assert_eq!(follow_name(" {obs:recording} "), "obs:recording");
        assert_eq!(
            follow_name("{ plugin:dev.perritodeck.obs:muted:Mic/Aux }"),
            "plugin:dev.perritodeck.obs:muted:Mic/Aux"
        );
        assert_eq!(follow_name("{}"), "");
        // Only a matching pair is taken off.
        assert_eq!(follow_name("{obs:scene"), "{obs:scene");
    }

    #[test]
    fn replaces_known_tokens_only() {
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 21, 7, 45).unwrap();
        let none = HashMap::new();
        let nothing = HashSet::new();
        let render = |t: &str, live: &HashMap<String, String>| render_title(t, now, live, &nothing);
        assert_eq!(render("{time}", &none), "21:07");
        assert_eq!(render("{time_s}", &none), "21:07:45");
        assert_eq!(render("{date}", &none), "Sun 11 Oct");
        assert_eq!(
            render("It's {time}\n{weather}", &none),
            "It's 21:07\n{weather}"
        );
        assert_eq!(render("plain", &none), "plain");
        assert_eq!(render("open { brace", &none), "open { brace");

        let live = HashMap::from([
            ("cpu".to_string(), "12%".to_string()),
            ("ha:sensor.out".into(), "4 °C".into()),
        ]);
        assert_eq!(render("CPU {cpu}\n{ha:sensor.out}", &live), "CPU 12%\n4 °C");
    }

    #[test]
    fn declared_values_wait_with_a_dash() {
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 21, 7, 45).unwrap();
        let declared = HashSet::from([
            "plugin:dev.perritodeck.sysmon:disk".to_string(),
            "plugin:dev.perritodeck.uptime:up:".to_string(),
        ]);
        let live = HashMap::from([(
            "plugin:dev.perritodeck.uptime:up:nas".to_string(),
            "up".to_string(),
        )]);
        let render = |t: &str| render_title(t, now, &live, &declared);
        // Declared but not reported yet: a dash, not the raw token.
        assert_eq!(
            render("Disk {plugin:dev.perritodeck.sysmon:disk}"),
            "Disk –"
        );
        assert_eq!(
            render("{plugin:dev.perritodeck.uptime:up:10.0.0.1:443}"),
            "–"
        );
        assert_eq!(render("{plugin:dev.perritodeck.uptime:up:nas}"), "up");
        // Typos and unknown names stay as written.
        assert_eq!(
            render("{plugin:dev.perritodeck.sysmon:dsik}"),
            "{plugin:dev.perritodeck.sysmon:dsik}"
        );
        assert_eq!(
            render("{plugin:dev.perritodeck.sysmon:disk:x}"),
            "{plugin:dev.perritodeck.sysmon:disk:x}"
        );
        assert_eq!(render("{weather}"), "{weather}");
    }

    /// Before the host's first round (a tablet that connects right after it starts),
    /// its own values show "–", as they do while OBS or Home Assistant can't be asked.
    #[test]
    fn the_hosts_own_values_wait_with_a_dash() {
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 21, 7, 45).unwrap();
        let nothing = HashSet::new();
        let none = HashMap::new();
        let render = |t: &str, live: &HashMap<String, String>| render_title(t, now, live, &nothing);
        assert_eq!(render("{obs:scene}", &none), "–");
        assert_eq!(render("{obs:recording} {obs:streaming}", &none), "– –");
        assert_eq!(render("CPU {cpu}\nRAM {mem}", &none), "CPU –\nRAM –");
        assert_eq!(render("{ha:sensor.outside}", &none), "–");
        // Reported: the value.
        let live = HashMap::from([("obs:scene".to_string(), "Gameplay".to_string())]);
        assert_eq!(render("{obs:scene}", &live), "Gameplay");
        // Names the host doesn't keep stay as written.
        assert_eq!(render("{cpuu} {obs}", &none), "{cpuu} {obs}");
    }

    #[test]
    fn finds_tokens_and_truthiness() {
        assert_eq!(
            tokens("{cpu} {time} {ha:light.x}").collect::<Vec<_>>(),
            ["cpu", "ha:light.x"]
        );
        assert!(truthy("On") && truthy("recording") && !truthy("off") && !truthy("unavailable"));
    }
}
