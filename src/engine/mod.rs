//! The engine: the single owner of every connected SteelSeries device.
//!
//! Every front end goes through [`Engine::execute`] with a [`Command`]: the CLI (directly, or
//! over the control API when the daemon is running), the web control panel, profiles, and the
//! daemon's own loops (hot-plug, lighting animation, battery polling). Because one owner holds
//! the devices, a colour set from the CLI is no longer overwritten by the daemon's next frame,
//! and a device plugged in later gets its settings and lighting back.

pub mod audio;
pub mod bindings;
pub mod command;
pub mod control;
pub mod gamesense;
pub mod screen;
pub mod state;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

pub use command::{Command, DeviceSnapshot, EngineSnapshot, ProfileSummary, SettingEntry, SettingsView};
pub use state::{DeviceRecord, EngineState, Lighting};

use crate::autoswitch::{AutoSwitch, ProcessWatcher, Rule};
use crate::config::Config;
use crate::devices::headsets::Headset;
use crate::devices::key_mapping::KeyId;
use crate::devices::keyboards::Keyboard;
use crate::devices::mice::Mouse;
use crate::devices::settings::{Configurable, DeviceStatus, SettingDescriptor, SettingKind, SettingValue};
use crate::devices::{DeviceInfo, DeviceManager, DeviceType};
use crate::moments::MomentsRecorder;
use crate::notify::{self, BatteryWatch, Urgency};
use crate::oled::OledFrame;
use crate::profiles::{DeviceProfile, Profile, ProfileManager};
use crate::rgb::{Color, Effect, RgbController};
use crate::{Error, Result};
use screen::ScreenManager;

/// How often the daemon rescans for plugged/unplugged devices.
const HOTPLUG_INTERVAL: Duration = Duration::from_secs(2);
/// Lighting frame period (~30 fps). Unchanged frames are not re-sent.
const FRAME_INTERVAL: Duration = Duration::from_millis(33);
/// How often battery / ChatMix readings are refreshed.
const STATUS_INTERVAL: Duration = Duration::from_secs(10);
/// How often dirty state is flushed to disk.
const SAVE_INTERVAL: Duration = Duration::from_secs(3);
/// How often running programs are checked for automatic profile switching.
const AUTOSWITCH_INTERVAL: Duration = Duration::from_secs(2);
/// OLED refresh tick; unchanged frames are not re-sent.
const OLED_INTERVAL: Duration = Duration::from_millis(100);
/// How long a temporary OLED screen stays when no duration is given.
const OLED_DEFAULT_SECONDS: u32 = 5;
/// Tick for input-engine notices and the headset ChatMix dial (dial read every other tick).
const INPUT_INTERVAL: Duration = Duration::from_millis(250);
/// Every this many hot-plug ticks (~30 s) every frame is re-sent, which also notices dead
/// handles and relights wireless devices that slept.
const KEEPALIVE_TICKS: u64 = 15;
/// Consecutive write failures after which a device is dropped so the next scan reopens it.
const MAX_WRITE_FAILURES: u32 = 10;
/// Safety cap on live overlays.
const MAX_OVERLAYS: usize = 2048;
/// Animation frame spacing for mice (their reports are slower than keyboards').
const MOUSE_FRAME_INTERVAL: Duration = Duration::from_millis(100);
/// Frame spacing once a mouse's writes prove slow (wireless read-back).
const SLOW_MOUSE_FRAME_INTERVAL: Duration = Duration::from_secs(1);

/// An opened device of any family.
pub enum DeviceHandle {
    Keyboard(Box<dyn Keyboard>),
    Mouse(Box<dyn Mouse>),
    Headset(Box<dyn Headset>),
}

impl DeviceHandle {
    pub fn kind(&self) -> DeviceType {
        match self {
            DeviceHandle::Keyboard(_) => DeviceType::Keyboard,
            DeviceHandle::Mouse(_) => DeviceType::Mouse,
            DeviceHandle::Headset(_) => DeviceType::Headset,
        }
    }

    /// The uniform settings interface, when this device family provides one.
    pub fn configurable(&mut self) -> Option<&mut dyn Configurable> {
        match self {
            DeviceHandle::Keyboard(keyboard) => Some(keyboard.as_mut()),
            DeviceHandle::Mouse(mouse) => Some(mouse.as_mut()),
            DeviceHandle::Headset(headset) => Some(headset.as_mut()),
        }
    }

    fn descriptors(&self) -> Vec<SettingDescriptor> {
        match self {
            DeviceHandle::Keyboard(keyboard) => keyboard.setting_descriptors(),
            DeviceHandle::Mouse(mouse) => mouse.setting_descriptors(),
            DeviceHandle::Headset(headset) => headset.setting_descriptors(),
        }
    }

    /// Number of independently coloured zones the lighting engine drives (0 = no lighting).
    fn lighting_zones(&self) -> usize {
        match self {
            DeviceHandle::Keyboard(keyboard) => keyboard.zone_count().max(1),
            DeviceHandle::Mouse(mouse) => mouse.color_zone_names().len(),
            DeviceHandle::Headset(_) => 0,
        }
    }

    fn supports_per_key(&self) -> bool {
        matches!(self, DeviceHandle::Keyboard(k) if k.supports_per_key_rgb())
    }

    async fn write_zones(&mut self, colors: &[Color]) -> Result<()> {
        match self {
            DeviceHandle::Keyboard(keyboard) => keyboard.set_zone_colors(colors).await,
            DeviceHandle::Mouse(mouse) => mouse.set_zone_colors_direct(colors),
            DeviceHandle::Headset(_) => Ok(()),
        }
    }
}

/// A colour pushed on top of the normal lighting for a limited time (GameSense events,
/// notifications).
#[derive(Clone, Debug)]
pub struct Overlay {
    /// Which devices: a specific device key, or every device of a type.
    pub target: OverlayTarget,
    /// Which LEDs on those devices.
    pub zone: OverlayZone,
    pub color: Color,
    pub expires: Instant,
    /// Flash on/off at this frequency instead of a steady colour.
    pub flash_hz: Option<f32>,
    /// Who pushed it (a GameSense game name), so it can be cleared with its owner.
    pub source: Option<String>,
    /// GameSense event that produced it; newer colours of the same event replace older ones.
    pub event: Option<String>,
}

impl Overlay {
    fn applies_to(&self, key: &str, kind: DeviceType) -> bool {
        match &self.target {
            OverlayTarget::Device(k) => k == key,
            OverlayTarget::Kind(t) => *t == kind,
        }
    }

    fn color_at(&self, now: Instant, epoch: Instant) -> Color {
        let lit = self.flash_hz.is_none_or(|hz| {
            let t = now.duration_since(epoch).as_secs_f32();
            ((t * hz * 2.0) as u64).is_multiple_of(2)
        });
        if lit { self.color } else { Color::BLACK }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum OverlayTarget {
    Device(String),
    Kind(DeviceType),
}

/// LEDs an overlay covers.
#[derive(Clone, Debug, PartialEq)]
pub enum OverlayZone {
    All,
    /// Zone by index (0-based).
    Index(usize),
    /// Zone by name (`logo`, `wheel`, ...), matched against the device's zone names; every
    /// zone when no name matches.
    Named(String),
    /// Individual keys on per-key keyboards.
    Keys(Vec<KeyId>),
}

struct Slot {
    info: DeviceInfo,
    model_key: String,
    handle: DeviceHandle,
    lighting: Option<RgbController>,
    last_frame: Vec<Color>,
    key_colors: Vec<(KeyId, Color)>,
    status: DeviceStatus,
    status_read_at: Option<Instant>,
    battery_watch: BatteryWatch,
    write_failures: u32,
    oled_size: Option<(u32, u32)>,
    last_oled: Option<OledFrame>,
    /// LED zone names (mice), for GameSense named zones.
    zone_names: Vec<String>,
    /// Whether the last lighting frame went out per key (vs. per zone).
    last_per_key: bool,
    oled_failures: u32,
    last_write_at: Option<Instant>,
    slow_writes: bool,
}

impl Slot {
    fn name(&self) -> &str {
        &self.info.name
    }

    fn capabilities(&self) -> Vec<String> {
        let mut caps = Vec::new();
        if self.lighting.is_some() {
            caps.push("lighting".to_string());
        }
        if self.handle.supports_per_key() {
            caps.push("per_key".to_string());
        }
        if !self.handle.descriptors().is_empty() {
            caps.push("settings".to_string());
        }
        if self.oled_size.is_some() {
            caps.push("oled".to_string());
        }
        caps
    }

    fn configure_lighting(&mut self, lighting: &Lighting) {
        let zones = self.handle.lighting_zones();
        if zones == 0 {
            self.lighting = None;
            return;
        }
        let controller = self.lighting.get_or_insert_with(|| RgbController::new(zones));
        if controller.effect() != &lighting.effect {
            controller.set_effect(lighting.effect.clone());
        }
        controller.set_brightness(f32::from(lighting.brightness.min(100)) / 100.0);
        // Force the next frame out even if the colours happen to match the last one sent.
        self.last_frame.clear();
    }
}

struct Inner {
    manager: DeviceManager,
    slots: BTreeMap<String, Slot>,
    state: EngineState,
    state_dirty: bool,
    profiles: Option<ProfileManager>,
    overlays: Vec<Overlay>,
    /// Device key -> last open error, so a failing device is reported once, not every scan.
    open_failures: HashMap<String, String>,
    /// Time base for flashing overlays.
    epoch: Instant,
    config: Config,
    moments: MomentsRecorder,
    screens: ScreenManager,
    /// GameSense game that owns the temporary OLED content, if any.
    screen_source: Option<String>,
    input: bindings::InputState,
    daemon: bool,
    /// Mixer settings from a just-loaded profile, applied by `Engine` outside this lock.
    pending_mixer: Option<crate::mixer::MixerConfig>,
    /// Mixer settings captured by `Engine` for a profile save.
    mixer_snapshot: Option<Value>,
    /// Devices that lost contact repeatedly, for quieter logging.
    reconnects: HashMap<String, u32>,
}

/// Handles for the daemon's background loops; dropping it does not stop them, call
/// [`BackgroundTasks::abort`].
pub struct BackgroundTasks {
    handles: Vec<JoinHandle<()>>,
}

impl BackgroundTasks {
    pub fn abort(self) {
        for handle in self.handles {
            handle.abort();
        }
    }
}

pub struct Engine {
    inner: Mutex<Inner>,
    /// The mixer has its own lock: its calls run external programs and can take seconds,
    /// which must not stall lighting or the control API.
    audio: Mutex<audio::AudioState>,
    daemon: bool,
    started: Instant,
}

impl Engine {
    /// Open the HID layer, load remembered state and connect every present device.
    ///
    /// `daemon` marks the long-lived engine; a one-shot CLI engine passes `false`.
    pub async fn open(daemon: bool) -> Result<Arc<Self>> {
        let mut manager = DeviceManager::new()?;
        let config = match Config::load_async().await {
            Ok(config) => config,
            Err(e) => {
                warn!("config.toml is invalid ({e}); using defaults");
                Config::default()
            }
        };
        manager.set_device_override(config.device.clone());
        let profiles = match ProfileManager::new().await {
            Ok(profiles) => Some(profiles),
            Err(e) => {
                warn!("Profiles unavailable: {e}");
                None
            }
        };
        let state = EngineState::load();
        let screens = ScreenManager::new(state.oled_idle.clone());
        let moments_config = state.moments.clone().unwrap_or_else(|| config.moments.clone());
        let engine = Arc::new(Self {
            inner: Mutex::new(Inner {
                manager,
                slots: BTreeMap::new(),
                state,
                screens,
                screen_source: None,
                input: bindings::InputState::default(),
                daemon,
                pending_mixer: None,
                mixer_snapshot: None,
                reconnects: HashMap::new(),
                state_dirty: false,
                profiles,
                overlays: Vec::new(),
                open_failures: HashMap::new(),
                epoch: Instant::now(),
                moments: MomentsRecorder::new(moments_config),
                config,
            }),
            audio: Mutex::new(audio::AudioState::load()),
            daemon,
            started: Instant::now(),
        });
        engine.refresh_devices().await?;
        engine.start_services().await;
        Ok(engine)
    }

    /// Bring up what the remembered state asks for: the mixer and the active profile's key
    /// bindings (daemon only).
    async fn start_services(&self) {
        if !self.daemon {
            return;
        }
        let mixer_enabled = self.inner.lock().await.state.mixer_enabled;
        if mixer_enabled {
            let mut audio = self.audio.lock().await;
            if let Err(e) = tokio::task::block_in_place(|| audio.start()) {
                warn!("Audio mixer not started: {e}");
            }
        }
        let mut inner = self.inner.lock().await;
        let bindings = inner
            .state
            .active_profile
            .clone()
            .and_then(|name| inner.profiles.as_ref().and_then(|p| p.get(&name)).cloned())
            .map(|profile| bindings::bindings_from_profile(profile.bindings.as_ref()))
            .transpose();
        match bindings {
            Ok(Some(set)) => inner.input.apply(set, true),
            Ok(None) => {}
            Err(e) => warn!("Active profile's key bindings ignored: {e}"),
        }
    }

    /// Stop everything that changes the system (mixer defaults, grabbed input, recorder) and
    /// save state. Called by the daemon on exit.
    pub async fn shutdown(&self) {
        {
            let mut audio = self.audio.lock().await;
            tokio::task::block_in_place(|| audio.stop());
        }
        let mut inner = self.inner.lock().await;
        inner.input.stop();
        inner.moments.stop();
        inner.save_state();
    }

    pub fn is_daemon(&self) -> bool {
        self.daemon
    }

    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    /// Rescan the bus: open new devices (restoring their settings and lighting) and drop
    /// unplugged ones.
    pub async fn refresh_devices(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.sync_devices().await
    }

    /// Execute one command and return its JSON result.
    pub async fn execute(&self, command: Command) -> Result<Value> {
        match command {
            Command::MacroRecord { stop_key, timeout_secs } => return self.record_macro(stop_key, timeout_secs).await,
            Command::Mixer { patch, enabled } => return self.mixer_command(patch, enabled).await,
            _ => {}
        }
        if matches!(command, Command::ProfileSave { .. }) {
            let enabled = self.inner.lock().await.state.mixer_enabled;
            let snapshot = if enabled {
                Some(serde_json::to_value(&self.audio.lock().await.config)?)
            } else {
                None
            };
            self.inner.lock().await.mixer_snapshot = snapshot;
        }
        let (result, pending) = {
            let mut inner = self.inner.lock().await;
            let result = inner.execute(command, self.daemon).await;
            if !self.daemon && inner.state_dirty {
                inner.save_state();
            }
            (result, inner.pending_mixer.take())
        };
        if let Some(config) = pending {
            let mut audio = self.audio.lock().await;
            if let Err(e) = tokio::task::block_in_place(|| audio.replace_config(config)) {
                warn!("Profile mixer settings not applied: {e}");
            }
        }
        result
    }

    /// Mixer settings and on/off, under the mixer's own lock.
    async fn mixer_command(&self, patch: Option<audio::MixerPatch>, enabled: Option<bool>) -> Result<Value> {
        let enabled_now = {
            let mut inner = self.inner.lock().await;
            if let Some(enabled) = enabled {
                inner.state.mixer_enabled = enabled;
                inner.state_dirty = true;
                if !self.daemon {
                    inner.save_state();
                }
            }
            inner.state.mixer_enabled
        };
        let daemon = self.daemon;
        let mut audio = self.audio.lock().await;
        tokio::task::block_in_place(|| {
            if let Some(enabled) = enabled
                && daemon
            {
                if enabled {
                    audio.start()?;
                } else {
                    audio.stop();
                }
            }
            if let Some(patch) = patch {
                audio.update(&patch)?;
            }
            Ok(serde_json::to_value(audio.view(enabled_now))?)
        })
    }

    /// Push a temporary lighting overlay (GameSense, notifications).
    pub async fn push_overlay(&self, overlay: Overlay) {
        self.inner.lock().await.push_overlay(overlay);
    }

    /// Drop every overlay (e.g. a game stopped).
    pub async fn clear_overlays(&self) {
        self.inner.lock().await.overlays.clear();
    }

    /// Apply one GameSense output: lighting overlays, per-key colours, OLED screens.
    pub async fn apply_gamesense(&self, output: crate::gamesense::GameSenseOutput) {
        let action = gamesense::translate(output, Instant::now());
        let mut inner = self.inner.lock().await;
        match action {
            gamesense::Action::Overlays(overlays) => {
                for overlay in overlays {
                    inner.push_overlay(overlay);
                }
            }
            gamesense::Action::Screen {
                game,
                content,
                duration,
                size,
            } => {
                // A generic screen handler sends one variant per screen size; keep the one that
                // fits the connected keyboard.
                let fits = inner
                    .slots
                    .values()
                    .any(|s| s.oled_size.is_some() && (size.is_none() || s.oled_size == size));
                if !fits {
                    return;
                }
                match inner.screens.show(&content, duration) {
                    Ok(()) => inner.screen_source = Some(game),
                    Err(e) => debug!("GameSense screen from {game} not shown: {e}"),
                }
            }
            gamesense::Action::Clear { game } => {
                inner.overlays.retain(|o| o.source.as_deref() != Some(game.as_str()));
                if inner.screen_source.as_deref() == Some(game.as_str()) {
                    inner.screens.clear_temporary();
                    inner.screen_source = None;
                }
            }
            gamesense::Action::Ignore => {}
        }
    }

    /// Write the first lighting frame to every device right away. One-shot CLI engines call
    /// this after a lighting change, since they have no animation loop.
    pub async fn render_once(&self) {
        let mut inner = self.inner.lock().await;
        inner.render_frame(Instant::now()).await;
    }

    /// Draw the current OLED frame on every keyboard with a screen (one-shot CLI engines).
    pub async fn render_oled_once(&self) {
        let mut inner = self.inner.lock().await;
        inner.render_oled(Instant::now()).await;
    }

    /// Persist state now (the daemon also does it periodically and on shutdown).
    pub async fn save(&self) {
        let mut inner = self.inner.lock().await;
        inner.save_state();
    }

    /// Start the daemon loops: hot-plug, lighting animation, status polling, state saving.
    pub fn spawn_background(self: &Arc<Self>) -> BackgroundTasks {
        let mut handles = Vec::new();

        let engine = Arc::clone(self);
        handles.push(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(HOTPLUG_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut tick: u64 = 0;
            loop {
                ticker.tick().await;
                tick += 1;
                if let Err(e) = engine.refresh_devices().await {
                    debug!("Device rescan failed: {e}");
                }
                if tick.is_multiple_of(KEEPALIVE_TICKS) {
                    engine.inner.lock().await.force_resend();
                }
            }
        }));

        let engine = Arc::clone(self);
        handles.push(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(FRAME_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let mut inner = engine.inner.lock().await;
                inner.render_frame(Instant::now()).await;
            }
        }));

        let engine = Arc::clone(self);
        handles.push(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(INPUT_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut tick: u64 = 0;
            loop {
                ticker.tick().await;
                tick += 1;
                engine.input_tick(tick.is_multiple_of(2)).await;
            }
        }));

        let engine = Arc::clone(self);
        handles.push(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(OLED_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let mut inner = engine.inner.lock().await;
                inner.render_oled(Instant::now()).await;
            }
        }));

        let engine = Arc::clone(self);
        handles.push(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(STATUS_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let mut inner = engine.inner.lock().await;
                inner.poll_status(true);
            }
        }));

        let engine = Arc::clone(self);
        handles.push(tokio::spawn(async move {
            let mut watcher = ProcessWatcher::new();
            let mut auto = AutoSwitch::default();
            let mut ticker = tokio::time::interval(AUTOSWITCH_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                engine.inner.lock().await.moments.supervise();
                engine.autoswitch_tick(&mut watcher, &mut auto).await;
            }
        }));

        let engine = Arc::clone(self);
        handles.push(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(SAVE_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let mut inner = engine.inner.lock().await;
                if inner.state_dirty {
                    inner.save_state();
                }
            }
        }));

        BackgroundTasks { handles }
    }

    /// Record a macro from SteelSeries devices. Live bindings are paused meanwhile, because a
    /// device grabbed by the input engine delivers nothing to the recorder.
    async fn record_macro(&self, stop_key: Option<crate::input::InputKey>, timeout_secs: Option<u32>) -> Result<Value> {
        let stop_key = match stop_key {
            Some(key) => key,
            None => crate::input::InputKey::parse("KEY_ESC")?,
        };
        // Below the control client's 60 s request limit.
        let timeout = Duration::from_secs(u64::from(timeout_secs.unwrap_or(30).clamp(1, 55)));
        self.inner.lock().await.input.pause();
        let filter = crate::input::DeviceFilter::steelseries();
        let recorded =
            tokio::task::spawn_blocking(move || crate::input::InputEngine::record_macro(&filter, stop_key, timeout))
                .await
                .map_err(|e| Error::Other(format!("macro recorder task failed: {e}")));
        // Resume with the bindings active *now* (a profile may have switched meanwhile).
        let daemon = self.daemon;
        self.inner.lock().await.input.resume(daemon);
        Ok(json!({ "steps": recorded?? }))
    }

    /// One input/ChatMix step (daemon loop): profile switches requested by bindings, and the
    /// headset dial driving the mixer.
    async fn input_tick(&self, read_dial: bool) {
        let mixer_running = read_dial && self.audio.try_lock().is_ok_and(|mut audio| audio.is_running());
        let (switches, dial) = {
            let mut inner = self.inner.lock().await;
            let mut dial = None;
            if mixer_running {
                for slot in inner.slots.values_mut() {
                    if !matches!(slot.handle, DeviceHandle::Headset(_)) || slot.status.chatmix.is_none() {
                        continue;
                    }
                    if let Some(configurable) = slot.handle.configurable()
                        && let Ok(status) = configurable.read_status()
                    {
                        dial = status.chatmix.or(dial);
                        slot.status = status;
                    }
                }
            }
            (inner.input.poll(), dial)
        };
        if let Some(chatmix) = dial {
            let mut audio = self.audio.lock().await;
            tokio::task::block_in_place(|| audio.follow_dial(chatmix));
        }
        for name in switches {
            match self.execute(Command::ProfileLoad { name: name.clone() }).await {
                Ok(_) => notify::send("Profile switched", &name, Urgency::Low),
                Err(e) => warn!("Binding asked for profile {name}: {e}"),
            }
        }
    }

    /// One automatic-profile-switching step (daemon loop).
    async fn autoswitch_tick(&self, watcher: &mut ProcessWatcher, auto: &mut AutoSwitch) {
        let (rules, current, fallback, enabled, notify_on_switch) = {
            let inner = self.inner.lock().await;
            let rules: Vec<Rule> = inner
                .profiles
                .as_ref()
                .map(|p| {
                    let mut rules: Vec<Rule> = p
                        .all()
                        .values()
                        .filter(|profile| !profile.apps.is_empty())
                        .map(|profile| Rule {
                            profile: profile.name.clone(),
                            apps: profile.apps.clone(),
                        })
                        .collect();
                    rules.sort_by(|a, b| a.profile.cmp(&b.profile));
                    rules
                })
                .unwrap_or_default();
            (
                rules,
                inner.state.active_profile.clone(),
                inner.config.default_profile.clone(),
                inner.config.autoswitch.enabled,
                inner.config.autoswitch.notify,
            )
        };
        let rules = if !enabled || rules.is_empty() {
            if auto.auto_active().is_none() {
                return;
            }
            // Switching was turned off (or its rules removed) while a program's profile was
            // active: deciding with no rules restores the previous profile.
            Vec::new()
        } else {
            rules
        };
        let running = tokio::task::block_in_place(|| watcher.running());
        let Some(target) = auto.decide(current.as_deref(), &rules, &running, fallback.as_deref()) else {
            return;
        };
        match self.execute(Command::ProfileLoad { name: target.clone() }).await {
            Ok(_) => {
                info!("Auto-switched to profile {target}");
                if notify_on_switch {
                    notify::send("Profile switched", &target, Urgency::Low);
                }
            }
            Err(e) => warn!("Automatic switch to {target} failed: {e}"),
        }
    }

    /// Latest status reading for every device that reports one, keyed by device key.
    pub async fn statuses(&self) -> BTreeMap<String, DeviceStatus> {
        let inner = self.inner.lock().await;
        inner
            .slots
            .iter()
            .filter(|(_, slot)| !slot.status.is_empty())
            .map(|(key, slot)| (key.clone(), slot.status.clone()))
            .collect()
    }

    /// Run `f` with exclusive access to one keyboard (used by OLED and diagnostics code).
    pub async fn with_keyboard<R>(
        &self,
        selector: &str,
        f: impl AsyncFnOnce(&mut dyn Keyboard) -> Result<R>,
    ) -> Result<R> {
        let mut inner = self.inner.lock().await;
        let key = inner.resolve(selector)?;
        let slot = inner
            .slots
            .get_mut(&key)
            .ok_or_else(|| Error::DeviceNotFound(selector.to_string()))?;
        match &mut slot.handle {
            DeviceHandle::Keyboard(keyboard) => f(keyboard.as_mut()).await,
            _ => Err(Error::InvalidConfig(format!("{} is not a keyboard", slot.info.name))),
        }
    }
}

/// Stable identity for a physical device: vendor, product and serial (when present).
fn device_key(info: &DeviceInfo) -> String {
    let serial = info
        .serial_number
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("0");
    format!("{:04x}:{:04x}:{}", info.vendor_id, info.product_id, serial)
}

/// Identity of a device model, used by profiles so they apply to any unit of that model.
fn model_key(info: &DeviceInfo) -> String {
    format!("{:04x}:{:04x}", info.vendor_id, info.product_id)
}

fn kind_from_selector(selector: &str) -> Option<DeviceType> {
    match selector.to_ascii_lowercase().as_str() {
        "keyboard" | "kb" => Some(DeviceType::Keyboard),
        "mouse" | "mice" => Some(DeviceType::Mouse),
        "headset" | "headphones" => Some(DeviceType::Headset),
        _ => None,
    }
}

impl Inner {
    async fn sync_devices(&mut self) -> Result<()> {
        self.manager.refresh()?;

        let mut present: Vec<(String, DeviceInfo)> = Vec::new();
        let mut taken: BTreeSet<String> = BTreeSet::new();
        for kind in [DeviceType::Keyboard, DeviceType::Mouse, DeviceType::Headset] {
            for info in self.manager.devices_by_type(kind) {
                let base = device_key(info);
                let mut key = base.clone();
                let mut n = 2;
                while taken.contains(&key) {
                    key = format!("{base}#{n}");
                    n += 1;
                }
                taken.insert(key.clone());
                present.push((key, info.clone()));
            }
        }

        let before = self.slots.len();
        self.slots.retain(|key, slot| {
            let keep = taken.contains(key);
            if !keep {
                info!("Disconnected: {} ({key})", slot.info.name);
            }
            keep
        });
        self.open_failures.retain(|key, _| taken.contains(key));
        let removed = before != self.slots.len();

        for (key, info) in present {
            if self.slots.contains_key(&key) {
                continue;
            }
            match self.open(&info) {
                Ok(handle) => {
                    self.open_failures.remove(&key);
                    let oled_size = match &handle {
                        DeviceHandle::Keyboard(keyboard) => keyboard.oled_size(),
                        _ => None,
                    };
                    let zone_names = match &handle {
                        DeviceHandle::Mouse(mouse) => mouse.color_zone_names(),
                        _ => Vec::new(),
                    };
                    let mut slot = Slot {
                        oled_size,
                        last_oled: None,
                        zone_names,
                        last_per_key: false,
                        oled_failures: 0,
                        last_write_at: None,
                        slow_writes: false,
                        model_key: model_key(&info),
                        info,
                        handle,
                        lighting: None,
                        last_frame: Vec::new(),
                        key_colors: Vec::new(),
                        status: DeviceStatus::default(),
                        status_read_at: None,
                        battery_watch: BatteryWatch::default(),
                        write_failures: 0,
                    };
                    self.restore(&key, &mut slot);
                    info!("Connected: {} ({key})", slot.info.name);
                    self.slots.insert(key, slot);
                }
                Err(e) => {
                    let message = describe_open_error(&info, &e);
                    if self.open_failures.get(&key) != Some(&message) {
                        warn!("{message}");
                        self.open_failures.insert(key, message);
                    }
                }
            }
        }
        if removed {
            self.overlays.retain(|o| match &o.target {
                OverlayTarget::Device(key) => taken.contains(key),
                OverlayTarget::Kind(_) => true,
            });
        }
        Ok(())
    }

    fn open(&self, info: &DeviceInfo) -> Result<DeviceHandle> {
        let mut handle = match info.device_type {
            DeviceType::Keyboard => DeviceHandle::Keyboard(self.manager.open_keyboard(info)?),
            DeviceType::Mouse => DeviceHandle::Mouse(self.manager.open_mouse(info)?),
            DeviceType::Headset => DeviceHandle::Headset(self.manager.open_headset(info)?),
            DeviceType::Unknown => {
                return Err(Error::UnsupportedDevice {
                    vendor_id: info.vendor_id,
                    product_id: info.product_id,
                });
            }
        };
        match &mut handle {
            DeviceHandle::Keyboard(d) => d.initialize()?,
            DeviceHandle::Mouse(d) => d.initialize()?,
            DeviceHandle::Headset(d) => d.initialize()?,
        }
        Ok(handle)
    }

    /// Re-apply remembered settings the device does not keep itself, and set up its lighting.
    fn restore(&mut self, key: &str, slot: &mut Slot) {
        // A unit seen for the first time while a profile is active starts from that profile.
        if !self.state.devices.contains_key(key)
            && let Some(device_profile) = self
                .state
                .active_profile
                .as_ref()
                .and_then(|name| self.profiles.as_ref()?.get(name))
                .and_then(|profile| profile.devices.get(&slot.model_key))
                .cloned()
        {
            let record = self.state.record_mut(key, &slot.info.name);
            record.lighting = device_profile.lighting;
            record.settings = device_profile.settings;
            record.reapply_all = true;
            self.state_dirty = true;
        }

        let lighting = self.state.effective_lighting(key).clone();
        slot.configure_lighting(&lighting);

        let Some(record) = self.state.devices.get_mut(key) else {
            return;
        };
        let apply_all = std::mem::take(&mut record.reapply_all);
        let settings = record.settings.clone();
        if apply_all {
            self.state_dirty = true;
        }
        let descriptors = slot.handle.descriptors();
        let Some(configurable) = slot.handle.configurable() else {
            return;
        };
        for (id, value) in &settings {
            let descriptor = match crate::devices::settings::validate_against(&descriptors, id, value) {
                Ok(descriptor) => descriptor,
                Err(e) => {
                    debug!("Stored {id} not restored on {}: {e}", slot.info.name);
                    continue;
                }
            };
            if matches!(descriptor.kind, SettingKind::Action) || (descriptor.persists_on_device && !apply_all) {
                continue;
            }
            if let Err(e) = configurable.apply_setting(id, value) {
                warn!("Could not restore {id} on {}: {e}", slot.info.name);
            }
        }
    }

    fn push_overlay(&mut self, overlay: Overlay) {
        // A newer colour for the same LEDs from the same owner replaces the older one; for key
        // lists this is per key, so animated frames never pile up.
        let same_owner =
            |o: &Overlay| o.target == overlay.target && o.source == overlay.source && o.event == overlay.event;
        match &overlay.zone {
            OverlayZone::Keys(keys) => {
                for old in self.overlays.iter_mut().filter(|o| same_owner(o)) {
                    if let OverlayZone::Keys(old_keys) = &mut old.zone {
                        old_keys.retain(|k| !keys.contains(k));
                    }
                }
                self.overlays
                    .retain(|o| !matches!(&o.zone, OverlayZone::Keys(keys) if keys.is_empty()));
            }
            OverlayZone::All => self.overlays.retain(|o| !same_owner(o)),
            zone => self.overlays.retain(|o| !(same_owner(o) && &o.zone == zone)),
        }
        self.overlays.push(overlay);
        if self.overlays.len() > MAX_OVERLAYS {
            let excess = self.overlays.len() - MAX_OVERLAYS;
            self.overlays.drain(..excess);
        }
    }

    /// Forget what was last sent so every device gets its full frame again.
    fn force_resend(&mut self) {
        for slot in self.slots.values_mut() {
            slot.last_frame.clear();
            slot.last_oled = None;
        }
    }

    /// Drop devices whose writes keep failing, so the next scan reopens them and restores
    /// their settings (quick replug, suspend/resume, usbhid re-bind).
    fn drop_dead_slots(&mut self) {
        let dead: Vec<String> = self
            .slots
            .iter()
            .filter(|(_, s)| s.write_failures >= MAX_WRITE_FAILURES || s.oled_failures >= MAX_WRITE_FAILURES)
            .map(|(k, _)| k.clone())
            .collect();
        for key in dead {
            if let Some(slot) = self.slots.remove(&key) {
                let count = self.reconnects.entry(key.clone()).or_insert(0);
                *count += 1;
                if *count <= 3 {
                    info!("Lost contact with {} ({key}); reopening", slot.info.name);
                } else {
                    debug!("Lost contact with {} ({key}) again", slot.info.name);
                }
            }
        }
    }

    fn resolve(&self, selector: &str) -> Result<String> {
        let selector = selector.trim();
        if self.slots.contains_key(selector) {
            return Ok(selector.to_string());
        }
        let matches: Vec<&String> = if let Some(kind) = kind_from_selector(selector) {
            self.slots
                .iter()
                .filter(|(_, slot)| slot.handle.kind() == kind)
                .map(|(key, _)| key)
                .collect()
        } else {
            let needle = selector.to_ascii_lowercase();
            self.slots
                .iter()
                .filter(|(key, slot)| slot.info.name.to_ascii_lowercase().contains(&needle) || key.starts_with(&needle))
                .map(|(key, _)| key)
                .collect()
        };
        match matches.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(Error::DeviceNotFound(format!(
                "no connected device matches '{selector}'. Connected: {}",
                self.connected_list()
            ))),
            [first, ..]
                if matches
                    .iter()
                    .all(|k| self.slots[*k].model_key == self.slots[*first].model_key) =>
            {
                Ok((*first).clone())
            }
            _ => Err(Error::InvalidConfig(format!(
                "'{selector}' matches several devices; use a device key: {}",
                matches.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")
            ))),
        }
    }

    fn connected_list(&self) -> String {
        if self.slots.is_empty() {
            return "none".to_string();
        }
        self.slots
            .iter()
            .map(|(key, slot)| format!("{} [{key}]", slot.name()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    async fn execute(&mut self, command: Command, daemon: bool) -> Result<Value> {
        match command {
            Command::Ping => Ok(json!("pong")),
            Command::Status => {
                if !daemon {
                    self.poll_status(false);
                }
                Ok(serde_json::to_value(self.snapshot(daemon))?)
            }
            Command::Refresh => {
                self.sync_devices().await?;
                Ok(serde_json::to_value(self.snapshot(daemon))?)
            }
            Command::Settings { device } => {
                let key = self.resolve(&device)?;
                Ok(serde_json::to_value(self.settings_view(&key))?)
            }
            Command::Set { device, setting, value } => {
                let key = self.resolve(&device)?;
                self.apply_setting(&key, &setting, value)?;
                Ok(json!({ "device": key, "setting": setting }))
            }
            Command::SetText { device, setting, value } => {
                let key = self.resolve(&device)?;
                let descriptors = self.slots[&key].handle.descriptors();
                let descriptor = descriptors.iter().find(|d| d.id == setting).ok_or_else(|| {
                    let ids: Vec<&str> = descriptors.iter().map(|d| d.id.as_str()).collect();
                    Error::Unsupported(format!(
                        "{} has no setting '{setting}'. Available: {}",
                        self.slots[&key].name(),
                        if ids.is_empty() {
                            "none".to_string()
                        } else {
                            ids.join(", ")
                        }
                    ))
                })?;
                let parsed = descriptor.parse_value(&value)?;
                self.apply_setting(&key, &setting, parsed)?;
                Ok(json!({ "device": key, "setting": setting }))
            }
            Command::Lighting {
                device,
                effect,
                brightness,
                follow_global,
            } => {
                self.set_lighting(device.as_deref(), effect, brightness, follow_global)?;
                Ok(serde_json::to_value(self.snapshot(daemon))?)
            }
            Command::KeyColors { device, colors, clear } => {
                let keys = match device {
                    Some(selector) => vec![self.resolve(&selector)?],
                    None => self
                        .slots
                        .iter()
                        .filter(|(_, s)| s.handle.supports_per_key())
                        .map(|(k, _)| k.clone())
                        .collect(),
                };
                if keys.is_empty() {
                    return Err(Error::Unsupported(
                        "no connected keyboard supports per-key lighting".to_string(),
                    ));
                }
                for key in &keys {
                    let slot = self
                        .slots
                        .get_mut(key)
                        .ok_or_else(|| Error::DeviceNotFound(key.clone()))?;
                    let DeviceHandle::Keyboard(keyboard) = &mut slot.handle else {
                        return Err(Error::Unsupported(format!("{} is not a keyboard", slot.info.name)));
                    };
                    if clear {
                        slot.key_colors.clear();
                        slot.last_frame.clear();
                    } else {
                        for (key_id, color) in &colors {
                            slot.key_colors.retain(|(k, _)| k != key_id);
                            slot.key_colors.push((*key_id, *color));
                        }
                        keyboard.set_key_colors(&slot.key_colors).await?;
                    }
                }
                Ok(json!({ "devices": keys }))
            }
            Command::ProfileList => Ok(serde_json::to_value(self.profile_list())?),
            Command::ProfileSave { name, description } => {
                self.save_profile(&name, description, None)?;
                Ok(json!({ "saved": name }))
            }
            Command::ProfilePull { device, name, slot } => {
                let label = self.pull_profile(device.as_deref(), name.clone(), slot)?;
                Ok(json!({ "profile": label }))
            }
            Command::ProfileLoad { name } => {
                self.load_profile(&name)?;
                Ok(serde_json::to_value(self.snapshot(daemon))?)
            }
            Command::ProfileDelete { name } => {
                let profiles = self.profiles_mut()?;
                profiles.delete(&name)?;
                if self.state.active_profile.as_deref() == Some(name.as_str()) {
                    self.state.active_profile = None;
                    self.state_dirty = true;
                }
                Ok(json!({ "deleted": name }))
            }
            Command::Oled {
                content,
                seconds,
                idle,
                clear,
            } => {
                let Some((width, height)) = self.slots.values().find_map(|s| s.oled_size) else {
                    return Err(Error::Unsupported(
                        "no connected keyboard has an OLED screen".to_string(),
                    ));
                };
                let bitmap_size = match (&content, &idle) {
                    (Some(screen::ScreenContent::Bitmap { width, height, .. }), _)
                    | (_, Some(screen::IdleScreen::Bitmap { width, height, .. })) => Some((*width, *height)),
                    _ => None,
                };
                if let Some(size) = bitmap_size
                    && size != (width, height)
                {
                    return Err(Error::InvalidConfig(format!(
                        "bitmap is {}x{}, the screen is {width}x{height}",
                        size.0, size.1
                    )));
                }
                if let Some(idle) = idle {
                    self.screens.set_idle(idle.clone())?;
                    self.state.oled_idle = idle;
                    self.state_dirty = true;
                }
                if clear {
                    self.screens.clear_temporary();
                }
                if let Some(content) = content {
                    let seconds = seconds.unwrap_or(OLED_DEFAULT_SECONDS);
                    let duration = (seconds > 0).then(|| Duration::from_secs(u64::from(seconds)));
                    self.screens.show(&content, duration)?;
                }
                for slot in self.slots.values_mut() {
                    slot.last_oled = None;
                }
                Ok(json!({ "idle": self.state.oled_idle }))
            }
            Command::Mixer { .. } => Err(Error::Other("mixer commands are handled by the engine".to_string())),
            Command::Bindings { profile } => {
                let name = match profile {
                    Some(name) => Some(name),
                    None => self.state.active_profile.clone(),
                };
                let set = match &name {
                    Some(name) => {
                        let profile = self
                            .profiles
                            .as_ref()
                            .and_then(|p| p.get(name))
                            .ok_or_else(|| Error::Profile(format!("profile '{name}' not found")))?;
                        bindings::bindings_from_profile(profile.bindings.as_ref())?
                    }
                    None => crate::input::BindingSet::default(),
                };
                Ok(json!({
                    "profile": name,
                    "bindings": set,
                    "running": self.input.is_running(),
                    "error": self.input.last_error(),
                }))
            }
            Command::Bind { profile, binding } => {
                let name = self.bindings_profile(profile)?;
                self.edit_bindings(&name, |set| {
                    set.insert(binding);
                })?;
                Ok(json!({ "profile": name }))
            }
            Command::Unbind { profile, source } => {
                let name = self.bindings_profile(profile)?;
                self.edit_bindings(&name, |set| {
                    set.remove(source);
                })?;
                Ok(json!({ "profile": name }))
            }
            Command::MacroRecord { .. } => Err(Error::Other("macro recording is handled by the engine".to_string())),
            Command::MomentsSave => {
                let dir = self.moments.save_clip()?;
                notify::send("Clip saved", &dir.to_string_lossy(), Urgency::Low);
                Ok(json!({ "saved_to": dir }))
            }
            Command::MomentsStatus => Ok(serde_json::to_value(self.moments.status())?),
            Command::MomentsEnable {
                enabled,
                replay_seconds,
            } => {
                let mut config = self.moments.config().clone();
                config.enabled = enabled;
                if let Some(seconds) = replay_seconds {
                    config.replay_seconds = seconds.clamp(5, 1200);
                }
                self.moments.set_config(config.clone());
                if daemon && enabled {
                    self.moments.supervise();
                }
                // Remembered in engine-state.json; config.toml stays as the user wrote it.
                self.state.moments = Some(config);
                self.state_dirty = true;
                Ok(serde_json::to_value(self.moments.status())?)
            }
            Command::ProfileApps { name, apps } => {
                let profiles = self.profiles_mut()?;
                let mut profile = profiles
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| Error::Profile(format!("profile '{name}' not found")))?;
                profile.apps = apps
                    .into_iter()
                    .map(|a| a.trim().to_string())
                    .filter(|a| !a.is_empty())
                    .collect();
                profiles.set(profile)?;
                Ok(json!({ "profile": name }))
            }
        }
    }

    fn apply_setting(&mut self, key: &str, setting: &str, value: SettingValue) -> Result<()> {
        let slot = self
            .slots
            .get_mut(key)
            .ok_or_else(|| Error::DeviceNotFound(key.to_string()))?;
        let descriptors = slot.handle.descriptors();
        let descriptor = crate::devices::settings::validate_against(&descriptors, setting, &value)?;
        let is_action = matches!(descriptor.kind, SettingKind::Action);
        let name = slot.info.name.to_string();
        let configurable = slot
            .handle
            .configurable()
            .ok_or_else(|| Error::Unsupported(format!("{name} has no configurable settings")))?;
        configurable.apply_setting(setting, &value)?;
        if !is_action {
            self.state
                .record_mut(key, &name)
                .settings
                .insert(setting.to_string(), value);
            self.state_dirty = true;
        }
        Ok(())
    }

    fn set_lighting(
        &mut self,
        device: Option<&str>,
        effect: Option<Effect>,
        brightness: Option<u8>,
        follow_global: bool,
    ) -> Result<()> {
        if let Some(b) = brightness
            && b > 100
        {
            return Err(Error::InvalidConfig(format!("brightness {b} is above 100")));
        }
        match device {
            None => {
                if let Some(effect) = effect {
                    self.state.lighting.effect = effect;
                }
                if let Some(b) = brightness {
                    self.state.lighting.brightness = b;
                }
            }
            Some(selector) => {
                let key = self.resolve(selector)?;
                let name = self.slots[&key].info.name.to_string();
                let global = self.state.lighting.clone();
                let record = self.state.record_mut(&key, &name);
                if follow_global {
                    record.lighting = None;
                } else {
                    let lighting = record.lighting.get_or_insert(global);
                    if let Some(effect) = effect {
                        lighting.effect = effect;
                    }
                    if let Some(b) = brightness {
                        lighting.brightness = b;
                    }
                }
            }
        }
        self.state_dirty = true;
        let keys: Vec<String> = self.slots.keys().cloned().collect();
        for key in keys {
            let lighting = self.state.effective_lighting(&key).clone();
            if let Some(slot) = self.slots.get_mut(&key) {
                slot.configure_lighting(&lighting);
            }
        }
        Ok(())
    }

    /// Compute and send one lighting frame to every device whose colours changed.
    async fn render_frame(&mut self, now: Instant) {
        self.overlays.retain(|o| o.expires > now);
        let overlays = self.overlays.clone();
        let epoch = self.epoch;
        for (key, slot) in self.slots.iter_mut() {
            let Some(controller) = slot.lighting.as_mut() else {
                continue;
            };
            let mut colors = controller.compute_colors().to_vec();
            let kind = slot.handle.kind();
            let mut key_layer: Vec<(KeyId, Color)> = slot.key_colors.clone();
            for overlay in overlays.iter().filter(|o| o.applies_to(key, kind)) {
                let color = overlay.color_at(now, epoch);
                match &overlay.zone {
                    OverlayZone::All => {
                        colors.iter_mut().for_each(|c| *c = color);
                        key_layer.clear();
                    }
                    OverlayZone::Index(zone) => {
                        if let Some(c) = colors.get_mut(*zone) {
                            *c = color;
                        }
                    }
                    OverlayZone::Named(name) => {
                        let needle = name.to_ascii_lowercase();
                        match slot
                            .zone_names
                            .iter()
                            .position(|z| z.to_ascii_lowercase().contains(&needle))
                        {
                            Some(zone) if zone < colors.len() => colors[zone] = color,
                            _ => colors.iter_mut().for_each(|c| *c = color),
                        }
                    }
                    OverlayZone::Keys(keys) => {
                        for key_id in keys {
                            match key_layer.iter_mut().find(|(k, _)| k == key_id) {
                                Some(entry) => entry.1 = color,
                                None => key_layer.push((*key_id, color)),
                            }
                        }
                    }
                }
            }

            let is_mouse = matches!(slot.handle, DeviceHandle::Mouse(_));
            if is_mouse {
                let spacing = if slot.slow_writes {
                    SLOW_MOUSE_FRAME_INTERVAL
                } else {
                    MOUSE_FRAME_INTERVAL
                };
                if slot.last_write_at.is_some_and(|at| now.duration_since(at) < spacing) {
                    continue;
                }
            }
            let write_started = Instant::now();
            let per_key = !key_layer.is_empty() && slot.handle.supports_per_key();
            let result = if per_key {
                let DeviceHandle::Keyboard(keyboard) = &mut slot.handle else {
                    continue;
                };
                // Every mapped key gets the effect colour, then the per-key layer on top.
                let base = colors.first().copied().unwrap_or(Color::BLACK);
                let mut frame: Vec<(KeyId, Color)> = keyboard
                    .get_key_mapping()
                    .map(|m| m.get_all_keys().iter().map(|k| (*k, base)).collect())
                    .unwrap_or_default();
                for (key_id, color) in &key_layer {
                    match frame.iter_mut().find(|(k, _)| k == key_id) {
                        Some(entry) => entry.1 = *color,
                        None => frame.push((*key_id, *color)),
                    }
                }
                let signature: Vec<Color> = frame.iter().map(|(_, c)| *c).collect();
                if slot.last_per_key && signature == slot.last_frame {
                    continue;
                }
                keyboard.set_all_key_colors(&frame).await.map(|()| (signature, true))
            } else {
                if !slot.last_per_key && colors == slot.last_frame {
                    continue;
                }
                slot.handle.write_zones(&colors).await.map(|()| (colors, false))
            };
            slot.last_write_at = Some(now);
            if is_mouse && write_started.elapsed() > Duration::from_millis(50) {
                slot.slow_writes = true;
            }
            match result {
                Ok((sent, was_per_key)) => {
                    slot.last_frame = sent;
                    slot.last_per_key = was_per_key;
                    slot.write_failures = 0;
                }
                Err(e) => {
                    slot.write_failures += 1;
                    if slot.write_failures == 1 {
                        warn!("Lighting write to {} failed: {e}", slot.info.name);
                    }
                }
            }
        }
        self.drop_dead_slots();
    }

    /// Send the current OLED frame to every keyboard with a screen, when it changed.
    async fn render_oled(&mut self, now: Instant) {
        if !self.slots.values().any(|s| s.oled_size.is_some()) {
            return;
        }
        let frame = self.screens.frame(now);
        for slot in self.slots.values_mut() {
            if slot.oled_size.is_none() || slot.last_oled == frame {
                continue;
            }
            let Some(frame) = &frame else {
                // Nothing to show: leave the keyboard's last image (no command hands it back).
                slot.last_oled = None;
                continue;
            };
            let DeviceHandle::Keyboard(keyboard) = &mut slot.handle else {
                continue;
            };
            match keyboard.draw_oled(frame).await {
                Ok(()) => {
                    slot.last_oled = Some(frame.clone());
                    slot.oled_failures = 0;
                }
                Err(e) => {
                    slot.oled_failures += 1;
                    if slot.oled_failures == 1 {
                        warn!("OLED write to {} failed: {e}", slot.info.name);
                    }
                }
            }
        }
        self.drop_dead_slots();
    }

    /// Refresh battery / ChatMix readings. `notify` raises desktop notifications for low battery.
    fn poll_status(&mut self, notify: bool) {
        for slot in self.slots.values_mut() {
            let Some(configurable) = slot.handle.configurable() else {
                continue;
            };
            match configurable.read_status() {
                Ok(status) => {
                    if notify
                        && let Some(percent) = status.battery_percent
                        && let Some(urgency) = slot.battery_watch.observe(percent, status.charging.unwrap_or(false))
                    {
                        notify::send(
                            &format!("{} battery low", slot.info.name),
                            &format!("{percent}% remaining"),
                            urgency,
                        );
                    }
                    slot.status = status;
                    slot.status_read_at = Some(Instant::now());
                }
                Err(e) => debug!("Status read from {} failed: {e}", slot.info.name),
            }
        }
    }

    fn snapshot(&self, daemon: bool) -> EngineSnapshot {
        let devices = self
            .slots
            .iter()
            .map(|(key, slot)| DeviceSnapshot {
                key: key.clone(),
                name: slot.info.name.to_string(),
                kind: slot.handle.kind(),
                product_id: slot.info.product_id,
                status: slot.status.clone(),
                lighting: slot
                    .lighting
                    .as_ref()
                    .map(|_| self.state.effective_lighting(key).clone()),
                follows_global_lighting: self.state.devices.get(key).is_none_or(|r| r.lighting.is_none()),
                capabilities: slot.capabilities(),
            })
            .collect();
        EngineSnapshot {
            version: env!("CARGO_PKG_VERSION").to_string(),
            daemon,
            devices,
            lighting: self.state.lighting.clone(),
            active_profile: self.state.active_profile.clone(),
            oled_idle: self.state.oled_idle.clone(),
            warnings: self.open_failures.values().cloned().collect(),
        }
    }

    fn settings_view(&self, key: &str) -> SettingsView {
        let slot = &self.slots[key];
        let record = self.state.devices.get(key);
        let settings = slot
            .handle
            .descriptors()
            .into_iter()
            .map(|descriptor| {
                let value = record
                    .and_then(|r| r.settings.get(&descriptor.id).cloned())
                    .or_else(|| descriptor.default.clone());
                SettingEntry { descriptor, value }
            })
            .collect();
        SettingsView {
            device: key.to_string(),
            name: slot.info.name.to_string(),
            settings,
        }
    }

    fn profiles_mut(&mut self) -> Result<&mut ProfileManager> {
        self.profiles
            .as_mut()
            .ok_or_else(|| Error::Profile("profile storage is unavailable".to_string()))
    }

    fn profile_list(&self) -> Vec<ProfileSummary> {
        let Some(profiles) = &self.profiles else {
            return Vec::new();
        };
        let mut list: Vec<ProfileSummary> = profiles
            .all()
            .values()
            .map(|p| ProfileSummary {
                name: p.name.clone(),
                description: p.description.clone(),
                apps: p.apps.clone(),
                active: self.state.active_profile.as_deref() == Some(p.name.as_str()),
            })
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Profile whose bindings a `bind`/`unbind` edits: the named one, else the active one, else
    /// a new "Default" profile that becomes active.
    fn bindings_profile(&mut self, profile: Option<String>) -> Result<String> {
        if let Some(name) = profile.or_else(|| self.state.active_profile.clone()) {
            return Ok(name);
        }
        let name = "Default".to_string();
        let profiles = self.profiles_mut()?;
        if profiles.get(&name).is_none() {
            profiles.set(Profile::new(name.clone()))?;
        }
        self.state.active_profile = Some(name.clone());
        self.state_dirty = true;
        Ok(name)
    }

    fn edit_bindings(&mut self, name: &str, edit: impl FnOnce(&mut crate::input::BindingSet)) -> Result<()> {
        let daemon = self.daemon;
        let active = self.state.active_profile.as_deref() == Some(name);
        let profiles = self.profiles_mut()?;
        let mut profile = profiles
            .get(name)
            .cloned()
            .ok_or_else(|| Error::Profile(format!("profile '{name}' not found")))?;
        let mut set = bindings::bindings_from_profile(profile.bindings.as_ref())?;
        edit(&mut set);
        set.validate()?;
        profile.bindings = if set.bindings.is_empty() {
            None
        } else {
            Some(serde_json::to_value(&set)?)
        };
        profiles.set(profile)?;
        if active {
            self.input.apply(set, daemon);
        }
        Ok(())
    }

    /// Read one keyboard's own memory and save what it holds as a profile: the name the device
    /// spells inside its payload and the bindings its payload carry. Lighting and every device's
    /// settings come from what the engine sees now, since reading that memory brings back only
    /// bindings and a name, and this fork has never watched a reply for the lighting a profile also
    /// holds (`lighting_profile` at bytes 27 to 83 of the payload has no reader here).
    ///
    /// `device` is needed once more than one keyboard is connected; `slot` picks the onboard
    /// profile to read through the keyboard's own `onboard-slot` setting, the same one a keybinding
    /// rewrite writes into.
    fn pull_profile(&mut self, device: Option<&str>, name: Option<String>, slot: Option<u8>) -> Result<String> {
        let keys = match device {
            Some(selector) => vec![self.resolve(&selector)?],
            None => self
                .slots
                .iter()
                .filter(|(_, s)| matches!(s.handle, DeviceHandle::Keyboard(_)))
                .map(|(k, _)| k.clone())
                .collect(),
        };
        let Some(found) = keys.first() else {
            return Err(Error::Unsupported(
                "no connected keyboard holds its own configuration, so there is nothing to read back".to_string(),
            ));
        };
        let key = found.clone();
        if keys.len() > 1 {
            return Err(Error::Unsupported(format!(
                "{} keyboards are connected, name the one to read",
                keys.len()
            )));
        }
        if let Some(number) = slot {
            self.apply_setting(&key, "onboard-slot", SettingValue::Choice(format!("{}", number)))?;
        }
        let slot_ref = self.slots.get_mut(&key).ok_or_else(|| Error::DeviceNotFound(key))?;
        let DeviceHandle::Keyboard(keyboard) = &mut slot_ref.handle else {
            return Err(Error::Unsupported(format!(
                "{} holds no configuration of its own",
                slot_ref.info.name
            )));
        };
        let found = keyboard.as_mut().pull_key_bindings()?;
        found.bindings.validate()?;
        let device_name = slot_ref.info.name.to_string();
        let label = name
            .clone()
            .or_else(|| found.name.clone())
            .unwrap_or_else(|| format!("{} has no name for it", device_name));
        let description = format!("read from the keyboard's own memory of {device_name}");
        self.save_profile(&label, Some(description), Some(&found.bindings))?;
        info!(
            "Read {} key bindings back from {} into profile {}",
            found.bindings.bindings.len(),
            device_name,
            label,
        );
        return Ok(label);
    }

    /// Snapshot the current state of every connected device into a profile. `pulled` is the set of
    /// bindings read off a keyboard's own memory: a profile that carries one takes it over, since
    /// what the device holds is the device's own, not the engine's current set.
    fn save_profile(
        &mut self,
        name: &str,
        description: Option<String>,
        pulled: Option<&crate::input::BindingSet>,
    ) -> Result<()> {
        let mut devices = BTreeMap::new();
        for (key, slot) in &self.slots {
            let record = self.state.devices.get(key);
            let descriptors = slot.handle.descriptors();
            let settings: BTreeMap<String, SettingValue> = record
                .map(|r| {
                    r.settings
                        .iter()
                        .filter(|(id, _)| {
                            descriptors
                                .iter()
                                .any(|d| &d.id == *id && !matches!(d.kind, SettingKind::Action))
                        })
                        .map(|(id, v)| (id.clone(), v.clone()))
                        .collect()
                })
                .unwrap_or_default();
            devices.insert(
                slot.model_key.clone(),
                DeviceProfile {
                    name: slot.info.name.to_string(),
                    settings,
                    lighting: record.and_then(|r| r.lighting.clone()),
                },
            );
        }
        let lighting = self.state.lighting.clone();
        let mixer = self.mixer_snapshot.take();
        let bindings = match pulled {
            Some(set) => {
                if set.is_empty() {
                    None
                } else {
                    Some(serde_json::to_value(set)?)
                }
            }
            None => {
                if self.input.active().bindings.is_empty() {
                    None
                } else {
                    Some(serde_json::to_value(self.input.active())?)
                }
            }
        };
        let profiles = self.profiles_mut()?;
        let mut profile = profiles.get(name).cloned().unwrap_or_else(|| Profile::new(name));
        if description.is_some() {
            profile.description = description;
        }
        profile.lighting = Some(lighting);
        if mixer.is_some() {
            profile.mixer = mixer;
        }
        if profile.bindings.is_none() || pulled.is_some() {
            profile.bindings = bindings;
        }
        // Keep entries for models that are not plugged in right now.
        for (model, device_profile) in devices {
            profile.devices.insert(model, device_profile);
        }
        profiles.set(profile)?;
        Ok(())
    }

    fn load_profile(&mut self, name: &str) -> Result<()> {
        let profile = self
            .profiles
            .as_ref()
            .and_then(|p| p.get(name))
            .cloned()
            .ok_or_else(|| Error::Profile(format!("profile '{name}' not found")))?;

        if let Some(lighting) = &profile.lighting {
            self.state.lighting = lighting.clone();
        } else if let Some(keyboard) = &profile.keyboard {
            self.state.lighting = Lighting {
                effect: keyboard.effect.clone(),
                brightness: keyboard.brightness,
            };
        }

        let keys: Vec<String> = self.slots.keys().cloned().collect();
        let mut errors = Vec::new();
        for key in keys {
            let (model, device_name) = {
                let slot = &self.slots[&key];
                (slot.model_key.clone(), slot.info.name.to_string())
            };
            let Some(device_profile) = profile.devices.get(&model) else {
                continue;
            };
            self.state.record_mut(&key, &device_name).lighting = device_profile.lighting.clone();
            for (id, value) in &device_profile.settings {
                if let Err(e) = self.apply_setting(&key, id, value.clone()) {
                    errors.push(format!("{device_name}: {id}: {e}"));
                }
            }
        }
        // Known units of these models that are unplugged get the profile when they return.
        for (model, device_profile) in &profile.devices {
            let prefix = format!("{model}:");
            for (key, record) in self.state.devices.iter_mut() {
                if !key.starts_with(&prefix) || self.slots.contains_key(key) {
                    continue;
                }
                record.lighting = device_profile.lighting.clone();
                record
                    .settings
                    .extend(device_profile.settings.iter().map(|(k, v)| (k.clone(), v.clone())));
                record.reapply_all = true;
            }
        }

        match bindings::bindings_from_profile(profile.bindings.as_ref()) {
            Ok(set) => {
                // A keyboard whose own memory can hold the bindings gets them written there first:
                // the Apex 7 (2023) rewrites its onboard profile FS object, and a
                // keyboard that did that answers the remapped keys itself, so the engine's own
                // remapping - the evdev grab and the uinput replay - stays off for it. Other Apex
                // keyboards have onboard profiles too, but this fork has no decoded payload for
                // them, so they refuse and their bindings stay the engine's.
                let (onboard_errors, all_onboard) = self.write_bindings_to_keyboards(&set);
                errors.extend(onboard_errors);
                if all_onboard {
                    self.input.stop();
                } else {
                    self.input.apply(set, self.daemon);
                }
            }
            Err(e) => errors.push(e.to_string()),
        }
        if let Some(mixer) = &profile.mixer {
            match serde_json::from_value::<crate::mixer::MixerConfig>(mixer.clone()) {
                Ok(config) => self.pending_mixer = Some(config),
                Err(e) => errors.push(format!("mixer settings: {e}")),
            }
        }

        self.state.active_profile = Some(profile.name.clone());
        self.state_dirty = true;
        let keys: Vec<String> = self.slots.keys().cloned().collect();
        for key in keys {
            let lighting = self.state.effective_lighting(&key).clone();
            if let Some(slot) = self.slots.get_mut(&key) {
                slot.configure_lighting(&lighting);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::Profile(format!(
                "profile '{name}' loaded with errors: {}",
                errors.join("; ")
            )))
        }
    }

    /// Push a profile's bindings into every connected keyboard this fork can write them into -
    /// the ones whose onboard payload it has decoded, which so far is the Apex 7 (2023), see
    /// [`keyboards::Keyboard::set_key_bindings`] - and report whether every connected keyboard
    /// did that: such a keyboard answers the remapped keys itself, so the engine's remapping can
    /// stay off for it. A keyboard this fork cannot write is not an error and is not counted:
    /// its bindings stay the engine's.
    ///
    /// The switch is one for the whole engine, not one per keyboard, so a mixed set of connected
    /// keyboards is a limitation rather than a success: a keyboard that took the set into its own
    /// memory also sees the engine's replay, and nothing here has watched a device do that twice.
    /// Such a set is reported in the errors so the user sees it.
    ///
    /// TODO: check what the original Engine does for a binding this model cannot spell. A
    /// complex macro has no onboard record (`src/input/binding.rs::Action::Macro` has no function
    /// octet, and `compile_bindings` says so), so a profile that carries one still needs the
    /// engine's remapping turned back on for that keyboard - the original Engine only ever ran
    /// such macros in software, so the switch may have to come back on for a keyboard that took
    /// the rest of the set onboard. This fork has never watched a device answer a macro it cannot
    /// hold, and leaves the switch off for every keyboard it writes.
    fn write_bindings_to_keyboards(&mut self, set: &crate::input::BindingSet) -> (Vec<String>, bool) {
        let keys: Vec<String> = self.slots.keys().cloned().collect();
        let mut errors = Vec::new();
        let mut keyboards = 0;
        let mut onboard = 0;
        for key in keys {
            if let Some(slot) = self.slots.get_mut(&key) {
                match &mut slot.handle {
                    DeviceHandle::Keyboard(keyboard) => {
                        keyboards += 1;
                        if let Err(e) = keyboard.as_mut().set_key_bindings(set) {
                            errors.push(format!("{key}: {e}"));
                            info!("Key bindings for {} stay with the engine: {e}", slot.info.name);
                        } else {
                            onboard += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        if onboard > 0 && onboard < keyboards {
            errors.push(format!(
                concat!(
                    "{} of {} connected keyboards hold these key bindings in their own memory, so the ",
                    "engine's one switch stays on for all of them and those that hold their own see ",
                    "the bindings twice",
                ),
                onboard, keyboards,
            ));
        }
        return (errors, keyboards > 0 && onboard == keyboards);
    }

    fn save_state(&mut self) {
        match self.state.save() {
            Ok(()) => self.state_dirty = false,
            Err(e) => warn!("Could not save engine state: {e}"),
        }
    }
}

fn describe_open_error(info: &DeviceInfo, error: &Error) -> String {
    let text = error.to_string();
    let permission = text.contains("Permission denied") || text.contains("EACCES") || text.contains("os error 13");
    if permission {
        format!(
            "Cannot open {} ({:04x}:{:04x}): permission denied. Install assets/70-steelseries.rules into \
             /etc/udev/rules.d/, run `sudo udevadm control --reload-rules && sudo udevadm trigger`, then replug it.",
            info.name, info.vendor_id, info.product_id
        )
    } else {
        format!(
            "Cannot open {} ({:04x}:{:04x}): {text}",
            info.name, info.vendor_id, info.product_id
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(pid: u16, serial: Option<&str>) -> DeviceInfo {
        DeviceInfo {
            name: "Test".into(),
            device_type: DeviceType::Keyboard,
            vendor_id: 0x1038,
            product_id: pid,
            interface_number: 1,
            usage_page: 0xFFC0,
            usage: 1,
            serial_number: serial.map(str::to_string),
            manufacturer: None,
            path: "/dev/hidraw3".into(),
        }
    }

    #[test]
    fn device_keys_ignore_the_hidraw_path() {
        let mut a = info(0x1628, Some("ABC"));
        let key = device_key(&a);
        a.path = "/dev/hidraw7".into();
        assert_eq!(device_key(&a), key);
        assert_eq!(key, "1038:1628:ABC");
        assert_eq!(device_key(&info(0x1628, Some("  "))), "1038:1628:0");
        assert_eq!(model_key(&a), "1038:1628");
    }

    #[test]
    fn type_selectors() {
        assert_eq!(kind_from_selector("Keyboard"), Some(DeviceType::Keyboard));
        assert_eq!(kind_from_selector("mouse"), Some(DeviceType::Mouse));
        assert_eq!(kind_from_selector("headset"), Some(DeviceType::Headset));
        assert_eq!(kind_from_selector("apex"), None);
    }
}
