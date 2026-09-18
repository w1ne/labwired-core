//! ⚠️ VERBATIM COPY of the deleted `components/sh1107.rs`. See `mod.rs`.
//! Do not edit: its only job is to disagree with the YAML model if the port moved a byte.

// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_core::peripherals::i2c::I2cDevice;
use std::any::Any;

const WIDTH: usize = 128;
const PAGES: usize = 16; // 128 rows / 8 rows per page

/// SH1107 OLED display controller model (128×128 pixels, I²C).
///
/// Implements the paged GDDRAM framebuffer with the SH1107's page- and
/// vertical-(column-)addressing modes. Control bytes 0x00 (command stream) and
/// 0x40 (data stream) are honoured; unsupported commands are silently ignored.
///
/// The SH1107 differs from the SSD1306 (`configs/devices/ssd1306.yaml`, read by
/// the `declarative_display` engine) in three ways that
/// matter for the framebuffer: 16 pages instead of 8 (128 rows), a 7-bit column
/// address (higher-nibble commands 0x10–0x17), and single-byte addressing-mode
/// selects (0x20 = page, 0x21 = vertical) rather than the SSD1306's
/// parameterised 0x20 memory-addressing-mode command. Every SH1107 multi-byte
/// command takes exactly one parameter, so there is no two-parameter
/// column/page-range command (the 0x21/0x22 pair on the SSD1306).
#[derive(Debug, serde::Serialize)]
pub struct Sh1107 {
    address: u8,
    /// Control byte received at the start of the current I²C transaction.
    /// None = waiting for the first byte (which will be the control byte).
    control_byte: Option<u8>,
    register_address_written: bool,

    // Display state
    display_on: bool,
    /// 0 = page addressing (reset default), 1 = vertical addressing.
    addressing_mode: u8,
    col_pointer: u8,
    page_pointer: u8,

    // Single-byte-parameter command state machine.
    pending_command: Option<u8>,
    pending_param_remaining: u8,

    // 128 cols × 16 pages, each byte = 8 vertical pixels.
    gddram: Vec<u8>,
}

impl Default for Sh1107 {
    fn default() -> Self {
        Self::new(0x3C)
    }
}

impl Sh1107 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            control_byte: None,
            register_address_written: false,
            display_on: false,
            addressing_mode: 0, // SH1107 powers up in page addressing mode
            col_pointer: 0,
            page_pointer: 0,
            pending_command: None,
            pending_param_remaining: 0,
            gddram: vec![0u8; WIDTH * PAGES],
        }
    }

    /// Return the raw GDDRAM framebuffer (2048 bytes: page-major, column-minor).
    ///
    /// Pixel (x, y) is bit `(y % 8)` of byte `gddram[(y / 8) * 128 + x]`.
    pub fn framebuffer(&self) -> &[u8] {
        &self.gddram
    }

    /// Count framebuffer bytes that contain at least one lit OLED pixel.
    pub fn ink_bytes(&self) -> usize {
        self.gddram.iter().filter(|b| **b != 0).count()
    }

    /// Count lit OLED pixels across the page-major GDDRAM framebuffer.
    pub fn lit_pixels(&self) -> usize {
        self.gddram.iter().map(|b| b.count_ones() as usize).sum()
    }

    /// Panel width in pixels (128).
    pub fn width(&self) -> usize {
        WIDTH
    }

    /// Panel height in pixels (128 — `PAGES` × 8 rows/page).
    pub fn height(&self) -> usize {
        PAGES * 8
    }

    pub fn display_on(&self) -> bool {
        self.display_on
    }

    fn handle_command(&mut self, cmd: u8) {
        // Consume a pending single-byte command parameter first.
        if self.pending_param_remaining > 0 {
            self.pending_param_remaining -= 1;
            // Parameters (contrast, multiplex, offset, clock divide, pre-charge,
            // VCOMH, DC-DC, display-start-line) have no effect on the modeled
            // framebuffer; we still consume them so they are not mis-read as
            // column-/page-address commands.
            if self.pending_param_remaining == 0 {
                self.pending_command = None;
            }
            return;
        }

        match cmd {
            // Single-parameter commands (parameter value ignored):
            //   0x81 contrast, 0xA8 multiplex ratio, 0xD3 display offset,
            //   0xD5 clock divide, 0xD9 pre-charge, 0xDB VCOMH deselect,
            //   0xAD DC-DC control, 0xDC display start line.
            0x81 | 0xA8 | 0xD3 | 0xD5 | 0xD9 | 0xDB | 0xAD | 0xDC => {
                self.pending_command = Some(cmd);
                self.pending_param_remaining = 1;
            }
            // Memory addressing mode — single byte on the SH1107.
            0x20 => self.addressing_mode = 0, // page addressing
            0x21 => self.addressing_mode = 1, // vertical (column) addressing
            0xAE => self.display_on = false,
            0xAF => self.display_on = true,
            // Page address (0..15).
            0xB0..=0xBF => self.page_pointer = cmd & 0x0F,
            // Column lower nibble.
            0x00..=0x0F => {
                self.col_pointer = (self.col_pointer & 0x70) | (cmd & 0x0F);
            }
            // Column higher nibble (7-bit column ⇒ commands 0x10–0x17).
            0x10..=0x17 => {
                self.col_pointer = (self.col_pointer & 0x0F) | ((cmd & 0x07) << 4);
            }
            _ => { /* unsupported — ignore */ }
        }
    }

    fn handle_data(&mut self, byte: u8) {
        let idx = (self.page_pointer as usize) * WIDTH + (self.col_pointer as usize);
        if idx < self.gddram.len() {
            self.gddram[idx] = byte;
        }

        match self.addressing_mode {
            1 => {
                // Vertical: page++ wrapping into the next column.
                if (self.page_pointer as usize) + 1 >= PAGES {
                    self.page_pointer = 0;
                    self.col_pointer = if (self.col_pointer as usize) + 1 >= WIDTH {
                        0
                    } else {
                        self.col_pointer + 1
                    };
                } else {
                    self.page_pointer += 1;
                }
            }
            _ => {
                // Page addressing: column auto-increments and wraps within the
                // current page; the page pointer is set explicitly by firmware.
                self.col_pointer = if (self.col_pointer as usize) + 1 >= WIDTH {
                    0
                } else {
                    self.col_pointer + 1
                };
            }
        }
    }
}

impl I2cDevice for Sh1107 {
    /// Same shape as the SSD1306's: a page-addressed 1-bpp OLED reports the
    /// bytes that carry ink and the pixels that are lit, read off the real
    /// GDDRAM. Only the `format` differs, because the page geometry does
    /// (16 pages of 128 columns, not 8).
    ///
    /// This panel painted a full console in the browser while `inspect`
    /// reported no artifact at all: the renderer had an accessor for it and the
    /// evidence layer had no arm for it. Both were telling the truth about
    /// their own path.
    fn artifacts(
        &self,
        id: &str,
        opts: &labwired_core::inspect::InspectOpts,
    ) -> Vec<labwired_core::inspect::Artifact> {
        let fb = self.framebuffer();
        vec![labwired_core::inspect::Artifact {
            kind: "framebuffer".to_string(),
            id: id.to_string(),
            meta: serde_json::json!({
                "w": self.width(),
                "h": self.height(),
                "format": labwired_core::inspect::artifact_format::SH1107_PAGE,
                "generation": labwired_core::inspect::artifact_generation(fb),
                "ink_bytes": self.ink_bytes(),
                "lit_pixels": self.lit_pixels(),
                "display_on": self.display_on(),
            }),
            bytes: labwired_core::inspect::artifact_bytes(fb, opts),
        }]
    }

    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        0 // SH1107 is write-only over I²C
    }

    /// A START (or repeated START) begins a new transaction, so the next byte
    /// is a control byte again. See the SSD1306 model for the failure this
    /// prevents: a driver issuing several transfers under one STOP loses the
    /// control byte on every transfer after the first, and a `0x40` data
    /// prefix turns the framebuffer behind it into a command stream.
    fn start(&mut self) {
        self.register_address_written = false;
        self.control_byte = None;
    }

    fn write(&mut self, data: u8) {
        if !self.register_address_written {
            // First byte of a transaction is the control byte.
            // 0x00 = command stream, 0x40 = data stream (bit 6 set).
            self.control_byte = Some(data);
            self.register_address_written = true;
            return;
        }
        match self.control_byte.unwrap_or(0x00) {
            0x00 => self.handle_command(data),
            _ => self.handle_data(data),
        }
    }

    fn stop(&mut self) {
        self.register_address_written = false;
        self.control_byte = None;
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
