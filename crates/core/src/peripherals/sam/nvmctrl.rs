// SPDX-License-Identifier: MIT
//! Opt-in SAMD51 manual main-array storage slice, DS60001507F 25.6.6/25.8.
//! 512 KiB main array, 512-byte pages, 8 KiB erase blocks; only the final
//! 32 KiB storage window is admitted. No user rows, fuse/protection, automatic
//! modes, ECC, IRQ delivery, physical latency, or full-controller claim.
//! Stores fill a page buffer; the bus commits pending operations to its own
//! flash backing. Four instruction-boundary services model a bounded busy
//! interval, not silicon time. Construction is initialization, not reset proof.
use crate::{Peripheral, SimResult, SimulationError};
use std::any::Any;

pub const FLASH_SIZE: u64 = 0x80000;
pub const STORAGE_START: u64 = 0x78000;
const PAGE: usize = 512;
const BLOCK: usize = 8192;
const DONE: u16 = 1;
const ADDRE: u16 = 2;
const PROGE: u16 = 4;
const INT_MASK: u16 = 0x07ff;
/// Hardware CTRLA mask (`0xFFFC`) without WMODE. The model only implements
/// manual writes, but AUTOWS/SUSPEN/PRM/RWS/AHBNS/CACHEDIS are accepted and
/// ignored so a normal clock/wait-state setup does not hard-fail.
const CTRLA_WRITE_MASK: u16 = 0xffcc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Erase { address: usize },
    Program { address: usize, bytes: [u8; 16] },
    Clear,
}

/// Main-array buffer and deferred-commit capability for the manual storage
/// slice. The bus owns the backing bytes; the peripheral owns command state.
pub trait ManualFlashStorage {
    fn store(&mut self, address: u64, width: u8, value: u32) -> SimResult<()>;
    fn boundary(&mut self) -> Option<Operation>;
    fn finish(&mut self);
}
#[derive(Debug)]
pub struct Samd51Nvmctrl {
    control: u16,
    address: u32,
    flags: u16,
    enables: u16,
    page_address: Option<u64>,
    page: [u8; PAGE],
    holding_address: Option<u64>,
    holding: [u8; 8],
    pending: Option<Operation>,
    boundaries: u8,
}
impl Default for Samd51Nvmctrl {
    fn default() -> Self {
        Self::new()
    }
}
impl Samd51Nvmctrl {
    pub fn new() -> Self {
        Self {
            control: 4,
            address: 0,
            flags: 0,
            enables: 0,
            page_address: None,
            page: [0xff; PAGE],
            holding_address: None,
            holding: [0xff; 8],
            pending: None,
            boundaries: 0,
        }
    }
    fn refuse<T>() -> SimResult<T> {
        Err(SimulationError::NotImplemented(
            "samd51 manual-storage slice: unsupported access/mode/range".into(),
        ))
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    fn clear_buffer(&mut self) {
        self.page.fill(0xff);
        self.page_address = None;
        self.holding.fill(0xff);
        self.holding_address = None;
    }
    /// A flash-space AHB store, not a programming commit. Only aligned words
    /// are admitted (smaller accesses would need PAC fault qualification).
    pub fn store(&mut self, address: u64, width: u8, value: u32) -> SimResult<()> {
        if width != 4
            || address & 3 != 0
            || !(STORAGE_START..FLASH_SIZE).contains(&address)
            || address + 4 > FLASH_SIZE
            || self.control & 0x30 != 0
        {
            return Self::refuse();
        }
        // AHB stalls during programming require a separate CPU/bus stall model;
        // never silently commit or drop an unsupported concurrent store.
        if self.pending() {
            return Self::refuse();
        }
        let page = address & !511;
        if self.page_address.is_some_and(|old| old != page) {
            return Self::refuse();
        }
        self.page_address = Some(page);
        let holding = address & !7;
        if self.holding_address != Some(holding) {
            self.holding.fill(0xff);
            self.holding_address = Some(holding);
        }
        let offset = (address & 7) as usize;
        self.holding[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        let offset = (holding & 511) as usize;
        self.page[offset..offset + 8].copy_from_slice(&self.holding);
        self.address = address as u32;
        Ok(())
    }
    fn command(&mut self, value: u16) -> SimResult<()> {
        if value >> 8 != 0xa5 || value & 0x80 != 0 || self.pending() {
            self.flags |= PROGE;
            return Ok(());
        }
        let operation = match value & 0x7f {
            0x15 => Operation::Clear,
            1 | 4 => {
                let address = u64::from(self.address);
                if !(STORAGE_START..FLASH_SIZE).contains(&address) {
                    return Self::refuse();
                }
                if self.flags & ADDRE != 0 {
                    self.flags |= PROGE;
                    return Ok(());
                }
                if value & 0x7f == 1 {
                    Operation::Erase {
                        address: (address as usize) & !(BLOCK - 1),
                    }
                } else {
                    if self.page_address.is_some_and(|page| page != address & !511) {
                        return Self::refuse();
                    }
                    let start = (address as usize) & 0x1f0;
                    let mut bytes = [0xff; 16];
                    bytes.copy_from_slice(&self.page[start..start + 16]);
                    Operation::Program {
                        address: (address as usize) & !15,
                        bytes,
                    }
                }
            }
            _ => return Self::refuse(),
        };
        self.pending = Some(operation);
        self.boundaries = 4;
        Ok(())
    }
    /// Peek until due; the bus must commit backing bytes before calling finish.
    pub fn boundary(&mut self) -> Option<Operation> {
        self.pending.as_ref()?;
        self.boundaries = self.boundaries.saturating_sub(1);
        (self.boundaries == 0).then(|| self.pending.as_ref().unwrap().clone())
    }
    pub fn finish(&mut self) {
        if matches!(
            self.pending,
            Some(Operation::Clear | Operation::Program { .. })
        ) {
            self.clear_buffer();
        }
        self.pending = None;
        self.flags |= DONE;
    }
    fn byte(&self, offset: u64) -> SimResult<u8> {
        let (start, bytes): (u64, [u8; 4]) = match offset {
            0..=1 => (0, (self.control as u32).to_le_bytes()),
            4..=5 => (4, [0; 4]),
            8..=11 => (8, 0x00060400u32.to_le_bytes()), // PSZ=512, NVMP=1024
            0x0c..=0x0d => (0x0c, (self.enables as u32).to_le_bytes()),
            0x0e..=0x0f => (0x0e, (self.enables as u32).to_le_bytes()),
            0x10..=0x11 => (0x10, (self.flags as u32).to_le_bytes()),
            0x12..=0x13 => (
                0x12,
                (u32::from(!self.pending()) | (u32::from(self.page_address.is_some()) << 2))
                    .to_le_bytes(),
            ),
            0x14..=0x17 => (0x14, self.address.to_le_bytes()),
            _ => return Self::refuse(),
        };
        Ok(bytes[(offset - start) as usize])
    }
}
impl ManualFlashStorage for Samd51Nvmctrl {
    fn store(&mut self, address: u64, width: u8, value: u32) -> SimResult<()> {
        Samd51Nvmctrl::store(self, address, width, value)
    }
    fn boundary(&mut self) -> Option<Operation> {
        Samd51Nvmctrl::boundary(self)
    }
    fn finish(&mut self) {
        Samd51Nvmctrl::finish(self)
    }
}

impl Peripheral for Samd51Nvmctrl {
    fn manual_flash_storage(&self) -> Option<&dyn ManualFlashStorage> {
        Some(self)
    }
    fn manual_flash_storage_mut(&mut self) -> Option<&mut dyn ManualFlashStorage> {
        Some(self)
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        self.byte(offset)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Self::refuse()
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        match offset {
            0 => {
                if value & !CTRLA_WRITE_MASK != 0 {
                    return Self::refuse();
                }
                self.control = value;
                Ok(())
            }
            4 => self.command(value),
            0x0c => {
                self.enables &= !value;
                Ok(())
            }
            0x0e => {
                self.enables |= value & INT_MASK;
                Ok(())
            }
            0x10 => {
                if value & !7 != 0 {
                    return Self::refuse();
                }
                self.flags &= !value;
                Ok(())
            }
            _ => Self::refuse(),
        }
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset != 0x14 {
            return Self::refuse();
        }
        if self.pending() || self.page_address.is_some() {
            self.flags |= ADDRE;
        } else {
            self.address = value;
        }
        Ok(())
    }
    fn has_pending_op(&self) -> bool {
        self.pending()
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.byte(offset).ok()
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
    fn finish(nvm: &mut Samd51Nvmctrl) -> Operation {
        for _ in 0..3 {
            assert_eq!(nvm.boundary(), None);
        }
        let op = nvm.boundary().unwrap();
        nvm.finish();
        op
    }
    #[test]
    fn geometry_busy_admission_and_w1c_are_distinct() {
        let mut nvm = Samd51Nvmctrl::new();
        assert_eq!(nvm.read_u32(8).unwrap(), 0x60400);
        assert_eq!(nvm.read_u16(0).unwrap(), 4);
        assert_eq!(nvm.read_u16(0x12).unwrap(), 1);
        nvm.write_u32(0x14, STORAGE_START as u32).unwrap();
        nvm.write_u16(4, 0xa501).unwrap();
        assert_eq!(nvm.read_u16(0x12).unwrap() & 1, 0);
        nvm.write_u16(4, 0xa515).unwrap(); // busy command ignored; erase remains
        nvm.write_u32(0x14, 0x7a000).unwrap(); // locked address retained
        assert_eq!(nvm.read_u32(0x14).unwrap(), STORAGE_START as u32);
        assert_eq!(
            finish(&mut nvm),
            Operation::Erase {
                address: STORAGE_START as usize
            }
        );
        assert_eq!(nvm.read_u16(0x10).unwrap(), DONE | PROGE | ADDRE);
        nvm.write_u16(0x10, 7).unwrap();
        assert_eq!(nvm.read_u16(0x10).unwrap(), 0);
        nvm.write_u16(4, 0x1515).unwrap();
        assert!(!nvm.pending());
        assert_eq!(nvm.read_u16(0x10).unwrap(), PROGE);
    }
    #[test]
    fn manual_buffer_quadword_and_holding_transitions() {
        let mut nvm = Samd51Nvmctrl::new();
        for i in 0..4 {
            nvm.store(STORAGE_START + i * 4, 4, 0x11223300 + i as u32)
                .unwrap();
        }
        assert_eq!(nvm.read_u16(0x12).unwrap(), 5);
        nvm.write_u16(4, 0xa504).unwrap();
        let bytes = [0x11223300u32, 0x11223301, 0x11223302, 0x11223303]
            .map(u32::to_le_bytes)
            .concat();
        assert_eq!(
            finish(&mut nvm),
            Operation::Program {
                address: STORAGE_START as usize,
                bytes: bytes.try_into().unwrap()
            }
        );
        assert_eq!(nvm.read_u16(0x12).unwrap(), 1);
        nvm.store(STORAGE_START, 4, 0).unwrap();
        nvm.store(STORAGE_START + 8, 4, 1).unwrap();
        nvm.store(STORAGE_START + 4, 4, 2).unwrap(); // crossing back resets opposite word
        nvm.write_u16(4, 0xa504).unwrap();
        let Operation::Program { bytes, .. } = finish(&mut nvm) else {
            panic!()
        };
        assert_eq!(&bytes[..4], &[0xff; 4]);
        assert_eq!(&bytes[4..8], &2u32.to_le_bytes());
    }
    #[test]
    fn unsupported_width_mode_range_and_concurrent_store_refuse() {
        let mut nvm = Samd51Nvmctrl::new();
        for (address, width) in [
            (STORAGE_START, 1),
            (STORAGE_START, 2),
            (STORAGE_START + 1, 4),
            (0, 4),
            (FLASH_SIZE, 4),
        ] {
            assert!(nvm.store(address, width, 0).is_err());
            assert!(!nvm.pending());
        }
        assert!(nvm.write_u16(0, 0x14).is_err());
        assert!(nvm.write_u32(0, 4).is_err());
        assert!(nvm.write(4, 0x15).is_err());
        assert!(nvm.write_u16(4, 0xa501).is_err()); // outside supported storage window
        nvm.write_u16(4, 0xa515).unwrap();
        assert!(nvm.store(STORAGE_START, 4, 0).is_err());
        assert_eq!(finish(&mut nvm), Operation::Clear);
        assert!(!nvm.pending());
    }
    #[test]
    fn ctrla_wait_states_and_interrupt_enables_are_admitted() {
        let mut nvm = Samd51Nvmctrl::new();
        nvm.write_u16(0, 0x0f04).unwrap(); // AUTOWS + RWS=0xF
        assert_eq!(nvm.read_u16(0).unwrap(), 0x0f04);
        nvm.write_u16(0x0e, 0x0007).unwrap();
        assert_eq!(nvm.read_u16(0x0e).unwrap(), 7);
        nvm.write_u16(0x0c, 0x0002).unwrap();
        assert_eq!(nvm.read_u16(0x0e).unwrap(), 5);
        assert!(nvm.write_u16(0, 0x0010).is_err()); // WMODE automatic is outside the slice
    }
}
