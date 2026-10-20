// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! LiteX countdown timer (`litex/soc/cores/timer.py`), the block a LiteX SoC
//! maps at its `timer0` CSR page (0x60002800 on the 0x60000000-based map).
//!
//! Register map (32-bit CSRs, 4-byte strides):
//!
//! | Offset | Name | Access | Meaning |
//! | --- | --- | --- | --- |
//! | 0x00 | `load` | R/W | countdown value loaded while the timer is disabled |
//! | 0x04 | `reload` | R/W | value reloaded when the counter reaches zero (0 = one-shot) |
//! | 0x08 | `en` | R/W | bit 0: enable |
//! | 0x0C | `update_value` | W | any write latches the live counter into `value` |
//! | 0x10 | `value` | R | latched countdown value |
//! | 0x14 | `ev_status` | R | bit 0: the zero event is asserted right now (counter == 0) |
//! | 0x18 | `ev_pending` | R/W1C | bit 0: latched zero event; write 1 to clear |
//! | 0x1C | `ev_enable` | R/W | bit 0: zero-event interrupt enable |
//!
//! Semantics, straight from the gateware:
//!
//! * While **disabled** the counter continuously takes `load` (the RTL's
//!   `else` branch), so a firmware that stages a duration writes `load` and
//!   then enables.
//! * While **enabled** the counter counts *down* at the SoC clock. On reaching
//!   zero it reloads from `reload` on the following cycle; `reload == 0` pins
//!   it at zero (one-shot) instead of re-arming.
//! * `ev_status` is the event's *level* (`counter == 0`); `ev_pending` latches
//!   the rising edge and is write-1-to-clear; `ev_enable` gates the IRQ.
//! * `update_value` latches the live counter into `value` so software can read
//!   remaining time without tearing a 32-bit counter read.
//!
//! The model advances from the bus's elapsed-cycle ticks, so a polled one-shot
//! (the LiteX boot path's use) and a periodic timer both progress exactly with
//! simulated time. IRQ delivery goes through the bus's normal
//! `PeripheralTickResult::irq` path; on the RISC-V chips this was first
//! modelled for, external IRQ delivery into the core is not wired yet, so the
//! status/pending registers are the firmware-visible surface (documented in
//! the board's VALIDATION.md).

use crate::{PeripheralTickResult, SimResult};

const LOAD: u64 = 0x00;
const RELOAD: u64 = 0x04;
const EN: u64 = 0x08;
const UPDATE_VALUE: u64 = 0x0C;
const VALUE: u64 = 0x10;
const EV_STATUS: u64 = 0x14;
const EV_PENDING: u64 = 0x18;
const EV_ENABLE: u64 = 0x1C;

/// LiteX countdown timer CSR block.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct LitexTimer {
    load: u32,
    reload: u32,
    en: bool,
    /// The latched `value` CSR (updated by a write to `update_value`).
    value: u32,
    /// Last value written to `update_value` — a CSRStorage bit in the RTL, so
    /// it reads back; the same write is the latch strobe.
    update_value: bool,
    /// The live countdown, in SoC clock cycles.
    counter: u32,
    ev_pending: bool,
    ev_enable: bool,
    /// The zero-event level at the end of the previous observation. The RTL's
    /// trigger is `value == 0` and the event manager latches its rising edge,
    /// independently of `en` — so the level is tracked across disabled ticks
    /// and `load` rewrites too. Starts `false`: at reset the counter is zero,
    /// so the first observation is the rising edge the event manager sees.
    #[serde(default)]
    zero_level: bool,
}

impl LitexTimer {
    pub fn new() -> Self {
        Self {
            load: 0,
            reload: 0,
            en: false,
            value: 0,
            update_value: false,
            counter: 0,
            ev_pending: false,
            ev_enable: false,
            zero_level: false,
        }
    }

    /// Latch the zero event on the rising edge of `counter == 0`.
    ///
    /// The event detector runs off the counter level, not off `en`: a disabled
    /// timer whose `load` is rewritten to zero (or which resets at zero) still
    /// produces the event on silicon.
    fn update_zero_level(&mut self) {
        let zero = self.counter == 0;
        if zero && !self.zero_level {
            self.ev_pending = true;
        }
        self.zero_level = zero;
    }

    /// Advance the countdown by `cycles` SoC clocks.
    ///
    /// Closed-form for the periodic case: a full re-arm takes `reload + 1`
    /// cycles (one cycle parked at zero, then `reload` decrements), so a large
    /// idle-fast-forward window does not walk the counter cycle by cycle.
    fn advance(&mut self, cycles: u64) {
        self.advance_counter(cycles);
        self.update_zero_level();
    }

    fn advance_counter(&mut self, cycles: u64) {
        if !self.en {
            // The RTL's disabled branch continuously reloads from `load`.
            self.counter = self.load;
            return;
        }
        let mut remaining = cycles;
        if remaining == 0 {
            return;
        }
        if self.counter == 0 {
            if self.reload == 0 {
                // One-shot, already spent: pinned at zero, no new edge.
                return;
            }
            self.counter = self.reload;
            remaining -= 1;
        }
        // `counter > 0` here unless reload == 0 was handled above.
        let to_zero = self.counter as u64;
        if remaining < to_zero {
            self.counter -= remaining as u32;
            return;
        }
        // The counter reaches zero inside this window: that is the zero event.
        remaining -= to_zero;
        self.counter = 0;
        self.ev_pending = true;
        if self.reload == 0 {
            return;
        }
        // Periodic: every `reload + 1` cycles returns to the zero state, and
        // each return is a new zero event.
        let period = self.reload as u64 + 1;
        if remaining >= period {
            self.ev_pending = true;
        }
        let rest = remaining % period;
        self.counter = match rest {
            0 => 0,
            1 => self.reload,
            n => self.reload - (n as u32 - 1),
        };
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            LOAD => self.load,
            RELOAD => self.reload,
            EN => self.en as u32,
            // The write-only latch reads as the value last written to it (the
            // RTL keeps `update_value` as a CSRStorage bit).
            UPDATE_VALUE => self.update_value as u32,
            VALUE => self.value,
            EV_STATUS => (self.counter == 0) as u32,
            EV_PENDING => self.ev_pending as u32,
            EV_ENABLE => self.ev_enable as u32,
            _ => {
                crate::census_reg!("litex_timer:LitexTimer", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            LOAD => {
                self.load = value;
                // The RTL's disabled branch continuously loads `value` from
                // `load`; reflect that immediately so a write-then-enable
                // sequence (no intervening tick) starts from the staged value.
                // A rewrite to zero is a zero-event edge even while disabled.
                if !self.en {
                    self.counter = self.load;
                    self.update_zero_level();
                }
            }
            RELOAD => self.reload = value,
            EN => self.en = value & 1 != 0,
            // Any write is the latch strobe; the data is kept in the storage bit.
            UPDATE_VALUE => {
                self.update_value = value & 1 != 0;
                self.value = self.counter;
            }
            VALUE => {}     // read-only
            EV_STATUS => {} // read-only
            // Write 1 to clear the latched event.
            EV_PENDING => {
                if value & 1 != 0 {
                    self.ev_pending = false;
                }
            }
            EV_ENABLE => self.ev_enable = value & 1 != 0,
            _ => {
                crate::census_reg!("litex_timer:LitexTimer", offset, "write");
            }
        }
    }
}

impl Default for LitexTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for LitexTimer {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        // W1C: only the bits this byte writes count. A read-modify-write would
        // write the latched 1 back and clear the event on a write of 0 (and on
        // every byte of a 32-bit store, which arrives here byte by byte).
        if reg == EV_PENDING {
            self.write_reg(reg, (value as u32) << (byte * 8));
            return Ok(());
        }
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }

    /// A word store must land as ONE register write.
    ///
    /// The trait's default decomposes it into four byte writes; for `LOAD`
    /// that would rewrite the counter four times and could latch a false zero
    /// event (writing 256 from 1 passes through 0 in the low byte). The
    /// counters are 32-bit CSRs — the bus's word write is the accurate one.
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset % 4 == 0 {
            self.write_reg(offset, value);
            return Ok(());
        }
        // Unaligned stores keep the byte-lane contract.
        for (i, byte) in value.to_le_bytes().iter().enumerate() {
            self.write(offset + i as u64, *byte)?;
        }
        Ok(())
    }

    /// A half-word store is likewise one write, at either aligned half-word
    /// position: a split byte store could pass through zero and latch a false
    /// event.
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        if offset % 2 == 0 {
            let reg = offset & !3;
            let (mask, shift) = if offset % 4 == 0 {
                (0x0000_FFFFu32, 0u32)
            } else {
                (0xFFFF_0000u32, 16u32)
            };
            let written = u32::from(value) << shift;
            if reg == EV_PENDING {
                // W1C: a read-modify-write would feed the latched bit back and
                // clear the event on a store that never wrote bit 0.
                self.write_reg(reg, written);
                return Ok(());
            }
            let mut v = self.read_reg(reg);
            v = (v & !mask) | written;
            self.write_reg(reg, v);
            return Ok(());
        }
        // Odd offsets keep the byte-lane contract.
        self.write(offset, value as u8)?;
        self.write(offset + 1, (value >> 8) as u8)
    }

    /// A countdown must see every elapsed cycle; the counter is the model.
    fn needs_legacy_walk(&self) -> bool {
        true
    }

    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        self.advance(cycles);
        PeripheralTickResult {
            irq: self.ev_pending && self.ev_enable,
            ..Default::default()
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Restore the JSON snapshot the determinism/replay tools carry.
    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("LitexTimer snapshot decode: {error}"))
        })?;
        *self = restored;
        Ok(())
    }

    /// Binary mid-flight snapshot: the whole countdown state, so a resume
    /// continues the same count instead of restarting from `load`.
    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize LitexTimer")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        // Empty is the trait default from a snapshot taken before this model
        // overrode the hook. Leave the live timer alone.
        if bytes.is_empty() {
            return Ok(());
        }
        let restored: Self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "LitexTimer runtime snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    fn armed(load: u32, reload: u32) -> LitexTimer {
        let mut t = LitexTimer::new();
        t.write_u32(LOAD, load).unwrap();
        t.write_u32(RELOAD, reload).unwrap();
        t.write_u32(EN, 1).unwrap();
        t
    }

    #[test]
    fn reset_values_match_the_gateware() {
        let t = LitexTimer::new();
        assert_eq!(t.read_u32(LOAD).unwrap(), 0);
        assert_eq!(t.read_u32(RELOAD).unwrap(), 0);
        assert_eq!(t.read_u32(EN).unwrap(), 0);
        assert_eq!(t.read_u32(VALUE).unwrap(), 0);
        // The event level is `counter == 0`, which is true at reset.
        assert_eq!(t.read_u32(EV_STATUS).unwrap(), 1);
        assert_eq!(t.read_u32(EV_PENDING).unwrap(), 0);
        assert_eq!(t.read_u32(EV_ENABLE).unwrap(), 0);
    }

    #[test]
    fn disabled_counter_continuously_takes_load() {
        let mut t = LitexTimer::new();
        t.write_u32(LOAD, 500).unwrap();
        let _ = t.tick_elapsed(7);
        t.write_u32(UPDATE_VALUE, 0).unwrap();
        assert_eq!(t.read_u32(VALUE).unwrap(), 500);
        assert_eq!(t.read_u32(EV_STATUS).unwrap(), 0);
    }

    #[test]
    fn one_shot_counts_down_and_latches_the_zero_event() {
        let mut t = armed(1_000, 0);
        let _ = t.tick_elapsed(999);
        t.write_u32(UPDATE_VALUE, 0).unwrap();
        assert_eq!(t.read_u32(VALUE).unwrap(), 1, "one cycle left");
        assert_eq!(t.read_u32(EV_STATUS).unwrap(), 0);

        let r = t.tick_elapsed(1);
        assert_eq!(t.read_u32(EV_STATUS).unwrap(), 1, "counter reached zero");
        assert_eq!(t.read_u32(EV_PENDING).unwrap(), 1, "event latched");
        t.write_u32(UPDATE_VALUE, 0).unwrap();
        assert_eq!(t.read_u32(VALUE).unwrap(), 0);
        assert!(!r.irq, "IRQ is gated by ev_enable");

        // Stays at zero; a further window is not a new edge.
        let _ = t.tick_elapsed(10_000);
        assert_eq!(t.read_u32(VALUE).unwrap(), 0);
        assert_eq!(t.read_u32(EV_PENDING).unwrap(), 1);

        // W1C clears it.
        t.write_u32(EV_PENDING, 1).unwrap();
        assert_eq!(t.read_u32(EV_PENDING).unwrap(), 0);
    }

    #[test]
    fn ev_pending_clears_only_on_a_written_one() {
        let mut t = armed(10, 0);
        let _ = t.tick_elapsed(10);
        assert_eq!(t.read_u32(EV_PENDING).unwrap(), 1);
        t.write_u32(EV_PENDING, 0).unwrap();
        assert_eq!(
            t.read_u32(EV_PENDING).unwrap(),
            1,
            "writing 0 leaves the event"
        );
        t.write(EV_PENDING + 1, 0).unwrap();
        assert_eq!(
            t.read_u32(EV_PENDING).unwrap(),
            1,
            "an upper byte holds no bit 0"
        );
        t.write_u32(EV_PENDING, 1).unwrap();
        assert_eq!(t.read_u32(EV_PENDING).unwrap(), 0);
    }

    #[test]
    fn periodic_timer_rearms_on_reload_plus_one_cycles() {
        let mut t = LitexTimer::new();
        t.write_u32(RELOAD, 3).unwrap();
        t.write_u32(EN, 1).unwrap();
        // From the reset zero state the first cycle re-arms to `reload`.
        let _ = t.tick_elapsed(1);
        t.write_u32(UPDATE_VALUE, 0).unwrap();
        assert_eq!(t.read_u32(VALUE).unwrap(), 3);
        // Three decrements reach zero and latch the event.
        let _ = t.tick_elapsed(3);
        assert_eq!(t.read_u32(EV_STATUS).unwrap(), 1);
        assert_eq!(t.read_u32(EV_PENDING).unwrap(), 1);
        // A full period (1 parked + 3 decrements) lands back on zero.
        let _ = t.tick_elapsed(4);
        assert_eq!(t.read_u32(EV_STATUS).unwrap(), 1);
        // One cycle past the zero state sits at `reload`.
        let _ = t.tick_elapsed(1);
        t.write_u32(UPDATE_VALUE, 0).unwrap();
        assert_eq!(t.read_u32(VALUE).unwrap(), 3);
    }

    /// The zero-event detector follows the counter level, not `en`: a
    /// disabled timer still latches the event when its load is rewritten to
    /// zero, and the reset zero level is the event manager's first rising edge.
    #[test]
    fn zero_event_latches_while_disabled_and_at_reset() {
        let mut t = LitexTimer::new();
        assert_eq!(t.read_u32(EV_PENDING).unwrap(), 0, "no event before a tick");
        // Reset: counter == 0, so the first observation is the rising edge.
        let _ = t.tick_elapsed(1);
        assert_eq!(t.read_u32(EV_PENDING).unwrap(), 1, "reset zero level");
        t.write_u32(EV_PENDING, 1).unwrap();
        let _ = t.tick_elapsed(1);
        assert_eq!(
            t.read_u32(EV_PENDING).unwrap(),
            0,
            "a level held at zero is not a new edge"
        );

        // Disabled, load rewritten nonzero -> zero: a rising edge.
        t.write_u32(LOAD, 5).unwrap();
        assert_eq!(t.read_u32(EV_STATUS).unwrap(), 0);
        assert_eq!(t.read_u32(EV_PENDING).unwrap(), 0);
        t.write_u32(LOAD, 0).unwrap();
        assert_eq!(
            t.read_u32(EV_PENDING).unwrap(),
            1,
            "a disabled load rewrite to zero is a zero event"
        );
    }

    /// A word store to `LOAD` must land atomically: the trait's default
    /// four byte writes would pass through zero in the low byte and latch a
    /// false expiry (1 -> 256).
    #[test]
    fn word_write_to_load_is_atomic() {
        let mut t = LitexTimer::new();
        t.write_u32(LOAD, 1).unwrap();
        t.write_u32(EV_PENDING, 1).unwrap();
        assert_eq!(t.read_u32(EV_PENDING).unwrap(), 0);
        t.write_u32(LOAD, 256).unwrap();
        assert_eq!(t.read_u32(EV_STATUS).unwrap(), 0);
        assert_eq!(
            t.read_u32(EV_PENDING).unwrap(),
            0,
            "atomic LOAD store must not latch a transient zero"
        );
    }

    /// An upper-halfword store to `LOAD` must not transiently zero the low
    /// half (0x0001_0000 -> 0x0100_0000 passes through 0 in a byte split).
    #[test]
    fn upper_halfword_write_to_load_is_atomic() {
        let mut t = LitexTimer::new();
        t.write_u32(LOAD, 0x0001_0000).unwrap();
        t.write_u32(EV_PENDING, 1).unwrap();
        assert_eq!(t.read_u32(EV_PENDING).unwrap(), 0);
        t.write_u16(LOAD + 2, 0x0100).unwrap();
        assert_eq!(t.read_u32(LOAD).unwrap(), 0x0100_0000);
        assert_eq!(
            t.read_u32(EV_PENDING).unwrap(),
            0,
            "upper-halfword store must not latch a transient zero"
        );
    }

    /// W1C must survive halfword stores: writing the upper half (or a zero
    /// low half) must not clear the event; only writing bit 0 as 1 does.
    #[test]
    fn halfword_writes_respect_w1c_on_ev_pending() {
        let mut t = armed(10, 0);
        let _ = t.tick_elapsed(10);
        assert_eq!(t.read_u32(EV_PENDING).unwrap(), 1);

        t.write_u16(EV_PENDING + 2, 0).unwrap();
        assert_eq!(
            t.read_u32(EV_PENDING).unwrap(),
            1,
            "an upper-halfword store must not clear the event"
        );
        t.write_u16(EV_PENDING, 0).unwrap();
        assert_eq!(
            t.read_u32(EV_PENDING).unwrap(),
            1,
            "writing 0 to the low half leaves the event"
        );
        t.write_u16(EV_PENDING, 1).unwrap();
        assert_eq!(
            t.read_u32(EV_PENDING).unwrap(),
            0,
            "writing bit 0 as 1 clears it"
        );
    }

    /// A resume must continue the countdown, not restart it: both snapshot
    /// paths carry the live counter, flags, and a latched event.
    #[test]
    fn snapshot_round_trip_preserves_a_running_countdown() {
        let mut running = armed(1_000, 0);
        let _ = running.tick_elapsed(300);
        let json = running.snapshot();
        let bytes = running.runtime_snapshot();

        let mut from_json = LitexTimer::new();
        from_json.restore(json).unwrap();
        let _ = from_json.tick_elapsed(200);
        from_json.write_u32(UPDATE_VALUE, 0).unwrap();
        assert_eq!(
            from_json.read_u32(VALUE).unwrap(),
            500,
            "the JSON restore resumed at 700 and counted 200 more"
        );
        assert_eq!(from_json.read_u32(EN).unwrap(), 1, "still enabled");

        let mut from_bytes = LitexTimer::new();
        from_bytes.restore_runtime_snapshot(&bytes).unwrap();
        let _ = from_bytes.tick_elapsed(200);
        from_bytes.write_u32(UPDATE_VALUE, 0).unwrap();
        assert_eq!(from_bytes.read_u32(VALUE).unwrap(), 500);

        // A latched event survives the round trip too.
        let _ = running.tick_elapsed(700);
        assert_eq!(running.read_u32(EV_PENDING).unwrap(), 1);
        let mut pending = LitexTimer::new();
        pending.restore(running.snapshot()).unwrap();
        assert_eq!(pending.read_u32(EV_PENDING).unwrap(), 1);
        assert_eq!(pending.read_u32(EV_STATUS).unwrap(), 1);
    }

    #[test]
    fn ev_enable_gates_the_irq_level() {
        let mut t = armed(10, 0);
        t.write_u32(EV_ENABLE, 1).unwrap();
        let r = t.tick_elapsed(10);
        assert!(r.irq, "pending + enable raises the IRQ");
        t.write_u32(EV_PENDING, 1).unwrap();
        let r = t.tick_elapsed(1);
        assert!(!r.irq, "cleared pending drops the IRQ");
    }

    #[test]
    fn update_value_latches_the_live_counter_not_a_stale_one() {
        let mut t = armed(100, 0);
        let _ = t.tick_elapsed(30);
        t.write_u32(UPDATE_VALUE, 0).unwrap();
        assert_eq!(t.read_u32(VALUE).unwrap(), 70);
        let _ = t.tick_elapsed(20);
        t.write_u32(UPDATE_VALUE, 0).unwrap();
        assert_eq!(t.read_u32(VALUE).unwrap(), 50);
    }
}
