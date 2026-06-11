// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! I2C **binding adapter** over the bus-agnostic [`IrCore`] engine.
//!
//! This type holds only the I2C binding (the device address) and translates
//! the [`I2cDevice`] protocol onto `IrCore`'s bus-neutral primitives
//! (`reset_frame` / `write` / `read`). All device *behavior* lives in
//! [`IrCore`]; see that module for the engine and the generic design.

use crate::peripherals::components::ir_core::IrCore;
use crate::peripherals::i2c::I2cDevice;
use labwired_ir::component::{IrComponent, IrComponentInterface};

/// An [`IrComponent`] bound to an I2C bus.
pub struct IrI2cComponent {
    core: IrCore,
    addr: u8,
}

impl IrI2cComponent {
    /// Build from a spec with an `i2c` interface. `address_override` comes from
    /// the manifest's `i2c_address` config when present. Returns `Err` if the
    /// spec is not I2C-bound or fails validation.
    pub fn new(spec: IrComponent, address_override: Option<u8>) -> Result<Self, String> {
        let IrComponentInterface::I2c { default_address } = &spec.interface else {
            return Err("IrI2cComponent requires an i2c interface".to_string());
        };
        let addr = address_override.unwrap_or(*default_address);
        let core = IrCore::new(spec)?;
        Ok(Self { core, addr })
    }

    /// Read a named observable for a specific channel. Delegates to
    /// [`IrCore::observable`]; returns `None` for an unknown name, an
    /// out-of-range channel, or a `none_when_raw_zero` observable reading 0.
    pub fn observable(&self, name: &str, channel: u8) -> Option<f32> {
        self.core.observable(name, channel)
    }
}

impl I2cDevice for IrI2cComponent {
    fn address(&self) -> u8 {
        self.addr
    }

    fn start(&mut self) {
        self.core.reset_frame();
    }

    fn write(&mut self, data: u8) {
        self.core.write(data);
    }

    fn read(&mut self) -> u8 {
        self.core.read()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pca_like_spec() -> IrComponent {
        serde_yaml::from_str(
            r#"
name: PCA-like
interface: { i2c: { default_address: 0x40 } }
register_file:
  size: 256
  reset: { 0x00: 0x11 }
pointer:
  first_write_after_start_sets_pointer: true
  auto_increment:
    when_field_set: { reg: 0x00, mask: 0x20 }
"#,
        )
        .unwrap()
    }

    #[test]
    fn rejects_invalid_spec() {
        let mut s = pca_like_spec();
        s.register_file.reset.insert(999, 1);
        assert!(IrI2cComponent::new(s, None).is_err());
    }

    #[test]
    fn address_default_and_override() {
        let d = IrI2cComponent::new(pca_like_spec(), None).unwrap();
        assert_eq!(d.address(), 0x40);
        let d = IrI2cComponent::new(pca_like_spec(), Some(0x41)).unwrap();
        assert_eq!(d.address(), 0x41);
    }

    #[test]
    fn reset_value_reads_back_via_pointer() {
        let mut d = IrI2cComponent::new(pca_like_spec(), None).unwrap();
        d.start();
        d.write(0x00); // pointer = MODE1
        assert_eq!(d.read(), 0x11);
    }

    #[test]
    fn auto_increment_block_write_lands_consecutively() {
        let mut d = IrI2cComponent::new(pca_like_spec(), None).unwrap();
        // Enable AI: MODE1 |= 0x20 (write 0xA1 like the firmware does).
        d.start();
        d.write(0x00);
        d.write(0xA1);
        // 4-byte block write at 0x06.
        d.start();
        d.write(0x06);
        for v in [0xDE, 0xAD, 0xBE, 0xEF] {
            d.write(v);
        }
        d.start();
        d.write(0x06);
        assert_eq!(
            [d.read(), d.read(), d.read(), d.read()],
            [0xDE, 0xAD, 0xBE, 0xEF]
        );
    }

    #[test]
    fn without_ai_pointer_stays_put() {
        let mut d = IrI2cComponent::new(pca_like_spec(), None).unwrap();
        d.start();
        d.write(0x06);
        d.write(0x01);
        d.write(0x02); // overwrites same register: AI is off (MODE1=0x11)
        d.start();
        d.write(0x06);
        assert_eq!(d.read(), 0x02);
    }

    // ── Part B1: absorb data writes when pointer selects a wide register ──────

    /// TMP102-shaped spec: size=4, pointer_mask=0x03, wide registers at all
    /// four pointers with distinct reset values.
    fn tmp102_like_spec() -> IrComponent {
        serde_yaml::from_str(
            r#"
name: TMP102-like
interface: { i2c: { default_address: 0x48 } }
register_file:
  size: 4
  reset: {}
pointer:
  first_write_after_start_sets_pointer: true
  pointer_mask: 0x03
  auto_increment: never
wide_registers:
  - { pointer: 0, bits: 16, reset: 0x1900 }
  - { pointer: 1, bits: 16, reset: 0x60A0 }
  - { pointer: 2, bits: 16, reset: 0x4B00 }
  - { pointer: 3, bits: 16, reset: 0x5000 }
"#,
        )
        .unwrap()
    }

    #[test]
    fn wide_pointer_data_write_is_absorbed_not_stored() {
        let mut d = IrI2cComponent::new(tmp102_like_spec(), None).unwrap();
        // Select config register (pointer 1, reset 0x60A0).
        d.start();
        d.write(0x01); // pointer select
        d.write(0xFF); // config byte — must be absorbed, not stored
                       // Read config MSB+LSB; should equal reset 0x60A0.
        d.start();
        d.write(0x01);
        let msb = d.read();
        let lsb = d.read();
        assert_eq!(
            (msb, lsb),
            (0x60, 0xA0),
            "config corrupted by absorbed write: got {msb:#04x} {lsb:#04x}"
        );
        // Temperature register (pointer 0) also unaffected.
        d.start();
        d.write(0x00);
        let t_msb = d.read();
        let t_lsb = d.read();
        assert_eq!(
            (t_msb, t_lsb),
            (0x19, 0x00),
            "temperature corrupted: {t_msb:#04x} {t_lsb:#04x}"
        );
    }

    #[test]
    fn wide_pointer_absorb_still_counts_writes_since_start() {
        // writes_since_start must increment even for absorbed bytes so that a
        // re-select (a new pointer byte) after a START is still treated as the
        // first write.
        let mut d = IrI2cComponent::new(tmp102_like_spec(), None).unwrap();
        d.start();
        d.write(0x01); // pointer → 1; writes_since_start = 1
        d.write(0xFF); // absorbed;   writes_since_start = 2
                       // A new START resets writes_since_start; next write sets pointer again.
        d.start();
        d.write(0x00); // pointer → 0 (temperature)
        let msb = d.read();
        let lsb = d.read();
        assert_eq!(
            (msb, lsb),
            (0x19, 0x00),
            "temperature wrong after re-select: {msb:#04x} {lsb:#04x}"
        );
    }

    // ── Part B2: wide storage as u16, signed semantics in AddWrap only ────────

    fn wide_signed_spec() -> IrComponent {
        // A single wide register with reset 0xE700 (negative as i16 = -6400).
        // AddWrap: add=0x80, max=0x2300, reset=0x1400.
        // First add: (0xE700 as i16) + 0x80 = -6400 + 128 = -6272 = 0xE780 as u16.
        // -6272 > 0x2300 (8960)? No (signed: -6272 < 8960) → no reset triggered.
        serde_yaml::from_str(
            r#"
name: signed-wide
interface: { i2c: { default_address: 0x10 } }
register_file:
  size: 4
  reset: {}
pointer:
  first_write_after_start_sets_pointer: true
  pointer_mask: 0x03
  auto_increment: never
wide_registers:
  - { pointer: 0, bits: 16, reset: 0xE700 }
updates:
  - trigger: { wide_read_complete: { pointer: 0 } }
    action:
      add_wrap: { add: 128, max: 8960, reset: 5120 }
"#,
        )
        .unwrap()
    }

    #[test]
    fn wide_stored_as_u16_signed_addwrap_no_false_reset() {
        let mut d = IrI2cComponent::new(wide_signed_spec(), None).unwrap();
        // First read: should return reset bytes 0xE7, 0x00.
        d.start();
        d.write(0x00);
        let b0 = d.read(); // MSB triggers update after LSB
        let b1 = d.read(); // LSB → update fires: (0xE700 as i16) + 128 = 0xE780
        assert_eq!(
            (b0, b1),
            (0xE7, 0x00),
            "first read wrong: {b0:#04x} {b1:#04x}"
        );
        // Second read: post-update value 0xE780 → bytes 0xE7, 0x80.
        d.start();
        d.write(0x00);
        let b2 = d.read();
        let b3 = d.read();
        assert_eq!(
            (b2, b3),
            (0xE7, 0x80),
            "second read wrong (signed wrap check): {b2:#04x} {b3:#04x}"
        );
    }

    // ── Part A: observable() ──────────────────────────────────────────────────

    /// PCA9685-shaped spec with servo_angle observable.
    fn pca_like_with_observable() -> IrComponent {
        serde_yaml::from_str(
            r#"
name: PCA-obs
interface: { i2c: { default_address: 0x40 } }
register_file:
  size: 256
  reset: { 0x00: 0x11 }
pointer:
  first_write_after_start_sets_pointer: true
  auto_increment:
    when_field_set: { reg: 0x00, mask: 0x20 }
observables:
  - name: servo_angle
    channels: 16
    base: 0x06
    stride: 4
    value:
      u12_compose: { lo_rel: 2, hi_rel: 3, hi_mask: 0x0F }
    map:
      linear: { scale: 0.46258224, offset: -47.368423, clamp: [0.0, 180.0] }
      none_when_raw_zero: true
"#,
        )
        .unwrap()
    }

    /// Write a 12-bit ON_TIME=0, OFF_TIME=raw to channel `ch` of a PCA9685-like
    /// device (registers at base=0x06, stride=4: OFF_L at +2, OFF_H at +3).
    fn ir_set_angle(d: &mut IrI2cComponent, ch: u8, raw: u16) {
        // Enable AI in MODE1 first (bit 5).
        d.start();
        d.write(0x00); // pointer
        d.write(0xA1); // MODE1: ALLCALL | AI
                       // Write all 4 bytes of the channel block.
        let base = 0x06u8 + ch * 4;
        d.start();
        d.write(base);
        d.write(0x00); // ON_L
        d.write(0x00); // ON_H
        d.write((raw & 0xFF) as u8); // OFF_L  (+2)
        d.write(((raw >> 8) & 0x0F) as u8); // OFF_H  (+3, hi_mask=0x0F)
    }

    #[test]
    fn observable_none_before_write_then_tracks_angle() {
        let mut d = IrI2cComponent::new(pca_like_with_observable(), None).unwrap();
        d.start();
        // Before any write, channel 0 regs are 0 → none_when_raw_zero → None.
        assert_eq!(d.observable("servo_angle", 0), None);

        // Write a specific raw value for channel 0.
        // raw 135 → angle = 135 * 0.46258224 + (-47.368423) ≈ 62.449 - 47.368 ≈ 15.080°
        ir_set_angle(&mut d, 0, 135);
        let angle = d
            .observable("servo_angle", 0)
            .expect("should be Some after write");
        assert!((angle - 15.0).abs() < 0.1, "expected ~15°, got {angle:.3}");

        // Other channels still None.
        assert_eq!(d.observable("servo_angle", 1), None);

        // Last channel (15) also independent.
        ir_set_angle(&mut d, 15, 135);
        let angle15 = d
            .observable("servo_angle", 15)
            .expect("ch15 should be Some");
        assert!(
            (angle15 - 15.0).abs() < 0.1,
            "ch15 expected ~15°, got {angle15:.3}"
        );
    }

    #[test]
    fn observable_unknown_name_or_channel_is_none() {
        let mut d = IrI2cComponent::new(pca_like_with_observable(), None).unwrap();
        ir_set_angle(&mut d, 0, 135);
        // Unknown name.
        assert_eq!(d.observable("no_such", 0), None);
        // Out-of-range channel (channels=16, so 16 is invalid).
        assert_eq!(d.observable("servo_angle", 16), None);
    }
}
