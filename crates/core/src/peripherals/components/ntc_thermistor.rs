// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::any::Any;

/// NTC thermistor + pull-down voltage divider model.
///
/// Beta equation: R(T) = R0 * exp(B * (1/T - 1/T0))
///   - R0 = 10 kΩ at T0 = 298.15 K (25 °C)
///   - B  = 3950 (NTC 3950 coefficient)
///
/// Voltage divider: V_out = V_ref * R_pulldown / (R_ntc + R_pulldown)
///   - V_ref = 3.3 V, R_pulldown = 10 kΩ
///
/// All Steinhart-Hart / Beta-equation math lives here in Rust core.
/// The WASM bridge passes °C in and reads mV + ADC count out.
#[derive(Debug, serde::Serialize)]
pub struct NtcThermistor {
    /// ADC channel this thermistor is wired to.
    channel: u8,
    /// Current temperature in °C.
    temperature_c: f32,
    /// Calibration constants.
    r0_ohm: f32, // 10 000.0
    t0_k: f32,           // 298.15
    beta: f32,           // 3950.0
    r_pulldown_ohm: f32, // 10 000.0
    v_ref_mv: f32,       // 3300.0
    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Default for NtcThermistor {
    fn default() -> Self {
        Self::new(0, 25.0)
    }
}

impl NtcThermistor {
    pub fn new(channel: u8, temperature_c: f32) -> Self {
        Self {
            channel,
            temperature_c,
            r0_ohm: 10_000.0,
            t0_k: 298.15,
            beta: 3950.0,
            r_pulldown_ohm: 10_000.0,
            v_ref_mv: 3300.0,
            component_id: None,
        }
    }

    pub fn set_temperature(&mut self, temperature_c: f32) {
        self.temperature_c = temperature_c;
    }

    pub fn temperature(&self) -> f32 {
        self.temperature_c
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// Compute the voltage-divider output in mV for the current temperature.
    ///
    /// This is the Steinhart-Hart / Beta-equation math — it lives here in Rust core.
    /// The WASM bridge and UI never reimplement this.
    pub fn divider_output_mv(&self) -> u16 {
        let t_k = self.temperature_c + 273.15;
        // R(T) = R0 * exp(B * (1/T - 1/T0))
        let exponent = self.beta * (1.0 / t_k - 1.0 / self.t0_k);
        let r_ntc = self.r0_ohm * exponent.exp();
        // Voltage divider: V_out = V_ref * R_pull / (R_ntc + R_pull)
        let v_out = self.v_ref_mv * self.r_pulldown_ohm / (r_ntc + self.r_pulldown_ohm);
        v_out.clamp(0.0, self.v_ref_mv) as u16
    }

    /// Convert divider_output_mv to a 12-bit ADC count (0..4095) for 3.3 V Vref.
    pub fn adc_count(&self) -> u16 {
        let mv = self.divider_output_mv() as u32;
        ((mv * 4095) / 3300).min(4095) as u16
    }

    pub fn as_any(&self) -> &dyn Any {
        self
    }
    pub fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The sensed temperature. Range is the usable span of a 10 kΩ / B=3950 NTC.
/// One table backs BOTH the `SimInput` impl and the kit metadata, so the
/// device schema and the runtime API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[crate::sim_input::InputChannel {
    key: "temperature",
    label: "Temperature",
    unit: "°C",
    min: -55.0,
    max: 150.0,
}];

impl crate::sim_input::SimInput for NtcThermistor {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        // Only the temperature moves; the beta-equation resistance and the
        // divider that turns it into a voltage are unchanged and still live in
        // `divider_output_mv`.
        self.set_temperature(value as f32);
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

impl crate::bus::sim_inputs::AnalogSource for NtcThermistor {
    fn output_mv(&self) -> u16 {
        self.divider_output_mv()
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct NtcThermistorKit;
pub static NTC_THERMISTOR_KIT: NtcThermistorKit = NtcThermistorKit;

static NTC_THERMISTOR_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "ntc-thermistor",
    label: "NTC Thermistor",
    summary: "10 kΩ NTC + voltage divider on an ADC channel.",
    detail: "Beta-equation thermistor model (R0 = 10 kΩ at 25 °C, B = 3950) feeding a 10 kΩ \
             pull-down divider. Drive the `temperature` channel (°C) at runtime and the ADC \
             channel follows through the real resistance→voltage math. Starts at 25 °C, where \
             the divider sits at exactly Vref/2.",
    transport: Transport::Analog,
    category: Category::Analog,
    config_keys: &[ConfigKey {
        name: "channel",
        ty: ConfigType::Int,
        doc: "ADC channel index (0..N). Defaults to 0.",
    }],
    labs: &[LabRef {
        board_id: "ntc-thermistor-lab",
        chip: "stm32f103",
        example_dir: "ntc-thermistor-lab",
        demo_elf: "demo-ntc-thermistor-lab.elf",
    }],
};

impl PeripheralKit for NtcThermistorKit {
    fn metadata(&self) -> &'static KitMetadata {
        &NTC_THERMISTOR_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let channel = ctx.config_i64("channel").unwrap_or(0).clamp(0, 255) as u8;
        // Retained on the bus so `set_input("temperature", …)` can drive it;
        // the divider level is seeded from the 25 °C default at attach.
        ctx.attach_analog_source(channel, Box::new(NtcThermistor::new(channel, 25.0)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ntc_at_25c() {
        let ntc = NtcThermistor::new(0, 25.0);
        // At 25 °C, R_ntc = R0 = 10 kΩ; divider is exactly V_ref/2 = 1650 mV.
        let mv = ntc.divider_output_mv();
        assert!(
            (mv as i32 - 1650).abs() <= 1,
            "expected ~1650 mV at 25°C, got {mv}"
        );
    }

    #[test]
    fn test_ntc_hot() {
        // At high temperature R_ntc drops; V_out should be > V_ref/2.
        let ntc_hot = NtcThermistor::new(0, 80.0);
        let ntc_cold = NtcThermistor::new(0, -10.0);
        assert!(ntc_hot.divider_output_mv() > ntc_cold.divider_output_mv());
    }

    #[test]
    fn test_adc_count_midpoint() {
        let ntc = NtcThermistor::new(0, 25.0);
        let count = ntc.adc_count();
        // ~4095/2 = 2047 at midpoint
        assert!(
            (count as i32 - 2047).abs() <= 2,
            "expected ~2047 at 25°C, got {count}"
        );
    }
}
