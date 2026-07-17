// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub start_addr: u64,
    pub data: Vec<u8>,
}

pub mod guest_buf;
pub use guest_buf::{GuestBuf, JIT_PREFIX_BYTES, WASM_PAGE_BYTES};

use crate::Arch;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramImage {
    pub entry_point: u64,
    pub segments: Vec<Segment>,
    pub arch: Arch,
}

impl ProgramImage {
    pub fn new(entry_point: u64, arch: Arch) -> Self {
        Self {
            entry_point,
            segments: Vec::new(),
            arch,
        }
    }

    pub fn add_segment(&mut self, start_addr: u64, data: Vec<u8>) {
        self.segments.push(Segment { start_addr, data });
    }
}

/// A simple flat memory storage.
///
/// `data` is a [`GuestBuf`], not a `Vec<u8>`: it derefs to the guest bytes
/// exactly like a `Vec` did, but its allocation is shaped so the RV32IMC
/// wasm-JIT can import it *directly* as a wasm linear memory. That is what
/// lets a compiled block and the interpreter share one copy of guest RAM
/// instead of marshalling the whole window in and out around every block run.
/// See [`guest_buf`] for the layout and the aliasing-soundness argument.
pub struct LinearMemory {
    pub data: GuestBuf,
    pub base_addr: u64,
}

impl LinearMemory {
    pub fn new(size: usize, base_addr: u64) -> Self {
        Self {
            data: GuestBuf::new(size),
            base_addr,
        }
    }

    pub fn read_u8(&self, addr: u64) -> Option<u8> {
        if addr >= self.base_addr && addr < self.base_addr + self.data.len() as u64 {
            Some(self.data[(addr - self.base_addr) as usize])
        } else {
            None
        }
    }

    pub fn write_u8(&mut self, addr: u64, value: u8) -> bool {
        if addr >= self.base_addr && addr < self.base_addr + self.data.len() as u64 {
            self.data[(addr - self.base_addr) as usize] = value;
            true
        } else {
            false
        }
    }

    pub fn read_u16(&self, addr: u64) -> Option<u16> {
        if addr >= self.base_addr && addr + 1 < self.base_addr + self.data.len() as u64 {
            let offset = (addr - self.base_addr) as usize;
            let bytes = [self.data[offset], self.data[offset + 1]];
            Some(u16::from_le_bytes(bytes))
        } else {
            None
        }
    }

    pub fn read_u32(&self, addr: u64) -> Option<u32> {
        if addr >= self.base_addr && addr + 3 < self.base_addr + self.data.len() as u64 {
            let offset = (addr - self.base_addr) as usize;
            let bytes = [
                self.data[offset],
                self.data[offset + 1],
                self.data[offset + 2],
                self.data[offset + 3],
            ];
            Some(u32::from_le_bytes(bytes))
        } else {
            None
        }
    }

    pub fn write_u16(&mut self, addr: u64, value: u16) -> bool {
        if addr >= self.base_addr && addr + 1 < self.base_addr + self.data.len() as u64 {
            let offset = (addr - self.base_addr) as usize;
            let bytes = value.to_le_bytes();
            self.data[offset] = bytes[0];
            self.data[offset + 1] = bytes[1];
            true
        } else {
            false
        }
    }

    pub fn write_u32(&mut self, addr: u64, value: u32) -> bool {
        if addr >= self.base_addr && addr + 3 < self.base_addr + self.data.len() as u64 {
            let offset = (addr - self.base_addr) as usize;
            let bytes = value.to_le_bytes();
            self.data[offset] = bytes[0];
            self.data[offset + 1] = bytes[1];
            self.data[offset + 2] = bytes[2];
            self.data[offset + 3] = bytes[3];
            true
        } else {
            false
        }
    }

    /// Fill `[offset, offset+len)` with `byte`, where `offset` is buffer-relative
    /// (i.e. `absolute_addr - self.base_addr`, the same buffer space `read_u8`/
    /// `write_u8` reach after subtracting `base_addr`). Returns false if the
    /// range is outside the backing buffer.
    pub fn fill(&mut self, offset: u64, len: u64, byte: u8) -> bool {
        let (Ok(start), Some(Ok(end))) = (
            usize::try_from(offset),
            offset.checked_add(len).map(usize::try_from),
        ) else {
            return false;
        };
        if end > self.data.len() {
            return false;
        }
        self.data[start..end].iter_mut().for_each(|b| *b = byte);
        true
    }

    /// Swap the two `bank_size` halves of the buffer in place (models H5
    /// hardware SWAP_BANK). Returns false unless the buffer is exactly two banks.
    pub fn swap_banks(&mut self, bank_size: u64) -> bool {
        let Some(total) = bank_size.checked_mul(2) else {
            return false;
        };
        let (Ok(bank), Ok(total)) = (usize::try_from(bank_size), usize::try_from(total)) else {
            return false;
        };
        if self.data.len() != total {
            return false;
        }
        let (lo, hi) = self.data.split_at_mut(bank);
        lo.swap_with_slice(hi);
        true
    }

    pub fn load_from_segment(&mut self, segment: &Segment) -> bool {
        // Simple overlap check
        let end_addr = segment.start_addr + segment.data.len() as u64;
        let mem_end = self.base_addr + self.data.len() as u64;

        if segment.start_addr >= self.base_addr && end_addr <= mem_end {
            let offset = (segment.start_addr - self.base_addr) as usize;
            self.data[offset..offset + segment.data.len()].copy_from_slice(&segment.data);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod bank_tests {
    use super::LinearMemory;

    #[test]
    fn fill_sets_range_relative_to_base() {
        let mut m = LinearMemory::new(0x4000, 0x0800_0000);
        assert!(m.fill(0x2000, 0x2000, 0xFF));
        assert_eq!(m.read_u8(0x0800_1FFF).unwrap(), 0x00);
        assert_eq!(m.read_u8(0x0800_2000).unwrap(), 0xFF);
        assert_eq!(m.read_u8(0x0800_3FFF).unwrap(), 0xFF);
    }

    #[test]
    fn fill_rejects_out_of_range() {
        let mut m = LinearMemory::new(0x2000, 0x0800_0000);
        assert!(!m.fill(0x1000, 0x2000, 0xFF));
    }

    #[test]
    fn swap_banks_exchanges_halves() {
        let mut m = LinearMemory::new(0x4, 0x0800_0000); // tiny 2-byte banks
        m.write_u8(0x0800_0000, 0xA1);
        m.write_u8(0x0800_0001, 0xA2);
        m.write_u8(0x0800_0002, 0xB1);
        m.write_u8(0x0800_0003, 0xB2);
        assert!(m.swap_banks(0x2));
        assert_eq!(m.read_u8(0x0800_0000).unwrap(), 0xB1);
        assert_eq!(m.read_u8(0x0800_0001).unwrap(), 0xB2);
        assert_eq!(m.read_u8(0x0800_0002).unwrap(), 0xA1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_read_write() {
        let mut mem = LinearMemory::new(1024, 0x1000);

        // Valid write
        assert!(mem.write_u8(0x1000, 42));
        assert!(mem.write_u8(0x13FF, 99)); // Last byte

        // Invalid write (out of bounds)
        assert!(!mem.write_u8(0x0FFF, 1));
        assert!(!mem.write_u8(0x1400, 1));

        // Valid read
        assert_eq!(mem.read_u8(0x1000), Some(42));
        assert_eq!(mem.read_u8(0x13FF), Some(99));

        // Invalid read
        assert_eq!(mem.read_u8(0x0FFF), None);
        assert_eq!(mem.read_u8(0x1400), None);
    }

    #[test]
    fn test_load_from_segment() {
        let mut mem = LinearMemory::new(1024, 0x1000);

        // Segment 1: Fits inside
        let seg1 = Segment {
            start_addr: 0x1000,
            data: vec![1, 2, 3],
        };
        assert!(mem.load_from_segment(&seg1));
        assert_eq!(mem.read_u8(0x1000), Some(1));

        // Segment 2: Overlaps end boundary (should fail)
        let seg2 = Segment {
            start_addr: 0x13FE,
            data: vec![10, 20, 30], // 3 bytes: 13FE, 13FF, 1400 (out)
        };
        assert!(!mem.load_from_segment(&seg2));

        // Verify partial write didn't happen (atomic load not guaranteed but check logic)
        assert_eq!(mem.read_u8(0x13FF), Some(0)); // Still 0

        // Segment 3: Exact fit at the end
        let seg3 = Segment {
            start_addr: 0x13FE,
            data: vec![0xAA, 0xBB], // 2 bytes: 13FE, 13FF. Fits.
        };
        assert!(mem.load_from_segment(&seg3));
        assert_eq!(mem.read_u8(0x13FE), Some(0xAA));
        assert_eq!(mem.read_u8(0x13FF), Some(0xBB));
    }
}
