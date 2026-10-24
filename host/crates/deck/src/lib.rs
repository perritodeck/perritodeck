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
    collections::{HashMap, HashSet},
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

pub use templates::{follow_name, render_title, tokens, truthy};

/// Values for `{name}`s in titles (and toggles that follow them), kept up to date by
/// the host: `cpu`, `mem`, `ha:<entity>`, `obs:recording`, …
pub type LiveValues = Arc<std::sync::RwLock<HashMap<String, String>>>;
/// Live values something declares (plugins, from their manifests), reported or not: see
/// [`templates::is_declared`].
pub type DeclaredValues = Arc<std::sync::RwLock<HashSet<String>>>;

/// Where the runtime sends messages (the server in production, a recorder in tests).
pub trait Outbox: Send + Sync {
    fn send(&self, device_id: &str, msg: Message) -> bool;
}

impl Outbox for ServerHandle {
    fn send(&self, device_id: &str, msg: Message) -> bool {
        ServerHandle::send(self, device_id, msg)
    }
}

/// Something else that draws a device's keys and takes its presses: Bitfocus Companion,
/// for devices showing a profile with `companion = true`.
pub trait RemoteSurfaces: Send + Sync {
    /// The device now shows a Companion profile. Called again when its grid or key size
    /// changes.
    fn attach(&self, device_id: &str, grid: Grid, image_px: Option<u32>);
    fn detach(&self, device_id: &str);
    fn press(&self, device_id: &str, key_index: u32, down: bool);
}

/// One key as the remote draws it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemoteKey {
    /// The finished image, as PNG.
    pub png: Option<Vec<u8>>,
    pub title: String,
    /// `#rrggbb`
    pub bg: Option<String>,
    pub fg: Option<String>,
}

/// A key run by a Stream Deck plugin (or anything else that draws its own keys): one
/// instance per profile key, shared by every tablet showing it.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentKey {
    /// `profile/page/row/col`; stable while the key exists.
    pub context: String,
    pub profile: String,
    pub page: usize,
    pub row: u32,
    pub col: u32,
    /// A tablet showing it (the first one).
    pub device: String,
    pub plugin: String,
    pub action: String,
    /// Settings from the profile.
    pub settings: serde_json::Value,
}

/// Runs keys whose action is [`ActionSpec::SdPlugin`].
pub trait KeyAgents: Send + Sync {
    /// A tablet connected (with its grid) or went away (`None`).
    fn device(&self, device_id: &str, grid: Option<Grid>);
    /// The key is now on at least one tablet.
    fn appear(&self, key: &AgentKey);
    /// No tablet shows the key any more (or the profile changed).
    fn disappear(&self, key: &AgentKey);
    fn press(&self, key: &AgentKey, down: bool);
}

/// Told when a key switches the tablet it's on to a profile: `(device id, profile id)`.
pub type ProfileSwitched = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// What an agent drew on its key; `None` keeps the profile's own.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentLook {
    pub title: Option<String>,
    /// PNG, filling the key.
    pub image: Option<Vec<u8>>,
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
    /// Attached to the remote (Companion) with this grid and key size.
    remote: Option<(Grid, Option<u32>)>,
    /// Keys as the remote last drew them.
    remote_keys: HashMap<u32, RemoteKey>,
    /// Agent keys this device counts as showing (contexts).
    agent_keys: HashSet<String>,
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
    /// What agents (Stream Deck plugins) drew, per (page, position).
    agent_looks: HashMap<(usize, [u32; 2]), AgentLook>,
    /// Decoded animated icons (`None`: the icon isn't animated).
    animations: HashMap<PathBuf, Option<Arc<Animation>>>,
    /// Every frame of an animated key, rendered, so animating costs no drawing.
    frames: HashMap<FramesKey, Arc<Vec<String>>>,
    /// Why the last edit to the profile file was rejected, if it was.
    reload_error: Option<String>,
    live: LiveValues,
    declared: DeclaredValues,
}

/// Follows typed as in a title (`{obs:recording}`) work too, and a plugin slider's
/// may repeat `plugin:<its plugin>:` in front of the value.
fn tidy_follows(mut profile: Profile) -> Profile {
    for key in profile.pages.iter_mut().flat_map(|p| p.keys.iter_mut()) {
        key.follow = key
            .follow
            .take()
            .map(|f| follow_name(&f).to_owned())
            .filter(|f| !f.is_empty());
        if let Some(SliderSpec::Plugin { plugin, follow, .. }) = &mut key.slider {
            *follow = follow
                .take()
                .map(|f| {
                    let name = follow_name(&f);
                    let own = format!("plugin:{plugin}:");
                    name.strip_prefix(own.as_str()).unwrap_or(name).to_owned()
                })
                .filter(|f| !f.is_empty());
        }
    }
    profile
}

impl Slot {
    fn new(
        profile: Profile,
        base_dir: PathBuf,
        live: LiveValues,
        declared: DeclaredValues,
    ) -> Self {
        Self {
            live,
            declared,
            profile,
            base_dir,
            toggles: HashMap::new(),
            slider_values: HashMap::new(),
            icons: HashMap::new(),
            animations: HashMap::new(),
            frames: HashMap::new(),
            agent_looks: HashMap::new(),
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
    /// Agent keys on at least one tablet: context → (key, how many tablets).
    agent_refs: HashMap<String, (AgentKey, usize)>,
    /// Profiles whose agent looks changed since [`Deck::take_looks_changed`] last asked.
    looks_changed: HashSet<String>,
    /// The keys each device was given in its handshake, until [`Deck::connected`] takes
    /// them: what the tablet is really showing, which may be older than the profile now.
    handed_out: HashMap<String, Vec<KeySpec>>,
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
    /// A slider tablet started showing sliders: see [`Deck::sliders_shown`].
    sliders_shown: tokio::sync::Notify,
    live: LiveValues,
    declared: DeclaredValues,
    started: std::time::Instant,
    remote: Mutex<Option<Arc<dyn RemoteSurfaces>>>,
    agents: Mutex<Option<Arc<dyn KeyAgents>>>,
    on_switch: Mutex<Option<ProfileSwitched>>,
    /// Shown on Companion profiles while Companion has drawn nothing (e.g. it's offline).
    remote_note: Mutex<String>,
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
        let declared = DeclaredValues::default();
        let mut slots = HashMap::new();
        slots.insert(
            default_id.to_owned(),
            Slot::new(
                tidy_follows(profile),
                base_dir.into(),
                live.clone(),
                declared.clone(),
            ),
        );
        Arc::new(Self {
            state: Mutex::new(State {
                slots,
                default: default_id.to_owned(),
                assignments: HashMap::new(),
                devices: HashMap::new(),
                agent_refs: HashMap::new(),
                looks_changed: HashSet::new(),
                handed_out: HashMap::new(),
            }),
            outbox,
            executor,
            revision: Default::default(),
            paused: Default::default(),
            sliders: Default::default(),
            sliders_shown: Default::default(),
            live,
            declared,
            started: std::time::Instant::now(),
            remote: Mutex::new(None),
            agents: Mutex::new(None),
            on_switch: Mutex::new(None),
            remote_note: Mutex::new(NO_COMPANION.to_owned()),
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
        let mut st = self.lock();
        let slot = st.slot(&st.profile_for(&hello.device_id));
        let grid = device_grid(&slot.profile, demo::grid_for(hello));
        if slot.profile.companion {
            let note = self.remote_note.lock().expect("note poisoned").clone();
            return note_layout(grid, &note);
        }
        let layout = page_layout(
            slot,
            0,
            grid,
            hello.capabilities.iter().any(|c| c == "sliders"),
        );
        st.handed_out
            .insert(hello.device_id.clone(), layout.keys.clone());
        layout
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
        // Start from what the handshake gave the tablet, so whatever changed since (a live
        // value, say) is sent below rather than assumed to be there already.
        let handed_out = st
            .handed_out
            .remove(device_id)
            .filter(|keys| keys.len() == layout.keys.len());
        let sent = handed_out
            .unwrap_or_else(|| layout.keys.clone())
            .into_iter()
            .map(|k| (k.index, k))
            .collect();
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
                remote: None,
                remote_keys: HashMap::new(),
                agent_keys: HashSet::new(),
            },
        );
        if let Some(a) = self.agents() {
            a.device(device_id, Some(grid));
        }
        for index in layout.keys.iter().map(|k| k.index) {
            self.refresh_key(&mut st, device_id, index);
        }
        self.sync_agents(&mut st, device_id);
        if st.slot(&st.profile_for(device_id)).profile.companion {
            self.sync_remote(&mut st, device_id);
        }
        if image_px.is_some() {
            self.send_images(&mut st, device_id);
        }
        self.wake_slider_reads(&st, device_id);
    }

    pub fn disconnected(&self, device_id: &str) {
        let mut st = self.lock();
        if let Some(d) = st.devices.get_mut(device_id) {
            let keys: Vec<String> = d.agent_keys.drain().collect();
            self.release_agents(&mut st, &keys);
        }
        let gone = st.devices.remove(device_id);
        st.handed_out.remove(device_id);
        drop(st);
        if let Some(a) = self.agents() {
            a.device(device_id, None);
        }
        if gone.is_some_and(|d| d.remote.is_some()) {
            if let Some(r) = self.remote() {
                r.detach(device_id);
            }
        }
    }

    // ---- Agents (Stream Deck plugins) ----

    /// Let `agents` run keys with Stream Deck plugin actions.
    pub fn set_agents(&self, agents: Arc<dyn KeyAgents>) {
        *self.agents.lock().expect("agents poisoned") = Some(agents.clone());
        let mut st = self.lock();
        let ids: Vec<(String, Grid)> = st
            .devices
            .iter()
            .map(|(k, d)| (k.clone(), d.grid))
            .collect();
        for (id, grid) in ids {
            agents.device(&id, Some(grid));
            self.sync_agents(&mut st, &id);
        }
    }

    /// The agent key at a position of a profile page, shown on a tablet or not (for its
    /// settings page in the editor). `None` if that key isn't a Stream Deck plugin action.
    pub fn agent_key_for(&self, profile: &str, page: &str, row: u32, col: u32) -> Option<AgentKey> {
        let st = self.lock();
        let slot = st.slots.get(profile)?;
        let page_index = slot.profile.page_index(page)?;
        let key = slot.profile.pages[page_index]
            .keys
            .iter()
            .find(|k| k.at == [row, col])?;
        let Some(ActionSpec::SdPlugin {
            plugin,
            action,
            settings,
        }) = key.action(0)
        else {
            return None;
        };
        let context = format!("{profile}/{page_index}/{row}/{col}");
        let device = st
            .agent_refs
            .get(&context)
            .map(|(k, _)| k.device.clone())
            .or_else(|| st.devices.keys().next().cloned())
            .unwrap_or_else(|| "perritodeck".to_owned());
        Some(AgentKey {
            context,
            profile: profile.to_owned(),
            page: page_index,
            row,
            col,
            device,
            plugin: plugin.clone(),
            action: action.clone(),
            settings: settings.clone(),
        })
    }

    fn agents(&self) -> Option<Arc<dyn KeyAgents>> {
        self.agents.lock().expect("agents poisoned").clone()
    }

    /// Count the agent keys on a device's current page, telling agents about keys that
    /// appear or disappear.
    fn sync_agents(&self, st: &mut State, device_id: &str) {
        let Some(agents) = self.agents() else { return };
        let Some(d) = st.devices.get(device_id) else {
            return;
        };
        let slot = st.slot(&d.profile);
        let mut want: HashMap<String, AgentKey> = HashMap::new();
        if !slot.profile.companion {
            if let Some(p) = slot.profile.pages.get(d.page) {
                for k in p.keys.iter().filter(|k| in_grid(k, d.grid)) {
                    if let Some(ActionSpec::SdPlugin {
                        plugin,
                        action,
                        settings,
                    }) = k.action(0)
                    {
                        let context = format!("{}/{}/{}/{}", d.profile, d.page, k.at[0], k.at[1]);
                        want.insert(
                            context.clone(),
                            AgentKey {
                                context,
                                profile: d.profile.clone(),
                                page: d.page,
                                row: k.at[0],
                                col: k.at[1],
                                device: device_id.to_owned(),
                                plugin: plugin.clone(),
                                action: action.clone(),
                                settings: settings.clone(),
                            },
                        );
                    }
                }
            }
        }
        let gone: Vec<String> = d
            .agent_keys
            .iter()
            .filter(|c| !want.contains_key(*c))
            .cloned()
            .collect();
        let new: Vec<AgentKey> = want
            .into_values()
            .filter(|k| !d.agent_keys.contains(&k.context))
            .collect();
        if let Some(d) = st.devices.get_mut(device_id) {
            for c in &gone {
                d.agent_keys.remove(c);
            }
            for k in &new {
                d.agent_keys.insert(k.context.clone());
            }
        }
        self.release_agents(st, &gone);
        for k in new {
            let entry = st
                .agent_refs
                .entry(k.context.clone())
                .or_insert_with(|| (k.clone(), 0));
            entry.1 += 1;
            if entry.1 == 1 {
                agents.appear(&k);
            }
        }
    }

    fn release_agents(&self, st: &mut State, contexts: &[String]) {
        let agents = self.agents();
        for c in contexts {
            let Some(entry) = st.agent_refs.get_mut(c) else {
                continue;
            };
            entry.1 = entry.1.saturating_sub(1);
            if entry.1 == 0 {
                let (key, _) = st.agent_refs.remove(c).expect("just seen");
                let drawn = st
                    .slots
                    .get_mut(&key.profile)
                    .and_then(|slot| slot.agent_looks.remove(&(key.page, [key.row, key.col])));
                if drawn.is_some_and(|l| l != AgentLook::default()) {
                    st.looks_changed.insert(key.profile.clone());
                }
                if let Some(a) = &agents {
                    a.disappear(&key);
                }
            }
        }
    }

    /// An agent changed its key's title or image (`None` puts the profile's back).
    pub fn agent_draw(
        &self,
        context: &str,
        title: Option<Option<String>>,
        image: Option<Option<Vec<u8>>>,
    ) {
        let mut st = self.lock();
        let Some((key, _)) = st.agent_refs.get(context).cloned() else {
            return;
        };
        let Some(slot) = st.slots.get_mut(&key.profile) else {
            return;
        };
        let look = slot
            .agent_looks
            .entry((key.page, [key.row, key.col]))
            .or_default();
        let before = look.clone();
        if let Some(t) = title {
            look.title = t;
        }
        if let Some(i) = image {
            look.image = i;
        }
        // Plugins often send the same look again; only a real change reaches the editor.
        if *look != before {
            st.looks_changed.insert(key.profile.clone());
        }
        for (id, index) in showing(&st, &key) {
            if let Some(d) = st.devices.get_mut(&id) {
                d.sent.remove(&index); // force a redraw
            }
            self.refresh_key(&mut st, &id, index);
        }
    }

    /// What agents drew on a profile's keys, per (page index, `[row, col]`), for the
    /// editor's preview. Agents only draw keys that are on at least one tablet, so a page
    /// no tablet shows has no looks here.
    pub fn agent_looks(&self, profile: &str) -> HashMap<(usize, [u32; 2]), AgentLook> {
        let st = self.lock();
        st.slots
            .get(profile)
            .map(|slot| {
                slot.agent_looks
                    .iter()
                    .filter(|(_, look)| **look != AgentLook::default())
                    .map(|(at, look)| (*at, look.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The profiles whose agent looks changed since the last call, so UIs can refresh.
    pub fn take_looks_changed(&self) -> Vec<String> {
        let mut st = self.lock();
        let mut changed: Vec<String> = st.looks_changed.drain().collect();
        changed.sort();
        changed
    }

    /// Flash ✓ (`ok`) or ⚠ on an agent's key.
    pub fn agent_feedback(&self, context: &str, ok: bool) {
        let st = self.lock();
        let Some((key, _)) = st.agent_refs.get(context).cloned() else {
            return;
        };
        for (id, index) in showing(&st, &key) {
            let fb = KeyFeedback {
                key_index: index,
                message: None,
            };
            self.outbox.send(
                &id,
                if ok {
                    Message::ShowOk(fb)
                } else {
                    Message::ShowAlert(fb)
                },
            );
        }
    }

    /// The agent key under a device's key, if it is one.
    fn agent_at(&self, st: &State, device_id: &str, key_index: u32) -> Option<AgentKey> {
        let d = st.devices.get(device_id)?;
        let at = [
            key_index / d.grid.cols.max(1),
            key_index % d.grid.cols.max(1),
        ];
        let context = format!("{}/{}/{}/{}", d.profile, d.page, at[0], at[1]);
        d.agent_keys
            .contains(&context)
            .then(|| st.agent_refs.get(&context).map(|(k, _)| k.clone()))
            .flatten()
    }

    // ---- Companion (remote surfaces) ----

    /// Let `remote` draw devices that show a Companion profile.
    pub fn set_remote(&self, remote: Arc<dyn RemoteSurfaces>) {
        *self.remote.lock().expect("remote poisoned") = Some(remote);
        {
            // The remote is already running and may have said how it is (through
            // `remote_status`) before it was handed over; only "Set up" is out of date.
            let mut note = self.remote_note.lock().expect("note poisoned");
            if *note == NO_COMPANION {
                *note = "Connecting to\nCompanion".into();
            }
        }
        let mut st = self.lock();
        let ids: Vec<String> = st.devices.keys().cloned().collect();
        for id in ids {
            if st.slot(&st.devices[&id].profile.clone()).profile.companion {
                self.show_page(&mut st, &id);
            }
        }
    }

    /// Be told when a key switches its tablet to a profile, to remember the choice like
    /// one made in the app.
    pub fn on_profile_switch(&self, f: ProfileSwitched) {
        *self.on_switch.lock().expect("listener poisoned") = Some(f);
    }

    fn remote(&self) -> Option<Arc<dyn RemoteSurfaces>> {
        self.remote.lock().expect("remote poisoned").clone()
    }

    /// The remote drew a key.
    pub fn remote_key(&self, device_id: &str, index: u32, key: RemoteKey) {
        let mut st = self.lock();
        let Some(d) = st.devices.get_mut(device_id) else {
            return;
        };
        if d.remote.is_none() || index >= d.grid.key_count() {
            return;
        }
        if d.remote_keys.get(&index) == Some(&key) {
            return;
        }
        let first = d.remote_keys.is_empty();
        d.remote_keys.insert(index, key.clone());
        let spec = remote_spec(d, index, "");
        let colours_changed = d
            .sent
            .get(&index)
            .is_some_and(|p| p.bg_color != spec.bg_color || p.text_color != spec.text_color);
        if first || (d.image_px.is_none() && colours_changed) {
            // The placeholder note goes, or a text-only tablet needs new colours.
            self.show_page(&mut st, device_id);
            return;
        }
        let px = d.image_px;
        d.sent.insert(index, spec.clone());
        self.outbox.send(
            device_id,
            Message::SetTitle(SetTitle {
                key_index: index,
                title: spec.title.clone(),
            }),
        );
        if let Some(px) = px {
            self.outbox.send(
                device_id,
                Message::SetImage(SetImage {
                    key_index: index,
                    image: Some(remote_image(&key, &spec, px)),
                }),
            );
        }
    }

    /// The remote blanked a device's keys.
    pub fn remote_clear(&self, device_id: &str) {
        let mut st = self.lock();
        if let Some(d) = st.devices.get_mut(device_id) {
            d.remote_keys.clear();
            if d.remote.is_some() {
                self.show_page(&mut st, device_id);
            }
        }
    }

    /// Companion connected (`None`) or went away (why, shown on the keys until it's back).
    pub fn remote_status(&self, note: Option<&str>) {
        let mut st = self.lock();
        let ids: Vec<String> = st
            .devices
            .iter()
            .filter(|(_, d)| d.remote.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        *self.remote_note.lock().expect("note poisoned") = note.unwrap_or_default().to_owned();
        if note.is_none() {
            return; // Companion redraws everything once the tablets are registered.
        }
        for id in ids {
            if let Some(d) = st.devices.get_mut(&id) {
                d.remote_keys.clear();
            }
            self.show_page(&mut st, &id);
        }
    }

    /// Attach or detach a device from the remote to match the profile it shows.
    fn sync_remote(&self, st: &mut State, device_id: &str) {
        let Some(d) = st.devices.get(device_id) else {
            return;
        };
        let remote = self.remote();
        let want = (st.slot(&d.profile).profile.companion && remote.is_some())
            .then_some((d.grid, d.image_px));
        if want == d.remote {
            return;
        }
        let had = d.remote.is_some();
        if let Some(d) = st.devices.get_mut(device_id) {
            d.remote = want;
            if want.is_none() {
                d.remote_keys.clear();
            }
        }
        let Some(r) = remote else { return };
        match want {
            Some((grid, px)) => r.attach(device_id, grid, px),
            None if had => r.detach(device_id),
            None => {}
        }
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

    /// Whether `profile`, as read from its file, is what's loaded for `id` already, so
    /// loading it again would change nothing. Compared as `load_profile` keeps it, with
    /// follows tidied.
    pub fn is_current(&self, id: &str, profile: &Profile) -> bool {
        let tidied = tidy_follows(profile.clone());
        self.lock()
            .slots
            .get(id)
            .is_some_and(|s| s.profile == tidied)
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
        let profile = tidy_follows(profile);
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
                    Slot::new(
                        profile,
                        base_dir.unwrap_or_default(),
                        self.live.clone(),
                        self.declared.clone(),
                    ),
                );
            }
        }
        // Agent keys start over with the new definitions.
        let showing_it: Vec<String> = st
            .devices
            .iter()
            .filter(|(_, d)| d.profile == id)
            .map(|(k, _)| k.clone())
            .collect();
        for dev in showing_it {
            let keys: Vec<String> = st
                .devices
                .get_mut(&dev)
                .map(|d| d.agent_keys.drain().collect())
                .unwrap_or_default();
            self.release_agents(&mut st, &keys);
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
        self.switch_device_to(st, device_id, profile_id, 0);
    }

    /// Show `profile_id` on a device, starting at page `page`.
    fn switch_device_to(&self, st: &mut State, device_id: &str, profile_id: &str, page: usize) {
        let Some(d) = st.devices.get(device_id) else {
            return;
        };
        if d.profile == profile_id {
            return;
        }
        let fixed = st.slot(profile_id).profile.grid;
        if let Some(d) = st.devices.get_mut(device_id) {
            d.profile = profile_id.to_owned();
            d.page = page;
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

    /// Update live values (`cpu`, `ha:sensor.x`, …). Keys showing one that changed, in a
    /// title or as what a toggle follows, are redrawn on every tablet right away.
    pub fn set_live(&self, values: impl IntoIterator<Item = (String, String)>) {
        let mut changed = HashSet::new();
        {
            let mut live = self.live.write().expect("live values poisoned");
            for (k, v) in values {
                if live.get(&k) != Some(&v) {
                    live.insert(k.clone(), v);
                    changed.insert(k);
                }
            }
        }
        if changed.is_empty() {
            return;
        }
        let mut st = self.lock();
        let mut showing = Vec::new();
        for (id, d) in &st.devices {
            let slot = st.slot(&d.profile);
            let Some(page) = slot.profile.pages.get(d.page) else {
                continue;
            };
            if slot.profile.companion {
                continue;
            }
            for k in &page.keys {
                if in_grid(k, d.grid) && shows_any(k, &changed) {
                    showing.push((id.clone(), k.at[0] * d.grid.cols + k.at[1]));
                }
            }
        }
        for (id, index) in showing {
            self.refresh_key(&mut st, &id, index);
        }
    }

    /// The live values plugins declare (`plugin:<id>:<value>`, with a trailing `:` for
    /// values that take an argument). Titles show "–" for those until they're reported.
    pub fn set_declared(&self, names: HashSet<String>) {
        let mut declared = self.declared.write().expect("declared values poisoned");
        if *declared != names {
            *declared = names;
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
                if let Some(SliderSpec::Plugin {
                    plugin,
                    follow: Some(f),
                    ..
                }) = &key.slider
                {
                    out.push(format!("plugin:{plugin}:{f}"));
                }
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
        if st.slot(&profile).profile.companion {
            return;
        }
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

    /// Resolves when a tablet that draws sliders starts showing some (it connected or
    /// changed page). Its layout carried the last level known, which may be old, so
    /// that's the moment to [`poll_sliders`](Deck::poll_sliders) rather than the next poll.
    pub async fn sliders_shown(&self) {
        self.sliders_shown.notified().await;
    }

    /// For [`Deck::sliders_shown`]: the device's page has sliders and it draws them.
    fn wake_slider_reads(&self, st: &State, device_id: &str) {
        let Some(d) = st.devices.get(device_id) else {
            return;
        };
        let slot = st.slot(&d.profile);
        let has_sliders = slot
            .profile
            .pages
            .get(d.page)
            .is_some_and(|p| p.keys.iter().any(|k| k.slider.is_some()));
        if d.sliders && has_sliders && !slot.profile.companion {
            self.sliders_shown.notify_one();
        }
    }

    /// Read the current level of every slider on screen (volume changed on the
    /// computer, a light dimmed elsewhere) and move the tablets' sliders to match.
    pub async fn poll_sliders(self: &Arc<Self>) {
        let visible: Vec<(String, usize, [u32; 2], SliderSpec)> = {
            let st = self.lock();
            let mut seen = std::collections::HashSet::new();
            let mut out = Vec::new();
            for d in st
                .devices
                .values()
                .filter(|d| d.sliders && !st.slot(&d.profile).profile.companion)
            {
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
            let value = match &spec {
                SliderSpec::Plugin {
                    plugin,
                    follow: Some(id),
                    ..
                } => self
                    .live_value(&format!("plugin:{plugin}:{id}"))
                    .and_then(|v| {
                        let n: f64 = v.trim().trim_end_matches('%').parse().ok()?;
                        n.is_finite().then(|| n.round().clamp(0.0, 100.0) as u8)
                    }),
                _ => self.executor.read_slider(&spec).await,
            };
            let Some(value) = value else {
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

    /// Refresh keys whose titles contain live values like `{time}`. (A value set with
    /// [`Deck::set_live`] redraws its keys itself; this catches clocks and the rest.)
    pub fn tick(&self) {
        let mut st = self.lock();
        let ids: Vec<String> = st.devices.keys().cloned().collect();
        for id in ids {
            let (profile, page, grid) = match st.devices.get(&id) {
                Some(d) if !st.slot(&d.profile).profile.companion => {
                    (d.profile.clone(), d.page, d.grid)
                }
                _ => continue,
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

    /// A key came back up. Only Companion profiles care (Companion has press and release
    /// actions); PerritoDeck's own keys act on the way down.
    pub fn key_up(&self, device_id: &str, key_index: u32) {
        let st = self.lock();
        if !self.paused() {
            if let (Some(mut agent), Some(a)) =
                (self.agent_at(&st, device_id, key_index), self.agents())
            {
                agent.device = device_id.to_owned();
                a.press(&agent, false);
                return;
            }
        }
        let attached = st
            .devices
            .get(device_id)
            .is_some_and(|d| d.remote.is_some());
        drop(st);
        if attached {
            if let Some(r) = self.remote() {
                r.press(device_id, key_index, false);
            }
        }
    }

    /// A key went down on a device.
    pub fn key_down(self: &Arc<Self>, device_id: &str, key_index: u32) {
        let mut st = self.lock();
        let Some(d) = st.devices.get(device_id) else {
            return;
        };
        if st.slot(&d.profile).profile.companion {
            if d.remote.is_none() {
                return; // Companion isn't set up; the key says so
            }
            if self.paused() {
                self.outbox.send(
                    device_id,
                    Message::ShowAlert(KeyFeedback {
                        key_index,
                        message: Some("Keys are paused on the computer".into()),
                    }),
                );
            } else if let Some(r) = self.remote() {
                r.press(device_id, key_index, true);
            }
            return;
        }
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

        if let Some(mut agent) = self.agent_at(&st, device_id, key_index) {
            agent.device = device_id.to_owned(); // the tablet pressing it
            if let Some(a) = self.agents() {
                a.press(&agent, true);
            }
            return;
        }

        if let Some(NavSpec::Profile {
            profile: target,
            page: target_page,
        }) = &key.nav
        {
            if let Err(message) =
                self.key_switch(&mut st, device_id, target, target_page.as_deref())
            {
                warn!(device = device_id, key = key_index + 1, "{message}");
                self.outbox.send(
                    device_id,
                    Message::ShowAlert(KeyFeedback {
                        key_index,
                        message: Some(message),
                    }),
                );
                return;
            }
            drop(st);
            self.switched_by_key(device_id, target);
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
                deck.report(&device, key_index, &action, &result);
            });
        }
    }

    /// A switch-profile key on `device_id`: show `target` (at `target_page`), and
    /// remember it as the tablet's choice. `Err` says why not (no such profile or page).
    /// Call [`Deck::switched_by_key`] once the state is unlocked.
    fn key_switch(
        &self,
        st: &mut State,
        device_id: &str,
        target: &str,
        target_page: Option<&str>,
    ) -> Result<(), String> {
        let found = st.slots.get(target).map(|slot| match target_page {
            Some(name) => slot.profile.page_index(name),
            None => Some(0),
        });
        let to_page = match found {
            Some(Some(i)) => i,
            missing => {
                return Err(match (missing, target_page) {
                    (Some(None), Some(name)) => {
                        format!("Profile {target:?} has no page called {name:?}")
                    }
                    _ => format!("There's no profile called {target:?}"),
                })
            }
        };
        let Some(d) = st.devices.get(device_id) else {
            return Err(format!("Tablet {device_id:?} isn't connected"));
        };
        let (profile, page) = (d.profile.clone(), d.page);
        // Remembered like a choice made in the app.
        if target == st.default {
            st.assignments.remove(device_id);
        } else {
            st.assignments
                .insert(device_id.to_owned(), target.to_owned());
        }
        if target != profile {
            self.switch_device_to(st, device_id, target, to_page);
        } else if target_page.is_some() && to_page != page {
            if let Some(d) = st.devices.get_mut(device_id) {
                d.back.push(page);
                d.page = to_page;
            }
            self.show_page(st, device_id);
        }
        Ok(())
    }

    /// After [`Deck::key_switch`]: tell UIs and the switch listener.
    fn switched_by_key(&self, device_id: &str, target: &str) {
        self.bump();
        let listener = self.on_switch.lock().expect("listener poisoned").clone();
        if let Some(f) = listener {
            f(device_id, target);
        }
    }

    /// The name of the page a connected device shows (`None` for a Companion profile).
    pub fn device_page(&self, device_id: &str) -> Option<String> {
        let st = self.lock();
        let d = st.devices.get(device_id)?;
        let slot = st.slot(&d.profile);
        if slot.profile.companion {
            return None;
        }
        slot.profile.pages.get(d.page).map(|p| p.name.clone())
    }

    /// Press the key at `at` (`[row, col]`) on a page of a profile, as if it were tapped
    /// on a tablet; for AI assistants (the MCP server), which name keys by position.
    ///
    /// * Keys paused → `Err`.
    /// * An action key runs its action and flashes ✓/⚠ on the tablets showing it.
    /// * A toggle advances its (shared) state like a tap; one that follows a live value
    ///   runs the action for the state it shows, without flipping.
    /// * A page key changes the page on `tablet`, or else on every connected tablet
    ///   showing this profile; with none, it's an `Err`. A switch-profile key switches
    ///   `tablet` (or those same tablets) to its profile.
    /// * A Stream Deck plugin key gets a key down and a key up.
    /// * An empty position, or a key with nothing to do, gives `Ok(None)`.
    ///
    /// `Ok(Some(note))` means the action ran but reported a problem.
    pub async fn press_at(
        self: &Arc<Self>,
        profile: &str,
        page: &str,
        at: [u32; 2],
        tablet: Option<&str>,
    ) -> Result<Option<String>, String> {
        if self.paused() {
            return Err(
                "Keys are paused on the computer; resume them in PerritoDeck first.".into(),
            );
        }
        let (action, showing) = {
            let mut st = self.lock();
            let slot = st
                .slots
                .get(profile)
                .ok_or_else(|| format!("There's no profile called {profile:?}."))?;
            if slot.profile.companion {
                return Err(format!(
                    "Profile {profile:?} shows Bitfocus Companion's buttons, which are pressed in Companion."
                ));
            }
            let page_index = slot
                .profile
                .page_index(page)
                .ok_or_else(|| format!("Profile {profile:?} has no page called {page:?}."))?;
            let Some(key) = slot.profile.pages[page_index]
                .keys
                .iter()
                .find(|k| k.at == at)
                .cloned()
            else {
                return Ok(None);
            };
            if let Some(t) = tablet {
                if !st.devices.contains_key(t) {
                    return Err(format!("Tablet {t:?} isn't connected."));
                }
            }
            if key.slider.is_some() {
                return Err("That key is a slider; sliders can't be pressed.".into());
            }

            if let Some(ActionSpec::SdPlugin { .. }) = key.action(0) {
                let context = format!("{profile}/{page_index}/{}/{}", at[0], at[1]);
                let Some((mut agent, _)) = st.agent_refs.get(&context).cloned() else {
                    return Err("A Stream Deck plugin key works only while a tablet shows it; show its page on a tablet first.".into());
                };
                let agents = self
                    .agents()
                    .ok_or("Stream Deck plugins aren't running on this computer.")?;
                if let Some(t) = tablet {
                    agent.device = t.to_owned();
                }
                drop(st);
                agents.press(&agent, true);
                agents.press(&agent, false);
                return Ok(None);
            }

            // Tablets a page or profile key acts on.
            let targets = || -> Result<Vec<String>, String> {
                match tablet {
                    Some(t) => Ok(vec![t.to_owned()]),
                    None => {
                        let mut on: Vec<String> = st
                            .devices
                            .iter()
                            .filter(|(_, d)| d.profile == profile)
                            .map(|(id, _)| id.clone())
                            .collect();
                        on.sort();
                        if on.is_empty() {
                            Err(format!(
                                "No connected tablet shows profile {profile:?}; say which tablet to use."
                            ))
                        } else {
                            Ok(on)
                        }
                    }
                }
            };

            if let Some(NavSpec::Profile {
                profile: target,
                page: target_page,
            }) = &key.nav
            {
                let devices = targets()?;
                for d in &devices {
                    self.key_switch(&mut st, d, target, target_page.as_deref())
                        .map_err(|e| format!("{e}."))?;
                }
                drop(st);
                for d in &devices {
                    self.switched_by_key(d, target);
                }
                return Ok(None);
            }

            if let Some(nav) = &key.nav {
                let devices = targets()?;
                for id in &devices {
                    let mut d = st.devices.remove(id).expect("checked above");
                    if d.profile != profile {
                        let showing = d.profile.clone();
                        st.devices.insert(id.clone(), d);
                        return Err(format!(
                            "Tablet {id:?} shows profile {showing:?}, not {profile:?}."
                        ));
                    }
                    d.page = navigate(&st.slot(profile).profile, &mut d, page_index, nav);
                    st.devices.insert(id.clone(), d);
                    self.show_page(&mut st, id);
                }
                return Ok(None);
            }

            let slot = st.slot(profile);
            let state = state_of(slot, page_index, &key);
            let action = key.action(state).cloned();
            let showing: Vec<(String, u32)> = st
                .devices
                .iter()
                .filter(|(_, d)| {
                    d.profile == profile && d.page == page_index && in_grid(&key, d.grid)
                })
                .map(|(id, d)| (id.clone(), at[0] * d.grid.cols + at[1]))
                .collect();
            if key.states.len() >= 2 && key.follow.is_none() {
                st.slot_mut(profile)
                    .toggles
                    .insert((page_index, at), (state + 1) % key.states.len());
                for (id, idx) in &showing {
                    self.refresh_key(&mut st, id, *idx);
                }
            }
            (action, showing)
        };
        let Some(action) = action else {
            return Ok(None);
        };
        let result = self.executor.run(&action).await;
        for (id, idx) in &showing {
            self.report(id, *idx, &action, &result);
        }
        match result {
            Ok(Outcome::Ok) => Ok(None),
            Ok(Outcome::Alert(m)) => Ok(Some(m)),
            Err(e) => Err(e.to_string()),
        }
    }

    fn report(
        &self,
        device_id: &str,
        key_index: u32,
        action: &ActionSpec,
        result: &Result<Outcome, ActionError>,
    ) {
        let feedback = |message: Option<String>| KeyFeedback { key_index, message };
        match result {
            Ok(Outcome::Ok) => {
                // Commands can take a while, so confirm they finished; instant actions don't need it.
                if confirms(action) {
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
                    .send(device_id, Message::ShowAlert(feedback(Some(m.clone()))));
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
        self.sync_remote(st, device_id);
        self.sync_agents(st, device_id);
        let Some(d) = st.devices.get(device_id) else {
            return;
        };
        if st.slot(&d.profile).profile.companion {
            self.show_remote_page(st, device_id);
            return;
        }
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
        self.wake_slider_reads(st, device_id);
    }

    /// A Companion profile: the keys as Companion drew them (or a note while it hasn't).
    fn show_remote_page(&self, st: &mut State, device_id: &str) {
        let note = self.remote_note.lock().expect("note poisoned").clone();
        let Some(d) = st.devices.get_mut(device_id) else {
            return;
        };
        let layout = Layout {
            cols: d.grid.cols,
            rows: d.grid.rows,
            keys: (0..d.grid.key_count())
                .map(|i| remote_spec(d, i, &note))
                .collect(),
        };
        d.sent = layout.keys.iter().map(|k| (k.index, k.clone())).collect();
        d.frames_sent.clear();
        self.outbox.send(device_id, Message::Layout(layout.clone()));
        let Some(px) = d.image_px else { return };
        for spec in layout.keys {
            let key = d.remote_keys.get(&spec.index).cloned().unwrap_or_default();
            if key.png.is_none() && spec.title.is_empty() && spec.bg_color == EMPTY_BG {
                continue;
            }
            self.outbox.send(
                device_id,
                Message::SetImage(SetImage {
                    key_index: spec.index,
                    image: Some(remote_image(&key, &spec, px)),
                }),
            );
        }
    }

    fn send_images(&self, st: &mut State, device_id: &str) {
        let Some(d) = st.devices.get(device_id) else {
            return;
        };
        if st.slot(&d.profile).profile.companion {
            return; // drawn by show_remote_page
        }
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
            .filter(|(_, d)| d.image_px.is_some() && !st.slot(&d.profile).profile.companion)
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

/// Devices showing an agent key right now, with the key's index on each.
fn showing(st: &State, key: &AgentKey) -> Vec<(String, u32)> {
    st.devices
        .iter()
        .filter(|(_, d)| d.agent_keys.contains(&key.context))
        .map(|(id, d)| (id.clone(), key.row * d.grid.cols + key.col))
        .collect()
}

/// What the remote profile placeholder says before Companion is set up.
const NO_COMPANION: &str = "Set up\nCompanion";

fn note_layout(grid: Grid, note: &str) -> Layout {
    Layout {
        cols: grid.cols,
        rows: grid.rows,
        keys: (0..grid.key_count())
            .map(|index| KeySpec {
                index,
                title: if index == 0 {
                    note.to_owned()
                } else {
                    String::new()
                },
                bg_color: EMPTY_BG.to_owned(),
                text_color: DEFAULT_FG.to_owned(),
                control: None,
                value: None,
            })
            .collect(),
    }
}

/// A key of a Companion profile: as Companion drew it, or the note on the first key
/// while it has drawn nothing.
fn remote_spec(d: &DeviceView, index: u32, note: &str) -> KeySpec {
    let key = d.remote_keys.get(&index);
    let title = match key {
        Some(k) => k.title.clone(),
        None if index == 0 && d.remote_keys.is_empty() => note.to_owned(),
        None => String::new(),
    };
    KeySpec {
        index,
        title,
        bg_color: key
            .and_then(|k| k.bg.clone())
            .unwrap_or_else(|| EMPTY_BG.to_owned()),
        text_color: key
            .and_then(|k| k.fg.clone())
            .unwrap_or_else(|| DEFAULT_FG.to_owned()),
        control: None,
        value: None,
    }
}

/// Companion's image, or one drawn from its text and colours.
fn remote_image(key: &RemoteKey, spec: &KeySpec, px: u32) -> String {
    if let Some(png) = &key.png {
        return perritodeck_render::data_url(png);
    }
    let style = perritodeck_render::KeyStyle {
        title: spec.title.clone(),
        background: perritodeck_render::parse_hex(&spec.bg_color).unwrap_or([0x1E, 0x1E, 0x2E]),
        text: perritodeck_render::parse_hex(&spec.text_color).unwrap_or([0xCD, 0xD6, 0xF4]),
        icon_png: None,
    };
    perritodeck_render::data_url(&perritodeck_render::render_png(&style, px))
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

/// Whether a key shows any of the live values `names`, in a title (of any state) or as
/// what it follows.
fn shows_any(key: &KeyDef, names: &HashSet<String>) -> bool {
    let titles = key
        .title
        .iter()
        .chain(key.states.iter().filter_map(|s| s.title.as_ref()));
    key.follow.as_ref().is_some_and(|f| names.contains(f))
        || titles
            .flat_map(|t| tokens(t))
            .any(|name| names.contains(name))
}

/// Whether finishing `action` is worth a ✓ on the key (it may have taken a while).
fn confirms(action: &ActionSpec) -> bool {
    match action {
        ActionSpec::Command { .. } | ActionSpec::Multi { .. } => true,
        ActionSpec::Random { actions } => actions.iter().any(confirms),
        _ => false,
    }
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
        // Handled before getting here: it changes the profile, not just the page.
        NavSpec::Profile { .. } => page,
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
            let agent_title = st
                .agent_looks
                .get(&(page, at))
                .and_then(|l| l.title.clone());
            KeySpec {
                index,
                title: agent_title.unwrap_or_else(|| {
                    render_title(
                        &look.title,
                        chrono::Local::now(),
                        &st.live.read().expect("live values poisoned"),
                        &st.declared.read().expect("declared values poisoned"),
                    )
                }),
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
    let agent = st.agent_looks.get(&(page, at)).cloned().unwrap_or_default();
    let style = perritodeck_render::KeyStyle {
        title: agent.title.clone().unwrap_or_else(|| {
            render_title(
                &look.title,
                chrono::Local::now(),
                &st.live.read().expect("live values poisoned"),
                &st.declared.read().expect("declared values poisoned"),
            )
        }),
        background: perritodeck_render::parse_hex(&look.bg).unwrap_or([0x1E, 0x1E, 0x2E]),
        text: perritodeck_render::parse_hex(&look.fg).unwrap_or([0xCD, 0xD6, 0xF4]),
        icon_png: None,
    };
    if let Some(img) = agent
        .image
        .as_deref()
        .and_then(|b| perritodeck_render::Pixmap::decode_png(b).ok())
    {
        let png = perritodeck_render::render_full_image(&img, &style.title, style.text, px)
            .encode_png()
            .expect("encoding an in-memory pixmap");
        return Some((perritodeck_render::data_url(&png), None));
    }
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
