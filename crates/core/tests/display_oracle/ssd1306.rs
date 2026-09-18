//! ⚠️ VERBATIM COPY of the deleted `components/ssd1306.rs`. See `mod.rs`.
//! Do not edit: its only job is to disagree with the YAML model if the port moved a byte.

// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_core::peripherals::i2c::I2cDevice;
use std::any::Any;

const WIDTH: usize = 128;
/// GDDRAM pages for the 0.96″ 128×64 panel (64 rows / 8 rows per page).
const PAGES_128X64: usize = 8;
/// GDDRAM pages for the 0.91″ 128×32 panel (32 rows / 8 rows per page).
const PAGES_128X32: usize = 4;

/// SSD1306 OLED display controller model (I²C).
///
/// Covers both common form factors sold as bare I²C modules: the 0.96″ 128×64
/// panel (8 GDDRAM pages) and the thinner 0.91″ 128×32 panel (4 pages). The
/// command set is identical between them — only the page count differs — so the
/// same model serves both, parameterised by [`Ssd1306::pages`].
///
/// Implements the paged GDDRAM framebuffer with horizontal, vertical, and page
/// addressing modes.  Control bytes 0x00 (command stream) and 0x40 (data stream)
/// are honoured; unsupported commands are silently ignored.
#[derive(Debug, serde::Serialize)]
pub struct Ssd1306 {
    address: u8,
    /// GDDRAM page count: 8 for the 128×64 panel, 4 for the 128×32 panel.
    pages: usize,
    /// Control byte received at the start of the current I²C transaction.
    /// None = waiting for the first byte (which will be the control byte).
    control_byte: Option<u8>,
    register_address_written: bool,

    // Display state
    display_on: bool,
    /// 0 = horizontal, 1 = vertical, 2 = page addressing
    addressing_mode: u8,
    col_pointer: u8,
    page_pointer: u8,
    col_start: u8,
    col_end: u8,
    page_start: u8,
    page_end: u8,

    // Multi-byte command state machine
    pending_command: Option<u8>,
    pending_params_remaining: u8,
    pending_params: [u8; 2],

    // 128 cols × 8 pages, each byte = 8 vertical pixels
    gddram: Vec<u8>,
}

impl Default for Ssd1306 {
    fn default() -> Self {
        Self::new(0x3C)
    }
}

impl Ssd1306 {
    /// 0.96″ 128×64 panel (8 GDDRAM pages) — the default SSD1306 form factor.
    pub fn new(address: u8) -> Self {
        Self::with_pages(address, PAGES_128X64)
    }

    /// 0.91″ 128×32 panel (4 GDDRAM pages).
    pub fn new_128x32(address: u8) -> Self {
        Self::with_pages(address, PAGES_128X32)
    }

    /// Construct an SSD1306 with an explicit GDDRAM page count. `pages` is
    /// clamped to 1..=8 so a bad config can never allocate a zero-size or
    /// out-of-spec framebuffer.
    pub fn with_pages(address: u8, pages: usize) -> Self {
        let pages = pages.clamp(1, PAGES_128X64);
        Self {
            address,
            pages,
            control_byte: None,
            register_address_written: false,
            display_on: false,
            addressing_mode: 0,
            col_pointer: 0,
            page_pointer: 0,
            col_start: 0,
            col_end: (WIDTH as u8) - 1,
            page_start: 0,
            page_end: (pages as u8) - 1,
            pending_command: None,
            pending_params_remaining: 0,
            pending_params: [0; 2],
            gddram: vec![0u8; WIDTH * pages],
        }
    }

    /// Return the raw GDDRAM framebuffer (page-major, column-minor). Length is
    /// `128 × pages` — 1024 bytes for the 128×64 panel, 512 for the 128×32.
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

    /// Panel height in pixels (`pages` × 8 rows/page — 64 or 32).
    pub fn height(&self) -> usize {
        self.pages * 8
    }

    pub fn display_on(&self) -> bool {
        self.display_on
    }

    fn handle_command(&mut self, cmd: u8) {
        // Consume pending multi-byte command parameter bytes first.
        if self.pending_params_remaining > 0 {
            let idx = (2 - self.pending_params_remaining) as usize;
            self.pending_params[idx] = cmd;
            self.pending_params_remaining -= 1;
            if self.pending_params_remaining == 0 {
                self.complete_pending_command();
            }
            return;
        }

        match cmd {
            0x20 | 0x81 | 0x8D | 0xA8 | 0xD3 | 0xD5 | 0xD9 | 0xDA | 0xDB => {
                self.pending_command = Some(cmd);
                self.pending_params_remaining = 1;
            }
            0x21 => {
                self.pending_command = Some(0x21);
                self.pending_params_remaining = 2;
            }
            0x22 => {
                self.pending_command = Some(0x22);
                self.pending_params_remaining = 2;
            }
            0xAE => self.display_on = false,
            0xAF => self.display_on = true,
            // Page address (page addressing mode)
            0xB0..=0xB7 => {
                self.page_pointer = cmd & 0x07;
            }
            // Column lower nibble (page addressing mode)
            0x00..=0x0F => {
                self.col_pointer = (self.col_pointer & 0xF0) | (cmd & 0x0F);
            }
            // Column upper nibble (page addressing mode)
            0x10..=0x1F => {
                self.col_pointer = (self.col_pointer & 0x0F) | ((cmd & 0x0F) << 4);
            }
            _ => { /* unsupported — ignore */ }
        }
    }

    fn complete_pending_command(&mut self) {
        match self.pending_command.take() {
            Some(0x20) => {
                self.addressing_mode = self.pending_params[0] & 0x03;
            }
            Some(0x21) => {
                self.col_start = self.pending_params[0] & 0x7F;
                self.col_end = self.pending_params[1] & 0x7F;
                self.col_pointer = self.col_start;
            }
            Some(0x22) => {
                self.page_start = self.pending_params[0] & 0x07;
                self.page_end = self.pending_params[1] & 0x07;
                self.page_pointer = self.page_start;
            }
            _ => {}
        }
    }

    fn handle_data(&mut self, byte: u8) {
        let idx = (self.page_pointer as usize) * WIDTH + (self.col_pointer as usize);
        if idx < self.gddram.len() {
            self.gddram[idx] = byte;
        }

        // Advance pointers per addressing mode.
        match self.addressing_mode {
            0 => {
                // Horizontal: col++ wrapping into next page
                if self.col_pointer >= self.col_end {
                    self.col_pointer = self.col_start;
                    if self.page_pointer >= self.page_end {
                        self.page_pointer = self.page_start;
                    } else {
                        self.page_pointer += 1;
                    }
                } else {
                    self.col_pointer += 1;
                }
            }
            1 => {
                // Vertical: page++ wrapping into next column
                if self.page_pointer >= self.page_end {
                    self.page_pointer = self.page_start;
                    if self.col_pointer >= self.col_end {
                        self.col_pointer = self.col_start;
                    } else {
                        self.col_pointer += 1;
                    }
                } else {
                    self.page_pointer += 1;
                }
            }
            _ => {
                // Page addressing: col advances within the current page only
                if self.col_pointer < (WIDTH as u8 - 1) {
                    self.col_pointer += 1;
                }
            }
        }
    }
}

impl I2cDevice for Ssd1306 {
    /// Page-addressed 1-bpp GDDRAM: how many bytes carry ink and how many
    /// pixels are lit, both counted off the real buffer.
    ///
    /// Moved here verbatim from the central `device_artifacts` match; the keys
    /// and their definitions are unchanged, so every existing consumer of this
    /// payload keeps reading exactly what it read before.
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
                "format": labwired_core::inspect::artifact_format::SSD1306_PAGE,
                "generation": labwired_core::inspect::artifact_generation(fb),
                "ink_bytes": self.ink_bytes(),
                "lit_pixels": self.lit_pixels(),
            }),
            bytes: labwired_core::inspect::artifact_bytes(fb, opts),
        }]
    }

    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        0 // SSD1306 is write-only over I²C
    }

    /// A START (or repeated START) begins a new transaction, so the next byte
    /// is a control byte again.
    ///
    /// Resetting only in `stop()` was wrong: a driver that issues several
    /// transfers and one trailing STOP — which is what the nRF52 TWIM does,
    /// one STARTTX per transfer — kept `register_address_written` set across
    /// the whole burst. Every transfer after the first then lost its leading
    /// control byte into `handle_command`, and a `0x40` data-stream prefix
    /// silently became "set display start line". The 1 KiB framebuffer behind
    /// it was parsed as commands, which is a garbled panel and not a NACK.
    /// Real silicon re-reads the control byte after every START.
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
