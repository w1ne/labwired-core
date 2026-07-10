// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;

/// Simulated 74HC165 8-bit parallel-in / serial-out shift register.
///
/// Used as a digital-input expander: 8 parallel input channels are clocked out
/// serially MSB-first (QH = channel 7). The SH/LD (load) pulse is not separately
/// modeled on the STM32 SPI bus (which never drives CS callbacks), so the live
/// inputs are sampled at clock-out time inside `transfer`.
#[derive(Debug, serde::Serialize)]
pub struct Sn74hc165 {
    /// SH/LD line, wired to the GPIO used as SPI chip-select for this device.
    cs_pin: String,
    /// Live parallel input states; bit `i` = channel `i`.
    inputs: u8,
    /// Value captured at the last load (CS assert).
    latched: u8,
    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Sn74hc165 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            inputs: 0,
            latched: 0,
            component_id: None,
        }
    }

    /// Set all 8 input channels at once (bit `i` = channel `i`).
    pub fn set_inputs(&mut self, value: u8) {
        self.inputs = value;
    }

    /// Set a single input channel high or low (no-op for `ch >= 8`).
    pub fn set_channel(&mut self, ch: u8, high: bool) {
        if ch < 8 {
            if high {
                self.inputs |= 1 << ch;
            } else {
                self.inputs &= !(1 << ch);
            }
        }
    }

    /// Read back the live parallel input states.
    pub fn inputs(&self) -> u8 {
        self.inputs
    }
}

impl SpiDevice for Sn74hc165 {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        // SH/LD low → parallel load of the live inputs into the shift register.
        self.latched = self.inputs;
    }

    fn cs_release(&mut self) {}

    fn transfer(&mut self, _mosi: u8) -> u8 {
        // The STM32 SPI bus does not drive CS callbacks, and the SH/LD load
        // pulse is not separately modeled, so capture the live inputs at
        // clock-out time. Clocks out the 8 bits MSB-first (QH = bit 7 = channel 7).
        self.latched = self.inputs;
        self.latched
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

/// Drivable parallel input channels, one per pin: 0 = low, 1 = high
/// (values ≥ 0.5 read as high). One table backs BOTH the `SimInput` impl and
/// the kit metadata, so the device schema and the runtime API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = {
    macro_rules! ch {
        ($key:literal, $label:literal) => {
            crate::sim_input::InputChannel {
                key: $key,
                label: $label,
                unit: "level",
                min: 0.0,
                max: 1.0,
            }
        };
    }
    &[
        ch!("ch0", "D0"),
        ch!("ch1", "D1"),
        ch!("ch2", "D2"),
        ch!("ch3", "D3"),
        ch!("ch4", "D4"),
        ch!("ch5", "D5"),
        ch!("ch6", "D6"),
        ch!("ch7", "D7"),
    ]
};

impl crate::sim_input::SimInput for Sn74hc165 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        let ch = key
            .strip_prefix("ch")
            .and_then(|n| n.parse::<u8>().ok())
            .expect("require_channel validated the key");
        self.set_channel(ch, value >= 0.5);
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

pub struct Sn74hc165Kit;
pub static SN74HC165_KIT: Sn74hc165Kit = Sn74hc165Kit;

static SN74HC165_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "sn74hc165",
    label: "74HC165 Shift Register",
    summary: "8-bit parallel-in / serial-out shift register over SPI.",
    detail: "Lets a host MCU sample 8 GPIO inputs through one SPI clock burst. Used in the \
             IO-Link DI/DO lab to surface field-side switch state.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[
        ConfigKey {
            name: "cs_pin",
            ty: ConfigType::Str,
            doc: "Chip-select GPIO pin (e.g. \"PA4\"). Defaults to PA4.",
        },
        ConfigKey {
            name: "inputs",
            ty: ConfigType::Int,
            doc: "Optional initial 8-bit input state (0..0xFF) — useful for static test stimulus.",
        },
    ],
    labs: &[LabRef {
        board_id: "iolink-dido",
        chip: "stm32l476",
        example_dir: "iolink-dido",
        demo_elf: "demo-iolink-dido.elf",
    }],
};

impl PeripheralKit for Sn74hc165Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &SN74HC165_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs_pin = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let inputs = ctx.config_i64("inputs");
        let mut shifter = Sn74hc165::new(cs_pin);
        if let Some(v) = inputs {
            shifter.set_inputs(v as u8);
        }
        ctx.attach_spi_device(Box::new(shifter))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::spi::SpiDevice;

    #[test]
    fn loads_and_shifts_out_inputs_msb_first() {
        let mut d = Sn74hc165::new("PA4");
        d.set_inputs(0xA5);
        d.cs_select(); // SH/LD pulse → parallel load
                       // 74HC165 clocks out QH..QA MSB-first; SPI reads MSB-first → byte == inputs
        assert_eq!(d.transfer(0x00), 0xA5);
    }

    #[test]
    fn transfer_reflects_live_inputs_without_cs_select() {
        // Runtime guard: the STM32 SPI bus never calls cs_select, so transfer
        // alone must return the current inputs.
        let mut d = Sn74hc165::new("PA4");
        d.set_inputs(0xA5);
        assert_eq!(d.transfer(0x00), 0xA5);
        d.set_inputs(0x3C);
        assert_eq!(d.transfer(0x00), 0x3C);
    }

    #[test]
    fn set_channel_sets_individual_bits() {
        let mut d = Sn74hc165::new("PA4");
        d.set_channel(0, true);
        d.set_channel(7, true);
        assert_eq!(d.inputs(), 0b1000_0001);
    }
}
