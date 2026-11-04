// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The **`segment_display` primitive**: a multiplexed N-digit segment LED
//! display (7, 14 or 16 segments, or any other count) that the MCU drives
//! directly from GPIO segment lines and digit-select lines.
//!
//! # The part
//!
//! A bare LED display with no driver chip. All digits share the segment lines;
//! each digit has its own select line (its common anode or cathode, often
//! through a transistor). Firmware lights ONE digit at a time: it puts that
//! digit's segment pattern on the segment lines, turns on its select line,
//! waits, and moves to the next digit. The eye averages the light over time
//! (persistence of vision), so a human sees all digits lit at once.
//!
//! # The model
//!
//! * A segment LED of a digit emits light only while its segment line AND the
//!   digit's select line are BOTH active. Active levels are configuration
//!   (`segment_active_high`, `digit_active_high`).
//! * The model INTEGRATES that light over time, exactly, in CPU cycles: each
//!   GPIO store moves the pads, and the time since the previous store is added
//!   to every (digit, segment) LED that was lit during it.
//! * Every `persistence_us` (a persistence-of-vision window) the model decides
//!   what a human sees: an LED is visible when its on-time is at least
//!   `threshold_pct` of the brightest LED in that window, and at least
//!   `min_duty_pct` of the window. So a short ghost (segments changed before
//!   the digit select moved) is dark, as it is on the real display, and a
//!   display that is not refreshed is blank.
//! * The visible pattern of each digit is decoded to a character with a font:
//!   a built-in 7- or 14-segment font, plus `glyphs` from the placement, which
//!   take priority. A pattern the font does not know shows as `?`. A segment
//!   named `dp` is the decimal point: it is not part of the glyph and shows as
//!   `.` after the character.
//!
//! # Readback
//!
//! * [`BusResidentDevice::logs`]: `text` has one line per change of the
//!   visible text (`"P3C" at cycle 3200000000`); `frames` has one line per
//!   change of the visible segment masks (`0x0629 0x0000 0x0000 at cycle ...`).
//!   `labwired test` asserts on them with `peripheral_log`.
//! * [`BusResidentDevice::evidence`]: a `text_display` artifact with `text`,
//!   `texts_seen` (distinct panels across the run), `printable_chars`, and the
//!   per-digit `segments` masks.
//!
//! # Where it sits
//!
//! Exactly where the `gpio_device` and `logic_gate` primitives sit: one
//! [`BusResidentDevice`] on `SystemBus::gpio_devices`. It is serviced from the
//! MMIO write hook (a store to a GPIO port that hosts one of its pads), so the
//! integration sees every edge at its exact cycle. The end of each window is
//! a resident deadline, so a window closes also when firmware stops writing.
//! It needs no per-cycle tick: the pads move only on a store.
//!
//! # Not modelled
//!
//! * Brightness levels: a segment is visible or it is not.
//! * The pad direction: the output register level is taken as the pad level
//!   (a pad left as an input with its output bit set counts as driven).
//! * Electrical faults (two digits selected at once light both, as on a real
//!   display, but current sharing is not modelled).

use std::collections::VecDeque;

use anyhow::{anyhow, bail, Result};

use super::seven_seg_font;
use crate::bus::{BusResidentDevice, DevicePins};
use crate::inspect::{artifact_generation, Artifact, DeviceEvidence, InspectOpts};
use crate::peripheral_log::PeripheralLog;
use crate::sim_input::{InputChannel, SimInput, SimInputError};

/// Most log lines kept per log. The oldest lines go first.
pub const LOG_CAPACITY: usize = 4096;

/// Most distinct visible texts kept in artifact `texts_seen` (ring, oldest first).
const TEXTS_SEEN_CAP: usize = 32;

/// Most segment lines, and most digits, one display can have.
pub const MAX_LINES: usize = 32;

/// Segment names of the built-in 7-segment font, in the bit order of
/// [`seven_seg_font`] (`a` = bit 0 .. `g` = bit 6, `dp` = bit 7).
pub const SEVEN_SEGMENT_NAMES: &[&str] = &["a", "b", "c", "d", "e", "f", "g", "dp"];

/// Segment names of the built-in 14-segment font.
///
/// `a`..`f` are the outer segments as on a 7-segment digit, `g1` / `g2` the
/// left and right halves of the middle bar. The six inner segments:
/// `h` upper-left diagonal, `j` upper vertical, `k` upper-right diagonal,
/// `l` lower-left diagonal, `m` lower vertical, `n` lower-right diagonal.
/// `dp` is the decimal point.
pub const FOURTEEN_SEGMENT_NAMES: &[&str] = &[
    "a", "b", "c", "d", "e", "f", "g1", "g2", "h", "j", "k", "l", "m", "n", "dp",
];

/// The built-in 14-segment font: character and the segments it lights.
/// When two characters have the same pattern (`5` and `S`), the first wins.
const FOURTEEN_SEGMENT_FONT: &[(char, &str)] = &[
    ('0', "a b c d e f k l"),
    ('1', "b c"),
    ('2', "a b d e g1 g2"),
    ('3', "a b c d g2"),
    ('4', "b c f g1 g2"),
    ('5', "a c d f g1 g2"),
    ('6', "a c d e f g1 g2"),
    ('7', "a b c"),
    ('8', "a b c d e f g1 g2"),
    ('9', "a b c d f g1 g2"),
    ('A', "a b c e f g1 g2"),
    ('B', "a b c d g2 j m"),
    ('C', "a d e f"),
    ('D', "a b c d j m"),
    ('E', "a d e f g1 g2"),
    ('F', "a e f g1"),
    ('G', "a c d e f g2"),
    ('H', "b c e f g1 g2"),
    ('I', "a d j m"),
    ('J', "b c d e"),
    ('K', "e f g1 k n"),
    ('L', "d e f"),
    ('M', "b c e f h k"),
    ('N', "b c e f h n"),
    ('O', "a b c d e f"),
    ('P', "a b e f g1 g2"),
    ('Q', "a b c d e f n"),
    ('R', "a b e f g1 g2 n"),
    ('S', "a c d f g1 g2"),
    ('T', "a j m"),
    ('U', "b c d e f"),
    ('V', "e f k l"),
    ('W', "b c e f l n"),
    ('X', "h k l n"),
    ('Y', "h k m"),
    ('Z', "a d k l"),
    ('-', "g1 g2"),
];

/// Which built-in font a placement starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseFont {
    None,
    SevenSegment,
    FourteenSegment,
}

impl BaseFont {
    fn parse(name: &str) -> Result<Self> {
        match name {
            "none" => Ok(Self::None),
            "seven_segment" => Ok(Self::SevenSegment),
            "fourteen_segment" => Ok(Self::FourteenSegment),
            other => bail!("unknown font '{other}'; use seven_segment, fourteen_segment or none"),
        }
    }

    /// Default by segment count: 7/8 lines are a 7-segment digit (with or
    /// without dp), 14/15 lines a 14-segment digit.
    fn for_count(count: usize) -> Self {
        match count {
            7 | 8 => Self::SevenSegment,
            14 | 15 => Self::FourteenSegment,
            _ => Self::None,
        }
    }

    fn default_names(self) -> Option<&'static [&'static str]> {
        match self {
            Self::SevenSegment => Some(SEVEN_SEGMENT_NAMES),
            Self::FourteenSegment => Some(FOURTEEN_SEGMENT_NAMES),
            Self::None => None,
        }
    }

    /// The font as (character, segment names).
    fn glyphs(self) -> Vec<(char, Vec<String>)> {
        match self {
            Self::None => Vec::new(),
            Self::SevenSegment => seven_seg_font::FONT
                .iter()
                .filter(|(pattern, _)| *pattern != 0)
                .map(|(pattern, ch)| {
                    let names = (0..7)
                        .filter(|bit| pattern & (1 << bit) != 0)
                        .map(|bit| SEVEN_SEGMENT_NAMES[bit].to_string())
                        .collect();
                    (*ch, names)
                })
                .collect(),
            Self::FourteenSegment => FOURTEEN_SEGMENT_FONT
                .iter()
                .map(|(ch, segs)| (*ch, segs.split_whitespace().map(String::from).collect()))
                .collect(),
        }
    }
}

/// Everything a placement says about the display, before pad resolution.
#[derive(Debug, Clone)]
pub struct SegmentDisplaySpec {
    /// Pad labels of the segment lines. Index = segment bit in a mask.
    pub segment_pins: Vec<String>,
    /// Pad labels of the digit-select lines, leftmost digit first.
    pub digit_pins: Vec<String>,
    /// Name of each segment line, same order as `segment_pins`.
    pub segment_names: Vec<String>,
    pub segment_active_high: bool,
    pub digit_active_high: bool,
    /// Glyph mask (dp excluded) → character, placement glyphs first.
    pub font: Vec<(u32, char)>,
    /// Bit of the decimal point, when a segment is named `dp`.
    pub dp_bit: Option<usize>,
    pub persistence_us: u64,
    pub threshold_pct: u64,
    pub min_duty_pct: u64,
}

/// Read a YAML value as a list of strings: a sequence, or one string split at
/// spaces and commas.
fn string_list(value: &serde_yaml::Value, what: &str) -> Result<Vec<String>> {
    match value {
        serde_yaml::Value::Sequence(items) => items
            .iter()
            .map(|v| match v {
                serde_yaml::Value::String(s) => Ok(s.clone()),
                serde_yaml::Value::Number(n) => Ok(n.to_string()),
                other => Err(anyhow!("{what}: expected a string, got {other:?}")),
            })
            .collect(),
        serde_yaml::Value::String(s) => Ok(s
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()),
        other => Err(anyhow!("{what}: expected a list, got {other:?}")),
    }
}

fn glyph_char(key: &serde_yaml::Value) -> Result<char> {
    let text = match key {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Number(n) => n.to_string(),
        other => bail!("glyphs: a key must be one character, got {other:?}"),
    };
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(c),
        _ => bail!("glyphs: key '{text}' must be exactly one character"),
    }
}

fn get_bool(
    config: &std::collections::HashMap<String, serde_yaml::Value>,
    key: &str,
    default: bool,
) -> Result<bool> {
    match config.get(key) {
        None => Ok(default),
        Some(serde_yaml::Value::Bool(b)) => Ok(*b),
        Some(other) => bail!("{key}: expected true or false, got {other:?}"),
    }
}

impl SegmentDisplaySpec {
    /// Parse the placement `config:`. `params` gives the numeric settings
    /// (already resolved from placement or descriptor defaults).
    pub fn from_config(
        config: &std::collections::HashMap<String, serde_yaml::Value>,
        persistence_us: u64,
        threshold_pct: u64,
        min_duty_pct: u64,
    ) -> Result<Self> {
        let segment_pins = string_list(
            config
                .get("segment_pins")
                .ok_or_else(|| anyhow!("config is missing the 'segment_pins' list"))?,
            "segment_pins",
        )?;
        let digit_pins = string_list(
            config
                .get("digit_pins")
                .ok_or_else(|| anyhow!("config is missing the 'digit_pins' list"))?,
            "digit_pins",
        )?;
        if segment_pins.is_empty() || segment_pins.len() > MAX_LINES {
            bail!(
                "segment_pins must list 1..={MAX_LINES} pads, got {}",
                segment_pins.len()
            );
        }
        if digit_pins.is_empty() || digit_pins.len() > MAX_LINES {
            bail!(
                "digit_pins must list 1..={MAX_LINES} pads, got {}",
                digit_pins.len()
            );
        }
        let base = match config.get("font") {
            None => BaseFont::for_count(segment_pins.len()),
            Some(serde_yaml::Value::String(name)) => BaseFont::parse(name)?,
            Some(other) => bail!("font: expected a name, got {other:?}"),
        };
        let segment_names = match config.get("segment_names") {
            Some(v) => string_list(v, "segment_names")?,
            None => {
                let names = base.default_names().ok_or_else(|| {
                    anyhow!(
                        "{} segment pins and no 'segment_names': give one name per segment pin",
                        segment_pins.len()
                    )
                })?;
                names
                    .iter()
                    .take(segment_pins.len())
                    .map(|s| s.to_string())
                    .collect()
            }
        };
        if segment_names.len() != segment_pins.len() {
            bail!(
                "segment_names has {} names but segment_pins has {} pads",
                segment_names.len(),
                segment_pins.len()
            );
        }
        for (i, name) in segment_names.iter().enumerate() {
            if segment_names[..i].contains(name) {
                bail!("segment_names lists '{name}' twice");
            }
        }
        let dp_bit = segment_names.iter().position(|n| n == "dp");
        let mask_of = |names: &[String]| -> Option<u32> {
            let mut mask = 0u32;
            for name in names {
                let bit = segment_names.iter().position(|n| n == name)?;
                mask |= 1 << bit;
            }
            Some(mask)
        };

        let mut font: Vec<(u32, char)> = Vec::new();
        if let Some(glyphs) = config.get("glyphs") {
            let map = glyphs
                .as_mapping()
                .ok_or_else(|| anyhow!("glyphs: expected a map of character to segments"))?;
            for (key, value) in map {
                let ch = glyph_char(key)?;
                let names = string_list(value, "glyphs")?;
                if let Some(bad) = names.iter().find(|n| !segment_names.contains(n)) {
                    bail!("glyphs: '{ch}' names segment '{bad}', which is not in segment_names");
                }
                if names.iter().any(|n| n == "dp") {
                    bail!("glyphs: '{ch}' names 'dp'; the decimal point is not part of a glyph");
                }
                let mask = mask_of(&names).expect("names checked above");
                if mask == 0 {
                    bail!("glyphs: '{ch}' lights no segment; a blank digit is always ' '");
                }
                font.push((mask, ch));
            }
        }
        // Built-in glyphs after the placement's. A built-in glyph that uses a
        // segment this display does not have cannot show, so it is left out.
        for (ch, names) in base.glyphs() {
            if let Some(mask) = mask_of(&names) {
                font.push((mask, ch));
            }
        }

        if threshold_pct > 100 || min_duty_pct > 100 {
            bail!("threshold_pct and min_duty_pct must be 0..=100");
        }
        if persistence_us == 0 {
            bail!("persistence_us must be more than 0");
        }
        Ok(Self {
            segment_pins,
            digit_pins,
            segment_names,
            segment_active_high: get_bool(config, "segment_active_high", true)?,
            digit_active_high: get_bool(config, "digit_active_high", true)?,
            font,
            dp_bit,
            persistence_us,
            threshold_pct,
            min_duty_pct,
        })
    }
}

/// One resolved pad: output register address and bit.
#[derive(Debug, Clone, Copy)]
pub struct Pad {
    pub addr: u64,
    pub bit: u8,
}

/// The multiplexed segment display.
#[derive(Debug)]
pub struct DeclarativeSegmentDisplay {
    id: String,
    spec: SegmentDisplaySpec,
    segments: Vec<Pad>,
    digits: Vec<Pad>,
    /// Output registers that host a pad, for the write hook.
    edge_addrs: Vec<u64>,
    window_cycles: u64,
    /// Active segment lines and active digit lines, as last sampled.
    seg_active: u32,
    digit_active: u32,
    last_cycle: Option<u64>,
    window_start: u64,
    /// Lit time in cycles, `[digit][segment]`, in the open window.
    on_time: Vec<Vec<u64>>,
    /// Visible masks per digit, from the last closed window.
    shown: Vec<u32>,
    text: String,
    /// Distinct non-blank panels seen this run (capped); for oracle history.
    texts_seen: VecDeque<String>,
    text_log: VecDeque<String>,
    frame_log: VecDeque<String>,
}

impl DeclarativeSegmentDisplay {
    /// Build from a parsed spec whose pads are already resolved.
    pub fn new(
        id: String,
        spec: SegmentDisplaySpec,
        segments: Vec<Pad>,
        digits: Vec<Pad>,
        cpu_hz: u64,
    ) -> Result<Self> {
        if segments.len() != spec.segment_pins.len() || digits.len() != spec.digit_pins.len() {
            bail!("segment_display '{id}': resolved pads do not match the spec");
        }
        let window_cycles =
            ((spec.persistence_us as u128 * cpu_hz.max(1) as u128) / 1_000_000).max(1) as u64;
        let mut edge_addrs: Vec<u64> = segments.iter().chain(&digits).map(|p| p.addr).collect();
        edge_addrs.sort_unstable();
        edge_addrs.dedup();
        let blank = " ".repeat(digits.len());
        Ok(Self {
            id,
            on_time: vec![vec![0; segments.len()]; digits.len()],
            shown: vec![0; digits.len()],
            text: blank,
            texts_seen: VecDeque::new(),
            spec,
            segments,
            digits,
            edge_addrs,
            window_cycles,
            seg_active: 0,
            digit_active: 0,
            last_cycle: None,
            window_start: 0,
            text_log: VecDeque::new(),
            frame_log: VecDeque::new(),
        })
    }

    /// What a human sees now (from the last closed window).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Visible segment mask of each digit, bit = index in `segment_pins`.
    pub fn masks(&self) -> &[u32] {
        &self.shown
    }

    /// Length of one persistence window in cycles.
    pub fn window_cycles(&self) -> u64 {
        self.window_cycles
    }

    fn sample(&mut self, pins: &dyn DevicePins) {
        let active = |pad: &Pad, high: bool| {
            // An address that does not read back drives nothing: LED off.
            pins.output_bit(pad.addr, pad.bit)
                .is_some_and(|level| level == high)
        };
        let mut seg = 0u32;
        for (i, pad) in self.segments.iter().enumerate() {
            if active(pad, self.spec.segment_active_high) {
                seg |= 1 << i;
            }
        }
        let mut dig = 0u32;
        for (i, pad) in self.digits.iter().enumerate() {
            if active(pad, self.spec.digit_active_high) {
                dig |= 1 << i;
            }
        }
        self.seg_active = seg;
        self.digit_active = dig;
    }

    /// Add `dt` cycles to every LED that is lit in the current pad state.
    fn integrate(&mut self, dt: u64) {
        if dt == 0 || self.seg_active == 0 || self.digit_active == 0 {
            return;
        }
        for (d, row) in self.on_time.iter_mut().enumerate() {
            if self.digit_active & (1 << d) == 0 {
                continue;
            }
            for (s, t) in row.iter_mut().enumerate() {
                if self.seg_active & (1 << s) != 0 {
                    *t = t.saturating_add(dt);
                }
            }
        }
    }

    fn close_window(&mut self, now: u64) {
        let elapsed = now.saturating_sub(self.window_start).max(1);
        let brightest = self
            .on_time
            .iter()
            .flat_map(|row| row.iter().copied())
            .max()
            .unwrap_or(0);
        let floor = (elapsed as u128 * self.spec.min_duty_pct as u128) / 100;
        let masks: Vec<u32> = self
            .on_time
            .iter()
            .map(|row| {
                row.iter().enumerate().fold(0u32, |mask, (s, &t)| {
                    let t128 = t as u128;
                    let visible = t > 0
                        && t128 * 100 >= brightest as u128 * self.spec.threshold_pct as u128
                        && t128 >= floor;
                    if visible {
                        mask | (1 << s)
                    } else {
                        mask
                    }
                })
            })
            .collect();
        for row in &mut self.on_time {
            row.fill(0);
        }
        self.window_start = now;
        if masks != self.shown {
            self.shown = masks;
            let hex: Vec<String> = self
                .shown
                .iter()
                .map(|m| format!("0x{m:0width$x}", width = self.hex_width()))
                .collect();
            push_capped(
                &mut self.frame_log,
                format!("{} at cycle {now}", hex.join(" ")),
            );
            let text = self.decode();
            if text != self.text {
                push_capped(&mut self.text_log, format!("\"{text}\" at cycle {now}"));
                self.text = text.clone();
                // Blank panels are not history (same rule as LCD1602): a clear
                // does not wipe prior glyphs a prove may still need.
                if !text.chars().all(|c| c.is_whitespace()) {
                    if self.texts_seen.back().map(|s| s.as_str()) != Some(text.as_str()) {
                        if self.texts_seen.len() >= TEXTS_SEEN_CAP {
                            self.texts_seen.pop_front();
                        }
                        self.texts_seen.push_back(text);
                    }
                }
            }
        }
    }

    fn hex_width(&self) -> usize {
        self.segments.len().div_ceil(4)
    }

    /// Decode the visible masks to text through the font.
    fn decode(&self) -> String {
        let dp_mask = self.spec.dp_bit.map_or(0, |b| 1u32 << b);
        let mut out = String::new();
        for &mask in &self.shown {
            let glyph = mask & !dp_mask;
            let ch = if glyph == 0 {
                ' '
            } else {
                self.spec
                    .font
                    .iter()
                    .find(|(m, _)| *m == glyph)
                    .map_or('?', |(_, c)| *c)
            };
            out.push(ch);
            if mask & dp_mask != 0 {
                out.push('.');
            }
        }
        out
    }

    /// Integrate the pad state held since the last service up to `now`,
    /// closing every window whose end falls inside that span at its exact
    /// end cycle.
    fn advance(&mut self, now: u64) {
        let Some(mut last) = self.last_cycle else {
            self.last_cycle = Some(now);
            self.window_start = now;
            return;
        };
        if now <= last {
            return;
        }
        loop {
            let end = self.window_start.saturating_add(self.window_cycles);
            if now < end {
                self.integrate(now - last);
                break;
            }
            self.integrate(end - last);
            self.close_window(end);
            last = end;
            // The pad state is constant up to `now`, so every further whole
            // window looks the same: close one, then skip the rest.
            let whole = (now - end) / self.window_cycles;
            if whole >= 2 {
                let next = end + self.window_cycles;
                self.integrate(self.window_cycles);
                self.close_window(next);
                self.window_start = end + whole * self.window_cycles;
                last = self.window_start;
            }
        }
        self.last_cycle = Some(now);
    }
}

fn push_capped(log: &mut VecDeque<String>, line: String) {
    if log.len() == LOG_CAPACITY {
        log.pop_front();
    }
    log.push_back(line);
}

const NO_CHANNELS: &[InputChannel] = &[];

impl BusResidentDevice for DeclarativeSegmentDisplay {
    /// The tick pass: the pads cannot have moved since the last store (the
    /// write hook reads them), so only time advances.
    fn service(&mut self, _pins: &mut dyn DevicePins, now: u64) {
        self.advance(now);
    }

    fn service_edge(&mut self, pins: &mut dyn DevicePins, now: u64) {
        // Time up to `now` belongs to the levels that were on the pads before
        // this store; only then read the new levels.
        self.advance(now);
        self.sample(pins);
    }

    /// The pads move only on a GPIO store, which the write hook services. A
    /// window end with no store is a deadline (below), not a tick.
    fn needs_per_cycle_service(&self) -> bool {
        false
    }

    fn edge_service_addrs(&self) -> &[u64] {
        &self.edge_addrs
    }

    /// The end of the open persistence window.
    fn next_edge_deadline_cycle(&self, _now: u64, _interval: u64) -> Option<u64> {
        self.last_cycle
            .map(|_| self.window_start.saturating_add(self.window_cycles))
    }

    /// Close a window that ended with no GPIO store. The pads did not move,
    /// so they are not read.
    fn service_scheduled_edges(&mut self, _pins: &mut dyn DevicePins, now: u64, _interval: u64) {
        if self.last_cycle.is_some() && now.saturating_sub(self.window_start) >= self.window_cycles
        {
            self.advance(now);
        }
    }

    fn evidence(&self) -> Option<&dyn DeviceEvidence> {
        Some(self)
    }

    fn logs(&self) -> Vec<PeripheralLog> {
        vec![
            PeripheralLog::new("text", self.text_log.iter().cloned().collect()),
            PeripheralLog::new("frames", self.frame_log.iter().cloned().collect()),
        ]
    }

    fn as_sim_input(&mut self) -> &mut dyn SimInput {
        self
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl DeviceEvidence for DeclarativeSegmentDisplay {
    fn artifacts(&self, id: &str, _opts: &InspectOpts) -> Vec<Artifact> {
        let bytes: Vec<u8> = self.shown.iter().flat_map(|m| m.to_le_bytes()).collect();
        let meta = serde_json::json!({
            "format": "segment_mux",
            "generation": artifact_generation(&bytes),
            "text": self.text,
            "texts_seen": self.texts_seen.iter().cloned().collect::<Vec<_>>(),
            "printable_chars": self.text.chars().filter(|c| !c.is_whitespace()).count(),
            "digits": self.digits.len(),
            "segments": self.shown,
            "segment_names": self.spec.segment_names,
        });
        vec![Artifact {
            kind: "text_display".to_string(),
            id: id.to_string(),
            meta,
            bytes: None,
        }]
    }
}

impl SimInput for DeclarativeSegmentDisplay {
    /// None: the display is driven by its pads.
    fn input_channels(&self) -> &[InputChannel] {
        NO_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        unreachable!("a segment_display declares no channels")
    }

    fn component_id(&self) -> Option<&str> {
        Some(&self.id)
    }
}

/// Validate the static descriptor contract of the `segment_display` primitive.
pub(crate) fn validate_descriptor(desc: &labwired_config::DeviceDescriptor) -> Result<()> {
    if desc.behavior.primitive != "segment_display" {
        bail!(
            "'{}' is not a segment_display descriptor (primitive '{}')",
            desc.r#type,
            desc.behavior.primitive
        );
    }
    if !desc.behavior.pins.is_empty() || !desc.behavior.rules.is_empty() {
        bail!(
            "segment_display '{}': pins come from the placement lists \
             (segment_pins, digit_pins); the descriptor declares no pins or rules",
            desc.r#type
        );
    }
    for key in ["persistence_us", "threshold_pct", "min_duty_pct"] {
        let default = desc
            .behavior
            .params
            .get(key)
            .and_then(|v| v.get("default"))
            .and_then(|v| v.as_u64());
        if default.is_none() {
            bail!(
                "segment_display '{}': params.{key} needs an integer default",
                desc.r#type
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap};

    /// Two GPIO output registers: segments on 0x100, digits on 0x200.
    #[derive(Default)]
    struct Pads {
        regs: BTreeMap<u64, u32>,
    }

    impl DevicePins for Pads {
        fn output_bit(&self, addr: u64, bit: u8) -> Option<bool> {
            self.regs.get(&addr).map(|v| (v >> bit) & 1 != 0)
        }
        fn drive_idr_bit(&mut self, _: u64, _: u8, _: bool) {}
        fn drive_input_bit(&mut self, _: u64, _: u8, _: bool) -> bool {
            false
        }
    }

    const SEG: u64 = 0x100;
    const DIG: u64 = 0x200;
    /// 1 MHz: one cycle is one microsecond.
    const HZ: u64 = 1_000_000;

    fn config(yaml: &str) -> HashMap<String, serde_yaml::Value> {
        serde_yaml::from_str(yaml).expect("config parses")
    }

    /// A 3-digit 7-segment display with dp: segments a..g,dp on SEG bits
    /// 0..7, digits on DIG bits 0..2, all active high.
    fn display(extra: &str) -> DeclarativeSegmentDisplay {
        let yaml = format!(
            "segment_pins: [s0, s1, s2, s3, s4, s5, s6, s7]\n\
             digit_pins: [d0, d1, d2]\n{extra}"
        );
        let spec = SegmentDisplaySpec::from_config(&config(&yaml), 20_000, 50, 1).expect("spec");
        let segments = (0..8).map(|b| Pad { addr: SEG, bit: b }).collect();
        let digits = (0..3).map(|b| Pad { addr: DIG, bit: b }).collect();
        DeclarativeSegmentDisplay::new("disp".into(), spec, segments, digits, HZ).expect("new")
    }

    /// Store `seg` / `dig` to the pads at cycle `now` and service.
    fn store(d: &mut DeclarativeSegmentDisplay, pads: &mut Pads, now: u64, seg: u32, dig: u32) {
        pads.regs.insert(SEG, seg);
        pads.regs.insert(DIG, dig);
        d.service_edge(pads, now);
    }

    /// Firmware that scans `digits` (segment masks) with a 1 ms slot per
    /// digit, blanking the select for `blank_us` and holding the NEXT digit's
    /// segments with the OLD select for `ghost_us` (a ghost), until `end`.
    fn scan(
        d: &mut DeclarativeSegmentDisplay,
        pads: &mut Pads,
        digits: &[u32],
        ghost_us: u64,
        end: u64,
    ) {
        let mut now = 0;
        let mut cur = 0usize;
        store(d, pads, now, 0, 0);
        while now < end {
            let next = (cur + 1) % digits.len();
            // Segments for the next digit while the current one is still
            // selected: the ghost.
            now += 1;
            store(d, pads, now, digits[next], 1 << cur);
            now += ghost_us;
            store(d, pads, now, digits[next], 0);
            now += 1;
            store(d, pads, now, digits[next], 1 << next);
            now += 1000;
            cur = next;
        }
        d.service(pads, now);
    }

    fn logs(d: &DeclarativeSegmentDisplay, name: &str) -> Vec<String> {
        d.logs()
            .into_iter()
            .find(|l| l.name == name)
            .expect("log")
            .lines()
    }

    #[test]
    fn one_digit_lit_shows_its_character() {
        let mut d = display("");
        let mut pads = Pads::default();
        // '3' = a b c d g on digit 1 only, held for 50 ms.
        store(&mut d, &mut pads, 0, 0x4F, 0b010);
        d.service(&mut pads, 50_000);
        assert_eq!(d.text(), " 3 ");
        assert_eq!(logs(&d, "text"), vec!["\" 3 \" at cycle 20000".to_string()]);
    }

    #[test]
    fn segments_without_a_digit_select_stay_dark() {
        let mut d = display("");
        let mut pads = Pads::default();
        store(&mut d, &mut pads, 0, 0x7F, 0);
        d.service(&mut pads, 50_000);
        assert_eq!(d.text(), "   ");
        assert!(logs(&d, "text").is_empty());
    }

    #[test]
    fn multiplexing_shows_all_digits_at_once() {
        let mut d = display("");
        let mut pads = Pads::default();
        // "1.23": '1' with dp, '2', '3'.
        scan(&mut d, &mut pads, &[0x06 | 0x80, 0x5B, 0x4F], 0, 100_000);
        assert_eq!(d.text(), "1.23");
        assert!(logs(&d, "text").iter().any(|l| l.starts_with("\"1.23\"")));
    }

    #[test]
    fn texts_seen_records_distinct_panels_in_artifact() {
        let mut d = display("");
        let mut pads = Pads::default();
        // Hold '3' on the middle digit, then switch to '5'.
        store(&mut d, &mut pads, 0, 0x4F, 0b010);
        d.service(&mut pads, 50_000);
        assert_eq!(d.text(), " 3 ");
        store(&mut d, &mut pads, 50_000, 0x6D, 0b010);
        d.service(&mut pads, 100_000);
        assert_eq!(d.text(), " 5 ");
        let art = DeviceEvidence::artifacts(&d, "disp", &InspectOpts::default());
        assert_eq!(art.len(), 1);
        let meta = &art[0].meta;
        let seen = meta
            .get("texts_seen")
            .and_then(|v| v.as_array())
            .expect("texts_seen in artifact meta");
        let texts: Vec<&str> = seen.iter().filter_map(|v| v.as_str()).collect();
        assert!(
            texts.iter().any(|t| *t == " 3 "),
            "first panel must land in texts_seen: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| *t == " 5 "),
            "second panel must land in texts_seen: {texts:?}"
        );
        assert_eq!(
            meta.get("printable_chars").and_then(|v| v.as_u64()),
            Some(1),
            "printable_chars counts non-whitespace glyphs on the current panel"
        );
    }

    #[test]
    fn a_short_ghost_is_rejected_by_the_persistence_threshold() {
        let mut d = display("");
        let mut pads = Pads::default();
        // 20 us of ghost per 1 ms slot: 2 % of the slot.
        scan(&mut d, &mut pads, &[0x06, 0x5B, 0x4F], 20, 100_000);
        assert_eq!(d.text(), "123");
        // Negative control: a zero threshold makes the ghost visible and
        // the digits unreadable.
        let spec = SegmentDisplaySpec::from_config(
            &config("segment_pins: [s0, s1, s2, s3, s4, s5, s6, s7]\ndigit_pins: [d0, d1, d2]\n"),
            20_000,
            0,
            0,
        )
        .expect("spec");
        let segments = (0..8).map(|b| Pad { addr: SEG, bit: b }).collect();
        let digits = (0..3).map(|b| Pad { addr: DIG, bit: b }).collect();
        let mut raw =
            DeclarativeSegmentDisplay::new("raw".into(), spec, segments, digits, HZ).expect("new");
        let mut pads = Pads::default();
        scan(&mut raw, &mut pads, &[0x06, 0x5B, 0x4F], 20, 100_000);
        assert_ne!(raw.text(), "123");
    }

    #[test]
    fn a_display_that_stops_refreshing_goes_blank() {
        let mut d = display("");
        let mut pads = Pads::default();
        scan(&mut d, &mut pads, &[0x06, 0x5B, 0x4F], 0, 60_000);
        assert_eq!(d.text(), "123");
        // Firmware turns all selects off and stops.
        store(&mut d, &mut pads, 60_010, 0, 0);
        d.service(&mut pads, 200_000);
        assert_eq!(d.text(), "   ");
        let text = logs(&d, "text");
        assert_eq!(text.last().map(|l| l.starts_with("\"   \"")), Some(true));
    }

    #[test]
    fn active_low_lines_invert() {
        let mut d = display("segment_active_high: false\ndigit_active_high: false\n");
        let mut pads = Pads::default();
        // '7' = a b c; active low, so the other bits are high; digit 2 low.
        store(&mut d, &mut pads, 0, !0x07 & 0xFF, !0b100 & 0b111);
        d.service(&mut pads, 50_000);
        assert_eq!(d.text(), "  7");
    }

    #[test]
    fn placement_glyphs_win_and_unknown_patterns_show_as_question_mark() {
        let mut d = display("glyphs: { \"r\": [e, g] }\n");
        let mut pads = Pads::default();
        store(&mut d, &mut pads, 0, 0x50, 0b001);
        d.service(&mut pads, 50_000);
        assert_eq!(d.text(), "r  ");
        let mut d = display("");
        let mut pads = Pads::default();
        store(&mut d, &mut pads, 0, 0x09, 0b001); // a + d: no glyph
        d.service(&mut pads, 50_000);
        assert_eq!(d.text(), "?  ");
    }

    #[test]
    fn fourteen_segment_font_by_default_for_fifteen_lines() {
        let yaml = "segment_pins: [a,b,c,d,e,f,g,h,i,j,k,l,m,n,o]\ndigit_pins: [x]\n";
        let spec = SegmentDisplaySpec::from_config(&config(yaml), 20_000, 50, 1).expect("spec");
        assert_eq!(spec.segment_names, FOURTEEN_SEGMENT_NAMES);
        assert_eq!(spec.dp_bit, Some(14));
        // 'X' = h k l n.
        let x = (1 << 8) | (1 << 10) | (1 << 11) | (1 << 13);
        assert!(spec.font.contains(&(x, 'X')));
    }

    #[test]
    fn bad_config_is_refused() {
        let base = "segment_pins: [s0, s1, s2]\ndigit_pins: [d0]\n";
        let err = SegmentDisplaySpec::from_config(&config(base), 1, 50, 1).unwrap_err();
        assert!(err.to_string().contains("segment_names"), "{err}");
        let err = SegmentDisplaySpec::from_config(
            &config(&format!(
                "{base}segment_names: [a, b, c]\nglyphs: {{ X: [a, z] }}\n"
            )),
            1,
            50,
            1,
        )
        .unwrap_err();
        assert!(err.to_string().contains("'z'"), "{err}");
        let err = SegmentDisplaySpec::from_config(
            &config(&format!("{base}segment_names: [a, a, c]\n")),
            1,
            50,
            1,
        )
        .unwrap_err();
        assert!(err.to_string().contains("twice"), "{err}");
    }
}
