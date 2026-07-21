// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;

/// Simulated MAX31855 cold-junction-compensated thermocouple-to-digital converter.
///
/// Read-only SPI device: on CS low the MAX31855 clocks out a 32-bit big-endian
/// status word over 4 SPI byte transfers.
///
/// Word layout (big-endian bit numbering, MSB first):
/// - [31:18]  thermocouple temperature × 4 (14-bit signed, °C × 4)
/// - [17]     reserved (0)
/// - [16]     fault (1 if any fault flag is set)
/// - [15:4]   internal (cold-junction) temperature × 16 (12-bit signed, °C × 16)
/// - [3]      reserved (0)
/// - [2]      SCV: short to VCC fault
/// - [1]      SCG: short to GND fault
/// - [0]      OC:  open circuit fault
#[derive(Debug, serde::Serialize)]
pub struct Max31855 {
    cs_pin: String,
    /// Thermocouple temperature × 4 (14-bit signed range, i.e. –2048..2047)
    pub tc_temp_q14: i32,
    /// Internal (cold-junction) temperature × 16 (12-bit signed range, i.e. –128..127)
    pub internal_temp_q12: i32,
    pub fault: bool,
    /// Position within the 4-byte response (0..3). Reset on CS select.
    byte_index: u8,
    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Max31855 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            // Defaults: 25.0 °C thermocouple, 22.0 °C internal, no fault
            tc_temp_q14: 100,       // 25.0 × 4
            internal_temp_q12: 352, // 22.0 × 16
            fault: false,
            byte_index: 0,
            component_id: None,
        }
    }

    /// Update the simulated thermocouple and internal temperatures.
    pub fn set_temperature(&mut self, tc_c: f32, internal_c: f32) {
        self.tc_temp_q14 = (tc_c * 4.0).round() as i32;
        self.internal_temp_q12 = (internal_c * 16.0).round() as i32;
        // Recompute fault bit: set when any of OC/SCG/SCV is non-zero
        // (no explicit fault flags exposed in v1 — only read via `fault` field)
        // Users who want a fault scenario should set `fault` directly.
    }

    /// Read back the current temperatures.
    pub fn temperature(&self) -> (f32, f32) {
        (
            self.tc_temp_q14 as f32 / 4.0,
            self.internal_temp_q12 as f32 / 16.0,
        )
    }

    fn current_response_word(&self) -> u32 {
        // Sign-extend to 32-bit then mask to the field width before placing
        let tc14 = (self.tc_temp_q14 as u32) & 0x3FFF;
        let int12 = (self.internal_temp_q12 as u32) & 0x0FFF;
        let fault_bit: u32 = if self.fault { 1 } else { 0 };
        (tc14 << 18) | (fault_bit << 16) | (int12 << 4)
    }
}

impl SpiDevice for Max31855 {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        self.byte_index = 0;
    }

    fn cs_release(&mut self) {
        self.byte_index = 0;
    }

    fn transfer(&mut self, _mosi: u8) -> u8 {
        let word = self.current_response_word();
        let byte = match self.byte_index {
            0 => ((word >> 24) & 0xFF) as u8,
            1 => ((word >> 16) & 0xFF) as u8,
            2 => ((word >> 8) & 0xFF) as u8,
            _ => (word & 0xFF) as u8,
        };
        if self.byte_index < 3 {
            self.byte_index += 1;
        }
        byte
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        Some(self)
    }
}

/// Drivable temperatures, in °C: the thermocouple hot junction (K-type range)
/// and the cold-junction/internal sensor. Driving one preserves the other.
/// One table backs BOTH the `SimInput` impl and the kit metadata, so the
/// device schema and the runtime API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "temperature",
        label: "Thermocouple",
        unit: "°C",
        min: -200.0,
        max: 1350.0,
    },
    crate::sim_input::InputChannel {
        key: "internal",
        label: "Internal",
        unit: "°C",
        min: -55.0,
        max: 125.0,
    },
];

impl crate::sim_input::SimInput for Max31855 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        let (tc, internal) = self.temperature();
        match key {
            "temperature" => self.set_temperature(value as f32, internal),
            "internal" => self.set_temperature(tc, value as f32),
            _ => unreachable!("require_channel validated the key"),
        }
        Ok(())
    }
    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct Max31855Kit;
pub static MAX31855_KIT: Max31855Kit = Max31855Kit;

static MAX31855_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "max31855",
    label: "MAX31855 Thermocouple",
    summary: "Cold-junction-compensated K-type thermocouple amplifier (read-only SPI).",
    detail: "Returns the 32-bit MAX31855 response on every CS-framed transaction. Host \
             stimulus seeds thermocouple + cold-junction temperatures; bit layout matches the \
             real silicon's datasheet exactly.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[ConfigKey {
        name: "cs_pin",
        ty: ConfigType::Str,
        doc: "Chip-select GPIO pin (e.g. \"PA4\"). Defaults to PA4.",
    }],
    labs: &[LabRef {
        board_id: "max31855-thermocouple-lab",
        chip: "stm32f103",
        example_dir: "max31855-thermocouple-lab",
        demo_elf: "demo-max31855-thermocouple-lab.elf",
    }],
};

impl PeripheralKit for Max31855Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &MAX31855_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs_pin = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        ctx.attach_spi_device(Box::new(Max31855::new(cs_pin)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Max31855;
    use crate::peripherals::spi::SpiDevice;

    #[test]
    fn test_max31855_default_word() {
        let mut dev = Max31855::new("PA4");
        // Default: tc=25.0°C (q14=100), internal=22.0°C (q12=352), fault=false
        // word = (100 << 18) | (0 << 16) | (352 << 4)
        //      = 0x01900000 | 0x00001600
        //      = 0x01901600
        dev.cs_select();
        let b0 = dev.transfer(0x00);
        let b1 = dev.transfer(0x00);
        let b2 = dev.transfer(0x00);
        let b3 = dev.transfer(0x00);
        let word = ((b0 as u32) << 24) | ((b1 as u32) << 16) | ((b2 as u32) << 8) | (b3 as u32);

        let expected: u32 = (100u32 << 18) | (352u32 << 4);
        assert_eq!(
            word, expected,
            "word=0x{:08X} expected=0x{:08X}",
            word, expected
        );
    }

    #[test]
    fn test_max31855_set_temperature() {
        let mut dev = Max31855::new("PA4");
        dev.set_temperature(100.0, 25.0);
        let (tc, internal) = dev.temperature();
        assert!((tc - 100.0).abs() < 0.3, "tc={}", tc);
        assert!((internal - 25.0).abs() < 0.07, "internal={}", internal);
    }

    #[test]
    fn test_max31855_byte_index_resets_on_cs_select() {
        let mut dev = Max31855::new("PA4");
        dev.cs_select();
        let b0_first = dev.transfer(0x00);
        dev.cs_release();
        dev.cs_select();
        let b0_second = dev.transfer(0x00);
        assert_eq!(b0_first, b0_second, "byte 0 should repeat after cs_select");
    }
}
