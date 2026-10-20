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
        }
    }

    /// Advance the countdown by `cycles` SoC clocks.
    ///
    /// Closed-form for the periodic case: a full re-arm takes `reload + 1`
    /// cycles (one cycle parked at zero, then `reload` decrements), so a large
    /// idle-fast-forward window does not walk the counter cycle by cycle.
    fn advance(&mut self, cycles: u64) {
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
                if !self.en {
                    self.counter = self.load;
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
