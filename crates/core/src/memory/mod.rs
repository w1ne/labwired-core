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

/// A simple flat memory storage
pub struct LinearMemory {
    pub data: Vec<u8>,
    pub base_addr: u64,
}

impl LinearMemory {
    pub fn new(size: usize, base_addr: u64) -> Self {
        Self {
            data: vec![0; size],
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
