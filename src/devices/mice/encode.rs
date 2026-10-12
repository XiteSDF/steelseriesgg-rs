//! [EXPERIMENTAL] Typed encoders for SteelSeries mouse reports.
//!
//! Each function here is a port of one rivalcfg value handler (`rivalcfg/handlers/*.py`,
//! WTFPL, commit f16c521) and reproduces its byte output exactly, including its rounding and
//! its error cases. Nothing here touches a device: encoders turn typed values into bytes, and
//! `profile.rs` wraps those bytes in a command. None of it has been tested on hardware by this
//! project; correctness rests on matching rivalcfg's own test expectations.

use std::collections::BTreeMap;

use crate::devices::key_names::{keyboard_code, keyboard_names, multimedia_code, multimedia_names};
use crate::rgb::Color;
use crate::{Error, Result};

/// Report ID prepended to every report. rivalcfg always sends `0x00`.
pub const REPORT_ID: u8 = 0x00;

/// HID report type a command is sent with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportKind {
    /// Output report, sent with `hid_write`.
    Output,
    /// Feature report, sent with `HIDIOCSFEATURE` / `hid_send_feature_report`.
    Feature,
}

impl ReportKind {
    /// rivalcfg's code for this report type (`usbhid.HID_REPORT_TYPE_*`). Its test expectations
    /// start with this byte.
    pub const fn rivalcfg_code(self) -> u8 {
        match self {
            ReportKind::Output => 0x02,
            ReportKind::Feature => 0x03,
        }
    }
}

/// One encoded report, ready to send.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Report {
    pub kind: ReportKind,
    /// Report payload without the report ID.
    pub data: Vec<u8>,
}

impl Report {
    /// Bytes handed to hidapi: [`REPORT_ID`] followed by the payload.
    pub fn wire_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.data.len() + 1);
        bytes.push(REPORT_ID);
        bytes.extend_from_slice(&self.data);
        bytes
    }

    /// The bytes rivalcfg's fake device records for this report: report type, report ID,
    /// payload. Used to compare against rivalcfg's test expectations.
    pub fn rivalcfg_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.data.len() + 2);
        bytes.push(self.kind.rivalcfg_code());
        bytes.extend(self.wire_bytes());
        bytes
    }
}

/// Command bytes, then the encoded value, then the suffix, zero-padded to `packet_length`
/// (`0` = no padding). Mirrors `Mouse._hid_write` in `rivalcfg/mouse.py`.
pub fn frame(command: &[u8], payload: &[u8], suffix: &[u8], packet_length: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(packet_length.max(command.len() + payload.len() + suffix.len()));
    data.extend_from_slice(command);
    data.extend_from_slice(payload);
    data.extend_from_slice(suffix);
    if data.len() < packet_length {
        data.resize(packet_length, 0x00);
    }
    data
}

fn invalid(message: String) -> Error {
    Error::InvalidConfig(message)
}

/// Little-endian bytes of `value` in exactly `width` bytes; fails when it does not fit.
/// Port of `helpers.uint_to_little_endian_bytearray`.
pub fn le_bytes(value: u64, width: u8) -> Result<Vec<u8>> {
    let width = u32::from(width);
    if width < 8 && value >> (8 * width) != 0 {
        return Err(invalid(format!("value {value} does not fit in {width} byte(s)")));
    }
    Ok((0..width)
        .map(|i| value.checked_shr(8 * i).unwrap_or(0) as u8)
        .collect())
}

/// An inclusive arithmetic progression, rivalcfg's `[start, stop, step]` range triple.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub start: u32,
    pub stop: u32,
    pub step: u32,
}

impl Span {
    pub const fn new(start: u32, stop: u32, step: u32) -> Self {
        Self { start, stop, step }
    }

    /// Number of values, as Python's `len(range(start, stop + 1, step))`.
    pub fn len(&self) -> usize {
        if self.step == 0 || self.stop < self.start {
            return 0;
        }
        ((self.stop - self.start) / self.step) as usize + 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Round `value` to the nearest value of the progression, clamped to `start..=stop`.
    /// Port of `range.matches_value_in_range`: a value exactly halfway rounds up.
    pub fn snap(&self, value: i64) -> i64 {
        let (start, stop, step) = (i64::from(self.start), i64::from(self.stop), i64::from(self.step));
        if value <= start {
            return start;
        }
        if value >= stop || step == 0 {
            return stop;
        }
        let delta = (value - start) % step;
        if delta == 0 {
            value
        } else if delta * 2 < step {
            value - delta
        } else {
            value - delta + step
        }
    }

    /// Position of `value` in the progression, when it is one of its members.
    fn index_of(&self, value: i64) -> Option<usize> {
        let offset = value.checked_sub(i64::from(self.start))?;
        if offset < 0 || self.step == 0 || offset % i64::from(self.step) != 0 {
            return None;
        }
        let index = usize::try_from(offset / i64::from(self.step)).ok()?;
        (index < self.len()).then_some(index)
    }

    fn nth(&self, index: usize) -> Option<u32> {
        if index >= self.len() {
            return None;
        }
        u32::try_from(index)
            .ok()?
            .checked_mul(self.step)?
            .checked_add(self.start)
    }
}

/// Map `value` from `input` onto the same position in `output`. Port of
/// `range.process_range`.
pub fn map_range(input: Span, output: Span, value: i64) -> Result<u32> {
    if input.len() != output.len() {
        return Err(invalid(format!(
            "input range {input:?} and output range {output:?} have different lengths"
        )));
    }
    let snapped = input.snap(value);
    input
        .index_of(snapped)
        .and_then(|index| output.nth(index))
        .ok_or_else(|| invalid(format!("value {value} cannot be mapped from {input:?}")))
}

/// The "range" handler (`handlers/range.py`).
pub fn encode_range(input: Span, output: Span, width: u8, value: i64) -> Result<Vec<u8>> {
    le_bytes(u64::from(map_range(input, output, value)?), width)
}

/// Check that a lookup table covers exactly the values of `input`, as `range_choice.py` does.
pub fn check_table(input: Span, table: &[(u32, u32)]) -> Result<()> {
    let min = table.iter().map(|(k, _)| *k).min();
    let max = table.iter().map(|(k, _)| *k).max();
    if table.len() != input.len() || min != Some(input.start) || max != Some(input.stop) {
        return Err(invalid(format!("lookup table does not match input range {input:?}")));
    }
    Ok(())
}

/// Output of the table entry whose input is nearest to `value`; on a tie the lower input
/// wins. Port of `range_choice.find_nearest_choice`.
pub fn nearest_choice(table: &[(u32, u32)], value: i64) -> Option<u32> {
    table
        .iter()
        .min_by_key(|(input, _)| ((i64::from(*input) - value).unsigned_abs(), *input))
        .map(|(_, output)| *output)
}

/// The "range_choice" handler (`handlers/range_choice.py`).
pub fn encode_range_choice(input: Span, table: &[(u32, u32)], width: u8, value: i64) -> Result<Vec<u8>> {
    check_table(input, table)?;
    let output = nearest_choice(table, value).ok_or_else(|| invalid("empty lookup table".to_string()))?;
    le_bytes(u64::from(output), width)
}

/// One option of a "choice" setting: the stable id and the bytes it sends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChoiceEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub bytes: &'static [u8],
}

impl ChoiceEntry {
    pub const fn new(id: &'static str, label: &'static str, bytes: &'static [u8]) -> Self {
        Self { id, label, bytes }
    }
}

/// The "choice" handler (`handlers/choice.py`).
pub fn encode_choice(entries: &[ChoiceEntry], id: &str) -> Result<Vec<u8>> {
    entries
        .iter()
        .find(|e| e.id == id)
        .map(|e| e.bytes.to_vec())
        .ok_or_else(|| {
            let ids: Vec<&str> = entries.iter().map(|e| e.id).collect();
            invalid(format!("'{id}' is not one of: {}", ids.join(", ")))
        })
}

/// How a DPI value becomes a sensor code.
#[derive(Clone, Copy, Debug)]
pub enum DpiEncoding {
    /// Linear mapping, as the "range" handler (`multidpi_range.py`).
    Range { input: Span, output: Span },
    /// Lookup table, as the "range_choice" handler (`multidpi_range_choice*.py`).
    Table { input: Span, table: &'static [(u32, u32)] },
}

impl DpiEncoding {
    /// DPI values the device accepts.
    pub const fn input(&self) -> Span {
        match self {
            DpiEncoding::Range { input, .. } | DpiEncoding::Table { input, .. } => *input,
        }
    }

    fn code(&self, dpi: u32) -> Result<u32> {
        match self {
            DpiEncoding::Range { input, output } => map_range(*input, *output, i64::from(dpi)),
            DpiEncoding::Table { input, table } => {
                check_table(*input, table)?;
                nearest_choice(table, i64::from(dpi)).ok_or_else(|| invalid("empty DPI table".to_string()))
            }
        }
    }
}

/// How the stage count byte is written (`count_mode` in `multidpi_range.py`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CountMode {
    /// The number of stages.
    Number,
    /// One bit per stage: 3 stages -> `0b111`.
    Flag,
}

/// Order of X and Y codes for mice with separate X/Y DPI (`xy_mapping` in
/// `multidpi_range_choice_xy.py`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XyLayout {
    /// `x1 y1 x2 y2 ...` ("xyxy").
    Interleaved,
    /// All X codes padded to the maximum stage count, then all Y codes ("xxyy").
    Grouped,
}

/// Layout of a multi-stage DPI command.
#[derive(Clone, Copy, Debug)]
pub struct MultiDpi {
    pub encoding: DpiEncoding,
    /// Bytes per DPI code (`dpi_length_byte`).
    pub width: u8,
    /// Index the device uses for the first stage (`first_preset`); rivalcfg always selects it.
    pub first_preset: u8,
    pub count_mode: CountMode,
    pub max_stages: u8,
    /// `Some` when each stage carries separate X and Y codes.
    pub xy: Option<XyLayout>,
}

/// The "multidpi_range", "multidpi_range_choice" and "multidpi_range_choice_xy" handlers.
///
/// Each stage is `(x, y)`; mice without separate axes require `x == y`. `selected` is the
/// zero-based stage made active (rivalcfg's CLI always selects the first one).
pub fn encode_multidpi(spec: &MultiDpi, stages: &[(u32, u32)], selected: usize) -> Result<Vec<u8>> {
    if stages.is_empty() {
        return Err(invalid("at least one DPI stage is required".to_string()));
    }
    if stages.len() > usize::from(spec.max_stages) {
        return Err(invalid(format!(
            "{} DPI stages given but the device accepts at most {}",
            stages.len(),
            spec.max_stages
        )));
    }
    if selected >= stages.len() {
        return Err(invalid(format!("selected DPI stage {selected} does not exist")));
    }
    let selected = u8::try_from(selected)
        .ok()
        .and_then(|s| s.checked_add(spec.first_preset))
        .ok_or_else(|| invalid(format!("selected DPI stage {selected} is out of range")))?;
    let width = spec.width;
    let mut codes = Vec::new();
    match spec.xy {
        None => {
            for (x, y) in stages {
                if x != y {
                    return Err(invalid("this mouse takes one DPI value per stage".to_string()));
                }
                codes.extend(le_bytes(u64::from(spec.encoding.code(*x)?), width)?);
            }
        }
        Some(XyLayout::Interleaved) => {
            for (x, y) in stages {
                codes.extend(le_bytes(u64::from(spec.encoding.code(*x)?), width)?);
                codes.extend(le_bytes(u64::from(spec.encoding.code(*y)?), width)?);
            }
        }
        Some(XyLayout::Grouped) => {
            let padding = usize::from(spec.max_stages) - stages.len();
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            for (x, y) in stages {
                xs.extend(le_bytes(u64::from(spec.encoding.code(*x)?), width)?);
                ys.extend(le_bytes(u64::from(spec.encoding.code(*y)?), width)?);
            }
            xs.resize(xs.len() + padding * usize::from(width), 0x00);
            ys.resize(ys.len() + padding * usize::from(width), 0x00);
            codes.extend(xs);
            codes.extend(ys);
        }
    }
    let count = stages.len() as u8;
    let count = match spec.count_mode {
        CountMode::Number => count,
        CountMode::Flag => 0xFF >> (8 - count),
    };
    let mut payload = vec![count, selected];
    payload.extend(codes);
    Ok(payload)
}

/// The "rgbcolor" handler (`handlers/rgbcolor.py`).
pub const fn encode_rgb(color: Color) -> [u8; 3] {
    [color.r, color.g, color.b]
}

/// The "reactive_rgbcolor" handler (`handlers/reactive_rgbcolor.py`); `None` disables the
/// reaction.
pub fn encode_reactive(color: Option<Color>) -> Vec<u8> {
    match color {
        None => vec![0x00; 5],
        Some(c) => vec![0x01, 0x00, c.r, c.g, c.b],
    }
}

/// One color stop of a gradient; `position` is a percentage, 0-100.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GradientStop {
    pub position: u8,
    pub color: Color,
}

/// A looping color gradient.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Gradient {
    pub duration_ms: u32,
    pub stops: Vec<GradientStop>,
}

/// What a gradient-capable LED shows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ColorEffect {
    /// One color, no animation.
    Steady(Color),
    Gradient(Gradient),
}

/// rivalcfg's gradient duration when none is given (`_default_duration`).
pub const DEFAULT_GRADIENT_DURATION_MS: u32 = 1000;

/// Most color stops a gradient may have.
pub const MAX_GRADIENT_STOPS: usize = 14;

fn no_color() -> Error {
    invalid("a gradient needs at least one color".to_string())
}

/// Gradient stops after rivalcfg's smoothing: a gradient that does not end at 100 % gets the
/// first color appended at 100 % so the loop wraps without a jump (`_handle_rgbgradient_dict`).
/// The bool is `true` for a real gradient and `false` for a steady color.
fn gradient_stops(effect: &ColorEffect) -> Result<(bool, u32, Vec<GradientStop>)> {
    match effect {
        ColorEffect::Steady(color) => Ok((
            false,
            DEFAULT_GRADIENT_DURATION_MS,
            vec![GradientStop {
                position: 0,
                color: *color,
            }],
        )),
        ColorEffect::Gradient(gradient) => {
            let mut stops = gradient.stops.clone();
            let (first, last) = match (stops.first(), stops.last()) {
                (Some(first), Some(last)) => (*first, *last),
                _ => return Err(no_color()),
            };
            if stops.len() < MAX_GRADIENT_STOPS && last.position != 100 {
                stops.push(GradientStop {
                    position: 100,
                    color: first.color,
                });
            }
            Ok((true, gradient.duration_ms, stops))
        }
    }
}

/// Byte offsets of the "rgbgradient" header (`rgbgradient_header` in rivalcfg profiles).
#[derive(Clone, Copy, Debug)]
pub struct GradientLayout {
    pub header_len: usize,
    pub led_id_offsets: &'static [usize],
    pub duration_offset: usize,
    pub duration_len: u8,
    pub repeat_offset: usize,
    pub triggers_offset: usize,
    pub color_count_offset: usize,
}

fn put(buffer: &mut [u8], offset: usize, value: u8) -> Result<()> {
    let slot = buffer
        .get_mut(offset)
        .ok_or_else(|| invalid(format!("offset {offset} lies outside the report")))?;
    *slot = value;
    Ok(())
}

/// The "rgbgradient" handler (`handlers/rgbgradient.py`), used by the Rival 310, Rival 600,
/// Sensei 310 and Sensei TEN.
pub fn encode_gradient(layout: &GradientLayout, led_id: u8, effect: &ColorEffect) -> Result<Vec<u8>> {
    let (is_gradient, duration, stops) = gradient_stops(effect)?;
    if stops.len() > MAX_GRADIENT_STOPS {
        return Err(invalid(format!("at most {MAX_GRADIENT_STOPS} color stops are allowed")));
    }
    // rivalcfg never sets button triggers, so the repeat flag only marks steady colors.
    let triggers = 0x00;
    let repeat = u8::from(!is_gradient || triggers != 0x00);

    let mut header = vec![0x00; layout.header_len];
    put(&mut header, layout.repeat_offset, repeat)?;
    put(&mut header, layout.triggers_offset, triggers)?;
    put(&mut header, layout.color_count_offset, stops.len() as u8)?;
    for offset in layout.led_id_offsets {
        put(&mut header, *offset, led_id)?;
    }
    for (i, byte) in le_bytes(u64::from(duration), layout.duration_len)?
        .into_iter()
        .enumerate()
    {
        put(&mut header, layout.duration_offset + i, byte)?;
    }

    let first = stops.first().ok_or_else(no_color)?.color;
    let mut body = encode_rgb(first).to_vec();
    let mut last_real_position = 0u32;
    for stop in &stops {
        if stop.position > 100 {
            return Err(invalid(format!("color stop position {}% exceeds 100%", stop.position)));
        }
        let real_position = u32::from(stop.position) * 255 / 100;
        let step = real_position
            .checked_sub(last_real_position)
            .ok_or_else(|| invalid("gradient color stops must be in ascending order".to_string()))?;
        body.extend(encode_rgb(stop.color));
        body.push(step as u8);
        last_real_position = real_position;
    }

    header.extend(body);
    Ok(header)
}

/// Sizes of the "rgbgradientv2" command (`rgbgradientv2_header` in rivalcfg profiles).
#[derive(Clone, Copy, Debug)]
pub struct GradientV2Layout {
    /// Length the LED id, start header and stages are padded to.
    pub color_field_len: usize,
    pub duration_len: u8,
    pub max_stops: usize,
}

/// Fixed bytes after the LED id; rivalcfg marks them "[WIP] header command".
const GRADIENT_V2_START: [u8; 8] = [0x1D, 0x01, 0x02, 0x31, 0x51, 0xFF, 0xC8, 0x00];
/// Fixed bytes after the start color; rivalcfg notes they still "need to be fully tested".
const GRADIENT_V2_END: [u8; 2] = [0xFF, 0x00];
const GRADIENT_V2_FOCAL_X: u64 = 1500;
const GRADIENT_V2_FOCAL_Y: u64 = 650;
const GRADIENT_V2_TAIL: [u8; 6] = [0x00, 0x00, 0x00, 0x00, 0x01, 0x00];
/// SteelSeries Engine refuses gradients longer than 30 s (rivalcfg).
pub const GRADIENT_V2_MAX_DURATION_MS: u32 = 30_000;

/// Shortest duration SteelSeries Engine allows for a gradient of `stop_count` stops (rivalcfg:
/// `int(stop_count * 33.3)`).
pub fn gradient_v2_min_duration(stop_count: usize) -> u32 {
    (stop_count as f64 * 33.3) as u32
}

/// The "rgbgradientv2" handler (`handlers/rgbgradientv2.py`), used by the Rival 500 and
/// Rival 700/710.
pub fn encode_gradient_v2(layout: &GradientV2Layout, led_id: u8, effect: &ColorEffect) -> Result<Vec<u8>> {
    let (_, duration, stops) = gradient_stops(effect)?;
    let stop_count = stops.len();
    if stop_count > layout.max_stops {
        return Err(invalid(format!("at most {} color stops are allowed", layout.max_stops)));
    }
    let minimum = gradient_v2_min_duration(stop_count);
    if duration < minimum {
        return Err(invalid(format!(
            "a duration of {minimum} ms or more is needed for {stop_count} color stops"
        )));
    }
    if duration > GRADIENT_V2_MAX_DURATION_MS {
        return Err(invalid(format!(
            "a duration of at most {GRADIENT_V2_MAX_DURATION_MS} ms is allowed"
        )));
    }

    let mut header = vec![led_id];
    header.extend_from_slice(&GRADIENT_V2_START);

    // Each stage: index, padding, signed R/G/B ramp per ms * 16, padding, time since the
    // previous stage in ms (little endian).
    let start = *stops.first().ok_or_else(no_color)?;
    let mut last_position = start.position;
    let mut previous = [start.color.r, start.color.g, start.color.b];
    for (index, stop) in stops.iter().skip(1).enumerate() {
        if stop.position <= last_position {
            return Err(invalid(
                "gradient color stops must be in strictly ascending order".to_string(),
            ));
        }
        let time = (f64::from(duration) / 100.0 * f64::from(stop.position - last_position)) as u32;
        last_position = stop.position;
        if time == 0 {
            return Err(invalid(
                "gradient timings are too short; use a longer duration".to_string(),
            ));
        }
        header.push(index as u8);
        header.push(0x00);
        let target = encode_rgb(stop.color);
        for (channel, old) in target.iter().zip(previous.iter_mut()) {
            let diff = i32::from(*channel) - i32::from(*old);
            let ramp = (f64::from(diff) / f64::from(time) * 16.0) as i64;
            header.push((ramp & 0xFF) as u8);
            *old = *channel;
        }
        header.push(0x00);
        header.extend(le_bytes(u64::from(time), 2)?);
    }
    if header.len() < layout.color_field_len {
        header.resize(layout.color_field_len, 0x00);
    }

    // The start color is split into nibbles: low nibble shifted up, then high nibble.
    for channel in encode_rgb(start.color) {
        header.push((channel & 0x0F) << 4);
        header.push(channel >> 4);
    }
    header.extend_from_slice(&GRADIENT_V2_END);
    header.extend(le_bytes(GRADIENT_V2_FOCAL_X, 2)?);
    header.extend(le_bytes(GRADIENT_V2_FOCAL_Y, 2)?);
    header.extend_from_slice(&GRADIENT_V2_TAIL);
    header.extend(le_bytes((stop_count - 1) as u64, 2)?);
    header.extend(le_bytes(u64::from(duration), layout.duration_len)?);
    Ok(header)
}

/// One remappable physical button.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ButtonDef {
    /// Lowercase name, e.g. `"button1"` or `"scrollup"`.
    pub name: &'static str,
    /// Code written when another button is mapped to this one's function.
    pub id: u8,
    /// Byte offset of this button's field in the mapping payload.
    pub offset: usize,
    /// Lowercase action this button performs by default.
    pub default: &'static str,
}

impl ButtonDef {
    pub const fn new(name: &'static str, id: u8, offset: usize, default: &'static str) -> Self {
        Self {
            name,
            id,
            offset,
            default,
        }
    }
}

/// Everything the "buttons" handler needs about a mouse (`handlers/buttons/buttons.py`).
#[derive(Clone, Copy, Debug)]
pub struct ButtonLayout {
    pub buttons: &'static [ButtonDef],
    /// Bytes per button field.
    pub field_len: usize,
    pub disable: Option<u8>,
    pub dpi_switch: Option<u8>,
    pub scroll_up: Option<u8>,
    pub scroll_down: Option<u8>,
    /// Field prefix for a keyboard key; the key's HID usage follows.
    pub keyboard: Option<u8>,
    /// Field prefix for a multimedia key; the key's consumer usage follows.
    pub multimedia: Option<u8>,
}

/// Value to remap every button back to its default.
pub const DEFAULT_ACTION: &str = "default";

impl ButtonLayout {
    fn special(&self, action: &str) -> Option<u8> {
        match action {
            "disable" | "disabled" => self.disable,
            "dpi" => self.dpi_switch,
            "scrollup" => self.scroll_up,
            "scrolldown" | "scrolldwn" | "scrolldn" => self.scroll_down,
            _ => None,
        }
    }

    /// Every action name this mouse accepts, lowercase, without duplicates, in a stable order:
    /// `default`, buttons, special actions, multimedia keys, keyboard keys.
    pub fn action_names(&self) -> Vec<String> {
        let mut names: Vec<String> = vec![DEFAULT_ACTION.to_string()];
        let mut push = |name: &str| {
            if !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        };
        for button in self.buttons {
            push(button.name);
        }
        for (name, code) in [
            ("disable", self.disable),
            ("disabled", self.disable),
            ("dpi", self.dpi_switch),
            ("scrollup", self.scroll_up),
            ("scrolldown", self.scroll_down),
            ("scrolldwn", self.scroll_down),
            ("scrolldn", self.scroll_down),
        ] {
            if code.is_some() {
                push(name);
            }
        }
        if self.multimedia.is_some() {
            multimedia_names().for_each(&mut push);
        }
        if self.keyboard.is_some() {
            keyboard_names().for_each(&mut push);
        }
        names
    }

    /// Default mapping, button name -> action.
    pub fn default_mapping(&self) -> BTreeMap<String, String> {
        self.buttons
            .iter()
            .map(|b| (b.name.to_string(), b.default.to_string()))
            .collect()
    }
}

/// The "buttons" handler (`handlers/buttons/buttons.py`). `mapping` holds only the buttons to
/// change (button -> action, case-insensitive); every other button keeps its default.
pub fn encode_buttons(layout: &ButtonLayout, mapping: &BTreeMap<String, String>) -> Result<Vec<u8>> {
    let mut actions: Vec<String> = layout.buttons.iter().map(|b| b.default.to_string()).collect();
    for (button, action) in mapping {
        let button = button.to_ascii_lowercase();
        if button == "layout" {
            if !action.eq_ignore_ascii_case("qwerty") {
                return Err(invalid(format!("unsupported keyboard layout '{action}'")));
            }
            continue;
        }
        let index = layout
            .buttons
            .iter()
            .position(|b| b.name == button)
            .ok_or_else(|| invalid(format!("unknown button name '{button}'")))?;
        if !action.eq_ignore_ascii_case(DEFAULT_ACTION) {
            actions[index] = action.clone();
        }
    }

    let mut packet = vec![0x00; layout.buttons.len() * layout.field_len];
    for (button, action) in layout.buttons.iter().zip(&actions) {
        let action = action.to_ascii_lowercase();
        let (first, second) = if let Some(target) = layout.buttons.iter().find(|b| b.name == action) {
            (target.id, None)
        } else if let Some(code) = layout.special(&action) {
            (code, None)
        } else if let (Some(prefix), Some(code)) = (layout.multimedia, multimedia_code(&action)) {
            (prefix, Some(code))
        } else if let (Some(prefix), Some(code)) = (layout.keyboard, keyboard_code(&action)) {
            (prefix, Some(code))
        } else {
            return Err(invalid(format!("unknown button, key or action '{action}'")));
        };
        put(&mut packet, button.offset, first)?;
        if let Some(second) = second {
            put(&mut packet, button.offset + 1, second)?;
        }
    }
    Ok(packet)
}
