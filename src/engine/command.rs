//! The one request shape every front end sends to the engine.
//!
//! The CLI, the web control panel and the daemon's own loops all speak [`Command`]. When the
//! daemon is running the CLI sends it over the local control API; when it is not, the CLI runs
//! the same command against an in-process engine. Either way the same code executes it.

use serde::{Deserialize, Serialize};

use super::state::Lighting;
use crate::devices::DeviceType;
use crate::devices::key_mapping::KeyId;
use crate::devices::settings::{DeviceStatus, SettingDescriptor, SettingValue};
use crate::rgb::{Color, Effect};

/// A request to the engine.
///
/// `device` fields are selectors: a device key from `status`, a type (`keyboard`, `mouse`,
/// `headset`), or part of the product name (`apex`, `nova pro`).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Liveness check; answers `"pong"`.
    Ping,
    /// Everything at a glance: devices, readings, lighting, profile.
    Status,
    /// Rescan for devices now instead of waiting for the next hot-plug poll.
    Refresh,
    /// Setting descriptors and current values for one device.
    Settings {
        device: String,
    },
    /// Apply a typed value.
    Set {
        device: String,
        setting: String,
        value: SettingValue,
    },
    /// Apply a value written as text; parsed against the setting's descriptor.
    SetText {
        device: String,
        setting: String,
        value: String,
    },
    /// Change lighting. `device: None` changes the global (synced) lighting; `follow_global`
    /// drops a device's own override.
    Lighting {
        device: Option<String>,
        effect: Option<Effect>,
        brightness: Option<u8>,
        #[serde(default)]
        follow_global: bool,
    },
    /// Static per-key colors layered over the effect on per-key keyboards (`clear` removes them).
    KeyColors {
        device: Option<String>,
        colors: Vec<(KeyId, Color)>,
        #[serde(default)]
        clear: bool,
    },
    ProfileList,
    /// Snapshot the current state of every connected device into a profile.
    ProfileSave {
        name: String,
        description: Option<String>,
    },
    /// Read one keyboard's own memory back and save what it holds as a profile. `device` names the
    /// keyboard (required once more than one is connected), `slot` picks the onboard profile to read
    /// through the keyboard's own `onboard-slot` setting, and `name` the profile to write: without it
    /// the profile is called what the device spells inside its own memory.
    ProfilePull {
        #[serde(default)]
        device: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        slot: Option<u8>,
    },
    ProfileLoad {
        name: String,
    },
    ProfileDelete {
        name: String,
    },
    /// Set the applications (process names) that activate a profile automatically.
    ProfileApps {
        name: String,
        apps: Vec<String>,
    },
    /// Keyboard OLED: show `content` for `seconds` (0 = until replaced), change the idle
    /// screen, or `clear` temporary content.
    Oled {
        #[serde(default)]
        content: Option<super::screen::ScreenContent>,
        #[serde(default)]
        seconds: Option<u32>,
        #[serde(default)]
        idle: Option<super::screen::IdleScreen>,
        #[serde(default)]
        clear: bool,
    },
    /// Audio mixer: change settings (`patch`), turn it on/off, and return its state.
    Mixer {
        #[serde(default)]
        patch: Option<super::audio::MixerPatch>,
        #[serde(default)]
        enabled: Option<bool>,
    },
    /// Key bindings of a profile (`None` = the active one).
    Bindings {
        #[serde(default)]
        profile: Option<String>,
    },
    /// Add or replace one binding in a profile (`None` = the active one, created if needed).
    Bind {
        #[serde(default)]
        profile: Option<String>,
        binding: crate::input::Binding,
    },
    /// Remove the binding of one key.
    Unbind {
        #[serde(default)]
        profile: Option<String>,
        source: crate::input::InputKey,
    },
    /// Record key presses on SteelSeries devices until `stop_key` (default Escape) or the
    /// timeout, and return them as macro steps.
    MacroRecord {
        #[serde(default)]
        stop_key: Option<crate::input::InputKey>,
        #[serde(default)]
        timeout_secs: Option<u32>,
    },
    /// Save the instant-replay buffer to a clip.
    MomentsSave,
    /// Replay buffer state.
    MomentsStatus,
    /// Turn the replay buffer on or off (and optionally change its length).
    MomentsEnable {
        enabled: bool,
        replay_seconds: Option<u32>,
    },
}

impl Command {
    /// Commands that only read state; the CLI can answer them without touching devices twice.
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            Command::Ping
                | Command::Status
                | Command::Settings { .. }
                | Command::ProfileList
                | Command::MomentsStatus
                | Command::Bindings { .. }
        )
    }
}

/// One connected device as the front ends see it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceSnapshot {
    pub key: String,
    pub name: String,
    pub kind: DeviceType,
    pub product_id: u16,
    pub status: DeviceStatus,
    /// Lighting in effect for this device (its own, or the global one).
    pub lighting: Option<Lighting>,
    pub follows_global_lighting: bool,
    /// Names of the capabilities this device offers, e.g. `lighting`, `per_key`, `settings`.
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EngineSnapshot {
    pub version: String,
    /// True when answered by the background daemon, false for a one-shot CLI engine.
    pub daemon: bool,
    pub devices: Vec<DeviceSnapshot>,
    pub lighting: Lighting,
    pub active_profile: Option<String>,
    /// Idle screen of OLED keyboards.
    pub oled_idle: super::screen::IdleScreen,
    /// Problems worth showing the user (missing permissions, failed device opens, ...).
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SettingEntry {
    pub descriptor: SettingDescriptor,
    /// Last applied value (devices rarely report their own), or the descriptor's default.
    pub value: Option<SettingValue>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SettingsView {
    pub device: String,
    pub name: String,
    pub settings: Vec<SettingEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProfileSummary {
    pub name: String,
    pub description: Option<String>,
    pub apps: Vec<String>,
    pub active: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_use_a_flat_tagged_json_shape() {
        let cmd = Command::SetText {
            device: "mouse".into(),
            setting: "dpi".into(),
            value: "800,1600".into(),
        };
        let json = serde_json::to_value(&cmd).unwrap();
        assert_eq!(json["cmd"], "set_text");
        assert_eq!(json["device"], "mouse");

        let parsed: Command = serde_json::from_str(r#"{"cmd":"status"}"#).unwrap();
        assert!(matches!(parsed, Command::Status));
        assert!(parsed.is_read_only());
    }
}
