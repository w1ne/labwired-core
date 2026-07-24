// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SPI **binding adapter** over the bus-agnostic [`IrCore`] engine.
//!
//! Holds only the SPI binding (the chip-select pin) and translates the
//! [`SpiDevice`] protocol onto `IrCore`'s bus-neutral primitives. For a
//! read-only response-word device, `cs_select`/`cs_release` map to
//! `reset_frame` and each `transfer` clocks out one `read` byte (MOSI is
//! ignored). All behavior lives in [`IrCore`]; lets agents model read-only SPI
//! sensors/ADCs (MAX31855, MAX6675, MCP3xxx, …) declaratively.

use crate::peripherals::components::ir_core::IrCore;
use crate::peripherals::spi::SpiDevice;
use labwired_ir::component::{IrComponent, IrComponentInterface};
use std::any::Any;

/// An [`IrComponent`] bound to an SPI bus.
pub struct IrSpiComponent {
    core: IrCore,
    cs_pin: String,
}

impl IrSpiComponent {
    /// Build from a spec with an `spi` interface. `cs_override` comes from the
    /// manifest's `cs_pin` config when present. Returns `Err` if the spec is
    /// not SPI-bound or fails validation.
    pub fn new(spec: IrComponent, cs_override: Option<String>) -> Result<Self, String> {
        let IrComponentInterface::Spi { default_cs_pin } = &spec.interface else {
            return Err("IrSpiComponent requires an spi interface".to_string());
        };
        let cs_pin = cs_override.unwrap_or_else(|| default_cs_pin.clone());
        let core = IrCore::new(spec)?;
        Ok(Self { core, cs_pin })
    }

    /// Set a response field's raw value (host stimulus). Delegates to
    /// [`IrCore::set_field`]; returns `false` for an unknown field.
    pub fn set_field(&mut self, name: &str, value: i64) -> bool {
        self.core.set_field(name, value)
    }

    /// Read back a response field's raw value, or `None` if unknown.
    pub fn field(&self, name: &str) -> Option<i64> {
        self.core.field(name)
    }
}

impl SpiDevice for IrSpiComponent {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        self.core.reset_frame();
    }

    fn cs_release(&mut self) {
        self.core.reset_frame();
    }

    fn transfer(&mut self, _mosi: u8) -> u8 {
        // Read-only device: MOSI carries no state; clock out the next word byte.
        self.core.read()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX31855_YAML: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/components/max31855.yaml"
    );

    fn max31855_spec() -> IrComponent {
        let yaml = std::fs::read_to_string(MAX31855_YAML).expect("read max31855.yaml");
        serde_yaml::from_str(&yaml).expect("parse max31855.yaml")
    }

    /// Clock one full CS-framed response word out of any SPI device.
    fn read_frame(dev: &mut dyn SpiDevice, n: usize) -> Vec<u8> {
        dev.cs_select();
        let out: Vec<u8> = (0..n).map(|_| dev.transfer(0x00)).collect();
        dev.cs_release();
        out
    }

    // ── The equivalence gate: IR spec must clock out byte-for-byte identically
    //    to the MAX31855 datasheet's 32-bit frame formula across diverse
    //    states. ──────────────────────────────────────────────────────────────
    #[test]
    fn ir_spi_matches_datasheet_word() {
        // Cases: (tc_q14, internal_q12, fault). Defaults, positive, negative,
        // fault-set, and field saturation.
        let cases: &[(i32, i32, bool)] = &[
            (100, 352, false),     // power-on default (25.0 / 22.0 °C)
            (400, 400, false),     // 100.0 / 25.0 °C
            (-100, 296, false),    // -25.0 / 18.5 °C  (negative thermocouple)
            (100, 352, true),      // fault asserted
            (-2048, -2048, false), // extreme negatives (sign-bit set in both fields)
            (8191, 2047, true),    // wide positives + fault
        ];
        for &(tc, int, fault) in cases {
            // Reference: MAX31855 datasheet 32-bit frame formula, masked to
            // field widths to reproduce two's-complement wrap exactly as the
            // packed hardware word does.
            let word: u32 = ((tc as u32 & 0x3FFF) << 18)
                | ((fault as u32 & 0x1) << 16)
                | ((int as u32 & 0x0FFF) << 4);
            let expected_bytes = word.to_be_bytes().to_vec();

            // Subject: declarative IR interpreter.
            let mut ir = IrSpiComponent::new(max31855_spec(), None).expect("build ir spi");
            assert!(ir.set_field("tc_temp", tc as i64));
            assert!(ir.set_field("internal_temp", int as i64));
            assert!(ir.set_field("fault", fault as i64));

            let ir_bytes = read_frame(&mut ir, 4);
            assert_eq!(
                ir_bytes, expected_bytes,
                "mismatch for (tc={tc}, int={int}, fault={fault}): ir={ir_bytes:02X?} expected={expected_bytes:02X?}"
            );
        }
    }

    #[test]
    fn defaults_come_from_spec_reset() {
        let ir = IrSpiComponent::new(max31855_spec(), None).unwrap();
        assert_eq!(ir.field("tc_temp"), Some(100));
        assert_eq!(ir.field("internal_temp"), Some(352));
        assert_eq!(ir.field("fault"), Some(0));
        assert_eq!(ir.cs_pin(), "PA4");
    }

    #[test]
    fn cs_override_replaces_default_pin() {
        let ir = IrSpiComponent::new(max31855_spec(), Some("PB12".to_string())).unwrap();
        assert_eq!(ir.cs_pin(), "PB12");
    }

    #[test]
    fn byte_index_resets_on_cs_and_word_repeats() {
        let mut ir = IrSpiComponent::new(max31855_spec(), None).unwrap();
        let first = read_frame(&mut ir, 4);
        let second = read_frame(&mut ir, 4);
        assert_eq!(first, second, "read-only word must repeat each CS frame");
        // Over-clocking past the word width keeps returning the last byte.
        ir.cs_select();
        let b: Vec<u8> = (0..6).map(|_| ir.transfer(0)).collect();
        assert_eq!(b[3], b[4], "over-clock holds last byte");
        assert_eq!(b[4], b[5]);
    }

    #[test]
    fn determinism_two_instances_agree() {
        let mut a = IrSpiComponent::new(max31855_spec(), None).unwrap();
        let mut b = IrSpiComponent::new(max31855_spec(), None).unwrap();
        a.set_field("tc_temp", 1234);
        b.set_field("tc_temp", 1234);
        assert_eq!(read_frame(&mut a, 4), read_frame(&mut b, 4));
    }

    #[test]
    fn set_unknown_field_is_rejected() {
        let mut ir = IrSpiComponent::new(max31855_spec(), None).unwrap();
        assert!(!ir.set_field("no_such_field", 1));
        assert_eq!(ir.field("no_such_field"), None);
    }

    #[test]
    fn rejects_i2c_spec() {
        let i2c: IrComponent = serde_yaml::from_str(
            r#"
name: tmp102-ish
interface: { i2c: { default_address: 0x48 } }
register_file: { size: 4 }
"#,
        )
        .unwrap();
        assert!(IrSpiComponent::new(i2c, None).is_err());
    }

    #[test]
    fn rejects_spi_spec_without_response() {
        let bad: IrComponent = serde_yaml::from_str(
            r#"
name: no-response
interface: { spi: { default_cs_pin: PA4 } }
"#,
        )
        .unwrap();
        match IrSpiComponent::new(bad, None) {
            Ok(_) => panic!("spi spec without response should be rejected"),
            Err(err) => assert!(err.contains("ICOMP_SPI_NO_RESPONSE"), "got: {err}"),
        }
    }
}
