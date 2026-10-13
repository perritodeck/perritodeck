//! Real OS backends: synthetic input via `enigo`, opening via the `open` crate.

use std::{sync::Mutex, time::Duration};

use enigo::{Direction, Enigo, Keyboard, Settings};

use crate::{ActionError, Chord, InputBackend, Key, MediaKey, Modifier, SystemBackend};

/// How long a hotkey's main key stays down. Apps that poll the keyboard instead of reading
/// key messages miss a key that goes down and up in the same instant: OBS checks its
/// global hotkeys every 25 ms on Windows, and some games and overlays work the same way.
const KEY_HOLD: Duration = Duration::from_millis(50);

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

    /// Connect now instead of on the first key press. On Wayland this is when the
    /// desktop asks for permission to control the keyboard, so do it at startup.
    pub fn connect_now(&self) -> Result<(), ActionError> {
        self.with(|_| Ok(()))
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
            // Connect on a plain thread: on Wayland enigo talks to the desktop portal with
            // its own async runtime, which must not run inside one of tokio's workers.
            let e = std::thread::spawn(|| Enigo::new(&Settings::default()))
                .join()
                .map_err(|_| {
                    ActionError::Input("the keyboard backend crashed while starting".into())
                })?
                .map_err(|e| ActionError::Input(format!("cannot connect to the keyboard: {e}")))?;
            *guard = Some(e);
        }
        f(guard.as_mut().expect("initialised above"))
    }
}

fn input_err(e: impl std::fmt::Display) -> ActionError {
    ActionError::Input(e.to_string())
}

/// Run input work that waits (a held key) without stalling the async runtime: on
/// tokio's multi-thread runtime the worker first hands its other tasks to another
/// thread. Elsewhere (no runtime, or a single-threaded one) it just runs.
fn blocking<T>(f: impl FnOnce() -> T) -> T {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Ok(h) if h.runtime_flavor() == RuntimeFlavor::MultiThread => tokio::task::block_in_place(f),
        _ => f(),
    }
}

/// Press a chord through `key`: the modifiers in order, then the main key, held down for
/// `hold` (or down and up at once without it), then the modifiers released in reverse.
/// Whatever was pressed is released even after an error, so no modifier sticks.
fn press_chord<E: std::fmt::Display>(
    modifiers: &[Modifier],
    main: enigo::Key,
    hold: Option<Duration>,
    mut key: impl FnMut(enigo::Key, Direction) -> Result<(), E>,
) -> Result<(), ActionError> {
    let mut pressed = Vec::new();
    let mut result = Ok(());
    for &m in modifiers {
        if let Err(err) = key(modifier_key(m), Direction::Press) {
            result = Err(input_err(err));
            break;
        }
        pressed.push(m);
    }
    if result.is_ok() {
        result = match hold {
            Some(hold) => key(main, Direction::Press)
                .map_err(input_err)
                .and_then(|()| {
                    std::thread::sleep(hold);
                    key(main, Direction::Release).map_err(input_err)
                }),
            None => key(main, Direction::Click).map_err(input_err),
        };
    }
    for &m in pressed.iter().rev() {
        let _ = key(modifier_key(m), Direction::Release);
    }
    result
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
        blocking(|| {
            self.with(|e| press_chord(&chord.modifiers, key, Some(KEY_HOLD), |k, d| e.key(k, d)))
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

#[cfg(test)]
mod input_tests {
    use std::time::Instant;

    use enigo::Key as E;

    use super::*;

    /// Each key event and when it happened.
    type Log = Vec<(E, Direction, Instant)>;

    /// Press a chord against a log instead of the OS; `fail` makes that key's press fail.
    fn pressed(
        modifiers: &[Modifier],
        main: E,
        hold: Option<Duration>,
        fail: Option<E>,
    ) -> (Result<(), ActionError>, Log) {
        let mut log = Vec::new();
        let result = press_chord(modifiers, main, hold, |k, d| {
            log.push((k, d, Instant::now()));
            if Some(k) == fail && d != Direction::Release {
                return Err("blocked");
            }
            Ok(())
        });
        (result, log)
    }

    fn keys(log: &[(E, Direction, Instant)]) -> Vec<(E, Direction)> {
        log.iter().map(|&(k, d, _)| (k, d)).collect()
    }

    #[test]
    fn chord_holds_the_main_key_inside_the_modifiers() {
        let hold = Duration::from_millis(30);
        let (result, log) = pressed(
            &[Modifier::Ctrl, Modifier::Shift],
            E::Other(0x4D),
            Some(hold),
            None,
        );
        result.unwrap();
        use Direction::{Press, Release};
        assert_eq!(
            keys(&log),
            [
                (E::Control, Press),
                (E::Shift, Press),
                (E::Other(0x4D), Press),
                (E::Other(0x4D), Release),
                (E::Shift, Release),
                (E::Control, Release),
            ]
        );
        assert!(log[3].2 - log[2].2 >= hold, "the key was held");
    }

    #[test]
    fn chord_without_hold_clicks() {
        let (result, log) = pressed(&[], E::Unicode('€'), None, None);
        result.unwrap();
        assert_eq!(keys(&log), [(E::Unicode('€'), Direction::Click)]);
    }

    #[test]
    fn chord_releases_what_it_pressed_after_an_error() {
        use Direction::{Press, Release};
        // A modifier fails: the ones before it are released, the main key never pressed.
        let (result, log) = pressed(
            &[Modifier::Ctrl, Modifier::Alt, Modifier::Shift],
            E::F13,
            Some(Duration::ZERO),
            Some(E::Alt),
        );
        assert!(matches!(result, Err(ActionError::Input(m)) if m == "blocked"));
        assert_eq!(
            keys(&log),
            [(E::Control, Press), (E::Alt, Press), (E::Control, Release)]
        );
        // The main key fails: the modifiers are still released.
        let (result, log) = pressed(
            &[Modifier::Ctrl],
            E::F13,
            Some(Duration::ZERO),
            Some(E::F13),
        );
        assert!(result.is_err());
        assert_eq!(
            keys(&log),
            [(E::Control, Press), (E::F13, Press), (E::Control, Release)]
        );
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

    fn set_volume(&self, percent: u8) -> Result<(), ActionError> {
        volume::set(percent.min(100))
    }

    fn volume(&self) -> Option<u8> {
        volume::get()
    }
}

/// The main output volume, through each system's own tools.
#[cfg(target_os = "linux")]
mod volume {
    use super::ActionError;
    use std::process::Command;

    fn run(program: &str, args: &[&str]) -> Option<String> {
        let out = Command::new(program).args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// PipeWire's `wpctl`, else PulseAudio's `pactl`.
    pub fn set(percent: u8) -> Result<(), ActionError> {
        let level = format!("{:.2}", f64::from(percent) / 100.0);
        if run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &level]).is_some() {
            return Ok(());
        }
        if run(
            "pactl",
            &["set-sink-volume", "@DEFAULT_SINK@", &format!("{percent}%")],
        )
        .is_some()
        {
            return Ok(());
        }
        Err(ActionError::Unsupported(
            "setting the volume without wpctl or pactl".into(),
        ))
    }

    pub fn get() -> Option<u8> {
        if let Some(out) = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]) {
            // "Volume: 0.40" (maybe followed by "[MUTED]")
            let v: f64 = out.split_whitespace().nth(1)?.parse().ok()?;
            return Some((v * 100.0).round().clamp(0.0, 100.0) as u8);
        }
        let out = run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])?;
        let pct = out.split('/').nth(1)?.trim().trim_end_matches('%').trim();
        pct.parse::<u8>().ok().map(|v| v.min(100))
    }
}

#[cfg(target_os = "macos")]
mod volume {
    use super::ActionError;
    use std::process::Command;

    pub fn set(percent: u8) -> Result<(), ActionError> {
        let ok = Command::new("osascript")
            .args(["-e", &format!("set volume output volume {percent}")])
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            Ok(())
        } else {
            Err(ActionError::Input(
                "osascript couldn't set the volume".into(),
            ))
        }
    }

    pub fn get() -> Option<u8> {
        let out = Command::new("osascript")
            .args(["-e", "output volume of (get volume settings)"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<u8>()
            .ok()
            .map(|v| v.min(100))
    }
}

// The Windows volume API is COM.
#[cfg(windows)]
#[allow(unsafe_code)]
mod volume {
    use super::ActionError;
    use windows::Win32::{
        Media::Audio::{
            eConsole, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator,
            MMDeviceEnumerator,
        },
        System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED},
    };

    fn endpoint() -> windows::core::Result<IAudioEndpointVolume> {
        // SAFETY: COM is initialised for this thread first (a repeat call is harmless);
        // the interfaces are reference-counted by the `windows` crate.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            device.Activate(CLSCTX_ALL, None)
        }
    }

    pub fn set(percent: u8) -> Result<(), ActionError> {
        let e = endpoint().map_err(|e| ActionError::Input(format!("no sound output: {e}")))?;
        // SAFETY: a plain call on a live interface; no event context.
        unsafe { e.SetMasterVolumeLevelScalar(f32::from(percent) / 100.0, std::ptr::null()) }
            .map_err(|e| ActionError::Input(e.to_string()))
    }

    pub fn get() -> Option<u8> {
        let e = endpoint().ok()?;
        // SAFETY: a plain call on a live interface.
        let v = unsafe { e.GetMasterVolumeLevelScalar() }.ok()?;
        Some((v * 100.0).round().clamp(0.0, 100.0) as u8)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod volume {
    use super::ActionError;
    pub fn set(_: u8) -> Result<(), ActionError> {
        Err(ActionError::Unsupported("setting the volume".into()))
    }
    pub fn get() -> Option<u8> {
        None
    }
}
