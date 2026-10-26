// SPDX-License-Identifier: MIT
//! Opt-in SAMD51 main-array buffering and boundary commit. Other buses keep
//! their existing memory routing. No generic flash controller is substituted.
use super::*;
use crate::peripherals::sam::nvmctrl::{Operation, FLASH_SIZE, STORAGE_START};
use crate::SimulationError;

impl SystemBus {
    #[inline]
    pub(super) fn try_samd51_storage_store(
        &mut self,
        address: u64,
        width: u8,
        value: u32,
    ) -> Option<SimResult<()>> {
        let index = self.samd51_nvmctrl_idx?;
        // Device-originated writes program the array through their own path;
        // they must not populate the CPU-facing manual page buffer.
        if self.device_write_bypass() {
            return None;
        }
        // Only the qualified storage window is the manual controller's. Stores
        // elsewhere keep the bus's existing flash behaviour instead of turning
        // into a hard error on an opt-in board.
        if !(STORAGE_START..FLASH_SIZE).contains(&address) || self.flash.read_u8(address).is_none()
        {
            return None;
        }
        if self.flash.base_addr != 0 || self.flash.data.len() != FLASH_SIZE as usize {
            return Some(Err(SimulationError::NotImplemented(
                "SAMD51 storage requires exact 512KiB zero-based backing".into(),
            )));
        }
        Some(
            self.peripherals[index]
                .dev
                .manual_flash_storage_mut()
                .ok_or_else(|| {
                    SimulationError::NotImplemented("SAMD51 manual storage disappeared".into())
                })
                .and_then(|storage| storage.store(address, width, value)),
        )
    }

    /// One service at a retired CPU-window boundary. While pending, the bus
    /// pending-op predicate makes each such window end after one instruction.
    #[inline(always)]
    pub(crate) fn service_samd51_storage_boundary(&mut self) -> SimResult<()> {
        let Some(index) = self.samd51_nvmctrl_idx else {
            return Ok(());
        };
        self.service_samd51_storage_controller(index)
    }

    // Keep the opt-in controller's commit machinery out of the generic CPU
    // boundary. Buses without this capability need only the inline presence
    // check; enabled controllers retain the same service and error ordering.
    #[inline(never)]
    fn service_samd51_storage_controller(&mut self, index: usize) -> SimResult<()> {
        let operation = self
            .peripherals
            .get_mut(index)
            .and_then(|p| p.dev.manual_flash_storage_mut())
            .ok_or_else(|| {
                SimulationError::NotImplemented("SAMD51 manual storage disappeared".into())
            })?
            .boundary();
        let Some(operation) = operation else {
            return Ok(());
        };
        if self.flash.base_addr != 0 || self.flash.data.len() != FLASH_SIZE as usize {
            return Err(SimulationError::NotImplemented(
                "SAMD51 storage backing geometry differs".into(),
            ));
        }
        match operation {
            Operation::Clear => {}
            Operation::Erase { address } => {
                let end = address
                    .checked_add(8192)
                    .filter(|end| *end <= self.flash.data.len())
                    .ok_or_else(|| {
                        SimulationError::NotImplemented("SAMD51 erase outside flash backing".into())
                    })?;
                self.flash.data[address..end].fill(0xff);
                self.note_memory_write();
            }
            Operation::Program { address, bytes } => {
                let end = address
                    .checked_add(bytes.len())
                    .filter(|end| *end <= self.flash.data.len())
                    .ok_or_else(|| {
                        SimulationError::NotImplemented(
                            "SAMD51 program outside flash backing".into(),
                        )
                    })?;
                for (old, new) in self.flash.data[address..end].iter_mut().zip(bytes) {
                    *old &= new;
                }
                self.note_memory_write();
            }
        }
        self.peripherals
            .get_mut(index)
            .and_then(|p| p.dev.manual_flash_storage_mut())
            .ok_or_else(|| {
                SimulationError::NotImplemented("SAMD51 manual storage disappeared".into())
            })?
            .finish();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::LinearMemory;
    use crate::peripherals::sam::nvmctrl::Samd51Nvmctrl;
    use crate::Bus;
    const NVM: u64 = 0x41004000;
    fn bus(optimized: bool) -> SystemBus {
        let mut bus = SystemBus::new();
        bus.flash = LinearMemory::new_erased(FLASH_SIZE as usize, 0);
        bus.config.optimized_bus_access = optimized;
        bus.add_peripheral(
            "manual-nvm",
            NVM,
            0x100,
            None,
            Box::new(Samd51Nvmctrl::new()),
        );
        bus
    }
    fn complete(bus: &mut SystemBus) {
        assert!(bus.models_flash_ops());
        assert!(bus.has_pending_flash_op());
        for _ in 0..3 {
            bus.service_samd51_storage_boundary().unwrap();
            assert!(bus.has_pending_flash_op());
        }
        bus.service_samd51_storage_boundary().unwrap();
        assert!(!bus.has_pending_flash_op());
    }
    #[test]
    fn actual_backing_changes_only_at_final_boundary_on_both_bus_paths() {
        for optimized in [false, true] {
            let mut b = bus(optimized);
            b.write_u32(0x78000, 0x12345678).unwrap();
            assert_eq!(b.read_u32(0x78000).unwrap(), 0xffffffff);
            b.write_u16(NVM + 4, 0xa504).unwrap();
            for _ in 0..3 {
                b.service_samd51_storage_boundary().unwrap();
                assert_eq!(b.read_u32(0x78000).unwrap(), 0xffffffff);
            }
            b.service_samd51_storage_boundary().unwrap();
            assert_eq!(b.read_u32(0x78000).unwrap(), 0x12345678);
            assert_eq!(b.read_u32(0x78004).unwrap(), 0xffffffff);
            b.flash.write_u8(0x77fff, 0x5a);
            b.flash.write_u8(0x7a000, 0xa5);
            b.write_u32(NVM + 0x14, 0x78123).unwrap();
            b.write_u16(NVM + 4, 0xa501).unwrap();
            complete(&mut b);
            assert!(b.flash.data[0x78000..0x7a000].iter().all(|v| *v == 0xff));
            assert_eq!(b.read_u8(0x77fff).unwrap(), 0x5a);
            assert_eq!(b.read_u8(0x7a000).unwrap(), 0xa5);
            assert!(b.write_u8(0x78000, 0).is_err());
            assert!(b.write_u16(0x78000, 0).is_err());
        }
    }
    #[test]
    fn stores_outside_the_storage_window_keep_default_flash_behavior() {
        for optimized in [false, true] {
            let mut b = bus(optimized);
            b.write_u32(0x1000, 0x1234_5678).unwrap();
            assert_eq!(b.read_u32(0x1000).unwrap(), 0x1234_5678);
            assert!(!b.has_pending_flash_op());
            assert_eq!(b.read_u16(NVM + 0x12).unwrap() & 4, 0);
        }
    }

    #[test]
    fn device_originated_stores_do_not_enter_the_manual_page_buffer() {
        let mut b = bus(false);
        crate::with_device_write_bypass(&mut b, |b| {
            b.write_u32(STORAGE_START, 0x1122_3344).unwrap();
        });
        assert!(!b.has_pending_flash_op());
        assert_eq!(b.read_u16(NVM + 0x12).unwrap() & 4, 0);
    }

    #[test]
    fn absent_controller_boundary_preserves_memory_on_both_bus_paths() {
        for optimized in [false, true] {
            let mut b = SystemBus::new();
            b.config.optimized_bus_access = optimized;
            b.write_u8(0x4000, 0x12).unwrap();
            b.write_u16(0x4002, 0x3456).unwrap();
            b.write_u32(0x4004, 0x789abcde).unwrap();
            for _ in 0..8 {
                b.service_samd51_storage_boundary().unwrap();
            }
            assert_eq!(b.read_u8(0x4000).unwrap(), 0x12);
            assert_eq!(b.read_u16(0x4002).unwrap(), 0x3456);
            assert_eq!(b.read_u32(0x4004).unwrap(), 0x789abcde);
            assert!(!b.models_flash_ops());
            assert!(!b.has_pending_flash_op());
        }
    }

    #[test]
    fn absent_model_preserves_normal_flash_stores_and_wrong_geometry_refuses() {
        let mut b = SystemBus::new();
        b.write_u32(0x4000, 0x1234).unwrap();
        assert_eq!(b.read_u32(0x4000).unwrap(), 0x1234);
        b.add_peripheral("nvm", NVM, 0x100, None, Box::new(Samd51Nvmctrl::new()));
        assert!(b.write_u32(0x78000, 0).is_err()); // default backing is1MiB
    }
}
