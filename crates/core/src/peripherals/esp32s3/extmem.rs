// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 EXTMEM cache controller (`0x600C_4000`).
//!
//! The boot ROM and ESP-IDF drive cache invalidate / writeback / sync and
//! preload operations through this block. Each operation register follows the
//! same hardware contract: firmware writes an *enable* bit to launch the
//! operation, the controller performs it, then sets a *done* status bit and
//! auto-clears the enable bit. Firmware busy-polls the done bit.
//!
//! On real silicon these operations take a handful of cache cycles; the
//! simulator has no cache latency, so we complete them atomically on the
//! launching write — set the done bit, clear the enable bits — exactly as the
//! firmware's poll expects.
//!
//! ## Verified against silicon (ESP32-S3, JTAG `mdw`)
//!
//! `CACHE_SYNC_CTRL_REG` (offset 0x28) rests at `0x0000_0008` after the ROM's
//! cache init — bit 3 (`CACHE_SYNC_DONE`) set, enable bits clear. The BROM
//! routine at `0x4004e54d` launches a sync by setting bit 0 then spins on
//! `bnone a9, 8` until bit 3 appears. Modeling that handshake clears the wall.

use crate::{Peripheral, SimResult};
use std::collections::HashMap;

/// One-shot launch/done control registers: `(offset, launch_mask, done_bit)`.
/// Firmware sets a launch bit, then busy-polls a done bit; on real silicon the
/// operation runs in a few cache cycles and the controller asserts done +
/// auto-clears the launch bits. The simulator has no cache latency, so we
/// complete the op atomically on the launching write.
///   0x28  CACHE_SYNC_CTRL — launch bits[2:0] (invalidate/writeback/clean),
///                           done bit 3 (SYNC_DONE).
const LAUNCH_DONE_REGS: [(u64, u32, u32); 1] = [(0x28, 0b111, 1 << 3)];

/// Request/acknowledge handshake registers: `(offset, request_bit, ack_bit)`.
/// Unlike the one-shot launch/done regs, these hold a *level*: firmware asserts
/// the request bit and waits for the ack bit to follow high, then deasserts the
/// request and waits for ack to follow low. The controller drives ack to mirror
/// the request once the (de)assertion takes effect. Reverse-engineered from the
/// boot ROM's cache-freeze/suspend pair and confirmed against silicon:
///   0x150 cache freeze/suspend — request bit 0, ack bit 2. Rests at 0 (idle).
///         Func @0x4004e910 sets bit 0, polls bit 2 high; func @0x4004e950
///         clears bit 0, polls bit 2 low. A static seed can't satisfy both —
///         the ack must track the request.
/// 0x150/0x154/0x158/0x15c are the contiguous cache freeze/suspend/sync op
/// registers (ROM cache routines + the SMP per-core cache ops), all bit0
/// request / bit2 ack. The PRO_CPU's SMP cache op (ROM @0x4004e8a8) launches
/// 0x154 and polls bit2 — without the mirror it spins forever, deadlocking the
/// APP_CPU's call_start_cpu1 wait.
const ACK_MIRROR_REGS: [(u64, u32, u32); 4] = [
    (0x150, 1 << 0, 1 << 2),
    (0x154, 1 << 0, 1 << 2),
    (0x158, 1 << 0, 1 << 2),
    (0x15c, 1 << 0, 1 << 2),
];

/// EXTMEM_CACHE_SYNC_CTRL_REG offset (referenced by tests/docs). bit0
/// INVALIDATE_ENA, bit1 WRITEBACK_ENA, bit2 CLEAN_ENA, bit3 SYNC_DONE.
#[allow(dead_code)]
const CACHE_SYNC_CTRL: u64 = 0x28;
#[allow(dead_code)]
const SYNC_ENABLE_BITS: u32 = 0b111;
#[allow(dead_code)]
const SYNC_DONE_BIT: u32 = 1 << 3;

/// Hardware-driven idle/status registers the boot ROM busy-polls but never
/// launches — `(offset, idle value)`. These hold completion/state bits the
/// controller asserts when no operation is pending. Every value was read off
/// silicon via JTAG `mdw` while the chip sat idle:
/// All the cache-block registers read off silicon as nonzero while idle. These
/// hold completion/state/default bits the boot ROM (and the ROM cache routines
/// the 2nd-stage bootloader calls) poll or read before programming:
///   0x28  CACHE_SYNC_CTRL   = 0x8    (SYNC_DONE)      — also dynamically driven
///   0x40  ICACHE state      = 0x2    (bit 1 done/idle; launch bit 0 ORs in)
///   0x4c  autoload/preload  = 0x8    (DONE bit 3)
///   0x60  cache config      = 0xf    (ways/size default)
///   0x68  cache config      = 0x4
///   0x7c  cache config      = 0x4
///   0x88  cache op state    = 0x2    (bit 1 done/idle; launch bit 0 ORs in)
///   0x94  DCACHE state      = 0x2    (bit 1 done/idle)
///   0xa0  autoload/preload  = 0x8    (DONE bit 3)
///   0x130 CACHE_STATE       = 0x1001 (ICACHE bits[11:0]=1, DCACHE bits[23:12]=1
///                                     — both caches enabled/idle)
/// We never seed the runtime MMU-tag / address registers (0xb0+), which the ROM
/// programs itself from reset; those would corrupt a from-reset boot.
const IDLE_STATUS_SEEDS: [(u64, u32); 10] = [
    (0x28, 1 << 3),
    (0x40, 1 << 1),
    (0x4c, 1 << 3),
    (0x60, 0xf),
    (0x68, 0x4),
    (0x7c, 0x4),
    (0x88, 1 << 1),
    (0x94, 1 << 1),
    (0xa0, 1 << 3),
    (0x130, 0x1001),
];

#[derive(Debug)]
pub struct Esp32s3Extmem {
    words: HashMap<u64, u32>,
}

impl Default for Esp32s3Extmem {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32s3Extmem {
    pub fn new() -> Self {
        let mut words = HashMap::new();
        // Seed the hardware-driven idle/status registers to their silicon
        // resting values so the boot ROM's completion polls exit immediately.
        for (off, val) in IDLE_STATUS_SEEDS {
            words.insert(off, val);
        }
        Self { words }
    }

    fn reg(&self, off: u64) -> u32 {
        self.words.get(&off).copied().unwrap_or(0)
    }

    /// Apply hardware side effects after a full-word write settles.
    fn on_word_write(&mut self, word_off: u64) {
        for (off, launch_mask, done_bit) in LAUNCH_DONE_REGS {
            if word_off == off {
                let v = self.reg(off);
                if v & launch_mask != 0 {
                    // Launch requested: complete instantly — clear the launch
                    // bits (hardware auto-clears them) and assert the done bit.
                    self.words.insert(off, (v & !launch_mask) | done_bit);
                }
            }
        }
        for (off, request_bit, ack_bit) in ACK_MIRROR_REGS {
            if word_off == off {
                let v = self.reg(off);
                // Drive the ack/state bit to mirror the request bit.
                let new = if v & request_bit != 0 {
                    v | ack_bit
                } else {
                    v & !ack_bit
                };
                if new != v {
                    self.words.insert(off, new);
                }
            }
        }
    }
}

impl Peripheral for Esp32s3Extmem {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.reg(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let entry = self.words.entry(word_off).or_insert(0);
        *entry &= !(0xFFu32 << byte_off);
        *entry |= (value as u32) << byte_off;
        self.on_word_write(word_off);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let word_off = offset & !3;
        self.words.insert(word_off, value);
        self.on_word_write(word_off);
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        let words: Vec<(u64, u32)> = self.words.iter().map(|(k, v)| (*k, *v)).collect();
        bincode::serialize(&words).expect("bincode serialize Esp32s3Extmem")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        let words: Vec<(u64, u32)> = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("Esp32s3Extmem snapshot decode: {e}"))
        })?;
        self.words = words.into_iter().collect();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_done_set_at_power_on() {
        let e = Esp32s3Extmem::new();
        // Matches silicon resting value 0x0000_0008.
        assert_eq!(e.read_u32(CACHE_SYNC_CTRL).unwrap(), SYNC_DONE_BIT);
    }

    #[test]
    fn idle_status_seeds_match_silicon() {
        let e = Esp32s3Extmem::new();
        // Each polled status register rests at its silicon-measured idle value.
        for (off, val) in IDLE_STATUS_SEEDS {
            assert_eq!(e.read_u32(off).unwrap(), val, "reg 0x{off:x}");
        }
    }

    #[test]
    fn launching_sync_completes_and_sets_done() {
        let mut e = Esp32s3Extmem::new();
        // Firmware sets INVALIDATE_ENA (bit 0) to launch.
        e.write_u32(CACHE_SYNC_CTRL, 0b0001).unwrap();
        let v = e.read_u32(CACHE_SYNC_CTRL).unwrap();
        // Enable bit auto-cleared, DONE asserted — the firmware's
        // `bnone a9, 8` poll exits on the next read.
        assert_eq!(v & SYNC_ENABLE_BITS, 0, "enable bits should auto-clear");
        assert_eq!(v & SYNC_DONE_BIT, SYNC_DONE_BIT, "DONE should be set");
    }

    #[test]
    fn byte_write_launch_also_completes() {
        let mut e = Esp32s3Extmem::new();
        // Same launch via the RMW byte path the ROM actually uses.
        e.write(CACHE_SYNC_CTRL, 0x01).unwrap();
        assert_eq!(e.read(CACHE_SYNC_CTRL).unwrap() & 0x08, 0x08);
        assert_eq!(e.read(CACHE_SYNC_CTRL).unwrap() & 0x07, 0x00);
    }

    #[test]
    fn reg_0x150_ack_mirrors_request() {
        let mut e = Esp32s3Extmem::new();
        // Rests at 0 (idle) — confirmed off silicon.
        assert_eq!(e.read_u32(0x150).unwrap(), 0);
        // Func A: assert request bit 0 → ack bit 2 follows high (request stays).
        e.write_u32(0x150, 1 << 0).unwrap();
        let v = e.read_u32(0x150).unwrap();
        assert_eq!(
            v & (1 << 0),
            1 << 0,
            "request bit must persist (level-held)"
        );
        assert_eq!(v & (1 << 2), 1 << 2, "ack bit 2 should follow request high");
        // Func B: clear request bit 0 → ack bit 2 follows low.
        e.write_u32(0x150, v & !(1 << 0)).unwrap();
        let v = e.read_u32(0x150).unwrap();
        assert_eq!(v & (1 << 2), 0, "ack bit 2 should follow request low");
    }

    #[test]
    fn other_registers_round_trip() {
        let mut e = Esp32s3Extmem::new();
        e.write_u32(0x2c, 0x3c05_0000).unwrap();
        e.write_u32(0x30, 0x0000_0400).unwrap();
        assert_eq!(e.read_u32(0x2c).unwrap(), 0x3c05_0000);
        assert_eq!(e.read_u32(0x30).unwrap(), 0x0000_0400);
    }
}
