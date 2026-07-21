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
//! (see [`crate::peripherals::esp32s3::rmt`]), which flips the routed GPIO pad
//! through [`Esp32s3Gpio::drive_pad_output`](crate::peripherals::esp32s3::gpio::Esp32s3Gpio::drive_pad_output).
//! This component registers as an S3
//! [`GpioObserver`](crate::peripherals::esp32s3::gpio::GpioObserver) and decodes
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
            state: Mutex::new(DecodeState::default()),
        }
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
impl crate::peripherals::esp32s3::gpio::GpioObserver for Ws2812 {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_edge(pin, to, sim_cycle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// 8 MHz test clock: HIGH threshold = 4 cycles, reset threshold = 320.
    const CPU_HZ: u64 = 8_000_000;
    const PIN: u8 = 48;
    /// Short high (bit 0) and long high (bit 1), straddling the 4-cycle
    /// threshold, plus a short inter-bit low.
    const T0H: u64 = 2;
    const T1H: u64 = 6;
    const TLOW: u64 = 2;

    /// Drive one WS2812 bit onto the strip starting at `cycle`; returns the
    /// cycle after the bit's low gap.
    fn feed_bit(s: &Ws2812, bit: bool, cycle: u64) -> u64 {
        let high = if bit { T1H } else { T0H };
        s.on_edge(PIN, true, cycle); // rising
        s.on_edge(PIN, false, cycle + high); // falling after HIGH
        cycle + high + TLOW
    }

    /// Feed a 24-bit GRB pixel MSB-first starting at `cycle`; returns next cycle.
    fn feed_pixel(s: &Ws2812, grb: u32, mut cycle: u64) -> u64 {
        for i in (0..24).rev() {
            cycle = feed_bit(s, (grb >> i) & 1 != 0, cycle);
        }
        cycle
    }

    #[test]
    fn thresholds_scale_with_cpu_hz() {
        let s = Ws2812::new(PIN, 1, CPU_HZ);
        assert_eq!(s.high_threshold_cycles, 4, "0.5us @ 8MHz = 4 cycles");
        assert_eq!(s.reset_threshold_cycles, 320, "40us @ 8MHz = 320 cycles");
        // Stock S3 clock: 0.5us @160MHz = 80, 40us = 6400.
        let s160 = Ws2812::new(PIN, 1, 160_000_000);
        assert_eq!(s160.high_threshold_cycles, 80);
        assert_eq!(s160.reset_threshold_cycles, 6400);
    }

    #[test]
    fn decodes_single_green_pixel() {
        let s = Ws2812::new(PIN, 1, CPU_HZ);
        // Green = G:0xFF R:0x00 B:0x00 → 24-bit GRB 0xFF0000.
        feed_pixel(&s, 0x00FF_0000 & 0x00FF_FFFF, 0);
        assert_eq!(s.pixels(), vec![[0xFF, 0x00, 0x00]], "green pixel (GRB)");
    }

    #[test]
    fn decodes_rgb_bytes_in_grb_order() {
        let s = Ws2812::new(PIN, 1, CPU_HZ);
        // GRB = G:0x12 R:0x34 B:0x56 → 0x123456.
        feed_pixel(&s, 0x0012_3456, 0);
        assert_eq!(s.pixels(), vec![[0x12, 0x34, 0x56]]);
    }

    #[test]
    fn edges_on_other_pins_are_ignored() {
        let s = Ws2812::new(PIN, 1, CPU_HZ);
        // Noise on a different pin must not shift any bits.
        s.on_edge(7, true, 0);
        s.on_edge(7, false, 100);
        feed_pixel(&s, 0x0000_00FF, 0); // blue
        assert_eq!(s.pixels(), vec![[0x00, 0x00, 0xFF]]);
    }

    #[test]
    fn decodes_multi_pixel_frame() {
        let s = Ws2812::new(PIN, 3, CPU_HZ);
        let mut c = 0;
        c = feed_pixel(&s, 0x0000_00FF, c); // blue  → [0,0,255]
        c = feed_pixel(&s, 0x00FF_0000, c); // green → [255,0,0]
        feed_pixel(&s, 0x0000_FF00, c); // red   → [0,255,0]
        assert_eq!(
            s.pixels(),
            vec![[0x00, 0x00, 0xFF], [0xFF, 0x00, 0x00], [0x00, 0xFF, 0x00]]
        );
    }

    #[test]
    fn caps_frame_at_num_pixels() {
        let s = Ws2812::new(PIN, 1, CPU_HZ);
        let mut c = 0;
        c = feed_pixel(&s, 0x00FF_0000, c); // pixel 0 (kept)
        feed_pixel(&s, 0x0000_00FF, c); // pixel 1 (dropped — strip is 1 long)
        assert_eq!(s.pixels(), vec![[0xFF, 0x00, 0x00]], "extra pixel dropped");
    }

    #[test]
    fn reset_gap_latches_frame_and_starts_next() {
        let s = Ws2812::new(PIN, 1, CPU_HZ);
        // Frame 1: one green pixel.
        let c = feed_pixel(&s, 0x00FF_0000, 0);
        assert_eq!(s.pixels(), vec![[0xFF, 0x00, 0x00]], "in-progress frame 1");
        // A reset gap (> 320 cycles low) then frame 2's first rising edge
        // latches frame 1 and begins frame 2.
        let reset_start = c; // last falling edge cycle
        let frame2 = reset_start + s.reset_threshold_cycles + 10;
        feed_pixel(&s, 0x0000_00FF, frame2); // blue
                                             // The displayed (latched) frame is frame 1 until the NEXT reset.
        assert_eq!(
            s.pixels(),
            vec![[0xFF, 0x00, 0x00]],
            "reset latched frame 1 for display"
        );
    }

    #[test]
    fn registers_as_s3_gpio_observer() {
        use crate::peripherals::esp32s3::gpio::{Esp32s3Gpio, GpioObserver};
        let strip = Arc::new(Ws2812::new(PIN, 1, CPU_HZ));
        let mut g = Esp32s3Gpio::new();
        g.add_observer(strip.clone() as Arc<dyn GpioObserver>);
        // Sanity: the Arc bridge compiles and the observer path is wired.
        assert_eq!(strip.pin(), PIN);
    }
}
