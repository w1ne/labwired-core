// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;

const WIDTH: usize = 240;
const HEIGHT: usize = 320;
const FB_BYTES: usize = WIDTH * HEIGHT * 2; // RGB565, 2 bytes per pixel

/// MADCTL bits (datasheet §8.2.29). MV swaps the addressing axes — that is what
/// `setRotation(1)`/`(3)` sets, so ignoring it clamps a landscape window to
/// portrait width and silently draws the wrong picture.
const MADCTL_MY: u8 = 0x80;
const MADCTL_MX: u8 = 0x40;
const MADCTL_MV: u8 = 0x20;

/// Protocol state machine for the ILI9341 SPI command/data stream.
///
/// Framing comes from the **D/C line**, exactly as on silicon: D/C low marks a
/// command byte, D/C high marks a data byte. The bus latches the configured
/// GPIO's output level into [`SpiDevice::set_dc_level`] before each transfer.
///
/// This used to infer framing from the byte values instead — a table of "how
/// many parameters does this command take", with unknown commands assumed to
/// take none. That could not survive a real driver's init sequence: Adafruit's
/// stock table sends the undocumented power-control command 0xCB with five
/// parameters, the model treated 0xCB as unknown/zero-parameter, and its second
/// parameter 0x2C was then decoded as RAMWR. The pixel stream opened mid-init
/// and every remaining init byte — including SLPOUT and DISPON — was written
/// into the framebuffer as pixel data. The panel never turned on, and the
/// firmware was blameless.
///
/// With D/C wired, a parameter byte can never be mistaken for a command, so
/// unknown and undocumented commands are harmless: their parameters are
/// consumed as data and ignored. The per-command byte counts below now decide
/// only WHEN a command's parameters are complete enough to apply, never where
/// one command ends and the next begins.
///
/// `Ili9341::new` leaves D/C unwired for backward compatibility with callers
/// that never supplied a pin; that path keeps the old inference and its limits.
/// Prefer [`Ili9341::with_dc_pin`].
#[derive(Debug, Clone, Copy, PartialEq)]
enum ProtoState {
    Idle,
    AwaitingParams {
        cmd: u8,
        params: [u8; 4],
        have: u8,
        want: u8,
    },
    /// Waiting for the high byte of the next RGB565 pixel.
    AwaitingPixelHi,
    /// Have the high byte; waiting for the low byte.
    AwaitingPixelLo {
        hi: u8,
    },
}

/// Simulated ILI9341 240×320 RGB565 TFT display controller.
///
/// Implements the minimal command set needed to support firmware init sequences
/// and full-framebuffer writes:
/// - CASET (0x2A) / PASET (0x2B) — set the pixel-write addressing window
/// - RAMWR (0x2C)                  — open a pixel data stream
/// - DISPON (0x29) / DISPOFF (0x28)
/// - SWRESET (0x01)                — clear framebuffer and reset window
/// - MADCTL, COLMOD, power commands — parameter bytes consumed, values ignored
///
/// RGB565 pixels are stored big-endian (high byte first) in a row-major
/// 153,600-byte Vec: `framebuffer[(row * 240 + col) * 2]` = high byte.
#[derive(Debug, serde::Serialize)]
pub struct Ili9341 {
    cs_pin: String,
    display_on: bool,
    /// Current column pointer for RAMWR writes.
    cur_col: u16,
    /// Current row pointer for RAMWR writes.
    cur_row: u16,
    /// Column window start (set by CASET).
    col_start: u16,
    /// Column window end (set by CASET, inclusive).
    col_end: u16,
    /// Row window start (set by PASET).
    row_start: u16,
    /// Row window end (set by PASET, inclusive).
    row_end: u16,
    /// RGB565 framebuffer, row-major, 2 bytes per pixel (big-endian per ILI9341 wire order).
    /// Skipped in JSON serialization (153 KB is too large for a state snapshot).
    #[serde(skip_serializing)]
    framebuffer: Vec<u8>,
    /// Command/data state machine.
    #[serde(skip_serializing)]
    state: ProtoState,
    /// D/C GPIO label, when the manifest wired one.
    dc_pin: Option<String>,
    /// Latched D/C level at transfer time: false = command, true = data.
    dc_level: bool,
    /// GPIO output register + bit the bus samples the D/C level from. Resolved
    /// once at install time from `dc_pin`. Without this the bus's
    /// `maybe_latch_dc` skips the device entirely — `dc_source()` defaults to
    /// `None` and the filter drops it — so `dc_level` would stay false and
    /// every byte would be read as a command. Declaring `dc_pin` alone is not
    /// enough; this is the half that makes the wire actually drive the model.
    dc_source: Option<(u64, u8)>,
    /// MADCTL (0x36) value — orientation and mirroring.
    madctl: u8,
    /// Command byte currently collecting parameters (D/C-framed path).
    cur_cmd: u8,
    /// Parameters gathered for `cur_cmd`. Sized for the longest command we
    /// interpret; longer ones (gamma tables) are consumed and ignored.
    #[serde(skip_serializing)]
    param_buf: [u8; 4],
    param_len: usize,
    /// True while a RAMWR (0x2C) / RAMWR-continue (0x3C) pixel stream is open.
    in_ramwr: bool,
    /// High byte of a pixel awaiting its low byte.
    pixel_hi: Option<u8>,
}

impl Default for Ili9341 {
    fn default() -> Self {
        Self::new("PA4")
    }
}

impl Ili9341 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            display_on: false,
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
            madctl: 0,
            cur_cmd: 0,
            param_buf: [0; 4],
            param_len: 0,
            in_ramwr: false,
            pixel_hi: None,
        }
    }

    /// Wire the D/C line, which is what makes framing real rather than inferred.
    pub fn with_dc_pin(mut self, dc_pin: impl Into<String>) -> Self {
        self.dc_pin = Some(dc_pin.into());
        self
    }

    /// Addressable extent in the CURRENT orientation. MADCTL's MV bit swaps the
    /// axes, so a landscape driver legitimately sets columns to 0..=319.
    /// Clamping those to 239 (the panel's physical width) silently folded a
    /// correct landscape image into portrait.
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

    /// Map a logical (column, row) in the current orientation onto the physical
    /// 240×320 frame memory, which does not rotate.
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

    /// Return the raw RGB565 framebuffer (153,600 bytes: row-major, 2 bytes per pixel).
    ///
    /// Pixel (col, row) occupies bytes at index `(row * 240 + col) * 2` (high byte)
    /// and `(row * 240 + col) * 2 + 1` (low byte).
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    /// Return the framebuffer as the physical display would scan it: MADCTL
    /// applied so the bytes are in the orientation the firmware intended.
    ///
    /// The raw framebuffer stores pixels at the physical positions `to_physical`
    /// maps them to. A host that renders those bytes row-major sees the mirror/
    /// rotation the panel's scan-out would undo — exactly what the browser canvas
    /// does today, which is why `setRotation(0)` (Adafruit's stock MADCTL 0x48)
    /// shows up mirrored. This view reads back through the same mapping, so the
    /// host can paint row-major and get the right picture.
    pub fn oriented_framebuffer(&self) -> Vec<u8> {
        let mut out = vec![0u8; FB_BYTES];
        for col in 0..WIDTH as u16 {
            for row in 0..HEIGHT as u16 {
                let (x, y) = self.to_physical(col, row);
                let src = (y * WIDTH + x) * 2;
                let dst = (row as usize * WIDTH + col as usize) * 2;
                out[dst] = self.framebuffer[src];
                out[dst + 1] = self.framebuffer[src + 1];
            }
        }
        out
    }

    pub fn display_on(&self) -> bool {
        self.display_on
    }

    pub fn dimensions(&self) -> (usize, usize) {
        (WIDTH, HEIGHT)
    }

    // ---- Internal command dispatch ----

    /// Number of parameter bytes a command must gather before it is applied.
    /// With D/C framing this is a semantic threshold only — it no longer
    /// decides where one command ends and the next begins, so an unlisted
    /// command is harmless rather than stream-corrupting.
    fn param_count(cmd: u8) -> usize {
        match cmd {
            0x2A | 0x2B => 4, // CASET / PASET
            0x36 | 0x3A => 1, // MADCTL / COLMOD
            _ => 0,
        }
    }

    /// A byte arriving with D/C low: a command.
    fn dc_command(&mut self, cmd: u8) {
        self.cur_cmd = cmd;
        self.param_len = 0;
        self.param_buf = [0; 4];
        // Any command other than a RAMWR continue closes an open pixel stream.
        match cmd {
            0x2C => {
                // RAMWR — restart the pixel pointer at the window origin.
                self.cur_col = self.col_start;
                self.cur_row = self.row_start;
                self.in_ramwr = true;
                self.pixel_hi = None;
            }
            0x3C => {
                // RAMWR continue (§8.2.34): resume WITHOUT resetting the
                // pointer, which is how drivers chunk a large blit across
                // transactions. Previously undecoded, so those pixel bytes were
                // read as commands.
                self.in_ramwr = true;
            }
            _ => {
                self.in_ramwr = false;
                self.pixel_hi = None;
                self.apply_simple_command(cmd);
            }
        }
    }

    /// Commands that act immediately and take no parameters.
    fn apply_simple_command(&mut self, cmd: u8) {
        match cmd {
            0x01 => {
                // SWRESET. Datasheet §8.2.2 is explicit: "the Frame Memory
                // contents are unaffected by this command". Clearing it here
                // let firmware that relies on SWRESET to blank the screen pass
                // in the twin and show stale pixels on silicon.
                self.col_start = 0;
                self.col_end = self.addressable_width() - 1;
                self.row_start = 0;
                self.row_end = self.addressable_height() - 1;
                self.display_on = false;
                self.madctl = 0;
            }
            0x28 => self.display_on = false,
            0x29 => self.display_on = true,
            _ => {}
        }
    }

    /// A byte arriving with D/C high: parameter or pixel data.
    fn dc_data(&mut self, byte: u8) {
        if self.in_ramwr {
            match self.pixel_hi.take() {
                None => self.pixel_hi = Some(byte),
                Some(hi) => self.write_pixel(hi, byte),
            }
            return;
        }
        if self.param_len < self.param_buf.len() {
            self.param_buf[self.param_len] = byte;
        }
        self.param_len += 1;
        let want = Self::param_count(self.cur_cmd);
        if want > 0 && self.param_len == want {
            let (cmd, params) = (self.cur_cmd, self.param_buf);
            self.handle_params_complete(cmd, &params);
        }
    }

    fn handle_command(&mut self, cmd: u8) {
        match cmd {
            0x01 => {
                // SWRESET. Frame memory is deliberately NOT cleared — see
                // `apply_simple_command`. Both framing paths must agree, or the
                // same command would mean two things depending on whether a D/C
                // pin happened to be wired.
                self.apply_simple_command(0x01);
                self.state = ProtoState::Idle;
            }
            0x11 => {
                // SLPOUT — sleep out; no parameters
                self.state = ProtoState::Idle;
            }
            0x28 => {
                // DISPOFF
                self.display_on = false;
                self.state = ProtoState::Idle;
            }
            0x29 => {
                // DISPON
                self.display_on = true;
                self.state = ProtoState::Idle;
            }
            0x2A => {
                // CASET — 4 parameter bytes: start_MSB, start_LSB, end_MSB, end_LSB
                self.state = ProtoState::AwaitingParams {
                    cmd,
                    params: [0; 4],
                    have: 0,
                    want: 4,
                };
            }
            0x2B => {
                // PASET — 4 parameter bytes
                self.state = ProtoState::AwaitingParams {
                    cmd,
                    params: [0; 4],
                    have: 0,
                    want: 4,
                };
            }
            0x2C => {
                // RAMWR — open pixel data stream; reset pointer to window start
                self.cur_col = self.col_start;
                self.cur_row = self.row_start;
                self.state = ProtoState::AwaitingPixelHi;
            }
            0x36 => {
                // MADCTL — 1 parameter (orientation bits); ignored in v1
                self.state = ProtoState::AwaitingParams {
                    cmd,
                    params: [0; 4],
                    have: 0,
                    want: 1,
                };
            }
            0x3A => {
                // COLMOD — 1 parameter (color format); ignored in v1 (only RGB565 supported)
                self.state = ProtoState::AwaitingParams {
                    cmd,
                    params: [0; 4],
                    have: 0,
                    want: 1,
                };
            }
            0xC0 => {
                // PWCTR1 — 1 parameter; ignored
                self.state = ProtoState::AwaitingParams {
                    cmd,
                    params: [0; 4],
                    have: 0,
                    want: 1,
                };
            }
            0xC1 => {
                // PWCTR2 — 1 parameter; ignored
                self.state = ProtoState::AwaitingParams {
                    cmd,
                    params: [0; 4],
                    have: 0,
                    want: 1,
                };
            }
            0xC5 => {
                // VMCTR1 — 2 parameters; ignored
                self.state = ProtoState::AwaitingParams {
                    cmd,
                    params: [0; 4],
                    have: 0,
                    want: 2,
                };
            }
            // Datasheet parameter counts: FRMCTR1 (B1h) §8.3.2 takes 2,
            // DISCTRL (B6h) §8.3.7 takes 4. Both were 3 here, so B1h ate the
            // following command byte and B6h left one to be decoded as a
            // command. Only reachable on the legacy no-D/C path now, but wrong
            // is wrong.
            0xB1 => {
                self.state = ProtoState::AwaitingParams {
                    cmd,
                    params: [0; 4],
                    have: 0,
                    want: 2,
                };
            }
            0xB6 => {
                self.state = ProtoState::AwaitingParams {
                    cmd,
                    params: [0; 4],
                    have: 0,
                    want: 4,
                };
            }
            // Single-parameter commands a stock driver sends: VMCTR2 (C7h),
            // GAMMASET (26h), VSCRSADD (37h).
            0xC7 | 0x26 | 0x37 => {
                self.state = ProtoState::AwaitingParams {
                    cmd,
                    params: [0; 4],
                    have: 0,
                    want: 1,
                };
            }
            _ => {
                // Unknown command: no parameters, return to Idle
                self.state = ProtoState::Idle;
            }
        }
    }

    fn handle_params_complete(&mut self, cmd: u8, params: &[u8; 4]) {
        match cmd {
            0x2A => {
                // CASET: set column window, clamped to the CURRENT orientation.
                let start = ((params[0] as u16) << 8) | (params[1] as u16);
                let end = ((params[2] as u16) << 8) | (params[3] as u16);
                let limit = self.addressable_width() - 1;
                self.col_start = start.min(limit);
                self.col_end = end.min(limit);
            }
            0x2B => {
                // PASET: set row window, clamped to the current orientation.
                let start = ((params[0] as u16) << 8) | (params[1] as u16);
                let end = ((params[2] as u16) << 8) | (params[3] as u16);
                let limit = self.addressable_height() - 1;
                self.row_start = start.min(limit);
                self.row_end = end.min(limit);
            }
            0x36 => {
                // MADCTL — orientation. Previously consumed and dropped, which
                // is why a landscape sketch drew a portrait image.
                self.madctl = params[0];
            }
            // Other commands' parameters are consumed and ignored. With D/C
            // framing that is safe: they can never be read as commands.
            _ => {}
        }
    }

    fn write_pixel(&mut self, hi: u8, lo: u8) {
        let (x, y) = self.to_physical(self.cur_col, self.cur_row);
        let idx = (y * WIDTH + x) * 2;
        if x < WIDTH && idx + 1 < self.framebuffer.len() {
            self.framebuffer[idx] = hi;
            self.framebuffer[idx + 1] = lo;
        }
        // Advance column first; when column overflows, advance row (wraps within window)
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

impl SpiDevice for Ili9341 {
    /// An RGB565 TFT has no e-paper "refresh": frame memory IS the screen, so
    /// the evidence is DISPON plus how much of the buffer the firmware actually
    /// wrote. `painted_bytes` counts non-zero bytes — the SAME definition the
    /// CLI's `painted bytes=` line prints, so the two agree by construction
    /// instead of by coincidence.
    ///
    /// Moved here verbatim from the central `device_artifacts` match; the keys
    /// and their definitions are unchanged.
    fn artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        let fb = self.oriented_framebuffer();
        let painted = fb.iter().filter(|&&b| b != 0x00).count();
        let (w, h) = self.dimensions();
        // The most common non-black pixel: says WHAT was drawn, not merely that
        // something was. "6352 bytes changed" cannot be checked against a photo
        // of real silicon; "top colour 0x07E0" (RGB565 green) can.
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
        // On CS assert, reset to Idle so each new transaction starts with a command byte.
        // Real ILI9341 firmware drivers assert CS, send command (+ params + pixel data),
        // then deassert CS — each CS burst is self-contained.
        self.state = ProtoState::Idle;
    }

    fn cs_release(&mut self) {
        // State is preserved on release so a firmware driver that holds CS across
        // the entire RAMWR + pixel-data burst works correctly.
        // The next cs_select() will reset state for the following command.
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
        // With a D/C line wired, framing is the wire's, not a guess.
        if self.dc_pin.is_some() {
            if self.dc_level {
                self.dc_data(mosi);
            } else {
                self.dc_command(mosi);
            }
            return 0;
        }
        let state = self.state;
        match state {
            ProtoState::Idle => {
                self.handle_command(mosi);
            }
            ProtoState::AwaitingParams {
                cmd,
                mut params,
                mut have,
                want,
            } => {
                params[have as usize] = mosi;
                have += 1;
                if have >= want {
                    self.handle_params_complete(cmd, &params);
                    self.state = ProtoState::Idle;
                } else {
                    self.state = ProtoState::AwaitingParams {
                        cmd,
                        params,
                        have,
                        want,
                    };
                }
            }
            ProtoState::AwaitingPixelHi => {
                self.state = ProtoState::AwaitingPixelLo { hi: mosi };
            }
            ProtoState::AwaitingPixelLo { hi } => {
                self.write_pixel(hi, mosi);
                self.state = ProtoState::AwaitingPixelHi;
            }
        }
        // ILI9341 MISO is not used in write-only display mode
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
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct Ili9341Kit;
pub static ILI9341_KIT: Ili9341Kit = Ili9341Kit;

static ILI9341_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "ili9341",
    label: "ILI9341 TFT",
    summary: "240×320 RGB565 SPI TFT display.",
    detail: "Implements the cmd / RAMWR SPI protocol against an in-memory framebuffer. \
             The playground surfaces pixels through the simulator bridge so the host can render \
             the display verbatim.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[
        ConfigKey {
            name: "cs_pin",
            ty: ConfigType::Str,
            doc: "Chip-select GPIO pin (e.g. \"PA4\"). Defaults to PA4.",
        },
        ConfigKey {
            name: "dc_pin",
            ty: ConfigType::Str,
            doc: "Data/command GPIO pin (e.g. \"GPIO33\"). Strongly recommended: \
                  with it, command/data framing is read from the wire like real \
                  silicon. Without it the model must infer framing from byte \
                  values, which a real driver's init sequence desynchronises.",
        },
    ],
    labs: &[LabRef {
        board_id: "ili9341-tft-lab",
        chip: "stm32f103",
        example_dir: "ili9341-tft-lab",
        demo_elf: "demo-ili9341-tft-lab.elf",
    }],
};

impl PeripheralKit for Ili9341Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &ILI9341_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs_pin = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let dc_pin = ctx.config_str("dc_pin").map(|s| s.to_string());
        let mut dev = Ili9341::new(cs_pin);
        if let Some(dc) = dc_pin {
            // Resolving the pin to its GPIO output register is the half that
            // makes D/C real: the bus samples that register before each
            // transfer. Declaring `dc_pin` without this leaves the level stuck
            // low, so every byte frames as a command and not one pixel lands —
            // a blank panel with no error anywhere.
            let dc_src = ctx.resolve_pin_odr(&dc).ok_or_else(|| {
                anyhow::anyhow!(
                    "ili9341 '{}': D/C pin '{}' does not resolve to a driveable GPIO output. \
                     The bus latches command-vs-data from this pin's output register, so an \
                     unmapped pin leaves D/C stuck low and the display renders blank.",
                    ctx.device_id(),
                    dc,
                )
            })?;
            dev = dev.with_dc_pin(dc);
            let (odr_addr, bit) = dc_src;
            crate::peripherals::spi::SpiDevice::set_dc_source(&mut dev, odr_addr, bit);
        }
        ctx.attach_spi_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::spi::SpiDevice;

    fn send_cmd(dev: &mut Ili9341, cmd: u8) {
        dev.cs_select();
        dev.transfer(cmd);
        dev.cs_release();
    }

    fn send_cmd_params(dev: &mut Ili9341, cmd: u8, params: &[u8]) {
        dev.cs_select();
        dev.transfer(cmd);
        for &b in params {
            dev.transfer(b);
        }
        dev.cs_release();
    }

    /// Drive a D/C-wired panel the way the bus does: latch the level, then
    /// clock the byte.
    fn dc_send(dev: &mut Ili9341, dc_high: bool, bytes: &[u8]) {
        dev.set_dc_level(dc_high);
        for &b in bytes {
            dev.transfer(b);
        }
    }

    fn cmd(dev: &mut Ili9341, c: u8, params: &[u8]) {
        dc_send(dev, false, &[c]);
        if !params.is_empty() {
            dc_send(dev, true, params);
        }
    }

    // Adafruit_ILI9341's stock init table, run through the model exactly as the
    // bus delivers it. This is the sequence that used to destroy the model:
    // 0xCB is undocumented, the old value-inference treated it as taking no
    // parameters, and its second parameter 0x2C was decoded as RAMWR — opening
    // the pixel stream mid-init so SLPOUT and DISPON were written into the
    // framebuffer as pixel data and the display never came on.
    #[test]
    fn adafruit_init_sequence_leaves_the_display_on_and_the_framebuffer_clean() {
        let mut dev = Ili9341::new("PA4").with_dc_pin("PB0");
        cmd(&mut dev, 0xEF, &[0x03, 0x80, 0x02]);
        cmd(&mut dev, 0xCF, &[0x00, 0xC1, 0x30]);
        cmd(&mut dev, 0xED, &[0x64, 0x03, 0x12, 0x81]);
        cmd(&mut dev, 0xE8, &[0x85, 0x00, 0x78]);
        cmd(&mut dev, 0xCB, &[0x39, 0x2C, 0x00, 0x34, 0x02]); // the killer
        cmd(&mut dev, 0xF7, &[0x20]);
        cmd(&mut dev, 0xEA, &[0x00, 0x00]);
        cmd(&mut dev, 0xC0, &[0x23]);
        cmd(&mut dev, 0xC1, &[0x10]);
        cmd(&mut dev, 0xC5, &[0x3E, 0x28]);
        cmd(&mut dev, 0xC7, &[0x86]);
        cmd(&mut dev, 0x36, &[0x48]);
        cmd(&mut dev, 0x3A, &[0x55]);
        cmd(&mut dev, 0xB1, &[0x00, 0x18]);
        cmd(&mut dev, 0xB6, &[0x08, 0x82, 0x27]);
        cmd(&mut dev, 0x11, &[]); // SLPOUT
        cmd(&mut dev, 0x29, &[]); // DISPON

        assert!(
            dev.display_on(),
            "DISPON must be seen as a command, not swallowed as pixel data"
        );
        assert!(
            dev.framebuffer().iter().all(|&b| b == 0),
            "init must not write a single pixel"
        );
    }

    // A landscape sketch (`setRotation(1)`) sets MADCTL MV and then addresses
    // columns 0..=319. Dropping MADCTL and clamping columns to 239 folded that
    // into portrait: a correct firmware, the wrong picture, no diagnostic.
    #[test]
    fn madctl_landscape_allows_the_full_320_column_window() {
        let mut dev = Ili9341::new("PA4").with_dc_pin("PB0");
        cmd(&mut dev, 0x36, &[MADCTL_MV]);
        cmd(&mut dev, 0x2A, &[0x00, 0x00, 0x01, 0x3F]); // columns 0..=319
        assert_eq!(dev.col_end, 319, "landscape column window must not clamp");

        // The pixel at logical (319, 0) is physical (x=0, y=319) once MV swaps
        // the axes — inside the 240x320 frame memory, not discarded.
        cmd(&mut dev, 0x2B, &[0x00, 0x00, 0x00, 0x00]);
        dc_send(&mut dev, false, &[0x2C]);
        dev.cur_col = 319;
        dev.cur_row = 0;
        dc_send(&mut dev, true, &[0xF8, 0x00]);
        let idx = (319 * WIDTH) * 2;
        assert_eq!(
            (dev.framebuffer()[idx], dev.framebuffer()[idx + 1]),
            (0xF8, 0x00),
            "landscape pixel must land in frame memory"
        );
    }

    // The raw framebuffer is physical scan-out order; the canvas needs the
    // firmware's logical order. `setRotation(0)` (Adafruit's stock MADCTL 0x48 =
    // MX | BGR) mirrors horizontally, so the raw bytes read mirrored on a
    // row-major canvas. The oriented view applies MADCTL back so the host sees
    // the picture the firmware drew.
    #[test]
    fn oriented_framebuffer_applies_madctl_mirror() {
        let mut dev = Ili9341::new("PA4").with_dc_pin("PB0");
        cmd(&mut dev, 0x36, &[MADCTL_MX]); // horizontal mirror
        cmd(&mut dev, 0x2A, &[0x00, 0x00, 0x00, 0x01]); // cols 0..1
        cmd(&mut dev, 0x2B, &[0x00, 0x00, 0x00, 0x00]); // row 0
        dc_send(&mut dev, false, &[0x2C]);
        dc_send(&mut dev, true, &[0xF8, 0x00, 0x07, 0xE0]); // red then green

        // Raw framebuffer: red landed at physical x=239 (logical col 0 with MX=1),
        // green at physical x=238 (logical col 1).
        let raw = dev.framebuffer();
        let red_idx = 239 * 2;
        let green_idx = 238 * 2;
        assert_eq!((raw[red_idx], raw[red_idx + 1]), (0xF8, 0x00));
        assert_eq!((raw[green_idx], raw[green_idx + 1]), (0x07, 0xE0));

        // Oriented framebuffer: red at logical col 0, green at logical col 1.
        let oriented = dev.oriented_framebuffer();
        assert_eq!((oriented[0], oriented[1]), (0xF8, 0x00));
        assert_eq!((oriented[2], oriented[3]), (0x07, 0xE0));
    }

    #[test]
    fn oriented_framebuffer_is_identity_when_madctl_is_zero() {
        let mut dev = Ili9341::new("PA4").with_dc_pin("PB0");
        cmd(&mut dev, 0x2A, &[0x00, 0x00, 0x00, 0x01]); // cols 0..1
        cmd(&mut dev, 0x2B, &[0x00, 0x00, 0x00, 0x00]); // row 0
        dc_send(&mut dev, false, &[0x2C]);
        dc_send(&mut dev, true, &[0xF8, 0x00, 0x07, 0xE0]);

        assert_eq!(dev.oriented_framebuffer()[..4], dev.framebuffer()[..4]);
    }

    // Datasheet 8.2.2: "the Frame Memory contents are unaffected by this
    // command". Clearing on SWRESET let firmware that relies on it to blank the
    // screen pass here and show stale pixels on real hardware.
    #[test]
    fn swreset_does_not_clear_frame_memory() {
        let mut dev = Ili9341::new("PA4").with_dc_pin("PB0");
        cmd(&mut dev, 0x2A, &[0, 0, 0, 0]);
        cmd(&mut dev, 0x2B, &[0, 0, 0, 0]);
        dc_send(&mut dev, false, &[0x2C]);
        dc_send(&mut dev, true, &[0xAB, 0xCD]);
        assert_eq!((dev.framebuffer()[0], dev.framebuffer()[1]), (0xAB, 0xCD));

        cmd(&mut dev, 0x01, &[]); // SWRESET
        assert_eq!(
            (dev.framebuffer()[0], dev.framebuffer()[1]),
            (0xAB, 0xCD),
            "SWRESET must leave frame memory untouched"
        );
        assert!(!dev.display_on(), "but it does turn the display off");
    }

    // RAMWR-continue (0x3C) resumes the pixel stream WITHOUT resetting the
    // pointer — how drivers chunk a large blit. Undecoded before, so those
    // pixel bytes were interpreted as commands.
    #[test]
    fn ramwr_continue_resumes_without_restarting_the_window() {
        let mut dev = Ili9341::new("PA4").with_dc_pin("PB0");
        cmd(&mut dev, 0x2A, &[0x00, 0x00, 0x00, 0x03]);
        cmd(&mut dev, 0x2B, &[0x00, 0x00, 0x00, 0x00]);
        dc_send(&mut dev, false, &[0x2C]);
        dc_send(&mut dev, true, &[0x11, 0x11, 0x22, 0x22]);
        dc_send(&mut dev, false, &[0x3C]);
        dc_send(&mut dev, true, &[0x33, 0x33]);
        assert_eq!(
            &dev.framebuffer()[0..6],
            &[0x11, 0x11, 0x22, 0x22, 0x33, 0x33],
            "continue must append at the third pixel, not restart at the first"
        );
    }

    #[test]
    fn test_dispon_dispoff() {
        let mut dev = Ili9341::new("PA4");
        assert!(!dev.display_on());
        send_cmd(&mut dev, 0x29);
        assert!(dev.display_on());
        send_cmd(&mut dev, 0x28);
        assert!(!dev.display_on());
    }

    #[test]
    fn test_caset_paset() {
        let mut dev = Ili9341::new("PA4");
        // Set column window 10..50
        send_cmd_params(&mut dev, 0x2A, &[0x00, 0x0A, 0x00, 0x32]);
        assert_eq!(dev.col_start, 10);
        assert_eq!(dev.col_end, 50);
        // Set row window 20..100
        send_cmd_params(&mut dev, 0x2B, &[0x00, 0x14, 0x00, 0x64]);
        assert_eq!(dev.row_start, 20);
        assert_eq!(dev.row_end, 100);
    }

    #[test]
    fn test_ramwr_single_pixel() {
        let mut dev = Ili9341::new("PA4");
        // Window: col 0..239, row 0..319 (default)
        // Write one red pixel (RGB565: 0xF800)
        dev.cs_select();
        dev.transfer(0x2C); // RAMWR
        dev.transfer(0xF8); // hi
        dev.transfer(0x00); // lo
        dev.cs_release();
        let fb = dev.framebuffer();
        assert_eq!(fb[0], 0xF8, "framebuffer[0] should be pixel hi byte");
        assert_eq!(fb[1], 0x00, "framebuffer[1] should be pixel lo byte");
    }

    #[test]
    fn test_ramwr_advances_column() {
        let mut dev = Ili9341::new("PA4");
        // Write two pixels: red (0xF800) then green (0x07E0)
        dev.cs_select();
        dev.transfer(0x2C);
        // Pixel 0: red
        dev.transfer(0xF8);
        dev.transfer(0x00);
        // Pixel 1: green
        dev.transfer(0x07);
        dev.transfer(0xE0);
        dev.cs_release();
        let fb = dev.framebuffer();
        assert_eq!(fb[0], 0xF8);
        assert_eq!(fb[1], 0x00);
        assert_eq!(fb[2], 0x07);
        assert_eq!(fb[3], 0xE0);
    }

    // Datasheet §8.2.2: "the Frame Memory contents are unaffected by this
    // command". This test previously asserted the opposite, pinning a model
    // behaviour that let firmware relying on SWRESET to blank the screen pass
    // in the twin and show stale pixels on silicon. The no-D/C path must agree
    // with the D/C path (see `swreset_does_not_clear_frame_memory`) — one
    // command cannot mean two things depending on how it was framed.
    #[test]
    fn test_swreset_preserves_framebuffer_and_turns_display_off() {
        let mut dev = Ili9341::new("PA4");
        send_cmd(&mut dev, 0x29);
        dev.cs_select();
        dev.transfer(0x2C);
        dev.transfer(0xFF);
        dev.transfer(0xFF);
        dev.cs_release();
        assert_ne!(dev.framebuffer()[0], 0);

        send_cmd(&mut dev, 0x01);
        assert_eq!(
            dev.framebuffer()[0],
            0xFF,
            "SWRESET must leave frame memory untouched"
        );
        assert!(!dev.display_on(), "SWRESET does turn the display off");
    }

    #[test]
    fn test_window_wrap_on_row_overflow() {
        let mut dev = Ili9341::new("PA4");
        // Set a 2-column × 2-row window
        send_cmd_params(&mut dev, 0x2A, &[0x00, 0x00, 0x00, 0x01]); // col 0..1
        send_cmd_params(&mut dev, 0x2B, &[0x00, 0x00, 0x00, 0x01]); // row 0..1
                                                                    // Write 4 pixels (fills the 2×2 window)
        dev.cs_select();
        dev.transfer(0x2C);
        for _ in 0..4 {
            dev.transfer(0xF8); // hi
            dev.transfer(0x00); // lo
        }
        // 5th pixel wraps back to (col=0, row=0)
        dev.transfer(0x07);
        dev.transfer(0xE0);
        dev.cs_release();
        let fb = dev.framebuffer();
        // (0,0) should now be green (0x07E0), overwritten by wrap
        assert_eq!(fb[0], 0x07, "wrapped pixel hi");
        assert_eq!(fb[1], 0xE0, "wrapped pixel lo");
    }
}
