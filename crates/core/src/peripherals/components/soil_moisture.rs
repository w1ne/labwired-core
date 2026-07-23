// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::any::Any;

/// Capacitive **soil moisture** probe module as an [`AnalogSource`] on an ADC
/// channel.
///
/// Maker-kit capacitive probes (the corrosion-resistant PCB style with an
/// onboard oscillator + comparator) expose an analog pin whose voltage *falls*
/// as the soil around the fork gets wetter — water raises the dielectric
/// constant and the oscillator frequency, and the module's filter maps that
/// into a lower AOUT. This model captures that module-level polarity:
///
///   V_out = V_ref * (1 − clamp(moisture / 100, 0..1))
///   - V_ref    = 3.3 V (3300 mV)
///   - moisture = volumetric water content as a 0..100 % scale
///
/// Monotonicity: wetter → lower AOUT. Bone-dry (0 %) sits at Vref; fully
/// saturated (100 %) sits at the ground rail. DOUT (the module comparator) is
/// not modelled — firmware that needs a digital trip threshold can compare
/// AOUT in software.
///
/// All conversion math lives here in Rust core. The WASM bridge passes moisture
/// in and reads mV + ADC count out; it never reimplements this.
#[derive(Debug, serde::Serialize)]
pub struct SoilMoisture {
    /// ADC channel this module's AOUT is wired to.
    channel: u8,
    /// Current moisture as a 0..100 percentage.
    moisture_pct: f32,
    v_ref_mv: f32, // 3300.0
    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Default for SoilMoisture {
    fn default() -> Self {
        // Default to "slightly damp" rather than bone-dry so a freshly placed
        // sensor does not sit hard against a rail.
        Self::new(0, 40.0)
    }
}

impl SoilMoisture {
    pub fn new(channel: u8, moisture_pct: f32) -> Self {
        Self {
            channel,
            moisture_pct: moisture_pct.clamp(0.0, 100.0),
            v_ref_mv: 3300.0,
            component_id: None,
        }
    }

    pub fn set_moisture_pct(&mut self, moisture_pct: f32) {
        self.moisture_pct = moisture_pct.clamp(0.0, 100.0);
    }

    pub fn moisture_pct(&self) -> f32 {
        self.moisture_pct
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// AOUT voltage in mV for the current moisture.
    pub fn aout_mv(&self) -> u16 {
        let dry_fraction = 1.0 - (self.moisture_pct / 100.0).clamp(0.0, 1.0);
        (self.v_ref_mv * dry_fraction) as u16
    }

    /// Convert `aout_mv` to a 12-bit ADC count (0..4095) for 3.3 V Vref.
    pub fn adc_count(&self) -> u16 {
        let mv = self.aout_mv() as u32;
        ((mv * 4095) / 3300).min(4095) as u16
    }

    pub fn as_any(&self) -> &dyn Any {
        self
    }
    pub fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Volumetric soil moisture as a 0..100 % scale. One table backs BOTH the
/// `SimInput` impl and the kit metadata, so the device schema and the runtime
/// API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[crate::sim_input::InputChannel {
    key: "moisture",
    label: "Soil moisture",
    unit: "%",
    min: 0.0,
    max: 100.0,
}];

impl crate::sim_input::SimInput for SoilMoisture {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        self.set_moisture_pct(value as f32);
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

impl crate::bus::sim_inputs::AnalogSource for SoilMoisture {
    fn output_mv(&self) -> u16 {
        self.aout_mv()
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct SoilMoistureKit;
pub static SOIL_MOISTURE_KIT: SoilMoistureKit = SoilMoistureKit;

static SOIL_MOISTURE_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "soil-moisture",
    label: "Soil Moisture Sensor",
    summary: "Capacitive soil moisture probe module (AOUT on ADC; wetter → lower voltage).",
    detail: "Module-level model of the common capacitive soil-moisture breakout: \
             AOUT is a 0..Vref voltage that falls as the soil gets wetter. Drive the \
             `moisture` channel (0..100 %) at runtime and the ADC channel follows. \
             Starts at 40 % (slightly damp). DOUT (comparator trip) is not modelled \
             — compare AOUT in firmware.",
    transport: Transport::Analog,
    category: Category::Analog,
    config_keys: &[ConfigKey {
        name: "channel",
        ty: ConfigType::Int,
        doc: "ADC channel index (0..N). Defaults to 0.",
    }],
    labs: &[],
};

impl PeripheralKit for SoilMoistureKit {
    fn metadata(&self) -> &'static KitMetadata {
        &SOIL_MOISTURE_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let channel = ctx.config_i64("channel").unwrap_or(0).clamp(0, 255) as u8;
        // Retained on the bus so `set_input("moisture", …)` can drive it; AOUT
        // is seeded from the 40 % default at attach.
        ctx.attach_analog_source(channel, Box::new(SoilMoisture::new(channel, 40.0)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_input::SimInput;

    #[test]
    fn test_soil_moisture_inverse_monotonic() {
        // Wetter → lower voltage (capacitive module polarity).
        let mut dry = SoilMoisture::new(0, 0.0);
        dry.set_input("moisture", 0.0).unwrap();
        let mut mid = SoilMoisture::new(0, 0.0);
        mid.set_input("moisture", 50.0).unwrap();
        let mut wet = SoilMoisture::new(0, 0.0);
        wet.set_input("moisture", 100.0).unwrap();
        assert!(
            dry.aout_mv() > mid.aout_mv(),
            "expected dry ({}) > mid ({})",
            dry.aout_mv(),
            mid.aout_mv()
        );
        assert!(
            mid.aout_mv() > wet.aout_mv(),
            "expected mid ({}) > wet ({})",
            mid.aout_mv(),
            wet.aout_mv()
        );
    }

    #[test]
    fn test_soil_moisture_set_input_updates() {
        let mut sensor = SoilMoisture::new(0, 40.0);
        sensor.set_input("moisture", 75.0).unwrap();
        assert_eq!(sensor.moisture_pct(), 75.0);
    }

    #[test]
    fn test_soil_moisture_rails() {
        let dry = SoilMoisture::new(0, 0.0);
        assert_eq!(dry.aout_mv(), 3300, "bone-dry should read Vref");
        assert_eq!(dry.adc_count(), 4095);
        let wet = SoilMoisture::new(0, 100.0);
        assert_eq!(wet.aout_mv(), 0, "saturated should read ~0 mV");
    }
}
