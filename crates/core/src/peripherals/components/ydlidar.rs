// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Simulated 360° scanning lidar speaking the YDLIDAR frame protocol.
//!
//! # Where the numbers come from
//!
//! Every wire constant below was **measured off a physical unit** on
//! 2026-09-01 (22.2 s capture, 306 098 bytes, 3 794 frames, 0 rejected by
//! checksum), not taken from a datasheet paraphrase:
//!
//! | property | measured |
//! |---|---|
//! | link | 230400 8N1, free-running — the host writes nothing |
//! | header | `0xAA 0x55`, LSN=25 data frames, LSN=1 zero packet |
//! | rate | 171.2 frames/s, 4 031 samples/s, 10.06 rev/s |
//! | line | 13 806 B/s — 60% of what 230400 8N1 carries; the wire idles between frames |
//! | `CT` | `0` = data; `bit0` = start-of-revolution, `CT >> 1` = spin Hz × 10 |
//! | `FSA`/`LSA` | `bit0` is a check bit and is **always 1**; angle = `raw >> 1` / 64 |
//! | distance | `raw / 4` mm — quarter-millimetre, `raw & 3` genuinely varies |
//! | intensity | 6-bit, stored `<< 2`; the low two bits were 0 in all 89 331 samples |
//!
//! The encoder in this module was validated by re-encoding all 3 794 captured
//! frames from their decoded fields: **3 794/3 794 byte-identical**. That check
//! is the gate in `tests/ydlidar_silicon_parity.rs`.
//!
//! # Two things this model deliberately does not do
//!
//! **No inverse-square intensity.** The obvious model — intensity falls with
//! range — is contradicted by the capture: median 6-bit intensity is flat
//! between 25 and 40 from 0.25 m to 6.25 m. Return strength tracks surface
//! reflectivity, not distance, so [`DEFAULT_INTENSITY6`] is the measured
//! population median (31 of 63, n = 81 066) and is configurable per scene
//! rather than being computed from an invented law.
//!
//! **No replay of the capture at runtime.** Replay is a test fixture, so the
//! silicon bytes gate the encoder without riding into the wasm bundle.

use crate::peripherals::device::UartStreamDevice;
use std::any::Any;
use std::collections::VecDeque;

/// Frame header, low byte first on the wire.
pub const HEADER: [u8; 2] = [0xAA, 0x55];
/// Samples carried by a normal data frame.
pub const SAMPLES_PER_FRAME: usize = 25;
/// Nominal revolutions per second. The unit reported `CT >> 1 == 100` (10.0 Hz)
/// on 219 of 223 start packets and 99 (9.9 Hz) on the other 4.
pub const DEFAULT_SPIN_HZ: f64 = 10.0;
/// Ranging samples per second, measured. This — not the baud rate — sets the
/// frame rate: 4 031 / 25 = 161.2 data frames/s against 161.1 measured, and
/// with the 10 Hz default it gives 403 samples per revolution against 400.6.
///
/// It also predicts the line rate: 3 bytes per sample plus 10 bytes of frame
/// overhead plus the zero packets is 13 836 B/s, and the unit measured
/// 13 806 B/s.
pub const DEFAULT_SAMPLE_RATE_HZ: f64 = 4031.0;
/// Population median 6-bit return intensity (31 of 63, n = 81 066).
pub const DEFAULT_INTENSITY6: u8 = 31;
/// Line rate. 20 022 B/s measured, which is 230400 / 10 bits + framing losses.
pub const DEFAULT_BAUD: u32 = 230_400;
/// Bus tick granularity the UART stream path polls on.
const TICK_US: f64 = 1000.0;

// ── wire codec ──────────────────────────────────────────────────────────────

/// Encode a mechanical angle into the `FSA`/`LSA` field.
///
/// The field is `angle_deg * 64` shifted left one, with the low bit — a check
/// bit — set. It was 1 in every one of the 7 588 captured angle fields; a
/// decoder that only does `raw >> 1` never notices, but an encoder that leaves
/// it clear produces bytes the real device never emits.
pub fn angle_to_raw(angle_deg: f64) -> u16 {
    let wrapped = angle_deg.rem_euclid(360.0);
    (((wrapped * 64.0).round() as u16) << 1) | 1
}

/// Inverse of [`angle_to_raw`], discarding the check bit.
pub fn raw_to_angle(raw: u16) -> f64 {
    (raw >> 1) as f64 / 64.0
}

/// Millimetres to the on-wire quarter-millimetre distance word.
pub fn mm_to_raw(mm: f64) -> u16 {
    (mm * 4.0).round().clamp(0.0, u16::MAX as f64) as u16
}

/// The device's XOR-16 frame checksum, over the header, `CT|LSN<<8`, `FSA`,
/// every sample (distance word then intensity byte), and finally `LSA`.
/// The `CS` field itself is not part of the sum.
pub fn checksum(ct: u8, lsn: u8, fsa: u16, lsa: u16, samples: &[(u8, u16)]) -> u16 {
    let mut x = 0x55AAu16 ^ (ct as u16 | ((lsn as u16) << 8)) ^ fsa;
    for &(intensity, distance) in samples {
        x ^= distance;
        x ^= intensity as u16;
    }
    x ^ lsa
}

/// Serialise one frame exactly as the device puts it on the wire.
///
/// `samples` are `(intensity_byte, distance_word)` pairs already in wire form.
/// Note the layout quirk this reproduces: `CS` sits **before** the sample
/// block even though it is computed over it.
pub fn encode_frame(ct: u8, fsa: u16, lsa: u16, samples: &[(u8, u16)]) -> Vec<u8> {
    let lsn = samples.len() as u8;
    let mut out = Vec::with_capacity(10 + 3 * samples.len());
    out.extend_from_slice(&HEADER);
    out.push(ct);
    out.push(lsn);
    out.extend_from_slice(&fsa.to_le_bytes());
    out.extend_from_slice(&lsa.to_le_bytes());
    out.extend_from_slice(&checksum(ct, lsn, fsa, lsa, samples).to_le_bytes());
    for &(intensity, distance) in samples {
        out.push(intensity);
        out.extend_from_slice(&distance.to_le_bytes());
    }
    out
}

/// The published YDLIDAR angle correction, in degrees.
///
/// The emitter sits off the rotation axis, so the mechanical angle a frame
/// reports is not the bearing the beam actually travelled. The correction is
/// `atan(21.8 · (155.3 − d) / (155.3 · d))`: zero at d = 155.3 mm and
/// asymptotic to −7.99° far away. It is not a small term and the scene
/// generator has to invert it (see [`YdLidar::range_for_mechanical_angle`]).
pub fn angle_correction_deg(distance_mm: f64) -> f64 {
    if distance_mm <= 0.0 {
        return 0.0;
    }
    (21.8 * (155.3 - distance_mm) / (155.3 * distance_mm))
        .atan()
        .to_degrees()
}

// ── scene ───────────────────────────────────────────────────────────────────

/// A rectangular room, centred on the origin, that beams are cast against.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Room {
    /// Interior width along X, in millimetres.
    pub width_mm: f64,
    /// Interior depth along Y, in millimetres.
    pub depth_mm: f64,
    /// Scanner position along X relative to the room centre.
    pub x_mm: f64,
    /// Scanner position along Y relative to the room centre.
    pub y_mm: f64,
}

impl Default for Room {
    fn default() -> Self {
        // 4 m x 3 m, scanner centred: the same order of magnitude as the
        // captured room, whose returns spanned 0.18 m to 7.61 m.
        Self {
            width_mm: 4000.0,
            depth_mm: 3000.0,
            x_mm: 0.0,
            y_mm: 0.0,
        }
    }
}

impl Room {
    /// Distance from the scanner to the wall along `bearing_deg`, where 0° is
    /// +Y and the sweep runs clockwise (the device's own convention).
    ///
    /// Slab method against the four walls: for each candidate plane take the
    /// ray parameter, reject it if it is behind the scanner or if the hit
    /// falls outside that wall's extent, and keep the nearest survivor.
    fn range_mm(&self, bearing_deg: f64) -> f64 {
        let (hw, hd) = (self.width_mm / 2.0, self.depth_mm / 2.0);
        let theta = bearing_deg.to_radians();
        let (dx, dy) = (theta.sin(), theta.cos());
        let mut best = f64::INFINITY;
        // Vertical walls at x = ±hw.
        if dx.abs() > 1e-12 {
            for wall_x in [-hw, hw] {
                let t = (wall_x - self.x_mm) / dx;
                if t > 0.0 && (self.y_mm + t * dy).abs() <= hd + 1e-6 && t < best {
                    best = t;
                }
            }
        }
        // Horizontal walls at y = ±hd.
        if dy.abs() > 1e-12 {
            for wall_y in [-hd, hd] {
                let t = (wall_y - self.y_mm) / dy;
                if t > 0.0 && (self.x_mm + t * dx).abs() <= hw + 1e-6 && t < best {
                    best = t;
                }
            }
        }
        if best.is_finite() {
            best
        } else {
            // Scanner outside the room, or degenerate dimensions: report the
            // no-return marker rather than an invented range.
            0.0
        }
    }
}

/// A movable obstacle in front of the scanner: a flat arc of constant range,
/// which is what a hand or a chair leg looks like at this angular resolution.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Target {
    /// Bearing of the arc centre, degrees.
    pub bearing_deg: f64,
    /// Range of the arc, millimetres. Zero disables the target.
    pub range_mm: f64,
    /// Total angular width of the arc, degrees.
    pub width_deg: f64,
}

impl Default for Target {
    fn default() -> Self {
        Self {
            bearing_deg: 0.0,
            range_mm: 0.0,
            width_deg: 12.0,
        }
    }
}

// ── device ──────────────────────────────────────────────────────────────────

/// Simulated scanning lidar attached to a UART's RX path.
#[derive(Debug, serde::Serialize)]
pub struct YdLidar {
    room: Room,
    target: Target,
    spin_hz: f64,
    sample_rate_hz: f64,
    intensity6: u8,
    baud: u32,
    /// Mechanical angle of the next sample, degrees.
    angle_deg: f64,
    /// Set once the head has swept past 360°, so the next sample is spent on
    /// the LSN=1 zero packet that marks a new revolution.
    revolution_pending: bool,
    /// Samples ranged but not yet packed into a frame, as
    /// `(mechanical_angle_deg, distance_word)`.
    #[serde(skip)]
    pending: Vec<(f64, u16)>,
    /// Ranging time earned but not yet spent, in microseconds.
    sample_credit_us: f64,
    #[serde(skip)]
    out_queue: VecDeque<u8>,
    /// Wire time earned but not yet spent, in microseconds.
    byte_credit_us: f64,
    /// `external_devices` id, stamped at attach.
    component_id: Option<String>,
}

impl Default for YdLidar {
    fn default() -> Self {
        Self::new()
    }
}

impl YdLidar {
    /// A scanner in the default 4 m × 3 m room, spinning at 10 Hz.
    pub fn new() -> Self {
        Self {
            room: Room::default(),
            target: Target::default(),
            spin_hz: DEFAULT_SPIN_HZ,
            sample_rate_hz: DEFAULT_SAMPLE_RATE_HZ,
            intensity6: DEFAULT_INTENSITY6,
            baud: DEFAULT_BAUD,
            angle_deg: 0.0,
            revolution_pending: true,
            pending: Vec::with_capacity(SAMPLES_PER_FRAME),
            sample_credit_us: 0.0,
            out_queue: VecDeque::new(),
            byte_credit_us: 0.0,
            component_id: None,
        }
    }

    /// Replace the room geometry.
    pub fn with_room(mut self, room: Room) -> Self {
        self.room = room;
        self
    }

    /// Set spin rate in Hz. Also changes the angular step, since the sample
    /// rate is fixed by the ranging hardware, not by how fast the head turns.
    pub fn with_spin_hz(mut self, hz: f64) -> Self {
        if hz > 0.0 {
            self.spin_hz = hz;
        }
        self
    }

    /// Set the ranging sample rate in Hz.
    pub fn with_sample_rate_hz(mut self, hz: f64) -> Self {
        if hz > 0.0 {
            self.sample_rate_hz = hz;
        }
        self
    }

    /// Set the reported 6-bit return intensity (0–63).
    pub fn with_intensity6(mut self, intensity6: u8) -> Self {
        self.intensity6 = intensity6.min(63);
        self
    }

    /// Set the link rate, which paces bytes onto the wire.
    pub fn with_baud(mut self, baud: u32) -> Self {
        if baud > 0 {
            self.baud = baud;
        }
        self
    }

    /// Degrees between consecutive samples: the head turns `spin_hz`
    /// revolutions per second while ranging `sample_rate_hz` times per second.
    pub fn angular_step_deg(&self) -> f64 {
        360.0 * self.spin_hz / self.sample_rate_hz
    }

    /// Microseconds one 8N1 character occupies on the wire (10 bits).
    fn us_per_byte(&self) -> f64 {
        10.0 * 1_000_000.0 / self.baud as f64
    }

    /// The `CT` byte for a start-of-revolution packet: spin rate in tenths of
    /// a hertz in the high seven bits, start flag in bit 0.
    fn start_ct(&self) -> u8 {
        let tenths = (self.spin_hz * 10.0).round().clamp(0.0, 127.0) as u8;
        (tenths << 1) | 1
    }

    /// Range reported for a given **mechanical** angle.
    ///
    /// The scene is defined in true bearings, but a frame carries mechanical
    /// angles, and firmware recovers the bearing by adding
    /// [`angle_correction_deg`] — which depends on the range being measured.
    /// That is a fixed point, so solve it: seed with the uncorrected angle and
    /// iterate. Two passes are enough because the room's range function is
    /// smooth and the correction saturates near −8°.
    ///
    /// Skipping this would leave every decoded bearing rotated by up to 8°
    /// from the room the author declared — wrong in a way that still looks
    /// like a plausible scan.
    fn range_for_mechanical_angle(&self, mechanical_deg: f64) -> f64 {
        let mut bearing = mechanical_deg;
        let mut range = self.scene_range(bearing);
        for _ in 0..2 {
            bearing = mechanical_deg + angle_correction_deg(range);
            range = self.scene_range(bearing);
        }
        range
    }

    /// Range at a true bearing: the target arc if the beam falls inside it,
    /// otherwise the room wall.
    fn scene_range(&self, bearing_deg: f64) -> f64 {
        if self.target.range_mm > 0.0 && self.target.width_deg > 0.0 {
            let mut delta = (bearing_deg - self.target.bearing_deg).rem_euclid(360.0);
            if delta > 180.0 {
                delta = 360.0 - delta;
            }
            if delta <= self.target.width_deg / 2.0 {
                return self.target.range_mm;
            }
        }
        self.room.range_mm(bearing_deg)
    }

    /// The intensity byte as it appears on the wire: 6 significant bits, `<< 2`.
    fn intensity_byte(&self) -> u8 {
        self.intensity6 << 2
    }

    /// Microseconds between ranging samples.
    fn us_per_sample(&self) -> f64 {
        1_000_000.0 / self.sample_rate_hz
    }

    /// Advance the head by `elapsed_us` of simulated time, ranging as many
    /// samples as that time buys and packing completed frames into the queue.
    ///
    /// The head is driven by TIME, not by frame emission. Tying the angle to
    /// the byte stream instead makes the scan rate a function of the baud rate:
    /// the device would spin at 16.8 Hz on a 230400 link no matter what
    /// `spin_hz` said, because the wire can carry more frames than the head
    /// produces. The real unit occupies only 60% of its line and idles between
    /// frames.
    fn advance_scene(&mut self, elapsed_us: f64) {
        self.sample_credit_us += elapsed_us;
        let per_sample = self.us_per_sample();
        if per_sample <= 0.0 {
            return;
        }
        // Bound the catch-up so a large time jump cannot spin here unboundedly.
        let mut budget = 4 * SAMPLES_PER_FRAME * 64;
        while self.sample_credit_us >= per_sample && budget > 0 {
            self.sample_credit_us -= per_sample;
            budget -= 1;
            self.take_sample();
        }
    }

    /// Range one beam at the current head angle and advance the head.
    fn take_sample(&mut self) {
        let step = self.angular_step_deg();
        if self.revolution_pending {
            self.revolution_pending = false;
            // A revolution boundary cuts the frame in progress short. That is
            // why the captured LSN population is not all 25: 145 frames of 24
            // and 11 of 23 across 223 revolutions. Dropping the partial would
            // also silently drop those samples from the scan.
            self.flush_pending();
            let angle = self.angle_deg;
            let range = self.range_for_mechanical_angle(angle);
            let raw = angle_to_raw(angle);
            let samples = [(self.intensity_byte(), mm_to_raw(range))];
            let frame = encode_frame(self.start_ct(), raw, raw, &samples);
            self.out_queue.extend(frame);
            self.advance_angle(step);
            return;
        }
        let angle = self.angle_deg;
        let range = self.range_for_mechanical_angle(angle);
        self.pending.push((angle, mm_to_raw(range)));
        self.advance_angle(step);
        if self.pending.len() >= SAMPLES_PER_FRAME {
            self.flush_pending();
        }
    }

    /// Pack whatever has been ranged into one data frame. No-op when empty.
    fn flush_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let intensity = self.intensity_byte();
        let first = self.pending[0].0;
        let last = self.pending[self.pending.len() - 1].0;
        let samples: Vec<(u8, u16)> = self.pending.iter().map(|&(_, d)| (intensity, d)).collect();
        let frame = encode_frame(0, angle_to_raw(first), angle_to_raw(last), &samples);
        self.out_queue.extend(frame);
        self.pending.clear();
    }

    /// Turn the head, flagging a revolution boundary on wrap.
    fn advance_angle(&mut self, delta_deg: f64) {
        let next = self.angle_deg + delta_deg;
        if next >= 360.0 {
            self.revolution_pending = true;
        }
        self.angle_deg = next.rem_euclid(360.0);
    }
}

impl UartStreamDevice for YdLidar {
    fn poll(&mut self, elapsed_us: u32) -> Option<u8> {
        // Turn the head first: ranging is what produces frames, and it runs on
        // its own clock rather than on however fast the wire happens to be.
        self.advance_scene(elapsed_us as f64);

        // Bytes then leave at the wire rate. The first poll of a tick carries
        // the elapsed time; the rest carry 0 and only spend credit already
        // earned, so the stream cannot run fast.
        self.byte_credit_us += elapsed_us as f64;
        let cost = self.us_per_byte();
        if self.out_queue.is_empty() {
            // Wire idle. Hold at most one byte of credit: banking it would let
            // the next frame burst out at once and collapse the idle gap that
            // makes this a 13.8 kB/s stream on a 23 kB/s line.
            self.byte_credit_us = self.byte_credit_us.min(cost);
            return None;
        }
        if self.byte_credit_us < cost {
            return None;
        }
        self.byte_credit_us -= cost;
        self.out_queue.pop_front()
    }

    /// A scan frame is raw octets, not console text. Without this the scanner's
    /// UART would splice into the console capture and no serial assertion in a
    /// lab that also prints would be trustworthy.
    fn carries_protocol_octets(&self) -> bool {
        true
    }

    /// 230400 baud is ~23 bytes per millisecond; the default budget of 1 would
    /// deliver 4% of the stream and no frame would ever complete. One byte of
    /// headroom absorbs the fractional credit carried across ticks.
    fn max_bytes_per_tick(&self) -> usize {
        (TICK_US / self.us_per_byte()).ceil() as usize + 1
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        Some(self)
    }
}

/// Runtime-drivable scene state. One table backs both the [`crate::sim_input::SimInput`]
/// impl and the kit metadata, so the device schema and the runtime API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "target_bearing",
        label: "Target bearing",
        unit: "°",
        min: 0.0,
        max: 360.0,
    },
    crate::sim_input::InputChannel {
        key: "target_range",
        label: "Target range",
        unit: "mm",
        min: 0.0,
        max: 12000.0,
    },
    crate::sim_input::InputChannel {
        key: "target_width",
        label: "Target width",
        unit: "°",
        min: 0.0,
        max: 180.0,
    },
    crate::sim_input::InputChannel {
        key: "spin_hz",
        label: "Spin rate",
        unit: "Hz",
        min: 1.0,
        max: 20.0,
    },
    crate::sim_input::InputChannel {
        key: "room_width",
        label: "Room width",
        unit: "mm",
        min: 200.0,
        max: 20000.0,
    },
    crate::sim_input::InputChannel {
        key: "room_depth",
        label: "Room depth",
        unit: "mm",
        min: 200.0,
        max: 20000.0,
    },
];

impl crate::sim_input::SimInput for YdLidar {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "target_bearing" => self.target.bearing_deg = value,
            "target_range" => self.target.range_mm = value,
            "target_width" => self.target.width_deg = value,
            "spin_hz" => self.spin_hz = value,
            "room_width" => self.room.width_mm = value,
            "room_depth" => self.room.depth_mm = value,
            _ => unreachable!("require_channel validated the key"),
        }
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

// ── kit registration ────────────────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct YdLidarKit;
pub static YDLIDAR_KIT: YdLidarKit = YdLidarKit;

static YDLIDAR_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "ydlidar-scanner",
    label: "360° Scanning Lidar",
    summary: "Spinning lidar streaming YDLIDAR scan frames over UART RX at 230400 baud.",
    detail: "Emits 0xAA55 frames — 25 samples of quarter-millimetre range plus 6-bit \
             intensity, with a one-sample zero packet marking each revolution. Ranges come \
             from raycasting a declared rectangular room plus a drivable target arc, and the \
             generator inverts the atan angle correction so decoded bearings match the room \
             as authored. Wire format is gated byte-for-byte against a capture from physical \
             silicon.",
    transport: Transport::Uart,
    category: Category::Uart,
    config_keys: &[
        ConfigKey {
            name: "room_width_mm",
            ty: ConfigType::Float,
            doc: "Interior room width along X in millimetres (default 4000).",
        },
        ConfigKey {
            name: "room_depth_mm",
            ty: ConfigType::Float,
            doc: "Interior room depth along Y in millimetres (default 3000).",
        },
        ConfigKey {
            name: "scanner_x_mm",
            ty: ConfigType::Float,
            doc: "Scanner offset from the room centre along X (default 0).",
        },
        ConfigKey {
            name: "scanner_y_mm",
            ty: ConfigType::Float,
            doc: "Scanner offset from the room centre along Y (default 0).",
        },
        ConfigKey {
            name: "spin_hz",
            ty: ConfigType::Float,
            doc: "Revolutions per second (default 10.0; the measured unit reported 10.0).",
        },
        ConfigKey {
            name: "sample_rate_hz",
            ty: ConfigType::Float,
            doc: "Ranging samples per second (default 4000; measured 4031).",
        },
        ConfigKey {
            name: "intensity",
            ty: ConfigType::Float,
            doc: "Reported 6-bit return intensity 0-63 (default 31, the measured median).",
        },
        ConfigKey {
            name: "baud",
            ty: ConfigType::Float,
            doc: "Link rate used to pace bytes onto the wire (default 230400).",
        },
    ],
    labs: &[LabRef {
        board_id: "ydlidar-scan-lab",
        chip: "stm32f103",
        example_dir: "ydlidar-scan-lab",
        demo_elf: "demo-ydlidar-scan-lab.elf",
    }],
};

impl PeripheralKit for YdLidarKit {
    fn metadata(&self) -> &'static KitMetadata {
        &YDLIDAR_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let mut room = Room::default();
        if let Some(v) = ctx.config_f64("room_width_mm") {
            room.width_mm = v;
        }
        if let Some(v) = ctx.config_f64("room_depth_mm") {
            room.depth_mm = v;
        }
        if let Some(v) = ctx.config_f64("scanner_x_mm") {
            room.x_mm = v;
        }
        if let Some(v) = ctx.config_f64("scanner_y_mm") {
            room.y_mm = v;
        }

        let mut lidar = YdLidar::new().with_room(room);
        if let Some(v) = ctx.config_f64("spin_hz") {
            lidar = lidar.with_spin_hz(v);
        }
        if let Some(v) = ctx.config_f64("sample_rate_hz") {
            lidar = lidar.with_sample_rate_hz(v);
        }
        if let Some(v) = ctx.config_f64("intensity") {
            lidar = lidar.with_intensity6(v.clamp(0.0, 63.0) as u8);
        }
        if let Some(v) = ctx.config_f64("baud") {
            lidar = lidar.with_baud(v.max(1.0) as u32);
        }

        crate::sim_input::SimInput::set_component_id(&mut lidar, ctx.device_id().to_string());
        let uart = ctx.uart()?;
        uart.attach_stream(Box::new(lidar));
        Ok(())
    }
}
