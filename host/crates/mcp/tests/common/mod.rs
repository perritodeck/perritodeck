//! A fake host for the tests: real profile files (the starter profile) edited with the
//! real editor, in memory, and records of what was pressed and run.

#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use perritodeck_mcp::{
    ActionSpec, Backend, BoxFuture, Edit, KeyDetail, KeyFields, LiveValue, PageDetail, Permissions,
    Press, ProfileDetail, ProfileInfo, Server, Session, TabletInfo,
};
use perritodeck_profile::{edit::apply, Profile, STARTER_PROFILE};
use serde_json::{json, Value};

#[derive(Default)]
pub struct Fake {
    pub perms: Mutex<Permissions>,
    pub profiles: Mutex<BTreeMap<String, String>>,
    pub tablets: Mutex<Vec<TabletInfo>>,
    pub pressed: Mutex<Vec<Press>>,
    pub ran: Mutex<Vec<ActionSpec>>,
    pub shown: Mutex<Vec<(String, String)>>,
}

impl Fake {
    pub fn new() -> Arc<Self> {
        let f = Fake::default();
        f.profiles
            .lock()
            .unwrap()
            .insert("starter".into(), STARTER_PROFILE.into());
        f.profiles.lock().unwrap().insert(
            "streaming".into(),
            "schema_version = 1\nname = \"Streaming\"\n\n[[pages]]\nname = \"Scenes\"\n".into(),
        );
        *f.tablets.lock().unwrap() = vec![
            TabletInfo {
                id: "tab-1".into(),
                name: "Studio tablet".into(),
                connected: true,
                profile: "starter".into(),
                page: Some("Main".into()),
            },
            TabletInfo {
                id: "tab-2".into(),
                name: "Old phone".into(),
                connected: false,
                profile: "starter".into(),
                page: None,
            },
        ];
        Arc::new(f)
    }

    pub fn allow(&self, press: bool, edit: bool, run: bool) {
        *self.perms.lock().unwrap() = Permissions { press, edit, run };
    }

    pub fn profile(&self, id: &str) -> Profile {
        Profile::parse(&self.profiles.lock().unwrap()[id]).unwrap()
    }

    pub fn key(&self, id: &str, page: &str, at: [u32; 2]) -> Option<perritodeck_profile::KeyDef> {
        let p = self.profile(id);
        p.pages[p.page_index(page).unwrap()]
            .keys
            .iter()
            .find(|k| k.at == at)
            .cloned()
    }
}

impl Backend for Fake {
    fn permissions(&self) -> Permissions {
        *self.perms.lock().unwrap()
    }

    fn list_profiles(&self) -> Vec<ProfileInfo> {
        self.profiles
            .lock()
            .unwrap()
            .iter()
            .map(|(id, text)| {
                let p = Profile::parse(text).unwrap();
                ProfileInfo {
                    id: id.clone(),
                    name: p.name,
                    pages: p.pages.len(),
                    is_default: id == "starter",
                    has_error: false,
                }
            })
            .collect()
    }

    fn get_profile(&self, id: &str) -> Result<ProfileDetail, String> {
        let text = self
            .profiles
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| format!("There's no profile called {id}."))?;
        let p = Profile::parse(&text)?;
        Ok(ProfileDetail {
            id: id.into(),
            name: p.name.clone(),
            rows: p.grid.map_or(3, |g| g.rows),
            cols: p.grid.map_or(5, |g| g.cols),
            fixed_grid: p.grid.is_some(),
            companion: p.companion,
            reload_error: None,
            pages: p
                .pages
                .iter()
                .map(|pg| PageDetail {
                    name: pg.name.clone(),
                    keys: pg
                        .keys
                        .iter()
                        .map(|k| KeyDetail {
                            row: k.at[0],
                            col: k.at[1],
                            summary: String::new(),
                            fields: KeyFields {
                                title: k.title.clone(),
                                bg: k.bg.clone(),
                                fg: k.fg.clone(),
                                action: k.action.clone(),
                                nav: k.nav.clone(),
                                icon: k.icon.clone(),
                                slider: k.slider.clone(),
                                states: k.states.clone(),
                                follow: k.follow.clone(),
                            },
                        })
                        .collect(),
                })
                .collect(),
        })
    }

    fn list_tablets(&self) -> Vec<TabletInfo> {
        self.tablets.lock().unwrap().clone()
    }

    fn live_values(&self) -> Vec<LiveValue> {
        vec![LiveValue {
            name: "cpu".into(),
            value: Some("12%".into()),
        }]
    }

    fn press_key<'a>(&'a self, press: &'a Press) -> BoxFuture<'a, Result<Option<String>, String>> {
        Box::pin(async move {
            self.pressed.lock().unwrap().push(press.clone());
            Ok(None)
        })
    }

    fn show_profile(&self, tablet_id: &str, profile_id: &str) -> Result<(), String> {
        self.shown
            .lock()
            .unwrap()
            .push((tablet_id.into(), profile_id.into()));
        Ok(())
    }

    fn edit_profile(&self, profile_id: &str, edits: &[Edit]) -> Result<(), String> {
        let mut all = self.profiles.lock().unwrap();
        let text = all
            .get(profile_id)
            .ok_or_else(|| format!("There's no profile called {profile_id}."))?;
        let out = apply(text, edits).map_err(|e| e.to_string())?;
        all.insert(profile_id.into(), out);
        Ok(())
    }

    fn create_profile(&self, name: &str) -> Result<String, String> {
        let id: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        self.profiles.lock().unwrap().insert(
            id.clone(),
            format!("schema_version = 1\nname = {name:?}\n\n[[pages]]\nname = \"Main\"\n"),
        );
        Ok(id)
    }

    fn run_action<'a>(
        &'a self,
        action: &'a ActionSpec,
    ) -> BoxFuture<'a, Result<Option<String>, String>> {
        Box::pin(async move {
            self.ran.lock().unwrap().push(action.clone());
            match action {
                ActionSpec::Command { program, .. } if program == "false" => {
                    Ok(Some("exited with status 1".into()))
                }
                _ => Ok(None),
            }
        })
    }
}

/// Send one message; the reply, parsed.
pub async fn rpc(server: &Server, session: &mut Session, msg: Value) -> Option<Value> {
    server
        .handle_text(&msg.to_string(), session)
        .await
        .map(|s| serde_json::from_str(&s).unwrap())
}

/// A server with an initialized 2025-11-25 session.
pub async fn ready(fake: &Arc<Fake>) -> (Server, Session) {
    ready_at(fake, "2025-11-25").await
}

pub async fn ready_at(fake: &Arc<Fake>, version: &str) -> (Server, Session) {
    let server = Server::new(fake.clone());
    let mut s = Session::new();
    let r = rpc(
        &server,
        &mut s,
        json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
    )
    .await
    .unwrap();
    assert_eq!(r["result"]["protocolVersion"], version);
    assert!(rpc(
        &server,
        &mut s,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .await
    .is_none());
    (server, s)
}

/// Call a tool; the whole result object.
pub async fn call(server: &Server, s: &mut Session, name: &str, args: Value) -> Value {
    let r = rpc(
        server,
        s,
        json!({"jsonrpc":"2.0","id":"c","method":"tools/call","params":{"name":name,"arguments":args}}),
    )
    .await
    .unwrap();
    assert!(r.get("error").is_none(), "{r}");
    r["result"].clone()
}

pub fn text(result: &Value) -> String {
    result["content"][0]["text"].as_str().unwrap().to_owned()
}

pub fn is_error(result: &Value) -> bool {
    result["isError"].as_bool().unwrap()
}

pub fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "1" },
    })
}
