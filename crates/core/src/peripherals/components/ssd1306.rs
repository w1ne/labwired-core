// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::i2c::I2cDevice;
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

pub struct Ssd1306Kit;
pub static SSD1306_KIT: Ssd1306Kit = Ssd1306Kit;

static SSD1306_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "oled-ssd1306",
    label: "SSD1306 OLED",
    summary: "128×64 monochrome OLED display over I2C with a paged framebuffer.",
    detail: "Solomon Systech SSD1306 with the canonical 0x3C / 0x3D address pair. Tracks the \
             8-page-by-128-column framebuffer; the WASM bridge surfaces pixel state for the \
             playground's display overlay.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x3C; 0x3D selects the SA0=high variant.",
    }],
    labs: &[LabRef {
        board_id: "ssd1306-hello-lab",
        chip: "stm32f103",
        example_dir: "ssd1306-hello-lab",
        demo_elf: "demo-ssd1306-hello-lab.elf",
    }],
};

impl PeripheralKit for Ssd1306Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &SSD1306_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x3C)?;
        // attach_i2c_device works on both the STM32 I2c and the ESP32-C3
        // Esp32c3I2c controllers, so the OLED can sit on either family's bus.
        ctx.attach_i2c_device(Box::new(Ssd1306::new(address)))
    }
}

// ─── 0.91″ 128×32 variant ───────────────────────────────────────────────────

pub struct Ssd1306Oled091Kit;
pub static SSD1306_128X32_KIT: Ssd1306Oled091Kit = Ssd1306Oled091Kit;

static SSD1306_128X32_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "oled-ssd1306-128x32",
    label: "SSD1306 OLED 0.91″",
    summary: "0.91″ 128×32 monochrome OLED display over I2C with a paged framebuffer.",
    detail: "Solomon Systech SSD1306 in the 0.91-inch 128×32 form factor (4 GDDRAM pages). \
             Identical command set to the 128×64 panel — only the page count differs. \
             Canonical 0x3C / 0x3D address pair; the WASM bridge surfaces pixel state for the \
             playground's display overlay.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x3C; 0x3D selects the SA0=high variant.",
    }],
    // No lab yet: examples/ssd1306-128x32-lab has only a README + system.yaml — no demo
    // firmware/ELF is built or published. Declaring a LabRef would promise a
    // one-click demo that 404s (the playground gate rightly rejects it).
    // Re-add the LabRef when the demo firmware ships.
    labs: &[],
};

impl PeripheralKit for Ssd1306Oled091Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &SSD1306_128X32_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x3C)?;
        ctx.attach_i2c_device(Box::new(Ssd1306::new_128x32(address)))
    }
}

#[cfg(test)]
mod tests {
    use super::Ssd1306;
    use crate::peripherals::i2c::I2cDevice;

    fn command(dev: &mut Ssd1306, byte: u8) {
        dev.write(0x00);
        dev.write(byte);
        dev.stop();
    }

    #[test]
    fn split_init_and_window_commands_do_not_shift_framebuffer() {
        let mut dev = Ssd1306::new(0x3c);
        for cmd in [
            0xAE, 0xD5, 0x80, 0xA8, 0x3F, 0xD3, 0x00, 0x40, 0x8D, 0x14, 0x20, 0x00, 0xA1, 0xC8,
            0xDA, 0x12, 0x81, 0xCF, 0xD9, 0xF1, 0xDB, 0x40, 0xA4, 0xA6, 0x2E, 0xAF,
        ] {
            command(&mut dev, cmd);
        }

        for cmd in [0x21, 0, 127, 0x22, 0, 7] {
            command(&mut dev, cmd);
        }

        dev.write(0x40);
        dev.write(0xaa);
        dev.stop();

        assert_eq!(
            dev.framebuffer()[0],
            0xaa,
            "split Wire command transactions must still start data at column 0"
        );
        assert_eq!(
            dev.framebuffer()[39],
            0,
            "init command parameters must not be misread as column-nibble commands"
        );
    }

    #[test]
    fn panel_128x32_has_four_pages_and_half_size_framebuffer() {
        let dev = Ssd1306::new_128x32(0x3c);
        assert_eq!(dev.width(), 128);
        assert_eq!(dev.height(), 32, "0.91″ panel is 32 rows tall");
        assert_eq!(
            dev.framebuffer().len(),
            128 * 4,
            "128×32 GDDRAM is 4 pages (512 bytes), half the 128×64 panel"
        );
    }

    #[test]
    fn panel_128x32_writes_data_into_all_four_pages() {
        let mut dev = Ssd1306::new_128x32(0x3c);
        // Horizontal addressing across the full 128×32 window.
        for cmd in [0x20, 0x00, 0x21, 0, 127, 0x22, 0, 3] {
            command(&mut dev, cmd);
        }
        // Stream one full page-row worth of columns into the last page.
        for cmd in [0xB3, 0x00, 0x10] {
            command(&mut dev, cmd);
        }
        dev.write(0x40);
        dev.write(0xff);
        dev.stop();
        // Last page starts at byte 3*128 = 384; column 0 there must be lit.
        assert_eq!(
            dev.framebuffer()[3 * 128],
            0xff,
            "page 3 (rows 24..31) must be addressable on the 128×32 panel"
        );
    }
}
