//! Live values in key titles: `{time}`, `{time_s}`, `{date}`.

use chrono::{DateTime, TimeZone};

pub fn render_title<Tz: TimeZone>(title: &str, now: DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if !title.contains('{') {
        return title.to_owned();
    }
    title
        .replace("{time_s}", &now.format("%H:%M:%S").to_string())
        .replace("{time}", &now.format("%H:%M").to_string())
        .replace("{date}", &now.format("%a %-d %b").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn replaces_known_tokens_only() {
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 21, 7, 45).unwrap();
        assert_eq!(render_title("{time}", now), "21:07");
        assert_eq!(render_title("{time_s}", now), "21:07:45");
        assert_eq!(render_title("{date}", now), "Sun 11 Oct");
        assert_eq!(
            render_title("It's {time}\n{weather}", now),
            "It's 21:07\n{weather}"
        );
        assert_eq!(render_title("plain", now), "plain");
    }
}
