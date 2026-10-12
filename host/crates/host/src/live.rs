//! Keeps live values on keys current: `{cpu}`, `{mem}`, `{ha:<entity>}` (state and
//! unit, or `{ha:<entity>.<attribute>}`), `{obs:recording}`, `{obs:streaming}` and
//! `{obs:scene}`. Only what some profile uses is fetched.

use std::{sync::Arc, time::Duration};

use perritodeck_actions::Integrations;
use perritodeck_deck::Deck;
use serde_json::{json, Value};

const EVERY: Duration = Duration::from_secs(2);
/// Shown when a value can't be fetched (Home Assistant down, OBS closed).
pub(crate) const UNKNOWN: &str = "–";

pub fn spawn(deck: Arc<Deck>, integrations: Option<Arc<dyn Integrations>>) {
    tokio::spawn(async move {
        let mut sys = sysinfo::System::new();
        let mut every = tokio::time::interval(EVERY);
        loop {
            every.tick().await;
            let tokens = deck.live_tokens();
            if tokens.is_empty() {
                continue;
            }
            let mut values = Vec::new();
            if tokens.iter().any(|t| t == "cpu") {
                sys.refresh_cpu_usage();
                values.push(("cpu".to_owned(), format!("{:.0}%", sys.global_cpu_usage())));
            }
            if tokens.iter().any(|t| t == "mem") {
                sys.refresh_memory();
                let pct = sys.used_memory() as f64 / sys.total_memory().max(1) as f64 * 100.0;
                values.push(("mem".to_owned(), format!("{pct:.0}%")));
            }
            for t in &tokens {
                let value = if let Some(entity) = t.strip_prefix("ha:") {
                    Some(home_assistant(integrations.as_deref(), entity).await)
                } else if let Some(what) = t.strip_prefix("obs:") {
                    Some(obs(integrations.as_deref(), what).await)
                } else {
                    None
                };
                if let Some(v) = value {
                    values.push((t.clone(), v));
                }
            }
            deck.set_live(values);
        }
    });
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
    let (request, field) = match what {
        "recording" => ("GetRecordStatus", "outputActive"),
        "streaming" => ("GetStreamStatus", "outputActive"),
        "scene" => ("GetCurrentProgramScene", "currentProgramSceneName"),
        _ => return UNKNOWN.into(),
    };
    match obs.obs_request(request, json!({})).await {
        Ok(resp) => match &resp[field] {
            Value::Bool(true) => "on".into(),
            Value::Bool(false) => "off".into(),
            v => display(v),
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
    use perritodeck_actions::{ActionError, BoxFuture, ObsAction, Outcome};

    struct Fake;
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
                    "GetCurrentProgramScene" => json!({ "currentProgramSceneName": "Gameplay" }),
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
        let f: &dyn Integrations = &Fake;
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
}
