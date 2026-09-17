// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WS2812 / WS2812B / SK6812 "NeoPixel" addressable-LED strip — a digital twin
//! that decodes the single-wire bit-stream a GPIO pad carries into pixel colors.
//!
//! ## Line coding
//!
//! A NeoPixel strip is driven by ONE data wire carrying a self-clocked NRZ
//! stream: every bit is a HIGH pulse immediately followed by a LOW pulse, and a
//! `0` vs `1` is distinguished purely by how long the HIGH portion lasts —
//! `0` is a short high (~0.35 µs), `1` is a long high (~0.7 µs); the following
//! low pads each bit to ~1.25 µs. So the decoder needs only the HIGH duration of
//! each bit, compared against a threshold half-way between the two (~0.5 µs).
//!
//! ```text
//!  bit 0:  ▁▁███▁▁▁▁▁▁▁   T0H≈0.35µs  T0L≈0.8µs
//!  bit 1:  ▁▁██████▁▁▁▁▁   T1H≈0.7µs   T1L≈0.6µs
//! ```
//!
//! Each pixel is 24 bits sent **MSB first in GRB order** (green byte, then red,
//! then blue). A **reset / latch** gap — the line held low for ≥ ~50 µs (a safe
//! 40 µs threshold here) — ends a frame and displays it.
//!
//! ## How it is driven
//!
//! On the ESP32-S3 the pad is driven by the RMT peripheral's timed playback
//! (see `peripherals::esp32s3::rmt`), which flips the routed GPIO pad
//! through `Esp32s3Gpio::drive_pad_output`.
//! This component registers as an S3
//! [`GpioObserver`](labwired_core::peripherals::device::GpioObserver) and decodes
//! purely from the `(pin, from, to, sim_cycle)` callbacks — accumulating each
//! bit's HIGH duration, shifting it into a 24-bit register, and pushing a pixel
//! every 24 bits.
//!
//! The µs thresholds are converted to sim cycles with `cpu_hz` (the clock the
//! firmware was built against — 160 MHz on a stock S3), so the decode tracks the
//! same time base the RMT edges were scheduled on.
//!
//! Interior mutability: the observer hook is `&self`, so decode state lives
//! behind a `Mutex`. Hold the strip as `Arc<Ws2812>` — register a clone as a
//! GPIO observer and keep a clone to read [`Ws2812::pixels`] back.

use std::sync::Mutex;

/// A decoded pixel in wire order: `[green, red, blue]` (the byte order WS2812
/// sends). Convert to RGB at the readback boundary if needed.
pub type Grb = [u8; 3];

/// Bits per pixel (24 = 3 bytes, GRB).
const BITS_PER_PIXEL: u32 = 24;
/// HIGH-duration threshold, in nanoseconds, separating a `0` (~0.35 µs) from a
/// `1` (~0.7 µs): the mid-point ~0.5 µs.
const HIGH_THRESHOLD_NS: u64 = 500;
/// LOW-gap threshold, in nanoseconds, that ends a frame (reset/latch). The
/// datasheet minimum is ~50 µs; 40 µs is a safe detector that no inter-bit low
/// (≤ ~0.8 µs) can trip.
const RESET_THRESHOLD_NS: u64 = 40_000;

/// Convert a nanosecond span to sim cycles at `cpu_hz` (`ns * cpu_hz / 1e9`),
/// never rounding down to zero (a threshold of 0 cycles would classify every
/// pulse the same way).
fn ns_to_cycles(ns: u64, cpu_hz: u64) -> u64 {
    ((ns as u128 * cpu_hz as u128) / 1_000_000_000).max(1) as u64
}

#[derive(Debug, Default)]
struct DecodeState {
    /// Current pad level (true = high).
    level: bool,
    /// sim_cycle of the most recent rising edge (start of a bit's HIGH).
    last_rise: Option<u64>,
    /// sim_cycle of the most recent falling edge (start of the LOW gap).
    last_fall: Option<u64>,
    /// 24-bit shift register for the pixel currently being received (MSB first).
    shift: u32,
    /// Bits accumulated into `shift` (0..24).
    nbits: u32,
    /// Pixels of the frame currently being received.
    current: Vec<Grb>,
    /// The last frame closed by a reset/latch gap (empty until the first reset).
    latched: Vec<Grb>,
}

/// A WS2812 / NeoPixel strip digital twin. See module docs.
#[derive(Debug)]
pub struct Ws2812 {
    /// GPIO pin the data wire is connected to; edges on other pins are ignored.
    pin: u8,
    /// Strip length — the decoder keeps at most this many pixels per frame.
    num_pixels: usize,
    /// HIGH-duration `0`/`1` threshold, in sim cycles (derived from `cpu_hz`).
    high_threshold_cycles: u64,
    /// LOW-gap reset/latch threshold, in sim cycles (derived from `cpu_hz`).
    reset_threshold_cycles: u64,
    /// The `external_devices:` id this strip was declared as, stamped at
    /// attach. Identity, not behaviour: nothing in the decoder reads it.
    component_id: Option<String>,
    state: Mutex<DecodeState>,
}

impl Ws2812 {
    /// Create a strip of `num_pixels` LEDs listening on GPIO `pin`, with the
    /// bit/reset timing thresholds scaled from `cpu_hz` (the firmware clock).
    pub fn new(pin: u8, num_pixels: usize, cpu_hz: u64) -> Self {
        let cpu_hz = cpu_hz.max(1);
        Self {
            pin,
            num_pixels: num_pixels.max(1),
            high_threshold_cycles: ns_to_cycles(HIGH_THRESHOLD_NS, cpu_hz),
            reset_threshold_cycles: ns_to_cycles(RESET_THRESHOLD_NS, cpu_hz),
            component_id: None,
            state: Mutex::new(DecodeState::default()),
        }
    }

    /// Stamp the manifest id this strip was declared as, so `inspect` can name
    /// it as the author did rather than reporting anonymous hardware.
    pub fn with_component_id(mut self, id: impl Into<String>) -> Self {
        self.component_id = Some(id.into());
        self
    }

    /// The manifest id this strip was declared as, when it was declared.
    pub fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    /// The GPIO pin this strip's data wire is on.
    pub fn pin(&self) -> u8 {
        self.pin
    }

    /// Configured strip length.
    pub fn num_pixels(&self) -> usize {
        self.num_pixels
    }

    /// The strip's displayed pixels, in wire (GRB) order. Returns the last
    /// reset-latched frame once one has completed, otherwise the frame currently
    /// being received (so a single frame with no trailing reset is still
    /// readable).
    pub fn pixels(&self) -> Vec<Grb> {
        let s = self.state.lock().unwrap();
        if s.latched.is_empty() {
            s.current.clone()
        } else {
            s.latched.clone()
        }
    }

    /// Feed one pad transition (the GPIO-observer hook). Decodes bit HIGH
    /// durations into pixels and detects the reset/latch gap. No-op for edges on
    /// other pins or non-transitions.
    fn on_edge(&self, pin: u8, to: bool, sim_cycle: u64) {
        if pin != self.pin {
            return;
        }
        let mut s = self.state.lock().unwrap();
        if to == s.level {
            return; // not a transition on this pin
        }
        s.level = to;
        if to {
            // Rising edge — start of a bit's HIGH. A long preceding LOW is the
            // reset/latch gap: display the frame just received and start fresh.
            if let Some(fall) = s.last_fall {
                if sim_cycle.saturating_sub(fall) >= self.reset_threshold_cycles {
                    if !s.current.is_empty() {
                        s.latched = std::mem::take(&mut s.current);
                    }
                    s.current.clear();
                    s.shift = 0;
                    s.nbits = 0;
                }
            }
            s.last_rise = Some(sim_cycle);
        } else {
            // Falling edge — the HIGH duration decides this bit.
            if let Some(rise) = s.last_rise {
                let high = sim_cycle.saturating_sub(rise);
                let bit = (high > self.high_threshold_cycles) as u32;
                s.shift = (s.shift << 1) | bit;
                s.nbits += 1;
                if s.nbits == BITS_PER_PIXEL {
                    let g = ((s.shift >> 16) & 0xFF) as u8;
                    let r = ((s.shift >> 8) & 0xFF) as u8;
                    let b = (s.shift & 0xFF) as u8;
                    if s.current.len() < self.num_pixels {
                        s.current.push([g, r, b]);
                    }
                    s.shift = 0;
                    s.nbits = 0;
                }
            }
            s.last_fall = Some(sim_cycle);
        }
    }
}

// Bridge into the ESP32-S3 GPIO observer protocol (the chip whose RMT drives the
// pad today). `from` is unused — only the new level and the sim cycle matter.
impl labwired_core::peripherals::device::GpioObserver for Ws2812 {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_edge(pin, to, sim_cycle);
    }
}

/// An addressable LED strip is a display surface wired to ONE pin, so like the
/// TM1637 it binds to the bus rather than to a controller and reports through
/// the evidence seam directly.
///
/// The pixels are whatever the decoder reconstructed from real edge timing on
/// the data pad; a strip that never saw an edge reports zero lit pixels, not a
/// plausible pattern. Bytes are the decoded frame in wire (GRB) order.
impl labwired_core::inspect::DeviceEvidence for Ws2812 {
    fn artifacts(
        &self,
        id: &str,
        opts: &labwired_core::inspect::InspectOpts,
    ) -> Vec<labwired_core::inspect::Artifact> {
        let pixels = self.pixels();
        let flat: Vec<u8> = pixels.iter().flatten().copied().collect();
        vec![labwired_core::inspect::Artifact {
            kind: "framebuffer".to_string(),
            id: id.to_string(),
            meta: serde_json::json!({
                "w": self.num_pixels(),
                "h": 1,
                "format": labwired_core::inspect::artifact_format::WS2812_GRB,
                "generation": labwired_core::inspect::artifact_generation(&flat),
                "pixels_decoded": pixels.len(),
                "lit_pixels": pixels.iter().filter(|p| p.iter().any(|&c| c != 0)).count(),
                "data_pin": self.pin(),
            }),
            bytes: labwired_core::inspect::artifact_bytes(&flat, opts),
        }]
    }
}
