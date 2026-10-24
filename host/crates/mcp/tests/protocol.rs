//! JSON-RPC and MCP conformance, through the transport-independent engine.

mod common;

use common::*;
use perritodeck_mcp::{Server, Session};
use serde_json::{json, Value};

fn names(list: &Value) -> Vec<String> {
    list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn initialize_handshake_and_version_negotiation() {
    let fake = Fake::new();
    let server = Server::new(fake.clone());
    let mut s = Session::new();
    let r = rpc(&server, &mut s, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}})).await.unwrap();
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(r["result"]["capabilities"]["tools"]["listChanged"], false);
    assert_eq!(r["result"]["serverInfo"]["name"], "perritodeck");
    assert!(r["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("[row, col]"));
    assert_eq!(s.protocol_version(), Some("2025-06-18"));
    assert!(!s.initialized());
    assert!(rpc(
        &server,
        &mut s,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .await
    .is_none());
    assert!(s.initialized());

    // A version this server doesn't know: it answers with its latest handshake version.
    let mut s = Session::new();
    let r = rpc(&server, &mut s, json!({"jsonrpc":"2.0","id":"x","method":"initialize","params":{"protocolVersion":"2099-01-01","capabilities":{}}})).await.unwrap();
    assert_eq!(r["result"]["protocolVersion"], "2025-11-25");

    // Old clients still get served.
    let mut s = Session::new();
    let r = rpc(&server, &mut s, json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}}})).await.unwrap();
    assert_eq!(r["result"]["protocolVersion"], "2024-11-05");

    // No version at all is an invalid request.
    let r = rpc(
        &server,
        &mut s,
        json!({"jsonrpc":"2.0","id":3,"method":"initialize","params":{}}),
    )
    .await
    .unwrap();
    assert_eq!(r["error"]["code"], -32602);

    let r = rpc(
        &server,
        &mut s,
        json!({"jsonrpc":"2.0","id":4,"method":"ping"}),
    )
    .await
    .unwrap();
    assert_eq!(r, json!({"jsonrpc":"2.0","id":4,"result":{}}));
}

#[tokio::test]
async fn tools_are_listed_by_permission() {
    let fake = Fake::new();
    let (server, mut s) = ready(&fake).await;
    let list = |s: &mut Session| {
        let server = server.clone();
        let mut s2 = s.clone();
        async move {
            rpc(
                &server,
                &mut s2,
                json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            )
            .await
            .unwrap()
        }
    };
    let read = [
        "list_profiles",
        "get_profile",
        "list_tablets",
        "list_live_values",
    ];
    assert_eq!(names(&list(&mut s).await), read);

    fake.allow(true, false, false);
    assert_eq!(
        names(&list(&mut s).await),
        [&read[..], &["press_key", "show_profile"]].concat()
    );

    fake.allow(false, true, false);
    let edit = [
        "set_key",
        "clear_key",
        "move_key",
        "add_page",
        "rename_page",
        "remove_page",
        "create_profile",
    ];
    assert_eq!(names(&list(&mut s).await), [&read[..], &edit[..]].concat());

    fake.allow(false, false, true);
    assert_eq!(
        names(&list(&mut s).await),
        [&read[..], &["run_action"]].concat()
    );

    fake.allow(true, true, true);
    let all = list(&mut s).await;
    assert_eq!(names(&all).len(), 14);
    for t in all["result"]["tools"].as_array().unwrap() {
        assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        assert!(
            t["description"].as_str().unwrap().len() > 40,
            "{}",
            t["name"]
        );
        assert!(t["annotations"].is_object());
    }
    // The list has one page; a cursor is an error.
    let r = rpc(
        &server,
        &mut s,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"cursor":"x"}}),
    )
    .await
    .unwrap();
    assert_eq!(r["error"]["code"], -32602);
}

#[tokio::test]
async fn calls_that_are_not_allowed_say_why() {
    let fake = Fake::new();
    let (server, mut s) = ready(&fake).await;
    for (tool, args, setting) in [
        ("press_key", json!({"row":1,"col":1}), "press keys"),
        ("set_key", json!({"row":2,"col":2,"title":"x"}), "edit keys"),
        (
            "run_action",
            json!({"action":{"type":"none"}}),
            "run actions",
        ),
    ] {
        let r = call(&server, &mut s, tool, args).await;
        assert!(is_error(&r));
        assert!(text(&r).contains(setting), "{}", text(&r));
        assert!(text(&r).contains("settings"));
    }
    assert!(fake.pressed.lock().unwrap().is_empty());
    assert!(fake.ran.lock().unwrap().is_empty());
}

#[tokio::test]
async fn protocol_errors() {
    let fake = Fake::new();
    let (server, mut s) = ready(&fake).await;
    let err = |r: Option<Value>| r.unwrap()["error"]["code"].as_i64().unwrap();

    // Not JSON: parse error with a null id.
    let r: Value =
        serde_json::from_str(&server.handle_text("{nope", &mut s).await.unwrap()).unwrap();
    assert_eq!(r["error"]["code"], -32700);
    assert_eq!(r["id"], Value::Null);

    assert_eq!(
        err(rpc(&server, &mut s, json!({"id":1,"method":"ping"})).await),
        -32600
    );
    assert_eq!(
        err(rpc(&server, &mut s, json!({"jsonrpc":"2.0","id":1})).await),
        -32600
    );
    assert_eq!(
        err(rpc(
            &server,
            &mut s,
            json!({"jsonrpc":"2.0","id":null,"method":"ping"})
        )
        .await),
        -32600
    );
    assert_eq!(err(rpc(&server, &mut s, json!("hello")).await), -32600);
    assert_eq!(
        err(rpc(
            &server,
            &mut s,
            json!({"jsonrpc":"2.0","id":1,"method":"resources/list"})
        )
        .await),
        -32601
    );
    assert_eq!(
        err(rpc(
            &server,
            &mut s,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":[1]})
        )
        .await),
        -32602
    );
    assert_eq!(
        err(rpc(
            &server,
            &mut s,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}})
        )
        .await),
        -32602
    );
    assert_eq!(err(rpc(&server, &mut s, json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_profile","arguments":"x"}})).await), -32602);
    let r = rpc(
        &server,
        &mut s,
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"format_disk"}}),
    )
    .await
    .unwrap();
    assert_eq!(r["error"]["code"], -32602);
    assert_eq!(r["id"], 7);
    assert!(r["error"]["message"]
        .as_str()
        .unwrap()
        .contains("format_disk"));

    // Notifications never get an answer, known or not; nor do responses.
    for n in [
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}),
        json!({"jsonrpc":"2.0","method":"notifications/whatever"}),
        json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"press_key"}}),
        json!({"jsonrpc":"2.0","id":5,"result":{}}),
    ] {
        assert!(rpc(&server, &mut s, n).await.is_none());
    }
    assert!(fake.pressed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn bad_tool_arguments_are_tool_errors() {
    let fake = Fake::new();
    fake.allow(true, true, true);
    let (server, mut s) = ready(&fake).await;
    for (tool, args, says) in [
        ("get_profile", json!({"profile":"nope"}), "no profile"),
        (
            "get_profile",
            json!({"profle":"starter"}),
            "Unknown argument",
        ),
        ("press_key", json!({"row":-1,"col":0}), "whole number"),
        ("press_key", json!({"row":2,"col":2}), "no key at [2,2]"),
        (
            "press_key",
            json!({"page":"Nope","row":0,"col":0}),
            "no page",
        ),
        (
            "set_key",
            json!({"row":2,"col":2,"action":{"type":"hotkey","keys":"Ctrl+Banana"}}),
            "unknown key",
        ),
        (
            "set_key",
            json!({"row":2,"col":2,"action":{"type":"teleport"}}),
            "isn't valid",
        ),
        ("set_key", json!({"row":2,"col":2,"bg":"blue"}), "colour"),
        ("move_key", json!({"from":[0],"to":[1,1]}), "[row, col]"),
        ("run_action", json!({}), "required"),
        (
            "show_profile",
            json!({"profile":"starter","tablet":"Old phone"}),
            "isn't connected",
        ),
    ] {
        let r = call(&server, &mut s, tool, args.clone()).await;
        assert!(is_error(&r), "{tool} {args}");
        assert!(text(&r).contains(says), "{tool} {args}: {}", text(&r));
    }
}

#[tokio::test]
async fn batches_follow_the_protocol_version() {
    let fake = Fake::new();
    // Not before initialize, and not in 2025-06-18 or later.
    let server = Server::new(fake.clone());
    let r = rpc(
        &server,
        &mut Session::new(),
        json!([{"jsonrpc":"2.0","id":1,"method":"ping"}]),
    )
    .await
    .unwrap();
    assert_eq!(r["error"]["code"], -32600);
    let (server, mut s) = ready_at(&fake, "2025-06-18").await;
    let r = rpc(
        &server,
        &mut s,
        json!([{"jsonrpc":"2.0","id":1,"method":"ping"}]),
    )
    .await
    .unwrap();
    assert_eq!(r["error"]["code"], -32600);

    // 2025-03-26 had them.
    let (server, mut s) = ready_at(&fake, "2025-03-26").await;
    let r = rpc(&server, &mut s, json!([
        {"jsonrpc":"2.0","id":1,"method":"ping"},
        {"jsonrpc":"2.0","method":"notifications/initialized"},
        {"jsonrpc":"2.0","id":2,"method":"nope"},
        {"jsonrpc":"2.0","id":3,"method":"initialize","params":{"protocolVersion":"2025-03-26"}},
    ])).await.unwrap();
    let r = r.as_array().unwrap();
    assert_eq!(r.len(), 3);
    assert_eq!(r[0], json!({"jsonrpc":"2.0","id":1,"result":{}}));
    assert_eq!(r[1]["error"]["code"], -32601);
    assert_eq!(r[2]["error"]["code"], -32600);
    assert!(rpc(
        &server,
        &mut s,
        json!([{"jsonrpc":"2.0","method":"notifications/initialized"}])
    )
    .await
    .is_none());
    assert_eq!(
        rpc(&server, &mut s, json!([])).await.unwrap()["error"]["code"],
        -32600
    );
}

#[tokio::test]
async fn structured_content_only_for_versions_that_have_it() {
    let fake = Fake::new();
    let (server, mut s) = ready_at(&fake, "2025-03-26").await;
    let r = call(&server, &mut s, "list_profiles", json!({})).await;
    assert!(r.get("structuredContent").is_none());
    assert!(text(&r).contains("starter: \"Starter\" (default"));
    let (server, mut s) = ready_at(&fake, "2025-06-18").await;
    let r = call(&server, &mut s, "list_profiles", json!({})).await;
    assert_eq!(r["structuredContent"]["profiles"][0]["id"], "starter");
    assert_eq!(r["isError"], false);
}

#[tokio::test]
async fn stateless_requests() {
    let fake = Fake::new();
    let server = Server::new(fake.clone());
    let mut s = Session::new();
    let r = rpc(
        &server,
        &mut s,
        json!({"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":modern_meta()}}),
    )
    .await
    .unwrap();
    assert_eq!(r["result"]["resultType"], "complete");
    let versions = r["result"]["supportedVersions"].as_array().unwrap();
    assert_eq!(versions[0], "2026-07-28");
    assert!(versions.contains(&json!("2025-11-25")));
    assert!(r["result"]["capabilities"]["tools"].is_object());
    assert_eq!(
        r["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "perritodeck"
    );

    let r = rpc(
        &server,
        &mut s,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta":modern_meta()}}),
    )
    .await
    .unwrap();
    assert_eq!(r["result"]["resultType"], "complete");
    assert_eq!(r["result"]["cacheScope"], "private");
    assert!(r["result"]["ttlMs"].is_u64());
    assert_eq!(names(&r).len(), 4);

    let r = rpc(&server, &mut s, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_tablets","arguments":{},"_meta":modern_meta()}})).await.unwrap();
    assert_eq!(r["result"]["resultType"], "complete");
    assert_eq!(
        r["result"]["structuredContent"]["tablets"][0]["id"],
        "tab-1"
    );
    assert!(text(&r["result"]).contains(
        "\"Studio tablet\" (id tab-1): connected, showing \"Starter\" (starter), page \"Main\""
    ));

    // Unsupported version, with the list of supported ones.
    let mut meta = modern_meta();
    meta["io.modelcontextprotocol/protocolVersion"] = json!("1900-01-01");
    let r = rpc(
        &server,
        &mut s,
        json!({"jsonrpc":"2.0","id":4,"method":"tools/list","params":{"_meta":meta}}),
    )
    .await
    .unwrap();
    assert_eq!(r["error"]["code"], -32022);
    assert_eq!(r["error"]["data"]["requested"], "1900-01-01");
    assert!(r["error"]["data"]["supported"]
        .as_array()
        .unwrap()
        .contains(&json!("2026-07-28")));

    // Client capabilities are required.
    let mut meta = modern_meta();
    meta.as_object_mut()
        .unwrap()
        .remove("io.modelcontextprotocol/clientCapabilities");
    let r = rpc(
        &server,
        &mut s,
        json!({"jsonrpc":"2.0","id":5,"method":"tools/list","params":{"_meta":meta}}),
    )
    .await
    .unwrap();
    assert_eq!(r["error"]["code"], -32602);

    // ping is gone in this version; server/discover needs _meta.
    let r = rpc(
        &server,
        &mut s,
        json!({"jsonrpc":"2.0","id":6,"method":"ping","params":{"_meta":modern_meta()}}),
    )
    .await
    .unwrap();
    assert_eq!(r["error"]["code"], -32601);
    let r = rpc(
        &server,
        &mut s,
        json!({"jsonrpc":"2.0","id":7,"method":"server/discover"}),
    )
    .await
    .unwrap();
    assert_eq!(r["error"]["code"], -32602);
    // None of this touched the session.
    assert_eq!(s, Session::new());
}

#[tokio::test]
async fn reading_profiles_tablets_and_values() {
    let fake = Fake::new();
    let (server, mut s) = ready(&fake).await;
    let r = call(&server, &mut s, "get_profile", json!({})).await;
    let t = text(&r);
    assert!(t.contains("Profile \"Starter\" (id starter)"), "{t}");
    assert!(t.contains("3 rows x 5 columns"), "{t}");
    assert!(t.contains("Page \"More\""), "{t}");
    assert!(t.contains("[1,1] \"Copy\" - hotkey Ctrl+C  {\"action\":{\"keys\":\"Ctrl+C\",\"type\":\"hotkey\"},\"title\":\"Copy\"}"), "{t}");
    assert!(t.contains("[2,0] \"More ›\" - opens page \"More\""), "{t}");
    assert!(
        t.contains("[1,0] \"Mic\\nlive\" - toggle with 2 states"),
        "{t}"
    );
    assert_eq!(r["structuredContent"]["pages"][0]["name"], "Main");

    // By name, case-insensitively.
    let r = call(
        &server,
        &mut s,
        "get_profile",
        json!({"profile":"STREAMING"}),
    )
    .await;
    assert!(
        text(&r).contains("Page \"Scenes\" (0 keys)"),
        "{}",
        text(&r)
    );

    let r = call(&server, &mut s, "list_live_values", json!({})).await;
    assert!(text(&r).contains("{cpu} = \"12%\""), "{}", text(&r));
    assert!(text(&r).contains("{time}"));
}

#[tokio::test]
async fn pressing_and_showing() {
    let fake = Fake::new();
    fake.allow(true, false, false);
    let (server, mut s) = ready(&fake).await;
    let r = call(&server, &mut s, "press_key", json!({"row":1,"col":1})).await;
    assert!(!is_error(&r), "{}", text(&r));
    assert!(
        text(&r)
            .contains("Pressed \"Copy\" at [1,1] on page \"Main\" of \"Starter\": hotkey Ctrl+C"),
        "{}",
        text(&r)
    );
    let r = call(
        &server,
        &mut s,
        "press_key",
        json!({"profile":"starter","page":"more","row":0,"col":0,"tablet":"studio tablet"}),
    )
    .await;
    assert!(!is_error(&r), "{}", text(&r));
    let pressed = fake.pressed.lock().unwrap().clone();
    assert_eq!(pressed.len(), 2);
    assert_eq!(
        (
            pressed[0].profile.as_str(),
            pressed[0].page.as_str(),
            pressed[0].row,
            pressed[0].col,
            pressed[0].tablet.as_deref()
        ),
        ("starter", "Main", 1, 1, None)
    );
    assert_eq!(
        (pressed[1].page.as_str(), pressed[1].tablet.as_deref()),
        ("More", Some("tab-1"))
    );

    // One connected tablet: no need to name it.
    let r = call(
        &server,
        &mut s,
        "show_profile",
        json!({"profile":"Streaming"}),
    )
    .await;
    assert!(!is_error(&r), "{}", text(&r));
    assert_eq!(
        fake.shown.lock().unwrap()[0],
        ("tab-1".to_owned(), "streaming".to_owned())
    );
}

#[tokio::test]
async fn sliders_cannot_be_pressed() {
    let fake = Fake::new();
    fake.allow(true, true, false);
    let (server, mut s) = ready(&fake).await;
    let r = call(
        &server,
        &mut s,
        "set_key",
        json!({"row":2,"col":2,"title":"Vol","slider":{"type":"volume"}}),
    )
    .await;
    assert!(!is_error(&r), "{}", text(&r));
    let r = call(&server, &mut s, "press_key", json!({"row":2,"col":2})).await;
    assert!(is_error(&r));
    assert!(text(&r).contains("slider"));
    assert!(fake.pressed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn making_and_changing_keys() {
    let fake = Fake::new();
    fake.allow(false, true, false);
    let (server, mut s) = ready(&fake).await;

    // A new key in an empty cell.
    let r = call(&server, &mut s, "set_key", json!({"row":2,"col":2,"title":"Zoom\nmute","bg":"#F38BA8","action":{"type":"hotkey","keys":"Alt+A"}})).await;
    assert!(!is_error(&r), "{}", text(&r));
    assert!(
        text(&r).starts_with("Created the key at [2,2]"),
        "{}",
        text(&r)
    );
    let k = fake.key("starter", "Main", [2, 2]).unwrap();
    assert_eq!(k.title.as_deref(), Some("Zoom\nmute"));
    assert_eq!(
        k.action,
        Some(perritodeck_mcp::ActionSpec::Hotkey {
            keys: "Alt+A".into()
        })
    );

    // Changing only the title keeps the rest; null removes a field.
    let r = call(
        &server,
        &mut s,
        "set_key",
        json!({"row":2,"col":2,"title":"Mute","bg":null}),
    )
    .await;
    assert!(!is_error(&r), "{}", text(&r));
    let k = fake.key("starter", "Main", [2, 2]).unwrap();
    assert_eq!(k.title.as_deref(), Some("Mute"));
    assert_eq!(k.bg, None);
    assert!(k.action.is_some());

    // Giving it a page to open replaces the action.
    let r = call(
        &server,
        &mut s,
        "set_key",
        json!({"row":2,"col":2,"nav":{"page":"More"}}),
    )
    .await;
    assert!(!is_error(&r), "{}", text(&r));
    let k = fake.key("starter", "Main", [2, 2]).unwrap();
    assert!(k.action.is_none());
    assert!(k.nav.is_some());
    assert_eq!(k.title.as_deref(), Some("Mute"));

    // replace: start over.
    let r = call(
        &server,
        &mut s,
        "set_key",
        json!({"row":2,"col":2,"replace":true,"action":{"type":"media","key":"mute"}}),
    )
    .await;
    assert!(!is_error(&r), "{}", text(&r));
    let k = fake.key("starter", "Main", [2, 2]).unwrap();
    assert!(k.title.is_none() && k.nav.is_none());

    // A toggle.
    let r = call(
        &server,
        &mut s,
        "set_key",
        json!({"page":"More","row":2,"col":4,"follow":"obs:recording","states":[
            {"title":"REC","action":{"type":"obs","op":"startRecord"}},
            {"title":"STOP","bg":"#F38BA8","action":{"type":"obs","op":"stopRecord"}}
        ]}),
    )
    .await;
    assert!(!is_error(&r), "{}", text(&r));
    let k = fake.key("starter", "More", [2, 4]).unwrap();
    assert_eq!(k.states.len(), 2);
    assert_eq!(k.follow.as_deref(), Some("obs:recording"));

    // The profile's own checks still apply (the host's error comes back).
    let r = call(
        &server,
        &mut s,
        "set_key",
        json!({"row":2,"col":3,"nav":{"page":"Nowhere"}}),
    )
    .await;
    assert!(is_error(&r));
    assert!(text(&r).contains("Nowhere"), "{}", text(&r));

    // Remove, move (swapping), pages.
    let r = call(&server, &mut s, "clear_key", json!({"row":2,"col":2})).await;
    assert!(!is_error(&r), "{}", text(&r));
    assert!(fake.key("starter", "Main", [2, 2]).is_none());
    let r = call(
        &server,
        &mut s,
        "move_key",
        json!({"from":[1,1],"to":[1,2]}),
    )
    .await;
    assert!(text(&r).contains("\"Paste\" was there"), "{}", text(&r));
    assert_eq!(
        fake.key("starter", "Main", [1, 2])
            .unwrap()
            .title
            .as_deref(),
        Some("Copy")
    );
    let r = call(
        &server,
        &mut s,
        "move_key",
        json!({"from":[1,2],"to":[2,3],"to_page":"More"}),
    )
    .await;
    assert!(!is_error(&r), "{}", text(&r));
    assert_eq!(
        fake.key("starter", "More", [2, 3])
            .unwrap()
            .title
            .as_deref(),
        Some("Copy")
    );

    for (tool, args) in [
        ("add_page", json!({"name":"Stream"})),
        ("rename_page", json!({"from":"stream","to":"OBS"})),
        ("remove_page", json!({"name":"OBS"})),
    ] {
        let r = call(&server, &mut s, tool, args).await;
        assert!(!is_error(&r), "{tool}: {}", text(&r));
    }
    let names: Vec<String> = fake
        .profile("starter")
        .pages
        .iter()
        .map(|p| p.name.clone())
        .collect();
    assert_eq!(names, ["Main", "More"]);

    let r = call(
        &server,
        &mut s,
        "create_profile",
        json!({"name":"Zoom calls"}),
    )
    .await;
    assert!(text(&r).contains("id zoom-calls"), "{}", text(&r));
    let r = call(&server, &mut s, "set_key", json!({"profile":"zoom-calls","row":0,"col":0,"title":"Hi","action":{"type":"text","text":"Hello"}})).await;
    assert!(!is_error(&r), "{}", text(&r));
    // A key that switches the tablet to another profile, at one of its pages.
    let r = call(&server, &mut s, "set_key", json!({"profile":"zoom-calls","row":0,"col":1,"title":"Home","nav":{"profile":"starter","page":"More"}})).await;
    assert!(!is_error(&r), "{}", text(&r));
    assert!(
        text(&r).contains("switches the tablet to profile \"starter\", page \"More\""),
        "{}",
        text(&r)
    );
}

#[tokio::test]
async fn keys_that_run_programs_need_the_run_permission() {
    let fake = Fake::new();
    fake.allow(true, true, false);
    let (server, mut s) = ready(&fake).await;
    let before = fake.profiles.lock().unwrap()["starter"].clone();
    for action in [
        json!({"type":"command","program":"rm","args":["-rf","/"]}),
        json!({"type":"launch","program":"calc"}),
        json!({"type":"openPath","path":"/tmp/x.sh"}),
        json!({"type":"plugin","plugin":"p","action":"a"}),
        json!({"type":"multi","steps":[{"action":{"type":"hotkey","keys":"Ctrl+C"}},{"action":{"type":"launch","program":"calc"}}]}),
        json!({"type":"random","actions":[{"type":"hotkey","keys":"Ctrl+C"},{"type":"launch","program":"calc"}]}),
        json!({"type":"multi","steps":[{"action":{"type":"random","actions":[{"type":"command","program":"rm"}]}}]}),
    ] {
        let r = call(
            &server,
            &mut s,
            "set_key",
            json!({"row":2,"col":2,"action":action}),
        )
        .await;
        assert!(is_error(&r), "{action}");
        assert!(text(&r).contains("run actions"), "{}", text(&r));
    }
    let r = call(
        &server,
        &mut s,
        "set_key",
        json!({"row":2,"col":2,"slider":{"type":"command","program":"vol","args":["{value}"]}}),
    )
    .await;
    assert!(is_error(&r));
    // Changing an existing command's arguments is a new command, too.
    let r = call(&server, &mut s, "set_key", json!({"page":"More","row":0,"col":1,"action":{"type":"command","program":"hostname","args":["-x"]}})).await;
    assert!(is_error(&r));
    assert_eq!(fake.profiles.lock().unwrap()["starter"], before);

    // Retitling a key that already runs a command is fine.
    let r = call(
        &server,
        &mut s,
        "set_key",
        json!({"page":"More","row":0,"col":1,"title":"Host"}),
    )
    .await;
    assert!(!is_error(&r), "{}", text(&r));
    // And with the permission, commands are allowed.
    fake.allow(true, true, true);
    let r = call(
        &server,
        &mut s,
        "set_key",
        json!({"row":2,"col":2,"action":{"type":"command","program":"git","args":["pull"]}}),
    )
    .await;
    assert!(!is_error(&r), "{}", text(&r));
}

/// Typing a command and pressing Enter runs it as surely as a command action, so a key
/// that does both needs the same permission (the import check lists the same keys).
#[tokio::test]
async fn keys_that_type_and_press_enter_need_the_run_permission() {
    let fake = Fake::new();
    fake.allow(true, true, false);
    let (server, mut s) = ready(&fake).await;
    let before = fake.profiles.lock().unwrap()["starter"].clone();
    let enter_after = |text: &str, chord: &str| {
        json!({"type":"multi","steps":[
            {"action":{"type":"hotkey","keys":"Win+R"}},
            {"action":{"type":"text","text":text}},
            {"action":{"type":"hotkey","keys":chord}}]})
    };
    for args in [
        json!({"row":2,"col":2,"action":{"type":"text","text":"calc
"}}),
        json!({"row":2,"col":2,"action":{"type":"text","text":"calc
"}}),
        json!({"row":2,"col":2,"action":enter_after("calc", "Enter")}),
        json!({"row":2,"col":2,"action":enter_after("calc", "Ctrl+M")}),
        json!({"row":2,"col":2,"action":{"type":"random","actions":[{"type":"text","text":"gg"},{"type":"hotkey","keys":"Return"}]}}),
        // Text in one state, Enter in the other.
        json!({"row":2,"col":2,"states":[
            {"title":"A","action":{"type":"text","text":"calc"}},
            {"title":"B","action":{"type":"hotkey","keys":"Enter"}}]}),
        // The starter's "Type hello" key, given an Enter after its text.
        json!({"row":1,"col":3,"action":{"type":"multi","steps":[
            {"action":{"type":"text","text":"Hello from Perrito! "}},
            {"action":{"type":"hotkey","keys":"Enter"}}]}}),
    ] {
        let r = call(&server, &mut s, "set_key", args.clone()).await;
        assert!(is_error(&r), "{args}");
        assert!(text(&r).contains("run actions"), "{}", text(&r));
        assert!(text(&r).contains("presses Enter"), "{}", text(&r));
    }
    assert_eq!(fake.profiles.lock().unwrap()["starter"], before);

    // Typing alone, Enter alone, and a chord that isn't Enter are fine.
    for args in [
        json!({"row":2,"col":2,"action":{"type":"text","text":"Hello"}}),
        json!({"row":2,"col":2,"action":{"type":"hotkey","keys":"Enter"}}),
        json!({"row":2,"col":2,"action":enter_after("calc", "Ctrl+Shift+M")}),
    ] {
        let r = call(&server, &mut s, "set_key", args.clone()).await;
        assert!(!is_error(&r), "{args}: {}", text(&r));
    }

    // With the permission it's allowed; without it, retitling that key still is.
    fake.allow(true, true, true);
    let r = call(
        &server,
        &mut s,
        "set_key",
        json!({"row":2,"col":3,"action":{"type":"text","text":"ls
"}}),
    )
    .await;
    assert!(!is_error(&r), "{}", text(&r));
    fake.allow(true, true, false);
    let r = call(
        &server,
        &mut s,
        "set_key",
        json!({"row":2,"col":3,"title":"ls"}),
    )
    .await;
    assert!(!is_error(&r), "{}", text(&r));
    // But changing what it types is new.
    let r = call(
        &server,
        &mut s,
        "set_key",
        json!({"row":2,"col":3,"action":{"type":"text","text":"rm -rf ~
"}}),
    )
    .await;
    assert!(is_error(&r), "{}", text(&r));
}

#[tokio::test]
async fn running_actions() {
    let fake = Fake::new();
    fake.allow(false, false, true);
    let (server, mut s) = ready(&fake).await;
    let r = call(
        &server,
        &mut s,
        "run_action",
        json!({"action":{"type":"hotkey","keys":"Ctrl+Shift+M"}}),
    )
    .await;
    assert_eq!(text(&r), "Done: hotkey Ctrl+Shift+M.");
    let r = call(
        &server,
        &mut s,
        "run_action",
        json!({"action":{"type":"command","program":"false"}}),
    )
    .await;
    assert!(!is_error(&r));
    assert!(text(&r).contains("reported a problem: exited with status 1"));
    let r = call(
        &server,
        &mut s,
        "run_action",
        json!({"action":{"type":"hotkey","keys":"Ctrl+Nope"}}),
    )
    .await;
    assert!(is_error(&r));
    assert_eq!(fake.ran.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn tool_calls_are_rate_limited() {
    let fake = Fake::new();
    let (server, mut s) = ready(&fake).await;
    let mut limited = 0;
    for _ in 0..70 {
        let r = call(&server, &mut s, "list_tablets", json!({})).await;
        if is_error(&r) {
            assert!(text(&r).contains("Too many"));
            limited += 1;
        }
    }
    assert_eq!(limited, 10);
}
