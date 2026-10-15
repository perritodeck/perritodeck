//! Keeps live values on keys current: `{cpu}`, `{mem}`, `{ha:<entity>}` (state and
//! unit, or `{ha:<entity>.<attribute>}`), `{obs:recording}`, `{obs:streaming}` and
//! `{obs:scene}`. Only what some profile uses is fetched.

use std::{sync::Arc, time::Duration};

use perritodeck_actions::Integrations;
use perritodeck_deck::Deck;
use serde_json::{json, Value};
use tokio::task::JoinHandle;

const EVERY: Duration = Duration::from_secs(2);
/// Shown when a value can't be fetched (Home Assistant down, OBS closed).
pub(crate) const UNKNOWN: &str = "–";
/// Values asked of another app, which may take a while to answer (a connection attempt
/// to an OBS that's closed waits up to its 5 s timeout). Each app is asked beside the
/// loop, so the rest keeps its rhythm.
const ASKED: [&str; 2] = ["ha:", "obs:"];

pub fn spawn(deck: Arc<Deck>, integrations: Option<Arc<dyn Integrations>>) {
    tokio::spawn(async move {
        let mut live = Live::new(deck, integrations);
        let mut every = crate::poll_timer(EVERY);
        loop {
            every.tick().await;
            live.round();
        }
    });
}

struct Live {
    deck: Arc<Deck>,
    integrations: Option<Arc<dyn Integrations>>,
    sys: sysinfo::System,
    /// Each of [`ASKED`]'s last round of questions, which may still be waiting.
    asking: [Option<JoinHandle<()>>; 2],
}

impl Live {
    fn new(deck: Arc<Deck>, integrations: Option<Arc<dyn Integrations>>) -> Self {
        Self {
            deck,
            integrations,
            sys: sysinfo::System::new(),
            asking: Default::default(),
        }
    }

    /// Show this computer's values now, and ask Home Assistant and OBS for theirs; those
    /// show when they answer. An app still answering the last round isn't asked again.
    fn round(&mut self) {
        let tokens = self.deck.live_tokens();
        if tokens.is_empty() {
            return;
        }
        let mut values = Vec::new();
        if tokens.iter().any(|t| t == "cpu") {
            self.sys.refresh_cpu_usage();
            let cpu = self.sys.global_cpu_usage();
            values.push(("cpu".to_owned(), format!("{cpu:.0}%")));
        }
        if tokens.iter().any(|t| t == "mem") {
            self.sys.refresh_memory();
            let pct = self.sys.used_memory() as f64 / self.sys.total_memory().max(1) as f64 * 100.0;
            values.push(("mem".to_owned(), format!("{pct:.0}%")));
        }
        self.deck.set_live(values);
        for (asking, prefix) in self.asking.iter_mut().zip(ASKED) {
            let wanted: Vec<String> = tokens
                .iter()
                .filter(|t| t.starts_with(prefix))
                .cloned()
                .collect();
            if wanted.is_empty() || asking.as_ref().is_some_and(|a| !a.is_finished()) {
                continue;
            }
            let (deck, integrations) = (self.deck.clone(), self.integrations.clone());
            *asking = Some(tokio::spawn(async move {
                let mut values = Vec::new();
                for t in wanted {
                    let v = ask(integrations.as_deref(), &t).await;
                    values.push((t, v));
                }
                deck.set_live(values);
            }));
        }
    }
}

/// A Home Assistant or OBS value.
async fn ask(integrations: Option<&dyn Integrations>, token: &str) -> String {
    if let Some(entity) = token.strip_prefix("ha:") {
        home_assistant(integrations, entity).await
    } else if let Some(what) = token.strip_prefix("obs:") {
        obs(integrations, what).await
    } else {
        UNKNOWN.into()
    }
}

/// `sensor.outside` → "4.5 °C"; `light.desk` → "on"; `light.desk.brightness` → "128".
async fn home_assistant(ha: Option<&dyn Integrations>, token: &str) -> String {
    let Some(ha) = ha else { return UNKNOWN.into() };
    // Entity ids are domain.object; anything after a second dot is an attribute.
    let mut parts = token.splitn(3, '.');
    let (domain, object, attribute) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next(),
    );
    let entity = format!("{domain}.{object}");
    let Ok(state) = ha.ha_state(&entity).await else {
        return UNKNOWN.into();
    };
    match attribute {
        Some(a) => display(&state["attributes"][a]),
        None => {
            let s = display(&state["state"]);
            match state["attributes"]["unit_of_measurement"].as_str() {
                Some(unit) if s != UNKNOWN => format!("{s} {unit}"),
                _ => s,
            }
        }
    }
}

async fn obs(obs: Option<&dyn Integrations>, what: &str) -> String {
    let Some(obs) = obs else {
        return UNKNOWN.into();
    };
    // `currentProgramSceneName` is deprecated in favour of `sceneName` (obs-websocket
    // 5.3+); either will do.
    let (request, fields): (_, &[&str]) = match what {
        "recording" => ("GetRecordStatus", &["outputActive"]),
        "streaming" => ("GetStreamStatus", &["outputActive"]),
        "scene" => (
            "GetCurrentProgramScene",
            &["currentProgramSceneName", "sceneName"],
        ),
        _ => return UNKNOWN.into(),
    };
    match obs.obs_request(request, json!({})).await {
        Ok(resp) => match fields.iter().map(|f| &resp[f]).find(|v| !v.is_null()) {
            Some(Value::Bool(true)) => "on".into(),
            Some(Value::Bool(false)) => "off".into(),
            Some(v) => display(v),
            None => UNKNOWN.into(),
        },
        Err(_) => UNKNOWN.into(),
    }
}

fn display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => if *b { "on" } else { "off" }.into(),
        Value::Null => UNKNOWN.into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use perritodeck_actions::{
        system::{EnigoInput, OpenSystem},
        ActionError, BoxFuture, ObsAction, Outcome,
    };

    /// What `GetCurrentProgramScene` answers.
    struct Fake(Value);
    impl Integrations for Fake {
        fn obs<'a>(&'a self, _: &'a ObsAction) -> BoxFuture<'a, Result<Outcome, ActionError>> {
            Box::pin(async { Ok(Outcome::Ok) })
        }
        fn home_assistant<'a>(
            &'a self,
            _: &'a str,
            _: Option<&'a str>,
            _: Option<&'a Value>,
        ) -> BoxFuture<'a, Result<Outcome, ActionError>> {
            Box::pin(async { Ok(Outcome::Ok) })
        }
        fn obs_request<'a>(
            &'a self,
            req: &'a str,
            _: Value,
        ) -> BoxFuture<'a, Result<Value, ActionError>> {
            Box::pin(async move {
                Ok(match req {
                    "GetRecordStatus" => json!({ "outputActive": true }),
                    "GetCurrentProgramScene" => self.0.clone(),
                    _ => return Err(ActionError::NotConfigured("OBS".into())),
                })
            })
        }
        fn ha_state<'a>(&'a self, entity: &'a str) -> BoxFuture<'a, Result<Value, ActionError>> {
            Box::pin(async move {
                Ok(match entity {
                    "sensor.outside" => {
                        json!({ "state": "4.5", "attributes": { "unit_of_measurement": "°C" } })
                    }
                    "light.desk" => json!({ "state": "on", "attributes": { "brightness": 128 } }),
                    _ => return Err(ActionError::NotConfigured("Home Assistant".into())),
                })
            })
        }
    }

    #[tokio::test]
    async fn values_from_home_assistant_and_obs() {
        let fake = Fake(json!({ "currentProgramSceneName": "Gameplay" }));
        let f: &dyn Integrations = &fake;
        assert_eq!(home_assistant(Some(f), "sensor.outside").await, "4.5 °C");
        assert_eq!(home_assistant(Some(f), "light.desk").await, "on");
        assert_eq!(
            home_assistant(Some(f), "light.desk.brightness").await,
            "128"
        );
        assert_eq!(home_assistant(Some(f), "light.gone").await, UNKNOWN);
        assert_eq!(home_assistant(None, "light.desk").await, UNKNOWN);
        assert_eq!(obs(Some(f), "recording").await, "on");
        assert_eq!(obs(Some(f), "scene").await, "Gameplay");
        assert_eq!(obs(Some(f), "streaming").await, UNKNOWN);
        assert_eq!(obs(Some(f), "nonsense").await, UNKNOWN);
    }

    /// An OBS that answers only when let through, as slowly as a closed OBS on an
    /// address that never answers, beside a Home Assistant that answers at once.
    struct SlowObs {
        answer: tokio::sync::Semaphore,
        asked: std::sync::atomic::AtomicUsize,
    }
    impl Integrations for SlowObs {
        fn obs<'a>(&'a self, _: &'a ObsAction) -> BoxFuture<'a, Result<Outcome, ActionError>> {
            Box::pin(async { Ok(Outcome::Ok) })
        }
        fn home_assistant<'a>(
            &'a self,
            _: &'a str,
            _: Option<&'a str>,
            _: Option<&'a Value>,
        ) -> BoxFuture<'a, Result<Outcome, ActionError>> {
            Box::pin(async { Ok(Outcome::Ok) })
        }
        fn obs_request<'a>(
            &'a self,
            _: &'a str,
            _: Value,
        ) -> BoxFuture<'a, Result<Value, ActionError>> {
            self.asked
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Box::pin(async move {
                self.answer.acquire().await.unwrap().forget();
                Ok(json!({ "sceneName": "Gameplay" }))
            })
        }
        fn ha_state<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Value, ActionError>> {
            Box::pin(async { Ok(json!({ "state": "on" })) })
        }
    }

    /// While OBS takes its time, {cpu} keeps its rhythm and {ha:…} still comes; OBS's
    /// value shows when it answers, and only then is OBS asked again.
    #[tokio::test]
    async fn a_slow_obs_holds_up_no_other_value() {
        let profile = perritodeck_profile::Profile::parse(
            "schema_version = 1\nname = \"L\"\n[[pages]]\nname = \"Main\"\n[[pages.keys]]\nat = [0, 0]\ntitle = \"{cpu}\\n{obs:scene}\\n{ha:light.desk}\"\n",
        )
        .unwrap();
        let deck = Deck::new(
            profile,
            ".",
            Arc::new(crate::LateOutbox::default()),
            perritodeck_actions::Executor::new(Arc::new(EnigoInput::new()), Arc::new(OpenSystem)),
        );
        let slow = Arc::new(SlowObs {
            answer: tokio::sync::Semaphore::new(0),
            asked: Default::default(),
        });
        let asked = || slow.asked.load(std::sync::atomic::Ordering::Relaxed);
        let mut live = Live::new(deck.clone(), Some(slow.clone()));

        live.round();
        tokio::task::yield_now().await;
        assert!(deck.live_value("cpu").is_some());
        assert_eq!(deck.live_value("ha:light.desk").as_deref(), Some("on"));
        assert_eq!(deck.live_value("obs:scene"), None);
        assert_eq!(asked(), 1);

        deck.set_live([("cpu".to_owned(), "old".to_owned())]);
        live.round();
        tokio::task::yield_now().await;
        assert_ne!(deck.live_value("cpu").as_deref(), Some("old"));
        assert_eq!(asked(), 1, "OBS is still answering the first round");

        slow.answer.add_permits(1);
        for _ in 0..100 {
            if deck.live_value("obs:scene").is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(deck.live_value("obs:scene").as_deref(), Some("Gameplay"));
        live.round();
        tokio::task::yield_now().await;
        assert_eq!(asked(), 2);
    }

    #[tokio::test]
    async fn the_scene_comes_from_either_field() {
        // An obs-websocket without the deprecated field.
        let newer = Fake(json!({ "sceneName": "Just Chatting", "sceneUuid": "u" }));
        assert_eq!(obs(Some(&newer), "scene").await, "Just Chatting");
        let neither = Fake(json!({}));
        assert_eq!(obs(Some(&neither), "scene").await, UNKNOWN);
    }
}
