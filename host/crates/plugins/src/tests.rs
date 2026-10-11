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
    candidates.iter().any(|p| {
        std::process::Command::new(p)
            .arg("--version")
            .output()
            .is_ok()
    })
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
