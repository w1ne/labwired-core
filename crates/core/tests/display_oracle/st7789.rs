//! ⚠️ VERBATIM COPY of the deleted `components/st7789.rs`. See `mod.rs`.
//! Do not edit: its only job is to disagree with the YAML model if the port moved a byte.

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

use labwired_core::peripherals::spi::SpiDevice;
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

/// The visible strip in physical frame-memory coordinates for a particular glass.
/// Its origin and extent stay fixed when firmware changes MADCTL orientation.
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
    /// Whether the module's supply pins are actually connected in the design.
    ///
    /// THE BUG THIS EXISTS FOR. A diagram could wire this panel's SIGNAL pins
    /// only -- SCL/SDA/CS/DC/RES, no VCC and no GND -- and the twin would clock
    /// in the whole init sequence and report `painted_bytes: 2048, lit: true,
    /// display_on: true, awake: true`. On a bench that panel is dark: an
    /// unpowered ST7789V does not latch SPI, does not run its charge pump, and
    /// does not drive the glass. The twin was reporting a working display for a
    /// circuit that cannot work, which is the one thing a digital twin must
    /// never do.
    ///
    /// ⚠️ DEFAULTS TO `true`, DELIBERATELY. The emitter side
    /// (`packages/board-config/src/compile/emitters.ts`) writes `powered:
    /// false` and NOTHING else -- an absent key means "assume powered". That
    /// asymmetry is load-bearing, not laziness: measured over the 551-diagram
    /// ERC corpus, 110 powered external devices across 66 lab manifests have
    /// ZERO supply pins on any net, including every curated
    /// `core/examples/brd2709a/*.yaml`. Curated labs state signals and leave
    /// the rails implicit, essentially universally. Reading an absent key as
    /// "unpowered" would black out every shipped lab on the spot. So only an
    /// EXPLICIT `powered: false` -- which only the diagram compiler emits, and
    /// only when it can see the supply pins are on no net -- darkens a panel.
    ///
    /// It stays dark-and-honest rather than raising a fault: that is what the
    /// hardware does, an `attach` error would refuse to build a circuit the
    /// user can still legitimately want to run, and the reason is already on
    /// the design-time side as the `PWR_SUPPLY_UNCONNECTED` ERC warning. The
    /// artifact carries `"powered"` in its meta so the dark frame is
    /// self-explaining instead of looking like a firmware bug.
    powered: bool,
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
            // Absent supply information means "powered" — see the field's note.
            powered: true,
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

    pub fn display_on(&self) -> bool {
        // An unpowered controller cannot hold DISPON, so this is not a second
        // opinion layered over the state — it is the state. `transfer` already
        // refuses the bus when unpowered, so `self.display_on` can never be
        // true here; the `&&` is the guard that survives someone later adding
        // another way to set it.
        self.powered && self.display_on
    }

    /// What a camera would see: DISPON **and** awake. §9.1.19 p.196 makes
    /// DISPON meaningful only out of sleep, so reporting DISPON alone would
    /// call a sleeping panel lit. A panel with no supply is darker still.
    pub fn lit(&self) -> bool {
        self.powered && self.display_on && self.awake
    }

    /// SLPOUT seen **and** the module has a supply. An unpowered ST7789V is not
    /// asleep, it is off; either way nothing reaches the glass.
    pub fn awake(&self) -> bool {
        self.powered && self.awake
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

    /// The configured glass in fixed physical coordinates, or the whole frame
    /// memory in firmware coordinates when no glass was declared. MADCTL has
    /// already mapped pixel writes into physical memory; applying it again to
    /// a glass window would move the crop and clip landscape writes.
    pub fn oriented_framebuffer(&self) -> Vec<u8> {
        let (w, h) = self.logical_dimensions();
        let mut out = vec![0u8; w * h * 2];
        let (cx, cy) = match self.visible {
            Some(v) => (v.col_offset as usize, v.row_offset as usize),
            None => (0, 0),
        };
        for row in 0..h {
            for col in 0..w {
                let (x, y) = if self.visible.is_some() {
                    (col + cx, row + cy)
                } else {
                    self.to_physical(col as u16, row as u16)
                };
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

    /// Extent of the artifact: the fixed physical glass size if declared, else
    /// the whole frame memory in the current firmware orientation.
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
                "display_on": self.display_on(),
                // `lit` is the one a photo can be checked against: a panel that
                // got DISPON but never SLPOUT is dark on the bench.
                "lit": self.lit(),
                "awake": self.awake(),
                // Reported so a dark frame explains itself. Without this, a
                // panel darkened for having no supply is indistinguishable
                // from one whose firmware forgot SLPOUT.
                "powered": self.powered,
                "inverted": self.inverted,
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
        // THE ONE GATE THAT MAKES AN UNPOWERED PANEL BEHAVE LIKE ONE. Every
        // state change this model has — DISPON, SLPOUT, CASET/RASET, and every
        // pixel byte — arrives through `transfer`. Refusing the bus here is
        // what an unpowered ST7789V does (no supply, no input latches, no
        // charge pump), and it means the reported `display_on` / `awake` /
        // `lit` / `painted_bytes` all stay at their power-on-dark values by
        // construction rather than by masking them at report time.
        if !self.powered {
            return 0;
        }
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
