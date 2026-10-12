# steelseriesgg-rs – SteelSeries GG for Linux

[![License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.97.1%20stable-orange?style=flat-square)](https://www.rust-lang.org/)
![GitHub repo size](https://img.shields.io/github/repo-size/Ven0m0/steelseriesgg-rs)
[![Maintainability](https://qlty.sh/gh/Ven0m0/projects/steelseriesgg-rs/maintainability.svg)](https://qlty.sh/gh/Ven0m0/projects/steelseriesgg-rs)

Open-source replacement for [SteelSeries GG](https://steelseries.com/gg) on Linux: device
settings and lighting (Engine), a Sonar-style audio mixer, GameSense for games, OLED screens,
key bindings and macros, per-game profiles, instant-replay clips (Moments), and a control
panel in the browser.

> **About this fork.** This is [SarhanSoft/steelseriesgg-rs](https://github.com/SarhanSoft/steelseriesgg-rs),
> a fork of [Ven0m0/steelseriesgg-rs](https://github.com/Ven0m0/steelseriesgg-rs) that adds the
> full GG feature set for Linux. It is an independent community project, not affiliated with or
> endorsed by SteelSeries.

> **Hardware status.** Only the Apex Pro TKL (2023) was ever tested on real hardware by this
> project. Every other protocol comes from a published open-source driver (OpenRGB, rivalcfg,
> HeadsetControl, apex-tux, nova-chatmix-linux) and is pinned by unit tests, but has not been
> tried here. The frames this fork writes to put a keybinding into an Apex 7's own memory, and
> the frames that read that memory back, are its own as well: no device was watched answering
> them, so both directions stay pinned by tests alone. The CLI and the control panel mark such
> settings **untested**, and extrapolated ones **guess**. Reports from real devices are what
> turn them into confirmed support.

## Quickstart

```bash
git clone https://github.com/SarhanSoft/steelseriesgg-rs.git
cd steelseriesgg-rs
cargo build --release
cargo build --release --features gui --bin ssgg-gui   # desktop app; needs libwebkit2gtk-4.1-dev
sudo install -Dm644 assets/70-steelseries.rules /etc/udev/rules.d/70-steelseries.rules
sudo udevadm control --reload-rules && sudo udevadm trigger   # then replug your devices
./target/release/ssgg devices
```

### Desktop app

Open **SteelSeries GG** from your application menu (or run `ssgg-gui`). The window starts the
background service when it is not running and shows the full control panel: devices,
lighting, audio mixer, key bindings, profiles, OLED screen and Moments. It needs WebKitGTK 4.1
(Arch `webkit2gtk-4.1`, Debian/Ubuntu `libwebkit2gtk-4.1-0`, Fedora `webkit2gtk4.1`).
Everything is also available from the command line below.

### Background service

Run the background service (hot-plug, animations, battery alerts, GameSense, mixer, bindings):

```bash
sudo install -Dm755 target/release/ssgg target/release/ssgg-gui /usr/local/bin/
sudo install -Dm644 assets/ssgg.desktop /usr/share/applications/ssgg.desktop
sudo sed -i 's|Exec=ssgg-gui|Exec=/usr/local/bin/ssgg-gui|' /usr/share/applications/ssgg.desktop
install -Dm644 assets/ssgg.service ~/.config/systemd/user/ssgg.service
sed -i 's|/usr/bin/ssgg|/usr/local/bin/ssgg|' ~/.config/systemd/user/ssgg.service
systemctl --user daemon-reload
systemctl --user enable --now ssgg
ssgg ui          # opens the desktop app (or the panel in a browser)
```

## What it does, next to GG on Windows

| GG feature | Here | How |
|---|---|---|
| Lighting (Prism), effects, per-zone/per-key | Yes | `ssgg rgb`, panel *Lighting*; one effect synced across keyboards and mice, or per device |
| Device settings: DPI stages, polling rate, buttons, sleep timers, sidetone, mic volume, EQ, ANC... | Yes, per model | `ssgg settings <device>`, `ssgg set <device> <setting> <value>`, panel *Devices* |
| Battery level and low-battery alerts | Yes | `ssgg devices`, desktop notification at 20 % and 10 % |
| Actuation (Apex Pro), Rapid Tap | Where a reference exists | `ssgg actuation set 1.2`, `ssgg set keyboard rapid_tap on` |
| OLED screen (Apex 5/7/Pro) | Yes | `ssgg oled clock\|stats\|nowplaying\|text\|image`, panel *Screen* |
| Sonar: Game/Chat/Media/Aux/Mic devices, ChatMix, 10-band EQ, noise reduction, streamer mix | Yes, on PipeWire | `ssgg mixer ...`, panel *Audio* |
| Key bindings, macros, macro recording, launch apps | Yes (X11 and Wayland) | `ssgg bind`, `ssgg macro record`, panel *Key bindings* |
| Profiles and per-game switching | Yes | `ssgg profile save/load/apps`, switches while a linked program (also Proton games) runs |
| The keyboard's own onboard profiles (Apex 7) | Yes | a profile's key bindings are written into the keyboard itself, `ssgg profile pull` reads one back, panel *Profiles* |
| GameSense (game events → lighting and screens) | Yes | built into the daemon on port 27301; `coreProps.json` is also written into Proton/Wine prefixes |
| Moments (instant replay) | Yes, via gpu-screen-recorder | `ssgg moments enable`, `ssgg moments save` (bind it to a key) |
| Polling rate | Device-side per model; kernel-side up to 1000 Hz | `ssgg set mouse polling_rate 1000`, `sudo ssgg pollrate mouse 1000 --persistent` |

Not provided: firmware updates (a wrong guess could brick a device), Rapid Trigger on boards
whose protocol is not public, tactile/vibration events, and GG's account and cloud features.
Virtual surround needs a SOFA HRTF file set in `mixer.toml`.

## Devices

- **Keyboards:** Apex Pro, Pro TKL, Pro Mini, Pro (2023/2024 Gen 3, wired and wireless), Apex 3,
  3 TKL, 5, 7, 7 TKL, 9, M750, original Apex. Details: [protocol-keyboard.md](docs/development/protocol-keyboard.md),
  [oled.md](docs/development/oled.md).
- **Mice:** 76 models — Aerox 3/5/9 (wired and wireless), Prime family, Rival 3/5/100/110/300/310/500/600/650/700,
  Sensei, Kana, Kinzu. Details: [mice.md](docs/development/mice.md).
- **Headsets:** Arctis 1/5/7/7+/9/Pro/Pro Wireless, Arctis Nova 3/3P/5/7/7P/Pro Wireless, GameBuds.
  Details: [devices.md](docs/development/devices.md).

`ssgg devices` lists what is connected and what each device can do.

## Installation

### Arch Linux

```bash
makepkg -si          # uses the PKGBUILD in this repository
```

Installs `/usr/bin/ssgg`, the user service, udev rules (`70-steelseries.rules`), the uinput
module loader and a desktop entry ("SteelSeries GG" opens the control panel).

### From source (any distribution)

Rust 1.97.1 or newer (`rustup` recommended; distribution Rust may be too old). No C libraries
are needed for the default build: HID uses a pure-Rust hidraw backend.

```bash
cargo build --release
```

Then install the binary, `assets/70-steelseries.rules` (udev), `assets/ssgg-uinput.conf`
(`/etc/modules-load.d/`, for key bindings), `assets/ssgg.service` (systemd user unit) and
`assets/ssgg.desktop` as shown in the Quickstart.

### Runtime tools (optional, per feature)

| Feature | Packages |
|---|---|
| Audio mixer | `pipewire`, `wireplumber`, `pactl` (Arch `libpulse`, Debian/Fedora `pulseaudio-utils`); `noise-suppression-for-voice` for RNNoise |
| Now playing on OLED | `playerctl` |
| Moments | `gpu-screen-recorder` |
| Notifications | `libnotify` (`notify-send`) |
| Opening the panel | `xdg-utils` |

### Permissions

The udev rules give the logged-in user access to SteelSeries HID devices (`uaccess`), to their
input event nodes and to `/dev/uinput` (for key bindings). Replug devices after installing them.
For a service that runs without a login session (`loginctl enable-linger`), also add yourself to
the `input` group.

## Usage

### Devices and settings

```bash
ssgg devices                         # what is connected, battery, lighting, capabilities
ssgg settings mouse                  # every setting with its range and current value
ssgg set mouse sensitivity 400,800,1600
ssgg set mouse polling_rate 1000
ssgg set headset sidetone 2
ssgg set headset equalizer 3,2,0,0,-1,0,1,2,3,2
ssgg set "aerox 3" save now          # store on the mouse itself, where supported
```

A device can be named by type (`keyboard`, `mouse`, `headset`), part of its name, or the key
shown by `ssgg devices`. Settings are remembered and re-applied when a device reconnects.

### Lighting

```bash
ssgg rgb color "#ff5500"                     # every device, synced
ssgg rgb effect breathing --color cyan --speed 0.5
ssgg rgb effect wave --color red --color2 blue
ssgg rgb --device mouse color green          # this device only
ssgg rgb --device mouse sync                 # follow the synced lighting again
ssgg rgb brightness 60
```

Animated effects play while the daemon runs.

### OLED screen

```bash
ssgg oled clock          # idle screen: clock | stats | nowplaying | off
ssgg oled text "Hello" "World" --seconds 10
ssgg oled image ~/Pictures/logo.gif --idle
```

### Audio mixer (Sonar for Linux)

```bash
ssgg mixer enable                    # creates SteelSeries Game/Chat/Media/Aux/Microphone devices
ssgg mixer                           # status
ssgg mixer volume chat 70
ssgg mixer chatmix -30               # -100 all game … 100 all chat; the headset dial also moves it
ssgg mixer eq game fps_footsteps     # or: ssgg mixer eq game 2,1,0,0,0,1,3,4,2,0
ssgg mixer route discord chat        # remembered per application
ssgg mixer noise on                  # microphone noise suppression
ssgg mixer streamer on               # separate "SteelSeries Stream" mix for OBS
ssgg mixer disable                   # removes the devices, restores your defaults
```

Game is set as the default output while the mixer runs; chat programs, browsers and music
players are routed by built-in rules you can change.

### Key bindings and macros

```bash
ssgg bind capslock key ctrl
ssgg bind mouse4 combo ctrl+c
ssgg bind f13 text "gg wp\n"
ssgg bind f14 launch firefox --new-window
ssgg bind f15 media play_pause
ssgg bind f16 profile fps
ssgg macro record f17                # type the sequence, press Esc
ssgg bindings
ssgg unbind capslock
```

Bindings belong to the active profile and work below the display server, so X11 and every
Wayland compositor behave the same. Holding both Ctrl and both Shift keys for two seconds stops
the bindings in an emergency.

### Profiles

```bash
ssgg profile save fps -d "Low DPI, red"
ssgg profile apps fps cs2 valorant.exe   # switch automatically while one of these runs
ssgg profile load default
ssgg profile list
```

A profile stores every device's settings and lighting, the key bindings and (when enabled) the
mixer settings, per device model.

### The keyboard's own memory

An Apex 7 keeps five keybinding profiles inside itself, and a profile here can be put into
that memory rather than replayed by the engine: loading the profile writes its key bindings
into the keyboard, which then answers the remapped keys on its own, so the engine's own key
interception stays off for it. A keyboard that refuses such a binding keeps it the engine's.
Which of the five profiles is written, and which is read back, is the keyboard's own setting:

```bash
ssgg set keyboard onboard-slot 3                  # which onboard profile a write or a read uses
ssgg profile pull                                 # read the keyboard's own configuration back
ssgg profile pull --device apex --slot 2 --name fps   # a chosen keyboard, slot and profile name
ssgg profile list
```

What a read brings back is named by what the keyboard spells inside its own memory - `Config
2`, or `Config Main` once something has renamed the slot - and its bindings come back as the
same records a write compiles, so saving what was read and loading it again writes the
keyboard's own configuration back. A payload whose own checksum does not agree is refused
rather than saved. The scroll wheel of the Apex 7 is part of that memory too: the device binds
volume up to scroll up and volume down to scroll down as its own keybinding, rather than the
engine intercepting a scroll.

### Moments

```bash
ssgg moments enable --seconds 30
ssgg moments save                    # saves the last 30 s to ~/Videos/SteelSeries Moments
```

### Control panel

`ssgg ui` opens the daemon's control panel at `http://127.0.0.1:27311/`. It covers devices,
lighting, audio, key bindings, profiles, the OLED screen and Moments. Append `#demo` to the
address to explore it with simulated devices.

### GameSense

The daemon serves the GameSense API on `127.0.0.1:27301` and writes `coreProps.json` to
`/tmp/steelseries-engine/` and into every existing Steam Proton prefix, `~/.wine` and
Lutris/Bottles prefixes, so Windows games running under Proton find it. Lighting events reach
keyboards, mice and per-key zones; screen events reach OLED keyboards. See
[gamesense-api.md](docs/development/gamesense-api.md).

### Polling rate

Mice that support it change their own rate (`ssgg set mouse polling_rate 1000`). The kernel's
`usbhid` override is also available and applies to every USB mouse or keyboard:

```bash
sudo ssgg pollrate mouse 1000 --persistent   # writes /etc/modprobe.d/ssgg-usbhid.conf
ssgg pollrate status
```

The kernel route stops at 1000 Hz.

## Configuration

`~/.config/ssgg/config.toml` (every key optional):

```toml
default_profile = "default"   # loaded when the daemon starts

[gamesense]
enabled = true
bind_address = "127.0.0.1"
port = 27301

[control]                     # CLI <-> daemon API and control panel
enabled = true
port = 27311

[autoswitch]
enabled = true
notify = true

[moments]
enabled = false
replay_seconds = 30
output_dir = ""               # default ~/Videos/SteelSeries Moments
capture = "screen"            # "portal" on GNOME/KDE Wayland
```

Other files in the same folder: `engine-state.json` (last applied settings and lighting),
`mixer.toml` (audio mixer), `profiles/`.

## Troubleshooting

- **`permission denied` opening a device:** install the udev rules and replug (see Permissions).
- **A setting does nothing:** it is probably marked *untested* or *guess*. Run with `--debug`,
  then open an issue with the model, the command and the log.
- **Key bindings do nothing:** `ssgg bindings` shows the reason; usually `/dev/uinput` access
  (udev rule, `sudo modprobe uinput`).
- **Mixer reports missing tools:** install the packages in *Runtime tools*.
- **Bug report:** `ssgg bug-report` writes a JSON file with system and device diagnostics.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Module guides live in [`docs/development/`](docs/development/).

## License

MIT — see [LICENSE](LICENSE). Protocol facts were taken from OpenRGB (GPL-2.0),
HeadsetControl (GPL-3.0), rivalcfg (WTFPL), apex-tux (Unlicense), apexctl (Apache-2.0) and
nova-chatmix-linux (0BSD); no code from the GPL projects was copied.

## Acknowledgments

- [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB), [rivalcfg](https://github.com/flozz/rivalcfg),
  [HeadsetControl](https://github.com/Sapd/HeadsetControl), [apex-tux](https://github.com/not-jan/apex-tux),
  [apexctl](https://github.com/AstroSnail/apexctl), [nova-chatmix-linux](https://github.com/Dymstro/nova-chatmix-linux),
  [apex-web](https://github.com/trottyva/apex-web), [apex-control](https://github.com/zunuza/apex-control)
- [hidapi](https://crates.io/crates/hidapi), [gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder/about/)
