//! ⚠️ VERBATIM COPY of the deleted `components/pcd8544.rs`. See `mod.rs`.
//! Do not edit: its only job is to disagree with the YAML model if the port moved a byte.

// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_core::peripherals::spi::SpiDevice;
use std::any::Any;

const WIDTH: usize = 84;
const BANKS: usize = 6; // 48 rows / 8 rows per bank

/// PCD8544 LCD controller model — the Nokia 5110 display (84×48, monochrome,
/// SPI).
///
/// Unlike the SSD1306 (which tags command-vs-data with an I²C control byte),
/// the PCD8544 uses a dedicated **D/C GPIO line**: when D/C is low a byte is a
/// command, when high it is display RAM data. The bus latches that pin's level
/// into `Pcd8544::set_dc_level` before each transfer (see `SpiDevice::dc_pin`).
///
/// DDRAM layout matches the SSD1306: byte at `bank * 84 + x` holds 8 vertical
/// pixels of column `x` in `bank` (bit 0 = top row of the bank). Pixel (x, y)
/// is bit `(y % 8)` of byte `ddram[(y / 8) * 84 + x]`.
#[derive(Debug, serde::Serialize)]
pub struct Pcd8544 {
    cs_pin: String,
    dc_pin: String,
    /// Whether the module's supply pins (VCC, GND) are connected in the design.
    ///
    /// ⚠️ NOT the PCD8544's own `power_down` (PD) bit below. PD is a chip mode
    /// the firmware selects over a bus that is working; this is whether the
    /// module has a rail at all. A diagram wiring only CLK/DIN/CE/DC/RST used
    /// to run the whole init and report the panel `display_on: true` with ink
    /// in DDRAM; on a bench it is blank.
    ///
    /// ⚠️ DEFAULTS TO `true`. Only an explicit `powered: false` in the compiled
    /// manifest darkens it — see `peripherals::components::supply`.
    powered: bool,
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
            // Absent supply information means "powered" — see the field's note.
            powered: true,
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
        self.powered && !self.power_down && (self.display_mode & 0x04) != 0
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
    /// Bank-addressed 1-bpp LCD, same evidence shape as the OLEDs: inked bytes
    /// and lit pixels off the real buffer, plus the display-control state that
    /// decides whether any of it is visible.
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
                "w": WIDTH,
                "h": BANKS * 8,
                "format": labwired_core::inspect::artifact_format::PCD8544_BANK,
                "generation": labwired_core::inspect::artifact_generation(fb),
                "ink_bytes": fb.iter().filter(|&&b| b != 0).count(),
                "lit_pixels": fb.iter().map(|b| b.count_ones() as usize).sum::<usize>(),
                "display_on": self.display_on(),
                // Reported so a blank panel explains itself: without it, "no
                // supply" is indistinguishable from "firmware never sent the
                // display-control command".
                "powered": self.powered,
                "inverse": self.inverse(),
            }),
            bytes: labwired_core::inspect::artifact_bytes(fb, opts),
        }]
    }

    fn transfer(&mut self, mosi_byte: u8) -> u8 {
        // THE ONE GATE THAT MAKES AN UNPOWERED PANEL BEHAVE LIKE ONE. Every
        // state change this model has — the function set, display control, the
        // X/Y cursor and every DDRAM byte — arrives through `transfer`.
        // Refusing the bus here leaves DDRAM and the addressing at their
        // power-on values by construction rather than masking them at report
        // time. See the `powered` field.
        if !self.powered {
            return 0;
        }
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

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> labwired_core::SimResult<()> {
        if bytes.len() == self.ddram.len() {
            self.ddram.copy_from_slice(bytes);
        }
        Ok(())
    }
}
