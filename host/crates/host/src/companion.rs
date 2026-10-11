//! Connects the deck to Bitfocus Companion: tablets showing a profile with
//! `companion = true` become Companion surfaces (see `perritodeck-companion`).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use perritodeck_companion::{Client, Event, Status, Surface};
use perritodeck_deck::{Deck, RemoteKey, RemoteSurfaces};
use perritodeck_protocol::Grid;

/// What the Settings page shows about Companion.
pub type CompanionState = Arc<Mutex<String>>;

/// A tablet's name from its id, for Companion's Surfaces page.
pub type TabletNames = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

pub struct Bridge {
    client: Client,
    /// Satellite surface id → tablet id.
    tablets: Arc<Mutex<HashMap<String, String>>>,
    names: TabletNames,
}

/// Companion's id for a tablet: letters, digits, `-` and `_` only.
pub fn surface_id(device_id: &str) -> String {
    let clean: String = device_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(32)
        .collect();
    format!("perritodeck-{clean}")
}

impl Bridge {
    /// Start talking to Companion at `host:port` on behalf of `deck`.
    pub fn start(
        host: String,
        port: u16,
        deck: Weak<Deck>,
        names: TabletNames,
        state: CompanionState,
    ) -> Arc<Self> {
        let tablets: Arc<Mutex<HashMap<String, String>>> = Arc::default();
        let map = tablets.clone();
        let client = Client::start(
            host,
            port,
            Arc::new(move |event| {
                let Some(deck) = deck.upgrade() else { return };
                match event {
                    Event::Status(status) => {
                        let text = match &status {
                            Status::Connecting => "Connecting…".to_owned(),
                            Status::Connected { companion_version } => {
                                format!("Connected to Companion {companion_version}")
                            }
                            Status::Disconnected { reason } => reason.clone(),
                        };
                        *state.lock().expect("companion state poisoned") = text;
                        match status {
                            Status::Connecting => {}
                            Status::Connected { .. } => deck.remote_status(None),
                            Status::Disconnected { .. } => {
                                deck.remote_status(Some("Companion\noffline"))
                            }
                        }
                    }
                    Event::Key {
                        surface,
                        key,
                        state,
                    } => {
                        let tablet = map.lock().expect("tablets poisoned").get(&surface).cloned();
                        if let Some(tablet) = tablet {
                            deck.remote_key(
                                &tablet,
                                key,
                                RemoteKey {
                                    png: state.png,
                                    title: state.text.unwrap_or_default(),
                                    bg: state.color,
                                    fg: state.text_color,
                                },
                            );
                        }
                    }
                    Event::Clear { surface } => {
                        let tablet = map.lock().expect("tablets poisoned").get(&surface).cloned();
                        if let Some(tablet) = tablet {
                            deck.remote_clear(&tablet);
                        }
                    }
                }
            }),
        );
        Arc::new(Self {
            client,
            tablets,
            names,
        })
    }
}

impl RemoteSurfaces for Bridge {
    fn attach(&self, device_id: &str, grid: Grid, image_px: Option<u32>) {
        let id = surface_id(device_id);
        self.tablets
            .lock()
            .expect("tablets poisoned")
            .insert(id.clone(), device_id.to_owned());
        let name = (self.names)(device_id).unwrap_or_else(|| "tablet".into());
        self.client.add(Surface {
            id,
            name: format!("PerritoDeck {name}"),
            cols: grid.cols,
            rows: grid.rows,
            bitmap_px: image_px,
        });
    }

    fn detach(&self, device_id: &str) {
        let id = surface_id(device_id);
        self.tablets.lock().expect("tablets poisoned").remove(&id);
        self.client.remove(&id);
    }

    fn press(&self, device_id: &str, key_index: u32, down: bool) {
        self.client.press(&surface_id(device_id), key_index, down);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn surface_ids_are_safe() {
        assert_eq!(super::surface_id("aB3_-x/+=="), "perritodeck-aB3_-x");
    }
}
