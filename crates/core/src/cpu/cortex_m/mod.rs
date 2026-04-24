// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::decoder::arm::Instruction;
use crate::{Bus, Cpu, SimResult, SimulationObserver};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

mod helpers;
mod step;
mod step_thumb2;

// Register-file indices. Reads/writes to the `regs` array use these.
pub const SP: usize = 13; // R13 — Stack Pointer
pub const LR: usize = 14; // R14 — Link Register
pub const PC: usize = 15; // R15 — Program Counter
pub const XPSR: usize = 16;

/// Direct-mapped decode cache size. Must be a power of two. A 256-entry
/// cache covers ~512 bytes of hot code per way, which is enough for the
/// inner loops of typical embedded firmware.
const DECODE_CACHE_SIZE: usize = 256;
const DECODE_CACHE_MASK: usize = DECODE_CACHE_SIZE - 1;

#[derive(Clone, Copy)]
struct DecodeCacheEntry {
    // `pc` is the full 32-bit instruction address that produced this decode.
    // Sentinel value `u32::MAX` (never a legal Thumb PC — would be unaligned)
    // marks the slot empty.
    pc: u32,
    opcode: u16,
    instruction: Instruction,
}

impl DecodeCacheEntry {
    const EMPTY: Self = Self {
        pc: u32::MAX,
        opcode: 0,
        instruction: Instruction::Unknown(0),
    };
}

pub struct CortexM {
    /// General-purpose + special registers: R0..R12, SP (13), LR (14), PC (15), XPSR (16).
    pub regs: [u32; 17],
    pub pending_exceptions: u32, // Bitmask
    pub primask: bool,           // Interrupt mask (true = disabled)
    pub vtor: Arc<AtomicU32>,    // Shared Vector Table Offset Register
    /// Direct-mapped decode cache. Flushed on reset / apply_snapshot.
    /// Self-modifying code writing to flash at runtime is not tracked; this
    /// is a deliberate trade-off — documented in docs/architecture.md.
    decode_cache: Box<[DecodeCacheEntry; DECODE_CACHE_SIZE]>,
    /// Running counters: number of decode-cache hits vs misses since the
    /// last flush. Reset by `decode_flush` and `reset`.
    decode_hits: u64,
    decode_misses: u64,
}

impl std::fmt::Debug for CortexM {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CortexM")
            .field("regs", &self.regs)
            .field("pending_exceptions", &self.pending_exceptions)
            .field("primask", &self.primask)
            .field("vtor", &self.vtor)
            .finish_non_exhaustive()
    }
}

impl Default for CortexM {
    fn default() -> Self {
        Self {
            regs: [0; 17],
            pending_exceptions: 0,
            primask: false,
            vtor: Arc::default(),
            decode_cache: Box::new([DecodeCacheEntry::EMPTY; DECODE_CACHE_SIZE]),
            decode_hits: 0,
            decode_misses: 0,
        }
    }
}

impl CortexM {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_vtor(&self) -> u32 {
        self.vtor.load(Ordering::SeqCst)
    }

    pub fn set_vtor(&mut self, val: u32) {
        self.vtor.store(val, Ordering::SeqCst);
    }

    pub fn set_shared_vtor(&mut self, vtor: Arc<AtomicU32>) {
        self.vtor = vtor;
    }

    /// Look up a previously decoded instruction at `pc` in the decode
    /// cache. In debug builds we also tick the hit / miss counters; in
    /// release builds the counters stay zero and the lookup compiles to
    /// a pure tag compare + branch on the hot path.
    pub(super) fn decode_lookup(&mut self, pc: u32) -> Option<(u16, Instruction)> {
        let idx = (pc >> 1) as usize & DECODE_CACHE_MASK;
        let e = &self.decode_cache[idx];
        if e.pc == pc {
            #[cfg(debug_assertions)]
            {
                self.decode_hits = self.decode_hits.wrapping_add(1);
            }
            Some((e.opcode, e.instruction))
        } else {
            #[cfg(debug_assertions)]
            {
                self.decode_misses = self.decode_misses.wrapping_add(1);
            }
            None
        }
    }

    /// Record a fresh decode result so future fetches at the same `pc` hit.
    pub(super) fn decode_store(&mut self, pc: u32, opcode: u16, instruction: Instruction) {
        let idx = (pc >> 1) as usize & DECODE_CACHE_MASK;
        self.decode_cache[idx] = DecodeCacheEntry { pc, opcode, instruction };
    }

    /// Drop every cached decode and reset the hit / miss counters. Called
    /// on reset, snapshot restore, or any time the flash/ROM contents may
    /// have changed.
    pub(super) fn decode_flush(&mut self) {
        for e in self.decode_cache.iter_mut() {
            *e = DecodeCacheEntry::EMPTY;
        }
        self.decode_hits = 0;
        self.decode_misses = 0;
    }

    /// Cache performance counters since the last flush. Returns
    /// `(hits, misses)`; compute `hits / (hits + misses)` for a hit rate.
    /// Counters are only updated in debug builds — release builds always
    /// return `(0, 0)`. This keeps the interpreter hot path free of the
    /// per-step atomics-equivalent overhead in production.
    pub fn decode_cache_stats(&self) -> (u64, u64) {
        (self.decode_hits, self.decode_misses)
    }

    fn read_reg(&self, n: u8) -> u32 {
        self.regs.get(n as usize).copied().unwrap_or(0)
    }

    fn write_reg(&mut self, n: u8, val: u32) {
        if let Some(r) = self.regs.get_mut(n as usize) {
            *r = val;
        }
    }

    fn update_nz(&mut self, result: u32) {
        let n = (result >> 31) & 1;
        let z = if result == 0 { 1 } else { 0 };
        // Clear N/Z (bits 31, 30)
        self.regs[16] &= !(0xC000_0000);
        self.regs[16] |= (n << 31) | (z << 30);
    }

    fn update_nzcv(&mut self, result: u32, carry: bool, overflow: bool) {
        let n = (result >> 31) & 1;
        let z = if result == 0 { 1 } else { 0 };
        let c = if carry { 1 } else { 0 };
        let v = if overflow { 1 } else { 0 };

        self.regs[16] &= !(0xF000_0000);
        self.regs[16] |= (n << 31) | (z << 30) | (c << 29) | (v << 28);
    }

    fn check_condition(&self, cond: u8) -> bool {
        let n = (self.regs[16] >> 31) & 1 == 1;
        let z = (self.regs[16] >> 30) & 1 == 1;
        let c = (self.regs[16] >> 29) & 1 == 1;
        let v = (self.regs[16] >> 28) & 1 == 1;

        match cond {
            0x0 => z,              // EQ (Equal)
            0x1 => !z,             // NE (Not Equal)
            0x2 => c,              // CS/HS (Carry Set)
            0x3 => !c,             // CC/LO (Carry Clear)
            0x4 => n,              // MI (Minus)
            0x5 => !n,             // PL (Plus)
            0x6 => v,              // VS (Overflow)
            0x7 => !v,             // VC (No Overflow)
            0x8 => c && !z,        // HI (Unsigned Higher)
            0x9 => !c || z,        // LS (Unsigned Lower or Same)
            0xA => n == v,         // GE (Signed Greater or Equal)
            0xB => n != v,         // LT (Signed Less Than)
            0xC => !z && (n == v), // GT (Signed Greater Than)
            0xD => z || (n != v),  // LE (Signed Less or Equal)
            0xE => true,           // AL (Always)
            _ => false,            // Undefined/Reserved
        }
    }

    fn branch_to(&mut self, addr: u32, bus: &mut dyn Bus) -> SimResult<()> {
        if (addr & 0xF000_0000) == 0xF000_0000 {
            // EXC_RETURN logic
            self.exception_return(bus)?;
        } else {
            self.regs[15] = addr & !1;
        }
        Ok(())
    }

    fn exception_return(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        // Perform Unstacking
        let frame_ptr = self.regs[13];

        self.regs[0] = bus.read_u32(frame_ptr)?;
        self.regs[1] = bus.read_u32(frame_ptr + 4)?;
        self.regs[2] = bus.read_u32(frame_ptr + 8)?;
        self.regs[3] = bus.read_u32(frame_ptr + 12)?;
        self.regs[12] = bus.read_u32(frame_ptr + 16)?;
        self.regs[14] = bus.read_u32(frame_ptr + 20)?;
        self.regs[15] = bus.read_u32(frame_ptr + 24)?;
        self.regs[16] = bus.read_u32(frame_ptr + 28)?;

        self.regs[13] = frame_ptr + 32;

        tracing::info!("Exception return to {:#x}", self.regs[15]);
        Ok(())
    }
}

impl Cpu for CortexM {
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        self.regs[15] = 0x0000_0000;
        self.regs[13] = 0x2000_0000;
        self.pending_exceptions = 0;
        self.decode_flush();

        let vtor = self.vtor.load(Ordering::SeqCst);
        if let Ok(sp) = bus.read_u32(vtor) {
            self.regs[13] = sp;
        }
        if let Ok(pc) = bus.read_u32(vtor + 4) {
            self.regs[15] = pc;
        }

        Ok(())
    }

    fn get_pc(&self) -> u32 {
        self.regs[15]
    }
    fn set_pc(&mut self, val: u32) {
        self.regs[15] = val;
    }
    fn set_sp(&mut self, val: u32) {
        self.regs[13] = val;
    }
    fn set_exception_pending(&mut self, exception_num: u32) {
        if exception_num < 32 {
            self.pending_exceptions |= 1 << exception_num;
        }
    }

    fn get_register(&self, id: u8) -> u32 {
        self.read_reg(id)
    }

    fn set_register(&mut self, id: u8, val: u32) {
        self.write_reg(id, val);
    }

    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        crate::snapshot::CpuSnapshot::Arm(crate::snapshot::ArmCpuSnapshot {
            registers: self.regs[..16].to_vec(),
            xpsr: self.regs[16],
            primask: self.primask,
            pending_exceptions: self.pending_exceptions,
            vtor: self.vtor.load(Ordering::Relaxed),
        })
    }

    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        if let crate::snapshot::CpuSnapshot::Arm(s) = snapshot {
            let n = s.registers.len().min(16);
            self.regs[..n].copy_from_slice(&s.registers[..n]);
            self.regs[16] = s.xpsr;
            self.primask = s.primask;
            self.pending_exceptions = s.pending_exceptions;
            self.vtor.store(s.vtor, Ordering::Relaxed);
            // Snapshot restore implies the flash/vector-table contents may
            // differ from whatever we had cached; drop every decoded entry.
            self.decode_flush();
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for i in 0..13 {
            names.push(format!("R{}", i));
        }
        names.push("SP".to_string());
        names.push("LR".to_string());
        names.push("PC".to_string());
        names
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
    ) -> SimResult<()> {
        self.step_execute(bus, observers)
    }
}

