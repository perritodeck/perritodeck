//! Which actions run programs, wherever they sit inside another action. MCP's "run
//! actions" permission and the profile import check both use this one list.

use serde::Serialize;

use crate::{ActionSpec, SliderSpec};

/// Where an action sits inside another, counted from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "in", rename_all = "camelCase")]
pub enum Place {
    /// Step `n` of a several-step action with `of` steps.
    Step { n: usize, of: usize },
    /// Choice `n` of a random action with `of` choices.
    Choice { n: usize, of: usize },
}

impl ActionSpec {
    /// Starts a program, opens a file with its app, or hands the press to a plugin: what
    /// MCP's "run actions" permission and the import check guard. False for multi and
    /// random; look inside them with [`ActionSpec::leaves`].
    pub fn runs_programs(&self) -> bool {
        // No catch-all arm: a new action type has to be sorted here.
        match self {
            ActionSpec::Command { .. }
            | ActionSpec::Launch { .. }
            | ActionSpec::OpenPath { .. }
            | ActionSpec::Plugin { .. }
            | ActionSpec::SdPlugin { .. } => true,
            ActionSpec::Hotkey { .. }
            | ActionSpec::Text { .. }
            | ActionSpec::Media { .. }
            | ActionSpec::OpenUrl { .. }
            | ActionSpec::Multi { .. }
            | ActionSpec::Random { .. }
            | ActionSpec::Obs(_)
            | ActionSpec::HomeAssistant { .. }
            | ActionSpec::None => false,
        }
    }

    /// This action, or every action inside it (multi steps, random choices, any depth),
    /// in order, each with where it sits. Never a multi or random itself. Walks with a
    /// stack of its own, so even a file nested far deeper than validation allows can't
    /// overflow the thread's stack.
    pub fn leaves(&self) -> Vec<(Vec<Place>, &ActionSpec)> {
        let mut out = Vec::new();
        let mut stack = vec![(Vec::new(), self)];
        while let Some((place, a)) = stack.pop() {
            // Pushed last to first, so they come off the stack in order.
            let inside: Vec<(Place, &ActionSpec)> = match a {
                ActionSpec::Multi { steps } => steps
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        let p = Place::Step {
                            n: i + 1,
                            of: steps.len(),
                        };
                        (p, &s.action)
                    })
                    .collect(),
                ActionSpec::Random { actions } => actions
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        let p = Place::Choice {
                            n: i + 1,
                            of: actions.len(),
                        };
                        (p, a)
                    })
                    .collect(),
                _ => {
                    out.push((place, a));
                    continue;
                }
            };
            for (p, inner) in inside.into_iter().rev() {
                let mut at = place.clone();
                at.push(p);
                stack.push((at, inner));
            }
        }
        out
    }
}

impl SliderSpec {
    /// Runs a program or a plugin action on every move.
    pub fn runs_programs(&self) -> bool {
        match self {
            SliderSpec::Command { .. } | SliderSpec::Plugin { .. } => true,
            SliderSpec::Volume | SliderSpec::HomeAssistant { .. } | SliderSpec::Obs { .. } => false,
        }
    }
}
