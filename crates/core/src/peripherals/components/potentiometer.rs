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

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct PotentiometerKit;
pub static POTENTIOMETER_KIT: PotentiometerKit = PotentiometerKit;

static POTENTIOMETER_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "potentiometer",
    label: "Potentiometer",
    summary: "3-pin linear potentiometer on an ADC channel (rotary or slide).",
    detail: "Linear voltage-divider model: the wiper taps off Vref * position/100. \
             Constructor takes a channel + initial wiper position; the kit seeds the \
             ADC channel with the corresponding wiper voltage. Live updates come from \
             the WASM bridge as the host moves the position slider. A rotary pot and a \
             slide pot are electrically identical and share this model.",
    transport: Transport::Analog,
    category: Category::Analog,
    config_keys: &[
        ConfigKey {
            name: "channel",
            ty: ConfigType::Int,
            doc: "ADC channel index (0..N). Defaults to 0.",
        },
        ConfigKey {
            name: "initial_position_pct",
            ty: ConfigType::Float,
            doc: "Initial wiper position 0..100 %. Defaults to 50.0.",
        },
    ],
    labs: &[],
};

impl PeripheralKit for PotentiometerKit {
    fn metadata(&self) -> &'static KitMetadata {
        &POTENTIOMETER_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let channel = ctx.config_i64("channel").unwrap_or(0).clamp(0, 255) as u8;
        let position_pct = ctx.config_f64("initial_position_pct").unwrap_or(50.0) as f32;
        // Compute the wiper voltage up front; the Potentiometer instance itself
        // is not stored — the bus seeds the ADC channel once and the wasm bridge
        // mutates that channel directly on host stimulus.
        let mv = Potentiometer::new(channel, position_pct).wiper_output_mv();
        let adc = ctx.adc()?;
        adc.set_channel_input(channel, mv);
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
