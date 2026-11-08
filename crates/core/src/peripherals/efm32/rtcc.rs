// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! EFR32 Series 1 RTCC (EFR32xG12/xG13 reference manual "RTCC – Real Time
//! Counter and Calendar"; register map from Gecko SDK v4.4.0
//! `efr32mg12p_rtcc.h`, RTCC_TypeDef, identical in `efr32mg13p_rtcc.h`) at
//! 0x40042000: the 15-bit pre-counter, the 32-bit main counter in normal
//! mode, three compare channels against CNT, the OF/CC0..CC2/CNTTICK flags
//! behind IEN as one level IRQ, and the 32 retention words.
//!
//! This is the block Renode maps as `Timers.EFR32_RTCC` (frequency 32768) on
//! EFR32MG12 (IRQ 30) and EFR32MG13 (IRQ 31). Like Renode's model it counts
//! the CNT and raises OF/CCx/CNTTICK; unlike it, the pre-counter feeds CNT
//! through the CNTPRESC tap, so PRECNT, CNT and COMBCNT stay consistent.
//!
//! Clocking: PRECNT counts LFECLK, `lfeclk_hz` (default 32768, the LFXO the
//! repl declares), converted from CPU cycles with the attached `cpu_hz`. CNT
//! advances each time the low CTRL.CNTPRESC bits of PRECNT wrap, i.e. at
//! LFECLK / 2^CNTPRESC; each advance raises IF.CNTTICK. The CMU is not
//! modelled, so neither LFECLKSEL nor the LFECLKEN0.RTCC gate is honoured.
//!
//! CNT wraps 0xFFFFFFFF → 0 raising OF. A channel in OUTPUTCOMPARE with
//! COMPBASE = CNT raises CCn when CNT becomes CCV. Writes synchronise at
//! once: SYNCBUSY and STATUS read 0, CMD.CLRSTATUS is accepted. LOCK takes
//! the unlock key (0xAEE8) and reads 0; locking it is refused. A read of IFC
//! returns 0 (MSC_CTRL.IFCREADCLEAR is not modelled).
//!
//! Not modelled, refused on write: calendar mode (CTRL.CNTMODE), the
//! CC0-match pre-counter tick (CTRL.CNTTICK) and its PRECCV0TOP top, the
//! CCV1TOP top, oscillator-failure detection (CTRL.OSCFDETEN), input
//! capture (CC MODE = INPUTCAPTURE), compare against PRECNT (COMPBASE),
//! COMPMASK, DAYCC, compare-match output actions (CMOA) and PRS inputs.
//! TIME/DATE and the CC TIME/DATE words are stored only.
use crate::{Peripheral, PeripheralTickResult, SimResult, SimulationError};
use serde::Serialize;
use std::any::Any;

const CTRL: u64 = 0x00;
const PRECNT: u64 = 0x04;
const CNT: u64 = 0x08;
const COMBCNT: u64 = 0x0C;
const TIME: u64 = 0x10;
const DATE: u64 = 0x14;
const IF: u64 = 0x18;
const IFS: u64 = 0x1C;
const IFC: u64 = 0x20;
const IEN: u64 = 0x24;
const STATUS: u64 = 0x28;
const CMD: u64 = 0x2C;
const SYNCBUSY: u64 = 0x30;
const POWERDOWN: u64 = 0x34;
const LOCK: u64 = 0x38;
const EM4WUEN: u64 = 0x3C;
/// CC0 at 0x40, CC1 at 0x50, CC2 at 0x60: CTRL, CCV, TIME, DATE.
const CC_BASE: u64 = 0x40;
/// RET[0..31] after RESERVED0[37]: 0x104..0x183.
const RET_BASE: u64 = 0x104;

/// The LFECLK the Renode repl declares (`frequency: 32768`).
pub const RTCC_DEFAULT_LFECLK_HZ: u64 = 32_768;
/// Series-1 HFRCO reset band, used until a `cpu_hz` is attached.
const DEFAULT_CPU_HZ: u64 = 19_000_000;

const CTRL_ENABLE: u32 = 1;
const CTRL_PRECCV0TOP: u32 = 1 << 4;
const CTRL_CCV1TOP: u32 = 1 << 5;
const CTRL_CNTTICK: u32 = 1 << 12;
const CTRL_OSCFDETEN: u32 = 1 << 15;
const CTRL_CNTMODE: u32 = 1 << 16;
/// _RTCC_CTRL_MASK.
const CTRL_MASK: u32 = 0x0003_9F35;

const PRECNT_MASK: u32 = 0x7FFF;
const IF_OF: u32 = 1;
const IF_CC0: u32 = 1 << 1;
const IF_CNTTICK: u32 = 1 << 5;
/// _RTCC_IF_MASK: OF, CC0..2, OSCFAIL, CNTTICK and the calendar ticks.
const IF_MASK: u32 = 0x07FF;

/// _RTCC_LOCK_LOCKKEY_UNLOCK.
const LOCK_UNLOCK: u32 = 0xAEE8;

const CC_MODE_INPUTCAPTURE: u32 = 1;
const CC_MODE_OUTPUTCOMPARE: u32 = 2;
/// _RTCC_CC_CTRL_MASK.
const CC_CTRL_MASK: u32 = 0x0003_FBFF;
const CC_CTRL_CMOA: u32 = 0x0C;
const CC_CTRL_PRSSEL: u32 = 0x3C0;
const CC_CTRL_COMPBASE: u32 = 1 << 11;
const CC_CTRL_COMPMASK: u32 = 0x1F000;
const CC_CTRL_DAYCC: u32 = 1 << 17;

#[derive(Debug, Default, Clone, Copy, Serialize)]
struct Cc {
    ctrl: u32,
    ccv: u32,
    time: u32,
    date: u32,
}

impl Cc {
    fn compares(&self) -> bool {
        self.ctrl & 3 == CC_MODE_OUTPUTCOMPARE
    }
}

#[derive(Debug, Serialize)]
pub struct Efm32Rtcc {
    irq: u32,
    lfeclk_hz: u64,
    cpu_hz: u64,
    ctrl: u32,
    precnt: u32,
    cnt: u32,
    time: u32,
    date: u32,
    ien: u32,
    flags: u32,
    powerdown: u32,
    em4wuen: u32,
    cc: [Cc; 3],
    ret: [u32; 32],
    /// `cycles * lfeclk_hz` not yet worth a whole LFECLK edge. Not a register.
    #[serde(skip)]
    carry: u128,
    refusal: Option<String>,
}

impl Efm32Rtcc {
    pub fn new(irq: u32, lfeclk_hz: u64) -> Self {
        Self {
            irq,
            lfeclk_hz: lfeclk_hz.max(1),
            cpu_hz: DEFAULT_CPU_HZ,
            ctrl: 0,
            precnt: 0,
            cnt: 0,
            time: 0,
            date: 0,
            ien: 0,
            flags: 0,
            powerdown: 0,
            em4wuen: 0,
            cc: [Cc::default(); 3],
            ret: [0; 32],
            carry: 0,
            refusal: None,
        }
    }

    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "efr32 RTCC slice: {what}"
        )))
    }

    pub fn counter(&self) -> u32 {
        self.cnt
    }

    fn enabled(&self) -> bool {
        self.ctrl & CTRL_ENABLE != 0
    }

    fn presc(&self) -> u32 {
        (self.ctrl >> 8) & 0xF
    }

    fn line(&self) -> bool {
        self.flags & self.ien != 0
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            CTRL => self.ctrl,
            PRECNT => self.precnt,
            CNT => self.cnt,
            COMBCNT => self.precnt | (self.cnt << 15),
            TIME => self.time,
            DATE => self.date,
            IF => self.flags,
            IEN => self.ien,
            POWERDOWN => self.powerdown,
            EM4WUEN => self.em4wuen,
            0x40..=0x6F => {
                let c = &self.cc[((offset - CC_BASE) / 0x10) as usize];
                match (offset & 0xF) / 4 {
                    0 => c.ctrl,
                    1 => c.ccv,
                    2 => c.time,
                    _ => c.date,
                }
            }
            RET_BASE..=0x183 => self.ret[((offset - RET_BASE) / 4) as usize],
            // IFS, IFC, STATUS, CMD, SYNCBUSY, LOCK and reserved words.
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            CTRL => {
                if value & CTRL_CNTMODE != 0 {
                    return Self::refuse("calendar mode (CTRL.CNTMODE)");
                }
                if value & (CTRL_CNTTICK | CTRL_PRECCV0TOP) != 0 {
                    return Self::refuse("CC0-match pre-counter tick (CTRL.CNTTICK/PRECCV0TOP)");
                }
                if value & CTRL_CCV1TOP != 0 {
                    return Self::refuse("CC1 counter top (CTRL.CCV1TOP)");
                }
                if value & CTRL_OSCFDETEN != 0 {
                    return Self::refuse("oscillator failure detection (CTRL.OSCFDETEN)");
                }
                self.ctrl = value & CTRL_MASK;
            }
            PRECNT => self.precnt = value & PRECNT_MASK,
            CNT => self.cnt = value,
            TIME => self.time = value & 0x003F_7F7F,
            DATE => self.date = value & 0x07FF_1F3F,
            IFS => self.flags |= value & IF_MASK,
            IFC => self.flags &= !value,
            IEN => self.ien = value & IF_MASK,
            POWERDOWN => self.powerdown = value & 1,
            EM4WUEN => self.em4wuen = value & 1,
            LOCK => {
                if value & 0xFFFF != LOCK_UNLOCK {
                    return Self::refuse("LOCK (configuration lock)");
                }
            }
            0x40..=0x6F => {
                let c = &mut self.cc[((offset - CC_BASE) / 0x10) as usize];
                match (offset & 0xF) / 4 {
                    0 => {
                        if value & 3 == CC_MODE_INPUTCAPTURE {
                            return Self::refuse("CCn input capture");
                        }
                        if value & CC_CTRL_COMPBASE != 0 {
                            return Self::refuse("CCn compare against PRECNT (COMPBASE)");
                        }
                        if value & (CC_CTRL_COMPMASK | CC_CTRL_DAYCC) != 0 {
                            return Self::refuse("CCn COMPMASK/DAYCC");
                        }
                        if value & (CC_CTRL_CMOA | CC_CTRL_PRSSEL) != 0 {
                            return Self::refuse("CCn output action (CMOA) / PRS input");
                        }
                        c.ctrl = value & CC_CTRL_MASK;
                    }
                    1 => c.ccv = value,
                    2 => c.time = value & 0x003F_7F7F,
                    _ => c.date = value & 0x0000_1F3F,
                }
            }
            RET_BASE..=0x183 => self.ret[((offset - RET_BASE) / 4) as usize] = value,
            // COMBCNT, IF, STATUS and SYNCBUSY are read-only; CMD.CLRSTATUS
            // has no status to clear.
            _ => {}
        }
        Ok(())
    }

    fn checked(&mut self, result: SimResult<()>) -> SimResult<()> {
        if let Err(e) = &result {
            self.refusal.get_or_insert_with(|| e.to_string());
        }
        result
    }

    /// `lfe` LFECLK edges: move PRECNT, and CNT by every CNTPRESC tap wrap.
    fn advance(&mut self, lfe: u64) {
        let presc = self.presc();
        let p = u64::from(self.precnt);
        let n = p + lfe;
        self.precnt = (n & u64::from(PRECNT_MASK)) as u32;
        let ticks = (n >> presc) - (p >> presc);
        if ticks == 0 {
            return;
        }
        self.flags |= IF_CNTTICK;
        let cnt = u64::from(self.cnt);
        for (i, c) in self.cc.iter().enumerate() {
            if !c.compares() {
                continue;
            }
            // Edges until CNT next becomes CCV (a full turn if it already is).
            let d = match u64::from(c.ccv.wrapping_sub(self.cnt)) {
                0 => 1 << 32,
                d => d,
            };
            if d <= ticks {
                self.flags |= IF_CC0 << i;
            }
        }
        if cnt + ticks > u64::from(u32::MAX) {
            self.flags |= IF_OF;
        }
        self.cnt = (cnt + ticks) as u32;
    }
}

impl Peripheral for Efm32Rtcc {
    fn on_system_reset(&mut self) {
        *self = Self {
            cpu_hz: self.cpu_hz,
            ..Self::new(self.irq, self.lfeclk_hz)
        };
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let aligned = offset & !3;
        Ok((self.read_reg(aligned) >> (8 * (offset - aligned))) as u8)
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        let r = Self::refuse("byte stores (RTCC registers are word-wide)");
        self.checked(r)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let r = self.write_reg(offset & !3, value);
        self.checked(r)
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn attach_cpu_hz(&mut self, hz: u64) {
        if hz > 0 {
            self.cpu_hz = hz;
        }
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn legacy_tick_active(&self) -> bool {
        self.enabled() || self.line()
    }
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        report(self.irq, self.line());
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        if self.enabled() {
            self.carry += u128::from(cycles) * u128::from(self.lfeclk_hz);
            let div = u128::from(self.cpu_hz);
            let lfe = (self.carry / div) as u64;
            self.carry %= div;
            if lfe > 0 {
                self.advance(lfe);
            }
        }
        PeripheralTickResult {
            explicit_irqs: self.line().then(|| vec![self.irq]),
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
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One LFECLK edge per CPU cycle.
    fn rtcc() -> Efm32Rtcc {
        let mut r = Efm32Rtcc::new(30, 1000);
        r.attach_cpu_hz(1000);
        r
    }

    #[test]
    fn disabled_rtcc_holds_and_enable_counts_lfeclk() {
        let mut r = rtcc();
        r.tick_elapsed(100);
        assert_eq!(r.counter(), 0);
        assert_eq!(r.read_u32(IF).unwrap(), 0);
        r.write_u32(CTRL, CTRL_ENABLE).unwrap();
        r.tick_elapsed(5);
        assert_eq!(r.counter(), 5);
        assert_eq!(r.read_u32(PRECNT).unwrap(), 5);
        assert_eq!(r.read_u32(IF).unwrap(), IF_CNTTICK);
    }

    #[test]
    fn cntpresc_taps_the_pre_counter_and_combcnt_joins_them() {
        let mut r = rtcc();
        r.write_u32(CTRL, CTRL_ENABLE | (3 << 8)).unwrap(); // DIV8
        r.tick_elapsed(7);
        assert_eq!(r.counter(), 0);
        assert_eq!(r.read_u32(IF).unwrap(), 0, "no tick before the tap wraps");
        r.tick_elapsed(1);
        assert_eq!(r.counter(), 1);
        r.tick_elapsed(8 * 10 + 3);
        assert_eq!(r.counter(), 11);
        assert_eq!(r.read_u32(PRECNT).unwrap(), 91);
        assert_eq!(r.read_u32(COMBCNT).unwrap(), 91 | (11 << 15));
    }

    #[test]
    fn real_clock_ratio_needs_many_cpu_cycles_per_edge() {
        let mut r = Efm32Rtcc::new(30, 32_768);
        r.attach_cpu_hz(19_000_000);
        r.write_u32(CTRL, CTRL_ENABLE).unwrap();
        r.tick_elapsed(579);
        assert_eq!(r.counter(), 0);
        r.tick_elapsed(1);
        assert_eq!(r.counter(), 1, "19 MHz / 32768 Hz = 579.8 cycles per edge");
    }

    #[test]
    fn compare_flags_when_cnt_becomes_ccv_and_ien_drives_the_line() {
        let mut r = rtcc();
        r.write_u32(CC_BASE + 0x10, CC_MODE_OUTPUTCOMPARE).unwrap(); // CC1
        r.write_u32(CC_BASE + 0x14, 10).unwrap();
        r.write_u32(CTRL, CTRL_ENABLE).unwrap();
        r.tick_elapsed(9);
        assert_eq!(r.read_u32(IF).unwrap() & (IF_CC0 << 1), 0);
        let res = r.tick_elapsed(1);
        assert_eq!(r.read_u32(IF).unwrap() & (IF_CC0 << 1), IF_CC0 << 1);
        assert!(res.explicit_irqs.is_none(), "flag latches without IEN");
        r.write_u32(IEN, IF_CC0 << 1).unwrap();
        assert_eq!(r.tick_elapsed(0).explicit_irqs, Some(vec![30]));
        r.write_u32(IFC, IF_CC0 << 1).unwrap();
        assert!(r.tick_elapsed(0).explicit_irqs.is_none());
        // A channel left OFF never flags.
        assert_eq!(r.read_u32(IF).unwrap() & (IF_CC0 | IF_CC0 << 2), 0);
    }

    #[test]
    fn cnt_wraps_raising_of_and_a_long_tick_still_hits_compares() {
        let mut r = rtcc();
        r.write_u32(CNT, 0xFFFF_FFFE).unwrap();
        r.write_u32(CC_BASE, CC_MODE_OUTPUTCOMPARE).unwrap();
        r.write_u32(CC_BASE + 4, 3).unwrap();
        r.write_u32(CTRL, CTRL_ENABLE).unwrap();
        r.tick_elapsed(1);
        assert_eq!(r.read_u32(IF).unwrap() & IF_OF, 0);
        r.tick_elapsed(10);
        assert_eq!(r.counter(), 9);
        assert_eq!(r.read_u32(IF).unwrap() & (IF_OF | IF_CC0), IF_OF | IF_CC0);
    }

    #[test]
    fn ifs_sets_ifc_clears_and_ifc_reads_zero() {
        let mut r = rtcc();
        r.write_u32(IFS, 0xFFFF_FFFF).unwrap();
        assert_eq!(r.read_u32(IF).unwrap(), IF_MASK);
        assert_eq!(r.read_u32(IFC).unwrap(), 0);
        r.write_u32(IFC, IF_OF).unwrap();
        assert_eq!(r.read_u32(IF).unwrap(), IF_MASK & !IF_OF);
    }

    #[test]
    fn retention_words_and_lock_key() {
        let mut r = rtcc();
        r.write_u32(RET_BASE, 0xDEAD_BEEF).unwrap();
        r.write_u32(RET_BASE + 31 * 4, 0x1234_5678).unwrap();
        assert_eq!(r.read_u32(RET_BASE).unwrap(), 0xDEAD_BEEF);
        assert_eq!(r.read_u32(0x180).unwrap(), 0x1234_5678);
        assert_eq!(r.read_u32(0x100).unwrap(), 0, "RESERVED0 reads 0");
        r.write_u32(LOCK, LOCK_UNLOCK).unwrap();
        assert_eq!(r.read_u32(LOCK).unwrap(), 0);
        assert!(r.write_u32(LOCK, 0).is_err());
        r.on_system_reset();
        assert_eq!(r.read_u32(RET_BASE).unwrap(), 0);
    }

    #[test]
    fn unmodelled_modes_are_refused() {
        let mut r = rtcc();
        assert!(r.write_u32(CTRL, CTRL_ENABLE | CTRL_CNTMODE).is_err());
        assert!(r.write_u32(CTRL, CTRL_CNTTICK).is_err());
        assert!(r.write_u32(CTRL, CTRL_CCV1TOP).is_err());
        assert!(r.write_u32(CC_BASE, CC_MODE_INPUTCAPTURE).is_err());
        assert!(r
            .write_u32(CC_BASE, CC_MODE_OUTPUTCOMPARE | CC_CTRL_COMPBASE)
            .is_err());
        assert!(r.write(0, 1).is_err());
        assert!(r.refusal.is_some());
        assert_eq!(r.read_u32(CTRL).unwrap(), 0);
    }
}
