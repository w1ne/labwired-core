// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Classic ATmega watchdog timer: `WDTCSR` (data space 0x60) and the `WDR`
//! instruction. ATmega328P datasheet section 10.9; the ATmega1281/2560 WDT
//! (section 12.4) has the same register and timing.
//!
//! | Bit | Name | Behaviour modelled |
//! |-----|------|--------------------|
//! | 7 | `WDIF` | set on a time-out in interrupt mode; cleared by writing 1 or on vector entry |
//! | 6 | `WDIE` | interrupt enable; cleared on vector entry when `WDE` is also set |
//! | 5, 2:0 | `WDP3..0` | time-out 2K..1024K cycles of the 128 kHz oscillator (16 ms..8 s) |
//! | 4 | `WDCE` | opens the 4-clock change window when written with `WDE` |
//! | 3 | `WDE` | reset enable; forced on while `MCUSR.WDRF` is set |
//!
//! Semantics kept from the datasheet (safety level 1, `WDTON` unprogrammed):
//! - `WDE` may be set at any time, but clearing it or changing `WDP` needs the
//!   timed sequence: write `WDCE` and `WDE` together, then the new value with
//!   `WDCE` clear within four clock cycles;
//! - `WDIE` only: interrupt mode. `WDE` only: system reset mode. Both: the
//!   first time-out raises the interrupt and its vector entry clears `WDIE`,
//!   so the next time-out resets;
//! - a watchdog reset sets `MCUSR.WDRF`, and with `WDRF` set the core comes
//!   back with `WDE` on and the shortest (16 ms) time-out, which is why
//!   avr-libc tells firmware to clear `MCUSR` and call `wdt_disable()` early;
//! - the oscillator is separate from the CPU clock, so the watchdog keeps
//!   counting in every sleep mode and wakes (or resets) a sleeping core;
//! - SRAM is not cleared by the reset, so `.noinit` data survives it.
//!
//! Not modelled: the `WDTON` fuse (safety level 2), oscillator tolerance
//! (the 128 kHz clock is exact here), and the classic tinyAVR `WDTCR`. On the
//! parts without this register `WDR` is a one-cycle no-op.

use super::{Avr, ADDR_MCUSR};
use crate::{Bus, Cpu, SimResult};

/// `WDTCSR` in data space.
pub(super) const ADDR_WDTCSR: u16 = 0x0060;
/// WDT time-out, datasheet 1-based vector number (`__vector_6`, `WDT_vect`).
pub(super) const VEC_WDT: u32 = 7;

pub(super) const WDIF: u8 = 1 << 7;
pub(super) const WDIE: u8 = 1 << 6;
pub(super) const WDP3: u8 = 1 << 5;
pub(super) const WDCE: u8 = 1 << 4;
pub(super) const WDE: u8 = 1 << 3;
const WDP_MASK: u8 = WDP3 | 0x07;
/// `MCUSR.WDRF`: watchdog reset flag.
pub(super) const WDRF: u8 = 1 << 3;

/// The watchdog oscillator.
const WDT_OSC_HZ: u64 = 128_000;
/// Clocks the `WDCE` window stays open after it is set.
const CHANGE_WINDOW: u64 = 4;

#[derive(Debug, Clone)]
pub(super) struct Wdt {
    /// The part has `WDTCSR` at 0x60.
    present: bool,
    /// `WDTCSR` as stored, without `WDCE` (which is derived from `ce_until`).
    csr: u8,
    /// CPU cycle at which the counter last started from zero (`WDR`, enable).
    start: u64,
    /// CPU cycle of the next time-out; `u64::MAX` while stopped.
    deadline: u64,
    /// `WDCE` reads 1 and the protected bits are writable before this cycle.
    ce_until: u64,
    cpu_hz: u64,
}

impl Wdt {
    pub(super) fn new(present: bool) -> Self {
        Self {
            present,
            csr: 0,
            start: 0,
            deadline: u64::MAX,
            ce_until: 0,
            cpu_hz: 16_000_000,
        }
    }

    pub(super) fn handles(&self, addr: u16) -> bool {
        self.present && addr == ADDR_WDTCSR
    }

    pub(super) fn read(&self, now: u64) -> u8 {
        if now < self.ce_until {
            self.csr | WDCE
        } else {
            self.csr
        }
    }

    /// Time-out period in CPU cycles for the current `WDP` (reserved codes
    /// 10..15 are treated as the longest, 1024K).
    fn period(&self) -> u64 {
        let wdp = (self.csr & 0x07) | ((self.csr & WDP3) >> 2);
        let osc_cycles = 2048u64 << u64::from(wdp.min(9));
        osc_cycles * self.cpu_hz / WDT_OSC_HZ
    }

    fn running(&self) -> bool {
        self.csr & (WDE | WDIE) != 0
    }

    /// Recompute the deadline after a change of mode or prescaler. A stopped
    /// watchdog that starts restarts its counter from zero.
    fn rearm(&mut self, was_running: bool, now: u64) {
        if !self.running() {
            self.deadline = u64::MAX;
            return;
        }
        if !was_running {
            self.start = now;
        }
        self.deadline = self.start + self.period();
    }

    /// `WDR`: restart the counter.
    pub(super) fn kick(&mut self, now: u64) {
        if self.present && self.running() {
            self.start = now;
            self.deadline = now + self.period();
        }
    }

    /// Cycles until the next time-out, for the sleep fast-forward.
    pub(super) fn wake_cycles(&self, now: u64) -> Option<u64> {
        (self.deadline != u64::MAX).then(|| self.deadline.saturating_sub(now))
    }

    pub(super) fn due(&self, now: u64) -> bool {
        now >= self.deadline
    }
}

impl Avr {
    /// The core clock the watchdog's 128 kHz period is counted in.
    pub fn set_wdt_cpu_hz(&mut self, hz: u64) {
        if hz != 0 {
            self.wdt.cpu_hz = hz;
            let was = self.wdt.running();
            self.wdt.rearm(was, self.cycles);
        }
    }

    fn wdrf(&self) -> bool {
        self.io[usize::from(ADDR_MCUSR - 0x20)] & WDRF != 0
    }

    /// Store to `WDTCSR`.
    pub(super) fn wdt_write(&mut self, value: u8) {
        let now = self.cycles;
        let was = self.wdt.running();
        let mut csr = self.wdt.csr;
        // WDIF is cleared by writing a one to it.
        if value & WDIF != 0 {
            csr &= !WDIF;
        }
        csr = (csr & !WDIE) | (value & WDIE);
        if now < self.wdt.ce_until {
            // Inside the window: WDE and WDP take the written value, and the
            // window closes.
            csr = (csr & !(WDE | WDP_MASK)) | (value & (WDE | WDP_MASK));
            self.wdt.ce_until = 0;
        } else {
            // Outside it WDE can only be set.
            csr |= value & WDE;
            if value & (WDCE | WDE) == WDCE | WDE {
                self.wdt.ce_until = now + CHANGE_WINDOW;
            }
        }
        if self.wdrf() {
            csr |= WDE;
        }
        self.wdt.csr = csr;
        self.wdt.rearm(was, now);
        self.sync_wdt_irq();
    }

    /// `WDR`.
    pub(super) fn wdt_kick(&mut self) {
        self.wdt.kick(self.cycles);
    }

    fn sync_wdt_irq(&mut self) {
        let bit = 1u64 << VEC_WDT;
        if self.wdt.csr & (WDIF | WDIE) == WDIF | WDIE {
            self.pending_irq |= bit;
        } else {
            self.pending_irq &= !bit;
        }
    }

    /// Hardware clears `WDIF` on vector entry, and `WDIE` too in interrupt
    /// and system reset mode, which leaves the watchdog in reset mode.
    pub(super) fn wdt_vector_entered(&mut self) {
        self.wdt.csr &= !WDIF;
        if self.wdt.csr & WDE != 0 {
            self.wdt.csr &= !WDIE;
        }
        self.sync_wdt_irq();
    }

    /// Check the time-out at an instruction boundary. Returns `true` when the
    /// watchdog reset the core.
    #[inline(always)]
    pub(super) fn wdt_poll(&mut self, bus: &mut dyn Bus) -> SimResult<bool> {
        if !self.wdt.due(self.cycles) {
            return Ok(false);
        }
        self.wdt_timeout(bus)
    }

    #[cold]
    fn wdt_timeout(&mut self, bus: &mut dyn Bus) -> SimResult<bool> {
        let deadline = self.wdt.deadline;
        if self.wdt.csr & WDIE != 0 {
            self.wdt.csr |= WDIF;
            self.sync_wdt_irq();
            // The counter is free running: the next time-out is one period on.
            self.wdt.start = deadline;
            self.wdt.deadline = deadline + self.wdt.period();
            return Ok(false);
        }
        // System reset. The cycle counter is the machine's clock and keeps
        // running across the reset; everything else resets as at power-on,
        // except SRAM, MCUSR's other flags and the watchdog's own WDE.
        let cycles = self.cycles;
        self.io[usize::from(ADDR_MCUSR - 0x20)] |= WDRF;
        Cpu::reset(self, bus)?;
        self.cycles = cycles;
        self.wdt_after_reset();
        Ok(true)
    }

    /// `WDTCSR` after any reset: `WDE` comes back set while `WDRF` is, with
    /// the shortest time-out, and the counter starts again from zero.
    pub(super) fn wdt_after_reset(&mut self) {
        self.wdt.ce_until = 0;
        self.wdt.csr = if self.wdrf() { WDE } else { 0 };
        self.wdt.start = self.cycles;
        self.wdt.deadline = u64::MAX;
        self.wdt.rearm(false, self.cycles);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::avr::AvrModel;

    fn cpu() -> Avr {
        let mut cpu = Avr::new_model(AvrModel::Atmega328p);
        cpu.set_wdt_cpu_hz(16_000_000);
        cpu
    }

    #[test]
    fn prescaler_periods_follow_the_128k_oscillator() {
        let mut cpu = cpu();
        cpu.wdt_write(WDIE);
        // 2K cycles of 128 kHz = 16 ms = 256 000 clocks at 16 MHz.
        assert_eq!(cpu.wdt.wake_cycles(0), Some(256_000));
        // WDP = 9 (WDP3 | WDP0): 1024K cycles = 8.192 s (the datasheet's "8.0 s").
        cpu.wdt_write(WDCE | WDE);
        cpu.wdt_write(WDIE | WDP3 | 1);
        assert_eq!(cpu.wdt.period(), 131_072_000);
    }

    #[test]
    fn wde_clear_and_prescaler_change_need_the_timed_sequence() {
        let mut cpu = cpu();
        cpu.wdt_write(WDE | 6); // WDE sets freely, WDP does not change
        assert_eq!(cpu.wdt.read(cpu.cycles), WDE);
        cpu.wdt_write(0); // clearing WDE without WDCE is ignored
        assert_eq!(cpu.wdt.read(cpu.cycles), WDE);
        cpu.wdt_write(WDCE | WDE);
        assert_eq!(cpu.wdt.read(cpu.cycles), WDCE | WDE);
        cpu.cycles += 2;
        cpu.wdt_write(WDE | 6);
        assert_eq!(cpu.wdt.read(cpu.cycles), WDE | 6);
        // The window has closed: a late write cannot clear WDE.
        cpu.wdt_write(WDCE | WDE);
        cpu.cycles += 5;
        assert_eq!(cpu.wdt.read(cpu.cycles), WDE | 6);
        cpu.wdt_write(0);
        assert_eq!(cpu.wdt.read(cpu.cycles), WDE | 6);
    }

    #[test]
    fn wdrf_forces_wde_on() {
        let mut cpu = cpu();
        cpu.io[usize::from(ADDR_MCUSR - 0x20)] = WDRF;
        cpu.wdt_after_reset();
        assert_eq!(cpu.wdt.read(0), WDE);
        cpu.wdt_write(WDCE | WDE);
        cpu.wdt_write(0);
        assert_eq!(cpu.wdt.read(cpu.cycles), WDE, "WDRF must be cleared first");
        cpu.io[usize::from(ADDR_MCUSR - 0x20)] = 0;
        cpu.wdt_write(WDCE | WDE);
        cpu.wdt_write(0);
        assert_eq!(cpu.wdt.read(cpu.cycles), 0);
        assert_eq!(cpu.wdt.wake_cycles(cpu.cycles), None);
    }

    #[test]
    fn wdr_restarts_the_count() {
        let mut cpu = cpu();
        cpu.wdt_write(WDE);
        cpu.cycles = 200_000;
        cpu.wdt_kick();
        assert_eq!(cpu.wdt.wake_cycles(cpu.cycles), Some(256_000));
    }

    #[test]
    fn interrupt_then_reset_mode() {
        let mut cpu = cpu();
        let mut bus = crate::bus::SystemBus::new();
        cpu.wdt_write(WDE | WDIE);
        cpu.cycles = 256_000;
        assert!(!cpu.wdt_poll(&mut bus).unwrap());
        assert_eq!(cpu.wdt.read(cpu.cycles) & WDIF, WDIF);
        assert_ne!(cpu.pending_irq & (1 << VEC_WDT), 0);
        cpu.wdt_vector_entered();
        assert_eq!(
            cpu.wdt.read(cpu.cycles),
            WDE,
            "vector entry clears WDIE and WDIF"
        );
        assert_eq!(cpu.pending_irq & (1 << VEC_WDT), 0);
        cpu.pc = 0x100;
        cpu.cycles = 512_000;
        assert!(cpu.wdt_poll(&mut bus).unwrap());
        assert_eq!(cpu.pc, 0);
        assert_eq!(cpu.cycles, 512_000, "the clock runs across the reset");
        assert_eq!(cpu.io[usize::from(ADDR_MCUSR - 0x20)] & WDRF, WDRF);
        assert_eq!(cpu.wdt.read(cpu.cycles), WDE);
        assert_eq!(cpu.wdt.wake_cycles(cpu.cycles), Some(256_000));
    }

    /// Records what reaches the bus-side port windows.
    #[derive(Default)]
    struct MirrorBus {
        writes: Vec<(u64, u8)>,
        config: crate::SimulationConfig,
    }

    impl Bus for MirrorBus {
        fn read_u8(&self, _addr: u64) -> SimResult<u8> {
            Ok(0)
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
            self.writes.push((addr, value));
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[crate::DmaRequest]) -> SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &crate::SimulationConfig {
            &self.config
        }
    }

    #[test]
    fn reset_returns_the_ports_to_inputs() {
        use crate::cpu::avr::{AVR_IO_MIRROR_BASE, AVR_PINB, AVR_PIND};
        let mut cpu = cpu();
        let mut bus = MirrorBus::default();
        // PB5 an output driven high, PD2 an input with its pull-up on.
        cpu.data_write(AVR_PINB + 1, 0x20, &mut bus).unwrap();
        cpu.data_write(AVR_PINB + 2, 0x20, &mut bus).unwrap();
        cpu.data_write(AVR_PIND + 2, 0x04, &mut bus).unwrap();
        bus.writes.clear();
        cpu.wdt_write(WDE);
        cpu.cycles = 256_000;
        assert!(cpu.wdt_poll(&mut bus).unwrap());
        let mirror = |addr: u16| AVR_IO_MIRROR_BASE + u64::from(addr);
        // DDRB first: the high output is released, never driven low.
        assert_eq!(
            bus.writes,
            [
                (mirror(AVR_PINB + 1), 0),
                (mirror(AVR_PINB + 2), 0),
                (mirror(AVR_PIND + 2), 0),
            ]
        );
        for addr in [AVR_PINB + 1, AVR_PINB + 2, AVR_PIND + 2] {
            assert_eq!(cpu.data_read(addr, &bus).unwrap(), 0);
        }
    }

    #[test]
    fn stopped_parts_without_wdtcsr_leave_0x60_alone() {
        let cpu = Avr::new_model(AvrModel::Attiny85);
        assert!(!cpu.wdt.handles(ADDR_WDTCSR));
    }
}
