// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::any::Any;

/// Single-cell LiPo battery behind a TP4056-class analog charge controller,
/// read through a resistor divider on an ADC channel.
///
/// This is the classic "battery gauge on a spare ADC pin" a firmware uses to
/// show a charge percentage. The controller is *analog* — no I²C, no register
/// file. The one thing the MCU can observe is the battery terminal voltage,
/// scaled down through a divider so a 4.2 V pack stays inside the 3.3 V ADC
/// window. The firmware reads that voltage and maps it back to a state of
/// charge.
///
/// Terminal-voltage model (a linear approximation of the LiPo discharge
/// curve — good enough for a gauge, and deterministic):
///   - SoC   0 % → 3300 mV (empty cutoff)
///   - SoC 100 % → 4200 mV (full)
///   - linear between the two.
///
/// When the charger is connected (`usb_present`) the terminal sits a little
/// higher under charge — a fixed +150 mV bump, clamped at the 4200 mV ceiling.
///
/// ADC-pin voltage: the pack is divided by two before reaching the pin, so the
/// firmware sees `battery_mv / 2`. All of this math lives here in Rust core;
/// the WASM bridge passes SoC / USB state in and reads mV + ADC count out.
#[derive(Debug, serde::Serialize)]
pub struct LipoCharger {
    /// ADC channel the (divided) battery voltage is wired to.
    channel: u8,
    /// Current state of charge, 0..100 %.
    soc_pct: f32,
    /// Whether the charger (USB) is connected. Adds the charging offset.
    usb_present: bool,
    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

/// Terminal voltage at empty cutoff (SoC 0 %), in mV.
const EMPTY_MV: f32 = 3300.0;
/// Terminal voltage when full (SoC 100 %), in mV.
const FULL_MV: f32 = 4200.0;
/// Extra terminal voltage seen while the charger is connected, in mV. A real
/// pack rises a little under charge current; TP4056 tops out around 4.2 V, so
/// this bump is clamped to `FULL_MV`.
const CHARGE_OFFSET_MV: f32 = 150.0;

/// Resistor-divider ratio between the battery terminal and the ADC pin.
///
/// A full LiPo (4.2 V) exceeds a 3.3 V ADC's input range, so the pack is
/// divided down before it reaches the pin. A ÷2 divider (two equal resistors)
/// maps 4.2 V → 2.1 V, comfortably inside the window. The firmware multiplies
/// the reading back by this ratio to recover the terminal voltage.
const DIVIDER: u16 = 2;

impl Default for LipoCharger {
    fn default() -> Self {
        Self::new(0, 50.0, false)
    }
}

impl LipoCharger {
    pub fn new(channel: u8, soc_pct: f32, usb_present: bool) -> Self {
        Self {
            channel,
            soc_pct,
            usb_present,
            component_id: None,
        }
    }

    pub fn set_soc_pct(&mut self, pct: f32) {
        self.soc_pct = pct;
    }

    pub fn soc_pct(&self) -> f32 {
        self.soc_pct
    }

    pub fn set_usb_present(&mut self, present: bool) {
        self.usb_present = present;
    }

    pub fn usb_present(&self) -> bool {
        self.usb_present
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// Battery terminal voltage in mV for the current SoC / charge state.
    ///
    /// This is the voltage across the pack *before* the divider — exposed so
    /// tests (and any consumer that cares about the physical battery, not the
    /// pin) can assert it directly. The divider math lives in [`Self::output_mv`].
    pub fn battery_mv(&self) -> u16 {
        // Linear SoC → voltage between the empty cutoff and full.
        let soc = (self.soc_pct / 100.0).clamp(0.0, 1.0);
        let mut mv = EMPTY_MV + (FULL_MV - EMPTY_MV) * soc;
        if self.usb_present {
            mv += CHARGE_OFFSET_MV;
        }
        mv.clamp(EMPTY_MV, FULL_MV) as u16
    }

    /// Voltage present at the ADC pin, i.e. the terminal voltage through the
    /// [`DIVIDER`]. This is what the firmware's ADC actually samples.
    pub fn adc_pin_mv(&self) -> u16 {
        self.battery_mv() / DIVIDER
    }

    /// Convert the ADC-pin voltage to a 12-bit ADC count (0..4095) for 3.3 V Vref.
    pub fn adc_count(&self) -> u16 {
        let mv = self.adc_pin_mv() as u32;
        ((mv * 4095) / 3300).min(4095) as u16
    }

    pub fn as_any(&self) -> &dyn Any {
        self
    }
    pub fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The two drivable channels: state of charge and charger-present. One table
/// backs BOTH the `SimInput` impl and the kit metadata, so the device schema
/// and the runtime API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "soc_pct",
        label: "State of charge",
        unit: "%",
        min: 0.0,
        max: 100.0,
    },
    crate::sim_input::InputChannel {
        key: "usb_present",
        label: "Charger connected",
        unit: "",
        min: 0.0,
        max: 1.0,
    },
];

impl crate::sim_input::SimInput for LipoCharger {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        // Only the two state inputs move; the divider / LiPo-curve math that
        // turns them into a pin voltage is unchanged and lives in `battery_mv`
        // / `adc_pin_mv`.
        match key {
            "soc_pct" => self.set_soc_pct(value as f32),
            // Boolean flag carried as 0/1: anything at/above the midpoint is on.
            "usb_present" => self.set_usb_present(value >= 0.5),
            // require_channel already rejected any other key.
            _ => unreachable!("require_channel accepted an unknown channel"),
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

impl crate::bus::sim_inputs::AnalogSource for LipoCharger {
    fn output_mv(&self) -> u16 {
        self.adc_pin_mv()
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct LipoChargerKit;
pub static LIPO_CHARGER_KIT: LipoChargerKit = LipoChargerKit;

static LIPO_CHARGER_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "lipo_charger",
    label: "LiPo battery + charger",
    summary: "Single-cell LiPo pack + TP4056-class analog charger on an ADC channel.",
    detail: "Analog battery gauge: the LiPo terminal voltage (3300 mV empty → 4200 mV full, \
             linear in state of charge) is read through a ÷2 resistor divider so a 4.2 V pack \
             stays inside the 3.3 V ADC range. Drive `soc_pct` (0..100 %) and `usb_present` \
             (0/1) at runtime; the ADC channel follows through the real divider math, and \
             `usb_present` adds a small charging offset clamped to 4200 mV. No I²C — the only \
             observable is the divided pin voltage.",
    transport: Transport::Analog,
    category: Category::Analog,
    config_keys: &[ConfigKey {
        name: "channel",
        ty: ConfigType::Int,
        doc: "ADC channel index (0..N) the divided battery voltage is wired to. Defaults to 0.",
    }],
    labs: &[],
};

impl PeripheralKit for LipoChargerKit {
    fn metadata(&self) -> &'static KitMetadata {
        &LIPO_CHARGER_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let channel = ctx.config_i64("channel").unwrap_or(0).clamp(0, 255) as u8;
        // Retained on the bus so `set_input("soc_pct"/"usb_present", …)` can
        // drive it; the pin level is seeded from the default half-charged,
        // unplugged state at attach.
        ctx.attach_analog_source(channel, Box::new(LipoCharger::new(channel, 50.0, false)))?;
        Ok(())
    }
}

// NOTE (follow-up): a real TP4056 also exposes a CHRG open-drain status pin,
// pulled LOW while charging (usb_present && soc_pct < 100) and released
// otherwise, which firmware often wires to a GPIO input. There is no clean
// kit-attach seam for a component to *drive* a GPIO input line today — the
// digital-device path (`bus.gpio_devices`, the `GpioDevice` trait + service
// hooks) is a heavier mechanism than `attach_analog_source`, so the
// charge-status GPIO is deliberately left out here. Add it once an
// `attach_gpio_source`-style helper exists on `AttachCtx`.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::sim_inputs::AnalogSource;
    use crate::sim_input::SimInput;

    #[test]
    fn test_empty_battery() {
        let bat = LipoCharger::new(0, 0.0, false);
        assert_eq!(
            bat.battery_mv(),
            3300,
            "expected 3300 mV terminal at 0% SoC"
        );
        assert_eq!(bat.adc_pin_mv(), 1650, "expected 1650 mV at pin (÷2) at 0%");
        assert_eq!(
            bat.output_mv(),
            1650,
            "output_mv is the divided pin voltage"
        );
    }

    #[test]
    fn test_full_battery() {
        let bat = LipoCharger::new(0, 100.0, false);
        assert_eq!(
            bat.battery_mv(),
            4200,
            "expected 4200 mV terminal at 100% SoC"
        );
        assert_eq!(
            bat.adc_pin_mv(),
            2100,
            "expected 2100 mV at pin (÷2) at 100%"
        );
    }

    #[test]
    fn test_midpoint_linear() {
        let bat = LipoCharger::new(0, 50.0, false);
        // 3300 + 900*0.5 = 3750 mV terminal.
        assert_eq!(
            bat.battery_mv(),
            3750,
            "expected 3750 mV terminal at 50% SoC"
        );
        assert_eq!(bat.adc_pin_mv(), 1875, "expected 1875 mV at pin at 50%");
    }

    #[test]
    fn test_usb_present_raises_voltage_and_clamps() {
        // Mid charge: 3300 + 900*0.5 = 3750, +150 offset = 3900 mV.
        let charging = LipoCharger::new(0, 50.0, true);
        let idle = LipoCharger::new(0, 50.0, false);
        assert!(
            charging.battery_mv() > idle.battery_mv(),
            "usb_present should raise the terminal voltage"
        );
        assert_eq!(
            charging.battery_mv(),
            3900,
            "expected 3750 + 150 mV under charge"
        );

        // Near-full + charge would exceed 4200; must clamp to the ceiling.
        let topping = LipoCharger::new(0, 95.0, true);
        assert_eq!(
            topping.battery_mv(),
            4200,
            "charge offset must clamp to the 4200 mV ceiling"
        );
    }

    #[test]
    fn test_adc_count_math() {
        // At 100% the pin sees 2100 mV; count = 2100*4095/3300 = 2605.
        let bat = LipoCharger::new(0, 100.0, false);
        assert_eq!(bat.adc_count(), (2100u32 * 4095 / 3300) as u16);
        assert_eq!(bat.adc_count(), 2605);
    }

    #[test]
    fn test_set_input_drives_output() {
        let mut bat = LipoCharger::default();

        // Drive SoC to full through the generic SimInput API.
        bat.set_input("soc_pct", 100.0).expect("soc_pct in range");
        assert_eq!(
            bat.output_mv(),
            2100,
            "set_input soc_pct should move the pin voltage"
        );

        // Drive the charger on and confirm the offset (clamped at full).
        bat.set_input("usb_present", 1.0)
            .expect("usb_present in range");
        assert!(bat.usb_present());
        assert_eq!(bat.battery_mv(), 4200, "full + charge clamps at 4200 mV");

        // Back down to empty and unplugged.
        bat.set_input("soc_pct", 0.0).expect("soc_pct in range");
        bat.set_input("usb_present", 0.0)
            .expect("usb_present in range");
        assert!(!bat.usb_present());
        assert_eq!(bat.output_mv(), 1650, "empty + unplugged → 1650 mV at pin");
    }

    #[test]
    fn test_set_input_rejects_out_of_range() {
        let mut bat = LipoCharger::default();
        assert!(
            bat.set_input("soc_pct", 150.0).is_err(),
            "150% is out of range"
        );
        assert!(
            bat.set_input("usb_present", 5.0).is_err(),
            "usb_present > 1 is out of range"
        );
        assert!(
            bat.set_input("bogus", 1.0).is_err(),
            "unknown channel rejected"
        );
    }
}
