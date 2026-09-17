// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Raydium **RM67162** — 240x536 RGB565 AMOLED, driven over 4-wire SPI.
//!
//! # Why an AMOLED is not just a TFT with nicer colours
//!
//! The modelled difference that matters is **brightness**. A TFT panel is lit
//! by a backlight the firmware drives on a separate GPIO or PWM pin, so an
//! ILI9341 model can ignore brightness entirely and still be right. An AMOLED
//! has no backlight — every pixel emits — and its brightness lives INSIDE the
//! controller, written with the DCS command `WRDISBV` (0x51). Its reset value
//! is 0x00, i.e. **black**. A driver that sends a perfect init sequence and a
//! full frame but never writes 0x51 lights nothing on real hardware, and a
//! model that ignored 0x51 would show a bright picture for firmware that
//! cannot work. So brightness is modelled, reported in the artifact, and
//! folded into `display_on`.
//!
//! # Interface
//!
//! The RM67162 supports QSPI and DBI Type-C 3/4-wire SPI. This models the
//! 4-wire mode: a D/C line frames command versus data, exactly as on silicon.
//!
//! D/C can arrive two ways, and BOTH are real hardware:
//!
//! * a plain GPIO the firmware toggles between transfers (`dc_pin`), which is
//!   how every nRF52-era and STM32 board wires a panel; or
//! * the **controller's own DCX line** (`hw_dcx`), which is what the nRF54L
//!   SPIM drives from `PSEL.DCX` + `DCXCNT` — the controller holds D/C low for
//!   the first DCXCNT bytes of a transfer and high for the rest, with no
//!   firmware pin write anywhere.
//!
//! One of the two is REQUIRED. This model deliberately has no
//! infer-framing-from-byte-values fallback: `ili9341.rs` documents in detail
//! how that fails on a real init sequence — Adafruit's stock table sends a
//! command whose second parameter is 0x2C, the inference decodes it as RAMWR,
//! and the rest of the init sequence is written into the framebuffer as
//! pixels. Refusing to attach is better than rendering that.

use labwired_core::peripherals::spi::SpiDevice;
use std::any::Any;

/// Physical frame memory. The RM67162 die addresses 240 x 536.
const WIDTH: usize = 240;
const HEIGHT: usize = 536;
const FB_BYTES: usize = WIDTH * HEIGHT * 2; // RGB565, 2 bytes per pixel

/// MADCTL bits (DCS 0x36). MV swaps the addressing axes.
const MADCTL_MY: u8 = 0x80;
const MADCTL_MX: u8 = 0x40;
const MADCTL_MV: u8 = 0x20;

// ── DCS commands this model acts on ────────────────────────────────────────
const CMD_SWRESET: u8 = 0x01;
const CMD_SLPIN: u8 = 0x10;
const CMD_SLPOUT: u8 = 0x11;
const CMD_DISPOFF: u8 = 0x28;
const CMD_DISPON: u8 = 0x29;
const CMD_CASET: u8 = 0x2A;
const CMD_RASET: u8 = 0x2B;
const CMD_RAMWR: u8 = 0x2C;
const CMD_MADCTL: u8 = 0x36;
const CMD_COLMOD: u8 = 0x3A;
const CMD_WRDISBV: u8 = 0x51;

/// `COLMOD` value for 16-bit RGB565 (DCS 0x3A = 0x55). The panel also supports
/// 0x75 (18-bit) and 0x77 (24-bit); this model implements 16-bit only and says
/// so in the artifact rather than silently mis-packing a wider stream.
const COLMOD_RGB565: u8 = 0x55;

#[derive(Debug, Clone, Copy, PartialEq)]
enum ProtoState {
    /// Between commands. A data byte here is a stray parameter and is dropped.
    Idle,
    /// Gathering a command's parameters.
    Params { cmd: u8, params: [u8; 4], have: u8 },
    /// Streaming pixels after RAMWR; waiting for a pixel's high byte.
    PixelHi,
    /// Have the high byte; waiting for the low byte.
    PixelLo { hi: u8 },
}

/// Where this panel's D/C level comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DcSourceKind {
    /// A GPIO the firmware toggles; the bus latches its output register.
    Gpio,
    /// The SPI controller's own DCX line (nRF54L `PSEL.DCX` + `DCXCNT`).
    Controller,
}

/// Simulated Raydium RM67162 240x536 RGB565 AMOLED.
#[derive(Debug, serde::Serialize)]
pub struct Rm67162 {
    cs_pin: String,
    /// Whether the module's supply pins are actually connected in the design.
    ///
    /// ⚠️ DEFAULTS TO `true`. Only an explicit `powered: false` in the compiled
    /// manifest darkens the panel; an absent key means powered, because every
    /// curated lab states signals and leaves the rails implicit. See
    /// `peripherals::components::supply` for the measurement behind
    /// that asymmetry, and `st7789.rs` for the reference implementation.
    powered: bool,
    /// DISPON/DISPOFF state. NOT the same as "visible" — see `is_lit`.
    display_on: bool,
    /// Sleep state (SLPIN/SLPOUT). Resets to asleep, as the panel does.
    asleep: bool,
    /// WRDISBV brightness, 0..=255. Resets to 0 = black.
    brightness: u8,
    /// COLMOD pixel format. Resets to RGB565.
    colmod: u8,
    madctl: u8,

    cur_col: u16,
    cur_row: u16,
    col_start: u16,
    col_end: u16,
    row_start: u16,
    row_end: u16,

    /// RGB565 framebuffer, row-major, big-endian per pixel (wire order).
    #[serde(skip_serializing)]
    framebuffer: Vec<u8>,
    #[serde(skip_serializing)]
    state: ProtoState,

    /// D/C GPIO label, when a pin wires it.
    dc_pin: Option<String>,
    /// Latched D/C level: false = command, true = data.
    dc_level: bool,
    /// GPIO output register + bit the bus samples for D/C.
    dc_source: Option<(u64, u8)>,
    /// Which of the two real wirings this panel uses.
    dc_kind: DcSourceKind,
}

impl Rm67162 {
    /// Panel driven by a GPIO D/C line.
    pub fn with_gpio_dc(cs_pin: impl Into<String>, dc_pin: impl Into<String>) -> Self {
        Self {
            dc_pin: Some(dc_pin.into()),
            dc_kind: DcSourceKind::Gpio,
            ..Self::bare(cs_pin)
        }
    }

    /// Panel whose D/C is driven by the SPI controller's own DCX line.
    pub fn with_controller_dc(cs_pin: impl Into<String>) -> Self {
        Self {
            dc_kind: DcSourceKind::Controller,
            ..Self::bare(cs_pin)
        }
    }

    fn bare(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            // Absent supply information means "powered" — see the field's note.
            powered: true,
            display_on: false,
            // The panel powers up ASLEEP; SLPOUT is not optional on silicon.
            asleep: true,
            // 0x00 is the real reset value and it means black.
            brightness: 0x00,
            colmod: COLMOD_RGB565,
            madctl: 0,
            cur_col: 0,
            cur_row: 0,
            col_start: 0,
            col_end: (WIDTH as u16) - 1,
            row_start: 0,
            row_end: (HEIGHT as u16) - 1,
            framebuffer: vec![0u8; FB_BYTES],
            state: ProtoState::Idle,
            dc_pin: None,
            dc_level: false,
            dc_source: None,
            dc_kind: DcSourceKind::Controller,
        }
    }

    /// Everything that has to be true for this panel to emit light.
    ///
    /// This is the assertion an AMOLED lab should make instead of `display_on`:
    /// a panel that is on, awake and at zero brightness is black on real
    /// hardware, and calling that "on" is how a model flatters broken firmware.
    pub fn is_lit(&self) -> bool {
        self.powered && self.display_on && !self.asleep && self.brightness > 0
    }

    /// Declare whether the module's supply is connected. See the `powered`
    /// field. Only ever called with `false`, from `attach`, when the compiled
    /// manifest explicitly says the supply pins are on no net.
    pub fn with_powered(mut self, powered: bool) -> Self {
        self.powered = powered;
        self
    }

    /// True when the module has a supply. See the `powered` field.
    pub fn powered(&self) -> bool {
        self.powered
    }

    /// DISPON **and** a supply. `transfer` already refuses the bus when
    /// unpowered, so the inner flag can never be true here; the `&&` is the
    /// guard that survives someone later adding another way to set it.
    pub fn display_on(&self) -> bool {
        self.powered && self.display_on
    }
    pub fn brightness(&self) -> u8 {
        self.brightness
    }
    pub fn asleep(&self) -> bool {
        self.asleep
    }
    pub fn dimensions(&self) -> (usize, usize) {
        (WIDTH, HEIGHT)
    }
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    fn addressable_width(&self) -> u16 {
        if self.madctl & MADCTL_MV != 0 {
            HEIGHT as u16
        } else {
            WIDTH as u16
        }
    }
    fn addressable_height(&self) -> u16 {
        if self.madctl & MADCTL_MV != 0 {
            WIDTH as u16
        } else {
            HEIGHT as u16
        }
    }

    pub fn logical_dimensions(&self) -> (usize, usize) {
        (
            self.addressable_width() as usize,
            self.addressable_height() as usize,
        )
    }

    fn to_physical(&self, col: u16, row: u16) -> (usize, usize) {
        let (mut x, mut y) = if self.madctl & MADCTL_MV != 0 {
            (row, col)
        } else {
            (col, row)
        };
        if self.madctl & MADCTL_MX != 0 {
            x = (WIDTH as u16).saturating_sub(1).saturating_sub(x);
        }
        if self.madctl & MADCTL_MY != 0 {
            y = (HEIGHT as u16).saturating_sub(1).saturating_sub(y);
        }
        (x as usize, y as usize)
    }

    /// The framebuffer as the panel scans it out, MADCTL applied, so a host can
    /// paint it row-major and get the picture the firmware intended.
    pub fn oriented_framebuffer(&self) -> Vec<u8> {
        let lw = self.addressable_width() as usize;
        let lh = self.addressable_height() as usize;
        let mut out = vec![0u8; lw * lh * 2];
        for row in 0..lh {
            for col in 0..lw {
                let (x, y) = self.to_physical(col as u16, row as u16);
                let src = (y * WIDTH + x) * 2;
                let dst = (row * lw + col) * 2;
                if src + 1 < self.framebuffer.len() {
                    out[dst] = self.framebuffer[src];
                    out[dst + 1] = self.framebuffer[src + 1];
                }
            }
        }
        out
    }

    /// Parameter bytes a command gathers before it is applied.
    ///
    /// With D/C framing this is a semantic threshold only: it decides WHEN a
    /// command is complete, never where one command ends and the next begins.
    /// An unlisted command is therefore harmless — its parameters are consumed
    /// as data and ignored — which is the whole reason the D/C line is required.
    fn param_count(cmd: u8) -> u8 {
        match cmd {
            CMD_CASET | CMD_RASET => 4,
            CMD_MADCTL | CMD_COLMOD | CMD_WRDISBV => 1,
            _ => 0,
        }
    }

    fn on_command(&mut self, cmd: u8) {
        match cmd {
            CMD_SWRESET => {
                self.framebuffer.iter_mut().for_each(|b| *b = 0);
                self.display_on = false;
                self.asleep = true;
                self.brightness = 0;
                self.madctl = 0;
                self.colmod = COLMOD_RGB565;
                self.col_start = 0;
                self.col_end = (WIDTH as u16) - 1;
                self.row_start = 0;
                self.row_end = (HEIGHT as u16) - 1;
                self.cur_col = 0;
                self.cur_row = 0;
                self.state = ProtoState::Idle;
                return;
            }
            CMD_SLPOUT => self.asleep = false,
            CMD_SLPIN => self.asleep = true,
            CMD_DISPON => self.display_on = true,
            CMD_DISPOFF => self.display_on = false,
            CMD_RAMWR => {
                self.cur_col = self.col_start;
                self.cur_row = self.row_start;
                self.state = ProtoState::PixelHi;
                return;
            }
            _ => {}
        }
        let want = Self::param_count(cmd);
        self.state = if want > 0 {
            ProtoState::Params {
                cmd,
                params: [0; 4],
                have: 0,
            }
        } else {
            ProtoState::Idle
        };
    }

    fn on_params_complete(&mut self, cmd: u8, p: &[u8; 4]) {
        let a = u16::from_be_bytes([p[0], p[1]]);
        let b = u16::from_be_bytes([p[2], p[3]]);
        match cmd {
            CMD_CASET => {
                // Clamp to the addressable frame. A driver written for a panel
                // with a column offset can ask for a window past the die's
                // edge; silicon clips, and so does this.
                let max = self.addressable_width().saturating_sub(1);
                self.col_start = a.min(max);
                self.col_end = b.min(max).max(self.col_start);
            }
            CMD_RASET => {
                let max = self.addressable_height().saturating_sub(1);
                self.row_start = a.min(max);
                self.row_end = b.min(max).max(self.row_start);
            }
            CMD_MADCTL => self.madctl = p[0],
            CMD_COLMOD => self.colmod = p[0],
            CMD_WRDISBV => self.brightness = p[0],
            _ => {}
        }
    }

    fn on_data(&mut self, byte: u8) {
        match self.state {
            ProtoState::Idle => {
                // A data byte with no command open. Real silicon ignores it.
            }
            ProtoState::Params {
                cmd,
                mut params,
                mut have,
            } => {
                if (have as usize) < params.len() {
                    params[have as usize] = byte;
                }
                have += 1;
                if have >= Self::param_count(cmd) {
                    self.on_params_complete(cmd, &params);
                    self.state = ProtoState::Idle;
                } else {
                    self.state = ProtoState::Params { cmd, params, have };
                }
            }
            ProtoState::PixelHi => self.state = ProtoState::PixelLo { hi: byte },
            ProtoState::PixelLo { hi } => {
                self.write_pixel(hi, byte);
                self.state = ProtoState::PixelHi;
            }
        }
    }

    fn write_pixel(&mut self, hi: u8, lo: u8) {
        let (x, y) = self.to_physical(self.cur_col, self.cur_row);
        let idx = (y * WIDTH + x) * 2;
        if x < WIDTH && idx + 1 < self.framebuffer.len() {
            self.framebuffer[idx] = hi;
            self.framebuffer[idx + 1] = lo;
        }
        if self.cur_col >= self.col_end {
            self.cur_col = self.col_start;
            if self.cur_row >= self.row_end {
                self.cur_row = self.row_start;
            } else {
                self.cur_row += 1;
            }
        } else {
            self.cur_col += 1;
        }
    }
}

impl SpiDevice for Rm67162 {
    fn artifacts(
        &self,
        id: &str,
        opts: &labwired_core::inspect::InspectOpts,
    ) -> Vec<labwired_core::inspect::Artifact> {
        let fb = self.oriented_framebuffer();
        let painted = fb.iter().filter(|&&b| b != 0x00).count();
        let (w, h) = self.logical_dimensions();
        let mut counts: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
        for px in fb.chunks_exact(2) {
            let v = u16::from_be_bytes([px[0], px[1]]);
            if v != 0 {
                *counts.entry(v).or_default() += 1;
            }
        }
        let top = counts.iter().max_by_key(|&(_, n)| *n);
        vec![labwired_core::inspect::Artifact {
            kind: "framebuffer".to_string(),
            id: id.to_string(),
            meta: serde_json::json!({
                "w": w,
                "h": h,
                "format": labwired_core::inspect::artifact_format::RGB565_BE,
                "generation": labwired_core::inspect::artifact_generation(&fb),
                // `display_on` is DISPON alone, kept for parity with the other
                // panels. `lit` is the honest answer for an emissive display:
                // DISPON, awake, AND non-zero brightness.
                "display_on": self.display_on(),
                "lit": self.is_lit(),
                // Reported so a dark frame explains itself. Without this, a
                // panel darkened for having no supply is indistinguishable
                // from one whose firmware left brightness at 0.
                "powered": self.powered,
                "asleep": self.asleep,
                "brightness": self.brightness,
                "colmod": format!("0x{:02X}", self.colmod),
                "madctl": format!("0x{:02X}", self.madctl),
                "dc_source": match self.dc_kind {
                    DcSourceKind::Gpio => "gpio",
                    DcSourceKind::Controller => "controller_dcx",
                },
                "painted_bytes": painted,
                "total_bytes": fb.len(),
                "top_colour": top.map(|(v, _)| format!("0x{v:04X}")),
                "top_colour_pixels": top.map(|(_, n)| *n),
            }),
            bytes: labwired_core::inspect::artifact_bytes(&fb, opts),
        }]
    }

    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        // Each CS burst starts with a command byte. Pixel streams that hold CS
        // low across the whole burst are unaffected — state is only reset on
        // the falling edge, not the rising one.
        self.state = ProtoState::Idle;
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

    fn transfer(&mut self, mosi: u8) -> u8 {
        // THE ONE GATE THAT MAKES AN UNPOWERED PANEL BEHAVE LIKE ONE. Every
        // state change this model has — SLPOUT, DISPON, WRDISBV brightness,
        // CASET/RASET and every pixel byte — arrives through `transfer`.
        // Refusing the bus here is what an unpowered RM67162 does, and it
        // leaves `lit` / `brightness` / `painted_bytes` at their power-on-dark
        // values by construction rather than masking them at report time.
        if !self.powered {
            return 0;
        }
        // Framing is always the wire's. There is no value-inference path: see
        // the module header for what that costs on a real init sequence.
        if self.dc_level {
            self.on_data(mosi);
        } else {
            self.on_command(mosi);
        }
        0
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
