// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! System Controller (SYSCON) peripheral for ESP32-classic.
//!
//! Per ESP32 TRM v5.0 §13.2 ("System Registers"). The SYSCON block sits
//! at base `0x3FF6_6000`, spans 0x100 bytes, and owns the system-glue
//! registers BROM init reads and ESP-IDF init writes:
//!
//!   * `SYSCON_SYSCLK_CONF` (offset 0x00) — system-clock divider /
//!     `SOC_CLK_SEL` mux. Reset value = 0 (PRE_DIV_CNT=0, SOC_CLK_SEL=0
//!     i.e. XTAL/40 MHz).
//!   * `SYSCON_TICK_CONF` (offset 0x04) — XTAL_TICK_NUM (bits[7:0]),
//!     CK8M_TICK_NUM (bits[15:8]), TICK_ENABLE (bit 16). Reset value
//!     has XTAL_TICK_NUM = 39 (the "40 MHz - 1" divisor that makes the
//!     1 MHz tick rate that the RTC / WDT / RNG sample chain expects).
//!   * `SYSCON_SARADC_CTRL_REG` (offset 0x18) — ADC1 control. Round-
//!     trips for firmware that doesn't touch ADC.
//!   * `SYSCON_FRONT_END_MEM_PD` (offset 0x98) — front-end power-down.
//!   * `SYSCON_RND_DATA` (offset 0xB0) — true RNG output (low 8 bits
//!     valid). Real silicon samples thermal/jitter sources; we run a
//!     deterministic xorshift32 LFSR so replay is bit-exact while still
//!     returning a changing sequence across back-to-back reads.
//!
//! All other offsets inside the 0x100 window read-as-zero unless
//! previously written, and accept any write. This satisfies the
//! "acknowledge the access, don't crash" contract that BROM + ESP-IDF
//! depend on without needing a full bit-level model of every knob.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::collections::HashMap;

// ── Register offsets (per ESP32 TRM v5.0 §13.2 and ESP-IDF
// `soc/esp32/include/soc/syscon_reg.h`) ──────────────────────────────────

/// SYSCON_SYSCLK_CONF_REG — PRE_DIV_CNT (bits[9:0]), SOC_CLK_SEL (bits[11:10]).
pub const SYSCON_SYSCLK_CONF_OFFSET: u64 = 0x00;
/// SYSCON_TICK_CONF_REG — XTAL_TICK_NUM[7:0], CK8M_TICK_NUM[15:8], TICK_ENABLE[16].
pub const SYSCON_TICK_CONF_OFFSET: u64 = 0x04;
/// SYSCON_SARADC_CTRL_REG — ADC1 control register (round-trip stub).
pub const SYSCON_SARADC_CTRL_OFFSET: u64 = 0x18;
/// SYSCON_FRONT_END_MEM_PD_REG — front-end memory power-down.
pub const SYSCON_FRONT_END_MEM_PD_OFFSET: u64 = 0x98;
/// SYSCON_RND_DATA_REG — true random number generator output (low byte valid).
pub const SYSCON_RND_DATA_OFFSET: u64 = 0xB0;

/// Reset value for `SYSCON_SYSCLK_CONF`: PRE_DIV_CNT=0, SOC_CLK_SEL=0 (XTAL).
pub const SYSCLK_CONF_RESET: u32 = 0;
/// Reset value for `SYSCON_TICK_CONF`: XTAL_TICK_NUM=39 (per TRM §13.2.2),
/// CK8M_TICK_NUM=0, TICK_ENABLE=0. The 39 = (40 MHz / 1 MHz) - 1 prescale
/// is what ESP-IDF reads back to validate XTAL frequency at boot.
pub const TICK_CONF_RESET: u32 = 39;

/// Initial xorshift32 seed for the RNG state. Choice of constant is
/// arbitrary but non-zero (xorshift32 fixed-points at 0).
const RNG_SEED: u32 = 0xDEAD_BEEF;

/// SYSCON peripheral.
///
/// Word-granular sparse storage (HashMap) keeps the model compact —
/// most of the 0x100-byte address window is unused on a first-boot
/// probe. RNG side-effect is handled in `read_word`; all other
/// offsets are pure storage.
#[derive(Debug)]
pub struct Syscon {
    /// Base MMIO address (for debugging / logs only — not used in
    /// offset math since the bus already dispatches by offset).
    base: u32,
    /// Backing word store. Indexed by 4-byte-aligned offset.
    regs: HashMap<u32, u32>,
    /// xorshift32 RNG state. Advances on every byte-read of any byte
    /// inside the RND_DATA word so back-to-back reads see a different
    /// value, but the sequence is deterministic for replay.
    rng_state: std::cell::Cell<u32>,
}

impl Default for Syscon {
    fn default() -> Self {
        Self::new()
    }
}

impl Syscon {
    /// Canonical MMIO base address on ESP32-classic.
    pub const BASE: u32 = 0x3FF6_6000;

    /// Construct a freshly-powered SYSCON block.
    ///
    /// Seeds:
    ///   * `SYSCLK_CONF` = 0 (PRE_DIV_CNT=0, SOC_CLK_SEL=0 → XTAL/40 MHz).
    ///   * `TICK_CONF` = 39 (XTAL_TICK_NUM = 40 MHz / 1 MHz - 1).
    ///   * `rng_state` = 0xDEADBEEF.
    pub fn new() -> Self {
        let mut regs = HashMap::new();
        regs.insert(SYSCON_SYSCLK_CONF_OFFSET as u32, SYSCLK_CONF_RESET);
        regs.insert(SYSCON_TICK_CONF_OFFSET as u32, TICK_CONF_RESET);
        Self {
            base: Self::BASE,
            regs,
            rng_state: std::cell::Cell::new(RNG_SEED),
        }
    }

    /// Base MMIO address (informational).
    pub fn base(&self) -> u32 {
        self.base
    }

    /// Advance the xorshift32 RNG state and return the new value.
    /// Marsaglia's classic 13/17/5 triple — full 2^32 - 1 period.
    fn step_rng(&self) -> u32 {
        let mut x = self.rng_state.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state.set(x);
        x
    }

    fn read_word(&self, word_off: u32) -> u32 {
        if u64::from(word_off) == SYSCON_RND_DATA_OFFSET {
            // Each read advances state and returns the low 8 bits in the
            // valid field. Upper bits are documented as "reserved" — we
            // return zero there to make the masking explicit.
            return self.step_rng() & 0xFF;
        }
        self.regs.get(&word_off).copied().unwrap_or(0)
    }

    fn write_word(&mut self, word_off: u32, value: u32) {
        // No side-effects modeled; every offset round-trips as storage.
        // Writes to RND_DATA are accepted but have no effect on the next
        // read (real silicon ignores RND_DATA writes too).
        if u64::from(word_off) == SYSCON_RND_DATA_OFFSET {
            return;
        }
        self.regs.insert(word_off, value);
    }
}

impl Peripheral for Syscon {
    // Inert walk: SYSCON register bank; the RNG word advances on read, never in the walk; tick() is an explicit no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = (offset & !3) as u32;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = (offset & !3) as u32;
        let byte_off = (offset & 3) * 8;
        // Read-modify-write on the existing word (or 0 if unwritten).
        let mut word = self.regs.get(&word_off).copied().unwrap_or(0);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            regs: Vec<(u32, u32)>,
            rng_state: u32,
        }
        let snap = Snap {
            regs: self.regs.iter().map(|(k, v)| (*k, *v)).collect(),
            rng_state: self.rng_state.get(),
        };
        bincode::serialize(&snap).expect("bincode serialize Syscon")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            regs: Vec<(u32, u32)>,
            rng_state: u32,
        }
        let snap: Snap = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("Syscon snapshot decode: {e}"))
        })?;
        self.regs = snap.regs.into_iter().collect();
        self.rng_state.set(snap.rng_state);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u32_at(p: &Syscon, offset: u64) -> u32 {
        let mut v = 0u32;
        for i in 0..4u64 {
            v |= (p.read(offset + i).unwrap() as u32) << (i * 8);
        }
        v
    }

    fn write_u32_at(p: &mut Syscon, offset: u64, value: u32) {
        for i in 0..4u64 {
            p.write(offset + i, ((value >> (i * 8)) & 0xFF) as u8)
                .unwrap();
        }
    }

    #[test]
    fn fresh_syscon_returns_tick_conf_default() {
        let p = Syscon::new();
        assert_eq!(
            read_u32_at(&p, SYSCON_TICK_CONF_OFFSET),
            TICK_CONF_RESET,
            "TICK_CONF must read back XTAL_TICK_NUM=39 at construction (TRM §13.2.2)"
        );
    }

    #[test]
    fn fresh_syscon_returns_sysclk_conf_default() {
        let p = Syscon::new();
        assert_eq!(
            read_u32_at(&p, SYSCON_SYSCLK_CONF_OFFSET),
            SYSCLK_CONF_RESET,
            "SYSCLK_CONF must default to 0 (PRE_DIV_CNT=0, SOC_CLK_SEL=XTAL)"
        );
    }

    #[test]
    fn rnd_data_returns_changing_sequence() {
        // Five back-to-back word reads of RND_DATA must NOT all return
        // the same value. The xorshift32 LFSR guarantees diversity in
        // the low byte across the early steps from the 0xDEADBEEF seed.
        let p = Syscon::new();
        let mut samples = Vec::new();
        for _ in 0..5 {
            samples.push(read_u32_at(&p, SYSCON_RND_DATA_OFFSET));
        }
        let unique: std::collections::HashSet<_> = samples.iter().copied().collect();
        assert!(
            unique.len() > 1,
            "RND_DATA must return a changing sequence, got {samples:?}"
        );
    }

    #[test]
    fn rnd_data_sequence_is_deterministic() {
        // Two fresh Syscon instances must produce the bit-identical RNG
        // stream — required for replay-based testing.
        let p1 = Syscon::new();
        let p2 = Syscon::new();
        for _ in 0..16 {
            assert_eq!(
                read_u32_at(&p1, SYSCON_RND_DATA_OFFSET),
                read_u32_at(&p2, SYSCON_RND_DATA_OFFSET),
            );
        }
    }

    #[test]
    fn rnd_data_returns_only_low_byte() {
        // RND_DATA upper 24 bits are reserved — they must read as zero.
        let p = Syscon::new();
        for _ in 0..8 {
            let v = read_u32_at(&p, SYSCON_RND_DATA_OFFSET);
            assert_eq!(v & 0xFFFF_FF00, 0, "RND_DATA upper bits must be zero");
        }
    }

    #[test]
    fn saradc_ctrl_round_trips() {
        let mut p = Syscon::new();
        write_u32_at(&mut p, SYSCON_SARADC_CTRL_OFFSET, 0xCAFE_F00D);
        assert_eq!(
            read_u32_at(&p, SYSCON_SARADC_CTRL_OFFSET),
            0xCAFE_F00D,
            "SARADC_CTRL must round-trip writes for firmware that ignores ADC"
        );
    }

    #[test]
    fn unhandled_offsets_read_as_zero_before_write() {
        let p = Syscon::new();
        assert_eq!(read_u32_at(&p, SYSCON_FRONT_END_MEM_PD_OFFSET), 0);
        assert_eq!(read_u32_at(&p, 0x40), 0);
        assert_eq!(read_u32_at(&p, 0x80), 0);
    }

    #[test]
    fn unhandled_offsets_round_trip_writes() {
        let mut p = Syscon::new();
        write_u32_at(&mut p, SYSCON_FRONT_END_MEM_PD_OFFSET, 0x1111_2222);
        write_u32_at(&mut p, 0x40, 0x3333_4444);
        assert_eq!(read_u32_at(&p, SYSCON_FRONT_END_MEM_PD_OFFSET), 0x1111_2222);
        assert_eq!(read_u32_at(&p, 0x40), 0x3333_4444);
    }

    #[test]
    fn writes_to_rnd_data_are_ignored() {
        // Real silicon's RND_DATA is read-only; writes don't affect the
        // sample stream. Verify by writing then sampling — the post-write
        // first sample must equal what an untouched instance produces.
        let p_ref = Syscon::new();
        let expected = read_u32_at(&p_ref, SYSCON_RND_DATA_OFFSET);
        let mut p = Syscon::new();
        write_u32_at(&mut p, SYSCON_RND_DATA_OFFSET, 0xFFFF_FFFF);
        assert_eq!(read_u32_at(&p, SYSCON_RND_DATA_OFFSET), expected);
    }

    #[test]
    fn base_is_esp32_classic_canonical_address() {
        let p = Syscon::new();
        assert_eq!(p.base(), 0x3FF6_6000);
    }

    #[test]
    fn runtime_snapshot_round_trip_preserves_state() {
        let mut p = Syscon::new();
        write_u32_at(&mut p, SYSCON_SARADC_CTRL_OFFSET, 0xAAAA_5555);
        // Advance the RNG a few steps so the saved state diverges from
        // the construction seed.
        for _ in 0..4 {
            let _ = read_u32_at(&p, SYSCON_RND_DATA_OFFSET);
        }
        let saved_next = read_u32_at(&p, SYSCON_RND_DATA_OFFSET);
        // Re-snapshot AFTER consuming the sample we'll compare against:
        // snapshot the state JUST BEFORE that read, by reconstructing.
        // Simpler: snapshot here, then verify a fresh restore + N+1 reads
        // produces the matching value at step N+1.
        let snap = p.runtime_snapshot();

        let mut restored = Syscon::new();
        restored.restore_runtime_snapshot(&snap).unwrap();
        assert_eq!(
            read_u32_at(&restored, SYSCON_SARADC_CTRL_OFFSET),
            0xAAAA_5555
        );
        assert_eq!(
            read_u32_at(&restored, SYSCON_TICK_CONF_OFFSET),
            TICK_CONF_RESET
        );
        // RNG stream continues from where we left off: the NEXT sample on
        // both must match.
        let next_after_restore = read_u32_at(&restored, SYSCON_RND_DATA_OFFSET);
        let next_on_original = read_u32_at(&p, SYSCON_RND_DATA_OFFSET);
        assert_eq!(next_after_restore, next_on_original);
        // And both differ from the pre-snapshot sample (proves state moved).
        assert_ne!(next_after_restore, saved_next);
    }
}
