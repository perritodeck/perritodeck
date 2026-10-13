//! Real OS backends: synthetic input via `enigo`, opening via the `open` crate.

use std::{sync::Mutex, time::Duration};

use enigo::{Direction, Enigo, Keyboard, Settings};

use crate::{ActionError, Chord, InputBackend, Key, MediaKey, Modifier, SystemBackend};

/// How long a hotkey's main key stays down. Apps that poll the keyboard instead of reading
/// key messages miss a key that goes down and up in the same instant: OBS checks its
/// global hotkeys every 25 ms on Windows, and some games and overlays work the same way.
const KEY_HOLD: Duration = Duration::from_millis(50);

/// The pause between the characters of a text action on Windows. Windows 11 Notepad (a
/// WinUI text box) drops and repeats characters that arrive less than about 10 ms apart;
/// classic edit controls take any speed. 100 characters take about 1.5 s.
const CHAR_PACE: Duration = Duration::from_millis(15);

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

/// Run input work that waits (a held key, paced text) without stalling the async runtime:
/// on tokio's multi-thread runtime the worker first hands its other tasks to another
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

/// Type `text` through `send` a piece at a time (see [`text_pieces`]), waiting `pace`
/// between pieces.
fn type_paced<E: std::fmt::Display>(
    text: &str,
    pace: Duration,
    mut send: impl FnMut(&str) -> Result<(), E>,
) -> Result<(), ActionError> {
    // enigo refuses a null character; refuse it before typing anything rather than partway.
    if text.contains('\0') {
        return Err(ActionError::Input(
            "the text contains a null character".into(),
        ));
    }
    for (i, piece) in text_pieces(text).enumerate() {
        if i > 0 {
            std::thread::sleep(pace);
        }
        send(piece).map_err(input_err)?;
    }
    Ok(())
}

/// Split text into what is typed in one go: a character (both halves of an emoji's UTF-16
/// pair go together), or "\r\n", which is a single line break.
fn text_pieces(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let c = rest.chars().next()?;
        let len = if rest.starts_with("\r\n") {
            2
        } else {
            c.len_utf8()
        };
        let (piece, tail) = rest.split_at(len);
        rest = tail;
        Some(piece)
    })
}

/// The modifiers to hold and the key to press for a chord. On Windows a character can
/// bring modifiers of its own (see `windows_char`).
fn resolve(chord: &Chord) -> Result<(Vec<Modifier>, enigo::Key), ActionError> {
    #[cfg(windows)]
    if let Key::Char(c) = chord.key {
        return windows_char(c, &chord.modifiers, win::vk_scan);
    }
    Ok((chord.modifiers.clone(), main_key(chord.key)?))
}

/// A hotkey character on Windows, where apps see virtual keys and the keyboard layout
/// decides which key types which character. `vk_scan` is `VkKeyScanExW` for the layout of
/// the window in front.
#[cfg(any(windows, test))]
fn windows_char(
    c: char,
    modifiers: &[Modifier],
    vk_scan: impl FnOnce(char) -> i16,
) -> Result<(Vec<Modifier>, enigo::Key), ActionError> {
    // Letters and digits go by virtual key (VK_A to VK_Z and VK_0 to VK_9 are their ASCII
    // codes), as Windows shortcuts do on every layout: Ctrl+C is VK_C on AZERTY, Dvorak and
    // Russian alike, though a Russian layout has no 'c' to look up, and Ctrl+1 stays
    // unshifted on AZERTY, where typing a 1 needs Shift.
    if c.is_ascii_alphanumeric() {
        let vk = u32::from(c.to_ascii_uppercase());
        return Ok((modifiers.to_vec(), enigo::Key::Other(vk)));
    }
    // The virtual key is in the low byte, and the high byte holds the Shift (1), Ctrl (2)
    // and Alt (4) that type the character on this layout; -1 means no key types it.
    let found = vk_scan(c);
    if found < 0 {
        if modifiers.is_empty() {
            // On its own it can still be typed: enigo sends it as a character.
            return Ok((Vec::new(), enigo::Key::Unicode(c)));
        }
        // Sent as a character with the modifiers held, no app would see a shortcut.
        return Err(ActionError::Input(format!(
            "no key types {c:?} on the current keyboard layout, so it can't be part of a shortcut"
        )));
    }
    let [vk, state] = found.to_le_bytes();
    if state & !0b111 != 0 {
        return Err(ActionError::Input(format!(
            "{c:?} needs a keyboard mode a hotkey can't switch to; use the key it's on"
        )));
    }
    // Add the Shift or AltGr (Ctrl+Alt) the layout needs, so "ctrl+!" is Ctrl+Shift+1 on a
    // US layout and "@" is AltGr+Q on a German one. Except for '+': "Ctrl+Plus" names the
    // =/+ key without Shift, the zoom-in shortcut of browsers, editors and terminals, while
    // Ctrl+Shift+= is a different shortcut in some apps (superscript in Word).
    let mut all = modifiers.to_vec();
    if state & 1 != 0 && c != '+' {
        all.push(Modifier::Shift);
    }
    if state & 2 != 0 {
        all.push(Modifier::Ctrl);
    }
    if state & 4 != 0 {
        all.push(Modifier::Alt);
    }
    all.sort();
    all.dedup();
    Ok((all, enigo::Key::Other(u32::from(vk))))
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
        let (modifiers, key) = resolve(chord)?;
        // On Windows a character no key on the layout types is sent as text, down and up
        // in one go, so holding it would type it twice. Everything else is a real key.
        let hold = if cfg!(windows) && matches!(key, enigo::Key::Unicode(_)) {
            None
        } else {
            Some(KEY_HOLD)
        };
        blocking(|| self.with(|e| press_chord(&modifiers, key, hold, |k, d| e.key(k, d))))
    }

    fn text(&self, text: &str) -> Result<(), ActionError> {
        if cfg!(windows) {
            // A character at a time: Windows 11 Notepad and other WinUI text boxes drop
            // and repeat characters sent in one burst.
            blocking(|| self.with(|e| type_paced(text, CHAR_PACE, |piece| e.text(piece))))
        } else {
            self.with(|e| e.text(text).map_err(input_err))
        }
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

/// The Windows calls hotkeys need: the keyboard layout of the window in front.
#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use windows::Win32::UI::{
        Input::KeyboardAndMouse::{GetKeyboardLayout, VkKeyScanExW},
        WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
    };

    /// `VkKeyScanExW` for `c` on the keyboard layout of the window in front, the one that
    /// will read the keys. -1 when no key types it (always for characters beyond U+FFFF).
    pub fn vk_scan(c: char) -> i16 {
        let mut units = [0; 2];
        let units = c.encode_utf16(&mut units);
        if units.len() != 1 {
            return -1;
        }
        // SAFETY: plain calls without pointers. With no window in front the thread id is 0,
        // which gives this thread's layout.
        unsafe {
            let thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
            VkKeyScanExW(units[0], GetKeyboardLayout(thread))
        }
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

    fn pieces(text: &str) -> Vec<&str> {
        text_pieces(text).collect()
    }

    #[test]
    fn text_is_typed_a_piece_at_a_time() {
        assert_eq!(pieces("ab"), ["a", "b"]);
        assert_eq!(pieces("é€😀"), ["é", "€", "😀"]);
        assert_eq!(
            pieces("a\r\nb\nc\rd\te"),
            ["a", "\r\n", "b", "\n", "c", "\r", "d", "\t", "e"]
        );
        assert_eq!(pieces("\r\r\n\n"), ["\r", "\r\n", "\n"]);
        assert!(pieces("").is_empty());

        let pace = Duration::from_millis(10);
        let mut sent = Vec::new();
        let start = Instant::now();
        type_paced("hé\r\n!", pace, |s| {
            sent.push(s.to_owned());
            Ok::<_, String>(())
        })
        .unwrap();
        assert_eq!(sent, ["h", "é", "\r\n", "!"]);
        assert!(start.elapsed() >= pace * 3, "paused between pieces");
    }

    #[test]
    fn text_with_a_null_character_types_nothing() {
        let mut sent = 0;
        let result = type_paced("ab\0c", Duration::ZERO, |_| {
            sent += 1;
            Ok::<_, String>(())
        });
        assert!(result.is_err());
        assert_eq!(sent, 0);
    }

    #[test]
    fn text_stops_at_the_first_failure() {
        let mut sent = 0;
        let result = type_paced("abc", Duration::ZERO, |_| {
            sent += 1;
            if sent == 2 {
                Err("blocked")
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert_eq!(sent, 2);
    }

    /// `windows_char` with a fixed `VkKeyScanExW` answer.
    fn on_layout(
        c: char,
        modifiers: &[Modifier],
        scan: i16,
    ) -> Result<(Vec<Modifier>, E), ActionError> {
        windows_char(c, modifiers, |_| scan)
    }

    #[test]
    fn windows_letters_and_digits_go_by_virtual_key() {
        // The layout isn't asked: the answer here would be wrong on purpose.
        for (c, vk) in [
            ('c', 0x43),
            ('C', 0x43),
            ('z', 0x5A),
            ('0', 0x30),
            ('9', 0x39),
        ] {
            assert_eq!(
                on_layout(c, &[Modifier::Ctrl], -1).unwrap(),
                (vec![Modifier::Ctrl], E::Other(vk)),
                "{c}"
            );
        }
    }

    #[test]
    fn windows_symbols_bring_the_modifiers_the_layout_needs() {
        use Modifier::{Alt, Ctrl, Shift};
        // US: '/' is unshifted (0x00BF), '!' is Shift+1 (0x0131), '?' Shift+/ (0x01BF).
        assert_eq!(
            on_layout('/', &[Ctrl], 0x00BF).unwrap(),
            (vec![Ctrl], E::Other(0xBF))
        );
        assert_eq!(
            on_layout('!', &[Ctrl], 0x0131).unwrap(),
            (vec![Ctrl, Shift], E::Other(0x31))
        );
        assert_eq!(
            on_layout('?', &[Ctrl, Shift], 0x01BF).unwrap(),
            (vec![Ctrl, Shift], E::Other(0xBF))
        );
        assert_eq!(
            on_layout('!', &[], 0x0131).unwrap(),
            (vec![Shift], E::Other(0x31))
        );
        // German: '@' is AltGr+Q (0x0651), so it gets Ctrl+Alt; Ctrl isn't doubled.
        assert_eq!(
            on_layout('@', &[Ctrl], 0x0651).unwrap(),
            (vec![Ctrl, Alt], E::Other(0x51))
        );
        assert_eq!(
            on_layout('@', &[], 0x0651).unwrap(),
            (vec![Ctrl, Alt], E::Other(0x51))
        );
        // '+' stays the =/+ key without Shift (US 0x01BB), so Ctrl+Plus is zoom-in.
        assert_eq!(
            on_layout('+', &[Ctrl], 0x01BB).unwrap(),
            (vec![Ctrl], E::Other(0xBB))
        );
        assert_eq!(
            on_layout('+', &[Ctrl, Shift], 0x01BB).unwrap(),
            (vec![Ctrl, Shift], E::Other(0xBB))
        );
    }

    #[test]
    fn windows_characters_off_the_layout() {
        // Alone it is typed as text; in a shortcut it's an error, not a typed character.
        assert_eq!(on_layout('€', &[], -1).unwrap(), (vec![], E::Unicode('€')));
        let err = on_layout('€', &[Modifier::Ctrl], -1)
            .unwrap_err()
            .to_string();
        assert!(err.contains("current keyboard layout"), "{err}");
        // A shift state other than Shift, Ctrl and Alt (here Hankaku) can't be pressed.
        assert!(on_layout('ア', &[], 0x0841).is_err());
    }

    /// The real lookup, with what gives the same answer on every keyboard layout.
    #[cfg(windows)]
    #[test]
    fn windows_chords_resolve() {
        let chord = |s: &str| resolve(&s.parse().unwrap());
        assert_eq!(
            chord("Ctrl+Shift+M").unwrap(),
            (vec![Modifier::Ctrl, Modifier::Shift], E::Other(0x4D))
        );
        assert_eq!(
            chord("Ctrl+1").unwrap(),
            (vec![Modifier::Ctrl], E::Other(0x31))
        );
        assert_eq!(chord("Alt+F4").unwrap(), (vec![Modifier::Alt], E::F4));
        // No layout has a key for a character beyond U+FFFF.
        assert!(chord("Ctrl+😀").is_err());
        assert_eq!(chord("😀").unwrap(), (vec![], E::Unicode('😀')));
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
