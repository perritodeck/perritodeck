use super::*;

#[tokio::test]
async fn starts_and_answers_ui_questions() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = HostOptions::new(dir.path().to_owned());
    opts.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
    opts.port = 0;
    opts.mdns = false;
    opts.watch_focus = false;
    opts.usb = false;
    let host = Host::start(opts).await.unwrap();

    let status = host.status();
    assert!(status.fingerprint.is_some());
    assert_eq!(
        status.addresses,
        vec![format!("127.0.0.1:{}", host.addr.port())]
    );
    assert!(
        Path::new(&status.profile_path).exists(),
        "starter profile written"
    );

    let offer = host.new_pairing_offer().unwrap();
    assert_eq!(offer.code.len(), 6);
    assert!(offer.uri.starts_with("perritodeck://pair?"));
    assert!(offer.qr_svg.contains("<svg"));

    assert!(host.devices().is_empty());
    assert!(!host.revoke("nobody"));

    let view = host.profile_view(None).unwrap();
    assert!(!view.pages.is_empty());
    assert!(view.pages[0].keys.iter().any(|k| k.kind == "page"));
    assert!(view.reload_error.is_none());

    // A broken edit is reported to subscribers and in the view.
    let mut events = host.subscribe();
    std::fs::write(&status.profile_path, "this is not toml [").unwrap();
    let got = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(HostEvent::ProfileChanged) = events.recv().await {
                break;
            }
        }
    })
    .await;
    assert!(got.is_ok(), "no ProfileChanged event");
    assert!(host.profile_view(None).unwrap().reload_error.is_some());
}

#[tokio::test]
async fn edits_save_reload_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = HostOptions::new(dir.path().to_owned());
    opts.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
    opts.port = 0;
    opts.mdns = false;
    opts.watch_focus = false;
    opts.usb = false;
    let host = Host::start(opts).await.unwrap();
    let original = std::fs::read_to_string(&host.profile_path).unwrap();
    let title = |v: &ProfileView| {
        v.pages[0]
            .keys
            .iter()
            .find(|k| k.row == 1 && k.col == 1)
            .map(|k| k.title.clone())
    };

    let edit = Edit::SetKey {
        page: "Main".into(),
        at: [1, 1],
        key: KeyFields {
            title: Some("Copied".into()),
            ..Default::default()
        },
    };
    let view = host
        .edit_profile("default", std::slice::from_ref(&edit))
        .unwrap();
    assert_eq!(title(&view).as_deref(), Some("Copied"));
    assert!(view.can_undo && !view.can_redo);
    // Saved to disk and already live in the deck (no waiting for the watcher).
    assert!(std::fs::read_to_string(&host.profile_path)
        .unwrap()
        .contains("\"Copied\""));
    assert_eq!(
        title(&view_of("default", &host.deck.profile(), None)).as_deref(),
        Some("Copied")
    );

    let view = host.undo().unwrap();
    assert_eq!(title(&view).as_deref(), Some("Copy"));
    assert_eq!(
        std::fs::read_to_string(&host.profile_path).unwrap(),
        original
    );
    assert!(!view.can_undo && view.can_redo);
    let view = host.redo().unwrap();
    assert_eq!(title(&view).as_deref(), Some("Copied"));

    // A bad edit changes nothing.
    let bad = Edit::SetKey {
        page: "Main".into(),
        at: [0, 0],
        key: KeyFields {
            bg: Some("red".into()),
            ..Default::default()
        },
    };
    assert!(host.edit_profile("default", &[bad]).is_err());
    assert_eq!(
        title(&host.profile_view(None).unwrap()).as_deref(),
        Some("Copied")
    );

    // Actions can be tried from the editor.
    let ok = perritodeck_actions::ActionSpec::Command {
        program: "true".into(),
        args: vec![],
        cwd: None,
        timeout_ms: None,
    };
    if cfg!(unix) {
        assert_eq!(host.test_action(&ok).await, Ok(None));
        let fails = perritodeck_actions::ActionSpec::Command {
            program: "false".into(),
            args: vec![],
            cwd: None,
            timeout_ms: None,
        };
        assert!(host.test_action(&fails).await.unwrap().is_some());
    }
    let bad = perritodeck_actions::ActionSpec::OpenUrl {
        url: "file:///etc/passwd".into(),
    };
    assert!(host.test_action(&bad).await.is_err());

    // After a hand edit, undo refuses rather than overwrite it.
    std::fs::write(&host.profile_path, &original).unwrap();
    assert!(host.undo().unwrap_err().contains("outside"));
    assert!(!host.profile_view(None).unwrap().can_undo);
}

#[tokio::test]
async fn insecure_needs_loopback() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = HostOptions::new(dir.path().to_owned());
    opts.insecure = true;
    assert!(Host::start(opts).await.is_err());
}

#[test]
fn events_serialize_for_the_ui() {
    let e = HostEvent::KeyPressed {
        device_id: "d".into(),
        key_index: 3,
    };
    assert_eq!(
        serde_json::to_string(&e).unwrap(),
        r#"{"kind":"keyPressed","deviceId":"d","keyIndex":3}"#
    );
    let e = HostEvent::KeyLooksChanged {
        profile: "default".into(),
    };
    assert_eq!(
        serde_json::to_string(&e).unwrap(),
        r#"{"kind":"keyLooksChanged","profile":"default"}"#
    );
}

/// Stream Deck plugins that do nothing, so a test draws their keys itself.
struct QuietAgents;

impl perritodeck_deck::KeyAgents for QuietAgents {
    fn device(&self, _: &str, _: Option<perritodeck_protocol::Grid>) {}
    fn appear(&self, _: &perritodeck_deck::AgentKey) {}
    fn disappear(&self, _: &perritodeck_deck::AgentKey) {}
    fn press(&self, _: &perritodeck_deck::AgentKey, _: bool) {}
}

#[tokio::test]
async fn keys_a_plugin_draws_show_in_the_editor() {
    let dir = tempfile::tempdir().unwrap();
    let host = quiet_host(dir.path()).await;
    host.deck.set_agents(Arc::new(QuietAgents));
    let counter = Edit::SetKey {
        page: "Main".into(),
        at: [2, 2],
        key: KeyFields {
            title: Some("Count".into()),
            action: Some(perritodeck_actions::ActionSpec::SdPlugin {
                plugin: "com.example.counter".into(),
                action: "com.example.counter.up".into(),
                settings: serde_json::Value::Null,
            }),
            ..Default::default()
        },
    };
    host.edit_profile("default", &[counter]).unwrap();
    let live = |v: &ProfileView| {
        let k = v.pages[0]
            .keys
            .iter()
            .find(|k| k.row == 2 && k.col == 2)
            .expect("the counter key");
        (k.title.clone(), k.live_title.clone(), k.live_image.clone())
    };
    // No tablet shows it, so the plugin draws nothing: the profile's own look.
    assert_eq!(
        live(&host.profile_view(None).unwrap()),
        ("Count".into(), None, None)
    );

    let mut events = host.subscribe();
    let screen = perritodeck_protocol::Screen {
        width_px: 1920,
        height_px: 1200,
        density_dpi: 224,
    };
    let grid = perritodeck_protocol::Grid { cols: 5, rows: 3 };
    host.deck.connected("tab", grid, screen, &["keys".into()]);
    let png = perritodeck_render::Pixmap::new(4, 4)
        .unwrap()
        .encode_png()
        .unwrap();
    host.deck.agent_draw(
        "default/0/2/2",
        Some(Some("3".into())),
        Some(Some(png.clone())),
    );
    assert_eq!(
        live(&host.profile_view(None).unwrap()),
        (
            "Count".into(),
            Some("3".into()),
            Some(perritodeck_render::data_url(&png))
        ),
        "the drawing is shown; the key's own title stays for editing"
    );
    let told = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(HostEvent::KeyLooksChanged { profile }) = events.recv().await {
                break profile;
            }
        }
    })
    .await;
    assert_eq!(told.as_deref(), Ok("default"), "no KeyLooksChanged event");
}

async fn quiet_host(dir: &std::path::Path) -> Host {
    let mut opts = HostOptions::new(dir.to_owned());
    opts.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
    opts.port = 0;
    opts.mdns = false;
    opts.watch_focus = false;
    opts.usb = false;
    Host::start(opts).await.unwrap()
}

#[tokio::test]
async fn profiles_create_copy_rename_delete() {
    let dir = tempfile::tempdir().unwrap();
    let host = quiet_host(dir.path()).await;
    assert_eq!(host.profiles().len(), 1);

    let blank = host.create_profile("Streaming (OBS)", None).unwrap();
    assert_eq!(blank, "streaming-obs");
    let copy = host
        .create_profile("Streaming (OBS)", Some("default"))
        .unwrap();
    assert_eq!(copy, "streaming-obs-2", "ids don't clash");
    let names: Vec<_> = host
        .profiles()
        .into_iter()
        .map(|p| (p.id, p.name, p.is_default))
        .collect();
    assert!(names[0].2, "default first");
    assert_eq!(names.len(), 3);

    // The copy has the default's keys; the blank one has an empty page.
    let v = host.profile_view(Some(&copy)).unwrap();
    assert!(v.pages[0].keys.iter().any(|k| k.title == "Copy"));
    let v = host.profile_view(Some(&blank)).unwrap();
    assert_eq!(v.pages.len(), 1);
    assert!(v.pages[0].keys.is_empty());

    // Edits to one profile; rename via edit; undo works across profiles.
    host.edit_profile(
        &blank,
        &[Edit::SetName {
            name: "Stream".into(),
        }],
    )
    .unwrap();
    assert_eq!(host.profile_view(Some(&blank)).unwrap().name, "Stream");
    host.edit_profile(
        "default",
        &[Edit::AddPage {
            name: "Extra".into(),
        }],
    )
    .unwrap();
    assert_eq!(host.undo().unwrap().id, "default");
    assert_eq!(host.undo().unwrap().id, blank);
    assert_eq!(
        host.profile_view(Some(&blank)).unwrap().name,
        "Streaming (OBS)"
    );

    // Delete keeps a copy; the default can't go.
    assert!(host.delete_profile("default").is_err());
    host.delete_profile(&copy).unwrap();
    assert_eq!(host.profiles().len(), 2);
    assert!(
        std::fs::read_dir(dir.path().join("deleted-profiles"))
            .unwrap()
            .count()
            == 1
    );
    assert!(host.profile_view(Some(&copy)).is_err());
}

#[tokio::test]
async fn tablets_get_assigned_profiles_and_auto_switch() {
    let dir = tempfile::tempdir().unwrap();
    let host = quiet_host(dir.path()).await;
    let stream = host.create_profile("Stream", None).unwrap();

    // Assigning is remembered across restarts (in app-settings.json).
    host.assign_profile("tab", Some(&stream)).unwrap();
    assert!(host.assign_profile("tab", Some("nope")).is_err());
    let saved = AppSettings::load(&settings_path(dir.path()));
    assert_eq!(saved.device_profiles.get("tab"), Some(&stream));
    assert_eq!(host.deck.device_profile("tab"), stream);

    host.set_auto_switch(AutoSwitch {
        enabled: true,
        rules: vec![SwitchRule {
            app: "obs64".into(),
            profile: stream.clone(),
            device: None,
        }],
    })
    .unwrap();
    assert_eq!(host.auto_switch().rules.len(), 1);
    // Deleting the profile drops its rules and assignments.
    host.delete_profile(&stream).unwrap();
    assert!(host.auto_switch().rules.is_empty());
    assert_eq!(host.deck.device_profile("tab"), "default");

    host.set_paused(true);
    assert!(host.status().paused);
    host.set_paused(false);
}

#[tokio::test]
async fn a_switch_profile_key_is_a_manual_choice() {
    use perritodeck_profile::NavSpec;
    use perritodeck_protocol::{Grid, Screen};
    let dir = tempfile::tempdir().unwrap();
    let host = quiet_host(dir.path()).await;
    let stream = host.create_profile("Stream", None).unwrap();
    let switch_to = |profile: &str| Edit::SetKey {
        page: "Main".into(),
        at: [2, 2],
        key: KeyFields {
            nav: Some(NavSpec::Profile {
                profile: profile.into(),
                page: None,
            }),
            ..Default::default()
        },
    };
    host.edit_profile("default", &[switch_to(&stream)]).unwrap();
    let screen = Screen {
        width_px: 1920,
        height_px: 1200,
        density_dpi: 224,
    };
    host.deck
        .connected("tab", Grid { cols: 5, rows: 3 }, screen, &["keys".into()]);
    let press = || host.deck.key_down("tab", 2 * 5 + 2);

    // An auto-switch rule shows the default while "editor" is in front.
    host.set_auto_switch(AutoSwitch {
        enabled: true,
        rules: vec![SwitchRule {
            app: "editor".into(),
            profile: "default".into(),
            device: None,
        }],
    })
    .unwrap();
    *host.shared.foreground.lock().unwrap() = Some("editor".into());
    host.shared.apply_auto_switch();
    assert_eq!(host.deck.device_profile("tab"), "default");

    // The key switches, and the rule doesn't take it straight back.
    press();
    assert_eq!(host.deck.device_profile("tab"), stream);
    host.shared.apply_auto_switch();
    assert_eq!(host.deck.device_profile("tab"), stream);
    // Remembered like choosing it in the app.
    let saved = AppSettings::load(&settings_path(dir.path()));
    assert_eq!(saved.device_profiles.get("tab"), Some(&stream));

    // Once another app comes to the front and back, the rule applies again.
    host.shared.key_switched.lock().unwrap().clear();
    host.shared.apply_auto_switch();
    assert_eq!(host.deck.device_profile("tab"), "default");

    // Without a matching rule the tablet keeps its chosen profile; a key back to the
    // default forgets the choice.
    *host.shared.foreground.lock().unwrap() = Some("browser".into());
    host.shared.apply_auto_switch();
    assert_eq!(host.deck.device_profile("tab"), stream);
    host.edit_profile(&stream, &[switch_to("default")]).unwrap();
    press();
    assert_eq!(host.deck.device_profile("tab"), "default");
    let saved = AppSettings::load(&settings_path(dir.path()));
    assert_eq!(saved.device_profiles.get("tab"), None);
}

#[test]
fn slugs_and_unique_ids() {
    assert_eq!(slug("Streaming (OBS)"), "streaming-obs");
    assert_eq!(slug("  ¡Hola!  "), "hola");
    assert_eq!(slug("***"), "profile");
    assert_eq!(unique_id("a", |id| id == "a" || id == "a-2"), "a-3");
}

#[tokio::test]
async fn icons_are_imported_and_shown_in_the_preview() {
    let dir = tempfile::tempdir().unwrap();
    let host = quiet_host(dir.path()).await;
    let src = dir.path().join("big.png");
    image::RgbaImage::from_pixel(600, 300, image::Rgba([255, 0, 0, 255]))
        .save(&src)
        .unwrap();
    let rel = host.import_icon(&src).unwrap();
    assert!(rel.starts_with("icons/") && rel.ends_with(".png"));
    let saved = image::open(host.profiles_dir.join(&rel)).unwrap();
    assert_eq!((saved.width(), saved.height()), (288, 144), "scaled to fit");
    assert!(host
        .import_icon(&dir.path().join("app-settings.json"))
        .is_err());

    let edit = Edit::SetKey {
        page: "Main".into(),
        at: [2, 2],
        key: KeyFields {
            title: Some("Red".into()),
            icon: Some(rel.clone().into()),
            ..Default::default()
        },
    };
    let view = host.edit_profile("default", &[edit]).unwrap();
    let key = view.pages[0]
        .keys
        .iter()
        .find(|k| k.row == 2 && k.col == 2)
        .unwrap();
    assert!(key
        .icon_data
        .as_deref()
        .unwrap()
        .starts_with("data:image/png;base64,"));
    assert!(std::fs::read_to_string(&host.profile_path)
        .unwrap()
        .contains(&format!("icon = \"{rel}\"")));
}

#[tokio::test]
async fn profiles_export_and_import_with_their_images() {
    let dir = tempfile::tempdir().unwrap();
    let host = quiet_host(dir.path()).await;
    let src = dir.path().join("icon.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 255, 255]))
        .save(&src)
        .unwrap();
    let rel = host.import_icon(&src).unwrap();
    let edit = Edit::SetKey {
        page: "Main".into(),
        at: [2, 2],
        key: KeyFields {
            title: Some("Blue".into()),
            icon: Some(rel.clone().into()),
            ..Default::default()
        },
    };
    host.edit_profile("default", &[edit]).unwrap();

    let file = dir.path().join("starter.perritoprofile");
    host.export_profile("default", &file).unwrap();

    // Into another host, which has never seen the image.
    let dir2 = tempfile::tempdir().unwrap();
    let other = quiet_host(dir2.path()).await;
    let id = other.import_profile(&file).unwrap();
    assert_eq!(id, "starter");
    let view = other.profile_view(Some(&id)).unwrap();
    let key = view.pages[0]
        .keys
        .iter()
        .find(|k| k.title == "Blue")
        .unwrap();
    assert!(key.icon_data.is_some(), "image came along");
    assert!(other.profiles_dir.join(&rel).exists());
    // Importing twice makes a second profile rather than overwriting.
    assert_eq!(other.import_profile(&file).unwrap(), "starter-2");
    // Not a profile file.
    assert!(other.import_profile(&src).is_err());
}

#[tokio::test]
async fn imports_cannot_write_outside_the_profiles_folder() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let host = quiet_host(dir.path()).await;
    let file = dir.path().join("evil.perritoprofile");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&file).unwrap());
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file("profile.toml", opts).unwrap();
    zip.write_all(perritodeck_profile::STARTER_PROFILE.as_bytes())
        .unwrap();
    zip.start_file("images/../../escaped.png", opts).unwrap();
    zip.write_all(b"x").unwrap();
    zip.finish().unwrap();
    host.import_profile(&file).unwrap();
    assert!(!dir.path().join("escaped.png").exists());
    assert!(!host.profiles_dir.join("../escaped.png").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_actions_run_from_keys_and_values_reach_titles() {
    let has_python = ["python3", "python", "py"].iter().any(|p| {
        std::process::Command::new(p)
            .arg("--version")
            .output()
            .is_ok()
    });
    if !has_python {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    std::fs::write(
        src.path().join("perritodeck-plugin.json"),
        r#"{ "id": "test.counter", "name": "Counter", "version": "1", "runtime": "python", "main": "p.py",
             "actions": [{ "id": "bump", "name": "Bump" }], "values": [{ "id": "count", "name": "Count" }] }"#,
    )
    .unwrap();
    std::fs::write(
        src.path().join("p.py"),
        "import json,sys\nn=0\ndef send(m):\n    print(json.dumps(m), flush=True)\nfor l in sys.stdin:\n    m=json.loads(l)\n    if m['event']=='hello': send({'event':'ready'}); send({'event':'value','id':'count','value':str(n)})\n    elif m['event']=='run':\n        n+=1; send({'event':'value','id':'count','value':str(n)}); send({'event':'result','id':m['id'],'ok':True})\n",
    )
    .unwrap();
    let host = quiet_host(dir.path()).await;
    assert_eq!(host.install_plugin(src.path()).unwrap(), "test.counter");
    for _ in 0..200 {
        if host
            .plugins()
            .installed
            .first()
            .is_some_and(|p| p.status == PluginStatus::Running)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let bump = ActionSpec::Plugin {
        plugin: "test.counter".into(),
        action: "bump".into(),
        settings: serde_json::Value::Null,
    };
    assert_eq!(host.test_action(&bump).await, Ok(None));
    assert_eq!(host.test_action(&bump).await, Ok(None));

    // The plugin's value shows in a title.
    let edit = Edit::SetKey {
        page: "Main".into(),
        at: [2, 2],
        key: KeyFields {
            title: Some("n={plugin:test.counter:count}".into()),
            ..Default::default()
        },
    };
    host.edit_profile("default", &[edit]).unwrap();
    assert!(host
        .deck
        .live_tokens()
        .contains(&"plugin:test.counter:count".to_string()));
    for _ in 0..100 {
        if host.deck.live_value("plugin:test.counter:count").as_deref() == Some("2") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        host.deck.live_value("plugin:test.counter:count").as_deref(),
        Some("2")
    );
    host.remove_plugin("test.counter").unwrap();
    assert!(host.plugins().installed.is_empty());
}

#[tokio::test]
async fn first_party_plugins_install_from_the_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = HostOptions::new(dir.path().to_owned());
    opts.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
    opts.port = 0;
    opts.mdns = false;
    opts.watch_focus = false;
    opts.usb = false;
    opts.plugin_catalog =
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../plugins"));
    let host = Host::start(opts).await.unwrap();
    let ids: Vec<String> = host
        .plugins()
        .catalog
        .iter()
        .map(|c| c.manifest.id.clone())
        .collect();
    assert!(ids.contains(&"dev.perritodeck.http".to_string()), "{ids:?}");
    assert!(ids.contains(&"dev.perritodeck.mqtt".to_string()));
    assert_eq!(
        host.install_from_catalog("dev.perritodeck.http").unwrap(),
        "dev.perritodeck.http"
    );
    let view = host.plugins();
    assert!(view
        .catalog
        .iter()
        .any(|c| c.manifest.id == "dev.perritodeck.http" && c.installed));
    assert!(host.install_from_catalog("nope").is_err());
}

#[tokio::test]
async fn animated_gifs_are_kept_as_gifs() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = HostOptions::new(dir.path().to_owned());
    opts.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
    opts.port = 0;
    opts.mdns = false;
    opts.watch_focus = false;
    opts.usb = false;
    let host = Host::start(opts).await.unwrap();
    let gif = perritodeck_render::test_gif();
    let path = host.import_icon_bytes(&gif).unwrap();
    assert!(
        path.starts_with("icons/") && path.ends_with(".gif"),
        "{path}"
    );
    assert_eq!(std::fs::read(host.profiles_dir.join(&path)).unwrap(), gif);
    assert!(host.import_icon_bytes(b"nope").unwrap_err().contains("GIF"));
}

/// Serve fixed files over plain HTTP on 127.0.0.1, one request per connection.
fn serve(files: Vec<(&'static str, Vec<u8>)>) -> u16 {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap() == 0 || h == "\r\n" {
                    break;
                }
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let mut stream = stream;
            match files.iter().find(|(p, _)| *p == path) {
                Some((_, body)) => {
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .unwrap();
                    stream.write_all(body).unwrap();
                }
                None => {
                    write!(
                        stream,
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                }
            }
        }
    });
    port
}

#[tokio::test(flavor = "multi_thread")]
async fn installs_plugins_from_the_online_catalogue() {
    use sha2::{Digest, Sha256};
    let zip = crate::online::tests::plugin_zip("weather/", "dev.example.weather");
    let sha: String = Sha256::digest(&zip)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let index = format!(
        r#"{{"version":1,"plugins":[
            {{"id":"dev.example.weather","name":"Weather","version":"2.0.0","url":"w.zip","sha256":"{sha}"}},
            {{"id":"dev.example.bad","name":"Bad","version":"1.0.0","url":"w.zip","sha256":"00"}},
            {{"id":"dev.example.other","name":"Other","version":"1.0.0","url":"w.zip","sha256":"{sha}"}}
        ]}}"#
    );
    let port = serve(vec![
        ("/plugins/index.json", index.into_bytes()),
        ("/plugins/w.zip", zip),
    ]);

    let dir = tempfile::tempdir().unwrap();
    let mut opts = HostOptions::new(dir.path().to_owned());
    opts.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
    opts.port = 0;
    opts.mdns = false;
    opts.watch_focus = false;
    opts.usb = false;
    opts.plugin_index = format!("http://127.0.0.1:{port}/plugins/index.json");
    let host = Arc::new(Host::start(opts).await.unwrap());

    let h = host.clone();
    let listed = tokio::task::spawn_blocking(move || h.online_plugins())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(listed.len(), 3);
    assert!(listed.iter().all(|e| e.installed.is_none() && !e.update));

    let h = host.clone();
    let id = tokio::task::spawn_blocking(move || h.install_online_plugin("dev.example.weather"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(id, "dev.example.weather");
    assert!(host
        .plugins()
        .installed
        .iter()
        .any(|p| p.manifest.id == id && p.manifest.version == "2.0.0"));
    assert!(
        std::fs::read_dir(&host.data_dir)
            .unwrap()
            .flatten()
            .all(|e| !e
                .file_name()
                .to_string_lossy()
                .starts_with("plugin-download-")),
        "the download folder is cleaned up"
    );

    let h = host.clone();
    let bad = tokio::task::spawn_blocking(move || h.install_online_plugin("dev.example.bad"))
        .await
        .unwrap();
    assert!(bad.unwrap_err().contains("SHA-256"));
    let h = host.clone();
    let other = tokio::task::spawn_blocking(move || h.install_online_plugin("dev.example.other"))
        .await
        .unwrap();
    assert!(other.unwrap_err().contains("not dev.example.other"));
}

/// A tablet whose Wi-Fi attempts got no answer says so when it comes over the USB cable
/// (test tablets all arrive from 127.0.0.1, as a cable does), and the hint names it
/// until it comes back without that.
#[tokio::test(flavor = "multi_thread")]
async fn tablets_that_fell_back_to_the_cable_are_named() {
    use perritodeck_fakedeck::{fake_hello, Client, ClientSecurity, Endpoint};

    let dir = tempfile::tempdir().unwrap();
    let mut opts = HostOptions::new(dir.path().to_owned());
    opts.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
    opts.port = 0;
    opts.mdns = false;
    opts.watch_focus = false;
    opts.usb = false;
    opts.pair_codes = vec!["246810".into()];
    let host = Host::start(opts).await.unwrap();
    let addr = host.addr.to_string();
    let first = Endpoint {
        addr: addr.clone(),
        security: ClientSecurity::TrustOnFirstUse,
    };
    let paired = Client::pair(&first, "246810", "tab-1", "Fire 7")
        .await
        .unwrap();
    let ep = Endpoint {
        addr,
        security: ClientSecurity::Pinned(paired.host_id.clone()),
    };

    // Listening on this computer only, tablets can't use the network anyway.
    assert_eq!(host.network_hint(), None);
    // Nothing went wrong yet; it's up to what Windows Firewall would do.
    use perritodeck_core::discovery::Firewall;
    let unknown = || None;
    assert_eq!(host.network_hint_with(true, unknown), None);
    let fw = |lets_in| {
        move || {
            Some(Firewall {
                lets_in,
                public: true,
            })
        }
    };
    assert_eq!(host.network_hint_with(true, fw(true)), None);
    let blocked = host.network_hint_with(true, fw(false)).unwrap();
    assert!(blocked.firewall_blocks && blocked.public_network && blocked.fell_back.is_empty());

    let mut events = host.subscribe();
    let mut connected = async move || {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !matches!(events.recv().await, Ok(HostEvent::Connected { .. })) {}
        })
        .await
        .expect("a Connected event");
    };
    let mut hello = fake_hello("tab-1", Some(paired.device_secret.clone()), None);
    hello.network_failed = true;
    let (tablet, _, _) = Client::handshake(&ep, hello.clone()).await.unwrap();
    connected().await;
    let hint = host.network_hint_with(true, unknown).unwrap();
    assert_eq!(hint.fell_back, vec!["Fire 7".to_owned()]);
    assert!(!hint.firewall_blocks);
    tablet.close().await;

    // It got through over Wi-Fi meanwhile, so it comes back with nothing to report.
    hello.network_failed = false;
    let (tablet, _, _) = Client::handshake(&ep, hello).await.unwrap();
    connected().await;
    assert_eq!(host.network_hint_with(true, unknown), None);
    tablet.close().await;
}

/// A tablet showing a Companion profile appears in (a fake) Companion, shows what it
/// draws, and sends presses and releases back.
#[tokio::test(flavor = "multi_thread")]
async fn tablets_become_companion_surfaces() {
    use perritodeck_fakedeck::{fake_hello, Client, ClientSecurity, Endpoint};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let companion = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = companion.local_addr().unwrap().port();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("integrations.toml"),
        format!("[companion]\nhost = \"127.0.0.1\"\nport = {port}\n"),
    )
    .unwrap();
    let profile = dir.path().join("profiles").join("default.toml");
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(
        &profile,
        "schema_version = 1\nname = \"Companion\"\ncompanion = true\n",
    )
    .unwrap();
    let mut opts = HostOptions::new(dir.path().to_owned());
    opts.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
    opts.port = 0;
    opts.mdns = false;
    opts.watch_focus = false;
    opts.usb = false;
    opts.insecure = true;
    opts.profile = Some(profile);
    let host = Host::start(opts).await.unwrap();
    assert!(host
        .status()
        .integrations
        .contains(&"Companion".to_string()));

    let (sock, _) = companion.accept().await.unwrap();
    let (r, mut w) = sock.into_split();
    let mut lines = BufReader::new(r).lines();
    w.write_all(b"BEGIN CompanionVersion=4.1.0 ApiVersion=1.12.0\nCAPS BITMAP_FORMATS=rgb,png\n")
        .await
        .unwrap();
    let mut hear = async move || loop {
        let l = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if !l.starts_with("PING") {
            return perritodeck_companion::line::parse(&l);
        }
    };

    let ep = Endpoint {
        addr: host.addr.to_string(),
        security: ClientSecurity::Insecure,
    };
    let mut hello = fake_hello(
        "tab-1",
        None,
        Some(perritodeck_protocol::Grid { cols: 4, rows: 2 }),
    );
    hello.capabilities.push("images".into());
    let (mut tablet, _, layout) = Client::handshake(&ep, hello).await.unwrap();
    assert_eq!(layout.keys.len(), 8);
    assert!(
        layout.keys.iter().all(|k| k.title.is_empty()),
        "blank until Companion draws"
    );

    let add = hear().await;
    assert_eq!(add.command, "ADD-DEVICE");
    assert_eq!(add.get("DEVICEID"), Some("perritodeck-tab-1"));
    assert_eq!(add.get("KEYS_TOTAL"), Some("8"));
    assert_eq!(add.get("BITMAP_FORMAT"), Some("png"));
    w.write_all(b"ADD-DEVICE OK DEVICEID=perritodeck-tab-1\n")
        .await
        .unwrap();
    let text = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "Go live");
    w.write_all(
        format!("KEY-STATE DEVICEID=perritodeck-tab-1 KEY=2 COLOR=#00ff00 TEXTCOLOR=#000000 TEXT={text}\n")
            .as_bytes(),
    )
    .await
    .unwrap();

    // The tablet gets the page again with Companion's key, then its image.
    let mut got_layout = false;
    let mut got_image = false;
    for _ in 0..20 {
        let Ok(env) = tablet.recv_msg(Duration::from_secs(5)).await else {
            break;
        };
        match env.msg {
            perritodeck_protocol::Message::Layout(l) if l.keys[2].title == "Go live" => {
                assert_eq!(l.keys[2].bg_color, "#00ff00");
                got_layout = true;
            }
            perritodeck_protocol::Message::SetImage(i) if i.key_index == 2 => got_image = true,
            _ => {}
        }
        if got_layout && got_image {
            break;
        }
    }
    assert!(got_layout && got_image);

    tablet.key_down(2).await.unwrap();
    tablet.key_up(2).await.unwrap();
    let down = hear().await;
    assert_eq!(
        (down.command.as_str(), down.get("KEY"), down.get("PRESSED")),
        ("KEY-PRESS", Some("2"), Some("true"))
    );
    assert_eq!(hear().await.get("PRESSED"), Some("false"));

    tablet.close().await;
    let rm = hear().await;
    assert_eq!(rm.command, "REMOVE-DEVICE");
}

#[tokio::test]
async fn companion_profiles_can_be_made() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = HostOptions::new(dir.path().to_owned());
    opts.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
    opts.port = 0;
    opts.mdns = false;
    opts.watch_focus = false;
    opts.usb = false;
    let host = Host::start(opts).await.unwrap();
    let c = host.create_companion_profile("Show control").unwrap();
    let v = host.profile_view(Some(&c)).unwrap();
    assert!(v.companion && v.pages.is_empty());
    assert!(
        std::fs::read_to_string(host.profiles_dir.join(format!("{c}.toml")))
            .unwrap()
            .contains("[companion]")
    );
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Talks to the host's MCP endpoint through the stdio bridge, as an assistant would.
struct Assistant {
    write: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    lines: tokio::io::Lines<tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
    next_id: u64,
}

impl Assistant {
    fn connect(url: String, token: String) -> Self {
        let (client, bridge_side) = tokio::io::duplex(1 << 16);
        let (bin, bout) = tokio::io::split(bridge_side);
        tokio::spawn(async move { perritodeck_mcp::bridge(&url, &token, bin, bout).await });
        Self::over(client)
    }

    /// Through the bridge as `perritodeck-mcp` runs it (`agent` says which bridge): it
    /// finds the endpoint from the data folder.
    fn for_data_dir(dir: PathBuf, agent: &'static str) -> Self {
        let (client, bridge_side) = tokio::io::duplex(1 << 16);
        let (bin, bout) = tokio::io::split(bridge_side);
        tokio::spawn(async move {
            perritodeck_mcp::bridge_with(agent, || mcp::bridge_endpoint(&dir), bin, bout).await
        });
        Self::over(client)
    }

    fn over(client: tokio::io::DuplexStream) -> Self {
        use tokio::io::AsyncBufReadExt;
        let (read, write) = tokio::io::split(client);
        Self {
            write,
            lines: tokio::io::BufReader::new(read).lines(),
            next_id: 0,
        }
    }

    async fn send(&mut self, v: serde_json::Value) {
        use tokio::io::AsyncWriteExt;
        self.write
            .write_all(format!("{v}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await;
        let line = tokio::time::timeout(Duration::from_secs(10), self.lines.next_line())
            .await
            .expect("an answer in time")
            .unwrap()
            .expect("a line");
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["id"], id, "{v}");
        v
    }

    async fn tool(&mut self, name: &str, args: serde_json::Value) -> (bool, String) {
        let r = self
            .request(
                "tools/call",
                serde_json::json!({"name": name, "arguments": args}),
            )
            .await;
        let result = &r["result"];
        (
            result["isError"].as_bool().unwrap_or(true),
            result["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        )
    }
}

#[tokio::test]
async fn the_mcp_endpoint_follows_the_settings_and_answers_from_the_host() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(quiet_host(dir.path()).await);
    host.apply_mcp().await;
    assert_eq!(host.mcp_state(), McpState::Off, "off by default");

    let port = free_port();
    let mut m = host.mcp_view().settings;
    assert!(!m.enabled && !m.press && !m.edit && !m.run);
    m.port = host.addr.port();
    assert!(
        host.set_mcp(m.clone()).await.is_err(),
        "not the tablets' port"
    );
    m.enabled = true;
    m.port = port;
    let view = host.set_mcp(m.clone()).await.unwrap();
    assert_eq!(
        view.state,
        McpState::Running {
            url: format!("http://127.0.0.1:{port}/mcp")
        }
    );
    let token = mcp::read_token(dir.path()).expect("a token was made");
    assert!(view.snippets.http_config.contains(&token));
    assert_eq!(view.snippets.args[0], "mcp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(mcp::token_path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the token is owner-only");
    }

    let mut a = Assistant::connect(view.url.clone(), token.clone());
    let init = a
        .request(
            "initialize",
            serde_json::json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}),
        )
        .await;
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    a.send(serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;
    let tools = a.request("tools/list", serde_json::json!({})).await;
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"list_profiles"), "{names:?}");
    assert!(!names.contains(&"press_key"), "pressing is off: {names:?}");

    let (err, text) = a.tool("list_profiles", serde_json::json!({})).await;
    assert!(!err, "{text}");
    assert!(text.contains("default"), "{text}");
    let (err, text) = a
        .tool("get_profile", serde_json::json!({"profile": "default"}))
        .await;
    assert!(!err && text.contains("Main"), "{text}");
    let (err, text) = a
        .tool("press_key", serde_json::json!({"row": 0, "col": 0}))
        .await;
    assert!(err && text.contains("press"), "{text}");

    // Permissions apply to the next request, without a restart.
    m.press = true;
    m.edit = true;
    host.set_mcp(m.clone()).await.unwrap();
    let nav = host.profile_view(None).unwrap().pages[0]
        .keys
        .iter()
        .find(|k| k.kind == "page")
        .map(|k| [k.row, k.col])
        .expect("the starter profile has a page key");
    let (err, text) = a
        .tool(
            "press_key",
            serde_json::json!({"row": nav[0], "col": nav[1]}),
        )
        .await;
    assert!(err && text.contains("No connected tablet"), "{text}");

    // Edits go through the host, so they can be undone in the app.
    let (err, text) = a
        .tool(
            "set_key",
            serde_json::json!({"page": "Main", "row": 2, "col": 4, "title": "From AI"}),
        )
        .await;
    assert!(!err, "{text}");
    let titled = |h: &Host| {
        h.profile_view(None).unwrap().pages[0]
            .keys
            .iter()
            .any(|k| k.title == "From AI")
    };
    assert!(titled(&host));
    host.undo().unwrap();
    assert!(!titled(&host));

    // Running actions stays off.
    let (err, text) = a
        .tool(
            "run_action",
            serde_json::json!({"action": {"type": "text", "text": "x"}}),
        )
        .await;
    assert!(err, "{text}");

    // A new token restarts the endpoint; the old one stops working.
    let view = host.new_mcp_token().await.unwrap();
    let new_token = mcp::read_token(dir.path()).unwrap();
    assert_ne!(new_token, token);
    assert!(matches!(view.state, McpState::Running { .. }));
    let mut stale = Assistant::connect(view.url.clone(), token);
    let r = stale.request("tools/list", serde_json::json!({})).await;
    assert!(r.get("error").is_some(), "{r}");

    // Turning it off stops it.
    m.enabled = false;
    let view = host.set_mcp(m).await.unwrap();
    assert_eq!(view.state, McpState::Off);
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());

    // Diagnostics mention it, but never the token.
    let report = host.diagnostics(Some("9.9.9"));
    assert!(report.contains("AI assistants (MCP)"), "{report}");
    assert!(report.contains("9.9.9"));
    assert!(!report.contains(&new_token));
}

#[tokio::test]
async fn the_bridge_keeps_working_after_a_new_token_or_port() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(quiet_host(dir.path()).await);
    let mut m = host.mcp_view().settings;
    m.enabled = true;
    m.port = free_port();
    host.set_mcp(m.clone()).await.unwrap();
    assert_eq!(
        mcp::bridge_endpoint(dir.path()).url,
        format!("http://127.0.0.1:{}/mcp", m.port)
    );

    let mut a = Assistant::for_data_dir(dir.path().to_owned(), perritodeck_mcp::AGENT);
    let init = a
        .request(
            "initialize",
            serde_json::json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}),
        )
        .await;
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    a.send(serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;
    let (err, text) = a.tool("list_profiles", serde_json::json!({})).await;
    assert!(!err, "{text}");

    // "Make a new token" in Settings.
    host.new_mcp_token().await.unwrap();
    let (err, text) = a.tool("list_profiles", serde_json::json!({})).await;
    assert!(!err, "after a new token: {text}");

    // "Port for assistants" in Settings.
    m.port = free_port();
    host.set_mcp(m).await.unwrap();
    let (err, text) = a.tool("list_profiles", serde_json::json!({})).await;
    assert!(!err, "after a new port: {text}");
    assert!(!host.mcp_view().app_bridge_used);

    // An assistant still set up with the app's own program: Settings says so.
    let mut old = Assistant::for_data_dir(dir.path().to_owned(), "perritodeck-app/0.1.0");
    let init = old
        .request(
            "initialize",
            serde_json::json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}),
        )
        .await;
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(host.mcp_view().app_bridge_used);
}

#[test]
fn usb_needs_a_host_listening_where_adb_arrives() {
    assert!(crate::usb_reaches("0.0.0.0".parse().unwrap()));
    assert!(crate::usb_reaches("::".parse().unwrap()));
    assert!(crate::usb_reaches("127.0.0.1".parse().unwrap()));
    assert!(!crate::usb_reaches("192.168.1.60".parse().unwrap()));
}

#[test]
fn plugins_declare_their_values() {
    let manifest = perritodeck_plugins::Manifest::parse(
        &serde_json::json!({
            "id": "dev.example.mon", "name": "Mon", "version": "1.0.0", "runtime": "exec", "main": "x",
            "values": [{ "id": "disk", "name": "Disk" }, { "id": "up", "name": "Up", "arg": "Host" }]
        })
        .to_string(),
    )
    .unwrap();
    let info = perritodeck_plugins::PluginInfo {
        manifest,
        status: perritodeck_plugins::Status::Starting,
        folder: String::new(),
    };
    let declared = crate::declared_plugin_values(&[info]);
    assert_eq!(
        declared,
        HashSet::from([
            "plugin:dev.example.mon:disk".to_owned(),
            "plugin:dev.example.mon:up:".to_owned()
        ])
    );
}

#[test]
fn real_paths() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("app-settings.json");
    std::fs::write(&file, "{}").unwrap();
    // An ordinary file is where it says (no `\\?\` prefix), and isn't in a package.
    let real = crate::real_path(&file);
    assert!(
        !real.to_string_lossy().starts_with(r"\\?\"),
        "{}",
        real.display()
    );
    assert_eq!(
        std::fs::canonicalize(&real).unwrap(),
        std::fs::canonicalize(&file).unwrap()
    );
    assert!(!crate::redirected_into_package(&file));
    // Something that doesn't exist stays as given.
    let missing = dir.path().join("nope");
    assert_eq!(crate::real_path(&missing), missing);
}

#[tokio::test]
async fn start_at_login_is_remembered() {
    let dir = tempfile::tempdir().unwrap();
    let host = quiet_host(dir.path()).await;
    assert!(!host.start_at_login());
    host.set_start_at_login(true).unwrap();
    assert!(host.start_at_login());
    let saved = crate::AppSettings::load(&crate::settings_path(dir.path()));
    assert!(saved.start_at_login);
}

#[test]
fn the_bridge_finds_the_same_data_folder_as_the_host() {
    assert_eq!(
        perritodeck_mcp::cli::default_data_dir(),
        crate::default_data_dir().ok()
    );
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        dir.path().join(perritodeck_mcp::cli::SETTINGS_FILE),
        crate::settings_path(dir.path())
    );
    assert_eq!(
        dir.path().join(perritodeck_mcp::cli::TOKEN_FILE),
        mcp::token_path(dir.path())
    );
}

#[tokio::test]
async fn the_bridge_is_copied_to_the_data_folder() {
    let dir = tempfile::tempdir().unwrap();
    let shipped_dir = tempfile::tempdir().unwrap();
    let shipped = shipped_dir.path().join(mcp::BRIDGE_FILE);
    std::fs::write(&shipped, b"bridge 1").unwrap();
    let copy = mcp::bridge_path(dir.path());

    // Without a copy, assistants start this program's own bridge.
    let (command, args) = mcp::bridge_command(dir.path());
    assert_ne!(command, copy.display().to_string());
    assert_eq!(args[0], "mcp");

    let mut opts = HostOptions::new(dir.path().to_owned());
    opts.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
    opts.port = 0;
    opts.mdns = false;
    opts.watch_focus = false;
    opts.usb = false;
    opts.mcp_bridge = Some(shipped.clone());
    let host = Host::start(opts).await.unwrap();
    assert_eq!(std::fs::read(&copy).unwrap(), b"bridge 1");
    let view = host.mcp_view();
    assert_eq!(view.snippets.command, copy.display().to_string());
    // Not `mcp`: that's the app's own bridge. The data folder isn't the usual one here.
    assert_eq!(
        view.snippets.args,
        vec!["--data-dir".to_owned(), dir.path().display().to_string()]
    );
    assert!(view.snippets.claude_code_stdio.contains(mcp::BRIDGE_FILE));
    drop(host);

    assert_eq!(
        mcp::install_bridge(&shipped, dir.path()).unwrap(),
        mcp::BridgeInstall::Unchanged
    );
    std::fs::write(&shipped, b"bridge 2").unwrap();
    assert_eq!(
        mcp::install_bridge(&shipped, dir.path()).unwrap(),
        mcp::BridgeInstall::Copied
    );
    assert_eq!(std::fs::read(&copy).unwrap(), b"bridge 2");
    let left: Vec<_> = std::fs::read_dir(copy.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(left, vec![std::ffi::OsString::from(mcp::BRIDGE_FILE)]);
}

/// Windows won't overwrite a program that's running, which is the whole point of the
/// copy: the one an assistant runs is moved aside instead.
#[cfg(windows)]
#[test]
fn a_bridge_an_assistant_is_running_is_moved_aside() {
    let dir = tempfile::tempdir().unwrap();
    let shipped_dir = tempfile::tempdir().unwrap();
    let shipped = shipped_dir.path().join(mcp::BRIDGE_FILE);
    // Any program will do as the "bridge"; ping waits as long as it's told to.
    let ping = std::fs::read(r"C:\Windows\System32\PING.EXE").unwrap();
    std::fs::write(&shipped, &ping).unwrap();
    assert_eq!(
        mcp::install_bridge(&shipped, dir.path()).unwrap(),
        mcp::BridgeInstall::Copied
    );
    let copy = mcp::bridge_path(dir.path());
    let mut running = std::process::Command::new(&copy)
        .args(["-n", "60", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        std::fs::write(&copy, b"x").is_err(),
        "a running program can't be overwritten"
    );

    let mut newer = ping.clone();
    newer.extend_from_slice(b"newer");
    std::fs::write(&shipped, &newer).unwrap();
    let outcome = mcp::install_bridge(&shipped, dir.path());
    let bin = copy.parent().unwrap().to_owned();
    let names = || -> Vec<String> {
        let mut n: Vec<String> = std::fs::read_dir(&bin)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        n.sort();
        n
    };
    let after = names();
    running.kill().unwrap();
    running.wait().unwrap();
    assert_eq!(outcome.unwrap(), mcp::BridgeInstall::Copied);
    assert_eq!(std::fs::read(&copy).unwrap(), newer);
    assert_eq!(after.len(), 2, "{after:?}");
    assert!(after[1].ends_with(".old"), "{after:?}");

    // Once nothing runs the old one, the next start removes it.
    assert_eq!(
        mcp::install_bridge(&shipped, dir.path()).unwrap(),
        mcp::BridgeInstall::Unchanged
    );
    assert_eq!(names(), vec![mcp::BRIDGE_FILE.to_owned()]);
}

#[tokio::test]
async fn detailed_logs_are_a_saved_setting() {
    let dir = tempfile::tempdir().unwrap();
    let host = quiet_host(dir.path()).await;
    assert!(!host.detailed_logs());
    host.set_detailed_logs(true).unwrap();
    assert!(host.detailed_logs());
    assert!(AppSettings::load(&settings_path(dir.path())).detailed_logs);
    assert!(host.status().logs_dir.ends_with("logs"));
}

#[tokio::test]
async fn background_loops_skip_the_ticks_they_missed() {
    let mut timer = poll_timer(Duration::from_millis(50));
    timer.tick().await;
    // A round that took ten periods, as after a slow call or a night asleep.
    std::thread::sleep(Duration::from_millis(500));
    let mut ticks = 0;
    let window = tokio::time::sleep(Duration::from_millis(30));
    tokio::pin!(window);
    loop {
        tokio::select! {
            _ = timer.tick() => ticks += 1,
            _ = &mut window => break,
        }
    }
    // Catching up would tick about ten times at once.
    assert!(ticks <= 2, "{ticks} ticks");
}

#[test]
fn network_addresses() {
    let ip = |s: &str| s.parse::<IpAddr>().unwrap();
    let lan = || vec![ip("192.168.1.60"), ip("172.17.0.1")];
    let any = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    let localhost = IpAddr::V4(Ipv4Addr::LOCALHOST);

    // Listening everywhere: this machine's addresses, looked up when asked.
    assert_eq!(lan_addrs_for(any, lan), lan());
    // On one address: only that one. On loopback: none (and no lookup).
    assert_eq!(
        lan_addrs_for(ip("192.168.1.60"), || unreachable!()),
        [ip("192.168.1.60")]
    );
    assert!(lan_addrs_for(localhost, || unreachable!()).is_empty());

    // With adb, 127.0.0.1 comes after the network addresses, for tablets on a cable.
    assert_eq!(
        pairing_addrs_for(any, lan(), true),
        [ip("192.168.1.60"), ip("172.17.0.1"), localhost]
    );
    assert_eq!(pairing_addrs_for(any, lan(), false), lan());
    // No network at all: the cable still works, so the link isn't empty.
    assert_eq!(pairing_addrs_for(any, vec![], true), [localhost]);
    assert_eq!(pairing_addrs_for(localhost, vec![], false), [localhost]);
    assert_eq!(pairing_addrs_for(localhost, vec![], true), [localhost]);
    // Listening on IPv6's "every address" too.
    let any6 = IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED);
    assert_eq!(pairing_addrs_for(any6, vec![], true), [localhost]);
    // On one network address, adb's forward to 127.0.0.1 here wouldn't reach the host.
    let one = ip("192.168.1.60");
    assert_eq!(pairing_addrs_for(one, vec![one], true), [one]);
}

#[tokio::test]
async fn diagnostics_say_what_kind_of_network_this_is() {
    let dir = tempfile::tempdir().unwrap();
    let host = quiet_host(dir.path()).await;
    let report = host.diagnostics(None);
    // Only Windows says (and only for connected networks), so this checks the wiring.
    for category in discovery::network_categories() {
        assert!(report.contains(&category), "{report}");
    }
}
