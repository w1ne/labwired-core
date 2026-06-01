// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;

const WIDTH: usize = 84;
const BANKS: usize = 6; // 48 rows / 8 rows per bank

/// PCD8544 LCD controller model — the Nokia 5110 display (84×48, monochrome,
/// SPI).
///
/// Unlike the SSD1306 (which tags command-vs-data with an I²C control byte),
/// the PCD8544 uses a dedicated **D/C GPIO line**: when D/C is low a byte is a
/// command, when high it is display RAM data. The bus latches that pin's level
/// into [`Pcd8544::set_dc_level`] before each transfer (see [`SpiDevice::dc_pin`]).
///
/// DDRAM layout matches the SSD1306: byte at `bank * 84 + x` holds 8 vertical
/// pixels of column `x` in `bank` (bit 0 = top row of the bank). Pixel (x, y)
/// is bit `(y % 8)` of byte `ddram[(y / 8) * 84 + x]`.
#[derive(Debug, serde::Serialize)]
pub struct Pcd8544 {
    cs_pin: String,
    dc_pin: String,
    /// Latched level of the D/C line at transfer time (false = command).
    dc_level: bool,
    /// Resolved `(ODR address, bit)` of the D/C line, set by the bus at
    /// install time. `None` until resolved.
    dc_source: Option<(u64, u8)>,

    // Addressing
    x: u8,               // column, 0..=83
    y: u8,               // bank,   0..=5
    vertical_addr: bool, // V bit: true = advance bank-first, false = column-first
    extended: bool,      // H bit: true = extended instruction set selected
    power_down: bool,    // PD bit

    // Display control (basic instruction set 0b0000_1D0E)
    display_mode: u8, // bits: D (0x04) and E (0x01)

    // Extended-set config (stored for fidelity; no visual effect modeled)
    vop: u8,  // contrast
    bias: u8, // bias system
    temp: u8, // temperature coefficient

    // 84 cols × 6 banks, each byte = 8 vertical pixels
    ddram: Vec<u8>,
}

impl Default for Pcd8544 {
    fn default() -> Self {
        Self::new("PB6".to_string(), "PC7".to_string())
    }
}

impl Pcd8544 {
    /// `cs_pin` is the chip-select label, `dc_pin` the data/command label
    /// (e.g. "PC7"). Both are GPIO labels the bus resolves to drive D/C
    /// observation; CS is informational (v1 SPI routing broadcasts).
    pub fn new(cs_pin: String, dc_pin: String) -> Self {
        Self {
            cs_pin,
            dc_pin,
            dc_level: false,
            dc_source: None,
            x: 0,
            y: 0,
            vertical_addr: false,
            extended: false,
            power_down: false,
            display_mode: 0x04, // D=1, E=0 → normal
            vop: 0,
            bias: 0,
            temp: 0,
            ddram: vec![0u8; WIDTH * BANKS],
        }
    }

    /// Raw DDRAM framebuffer (504 bytes: bank-major, column-minor).
    pub fn framebuffer(&self) -> &[u8] {
        &self.ddram
    }

    /// True when the panel is showing RAM (powered up, display mode = normal
    /// or inverse). The renderer can use this to blank the screen.
    pub fn display_on(&self) -> bool {
        !self.power_down && (self.display_mode & 0x04) != 0
    }

    /// True when the display is in inverse-video mode (DE = 0b01).
    pub fn inverse(&self) -> bool {
        (self.display_mode & 0x05) == 0x05
    }

    fn handle_command(&mut self, cmd: u8) {
        // Function set: 0b0010_0PVH — selects PD / vertical-addressing / H.
        if cmd & 0xF8 == 0x20 {
            self.power_down = (cmd & 0x04) != 0;
            self.vertical_addr = (cmd & 0x02) != 0;
            self.extended = (cmd & 0x01) != 0;
            return;
        }

        if self.extended {
            // Extended instruction set (H = 1).
            if cmd & 0x80 == 0x80 {
                self.vop = cmd & 0x7F; // Set Vop (contrast)
            } else if cmd & 0xF8 == 0x10 {
                self.bias = cmd & 0x07; // Bias system
            } else if cmd & 0xFC == 0x04 {
                self.temp = cmd & 0x03; // Temperature control
            }
        } else {
            // Basic instruction set (H = 0).
            if cmd & 0x80 == 0x80 {
                // Set X address (column), 0..=83.
                let x = cmd & 0x7F;
                self.x = if (x as usize) < WIDTH { x } else { 0 };
            } else if cmd & 0xF8 == 0x40 {
                // Set Y address (bank), 0..=5.
                let y = cmd & 0x07;
                self.y = if (y as usize) < BANKS { y } else { 0 };
            } else if cmd & 0xF8 == 0x08 {
                // Display control: bits D (0x04) and E (0x01).
                self.display_mode = cmd & 0x05;
            }
            // Other basic commands (NOP 0x00, etc.) ignored.
        }
    }

    fn handle_data(&mut self, byte: u8) {
        let idx = (self.y as usize) * WIDTH + (self.x as usize);
        if idx < self.ddram.len() {
            self.ddram[idx] = byte;
        }

        // Auto-advance the address pointer per the V bit.
        if self.vertical_addr {
            // Bank-first.
            if (self.y as usize) >= BANKS - 1 {
                self.y = 0;
                self.x = if (self.x as usize) >= WIDTH - 1 {
                    0
                } else {
                    self.x + 1
                };
            } else {
                self.y += 1;
            }
        } else {
            // Column-first (default).
            if (self.x as usize) >= WIDTH - 1 {
                self.x = 0;
                self.y = if (self.y as usize) >= BANKS - 1 {
                    0
                } else {
                    self.y + 1
                };
            } else {
                self.x += 1;
            }
        }
    }
}

impl SpiDevice for Pcd8544 {
    fn transfer(&mut self, mosi_byte: u8) -> u8 {
        if self.dc_level {
            self.handle_data(mosi_byte);
        } else {
            self.handle_command(mosi_byte);
        }
        0 // PCD8544 has no MISO line — write-only.
    }

    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn dc_pin(&self) -> Option<&str> {
        Some(&self.dc_pin)
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

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        self.ddram.clone()
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> crate::SimResult<()> {
        if bytes.len() == self.ddram.len() {
            self.ddram.copy_from_slice(bytes);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive a realistic init + pixel-write sequence and assert the D/C line
    /// correctly steers command vs data, the addressing advances, and the
    /// framebuffer reflects what was written.
    #[test]
    fn init_and_pixel_write() {
        let mut lcd = Pcd8544::new("PB6".into(), "PC7".into());

        // ── Init: all commands (D/C low) ──
        lcd.set_dc_level(false);
        lcd.transfer(0x21); // function set: extended (H=1)
        lcd.transfer(0xBF); // set Vop (contrast) = 0x3F
        lcd.transfer(0x14); // bias = 0x04
        lcd.transfer(0x20); // function set: basic (H=0)
        lcd.transfer(0x0C); // display control: normal (D=1, E=0)

        assert!(lcd.display_on(), "normal mode + powered → display on");
        assert!(!lcd.inverse());
        assert_eq!(lcd.vop, 0x3F, "extended-set Vop latched");
        assert_eq!(lcd.bias, 0x04, "bias latched");

        // Position the cursor at column 5, bank 2.
        lcd.transfer(0x40 | 2); // set Y (bank) = 2
        lcd.transfer(0x80 | 5); // set X (col) = 5
        assert_eq!((lcd.x, lcd.y), (5, 2));

        // ── Data phase (D/C high) ──
        lcd.set_dc_level(true);
        lcd.transfer(0xAA);
        lcd.transfer(0x55);

        // First byte landed at bank 2, col 5; column auto-advanced.
        assert_eq!(lcd.framebuffer()[2 * WIDTH + 5], 0xAA);
        assert_eq!(lcd.framebuffer()[2 * WIDTH + 6], 0x55);
        assert_eq!((lcd.x, lcd.y), (7, 2), "column advanced twice");

        // A data byte must NOT be decoded as a command even if it looks like
        // one (0x20 == function-set opcode) — the D/C line is what matters.
        lcd.transfer(0x20);
        assert_eq!(lcd.framebuffer()[2 * WIDTH + 7], 0x20);
        assert!(!lcd.extended, "0x20 as data must not flip the H bit");
    }

    /// End-to-end keystone test: the bus latches the D/C level from a real
    /// GPIO output pin (PC7 → GPIOC ODR bit 7) before each SPI transfer, so
    /// the *same* SPI byte stream is decoded as command or data purely by the
    /// pin level the firmware drives.
    #[test]
    fn dc_latched_from_gpio_through_bus() {
        use crate::bus::SystemBus;
        use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
        use crate::peripherals::spi::{Spi, SpiDevice};

        const GPIOC: u64 = 0x4800_0800; // stm32v2 GPIOC
        const SPI1: u64 = 0x4001_3000;
        const ODR: u64 = 0x14;
        const BSRR: u64 = 0x18;
        const DR: u64 = 0x0C;
        const CR1: u64 = 0x00;

        let mut bus = SystemBus::empty();
        bus.add_peripheral(
            "gpioc",
            GPIOC,
            0x400,
            None,
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
        );

        let mut spi = Spi::new();
        let mut lcd = Pcd8544::new("PB6".into(), "PC7".into());
        // D/C resolves to GPIOC ODR bit 7 (PC7) — exactly what bus install does.
        SpiDevice::set_dc_source(&mut lcd, GPIOC + ODR, 7);
        spi.attach(Box::new(lcd));
        bus.add_peripheral("spi1", SPI1, 0x400, None, Box::new(spi));

        // Enable SPE so DR writes kick off transfers.
        bus.write_u16(SPI1 + CR1, 1 << 6).unwrap();

        // D/C low (PC7=0, the reset state): two command bytes position the
        // cursor at bank 2, column 5.
        bus.write_u16(SPI1 + DR, 0x40 | 2).unwrap(); // set Y (bank) = 2
        bus.write_u16(SPI1 + DR, 0x80 | 5).unwrap(); // set X (col)  = 5

        // Drive PC7 high (D/C = data) via BSRR, then stream a data byte.
        bus.write_u32(GPIOC + BSRR, 1 << 7).unwrap();
        bus.write_u16(SPI1 + DR, 0xAB).unwrap();

        // Inspect the attached panel's framebuffer through the bus.
        let idx = bus.find_peripheral_index_by_name("spi1").unwrap();
        let spi = bus.peripherals[idx]
            .dev
            .as_any()
            .unwrap()
            .downcast_ref::<Spi>()
            .unwrap();
        let lcd = spi.attached_devices[0]
            .as_any()
            .unwrap()
            .downcast_ref::<Pcd8544>()
            .unwrap();

        assert_eq!(
            lcd.framebuffer()[2 * WIDTH + 5],
            0xAB,
            "data byte landed at the command-addressed cursor (bank 2, col 5)"
        );
        // And the commands were NOT misread as data: only one byte was written.
        assert_eq!(lcd.framebuffer()[2 * WIDTH + 6], 0x00);
    }

    /// Column-pointer wraps to the next bank at the right edge.
    #[test]
    fn column_wrap_advances_bank() {
        let mut lcd = Pcd8544::new("PB6".into(), "PC7".into());
        lcd.set_dc_level(false);
        lcd.transfer(0x40 | 0); // bank 0
        lcd.transfer(0x80 | (WIDTH as u8 - 1)); // last column (83)
        lcd.set_dc_level(true);
        lcd.transfer(0x11);
        assert_eq!(lcd.framebuffer()[83], 0x11);
        assert_eq!((lcd.x, lcd.y), (0, 1), "wrapped to col 0, bank 1");
    }
}
