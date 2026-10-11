//! Sliders: a drag on the tablet sets a level on the computer (volume, a light's
//! brightness, an OBS source's volume) or runs a program with the value.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{ActionError, Executor, Outcome};

/// What a slider controls. `slider = { type = "volume" }` in a profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SliderSpec {
    /// The computer's main output volume.
    Volume,
    /// A light's brightness in Home Assistant (`light.turn_on` with `brightness_pct`).
    HomeAssistant { entity: String },
    /// An audio source's volume in OBS, e.g. "Mic/Aux".
    Obs { input: String },
    /// Run a program, with `{value}` (0–100) replaced in its arguments. Never through a shell.
    Command {
        program: String,
        #[serde(default)]
        args: Vec<String>,
    },
}

impl SliderSpec {
    /// A short description for the editor, e.g. "volume", "OBS Mic/Aux".
    pub fn describe(&self) -> String {
        match self {
            SliderSpec::Volume => "volume".into(),
            SliderSpec::HomeAssistant { entity } => format!("brightness of {entity}"),
            SliderSpec::Obs { input } => format!("OBS volume of {input}"),
            SliderSpec::Command { program, .. } => format!("runs {program}"),
        }
    }
}

/// OBS volumes are a multiplier; squaring makes the slider feel even (loudness is roughly
/// logarithmic).
fn obs_mul(value: u8) -> f64 {
    (f64::from(value.min(100)) / 100.0).powi(2)
}

fn from_obs_mul(mul: f64) -> u8 {
    (mul.clamp(0.0, 1.0).sqrt() * 100.0).round() as u8
}

impl Executor {
    /// Set what the slider controls to `value` (0–100).
    pub async fn set_slider(&self, spec: &SliderSpec, value: u8) -> Result<Outcome, ActionError> {
        let value = value.min(100);
        match spec {
            SliderSpec::Volume => {
                let system = self.system.clone();
                tokio::task::spawn_blocking(move || system.set_volume(value))
                    .await
                    .map_err(|e| ActionError::Input(e.to_string()))??;
                Ok(Outcome::Ok)
            }
            SliderSpec::HomeAssistant { entity } => {
                let data = json!({ "brightness_pct": value });
                self.integrations("Home Assistant")?
                    .home_assistant("light.turn_on", Some(entity), Some(&data))
                    .await
            }
            SliderSpec::Obs { input } => {
                let data = json!({ "inputName": input, "inputVolumeMul": obs_mul(value) });
                self.integrations("OBS")?
                    .obs_request("SetInputVolume", data)
                    .await?;
                Ok(Outcome::Ok)
            }
            SliderSpec::Command { program, args } => {
                let args: Vec<String> = args
                    .iter()
                    .map(|a| a.replace("{value}", &value.to_string()))
                    .collect();
                crate::run_command(program, &args, None, None).await
            }
        }
    }

    /// Where the slider should sit now, if the computer can say.
    pub async fn read_slider(&self, spec: &SliderSpec) -> Option<u8> {
        match spec {
            SliderSpec::Volume => {
                let system = self.system.clone();
                tokio::task::spawn_blocking(move || system.volume())
                    .await
                    .ok()
                    .flatten()
            }
            SliderSpec::HomeAssistant { entity } => {
                let state = self.integrations.as_ref()?.ha_state(entity).await.ok()?;
                if state["state"] == "off" {
                    return Some(0);
                }
                let b = state["attributes"]["brightness"].as_f64()?;
                Some((b / 255.0 * 100.0).round().clamp(0.0, 100.0) as u8)
            }
            SliderSpec::Obs { input } => {
                let resp = self
                    .integrations
                    .as_ref()?
                    .obs_request("GetInputVolume", json!({ "inputName": input }))
                    .await
                    .ok()?;
                Some(from_obs_mul(resp["inputVolumeMul"].as_f64()?))
            }
            SliderSpec::Command { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_read_from_toml_style_json() {
        let s: SliderSpec = serde_json::from_str(r#"{"type":"volume"}"#).unwrap();
        assert_eq!(s, SliderSpec::Volume);
        let s: SliderSpec = serde_json::from_str(r#"{"type":"obs","input":"Mic/Aux"}"#).unwrap();
        assert_eq!(s.describe(), "OBS volume of Mic/Aux");
        assert!(serde_json::from_str::<SliderSpec>(r#"{"type":"brightness"}"#).is_err());
        assert!(serde_json::from_str::<SliderSpec>(r#"{"type":"obs","input":"a","x":1}"#).is_err());
    }

    #[test]
    fn obs_volume_mapping_round_trips() {
        for v in [0u8, 1, 25, 50, 99, 100] {
            assert_eq!(from_obs_mul(obs_mul(v)), v);
        }
        assert_eq!(obs_mul(200), 1.0);
    }
}
