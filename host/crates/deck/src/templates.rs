//! Live values in key titles: `{time}`, `{time_s}`, `{date}`, and values the host
//! keeps up to date such as `{cpu}`, `{mem}`, `{ha:sensor.outside}` or
//! `{obs:recording}`. Unknown names are left as written.

use std::collections::HashMap;

use chrono::{DateTime, TimeZone};

pub fn render_title<Tz: TimeZone>(
    title: &str,
    now: DateTime<Tz>,
    live: &HashMap<String, String>,
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
    fn replaces_known_tokens_only() {
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 21, 7, 45).unwrap();
        let none = HashMap::new();
        assert_eq!(render_title("{time}", now, &none), "21:07");
        assert_eq!(render_title("{time_s}", now, &none), "21:07:45");
        assert_eq!(render_title("{date}", now, &none), "Sun 11 Oct");
        assert_eq!(
            render_title("It's {time}\n{weather}", now, &none),
            "It's 21:07\n{weather}"
        );
        assert_eq!(render_title("plain", now, &none), "plain");
        assert_eq!(render_title("open { brace", now, &none), "open { brace");

        let live = HashMap::from([
            ("cpu".to_string(), "12%".to_string()),
            ("ha:sensor.out".into(), "4 °C".into()),
        ]);
        assert_eq!(
            render_title("CPU {cpu}\n{ha:sensor.out}", now, &live),
            "CPU 12%\n4 °C"
        );
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
