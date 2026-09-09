// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! MAX7219 8×8 LED matrix driver.
//!
//! The MAX7219 is a serially-interfaced common-cathode display driver. The host
//! drives it exactly like an SPI slave: `DIN`→MOSI, `CLK`→SCK, `CS`/`LOAD`→chip
//! select. Every transaction is a **16-bit register write**, MSB first: an
//! address byte followed by a data byte, latched on the rising edge of `CS`.
//!
//! Unlike the chained-74HC595 module ([`super::hc595_7seg`]) there is no need to
//! auto-detect which byte is which — the address is explicit — so the frame
//! decode is a straight match on the address nibble.
//!
//! Register map (datasheet Table 2):
//!
//! | Address | Register     |
//! |---------|--------------|
//! | `0x00`  | No-op (used to shift through cascaded drivers) |
//! | `0x01`…`0x08` | Digit 0…7 — one byte per matrix row |
//! | `0x09`  | Decode mode  |
//! | `0x0A`  | Intensity    |
//! | `0x0B`  | Scan limit   |
//! | `0x0C`  | Shutdown (0 = shutdown, 1 = normal operation) |
//! | `0x0F`  | Display test (1 = all LEDs on) |
//!
//! On an 8×8 matrix module the eight digit registers are the eight rows, so the
//! digit RAM *is* the framebuffer: [`Max7219::framebuffer`] hands back the eight
//! row bytes the panel is lighting, honouring shutdown and display-test the way
//! real silicon blanks or floods the display.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;

/// Rows on the matrix — equivalently, the number of digit registers driven.
const ROWS: usize = 8;

// Register addresses (datasheet Table 2).
const REG_NOOP: u8 = 0x00;
const REG_DIGIT0: u8 = 0x01;
const REG_DIGIT7: u8 = 0x08;
const REG_DECODE_MODE: u8 = 0x09;
const REG_INTENSITY: u8 = 0x0A;
const REG_SCAN_LIMIT: u8 = 0x0B;
const REG_SHUTDOWN: u8 = 0x0C;
const REG_DISPLAY_TEST: u8 = 0x0F;

/// Simulated MAX7219-driven 8×8 LED matrix.
#[derive(Debug, serde::Serialize)]
pub struct Max7219 {
    /// `CS`/`LOAD` line, wired to the GPIO used as SPI chip-select.
    cs_pin: String,
    /// Whether the module's supply pins (VCC, GND) are connected in the design.
    ///
    /// ⚠️ NOT the MAX7219's own `shutdown` register below. Shutdown is a mode
    /// the firmware selects over a bus that works; this is whether the module
    /// has a rail at all. A diagram wiring only DIN/CLK/CS used to run the
    /// whole init and report lit LEDs; on a bench the matrix is dark.
    ///
    /// ⚠️ DEFAULTS TO `true`. Only an explicit `powered: false` in the compiled
    /// manifest darkens it — see [`crate::peripherals::components::supply`].
    powered: bool,
    /// Two-byte shift accumulator for the 16-bit write currently clocking in.
    shift: [u8; 2],
    /// Number of bytes clocked into `shift` since the last latch.
    shift_len: usize,
    /// Digit RAM: one byte per row, index 0 = digit 0 (register `0x01`).
    framebuffer: [u8; ROWS],
    /// Decode-mode register (`0x09`). `0x00` = no decode, as an 8×8 matrix needs.
    decode_mode: u8,
    /// Intensity register (`0x0A`), 0…15 in the low nibble.
    intensity: u8,
    /// Scan-limit register (`0x0B`): how many digits are multiplexed, 0…7.
    scan_limit: u8,
    /// True while the driver is in shutdown mode (register `0x0C` data bit 0 clear).
    shutdown: bool,
    /// True while display test is active (register `0x0F` data bit 0 set).
    display_test: bool,
}

impl Max7219 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            // Absent supply information means "powered" — see the field's note.
            powered: true,
            shift: [0; 2],
            shift_len: 0,
            framebuffer: [0; ROWS],
            // Power-on defaults per datasheet: all registers cleared, which
            // leaves the part shut down with display test off.
            decode_mode: 0,
            intensity: 0,
            scan_limit: 0,
            shutdown: true,
            display_test: false,
        }
    }

    /// The eight row bytes the panel is currently lighting, row 0 first.
    ///
    /// Display test floods every LED and shutdown blanks the panel — both
    /// without disturbing digit RAM, so the stored rows reappear untouched when
    /// the mode is cleared. Reporting the raw RAM in those states would render a
    /// picture the real panel is not showing.
    pub fn framebuffer(&self) -> [u8; ROWS] {
        // No supply, no LEDs — ahead of display test, which on silicon
        // overrides shutdown but cannot override the absence of a rail.
        if !self.powered {
            return [0x00; ROWS];
        }
        if self.display_test {
            // Display test overrides shutdown (datasheet: "display-test mode
            // overrides shutdown mode").
            return [0xFF; ROWS];
        }
        if self.shutdown {
            return [0x00; ROWS];
        }
        self.framebuffer
    }

    /// Digit RAM as written by the firmware, ignoring shutdown / display test.
    pub fn digit_ram(&self) -> [u8; ROWS] {
        self.framebuffer
    }

    /// Latched decode-mode register (`0x09`).
    pub fn decode_mode(&self) -> u8 {
        self.decode_mode
    }

    /// Latched intensity register (`0x0A`), 0…15 in the low nibble.
    pub fn intensity(&self) -> u8 {
        self.intensity
    }

    /// Latched scan-limit register (`0x0B`), 0…7.
    pub fn scan_limit(&self) -> u8 {
        self.scan_limit
    }

    /// True while the driver is in shutdown mode. A module with no supply is
    /// reported as shut down as well: `transfer` refuses the bus, so the part
    /// stays at its power-on-shutdown default by construction.
    pub fn is_shutdown(&self) -> bool {
        self.shutdown
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

    /// True while display test is active.
    pub fn is_display_test(&self) -> bool {
        self.display_test
    }

    /// Apply a fully clocked-in 16-bit write: `shift[0]` is the address byte,
    /// `shift[1]` the data byte. Addresses outside the documented map are
    /// ignored, matching a part that simply decodes nothing for them.
    fn latch_frame(&mut self) {
        let (addr, data) = (self.shift[0], self.shift[1]);
        // Only the low nibble of the address byte is decoded; the upper bits are
        // don't-care on real silicon.
        match addr & 0x0F {
            REG_NOOP => {}
            REG_DIGIT0..=REG_DIGIT7 => {
                self.framebuffer[(addr & 0x0F) as usize - 1] = data;
            }
            REG_DECODE_MODE => self.decode_mode = data,
            REG_INTENSITY => self.intensity = data,
            REG_SCAN_LIMIT => self.scan_limit = data,
            // Data bit 0: 0 = shutdown, 1 = normal operation.
            REG_SHUTDOWN => self.shutdown = data & 1 == 0,
            REG_DISPLAY_TEST => self.display_test = data & 1 != 0,
            _ => {}
        }
    }

    fn push_byte(&mut self, byte: u8) {
        let idx = self.shift_len % 2;
        self.shift[idx] = byte;
        self.shift_len += 1;
        if self.shift_len == 2 {
            self.latch_frame();
            self.shift_len = 0;
        }
    }
}

impl SpiDevice for Max7219 {
    /// An 8x8 LED matrix: one byte per row, one bit per LED. `lit_pixels` is
    /// the count of set bits in the ROWS the scan limit actually drives —
    /// shutdown is reported rather than folded in, because a module holding a
    /// pattern while shut down is a different finding from a blank one.
    fn artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        let fb = self.framebuffer();
        vec![crate::inspect::Artifact {
            kind: "framebuffer".to_string(),
            id: id.to_string(),
            meta: serde_json::json!({
                "w": 8,
                "h": fb.len(),
                "format": crate::inspect::artifact_format::MAX7219_ROWS,
                "generation": crate::inspect::artifact_generation(&fb),
                "ink_bytes": fb.iter().filter(|&&b| b != 0).count(),
                "lit_pixels": fb.iter().map(|b| b.count_ones() as usize).sum::<usize>(),
                "shutdown": self.is_shutdown(),
                // Reported so a dark matrix explains itself: without it, "no
                // supply" is indistinguishable from "firmware never left
                // shutdown".
                "powered": self.powered,
                "intensity": self.intensity(),
                "scan_limit": self.scan_limit(),
            }),
            bytes: crate::inspect::artifact_bytes(&fb, opts),
        }]
    }

    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        // CS asserted → start of a fresh 16-bit write. Discard any partial frame
        // so a byte-misaligned burst can't shift the address/data pairing.
        self.shift_len = 0;
    }

    fn cs_release(&mut self) {
        // Each 16-bit write is latched in `push_byte`; nothing extra to do on
        // release. Reset the partial counter for the next assertion.
        self.shift_len = 0;
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        // THE ONE GATE THAT MAKES AN UNPOWERED MODULE BEHAVE LIKE ONE. Every
        // state change this model has — digit RAM, intensity, scan limit,
        // shutdown, display test — arrives through `transfer` as a 16-bit
        // register write. Refusing the bus here leaves the part at its
        // power-on defaults (shut down, RAM clear) by construction rather than
        // masking the readback at report time.
        if !self.powered {
            return 0;
        }
        self.push_byte(mosi);
        // DOUT is the delayed DIN used for cascading; a single (uncascaded)
        // module presents nothing meaningful on MISO.
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
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Max7219Kit;
pub static MAX7219_KIT: Max7219Kit = Max7219Kit;

static MAX7219_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "led-matrix",
    label: "MAX7219 8×8 LED Matrix",
    summary: "8×8 LED matrix driven by a MAX7219 serial display driver over SPI.",
    detail: "The common 8×8 dot-matrix module built around a MAX7219 common-cathode display \
             driver. Firmware clocks 16 bits per transaction — an address byte then a data byte \
             — and pulses CS/LOAD to latch. Digit registers 0x01–0x08 are the eight matrix rows, \
             so digit RAM is the framebuffer; the model also tracks decode mode, intensity, scan \
             limit, shutdown and display test, blanking or flooding the readback the way the real \
             part drives the panel.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[
        ConfigKey {
            name: "cs_pin",
            ty: ConfigType::Str,
            doc: "CS/LOAD GPIO pin, wired as SPI chip-select (e.g. \"PA4\"). Defaults to PA4.",
        },
        crate::peripherals::components::supply::POWERED_CONFIG_KEY,
    ],
    // No lab yet: no demo firmware/ELF is built or published for this module.
    // Declaring a LabRef would promise a one-click demo that 404s.
    labs: &[],
};

impl PeripheralKit for Max7219Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &MAX7219_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs_pin = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        // Supply state. `Some(false)` is the only value that changes anything;
        // an absent key means powered. See `components::supply`.
        ctx.attach_spi_device(Box::new(Max7219::new(cs_pin).with_powered(
            crate::peripherals::components::supply::powered_from_config(ctx),
        )))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::spi::SpiDevice;

    /// Clock one 16-bit register write the way a driver does: assert CS, shift
    /// address then data, release CS.
    fn write_reg(dev: &mut Max7219, addr: u8, data: u8) {
        dev.cs_select();
        dev.transfer(addr);
        dev.transfer(data);
        dev.cs_release();
    }

    /// Bring the part out of its power-on shutdown so `framebuffer()` reports
    /// digit RAM rather than a blanked panel.
    fn power_on(dev: &mut Max7219) {
        write_reg(dev, REG_SHUTDOWN, 0x01);
    }

    #[test]
    fn row_register_writes_framebuffer_row() {
        let mut dev = Max7219::new("PA4");
        power_on(&mut dev);
        // Digit register 0x03 is the third row → framebuffer index 2.
        write_reg(&mut dev, 0x03, 0b1010_0101);
        assert_eq!(dev.framebuffer()[2], 0b1010_0101);
        // No other row disturbed.
        assert_eq!(dev.framebuffer()[0], 0);
        assert_eq!(dev.framebuffer()[7], 0);
    }

    #[test]
    fn all_eight_row_registers_map_in_order() {
        let mut dev = Max7219::new("PA4");
        power_on(&mut dev);
        for row in 0..ROWS {
            write_reg(&mut dev, REG_DIGIT0 + row as u8, 0x10 + row as u8);
        }
        let fb = dev.framebuffer();
        for (row, byte) in fb.iter().enumerate() {
            assert_eq!(*byte, 0x10 + row as u8, "row {row} mismatched");
        }
    }

    #[test]
    fn shutdown_blanks_readback_but_retains_digit_ram() {
        let mut dev = Max7219::new("PA4");
        power_on(&mut dev);
        write_reg(&mut dev, 0x01, 0xAA);
        write_reg(&mut dev, 0x08, 0x55);
        assert_eq!(dev.framebuffer()[0], 0xAA);

        // Shutdown: data bit 0 clear.
        write_reg(&mut dev, REG_SHUTDOWN, 0x00);
        assert!(dev.is_shutdown());
        assert_eq!(dev.framebuffer(), [0u8; ROWS], "panel must read blank");
        // The underlying RAM is untouched.
        assert_eq!(dev.digit_ram()[0], 0xAA);
        assert_eq!(dev.digit_ram()[7], 0x55);

        // Back to normal operation: the stored rows reappear.
        write_reg(&mut dev, REG_SHUTDOWN, 0x01);
        assert!(!dev.is_shutdown());
        assert_eq!(dev.framebuffer()[0], 0xAA);
        assert_eq!(dev.framebuffer()[7], 0x55);
    }

    #[test]
    fn display_test_lights_every_led() {
        let mut dev = Max7219::new("PA4");
        power_on(&mut dev);
        write_reg(&mut dev, 0x02, 0x01);

        write_reg(&mut dev, REG_DISPLAY_TEST, 0x01);
        assert!(dev.is_display_test());
        assert_eq!(dev.framebuffer(), [0xFF; ROWS]);

        // Display test overrides shutdown too.
        write_reg(&mut dev, REG_SHUTDOWN, 0x00);
        assert_eq!(dev.framebuffer(), [0xFF; ROWS]);

        // Clearing display test restores the normal (here: shut down) view.
        write_reg(&mut dev, REG_DISPLAY_TEST, 0x00);
        assert_eq!(dev.framebuffer(), [0u8; ROWS]);
    }

    #[test]
    fn frame_latches_once_per_two_bytes_and_cs_resets_partials() {
        let mut dev = Max7219::new("PA4");
        power_on(&mut dev);

        // A single byte is not a frame — nothing latches yet.
        dev.cs_select();
        dev.transfer(0x04);
        assert_eq!(dev.framebuffer()[3], 0x00);
        dev.transfer(0x3C);
        assert_eq!(dev.framebuffer()[3], 0x3C, "latched on the second byte");
        dev.cs_release();

        // A stray odd byte followed by a fresh CS assertion must not pair the
        // orphan with the next transaction's address byte.
        dev.cs_select();
        dev.transfer(0xFF); // orphan
        dev.cs_select(); // re-assert: discard the partial frame
        dev.transfer(0x05);
        dev.transfer(0x99);
        dev.cs_release();
        assert_eq!(dev.framebuffer()[4], 0x99);
        // The orphan never became an address byte, so no other row moved.
        assert_eq!(dev.framebuffer()[3], 0x3C);
    }

    #[test]
    fn control_registers_are_stored() {
        let mut dev = Max7219::new("PA4");
        write_reg(&mut dev, REG_DECODE_MODE, 0x00);
        write_reg(&mut dev, REG_INTENSITY, 0x07);
        write_reg(&mut dev, REG_SCAN_LIMIT, 0x07);
        assert_eq!(dev.decode_mode(), 0x00);
        assert_eq!(dev.intensity(), 0x07);
        assert_eq!(dev.scan_limit(), 0x07);

        // BCD decode for all digits, as a 7-segment cascade would set.
        write_reg(&mut dev, REG_DECODE_MODE, 0xFF);
        assert_eq!(dev.decode_mode(), 0xFF);
    }

    #[test]
    fn noop_and_unknown_addresses_are_ignored() {
        let mut dev = Max7219::new("PA4");
        power_on(&mut dev);
        write_reg(&mut dev, 0x01, 0x11);
        write_reg(&mut dev, REG_NOOP, 0xFF); // cascade pass-through
        write_reg(&mut dev, 0x0D, 0xFF); // undocumented
        write_reg(&mut dev, 0x0E, 0xFF); // undocumented
        assert_eq!(dev.framebuffer()[0], 0x11);
        assert!(!dev.is_display_test());
    }

    // ─── Supply ───────────────────────────────────────────────────────────
    //
    // THE MEASURED BUG (st7789.rs carries the full account): a diagram wiring
    // only a module's signal pins — no VCC, no GND — ran and reported lit
    // LEDs. On a bench the matrix is dark.
    //
    // ⚠️ Note the name collision: the helper `power_on` above clears the
    // MAX7219's SHUTDOWN REGISTER, which is a firmware action over a working
    // bus. `powered` is whether the module has a rail at all.

    /// Leave shutdown, set intensity, and light all 8 LEDs of row 0.
    fn drive_a_lit_row(dev: &mut Max7219) {
        power_on(dev);
        write_reg(dev, REG_INTENSITY, 0x0F);
        write_reg(dev, REG_SCAN_LIMIT, 0x07);
        write_reg(dev, 0x01, 0xFF);
    }

    fn supply_meta(dev: &Max7219) -> serde_json::Value {
        SpiDevice::artifacts(dev, "matrix", &crate::inspect::InspectOpts::default())
            .into_iter()
            .next()
            .expect("one framebuffer artifact")
            .meta
    }

    /// The POSITIVE control. Without it, "unpowered is dark" would also pass on
    /// a model that never lights anything.
    #[test]
    fn a_powered_module_driven_this_way_lights_its_leds() {
        let mut dev = Max7219::new("PA4");
        drive_a_lit_row(&mut dev);

        assert!(dev.powered(), "no supply config at all must mean powered");
        assert!(!dev.is_shutdown(), "the shutdown register was cleared");
        let m = supply_meta(&dev);
        assert_eq!(m["lit_pixels"], 8, "row 0 fully lit");
        assert_eq!(m["intensity"], 0x0F);
        assert_eq!(m["shutdown"], false);
        assert_eq!(m["powered"], true);
    }

    /// The FIX. Identical drive, supply declared absent: nothing latches.
    #[test]
    fn an_unpowered_module_reports_dark_on_every_field_the_bug_reported() {
        let mut dev = Max7219::new("PA4").with_powered(false);
        drive_a_lit_row(&mut dev);

        assert!(
            dev.is_shutdown(),
            "an unpowered driver cannot honour the shutdown-clear write",
        );
        let m = supply_meta(&dev);
        assert_eq!(m["lit_pixels"], 0, "no supply, no light");
        assert_eq!(m["ink_bytes"], 0);
        assert_eq!(m["intensity"], 0, "the intensity write never latched");
        assert_eq!(m["powered"], false, "the artifact must say WHY it is dark");
    }

    /// ⚠️ DISPLAY TEST overrides shutdown on silicon — but it cannot override
    /// the absence of a rail. A gate placed only on `shutdown` would let a
    /// display-test write flood an unpowered panel with 64 lit LEDs.
    #[test]
    fn display_test_cannot_light_an_unpowered_module() {
        let mut powered = Max7219::new("PA4");
        write_reg(&mut powered, REG_DISPLAY_TEST, 0x01);
        assert_eq!(
            powered
                .framebuffer()
                .iter()
                .map(|b| b.count_ones())
                .sum::<u32>(),
            64,
            "positive control: display test floods a powered panel",
        );

        let mut dark = Max7219::new("PA4").with_powered(false);
        write_reg(&mut dark, REG_DISPLAY_TEST, 0x01);
        assert_eq!(dark.framebuffer(), [0x00; ROWS]);
        assert!(!dark.is_display_test(), "the write never latched at all");
    }

    /// Digit RAM must not ACCUMULATE either: the bus is refused, so the
    /// underlying RAM is untouched rather than merely blanked at readback.
    #[test]
    fn an_unpowered_module_never_accumulates_paint() {
        let mut dev = Max7219::new("PA4").with_powered(false);
        for _ in 0..5 {
            drive_a_lit_row(&mut dev);
        }
        assert_eq!(
            dev.digit_ram(),
            [0x00; ROWS],
            "digit RAM must be untouched, not just blanked on readback",
        );
        assert_eq!(dev.scan_limit(), 0, "nor may the scan limit latch");
    }

    /// ⚠️ THE BACKWARDS-COMPATIBILITY GUARD. Every hand-written lab manifest
    /// declares no supply at all.
    #[test]
    fn absent_supply_information_means_powered() {
        assert!(Max7219::new("PA4").powered(), "the default must be powered");
        assert!(
            Max7219::new("PA4").with_powered(true).powered(),
            "an explicit true is powered too — only `false` darkens",
        );
    }
}
