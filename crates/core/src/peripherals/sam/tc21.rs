// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! SAM D21 TC slice (DS40001882 "TC – Timer/Counter"): the 8- and 16-bit
//! counters with the NFRQ/MFRQ periods, up/down counting, ONESHOT, the CTRLB
//! RETRIGGER/STOP commands, STATUS.STOP and the OVF/MC0/MC1 interrupts.
//!
//! The SAM D21 register map is not the SAM D51 one ([`super::tc`]): CTRLA is
//! 16 bits, there is READREQ instead of SYNCBUSY, COUNT sits at 0x10 and
//! CC0 at 0x18, and there are no CCBUF/PERBUF buffers.
//!
//! Clocking: the counter advances from CPU cycles divided by
//! CTRLA.PRESCALER, i.e. it assumes the TC's generic clock is the CPU clock
//! (GCLK generator 0, which is what the Arduino SAMD core and ASF start-up
//! code route to TC3..TC7). Other GCLK routings are NOT modelled — the SAM
//! D21 GCLK in this tree is a declarative register bank. The PM.APBCMASK bit
//! of the instance IS honoured: with the APB clock off the counter stops
//! (register stores are still accepted, which silicon would drop).
//!
//! Not modelled, refused on write: COUNT32 pairing, capture (CTRLC.CPTEN),
//! events (EVCTRL), and output inversion. NPWM/MPWM are accepted and count
//! like NFRQ/MFRQ, but no waveform output is driven onto a pad. Writes
//! synchronise immediately: STATUS.SYNCBUSY reads 0, SYNCRDY never sets, and
//! COUNT always reads current without a READREQ.
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;

const PM_APBCMASK: u64 = 0x4000_0420;
const SWRST: u16 = 1;
const ENABLE: u16 = 1 << 1;
const OVF: u8 = 1;
const MC0: u8 = 1 << 4;
const STATUS_STOP: u8 = 1 << 3;
/// INTENSET/INTFLAG bits this slice can raise: OVF, MC0, MC1. ERR (capture)
/// and SYNCRDY are not, but their enable bits are kept as written.
const INT_MASK: u8 = 0x3b;

#[derive(Clone, Copy, Debug)]
pub struct Tc21Instance {
    pub base: u64,
    pub irq: u32,
    /// PM.APBCMASK bit of this TC.
    pub apbc_bit: u32,
}

/// TC3..TC7 on the SAM D21 (DS40001882 "Product Mapping", "Interrupt Line
/// Mapping" and PM.APBCMASK). E/G parts have TC3..TC5; J parts add TC6/TC7.
pub const TC21_INSTANCES: [Tc21Instance; 5] = [
    Tc21Instance {
        base: 0x4200_2c00,
        irq: 18,
        apbc_bit: 11,
    },
    Tc21Instance {
        base: 0x4200_3000,
        irq: 19,
        apbc_bit: 12,
    },
    Tc21Instance {
        base: 0x4200_3400,
        irq: 20,
        apbc_bit: 13,
    },
    Tc21Instance {
        base: 0x4200_3800,
        irq: 21,
        apbc_bit: 14,
    },
    Tc21Instance {
        base: 0x4200_3c00,
        irq: 22,
        apbc_bit: 15,
    },
];

#[derive(Debug)]
pub struct Samd21Tc {
    instance: Tc21Instance,
    ctrla: u16,
    readreq: u16,
    ctrlb: u8,
    dbgctrl: u8,
    inten: u8,
    intflag: u8,
    /// STATUS.STOP: set at reset, by CMD STOP and by a ONESHOT wrap; cleared
    /// by ENABLE and RETRIGGER.
    stopped: bool,
    count: u16,
    per: u8,
    cc: [u16; 2],
    /// PM.APBCMASK gate: re-read from the bus after each CTRLA store, and on
    /// every bus tick while an enabled counter sees it off.
    apb_clocked: bool,
    gate_dirty: bool,
    /// CPU cycles not yet worth a whole prescaled count.
    carry: u64,
    refusal: Option<String>,
}

impl Samd21Tc {
    pub fn new(instance: Tc21Instance) -> Self {
        Self {
            instance,
            ctrla: 0,
            readreq: 0,
            ctrlb: 0,
            dbgctrl: 0,
            inten: 0,
            intflag: 0,
            stopped: true,
            count: 0,
            per: 0xff,
            cc: [0; 2],
            apb_clocked: false,
            gate_dirty: true,
            carry: 0,
            refusal: None,
        }
    }
    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "samd21 TC slice: {what}"
        )))
    }
    fn mode(&self) -> u16 {
        (self.ctrla >> 2) & 3 // 0 COUNT16, 1 COUNT8, 2 COUNT32
    }
    fn enabled(&self) -> bool {
        self.ctrla & ENABLE != 0
    }
    fn running(&self) -> bool {
        self.enabled() && !self.stopped && self.apb_clocked
    }
    fn width_mask(&self) -> u16 {
        if self.mode() == 1 {
            0xff
        } else {
            0xffff
        }
    }
    fn prescaler(&self) -> u64 {
        [1, 2, 4, 8, 16, 64, 256, 1024][((self.ctrla >> 8) & 7) as usize]
    }
    /// TOP: CC0 for MFRQ/MPWM; PER for NFRQ/NPWM in 8-bit mode; else MAX.
    fn top(&self) -> u16 {
        match (self.ctrla >> 5) & 3 {
            1 | 3 => self.cc[0],
            _ if self.mode() == 1 => u16::from(self.per),
            _ => self.width_mask(),
        }
    }
    fn down(&self) -> bool {
        self.ctrlb & 1 != 0
    }
    pub fn count(&self) -> u16 {
        self.count
    }
    fn line(&self) -> bool {
        self.intflag & self.inten != 0
    }
    fn reset(&mut self) {
        *self = Self::new(self.instance);
    }
    /// Advance by `ticks` counter clocks, raising overflow and compare matches.
    fn advance(&mut self, mut ticks: u64) {
        let down = self.down();
        while ticks > 0 && !self.stopped {
            let top = u64::from(self.top());
            let here = u64::from(self.count);
            // Distance to the next wrap; matches are checked inside that span.
            let span = if down {
                here + 1
            } else {
                top.saturating_sub(here) + 1
            };
            let step = ticks.min(span);
            for (i, cc) in self.cc.iter().enumerate() {
                let cc = u64::from(*cc);
                let hit = if down {
                    cc <= here && here - cc < step
                } else {
                    cc > here && cc - here <= step
                };
                if hit {
                    self.intflag |= MC0 << i;
                }
            }
            if step == span {
                self.intflag |= OVF;
                self.count = if down { top as u16 } else { 0 };
                if self.ctrlb & 4 != 0 {
                    // ONESHOT: stop at the wrap; ENABLE stays set.
                    self.stopped = true;
                    return;
                }
            } else {
                self.count = if down {
                    (here - step) as u16
                } else {
                    (here + step) as u16
                };
            }
            ticks -= step;
        }
    }
    fn read_reg(&self, offset: u64) -> SimResult<u32> {
        let cc8 = self.mode() == 1;
        Ok(match offset {
            0x00 => u32::from(self.ctrla),
            0x02 => u32::from(self.readreq),
            0x04 | 0x05 => u32::from(self.ctrlb),
            0x06 => 0, // CTRLC: no inversion or capture
            0x08 => u32::from(self.dbgctrl),
            0x0a => 0, // EVCTRL
            0x0c | 0x0d => u32::from(self.inten),
            0x0e => u32::from(self.intflag),
            0x0f => {
                if self.stopped {
                    u32::from(STATUS_STOP)
                } else {
                    0
                }
            }
            0x10 => u32::from(self.count),
            0x14 if cc8 => u32::from(self.per),
            0x18 | 0x19 if cc8 => u32::from(self.cc[(offset - 0x18) as usize]),
            0x18 => u32::from(self.cc[0]),
            0x1a => u32::from(self.cc[1]),
            _ => return Self::refuse("register outside the 8/16-bit slice"),
        })
    }
    fn write_ctrla(&mut self, value: u16) -> SimResult<()> {
        if value & SWRST != 0 {
            self.reset();
            return Ok(());
        }
        let protected = !(ENABLE | SWRST);
        if self.enabled() && (value ^ self.ctrla) & protected != 0 {
            return Self::refuse("CTRLA is enable-protected");
        }
        if (value >> 2) & 3 > 1 {
            return Self::refuse("COUNT32 pairing");
        }
        if value & !0x3f6f != 0 {
            return Self::refuse("CTRLA reserved bits");
        }
        let was_enabled = self.enabled();
        self.ctrla = value;
        if !was_enabled && self.enabled() {
            self.stopped = false;
        } else if !self.enabled() {
            self.stopped = true;
        }
        self.gate_dirty = true;
        Ok(())
    }
    fn write_reg(&mut self, offset: u64, value: u32, size: u32) -> SimResult<()> {
        let cc8 = self.mode() == 1;
        match (offset, size) {
            (0x00, 2) | (0x00, 4) if value >> 16 == 0 => self.write_ctrla(value as u16)?,
            (0x00, 1) => self.write_ctrla((self.ctrla & 0xff00) | value as u16)?,
            (0x01, 1) => self.write_ctrla((self.ctrla & 0x00ff) | (value as u16) << 8)?,
            // READREQ: COUNT is always current, so RREQ completes at once.
            (0x02, 2) => self.readreq = (value as u16) & 0x401f,
            (0x04, 1) | (0x05, 1) => {
                let flags = (value as u8) & 0x05; // DIR, ONESHOT
                if offset == 0x05 {
                    self.ctrlb |= flags;
                } else {
                    self.ctrlb &= !flags;
                }
                match (value as u8) >> 6 {
                    0 => {}
                    1 => {
                        // RETRIGGER: restart from the counting direction's origin.
                        if self.enabled() {
                            self.count = if self.down() { self.top() } else { 0 };
                            self.stopped = false;
                        }
                    }
                    2 => self.stopped = true, // STOP
                    _ => return Self::refuse("CTRLB reserved command"),
                }
            }
            (0x06, 1) if value == 0 => {}
            (0x06, 1) => return Self::refuse("CTRLC inversion/capture"),
            (0x08, 1) => self.dbgctrl = (value as u8) & 1,
            (0x0a, 2) if value == 0 => {}
            (0x0a, 2) => return Self::refuse("EVCTRL events"),
            (0x0c, 1) => self.inten &= !(value as u8),
            (0x0d, 1) => self.inten |= (value as u8) & INT_MASK,
            (0x0e, 1) => self.intflag &= !(value as u8),
            (0x10, 1) if cc8 => self.count = value as u16 & 0xff,
            (0x10, 2) if !cc8 => self.count = value as u16,
            (0x14, 1) if cc8 => self.per = value as u8,
            (0x18, 1) | (0x19, 1) if cc8 => self.cc[(offset - 0x18) as usize] = value as u16 & 0xff,
            (0x18, 2) if !cc8 => self.cc[0] = value as u16,
            (0x1a, 2) if !cc8 => self.cc[1] = value as u16,
            _ => return Self::refuse("store outside the 8/16-bit slice"),
        }
        Ok(())
    }
    fn checked(&mut self, result: SimResult<()>) -> SimResult<()> {
        if let Err(e) = &result {
            self.refusal.get_or_insert_with(|| e.to_string());
        }
        result
    }
}

impl Peripheral for Samd21Tc {
    fn on_system_reset(&mut self) {
        self.reset();
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let aligned = match offset {
            0x00..=0x01 => 0x00,
            0x02..=0x03 => 0x02,
            0x0a..=0x0b => 0x0a,
            0x10..=0x11 if self.mode() == 0 => 0x10,
            0x18..=0x19 if self.mode() == 0 => 0x18,
            0x1a..=0x1b if self.mode() == 0 => 0x1a,
            _ => offset,
        };
        Ok((self.read_reg(aligned)? >> (8 * (offset - aligned))) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let r = self.write_reg(offset, u32::from(value), 1);
        self.checked(r)
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        let r = self.write_reg(offset, u32::from(value), 2);
        self.checked(r)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let r = match offset {
            // A word store to a 16-bit register keeps its low half.
            0x02 | 0x0a | 0x10 | 0x18 | 0x1a if value >> 16 == 0 => {
                self.write_reg(offset, value, 2)
            }
            _ => self.write_reg(offset, value, 4),
        };
        self.checked(r)
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn needs_bus_tick(&self) -> bool {
        self.enabled() && (self.gate_dirty || !self.apb_clocked)
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.apb_clocked = bus
            .read_u32(PM_APBCMASK)
            .is_ok_and(|m| m & (1 << self.instance.apbc_bit) != 0);
        self.gate_dirty = false;
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn legacy_tick_active(&self) -> bool {
        self.running() || self.line()
    }
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        report(self.instance.irq, self.line());
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        if self.running() && !self.gate_dirty {
            self.carry += cycles;
            let div = self.prescaler();
            let ticks = self.carry / div;
            self.carry %= div;
            if ticks > 0 {
                self.advance(ticks);
            }
        }
        PeripheralTickResult {
            explicit_irqs: self.line().then(|| vec![self.instance.irq]),
            ..Default::default()
        }
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"peripheral": "samd21_tc", "base": self.instance.base,
            "ctrla": self.ctrla, "ctrlb": self.ctrlb, "count": self.count, "per": self.per,
            "cc": self.cc, "intflag": self.intflag, "inten": self.inten,
            "stopped": self.stopped, "apb_clocked": self.apb_clocked,
            "refusal": self.refusal})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clocked(tc: &mut Samd21Tc) {
        tc.apb_clocked = true;
        tc.gate_dirty = false;
    }

    #[test]
    fn count16_mfrq_compare_overflow_and_irq() {
        let mut tc = Samd21Tc::new(TC21_INSTANCES[1]);
        assert_eq!(tc.read(0x0f).unwrap(), STATUS_STOP, "STOP at reset");
        // COUNT16, MFRQ (TOP = CC0), DIV4.
        tc.write_u16(0x00, 1 << 5 | 2 << 8).unwrap();
        tc.write_u16(0x18, 99).unwrap();
        tc.write_u16(0x1a, 40).unwrap();
        tc.write(0x0d, OVF | MC0 << 1).unwrap();
        tc.write_u16(0x00, 1 << 5 | 2 << 8 | ENABLE).unwrap();
        clocked(&mut tc);
        assert_eq!(tc.read(0x0f).unwrap(), 0);
        tc.tick_elapsed(4 * 39);
        assert_eq!(tc.count(), 39);
        assert!(!tc.line());
        tc.tick_elapsed(4);
        assert_eq!(tc.read(0x0e).unwrap(), MC0 << 1, "MC1 at CC1");
        assert!(tc.line());
        tc.write(0x0e, MC0 << 1).unwrap();
        tc.tick_elapsed(4 * 60); // 40 -> 100 wraps at TOP 99
        assert_eq!(tc.count(), 0);
        assert_eq!(tc.read(0x0e).unwrap() & (OVF | MC0), OVF | MC0);
        assert_eq!(tc.read_u16(0x10).unwrap(), 0);
    }

    #[test]
    fn count8_per_down_oneshot_and_commands() {
        let mut tc = Samd21Tc::new(TC21_INSTANCES[3]);
        tc.write_u16(0x00, 1 << 2).unwrap(); // COUNT8, NFRQ, DIV1
        tc.write(0x14, 9).unwrap(); // PER
        tc.write(0x05, 0x05).unwrap(); // DIR down, ONESHOT
        tc.write(0x10, 9).unwrap();
        tc.write_u16(0x00, 1 << 2 | ENABLE).unwrap();
        clocked(&mut tc);
        tc.tick_elapsed(9);
        assert_eq!(tc.count(), 0);
        tc.tick_elapsed(1); // underflow: OVF, reload PER, one-shot stop
        assert_eq!(tc.count(), 9);
        assert_eq!(tc.read(0x0e).unwrap() & OVF, OVF);
        assert_eq!(tc.read(0x0f).unwrap(), STATUS_STOP);
        tc.tick_elapsed(100);
        assert_eq!(tc.count(), 9, "stopped after the one-shot wrap");
        tc.write(0x05, 1 << 6).unwrap(); // RETRIGGER
        assert_eq!(tc.read(0x0f).unwrap(), 0);
        tc.tick_elapsed(3);
        assert_eq!(tc.count(), 6);
        tc.write(0x05, 2 << 6).unwrap(); // STOP
        tc.tick_elapsed(3);
        assert_eq!(tc.count(), 6);
        assert_eq!(tc.read_u16(0x00).unwrap() & ENABLE, ENABLE);
    }

    #[test]
    fn unclocked_and_refusals() {
        let mut tc = Samd21Tc::new(TC21_INSTANCES[0]);
        tc.write_u16(0x00, ENABLE).unwrap();
        tc.apb_clocked = false;
        tc.gate_dirty = false;
        tc.tick_elapsed(1000);
        assert_eq!(tc.count(), 0, "APBCMASK bit off: no counting");
        assert!(
            tc.write_u16(0x00, ENABLE | 3 << 8).is_err(),
            "enable-protected"
        );
        assert!(tc.write(0x06, 1).is_err(), "inversion");
        assert!(tc.write_u16(0x0a, 1).is_err(), "events");
        tc.write_u16(0x00, 0).unwrap();
        assert!(tc.write_u16(0x00, 2 << 2).is_err(), "COUNT32");
        assert!(tc.snapshot()["refusal"].is_string(), "first refusal kept");
        tc.write_u16(0x00, SWRST).unwrap();
        assert_eq!(tc.read_u16(0x00).unwrap(), 0);
        assert_eq!(tc.read(0x0f).unwrap(), STATUS_STOP);
    }
}
