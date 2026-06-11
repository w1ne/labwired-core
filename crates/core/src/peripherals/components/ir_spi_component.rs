// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Deterministic interpreter executing a read-only SPI [`IrComponent`] as an
//! [`SpiDevice`]. Pure state machine over the spec: the device clocks out a
//! fixed-width big-endian word (composed from named bit-fields) MSB-first on
//! every CS-framed transaction. No host time, no randomness, no I/O.
//!
//! This is the SPI sibling of
//! [`IrI2cComponent`](crate::peripherals::components::ir_component::IrI2cComponent)
//! and lets agents model read-only SPI sensors/ADCs (MAX31855, MAX6675,
//! MCP3xxx, …) declaratively instead of hand-writing a Rust device.

use crate::peripherals::spi::SpiDevice;
use labwired_ir::component::{IrComponent, IrComponentInterface};
use std::any::Any;

/// Mutable per-field state, parallel to the spec's `response.fields`.
struct FieldState {
    name: String,
    shift: u8,
    bits: u8,
    /// Raw value in field units; host stimulus mutates this via [`set_field`].
    ///
    /// [`set_field`]: IrSpiComponent::set_field
    value: i64,
}

/// Interpreter for one read-only SPI component instance.
pub struct IrSpiComponent {
    cs_pin: String,
    /// Word width in bytes (1..=8), validated at construction.
    bytes: u8,
    fields: Vec<FieldState>,
    /// Position within the response word (0..bytes-1). Reset on CS edges.
    byte_index: u8,
}

impl IrSpiComponent {
    /// Build an interpreter from a validated spec. `cs_override` comes from the
    /// manifest's `cs_pin` config when present.
    pub fn new(spec: IrComponent, cs_override: Option<String>) -> Result<Self, String> {
        let diags = spec.validate();
        if !diags.is_empty() {
            return Err(diags
                .iter()
                .map(|d| format!("{}: {}", d.code, d.message))
                .collect::<Vec<_>>()
                .join("; "));
        }
        let IrComponentInterface::Spi { default_cs_pin } = &spec.interface else {
            return Err("IrSpiComponent requires an spi interface".to_string());
        };
        // validate() guarantees response is present for the spi interface.
        let resp = spec
            .response
            .as_ref()
            .ok_or_else(|| "spi spec missing response".to_string())?;
        let fields = resp
            .fields
            .iter()
            .map(|f| FieldState {
                name: f.name.clone(),
                shift: f.shift,
                bits: f.bits,
                value: f.reset,
            })
            .collect();
        Ok(Self {
            cs_pin: cs_override.unwrap_or_else(|| default_cs_pin.clone()),
            bytes: resp.bytes,
            fields,
            byte_index: 0,
        })
    }

    /// Compose the current response word from field state.
    fn compose_word(&self) -> u64 {
        let mut word = 0u64;
        for f in &self.fields {
            // Low `bits` bits of the (possibly negative) value — two's-complement
            // truncation matches a hand-written `(v as uN) & mask` exactly.
            let mask = if f.bits >= 64 {
                u64::MAX
            } else {
                (1u64 << f.bits) - 1
            };
            word |= ((f.value as u64) & mask) << f.shift;
        }
        word
    }

    /// Set a field's raw value (host stimulus). Returns `false` for an unknown
    /// field name.
    pub fn set_field(&mut self, name: &str, value: i64) -> bool {
        if let Some(f) = self.fields.iter_mut().find(|f| f.name == name) {
            f.value = value;
            true
        } else {
            false
        }
    }

    /// Read back a field's current raw value, or `None` if the name is unknown.
    pub fn field(&self, name: &str) -> Option<i64> {
        self.fields.iter().find(|f| f.name == name).map(|f| f.value)
    }
}

impl SpiDevice for IrSpiComponent {
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
        let word = self.compose_word();
        let last = self.bytes - 1; // bytes >= 1 by validation
        let idx = self.byte_index.min(last);
        // MSB-first: byte 0 is the most-significant byte of the word.
        let shift = 8 * (last - idx) as u32;
        let byte = ((word >> shift) & 0xFF) as u8;
        if self.byte_index < last {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::components::max31855::Max31855;

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
    //    to the independent hand-written Max31855 across diverse states. ────────
    #[test]
    fn ir_spi_matches_handwritten_max31855() {
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
            // Reference: hand-written model.
            let mut hand = Max31855::new("PA4");
            hand.tc_temp_q14 = tc;
            hand.internal_temp_q12 = int;
            hand.fault = fault;

            // Subject: declarative IR interpreter.
            let mut ir = IrSpiComponent::new(max31855_spec(), None).expect("build ir spi");
            assert!(ir.set_field("tc_temp", tc as i64));
            assert!(ir.set_field("internal_temp", int as i64));
            assert!(ir.set_field("fault", fault as i64));

            let hand_bytes = read_frame(&mut hand, 4);
            let ir_bytes = read_frame(&mut ir, 4);
            assert_eq!(
                ir_bytes, hand_bytes,
                "mismatch for (tc={tc}, int={int}, fault={fault}): ir={ir_bytes:02X?} hand={hand_bytes:02X?}"
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
