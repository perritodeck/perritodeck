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

use perritodeck_actions::{ActionError, ActionSpec, Executor, Outcome, SliderSpec};
use perritodeck_core::{demo, ServerHandle};
use perritodeck_profile::{KeyDef, Look, NavSpec, Profile, SimpleNav, DEFAULT_FG, EMPTY_BG};
use perritodeck_protocol::{
    Control, Grid, Hello, KeyFeedback, KeySpec, Layout, Message, Screen, SetImage, SetTitle,
    SliderValue,
};
use perritodeck_render::Animation;
use tracing::{info, warn};

pub use templates::{render_title, tokens, truthy};

/// Values for `{name}`s in titles (and toggles that follow them), kept up to date by
/// the host: `cpu`, `mem`, `ha:<entity>`, `obs:recording`, …
pub type LiveValues = Arc<std::sync::RwLock<HashMap<String, String>>>;

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
    /// The device draws sliders itself.
    sliders: bool,
    page: usize,
    back: Vec<usize>,
    /// What each key showed last time, to send only changes.
    sent: HashMap<u32, KeySpec>,
    /// Which frame each animated key showed last.
    frames_sent: HashMap<u32, usize>,
}

/// One loaded profile and everything that belongs to it.
struct Slot {
    profile: Profile,
    base_dir: PathBuf,
    /// Toggle state per (page, position).
    toggles: HashMap<(usize, [u32; 2]), usize>,
    /// Last known slider value per (page, position).
    slider_values: HashMap<(usize, [u32; 2]), u8>,
    icons: HashMap<PathBuf, Option<Vec<u8>>>,
    /// Decoded animated icons (`None`: the icon isn't animated).
    animations: HashMap<PathBuf, Option<Arc<Animation>>>,
    /// Every frame of an animated key, rendered, so animating costs no drawing.
    frames: HashMap<FramesKey, Arc<Vec<String>>>,
    /// Why the last edit to the profile file was rejected, if it was.
    reload_error: Option<String>,
    live: LiveValues,
}

impl Slot {
    fn new(profile: Profile, base_dir: PathBuf, live: LiveValues) -> Self {
        Self {
            live,
            profile,
            base_dir,
            toggles: HashMap::new(),
            slider_values: HashMap::new(),
            icons: HashMap::new(),
            animations: HashMap::new(),
            frames: HashMap::new(),
            reload_error: None,
        }
    }
}

/// Icon, size and the rest of the key's look: what an animated key's frames depend on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FramesKey {
    icon: PathBuf,
    px: u32,
    title: String,
    bg: String,
    fg: String,
}

/// Rendered animations kept per profile; more than this and the cache starts over.
const MAX_CACHED_ANIMATIONS: usize = 32;

/// A slider's position before anything is known about it.
const SLIDER_START: u8 = 50;

/// Values waiting to be applied, per slider, so a fast drag runs the action for the
/// latest value instead of queueing every step.
#[derive(Default)]
struct SliderQueue {
    pending: HashMap<String, u8>,
    running: std::collections::HashSet<String>,
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
    sliders: Mutex<SliderQueue>,
    live: LiveValues,
    started: std::time::Instant,
    /// Tests: a fixed time in ms since `started` instead of the clock (`u64::MAX` = clock).
    fake_ms: std::sync::atomic::AtomicU64,
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
        let live = LiveValues::default();
        let mut slots = HashMap::new();
        slots.insert(
            default_id.to_owned(),
            Slot::new(profile, base_dir.into(), live.clone()),
        );
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
            sliders: Default::default(),
            live,
            started: std::time::Instant::now(),
            fake_ms: std::sync::atomic::AtomicU64::new(u64::MAX),
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
        page_layout(
            slot,
            0,
            grid,
            hello.capabilities.iter().any(|c| c == "sliders"),
        )
    }

    /// A device finished its handshake; it already has `initial_layout`.
    pub fn connected(&self, device_id: &str, grid: Grid, screen: Screen, capabilities: &[String]) {
        let mut st = self.lock();
        let image_px = capabilities
            .iter()
            .any(|c| c == "images")
            .then(|| perritodeck_render::key_pixel_size(&screen, grid));
        let sliders = capabilities.iter().any(|c| c == "sliders");
        let profile = st.profile_for(device_id);
        let layout = page_layout(st.slot(&profile), 0, grid, sliders);
        let sent = layout.keys.iter().map(|k| (k.index, k.clone())).collect();
        st.devices.insert(
            device_id.to_owned(),
            DeviceView {
                profile,
                grid,
                screen,
                image_px,
                sliders,
                page: 0,
                back: Vec::new(),
                sent,
                frames_sent: HashMap::new(),
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
                slot.animations.clear();
                slot.frames.clear();
                slot.toggles.retain(|(page, _), _| *page < pages);
                if let Some(dir) = base_dir {
                    slot.base_dir = dir;
                }
            }
            None => {
                st.slots.insert(
                    id.to_owned(),
                    Slot::new(profile, base_dir.unwrap_or_default(), self.live.clone()),
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

    /// Update live values (`cpu`, `ha:sensor.x`, …). Keys showing them refresh on the
    /// next tick.
    pub fn set_live(&self, values: impl IntoIterator<Item = (String, String)>) {
        let mut live = self.live.write().expect("live values poisoned");
        for (k, v) in values {
            live.insert(k, v);
        }
    }

    /// One live value, if known.
    pub fn live_value(&self, name: &str) -> Option<String> {
        self.live
            .read()
            .expect("live values poisoned")
            .get(name)
            .cloned()
    }

    /// Every live value name used by any loaded profile, in titles or toggles' `follow`.
    pub fn live_tokens(&self) -> Vec<String> {
        let st = self.lock();
        let mut out: Vec<String> = Vec::new();
        for slot in st.slots.values() {
            for key in slot.profile.pages.iter().flat_map(|p| p.keys.iter()) {
                let titles = key
                    .title
                    .iter()
                    .chain(key.states.iter().filter_map(|s| s.title.as_ref()));
                for t in titles {
                    out.extend(tokens(t).map(str::to_owned));
                }
                out.extend(key.follow.iter().cloned());
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// A slider moved on a device: remember the value, show it on other tablets, and
    /// set what it controls (coalescing a fast drag into the latest value).
    pub fn slider_change(self: &Arc<Self>, device_id: &str, key_index: u32, value: u8) {
        let value = value.min(100);
        let mut st = self.lock();
        let Some(d) = st.devices.get(device_id) else {
            return;
        };
        let (profile, page, grid) = (d.profile.clone(), d.page, d.grid);
        let at = [key_index / grid.cols.max(1), key_index % grid.cols.max(1)];
        let Some(spec) = slider_at(st.slot(&profile), page, at).cloned() else {
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
        st.slot_mut(&profile)
            .slider_values
            .insert((page, at), value);
        self.echo_slider(&st, Some(device_id), &profile, page, at, value);
        drop(st);

        let id = format!("{profile}/{page}/{}/{}", at[0], at[1]);
        let mut q = self.sliders.lock().expect("slider queue poisoned");
        q.pending.insert(id.clone(), value);
        if !q.running.insert(id.clone()) {
            return; // the running task will pick up the new value
        }
        drop(q);
        let deck = Arc::clone(self);
        let device = device_id.to_owned();
        tokio::spawn(async move {
            loop {
                let next = {
                    let mut q = deck.sliders.lock().expect("slider queue poisoned");
                    match q.pending.remove(&id) {
                        Some(v) => v,
                        None => {
                            q.running.remove(&id);
                            break;
                        }
                    }
                };
                let alert = match deck.executor.set_slider(&spec, next).await {
                    Ok(Outcome::Ok) => None,
                    Ok(Outcome::Alert(m)) => Some(m),
                    Err(e) => {
                        warn!(device = %device, "slider failed: {e}");
                        Some(e.to_string())
                    }
                };
                if let Some(message) = alert {
                    deck.outbox.send(
                        &device,
                        Message::ShowAlert(KeyFeedback {
                            key_index,
                            message: Some(message),
                        }),
                    );
                }
            }
        });
    }

    /// Send a slider's value to every device showing it, except `skip`.
    fn echo_slider(
        &self,
        st: &State,
        skip: Option<&str>,
        profile: &str,
        page: usize,
        at: [u32; 2],
        value: u8,
    ) {
        for (id, d) in &st.devices {
            if Some(id.as_str()) == skip || d.profile != profile || d.page != page || !d.sliders {
                continue;
            }
            if at[0] < d.grid.rows && at[1] < d.grid.cols {
                let key_index = at[0] * d.grid.cols + at[1];
                self.outbox
                    .send(id, Message::SetValue(SliderValue { key_index, value }));
            }
        }
    }

    /// Read the current level of every slider on screen (volume changed on the
    /// computer, a light dimmed elsewhere) and move the tablets' sliders to match.
    pub async fn poll_sliders(self: &Arc<Self>) {
        let visible: Vec<(String, usize, [u32; 2], SliderSpec)> = {
            let st = self.lock();
            let mut seen = std::collections::HashSet::new();
            let mut out = Vec::new();
            for d in st.devices.values().filter(|d| d.sliders) {
                let Some(p) = st.slot(&d.profile).profile.pages.get(d.page) else {
                    continue;
                };
                for k in &p.keys {
                    if let Some(spec) = &k.slider {
                        if seen.insert((d.profile.clone(), d.page, k.at)) {
                            out.push((d.profile.clone(), d.page, k.at, spec.clone()));
                        }
                    }
                }
            }
            out
        };
        for (profile, page, at, spec) in visible {
            {
                // Don't fight a drag in progress.
                let q = self.sliders.lock().expect("slider queue poisoned");
                if q.running
                    .contains(&format!("{profile}/{page}/{}/{}", at[0], at[1]))
                {
                    continue;
                }
            }
            let Some(value) = self.executor.read_slider(&spec).await else {
                continue;
            };
            let mut st = self.lock();
            let Some(slot) = st.slots.get_mut(&profile) else {
                continue;
            };
            if slot.slider_values.insert((page, at), value) != Some(value) {
                self.echo_slider(&st, None, &profile, page, at, value);
            }
        }
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
        let state = state_of(slot, page, &key);
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
        let layout = page_layout(st.slot(&d.profile), d.page, d.grid, d.sliders);
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
        let (profile, page, grid, px, sliders) = (
            d.profile.clone(),
            d.page,
            d.grid,
            d.image_px.unwrap_or(0),
            d.sliders,
        );
        let now = self.now_ms();
        let mut frames = HashMap::new();
        let slot = st.slot_mut(&profile);
        for index in 0..grid.key_count() {
            if let Some((url, frame)) = key_image(slot, page, grid, index, px, sliders, now) {
                if let Some(f) = frame {
                    frames.insert(index, f);
                }
                self.outbox.send(
                    device_id,
                    Message::SetImage(SetImage {
                        key_index: index,
                        image: Some(url),
                    }),
                );
            }
        }
        if let Some(d) = st.devices.get_mut(device_id) {
            d.frames_sent = frames;
        }
    }

    /// Advance animated keys (GIF icons) on every device that shows images. The host
    /// calls this often (every 40 ms); a frame is only sent when it changes.
    pub fn animate(&self) {
        let now = self.now_ms();
        let mut st = self.lock();
        let ids: Vec<String> = st
            .devices
            .iter()
            .filter(|(_, d)| d.image_px.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let Some(d) = st.devices.get(&id) else {
                continue;
            };
            let (profile, page, grid, px, sliders) = (
                d.profile.clone(),
                d.page,
                d.grid,
                d.image_px.unwrap_or(0),
                d.sliders,
            );
            let slot = st.slot_mut(&profile);
            let animated: Vec<(u32, usize)> = (0..grid.key_count())
                .filter_map(|index| {
                    let (look, _) = look_at(slot, page, grid, index)?;
                    let anim = animation(slot, look.icon.as_deref()?)?;
                    Some((index, anim.frame_at(now)))
                })
                .collect();
            for (index, frame) in animated {
                let already = st
                    .devices
                    .get(&id)
                    .and_then(|d| d.frames_sent.get(&index))
                    .copied();
                if already == Some(frame) {
                    continue;
                }
                let slot = st.slot_mut(&profile);
                let Some((url, Some(frame))) = key_image(slot, page, grid, index, px, sliders, now)
                else {
                    continue;
                };
                self.outbox.send(
                    &id,
                    Message::SetImage(SetImage {
                        key_index: index,
                        image: Some(url),
                    }),
                );
                if let Some(d) = st.devices.get_mut(&id) {
                    d.frames_sent.insert(index, frame);
                }
            }
        }
    }

    fn now_ms(&self) -> u64 {
        match self.fake_ms.load(std::sync::atomic::Ordering::Relaxed) {
            u64::MAX => self.started.elapsed().as_millis() as u64,
            ms => ms,
        }
    }

    /// Tests: pretend this many ms have passed since the deck started.
    #[doc(hidden)]
    pub fn set_clock_ms(&self, ms: u64) {
        self.fake_ms.store(ms, std::sync::atomic::Ordering::Relaxed);
    }

    /// Re-send one key if its look changed.
    fn refresh_key(&self, st: &mut State, device_id: &str, index: u32) {
        let Some(d) = st.devices.get(device_id) else {
            return;
        };
        let (profile, page, grid, image_px, sliders) =
            (d.profile.clone(), d.page, d.grid, d.image_px, d.sliders);
        let spec = key_spec(st.slot(&profile), page, grid, index, sliders);
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
            let now = self.now_ms();
            if let Some((url, frame)) =
                key_image(st.slot_mut(&profile), page, grid, index, px, sliders, now)
            {
                if let (Some(f), Some(d)) = (frame, st.devices.get_mut(device_id)) {
                    d.frames_sent.insert(index, f);
                }
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
    if key.follow.is_some() {
        return true;
    }
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
    Some((key.look(state_of(st, page, key)), at))
}

/// A toggle's current state: from what it follows, if anything, else from presses.
fn state_of(st: &Slot, page: usize, key: &KeyDef) -> usize {
    if let Some(token) = &key.follow {
        let on = st
            .live
            .read()
            .ok()
            .and_then(|l| l.get(token).map(|v| truthy(v)))
            .unwrap_or(false);
        return usize::from(on);
    }
    st.toggles.get(&(page, key.at)).copied().unwrap_or(0)
}

fn key_spec(st: &Slot, page: usize, grid: Grid, index: u32, sliders: bool) -> KeySpec {
    match look_at(st, page, grid, index) {
        Some((look, at)) => {
            let is_slider = sliders && slider_at(st, page, at).is_some();
            KeySpec {
                index,
                title: render_title(
                    &look.title,
                    chrono::Local::now(),
                    &st.live.read().expect("live values poisoned"),
                ),
                bg_color: look.bg,
                text_color: look.fg,
                control: is_slider.then_some(Control::Slider),
                value: is_slider.then(|| {
                    st.slider_values
                        .get(&(page, at))
                        .copied()
                        .unwrap_or(SLIDER_START)
                }),
            }
        }
        None => KeySpec {
            index,
            title: String::new(),
            bg_color: EMPTY_BG.to_owned(),
            text_color: DEFAULT_FG.to_owned(),
            control: None,
            value: None,
        },
    }
}

fn slider_at(st: &Slot, page: usize, at: [u32; 2]) -> Option<&SliderSpec> {
    st.profile
        .pages
        .get(page)?
        .keys
        .iter()
        .find(|k| k.at == at)?
        .slider
        .as_ref()
}

fn page_layout(st: &Slot, page: usize, grid: Grid, sliders: bool) -> Layout {
    Layout {
        cols: grid.cols,
        rows: grid.rows,
        keys: (0..grid.key_count())
            .map(|i| key_spec(st, page, grid, i, sliders))
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

/// The decoded animation for an icon, if it's an animated GIF.
fn animation(st: &mut Slot, path: &Path) -> Option<Arc<Animation>> {
    let full = st.base_dir.join(path);
    if let Some(a) = st.animations.get(&full) {
        return a.clone();
    }
    let decoded = icon_bytes(st, path)
        .filter(|b| perritodeck_render::is_gif(b))
        .and_then(|b| perritodeck_render::decode_animation(&b))
        .map(Arc::new);
    st.animations.insert(full, decoded.clone());
    decoded
}

/// Render one key as a PNG data URL, plus which frame it is for an animated key.
/// `None` for empty positions (the client's background is enough).
fn key_image(
    st: &mut Slot,
    page: usize,
    grid: Grid,
    index: u32,
    px: u32,
    sliders: bool,
    now_ms: u64,
) -> Option<(String, Option<usize>)> {
    let (look, at) = look_at(st, page, grid, index)?;
    if sliders && slider_at(st, page, at).is_some() {
        return None; // the tablet draws its sliders
    }
    let style = perritodeck_render::KeyStyle {
        title: render_title(
            &look.title,
            chrono::Local::now(),
            &st.live.read().expect("live values poisoned"),
        ),
        background: perritodeck_render::parse_hex(&look.bg).unwrap_or([0x1E, 0x1E, 0x2E]),
        text: perritodeck_render::parse_hex(&look.fg).unwrap_or([0xCD, 0xD6, 0xF4]),
        icon_png: None,
    };
    let Some(icon) = look.icon.as_deref() else {
        return Some((png_url(&style, None, px), None));
    };
    if let Some(anim) = animation(st, icon) {
        let key = FramesKey {
            icon: icon.to_owned(),
            px,
            title: style.title.clone(),
            bg: look.bg.clone(),
            fg: look.fg.clone(),
        };
        if !st.frames.contains_key(&key) && st.frames.len() >= MAX_CACHED_ANIMATIONS {
            st.frames.clear();
        }
        let frames = st
            .frames
            .entry(key)
            .or_insert_with(|| {
                Arc::new(
                    anim.frames
                        .iter()
                        .map(|f| png_url(&style, Some(f), px))
                        .collect(),
                )
            })
            .clone();
        let i = anim.frame_at(now_ms);
        return Some((frames[i].clone(), Some(i)));
    }
    let still = icon_bytes(st, icon).and_then(|b| perritodeck_render::decode_still(&b));
    Some((png_url(&style, still.as_ref(), px), None))
}

fn png_url(
    style: &perritodeck_render::KeyStyle,
    icon: Option<&perritodeck_render::Pixmap>,
    px: u32,
) -> String {
    let png = perritodeck_render::render_with_icon(style, icon, px)
        .encode_png()
        .expect("encoding an in-memory pixmap");
    perritodeck_render::data_url(&png)
}

#[cfg(test)]
mod tests;
