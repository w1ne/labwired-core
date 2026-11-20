// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RP2350 XIP support for the existing cacheless, identity-mapped flash backing.
//!
//! Source: RP2350 datasheet §4.4.1.1 / §4.4.8 and pico-sdk ee68c78d
//! `hardware/regs/qmi.h`. Clean/invalidate operations complete immediately:
//! this model has no dirty cache lines. Cache pinning, direct serial transfers,
//! non-identity address translation, cache timing and TrustZone are not modeled.
//! Unsupported operations return an error rather than claiming completion.

use crate::{Peripheral, SimResult, SimulationError};

/// Cache-maintenance mirror at 0x18000000, covering the 64 MiB XIP address space.
#[derive(Debug, Default)]
pub struct Rp2350XipMaintenance;

impl Peripheral for Rp2350XipMaintenance {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, _offset: u64) -> SimResult<u8> {
        Err(SimulationError::NotImplemented(
            "RP2350 XIP maintenance-window reads are not modeled".into(),
        ))
    }

    fn write(&mut self, offset: u64, _value: u8) -> SimResult<()> {
        if offset >= 0x0400_0000 {
            return Err(SimulationError::MemoryViolation(0x1800_0000 + offset));
        }
        match offset & 7 {
            // Datasheet §4.4.1.1: invalidate/clean by set-way/address. Write
            // data is ignored. Coherent backing means there is no stale or
            // dirty cache data to modify, including the E11 workaround range.
            0..=3 => Ok(()),
            7 => Err(SimulationError::NotImplemented(
                "RP2350 XIP cache pinning/cache-as-SRAM is not modeled".into(),
            )),
            _ => Err(SimulationError::MemoryViolation(0x1800_0000 + offset)),
        }
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // A bus write selects one maintenance operation, not four byte ops.
        self.write(offset, value as u8)
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write(offset, value as u8)
    }
}

const QMI_MEM_RESET: [u32; 5] = [0x4000_0004, 0x1000, 0xa003, 0x1000, 0xa002];
const QMI_MEM_MASK: [u32; 5] = [0xf3ff_f7ff, 0x1007_d3ff, 0xffff, 0x1007_d3ff, 0xffff];

/// QMI memory-window configuration/readback, with source-defined reset values.
///
/// This preserves the SDK's CS1 save/restore sequence around ROM flash calls.
/// It does not turn register writes into serial-flash wire transactions.
#[derive(Debug)]
pub struct Rp2350Qmi {
    memory: [[u32; 5]; 2],
    direct_csr: u32,
}

impl Default for Rp2350Qmi {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2350Qmi {
    pub fn new() -> Self {
        Self {
            memory: [QMI_MEM_RESET; 2],
            direct_csr: 0x0180_0000,
        }
    }

    fn memory_index(offset: u64) -> Option<(usize, usize)> {
        (0x0c..0x34).contains(&offset).then(|| {
            let word = ((offset - 0x0c) / 4) as usize;
            (word / 5, word % 5)
        })
    }

    fn identity_translation(offset: u64) -> Option<u32> {
        (0x34..0x54)
            .contains(&offset)
            .then(|| 0x0400_0000 | ((((offset - 0x34) / 4) as u32 & 3) << 10))
    }
}

impl Peripheral for Rp2350Qmi {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if offset & 3 != 0 {
            return Err(SimulationError::MemoryViolation(0x400d_0000 + offset));
        }
        if let Some((window, reg)) = Self::memory_index(offset) {
            return Ok(self.memory[window][reg]);
        }
        if let Some(value) = Self::identity_translation(offset) {
            return Ok(value);
        }
        match offset {
            // Retain the native configuration/reset value; FIFO operations
            // are unsupported and may not report synthetic progress.
            0 => Ok(self.direct_csr),
            4 | 8 => Err(SimulationError::NotImplemented(
                "RP2350 QMI direct serial FIFO is not modeled".into(),
            )),
            _ => Err(SimulationError::MemoryViolation(0x400d_0000 + offset)),
        }
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 != 0 {
            return Err(SimulationError::MemoryViolation(0x400d_0000 + offset));
        }
        if let Some((window, reg)) = Self::memory_index(offset) {
            self.memory[window][reg] = value & QMI_MEM_MASK[reg];
            return Ok(());
        }
        if let Some(identity) = Self::identity_translation(offset) {
            return if value & 0x07ff_0fff == identity {
                Ok(())
            } else {
                Err(SimulationError::NotImplemented(
                    "RP2350 QMI non-identity address translation is not modeled".into(),
                ))
            };
        }
        match offset {
            // Configuration may be retained, but starting/asserting a direct
            // transfer must fail before a caller can mistake it for success.
            0 if value & 0xcd == 0 => {
                self.direct_csr = value & 0xffc0_0000;
                Ok(())
            }
            0 | 4 | 8 => Err(SimulationError::NotImplemented(
                "RP2350 QMI direct serial transfers are not modeled".into(),
            )),
            _ => Err(SimulationError::MemoryViolation(0x400d_0000 + offset)),
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_u32(offset & !3)? >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // RP2350 narrow APB writes replicate the byte across all lanes.
        self.write_u32(offset & !3, u32::from(value) * 0x0101_0101)
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write_u32(offset & !3, u32::from(value) * 0x0001_0001)
    }

    fn on_peripheral_reset(&mut self, _cause: crate::PeripheralResetCause) {
        *self = Self::new();
    }
    fn on_system_reset(&mut self) {
        *self = Self::new();
    }
}
