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

use crate::peripherals::spi::SpiDevice;
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
        self.display_on && !self.asleep && self.brightness > 0
    }

    pub fn display_on(&self) -> bool {
        self.display_on
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
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
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
        vec![crate::inspect::Artifact {
            kind: "framebuffer".to_string(),
            id: id.to_string(),
            meta: serde_json::json!({
                "w": w,
                "h": h,
                "format": crate::inspect::artifact_format::RGB565_BE,
                "generation": crate::inspect::artifact_generation(&fb),
                // `display_on` is DISPON alone, kept for parity with the other
                // panels. `lit` is the honest answer for an emissive display:
                // DISPON, awake, AND non-zero brightness.
                "display_on": self.display_on,
                "lit": self.is_lit(),
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
            bytes: crate::inspect::artifact_bytes(&fb, opts),
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

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Rm67162Kit;
pub static RM67162_KIT: Rm67162Kit = Rm67162Kit;

static RM67162_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "amoled-rm67162",
    label: "RM67162 AMOLED",
    summary: "240x536 RGB565 AMOLED, 4-wire SPI (DBI Type-C).",
    detail: "Raydium RM67162 emissive panel. Implements the DCS command / RAMWR stream against \
             an in-memory framebuffer. Unlike a backlit TFT, brightness lives in the controller \
             (WRDISBV 0x51, reset 0x00 = black), so the artifact reports `lit` -- DISPON AND \
             awake AND non-zero brightness -- alongside the raw pixels. D/C framing comes either \
             from a GPIO or from the SPI controller's own DCX line.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[
        ConfigKey {
            name: "cs_pin",
            ty: ConfigType::Str,
            doc: "Chip-select pin label. With a controller-driven hardware CSN \
                  (nRF54L PSEL.CSN) this is documentation only -- the controller \
                  asserts chip select itself.",
        },
        ConfigKey {
            name: "dc_pin",
            ty: ConfigType::Str,
            doc: "Data/command GPIO pin. Use this when the firmware toggles D/C \
                  itself between transfers, which is how nRF52-era and STM32 \
                  boards wire a panel. Mutually exclusive with `hw_dcx`.",
        },
        ConfigKey {
            name: "hw_dcx",
            ty: ConfigType::Bool,
            doc: "Set when the SPI controller drives D/C itself (nRF54L SPIM \
                  PSEL.DCX + DCXCNT). No GPIO is involved. Mutually exclusive \
                  with `dc_pin`. Exactly one of the two is required: this model \
                  has no infer-framing-from-byte-values fallback, because that \
                  inference desynchronises on a real init sequence.",
        },
    ],
    labs: &[],
};

impl PeripheralKit for Rm67162Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &RM67162_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs_pin = ctx.config_str("cs_pin").unwrap_or("").to_string();
        let dc_pin = ctx.config_str("dc_pin").map(|s| s.to_string());
        let hw_dcx = ctx.config_bool("hw_dcx").unwrap_or(false);

        match (dc_pin, hw_dcx) {
            (Some(_), true) => anyhow::bail!(
                "amoled-rm67162 '{}': `dc_pin` and `hw_dcx` are mutually exclusive. \
                 Either the firmware drives D/C on a GPIO or the controller drives it \
                 from PSEL.DCX -- on real hardware only one line is connected.",
                ctx.device_id(),
            ),
            (None, false) => anyhow::bail!(
                "amoled-rm67162 '{}': no D/C source. Set `dc_pin` for a firmware-driven \
                 GPIO, or `hw_dcx: true` when the SPI controller drives D/C itself \
                 (nRF54L SPIM PSEL.DCX). This panel has no infer-from-byte-values \
                 fallback: that inference decodes a parameter byte 0x2C as RAMWR and \
                 writes the rest of the init sequence into the framebuffer as pixels.",
                ctx.device_id(),
            ),
            (None, true) => {
                ctx.attach_spi_device(Box::new(Rm67162::with_controller_dc(cs_pin)))?;
            }
            (Some(dc), false) => {
                // Resolving the pin to its GPIO output register is the half that
                // makes D/C real: the bus samples that register before each
                // transfer. Declaring the pin without this leaves D/C stuck low,
                // every byte frames as a command, and the panel renders blank
                // with no error anywhere.
                let (odr_addr, bit) = ctx.resolve_pin_odr(&dc).ok_or_else(|| {
                    anyhow::anyhow!(
                        "amoled-rm67162 '{}': D/C pin '{}' does not resolve to a driveable \
                         GPIO output.",
                        ctx.device_id(),
                        dc,
                    )
                })?;
                let mut dev = Rm67162::with_gpio_dc(cs_pin, dc);
                SpiDevice::set_dc_source(&mut dev, odr_addr, bit);
                ctx.attach_spi_device(Box::new(dev))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the panel the way a controller with hardware DCX does: D/C low for
    /// the command byte, high for its parameters.
    fn cmd(dev: &mut Rm67162, c: u8, params: &[u8]) {
        dev.set_dc_level(false);
        dev.transfer(c);
        dev.set_dc_level(true);
        for &p in params {
            dev.transfer(p);
        }
    }

    fn pixels(dev: &mut Rm67162, px: &[u16]) {
        dev.set_dc_level(false);
        dev.transfer(CMD_RAMWR);
        dev.set_dc_level(true);
        for &p in px {
            dev.transfer((p >> 8) as u8);
            dev.transfer((p & 0xFF) as u8);
        }
    }

    fn panel() -> Rm67162 {
        Rm67162::with_controller_dc("P1.12")
    }

    #[test]
    fn a_full_init_lights_the_panel() {
        let mut d = panel();
        cmd(&mut d, CMD_SLPOUT, &[]);
        cmd(&mut d, CMD_COLMOD, &[COLMOD_RGB565]);
        cmd(&mut d, CMD_WRDISBV, &[0xFF]);
        cmd(&mut d, CMD_DISPON, &[]);
        assert!(
            d.is_lit(),
            "SLPOUT + brightness + DISPON must light the panel"
        );
    }

    /// THE AMOLED ASSERTION.
    ///
    /// A driver ported from a backlit TFT does everything right except write
    /// brightness, because on a TFT brightness is a separate backlight pin that
    /// is not the controller's business. On an AMOLED that firmware displays
    /// nothing. A model that reported this panel as "on" would be flattering
    /// firmware that cannot work on the real part.
    #[test]
    fn dispon_without_brightness_is_not_lit() {
        let mut d = panel();
        cmd(&mut d, CMD_SLPOUT, &[]);
        cmd(&mut d, CMD_DISPON, &[]);
        cmd(&mut d, CMD_CASET, &[0, 0, 0, 9]);
        cmd(&mut d, CMD_RASET, &[0, 0, 0, 0]);
        pixels(&mut d, &[0xF800; 10]);

        assert!(d.display_on(), "DISPON was sent");
        assert_eq!(d.brightness(), 0, "WRDISBV was never written");
        assert!(
            !d.is_lit(),
            "an emissive panel at brightness 0 shows nothing, whatever DISPON says"
        );
        // The pixels really did land -- this is not a case of nothing happening.
        assert_ne!(
            d.framebuffer().iter().filter(|&&b| b != 0).count(),
            0,
            "frame memory must still hold what was written"
        );
    }

    /// A panel that is lit but still asleep is also not showing anything.
    #[test]
    fn sleep_beats_brightness() {
        let mut d = panel();
        cmd(&mut d, CMD_WRDISBV, &[0xFF]);
        cmd(&mut d, CMD_DISPON, &[]);
        assert!(
            !d.is_lit(),
            "the panel powers up asleep; SLPOUT is required"
        );
        cmd(&mut d, CMD_SLPOUT, &[]);
        assert!(d.is_lit());
    }

    /// NEGATIVE CONTROL for the framing rule.
    ///
    /// 0x2C is RAMWR as a COMMAND and an ordinary value as a PARAMETER. This is
    /// the exact byte that broke the value-inference framing the ILI9341 model
    /// documents. With D/C framing it must be consumed as a parameter and open
    /// no pixel stream -- if it ever does, the following init bytes are written
    /// into the framebuffer as pixels and the panel goes to garbage.
    #[test]
    fn a_parameter_byte_equal_to_ramwr_does_not_open_a_pixel_stream() {
        let mut d = panel();
        // MADCTL with the parameter 0x2C.
        cmd(&mut d, CMD_MADCTL, &[0x2C]);
        // Now send what would be the next init command. If 0x2C had been read
        // as RAMWR, these bytes would land in frame memory instead.
        cmd(&mut d, CMD_WRDISBV, &[0xFF]);
        cmd(&mut d, CMD_SLPOUT, &[]);
        cmd(&mut d, CMD_DISPON, &[]);

        assert_eq!(
            d.framebuffer().iter().filter(|&&b| b != 0).count(),
            0,
            "no pixel may have been written -- 0x2C was a parameter, not RAMWR"
        );
        assert_eq!(d.brightness(), 0xFF, "the init sequence must have applied");
        assert!(d.is_lit());
    }

    /// Pixels land where CASET/RASET put them, not merely somewhere.
    #[test]
    fn pixels_land_at_the_addressed_window() {
        let mut d = panel();
        // A 2x2 window at (10, 20).
        cmd(&mut d, CMD_CASET, &[0, 10, 0, 11]);
        cmd(&mut d, CMD_RASET, &[0, 20, 0, 21]);
        pixels(&mut d, &[0x07E0, 0x07E0, 0x07E0, 0x07E0]);

        let at = |x: usize, y: usize| -> u16 {
            let i = (y * WIDTH + x) * 2;
            u16::from_be_bytes([d.framebuffer()[i], d.framebuffer()[i + 1]])
        };
        assert_eq!(at(10, 20), 0x07E0);
        assert_eq!(at(11, 20), 0x07E0);
        assert_eq!(at(10, 21), 0x07E0);
        assert_eq!(at(11, 21), 0x07E0);
        // And nowhere else.
        assert_eq!(at(12, 20), 0x0000);
        assert_eq!(at(10, 22), 0x0000);
        assert_eq!(
            d.framebuffer().iter().filter(|&&b| b != 0).count(),
            8,
            "exactly four RGB565 pixels, and no more"
        );
    }

    /// A window past the die's edge is clipped, not wrapped into a shear.
    #[test]
    fn an_oversized_window_is_clipped_to_the_panel() {
        let mut d = panel();
        cmd(&mut d, CMD_CASET, &[0x01, 0x00, 0x02, 0x00]); // 256..512, past 239
        assert!(
            d.col_end < WIDTH as u16,
            "CASET must clip to the addressable width"
        );
        cmd(&mut d, CMD_RASET, &[0x00, 0x00, 0x0F, 0xFF]); // 0..4095, past 535
        assert!(
            d.row_end < HEIGHT as u16,
            "RASET must clip to the addressable height"
        );
    }

    /// SWRESET returns the panel to the state it powers up in -- including
    /// asleep and brightness zero, which is what makes a re-init observable.
    #[test]
    fn swreset_restores_the_power_up_state() {
        let mut d = panel();
        cmd(&mut d, CMD_SLPOUT, &[]);
        cmd(&mut d, CMD_WRDISBV, &[0xFF]);
        cmd(&mut d, CMD_DISPON, &[]);
        pixels(&mut d, &[0xFFFF; 16]);
        assert!(d.is_lit());

        cmd(&mut d, CMD_SWRESET, &[]);
        assert!(!d.is_lit());
        assert!(d.asleep());
        assert_eq!(d.brightness(), 0);
        assert_eq!(
            d.framebuffer().iter().filter(|&&b| b != 0).count(),
            0,
            "SWRESET clears frame memory"
        );
    }

    /// The artifact reports `lit`, not just `display_on` -- so a lab asserting
    /// on evidence cannot be fooled by a dark panel that answered DISPON.
    #[test]
    fn the_artifact_separates_display_on_from_lit() {
        let mut d = panel();
        cmd(&mut d, CMD_SLPOUT, &[]);
        cmd(&mut d, CMD_DISPON, &[]);
        let arts = SpiDevice::artifacts(&d, "panel", &crate::inspect::InspectOpts::default());
        let meta = &arts[0].meta;
        assert_eq!(meta["display_on"], serde_json::json!(true));
        assert_eq!(meta["lit"], serde_json::json!(false));
        assert_eq!(meta["brightness"], serde_json::json!(0));
        assert_eq!(meta["w"], serde_json::json!(240));
        assert_eq!(meta["h"], serde_json::json!(536));
        assert_eq!(meta["dc_source"], serde_json::json!("controller_dcx"));
    }
}
