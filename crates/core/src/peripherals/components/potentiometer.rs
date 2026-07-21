// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::any::Any;

/// Linear potentiometer as a voltage divider on an ADC channel.
///
/// A potentiometer is a three-terminal linear voltage divider: the two end
/// terminals sit across `Vref`, and the wiper taps off a fraction of that
/// voltage set by the mechanical position.
///
/// Wiper equation: V_wiper = V_ref * position_pct / 100
///   - V_ref = 3.3 V (3300 mV)
///
/// This model covers a rotary pot and a slide pot alike — they are
/// electrically identical, differing only in the physical actuator.
///
/// All divider math lives here in Rust core. The WASM bridge passes a wiper
/// position (0..100 %) in and reads mV + ADC count out.
#[derive(Debug, serde::Serialize)]
pub struct Potentiometer {
    /// ADC channel this potentiometer's wiper is wired to.
    channel: u8,
    /// Current wiper position, 0..100 %.
    position_pct: f32,
    /// Reference voltage across the pot, in mV.
    v_ref_mv: f32, // 3300.0
    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Default for Potentiometer {
    fn default() -> Self {
        Self::new(0, 50.0)
    }
}

impl Potentiometer {
    pub fn new(channel: u8, position_pct: f32) -> Self {
        Self {
            channel,
            position_pct,
            v_ref_mv: 3300.0,
            component_id: None,
        }
    }

    pub fn set_position_pct(&mut self, pct: f32) {
        self.position_pct = pct;
    }

    pub fn position_pct(&self) -> f32 {
        self.position_pct
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// Compute the wiper output in mV for the current position.
    ///
    /// This is the voltage-divider math — it lives here in Rust core.
    /// The WASM bridge and UI never reimplement this.
    pub fn wiper_output_mv(&self) -> u16 {
        (self.v_ref_mv * self.position_pct / 100.0).clamp(0.0, self.v_ref_mv) as u16
    }

    /// Convert wiper_output_mv to a 12-bit ADC count (0..4095) for 3.3 V Vref.
    pub fn adc_count(&self) -> u16 {
        let mv = self.wiper_output_mv() as u32;
        ((mv * 4095) / 3300).min(4095) as u16
    }

    pub fn as_any(&self) -> &dyn Any {
        self
    }
    pub fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The wiper position, in percent of full travel. One table backs BOTH the
/// `SimInput` impl and the kit metadata, so the device schema and the runtime
/// API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[crate::sim_input::InputChannel {
    key: "position",
    label: "Position",
    unit: "%",
    min: 0.0,
    max: 100.0,
}];

impl crate::sim_input::SimInput for Potentiometer {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        // Only the position moves; the divider math that turns it into a wiper
        // voltage is unchanged and still lives in `wiper_output_mv`.
        self.set_position_pct(value as f32);
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

impl crate::bus::sim_inputs::AnalogSource for Potentiometer {
    fn output_mv(&self) -> u16 {
        self.wiper_output_mv()
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct PotentiometerKit;
pub static POTENTIOMETER_KIT: PotentiometerKit = PotentiometerKit;

static POTENTIOMETER_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "potentiometer",
    label: "Potentiometer",
    summary: "3-pin linear potentiometer on an ADC channel (rotary or slide).",
    detail: "Linear voltage-divider model: the wiper taps off Vref * position/100. Drive the \
             `position` channel (0..100 %) at runtime and the ADC channel follows through the \
             real divider math. Starts centred at 50 %. A rotary pot and a slide pot are \
             electrically identical and share this model.",
    transport: Transport::Analog,
    category: Category::Analog,
    config_keys: &[ConfigKey {
        name: "channel",
        ty: ConfigType::Int,
        doc: "ADC channel index (0..N). Defaults to 0.",
    }],
    labs: &[],
};

impl PeripheralKit for PotentiometerKit {
    fn metadata(&self) -> &'static KitMetadata {
        &POTENTIOMETER_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let channel = ctx.config_i64("channel").unwrap_or(0).clamp(0, 255) as u8;
        // Retained on the bus so `set_input("position", …)` can drive it; the
        // wiper level is seeded from the centred default at attach.
        ctx.attach_analog_source(channel, Box::new(Potentiometer::new(channel, 50.0)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pot_zero() {
        let pot = Potentiometer::new(0, 0.0);
        assert_eq!(pot.wiper_output_mv(), 0, "expected 0 mV at 0%");
    }

    #[test]
    fn test_pot_full() {
        let pot = Potentiometer::new(0, 100.0);
        let mv = pot.wiper_output_mv();
        assert!(
            (mv as i32 - 3300).abs() <= 1,
            "expected ~3300 mV at 100%, got {mv}"
        );
        assert_eq!(pot.adc_count(), 4095, "expected 4095 at 100%");
    }

    #[test]
    fn test_pot_midpoint() {
        let pot = Potentiometer::new(0, 50.0);
        let mv = pot.wiper_output_mv();
        assert!(
            (mv as i32 - 1650).abs() <= 2,
            "expected ~1650 mV at 50%, got {mv}"
        );
        let count = pot.adc_count();
        assert!(
            (count as i32 - 2047).abs() <= 2,
            "expected ~2047 at 50%, got {count}"
        );
    }

    #[test]
    fn test_pot_monotonic() {
        let hi = Potentiometer::new(0, 75.0);
        let lo = Potentiometer::new(0, 25.0);
        assert!(hi.wiper_output_mv() > lo.wiper_output_mv());
    }
}
