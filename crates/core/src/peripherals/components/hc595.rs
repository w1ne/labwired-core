// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Standalone 74HC595 8-bit serial-in / parallel-out shift register (output).
//!
//! The host drives it as an SPI slave: `SER`→MOSI (data in), `SRCLK`→SCK (shift
//! clock), `RCLK`→chip-select (storage-register latch). Firmware shifts one
//! byte, then pulses `RCLK` to latch the shift register into the eight output
//! pins `QA..QH`. The parallel outputs do NOT change until that latch — so a
//! half-shifted value is never visible on the pins.
//!
//! Bit order (matches the datasheet and `hc595_7seg`): SPI transmits MSB-first,
//! so the first bit clocked in ends up at `QH` and the last at `QA`. One
//! transferred byte therefore lands verbatim in the shift register with
//!   - bit 7 (MSB) = QH
//!   - bit 0 (LSB) = QA

use crate::peripherals::spi::SpiDevice;
use std::any::Any;

/// Simulated 74HC595 8-bit serial-in / parallel-out shift register.
#[derive(Debug, serde::Serialize)]
pub struct Hc595 {
    /// Latch line (`RCLK`), wired to the GPIO used as SPI chip-select.
    cs_pin: String,
    /// Bits clocked in but not yet latched. bit 7 = QH end, bit 0 = QA end.
    shift_reg: u8,
    /// Latched value currently driven on the parallel outputs QA..QH.
    output_latch: u8,
    /// system.yaml `external_devices` id, stamped at attach if the bus wires
    /// one (see [`crate::sim_input::SimInput::component_id`]). Retained for
    /// readback identity; a pure output register serves no `SimInput` channel.
    component_id: Option<String>,
}

impl Hc595 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            shift_reg: 0,
            output_latch: 0,
            component_id: None,
        }
    }

    /// Read back the latched parallel outputs: bit 0 = QA … bit 7 = QH.
    pub fn outputs(&self) -> u8 {
        self.output_latch
    }
}

impl SpiDevice for Hc595 {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        // RCLK rising edge → latch the shift register into the output pins.
        self.output_latch = self.shift_reg;
    }

    fn cs_release(&mut self) {}

    fn transfer(&mut self, mosi: u8) -> u8 {
        // A single 8-bit stage: the transferred byte becomes the new shift
        // register contents (MSB-first → bit 7 = QH, bit 0 = QA). Outputs are
        // untouched until the next latch (`cs_select`).
        self.shift_reg = mosi;
        0 // 595 has no MISO (QH' daisy-chain out is not modelled here).
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

pub struct Hc595Kit;
pub static HC595_KIT: Hc595Kit = Hc595Kit;

static HC595_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "74hc595",
    label: "74HC595 Shift Register (8-bit output)",
    summary: "8-bit serial-in / parallel-out shift register over SPI.",
    detail: "Lets a host MCU drive 8 outputs (QA..QH) through one SPI byte: SER→MOSI, SRCLK→SCK, \
             RCLK→chip-select. Firmware shifts a byte then pulses RCLK to latch it onto the pins; \
             the parallel outputs hold until that latch. Bit 7 = QH, bit 0 = QA (MSB-first).",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[ConfigKey {
        name: "cs_pin",
        ty: ConfigType::Str,
        doc: "RCLK latch GPIO pin, wired as SPI chip-select (e.g. \"PA4\"). Defaults to PA4.",
    }],
    labs: &[],
};

impl PeripheralKit for Hc595Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &HC595_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs_pin = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        ctx.attach_spi_device(Box::new(Hc595::new(cs_pin)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::spi::SpiDevice;

    #[test]
    fn latched_byte_appears_on_outputs() {
        let mut dev = Hc595::new("PA4");
        dev.transfer(0xA5);
        dev.cs_select(); // RCLK latch
        assert_eq!(dev.outputs(), 0xA5);
    }

    #[test]
    fn transfer_without_latch_leaves_outputs_unchanged() {
        let mut dev = Hc595::new("PA4");
        // Latch a first value.
        dev.transfer(0x0F);
        dev.cs_select();
        assert_eq!(dev.outputs(), 0x0F);
        // Shift a new value in WITHOUT latching — outputs must still show 0x0F.
        dev.transfer(0xF0);
        assert_eq!(
            dev.outputs(),
            0x0F,
            "outputs must not change before RCLK latch"
        );
        // Now latch; the new value takes effect.
        dev.cs_select();
        assert_eq!(dev.outputs(), 0xF0);
    }

    #[test]
    fn bit_order_qa_is_lsb_qh_is_msb() {
        // 0x01 = only the LSB set → only QA lit, QH dark.
        let mut dev = Hc595::new("PA4");
        dev.transfer(0x01);
        dev.cs_select();
        assert_eq!(dev.outputs() & 0x01, 0x01, "bit 0 drives QA");
        assert_eq!(dev.outputs() & 0x80, 0x00, "QH (bit 7) must be dark");
        // 0x80 = only the MSB set → only QH lit, QA dark.
        dev.transfer(0x80);
        dev.cs_select();
        assert_eq!(dev.outputs() & 0x80, 0x80, "bit 7 drives QH");
        assert_eq!(dev.outputs() & 0x01, 0x00, "QA (bit 0) must be dark");
    }
}
