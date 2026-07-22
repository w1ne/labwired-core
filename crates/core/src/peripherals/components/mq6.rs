// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::any::Any;

/// Hanwei **MQ-6** LPG / propane / isobutane semiconductor gas sensor module
/// as an [`AnalogSource`] on an ADC channel.
///
/// Real MQ-6 parts measure gas via a heated SnO₂ element whose resistance
/// falls as LPG concentration rises. Breakout modules almost always put that
/// through an op-amp so the pin labelled `AOUT` is a 0..Vref voltage that
/// *rises* with gas concentration. This model captures that module-level
/// behaviour, not the bare Rs/R0 curve from the Hanwei datasheet:
///
///   V_out = V_ref * clamp(ppm / ppm_full_scale, 0..1)
///   - V_ref          = 3.3 V (3300 mV) — typical MCU ADC reference
///   - ppm_full_scale = 10 000 ppm LPG (top of the MQ-6 detection band)
///
/// Monotonicity: more gas → higher AOUT. Clean air (0 ppm) sits at the ground
/// rail; 10 000 ppm saturates at Vref. DOUT (the module comparator) is not
/// modelled — firmware that needs a digital trip threshold can compare AOUT
/// in software, which is what most kits already do.
///
/// All conversion math lives here in Rust core. The WASM bridge passes ppm in
/// and reads mV + ADC count out; it never reimplements this.
#[derive(Debug, serde::Serialize)]
pub struct Mq6 {
    /// ADC channel this module's AOUT is wired to.
    channel: u8,
    /// Current LPG-equivalent concentration in ppm.
    ppm: f32,
    /// Full-scale concentration that saturates AOUT at Vref.
    ppm_full_scale: f32, // 10_000.0
    v_ref_mv: f32,       // 3300.0
    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Default for Mq6 {
    fn default() -> Self {
        Self::new(0, 0.0)
    }
}

impl Mq6 {
    pub fn new(channel: u8, ppm: f32) -> Self {
        Self {
            channel,
            ppm,
            ppm_full_scale: 10_000.0,
            v_ref_mv: 3300.0,
            component_id: None,
        }
    }

    pub fn set_ppm(&mut self, ppm: f32) {
        self.ppm = ppm.max(0.0);
    }

    pub fn ppm(&self) -> f32 {
        self.ppm
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// AOUT voltage in mV for the current concentration.
    pub fn aout_mv(&self) -> u16 {
        let fraction = (self.ppm / self.ppm_full_scale).clamp(0.0, 1.0);
        (self.v_ref_mv * fraction) as u16
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

/// LPG-equivalent gas concentration in ppm. Range covers clean air (0) through
/// the top of the MQ-6 detection band (~10 000 ppm). One table backs BOTH the
/// `SimInput` impl and the kit metadata, so the device schema and the runtime
/// API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[crate::sim_input::InputChannel {
    key: "ppm",
    label: "LPG concentration",
    unit: "ppm",
    min: 0.0,
    max: 10000.0,
}];

impl crate::sim_input::SimInput for Mq6 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        self.set_ppm(value as f32);
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

impl crate::bus::sim_inputs::AnalogSource for Mq6 {
    fn output_mv(&self) -> u16 {
        self.aout_mv()
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Mq6Kit;
pub static MQ6_KIT: Mq6Kit = Mq6Kit;

static MQ6_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "mq-6",
    label: "MQ-6 Gas Sensor",
    summary: "Hanwei MQ-6 LPG/propane semiconductor gas sensor module (AOUT on ADC).",
    detail: "Module-level model of the common MQ-6 breakout: AOUT is a 0..Vref \
             voltage that rises with LPG-equivalent concentration. Drive the `ppm` \
             channel (0..10 000 ppm) at runtime and the ADC channel follows through \
             a linear full-scale map. Starts at 0 ppm (clean air → ground rail). \
             DOUT (comparator trip) is not modelled — compare AOUT in firmware.",
    transport: Transport::Analog,
    category: Category::Analog,
    config_keys: &[ConfigKey {
        name: "channel",
        ty: ConfigType::Int,
        doc: "ADC channel index (0..N). Defaults to 0.",
    }],
    labs: &[],
};

impl PeripheralKit for Mq6Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &MQ6_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let channel = ctx.config_i64("channel").unwrap_or(0).clamp(0, 255) as u8;
        // Retained on the bus so `set_input("ppm", …)` can drive it; AOUT is
        // seeded from the 0 ppm (clean air) default at attach.
        ctx.attach_analog_source(channel, Box::new(Mq6::new(channel, 0.0)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_input::SimInput;

    #[test]
    fn test_mq6_monotonic() {
        let mut clean = Mq6::new(0, 0.0);
        clean.set_input("ppm", 0.0).unwrap();
        let mut mid = Mq6::new(0, 0.0);
        mid.set_input("ppm", 2_500.0).unwrap();
        let mut rich = Mq6::new(0, 0.0);
        rich.set_input("ppm", 10_000.0).unwrap();
        assert!(
            clean.aout_mv() < mid.aout_mv(),
            "expected clean ({}) < mid ({})",
            clean.aout_mv(),
            mid.aout_mv()
        );
        assert!(
            mid.aout_mv() < rich.aout_mv(),
            "expected mid ({}) < rich ({})",
            mid.aout_mv(),
            rich.aout_mv()
        );
    }

    #[test]
    fn test_mq6_set_input_updates_ppm() {
        let mut sensor = Mq6::new(0, 0.0);
        sensor.set_input("ppm", 1500.0).unwrap();
        assert_eq!(sensor.ppm(), 1500.0);
    }

    #[test]
    fn test_mq6_clean_and_full_scale_rails() {
        let clean = Mq6::new(0, 0.0);
        assert_eq!(clean.aout_mv(), 0, "clean air should read ~0 mV");
        let full = Mq6::new(0, 10_000.0);
        assert_eq!(full.aout_mv(), 3300, "full scale should read Vref");
        assert_eq!(full.adc_count(), 4095);
    }
}
