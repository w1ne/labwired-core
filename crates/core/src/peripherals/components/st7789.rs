// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Sitronix ST7789V TFT controller over 4-wire SPI.
//!
//! Every constant here is read out of the ST7789V datasheet held in the corpus
//! (Version 1.3, 2014/03, sha256 8ecf0e43…), cited by page below rather than
//! carried over from the ILI9341 model next door. The two controllers share the
//! MIPI DCS command set, so the code is deliberately shaped like `ili9341.rs` —
//! but the agreement is checked, not assumed.
//!
//! ⚠️ THE FRAME MEMORY IS 240x320, NOT THE PANEL SIZE. §8.12 (p.124): "The
//! address ranges are X=0 to X=239 (Efh) and Y=0 to Y=319 (13Fh)." A 1.9"
//! 170x320 module is a smaller glass wired to a subset of the source lines; the
//! controller still has all 240 columns. Firmware picks the visible strip with
//! CASET, which this model already honours, so NO panel offset is baked in.
//!
//! That absence is deliberate. The column offset of a particular glass (35 for
//! many 170-wide modules) appears in NEITHER the Sitronix document NOR the
//! module vendor's drawing — it is an integration value. Hard-coding it would
//! put an unsourced number where a sourced one is indistinguishable, and would
//! be wrong for every other ST7789 panel. `visible` below makes it opt-in and
//! says where it has to come from.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;

/// Frame memory extent. Datasheet §8.12, p.124: X = 0..239 (0xEF),
/// Y = 0..319 (0x13F). "Addresses outside these ranges are not allowed."
const WIDTH: usize = 240;
const HEIGHT: usize = 320;
const FB_BYTES: usize = WIDTH * HEIGHT * 2; // RGB565, 2 bytes per pixel

/// MADCTL (36h) bits. Datasheet §9.1.28, p.215: D7 = page address order (MY),
/// D6 = column address order (MX), D5 = page/column exchange (MV).
const MADCTL_MY: u8 = 0x80;
const MADCTL_MX: u8 = 0x40;
const MADCTL_MV: u8 = 0x20;

// Command opcodes, all from the command table at §9.1 / p.157.
const CMD_SWRESET: u8 = 0x01;
const CMD_SLPIN: u8 = 0x10;
const CMD_SLPOUT: u8 = 0x11;
const CMD_NORON: u8 = 0x13;
const CMD_INVOFF: u8 = 0x20;
const CMD_INVON: u8 = 0x21;
const CMD_DISPOFF: u8 = 0x28;
const CMD_DISPON: u8 = 0x29;
const CMD_CASET: u8 = 0x2A;
const CMD_RASET: u8 = 0x2B;
const CMD_RAMWR: u8 = 0x2C;
const CMD_MADCTL: u8 = 0x36;
const CMD_COLMOD: u8 = 0x3A;
/// WRMEMC, §9.1.33 p.225: "continuing from the pixel location following the
/// previous write memory continue or memory write command" — so unlike RAMWR it
/// must NOT reset the address counters.
const CMD_WRMEMC: u8 = 0x3C;

/// Command/data framing state.
///
/// Framing comes from the D/C line only. The ILI9341 model kept a
/// byte-value-inference fallback for callers that never wired D/C, and that
/// path is exactly how an init sequence's parameter byte 0x2C got decoded as
/// RAMWR, opening the pixel stream mid-init. This model has no such fallback:
/// `attach` refuses a device with no D/C source rather than guess. Same
/// decision the RM67162 model made, for the same reason.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ProtoState {
    Idle,
    AwaitingParams {
        cmd: u8,
        params: [u8; 4],
        have: u8,
        want: u8,
    },
    AwaitingPixelHi,
    AwaitingPixelLo {
        hi: u8,
    },
}

/// The visible strip of frame memory for a particular glass.
///
/// NOT a datasheet value — see the module note. Supplied per-module or left
/// unset, in which case the artifact reports the whole 240x320 frame memory,
/// which is what the controller actually holds.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct VisibleWindow {
    pub col_offset: u16,
    pub row_offset: u16,
    pub cols: u16,
    pub rows: u16,
}

/// Simulated ST7789V 240x320 RGB565 TFT controller.
#[derive(Debug, serde::Serialize)]
pub struct St7789 {
    cs_pin: String,
    display_on: bool,
    /// SLPOUT seen. §9.1.12/§9.1.13: the panel is asleep out of reset, and a
    /// sleeping panel shows nothing however full frame memory is.
    awake: bool,
    /// INVON/INVOFF. Recorded because most IPS ST7789 modules are wired to run
    /// inverted, so whether the firmware sent 0x21 decides whether the picture
    /// is right or photographically negative.
    inverted: bool,
    cur_col: u16,
    cur_row: u16,
    col_start: u16,
    col_end: u16,
    row_start: u16,
    row_end: u16,
    #[serde(skip_serializing)]
    framebuffer: Vec<u8>,
    #[serde(skip_serializing)]
    state: ProtoState,
    dc_pin: Option<String>,
    dc_level: bool,
    /// Resolved GPIO output register + bit the bus samples D/C from. Without
    /// this the bus's latch filter drops the device, D/C stays low, and every
    /// byte frames as a command — declaring `dc_pin` alone is not enough.
    dc_source: Option<(u64, u8)>,
    madctl: u8,
    #[serde(skip_serializing)]
    param_buf: [u8; 4],
    param_len: usize,
    visible: Option<VisibleWindow>,
}

impl Default for St7789 {
    fn default() -> Self {
        Self::new("PA4")
    }
}

impl St7789 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            display_on: false,
            awake: false,
            inverted: false,
            cur_col: 0,
            cur_row: 0,
            col_start: 0,
            // §9.1.20 p.198, power-on default: XS=0x00, XE=0xEF.
            col_end: (WIDTH as u16) - 1,
            row_start: 0,
            // §9.1.21 p.200, power-on default: YS=0x00, YE=0x13F.
            row_end: (HEIGHT as u16) - 1,
            framebuffer: vec![0u8; FB_BYTES],
            state: ProtoState::Idle,
            dc_pin: None,
            dc_level: false,
            dc_source: None,
            madctl: 0,
            param_buf: [0; 4],
            param_len: 0,
            visible: None,
        }
    }

    pub fn with_dc_pin(mut self, dc_pin: impl Into<String>) -> Self {
        self.dc_pin = Some(dc_pin.into());
        self
    }

    pub fn with_visible_window(mut self, w: VisibleWindow) -> Self {
        self.visible = Some(w);
        self
    }

    pub fn display_on(&self) -> bool {
        self.display_on
    }

    /// What a camera would see: DISPON **and** awake. §9.1.19 p.196 makes
    /// DISPON meaningful only out of sleep, so reporting DISPON alone would
    /// call a sleeping panel lit.
    pub fn lit(&self) -> bool {
        self.display_on && self.awake
    }

    pub fn inverted(&self) -> bool {
        self.inverted
    }

    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    /// Addressable extent in the CURRENT orientation. §9.1.20 p.198 states the
    /// range as 0..239 when MV=0 and 0..319 when MV=1 — the controller itself
    /// changes what a legal column is, so clamping to 239 folds a correct
    /// landscape window into portrait.
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

    /// Map a logical (column, row) onto the physical 240x320 frame memory,
    /// which does not rotate. §8.12 p.124 / §9.1.28 p.215.
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

    /// Frame memory in the firmware's own coordinates, cropped to the visible
    /// strip when one was declared.
    pub fn oriented_framebuffer(&self) -> Vec<u8> {
        let (w, h) = self.logical_dimensions();
        let mut out = vec![0u8; w * h * 2];
        let (cx, cy) = match self.visible {
            Some(v) => (v.col_offset as usize, v.row_offset as usize),
            None => (0, 0),
        };
        for row in 0..h {
            for col in 0..w {
                let (x, y) = self.to_physical((col + cx) as u16, (row + cy) as u16);
                if x >= WIDTH || y >= HEIGHT {
                    continue;
                }
                let src = (y * WIDTH + x) * 2;
                let dst = (row * w + col) * 2;
                out[dst] = self.framebuffer[src];
                out[dst + 1] = self.framebuffer[src + 1];
            }
        }
        out
    }

    /// Extent of the artifact: the visible strip if declared, else the whole
    /// frame memory in the current orientation.
    pub fn logical_dimensions(&self) -> (usize, usize) {
        if let Some(v) = self.visible {
            return (v.cols as usize, v.rows as usize);
        }
        (
            self.addressable_width() as usize,
            self.addressable_height() as usize,
        )
    }

    pub fn dimensions(&self) -> (usize, usize) {
        (WIDTH, HEIGHT)
    }

    /// How many parameter bytes a command carries before it can be applied.
    /// With D/C framing this decides only WHEN to apply, never where a command
    /// ends — an unknown command's parameters are consumed as data and ignored.
    fn param_count(cmd: u8) -> usize {
        match cmd {
            CMD_CASET | CMD_RASET => 4,
            CMD_MADCTL | CMD_COLMOD => 1,
            _ => 0,
        }
    }

    fn apply_simple_command(&mut self, cmd: u8) {
        match cmd {
            CMD_DISPON => self.display_on = true,
            CMD_DISPOFF => self.display_on = false,
            CMD_SLPOUT => self.awake = true,
            CMD_SLPIN => self.awake = false,
            CMD_INVON => self.inverted = true,
            CMD_INVOFF => self.inverted = false,
            CMD_NORON => {}
            CMD_SWRESET => {
                // §9.1.22 p.202, S/W Reset: "Contents of memory is not
                // cleared." Only the control state resets. Clearing the buffer
                // here would erase a painted frame that real silicon keeps.
                self.display_on = false;
                self.awake = false;
                self.inverted = false;
                self.madctl = 0;
                self.col_start = 0;
                self.col_end = (WIDTH as u16) - 1;
                self.row_start = 0;
                self.row_end = (HEIGHT as u16) - 1;
                self.cur_col = 0;
                self.cur_row = 0;
            }
            _ => {}
        }
    }

    fn handle_params_complete(&mut self, cmd: u8, params: &[u8; 4]) {
        match cmd {
            CMD_CASET => {
                let xs = u16::from_be_bytes([params[0], params[1]]);
                let xe = u16::from_be_bytes([params[2], params[3]]);
                let max = self.addressable_width().saturating_sub(1);
                self.col_start = xs.min(max);
                self.col_end = xe.min(max);
                self.cur_col = self.col_start;
            }
            CMD_RASET => {
                let ys = u16::from_be_bytes([params[0], params[1]]);
                let ye = u16::from_be_bytes([params[2], params[3]]);
                let max = self.addressable_height().saturating_sub(1);
                self.row_start = ys.min(max);
                self.row_end = ye.min(max);
                self.cur_row = self.row_start;
            }
            CMD_MADCTL => self.madctl = params[0],
            CMD_COLMOD => {}
            _ => {}
        }
    }

    /// Write one RGB565 pixel and advance the counters.
    ///
    /// §8.12 p.124 states the counter rules exactly: a completed write
    /// increments the column and leaves the row alone; a column past XE returns
    /// to XS and increments the row; past YE as well and both return to start.
    fn write_pixel(&mut self, hi: u8, lo: u8) {
        let (x, y) = self.to_physical(self.cur_col, self.cur_row);
        if x < WIDTH && y < HEIGHT {
            let idx = (y * WIDTH + x) * 2;
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

    fn dc_command(&mut self, cmd: u8) {
        let want = Self::param_count(cmd);
        match cmd {
            CMD_RAMWR => {
                // §9.1.22 p.202: "When this command is accepted, the column
                // register and the page register are reset to the start
                // column/start page positions."
                self.cur_col = self.col_start;
                self.cur_row = self.row_start;
                self.state = ProtoState::AwaitingPixelHi;
            }
            CMD_WRMEMC => {
                // §9.1.33 p.225: continues from where the last write left off,
                // so the counters are deliberately NOT reset here.
                self.state = ProtoState::AwaitingPixelHi;
            }
            _ if want > 0 => {
                self.param_buf = [0; 4];
                self.param_len = 0;
                self.state = ProtoState::AwaitingParams {
                    cmd,
                    params: [0; 4],
                    have: 0,
                    want: want as u8,
                };
            }
            _ => {
                self.apply_simple_command(cmd);
                self.state = ProtoState::Idle;
            }
        }
    }

    fn dc_data(&mut self, byte: u8) {
        match self.state {
            ProtoState::AwaitingParams {
                cmd,
                mut params,
                have,
                want,
            } => {
                let have = have as usize;
                if have < params.len() {
                    params[have] = byte;
                }
                let have = have + 1;
                if have >= want as usize {
                    self.handle_params_complete(cmd, &params);
                    self.state = ProtoState::Idle;
                } else {
                    self.state = ProtoState::AwaitingParams {
                        cmd,
                        params,
                        have: have as u8,
                        want,
                    };
                }
            }
            ProtoState::AwaitingPixelHi => {
                self.state = ProtoState::AwaitingPixelLo { hi: byte };
            }
            ProtoState::AwaitingPixelLo { hi } => {
                self.write_pixel(hi, byte);
                self.state = ProtoState::AwaitingPixelHi;
            }
            // A data byte with no command open is a stray on real silicon too.
            ProtoState::Idle => {}
        }
    }
}

impl SpiDevice for St7789 {
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
                "display_on": self.display_on(),
                // `lit` is the one a photo can be checked against: a panel that
                // got DISPON but never SLPOUT is dark on the bench.
                "lit": self.lit(),
                "awake": self.awake,
                "inverted": self.inverted,
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
        self.state = ProtoState::Idle;
    }

    fn cs_release(&mut self) {}

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
        if self.dc_level {
            self.dc_data(mosi);
        } else {
            self.dc_command(mosi);
        }
        0
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    // Both halves, deliberately. A one-sided impl compiles and passes every
    // unit test while the downcast that actually reads the framebuffer from
    // outside silently gets None.
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct St7789Kit;
pub static ST7789_KIT: St7789Kit = St7789Kit;

static ST7789_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "st7789-170x320",
    label: "ST7789 IPS TFT",
    summary: "1.9in 170x320 IPS TFT on a Sitronix ST7789V, 4-wire SPI.",
    detail: "Sitronix ST7789V controller against an in-memory 240x320 RGB565 frame memory, \
             the extent the datasheet gives in section 8.12 (X 0..239, Y 0..319). The 1.9in \
             glass shows a 170-column strip of that memory; firmware selects it with CASET, \
             so no panel offset is assumed here -- that number is in neither Sitronix's \
             document nor the module vendor's drawing. Set `col_offset`/`cols` to crop the \
             artifact to a particular glass. Reports `lit` (DISPON AND awake) beside the raw \
             pixels, because a panel that never got SLPOUT is dark whatever is in memory.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[
        ConfigKey {
            name: "cs_pin",
            ty: ConfigType::Str,
            doc: "Chip-select pin label.",
        },
        ConfigKey {
            name: "dc_pin",
            ty: ConfigType::Str,
            doc: "Data/command GPIO pin. Required: this model has no \
                  infer-framing-from-byte-values fallback, because that inference \
                  decodes a parameter byte 0x2C as RAMWR and writes the rest of the \
                  init sequence into the framebuffer as pixels.",
        },
        ConfigKey {
            name: "col_offset",
            ty: ConfigType::Int,
            doc: "First frame-memory column the glass shows. NOT a datasheet value -- \
                  the ST7789V document describes a 240-column frame memory and says \
                  nothing about which strip a given panel exposes. Leave unset to get \
                  the whole 240x320 frame memory, which is what the controller holds.",
        },
        ConfigKey {
            name: "row_offset",
            ty: ConfigType::Int,
            doc: "First frame-memory row the glass shows. Same provenance caveat as \
                  `col_offset`.",
        },
        ConfigKey {
            name: "cols",
            ty: ConfigType::Int,
            doc: "Visible column count, e.g. 170 for the 1.9in module. Requires \
                  `col_offset`.",
        },
        ConfigKey {
            name: "rows",
            ty: ConfigType::Int,
            doc: "Visible row count, e.g. 320 for the 1.9in module. Requires \
                  `row_offset`.",
        },
    ],
    labs: &[],
};

impl PeripheralKit for St7789Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &ST7789_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs_pin = ctx.config_str("cs_pin").unwrap_or("").to_string();
        let dc = ctx.config_str("dc_pin").map(|s| s.to_string()).ok_or_else(|| {
            anyhow::anyhow!(
                "st7789-170x320 '{}': no `dc_pin`. This panel frames commands from the D/C \
                 line and has no infer-from-byte-values fallback: that inference decodes a \
                 parameter byte 0x2C as RAMWR and writes the remaining init bytes into the \
                 framebuffer as pixels, leaving a blank screen and a blameless firmware.",
                ctx.device_id(),
            )
        })?;

        // Resolving the pin to its GPIO output register is the half that makes
        // D/C real: the bus samples that register before each transfer.
        // Declaring the pin without this leaves D/C stuck low, every byte
        // frames as a command, and the panel renders blank with no error.
        let (odr_addr, bit) = ctx.resolve_pin_odr(&dc).ok_or_else(|| {
            anyhow::anyhow!(
                "st7789-170x320 '{}': D/C pin '{}' does not resolve to a driveable GPIO output.",
                ctx.device_id(),
                dc,
            )
        })?;

        let mut dev = St7789::new(cs_pin).with_dc_pin(dc);

        // A crop is all-or-nothing: a half-declared window would silently
        // report a strip at the wrong offset, which looks like a working
        // display showing the wrong part of the image.
        let col_offset = ctx.config_i64("col_offset");
        let row_offset = ctx.config_i64("row_offset");
        let cols = ctx.config_i64("cols");
        let rows = ctx.config_i64("rows");
        if col_offset.is_some() || row_offset.is_some() || cols.is_some() || rows.is_some() {
            let (Some(co), Some(ro), Some(c), Some(r)) = (col_offset, row_offset, cols, rows)
            else {
                anyhow::bail!(
                    "st7789-170x320 '{}': a visible window needs all four of `col_offset`, \
                     `row_offset`, `cols` and `rows`. Declaring some of them would crop the \
                     artifact at a guessed offset and render a plausible wrong picture.",
                    ctx.device_id(),
                );
            };
            if co as usize + c as usize > WIDTH || ro as usize + r as usize > HEIGHT {
                anyhow::bail!(
                    "st7789-170x320 '{}': visible window {}x{} at ({}, {}) runs past the \
                     240x320 frame memory the ST7789V has (datasheet section 8.12).",
                    ctx.device_id(),
                    c,
                    r,
                    co,
                    ro,
                );
            }
            dev = dev.with_visible_window(VisibleWindow {
                col_offset: co as u16,
                row_offset: ro as u16,
                cols: c as u16,
                rows: r as u16,
            });
        }

        SpiDevice::set_dc_source(&mut dev, odr_addr, bit);
        ctx.attach_spi_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the panel the way a wired D/C line does: low for the command
    /// byte, high for its parameters.
    fn cmd(dev: &mut St7789, c: u8, params: &[u8]) {
        dev.set_dc_level(false);
        dev.transfer(c);
        dev.set_dc_level(true);
        for &p in params {
            dev.transfer(p);
        }
    }

    fn pixels(dev: &mut St7789, px: &[u16]) {
        dev.set_dc_level(false);
        dev.transfer(CMD_RAMWR);
        dev.set_dc_level(true);
        for &p in px {
            dev.transfer((p >> 8) as u8);
            dev.transfer((p & 0xFF) as u8);
        }
    }

    fn window(dev: &mut St7789, xs: u16, xe: u16, ys: u16, ye: u16) {
        cmd(
            dev,
            CMD_CASET,
            &[(xs >> 8) as u8, xs as u8, (xe >> 8) as u8, xe as u8],
        );
        cmd(
            dev,
            CMD_RASET,
            &[(ys >> 8) as u8, ys as u8, (ye >> 8) as u8, ye as u8],
        );
    }

    fn px_at(dev: &St7789, x: usize, y: usize) -> u16 {
        let i = (y * WIDTH + x) * 2;
        u16::from_be_bytes([dev.framebuffer[i], dev.framebuffer[i + 1]])
    }

    fn dev() -> St7789 {
        St7789::new("PA4").with_dc_pin("PB0")
    }

    /// Datasheet section 8.12, p.124: "The address ranges are X=0 to X=239
    /// (Efh) and Y=0 to Y=319 (13Fh)."
    #[test]
    fn frame_memory_is_the_240x320_the_datasheet_states() {
        assert_eq!(dev().dimensions(), (240, 320));
    }

    /// Section 9.1.20 p.198 / 9.1.21 p.200: power-on defaults are the full
    /// frame memory, XE=0xEF and YE=0x13F.
    #[test]
    fn power_on_window_is_the_whole_frame_memory() {
        let d = dev();
        assert_eq!((d.col_start, d.col_end), (0, 239));
        assert_eq!((d.row_start, d.row_end), (0, 319));
    }

    #[test]
    fn writes_a_pixel_inside_the_window() {
        let mut d = dev();
        window(&mut d, 10, 10, 20, 20);
        pixels(&mut d, &[0xF800]);
        assert_eq!(px_at(&d, 10, 20), 0xF800);
    }

    /// Section 8.12 p.124: a completed write increments the column and leaves
    /// the row alone.
    #[test]
    fn a_completed_write_increments_the_column_only() {
        let mut d = dev();
        window(&mut d, 5, 8, 3, 4);
        pixels(&mut d, &[0x1111, 0x2222]);
        assert_eq!(px_at(&d, 5, 3), 0x1111);
        assert_eq!(px_at(&d, 6, 3), 0x2222);
        assert_eq!(px_at(&d, 5, 4), 0x0000, "row must not have advanced");
    }

    /// Section 8.12 p.124: "The Column counter value is larger than End Column
    /// (XE)" -> column returns to XS and the row increments.
    #[test]
    fn column_past_xe_wraps_to_xs_and_increments_the_row() {
        let mut d = dev();
        window(&mut d, 5, 6, 3, 4);
        pixels(&mut d, &[0xAAAA, 0xBBBB, 0xCCCC]);
        assert_eq!(px_at(&d, 5, 3), 0xAAAA);
        assert_eq!(px_at(&d, 6, 3), 0xBBBB);
        assert_eq!(px_at(&d, 5, 4), 0xCCCC);
    }

    /// Section 8.12 p.124: past XE and YE both, both counters return to start.
    #[test]
    fn past_both_ends_the_pointers_wrap_to_the_window_origin() {
        let mut d = dev();
        window(&mut d, 5, 6, 3, 4);
        // Five pixels into a 2x2 window: the fifth lands back on the origin.
        pixels(&mut d, &[0x1111, 0x2222, 0x3333, 0x4444, 0x5555]);
        assert_eq!(px_at(&d, 5, 3), 0x5555, "wrapped back to (XS, YS)");
    }

    /// Section 9.1.22 p.202: "When this command is accepted, the column
    /// register and the page register are reset to the start column/start page
    /// positions."
    #[test]
    fn ramwr_restarts_the_window() {
        let mut d = dev();
        window(&mut d, 5, 8, 3, 4);
        pixels(&mut d, &[0x1111, 0x2222]);
        pixels(&mut d, &[0x3333]);
        assert_eq!(px_at(&d, 5, 3), 0x3333, "second RAMWR restarted at XS/YS");
    }

    /// Section 9.1.33 p.225: WRMEMC continues "from the pixel location
    /// following the previous write memory continue or memory write command" —
    /// so it must NOT reset the counters the way RAMWR does.
    #[test]
    fn wrmemc_continues_where_ramwr_stopped() {
        let mut d = dev();
        window(&mut d, 5, 8, 3, 4);
        pixels(&mut d, &[0x1111, 0x2222]);
        d.set_dc_level(false);
        d.transfer(CMD_WRMEMC);
        d.set_dc_level(true);
        d.transfer(0x33);
        d.transfer(0x33);
        assert_eq!(px_at(&d, 7, 3), 0x3333, "continued, did not restart");
        assert_eq!(px_at(&d, 5, 3), 0x1111, "origin untouched");
    }

    /// Section 9.1.22 p.202, S/W Reset row: "Contents of memory is not
    /// cleared." Only the control state resets.
    #[test]
    fn swreset_keeps_frame_memory_and_drops_the_display() {
        let mut d = dev();
        window(&mut d, 10, 10, 20, 20);
        pixels(&mut d, &[0xF800]);
        cmd(&mut d, CMD_SLPOUT, &[]);
        cmd(&mut d, CMD_DISPON, &[]);
        assert!(d.lit());
        cmd(&mut d, CMD_SWRESET, &[]);
        assert_eq!(px_at(&d, 10, 20), 0xF800, "memory survives a software reset");
        assert!(!d.display_on());
        assert!(!d.lit());
    }

    /// DISPON alone is not a lit panel: section 9.1.19 p.196 makes it
    /// meaningful only out of sleep, and a panel that never got SLPOUT is dark
    /// on the bench however full frame memory is.
    #[test]
    fn dispon_without_slpout_is_not_lit() {
        let mut d = dev();
        cmd(&mut d, CMD_DISPON, &[]);
        assert!(d.display_on());
        assert!(!d.lit(), "DISPON without SLPOUT must not read as lit");
        cmd(&mut d, CMD_SLPOUT, &[]);
        assert!(d.lit());
        cmd(&mut d, CMD_SLPIN, &[]);
        assert!(!d.lit(), "SLPIN puts it back to dark");
    }

    /// Section 9.1.15/9.1.16 pp.188-190. Most IPS ST7789 glass runs inverted,
    /// so whether the firmware sent 0x21 decides whether the picture is right
    /// or photographically negative.
    #[test]
    fn inversion_is_tracked() {
        let mut d = dev();
        assert!(!d.inverted());
        cmd(&mut d, CMD_INVON, &[]);
        assert!(d.inverted());
        cmd(&mut d, CMD_INVOFF, &[]);
        assert!(!d.inverted());
    }

    /// Section 9.1.20 p.198 gives the legal column range as 0..239 when MV=0
    /// and 0..319 when MV=1 — the controller itself changes what a legal column
    /// is, so a landscape driver must get the full 320.
    #[test]
    fn madctl_mv_allows_the_full_320_column_window() {
        let mut d = dev();
        cmd(&mut d, CMD_MADCTL, &[MADCTL_MV]);
        window(&mut d, 0, 319, 0, 239);
        assert_eq!(d.col_end, 319, "landscape window must not clamp to 239");
        assert_eq!(d.logical_dimensions(), (320, 240));
    }

    /// A parameter byte must never be read as a command. 0x2C as the second
    /// parameter of an init command is the exact byte that opened a pixel
    /// stream mid-init in the inference-based model.
    #[test]
    fn a_parameter_byte_of_2c_is_not_decoded_as_ramwr() {
        let mut d = dev();
        window(&mut d, 0, 239, 0, 319);
        // CASET whose parameters happen to contain 0x2C.
        cmd(&mut d, CMD_CASET, &[0x00, 0x2C, 0x00, 0x2C]);
        cmd(&mut d, CMD_SLPOUT, &[]);
        cmd(&mut d, CMD_DISPON, &[]);
        assert!(d.lit(), "init survived: SLPOUT and DISPON were not eaten as pixels");
        assert_eq!(
            d.framebuffer.iter().filter(|&&b| b != 0).count(),
            0,
            "no init byte was written into frame memory as a pixel",
        );
    }

    /// The visible strip crops the artifact without moving what is in frame
    /// memory. Offsets are an integration value, so the model must take them
    /// rather than assume them.
    #[test]
    fn a_visible_window_crops_the_artifact_only() {
        let mut d = St7789::new("PA4").with_dc_pin("PB0").with_visible_window(VisibleWindow {
            col_offset: 35,
            row_offset: 0,
            cols: 170,
            rows: 320,
        });
        assert_eq!(d.logical_dimensions(), (170, 320));
        window(&mut d, 35, 35, 0, 0);
        pixels(&mut d, &[0x07E0]);
        // Physical memory still holds it at column 35 ...
        assert_eq!(px_at(&d, 35, 0), 0x07E0);
        // ... and the cropped artifact shows it at column 0.
        let fb = d.oriented_framebuffer();
        assert_eq!(fb.len(), 170 * 320 * 2);
        assert_eq!(u16::from_be_bytes([fb[0], fb[1]]), 0x07E0);
    }

    /// With no window declared the artifact is the whole frame memory, which is
    /// what the controller actually holds. No panel offset is assumed.
    #[test]
    fn without_a_window_the_artifact_is_the_whole_frame_memory() {
        let d = dev();
        assert_eq!(d.logical_dimensions(), (240, 320));
        assert_eq!(d.oriented_framebuffer().len(), 240 * 320 * 2);
    }
}
