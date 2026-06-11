// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Deterministic interpreter executing an [`labwired_ir::component::IrComponent`]
//! as an [`I2cDevice`]. Pure state machine over the spec: no host time, no
//! randomness, no I/O — determinism is preserved by construction.

use crate::peripherals::i2c::I2cDevice;
use labwired_ir::component::{
    IrAutoIncrement, IrComponent, IrComponentInterface, IrUpdateAction, IrUpdateTrigger,
};

/// Interpreter state for one component instance.
pub struct IrI2cComponent {
    spec: IrComponent,
    addr: u8,
    regs: Vec<u8>,
    wide: Vec<i16>, // parallel to spec.wide_registers
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
        let wide = spec.wide_registers.iter().map(|w| w.reset as i16).collect();
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
            let value = self.wide[wi] as u16;
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
            let mut v = self.wide[wi].wrapping_add(add);
            if v > max {
                v = reset;
            }
            self.wide[wi] = v;
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
}
