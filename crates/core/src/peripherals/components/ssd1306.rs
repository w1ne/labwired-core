// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::i2c::I2cDevice;
use std::any::Any;

const WIDTH: usize = 128;
const PAGES: usize = 8; // 64 rows / 8 rows per page

/// SSD1306 OLED display controller model (128×64 pixels, I²C).
///
/// Implements the paged GDDRAM framebuffer with horizontal, vertical, and page
/// addressing modes.  Control bytes 0x00 (command stream) and 0x40 (data stream)
/// are honoured; unsupported commands are silently ignored.
#[derive(Debug, serde::Serialize)]
pub struct Ssd1306 {
    address: u8,
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
    pub fn new(address: u8) -> Self {
        Self {
            address,
            control_byte: None,
            register_address_written: false,
            display_on: false,
            addressing_mode: 0,
            col_pointer: 0,
            page_pointer: 0,
            col_start: 0,
            col_end: (WIDTH as u8) - 1,
            page_start: 0,
            page_end: (PAGES as u8) - 1,
            pending_command: None,
            pending_params_remaining: 0,
            pending_params: [0; 2],
            gddram: vec![0u8; WIDTH * PAGES],
        }
    }

    /// Return the raw GDDRAM framebuffer (1024 bytes: page-major, column-minor).
    ///
    /// Pixel (x, y) is bit `(y % 8)` of byte `gddram[(y / 8) * 128 + x]`.
    pub fn framebuffer(&self) -> &[u8] {
        &self.gddram
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
            0x20 => {
                self.pending_command = Some(0x20);
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
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        0 // SSD1306 is write-only over I²C
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
        self.pending_command = None;
        self.pending_params_remaining = 0;
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
