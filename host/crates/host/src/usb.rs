//! Tablets on a USB cable: when Android's `adb` tool is installed, forward the host's
//! port to every connected tablet (`adb reverse`), so Perrito can reach this computer
//! at 127.0.0.1 without Wi-Fi.
//!
//! The forward lives in the tablet and goes away whenever adb reconnects to it: a
//! replug, `adb kill-server` or `adb reconnect`, another program's adb taking over the
//! server, wireless debugging dropping. That can happen between two polls, so the
//! tablet looks as if it never left. Every poll therefore checks that each tablet's
//! forward is still there (`adb reverse --list`) and puts it back if it isn't.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::{process::Command, time::MissedTickBehavior};
use tracing::{debug, info, warn};

/// How often to look for tablets and check their forwards.
const EVERY: Duration = Duration::from_secs(4);
/// How long one adb call may take. Starting the adb server takes a few seconds, but a
/// wedged server or a tablet stuck half-connected can keep adb waiting for good.
const ADB_TIMEOUT: Duration = Duration::from_secs(10);

/// What the last look at the USB tablets found.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UsbState {
    /// Tablets whose port is forwarded, so they can reach this computer.
    pub forwarded: BTreeSet<String>,
    /// Tablets adb lists that can't connect yet, and why, in plain words.
    pub waiting: BTreeMap<String, String>,
    /// Why adb itself didn't answer, if it didn't.
    pub error: Option<String>,
}

impl UsbState {
    /// One line per problem, for Settings.
    pub fn problems(&self) -> Vec<String> {
        self.error
            .iter()
            .map(|e| format!("adb isn't answering: {e}"))
            .chain(
                self.waiting
                    .iter()
                    .map(|(serial, why)| format!("{serial}: {why}")),
            )
            .collect()
    }

    /// A one-line summary for the diagnostics report. It leaves out serial numbers,
    /// as the report gets pasted into bug reports.
    pub fn describe(&self) -> String {
        let mut s = format!("adb found; {} tablet(s) on a cable", self.forwarded.len());
        if let Some(e) = &self.error {
            s.push_str(&format!("; adb isn't answering: {e}"));
        }
        for why in self.waiting.values() {
            s.push_str(&format!("; one can't connect: {why}"));
        }
        s
    }
}

/// The USB state the host reports; the poll keeps it current.
pub type UsbDevices = Arc<Mutex<UsbState>>;

/// `adb` from PATH, or from the usual Android SDK locations.
pub fn find_adb() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "adb.exe" } else { "adb" };
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(exe)).collect())
        .unwrap_or_default();
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(sdk) = std::env::var_os(var) {
            candidates.push(PathBuf::from(sdk).join("platform-tools").join(exe));
        }
    }
    if let Some(home) = directories::BaseDirs::new() {
        let h = home.home_dir();
        candidates.push(h.join("AppData/Local/Android/Sdk/platform-tools").join(exe));
        candidates.push(h.join("Library/Android/sdk/platform-tools").join(exe));
        candidates.push(h.join("Android/Sdk/platform-tools").join(exe));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Parse `adb devices` output: each device's serial and state ("device" when it's
/// ready). The two are separated by a tab, and a state can have spaces in it ("no
/// permissions (missing udev rules? ...)").
pub fn list_devices(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter(|l| !l.starts_with('*') && !l.starts_with("List of devices"))
        .filter_map(|l| {
            let (serial, state) = l.split_once('\t').or_else(|| l.trim().split_once(' '))?;
            let (serial, state) = (serial.trim(), state.trim());
            (!serial.is_empty() && !state.is_empty()).then(|| (serial.to_owned(), state.to_owned()))
        })
        .collect()
}

/// Whether `adb reverse --list` shows the forward for `spec` ("tcp:46710"). Each line
/// is "<connection> <port on the tablet> <port here>", e.g. "UsbFfs tcp:46710 tcp:46710".
fn has_reverse(list: &str, spec: &str) -> bool {
    list.lines().any(|l| {
        let mut parts = l.split_whitespace().rev();
        parts.next() == Some(spec) && parts.next() == Some(spec)
    })
}

/// Why a tablet adb lists can't connect yet, from its `adb devices` state.
fn not_ready(state: &str) -> String {
    match state {
        "unauthorized" | "authorizing" => {
            "tap Allow on the tablet's \"Allow USB debugging?\" prompt".to_owned()
        }
        "offline" => "adb can't talk to it; unplug the cable and plug it back in".to_owned(),
        "connecting" => "still connecting".to_owned(),
        s if s.starts_with("no permissions") => {
            "this computer isn't allowed to use it (on Linux, add a udev rule for it)".to_owned()
        }
        "recovery" | "rescue" | "sideload" | "bootloader" => {
            format!("it's in {state} mode; start it as usual")
        }
        other => format!("adb says it's \"{other}\""),
    }
}

/// What a failed adb call said: the last line it printed to stderr. adb puts progress
/// such as "* daemon started successfully" before the error.
fn failure(stderr: &str) -> Option<String> {
    stderr
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(str::to_owned)
}

/// An `adb` command that never opens a console window. The installed app is a GUI
/// process with no console, so without `CREATE_NO_WINDOW` Windows would open a new
/// console (a Terminal window on Windows 11) for every poll.
fn adb_command(adb: &Path) -> Command {
    let mut c = Command::new(adb);
    c.stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Runs one adb command and returns what it printed, or what went wrong. A trait so the
/// tests can stand in for adb.
trait Adb {
    async fn run(&self, args: &[&str]) -> Result<String, String>;
}

/// The real `adb`, with a time limit on every call.
struct AdbExe {
    path: PathBuf,
    timeout: Duration,
}

impl Adb for AdbExe {
    async fn run(&self, args: &[&str]) -> Result<String, String> {
        // When the time is up the call is dropped, and `kill_on_drop` ends adb.
        let call = adb_command(&self.path).args(args).output();
        match tokio::time::timeout(self.timeout, call).await {
            Err(_) => Err(format!("no answer after {} s", self.timeout.as_secs_f32())),
            Ok(Err(e)) => Err(e.to_string()),
            Ok(Ok(o)) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
            Ok(Ok(o)) => Err(failure(&String::from_utf8_lossy(&o.stderr))
                .unwrap_or_else(|| format!("adb failed ({})", o.status))),
        }
    }
}

/// One look at the USB tablets: list them, and make sure each ready one has its forward.
/// `before` is what the last look found; changes are logged once, not on every poll.
async fn poll(adb: &impl Adb, port: u16, before: &UsbState) -> UsbState {
    let listed = match adb.run(&["devices"]).await {
        Ok(out) => list_devices(&out),
        Err(e) => {
            if before.error.as_ref() != Some(&e) {
                warn!("adb isn't answering, so USB tablets can't connect: {e}");
            }
            return UsbState {
                error: Some(e),
                ..UsbState::default()
            };
        }
    };
    let spec = format!("tcp:{port}");
    let mut now = UsbState::default();
    for (serial, state) in listed {
        if state != "device" {
            let why = not_ready(&state);
            if before.waiting.get(&serial) != Some(&why) {
                info!("USB tablet {serial} can't connect yet: {why}");
            }
            now.waiting.insert(serial, why);
            continue;
        }
        let known = before.forwarded.contains(&serial);
        if known
            && adb
                .run(&["-s", &serial, "reverse", "--list"])
                .await
                .is_ok_and(|list| has_reverse(&list, &spec))
        {
            now.forwarded.insert(serial);
            continue;
        }
        match adb.run(&["-s", &serial, "reverse", &spec, &spec]).await {
            Ok(_) if known => {
                info!("USB tablet {serial}'s port forward was gone (adb reconnected?); set it up again");
                now.forwarded.insert(serial);
            }
            Ok(_) => {
                info!("USB tablet {serial} connected (adb reverse {spec})");
                now.forwarded.insert(serial);
            }
            Err(e) => {
                let why = format!("couldn't forward the port ({e})");
                if before.waiting.get(&serial) != Some(&why) {
                    warn!("USB tablet {serial} {why}; trying again every few seconds");
                }
                now.waiting.insert(serial, why);
            }
        }
    }
    for serial in before.forwarded.difference(&now.forwarded) {
        if !now.waiting.contains_key(serial) {
            info!("USB tablet {serial} unplugged");
        }
    }
    now
}

pub fn spawn(port: u16, devices: UsbDevices) {
    let Some(path) = find_adb() else {
        debug!("adb not found; USB tablets need Wi-Fi");
        return;
    };
    info!(
        "adb found at {}; USB tablets will be connected automatically",
        path.display()
    );
    let adb = AdbExe {
        path,
        timeout: ADB_TIMEOUT,
    };
    tokio::spawn(async move {
        let mut every = tokio::time::interval(EVERY);
        // After a slow poll (adb starting its server, or timing out) or a sleep, wait a
        // whole period rather than running the missed polls back to back.
        every.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut last = UsbState::default();
        loop {
            every.tick().await;
            let now = poll(&adb, port, &last).await;
            if now != last {
                devices.lock().expect("usb poisoned").clone_from(&now);
                last = now;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// Stands in for adb: answers by command line, and remembers what was run.
    #[derive(Default)]
    struct FakeAdb {
        answers: HashMap<String, Result<String, String>>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeAdb {
        fn answer(mut self, args: &str, answer: Result<&str, &str>) -> Self {
            let answer = answer.map(str::to_owned).map_err(str::to_owned);
            self.answers.insert(args.to_owned(), answer);
            self
        }

        /// What was run since the last call.
        fn calls(&self) -> Vec<String> {
            std::mem::take(&mut self.calls.lock().unwrap())
        }
    }

    impl Adb for FakeAdb {
        async fn run(&self, args: &[&str]) -> Result<String, String> {
            let line = args.join(" ");
            self.calls.lock().unwrap().push(line.clone());
            self.answers
                .get(&line)
                .cloned()
                .unwrap_or_else(|| Err(format!("nothing set up for `adb {line}`")))
        }
    }

    const ONE_TABLET: &str = "List of devices attached\nG000L40XXXXX\tdevice\n\n";
    const REVERSE: &str = "-s G000L40XXXXX reverse tcp:46710 tcp:46710";
    const LIST: &str = "-s G000L40XXXXX reverse --list";

    fn forwarded(serials: &[&str]) -> UsbState {
        UsbState {
            forwarded: serials.iter().map(|s| (*s).to_owned()).collect(),
            ..UsbState::default()
        }
    }

    #[test]
    fn reads_adb_devices() {
        let out = "List of devices attached\nG000L40XXXXX\tdevice\nemulator-5554\toffline\n\
                   ABC\tunauthorized\n192.168.1.40:5555\tdevice\n\
                   XYZ\tno permissions (missing udev rules? user is in the plugdev group); \
                   see [http://developer.android.com/tools/device.html]\n\n";
        let listed = list_devices(out);
        let states: Vec<_> = listed
            .iter()
            .map(|(serial, state)| (serial.as_str(), state.split(" (").next().unwrap()))
            .collect();
        assert_eq!(
            states,
            [
                ("G000L40XXXXX", "device"),
                ("emulator-5554", "offline"),
                ("ABC", "unauthorized"),
                ("192.168.1.40:5555", "device"),
                ("XYZ", "no permissions"),
            ]
        );
        assert!(list_devices("List of devices attached\n").is_empty());
        // Older adb printed the server's start-up lines to stdout.
        let starting = "* daemon not running; starting now at tcp:5037\n\
                        * daemon started successfully\n\
                        List of devices attached\nG000L40XXXXX\tdevice\n";
        assert_eq!(
            list_devices(starting),
            [("G000L40XXXXX".to_owned(), "device".to_owned())]
        );
    }

    #[test]
    fn finds_the_forward_in_reverse_list() {
        assert!(has_reverse("UsbFfs tcp:46710 tcp:46710\n", "tcp:46710"));
        assert!(has_reverse(
            "host-19 tcp:8081 tcp:8081\nUsbFfs tcp:46710 tcp:46710\n",
            "tcp:46710"
        ));
        assert!(!has_reverse("", "tcp:46710"));
        assert!(!has_reverse("UsbFfs tcp:46711 tcp:46711\n", "tcp:46710"));
        assert!(!has_reverse("UsbFfs tcp:467100 tcp:467100\n", "tcp:46710"));
        assert!(!has_reverse("UsbFfs tcp:46710 tcp:8081\n", "tcp:46710"));
    }

    #[test]
    fn keeps_the_last_line_of_an_adb_error() {
        let stderr = "* daemon not running; starting now at tcp:5037\n\
                      * daemon started successfully\n\
                      adb: error: cannot bind listener: Address already in use\n\n";
        assert_eq!(
            failure(stderr).as_deref(),
            Some("adb: error: cannot bind listener: Address already in use")
        );
        assert_eq!(failure(" \n"), None);
    }

    #[tokio::test]
    async fn new_tablets_are_forwarded() {
        let adb = FakeAdb::default()
            .answer("devices", Ok(ONE_TABLET))
            .answer(REVERSE, Ok(""));
        let now = poll(&adb, 46710, &UsbState::default()).await;
        assert_eq!(now, forwarded(&["G000L40XXXXX"]));
        assert_eq!(adb.calls(), ["devices", REVERSE]);
    }

    #[tokio::test]
    async fn a_failed_forward_is_not_counted_and_is_tried_again() {
        let adb = FakeAdb::default().answer("devices", Ok(ONE_TABLET)).answer(
            REVERSE,
            Err("adb: error: cannot bind listener: Address already in use"),
        );
        let first = poll(&adb, 46710, &UsbState::default()).await;
        assert!(first.forwarded.is_empty());
        assert_eq!(
            first.problems(),
            ["G000L40XXXXX: couldn't forward the port \
              (adb: error: cannot bind listener: Address already in use)"]
        );
        adb.calls();
        let second = poll(&adb, 46710, &first).await;
        assert_eq!(adb.calls(), ["devices", REVERSE]);
        assert_eq!(second, first);

        let adb = adb.answer(REVERSE, Ok(""));
        assert_eq!(
            poll(&adb, 46710, &second).await,
            forwarded(&["G000L40XXXXX"])
        );
    }

    #[tokio::test]
    async fn a_forward_that_went_missing_is_put_back() {
        // adb reconnected to the tablet between two polls, which drops the forward.
        let adb = FakeAdb::default()
            .answer("devices", Ok(ONE_TABLET))
            .answer(LIST, Ok(""))
            .answer(REVERSE, Ok(""));
        let before = forwarded(&["G000L40XXXXX"]);
        assert_eq!(poll(&adb, 46710, &before).await, before);
        assert_eq!(adb.calls(), ["devices", LIST, REVERSE]);
    }

    #[tokio::test]
    async fn a_forward_still_in_place_is_left_alone() {
        let adb = FakeAdb::default()
            .answer("devices", Ok(ONE_TABLET))
            .answer(LIST, Ok("UsbFfs tcp:46710 tcp:46710\n"));
        let before = forwarded(&["G000L40XXXXX"]);
        assert_eq!(poll(&adb, 46710, &before).await, before);
        assert_eq!(adb.calls(), ["devices", LIST]);
    }

    #[tokio::test]
    async fn network_serials_are_forwarded_too() {
        let adb = FakeAdb::default()
            .answer(
                "devices",
                Ok("List of devices attached\nG000L40XXXXX\tdevice\n192.168.1.40:5555\tdevice\n"),
            )
            .answer(REVERSE, Ok(""))
            .answer("-s 192.168.1.40:5555 reverse tcp:46710 tcp:46710", Ok(""));
        let now = poll(&adb, 46710, &UsbState::default()).await;
        assert_eq!(now, forwarded(&["192.168.1.40:5555", "G000L40XXXXX"]));
    }

    #[tokio::test]
    async fn tablets_that_are_not_ready_say_why() {
        let adb = FakeAdb::default().answer(
            "devices",
            Ok("List of devices attached\nG000L40XXXXX\tunauthorized\nemulator-5554\toffline\n"),
        );
        let now = poll(&adb, 46710, &forwarded(&["G000L40XXXXX"])).await;
        assert!(now.forwarded.is_empty());
        assert_eq!(
            now.problems(),
            [
                "G000L40XXXXX: tap Allow on the tablet's \"Allow USB debugging?\" prompt",
                "emulator-5554: adb can't talk to it; unplug the cable and plug it back in",
            ]
        );
        // Nothing is forwarded to a tablet that isn't ready.
        assert_eq!(adb.calls(), ["devices"]);
        assert_eq!(
            now.describe(),
            "adb found; 0 tablet(s) on a cable; one can't connect: tap Allow on the tablet's \
             \"Allow USB debugging?\" prompt; one can't connect: adb can't talk to it; unplug \
             the cable and plug it back in"
        );
    }

    #[tokio::test]
    async fn adb_not_answering_forgets_the_tablets() {
        let adb = FakeAdb::default().answer("devices", Err("no answer after 10 s"));
        let now = poll(&adb, 46710, &forwarded(&["G000L40XXXXX"])).await;
        assert!(now.forwarded.is_empty());
        assert_eq!(
            now.problems(),
            ["adb isn't answering: no answer after 10 s"]
        );
    }

    fn system_program(windows: &str, unix: &str) -> PathBuf {
        if cfg!(windows) {
            let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
            PathBuf::from(root).join("System32").join(windows)
        } else {
            PathBuf::from(unix)
        }
    }

    #[tokio::test]
    async fn a_hung_adb_call_times_out() {
        // Something that takes about 5 s, standing in for a wedged adb.
        let adb = AdbExe {
            path: system_program("PING.EXE", "/bin/sleep"),
            timeout: Duration::from_millis(300),
        };
        let args: &[&str] = if cfg!(windows) {
            &["-n", "6", "127.0.0.1"]
        } else {
            &["5"]
        };
        let started = std::time::Instant::now();
        assert_eq!(adb.run(args).await, Err("no answer after 0.3 s".to_owned()));
        let took = started.elapsed();
        assert!(took < Duration::from_secs(3), "took {took:?}");
    }

    #[tokio::test]
    async fn a_failing_adb_call_says_so() {
        let adb = AdbExe {
            path: system_program("cmd.exe", "/bin/sh"),
            timeout: ADB_TIMEOUT,
        };
        let args: &[&str] = if cfg!(windows) {
            &["/c", "exit 3"]
        } else {
            &["-c", "exit 3"]
        };
        let answer = adb.run(args).await.unwrap_err();
        assert!(
            answer.starts_with("adb failed (") && answer.contains('3'),
            "{answer}"
        );
    }
}
