//! The deck runtime: shows a [`Profile`] on every connected tablet and turns key
//! presses into actions, page changes and toggles.
//!
//! Several profiles can be loaded at once; each device shows one of them (the default
//! unless assigned another). Each device has its own current page (and Back history).
//! Toggle states belong to the profile, so a "Mic muted" key looks the same on every
//! tablet showing it.

mod templates;
pub mod watch;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use perritodeck_actions::{ActionError, ActionSpec, Executor, Outcome};
use perritodeck_core::{demo, ServerHandle};
use perritodeck_profile::{KeyDef, Look, NavSpec, Profile, SimpleNav, DEFAULT_FG, EMPTY_BG};
use perritodeck_protocol::{
    Grid, Hello, KeyFeedback, KeySpec, Layout, Message, Screen, SetImage, SetTitle,
};
use tracing::{info, warn};

pub use templates::render_title;

/// Where the runtime sends messages (the server in production, a recorder in tests).
pub trait Outbox: Send + Sync {
    fn send(&self, device_id: &str, msg: Message) -> bool;
}

impl Outbox for ServerHandle {
    fn send(&self, device_id: &str, msg: Message) -> bool {
        ServerHandle::send(self, device_id, msg)
    }
}

struct DeviceView {
    /// Which profile this device is showing.
    profile: String,
    grid: Grid,
    screen: Screen,
    /// Key image size in pixels, if the device wants host-rendered images.
    image_px: Option<u32>,
    page: usize,
    back: Vec<usize>,
    /// What each key showed last time, to send only changes.
    sent: HashMap<u32, KeySpec>,
}

/// One loaded profile and everything that belongs to it.
struct Slot {
    profile: Profile,
    base_dir: PathBuf,
    /// Toggle state per (page, position).
    toggles: HashMap<(usize, [u32; 2]), usize>,
    icons: HashMap<PathBuf, Option<Vec<u8>>>,
    /// Why the last edit to the profile file was rejected, if it was.
    reload_error: Option<String>,
}

impl Slot {
    fn new(profile: Profile, base_dir: PathBuf) -> Self {
        Self {
            profile,
            base_dir,
            toggles: HashMap::new(),
            icons: HashMap::new(),
            reload_error: None,
        }
    }
}

struct State {
    slots: HashMap<String, Slot>,
    /// The profile devices show unless told otherwise.
    default: String,
    /// Device id → profile id, for devices that show something else.
    assignments: HashMap<String, String>,
    devices: HashMap<String, DeviceView>,
}

impl State {
    /// The profile a device should show (falls back to the default).
    fn profile_for(&self, device_id: &str) -> String {
        self.assignments
            .get(device_id)
            .filter(|id| self.slots.contains_key(*id))
            .cloned()
            .unwrap_or_else(|| self.default.clone())
    }

    fn slot(&self, id: &str) -> &Slot {
        self.slots
            .get(id)
            .or_else(|| self.slots.get(&self.default))
            .expect("the default profile is always loaded")
    }

    fn slot_mut(&mut self, id: &str) -> &mut Slot {
        let key = if self.slots.contains_key(id) {
            id.to_owned()
        } else {
            self.default.clone()
        };
        self.slots
            .get_mut(&key)
            .expect("the default profile is always loaded")
    }
}

/// The id of the profile a single-profile deck starts with.
pub const DEFAULT_PROFILE: &str = "default";

pub struct Deck {
    state: Mutex<State>,
    outbox: Arc<dyn Outbox>,
    executor: Executor,
    revision: std::sync::atomic::AtomicU64,
    paused: std::sync::atomic::AtomicBool,
}

/// Default grid when the profile doesn't fix one: sized to the device's screen.
fn device_grid(profile: &Profile, hello_grid: Grid) -> Grid {
    match profile.grid {
        Some(g) => Grid {
            cols: g.cols,
            rows: g.rows,
        },
        None => hello_grid,
    }
}

impl Deck {
    /// A deck with one profile, called [`DEFAULT_PROFILE`].
    pub fn new(
        profile: Profile,
        base_dir: impl Into<PathBuf>,
        outbox: Arc<dyn Outbox>,
        executor: Executor,
    ) -> Arc<Self> {
        Self::with_default(DEFAULT_PROFILE, profile, base_dir, outbox, executor)
    }

    /// A deck whose default profile has the id `default_id`.
    pub fn with_default(
        default_id: &str,
        profile: Profile,
        base_dir: impl Into<PathBuf>,
        outbox: Arc<dyn Outbox>,
        executor: Executor,
    ) -> Arc<Self> {
        let mut slots = HashMap::new();
        slots.insert(default_id.to_owned(), Slot::new(profile, base_dir.into()));
        Arc::new(Self {
            state: Mutex::new(State {
                slots,
                default: default_id.to_owned(),
                assignments: HashMap::new(),
                devices: HashMap::new(),
            }),
            outbox,
            executor,
            revision: Default::default(),
            paused: Default::default(),
        })
    }

    /// Goes up whenever a profile is loaded, reloaded, removed or fails to reload, or a
    /// device switches profile, so UIs can refresh.
    pub fn revision(&self) -> u64 {
        self.revision.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn bump(&self) {
        self.revision
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("deck state poisoned")
    }

    /// Stop (or resume) running anything when keys are pressed.
    pub fn set_paused(&self, paused: bool) {
        self.paused
            .store(paused, std::sync::atomic::Ordering::Relaxed);
        self.bump();
    }

    pub fn paused(&self) -> bool {
        self.paused.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The first layout a device sees (used as the server's layout provider).
    pub fn initial_layout(&self, hello: &Hello) -> Layout {
        let st = self.lock();
        let slot = st.slot(&st.profile_for(&hello.device_id));
        let grid = device_grid(&slot.profile, demo::grid_for(hello));
        page_layout(slot, 0, grid)
    }

    /// A device finished its handshake; it already has `initial_layout`.
    pub fn connected(&self, device_id: &str, grid: Grid, screen: Screen, capabilities: &[String]) {
        let mut st = self.lock();
        let image_px = capabilities
            .iter()
            .any(|c| c == "images")
            .then(|| perritodeck_render::key_pixel_size(&screen, grid));
        let profile = st.profile_for(device_id);
        let layout = page_layout(st.slot(&profile), 0, grid);
        let sent = layout.keys.iter().map(|k| (k.index, k.clone())).collect();
        st.devices.insert(
            device_id.to_owned(),
            DeviceView {
                profile,
                grid,
                screen,
                image_px,
                page: 0,
                back: Vec::new(),
                sent,
            },
        );
        if image_px.is_some() {
            self.send_images(&mut st, device_id);
        }
    }

    pub fn disconnected(&self, device_id: &str) {
        self.lock().devices.remove(device_id);
    }

    /// The default profile.
    pub fn profile(&self) -> Profile {
        let st = self.lock();
        st.slot(&st.default).profile.clone()
    }

    pub fn default_profile_id(&self) -> String {
        self.lock().default.clone()
    }

    /// A loaded profile by id.
    pub fn profile_by_id(&self, id: &str) -> Option<Profile> {
        self.lock().slots.get(id).map(|s| s.profile.clone())
    }

    /// Ids of every loaded profile, sorted.
    pub fn profile_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.lock().slots.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Why the last edit to the default profile's file was rejected (cleared by a good reload).
    pub fn reload_error(&self) -> Option<String> {
        let st = self.lock();
        st.slot(&st.default).reload_error.clone()
    }

    pub fn reload_error_of(&self, id: &str) -> Option<String> {
        self.lock()
            .slots
            .get(id)
            .and_then(|s| s.reload_error.clone())
    }

    pub fn set_reload_error(&self, error: String) {
        let id = self.default_profile_id();
        self.set_reload_error_of(&id, error);
    }

    pub fn set_reload_error_of(&self, id: &str, error: String) {
        if let Some(s) = self.lock().slots.get_mut(id) {
            s.reload_error = Some(error);
        }
        self.bump();
    }

    /// Swap in a new default profile (after its file changed) and redraw.
    pub fn reload(&self, profile: Profile) {
        let id = self.default_profile_id();
        self.load_profile(&id, profile, None);
    }

    /// Add a profile, or replace it after its file changed, and redraw every device
    /// showing it. `base_dir` (for icons) defaults to the existing one.
    pub fn load_profile(&self, id: &str, profile: Profile, base_dir: Option<PathBuf>) {
        self.bump();
        let mut st = self.lock();
        let pages = profile.pages.len();
        match st.slots.get_mut(id) {
            Some(slot) => {
                slot.reload_error = None;
                slot.profile = profile;
                slot.icons.clear();
                slot.toggles.retain(|(page, _), _| *page < pages);
                if let Some(dir) = base_dir {
                    slot.base_dir = dir;
                }
            }
            None => {
                st.slots.insert(
                    id.to_owned(),
                    Slot::new(profile, base_dir.unwrap_or_default()),
                );
            }
        }
        let fixed_grid = st.slots[id].profile.grid;
        let ids: Vec<String> = st
            .devices
            .iter()
            .filter(|(_, d)| d.profile == id)
            .map(|(k, _)| k.clone())
            .collect();
        for dev in ids {
            if let Some(d) = st.devices.get_mut(&dev) {
                if let Some(g) = fixed_grid {
                    d.grid = Grid {
                        cols: g.cols,
                        rows: g.rows,
                    };
                    if d.image_px.is_some() {
                        d.image_px = Some(perritodeck_render::key_pixel_size(&d.screen, d.grid));
                    }
                }
                if d.page >= pages {
                    d.page = 0;
                    d.back.clear();
                }
                d.back.retain(|p| *p < pages);
            }
            self.show_page(&mut st, &dev);
        }
        info!("profile {id} loaded: {pages} page(s)");
    }

    /// Unload a profile (its file was deleted). Devices showing it go to the default.
    /// The default profile can't be removed.
    pub fn remove_profile(&self, id: &str) -> bool {
        let mut st = self.lock();
        if id == st.default || st.slots.remove(id).is_none() {
            return false;
        }
        st.assignments.retain(|_, p| p != id);
        let default = st.default.clone();
        let moved: Vec<String> = st
            .devices
            .iter()
            .filter(|(_, d)| d.profile == id)
            .map(|(k, _)| k.clone())
            .collect();
        for dev in moved {
            self.switch_device(&mut st, &dev, &default);
        }
        drop(st);
        self.bump();
        true
    }

    /// Which profile a device shows (or will show when it connects).
    pub fn device_profile(&self, device_id: &str) -> String {
        let st = self.lock();
        st.devices
            .get(device_id)
            .map(|d| d.profile.clone())
            .unwrap_or_else(|| st.profile_for(device_id))
    }

    /// Show `profile_id` on a device from now on (and next time it connects).
    /// `None` returns it to the default profile. False if there's no such profile.
    pub fn assign(&self, device_id: &str, profile_id: Option<&str>) -> bool {
        let mut st = self.lock();
        match profile_id {
            Some(id) if !st.slots.contains_key(id) => return false,
            Some(id) => {
                st.assignments.insert(device_id.to_owned(), id.to_owned());
            }
            None => {
                st.assignments.remove(device_id);
            }
        }
        let target = st.profile_for(device_id);
        self.switch_device(&mut st, device_id, &target);
        drop(st);
        self.bump();
        true
    }

    /// Show a profile on a connected device for now, without changing its assignment
    /// (used by auto-switching). False if there's no such profile or device.
    pub fn show_profile(&self, device_id: &str, profile_id: &str) -> bool {
        let mut st = self.lock();
        if !st.slots.contains_key(profile_id) || !st.devices.contains_key(device_id) {
            return false;
        }
        self.switch_device(&mut st, device_id, profile_id);
        true
    }

    /// Device ids connected right now, with the profile each shows.
    pub fn connected_devices(&self) -> Vec<(String, String)> {
        self.lock()
            .devices
            .iter()
            .map(|(id, d)| (id.clone(), d.profile.clone()))
            .collect()
    }

    fn switch_device(&self, st: &mut State, device_id: &str, profile_id: &str) {
        let Some(d) = st.devices.get(device_id) else {
            return;
        };
        if d.profile == profile_id {
            return;
        }
        let fixed = st.slot(profile_id).profile.grid;
        if let Some(d) = st.devices.get_mut(device_id) {
            d.profile = profile_id.to_owned();
            d.page = 0;
            d.back.clear();
            if let Some(g) = fixed {
                d.grid = Grid {
                    cols: g.cols,
                    rows: g.rows,
                };
                if d.image_px.is_some() {
                    d.image_px = Some(perritodeck_render::key_pixel_size(&d.screen, d.grid));
                }
            }
        }
        info!(device = device_id, "showing profile {profile_id}");
        self.show_page(st, device_id);
    }

    /// Refresh keys whose titles contain live values like `{time}`.
    pub fn tick(&self) {
        let mut st = self.lock();
        let ids: Vec<String> = st.devices.keys().cloned().collect();
        for id in ids {
            let (profile, page, grid) = match st.devices.get(&id) {
                Some(d) => (d.profile.clone(), d.page, d.grid),
                None => continue,
            };
            let indexes: Vec<u32> = st
                .slot(&profile)
                .profile
                .pages
                .get(page)
                .map(|p| {
                    p.keys
                        .iter()
                        .filter(|k| in_grid(k, grid) && has_template(k))
                        .map(|k| k.at[0] * grid.cols + k.at[1])
                        .collect()
                })
                .unwrap_or_default();
            for index in indexes {
                self.refresh_key(&mut st, &id, index);
            }
        }
    }

    /// A key went down on a device.
    pub fn key_down(self: &Arc<Self>, device_id: &str, key_index: u32) {
        let mut st = self.lock();
        let Some(d) = st.devices.get(device_id) else {
            return;
        };
        let (profile, page, grid) = (d.profile.clone(), d.page, d.grid);
        let at = [key_index / grid.cols.max(1), key_index % grid.cols.max(1)];
        let Some(key) = st
            .slot(&profile)
            .profile
            .pages
            .get(page)
            .and_then(|p| p.keys.iter().find(|k| k.at == at))
            .cloned()
        else {
            return;
        };

        if self.paused() {
            self.outbox.send(
                device_id,
                Message::ShowAlert(KeyFeedback {
                    key_index,
                    message: Some("Keys are paused on the computer".into()),
                }),
            );
            return;
        }

        if let Some(nav) = &key.nav {
            let mut d = st.devices.remove(device_id).expect("checked above");
            d.page = navigate(&st.slot(&profile).profile, &mut d, page, nav);
            st.devices.insert(device_id.to_owned(), d);
            self.show_page(&mut st, device_id);
            return;
        }

        let slot = st.slot(&profile);
        let state = slot.toggles.get(&(page, at)).copied().unwrap_or(0);
        let action = key.action(state).cloned();
        if key.states.len() >= 2 {
            st.slot_mut(&profile)
                .toggles
                .insert((page, at), (state + 1) % key.states.len());
            // Every device showing this page of this profile sees the new state.
            let showing: Vec<(String, u32)> = st
                .devices
                .iter()
                .filter(|(_, d)| d.profile == profile && d.page == page && in_grid(&key, d.grid))
                .map(|(id, d)| (id.clone(), at[0] * d.grid.cols + at[1]))
                .collect();
            for (id, idx) in showing {
                self.refresh_key(&mut st, &id, idx);
            }
        }
        drop(st);

        if let Some(action) = action {
            let deck = Arc::clone(self);
            let device = device_id.to_owned();
            tokio::spawn(async move {
                let result = deck.executor.run(&action).await;
                deck.report(&device, key_index, &action, result);
            });
        }
    }

    fn report(
        &self,
        device_id: &str,
        key_index: u32,
        action: &ActionSpec,
        result: Result<Outcome, ActionError>,
    ) {
        let feedback = |message: Option<String>| KeyFeedback { key_index, message };
        match result {
            Ok(Outcome::Ok) => {
                // Commands can take a while, so confirm they finished; instant actions don't need it.
                if matches!(
                    action,
                    ActionSpec::Command { .. } | ActionSpec::Multi { .. }
                ) {
                    self.outbox.send(device_id, Message::ShowOk(feedback(None)));
                }
            }
            Ok(Outcome::Alert(m)) => {
                warn!(
                    device = device_id,
                    key = key_index + 1,
                    "action reported a problem: {m}"
                );
                self.outbox
                    .send(device_id, Message::ShowAlert(feedback(Some(m))));
            }
            Err(e) => {
                warn!(
                    device = device_id,
                    key = key_index + 1,
                    "action failed: {e}"
                );
                self.outbox
                    .send(device_id, Message::ShowAlert(feedback(Some(e.to_string()))));
            }
        }
    }

    /// Send the device's current page in full.
    fn show_page(&self, st: &mut State, device_id: &str) {
        let Some(d) = st.devices.get(device_id) else {
            return;
        };
        let layout = page_layout(st.slot(&d.profile), d.page, d.grid);
        let sent = layout.keys.iter().map(|k| (k.index, k.clone())).collect();
        self.outbox.send(device_id, Message::Layout(layout));
        let images = d.image_px.is_some();
        if let Some(d) = st.devices.get_mut(device_id) {
            d.sent = sent;
        }
        if images {
            self.send_images(st, device_id);
        }
    }

    fn send_images(&self, st: &mut State, device_id: &str) {
        let Some(d) = st.devices.get(device_id) else {
            return;
        };
        let (profile, page, grid, px) =
            (d.profile.clone(), d.page, d.grid, d.image_px.unwrap_or(0));
        let slot = st.slot_mut(&profile);
        for index in 0..grid.key_count() {
            if let Some(url) = key_image(slot, page, grid, index, px) {
                self.outbox.send(
                    device_id,
                    Message::SetImage(SetImage {
                        key_index: index,
                        image: Some(url),
                    }),
                );
            }
        }
    }

    /// Re-send one key if its look changed.
    fn refresh_key(&self, st: &mut State, device_id: &str, index: u32) {
        let Some(d) = st.devices.get(device_id) else {
            return;
        };
        let (profile, page, grid, image_px) = (d.profile.clone(), d.page, d.grid, d.image_px);
        let spec = key_spec(st.slot(&profile), page, grid, index);
        let previous = d.sent.get(&index).cloned();
        if previous.as_ref() == Some(&spec) {
            return;
        }
        let colours_changed = previous
            .as_ref()
            .is_some_and(|p| p.bg_color != spec.bg_color || p.text_color != spec.text_color);
        if let Some(d) = st.devices.get_mut(device_id) {
            d.sent.insert(index, spec.clone());
        }
        if let Some(px) = image_px {
            self.outbox.send(
                device_id,
                Message::SetTitle(SetTitle {
                    key_index: index,
                    title: spec.title,
                }),
            );
            if let Some(url) = key_image(st.slot_mut(&profile), page, grid, index, px) {
                self.outbox.send(
                    device_id,
                    Message::SetImage(SetImage {
                        key_index: index,
                        image: Some(url),
                    }),
                );
            }
        } else if colours_changed {
            // Text-only clients can't recolour one key, so resend the page.
            self.show_page(st, device_id);
        } else {
            self.outbox.send(
                device_id,
                Message::SetTitle(SetTitle {
                    key_index: index,
                    title: spec.title,
                }),
            );
        }
    }
}

fn in_grid(key: &KeyDef, grid: Grid) -> bool {
    key.at[0] < grid.rows && key.at[1] < grid.cols
}

fn has_template(key: &KeyDef) -> bool {
    key.title.as_deref().is_some_and(|t| t.contains('{'))
        || key
            .states
            .iter()
            .any(|s| s.title.as_deref().is_some_and(|t| t.contains('{')))
}

fn navigate(profile: &Profile, d: &mut DeviceView, page: usize, nav: &NavSpec) -> usize {
    let n = profile.pages.len();
    match nav {
        NavSpec::Simple(SimpleNav::Back) => d.back.pop().unwrap_or(0),
        NavSpec::Simple(SimpleNav::Next) => (page + 1) % n,
        NavSpec::Simple(SimpleNav::Previous) => (page + n - 1) % n,
        NavSpec::Page { page: name } => match profile.page_index(name) {
            Some(target) if target != page => {
                d.back.push(page);
                target
            }
            _ => page,
        },
    }
}

fn look_at(st: &Slot, page: usize, grid: Grid, index: u32) -> Option<(Look, [u32; 2])> {
    let at = [index / grid.cols.max(1), index % grid.cols.max(1)];
    let key = st
        .profile
        .pages
        .get(page)?
        .keys
        .iter()
        .find(|k| k.at == at)?;
    let state = st.toggles.get(&(page, at)).copied().unwrap_or(0);
    Some((key.look(state), at))
}

fn key_spec(st: &Slot, page: usize, grid: Grid, index: u32) -> KeySpec {
    match look_at(st, page, grid, index) {
        Some((look, _)) => KeySpec {
            index,
            title: render_title(&look.title, chrono::Local::now()),
            bg_color: look.bg,
            text_color: look.fg,
        },
        None => KeySpec {
            index,
            title: String::new(),
            bg_color: EMPTY_BG.to_owned(),
            text_color: DEFAULT_FG.to_owned(),
        },
    }
}

fn page_layout(st: &Slot, page: usize, grid: Grid) -> Layout {
    Layout {
        cols: grid.cols,
        rows: grid.rows,
        keys: (0..grid.key_count())
            .map(|i| key_spec(st, page, grid, i))
            .collect(),
    }
}

fn icon_bytes(st: &mut Slot, path: &Path) -> Option<Vec<u8>> {
    let full = st.base_dir.join(path);
    st.icons
        .entry(full.clone())
        .or_insert_with(|| match std::fs::read(&full) {
            Ok(b) => Some(b),
            Err(e) => {
                warn!("icon {} can't be read: {e}", full.display());
                None
            }
        })
        .clone()
}

/// Render one key as a PNG data URL. `None` for empty positions (the client's
/// background is enough).
fn key_image(st: &mut Slot, page: usize, grid: Grid, index: u32, px: u32) -> Option<String> {
    let (look, _) = look_at(st, page, grid, index)?;
    let icon_png = look.icon.as_deref().and_then(|p| icon_bytes(st, p));
    let style = perritodeck_render::KeyStyle {
        title: render_title(&look.title, chrono::Local::now()),
        background: perritodeck_render::parse_hex(&look.bg).unwrap_or([0x1E, 0x1E, 0x2E]),
        text: perritodeck_render::parse_hex(&look.fg).unwrap_or([0xCD, 0xD6, 0xF4]),
        icon_png,
    };
    Some(perritodeck_render::data_url(
        &perritodeck_render::render_png(&style, px),
    ))
}

#[cfg(test)]
mod tests;
