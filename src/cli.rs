use clap::{Parser, Subcommand};

#[cfg(feature = "onboard")]
use std::path::PathBuf;

use steelseries_gg::devices::KeyId;
use steelseries_gg::rgb::Color;
use steelseries_gg::{Error, Result};

#[cfg(feature = "audio")]
use steelseries_gg::audio::Channel;

/// SteelSeries GG for Linux - Control your SteelSeries devices
#[derive(Parser)]
#[command(name = "ssgg")]
#[command(author, version, about, long_about = None)]
pub(crate) struct Cli {
    /// Enable debug logging
    #[arg(short, long)]
    pub(crate) debug: bool,

    /// Enable HID communication debugging and diagnostics
    #[arg(long)]
    pub(crate) debug_hid: bool,

    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// List connected SteelSeries devices
    Devices,

    /// Control RGB lighting
    Rgb {
        #[command(subcommand)]
        action: RgbAction,
    },

    /// Control actuation points on compatible keyboards
    Actuation {
        #[command(subcommand)]
        action: ActuationAction,
    },

    /// Manage profiles
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },

    /// Control audio mixer (Sonar replacement)
    #[cfg(feature = "audio")]
    Audio {
        #[command(subcommand)]
        action: AudioAction,
    },

    /// Control SteelSeries Sonar (direct API access)
    #[cfg(feature = "sonar")]
    Sonar {
        #[command(subcommand)]
        action: SonarAction,
    },

    /// Configure USB polling rate (requires sudo)
    Pollrate {
        #[command(subcommand)]
        action: PollrateAction,
    },

    /// Read the on-board profile of a legacy Apex keyboard (experimental, reads only)
    #[cfg(feature = "onboard")]
    Onboard {
        #[command(subcommand)]
        action: OnboardAction,
    },

    /// Start the GameSense server
    Server {
        /// Port to listen on
        #[arg(short, long, default_value = "27301")]
        port: u16,
    },

    /// Run validation tests on connected devices
    Validate {
        /// Enable performance benchmarks
        #[arg(short, long)]
        benchmark: bool,

        /// Test timeout in seconds
        #[arg(short, long, default_value = "30")]
        timeout: u64,

        /// Export validation report to file
        #[arg(short, long)]
        output: Option<String>,

        /// JSON output format
        #[arg(long)]
        json: bool,
    },

    /// Monitor and control RGB performance optimizations
    Performance {
        #[command(subcommand)]
        action: PerformanceAction,
    },

    /// Generate a comprehensive bug report with diagnostic information
    BugReport {
        /// Output file path
        #[arg(short, long, default_value = "ssgg_bug_report.json")]
        output: String,

        /// Include HID communication logs (if available)
        #[arg(long)]
        include_hid_logs: bool,

        /// Include performance metrics snapshot (not available yet: needs the daemon, currently omitted with a warning)
        #[arg(long)]
        include_performance: bool,
    },

    /// Show real-time device connection status
    Status {
        /// Filter by device type (keyboard, headset, or all)
        #[arg(short, long, default_value = "all")]
        device: String,

        /// Refresh interval in milliseconds
        #[arg(short, long, default_value = "1000")]
        refresh: u64,
    },

    /// View HID communication logs with filtering
    HidLogs {
        /// Enable file logging
        #[arg(short, long)]
        file: bool,

        /// Filter by device type (keyboard, headset, or all)
        #[arg(short, long)]
        device: Option<String>,
    },

    /// Run as a daemon (device control + GameSense server)
    Daemon,

    /// Run automated device tests to verify responsiveness
    TestDevice {
        /// Device name or path to test
        device: String,

        /// Enable performance benchmarks
        #[arg(short, long)]
        benchmark: bool,

        /// Show detailed output including passing tests
        #[arg(short, long)]
        verbose: bool,
    },

    /// Verify RGB performance metrics over time
    VerifyPerformance {
        /// Duration to monitor in seconds
        #[arg(short, long, default_value = "30")]
        duration: u64,

        /// Effect to test (breathing, spectrum, wave, etc.)
        #[arg(short, long, default_value = "breathing")]
        effect: String,

        /// Export metrics to JSON file
        #[arg(short, long)]
        output: Option<String>,
    },

    /// Protocol Fuzzer (Developer Tool)
    #[command(hide = true)]
    Fuzz {
        /// Start command byte
        #[arg(short, long, default_value = "0x00", value_parser = parse_hex_u8)]
        start: u8,

        /// End command byte
        #[arg(short, long, default_value = "0xFF", value_parser = parse_hex_u8)]
        end: u8,

        /// Delay between commands in ms
        #[arg(short, long, default_value = "100")]
        delay: u64,
    },
}

/// [EXPERIMENTAL] Read the on-board profile of a legacy Apex keyboard.
///
/// `plan` and `decode` work on the wire format and on saved files only; they never touch a device.
/// `read` and `schema` talk to a keyboard and need `--yes`.
#[cfg(feature = "onboard")]
#[derive(Subcommand)]
pub(crate) enum OnboardAction {
    /// Print the transfer plan of one profile read (no device, no hardware).
    Plan(OnboardWireArgs),

    /// Read one profile from a legacy Apex keyboard. Requires --yes.
    Read(OnboardReadArgs),

    /// Read the schema word of every profile slot and of the lighting files. Requires --yes.
    Schema(OnboardSchemaArgs),

    /// Decode a profile image saved by an earlier read (no device, no hardware).
    Decode(OnboardDecodeArgs),
}

/// Wire tunables of one on-board profile read. Defaults are the values measured in the reference
/// capture: the pacing frame's 1 ms gap and 500 ms USB timeout, a 1024-byte resource buffer, and
/// 512-byte reads out of it.
#[cfg(feature = "onboard")]
#[derive(clap::Args)]
pub(crate) struct OnboardWireArgs {
    /// On-board profile slot (0-4)
    #[arg(short, long, default_value = "0")]
    pub(crate) profile: u8,

    /// Resource-buffer length each pull fills, in bytes
    #[arg(long, default_value = "1024")]
    pub(crate) resource_len: usize,

    /// Payload bytes asked for in each read
    #[arg(long, default_value = "512")]
    pub(crate) read_len: usize,

    /// Pulls per profile file
    #[arg(short, long, default_value = "12")]
    pub(crate) chunks: usize,

    /// Profile payload length (default: 11648, the schema A length)
    #[arg(long)]
    pub(crate) payload_len: Option<usize>,

    /// First profile file id (0x0201 is profile 0 on the reference keyboard)
    #[arg(long, value_parser = parse_hex_u16)]
    pub(crate) fs_base: Option<u16>,

    /// Skip the pacing frame (the capture shows GG sending it before the profile read)
    #[arg(long)]
    pub(crate) no_config_frame: bool,

    /// File-access mode sent in the pacing frame
    #[arg(long, default_value = "2")]
    pub(crate) file_access: u32,

    /// USB timeout sent in the pacing frame, in milliseconds
    #[arg(long, default_value = "500")]
    pub(crate) max_usb_timeout_ms: u32,

    /// Time between commands sent in the pacing frame, in milliseconds
    #[arg(long, default_value = "1")]
    pub(crate) time_between_commands_ms: u32,

    /// Gap between transfers, in milliseconds
    #[arg(long, default_value = "5")]
    pub(crate) gap_ms: u64,
}

#[cfg(feature = "onboard")]
impl OnboardWireArgs {
    /// Turn the tunables into a read plan. Defaults match the reference capture.
    pub(crate) fn to_config(&self) -> steelseries_gg::onboard_profile::ReadConfig {
        steelseries_gg::onboard_profile::ReadConfig {
            profile: self.profile,
            resource_len: self.resource_len,
            read_len: self.read_len,
            chunks: self.chunks,
            payload_len: self
                .payload_len
                .unwrap_or(steelseries_gg::onboard_profile::PAYLOAD_LEN_A),
            fs_base: self.fs_base.unwrap_or(steelseries_gg::onboard_profile::PROFILE_FS_BASE),
            send_config: !self.no_config_frame,
            file_access: self.file_access,
            max_usb_timeout_ms: self.max_usb_timeout_ms,
            time_between_commands_ms: self.time_between_commands_ms,
            gap_ms: self.gap_ms,
        }
    }
}

#[cfg(feature = "onboard")]
#[derive(clap::Args)]
pub(crate) struct OnboardReadArgs {
    #[command(flatten)]
    pub(crate) wire: OnboardWireArgs,

    /// Device as `vid:pid`, a sysfs prefix, or a `/dev/hidrawN` node (default: the control
    /// collection of the first legacy Apex keyboard found)
    #[arg(short, long)]
    pub(crate) device: Option<String>,

    /// Confirm the device read (required: experimental protocol code, talks to hardware)
    #[arg(short, long)]
    pub(crate) yes: bool,

    /// Write the assembled profile image here
    #[arg(short, long)]
    pub(crate) out: Option<PathBuf>,

    /// Write the JSON report here
    #[arg(long)]
    pub(crate) json: Option<PathBuf>,

    /// Hex-dump the whole profile image to stdout
    #[arg(long)]
    pub(crate) hex: bool,

    /// Read twice and compare the profile bytes
    #[arg(long)]
    pub(crate) verify: bool,

    /// A reply header that disagrees with its request fails the read (research mode turns this off)
    #[arg(long, default_value = "true")]
    pub(crate) strict: bool,
}

#[cfg(feature = "onboard")]
#[derive(clap::Args)]
pub(crate) struct OnboardSchemaArgs {
    /// Device as `vid:pid`, a sysfs prefix, or a `/dev/hidrawN` node
    #[arg(short, long)]
    pub(crate) device: Option<String>,

    /// Confirm the device read (required: experimental protocol code, talks to hardware)
    #[arg(short, long)]
    pub(crate) yes: bool,

    /// Write the JSON report here
    #[arg(long)]
    pub(crate) json: Option<PathBuf>,

    /// Gap between transfers, in milliseconds
    #[arg(long, default_value = "5")]
    pub(crate) gap_ms: u64,

    /// Stop the probe after this many milliseconds per path group (0 removes the limit)
    #[arg(long, default_value = "30000")]
    pub(crate) deadline_ms: u64,

    /// Reject a pull answer on an unexpected report ID
    #[arg(long, default_value = "true")]
    pub(crate) strict: bool,
}

#[cfg(feature = "onboard")]
#[derive(clap::Args)]
pub(crate) struct OnboardDecodeArgs {
    /// Profile image file (assembled by `ssgg onboard read --out`)
    pub(crate) file: PathBuf,

    /// Write the JSON decode report here
    #[arg(short, long)]
    pub(crate) json: Option<PathBuf>,

    /// Hex-dump profile bytes
    #[arg(long)]
    pub(crate) hex: bool,

    /// First byte of the hex dump
    #[arg(long, default_value = "0")]
    pub(crate) hex_from: usize,

    /// Number of bytes in the hex dump
    #[arg(long, default_value = "64")]
    pub(crate) hex_len: usize,
}

#[derive(Subcommand)]
pub(crate) enum RgbAction {
    /// Set a static color
    Color {
        /// Color as hex (e.g., FF0000) or name (red, green, blue, etc.)
        color: String,
    },

    /// Set brightness (0-100)
    Brightness {
        /// Brightness level
        level: u8,
    },

    /// Set a lighting effect
    Effect {
        /// Effect name: static, breathing, spectrum, wave, off
        name: String,

        /// Effect speed (0.1 - 5.0)
        #[arg(short, long, default_value = "1.0")]
        speed: f32,
    },

    /// Turn off all LEDs
    Off,

    /// Per-key RGB control (requires supported keyboard with key mapping)
    Perkey {
        #[command(subcommand)]
        action: PerKeyAction,
    },
}

#[derive(Subcommand)]
pub(crate) enum ActuationAction {
    /// Set actuation point for all keys (global)
    Set {
        /// Actuation point in millimeters (0.1 to 4.0mm, e.g., 1.2, 2.5, 3.6)
        mm: f32,
    },

    /// Set actuation point in 0.1mm increments (e.g., 4 = 0.4mm, 25 = 2.5mm)
    SetValue {
        /// Actuation value in 0.1mm units (1-40)
        value: u8,
    },
}

#[derive(Subcommand)]
pub(crate) enum PerKeyAction {
    /// Set a single key to a specific color
    SetKey {
        /// Key name (e.g., A, Enter, Space, F1, etc.) - case insensitive
        key: String,
        /// Color as hex (e.g., FF0000) or name (red, green, blue, etc.)
        color: String,
    },

    /// Set multiple keys to specific colors
    SetKeys {
        /// Key-color pairs in format "key:color,key:color" (e.g., "A:red,S:green,D:blue")
        keys: String,
    },

    /// Set a range of keys to the same color using HID codes
    SetRegion {
        /// Starting HID code (0x00-0xFF)
        start_hid: u8,
        /// Number of keys to set
        count: u8,
        /// Color as hex (e.g., FF0000) or name (red, green, blue, etc.)
        color: String,
    },

    /// Turn off all per-key RGB (set all keys to black)
    Clear,

    /// Test individual key by HID code (direct addressing)
    TestMatrix {
        /// HID code (0x00-0xFF)
        hid_code: u8,
        /// Color as hex (e.g., FF0000) or name (red, green, blue, etc.)
        color: String,
    },

    /// Test a pattern across the keyboard
    TestPattern {
        /// Pattern name: rainbow, checkerboard, wave, test
        pattern: String,
    },

    /// Show keyboard mapping information
    ShowMapping,

    /// Show per-key RGB support status
    Status,
}

#[derive(Subcommand)]
pub(crate) enum ProfileAction {
    /// List all profiles
    List,

    /// Load a profile
    Load {
        /// Profile name
        name: String,
    },

    /// Save current settings as a profile
    Save {
        /// Profile name
        name: String,
    },

    /// Delete a profile
    Delete {
        /// Profile name
        name: String,
    },
}

#[cfg(feature = "audio")]
#[derive(Subcommand)]
pub(crate) enum AudioAction {
    /// Show current mixer state
    Status,

    /// Set channel volume
    Volume {
        /// Channel: master, game, chat, media, aux, mic
        channel: String,

        /// Volume level (0-100)
        level: u8,
    },

    /// Mute/unmute a channel
    Mute {
        /// Channel: master, game, chat, media, aux, mic
        channel: String,

        /// Mute state (true/false), omit to toggle
        #[arg(short, long)]
        state: Option<bool>,
    },

    /// Set chat mix balance
    ChatMix {
        /// Balance (-100 = game, 0 = balanced, 100 = chat)
        balance: i8,
    },
}

#[cfg(feature = "sonar")]
#[derive(Subcommand)]
pub(crate) enum SonarAction {
    /// Show current Sonar status and volumes
    Status,

    /// Discover the Sonar API port
    Discover,

    /// Get audio devices
    Devices,

    /// Get current mode (classic or streamer)
    Mode,

    /// Set volume for a channel (classic mode)
    Volume {
        /// Channel: master, game, chat, media, aux
        channel: String,

        /// Volume level (0-100)
        level: u8,
    },

    /// Get chat mix settings
    ChatMix,

    /// Control streamer mode
    Streamer {
        #[command(subcommand)]
        action: StreamerAction,
    },

    /// Get all configurations
    Configs,
}

#[cfg(feature = "sonar")]
#[derive(Subcommand)]
pub(crate) enum StreamerAction {
    /// Set monitoring volume for a channel
    Monitoring {
        /// Channel: master, game, chat
        channel: String,

        /// Volume level (0-100)
        level: u8,
    },

    /// Set streaming volume for a channel
    Streaming {
        /// Channel: master, game, chat
        channel: String,

        /// Volume level (0-100)
        level: u8,
    },
}

#[derive(Subcommand)]
pub(crate) enum PollrateAction {
    /// Set mouse polling rate
    Mouse {
        /// Polling rate in Hz (125, 500, 1000, 2000, 4000)
        /// Note: Rates above 1000Hz require hardware support
        rate: u32,

        /// Save to config and apply on daemon startup
        #[arg(long)]
        persistent: bool,
    },

    /// Set keyboard polling rate
    Keyboard {
        /// Polling rate in Hz (125, 500, 1000, 2000, 4000)
        /// Note: Rates above 1000Hz require hardware support
        rate: u32,

        /// Save to config and apply on daemon startup
        #[arg(long)]
        persistent: bool,
    },

    /// Show current polling rates
    Status,
}

#[derive(Subcommand)]
pub(crate) enum PerformanceAction {
    /// Show current performance statistics
    Stats {
        /// Continuously monitor (update every N seconds)
        #[arg(short, long)]
        monitor: Option<u64>,

        /// Export stats to file
        #[arg(short, long)]
        output: Option<String>,

        /// JSON output format
        #[arg(long)]
        json: bool,
    },

    /// Enable performance optimizations
    Enable,

    /// Disable performance optimizations
    Disable,

    /// Cleanup performance caches
    Cleanup,

    /// Run performance benchmark
    Benchmark {
        /// Duration in seconds
        #[arg(short, long, default_value = "10")]
        duration: u64,

        /// Export results to file
        #[arg(short, long)]
        output: Option<String>,
    },
}

pub(crate) fn parse_color(s: &str) -> Option<Color> {
    // Try named colors - use eq_ignore_ascii_case to avoid allocations
    if s.eq_ignore_ascii_case("red") {
        return Some(Color::RED);
    } else if s.eq_ignore_ascii_case("green") {
        return Some(Color::GREEN);
    } else if s.eq_ignore_ascii_case("blue") {
        return Some(Color::BLUE);
    } else if s.eq_ignore_ascii_case("white") {
        return Some(Color::WHITE);
    } else if s.eq_ignore_ascii_case("black") || s.eq_ignore_ascii_case("off") {
        return Some(Color::BLACK);
    } else if s.eq_ignore_ascii_case("cyan") {
        return Some(Color::CYAN);
    } else if s.eq_ignore_ascii_case("magenta") {
        return Some(Color::MAGENTA);
    } else if s.eq_ignore_ascii_case("yellow") {
        return Some(Color::YELLOW);
    } else if s.eq_ignore_ascii_case("orange") {
        return Some(Color::ORANGE);
    } else if s.eq_ignore_ascii_case("purple") {
        return Some(Color::PURPLE);
    } else if s.eq_ignore_ascii_case("pink") {
        return Some(Color::PINK);
    }

    // Try hex
    let hex = s.trim_start_matches('#');
    if hex.len() == 6
        && let Ok(val) = u32::from_str_radix(hex, 16)
    {
        return Some(Color::from_hex(val));
    }

    None
}

pub(crate) fn require_color(s: &str) -> Result<Color> {
    parse_color(s).ok_or_else(|| Error::Other(format!("Invalid color: {}", s)))
}

pub(crate) fn parse_hex_u8(s: &str) -> std::result::Result<u8, String> {
    let s = s.trim_start_matches("0x");
    u8::from_str_radix(s, 16).map_err(|e| format!("Invalid hex value: {}", e))
}

#[cfg(feature = "onboard")]
pub(crate) fn parse_hex_u16(s: &str) -> std::result::Result<u16, String> {
    let s = s.trim_start_matches("0x");
    u16::from_str_radix(s, 16).map_err(|e| format!("Invalid hex value: {}", e))
}

#[cfg(feature = "audio")]
pub(crate) fn parse_channel(s: &str) -> Option<Channel> {
    let s_lower = s.to_ascii_lowercase();
    match s_lower.as_str() {
        "master" => Some(Channel::Master),
        "game" => Some(Channel::Game),
        "chat" => Some(Channel::Chat),
        "media" => Some(Channel::Media),
        "aux" => Some(Channel::Aux),
        "mic" => Some(Channel::Mic),
        _ => None,
    }
}

/// Convert a volume level (0-100) to a normalized float (0.0-1.0)
#[cfg(any(feature = "audio", feature = "sonar"))]
pub(crate) fn normalize_volume(level: u8) -> f32 {
    (level.min(100) as f32) / 100.0
}

/// Parse and validate a Sonar channel name
#[cfg(feature = "sonar")]
pub(crate) fn parse_sonar_channel<'a>(channel: &'a str, valid_channels: &[&str]) -> Result<&'a str> {
    let channel_lower = channel.to_ascii_lowercase();
    if valid_channels.contains(&channel_lower.as_str()) {
        Ok(channel)
    } else {
        Err(Error::Other(format!(
            "Invalid channel: {}. Valid channels: {}",
            channel,
            valid_channels.join(", ")
        )))
    }
}

/// Parse a zone identifier (e.g., "zone1", "2", "all") into a zone index (0-based).
/// Returns None for "all"/"keyboard" which should apply to all zones.
#[inline]
pub(crate) fn parse_zone_number(zone: &str) -> Option<usize> {
    // Fast path: check common cases without string allocation
    if zone.eq_ignore_ascii_case("all") || zone.eq_ignore_ascii_case("keyboard") {
        return None;
    }

    // Try parsing as "zone<number>" or just "<number>"
    // Use case-insensitive prefix check to avoid allocation
    let number_part = if zone.len() > 4 && zone[0..4].eq_ignore_ascii_case("zone") {
        &zone[4..]
    } else {
        zone
    };

    number_part.parse::<usize>().ok().and_then(|one_based| {
        // Convert 1-based to 0-based index
        if one_based > 0 { Some(one_based - 1) } else { None }
    })
}

pub(crate) fn parse_key_name(name: &str) -> Option<KeyId> {
    let name_upper = name.to_ascii_uppercase();
    match name_upper.as_str() {
        // Letters
        "A" => Some(KeyId::A),
        "B" => Some(KeyId::B),
        "C" => Some(KeyId::C),
        "D" => Some(KeyId::D),
        "E" => Some(KeyId::E),
        "F" => Some(KeyId::F),
        "G" => Some(KeyId::G),
        "H" => Some(KeyId::H),
        "I" => Some(KeyId::I),
        "J" => Some(KeyId::J),
        "K" => Some(KeyId::K),
        "L" => Some(KeyId::L),
        "M" => Some(KeyId::M),
        "N" => Some(KeyId::N),
        "O" => Some(KeyId::O),
        "P" => Some(KeyId::P),
        "Q" => Some(KeyId::Q),
        "R" => Some(KeyId::R),
        "S" => Some(KeyId::S),
        "T" => Some(KeyId::T),
        "U" => Some(KeyId::U),
        "V" => Some(KeyId::V),
        "W" => Some(KeyId::W),
        "X" => Some(KeyId::X),
        "Y" => Some(KeyId::Y),
        "Z" => Some(KeyId::Z),

        // Numbers
        "1" => Some(KeyId::Key1),
        "2" => Some(KeyId::Key2),
        "3" => Some(KeyId::Key3),
        "4" => Some(KeyId::Key4),
        "5" => Some(KeyId::Key5),
        "6" => Some(KeyId::Key6),
        "7" => Some(KeyId::Key7),
        "8" => Some(KeyId::Key8),
        "9" => Some(KeyId::Key9),
        "0" => Some(KeyId::Key0),

        // Function keys
        "F1" => Some(KeyId::F1),
        "F2" => Some(KeyId::F2),
        "F3" => Some(KeyId::F3),
        "F4" => Some(KeyId::F4),
        "F5" => Some(KeyId::F5),
        "F6" => Some(KeyId::F6),
        "F7" => Some(KeyId::F7),
        "F8" => Some(KeyId::F8),
        "F9" => Some(KeyId::F9),
        "F10" => Some(KeyId::F10),
        "F11" => Some(KeyId::F11),
        "F12" => Some(KeyId::F12),

        // Special keys
        "ENTER" | "RETURN" => Some(KeyId::Enter),
        "SPACE" | "SPACEBAR" => Some(KeyId::Space),
        "ESC" | "ESCAPE" => Some(KeyId::Escape),
        "TAB" => Some(KeyId::Tab),
        "SHIFT" => Some(KeyId::LeftShift),
        "CTRL" => Some(KeyId::LeftCtrl),
        "ALT" => Some(KeyId::LeftAlt),
        "WIN" | "WINDOWS" => Some(KeyId::LeftWin),
        "CAPS" | "CAPSLOCK" => Some(KeyId::CapsLock),
        "BACKSPACE" => Some(KeyId::Backspace),

        // Arrows
        "UP" | "ARROWUP" => Some(KeyId::ArrowUp),
        "DOWN" | "ARROWDOWN" => Some(KeyId::ArrowDown),
        "LEFT" | "ARROWLEFT" => Some(KeyId::ArrowLeft),
        "RIGHT" | "ARROWRIGHT" => Some(KeyId::ArrowRight),

        // Punctuation
        ";" | "SEMICOLON" => Some(KeyId::Semicolon),
        "'" | "QUOTE" => Some(KeyId::Quote),
        "," | "COMMA" => Some(KeyId::Comma),
        "." | "PERIOD" => Some(KeyId::Period),
        "/" | "SLASH" => Some(KeyId::Slash),
        "[" | "LEFTBRACKET" => Some(KeyId::LeftBracket),
        "]" | "RIGHTBRACKET" => Some(KeyId::RightBracket),
        "\\" | "BACKSLASH" => Some(KeyId::Backslash),
        "-" | "MINUS" => Some(KeyId::Minus),
        "=" | "EQUAL" => Some(KeyId::Equal),
        "`" | "BACKTICK" => Some(KeyId::Backtick),

        _ => None,
    }
}
