use std::sync::{Arc, Mutex};

use super::*;

/// Records everything instead of touching the OS.
#[derive(Default)]
struct Recorder {
    log: Mutex<Vec<String>>,
}

impl Recorder {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.log.lock().unwrap())
    }
}

impl InputBackend for Recorder {
    fn chord(&self, chord: &Chord) -> Result<(), ActionError> {
        self.log.lock().unwrap().push(format!("chord {chord}"));
        Ok(())
    }
    fn text(&self, text: &str) -> Result<(), ActionError> {
        self.log.lock().unwrap().push(format!("text {text}"));
        Ok(())
    }
    fn media(&self, key: MediaKey) -> Result<(), ActionError> {
        self.log.lock().unwrap().push(format!("media {key:?}"));
        Ok(())
    }
}

impl SystemBackend for Recorder {
    fn open(&self, target: &str) -> Result<(), ActionError> {
        self.log.lock().unwrap().push(format!("open {target}"));
        Ok(())
    }
}

fn setup() -> (Executor, Arc<Recorder>) {
    let r = Arc::new(Recorder::default());
    (Executor::new(r.clone(), r.clone()), r)
}

fn spec(json: &str) -> ActionSpec {
    serde_json::from_str(json).unwrap()
}

#[tokio::test]
async fn input_actions_reach_the_backend() {
    let (ex, r) = setup();
    assert_eq!(
        ex.run(&spec(r#"{"type":"hotkey","keys":"Ctrl+Shift+M"}"#))
            .await
            .unwrap(),
        Outcome::Ok
    );
    ex.run(&spec(r#"{"type":"text","text":"héllo 👋"}"#))
        .await
        .unwrap();
    ex.run(&spec(r#"{"type":"media","key":"playPause"}"#))
        .await
        .unwrap();
    ex.run(&spec(
        r#"{"type":"openUrl","url":"https://perritodeck.dev"}"#,
    ))
    .await
    .unwrap();
    ex.run(&spec(r#"{"type":"none"}"#)).await.unwrap();
    assert_eq!(
        r.take(),
        vec![
            "chord Ctrl+Shift+M",
            "text héllo 👋",
            "media PlayPause",
            "open https://perritodeck.dev"
        ]
    );
}

#[tokio::test]
async fn unsafe_url_schemes_are_refused() {
    let (ex, r) = setup();
    for url in [
        "file:///etc/passwd",
        "javascript:alert(1)",
        "ms-settings:",
        "  ",
    ] {
        let err = ex
            .run(&ActionSpec::OpenUrl { url: url.into() })
            .await
            .unwrap_err();
        assert!(matches!(err, ActionError::UrlScheme(_)), "{url}");
    }
    assert!(r.take().is_empty());
}

#[tokio::test]
async fn multi_runs_in_order_and_validates_up_front() {
    let (ex, r) = setup();
    let m = spec(
        r#"{"type":"multi","steps":[
            {"action":{"type":"hotkey","keys":"Ctrl+C"},"delayMs":10},
            {"action":{"type":"text","text":"done"}}
        ]}"#,
    );
    let start = std::time::Instant::now();
    ex.run(&m).await.unwrap();
    assert!(start.elapsed() >= std::time::Duration::from_millis(10));
    assert_eq!(r.take(), vec!["chord Ctrl+C", "text done"]);

    // A bad hotkey anywhere means nothing runs at all.
    let bad = spec(
        r#"{"type":"multi","steps":[
            {"action":{"type":"text","text":"first"}},
            {"action":{"type":"hotkey","keys":"Ctrl+Nope"}}
        ]}"#,
    );
    assert!(matches!(ex.run(&bad).await, Err(ActionError::Hotkey(_))));
    assert!(r.take().is_empty());
}

#[tokio::test]
async fn random_runs_exactly_one_of_its_actions() {
    let (ex, r) = setup();
    let pick = spec(
        r#"{"type":"random","actions":[
            {"type":"text","text":"a"},
            {"type":"text","text":"b"},
            {"type":"text","text":"c"}
        ]}"#,
    );
    let mut seen = std::collections::HashSet::new();
    for _ in 0..200 {
        ex.run(&pick).await.unwrap();
        let log = r.take();
        assert_eq!(log.len(), 1, "{log:?}");
        assert!(["text a", "text b", "text c"].contains(&log[0].as_str()));
        seen.insert(log[0].clone());
    }
    // 200 picks from 3 all miss one with odds of about 3 × (2/3)^200: never.
    assert_eq!(seen.len(), 3);

    // One choice is always that one.
    let one = spec(r#"{"type":"random","actions":[{"type":"hotkey","keys":"Ctrl+C"}]}"#);
    ex.run(&one).await.unwrap();
    assert_eq!(r.take(), vec!["chord Ctrl+C"]);
}

#[tokio::test]
async fn random_validates_every_choice_up_front() {
    let (ex, r) = setup();
    let bad = spec(
        r#"{"type":"random","actions":[
            {"type":"text","text":"fine"},
            {"type":"hotkey","keys":"Ctrl+Nope"}
        ]}"#,
    );
    for _ in 0..20 {
        assert!(matches!(ex.run(&bad).await, Err(ActionError::Hotkey(_))));
    }
    assert!(r.take().is_empty());

    let empty = spec(r#"{"type":"random","actions":[]}"#);
    assert!(matches!(
        Executor::validate(&empty),
        Err(ActionError::NothingToPick)
    ));
    // Missing list is a parse error, not an empty pick.
    assert!(serde_json::from_str::<ActionSpec>(r#"{"type":"random"}"#).is_err());

    // Inside a multi too.
    let nested = ActionSpec::Multi {
        steps: vec![Step {
            action: empty,
            delay_ms: 0,
        }],
    };
    assert!(matches!(
        Executor::validate(&nested),
        Err(ActionError::NothingToPick)
    ));
}

#[test]
fn random_nesting_counts_like_multi() {
    // Exactly MAX_NESTING levels is fine, one more isn't, mixing both kinds.
    let wrap = |inner: ActionSpec, i: usize| {
        if i % 2 == 0 {
            ActionSpec::Random {
                actions: vec![inner],
            }
        } else {
            ActionSpec::Multi {
                steps: vec![Step {
                    action: inner,
                    delay_ms: 0,
                }],
            }
        }
    };
    let mut ok = ActionSpec::None;
    for i in 0..MAX_NESTING {
        ok = wrap(ok, i);
    }
    assert!(Executor::validate(&ok).is_ok());
    let deep = wrap(ok, MAX_NESTING);
    assert!(matches!(
        Executor::validate(&deep),
        Err(ActionError::TooDeep)
    ));

    let mut only_random = ActionSpec::None;
    for _ in 0..=MAX_NESTING {
        only_random = ActionSpec::Random {
            actions: vec![only_random],
        };
    }
    assert!(matches!(
        Executor::validate(&only_random),
        Err(ActionError::TooDeep)
    ));
}

#[test]
fn random_round_trips_through_json() {
    let s = spec(r#"{"type":"random","actions":[{"type":"media","key":"next"},{"type":"none"}]}"#);
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["type"], "random");
    assert_eq!(json["actions"].as_array().unwrap().len(), 2);
    let back: ActionSpec = serde_json::from_value(json).unwrap();
    assert_eq!(s, back);
}

#[test]
fn limits() {
    let long_delay = ActionSpec::Multi {
        steps: vec![Step {
            action: ActionSpec::None,
            delay_ms: MAX_DELAY_MS + 1,
        }],
    };
    assert!(matches!(
        Executor::validate(&long_delay),
        Err(ActionError::DelayTooLong(_))
    ));

    let mut deep = ActionSpec::None;
    for _ in 0..=MAX_NESTING {
        deep = ActionSpec::Multi {
            steps: vec![Step {
                action: deep,
                delay_ms: 0,
            }],
        };
    }
    assert!(matches!(
        Executor::validate(&deep),
        Err(ActionError::TooDeep)
    ));

    let long_cmd = spec(r#"{"type":"command","program":"x","timeoutMs":999999999}"#);
    assert!(matches!(
        Executor::validate(&long_cmd),
        Err(ActionError::TimeoutTooLong(_))
    ));
}

#[tokio::test]
async fn text_is_limited_in_characters() {
    let text = |s: String| ActionSpec::Text { text: s };
    // Characters, not bytes: the limit in two-byte characters is fine.
    assert!(Executor::validate(&text("é".repeat(MAX_TEXT_CHARS))).is_ok());
    let long = text("a".repeat(MAX_TEXT_CHARS + 1));
    let err = Executor::validate(&long).unwrap_err();
    assert!(matches!(err, ActionError::TextTooLong(n) if n == MAX_TEXT_CHARS + 1));
    assert!(err.to_string().contains("2000 character limit"), "{err}");
    // Inside a multi too, and nothing is typed.
    let (ex, r) = setup();
    let multi = ActionSpec::Multi {
        steps: vec![Step {
            action: long,
            delay_ms: 0,
        }],
    };
    assert!(matches!(
        ex.run(&multi).await,
        Err(ActionError::TextTooLong(_))
    ));
    assert!(r.take().is_empty());
}

#[test]
fn integration_errors_name_the_service_once() {
    let says = |service: &str, reason: &str| {
        ActionError::Integration {
            service: service.into(),
            reason: reason.into(),
        }
        .to_string()
    };
    assert_eq!(says("OBS", "already recording"), "OBS: already recording");
    assert_eq!(
        says("OBS", "OBS didn't answer in time"),
        "OBS didn't answer in time"
    );
    assert_eq!(
        says("OBS", "OBS's WebSocket server uses ws://"),
        "OBS's WebSocket server uses ws://"
    );
    assert_eq!(
        says("Home Assistant", "Home Assistant answered 500"),
        "Home Assistant answered 500"
    );
    // Only the whole name counts.
    assert_eq!(says("OBS", "OBSBOT not found"), "OBS: OBSBOT not found");

    let mistake = ActionError::IntegrationsMistake {
        service: "OBS".into(),
        mistake: "line 3: unknown field `pasword`".into(),
    };
    assert_eq!(
        mistake.to_string(),
        "integrations.toml has a mistake, so OBS is off: line 3: unknown field `pasword`"
    );
}

#[test]
fn unknown_fields_are_rejected() {
    assert!(
        serde_json::from_str::<ActionSpec>(r#"{"type":"text","text":"a","shell":true}"#).is_err()
    );
    assert!(serde_json::from_str::<ActionSpec>(r#"{"type":"rm-rf"}"#).is_err());
}

#[test]
fn specs_round_trip_through_json() {
    let s =
        spec(r#"{"type":"command","program":"git","args":["pull"],"cwd":"/tmp","timeoutMs":5000}"#);
    let back: ActionSpec = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(s, back);
}

#[tokio::test]
async fn pasted_paths_lose_their_quotes() {
    let (ex, r) = setup();
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().display().to_string();
    let open = ActionSpec::OpenPath {
        path: format!("\"{folder}\"").into(),
    };
    ex.run(&open).await.unwrap();
    assert_eq!(r.take(), vec![format!("open {folder}")]);
}

#[tokio::test]
async fn a_missing_file_is_an_error_not_a_dialog() {
    let (ex, r) = setup();
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no such file.txt");
    let open = ActionSpec::OpenPath {
        path: missing.clone(),
    };
    let err = ex.run(&open).await.unwrap_err().to_string();
    assert_eq!(
        err,
        format!("could not open \"{}\": it doesn't exist", missing.display())
    );
    assert!(r.take().is_empty());
    // Links and shell folders aren't files, so they go to the system as they are.
    let link = spec(r#"{"type":"openPath","path":"shell:Downloads"}"#);
    ex.run(&link).await.unwrap();
    assert_eq!(r.take(), vec!["open shell:Downloads"]);
}

#[tokio::test]
async fn launch_errors_show_the_path_as_typed() {
    let (ex, _) = setup();
    let missing = if cfg!(windows) {
        r"C:\no\such\folder\app.exe"
    } else {
        "/no/such/folder/app"
    };
    let launch = ActionSpec::Launch {
        program: missing.into(),
        args: vec![],
    };
    let err = ex.run(&launch).await.unwrap_err().to_string();
    // Not `"C:\\no\\such…"`, as `{:?}` would print it.
    assert!(
        err.starts_with(&format!("could not start \"{missing}\": ")),
        "{err}"
    );
}

// Command tests use programs present on every CI runner.
#[cfg(unix)]
mod commands {
    use super::*;

    #[tokio::test]
    async fn success_failure_timeout_and_missing_program() {
        let (ex, _) = setup();
        let ok = spec(r#"{"type":"command","program":"true"}"#);
        assert_eq!(ex.run(&ok).await.unwrap(), Outcome::Ok);

        let fail =
            spec(r#"{"type":"command","program":"sh","args":["-c","echo boom >&2; exit 3"]}"#);
        assert_eq!(
            ex.run(&fail).await.unwrap(),
            Outcome::Alert("exit 3: boom".into())
        );

        let slow = spec(r#"{"type":"command","program":"sleep","args":["5"],"timeoutMs":100}"#);
        assert!(
            matches!(ex.run(&slow).await.unwrap(), Outcome::Alert(a) if a.contains("timed out"))
        );

        let missing = spec(r#"{"type":"command","program":"definitely-not-a-real-program-xyz"}"#);
        assert!(matches!(
            ex.run(&missing).await,
            Err(ActionError::Spawn { .. })
        ));

        let quoted = spec(r#"{"type":"command","program":"\"/bin/sh\"","args":["-c","exit 0"]}"#);
        assert_eq!(ex.run(&quoted).await.unwrap(), Outcome::Ok);
    }

    #[tokio::test]
    async fn arguments_are_not_shell_interpreted() {
        let (ex, _) = setup();
        let dir = std::env::temp_dir().join(format!("perritodeck-argv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // If this went through a shell, `; touch pwned` would create a file.
        let s = ActionSpec::Command {
            program: "echo".into(),
            args: vec!["hi; touch pwned".into()],
            cwd: Some(dir.clone()),
            timeout_ms: None,
        };
        assert_eq!(ex.run(&s).await.unwrap(), Outcome::Ok);
        assert!(!dir.join("pwned").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn multi_continues_after_alert() {
        let (ex, r) = setup();
        let m = spec(
            r#"{"type":"multi","steps":[
                {"action":{"type":"command","program":"false"}},
                {"action":{"type":"text","text":"after"}}
            ]}"#,
        );
        assert!(matches!(ex.run(&m).await.unwrap(), Outcome::Alert(_)));
        assert_eq!(r.take(), vec!["text after"]);
    }
}

// The same through cmd.exe, which every Windows machine has. Commands get
// CREATE_NO_WINDOW, so nothing shows on screen.
#[cfg(windows)]
mod windows_commands {
    use super::*;

    fn command(program: &str, args: &[&str], timeout_ms: Option<u64>) -> ActionSpec {
        ActionSpec::Command {
            program: program.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: None,
            timeout_ms,
        }
    }

    fn system32() -> std::path::PathBuf {
        std::path::Path::new(&std::env::var("SystemRoot").unwrap()).join("System32")
    }

    #[tokio::test]
    async fn success_failure_timeout_and_missing_program() {
        let (ex, _) = setup();
        let ok = command("cmd", &["/c", "exit 0"], None);
        assert_eq!(ex.run(&ok).await.unwrap(), Outcome::Ok);

        let fail = command("cmd", &["/c", "echo boom 1>&2 & exit 3"], None);
        assert_eq!(
            ex.run(&fail).await.unwrap(),
            Outcome::Alert("exit 3: boom".into())
        );

        let slow = command("cmd", &["/c", "ping -n 5 127.0.0.1 >nul"], Some(300));
        assert!(
            matches!(ex.run(&slow).await.unwrap(), Outcome::Alert(a) if a.contains("timed out"))
        );

        let missing = command("definitely-not-a-real-program-xyz", &[], None);
        assert!(matches!(
            ex.run(&missing).await,
            Err(ActionError::Spawn { .. })
        ));
    }

    /// A folder with `start.cmd`, which starts `slow.cmd` and returns, and `slow.cmd`,
    /// which writes `marker.txt` after about `slow` seconds. `start.cmd` waits `wait`
    /// seconds after starting it (ping's `-n` counts from 1).
    fn scripts(wait: u32, slow: u32) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("start.cmd"),
            format!(
                "@start \"\" /b \"%~dp0slow.cmd\"\r\n@ping -n {} 127.0.0.1 >nul\r\n",
                wait + 1
            ),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("slow.cmd"),
            format!(
                "@ping -n {} 127.0.0.1 >nul\r\n@echo done>\"%~dp0marker.txt\"\r\n",
                slow + 1
            ),
        )
        .unwrap();
        dir
    }

    async fn marker_after(dir: &std::path::Path, secs: u64) -> bool {
        let marker = dir.join("marker.txt");
        for _ in 0..secs * 10 {
            if marker.exists() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        marker.exists()
    }

    #[tokio::test]
    async fn a_command_that_starts_a_program_finishes_when_it_exits() {
        let (ex, _) = setup();
        let dir = scripts(0, 6);
        let start = dir.path().join("start.cmd");
        // slow.cmd inherits the error pipe and holds it for 6 s; the key mustn't wait. The
        // limits leave a loaded machine room: waiting for the pipe would take 6 s, or hit
        // the 4.5 s timeout and come back as an alert.
        let began = std::time::Instant::now();
        let run = command(start.to_str().unwrap(), &[], Some(4500));
        assert_eq!(ex.run(&run).await.unwrap(), Outcome::Ok);
        assert!(began.elapsed().as_millis() < 4000, "{:?}", began.elapsed());
        // And what it started keeps running.
        assert!(marker_after(dir.path(), 10).await);
    }

    #[tokio::test]
    async fn a_timeout_ends_what_the_command_started() {
        let (ex, _) = setup();
        let dir = scripts(5, 2);
        let start = dir.path().join("start.cmd");
        let run = command(start.to_str().unwrap(), &[], Some(500));
        assert!(
            matches!(ex.run(&run).await.unwrap(), Outcome::Alert(a) if a.contains("timed out"))
        );
        // Ending only cmd.exe would leave slow.cmd to write its marker.
        assert!(!marker_after(dir.path(), 4).await);
    }

    #[tokio::test]
    async fn a_pasted_quoted_path_runs() {
        let (ex, _) = setup();
        let cmd = system32().join("cmd.exe");
        let quoted = command(&format!("\"{}\"", cmd.display()), &["/c", "exit 0"], None);
        assert_eq!(ex.run(&quoted).await.unwrap(), Outcome::Ok);
    }

    #[tokio::test]
    async fn a_relative_program_is_found_in_the_working_folder() {
        let (ex, _) = setup();
        // The test runs in the crate's folder, which has no cmd.exe.
        let s = ActionSpec::Command {
            program: r".\cmd.exe".into(),
            args: vec!["/c".into(), "exit 0".into()],
            cwd: Some(system32()),
            timeout_ms: None,
        };
        assert_eq!(ex.run(&s).await.unwrap(), Outcome::Ok);
    }
}

// ---- Which actions run programs (shared by MCP and the import check) ----

fn cmd(program: &str) -> ActionSpec {
    ActionSpec::Command {
        program: program.into(),
        args: vec![],
        cwd: None,
        timeout_ms: None,
    }
}

fn steps(actions: Vec<ActionSpec>) -> ActionSpec {
    ActionSpec::Multi {
        steps: actions
            .into_iter()
            .map(|action| Step {
                action,
                delay_ms: 0,
            })
            .collect(),
    }
}

#[test]
fn runs_programs_sorts_every_kind() {
    let runs = [
        cmd("rm"),
        ActionSpec::Launch {
            program: "notepad".into(),
            args: vec![],
        },
        ActionSpec::OpenPath {
            path: "C:/x.bat".into(),
        },
        ActionSpec::Plugin {
            plugin: "p".into(),
            action: "a".into(),
            settings: serde_json::Value::Null,
        },
        ActionSpec::SdPlugin {
            plugin: "com.x".into(),
            action: "com.x.y".into(),
            settings: serde_json::Value::Null,
        },
    ];
    for a in &runs {
        assert!(a.runs_programs(), "{a:?}");
    }
    let quiet = [
        ActionSpec::Hotkey {
            keys: "Win+R".into(),
        },
        ActionSpec::Text {
            text: "ls\n".into(),
        },
        ActionSpec::Media {
            key: MediaKey::Mute,
        },
        ActionSpec::OpenUrl {
            url: "https://perritodeck.dev".into(),
        },
        ActionSpec::Obs(ObsAction::StartStream),
        ActionSpec::HomeAssistant {
            service: "lock.unlock".into(),
            entity: None,
            data: None,
        },
        ActionSpec::None,
        // Not themselves: what's inside is looked at through `leaves`.
        steps(vec![cmd("rm")]),
        ActionSpec::Random {
            actions: vec![cmd("rm")],
        },
    ];
    for a in &quiet {
        assert!(!a.runs_programs(), "{a:?}");
    }
    let sliders_that_run = [
        SliderSpec::Command {
            program: "x".into(),
            args: vec!["{value}".into()],
        },
        SliderSpec::Plugin {
            plugin: "p".into(),
            action: "a".into(),
            settings: serde_json::Value::Null,
            follow: None,
        },
    ];
    for s in &sliders_that_run {
        assert!(s.runs_programs(), "{s:?}");
    }
    let quiet_sliders = [
        SliderSpec::Volume,
        SliderSpec::Obs {
            input: "Mic".into(),
        },
        SliderSpec::HomeAssistant {
            entity: "light.desk".into(),
        },
    ];
    for s in &quiet_sliders {
        assert!(!s.runs_programs(), "{s:?}");
    }
}

#[test]
fn leaves_keep_order_and_place() {
    // multi[none, random[media, multi[command]]]
    let a = steps(vec![
        ActionSpec::None,
        ActionSpec::Random {
            actions: vec![
                ActionSpec::Media {
                    key: MediaKey::Next,
                },
                steps(vec![cmd("rm")]),
            ],
        },
    ]);
    let leaves = a.leaves();
    assert_eq!(leaves.len(), 3);
    assert_eq!(
        leaves[0],
        (vec![Place::Step { n: 1, of: 2 }], &ActionSpec::None)
    );
    assert_eq!(
        leaves[1].0,
        [Place::Step { n: 2, of: 2 }, Place::Choice { n: 1, of: 2 }]
    );
    assert!(matches!(leaves[1].1, ActionSpec::Media { .. }));
    assert_eq!(
        leaves[2],
        (
            vec![
                Place::Step { n: 2, of: 2 },
                Place::Choice { n: 2, of: 2 },
                Place::Step { n: 1, of: 1 },
            ],
            &cmd("rm")
        )
    );
    // A plain action is its own only leaf, with no place.
    assert_eq!(cmd("rm").leaves(), [(vec![], &cmd("rm"))]);
    // An empty multi has none.
    assert!(steps(vec![]).leaves().is_empty());
    // On the wire, as the import preview sends it.
    assert_eq!(
        serde_json::to_value(Place::Choice { n: 1, of: 4 }).unwrap(),
        serde_json::json!({"in": "choice", "n": 1, "of": 4})
    );
}

/// Takes a deep action apart one level at a time, even when a test fails: dropping it
/// whole would recurse as deep as it is.
struct Deep(ActionSpec);

impl Drop for Deep {
    fn drop(&mut self) {
        let mut a = std::mem::replace(&mut self.0, ActionSpec::None);
        while let ActionSpec::Multi { mut steps } = a {
            match steps.pop() {
                Some(s) => a = s.action,
                None => break,
            }
        }
    }
}

#[test]
fn leaves_is_iterative() {
    const DEPTH: usize = 10_000;
    let mut a = Deep(cmd("deep"));
    for _ in 0..DEPTH {
        let inner = std::mem::replace(&mut a.0, ActionSpec::None);
        a.0 = steps(vec![inner]);
    }
    let leaves = a.0.leaves();
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].0.len(), DEPTH);
    assert_eq!(leaves[0].1, &cmd("deep"));
}

#[test]
fn presses_enter() {
    for s in [
        "Enter",
        "Return",
        "Ctrl+Enter",
        "Shift+Enter",
        "Ctrl+M",
        "Ctrl+J",
        "ctrl+m",
    ] {
        assert!(s.parse::<Chord>().unwrap().presses_enter(), "{s}");
    }
    for s in [
        "Ctrl+Shift+M",
        "Ctrl+C",
        "Alt+M",
        "M",
        "Win+R",
        "Ctrl+Alt+J",
    ] {
        assert!(!s.parse::<Chord>().unwrap().presses_enter(), "{s}");
    }
}
