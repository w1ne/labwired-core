// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Deterministic interpreter executing an [`labwired_ir::component::IrComponent`]
//! as an [`I2cDevice`]. Pure state machine over the spec: no host time, no
//! randomness, no I/O — determinism is preserved by construction.

use crate::peripherals::i2c::I2cDevice;
use labwired_ir::component::{
    IrAutoIncrement, IrComponent, IrComponentInterface, IrObservableValue, IrUpdateAction,
    IrUpdateTrigger,
};

/// Interpreter state for one component instance.
pub struct IrI2cComponent {
    spec: IrComponent,
    addr: u8,
    regs: Vec<u8>,
    wide: Vec<u16>, // parallel to spec.wide_registers; raw 16-bit register image
    pointer: u8,
    read_phase: u8, // 0 = MSB next (wide reads); reset on START
    writes_since_start: u32,
}

impl IrI2cComponent {
    /// Build an interpreter from a validated spec. `address_override` comes
    /// from the manifest's `i2c_address` config when present.
    pub fn new(spec: IrComponent, address_override: Option<u8>) -> Result<Self, String> {
        let diags = spec.validate();
        if !diags.is_empty() {
            return Err(diags
                .iter()
                .map(|d| format!("{}: {}", d.code, d.message))
                .collect::<Vec<_>>()
                .join("; "));
        }
        let IrComponentInterface::I2c { default_address } = &spec.interface;
        let default_address = *default_address;
        let mut regs = vec![0u8; spec.register_file.size];
        for (&off, &v) in &spec.register_file.reset {
            regs[off as usize] = v;
        }
        let wide = spec.wide_registers.iter().map(|w| w.reset).collect();
        Ok(Self {
            addr: address_override.unwrap_or(default_address),
            regs,
            wide,
            pointer: 0,
            read_phase: 0,
            writes_since_start: 0,
            spec,
        })
    }

    fn auto_increment_enabled(&self) -> bool {
        match self.spec.pointer.as_ref().map(|p| &p.auto_increment) {
            Some(IrAutoIncrement::Always) => true,
            Some(IrAutoIncrement::WhenFieldSet { reg, mask }) => {
                self.regs[*reg as usize] & mask != 0
            }
            Some(IrAutoIncrement::Never) | None => false,
        }
    }

    fn wide_index(&self, pointer: u8) -> Option<usize> {
        self.spec
            .wide_registers
            .iter()
            .position(|w| w.pointer == pointer)
    }
}

impl I2cDevice for IrI2cComponent {
    fn address(&self) -> u8 {
        self.addr
    }

    fn start(&mut self) {
        self.writes_since_start = 0;
        self.read_phase = 0;
    }

    fn write(&mut self, data: u8) {
        let pointered = self
            .spec
            .pointer
            .as_ref()
            .map(|p| p.first_write_after_start_sets_pointer)
            .unwrap_or(false);
        if pointered && self.writes_since_start == 0 {
            let mask = self.spec.pointer.as_ref().unwrap().pointer_mask;
            self.pointer = data & mask;
        } else if self.wide_index(self.pointer).is_some() {
            // Absorb: the current pointer selects a wide (multi-byte) register.
            // Wide registers have no writable byte representation; data writes
            // after the pointer-select are silently discarded, matching the
            // reference behavior for read-only wide-register devices (e.g. TMP102
            // config register: the host may send config bytes that the simulator
            // ignores, preserving the reset value intact).
        } else {
            let idx = self.pointer as usize % self.regs.len();
            self.regs[idx] = data;
            if self.auto_increment_enabled() {
                self.pointer = self.pointer.wrapping_add(1);
            }
        }
        self.writes_since_start = self.writes_since_start.saturating_add(1);
    }

    fn read(&mut self) -> u8 {
        if let Some(wi) = self.wide_index(self.pointer) {
            let value = self.wide[wi];
            let byte = if self.read_phase == 0 {
                (value >> 8) as u8
            } else {
                (value & 0xFF) as u8
            };
            self.read_phase ^= 1;
            if self.read_phase == 0 {
                let ptr = self.pointer;
                self.apply_updates_on_wide_read_complete(ptr);
            }
            return byte;
        }
        let idx = self.pointer as usize % self.regs.len();
        let v = self.regs[idx];
        if self.auto_increment_enabled() {
            self.pointer = self.pointer.wrapping_add(1);
        }
        v
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl IrI2cComponent {
    /// Read a named observable for a specific channel.
    ///
    /// Returns `None` when:
    /// - `name` does not match any declared observable,
    /// - `channel` is out of range for the observable's `channels` count,
    /// - the observable's map has `none_when_raw_zero` set and the raw value is 0.
    ///
    /// Otherwise returns the mapped engineering value (or the raw value as `f32`
    /// if no `map` is declared).
    pub fn observable(&self, name: &str, channel: u8) -> Option<f32> {
        let obs = self.spec.observables.iter().find(|o| o.name == name)?;
        if channel >= obs.channels {
            return None;
        }
        let base = obs.base as usize + obs.stride as usize * channel as usize;
        let IrObservableValue::U12Compose {
            lo_rel,
            hi_rel,
            hi_mask,
        } = obs.value;
        let lo = self.regs[base + lo_rel as usize];
        let hi = self.regs[base + hi_rel as usize] & hi_mask;
        let raw = ((hi as u16) << 8) | lo as u16;
        if let Some(map) = &obs.map {
            if map.none_when_raw_zero && raw == 0 {
                return None;
            }
            let eng = raw as f32 * map.linear.scale + map.linear.offset;
            let eng = if let Some((lo_clamp, hi_clamp)) = map.linear.clamp {
                eng.clamp(lo_clamp, hi_clamp)
            } else {
                eng
            };
            Some(eng)
        } else {
            Some(raw as f32)
        }
    }

    fn apply_updates_on_wide_read_complete(&mut self, pointer: u8) {
        // Collect first to satisfy the borrow checker; specs are small.
        let actions: Vec<IrUpdateAction> = self
            .spec
            .updates
            .iter()
            .filter(|u| {
                matches!(u.trigger, IrUpdateTrigger::WideReadComplete { pointer: p } if p == pointer)
            })
            .map(|u| u.action.clone())
            .collect();
        if actions.is_empty() {
            return;
        }
        let wi = self.wide_index(pointer).expect("validated");
        for a in actions {
            let IrUpdateAction::AddWrap { add, max, reset } = a;
            // Storage is the raw 16-bit register image; AddWrap is defined as
            // signed per the IR doc ("signed compare, as i16"), so the signed
            // view exists only here — registers with bit 15 set remain
            // well-defined (e.g. reset=0xE700 stays negative after wrapping add).
            let mut v = (self.wide[wi] as i16).wrapping_add(add);
            if v > max {
                v = reset;
            }
            self.wide[wi] = v as u16;
        }
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
      linear: { scale: 0.46291754, offset: -47.368423, clamp: [0.0, 180.0] }
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
        // raw 135 → angle = 135 * 0.46291754 + (-47.368423) ≈ 62.394 - 47.368 ≈ 15.026°
        ir_set_angle(&mut d, 0, 135);
        let angle = d
            .observable("servo_angle", 0)
            .expect("should be Some after write");
        assert!((angle - 15.0).abs() < 1.5, "expected ~15°, got {angle:.3}");

        // Other channels still None.
        assert_eq!(d.observable("servo_angle", 1), None);

        // Last channel (15) also independent.
        ir_set_angle(&mut d, 15, 135);
        let angle15 = d
            .observable("servo_angle", 15)
            .expect("ch15 should be Some");
        assert!(
            (angle15 - 15.0).abs() < 1.5,
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
