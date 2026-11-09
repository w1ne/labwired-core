// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Arm Dual-Timer Module (SP804), Arm DDI 0271D, as instantiated by Renode
//! `Timers.ARM_SP804_Timer` (UT32M0R500 timer0/timer1, Versatile/VExpress).
//!
//! Register map (one 0x20-byte bank per counter; Timer2 = Timer1 + 0x20):
//!
//! | Offset | Name | Access | Reset | Meaning |
//! | --- | --- | --- | --- | --- |
//! | 0x00 | `TimerXLoad` | R/W | 0 | reload value; a write also loads the counter now |
//! | 0x04 | `TimerXValue` | R | 0xFFFFFFFF | current count |
//! | 0x08 | `TimerXControl` | R/W | 0x20 | [0] OneShot [1] TimerSize (1 = 32-bit) [3:2] TimerPre [5] IntEnable [6] TimerMode (1 = periodic) [7] TimerEn |
//! | 0x0C | `TimerXIntClr` | W | - | any write clears the raw interrupt |
//! | 0x10 | `TimerXRIS` | R | 0 | [0] raw interrupt |
//! | 0x14 | `TimerXMIS` | R | 0 | [0] RIS & IntEnable |
//! | 0x18 | `TimerXBGLoad` | R/W | 0 | sets `Load` without touching the counter; reads `Load` |
//! | 0xF00 | `TimerITCR` | R/W | 0 | [0] integration test enable |
//! | 0xF04 | `TimerITOP` | W | 0 | [0] TIMINT1 [1] TIMINT2 while ITCR[0] = 1 |
//! | 0xFE0..0xFFC | PeriphID / PCellID | R | 0x04 0x18 0x14 0x00 / 0x0D 0xF0 0x05 0xB1 |
//!
//! Counter semantics (TRM §2.2): the counter decrements once per prescaled
//! `TIMCLK` (1, 16 or 256). Reaching zero sets the raw interrupt. On the next
//! clock a **periodic** counter reloads from `Load`, a **free-running**
//! counter wraps to its maximum (0xFFFF or 0xFFFFFFFF), and a **one-shot**
//! counter halts at zero until `Load` is written or OneShot is cleared. In
//! 16-bit mode only the low 16 bits count.
//!
//! The one IRQ output is `TIMINTC` (TIMINT1 | TIMINT2), the line Renode routes
//! to the NVIC. It is a level: it drops when firmware writes `IntClr`.
//!
//! Deviations from Renode, toward the TRM: Renode tags TimerSize and
//! TimerMode (always 32-bit periodic), clears the interrupt only on a written
//! 1, and does not model the ID words. The prescaler value 0b11 is
//! "undefined" in the TRM; this model divides by 1 there, as QEMU's
//! `arm_timer.c` does. `TIMCLK` is the bus cycle clock: the UT32M0R500 repl
//! clocks the SP804 at 50 MHz, the same as its NVIC SysTick frequency.

use crate::{PeripheralTickResult, SimResult};

const TIMER_STRIDE: u64 = 0x20;
const LOAD: u64 = 0x00;
const VALUE: u64 = 0x04;
const CONTROL: u64 = 0x08;
const INTCLR: u64 = 0x0C;
const RIS: u64 = 0x10;
const MIS: u64 = 0x14;
const BGLOAD: u64 = 0x18;
const ITCR: u64 = 0xF00;
const ITOP: u64 = 0xF04;
const PERIPH_ID: [u8; 8] = [0x04, 0x18, 0x14, 0x00, 0x0D, 0xF0, 0x05, 0xB1];

const CTRL_ONESHOT: u32 = 1 << 0;
const CTRL_32BIT: u32 = 1 << 1;
const CTRL_INTEN: u32 = 1 << 5;
const CTRL_PERIODIC: u32 = 1 << 6;
const CTRL_ENABLE: u32 = 1 << 7;
const CTRL_RESET: u32 = CTRL_INTEN;
const CTRL_MASK: u32 = 0xEF;

/// One SP804 counter.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Sp804Counter {
    load: u32,
    counter: u32,
    control: u32,
    ris: bool,
    /// Bus cycles not yet consumed by the prescaler.
    prescale_acc: u32,
}

impl Sp804Counter {
    fn new() -> Self {
        Self {
            load: 0,
            counter: 0xFFFF_FFFF,
            control: CTRL_RESET,
            ris: false,
            prescale_acc: 0,
        }
    }

    fn mask(&self) -> u32 {
        if self.control & CTRL_32BIT != 0 {
            0xFFFF_FFFF
        } else {
            0xFFFF
        }
    }

    fn divider(&self) -> u32 {
        match (self.control >> 2) & 3 {
            1 => 16,
            2 => 256,
            _ => 1,
        }
    }

    fn mis(&self) -> bool {
        self.ris && self.control & CTRL_INTEN != 0
    }

    fn enabled(&self) -> bool {
        self.control & CTRL_ENABLE != 0
    }

    /// The value the counter takes on the clock after it reached zero.
    fn reload_value(&self) -> u32 {
        if self.control & CTRL_PERIODIC != 0 {
            self.load & self.mask()
        } else {
            self.mask()
        }
    }

    fn write_control(&mut self, value: u32) {
        self.control = value & CTRL_MASK;
        self.counter &= self.mask();
    }

    fn advance(&mut self, cycles: u64) {
        if !self.enabled() || cycles == 0 {
            return;
        }
        let div = u64::from(self.divider());
        let total = u64::from(self.prescale_acc) + cycles;
        let clocks = total / div;
        self.prescale_acc = (total % div) as u32;
        self.count(clocks);
    }

    /// Apply `clocks` prescaled counter clocks in closed form.
    fn count(&mut self, clocks: u64) {
        if clocks == 0 {
            return;
        }
        let oneshot = self.control & CTRL_ONESHOT != 0;
        let mut remaining = clocks;
        if self.counter != 0 {
            let to_zero = u64::from(self.counter);
            if remaining < to_zero {
                self.counter -= remaining as u32;
                return;
            }
            self.counter = 0;
            self.ris = true;
            remaining -= to_zero;
        }
        // At zero: one-shot halts; otherwise each clock after zero walks a
        // cycle of `reload + 1` states (reload, ..., 1, 0).
        if oneshot || remaining == 0 {
            return;
        }
        let reload = u64::from(self.reload_value());
        let period = reload + 1;
        let position = (remaining - 1) % period;
        self.counter = (reload - position) as u32;
        if remaining - 1 >= reload {
            self.ris = true;
        }
    }
}

/// SP804 dual timer: two counters and the combined interrupt line.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArmSp804Timer {
    timers: [Sp804Counter; 2],
    itcr: bool,
    itop: u32,
}

impl ArmSp804Timer {
    pub fn new() -> Self {
        Self {
            timers: [Sp804Counter::new(), Sp804Counter::new()],
            itcr: false,
            itop: 0,
        }
    }

    /// `TIMINTC`: the combined, level interrupt output.
    fn timintc(&self) -> bool {
        if self.itcr {
            self.itop & 3 != 0
        } else {
            self.timers.iter().any(Sp804Counter::mis)
        }
    }

    fn bank(offset: u64) -> Option<(usize, u64)> {
        if offset < 2 * TIMER_STRIDE {
            Some(((offset / TIMER_STRIDE) as usize, offset % TIMER_STRIDE))
        } else {
            None
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        if let Some((idx, reg)) = Self::bank(offset) {
            let t = &self.timers[idx];
            return match reg {
                LOAD | BGLOAD => t.load,
                VALUE => t.counter,
                CONTROL => t.control,
                RIS => u32::from(t.ris),
                MIS => u32::from(t.mis()),
                _ => 0,
            };
        }
        match offset {
            ITCR => u32::from(self.itcr),
            0xFE0..=0xFFC => u32::from(PERIPH_ID[((offset - 0xFE0) / 4) as usize]),
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        if let Some((idx, reg)) = Self::bank(offset) {
            let t = &mut self.timers[idx];
            match reg {
                LOAD => {
                    t.load = value;
                    t.counter = value & t.mask();
                }
                BGLOAD => t.load = value,
                CONTROL => t.write_control(value),
                INTCLR => t.ris = false,
                _ => {}
            }
            return;
        }
        match offset {
            ITCR => self.itcr = value & 1 != 0,
            ITOP => self.itop = value & 3,
            _ => {}
        }
    }

    fn advance(&mut self, cycles: u64) {
        for t in &mut self.timers {
            t.advance(cycles);
        }
    }
}

impl Default for ArmSp804Timer {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for ArmSp804Timer {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(offset & !3) >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) as u32 * 8;
        let merged = (self.read_reg(reg) & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_reg(reg, merged);
        Ok(())
    }

    /// A word store is ONE register write: four byte writes to `Load` would
    /// reload the counter four times with partial values.
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset % 4 == 0 {
            self.write_reg(offset, value);
            return Ok(());
        }
        for (i, byte) in value.to_le_bytes().iter().enumerate() {
            self.write(offset + i as u64, *byte)?;
        }
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        if offset % 2 == 0 {
            let reg = offset & !3;
            let shift = (offset % 4) as u32 * 8;
            let merged = (self.read_reg(reg) & !(0xFFFF << shift)) | (u32::from(value) << shift);
            self.write_reg(reg, merged);
            return Ok(());
        }
        self.write(offset, value as u8)?;
        self.write(offset + 1, (value >> 8) as u8)
    }

    /// The counters are the model; they must see every elapsed cycle.
    fn needs_legacy_walk(&self) -> bool {
        true
    }

    fn legacy_tick_active(&self) -> bool {
        self.timers.iter().any(Sp804Counter::enabled)
    }

    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        self.advance(cycles);
        PeripheralTickResult {
            irq: self.timintc(),
            ..Default::default()
        }
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.timintc())
    }

    fn on_node_reset(&mut self) {
        *self = Self::new();
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "ArmSp804Timer snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize ArmSp804Timer")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let restored: Self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "ArmSp804Timer runtime snapshot decode: {error}"
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

    fn rd(t: &ArmSp804Timer, off: u64) -> u32 {
        t.read_reg(off)
    }

    #[test]
    fn reset_values_and_id_words_match_the_trm() {
        let t = ArmSp804Timer::new();
        for base in [0, TIMER_STRIDE] {
            assert_eq!(rd(&t, base + LOAD), 0);
            assert_eq!(rd(&t, base + VALUE), 0xFFFF_FFFF);
            assert_eq!(rd(&t, base + CONTROL), 0x20);
            assert_eq!(rd(&t, base + RIS), 0);
            assert_eq!(rd(&t, base + MIS), 0);
        }
        let ids: Vec<u32> = (0..8).map(|i| rd(&t, 0xFE0 + 4 * i)).collect();
        assert_eq!(ids, vec![0x04, 0x18, 0x14, 0x00, 0x0D, 0xF0, 0x05, 0xB1]);
        assert!(!t.timintc());
    }

    #[test]
    fn disabled_counter_does_not_move() {
        let mut t = ArmSp804Timer::new();
        t.write_u32(LOAD, 100).unwrap();
        t.tick_elapsed(1000);
        assert_eq!(rd(&t, VALUE), 100);
    }

    #[test]
    fn periodic_counter_reloads_after_load_plus_one_clocks() {
        let mut t = ArmSp804Timer::new();
        t.write_u32(LOAD, 9).unwrap();
        t.write_u32(
            CONTROL,
            CTRL_ENABLE | CTRL_PERIODIC | CTRL_32BIT | CTRL_INTEN,
        )
        .unwrap();
        t.tick_elapsed(8);
        assert_eq!(rd(&t, VALUE), 1);
        assert_eq!(rd(&t, RIS), 0);
        let r = t.tick_elapsed(1);
        assert_eq!(rd(&t, VALUE), 0);
        assert!(r.irq);
        assert_eq!(rd(&t, MIS), 1);
        t.tick_elapsed(1);
        assert_eq!(rd(&t, VALUE), 9);
        // Any write to IntClr drops the line.
        t.write_u32(INTCLR, 0).unwrap();
        assert_eq!(t.irq_line_level(), Some(false));
        // 10 clocks later: zero again.
        t.tick_elapsed(9);
        assert_eq!(rd(&t, VALUE), 0);
        assert_eq!(rd(&t, RIS), 1);
    }

    #[test]
    fn batched_advance_matches_single_clock_steps() {
        for ctrl in [
            CTRL_ENABLE | CTRL_PERIODIC | CTRL_32BIT,
            CTRL_ENABLE | CTRL_32BIT,
            CTRL_ENABLE | CTRL_ONESHOT | CTRL_32BIT,
            CTRL_ENABLE | CTRL_PERIODIC,
            CTRL_ENABLE | CTRL_PERIODIC | CTRL_32BIT | (1 << 2),
        ] {
            for steps in [0u64, 1, 6, 7, 8, 15, 33, 200] {
                let mut a = ArmSp804Timer::new();
                let mut b = ArmSp804Timer::new();
                for t in [&mut a, &mut b] {
                    t.write_u32(LOAD, 6).unwrap();
                    t.write_u32(CONTROL, ctrl).unwrap();
                }
                a.tick_elapsed(steps);
                for _ in 0..steps {
                    b.tick_elapsed(1);
                }
                assert_eq!(rd(&a, VALUE), rd(&b, VALUE), "ctrl {ctrl:#x} steps {steps}");
                assert_eq!(rd(&a, RIS), rd(&b, RIS), "ctrl {ctrl:#x} steps {steps}");
            }
        }
    }

    #[test]
    fn one_shot_halts_at_zero_until_reprogrammed() {
        let mut t = ArmSp804Timer::new();
        t.write_u32(LOAD, 5).unwrap();
        t.write_u32(CONTROL, CTRL_ENABLE | CTRL_ONESHOT | CTRL_32BIT)
            .unwrap();
        t.tick_elapsed(100);
        assert_eq!(rd(&t, VALUE), 0);
        assert_eq!(rd(&t, RIS), 1);
        // IntEnable is clear: raw set, masked clear, line low.
        assert_eq!(rd(&t, MIS), 0);
        assert!(!t.timintc());
        t.write_u32(LOAD, 3).unwrap();
        t.tick_elapsed(2);
        assert_eq!(rd(&t, VALUE), 1);
    }

    #[test]
    fn free_running_wraps_to_the_counter_maximum() {
        let mut t = ArmSp804Timer::new();
        t.write_u32(LOAD, 2).unwrap();
        t.write_u32(CONTROL, CTRL_ENABLE | CTRL_32BIT).unwrap();
        t.tick_elapsed(3);
        assert_eq!(rd(&t, VALUE), 0xFFFF_FFFF);
        // 16-bit size: the counter is the low half and wraps to 0xFFFF.
        let mut s = ArmSp804Timer::new();
        s.write_u32(LOAD, 0x1_0002).unwrap();
        s.write_u32(CONTROL, CTRL_ENABLE).unwrap();
        assert_eq!(rd(&s, VALUE), 2);
        s.tick_elapsed(3);
        assert_eq!(rd(&s, VALUE), 0xFFFF);
        assert_eq!(rd(&s, LOAD), 0x1_0002);
    }

    #[test]
    fn bgload_changes_the_next_period_only() {
        let mut t = ArmSp804Timer::new();
        t.write_u32(LOAD, 10).unwrap();
        t.write_u32(CONTROL, CTRL_ENABLE | CTRL_PERIODIC | CTRL_32BIT)
            .unwrap();
        t.tick_elapsed(4);
        t.write_u32(BGLOAD, 3).unwrap();
        assert_eq!(rd(&t, VALUE), 6);
        assert_eq!(rd(&t, LOAD), 3);
        assert_eq!(rd(&t, BGLOAD), 3);
        t.tick_elapsed(7);
        assert_eq!(rd(&t, VALUE), 3);
    }

    #[test]
    fn prescaler_divides_the_bus_clock() {
        let mut t = ArmSp804Timer::new();
        t.write_u32(LOAD, 100).unwrap();
        t.write_u32(CONTROL, CTRL_ENABLE | CTRL_PERIODIC | CTRL_32BIT | (1 << 2))
            .unwrap();
        t.tick_elapsed(15);
        assert_eq!(rd(&t, VALUE), 100);
        t.tick_elapsed(1);
        assert_eq!(rd(&t, VALUE), 99);
        t.write_u32(CONTROL, CTRL_ENABLE | CTRL_PERIODIC | CTRL_32BIT | (2 << 2))
            .unwrap();
        t.tick_elapsed(512);
        assert_eq!(rd(&t, VALUE), 97);
    }

    #[test]
    fn second_counter_and_integration_test_drive_the_combined_line() {
        let mut t = ArmSp804Timer::new();
        t.write_u32(TIMER_STRIDE + LOAD, 1).unwrap();
        t.write_u32(
            TIMER_STRIDE + CONTROL,
            CTRL_ENABLE | CTRL_32BIT | CTRL_INTEN,
        )
        .unwrap();
        assert!(t.tick_elapsed(1).irq);
        assert_eq!(rd(&t, RIS), 0);
        assert_eq!(rd(&t, TIMER_STRIDE + MIS), 1);
        t.write_u32(TIMER_STRIDE + INTCLR, 0).unwrap();
        assert_eq!(t.irq_line_level(), Some(false));
        t.write_u32(ITCR, 1).unwrap();
        t.write_u32(ITOP, 1).unwrap();
        assert_eq!(t.irq_line_level(), Some(true));
        t.write_u32(ITOP, 0).unwrap();
        assert_eq!(t.irq_line_level(), Some(false));
    }

    #[test]
    fn word_load_write_is_one_register_write() {
        let mut t = ArmSp804Timer::new();
        t.write_u32(LOAD, 0x0001_0000).unwrap();
        t.write_u32(CONTROL, CTRL_ENABLE | CTRL_ONESHOT | CTRL_32BIT)
            .unwrap();
        assert_eq!(rd(&t, VALUE), 0x0001_0000);
        assert_eq!(rd(&t, RIS), 0);
    }

    #[test]
    fn runtime_snapshot_round_trip_keeps_a_running_count() {
        let mut t = ArmSp804Timer::new();
        t.write_u32(LOAD, 1000).unwrap();
        t.write_u32(CONTROL, CTRL_ENABLE | CTRL_PERIODIC | CTRL_32BIT | (1 << 2))
            .unwrap();
        t.tick_elapsed(37);
        let bytes = t.runtime_snapshot();
        let mut r = ArmSp804Timer::new();
        r.restore_runtime_snapshot(&bytes).unwrap();
        t.tick_elapsed(1000);
        r.tick_elapsed(1000);
        assert_eq!(rd(&t, VALUE), rd(&r, VALUE));
    }
}
