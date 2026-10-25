use super::*;

const ECHO_PLUGIN: &str = r#"
import json, sys
def send(m):
    sys.stdout.write(json.dumps(m) + "\n"); sys.stdout.flush()
for line in sys.stdin:
    m = json.loads(line)
    if m["event"] == "hello":
        assert m["protocol"] == 1
        send({"event": "ready"})
        send({"event": "value", "id": "greeting", "value": "hi " + m["host"]})
        send({"event": "value", "id": "undeclared", "value": "ignored"})
        print("started", file=sys.stderr, flush=True)
    elif m["event"] == "run":
        s = m.get("settings") or {}
        if m["action"] == "fail":
            send({"event": "result", "id": m["id"], "ok": False, "message": "nope"})
        elif m["action"] == "slow":
            pass
        else:
            send({"event": "result", "id": m["id"], "ok": True, "message": s.get("warn")})
    elif m["event"] == "shutdown":
        break
"#;

fn write_plugin(root: &Path, id: &str, runtime: &str, main: &str, script: Option<&str>) -> PathBuf {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = json!({
        "id": id, "name": format!("Test {id}"), "version": "1.0.0", "runtime": runtime, "main": main,
        "actions": [{ "id": "hello", "name": "Hello" }, { "id": "fail", "name": "Fail" }, { "id": "slow", "name": "Slow" }],
        "values": [{ "id": "greeting", "name": "Greeting" }]
    });
    std::fs::write(dir.join(MANIFEST), manifest.to_string()).unwrap();
    if let Some(s) = script {
        std::fs::write(dir.join(main), s).unwrap();
    }
    dir
}

fn has_python() -> bool {
    let candidates: &[&str] = if cfg!(windows) {
        &["py", "python"]
    } else {
        &["python3", "python"]
    };
    // `.status.success()` and not just "it ran": on Windows without Python, `python` is
    // the Microsoft Store shortcut, which runs and exits with 9009.
    candidates.iter().any(|p| {
        std::process::Command::new(p)
            .args(if *p == "py" {
                &["-3", "--version"][..]
            } else {
                &["--version"][..]
            })
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

#[test]
fn a_python_must_name_its_interpreter() {
    let me = std::env::current_exe().unwrap();
    let printed = format!("{}\r\n", me.display());
    assert_eq!(interpreter_from(true, printed.as_bytes()), Some(me));
    // The Microsoft Store shortcut fails (9009), Python 2 prints nothing, and MSYS's
    // Python names a path Windows doesn't have.
    assert_eq!(interpreter_from(false, printed.as_bytes()), None);
    assert_eq!(interpreter_from(true, b""), None);
    assert_eq!(interpreter_from(true, b"/no/such/bin/python"), None);
}

#[cfg(windows)]
#[tokio::test]
async fn windows_finds_python_3_or_says_there_is_none() {
    // `py` exits with 103 when it has no such Python, and a missing program can't
    // start: both mean "no Python 3", not a plugin that crashes over and over.
    assert_eq!(
        find_python(&[("perritodeck-no-such-python", &[]), ("py", &["-3.99"])]).await,
        None
    );
    if has_python() {
        let python = find_python(WINDOWS_PYTHONS).await.expect("Python 3");
        let name = python.file_name().unwrap().to_string_lossy().to_lowercase();
        assert!(name.starts_with("python"), "{}", python.display());
    }
}

async fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn runs_actions_and_publishes_values() {
    if !has_python() {
        eprintln!("skipping: no Python");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    write_plugin(
        root.path(),
        "test.echo",
        "python",
        "plugin.py",
        Some(ECHO_PLUGIN),
    );
    let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let sink = seen.clone();
    let host = PluginHost::start(
        root.path().to_owned(),
        "9.9",
        Arc::new(move |k, v| sink.lock().unwrap().push((k, v))),
    );
    wait_for("ready", || {
        host.list()
            .first()
            .is_some_and(|p| p.status == Status::Running)
    })
    .await;
    wait_for("value", || !seen.lock().unwrap().is_empty()).await;
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [(
            "plugin:test.echo:greeting".to_string(),
            "hi 9.9".to_string()
        )]
    );

    assert_eq!(
        host.run_plugin("test.echo", "hello", &Value::Null)
            .await
            .unwrap(),
        Outcome::Ok
    );
    assert_eq!(
        host.run_plugin("test.echo", "hello", &json!({ "warn": "careful" }))
            .await
            .unwrap(),
        Outcome::Alert("careful".into())
    );
    let err = host
        .run_plugin("test.echo", "fail", &Value::Null)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("nope"), "{err}");
    assert!(host
        .run_plugin("test.echo", "nonexistent", &Value::Null)
        .await
        .is_err());
    assert!(host
        .run_plugin("not.installed", "hello", &Value::Null)
        .await
        .is_err());

    // Restarting answers pending runs and comes back.
    host.restart("test.echo").unwrap();
    wait_for("ready again", || {
        host.list()
            .first()
            .is_some_and(|p| p.status == Status::Running)
    })
    .await;
    assert_eq!(
        host.run_plugin("test.echo", "hello", &Value::Null)
            .await
            .unwrap(),
        Outcome::Ok
    );
    host.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn crashing_and_missing_plugins_are_reported() {
    let root = tempfile::tempdir().unwrap();
    write_plugin(root.path(), "test.missing", "exec", "no-such-program", None);
    std::fs::create_dir_all(root.path().join("broken")).unwrap();
    std::fs::write(root.path().join("broken").join(MANIFEST), "{ not json").unwrap();
    if has_python() {
        write_plugin(
            root.path(),
            "test.crash",
            "python",
            "crash.py",
            Some("import sys; sys.exit(3)\n"),
        );
    }
    let host = PluginHost::start(root.path().to_owned(), "1", Arc::new(|_, _| {}));
    wait_for("missing runtime", || {
        host.list().iter().any(|p| {
            p.manifest.id == "test.missing" && matches!(p.status, Status::MissingRuntime { .. })
        })
    })
    .await;
    assert_eq!(host.broken().len(), 1);
    if has_python() {
        wait_for("crash detected", || {
            host.list().iter().any(|p| {
                p.manifest.id == "test.crash" && matches!(p.status, Status::Crashed { .. })
            })
        })
        .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn install_and_remove() {
    let root = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let folder = write_plugin(src.path(), "test.copy", "exec", "missing", None);
    let host = PluginHost::start(root.path().join("plugins"), "1", Arc::new(|_, _| {}));
    assert!(host.list().is_empty());
    assert!(
        host.install_from(src.path()).is_err(),
        "a folder without a manifest"
    );
    // Installs can come from threads outside the runtime (UI commands).
    let h = host.clone();
    let f = folder.clone();
    let id = std::thread::spawn(move || h.install_from(&f))
        .join()
        .unwrap();
    assert_eq!(id.unwrap(), "test.copy");
    assert!(root
        .path()
        .join("plugins/test.copy")
        .join(MANIFEST)
        .exists());
    assert_eq!(host.list().len(), 1);
    host.remove("test.copy").unwrap();
    assert!(host.list().is_empty());
    assert!(!root.path().join("plugins/test.copy").exists());
}

/// Plugins that ship with PerritoDeck share one MIT LICENSE; an installed copy of one
/// carries it unless it has a licence of its own. Other plugins never get it.
#[tokio::test(flavor = "multi_thread")]
async fn bundled_plugins_carry_the_shared_licence() {
    let root = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let licence = src.path().join("LICENSE");
    std::fs::write(&licence, "MIT License\n").unwrap();
    let plain = write_plugin(src.path(), "test.plain", "exec", "missing", None);
    let own = write_plugin(src.path(), "test.own", "exec", "missing", None);
    std::fs::write(own.join("LICENSE-Apache.md"), "Apache\n").unwrap();
    let other = write_plugin(src.path(), "test.other", "exec", "missing", None);
    let host = PluginHost::start(root.path().join("plugins"), "1", Arc::new(|_, _| {}));
    let installed = |id: &str| root.path().join("plugins").join(id);

    host.install_bundled(&plain, Some(&licence)).unwrap();
    assert_eq!(
        std::fs::read_to_string(installed("test.plain").join("LICENSE")).unwrap(),
        "MIT License\n"
    );
    host.install_bundled(&own, Some(&licence)).unwrap();
    assert!(!installed("test.own").join("LICENSE").exists());
    assert!(installed("test.own").join("LICENSE-Apache.md").exists());
    // No shared licence to copy, or a plugin that doesn't ship with PerritoDeck.
    host.install_bundled(&other, Some(&src.path().join("nope")))
        .unwrap();
    assert!(!installed("test.other").join("LICENSE").exists());
    host.install_from(&other).unwrap();
    assert!(!installed("test.other").join("LICENSE").exists());
    assert_eq!(host.list().len(), 3);
}

#[test]
fn licence_files_are_recognised_like_the_catalogue_does() {
    for name in [
        "LICENSE",
        "licence",
        "COPYING",
        "LICENSE.md",
        "License.txt",
        "LICENSE-MIT",
        "license-apache-2.rst",
        "COPYING.text",
    ] {
        assert!(is_licence_name(name), "{name}");
    }
    for name in [
        "license_check.py",
        "copying.py",
        "licenses.json",
        "licenses",
        "LICENSE-",
        "LICENSE.MIT",
        "license.tar.gz",
        "MYLICENSE",
    ] {
        assert!(!is_licence_name(name), "{name}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_being_removed_or_updated_isnt_started_again_by_a_scan() {
    let root = tempfile::tempdir().unwrap();
    write_plugin(root.path(), "test.busy", "exec", "missing", None);
    let host = PluginHost::start(root.path().to_owned(), "1", Arc::new(|_, _| {}));
    assert_eq!(host.list().len(), 1);
    let (taken, busy) = host.take_busy("test.busy").unwrap();
    assert!(taken.is_some());
    // Another command scans while it's stopping, with its folder still there.
    host.scan();
    assert!(host.list().is_empty(), "the scan left it alone");
    // A second removal or update of it at the same time is turned away.
    assert!(host.take_busy("test.busy").is_err());
    let err = host.remove("test.busy").unwrap_err();
    assert!(err.contains("still being removed"), "{err}");
    // Once that's over (here the folder stayed), scans see it again.
    drop(busy);
    host.scan();
    assert_eq!(host.list().len(), 1);
}

const ASKS_TO_OPEN: &str = r#"
import json, sys
def send(m):
    sys.stdout.write(json.dumps(m) + "\n"); sys.stdout.flush()
for line in sys.stdin:
    m = json.loads(line)
    if m["event"] == "hello":
        send({"event": "value", "id": "greeting", "value": "features " + ",".join(m.get("features", []))})
        send({"event": "ready"})
        send({"event": "open", "id": 1, "target": "https://perritodeck.dev/guide/"})
        send({"event": "open", "id": 2, "program": "code", "args": ["."]})
        send({"event": "open", "id": 3, "args": ["nothing to open"]})
        send({"event": "open", "id": 4, "target": "C:/no/such/file.txt"})
        send({"event": "open", "target": "https://example.com/no-answer-wanted"})
    elif m["event"] == "opened":
        send({"event": "value", "id": "greeting", "value": "%s %s %s" % (m["id"], m["ok"], m.get("message", ""))})
    elif m["event"] == "shutdown":
        break
"#;

/// Plugins can't start programs that outlive them on Windows (their process tree ends
/// with them), so they ask the host to open things for the person instead.
#[tokio::test(flavor = "multi_thread")]
async fn plugins_ask_the_host_to_open_things() {
    if !has_python() {
        eprintln!("skipping: no Python");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    write_plugin(
        root.path(),
        "test.opener",
        "python",
        "plugin.py",
        Some(ASKS_TO_OPEN),
    );
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let opened: Arc<Mutex<Vec<OpenRequest>>> = Arc::default();
    let (sink, record) = (seen.clone(), opened.clone());
    let opener: Opener = Arc::new(move |r: OpenRequest| {
        let record = record.clone();
        Box::pin(async move {
            let missing = matches!(&r, OpenRequest::Target(t) if t.contains("no/such"));
            record.lock().unwrap().push(r);
            if missing {
                Err("it doesn't exist".into())
            } else {
                Ok(())
            }
        })
    });
    let _host = PluginHost::start_with_opener(
        root.path().to_owned(),
        "9.9",
        Arc::new(move |_, v| sink.lock().unwrap().push(v)),
        opener,
    );
    wait_for("the replies", || seen.lock().unwrap().len() >= 5).await;
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen[0], "features open");
    let mut replies = seen[1..].to_vec();
    replies.sort();
    assert_eq!(
        replies,
        [
            "1 True ",
            "2 True ",
            "3 False say what to open: a target, or a program",
            "4 False it doesn't exist",
        ]
    );
    wait_for("the last open", || opened.lock().unwrap().len() == 4).await;
    let opened = opened.lock().unwrap().clone();
    assert!(opened.contains(&OpenRequest::Target(
        "https://perritodeck.dev/guide/".into()
    )));
    assert!(opened.contains(&OpenRequest::Program {
        program: "code".into(),
        args: vec![".".into()],
    }));
    assert!(opened.contains(&OpenRequest::Target(
        "https://example.com/no-answer-wanted".into()
    )));
}

#[test]
fn what_an_open_message_asks_for() {
    let req = |v: Value| open_request(&v);
    assert_eq!(
        req(json!({ "target": " https://a.example " })),
        Some(OpenRequest::Target("https://a.example".into()))
    );
    assert_eq!(
        req(json!({ "program": "obs64" })),
        Some(OpenRequest::Program {
            program: "obs64".into(),
            args: vec![]
        })
    );
    assert_eq!(req(json!({ "program": "x", "args": [1] })), None);
    assert_eq!(req(json!({ "program": "x", "args": "-v" })), None);
    assert_eq!(req(json!({ "target": "" })), None);
    assert_eq!(
        OpenRequest::Target("https://x.example/?code=secret".into()).describe(),
        "a web address"
    );
    assert_eq!(
        OpenRequest::Program {
            program: r"C:\Program Files\obs-studio\bin\64bit\obs64.exe".into(),
            args: vec![]
        }
        .describe(),
        if cfg!(windows) {
            "the program obs64.exe"
        } else {
            r"the program C:\Program Files\obs-studio\bin\64bit\obs64.exe"
        }
    );
}

#[test]
fn a_plugin_being_stopped_gets_no_actions_when_it_says_ready() {
    let manifest = Manifest::parse(
        &json!({ "id": "test.ready", "name": "Ready", "version": "1.0.0", "runtime": "exec", "main": "x" })
            .to_string(),
    )
    .unwrap();
    let sinks = Sinks {
        values: Arc::new(|_, _| {}),
        opener: None,
    };
    let (tx, _rx) = mpsc::unbounded_channel();
    let ready = json!({ "event": "ready" });

    let stopping = Running::new(manifest.clone(), PathBuf::new(), json!({}), PathBuf::new());
    stopping.request_stop();
    handle(&stopping, &ready, &sinks, &tx);
    assert!(stopping.to_plugin.lock().unwrap().is_none());
    assert_eq!(*stopping.status.lock().unwrap(), Status::Starting);

    let running = Running::new(manifest, PathBuf::new(), json!({}), PathBuf::new());
    handle(&running, &ready, &sinks, &tx);
    assert!(running.to_plugin.lock().unwrap().is_some());
    assert_eq!(*running.status.lock().unwrap(), Status::Running);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_program_that_wont_start_is_reported_as_such_not_as_missing() {
    let root = tempfile::tempdir().unwrap();
    // A text file is no program: Windows says it isn't a valid application, and
    // elsewhere it isn't executable.
    let dir = write_plugin(root.path(), "test.notes", "exec", "notes.txt", None);
    std::fs::write(dir.join("notes.txt"), "just some notes\n").unwrap();
    let host = PluginHost::start(root.path().to_owned(), "1", Arc::new(|_, _| {}));
    wait_for("it to fail", || {
        host.list()
            .first()
            .is_some_and(|p| p.status != Status::Starting)
    })
    .await;
    match &host.list()[0].status {
        Status::Crashed { reason } => assert!(reason.contains("couldn't start"), "{reason}"),
        other => panic!("expected a start failure, got {other:?}"),
    }
}

const WATCH_PLUGIN: &str = r#"
import json, sys
def send(m):
    sys.stdout.write(json.dumps(m) + "\n"); sys.stdout.flush()
for line in sys.stdin:
    m = json.loads(line)
    if m["event"] == "hello":
        send({"event": "ready"})
        send({"event": "value", "id": "server", "value": m["settings"].get("url", "none")})
    elif m["event"] == "settings":
        send({"event": "value", "id": "server", "value": m["settings"].get("url", "none")})
    elif m["event"] == "watch":
        for v in m["values"]:
            send({"event": "value", "id": v, "value": "up"})
        send({"event": "value", "id": "server:not-an-arg-value", "value": "ignored"})
    elif m["event"] == "shutdown":
        break
"#;

#[tokio::test(flavor = "multi_thread")]
async fn plugin_settings_and_watched_values() {
    if !has_python() {
        eprintln!("skipping: no Python");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let plugins = root.path().join("plugins");
    let dir = plugins.join("test.watch");
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = json!({
        "id": "test.watch", "name": "Watch", "version": "1.0.0", "runtime": "python", "main": "plugin.py",
        "settings": [{ "id": "url", "label": "Server" }, { "id": "token", "label": "Token", "type": "password" }],
        "values": [{ "id": "server", "name": "Server" }, { "id": "up", "name": "Up", "arg": "Host" }]
    });
    std::fs::write(dir.join(MANIFEST), manifest.to_string()).unwrap();
    std::fs::write(dir.join("plugin.py"), WATCH_PLUGIN).unwrap();
    let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let sink = seen.clone();
    let host = PluginHost::start(
        plugins.clone(),
        "1",
        Arc::new(move |k, v| sink.lock().unwrap().push((k, v))),
    );
    let has = |k: &str, v: &str| seen.lock().unwrap().iter().any(|(a, b)| a == k && b == v);
    wait_for("first value", || has("plugin:test.watch:server", "none")).await;

    // Settings: saved (undeclared ones dropped), sent, and given to the next start.
    host.set_settings(
        "test.watch",
        json!({ "url": "https://pve:8006", "junk": 1 }),
    )
    .unwrap();
    wait_for("settings", || {
        has("plugin:test.watch:server", "https://pve:8006")
    })
    .await;
    assert_eq!(
        host.settings("test.watch").unwrap(),
        json!({ "url": "https://pve:8006" })
    );
    assert!(root
        .path()
        .join("plugin-settings/test.watch.json")
        .is_file());
    assert!(host.set_settings("test.watch", json!([1])).is_err());

    // Values with an argument: only what profiles use, and only arg-taking values.
    host.watch(&[
        "plugin:test.watch:up:10.0.0.1".into(),
        "plugin:test.watch:up:10.0.0.2:22".into(),
        "plugin:test.watch:server".into(),
        "plugin:other.plugin:up:x".into(),
        "cpu".into(),
    ]);
    wait_for("watched", || {
        has("plugin:test.watch:up:10.0.0.1", "up") && has("plugin:test.watch:up:10.0.0.2:22", "up")
    })
    .await;
    assert!(!seen
        .lock()
        .unwrap()
        .iter()
        .any(|(k, _)| k.contains("not-an-arg-value")));

    // After a restart it gets its settings in hello and the watch list again.
    seen.lock().unwrap().clear();
    host.restart("test.watch").unwrap();
    wait_for("hello settings", || {
        has("plugin:test.watch:server", "https://pve:8006")
    })
    .await;
    wait_for("watch again", || has("plugin:test.watch:up:10.0.0.1", "up")).await;
    host.shutdown();
}

const STUBBORN_PLUGIN: &str = r#"
import json, os, sys, time
open(os.path.join(os.environ["PERRITODECK_PLUGIN_DATA"], "pid"), "w").write(str(os.getpid()))
for line in sys.stdin:
    m = json.loads(line)
    if m["event"] == "hello":
        sys.stdout.write(json.dumps({"event": "ready"}) + "\n"); sys.stdout.flush()
    # ignores shutdown on purpose
"#;

/// Whether process `pid` is still running.
fn process_alive(pid: &str) -> bool {
    if cfg!(windows) {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
    } else {
        std::process::Command::new("kill")
            .args(["-0", pid])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn plugins_that_ignore_shutdown_are_ended_before_removal() {
    if !has_python() {
        eprintln!("skipping: no Python");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let plugins = root.path().join("plugins");
    write_plugin(
        &plugins,
        "test.stubborn",
        "python",
        "plugin.py",
        Some(STUBBORN_PLUGIN),
    );
    let host = PluginHost::start(plugins.clone(), "1", Arc::new(|_, _| {}));
    wait_for("running", || {
        host.list()
            .first()
            .is_some_and(|p| p.status == Status::Running)
    })
    .await;
    let pid_file = root.path().join("plugin-data/test.stubborn/pid");
    wait_for("pid", || pid_file.is_file()).await;
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    let pid = pid.trim();
    assert!(process_alive(pid), "the plugin is running");
    let h = host.clone();
    tokio::task::spawn_blocking(move || h.remove("test.stubborn"))
        .await
        .unwrap()
        .unwrap();
    assert!(!plugins.join("test.stubborn").exists());
    assert!(!process_alive(pid), "the old process was ended");
}

#[tokio::test(flavor = "multi_thread")]
async fn scans_while_a_plugin_is_being_removed_dont_bring_it_back() {
    if !has_python() {
        eprintln!("skipping: no Python");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let plugins = root.path().join("plugins");
    write_plugin(
        &plugins,
        "test.stubborn",
        "python",
        "plugin.py",
        Some(STUBBORN_PLUGIN),
    );
    let host = PluginHost::start(plugins.clone(), "1", Arc::new(|_, _| {}));
    wait_for("running", || {
        host.list()
            .first()
            .is_some_and(|p| p.status == Status::Running)
    })
    .await;
    // It ignores `shutdown`, so removing it takes the whole grace period, while other
    // commands (installing or restarting another plugin) scan the folder.
    let h = host.clone();
    let removing = tokio::task::spawn_blocking(move || h.remove("test.stubborn"));
    while !removing.is_finished() {
        host.scan();
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    removing.await.unwrap().unwrap();
    assert!(!plugins.join("test.stubborn").exists());
    assert!(host.list().is_empty(), "{:?}", host.list());
}

/// Answers nothing, not even `ready`, but exits on `shutdown`.
const NEVER_READY_PLUGIN: &str = r#"
import json, os, sys
open(os.path.join(os.environ["PERRITODECK_PLUGIN_DATA"], "pid"), "w").write(str(os.getpid()))
for line in sys.stdin:
    if json.loads(line)["event"] == "shutdown":
        break
"#;

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_that_isnt_ready_yet_is_asked_to_stop_too() {
    if !has_python() {
        eprintln!("skipping: no Python");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let plugins = root.path().join("plugins");
    write_plugin(
        &plugins,
        "test.slowstart",
        "python",
        "plugin.py",
        Some(NEVER_READY_PLUGIN),
    );
    let host = PluginHost::start(plugins.clone(), "1", Arc::new(|_, _| {}));
    let pid_file = root.path().join("plugin-data/test.slowstart/pid");
    wait_for("pid", || pid_file.is_file()).await;
    assert_eq!(host.list()[0].status, Status::Starting);
    let began = Instant::now();
    let h = host.clone();
    tokio::task::spawn_blocking(move || h.remove("test.slowstart"))
        .await
        .unwrap()
        .unwrap();
    // It got `shutdown` and left at once, instead of being killed after the grace.
    assert!(began.elapsed() < STOP_GRACE, "{:?}", began.elapsed());
    assert!(!plugins.join("test.slowstart").exists());
}

/// Starts a program of its own as soon as it runs, like the soundboard's players.
/// Ending the plugin's process doesn't end its children on Windows; only the host's
/// job object does.
#[cfg(windows)]
const PARENT_PLUGIN: &str = r#"
import json, os, subprocess, sys
data = os.environ["PERRITODECK_PLUGIN_DATA"]
child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(120)"])
open(os.path.join(data, "child"), "w").write(str(child.pid))
open(os.path.join(data, "pid"), "w").write(str(os.getpid()))
for line in sys.stdin:
    m = json.loads(line)
    if m["event"] == "hello":
        sys.stdout.write(json.dumps({"event": "ready"}) + "\n"); sys.stdout.flush()
    elif m["event"] == "shutdown":
        break
"#;

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn programs_a_plugin_started_end_with_it() {
    if !has_python() {
        eprintln!("skipping: no Python");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let plugins = root.path().join("plugins");
    write_plugin(
        &plugins,
        "test.parent",
        "python",
        "plugin.py",
        Some(PARENT_PLUGIN),
    );
    let host = PluginHost::start(plugins.clone(), "1", Arc::new(|_, _| {}));
    let data = root.path().join("plugin-data/test.parent");
    wait_for("pids", || data.join("pid").is_file()).await;
    let read = |f: &str| std::fs::read_to_string(data.join(f)).unwrap();
    let (pid, child) = (read("pid"), read("child"));
    assert!(process_alive(&child), "the plugin's own program is running");
    let h = host.clone();
    tokio::task::spawn_blocking(move || h.remove("test.parent"))
        .await
        .unwrap()
        .unwrap();
    assert!(!process_alive(&pid), "the plugin was ended");
    // Removing waits until the job's processes are gone, not only the plugin.
    assert!(!process_alive(&child), "the plugin's own program was ended");
}

/// Starts three programs: a plain child, one through the SDK's start_for_user and one
/// (a windowless script) through open_for_user, as a Focus key opens an app or a
/// sign-in opens a browser. Only the plain child should end with the plugin.
#[cfg(windows)]
const OPENER_PLUGIN: &str = r#"
import json, os, subprocess, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import open_for_user, start_for_user
data = os.environ["PERRITODECK_PLUGIN_DATA"]
def note(name, value):
    open(os.path.join(data, name + ".tmp"), "w").write(str(value))
    os.replace(os.path.join(data, name + ".tmp"), os.path.join(data, name))
pythonw = os.path.join(os.path.dirname(sys.executable), "pythonw.exe")
sleep = "import time; time.sleep(120)"
note("child", subprocess.Popen([pythonw, "-c", sleep]).pid)
note("started", start_for_user([pythonw, "-c", sleep]).pid)
def shell_opens(extension):
    # Whether ShellExecute opens this type with a program of its own, rather than
    # asking which to use. GitHub's runners register .pyw but send it to OpenWith.exe.
    import ctypes
    from ctypes import wintypes
    size = wintypes.DWORD(1024)
    found = ctypes.create_unicode_buffer(size.value)
    ok = ctypes.WinDLL("shlwapi").AssocQueryStringW(0, 2, extension, "open", found, ctypes.byref(size)) == 0
    return ok and os.path.basename(found.value).lower() != "openwith.exe"
script = os.path.join(data, "opened.pyw")
open(script, "w").write(
    "import os, time\n"
    "open(%r, 'w').write(str(os.getpid()))\n"
    "os.replace(%r, %r)\n"
    "time.sleep(120)\n" % ((os.path.join(data, "opened.tmp"),) * 2 + (os.path.join(data, "opened"),)))
if shell_opens(".pyw"):
    open_for_user(script)
else:
    # A batch file, which Windows always runs itself, runs the script instead (a
    # console shows meanwhile). cmd reads it in the OEM code page.
    try:
        batch_text = ('@"%s" "%s"\r\n' % (pythonw, script)).encode("oem")
    except UnicodeEncodeError:
        batch_text = None
    if batch_text is None:
        note("opened", "none")  # its path doesn't fit the console's code page
    else:
        batch = os.path.join(data, "opened.cmd")
        open(batch, "wb").write(batch_text)
        open_for_user(batch)
note("pid", os.getpid())
for line in sys.stdin:
    m = json.loads(line)
    if m["event"] == "hello":
        sys.stdout.write(json.dumps({"event": "ready"}) + "\n"); sys.stdout.flush()
    elif m["event"] == "shutdown":
        break
"#;

/// Kills the processes it names when dropped, so a failing test leaves none behind.
#[cfg(windows)]
struct KillOnDrop(Vec<String>);

#[cfg(windows)]
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        for pid in &self.0 {
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/PID", pid])
                .output();
        }
    }
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn programs_opened_for_the_user_outlive_the_plugin() {
    if !has_python() {
        eprintln!("skipping: no Python");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let plugins = root.path().join("plugins");
    let dir = write_plugin(
        &plugins,
        "test.opener",
        "python",
        "plugin.py",
        Some(OPENER_PLUGIN),
    );
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../plugins/sdk-python/perritodeck_plugin.py");
    std::fs::copy(sdk, dir.join("perritodeck_plugin.py")).unwrap();
    let host = PluginHost::start(plugins.clone(), "1", Arc::new(|_, _| {}));
    let data = root.path().join("plugin-data/test.opener");
    wait_for("pids", || {
        data.join("pid").is_file() && data.join("opened").is_file()
    })
    .await;
    let read = |f: &str| std::fs::read_to_string(data.join(f)).unwrap();
    let (child, started, opened) = (read("child"), read("started"), read("opened"));
    let mut survivors = KillOnDrop(vec![started.clone()]);
    if opened != "none" {
        survivors.0.push(opened.clone());
    }
    assert!(process_alive(&child) && process_alive(&started));
    let h = host.clone();
    tokio::task::spawn_blocking(move || h.remove("test.opener"))
        .await
        .unwrap()
        .unwrap();
    assert!(
        !process_alive(&child),
        "a plain child ended with the plugin"
    );
    assert!(
        process_alive(&started),
        "start_for_user's program outlived it"
    );
    if opened == "none" {
        eprintln!("skipped open_for_user: the data folder doesn't fit the console's code page");
    } else {
        assert!(
            process_alive(&opened),
            "open_for_user's program outlived it"
        );
    }
}
