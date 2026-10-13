//! Parsing hotkey strings like `Ctrl+Shift+M`, `Cmd+Space`, `Alt+F4`.

use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    /// Windows key / Command / Super.
    Meta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A printable character, stored lowercase for letters.
    Char(char),
    /// F1–F24.
    F(u8),
    Enter,
    Tab,
    Space,
    Escape,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    CapsLock,
}

/// Modifiers plus one key. Modifiers are kept sorted and unique.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub modifiers: Vec<Modifier>,
    pub key: Key,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HotkeyParseError {
    #[error("hotkey is empty")]
    Empty,
    #[error("unknown key {0:?} in hotkey")]
    UnknownKey(String),
    #[error("hotkey {0:?} has no key, only modifiers")]
    OnlyModifiers(String),
    #[error("hotkey {0:?} has more than one non-modifier key")]
    TwoKeys(String),
}

fn modifier(name: &str) -> Option<Modifier> {
    Some(match name {
        "ctrl" | "control" | "ctl" => Modifier::Ctrl,
        "alt" | "option" | "opt" => Modifier::Alt,
        "shift" => Modifier::Shift,
        "meta" | "cmd" | "command" | "super" | "win" | "windows" => Modifier::Meta,
        _ => return None,
    })
}

fn key(name: &str) -> Option<Key> {
    let k = match name {
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "space" | "spacebar" => Key::Space,
        "esc" | "escape" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "insert" | "ins" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "up" | "uparrow" => Key::Up,
        "down" | "downarrow" => Key::Down,
        "left" | "leftarrow" => Key::Left,
        "right" | "rightarrow" => Key::Right,
        "capslock" => Key::CapsLock,
        "plus" => Key::Char('+'),
        "minus" => Key::Char('-'),
        _ => {
            if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                return (1..=24).contains(&n).then_some(Key::F(n));
            }
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if !c.is_whitespace() && !c.is_control() => Key::Char(c),
                _ => return None,
            }
        }
    };
    Some(k)
}

impl FromStr for Chord {
    type Err = HotkeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(HotkeyParseError::Empty);
        }
        // "Ctrl++" means Ctrl and the plus key: a trailing '+' after a separator is the key itself.
        let mut parts: Vec<String> = Vec::new();
        let mut rest = trimmed;
        while !rest.is_empty() {
            // Spaces around a separator are fine ("Ctrl + +"), so skip them first: a '+'
            // after them is then the key, not an empty part before another separator.
            rest = rest.trim_start();
            match rest.find('+') {
                Some(0) => {
                    parts.push("+".into());
                    let after = rest[1..].trim_start();
                    rest = after.strip_prefix('+').unwrap_or(after);
                }
                Some(i) => {
                    parts.push(rest[..i].trim().to_owned());
                    rest = &rest[i + 1..];
                }
                None => {
                    parts.push(rest.trim().to_owned());
                    rest = "";
                }
            }
        }

        let mut modifiers = Vec::new();
        let mut main: Option<Key> = None;
        for part in &parts {
            let lower = part.to_lowercase();
            if let Some(m) = modifier(&lower) {
                modifiers.push(m);
                continue;
            }
            let k = key(&lower).ok_or_else(|| HotkeyParseError::UnknownKey(part.clone()))?;
            if main.replace(k).is_some() {
                return Err(HotkeyParseError::TwoKeys(trimmed.to_owned()));
            }
        }
        modifiers.sort();
        modifiers.dedup();
        let key = main.ok_or_else(|| HotkeyParseError::OnlyModifiers(trimmed.to_owned()))?;
        Ok(Chord { modifiers, key })
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for m in &self.modifiers {
            write!(f, "{m:?}+")?;
        }
        match self.key {
            Key::Char(c) => write!(f, "{}", c.to_uppercase()),
            Key::F(n) => write!(f, "F{n}"),
            k => write!(f, "{k:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Chord {
        s.parse().unwrap()
    }

    #[test]
    fn common_hotkeys() {
        assert_eq!(
            p("Ctrl+Shift+M"),
            Chord {
                modifiers: vec![Modifier::Ctrl, Modifier::Shift],
                key: Key::Char('m')
            }
        );
        assert_eq!(
            p("cmd + space"),
            Chord {
                modifiers: vec![Modifier::Meta],
                key: Key::Space
            }
        );
        assert_eq!(
            p("Alt+F4"),
            Chord {
                modifiers: vec![Modifier::Alt],
                key: Key::F(4)
            }
        );
        assert_eq!(
            p("F13"),
            Chord {
                modifiers: vec![],
                key: Key::F(13)
            }
        );
        assert_eq!(
            p("Win+Shift+S").modifiers,
            vec![Modifier::Shift, Modifier::Meta]
        );
        assert_eq!(p("PageDown").key, Key::PageDown);
    }

    #[test]
    fn modifier_order_and_duplicates_normalised() {
        assert_eq!(p("Shift+Ctrl+Ctrl+a"), p("ctrl+shift+A"));
    }

    #[test]
    fn plus_key() {
        assert_eq!(
            p("Ctrl++"),
            Chord {
                modifiers: vec![Modifier::Ctrl],
                key: Key::Char('+')
            }
        );
        assert_eq!(p("Ctrl+Plus"), p("Ctrl++"));
        // With spaces around the separator, as people type it.
        assert_eq!(p("Ctrl + +"), p("Ctrl++"));
        assert_eq!(p("Ctrl+ +"), p("Ctrl++"));
        assert_eq!(p("Ctrl + Shift + +"), p("Ctrl+Shift++"));
        assert_eq!(p("Ctrl + + + Shift"), p("Ctrl+Shift++"));
        assert_eq!(p(" + "), p("Plus"));
    }

    #[test]
    fn errors() {
        assert_eq!("".parse::<Chord>(), Err(HotkeyParseError::Empty));
        assert!(matches!(
            "Ctrl+Shift".parse::<Chord>(),
            Err(HotkeyParseError::OnlyModifiers(_))
        ));
        assert!(matches!(
            "Ctrl+A+B".parse::<Chord>(),
            Err(HotkeyParseError::TwoKeys(_))
        ));
        assert!(matches!(
            "Ctrl+Banana".parse::<Chord>(),
            Err(HotkeyParseError::UnknownKey(_))
        ));
        assert!(matches!(
            "F25".parse::<Chord>(),
            Err(HotkeyParseError::UnknownKey(_))
        ));
    }

    #[test]
    fn display_round_trips() {
        for s in ["Ctrl+Shift+M", "Meta+Space", "Alt+F4", "Ctrl++"] {
            let c = p(s);
            assert_eq!(c.to_string().parse::<Chord>().unwrap(), c, "{s}");
        }
    }
}
