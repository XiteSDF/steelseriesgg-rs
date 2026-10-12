//! SteelSeries GG for Linux - CLI
//!
//! A complete open-source replacement for SteelSeries GG on Linux.

mod cli;

use clap::{Parser, Subcommand};
use tokio::task::yield_now;
use tracing::{Level, info};
use tracing_subscriber::FmtSubscriber;

use steelseries_gg::config::Config;
use steelseries_gg::devices::keyboards::Keyboard;
use steelseries_gg::devices::{
    DeviceInfo, DeviceManager, DeviceType, KeyAddress, KeyId,
    diagnostics::{init_global_diagnostics, with_global_diagnostics},
    discovery::print_device_summary,
};
use steelseries_gg::fs_utils::{secure_write, secure_write_async};
use steelseries_gg::gamesense::GameSenseServer;
use steelseries_gg::rgb::{Color, Effect, RgbController, WaveDirection};
use steelseries_gg::validation::RgbValidator;
use steelseries_gg::{Error, Result};

use std::collections::HashMap;
use std::io::IsTerminal;
use std::time::{Duration, Instant};

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tabled::{Table, Tabled};

#[cfg(feature = "audio")]
use steelseries_gg::audio::{AudioMixer, Channel};

#[cfg(all(feature = "sonar", not(target_os = "linux")))]
use steelseries_gg::audio::SonarClient;

/// SteelSeries GG for Linux - Control your SteelSeries devices
#[derive(Parser)]
#[command(name = "ssgg")]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,

    /// Enable HID communication debugging and diagnostics
    #[arg(long)]
    debug_hid: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List connected SteelSeries devices with battery, lighting and capabilities
    Devices {
        /// Show every raw HID collection instead (diagnostics)
        #[arg(long)]
        raw: bool,
    },

    /// Show the settings a device supports and their current values
    Settings {
        /// Device: key from `ssgg devices`, a type (keyboard/mouse/headset) or part of its name
        device: String,
    },

    /// Change one device setting, e.g. `ssgg set mouse dpi 800,1600` or `ssgg set headset sidetone 3`
    Set {
        /// Device: key from `ssgg devices`, a type (keyboard/mouse/headset) or part of its name
        device: String,
        /// Setting id from `ssgg settings <device>`
        setting: String,
        /// New value
        value: String,
    },

    /// Open the control panel in a browser (needs the daemon)
    Ui,

    /// Sonar-style audio mixer: Game/Chat/Media/Aux/Microphone devices, EQ, ChatMix, routing
    Mixer {
        #[command(subcommand)]
        action: Option<cli::extras::MixerAction>,
    },

    /// Rebind a key or mouse button, e.g. `ssgg bind capslock key ctrl`, `ssgg bind f13 text "gg"`
    Bind {
        /// Key or button: capslock, f13, mouse4, KEY_A, BTN_SIDE...
        key: String,
        /// key <k> | combo <a+b> | text <...> | launch <cmd...> | media <m> | mouse <b> |
        /// profile <name> | macro <tap:K delay:ms ...> | disabled
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        action: Vec<String>,
        /// Profile to edit (default: the active one)
        #[arg(long)]
        profile: Option<String>,
        /// Fire when the key is released instead of pressed
        #[arg(long)]
        on_release: bool,
    },

    /// Give a key back its normal function
    Unbind {
        key: String,
        #[arg(long)]
        profile: Option<String>,
    },

    /// List key bindings
    Bindings {
        #[arg(long)]
        profile: Option<String>,
    },

    /// Record a macro from your SteelSeries keyboard/mouse and bind it to a key
    Macro {
        #[command(subcommand)]
        action: MacroAction,
    },

    /// Keyboard OLED screen: text, images, clock, system stats, now playing
    Oled {
        #[command(subcommand)]
        action: OledAction,
    },

    /// Instant-replay clips (GG Moments) via gpu-screen-recorder
    Moments {
        #[command(subcommand)]
        action: MomentsAction,
    },

    /// Control RGB lighting (all devices in sync unless --device is given)
    Rgb {
        /// Only this device (key, type or name); omit for synced lighting on every device
        #[arg(short, long, global = true)]
        device: Option<String>,

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

        /// Include performance metrics snapshot (if daemon is running)
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

#[derive(Subcommand)]
enum RgbAction {
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
        /// Effect name: static, breathing, spectrum, wave, reactive, gradient, off
        name: String,

        /// Effect speed (0.1 - 5.0)
        #[arg(short, long, default_value = "1.0")]
        speed: f32,

        /// Main color (name or hex)
        #[arg(short, long)]
        color: Option<String>,

        /// Second color for wave and gradient
        #[arg(long)]
        color2: Option<String>,
    },

    /// Turn off all LEDs
    Off,

    /// Make --device follow the synced lighting again
    Sync,

    /// Per-key RGB control (requires supported keyboard with key mapping)
    Perkey {
        #[command(subcommand)]
        action: PerKeyAction,
    },
}

#[derive(Subcommand)]
enum MacroAction {
    /// Record key presses until the stop key, then bind them to KEY
    Record {
        /// Key that will play the macro
        key: String,
        /// Key that ends recording (default Esc; it is not recorded)
        #[arg(long)]
        stop_key: Option<String>,
        /// Give up after this many seconds
        #[arg(long, default_value = "30")]
        timeout: u32,
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
enum OledAction {
    /// Show lines of text
    Text {
        /// One argument per line
        #[arg(required = true)]
        lines: Vec<String>,
        /// Seconds to show it (0 = until replaced)
        #[arg(short, long)]
        seconds: Option<u32>,
    },
    /// Show an image or animated GIF (scaled and dithered to the screen)
    Image {
        path: std::path::PathBuf,
        /// Seconds to show it (0 = until replaced)
        #[arg(short, long)]
        seconds: Option<u32>,
        /// Use it as the idle screen instead of showing it once
        #[arg(long)]
        idle: bool,
    },
    /// Idle screen: the time
    Clock,
    /// Idle screen: CPU and memory use
    Stats,
    /// Idle screen: the track playing in any MPRIS player (via playerctl)
    Nowplaying,
    /// Stop drawing; the keyboard keeps its last image
    Off,
    /// Remove temporary content and return to the idle screen
    Clear,
}

#[derive(Subcommand)]
enum MomentsAction {
    /// Save the last N seconds as a clip (bind this to a key)
    Save,
    /// Show whether the replay buffer is running
    Status,
    /// Keep a replay buffer running while the daemon runs
    Enable {
        /// Seconds to keep (5-1200)
        #[arg(short, long)]
        seconds: Option<u32>,
    },
    /// Stop the replay buffer
    Disable,
}

#[derive(Subcommand)]
enum ActuationAction {
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
enum PerKeyAction {
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
enum ProfileAction {
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

        /// Short description
        #[arg(short, long)]
        description: Option<String>,
    },

    /// Read one keyboard's own memory back and save what it holds as a profile
    Pull {
        /// Keyboard to read (name or slot); needed once more than one is connected
        #[arg(short, long)]
        device: Option<String>,

        /// Profile to write: the name the keyboard spells inside its own memory by default
        #[arg(short, long)]
        name: Option<String>,

        /// Which of the keyboard's own onboard profiles to read (1 to 5): its own choice by default
        #[arg(short, long)]
        slot: Option<u8>,
    },

    /// Switch to this profile automatically while one of these programs runs
    Apps {
        /// Profile name
        name: String,

        /// Process names (e.g. cs2 steam_app_730); none clears the list
        apps: Vec<String>,
    },

    /// Delete a profile
    Delete {
        /// Profile name
        name: String,
    },
}

#[cfg(feature = "audio")]
#[derive(Subcommand)]
enum AudioAction {
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
enum SonarAction {
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
enum StreamerAction {
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
enum PollrateAction {
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
enum PerformanceAction {
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

fn parse_color(s: &str) -> Option<Color> {
    Color::parse(s)
}

fn parse_hex_u8(s: &str) -> std::result::Result<u8, String> {
    let s = s.trim_start_matches("0x");
    u8::from_str_radix(s, 16).map_err(|e| format!("Invalid hex value: {}", e))
}

#[cfg(feature = "audio")]
fn parse_channel(s: &str) -> Option<Channel> {
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
#[cfg(any(feature = "audio", all(feature = "sonar", not(target_os = "linux"))))]
fn normalize_volume(level: u8) -> f32 {
    (level.min(100) as f32) / 100.0
}

/// Parse and validate a Sonar channel name
#[cfg(all(feature = "sonar", not(target_os = "linux")))]
fn parse_sonar_channel<'a>(channel: &'a str, valid_channels: &[&str]) -> Result<&'a str> {
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

/// Generate a comprehensive bug report with diagnostic information.
async fn cmd_bug_report(output: &str, include_hid_logs: bool, include_performance: bool) -> Result<()> {
    use steelseries_gg::diagnostics_export::{collect_bug_report, export_bug_report};

    println!("Collecting diagnostic information...");
    println!("  - System information (OS, kernel, memory, CPU)");
    println!("  - Device states (connected devices, current settings)");

    if include_hid_logs {
        println!("  - HID communication logs");
    }

    if include_performance {
        println!("  - Performance metrics snapshot");
    }

    println!();

    // Collect bug report (async, uses spawn_blocking internally for sysinfo)
    let report = collect_bug_report(include_hid_logs, include_performance).await?;

    // Export to file (async file I/O)
    export_bug_report(&report, output).await?;

    // Print privacy warning
    use colored::Colorize;
    println!();
    println!("{}", "WARNING:".yellow().bold());
    println!("  This report may contain:");
    println!("    - Device serial numbers and identifiers");
    println!("    - System paths and usernames");
    println!("    - Performance data and timing information");
    println!();
    println!("  Please review the file before sharing publicly.");

    Ok(())
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    // Logging: RUST_LOG wins; otherwise the daemon logs at info and one-shot commands only
    // show warnings, so their own output is not buried under log lines.
    let default_level = if cli.debug {
        Level::DEBUG
    } else if matches!(cli.command, Commands::Daemon | Commands::Server { .. }) {
        Level::INFO
    } else {
        Level::WARN
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level.to_string()));
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(filter)
        .with_target(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Initialize HID diagnostics if requested
    if cli.debug_hid {
        init_global_diagnostics(true)?;
        info!("HID diagnostics enabled - logging to timestamped file");
    } else {
        init_global_diagnostics(false)?;
    }

    let debug_hid = cli.debug_hid;
    match cli.command {
        Commands::Devices { raw } => {
            if raw {
                let manager = new_device_manager()?;
                cmd_devices(&manager)?;
            } else {
                cli::devices().await?;
            }
        }

        Commands::Settings { device } => cli::settings(device).await?,

        Commands::Set { device, setting, value } => cli::set(device, setting, value).await?,

        Commands::Ui => cli::open_ui().await?,

        Commands::Mixer { action } => {
            cli::extras::mixer(action.unwrap_or(cli::extras::MixerAction::Status)).await?;
        }

        Commands::Bind {
            key,
            action,
            profile,
            on_release,
        } => cli::extras::bind(key, action, profile, on_release).await?,

        Commands::Unbind { key, profile } => cli::extras::unbind(key, profile).await?,

        Commands::Bindings { profile } => cli::extras::bindings(profile).await?,

        Commands::Macro {
            action:
                MacroAction::Record {
                    key,
                    stop_key,
                    timeout,
                    profile,
                },
        } => cli::extras::record_macro(key, stop_key, timeout, profile).await?,

        Commands::Oled { action } => {
            use steelseries_gg::engine::Command;
            use steelseries_gg::engine::screen::{IdleScreen, ScreenContent};
            let oled = |content, seconds, idle, clear| Command::Oled {
                content,
                seconds,
                idle,
                clear,
            };
            let command = match action {
                OledAction::Text { lines, seconds } => oled(Some(ScreenContent::Text { lines }), seconds, None, false),
                OledAction::Image { path, seconds, idle } => {
                    let path = std::fs::canonicalize(&path)
                        .map_err(|e| Error::InvalidConfig(format!("{}: {e}", path.display())))?
                        .to_string_lossy()
                        .into_owned();
                    if idle {
                        oled(None, None, Some(IdleScreen::Image { path }), true)
                    } else {
                        oled(Some(ScreenContent::Image { path }), seconds, None, false)
                    }
                }
                OledAction::Clock => oled(None, None, Some(IdleScreen::Clock), true),
                OledAction::Stats => oled(None, None, Some(IdleScreen::Stats), true),
                OledAction::Nowplaying => oled(None, None, Some(IdleScreen::NowPlaying), true),
                OledAction::Off => oled(None, None, Some(IdleScreen::Off), true),
                OledAction::Clear => oled(None, None, None, true),
            };
            cli::oled(command).await?;
        }

        Commands::Moments { action } => {
            use steelseries_gg::engine::Command;
            let command = match action {
                MomentsAction::Save => Command::MomentsSave,
                MomentsAction::Status => Command::MomentsStatus,
                MomentsAction::Enable { seconds } => Command::MomentsEnable {
                    enabled: true,
                    replay_seconds: seconds,
                },
                MomentsAction::Disable => Command::MomentsEnable {
                    enabled: false,
                    replay_seconds: None,
                },
            };
            cli::moments(command).await?;
        }

        Commands::Rgb { device, action } => {
            cmd_rgb(device, action).await?;
        }

        Commands::Actuation { action } => {
            cmd_actuation(action).await?;
        }

        Commands::Profile { action } => {
            cmd_profile(action).await?;
        }

        #[cfg(feature = "audio")]
        Commands::Audio { action } => {
            cmd_audio(action)?;
        }

        #[cfg(feature = "sonar")]
        Commands::Sonar { action } => {
            cmd_sonar(action).await?;
        }

        Commands::Pollrate { action } => {
            cmd_pollrate(action).await?;
        }

        Commands::Server { port } => {
            cmd_server(port).await?;
        }

        Commands::Validate {
            benchmark,
            timeout,
            output,
            json,
        } => {
            let manager = new_device_manager()?;
            cmd_validate(&manager, benchmark, timeout, output, json).await?;
        }

        Commands::Performance { action } => {
            let manager = new_device_manager()?;
            cmd_performance(&manager, action).await?;
        }

        Commands::BugReport {
            output,
            include_hid_logs,
            include_performance,
        } => {
            cmd_bug_report(&output, include_hid_logs, include_performance).await?;
        }

        Commands::Status { device, refresh } => {
            let manager = new_device_manager()?;
            cmd_status(&manager, &device, refresh).await?;
        }

        Commands::HidLogs { file, device } => {
            cmd_hid_logs(file, device.as_deref()).await?;
        }

        Commands::Daemon => {
            cli::daemon::run().await?;
        }

        Commands::TestDevice {
            device,
            benchmark,
            verbose,
        } => {
            let manager = new_device_manager()?;
            cmd_test_device(&manager, &device, benchmark, verbose).await?;
        }

        Commands::VerifyPerformance {
            duration,
            effect,
            output,
        } => {
            let manager = new_device_manager()?;
            cmd_verify_performance(&manager, duration, &effect, output).await?;
        }

        Commands::Fuzz { start, end, delay } => {
            use steelseries_gg::devices::fuzz::{FuzzParams, PayloadPattern, fuzz_keyboard_protocol};

            let manager = new_device_manager()?;
            let params = FuzzParams {
                start_cmd: start,
                end_cmd: end,
                delay_ms: delay,
                payload_pattern: PayloadPattern::Zeros, // Default to zeros for now
            };

            fuzz_keyboard_protocol(&manager, params)?;
        }
    }

    // Display HID diagnostic summary if enabled
    if debug_hid && let Some(summary) = with_global_diagnostics(|diag| diag.get_summary()) {
        info!("HID Diagnostic Summary:\n{}", summary);
    }

    Ok(())
}

/// Create a `DeviceManager`, applying the `[device]` control-endpoint override from
/// `config.toml` if one is set. A missing or unparseable config file falls back to
/// auto-detection rather than failing the command, matching prior behavior where device
/// commands never depended on config.toml at all.
fn new_device_manager() -> Result<DeviceManager> {
    let mut manager = DeviceManager::new()?;
    if let Ok(config) = Config::load() {
        manager.set_device_override(config.device);
    }
    Ok(manager)
}

/// All connected devices, deduplicated to one representative per physical device.
///
/// `DeviceManager::devices()` returns every raw HID collection hidapi enumerates — on Windows
/// that is several entries per physical keyboard/headset (see `DeviceManager::devices_by_type`).
/// CLI listings should show one row per physical device, so this unions the deduped lists
/// instead of using `manager.devices()` directly.
fn deduped_devices(manager: &DeviceManager) -> Vec<&DeviceInfo> {
    let mut all = manager.devices_by_type(DeviceType::Keyboard);
    all.extend(manager.devices_by_type(DeviceType::Headset));
    all.extend(manager.devices_by_type(DeviceType::Mouse));
    all.extend(manager.devices_by_type(DeviceType::Unknown));
    all
}

fn cmd_devices(manager: &DeviceManager) -> Result<()> {
    print_device_summary(manager);
    Ok(())
}

async fn cmd_rgb(device: Option<String>, action: RgbAction) -> Result<()> {
    match action {
        RgbAction::Color { color } => {
            let color = parse_color(&color).ok_or_else(|| Error::Other(format!("Invalid color: {}", color)))?;
            cli::lighting(device, Some(Effect::Static { color }), None, false).await
        }
        RgbAction::Brightness { level } => cli::lighting(device, None, Some(level.min(100)), false).await,
        RgbAction::Effect {
            name,
            speed,
            color,
            color2,
        } => {
            let effect = cli::parse_effect(&name, color.as_deref(), color2.as_deref(), speed)?;
            cli::lighting(device, Some(effect), None, false).await
        }
        RgbAction::Off => cli::lighting(device, Some(Effect::Off), None, false).await,
        RgbAction::Sync => {
            if device.is_none() {
                return Err(Error::InvalidConfig("`rgb sync` needs --device".to_string()));
            }
            cli::lighting(device, None, None, true).await
        }
        RgbAction::Perkey { action } => {
            // Developer tools: talk to the first keyboard directly.
            let manager = new_device_manager()?;
            let keyboard_info = manager
                .first_device_of_type(DeviceType::Keyboard)
                .ok_or_else(|| Error::Other("No keyboard found".to_string()))?;
            println!("Using keyboard: {}", keyboard_info.name);
            let mut keyboard = manager.open_keyboard(keyboard_info)?;
            keyboard.initialize()?;
            cmd_per_key_rgb(&mut keyboard, action).await
        }
    }
}

async fn cmd_actuation(action: ActuationAction) -> Result<()> {
    use steelseries_gg::engine::{Command, SettingsView};
    let tenths = match action {
        ActuationAction::Set { mm } => {
            if !(0.1..=4.0).contains(&mm) {
                return Err(Error::InvalidConfig(
                    "Actuation point must be between 0.1mm and 4.0mm".to_string(),
                ));
            }
            (mm * 10.0).round() as u8
        }
        ActuationAction::SetValue { value } => {
            if !(1..=40).contains(&value) {
                return Err(Error::InvalidConfig(
                    "Actuation point value must be between 1 and 40".to_string(),
                ));
            }
            value
        }
    };
    // Prefer the model's live actuation frame (taken from a tested reference) over the
    // experimental 0x2D command.
    let (view, _) = cli::call(Command::Settings {
        device: "keyboard".to_string(),
    })
    .await?;
    let view: SettingsView = serde_json::from_value(view)?;
    let setting = ["actuation_live", "actuation"]
        .into_iter()
        .find(|id| view.settings.iter().any(|s| s.descriptor.id == *id))
        .ok_or_else(|| Error::Unsupported(format!("{} has no adjustable actuation", view.name)))?;
    cli::set(view.device, setting.to_string(), tenths.to_string()).await?;
    println!("Actuation point: {:.1} mm", f32::from(tenths) / 10.0);
    Ok(())
}

async fn cmd_per_key_rgb(keyboard: &mut Box<dyn Keyboard>, action: PerKeyAction) -> Result<()> {
    match action {
        PerKeyAction::SetKey { key, color } => {
            // Parse the key name to KeyId
            let key_id = parse_key_name(&key).ok_or_else(|| Error::Other(format!("Unknown key: {}", key)))?;

            // Parse the color
            let color = parse_color(&color).ok_or_else(|| Error::Other(format!("Invalid color: {}", color)))?;

            // Check if per-key RGB is supported
            if !keyboard.supports_per_key_rgb() {
                println!("Warning: Per-key RGB not supported on this keyboard");
                println!("Available key mapping: None");
                println!("Falling back to zone-based RGB (setting entire keyboard)");
                keyboard.set_color(color).await?;
                keyboard.apply().await?;
                return Ok(());
            }

            println!("Setting key '{}' to color {}", key, color);
            keyboard.set_key_color(key_id, color).await?;
            keyboard.apply().await?;
            println!("Done!");
        }

        PerKeyAction::SetKeys { keys } => {
            // Parse key-color pairs: "A:red,S:green,D:blue"
            let mut key_colors = Vec::new();
            for pair in keys.split(',') {
                let parts: Vec<&str> = pair.split(':').collect();
                if parts.len() != 2 {
                    return Err(Error::Other(format!("Invalid key:color pair: {}", pair)));
                }

                let key_name = parts[0].trim();
                let color_name = parts[1].trim();

                let key_id =
                    parse_key_name(key_name).ok_or_else(|| Error::Other(format!("Unknown key: {}", key_name)))?;

                let color =
                    parse_color(color_name).ok_or_else(|| Error::Other(format!("Invalid color: {}", color_name)))?;

                key_colors.push((key_id, color));
            }

            if key_colors.is_empty() {
                return Err(Error::Other("No valid key:color pairs found".to_string()));
            }

            // Check if per-key RGB is supported
            if !keyboard.supports_per_key_rgb() {
                println!("Warning: Per-key RGB not supported on this keyboard");
                println!("Falling back to setting first color on entire keyboard");
                keyboard.set_color(key_colors[0].1).await?;
                keyboard.apply().await?;
                return Ok(());
            }

            println!("Setting {} keys to their respective colors", key_colors.len());
            keyboard.set_key_colors(&key_colors).await?;
            keyboard.apply().await?;
            println!("Done!");
        }

        PerKeyAction::SetRegion {
            start_hid,
            count,
            color,
        } => {
            let color = parse_color(&color).ok_or_else(|| Error::Other(format!("Invalid color: {}", color)))?;

            println!(
                "Setting {} keys starting from HID code 0x{:02X} to color {}",
                count, start_hid, color
            );

            keyboard.set_key_region(start_hid, count, color).await?;
            keyboard.apply().await?;
            println!("Done!");
        }

        PerKeyAction::Clear => {
            println!("Clearing all per-key RGB (setting all keys to black)");
            keyboard.clear_per_key_rgb().await?;
            keyboard.apply().await?;
            println!("Done!");
        }

        PerKeyAction::TestMatrix { hid_code, color } => {
            let color = parse_color(&color).ok_or_else(|| Error::Other(format!("Invalid color: {}", color)))?;

            println!("Testing HID code 0x{:02X} with color {}", hid_code, color);
            let address = KeyAddress::new(hid_code);
            keyboard.set_key_color_direct(address, color).await?;
            keyboard.apply().await?;
            println!("Done! If no key lights up, this HID code might not exist.");
        }

        PerKeyAction::TestPattern { pattern } => {
            let pattern_name = pattern.to_ascii_lowercase();
            match pattern_name.as_str() {
                "rainbow" => {
                    println!("Testing rainbow pattern across keyboard");
                    test_rainbow_pattern(keyboard).await?;
                }
                "checkerboard" => {
                    println!("Testing checkerboard pattern");
                    test_checkerboard_pattern(keyboard).await?;
                }
                "wave" => {
                    println!("Testing wave pattern");
                    test_wave_pattern(keyboard).await?;
                }
                "test" => {
                    println!("Testing basic key positions");
                    test_basic_positions(keyboard).await?;
                }
                _ => {
                    return Err(Error::Other(format!(
                        "Unknown pattern: {}. Available: rainbow, checkerboard, wave, test",
                        pattern
                    )));
                }
            }
            keyboard.apply().await?;
            println!("Pattern applied!");
        }

        PerKeyAction::ShowMapping => {
            if let Some(mapping) = keyboard.get_key_mapping() {
                println!("Key mapping for {}:", mapping.name);
                println!("Layout: {:?}", mapping.layout);

                let stats = mapping.get_stats();
                println!("Statistics: {}", stats);

                // Show some sample key mappings
                let sample_keys = [KeyId::A, KeyId::S, KeyId::D, KeyId::Enter, KeyId::Space, KeyId::Escape];
                println!("\nSample key mappings:");
                for key_id in sample_keys.iter() {
                    if let Some(address) = mapping.get_key_address(*key_id) {
                        println!("  {} -> {}", key_id, address);
                    }
                }

                if mapping.total_keys > 6 {
                    println!("  ... and {} more keys", mapping.total_keys - 6);
                }
            } else {
                println!("No key mapping available for this keyboard");
                println!("Per-key RGB control is not supported");
            }
        }

        PerKeyAction::Status => {
            println!("Per-key RGB Status:");
            let perkey_supported = keyboard.supports_per_key_rgb();
            if perkey_supported {
                println!("Provisional: true (key mapping exists, hardware protocol unverified)");
                println!("Note: Per-key RGB protocol is experimental - actual hardware behavior may vary");
            } else {
                println!("Provisional: false");
            }

            if let Some(mapping) = keyboard.get_key_mapping() {
                println!("Key mapping: {} ({:?})", mapping.name, mapping.layout);
                println!("Total keys: {}", mapping.total_keys);
                let stats = mapping.get_stats();
                println!("Matrix utilization: {:.1}%", stats.utilization);
            } else {
                println!("Key mapping: None available");
            }

            println!("Zone count: {}", keyboard.zone_count());
        }
    }

    Ok(())
}

// Helper functions for pattern testing
async fn test_rainbow_pattern(keyboard: &mut Box<dyn Keyboard>) -> Result<()> {
    let colors = Color::RAINBOW_COLORS;

    if keyboard.supports_per_key_rgb() {
        // Use logical key mapping if available
        if let Some(mapping) = keyboard.get_key_mapping() {
            let all_keys = mapping.get_all_keys();
            let mut key_colors = Vec::new();

            for (i, key_id) in all_keys.iter().enumerate() {
                let color = colors[i % colors.len()];
                key_colors.push((*key_id, color));
            }

            keyboard.set_key_colors(&key_colors).await?;
        } else {
            // Fallback to HID code addressing
            let mut direct_colors = Vec::new();
            for hid_code in 0..120u8 {
                let color_idx = hid_code % colors.len() as u8;
                direct_colors.push((KeyAddress::new(hid_code), colors[color_idx as usize]));
            }
            keyboard.set_key_colors_direct(&direct_colors).await?;
        }
    } else {
        // Fallback to zone-based rainbow
        let zone_colors: Vec<Color> = (0..keyboard.zone_count()).map(|i| colors[i % colors.len()]).collect();
        keyboard.set_zone_colors(&zone_colors).await?;
    }

    Ok(())
}

async fn test_checkerboard_pattern(keyboard: &mut Box<dyn Keyboard>) -> Result<()> {
    let mut direct_colors = Vec::new();

    for hid_code in 0..120u8 {
        let color = if hid_code % 2 == 0 { Color::WHITE } else { Color::BLACK };
        direct_colors.push((KeyAddress::new(hid_code), color));
    }

    if keyboard.supports_per_key_rgb() {
        keyboard.set_key_colors_direct(&direct_colors).await?;
    } else {
        println!("Checkerboard pattern requires per-key RGB support");
        keyboard.set_color(Color::WHITE).await?;
    }

    Ok(())
}

async fn test_wave_pattern(keyboard: &mut Box<dyn Keyboard>) -> Result<()> {
    let mut direct_colors = Vec::new();

    for hid_code in 0..120u8 {
        let wave_pos = (hid_code as f32 / 120.0) * 2.0 * std::f32::consts::PI;
        let intensity = ((wave_pos.sin() + 1.0) / 2.0 * 255.0) as u8;
        let color = Color::new(intensity, 0, 255 - intensity);
        direct_colors.push((KeyAddress::new(hid_code), color));
    }

    if keyboard.supports_per_key_rgb() {
        keyboard.set_key_colors_direct(&direct_colors).await?;
    } else {
        println!("Wave pattern requires per-key RGB support");
        keyboard.set_color(Color::PURPLE).await?;
    }

    Ok(())
}

async fn test_basic_positions(keyboard: &mut Box<dyn Keyboard>) -> Result<()> {
    // Test various key positions using HID codes
    let test_positions = [
        (KeyAddress::new(0x29), Color::RED),    // ESC key
        (KeyAddress::new(0x04), Color::GREEN),  // A key
        (KeyAddress::new(0x16), Color::BLUE),   // S key
        (KeyAddress::new(0x2C), Color::YELLOW), // SPACE key
        (KeyAddress::new(0x28), Color::WHITE),  // ENTER key
    ];

    if keyboard.supports_per_key_rgb() {
        keyboard.set_key_colors_direct(&test_positions).await?;
    } else {
        println!("Basic position test requires per-key RGB support");
        keyboard.set_color(Color::WHITE).await?;
    }

    Ok(())
}

fn parse_key_name(name: &str) -> Option<KeyId> {
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

async fn cmd_profile(action: ProfileAction) -> Result<()> {
    use steelseries_gg::engine::Command;
    match action {
        ProfileAction::List => cli::profile_list().await,
        ProfileAction::Load { name } => {
            cli::profile_command(
                Command::ProfileLoad { name: name.clone() },
                &format!("Profile loaded: {name}"),
            )
            .await
        }
        ProfileAction::Save { name, description } => {
            cli::profile_command(
                Command::ProfileSave {
                    name: name.clone(),
                    description,
                },
                &format!("Profile saved: {name}"),
            )
            .await
        }
        ProfileAction::Delete { name } => {
            cli::profile_command(
                Command::ProfileDelete { name: name.clone() },
                &format!("Profile deleted: {name}"),
            )
            .await
        }
        ProfileAction::Pull { device, name, slot } => {
            let done = match name.as_ref() {
                Some(label) => format!("Profile {label} saved from the keyboard's own memory"),
                None => "Saved under the name the keyboard spells inside its own memory".to_string(),
            };
            cli::profile_command(Command::ProfilePull { device, name, slot }, &done).await
        }
        ProfileAction::Apps { name, apps } => {
            let done = if apps.is_empty() {
                format!("Profile {name} no longer switches automatically")
            } else {
                format!("Profile {name} activates while running: {}", apps.join(", "))
            };
            cli::profile_command(Command::ProfileApps { name, apps }, &done).await
        }
    }
}

async fn cmd_pollrate(action: PollrateAction) -> Result<()> {
    use steelseries_gg::pollrate::{DeviceType, PollRate, get_poll_rate, set_poll_rate};

    async fn set_one(device_type: DeviceType, rate: u32, persistent: bool) -> Result<()> {
        let poll_rate = PollRate::from_hz(rate)?;
        set_poll_rate(device_type, poll_rate).await?;
        println!("Kernel {} polling rate set to {}", device_type.name(), poll_rate);
        println!("  Applies to every USB {} on this machine.", device_type.name());

        if persistent {
            #[cfg(target_os = "linux")]
            {
                let note = steelseries_gg::pollrate::persist_poll_rate(device_type, poll_rate).await?;
                println!("{note}");
            }
            #[cfg(not(target_os = "linux"))]
            {
                let mut config = Config::load_async().await?;
                match device_type {
                    DeviceType::Mouse => config.poll_rate.mouse_hz = Some(rate),
                    DeviceType::Keyboard => config.poll_rate.keyboard_hz = Some(rate),
                }
                config.save_async().await?;
                println!("Setting saved to config (will apply on daemon startup)");
            }
        }
        Ok(())
    }

    match action {
        PollrateAction::Mouse { rate, persistent } => set_one(DeviceType::Mouse, rate, persistent).await?,
        PollrateAction::Keyboard { rate, persistent } => set_one(DeviceType::Keyboard, rate, persistent).await?,
        PollrateAction::Status => {
            fn print_pollrate_result(label: &str, result: steelseries_gg::error::Result<PollRate>) {
                match result {
                    Ok(rate) => println!("  {label}: {rate}"),
                    Err(e) if e.to_string().contains("not supported by this device's HID driver") => {
                        println!("  {label}: unsupported on this driver — {e}");
                    }
                    Err(e) => println!("  {label}: Error: {e}"),
                }
            }

            println!("Current USB Polling Rates (kernel override):");
            println!();

            print_pollrate_result("Mouse   ", get_poll_rate(DeviceType::Mouse).await);
            print_pollrate_result("Keyboard", get_poll_rate(DeviceType::Keyboard).await);

            println!();
            println!("Note: changes require root (sudo). Rates above 1000 Hz are a device setting:");
            println!("      use `ssgg set <mouse> polling_rate <hz>` on supported mice.");
        }
    }

    Ok(())
}

#[cfg(feature = "audio")]
fn cmd_audio(action: AudioAction) -> Result<()> {
    let mut mixer = AudioMixer::new()?;

    match action {
        AudioAction::Status => {
            println!("Audio Mixer Status:");
            println!();
            for (channel, state) in mixer.all_channels() {
                let mute_str = if state.muted { " (muted)" } else { "" };
                println!(
                    "  {:8} {:3}%{}",
                    channel.to_string(),
                    (state.volume * 100.0) as u8,
                    mute_str
                );
            }
            println!();
            println!("  Chat Mix: {:+.0}%", mixer.chat_mix() * 100.0);
        }

        AudioAction::Volume { channel, level } => {
            let ch = parse_channel(&channel).ok_or_else(|| Error::Other(format!("Invalid channel: {}", channel)))?;

            let volume = normalize_volume(level);
            mixer.set_volume(ch, volume)?;
            println!("{} volume set to {}%", ch, level.min(100));
        }

        AudioAction::Mute { channel, state } => {
            let ch = parse_channel(&channel).ok_or_else(|| Error::Other(format!("Invalid channel: {}", channel)))?;

            let muted = match state {
                Some(s) => {
                    mixer.set_mute(ch, s)?;
                    s
                }
                None => mixer.toggle_mute(ch)?,
            };

            println!("{} {}", ch, if muted { "muted" } else { "unmuted" });
        }

        AudioAction::ChatMix { balance } => {
            let balance = (balance.clamp(-100, 100) as f32) / 100.0;
            mixer.set_chat_mix(balance)?;
            println!("Chat mix set to {:+.0}%", balance * 100.0);
        }
    }

    Ok(())
}

#[cfg(feature = "sonar")]
async fn cmd_sonar(action: SonarAction) -> Result<()> {
    // SteelSeries Sonar is a Windows application; there is nothing to talk to on Linux.
    #[cfg(target_os = "linux")]
    {
        let _ = action;
        Err(Error::Unsupported(
            "SteelSeries Sonar only exists on Windows. On Linux the same features come from the built-in              mixer: `ssgg mixer enable`, then `ssgg mixer` (or the Audio page of `ssgg ui`)."
                .to_string(),
        ))
    }
    #[cfg(not(target_os = "linux"))]
    cmd_sonar_client(action).await
}

#[cfg(all(feature = "sonar", not(target_os = "linux")))]
async fn cmd_sonar_client(action: SonarAction) -> Result<()> {
    match action {
        SonarAction::Status => {
            println!("Connecting to SteelSeries Sonar...");
            let client = SonarClient::new().await?;
            println!("Connected to Sonar API: {}", client.base_url());
            println!();

            // Get mode
            match client.get_mode().await {
                Ok(mode) => println!("Mode: {:?}", mode),
                Err(e) => println!("Failed to get mode: {}", e),
            }

            // Get classic volumes
            match client.get_classic_volumes().await {
                Ok(volumes) => {
                    println!("\nClassic Mode Volumes:");
                    println!("  Master: {:.0}%", volumes.master * 100.0);
                    println!("  Game:   {:.0}%", volumes.game * 100.0);
                    println!("  Chat:   {:.0}%", volumes.chat * 100.0);
                    println!("  Media:  {:.0}%", volumes.media * 100.0);
                    println!("  Aux:    {:.0}%", volumes.aux * 100.0);
                }
                Err(e) => println!("\nFailed to get classic volumes: {}", e),
            }

            // Get chat mix
            match client.get_chat_mix().await {
                Ok(chat_mix) => println!("\nChat Mix: {:.0}%", chat_mix.value * 100.0),
                Err(e) => println!("\nFailed to get chat mix: {}", e),
            }
        }

        SonarAction::Discover => {
            println!("Discovering Sonar API port...");
            let client = SonarClient::new().await?;
            println!("Sonar API found at: {}", client.base_url());
        }

        SonarAction::Devices => {
            let client = SonarClient::new().await?;
            let devices = client.get_audio_devices().await?;

            println!("Audio Devices:");
            for device in devices {
                println!("  [{}] {} ({})", device.id, device.name, device.device_type);
            }
        }

        SonarAction::Mode => {
            let client = SonarClient::new().await?;
            let mode = client.get_mode().await?;
            println!("Current mode: {:?}", mode);
        }

        SonarAction::Volume { channel, level } => {
            const VALID_CHANNELS: &[&str] = &["master", "game", "chat", "media", "aux"];
            parse_sonar_channel(&channel, VALID_CHANNELS)?;

            let client = SonarClient::new().await?;
            let volume = normalize_volume(level);
            let channel_lower = channel.to_ascii_lowercase();

            match channel_lower.as_str() {
                "master" => client.set_classic_master_volume(volume).await?,
                "game" => client.set_classic_game_volume(volume).await?,
                "chat" => client.set_classic_chat_volume(volume).await?,
                "media" => client.set_classic_media_volume(volume).await?,
                "aux" => client.set_classic_aux_volume(volume).await?,
                _ => unreachable!("Validated by parse_sonar_channel"),
            }

            println!("{} volume set to {}%", channel, level.min(100));
        }

        SonarAction::ChatMix => {
            let client = SonarClient::new().await?;
            let chat_mix = client.get_chat_mix().await?;
            println!("Chat Mix: {:.0}%", chat_mix.value * 100.0);
        }

        SonarAction::Streamer { action } => {
            let client = SonarClient::new().await?;

            match action {
                StreamerAction::Monitoring { channel, level } => {
                    const VALID_CHANNELS: &[&str] = &["master", "game", "chat"];
                    parse_sonar_channel(&channel, VALID_CHANNELS)?;

                    let volume = normalize_volume(level);
                    let channel_lower = channel.to_ascii_lowercase();

                    match channel_lower.as_str() {
                        "master" => client.set_monitoring_master_volume(volume).await?,
                        "game" => client.set_monitoring_game_volume(volume).await?,
                        "chat" => client.set_monitoring_chat_volume(volume).await?,
                        _ => unreachable!("Validated by parse_sonar_channel"),
                    }

                    println!("Monitoring {} volume set to {}%", channel, level.min(100));
                }

                StreamerAction::Streaming { channel, level } => {
                    const VALID_CHANNELS: &[&str] = &["master"];
                    parse_sonar_channel(&channel, VALID_CHANNELS)?;

                    let volume = normalize_volume(level);
                    let channel_lower = channel.to_ascii_lowercase();

                    match channel_lower.as_str() {
                        "master" => client.set_streaming_master_volume(volume).await?,
                        _ => unreachable!("Validated by parse_sonar_channel"),
                    }

                    println!("Streaming {} volume set to {}%", channel, level.min(100));
                }
            }
        }

        SonarAction::Configs => {
            let client = SonarClient::new().await?;
            let configs = client.get_configs().await?;

            println!("Audio Configurations:");
            for config in configs {
                let selected = if config.selected { " (selected)" } else { "" };
                println!("  [{}] {}{}", config.id, config.name, selected);
            }
        }
    }

    Ok(())
}

async fn cmd_validate(
    manager: &DeviceManager,
    benchmark: bool,
    timeout: u64,
    output: Option<String>,
    json: bool,
) -> Result<()> {
    info!("Starting RGB system validation...");

    let timeout_duration = Duration::from_secs(timeout);
    let mut validator = RgbValidator::new().with_timeout(timeout_duration);

    if benchmark {
        validator = validator.with_benchmarks();
        info!("Performance benchmarks enabled");
    }

    let keyboards = manager.keyboards();

    if keyboards.is_empty() {
        println!("⚠️  No SteelSeries keyboards found for validation");
        return Ok(());
    }

    println!("🔍 Found {} keyboard(s) for validation", keyboards.len());
    let mut all_reports = Vec::new();

    for device_info in keyboards {
        println!(
            "\n📋 Validating: {} (PID: 0x{:04x})",
            device_info.name, device_info.product_id
        );

        match manager.open_keyboard(device_info) {
            Ok(mut keyboard) => {
                let report = validator.validate_keyboard(&mut *keyboard).await;

                // Display summary
                let status = if report.is_healthy() {
                    "✅ HEALTHY"
                } else {
                    "❌ ISSUES DETECTED"
                };
                println!("   Status: {}", status);
                println!("   Health Score: {:.1}%", report.health_score);
                println!(
                    "   Tests: {}/{} passed",
                    report.results.iter().filter(|r| r.passed).count(),
                    report.results.len()
                );

                if report.capabilities.per_key_rgb {
                    println!("   🎹 Per-key RGB: Supported ({} keys)", report.capabilities.key_count);
                } else {
                    println!("   🌈 Zone RGB: {} zones", report.capabilities.zone_count);
                }

                if benchmark {
                    println!(
                        "   🚀 Performance: {:.1}ms avg, {:.0} fps effective",
                        report.performance.avg_effect_compute_ms + report.performance.avg_hid_communication_ms,
                        report.performance.effective_refresh_rate
                    );
                }

                // Show failed tests if any
                let failed_tests = report.failed_tests();
                if !failed_tests.is_empty() {
                    println!("   ❌ Failed tests:");
                    for test in failed_tests {
                        println!(
                            "      - {}: {}",
                            test.name,
                            test.error.as_ref().unwrap_or(&"Unknown error".to_string())
                        );
                    }
                }

                all_reports.push(report);
            }
            Err(e) => {
                eprintln!("❌ Failed to open {}: {}", device_info.name, e);
            }
        }
    }

    // Export report if requested
    if let Some(output_path) = output {
        info!("Exporting validation report to: {}", output_path);

        let export_content = if json {
            // Export as JSON
            serde_json::to_string_pretty(&all_reports)
                .map_err(|e| Error::DeviceCommunication(format!("JSON serialization failed: {}", e)))?
        } else {
            // Export as human-readable text
            let mut content = String::new();
            content.push_str("# SteelSeries RGB Validation Report\n\n");
            content.push_str(&format!("Generated: {}\n", chrono::Utc::now().to_rfc3339()));
            content.push_str(&format!("Total devices: {}\n\n", all_reports.len()));

            for (i, report) in all_reports.iter().enumerate() {
                content.push_str(&format!("## Device {} - {}\n", i + 1, report.device_info.name));
                content.push_str(&format!("- Product ID: 0x{:04x}\n", report.device_info.product_id));
                content.push_str(&format!("- Health Score: {:.1}%\n", report.health_score));
                content.push_str(&format!(
                    "- Status: {}\n",
                    if report.is_healthy() {
                        "Healthy"
                    } else {
                        "Issues Detected"
                    }
                ));

                content.push_str("\n### Test Results\n");
                for result in &report.results {
                    let status = if result.passed { "✅" } else { "❌" };
                    content.push_str(&format!("- {} {} ({:.0}ms)\n", status, result.name, result.duration_ms));

                    if let Some(error) = &result.error {
                        content.push_str(&format!("  Error: {}\n", error));
                    }

                    for note in &result.notes {
                        content.push_str(&format!("  Note: {}\n", note));
                    }
                }
                content.push('\n');
            }
            content
        };

        secure_write_async(output_path.clone(), export_content)
            .await
            .map_err(|e| Error::FileSystemError(format!("Failed to write report: {}", e)))?;

        println!("\n📄 Validation report exported to: {}", output_path);
    }

    // Overall summary
    let total_healthy = all_reports.iter().filter(|r| r.is_healthy()).count();
    println!("\n📊 Validation Summary:");
    println!("   Total devices: {}", all_reports.len());
    println!("   Healthy devices: {}/{}", total_healthy, all_reports.len());

    if all_reports.is_empty() {
        println!("   ⚠️  No devices could be opened for validation - check errors above");
    } else if total_healthy == all_reports.len() {
        println!("   🎉 All devices passed validation!");
    } else {
        println!("   ⚠️  Some devices have issues - check details above");
    }

    Ok(())
}

async fn cmd_performance(manager: &DeviceManager, action: PerformanceAction) -> Result<()> {
    let keyboards = manager.keyboards();

    if keyboards.is_empty() {
        println!("⚠️  No SteelSeries keyboards found for performance monitoring");
        return Ok(());
    }

    match action {
        PerformanceAction::Stats { monitor, output, json } => {
            if let Some(interval_seconds) = monitor {
                println!("🔄 Monitoring RGB performance (press Ctrl+C to stop)...");
                loop {
                    display_performance_stats(manager, &keyboards, json)?;

                    if let Some(ref output_path) = output {
                        export_performance_stats(manager, &keyboards, output_path, json).await?;
                    }

                    tokio::time::sleep(Duration::from_secs(interval_seconds)).await;
                }
            } else {
                display_performance_stats(manager, &keyboards, json)?;

                if let Some(output_path) = output {
                    export_performance_stats(manager, &keyboards, &output_path, json).await?;
                    println!("📄 Performance stats exported to: {}", output_path);
                }
            }
        }

        PerformanceAction::Enable => {
            println!("🚀 Enabling performance optimizations...");
            let mut enabled_count = 0;

            for device_info in &keyboards {
                match manager.open_keyboard(device_info) {
                    Ok(mut keyboard) => {
                        keyboard.set_performance_optimization(true);
                        enabled_count += 1;
                        println!("   ✅ {} - Performance optimizations enabled", device_info.name);
                    }
                    Err(e) => {
                        eprintln!("   ❌ {} - Failed to open: {}", device_info.name, e);
                    }
                }
            }

            println!(
                "✅ Performance optimizations enabled on {}/{} keyboards",
                enabled_count,
                keyboards.len()
            );
        }

        PerformanceAction::Disable => {
            println!("⏸️  Disabling performance optimizations...");
            let mut disabled_count = 0;

            for device_info in &keyboards {
                match manager.open_keyboard(device_info) {
                    Ok(mut keyboard) => {
                        keyboard.set_performance_optimization(false);
                        disabled_count += 1;
                        println!("   ✅ {} - Performance optimizations disabled", device_info.name);
                    }
                    Err(e) => {
                        eprintln!("   ❌ {} - Failed to open: {}", device_info.name, e);
                    }
                }
            }

            println!(
                "✅ Performance optimizations disabled on {}/{} keyboards",
                disabled_count,
                keyboards.len()
            );
        }

        PerformanceAction::Cleanup => {
            println!("🧹 Cleaning up performance caches...");
            let mut cleaned_count = 0;

            for device_info in &keyboards {
                match manager.open_keyboard(device_info) {
                    Ok(mut keyboard) => {
                        keyboard.cleanup_rgb_caches();
                        cleaned_count += 1;
                        println!("   ✅ {} - Caches cleaned", device_info.name);
                    }
                    Err(e) => {
                        eprintln!("   ❌ {} - Failed to open: {}", device_info.name, e);
                    }
                }
            }

            println!("✅ Cleaned caches on {}/{} keyboards", cleaned_count, keyboards.len());
        }

        PerformanceAction::Benchmark { duration, output } => {
            println!("🏃 Running performance benchmark for {} seconds...", duration);

            let start_time = Instant::now();
            let mut benchmark_results = HashMap::new();

            // Run benchmark on each keyboard
            for &device_info in &keyboards {
                match manager.open_keyboard(device_info) {
                    Ok(mut keyboard) => {
                        println!("   🔬 Benchmarking: {}", device_info.name);
                        // Enable performance optimizations for benchmark
                        keyboard.set_performance_optimization(true);

                        let device_start = Instant::now();
                        let mut operation_count = 0;

                        let colors = [Color::RED, Color::GREEN, Color::BLUE, Color::WHITE, Color::BLACK];

                        // Run rapid RGB operations for benchmark duration
                        while device_start.elapsed().as_secs() < duration {
                            for color in colors {
                                let _ = keyboard.set_color(color).await;
                                operation_count += 1;

                                // Small delay to prevent overwhelming the device
                                yield_now().await;
                            }
                        }

                        let ops_per_second = operation_count as f64 / duration as f64;
                        benchmark_results.insert(device_info.name.to_string(), ops_per_second);

                        println!("      Operations/second: {:.1}", ops_per_second);

                        // Display performance stats if available
                        if let Some(stats) = keyboard.get_rgb_performance_stats() {
                            println!("      Cache hit rate: {:.1}%", stats.cache_hit_rate * 100.0);
                            println!("      Avg computation: {:.2}μs", stats.avg_computation_time_us);
                            println!("      Current refresh rate: {:.1} Hz", stats.current_refresh_rate);
                        }
                    }
                    Err(e) => {
                        eprintln!("   ❌ {} - Failed to benchmark: {}", device_info.name, e);
                    }
                }
            }

            // Export benchmark results if requested
            if let Some(output_path) = output {
                let benchmark_data = serde_json::json!({
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "duration_seconds": duration,
                    "results": benchmark_results
                });

                secure_write(&output_path, serde_json::to_string_pretty(&benchmark_data)?)
                    .map_err(|e| Error::DeviceCommunication(format!("Failed to write benchmark: {}", e)))?;

                println!("📄 Benchmark results exported to: {}", output_path);
            }

            println!("✅ Benchmark completed in {:.1}s", start_time.elapsed().as_secs_f64());
        }
    }

    Ok(())
}

struct StatsMapSerializer<'a> {
    manager: &'a DeviceManager,
    keyboards: &'a [&'a DeviceInfo],
}

impl<'a> serde::Serialize for StatsMapSerializer<'a> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        for device_info in self.keyboards {
            if let Ok(keyboard) = self.manager.open_keyboard(device_info)
                && let Some(stats) = keyboard.get_rgb_performance_stats()
            {
                map.serialize_entry(&device_info.name, stats)?;
            }
        }
        map.end()
    }
}

fn display_performance_stats(manager: &DeviceManager, keyboards: &[&DeviceInfo], json: bool) -> Result<()> {
    if json {
        let stats_map = StatsMapSerializer { manager, keyboards };
        println!("{}", serde_json::to_string_pretty(&stats_map)?);
    } else {
        println!("📊 RGB Performance Statistics:");
        println!();

        for device_info in keyboards {
            match manager.open_keyboard(device_info) {
                Ok(keyboard) => {
                    println!("🎹 {}", device_info.name);

                    if let Some(stats) = keyboard.get_rgb_performance_stats() {
                        println!("   💾 Cache hit rate: {:.1}%", stats.cache_hit_rate * 100.0);
                        println!("   ⚡ Avg computation: {:.2}μs", stats.avg_computation_time_us);
                        println!("   🔄 Current refresh rate: {:.1} Hz", stats.current_refresh_rate);
                        println!("   📈 Total computations: {}", stats.total_computations);
                        println!("   🔀 Operations batched: {}", stats.hid_operations_batched);
                        println!("   💽 Allocations saved: {}", stats.allocations_saved);
                        println!(
                            "   📊 Memory utilization: {:.1}%",
                            stats.memory_pool_utilization * 100.0
                        );

                        if let Some(frame_time) = keyboard.get_optimal_frame_time() {
                            println!("   🕐 Optimal frame time: {:.1}ms", frame_time.as_millis());
                        }
                    } else {
                        println!("   ⚠️  Performance stats not available (optimizations disabled)");
                    }
                    println!();
                }
                Err(e) => {
                    println!("❌ {}: Failed to open - {}", device_info.name, e);
                    println!();
                }
            }
        }
    }

    Ok(())
}

async fn export_performance_stats(
    manager: &DeviceManager,
    keyboards: &[&DeviceInfo],
    output_path: &str,
    json: bool,
) -> Result<()> {
    if json {
        let stats_map = StatsMapSerializer { manager, keyboards };
        let export_data = serde_json::json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "devices": stats_map
        });

        secure_write_async(output_path.to_string(), serde_json::to_string_pretty(&export_data)?)
            .await
            .map_err(|e| Error::FileSystemError(format!("Failed to write stats: {}", e)))?;
    } else {
        let mut content = String::new();
        content.push_str("# RGB Performance Statistics Report\n\n");
        content.push_str(&format!("Generated: {}\n\n", chrono::Utc::now().to_rfc3339()));

        for device_info in keyboards {
            if let Ok(keyboard) = manager.open_keyboard(device_info) {
                content.push_str(&format!("## {}\n", device_info.name));
                content.push_str(&format!("- Product ID: 0x{:04x}\n", device_info.product_id));

                if let Some(stats) = keyboard.get_rgb_performance_stats() {
                    content.push_str(&format!("- Cache hit rate: {:.1}%\n", stats.cache_hit_rate * 100.0));
                    content.push_str(&format!(
                        "- Avg computation time: {:.2}μs\n",
                        stats.avg_computation_time_us
                    ));
                    content.push_str(&format!(
                        "- Current refresh rate: {:.1} Hz\n",
                        stats.current_refresh_rate
                    ));
                    content.push_str(&format!("- Total computations: {}\n", stats.total_computations));
                    content.push_str(&format!("- Operations batched: {}\n", stats.hid_operations_batched));
                    content.push_str(&format!("- Allocations saved: {}\n", stats.allocations_saved));
                    content.push_str(&format!(
                        "- Memory pool utilization: {:.1}%\n",
                        stats.memory_pool_utilization * 100.0
                    ));
                } else {
                    content.push_str("- Performance stats: Not available\n");
                }
                content.push('\n');
            }
        }

        secure_write_async(output_path.to_string(), content)
            .await
            .map_err(|e| Error::FileSystemError(format!("Failed to write stats: {}", e)))?;
    }

    Ok(())
}

async fn cmd_server(port: u16) -> Result<()> {
    info!("Starting GameSense server on port {}", port);

    let server = GameSenseServer::new("127.0.0.1", port)?;
    server.run().await?;

    Ok(())
}

/// Show device status with optional live monitoring.
async fn cmd_status(_initial_manager: &DeviceManager, device_filter: &str, refresh_ms: u64) -> Result<()> {
    #[derive(Tabled)]
    struct DeviceRow {
        #[tabled(rename = "Name")]
        name: String,
        #[tabled(rename = "Type")]
        device_type: String,
        #[tabled(rename = "VID:PID")]
        vid_pid: String,
        #[tabled(rename = "Path")]
        path: String,
        #[tabled(rename = "Status")]
        status: String,
    }

    // Check if stdout is a TTY
    let is_tty = std::io::stdout().is_terminal();

    // Get initial device list (create new manager for mutations)
    let mut manager = new_device_manager()?;
    manager.refresh()?;

    // Filter devices by type
    let filter_type = match device_filter.to_lowercase().as_str() {
        "keyboard" => Some(DeviceType::Keyboard),
        "headset" => Some(DeviceType::Headset),
        "all" => None,
        _ => {
            return Err(Error::Other(format!(
                "Invalid device filter: {}. Use 'keyboard', 'headset', or 'all'",
                device_filter
            )));
        }
    };

    if is_tty {
        // TTY mode: Real-time updates with progress bars
        let multi_progress = MultiProgress::new();
        let spinner_style = ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] {prefix}: {msg}")
            .map_err(|e| Error::Other(format!("Template error: {}", e)))?;

        // Create progress bars for each device
        let mut progress_bars: Vec<(DeviceInfo, ProgressBar)> = Vec::new();

        for device_info in deduped_devices(&manager) {
            if let Some(filter) = filter_type
                && device_info.device_type != filter
            {
                continue;
            }

            let pb = multi_progress.add(ProgressBar::new_spinner());
            pb.set_style(spinner_style.clone());
            pb.set_prefix(device_info.name.to_string());

            let device_type_str = match device_info.device_type {
                DeviceType::Keyboard => "Keyboard",
                DeviceType::Headset => "Headset",
                DeviceType::Mouse => "Mouse",
                DeviceType::Unknown => "Unknown",
            };

            pb.set_message(format!("Status: Connected | Type: {}", device_type_str));
            progress_bars.push((device_info.clone(), pb));
        }

        if progress_bars.is_empty() {
            println!("No devices found matching filter: {}", device_filter);
            return Ok(());
        }

        println!("Monitoring {} device(s) - Press Ctrl+C to stop", progress_bars.len());

        // Create refresh interval
        let mut interval = tokio::time::interval(Duration::from_millis(refresh_ms));

        // Monitor until Ctrl+C
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Refresh device manager
                    manager.refresh()?;

                    // Update progress bars
                    for (device_info, pb) in &progress_bars {
                        let device_type_str = match device_info.device_type {
                            DeviceType::Keyboard => "Keyboard",
                            DeviceType::Headset => "Headset",
                DeviceType::Mouse => "Mouse",
                            DeviceType::Unknown => "Unknown",
                        };

                        // Check if device still exists
                        let still_connected = manager
                            .devices()
                            .iter()
                            .any(|d| d.path == device_info.path);

                        if still_connected {
                            pb.set_message(format!(
                                "Status: {} | Type: {}",
                                "Connected".green(),
                                device_type_str
                            ));
                        } else {
                            pb.set_message(format!(
                                "Status: {} | Type: {}",
                                "Disconnected".red(),
                                device_type_str
                            ));
                        }
                        pb.tick();
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    println!("
            Stopping device monitoring...");
                    break;
                }
            }
        }
    } else {
        // Non-TTY mode: Single table output
        let mut rows: Vec<DeviceRow> = Vec::new();

        for device_info in deduped_devices(&manager) {
            if let Some(filter) = filter_type
                && device_info.device_type != filter
            {
                continue;
            }

            let device_type_str = match device_info.device_type {
                DeviceType::Keyboard => "Keyboard",
                DeviceType::Headset => "Headset",
                DeviceType::Mouse => "Mouse",
                DeviceType::Unknown => "Unknown",
            };

            rows.push(DeviceRow {
                name: device_info.name.to_string(),
                device_type: device_type_str.to_string(),
                vid_pid: format!("{:04X}:{:04X}", device_info.vendor_id, device_info.product_id),
                path: device_info.path.clone(),
                status: "Connected".to_string(),
            });
        }

        if rows.is_empty() {
            println!("No devices found matching filter: {}", device_filter);
        } else {
            let table = Table::new(rows).to_string();
            println!("{}", table);
        }
    }

    Ok(())
}

/// View or manage HID communication logs.
async fn cmd_hid_logs(file_logging: bool, device_filter: Option<&str>) -> Result<()> {
    // Initialize diagnostics with recording enabled
    init_global_diagnostics(true)?;

    // Enable file logging if requested
    if file_logging {
        with_global_diagnostics(|diag| diag.enable_file_logging())
            .ok_or_else(|| Error::Other("Failed to access diagnostics".to_string()))??;

        info!("HID logging enabled - output will be written to timestamped file");
    }

    // Parse device filter
    let _filter_type = if let Some(filter) = device_filter {
        match filter.to_lowercase().as_str() {
            "keyboard" => Some(DeviceType::Keyboard),
            "headset" => Some(DeviceType::Headset),
            "all" => None,
            _ => {
                return Err(Error::Other(format!(
                    "Invalid device filter: {}. Use 'keyboard', 'headset', or 'all'",
                    filter
                )));
            }
        }
    } else {
        None
    };

    println!("HID Communication Log Viewer");
    println!("============================");
    if let Some(filter) = device_filter {
        println!("Filter: {}", filter);
    }
    println!("Press Ctrl+C to stop and view summary\n");

    // Create interval for periodic summary display
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let start_time = Instant::now();

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Get diagnostic summary
                if let Some(summary) = with_global_diagnostics(|diag| {
                    let analysis = diag.analyze_timing_patterns();
                    format!(
                        "Ops: {} | Failed: {} | Avg Send: {:.2}ms | Avg Recv: {:.2}ms | Elapsed: {:.1}s",
                        analysis.total_operations,
                        analysis.failed_operations,
                        analysis.avg_send_time.as_secs_f64() * 1000.0,
                        analysis.avg_receive_time.as_secs_f64() * 1000.0,
                        start_time.elapsed().as_secs_f64()
                    )
                }) {
                    print!("{}", summary);
                    std::io::Write::flush(&mut std::io::stdout())
                        .map_err(Error::Io)?;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("

Final Summary:");
                println!("==============");

                if let Some(summary) = with_global_diagnostics(|diag| diag.get_summary()) {
                    println!("{}", summary);
                } else {
                    println!("No diagnostic data collected.");
                }

                break;
            }
        }
    }

    Ok(())
}

/// Run automated device tests to verify responsiveness.
async fn cmd_test_device(manager: &DeviceManager, device: &str, benchmark: bool, verbose: bool) -> Result<()> {
    use std::io::{self, IsTerminal};
    use steelseries_gg::validation::{RgbValidator, print_test_results};

    // Create validator with benchmark mode if requested
    let mut validator = RgbValidator::new();
    if benchmark {
        validator = validator.with_benchmarks();
    }

    // Try to find and open the device by name or path
    let device_lower = device.to_lowercase();
    let device_info = deduped_devices(manager)
        .into_iter()
        .find(|d| d.path.contains(device) || d.name.to_lowercase().contains(&device_lower))
        .ok_or_else(|| {
            Error::Other(format!(
                "Device '{}' not found. Use 'ssgg devices' to list available devices.",
                device
            ))
        })?;

    println!("\nTesting device: {}\n", device_info.name);

    // Open device and run validation based on type
    let report = match device_info.device_type {
        DeviceType::Keyboard => {
            let mut keyboard = manager.open_keyboard(device_info)?;
            validator.validate_keyboard(&mut *keyboard).await
        }
        _ => {
            return Err(Error::Other(format!(
                "Device type {:?} not supported for testing yet. Only keyboards supported.",
                device_info.device_type
            )));
        }
    };

    // Display results with colored output if terminal supports it
    let use_colors = io::stdout().is_terminal();
    print_test_results(&report, verbose, use_colors);

    // Exit with appropriate code based on health
    if report.is_healthy() {
        std::process::exit(0);
    } else {
        std::process::exit(1);
    }
}

// Format metrics with color coding using Display structs to avoid intermediate allocations
struct FpsDisplay(f32, f32);
impl std::fmt::Display for FpsDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.1}/{:.1}", self.0, self.1)
    }
}

struct FrameTimeDisplay(f32);
impl std::fmt::Display for FrameTimeDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.2}ms", self.0)
    }
}

struct Colored<'a, T: std::fmt::Display> {
    val: &'a T,
    color_code: &'static str,
}
impl<'a, T: std::fmt::Display> std::fmt::Display for Colored<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\x1b[{}m{}\x1b[0m", self.color_code, self.val)
    }
}

enum MaybeColored<'a, T: std::fmt::Display> {
    Colored(Colored<'a, T>),
    Plain(&'a T),
}
impl<'a, T: std::fmt::Display> std::fmt::Display for MaybeColored<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Colored(c) => c.fmt(f),
            Self::Plain(p) => p.fmt(f),
        }
    }
}

/// Verify RGB performance metrics over time.
async fn cmd_verify_performance(
    manager: &DeviceManager,
    duration: u64,
    effect_name: &str,
    output: Option<String>,
) -> Result<()> {
    use std::io::{self, IsTerminal, Write};
    use steelseries_gg::performance::PerformanceMonitor;
    use tokio::time::{Duration, interval};

    // Find first keyboard device
    let device_info = manager
        .keyboards()
        .first()
        .map(|&d| d.clone())
        .ok_or_else(|| Error::Other("No keyboard device found. Connect a keyboard and try again.".to_string()))?;

    println!("\nMonitoring RGB performance for: {}", device_info.name);
    println!("Duration: {}s | Effect: {}\n", duration, effect_name);

    // Parse effect name to Effect enum
    let effect = match effect_name.to_lowercase().as_str() {
        "breathing" => Effect::Breathing {
            color: Color::CYAN,
            speed: 1.0,
        },
        "spectrum" => Effect::Spectrum { speed: 0.5 },
        "wave" => Effect::Wave {
            colors: vec![Color::RED, Color::GREEN, Color::BLUE],
            speed: 1.0,
            direction: WaveDirection::LeftToRight,
        },
        "static" => Effect::Static { color: Color::RED },
        _ => {
            return Err(Error::Other(format!(
                "Unknown effect '{}'. Valid: breathing, spectrum, wave, static",
                effect_name
            )));
        }
    };

    // Open keyboard and create RGB controller
    let mut keyboard = manager.open_keyboard(&device_info)?;
    let zone_count = keyboard.zone_count();
    let mut controller = RgbController::new(zone_count);
    controller.set_effect(effect.clone());
    controller.set_brightness(0.8);

    // Create performance monitor
    let mut monitor = PerformanceMonitor::new();

    // Determine target FPS for effect

    let complexity = steelseries_gg::performance::calculate_effect_complexity(&effect);
    monitor.set_effect_complexity(complexity);

    let use_colors = io::stdout().is_terminal();
    let start_time = Instant::now();
    let mut frame_interval = interval(Duration::from_millis(16)); // ~60 FPS
    let mut print_interval = interval(Duration::from_secs(1));

    println!("Starting performance monitoring...\n");

    loop {
        tokio::select! {
            _ = frame_interval.tick() => {
                let frame_start = Instant::now();

                // Compute colors
                let compute_start = Instant::now();
                let colors = controller.compute_colors();
                let compute_time = compute_start.elapsed();

                // Send to device - use single color for all zones
                keyboard.set_color(colors[0]).await?;
                keyboard.apply().await?;

                // Record timing
                let frame_duration = frame_start.elapsed();
                monitor.record_frame_timing(frame_duration, compute_time);
            }

            _ = print_interval.tick() => {
                let metrics = monitor.metrics();
                let elapsed = start_time.elapsed().as_secs();

                // Format metrics with color coding using Display structs to avoid intermediate allocations
                let fps_display = FpsDisplay(metrics.actual_fps, metrics.target_fps);
                let fps_colored = if use_colors {
                    let fps_ratio = metrics.actual_fps / metrics.target_fps;
                    let code = if fps_ratio >= 0.8 { "32" } else if fps_ratio >= 0.6 { "33" } else { "31" };
                    MaybeColored::Colored(Colored { val: &fps_display, color_code: code })
                } else {
                    MaybeColored::Plain(&fps_display)
                };

                let frame_time_display = FrameTimeDisplay(metrics.frame_time);
                let frame_time_colored = if use_colors {
                    let code = if metrics.frame_time <= 20.0 { "32" } else { "33" };
                    MaybeColored::Colored(Colored { val: &frame_time_display, color_code: code })
                } else {
                    MaybeColored::Plain(&frame_time_display)
                };

                let dropped_colored = if use_colors && metrics.dropped_frames > 0 {
                    let drop_rate = metrics.dropped_frames as f32 / metrics.total_frames as f32;
                    let code = if drop_rate > 0.05 { "31" } else { "33" };
                    MaybeColored::Colored(Colored { val: &metrics.dropped_frames, color_code: code })
                } else {
                    MaybeColored::Plain(&metrics.dropped_frames)
                };

                print!(
                    "\r[{:02}s] FPS: {} | Frame: {} | Cache: {:.1}% | Dropped: {}     ",
                    elapsed,
                    fps_colored,
                    frame_time_colored,
                    metrics.cache_hit_rate * 100.0,
                    dropped_colored
                );
                io::stdout().flush().ok();

                // Check if duration reached
                if elapsed >= duration {
                    break;
                }
            }
        }
    }

    println!("\n\nPerformance Monitoring Complete\n");
    println!("================================================================================");

    let metrics = monitor.metrics();

    // Print final summary
    println!("Summary:");
    println!("  Total frames:     {}", metrics.total_frames);
    println!(
        "  Average FPS:      {:.1} (target: {:.1})",
        metrics.actual_fps, metrics.target_fps
    );
    println!("  Average frame time: {:.2}ms", metrics.frame_time);
    println!("  Cache hit rate:   {:.1}%", metrics.cache_hit_rate * 100.0);
    println!(
        "  Dropped frames:   {} ({:.2}%)",
        metrics.dropped_frames,
        (metrics.dropped_frames as f32 / metrics.total_frames as f32) * 100.0
    );

    // Export to JSON if requested
    if let Some(output_path) = output {
        let json = serde_json::to_string_pretty(&metrics)
            .map_err(|e| Error::Other(format!("Failed to serialize metrics: {}", e)))?;
        secure_write(&output_path, json)
            .map_err(|e| Error::Other(format!("Failed to write {}: {}", output_path, e)))?;
        println!("\nMetrics exported to: {}", output_path);
    }

    Ok(())
}

async fn wait_for_shutdown() -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sigint = signal(SignalKind::interrupt())?;
        let mut sighup = signal(SignalKind::hangup())?;

        tokio::select! {
            _ = sigterm.recv() => {
                info!("Received SIGTERM, shutting down gracefully...");
            }
            _ = sigint.recv() => {
                info!("Received SIGINT, shutting down gracefully...");
            }
            _ = sighup.recv() => {
                info!("Received SIGHUP, shutting down gracefully...");
            }
        }
    }

    #[cfg(not(unix))]
    {
        use tokio::signal;
        signal::ctrl_c().await?;
        info!("Received Ctrl+C, shutting down gracefully...");
    }

    Ok(())
}
