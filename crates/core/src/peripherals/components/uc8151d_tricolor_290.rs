// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! UC8151D tri-color 2.9" e-paper panel model.
//!
//! Waveshare GDEW029Z13c / GDEW029Z13 / GDEM029C90 variants ship with a
//! UC8151D-family controller (not SSD1680) — same physical 128×296
//! tri-color glass, completely different SPI command set. The
//! `GxEPD2_290_Z13c` / `GxEPD2_290_C90c` Arduino driver emits
//! UC8151D-style bytes: PSR (0x00), PWR (0x01), PON (0x04), DTM1 (0x10
//! write-RAM-black), DTM2 (0x13 write-RAM-red), DRF (0x12 refresh),
//! TRES (0x61 resolution), CDI (0x50 VCOM/data interval), etc.
//!
//! These conflict with SSD1680 at multiple opcodes (0x10, 0x20, 0x22)
//! so the two protocols can't share a single panel model. Use this one
//! when the firmware is GxEPD2_290_Z13c / C90c (labwired-ereader); use
//! [`super::ssd1680_tricolor_290::Ssd1680Tricolor290`] for SSD1680-class
//! firmware (the reference firmware).
//!
//! ## Cmd / Data routing
//!
//! Real silicon multiplexes cmd vs data via a sideband D/C GPIO pin.
//! This model reads that pin for real: set a `dc_source` (the resolved
//! GPIO output register + bit, via [`crate::peripherals::spi::SpiDevice::set_dc_source`])
//! and the SPI bus latches the DC level from the GPIO output register
//! before each `transfer()`, so DC=low bytes route to `command_byte(u8)`
//! and DC=high bytes to `data_byte(u8)`. The firmware's own
//! `digitalWrite(DC, …)` drives that GPIO — no thunk, no caller-PC
//! inference. This is what GxEPD2 does on hardware, and what the real
//! compiled firmware exercises in tests/e2e_labwired_ereader.rs.
//!
//! When no `dc_source` is wired, `transfer()` falls back to "all bytes
//! are data" so the model still slots into a generic SPI bus without
//! breaking, just without full UC8151D protocol decoding.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;

const WIDTH: usize = 128;
const HEIGHT: usize = 296;
const WIDTH_BYTES: usize = WIDTH / 8;
const PLANE_BYTES: usize = WIDTH_BYTES * HEIGHT;

/// Param-collection state for fixed-length commands. UC8151D's RAM-stream
/// commands (DTM1, DTM2) use a separate `Streaming*` state because their
/// length is determined by the next command boundary, not a fixed count.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
enum ProtoState {
    Idle,
    AwaitingParams { cmd: u8, have: u8, want: u8 },
    StreamingBlack,
    StreamingRed,
}

#[derive(Debug, serde::Serialize)]
pub struct Uc8151dTricolor290 {
    cs_pin: String,

    hibernating: bool,
    power_on: bool,
    /// Set by DRF (0x12). UI uses [`Self::refresh_generation`] to
    /// invalidate its rendered cache after a refresh.
    refresh_pending: bool,

    /// Plane cursors. DTM1 and DTM2 fill row-major MSB-first, wrapping
    /// at HEIGHT (matches real-silicon counters from cold reset).
    cur_black_byte: usize,
    cur_red_byte: usize,

    /// Black plane: 1 = white (no ink), 0 = black. 4736 bytes.
    #[serde(skip_serializing)]
    black_plane: Vec<u8>,
    /// Red plane: 1 = no-red, 0 = red. UC8151D firmware (GxEPD2) sends
    /// the red plane already inverted vs. source bitmap.
    #[serde(skip_serializing)]
    red_plane: Vec<u8>,

    refresh_generation: u32,

    #[serde(skip_serializing)]
    state: ProtoState,

    /// Data/Command (D/C) GPIO label, if wired (e.g. "GPIO17"). When set, the
    /// bus latches that pin's output level via [`SpiDevice::set_dc_level`]
    /// before each transfer, so command/data framing comes from the real GPIO
    /// exactly like silicon — no library thunk, no calling-identity guess.
    #[serde(skip)]
    dc_pin: Option<String>,
    /// Latched D/C level (low = command, high = data), pushed by the bus.
    #[serde(skip)]
    dc_level: bool,
    /// Resolved `(GPIO output reg address, bit)` for the D/C line.
    #[serde(skip)]
    dc_source: Option<(u64, u8)>,
}

impl Default for Uc8151dTricolor290 {
    fn default() -> Self {
        Self::new("GPIO5")
    }
}

impl Uc8151dTricolor290 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            hibernating: false,
            power_on: false,
            refresh_pending: false,
            cur_black_byte: 0,
            cur_red_byte: 0,
            // Fresh panel: all white.
            black_plane: vec![0xFF; PLANE_BYTES],
            red_plane: vec![0xFF; PLANE_BYTES],
            refresh_generation: 0,
            state: ProtoState::Idle,
            dc_pin: None,
            dc_level: false,
            dc_source: None,
        }
    }

    /// Wire a Data/Command GPIO line (e.g. "GPIO17"). With a D/C pin the panel
    /// frames command vs data from the real GPIO level (silicon-accurate);
    /// without one `transfer()` cannot distinguish the two.
    pub fn with_dc_pin(mut self, dc_pin: impl Into<String>) -> Self {
        self.dc_pin = Some(dc_pin.into());
        self
    }

    pub fn dimensions(&self) -> (usize, usize) {
        (WIDTH, HEIGHT)
    }

    pub fn black_plane(&self) -> &[u8] {
        &self.black_plane
    }

    pub fn red_plane(&self) -> &[u8] {
        &self.red_plane
    }

    pub fn refresh_generation(&self) -> u32 {
        self.refresh_generation
    }

    pub fn power_on(&self) -> bool {
        self.power_on
    }

    /// Process one command byte (DC=low on real hardware). Resets plane
    /// streams, kicks the appropriate state transition or one-shot side
    /// effect (PON → power on, DRF → refresh increment).
    pub fn command_byte(&mut self, cmd: u8) {
        match cmd {
            0x00 => self.await_params(cmd, 1), // PSR — panel setting
            0x01 => self.await_params(cmd, 5), // PWR — power setting
            0x02 => {
                // POF — power off
                self.power_on = false;
                self.state = ProtoState::Idle;
            }
            0x03 => self.await_params(cmd, 1), // PFS — power off seq
            0x04 => {
                // PON — power on
                self.power_on = true;
                self.state = ProtoState::Idle;
            }
            0x05 => self.state = ProtoState::Idle, // PMES — power-on measure (no data)
            0x06 => self.await_params(cmd, 3),     // BTST — booster soft start
            0x07 => self.await_params(cmd, 1),     // DSLP — deep sleep
            0x10 => {
                // DTM1 — display start transmission 1 (black plane)
                self.cur_black_byte = 0;
                self.state = ProtoState::StreamingBlack;
            }
            0x11 => self.state = ProtoState::Idle, // DSP — data stop
            0x12 => {
                // DRF — display refresh
                self.refresh_pending = true;
                self.refresh_generation = self.refresh_generation.wrapping_add(1);
                self.state = ProtoState::Idle;
            }
            0x13 => {
                // DTM2 — display start transmission 2 (red plane)
                self.cur_red_byte = 0;
                self.state = ProtoState::StreamingRed;
            }
            0x16 | 0x17 => self.await_params(cmd, 1), // AUTO
            0x20 => self.await_params(cmd, 44),       // LUT_VCOM
            0x21..=0x24 => self.await_params(cmd, 42), // LUT_WW/BW/WB/BB
            0x30 => self.await_params(cmd, 1),        // PLL
            0x40 => self.state = ProtoState::Idle, // TSC — temperature sensor calibration (0 data)
            0x41 => self.await_params(cmd, 1),     // TSE — temperature sensor enable
            0x50 => self.await_params(cmd, 1),     // CDI — vcom and data interval
            0x51 => self.await_params(cmd, 1),     // LPD — low-power detect
            0x60 => self.await_params(cmd, 1),     // TCON — gate/source non-overlap
            0x61 => self.await_params(cmd, 3),     // TRES — resolution (HRES, VRES_MSB, VRES_LSB)
            0x65 => self.await_params(cmd, 4),     // GSST — gate/source start setting
            0x70 | 0x71 => self.state = ProtoState::Idle, // REV / FLG read (0 data on the in path)
            0x80..=0x82 => self.await_params(cmd, 1), // AMV / VV / VDCS
            0x90 => self.await_params(cmd, 7),     // PartialWindow
            0x91 | 0x92 => self.state = ProtoState::Idle, // PartialIn / PartialOut
            _ => {
                // Unknown command — return to Idle. Don't mis-consume the
                // next byte by leaving us in a streaming state.
                self.state = ProtoState::Idle;
            }
        }
    }

    /// Process one data byte (DC=high on real hardware). Routes to the
    /// active stream (DTM1 / DTM2) or consumes as a fixed-count param.
    pub fn data_byte(&mut self, byte: u8) {
        match self.state {
            ProtoState::StreamingBlack => {
                if self.cur_black_byte < PLANE_BYTES {
                    self.black_plane[self.cur_black_byte] = byte;
                    self.cur_black_byte += 1;
                }
            }
            ProtoState::StreamingRed => {
                if self.cur_red_byte < PLANE_BYTES {
                    self.red_plane[self.cur_red_byte] = byte;
                    self.cur_red_byte += 1;
                }
            }
            ProtoState::AwaitingParams { cmd, have, want } => {
                let new_have = have + 1;
                if new_have >= want {
                    self.handle_params_complete(cmd);
                    self.state = ProtoState::Idle;
                } else {
                    self.state = ProtoState::AwaitingParams {
                        cmd,
                        have: new_have,
                        want,
                    };
                }
                // Note: we don't currently *store* param bytes — the
                // commands we care about (PON / DRF / DTM1 / DTM2) are
                // edge-triggered, not param-driven. If a future command
                // requires its params (e.g. PartialWindow x/y to clip
                // RAM writes), grow `ProtoState::AwaitingParams` to
                // hold a small buffer like the SSD1680 model.
                let _ = byte;
            }
            ProtoState::Idle => {
                // Data byte arrived without a preceding command. On real
                // silicon this just gets clocked into the controller's
                // input register and ignored; mirror that.
            }
        }
    }

    fn await_params(&mut self, cmd: u8, want: u8) {
        if want == 0 {
            self.state = ProtoState::Idle;
        } else {
            self.state = ProtoState::AwaitingParams { cmd, have: 0, want };
        }
    }

    fn handle_params_complete(&mut self, _cmd: u8) {
        // Currently no param-driven side effects beyond the edge actions
        // already handled in `command_byte`. Reserved for future
        // PartialWindow / TRES clipping etc.
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Uc8151dSnap {
    cs_pin: String,
    hibernating: bool,
    power_on: bool,
    refresh_pending: bool,
    cur_black_byte: usize,
    cur_red_byte: usize,
    black_plane: Vec<u8>,
    red_plane: Vec<u8>,
    refresh_generation: u32,
    state: ProtoState,
}

impl SpiDevice for Uc8151dTricolor290 {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        let snap = Uc8151dSnap {
            cs_pin: self.cs_pin.clone(),
            hibernating: self.hibernating,
            power_on: self.power_on,
            refresh_pending: self.refresh_pending,
            cur_black_byte: self.cur_black_byte,
            cur_red_byte: self.cur_red_byte,
            black_plane: self.black_plane.clone(),
            red_plane: self.red_plane.clone(),
            refresh_generation: self.refresh_generation,
            state: self.state,
        };
        bincode::serialize(&snap).expect("bincode serialize Uc8151dSnap")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> crate::SimResult<()> {
        let snap: Uc8151dSnap = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("Uc8151d snapshot decode: {e}"))
        })?;
        self.cs_pin = snap.cs_pin;
        self.hibernating = snap.hibernating;
        self.power_on = snap.power_on;
        self.refresh_pending = snap.refresh_pending;
        self.cur_black_byte = snap.cur_black_byte;
        self.cur_red_byte = snap.cur_red_byte;
        if snap.black_plane.len() != self.black_plane.len()
            || snap.red_plane.len() != self.red_plane.len()
        {
            return Err(crate::SimulationError::NotImplemented(format!(
                "Uc8151d snapshot plane size mismatch: black {} vs {}, red {} vs {}",
                snap.black_plane.len(),
                self.black_plane.len(),
                snap.red_plane.len(),
                self.red_plane.len()
            )));
        }
        self.black_plane = snap.black_plane;
        self.red_plane = snap.red_plane;
        self.refresh_generation = snap.refresh_generation;
        self.state = snap.state;
        Ok(())
    }

    fn cs_select(&mut self) {
        // Each CS-low burst resets the protocol parser. Real silicon's
        // UC8151D doesn't strictly require this, but it's a useful guard
        // against malformed firmware that interrupts a stream mid-byte.
        self.state = ProtoState::Idle;
    }

    fn cs_release(&mut self) {
        // Preserve state — a DTM1/DTM2 stream may span the entire CS-low
        // window and we don't want CS-high to discard the framebuffer.
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        // Silicon-accurate framing: when a D/C line is wired the bus has
        // latched the real GPIO level (low = command, high = data) before
        // this transfer, so we route correctly with no thunk. Without a D/C
        // pin we genuinely can't tell cmd from data over the raw byte stream,
        // so we treat it as data (legacy fallback) and keep the bus happy.
        if self.dc_source.is_some() {
            if self.dc_level {
                self.data_byte(mosi);
            } else {
                self.command_byte(mosi);
            }
        } else {
            // CHEAT(INFER): no D/C line wired — can't tell command from data, so
            // treat every byte as data — real: sample the D/C GPIO. FIDELITY.md §E.
            self.data_byte(mosi);
        }
        0
    }

    fn dc_pin(&self) -> Option<&str> {
        self.dc_pin.as_deref()
    }

    fn set_dc_level(&mut self, level: bool) {
        self.dc_level = level;
    }

    fn dc_source(&self) -> Option<(u64, u8)> {
        self.dc_source
    }

    fn set_dc_source(&mut self, odr_addr: u64, bit: u8) {
        self.dc_source = Some((odr_addr, bit));
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Replay the bytes captured from the labwired-ereader sketch's
    /// `display.init() + drawPage()` and assert the panel reaches
    /// `power_on=true` and `refresh_generation > 0`.
    #[test]
    fn ereader_init_powers_panel_on() {
        let mut p = Uc8151dTricolor290::new("GPIO5");
        // PSR(0x8F) — panel setting
        p.command_byte(0x00);
        p.data_byte(0x8F);
        // TRES(0x80=128, 0x01_28=296)
        p.command_byte(0x61);
        p.data_byte(0x80);
        p.data_byte(0x01);
        p.data_byte(0x28);
        // CDI(0x77)
        p.command_byte(0x50);
        p.data_byte(0x77);
        // PON
        p.command_byte(0x04);
        assert!(p.power_on(), "PON should set power_on=true");
        // DRF
        p.command_byte(0x12);
        assert_eq!(
            p.refresh_generation(),
            1,
            "DRF should increment refresh_generation"
        );
    }

    #[test]
    fn dtm1_fills_black_plane() {
        let mut p = Uc8151dTricolor290::new("GPIO5");
        p.command_byte(0x10); // DTM1 — start black RAM write
                              // Send a recognizable pattern.
        for i in 0..256u32 {
            p.data_byte((i & 0xFF) as u8);
        }
        assert_eq!(p.black_plane()[0], 0x00);
        assert_eq!(p.black_plane()[255], 0xFF);
        // Stream should still be live; rest of plane untouched.
        assert_eq!(
            p.black_plane()[256],
            0xFF,
            "untouched bytes stay at 0xFF reset"
        );
    }

    #[test]
    fn next_command_ends_dtm_stream() {
        let mut p = Uc8151dTricolor290::new("GPIO5");
        p.command_byte(0x10);
        p.data_byte(0xAA);
        p.command_byte(0x13); // DTM2 — should end DTM1 stream
        p.data_byte(0x55);
        assert_eq!(p.black_plane()[0], 0xAA);
        assert_eq!(p.red_plane()[0], 0x55);
    }

    #[test]
    fn unknown_command_returns_to_idle() {
        let mut p = Uc8151dTricolor290::new("GPIO5");
        p.command_byte(0xFE); // unrecognized
        p.command_byte(0x04); // PON should still work after
        assert!(p.power_on());
    }
}
