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

/// Protocol state machine for the ILI9341 SPI command/data stream.
///
/// The ILI9341 uses a separate D/C GPIO pin to distinguish command vs data bytes.
/// Since the simulator cannot observe arbitrary GPIO state, we use a self-contained
/// state machine driven by command semantics: each command specifies how many parameter
/// bytes it needs, and RAMWR (0x2C) opens a pixel data stream.
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
        }
    }

    /// Return the raw RGB565 framebuffer (153,600 bytes: row-major, 2 bytes per pixel).
    ///
    /// Pixel (col, row) occupies bytes at index `(row * 240 + col) * 2` (high byte)
    /// and `(row * 240 + col) * 2 + 1` (low byte).
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    pub fn display_on(&self) -> bool {
        self.display_on
    }

    pub fn dimensions(&self) -> (usize, usize) {
        (WIDTH, HEIGHT)
    }

    // ---- Internal command dispatch ----

    fn handle_command(&mut self, cmd: u8) {
        match cmd {
            0x01 => {
                // SWRESET — software reset: clear framebuffer and reset window
                self.framebuffer.iter_mut().for_each(|b| *b = 0);
                self.col_start = 0;
                self.col_end = (WIDTH as u16) - 1;
                self.row_start = 0;
                self.row_end = (HEIGHT as u16) - 1;
                self.display_on = false;
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
            0xB1 | 0xB6 => {
                // FRMCTR / DFUNCTR — variable, consume up to 3 params; ignored
                self.state = ProtoState::AwaitingParams {
                    cmd,
                    params: [0; 4],
                    have: 0,
                    want: 3,
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
                // CASET: set column window
                let start = ((params[0] as u16) << 8) | (params[1] as u16);
                let end = ((params[2] as u16) << 8) | (params[3] as u16);
                self.col_start = start.min((WIDTH as u16) - 1);
                self.col_end = end.min((WIDTH as u16) - 1);
            }
            0x2B => {
                // PASET: set row window
                let start = ((params[0] as u16) << 8) | (params[1] as u16);
                let end = ((params[2] as u16) << 8) | (params[3] as u16);
                self.row_start = start.min((HEIGHT as u16) - 1);
                self.row_end = end.min((HEIGHT as u16) - 1);
            }
            // All other commands' parameters are consumed silently
            _ => {}
        }
    }

    fn write_pixel(&mut self, hi: u8, lo: u8) {
        let idx = (self.cur_row as usize * WIDTH + self.cur_col as usize) * 2;
        if idx + 1 < self.framebuffer.len() {
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

    fn transfer(&mut self, mosi: u8) -> u8 {
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
    config_keys: &[ConfigKey {
        name: "cs_pin",
        ty: ConfigType::Str,
        doc: "Chip-select GPIO pin (e.g. \"PA4\"). Defaults to PA4.",
    }],
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
        ctx.attach_spi_device(Box::new(Ili9341::new(cs_pin)))?;
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

    #[test]
    fn test_swreset_clears_framebuffer() {
        let mut dev = Ili9341::new("PA4");
        // Write a pixel
        dev.cs_select();
        dev.transfer(0x2C);
        dev.transfer(0xFF);
        dev.transfer(0xFF);
        dev.cs_release();
        assert_ne!(dev.framebuffer()[0], 0);
        // Software reset
        send_cmd(&mut dev, 0x01);
        assert_eq!(dev.framebuffer()[0], 0, "SWRESET should clear framebuffer");
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
