// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use labwired_config::PeripheralDescriptor;
use std::any::Any;

use std::cell::RefCell;

/// A generic peripheral implementation that uses a `PeripheralDescriptor` to define its
/// register layout and access permissions.
///
/// This allows for rapid modeling of memory-mapped peripherals without writing custom Rust code.
#[derive(Debug)]
pub struct GenericPeripheral {
    descriptor: PeripheralDescriptor,
    data: RefCell<Vec<u8>>,
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

        Self {
            descriptor,
            data: RefCell::new(data),
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
                return Ok(());
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
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
}
