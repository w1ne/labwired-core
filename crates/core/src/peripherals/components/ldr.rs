// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::any::Any;

/// Light-dependent resistor (photoresistor / CdS cell) in a pull-down voltage
/// divider on an ADC channel.
///
/// A photoresistor's resistance FALLS as illuminance rises. Wired as the top
/// leg of a divider to `Vref` with a fixed resistor to ground, the ADC node
/// therefore rises with light:
///
/// Resistance model: R(lux) = R_10lux * (lux / 10)^(-gamma)
///   - R_10lux = 10 kΩ  (typical GL5528-class cell resistance at 10 lx)
///   - gamma   = 0.7     (log-log slope of a CdS cell; datasheet 0.5..0.9)
///
/// Voltage divider (LDR on top to Vref, fixed pull-down to ground):
///   V_out = V_ref * R_fixed / (R_ldr + R_fixed)
///   - V_ref   = 3.3 V (3300 mV)
///   - R_fixed = 10 kΩ
///
/// Monotonicity: brighter → lower R_ldr → higher V_out. So
///   - DARK  (lux → 0)      ⇒ R_ldr → ∞ ⇒ V_out → 0 mV  (ground rail)
///   - BRIGHT (lux → large) ⇒ R_ldr → 0 ⇒ V_out → Vref  (upper rail)
///
/// All divider / power-law math lives here in Rust core. The WASM bridge passes
/// lux in and reads mV + ADC count out; it never reimplements this.
#[derive(Debug, serde::Serialize)]
pub struct Ldr {
    /// ADC channel this photoresistor divider is wired to.
    channel: u8,
    /// Current illuminance in lux.
    lux: f32,
    /// Calibration constants.
    r_10lux_ohm: f32, // 10 000.0
    gamma: f32,       // 0.7
    r_fixed_ohm: f32, // 10 000.0
    v_ref_mv: f32,    // 3300.0
    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Default for Ldr {
    fn default() -> Self {
        Self::new(0, 100.0)
    }
}

impl Ldr {
    pub fn new(channel: u8, lux: f32) -> Self {
        Self {
            channel,
            lux,
            r_10lux_ohm: 10_000.0,
            gamma: 0.7,
            r_fixed_ohm: 10_000.0,
            v_ref_mv: 3300.0,
            component_id: None,
        }
    }

    pub fn set_lux(&mut self, lux: f32) {
        self.lux = lux;
    }

    pub fn lux(&self) -> f32 {
        self.lux
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// Compute the divider output in mV for the current illuminance.
    ///
    /// This is the photoresistor power-law + voltage-divider math — it lives
    /// here in Rust core. The WASM bridge and UI never reimplement this.
    pub fn divider_output_mv(&self) -> u16 {
        // R(lux) = R_10lux * (lux/10)^(-gamma). As lux → 0 the ratio → 0 and
        // powf(0, -gamma) → +∞, so R_ldr → ∞ and V_out → 0 with no NaN.
        let ratio = (self.lux / 10.0).max(0.0);
        let r_ldr = self.r_10lux_ohm * ratio.powf(-self.gamma);
        // Voltage divider: V_out = V_ref * R_fixed / (R_ldr + R_fixed).
        let v_out = self.v_ref_mv * self.r_fixed_ohm / (r_ldr + self.r_fixed_ohm);
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

/// The sensed illuminance, in lux. Range covers pitch dark (0 lx) up to bright
/// direct sunlight (~100 000 lx) — the usable span of a CdS photoresistor. Key
/// / label / unit mirror the VEML7700 lux channel so the two light sensors
/// speak the same vocabulary. One table backs BOTH the `SimInput` impl and the
/// kit metadata, so the device schema and the runtime API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[crate::sim_input::InputChannel {
    key: "lux",
    label: "Illuminance",
    unit: "lx",
    min: 0.0,
    max: 100000.0,
}];

impl crate::sim_input::SimInput for Ldr {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        // Only the illuminance moves; the power-law resistance and the divider
        // that turns it into a voltage are unchanged and still live in
        // `divider_output_mv`.
        self.set_lux(value as f32);
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

impl crate::bus::sim_inputs::AnalogSource for Ldr {
    fn output_mv(&self) -> u16 {
        self.divider_output_mv()
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct LdrKit;
pub static LDR_KIT: LdrKit = LdrKit;

static LDR_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "ldr",
    label: "Photoresistor (LDR)",
    summary: "CdS photoresistor + voltage divider on an ADC channel.",
    detail: "Light-dependent-resistor model (R = 10 kΩ at 10 lx, gamma = 0.7) as the top leg of \
             a 10 kΩ pull-down divider. Drive the `lux` channel at runtime and the ADC channel \
             follows through the real resistance→voltage math: dark reads near 0 V, bright reads \
             toward Vref. Starts at 100 lx (dim indoor light).",
    transport: Transport::Analog,
    category: Category::Analog,
    config_keys: &[ConfigKey {
        name: "channel",
        ty: ConfigType::Int,
        doc: "ADC channel index (0..N). Defaults to 0.",
    }],
    labs: &[],
};

impl PeripheralKit for LdrKit {
    fn metadata(&self) -> &'static KitMetadata {
        &LDR_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let channel = ctx.config_i64("channel").unwrap_or(0).clamp(0, 255) as u8;
        // Retained on the bus so `set_input("lux", …)` can drive it; the divider
        // level is seeded from the 100 lx default at attach.
        ctx.attach_analog_source(channel, Box::new(Ldr::new(channel, 100.0)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_input::SimInput;

    #[test]
    fn test_ldr_monotonic() {
        // Drive three lux levels through the SimInput path and assert the ADC
        // output rises monotonically with light (dark < dim < bright).
        let mut dark = Ldr::new(0, 100.0);
        dark.set_input("lux", 1.0).unwrap();
        let mut dim = Ldr::new(0, 100.0);
        dim.set_input("lux", 100.0).unwrap();
        let mut bright = Ldr::new(0, 100.0);
        bright.set_input("lux", 10_000.0).unwrap();
        assert!(
            dark.divider_output_mv() < dim.divider_output_mv(),
            "expected dark ({}) < dim ({})",
            dark.divider_output_mv(),
            dim.divider_output_mv()
        );
        assert!(
            dim.divider_output_mv() < bright.divider_output_mv(),
            "expected dim ({}) < bright ({})",
            dim.divider_output_mv(),
            bright.divider_output_mv()
        );
    }

    #[test]
    fn test_ldr_set_input_updates_lux() {
        let mut ldr = Ldr::new(0, 100.0);
        ldr.set_input("lux", 500.0).unwrap();
        assert_eq!(
            ldr.lux(),
            500.0,
            "set_input('lux') must update the lux state"
        );
    }

    #[test]
    fn test_ldr_dark_and_bright_rails() {
        // Dark → ground rail (near 0 mV); very bright → toward Vref.
        let dark = Ldr::new(0, 0.0);
        assert_eq!(dark.divider_output_mv(), 0, "pitch dark should read ~0 mV");
        let bright = Ldr::new(0, 100_000.0);
        assert!(
            bright.divider_output_mv() > 3000,
            "bright sunlight should read near Vref, got {}",
            bright.divider_output_mv()
        );
    }
}
