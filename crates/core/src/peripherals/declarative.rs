// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use labwired_config::PeripheralDescriptor;
use std::any::Any;

use std::cell::RefCell;

#[derive(Debug)]
struct InflightEvent {
    #[allow(dead_code)]
    id: String,
    delay_remaining: u64,
    action: labwired_config::TimingAction,
    interrupt: Option<String>,
    periodic_interval: Option<u64>,
}

/// A generic peripheral implementation that uses a `PeripheralDescriptor` to define its
/// register layout and access permissions.
///
/// This allows for rapid modeling of memory-mapped peripherals without writing custom Rust code.
#[derive(Debug)]
pub struct GenericPeripheral {
    descriptor: PeripheralDescriptor,
    data: RefCell<Vec<u8>>,
    inflight_events: RefCell<Vec<InflightEvent>>,
}

impl GenericPeripheral {
    /// Creates a new `GenericPeripheral` from a descriptor.
    ///
    /// The backing memory is automatically sized to accommodate the highest register offset,
    /// and all registers are initialized to their specified `reset_value`.
    pub fn new(descriptor: PeripheralDescriptor) -> Self {
        let mut max_addr = 0;
        for reg in &descriptor.registers {
            let end_addr = reg.address_offset + (reg.size as u64 / 8);
            if end_addr > max_addr {
                max_addr = end_addr;
            }
        }

        let mut data = vec![0; max_addr as usize];

        // Initialize with reset values
        for reg in &descriptor.registers {
            let val = reg.reset_value;
            let offset = reg.address_offset as usize;
            match reg.size {
                8 => data[offset] = val as u8,
                16 => {
                    data[offset] = (val & 0xFF) as u8;
                    data[offset + 1] = ((val >> 8) & 0xFF) as u8;
                }
                32 => {
                    data[offset] = (val & 0xFF) as u8;
                    data[offset + 1] = ((val >> 8) & 0xFF) as u8;
                    data[offset + 2] = ((val >> 16) & 0xFF) as u8;
                    data[offset + 3] = ((val >> 24) & 0xFF) as u8;
                }
                _ => {}
            }
        }

        let p = Self {
            descriptor,
            data: RefCell::new(data),
            inflight_events: RefCell::new(Vec::new()),
        };

        // Initialize periodic events
        if let Some(timing) = &p.descriptor.timing {
            for hook in timing {
                if let labwired_config::TimingTrigger::Periodic { period_cycles } = &hook.trigger {
                    p.inflight_events.borrow_mut().push(InflightEvent {
                        id: hook.id.clone(),
                        delay_remaining: *period_cycles,
                        action: hook.action.clone(),
                        interrupt: hook.interrupt.clone(),
                        periodic_interval: Some(*period_cycles),
                    });
                }
            }
        }
        p
    }

    pub fn get_descriptor(&self) -> &labwired_config::PeripheralDescriptor {
        &self.descriptor
    }

    fn check_triggers(&self, register_id: &str, is_write: bool, value: Option<u32>) {
        if let Some(timing) = &self.descriptor.timing {
            for hook in timing {
                let triggered = match &hook.trigger {
                    labwired_config::TimingTrigger::Read { register } => {
                        !is_write && register == register_id
                    }
                    labwired_config::TimingTrigger::Write {
                        register,
                        value: trigger_value,
                        mask,
                    } => {
                        if !is_write || register != register_id {
                            false
                        } else if let Some(tv) = trigger_value {
                            let actual_val = value.unwrap_or(0);
                            if let Some(m) = mask {
                                (actual_val & m) == (*tv & m)
                            } else {
                                actual_val == *tv
                            }
                        } else {
                            true // Any write triggers it
                        }
                    }
                    labwired_config::TimingTrigger::Periodic { .. } => false,
                };

                if triggered {
                    self.inflight_events.borrow_mut().push(InflightEvent {
                        id: hook.id.clone(),
                        delay_remaining: hook.delay_cycles,
                        action: hook.action.clone(),
                        interrupt: hook.interrupt.clone(),
                        periodic_interval: None,
                    });
                }
            }
        }
    }

    fn apply_action(&self, action: &labwired_config::TimingAction) {
        let mut data = self.data.borrow_mut();
        match action {
            labwired_config::TimingAction::SetBits {
                register: reg_id,
                bits,
            } => {
                if let Some(reg) = self.descriptor.registers.iter().find(|r| &r.id == reg_id) {
                    let offset = reg.address_offset as usize;
                    // Apply bits to all bytes of the register based on its size
                    for i in 0..(reg.size / 8) {
                        let shift = i * 8;
                        let byte_bits = ((bits >> shift) & 0xFF) as u8;
                        data[offset + i as usize] |= byte_bits;
                    }
                }
            }
            labwired_config::TimingAction::ClearBits {
                register: reg_id,
                bits,
            } => {
                if let Some(reg) = self.descriptor.registers.iter().find(|r| &r.id == reg_id) {
                    let offset = reg.address_offset as usize;
                    for i in 0..(reg.size / 8) {
                        let shift = i * 8;
                        let byte_bits = ((bits >> shift) & 0xFF) as u8;
                        data[offset + i as usize] &= !byte_bits;
                    }
                }
            }
            labwired_config::TimingAction::WriteValue {
                register: reg_id,
                value,
            } => {
                if let Some(reg) = self.descriptor.registers.iter().find(|r| &r.id == reg_id) {
                    let offset = reg.address_offset as usize;
                    for i in 0..(reg.size / 8) {
                        let shift = i * 8;
                        let byte_val = ((value >> shift) & 0xFF) as u8;
                        data[offset + i as usize] = byte_val;
                    }
                }
            }
        }
    }
}

impl Peripheral for GenericPeripheral {
    fn read(&self, offset: u64) -> SimResult<u8> {
        // Find register containing this offset
        for reg in &self.descriptor.registers {
            let reg_start = reg.address_offset;
            let reg_end = reg_start + (reg.size as u64 / 8);
            if offset >= reg_start && offset < reg_end {
                if reg.access == labwired_config::Access::WriteOnly {
                    return Ok(0);
                }

                let mut data = self.data.borrow_mut();
                let val = data[offset as usize];

                // Side Effects: ReadAction
                if let Some(side_effects) = &reg.side_effects {
                    if let Some(labwired_config::ReadAction::Clear) = side_effects.read_action {
                        data[offset as usize] = 0;
                    }
                }

                self.check_triggers(&reg.id, false, None);

                return Ok(val);
            }
        }
        Ok(0)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // Find register containing this offset
        for reg in &self.descriptor.registers {
            let reg_start = reg.address_offset;
            let reg_end = reg_start + (reg.size as u64 / 8);
            if offset >= reg_start && offset < reg_end {
                if reg.access == labwired_config::Access::ReadOnly {
                    return Ok(());
                }

                let mut data = self.data.borrow_mut();

                // Side Effects: WriteAction
                if let Some(side_effects) = &reg.side_effects {
                    match side_effects.write_action {
                        Some(labwired_config::WriteAction::WriteOneToClear) => {
                            data[offset as usize] &= !value;
                        }
                        Some(labwired_config::WriteAction::WriteZeroToClear) => {
                            data[offset as usize] &= value;
                        }
                        _ => {
                            data[offset as usize] = value;
                        }
                    }
                } else {
                    data[offset as usize] = value;
                }

                // For triggers, we need the full register value being written (ideally).
                // But GenericPeripheral writes byte-by-byte.
                // This is a limitation: multi-byte write triggers might be tricky.
                // However, most SVD tools/emulators assume 32-bit writes for control registers.
                // Let's at least trigger on the byte write.
                // Calculate the shift for this byte within the register
                let byte_offset = (offset - reg_start) * 8;
                let shifted_val = (value as u32) << byte_offset;
                self.check_triggers(&reg.id, true, Some(shifted_val));

                return Ok(());
            }
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        for reg in &self.descriptor.registers {
            let reg_start = reg.address_offset;
            let reg_end = reg_start + (reg.size as u64 / 8);
            if offset >= reg_start && offset < reg_end {
                if reg.access == labwired_config::Access::WriteOnly {
                    return Some(0);
                }
                return self.data.borrow().get(offset as usize).copied();
            }
        }
        Some(0)
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let mut result = PeripheralTickResult::default();
        let mut events = self.inflight_events.borrow_mut();

        let mut i = 0;
        let mut re_adds = Vec::new();
        while i < events.len() {
            if events[i].delay_remaining > 0 {
                events[i].delay_remaining -= 1;
                i += 1;
            } else {
                let event = events.remove(i);
                self.apply_action(&event.action);
                if let Some(ref int_name) = event.interrupt {
                    if let Some(ints) = &self.descriptor.interrupts {
                        if let Some(&val) = ints.get(int_name) {
                            result.explicit_irqs.get_or_insert_with(Vec::new).push(val);
                        }
                    }
                }

                // If periodic, re-add after the loop to prevent same-tick processing
                if let Some(interval) = event.periodic_interval {
                    re_adds.push(InflightEvent {
                        id: event.id,
                        delay_remaining: interval,
                        action: event.action,
                        interrupt: event.interrupt,
                        periodic_interval: Some(interval),
                    });
                }

                // Do not increment i, as we removed an element
            }
        }
        events.extend(re_adds);

        result
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": self.descriptor.peripheral,
            "data": *self.data.borrow()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::{Access, PeripheralDescriptor, RegisterDescriptor};

    fn mock_descriptor() -> PeripheralDescriptor {
        PeripheralDescriptor {
            peripheral: "Mock".to_string(),
            version: "1.0".to_string(),
            registers: vec![
                RegisterDescriptor {
                    id: "REG1".to_string(),
                    address_offset: 0x00,
                    size: 32,
                    access: Access::ReadWrite,
                    reset_value: 0x12345678,
                    fields: vec![],
                    side_effects: None,
                },
                RegisterDescriptor {
                    id: "RO_REG".to_string(),
                    address_offset: 0x04,
                    size: 8,
                    access: Access::ReadOnly,
                    reset_value: 0xAA,
                    fields: vec![],
                    side_effects: None,
                },
                RegisterDescriptor {
                    id: "WO_REG".to_string(),
                    address_offset: 0x05,
                    size: 8,
                    access: Access::WriteOnly,
                    reset_value: 0x00,
                    fields: vec![],
                    side_effects: None,
                },
                RegisterDescriptor {
                    id: "REG16".to_string(),
                    address_offset: 0x06,
                    size: 16,
                    access: Access::ReadWrite,
                    reset_value: 0xABCD,
                    fields: vec![],
                    side_effects: None,
                },
            ],
            interrupts: None,
            timing: None,
        }
    }

    #[test]
    fn test_initialization() {
        let p = GenericPeripheral::new(mock_descriptor());
        assert_eq!(p.read(0x00).unwrap(), 0x78);
        assert_eq!(p.read(0x01).unwrap(), 0x56);
        assert_eq!(p.read(0x02).unwrap(), 0x34);
        assert_eq!(p.read(0x03).unwrap(), 0x12);
        assert_eq!(p.read(0x04).unwrap(), 0xAA);
        assert_eq!(p.read(0x06).unwrap(), 0xCD);
        assert_eq!(p.read(0x07).unwrap(), 0xAB);
    }

    #[test]
    fn test_write_read() {
        let mut p = GenericPeripheral::new(mock_descriptor());
        p.write(0x00, 0xFF).unwrap();
        assert_eq!(p.read(0x00).unwrap(), 0xFF);
    }

    #[test]
    fn test_read_only() {
        let mut p = GenericPeripheral::new(mock_descriptor());
        p.write(0x04, 0xBB).unwrap();
        assert_eq!(p.read(0x04).unwrap(), 0xAA); // Should not change
    }

    #[test]
    fn test_write_only() {
        let mut p = GenericPeripheral::new(mock_descriptor());
        p.write(0x05, 0xCC).unwrap();
        assert_eq!(p.read(0x05).unwrap(), 0x00); // Reads should return 0
    }

    #[test]
    fn test_16bit_access() {
        let mut p = GenericPeripheral::new(mock_descriptor());
        p.write(0x06, 0x11).unwrap();
        p.write(0x07, 0x22).unwrap();
        assert_eq!(p.read(0x06).unwrap(), 0x11);
        assert_eq!(p.read(0x07).unwrap(), 0x22);
    }
    #[test]
    fn test_side_effects_rtc() {
        let mut desc = mock_descriptor();
        desc.registers[0].side_effects = Some(labwired_config::SideEffectsDescriptor {
            read_action: Some(labwired_config::ReadAction::Clear),
            write_action: None,
            on_read: None,
            on_write: None,
        });

        let p = GenericPeripheral::new(desc);
        // Initial reset value 0x12345678. Byte 0 is 0x78.
        assert_eq!(p.read(0x00).unwrap(), 0x78);
        assert_eq!(p.read(0x00).unwrap(), 0x00); // Cleared on read
    }

    #[test]
    fn test_side_effects_w1c() {
        let mut desc = mock_descriptor();
        desc.registers[0].side_effects = Some(labwired_config::SideEffectsDescriptor {
            read_action: None,
            write_action: Some(labwired_config::WriteAction::WriteOneToClear),
            on_read: None,
            on_write: None,
        });

        let mut p = GenericPeripheral::new(desc);
        // Byte 0 is 0x78 (binary: 0111 1000)
        // Write 0x08 (binary: 0000 1000) to clear bit 3.
        p.write(0x00, 0x08).unwrap();
        assert_eq!(p.read(0x00).unwrap(), 0x70); // 0x78 & !0x08 = 0x70
    }

    #[test]
    fn test_timing_hook() {
        let mut desc = mock_descriptor();
        desc.registers.push(RegisterDescriptor {
            id: "STATUS".to_string(),
            address_offset: 0x10,
            size: 8,
            access: Access::ReadOnly,
            reset_value: 0x00,
            fields: vec![],
            side_effects: None,
        });
        desc.interrupts = Some({
            let mut h = std::collections::HashMap::new();
            h.insert("INT1".to_string(), 42);
            h
        });
        desc.timing = Some(vec![labwired_config::TimingDescriptor {
            id: "test_evt".to_string(),
            trigger: labwired_config::TimingTrigger::Write {
                register: "REG1".to_string(),
                value: Some(0xAA),
                mask: None,
            },
            delay_cycles: 1,
            action: labwired_config::TimingAction::SetBits {
                register: "STATUS".to_string(),
                bits: 0x01,
            },
            interrupt: Some("INT1".to_string()),
        }]);

        let mut p = GenericPeripheral::new(desc);

        // Byte 0 of REG1 is 0x78 initially.
        // Write 0xAA to trigger.
        p.write(0x00, 0xAA).unwrap();

        // Tick 1: Still 1 cycle left (delay 1 -> 0)
        let res = p.tick();
        assert!(res.explicit_irqs.is_none());
        assert_eq!(p.read(0x10).unwrap(), 0x00);

        // Tick 2: Triggered! (delay 0 -> fired)
        let res = p.tick();
        assert!(res.explicit_irqs.as_ref().is_some_and(|v| v.contains(&42)));
        assert_eq!(p.read(0x10).unwrap(), 0x01);
    }

    #[test]
    fn test_immediate_timing() {
        let mut desc = mock_descriptor();
        desc.registers.push(RegisterDescriptor {
            id: "STATUS".to_string(),
            address_offset: 0x10,
            size: 8,
            access: Access::ReadOnly,
            reset_value: 0x00,
            fields: vec![],
            side_effects: None,
        });
        desc.timing = Some(vec![labwired_config::TimingDescriptor {
            id: "immediate".to_string(),
            trigger: labwired_config::TimingTrigger::Read {
                register: "REG1".to_string(),
            },
            delay_cycles: 0,
            action: labwired_config::TimingAction::WriteValue {
                register: "STATUS".to_string(),
                value: 0x55,
            },
            interrupt: None,
        }]);

        let mut p = GenericPeripheral::new(desc);

        // Read to trigger
        p.read(0x00).unwrap();

        // Tick 1: Triggered immediately (delay 0 -> fired)
        let res = p.tick();
        assert!(res.explicit_irqs.is_none());
        assert_eq!(p.read(0x10).unwrap(), 0x55);
    }

    #[test]
    fn test_periodic_timing() {
        let mut desc = mock_descriptor();
        desc.registers.push(RegisterDescriptor {
            id: "STATUS".to_string(),
            address_offset: 0x10,
            size: 8,
            access: Access::ReadWrite,
            reset_value: 0x00,
            fields: vec![],
            side_effects: None,
        });
        desc.timing = Some(vec![labwired_config::TimingDescriptor {
            id: "heartbeat".to_string(),
            trigger: labwired_config::TimingTrigger::Periodic { period_cycles: 1 },
            delay_cycles: 0,
            action: labwired_config::TimingAction::SetBits {
                register: "STATUS".to_string(),
                bits: 0x01,
            },
            interrupt: None,
        }]);

        let mut p = GenericPeripheral::new(desc);

        // Tick 1: 1 -> 0
        p.tick();
        assert_eq!(p.read(0x10).unwrap(), 0x00);

        // Tick 2: 0 -> fired, re-added with 1
        p.tick();
        assert_eq!(p.read(0x10).unwrap(), 0x01);

        // Clear it
        p.write(0x10, 0x00).unwrap();
        assert_eq!(p.read(0x10).unwrap(), 0x00);

        // Tick 3: 1 -> 0
        p.tick();
        assert_eq!(p.read(0x10).unwrap(), 0x00);

        // Tick 4: 0 -> fired
        p.tick();
        assert_eq!(p.read(0x10).unwrap(), 0x01);
    }
}
