// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 64-bit microsecond timer (datasheet §4.6, base `0x40054000`).
//!
//! A single free-running 64-bit counter that, on real silicon, increments once
//! per microsecond off the watchdog tick. This model advances the counter by
//! one on every peripheral tick — the absolute rate is arbitrary (the sim has
//! no wall clock), but the counter is genuinely monotonic, so firmware that
//! samples it twice always sees forward progress.
//!
//! `TIMERAWL` / `TIMERAWH` read the live low / high words. `TIMELR` / `TIMEHR`
//! read the same live counter: the datasheet's read-latch protocol (TIMELR
//! latches the high word for the next TIMEHR read) is a read-time side effect
//! that this trait's `&self` read path cannot carry, so the latch is not
//! modelled — both pairs return the live counter. `PAUSE.bit0` freezes the
//! counter (the debug / firmware pause control). Writing `TIMELW` then `TIMEHW`
//! seeds the counter.
//!
//! ## Alarms (datasheet §4.6.3) — `ALARM0..3`, `ARMED`, `INT{R,E,F,S}`
//!
//! The four alarms are what make this a *tick source*, not just a stopwatch:
//! the Arduino Mbed-OS core (and any pico-sdk `hardware_alarm` user) drives the
//! RTOS `us_ticker` off `TIMER_IRQ_0`. Writing a 32-bit target to `ALARMx`
//! arms alarm `x` (sets `ARMED` bit `x`). When the low 32 bits of the counter
//! reach that target the alarm fires: it latches `INTR` bit `x` and disarms.
//! While `(INTR | INTF) & INTE` holds a bit, the matching `TIMER_IRQ_x`
//! (NVIC IRQ 0..3) is asserted — delivered level-sensitively (re-pended every
//! tick until firmware acknowledges by writing `INTR`), so the ISR always
//! observes the source. Without this the us_ticker never fires, boot stalls in
//! a critical section, and the RTOS aborts on the first mutex acquire.

use crate::{Peripheral, SimResult};

// Register offsets (relative to the TIMER base).
const TIMEHW: u64 = 0x00; // write high word (commits staged low word)
const TIMELW: u64 = 0x04; // write (stage) low word
const TIMEHR: u64 = 0x08; // read high word
const TIMELR: u64 = 0x0c; // read low word
const ALARM0: u64 = 0x10; // alarm 0 target (write arms); ALARM1..3 at +4 each
const ALARM3: u64 = 0x1c;
const ARMED: u64 = 0x20; // read: armed bits; write-1-clear: disarm
const TIMERAWH: u64 = 0x24; // read live high word
const TIMERAWL: u64 = 0x28; // read live low word
const PAUSE: u64 = 0x30; // bit0: freeze counter
const INTR: u64 = 0x34; // raw interrupts, write-1-clear
const INTE: u64 = 0x38; // interrupt enable
const INTF: u64 = 0x3c; // interrupt force
const INTS: u64 = 0x40; // masked status = (INTR | INTF) & INTE (read-only)

#[derive(Debug)]
pub struct Rp2040Timer {
    /// Live 64-bit microsecond counter.
    counter: u64,
    /// `PAUSE.bit0` — when set, `tick` does not advance the counter.
    paused: bool,
    /// Low word staged by a `TIMELW` write (committed by `TIMEHW`).
    pending_low: u32,
    /// 32-bit compare target per alarm (`ALARM0..3`).
    alarm: [u32; 4],
    /// `ARMED` — bit `x` set while alarm `x` is armed and awaiting its match.
    armed: u8,
    /// `INTR` — latched raw interrupt, bit `x` per fired alarm (write-1-clear).
    intr: u8,
    /// `INTE` — per-alarm interrupt enable.
    inte: u8,
    /// `INTF` — per-alarm interrupt force (pico-sdk sets this to fire an alarm
    /// whose target was already in the past).
    intf: u8,
}

impl Default for Rp2040Timer {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2040Timer {
    pub fn new() -> Self {
        Self {
            counter: 0,
            paused: false,
            pending_low: 0,
            alarm: [0; 4],
            armed: 0,
            intr: 0,
            inte: 0,
            intf: 0,
        }
    }

    /// Masked interrupt status: an alarm's IRQ line is asserted while its raw
    /// or forced bit is set and enabled.
    fn ints(&self) -> u8 {
        (self.intr | self.intf) & self.inte
    }
}

impl Peripheral for Rp2040Timer {
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let val = match offset {
            TIMERAWL | TIMELR => self.counter as u32,
            TIMERAWH | TIMEHR => (self.counter >> 32) as u32,
            ALARM0..=ALARM3 => self.alarm[((offset - ALARM0) / 4) as usize],
            ARMED => self.armed as u32,
            PAUSE => self.paused as u32,
            INTR => self.intr as u32,
            INTE => self.inte as u32,
            INTF => self.intf as u32,
            INTS => self.ints() as u32,
            _ => 0,
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            PAUSE => self.paused = value & 0x1 != 0,
            TIMELW => self.pending_low = value,
            // Writing the high word commits the staged low word as the new
            // counter base (datasheet: write TIMELW then TIMEHW).
            TIMEHW => self.counter = ((value as u64) << 32) | self.pending_low as u64,
            // Writing a target arms the corresponding alarm.
            ALARM0..=ALARM3 => {
                let idx = ((offset - ALARM0) / 4) as usize;
                self.alarm[idx] = value;
                self.armed |= 1 << idx;
            }
            // Write-1-to-clear: disarm the selected alarms.
            ARMED => self.armed &= !(value as u8),
            // Write-1-to-clear: acknowledge the raw interrupt.
            INTR => self.intr &= !(value as u8),
            INTE => self.inte = (value & 0xf) as u8,
            INTF => self.intf = (value & 0xf) as u8,
            _ => {}
        }
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !0x3)?;
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !0x3;
        let shift = (offset & 0x3) * 8;
        let cur = self.read_u32(aligned)?;
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, new)
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        if !self.paused {
            self.counter = self.counter.wrapping_add(1);
        }

        // Fire any armed alarm whose 32-bit target the counter has reached.
        // Silicon compares the low word for exact equality; the monotonic
        // +1 counter always hits it within one wrap. A target already in the
        // past never matches here — pico-sdk forces those via INTF instead.
        let low = self.counter as u32;
        for idx in 0..4 {
            if self.armed & (1 << idx) != 0 && self.alarm[idx as usize] == low {
                self.intr |= 1 << idx;
                self.armed &= !(1 << idx);
            }
        }

        // Level-sensitive IRQ delivery: while an alarm's masked status is set,
        // re-pend TIMER_IRQ_x (NVIC IRQ x) every tick until firmware writes
        // INTR to acknowledge. Matches silicon's held IRQ line and guarantees
        // the ISR sees the source even if it runs a tick after the pend.
        let ints = self.ints();
        let explicit_irqs = if ints != 0 {
            Some((0..4).filter(|x| ints & (1 << x) != 0).collect())
        } else {
            None
        };

        crate::PeripheralTickResult {
            explicit_irqs,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_advances_on_tick() {
        let mut t = Rp2040Timer::new();
        let a = t.read_u32(TIMERAWL).unwrap();
        for _ in 0..10 {
            t.tick();
        }
        let b = t.read_u32(TIMERAWL).unwrap();
        assert_eq!(b.wrapping_sub(a), 10, "counter must advance by tick count");
    }

    #[test]
    fn pause_freezes_counter() {
        let mut t = Rp2040Timer::new();
        t.write_u32(PAUSE, 1).unwrap();
        let a = t.read_u32(TIMERAWL).unwrap();
        for _ in 0..10 {
            t.tick();
        }
        assert_eq!(t.read_u32(TIMERAWL).unwrap(), a, "paused counter must hold");
        // Un-pausing resumes advancing.
        t.write_u32(PAUSE, 0).unwrap();
        t.tick();
        assert_eq!(t.read_u32(TIMERAWL).unwrap(), a.wrapping_add(1));
    }

    #[test]
    fn armed_alarm_fires_irq_on_match_then_disarms() {
        let mut t = Rp2040Timer::new();
        t.write_u32(INTE, 1).unwrap(); // enable alarm-0 interrupt
        t.write_u32(ALARM0, 3).unwrap(); // arm alarm 0 for low==3
        assert_eq!(t.read_u32(ARMED).unwrap(), 1, "ALARM0 write arms alarm 0");

        // Ticks 1,2: no match yet, no IRQ.
        for _ in 0..2 {
            assert!(t.tick().explicit_irqs.is_none());
        }
        // Tick 3: counter reaches 3 → fire TIMER_IRQ_0.
        let res = t.tick();
        assert_eq!(res.explicit_irqs, Some(vec![0]), "match raises NVIC IRQ 0");
        assert_eq!(t.read_u32(INTR).unwrap(), 1, "raw interrupt latched");
        assert_eq!(t.read_u32(ARMED).unwrap(), 0, "alarm disarms on fire");
        assert_eq!(t.read_u32(INTS).unwrap(), 1, "masked status set");
    }

    #[test]
    fn irq_is_level_sensitive_until_acknowledged() {
        let mut t = Rp2040Timer::new();
        t.write_u32(INTE, 1).unwrap();
        t.write_u32(ALARM0, 1).unwrap();
        assert_eq!(t.tick().explicit_irqs, Some(vec![0]), "fires at low==1");
        // Held asserted on subsequent ticks while unacknowledged.
        assert_eq!(t.tick().explicit_irqs, Some(vec![0]), "IRQ stays asserted");
        // Firmware acknowledges by write-1-clear to INTR.
        t.write_u32(INTR, 1).unwrap();
        assert!(t.tick().explicit_irqs.is_none(), "IRQ clears after ack");
    }

    #[test]
    fn disabled_or_masked_alarm_raises_no_irq() {
        let mut t = Rp2040Timer::new();
        // Armed but INTE=0: raw fires, but no NVIC IRQ.
        t.write_u32(ALARM0, 1).unwrap();
        let res = t.tick();
        assert!(res.explicit_irqs.is_none(), "masked alarm delivers no IRQ");
        assert_eq!(t.read_u32(INTR).unwrap(), 1, "raw bit still latches");
        assert_eq!(t.read_u32(INTS).unwrap(), 0, "masked status stays clear");
    }

    #[test]
    fn intf_forces_irq_for_past_target() {
        let mut t = Rp2040Timer::new();
        t.write_u32(INTE, 1).unwrap();
        // No armed alarm; firmware forces alarm 0 (pico-sdk past-target path).
        t.write_u32(INTF, 1).unwrap();
        assert_eq!(
            t.tick().explicit_irqs,
            Some(vec![0]),
            "forced IRQ delivered"
        );
        assert_eq!(t.read_u32(INTS).unwrap(), 1);
    }

    #[test]
    fn armed_write_clear_disarms_without_firing() {
        let mut t = Rp2040Timer::new();
        t.write_u32(INTE, 1).unwrap();
        t.write_u32(ALARM0, 5).unwrap();
        t.write_u32(ARMED, 1).unwrap(); // write-1-clear disarms alarm 0
        assert_eq!(t.read_u32(ARMED).unwrap(), 0);
        for _ in 0..8 {
            assert!(
                t.tick().explicit_irqs.is_none(),
                "disarmed alarm never fires"
            );
        }
    }

    #[test]
    fn raw_high_low_track_64bit_counter() {
        let mut t = Rp2040Timer::new();
        // Force the counter just below the 32-bit boundary via the write path.
        t.write_u32(TIMELW, 0xFFFF_FFFE).unwrap();
        t.write_u32(TIMEHW, 0).unwrap();
        t.tick();
        t.tick();
        // 0xFFFF_FFFE + 2 = 0x1_0000_0000.
        assert_eq!(t.read_u32(TIMERAWL).unwrap(), 0);
        assert_eq!(t.read_u32(TIMERAWH).unwrap(), 1);
    }
}
