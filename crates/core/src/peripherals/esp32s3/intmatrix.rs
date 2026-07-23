// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Interrupt Matrix peripheral for ESP32-S3.
//!
//! Per ESP32-S3 TRM §9.4. The interrupt matrix routes 99 peripheral
//! source IDs to 32 cpu0 IRQ slots AND a parallel 32 cpu1 IRQ slots.
//!
//! Both cores' map tables live in one MMIO block based at
//! `DR_REG_INTERRUPT_BASE = 0x600C_2000` (`DR_REG_INTERRUPT_CORE0_BASE`
//! and `_CORE1_BASE` are both aliases of it). Each peripheral source ID
//! has a 32-bit map register at:
//!   INTERRUPT_CORE0_<source>_MAP_REG = 0x000 + 4 * source_id   (cpu0)
//!   INTERRUPT_CORE1_<source>_MAP_REG = 0x800 + 4 * source_id   (cpu1)
//! (verified against esp-idf `soc/interrupt_core{0,1}_reg.h`: CORE1's
//! MAC_INTR_MAP — source 0 — sits at `BASE + 0x800`, and FROM_CPU_1 —
//! source 80 — at `BASE + 0x940 = 0x800 + 80*4`).
//!
//! ESP-IDF SMP uses this duality for the cross-core interrupt: each core
//! routes its own `FROM_CPU_INTR{core}` source (79 for core 0, 80 for
//! core 1) — see [`crate::peripherals::esp32s3::crosscore_ipi`].
//!
//! The register stores the cpu IRQ slot (0..31) the source delivers to.
//! Slot 0 is reserved for software interrupts; we treat any value 0..31
//! as a valid binding and only return None when the peripheral has never
//! been written. (TRM "0 = disconnect" is encoded by not writing / writing
//! then clearing the Option — see write path; live bindings keep 0..31.)
//!
//! ## Status registers (PRO_INTR_STATUS_REG_0..3)
//!
//! Per esp32s3-pac 0.35.2 `interrupt_core0`, four 32-bit status registers
//! at offsets 0x18C..0x19C (covering source bits 0..31, 32..63, 64..95,
//! 96..98) reflect which peripheral source IDs are currently asserting.
//! esp-hal's `__level_1_interrupt` reads these to discover which source
//! triggered the IRQ before invoking the bound handler from `__INTERRUPTS`.
//! The bus aggregator updates this set every tick from the peripheral
//! `explicit_irqs` aggregation (see `Bus::tick_peripherals_with_costs`).

use crate::{Peripheral, SimResult};

const NUM_SOURCES: usize = 99;

/// CORE1's map table is offset `0x800` from the shared interrupt base;
/// `BASE + 0x800 + 4*source_id` is `INTERRUPT_CORE1_<source>_MAP_REG`.
const CORE1_MAP_OFFSET: u64 = 0x800;

/// Offsets of PRO_INTR_STATUS_REG_0..3 (per esp32s3-pac).
/// Each reg covers 32 source bits; 4 regs × 32 = 128 ≥ NUM_SOURCES (99).
const INTR_STATUS_BASE: u64 = 0x18C;
const INTR_STATUS_END: u64 = 0x19C; // exclusive

#[derive(Debug)]
pub struct Esp32s3IntMatrix {
    /// For each peripheral source ID (0..99), which cpu0 IRQ slot it's
    /// bound to. None = never written (no binding).
    cpu0_route: [Option<u8>; NUM_SOURCES],
    /// Parallel cpu1 (APP_CPU) binding table, programmed via the CORE1
    /// map window at `CORE1_MAP_OFFSET`.
    cpu1_route: [Option<u8>; NUM_SOURCES],
    /// PRO_INTR_STATUS_REG_0..3 — bit `i` of word `n` reflects whether
    /// source `n*32 + i` is currently asserting. Updated each tick by the
    /// bus from peripheral `explicit_irqs`. The CORE1 status mirror reads
    /// the same raw source-assertion bitmap (assertion is core-independent;
    /// only routing differs).
    intr_status: [u32; 4],
}

impl Esp32s3IntMatrix {
    pub fn new() -> Self {
        Self {
            cpu0_route: [None; NUM_SOURCES],
            cpu1_route: [None; NUM_SOURCES],
            intr_status: [0u32; 4],
        }
    }

    /// Look up the cpu0 IRQ slot for `source_id` (back-compat alias for
    /// `route_for_core(source_id, 0)`).
    pub fn route(&self, source_id: u32) -> Option<u8> {
        self.route_for_core(source_id, 0)
    }

    /// Look up the IRQ slot `source_id` is bound to on the given core
    /// (0 = PRO_CPU, 1 = APP_CPU). Returns None if unbound or out of range.
    pub fn route_for_core(&self, source_id: u32, core_id: u8) -> Option<u8> {
        let idx = source_id as usize;
        if idx >= NUM_SOURCES {
            return None;
        }
        if core_id == 0 {
            self.cpu0_route[idx]
        } else {
            self.cpu1_route[idx]
        }
    }

    /// Replace the per-tick set of asserting source IDs. Called from the
    /// bus aggregator after each peripheral tick. `sources` is a 4-word
    /// bitmap (word 0: bits 0..31, word 1: 32..63, …).
    pub fn set_pending_sources(&mut self, sources: [u32; 4]) {
        self.intr_status = sources;
    }
}

impl Default for Esp32s3IntMatrix {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32s3IntMatrix {
    fn read(&self, offset: u64) -> SimResult<u8> {
        // Layout (each core's half is 0x800 apart on the shared base):
        //   CORE0: 0x000..0x18C — per-source map registers (route slot).
        //          0x18C..0x19C — PRO_INTR_STATUS_REG_0..3 (assertion bitmap).
        //   CORE1: 0x800..0x98C — per-source map registers (cpu1 route).
        //          0x98C..0x99C — APP_INTR_STATUS_REG_0..3 (same bitmap mirror).
        //   everything else — read-as-zero (CLOCK_GATE / VERSION not modeled).
        let (core_id, rel) = if offset >= CORE1_MAP_OFFSET {
            (1u8, offset - CORE1_MAP_OFFSET)
        } else {
            (0u8, offset)
        };
        if rel < INTR_STATUS_BASE {
            let word_off = rel & !3;
            let byte_off = (rel & 3) * 8;
            let src = (word_off / 4) as usize;
            let table = if core_id == 0 {
                &self.cpu0_route
            } else {
                &self.cpu1_route
            };
            let slot = if src < NUM_SOURCES {
                table[src].unwrap_or(0) as u32
            } else {
                0
            };
            Ok(((slot >> byte_off) & 0xFF) as u8)
        } else if rel < INTR_STATUS_END {
            let s = rel - INTR_STATUS_BASE;
            let word_off = (s & !3) / 4;
            let byte_off = (s & 3) * 8;
            let word = self.intr_status[word_off as usize];
            Ok(((word >> byte_off) & 0xFF) as u8)
        } else {
            Ok(0)
        }
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // INTR_STATUS regs are read-only on real silicon; CLOCK_GATE /
        // VERSION are accepted but ignored. Map writes bind the source on
        // the addressed core's half (offset >= 0x800 → CORE1 / cpu1).
        let (core_id, rel) = if offset >= CORE1_MAP_OFFSET {
            (1u8, offset - CORE1_MAP_OFFSET)
        } else {
            (0u8, offset)
        };
        if rel >= INTR_STATUS_BASE {
            return Ok(());
        }
        let word_off = rel & !3;
        let byte_off = (rel & 3) * 8;
        let src = (word_off / 4) as usize;
        if src >= NUM_SOURCES {
            return Ok(());
        }
        let table = if core_id == 0 {
            &mut self.cpu0_route
        } else {
            &mut self.cpu1_route
        };
        // R-M-W the addressed byte of the source's map word; slot is bits[4:0].
        let mut word = table[src].unwrap_or(0) as u32;
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        table[src] = Some((word & 0x1F) as u8);
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_unbound() {
        let m = Esp32s3IntMatrix::new();
        for src in 0u32..99 {
            assert!(m.route(src).is_none(), "source {src} should be unbound");
        }
    }

    #[test]
    fn out_of_range_source_returns_none() {
        let m = Esp32s3IntMatrix::new();
        assert!(m.route(99).is_none());
        assert!(m.route(1000).is_none());
    }

    #[test]
    fn bind_source_via_mmio_write() {
        let mut m = Esp32s3IntMatrix::new();
        // Bind SYSTIMER_TARGET0 (source 79) to cpu0 IRQ slot 15.
        // Offset = 79 * 4 = 316 = 0x13C. Write byte 0 = 15.
        let off = 79 * 4u64;
        m.write(off, 15).unwrap();
        m.write(off + 1, 0).unwrap();
        m.write(off + 2, 0).unwrap();
        m.write(off + 3, 0).unwrap();
        assert_eq!(m.route(79), Some(15));
    }

    #[test]
    fn bind_max_source() {
        let mut m = Esp32s3IntMatrix::new();
        // Source 98 (last valid).
        let off = 98 * 4u64;
        m.write(off, 7).unwrap();
        assert_eq!(m.route(98), Some(7));
    }

    #[test]
    fn slot_only_lower_5_bits_kept() {
        let mut m = Esp32s3IntMatrix::new();
        // Write 0x3F (would be IRQ 63 if all bits used; ESP32-S3 has 32 slots).
        m.write(0, 0x3F).unwrap();
        // Spec says bits[4:0] only.
        assert_eq!(m.route(0), Some(0x1F));
    }

    #[test]
    fn read_back_bound_source() {
        let mut m = Esp32s3IntMatrix::new();
        let off = 5 * 4u64;
        m.write(off, 12).unwrap();
        assert_eq!(m.read(off).unwrap(), 12);
        assert_eq!(m.read(off + 1).unwrap(), 0);
    }

    #[test]
    fn app_map_writes_silently_accepted() {
        let mut m = Esp32s3IntMatrix::new();
        m.write(0x400, 0xAB).unwrap();
        assert_eq!(m.read(0x400).unwrap(), 0);
        // PRO route for source 0 unaffected.
        assert!(m.route(0).is_none());
    }

    #[test]
    fn out_of_range_pro_offset_silently_dropped() {
        let mut m = Esp32s3IntMatrix::new();
        // Anything in 0x18C..0x400 outside the per-source map area is reserved for
        // INTR_STATUS / CLOCK_GATE; writes are dropped (no route binding).
        m.write(99 * 4, 0xAB).unwrap();
        assert!(m.route(99).is_none());
    }

    #[test]
    fn intr_status_word_read_returns_pending_sources() {
        // SYSTIMER_TARGET0 = source 57 → bit 25 of word 1 (57 / 32 = 1, 57 % 32 = 25).
        let mut m = Esp32s3IntMatrix::new();
        m.set_pending_sources([0u32, 0x0200_0000, 0u32, 0u32]);
        // Read u32 at offset 0x190 (INTR_STATUS_REG_1) byte by byte and reassemble.
        let mut val = 0u32;
        for i in 0u64..4 {
            val |= (m.read(0x190 + i).unwrap() as u32) << (i * 8);
        }
        assert_eq!(val, 0x0200_0000);
    }

    #[test]
    fn intr_status_writes_dont_pollute_route_table() {
        // A spurious write to the INTR_STATUS area must not bind any source ID.
        let mut m = Esp32s3IntMatrix::new();
        m.write(0x190, 0xFF).unwrap();
        m.write(0x191, 0xFF).unwrap();
        m.write(0x192, 0xFF).unwrap();
        m.write(0x193, 0xFF).unwrap();
        for src in 0u32..99 {
            assert!(m.route(src).is_none(), "source {src} unexpectedly bound");
        }
    }

    #[test]
    fn core1_map_binds_cpu1_route_independently() {
        let mut m = Esp32s3IntMatrix::new();
        // FROM_CPU_1 = source 80, CORE1 map at 0x800 + 80*4 = 0x940.
        let off = CORE1_MAP_OFFSET + 80 * 4;
        m.write(off, 9).unwrap();
        assert_eq!(m.route_for_core(80, 1), Some(9), "cpu1 binding");
        assert_eq!(m.route_for_core(80, 0), None, "cpu0 must stay unbound");
        // FROM_CPU_0 = source 79, CORE0 map at 79*4 = 0x13C — core 0 only.
        m.write(79 * 4, 13).unwrap();
        assert_eq!(m.route_for_core(79, 0), Some(13));
        assert_eq!(m.route_for_core(79, 1), None);
        // Read-back of the CORE1 binding through the APP map window.
        assert_eq!(m.read(off).unwrap(), 9);
    }

    #[test]
    fn intr_status_read_off_byte_unrelated_to_route_bytes() {
        // After writing route binding for source 5 (= offset 20, value 12), reading
        // INTR_STATUS bytes (0x18C..0x19C) must NOT see the route value spill in.
        let mut m = Esp32s3IntMatrix::new();
        m.write(20, 12).unwrap();
        for i in 0x18Cu64..0x19C {
            assert_eq!(
                m.read(i).unwrap(),
                0,
                "INTR_STATUS at offset {:#x} must be 0 before set_pending_sources",
                i
            );
        }
    }
}
