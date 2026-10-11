//! Real OS backends: synthetic input via `enigo`, opening via the `open` crate.

use std::sync::Mutex;

use enigo::{Direction, Enigo, Keyboard, Settings};

use crate::{ActionError, Chord, InputBackend, Key, MediaKey, Modifier, SystemBackend};

/// Keyboard input through `enigo`. The connection to the display server is made on
/// first use, so the host still starts on a machine where input isn't available
/// (e.g. a headless server); only the action fails.
#[derive(Default)]
pub struct EnigoInput {
    enigo: Mutex<Option<Enigo>>,
}

impl EnigoInput {
    pub fn new() -> Self {
        Self::default()
    }

    fn with<T>(
        &self,
        f: impl FnOnce(&mut Enigo) -> Result<T, ActionError>,
    ) -> Result<T, ActionError> {
        let mut guard = self
            .enigo
            .lock()
            .map_err(|_| ActionError::Input("input backend poisoned".into()))?;
        if guard.is_none() {
            let e = Enigo::new(&Settings::default())
                .map_err(|e| ActionError::Input(format!("cannot connect to the keyboard: {e}")))?;
            *guard = Some(e);
        }
        f(guard.as_mut().expect("initialised above"))
    }
}

fn input_err(e: impl std::fmt::Display) -> ActionError {
    ActionError::Input(e.to_string())
}

fn modifier_key(m: Modifier) -> enigo::Key {
    match m {
        Modifier::Ctrl => enigo::Key::Control,
        Modifier::Alt => enigo::Key::Alt,
        Modifier::Shift => enigo::Key::Shift,
        Modifier::Meta => enigo::Key::Meta,
    }
}

fn main_key(k: Key) -> Result<enigo::Key, ActionError> {
    use enigo::Key as E;
    Ok(match k {
        Key::Char(c) => E::Unicode(c),
        Key::F(n) => function_key(n)?,
        Key::Enter => E::Return,
        Key::Tab => E::Tab,
        Key::Space => E::Space,
        Key::Escape => E::Escape,
        Key::Backspace => E::Backspace,
        Key::Delete => E::Delete,
        #[cfg(not(target_os = "macos"))]
        Key::Insert => E::Insert,
        #[cfg(target_os = "macos")]
        Key::Insert => return Err(ActionError::Unsupported("the Insert key".into())),
        Key::Home => E::Home,
        Key::End => E::End,
        Key::PageUp => E::PageUp,
        Key::PageDown => E::PageDown,
        Key::Up => E::UpArrow,
        Key::Down => E::DownArrow,
        Key::Left => E::LeftArrow,
        Key::Right => E::RightArrow,
        Key::CapsLock => E::CapsLock,
    })
}

fn function_key(n: u8) -> Result<enigo::Key, ActionError> {
    use enigo::Key as E;
    Ok(match n {
        1 => E::F1,
        2 => E::F2,
        3 => E::F3,
        4 => E::F4,
        5 => E::F5,
        6 => E::F6,
        7 => E::F7,
        8 => E::F8,
        9 => E::F9,
        10 => E::F10,
        11 => E::F11,
        12 => E::F12,
        13 => E::F13,
        14 => E::F14,
        15 => E::F15,
        16 => E::F16,
        17 => E::F17,
        18 => E::F18,
        19 => E::F19,
        20 => E::F20,
        #[cfg(not(target_os = "macos"))]
        21 => E::F21,
        #[cfg(not(target_os = "macos"))]
        22 => E::F22,
        #[cfg(not(target_os = "macos"))]
        23 => E::F23,
        #[cfg(not(target_os = "macos"))]
        24 => E::F24,
        n => return Err(ActionError::Unsupported(format!("F{n}"))),
    })
}

impl InputBackend for EnigoInput {
    fn chord(&self, chord: &Chord) -> Result<(), ActionError> {
        let key = main_key(chord.key)?;
        self.with(|e| {
            let mut pressed = Vec::new();
            let mut result = Ok(());
            for &m in &chord.modifiers {
                if let Err(err) = e.key(modifier_key(m), Direction::Press) {
                    result = Err(input_err(err));
                    break;
                }
                pressed.push(m);
            }
            if result.is_ok() {
                result = e.key(key, Direction::Click).map_err(input_err);
            }
            // Always release what we pressed, even after an error, so no modifier sticks.
            for &m in pressed.iter().rev() {
                let _ = e.key(modifier_key(m), Direction::Release);
            }
            result
        })
    }

    fn text(&self, text: &str) -> Result<(), ActionError> {
        self.with(|e| e.text(text).map_err(input_err))
    }

    fn media(&self, key: MediaKey) -> Result<(), ActionError> {
        use enigo::Key as E;
        let k = match key {
            MediaKey::PlayPause => E::MediaPlayPause,
            MediaKey::Next => E::MediaNextTrack,
            MediaKey::Previous => E::MediaPrevTrack,
            #[cfg(not(target_os = "macos"))]
            MediaKey::Stop => E::MediaStop,
            #[cfg(target_os = "macos")]
            MediaKey::Stop => return Err(ActionError::Unsupported("the media Stop key".into())),
            MediaKey::VolumeUp => E::VolumeUp,
            MediaKey::VolumeDown => E::VolumeDown,
            MediaKey::Mute => E::VolumeMute,
        };
        self.with(|e| e.key(k, Direction::Click).map_err(input_err))
    }
}

/// Opens URLs and paths with the user's default applications.
#[derive(Default)]
pub struct OpenSystem;

impl SystemBackend for OpenSystem {
    fn open(&self, target: &str) -> Result<(), ActionError> {
        open::that_detached(target).map_err(|e| ActionError::Open {
            target: target.to_owned(),
            reason: e.to_string(),
        })
    }
}
