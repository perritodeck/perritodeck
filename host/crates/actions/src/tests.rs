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
