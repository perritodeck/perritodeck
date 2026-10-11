use super::*;

#[tokio::test]
async fn starts_and_answers_ui_questions() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = HostOptions::new(dir.path().to_owned());
    opts.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
    opts.port = 0;
    opts.mdns = false;
    opts.watch_focus = false;
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
}

async fn quiet_host(dir: &std::path::Path) -> Host {
    let mut opts = HostOptions::new(dir.to_owned());
    opts.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
    opts.port = 0;
    opts.mdns = false;
    opts.watch_focus = false;
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
