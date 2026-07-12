// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::i2c::I2cDevice;
use std::any::Any;

const WIDTH: usize = 128;
const PAGES: usize = 16; // 128 rows / 8 rows per page

/// SH1107 OLED display controller model (128×128 pixels, I²C).
///
/// Implements the paged GDDRAM framebuffer with the SH1107's page- and
/// vertical-(column-)addressing modes. Control bytes 0x00 (command stream) and
/// 0x40 (data stream) are honoured; unsupported commands are silently ignored.
///
/// The SH1107 differs from the [`super::ssd1306::Ssd1306`] in three ways that
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
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        0 // SH1107 is write-only over I²C
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

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Sh1107Kit;
pub static SH1107_KIT: Sh1107Kit = Sh1107Kit;

static SH1107_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "oled-sh1107",
    label: "SH1107 OLED",
    summary: "128×128 monochrome OLED display over I2C with a paged framebuffer.",
    detail: "Sino Wealth SH1107 128×128 OLED (e.g. the 1.5\" GME128128-01-IIC module) on the \
             canonical 0x3C / 0x3D address pair. Tracks the 16-page-by-128-column framebuffer; \
             the WASM bridge surfaces pixel state for the playground's display overlay.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x3C; 0x3D selects the SA0=high variant.",
    }],
    labs: &[],
};

impl PeripheralKit for Sh1107Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &SH1107_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x3C)?;
        // attach_i2c_device works on both the STM32 I2c and the ESP32-C3
        // Esp32c3I2c controllers, so the OLED can sit on either family's bus.
        ctx.attach_i2c_device(Box::new(Sh1107::new(address)))
    }
}

#[cfg(test)]
mod tests {
    use super::{Sh1107, PAGES, WIDTH};
    use crate::peripherals::i2c::I2cDevice;

    fn command(dev: &mut Sh1107, byte: u8) {
        dev.write(0x00);
        dev.write(byte);
        dev.stop();
    }

    fn set_page(dev: &mut Sh1107, page: u8) {
        command(dev, 0xB0 | (page & 0x0F));
    }

    fn set_column(dev: &mut Sh1107, col: u8) {
        command(dev, col & 0x0F); // lower nibble
        command(dev, 0x10 | (col >> 4)); // higher nibble
    }

    #[test]
    fn init_params_do_not_corrupt_framebuffer_and_data_lands_at_origin() {
        let mut dev = Sh1107::new(0x3c);
        // A representative SH1107 init stream: several single-parameter commands
        // whose parameters must be consumed, not mis-read as column/page moves.
        for cmd in [
            0xAE, 0xD5, 0x51, 0x20, 0x81, 0x4F, 0xAD, 0x8A, 0xA8, 0x7F, 0xD3, 0x60, 0xDC, 0x00,
            0xD9, 0x22, 0xDB, 0x35, 0xA4, 0xA6, 0xAF,
        ] {
            command(&mut dev, cmd);
        }

        set_page(&mut dev, 0);
        set_column(&mut dev, 0);
        dev.write(0x40);
        dev.write(0xAA);
        dev.stop();

        assert_eq!(
            dev.framebuffer()[0],
            0xAA,
            "data must start at page 0 / column 0 after the init stream"
        );
        assert!(dev.display_on(), "0xAF must turn the panel on");
    }

    #[test]
    fn addresses_the_full_128_rows_and_128_columns() {
        let mut dev = Sh1107::new(0x3c);
        // Bottom page (15) and last column (127) — only reachable on a 16-page,
        // 7-bit-column SH1107, not an 8-page SSD1306.
        set_page(&mut dev, 15);
        set_column(&mut dev, 127);
        dev.write(0x40);
        dev.write(0xFF);
        dev.stop();

        let idx = 15 * WIDTH + 127;
        assert_eq!(idx, PAGES * WIDTH - 1);
        assert_eq!(
            dev.framebuffer()[idx],
            0xFF,
            "page 15 / column 127 must be writable"
        );
    }

    #[test]
    fn page_addressing_auto_increments_column() {
        let mut dev = Sh1107::new(0x3c);
        command(&mut dev, 0x20); // page addressing mode
        set_page(&mut dev, 2);
        set_column(&mut dev, 10);
        dev.write(0x40);
        for b in [0x01u8, 0x02, 0x03] {
            dev.write(b);
        }
        dev.stop();
        assert_eq!(
            &dev.framebuffer()[2 * WIDTH + 10..2 * WIDTH + 13],
            &[0x01, 0x02, 0x03]
        );
    }

    #[test]
    fn vertical_addressing_auto_increments_page() {
        let mut dev = Sh1107::new(0x3c);
        command(&mut dev, 0x21); // vertical addressing mode
        set_page(&mut dev, 0);
        set_column(&mut dev, 5);
        dev.write(0x40);
        for b in [0x11u8, 0x22, 0x33] {
            dev.write(b);
        }
        dev.stop();
        assert_eq!(dev.framebuffer()[5], 0x11);
        assert_eq!(dev.framebuffer()[WIDTH + 5], 0x22);
        assert_eq!(dev.framebuffer()[2 * WIDTH + 5], 0x33);
    }
}
