//! The Satellite API's line format: `COMMAND KEY=value KEY="value with spaces" WORD`.

/// One parsed line: the command, `KEY=value` pairs in order, and bare words.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Line {
    pub command: String,
    pub args: Vec<(String, String)>,
    pub words: Vec<String>,
}

impl Line {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.args
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    /// `true`/`1` are true; anything else (or missing) is false.
    pub fn flag(&self, key: &str) -> bool {
        matches!(self.get(key), Some("1" | "true" | "TRUE" | "True"))
    }
}

/// Split a line into its parts. Quoted values may contain spaces; `\"` and `\\` inside
/// quotes are unescaped.
pub fn parse(line: &str) -> Line {
    let mut tokens = Vec::new();
    let mut chars = line.trim_end_matches(['\r', '\n']).chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut token = String::new();
        let mut quoted = false;
        while let Some(&c) = chars.peek() {
            if !quoted && c.is_whitespace() {
                break;
            }
            chars.next();
            match c {
                '"' => quoted = !quoted,
                '\\' if quoted => {
                    if let Some(next) = chars.next() {
                        token.push(next);
                    }
                }
                _ => token.push(c),
            }
        }
        tokens.push(token);
    }
    let mut it = tokens.into_iter();
    let mut out = Line {
        command: it.next().unwrap_or_default(),
        ..Default::default()
    };
    for t in it {
        match t.split_once('=') {
            Some((k, v)) if !k.is_empty() => out.args.push((k.to_owned(), v.to_owned())),
            _ => out.words.push(t),
        }
    }
    out
}

/// Build a line, quoting values that need it.
pub fn format(command: &str, args: &[(&str, String)]) -> String {
    let mut s = command.to_owned();
    for (k, v) in args {
        s.push(' ');
        s.push_str(k);
        s.push('=');
        if v.is_empty() || v.contains([' ', '"', '\\', '=']) {
            s.push('"');
            for c in v.chars() {
                if matches!(c, '"' | '\\') {
                    s.push('\\');
                }
                s.push(c);
            }
            s.push('"');
        } else {
            s.push_str(v);
        }
    }
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands_args_and_quotes() {
        let l = parse("KEY-STATE DEVICEID=abc KEY=3 TEXT=\"aGk=\" PRESSED=1\r\n");
        assert_eq!(l.command, "KEY-STATE");
        assert_eq!(l.get("deviceid"), Some("abc"));
        assert_eq!(l.get("TEXT"), Some("aGk="), "base64 padding stays");
        assert!(l.flag("PRESSED"));
        let l = parse("ADD-DEVICE OK DEVICEID=x");
        assert_eq!(l.words, vec!["OK"]);
        let l = parse(r#"ERROR MESSAGE="Unknown command: \"FOO\"""#);
        assert_eq!(l.get("MESSAGE"), Some(r#"Unknown command: "FOO""#));
        assert_eq!(parse("PONG 1234").words, vec!["1234"]);
        assert_eq!(parse("").command, "");
    }

    #[test]
    fn formats_and_round_trips() {
        let s = format(
            "ADD-DEVICE",
            &[
                ("DEVICEID", "pd-1".into()),
                ("PRODUCT_NAME", "PerritoDeck \"Fire\" 7".into()),
            ],
        );
        assert_eq!(
            s,
            "ADD-DEVICE DEVICEID=pd-1 PRODUCT_NAME=\"PerritoDeck \\\"Fire\\\" 7\"\n"
        );
        let l = parse(&s);
        assert_eq!(l.get("PRODUCT_NAME"), Some("PerritoDeck \"Fire\" 7"));
    }
}
