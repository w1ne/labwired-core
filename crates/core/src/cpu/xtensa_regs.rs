// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa AR register file with Windowed Registers Option + PS struct.

/// 64-entry physical AR file indexed via WindowBase. Logical registers
/// a0..a15 map to physical[(WindowBase*4 + idx) mod 64].
#[derive(Debug, Clone)]
pub struct ArFile {
    phys: [u32; 64],
    window_base: u8,   // 0..15
    window_start: u16, // 16 bits
}

impl Default for ArFile {
    fn default() -> Self { Self::new() }
}

impl ArFile {
    pub fn new() -> Self {
        // Xtensa reset: WindowBase=0, WindowStart=0x1 (bit 0 set — a0..a3 frame).
        Self { phys: [0; 64], window_base: 0, window_start: 0x1 }
    }

    pub fn windowbase(&self) -> u8 { self.window_base }
    pub fn set_windowbase(&mut self, v: u8) { self.window_base = v & 0x0F; }

    pub fn windowstart(&self) -> u16 { self.window_start }
    pub fn set_windowstart(&mut self, v: u16) { self.window_start = v; }

    pub fn windowstart_bit(&self, idx: u8) -> bool {
        (self.window_start >> (idx & 0xF)) & 1 == 1
    }

    pub fn set_windowstart_bit(&mut self, idx: u8, v: bool) {
        let b = idx & 0xF;
        if v {
            self.window_start |= 1 << b;
        } else {
            self.window_start &= !(1 << b);
        }
    }

    pub fn physical(&self, phys_idx: usize) -> u32 { self.phys[phys_idx & 63] }
    pub fn set_physical(&mut self, phys_idx: usize, v: u32) { self.phys[phys_idx & 63] = v; }

    #[inline]
    fn logical_to_physical(&self, logical: u8) -> usize {
        ((self.window_base as usize * 4) + logical as usize) & 63
    }

    pub fn read_logical(&self, logical: u8) -> u32 {
        self.phys[self.logical_to_physical(logical & 0xF)]
    }

    pub fn write_logical(&mut self, logical: u8, v: u32) {
        let p = self.logical_to_physical(logical & 0xF);
        self.phys[p] = v;
    }
}

/// Processor State (PS) fielded.
///
/// Bit layout per Xtensa ISA RM:
/// - `[3:0]`   INTLEVEL
/// - `[4]`     EXCM (exception mode)
/// - `[7:6]`   RING (privilege ring)
/// - `[11:8]`  OWB (old windowbase, for window exception)
/// - `[17:16]` CALLINC (set by CALL*; used by ENTRY)
/// - `[18]`    WOE (window overflow enable)
#[derive(Debug, Clone, Copy)]
pub struct Ps(u32);

impl Ps {
    pub fn from_raw(raw: u32) -> Self { Self(raw) }
    pub fn as_raw(self) -> u32 { self.0 }

    #[inline] pub fn intlevel(self) -> u8 { (self.0 & 0xF) as u8 }
    #[inline] pub fn set_intlevel(&mut self, v: u8) {
        self.0 = (self.0 & !0xF) | (v as u32 & 0xF);
    }

    #[inline] pub fn excm(self) -> bool { (self.0 >> 4) & 1 == 1 }
    #[inline] pub fn set_excm(&mut self, v: bool) {
        if v { self.0 |= 1 << 4 } else { self.0 &= !(1 << 4) }
    }

    #[inline] pub fn ring(self) -> u8 { ((self.0 >> 6) & 0x3) as u8 }
    #[inline] pub fn set_ring(&mut self, v: u8) {
        self.0 = (self.0 & !(0x3 << 6)) | ((v as u32 & 0x3) << 6);
    }

    #[inline] pub fn owb(self) -> u8 { ((self.0 >> 8) & 0xF) as u8 }
    #[inline] pub fn set_owb(&mut self, v: u8) {
        self.0 = (self.0 & !(0xF << 8)) | ((v as u32 & 0xF) << 8);
    }

    #[inline] pub fn callinc(self) -> u8 { ((self.0 >> 16) & 0x3) as u8 }
    #[inline] pub fn set_callinc(&mut self, v: u8) {
        self.0 = (self.0 & !(0x3 << 16)) | ((v as u32 & 0x3) << 16);
    }

    #[inline] pub fn woe(self) -> bool { (self.0 >> 18) & 1 == 1 }
    #[inline] pub fn set_woe(&mut self, v: bool) {
        if v { self.0 |= 1 << 18 } else { self.0 &= !(1 << 18) }
    }
}
