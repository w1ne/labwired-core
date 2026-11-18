// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The 16-bit Timer/Counters of the ATmega640/1280/1281/2560/2561: Timer1
//! with its third compare unit, and Timer3, 4 and 5 (DS2549 section 17).
//!
//! All four are the `timer1.rs` state machine with output compare unit C
//! (`Timer1::with_unit_c`). This file owns their register map, interrupt
//! vectors and input-capture pads on the mega parts; on every other part
//! Timer1 keeps the ATmega328P map in `timer1.rs`.
//!
//! | Timer | `TCCRnA` (+0..+0xD) | `TIFRn` | `TIMSKn` | Vectors (Table 14-1) | ICPn |
//! |-------|------|------|------|-------|------|
//! | 1 | 0x80 | 0x36 | 0x6F | 17..21 | PD4 |
//! | 3 | 0x90 | 0x38 | 0x71 | 32..36 | PE7 |
//! | 4 | 0xA0 | 0x39 | 0x72 | 42..46 | PL0 |
//! | 5 | 0x120 | 0x3A | 0x73 | 47..51 | PL1 |
//!
//! Above `TCCRnA`: `TCCRnB` +1, `TCCRnC` +2, `TCNTn` +4, `ICRn` +6, `OCRnA`
//! +8, `OCRnB` +0xA, `OCRnC` +0xC (17.11). Each timer has its own `TEMP`
//! (17.3). The vectors of a timer are CAPT, COMPA, COMPB, COMPC, OVF in that
//! order; `TIFRn`/`TIMSKn` bit 5 is ICFn/ICIEn, bits 3..1 OCFnC..A, bit 0 TOVn.
//!
//! Every waveform mode of Table 17-2 counts (normal, CTC with OCRnA or ICRn
//! as TOP, fast PWM 8/9/10-bit and with ICRn or OCRnA as TOP, phase correct
//! and phase and frequency correct), with the flags and TOP handling
//! described in `timer1.rs`. The PWM pads (OCnA/B/C: Timer1 PB5/PB6/PB7,
//! Timer3 PE3/PE4/PE5, Timer4 PH3/PH4/PH5, Timer5 PL3/PL4/PL5) follow
//! `oc_pins.rs`, including the output compare modulator on PB7 (section 19).
//!
//! Input capture: an edge of the `TCCRnB.ICESn` polarity on ICPn copies
//! `TCNTn` into `ICRn` and sets ICFn, sampled at instruction boundaries as
//! `PINx` reads the pad, under the same watch rule as the 328P's ICP1 (see
//! `timer1.rs`). The ATmega1281/2561 package bonds no port L, so ICP4/ICP5
//! never see an edge there.
//!
//! Not modelled: external clock sources (`CSn2:0` = 6, 7), the noise
//! canceler delay, the analog-comparator capture source, `FOCnx`,
//! `COMnx` = 1 (toggle) on the pads, the non-PWM compare-output modes and
//! double-buffered `OCRnx`.

use super::timer1::{Timer1, ICF1, OCF1A, OCF1B, OCF1C, TOV1};
use super::{Avr, AVR_IO_MIRROR_BASE};
use crate::Bus;

/// Register map and vectors of one 16-bit timer on the mega parts.
struct Unit {
    tccra: u16,
    tifr: u16,
    timsk: u16,
    /// Pending bits for CAPT, COMPA, COMPB, COMPC, OVF.
    vecs: [u32; 5],
    /// `PINx` address and bit of the ICPn pad.
    icp: (u16, u8),
}

/// Timer1, 3, 4, 5. Timer1 raises this core's 328P-numbered Timer1 bits
/// (remapped to 17..21 by `mega2560_vector`); Timer3/4/5 use their DS2549
/// vector numbers directly.
const UNITS: [Unit; 4] = [
    Unit {
        tccra: 0x80,
        tifr: 0x36,
        timsk: 0x6F,
        vecs: [
            super::timer1::VEC_TIMER1_CAPT,
            super::timer1::VEC_TIMER1_COMPA,
            super::timer1::VEC_TIMER1_COMPB,
            super::timer1::VEC_TIMER1_COMPC,
            super::timer1::VEC_TIMER1_OVF,
        ],
        icp: (0x29, 4),
    },
    Unit {
        tccra: 0x90,
        tifr: 0x38,
        timsk: 0x71,
        vecs: [32, 33, 34, 35, 36],
        icp: (0x2C, 7),
    },
    Unit {
        tccra: 0xA0,
        tifr: 0x39,
        timsk: 0x72,
        vecs: [42, 43, 44, 45, 46],
        icp: (0x109, 0),
    },
    Unit {
        tccra: 0x120,
        tifr: 0x3A,
        timsk: 0x73,
        vecs: [47, 48, 49, 50, 51],
        icp: (0x109, 1),
    },
];

/// The `TIFRn` bit behind each entry of [`Unit::vecs`].
const VEC_FLAGS: [u8; 5] = [ICF1, OCF1A, OCF1B, OCF1C, TOV1];

/// Pending bits of Timer3/4/5 (Timer1's are kept by `timer1.rs`).
const T345_IRQ_MASK: u64 = (0x1F << 32) | (0x3FF << 42);

/// Timer3, 4 and 5, and the input-capture pad watch of all four mega timers.
#[derive(Debug)]
pub struct MegaTimers {
    pub t: [Timer1; 3],
    /// Bit `u`: the timer of `UNITS[u]` has a running prescaler (1..3 only).
    running: u8,
    /// Bit `u`: the ICPn pad of `UNITS[u]` is sampled at every boundary.
    pub(super) icp_watch: u8,
    /// Last sampled ICPn levels, per bit of `icp_watch`.
    icp_last: u8,
}

impl Default for MegaTimers {
    fn default() -> Self {
        Self {
            t: std::array::from_fn(|_| Timer1::with_unit_c()),
            running: 0,
            icp_watch: 0,
            icp_last: 0,
        }
    }
}

impl MegaTimers {
    /// Is any of Timer3/4/5 counting?
    #[inline]
    pub(super) fn any_running(&self) -> bool {
        self.running != 0
    }
}

/// The pending bit with the highest priority on the ATmega2560: the lowest
/// DS2549 vector number after `mega2560_vector`, not the lowest internal bit.
pub(super) fn mega_highest_priority(pending: u64) -> u32 {
    let mut bits = pending & !1;
    let mut best = (u32::MAX, 0);
    while bits != 0 {
        let v = bits.trailing_zeros();
        bits &= bits - 1;
        let slot = super::mega2560_vector(v);
        if slot < best.0 {
            best = (slot, v);
        }
    }
    best.1
}

impl Avr {
    fn timer16(&self, u: usize) -> &Timer1 {
        if u == 0 {
            &self.t1
        } else {
            &self.t16.t[u - 1]
        }
    }

    fn timer16_mut(&mut self, u: usize) -> &mut Timer1 {
        if u == 0 {
            &mut self.t1
        } else {
            &mut self.t16.t[u - 1]
        }
    }

    /// Which timer and register `addr` is: `(unit, register)`, where the
    /// register is `Ok(offset above TCCRnA)`, or `Err(true)` for `TIFRn`
    /// and `Err(false)` for `TIMSKn`.
    fn timer16_decode(addr: u16) -> Option<(usize, Result<u16, bool>)> {
        UNITS.iter().enumerate().find_map(|(u, unit)| {
            if addr == unit.tifr {
                Some((u, Err(true)))
            } else if addr == unit.timsk {
                Some((u, Err(false)))
            } else if (unit.tccra..=unit.tccra + 0xD).contains(&addr) {
                Some((u, Ok(addr - unit.tccra)))
            } else {
                None
            }
        })
    }

    /// Read a 16-bit timer register on a mega part; reserved slots read 0.
    pub(super) fn timer16_read(&self, addr: u16) -> u8 {
        match Self::timer16_decode(addr) {
            Some((u, Err(true))) => self.timer16(u).tifr,
            Some((u, Err(false))) => self.timer16(u).timsk,
            Some((u, Ok(off))) => self.timer16(u).reg_read(off).unwrap_or(0),
            None => 0,
        }
    }

    /// Write a 16-bit timer register on a mega part.
    pub(super) fn timer16_write(&mut self, addr: u16, value: u8, bus: &dyn Bus) {
        let Some((u, reg)) = Self::timer16_decode(addr) else {
            return;
        };
        let t = self.timer16_mut(u);
        match reg {
            Err(true) => t.write_tifr(value),
            Err(false) => t.write_timsk(value),
            Ok(off) => {
                t.reg_write(off, value);
            }
        }
        if u == 0 {
            self.timer1_changed();
        } else {
            let on = self.t16.t[u - 1].prescaler() != 0;
            self.t16.running = (self.t16.running & !(1 << u)) | (u8::from(on) << u);
            self.sync_timer345_irq();
        }
        self.icp_config_changed(bus);
    }

    fn sync_timer345_irq(&mut self) {
        let mut want = 0u64;
        for (t, unit) in self.t16.t.iter().zip(&UNITS[1..]) {
            let on = t.requesting();
            for (flag, vec) in VEC_FLAGS.iter().zip(unit.vecs) {
                if on & flag != 0 {
                    want |= 1 << vec;
                }
            }
        }
        self.pending_irq = (self.pending_irq & !T345_IRQ_MASK) | want;
    }

    /// Advance Timer3/4/5 by CPU clocks. Called beside Timer0/1/2.
    pub(super) fn tick_timer345(&mut self, cpu_cycles: u32) {
        let mut moved = false;
        for t in &mut self.t16.t {
            moved |= t.tick(cpu_cycles);
        }
        if moved {
            self.sync_timer345_irq();
        }
    }

    /// CPU cycles until a Timer3/4/5 interrupt can wake a sleeping core.
    pub(super) fn timer345_wake_cycles(&self) -> Option<u64> {
        if !self.flag_i() || !self.io_clock_running() {
            return None;
        }
        self.t16.t.iter().filter_map(Timer1::wake_cycles).min()
    }

    /// Entering a Timer3/4/5 vector clears its flag.
    pub(super) fn timer345_vector_entered(&mut self, vec: u32) {
        let hit = self
            .t16
            .t
            .iter_mut()
            .zip(&UNITS[1..])
            .find_map(|(t, unit)| {
                let i = unit.vecs.iter().position(|&v| v == vec)?;
                t.tifr &= !VEC_FLAGS[i];
                Some(())
            });
        if hit.is_some() {
            self.sync_timer345_irq();
        }
    }

    /// The ICPn pad levels of the watched timers, as `PINx` reads them.
    fn icp_levels(&self, bus: &dyn Bus) -> u8 {
        let mut levels = 0u8;
        for (u, unit) in UNITS.iter().enumerate() {
            if self.t16.icp_watch & (1 << u) == 0 {
                continue;
            }
            let (pin, bit) = unit.icp;
            let ddr = self.io[usize::from(pin + 1 - 0x20)];
            let port = self.port_pad_value(pin + 2, self.io[usize::from(pin + 2 - 0x20)]);
            let external = bus
                .read_u8(AVR_IO_MIRROR_BASE + u64::from(pin))
                .unwrap_or(0);
            if ((port & ddr) | (external & !ddr)) & (1 << bit) != 0 {
                levels |= 1 << u;
            }
        }
        levels
    }

    /// Re-arm the ICPn watch after a timer register write. A pad that starts
    /// being watched takes its present level, so arming invents no edge.
    fn icp_config_changed(&mut self, bus: &dyn Bus) {
        let watch = (0..4).fold(0u8, |w, u| {
            w | (u8::from(self.timer16(u).capture_watched()) << u)
        });
        let fresh = watch & !self.t16.icp_watch;
        self.t16.icp_watch = watch;
        let now = self.icp_levels(bus);
        self.t16.icp_last = (self.t16.icp_last & !fresh) | (now & fresh);
    }

    /// Has a watched ICPn pad moved since the last sample?
    pub(super) fn mega_icp_moved(&self, bus: &dyn Bus) -> bool {
        self.t16.icp_watch != 0 && self.icp_levels(bus) != self.t16.icp_last & self.t16.icp_watch
    }

    /// Sample the watched ICPn pads at an instruction boundary.
    pub(super) fn sample_mega_icp(&mut self, bus: &dyn Bus) {
        let now = self.icp_levels(bus);
        let changed = (now ^ self.t16.icp_last) & self.t16.icp_watch;
        self.t16.icp_last = now;
        if changed == 0 || !self.io_clock_running() {
            return;
        }
        if changed & 1 != 0 {
            self.timer1_icp_edge(now & 1 != 0);
        }
        for (k, t) in self.t16.t.iter_mut().enumerate() {
            let bit = 1 << (k + 1);
            if changed & bit != 0 {
                t.capture_edge(now & bit != 0);
            }
        }
        self.sync_timer345_irq();
    }
}

#[cfg(test)]
mod tests {
    //! Black-box checks against DS2549 (ATmega640/1280/1281/2560/2561,
    //! 2549Q-AVR-02/2014): registers written through the data bus, interrupt
    //! handlers placed at the Table 14-1 program addresses, pads read where the
    //! bus-side port model receives them.

    use super::super::{Avr, AvrModel, AVR_IO_MIRROR_BASE};
    use crate::{Bus, Cpu, DmaRequest, SimResult, SimulationConfig};
    use std::collections::HashMap;

    struct PadBus {
        mem: HashMap<u64, u8>,
        config: SimulationConfig,
    }

    impl Bus for PadBus {
        fn read_u8(&self, addr: u64) -> SimResult<u8> {
            Ok(*self.mem.get(&addr).unwrap_or(&0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
            self.mem.insert(addr, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
    }

    // Table 14-1 vector numbers (1-based; handler at byte (n - 1) * 4) and
    // the register each handler increments.
    const HANDLERS: [(u32, usize); 18] = [
        (18, 2),  // TIMER1 COMPA
        (20, 3),  // TIMER1 COMPC
        (21, 4),  // TIMER1 OVF
        (32, 5),  // TIMER3 CAPT
        (33, 6),  // TIMER3 COMPA
        (34, 7),  // TIMER3 COMPB
        (35, 8),  // TIMER3 COMPC
        (36, 9),  // TIMER3 OVF
        (42, 10), // TIMER4 CAPT
        (43, 11), // TIMER4 COMPA
        (44, 12), // TIMER4 COMPB
        (45, 13), // TIMER4 COMPC
        (46, 14), // TIMER4 OVF
        (47, 15), // TIMER5 CAPT
        (48, 16), // TIMER5 COMPA
        (49, 17), // TIMER5 COMPB
        (50, 18), // TIMER5 COMPC
        (51, 19), // TIMER5 OVF
    ];
    /// Every other vector slot counts into r31: a vector taken at the wrong
    /// program address shows up there.
    const STRAY: usize = 31;
    const MAIN: u32 = 0x100;

    const PORTB: u16 = 0x25;
    const PORTE: u16 = 0x2E;
    const PORTH: u16 = 0x102;
    const PORTL: u16 = 0x10B;

    fn inc(rd: usize) -> u16 {
        0x9403 | ((rd as u16) << 4)
    }

    /// An ATmega2560 spinning in `RJMP .` with every vector slot counting.
    fn rig() -> (Avr, PadBus) {
        let mut cpu = Avr::new_model(AvrModel::Atmega2560);
        for n in 2..=57u32 {
            let rd = HANDLERS
                .iter()
                .find(|(v, _)| *v == n)
                .map_or(STRAY, |(_, r)| *r);
            cpu.load_words((n - 1) * 4, &[inc(rd), 0x9518]);
        }
        cpu.load_words(MAIN, &[0xCFFF]);
        cpu.set_pc(MAIN);
        let bus = PadBus {
            mem: HashMap::new(),
            config: SimulationConfig::default(),
        };
        (cpu, bus)
    }

    fn w(cpu: &mut Avr, bus: &mut PadBus, addr: u16, value: u8) {
        cpu.data_write(addr, value, bus).unwrap();
    }

    fn r(cpu: &Avr, bus: &PadBus, addr: u16) -> u8 {
        cpu.data_read(addr, bus).unwrap()
    }

    /// 16-bit write: high byte (through TEMP) first, then low (17.3).
    fn w16(cpu: &mut Avr, bus: &mut PadBus, addr_low: u16, value: u16) {
        w(cpu, bus, addr_low + 1, (value >> 8) as u8);
        w(cpu, bus, addr_low, value as u8);
    }

    /// 16-bit read: low byte first, then high (17.3).
    fn r16(cpu: &Avr, bus: &PadBus, addr_low: u16) -> u16 {
        let lo = r(cpu, bus, addr_low);
        let hi = r(cpu, bus, addr_low + 1);
        u16::from(hi) << 8 | u16::from(lo)
    }

    fn run_to(cpu: &mut Avr, bus: &mut PadBus, cycles: u64) {
        let cfg = SimulationConfig::default();
        while cpu.cycles < cycles {
            cpu.step(bus, &[], &cfg).unwrap();
        }
    }

    fn sei(cpu: &mut Avr) {
        cpu.sreg |= 0x80;
    }

    /// High-time fraction of each `(PORTx, bit)` pad over `[now, until)`.
    fn duty(cpu: &mut Avr, bus: &mut PadBus, pads: &[(u16, u8)], until: u64) -> Vec<f64> {
        let cfg = SimulationConfig::default();
        let level = |bus: &PadBus, (port, bit): (u16, u8)| {
            bus.mem
                .get(&(AVR_IO_MIRROR_BASE + u64::from(port)))
                .is_some_and(|v| v & (1 << bit) != 0)
        };
        let start = cpu.cycles;
        let mut high = vec![0u64; pads.len()];
        while cpu.cycles < until {
            let now: Vec<bool> = pads.iter().map(|p| level(bus, *p)).collect();
            let t = cpu.cycles;
            cpu.step(bus, &[], &cfg).unwrap();
            for (h, on) in high.iter_mut().zip(now) {
                if on {
                    *h += cpu.cycles - t;
                }
            }
        }
        let span = (cpu.cycles - start) as f64;
        high.iter().map(|h| *h as f64 / span).collect()
    }

    /// 17.9.2 CTC with TOP = OCR1A (WGM 4) at clk/1: a match every
    /// OCR1A + 1 clocks. TIMSK1 (17.11.33) OCIE1A is bit 1, OCIE1C bit 3;
    /// Table 14-1: TIMER1 COMPA is vector 18, COMPC vector 20. OCR1C is at
    /// 0x8C/0x8D (17.11.19).
    #[test]
    fn timer1_ctc_runs_compare_a_and_compare_c_at_their_vectors() {
        let (mut cpu, mut bus) = rig();
        w16(&mut cpu, &mut bus, 0x88, 999);
        w16(&mut cpu, &mut bus, 0x8C, 500);
        assert_eq!(r16(&cpu, &bus, 0x8C), 500, "OCR1C reads back");
        w(&mut cpu, &mut bus, 0x6F, 0b0000_1010);
        assert_eq!(r(&cpu, &bus, 0x6F), 0b0000_1010, "OCIE1C sticks");
        w(&mut cpu, &mut bus, 0x80, 0x00);
        w(&mut cpu, &mut bus, 0x81, 0x08 | 0x01);
        sei(&mut cpu);
        // Matches at 999, 1999, ... (A) and 500, 1500, ... (C).
        run_to(&mut cpu, &mut bus, 10_030);
        assert_eq!((cpu.r[2], cpu.r[3]), (10, 10), "COMPA, COMPC entries");
        assert_eq!((cpu.r[4], cpu.r[STRAY]), (0, 0), "no OVF, no stray vector");
    }

    /// 17.9.1 Normal mode: TOV3 (TIFR3 0x38 bit 0, 17.11.38) is set when
    /// TCNT3 (0x94/0x95) passes MAX; it clears by writing 1 and on entry to
    /// TIMER3 OVF (vector 36). TIMSK3 (0x71) keeps bits 5, 3..0 only.
    #[test]
    fn timer3_normal_mode_overflows_into_vector_36() {
        let (mut cpu, mut bus) = rig();
        w(&mut cpu, &mut bus, 0x71, 0xFF);
        assert_eq!(r(&cpu, &bus, 0x71), 0x2F, "TIMSK3 writable bits");
        w(&mut cpu, &mut bus, 0x71, 0x01);
        w16(&mut cpu, &mut bus, 0x94, 0xFFF0);
        w(&mut cpu, &mut bus, 0x91, 0x01); // clk/1
        run_to(&mut cpu, &mut bus, 20);
        assert_eq!(r16(&cpu, &bus, 0x94), 4, "0xFFF0 + 20 wraps to 4");
        // OCR3A/B/C reset to 0, so the wrap through 0 is also their compare
        // match: OCF3A..C rise with TOV3 (17.11.38).
        assert_eq!(
            r(&cpu, &bus, 0x38),
            0x0F,
            "TOV3, OCF3A..C set, interrupts off"
        );
        w(&mut cpu, &mut bus, 0x38, 0x01);
        assert_eq!(r(&cpu, &bus, 0x38), 0x0E, "write 1 clears TOV3 only");
        w16(&mut cpu, &mut bus, 0x94, 0xFFFE);
        sei(&mut cpu);
        run_to(&mut cpu, &mut bus, 40);
        assert_eq!(cpu.r[9], 1, "TIMER3 OVF entered once");
        assert_eq!(r(&cpu, &bus, 0x38) & 0x01, 0, "entry cleared TOV3");
        assert_eq!(cpu.r[STRAY], 0);
    }

    /// 17.9.2 CTC with TOP = ICR4 (WGM 12): the counter clears after ICR4,
    /// OCF4A/B/C rise at OCR4A/B/C, and with ICR4 as TOP, ICF4 is set when
    /// the counter reaches TOP (17.11.39). TOV4 is set only at MAX
    /// (Table 17-2), never reached here. Vectors 42..46 (Table 14-1).
    #[test]
    fn timer4_ctc_with_icr4_top_raises_capture_and_all_three_compares() {
        let (mut cpu, mut bus) = rig();
        w16(&mut cpu, &mut bus, 0xA6, 199);
        w16(&mut cpu, &mut bus, 0xA8, 50);
        w16(&mut cpu, &mut bus, 0xAA, 100);
        w16(&mut cpu, &mut bus, 0xAC, 150);
        w(&mut cpu, &mut bus, 0x72, 0x2F);
        w(&mut cpu, &mut bus, 0xA0, 0x00);
        w(&mut cpu, &mut bus, 0xA1, 0x18 | 0x01);
        sei(&mut cpu);
        run_to(&mut cpu, &mut bus, 2_030);
        assert_eq!(
            [cpu.r[10], cpu.r[11], cpu.r[12], cpu.r[13], cpu.r[14]],
            [10, 10, 10, 10, 0],
            "CAPT, COMPA, COMPB, COMPC, OVF"
        );
        assert!(r16(&cpu, &bus, 0xA4) <= 199, "TCNT4 stays at or below ICR4");
        assert_eq!(cpu.r[STRAY], 0);
    }

    /// Timer5 in CTC at clk/64 with OCR5A = 249: 16 MHz / 64 / 250 = 1 kHz,
    /// a compare match every 16 000 CPU clocks (18.1 prescaler, 17.9.2).
    /// TIMER5 COMPA is vector 48; TCCR5A/B at 0x120/0x121, OCR5A at 0x128.
    #[test]
    fn timer5_ctc_at_clk_64_runs_compare_a_every_16000_clocks() {
        let (mut cpu, mut bus) = rig();
        w16(&mut cpu, &mut bus, 0x128, 249);
        w(&mut cpu, &mut bus, 0x73, 0x02);
        w(&mut cpu, &mut bus, 0x120, 0x00);
        w(&mut cpu, &mut bus, 0x121, 0x08 | 0x03);
        sei(&mut cpu);
        run_to(&mut cpu, &mut bus, 10 * 16_000 + 100);
        assert_eq!(cpu.r[16], 10);
        assert_eq!(cpu.r[STRAY], 0);
    }

    /// Table 17-2: fast PWM 8-bit (WGM 5) has TOP 0xFF and sets TOV at TOP,
    /// once per 256 clocks; phase-correct 8-bit (WGM 1) counts up and down
    /// and sets TOV at BOTTOM, once per 2 * 255 = 510 clocks.
    #[test]
    fn fast_and_phase_correct_pwm_overflow_rates() {
        let (mut cpu, mut bus) = rig();
        w(&mut cpu, &mut bus, 0x71, 0x01);
        w(&mut cpu, &mut bus, 0x73, 0x01);
        w(&mut cpu, &mut bus, 0x90, 0x01); // WGM30
        w(&mut cpu, &mut bus, 0x91, 0x08 | 0x01); // WGM32, clk/1
        w(&mut cpu, &mut bus, 0x120, 0x01); // WGM50
        w(&mut cpu, &mut bus, 0x121, 0x01); // clk/1
        sei(&mut cpu);
        // Timer3's 20th TOV is at 5120, Timer5's 10th at 5100.
        run_to(&mut cpu, &mut bus, 5_115);
        assert_eq!(cpu.r[9], 19, "TIMER3 OVF: 5115 / 256");
        assert_eq!(cpu.r[19], 10, "TIMER5 OVF: 5115 / 510");
        assert_eq!(cpu.r[STRAY], 0);
    }

    /// 17.3: each 16-bit timer has its own TEMP byte ("The same Temporary
    /// Register is shared between all 16-bit registers within each 16-bit
    /// timer"); writing the low byte commits TEMP as the high byte; reading
    /// the low byte of TCNTn latches the high byte; "Reading the
    /// OCRnA/B/C 16-bit registers does not involve using the Temporary
    /// Register".
    #[test]
    fn each_timer_has_its_own_temp_and_ocr_reads_bypass_it() {
        let (mut cpu, mut bus) = rig();
        w(&mut cpu, &mut bus, 0x99, 0x12); // OCR3AH -> TEMP3
        w(&mut cpu, &mut bus, 0xA9, 0x34); // OCR4AH -> TEMP4
        w(&mut cpu, &mut bus, 0x98, 0x56); // OCR3AL commits
        w(&mut cpu, &mut bus, 0xA8, 0x78); // OCR4AL commits
        assert_eq!(r16(&cpu, &bus, 0x98), 0x1256);
        assert_eq!(r16(&cpu, &bus, 0xA8), 0x3478);
        w16(&mut cpu, &mut bus, 0x128, 0x0700); // OCR5A
        w16(&mut cpu, &mut bus, 0x124, 0xBEEF); // TCNT5, timer stopped
        assert_eq!(r(&cpu, &bus, 0x124), 0xEF, "TCNT5L latches 0xBE");
        assert_eq!(r(&cpu, &bus, 0x129), 0x07, "OCR5AH reads OCR5A, not TEMP");
        assert_eq!(r(&cpu, &bus, 0x125), 0xBE, "TCNT5H reads TEMP");
        w16(&mut cpu, &mut bus, 0x88, 0x5A00); // OCR1A
        w16(&mut cpu, &mut bus, 0x84, 0xC3C3); // TCNT1
        assert_eq!(r(&cpu, &bus, 0x84), 0xC3, "TCNT1L latches 0xC3");
        assert_eq!(r(&cpu, &bus, 0x89), 0x5A, "OCR1AH reads OCR1A, not TEMP");
        assert_eq!(r16(&cpu, &bus, 0x8C), 0, "OCR1C untouched by the others");
    }

    /// Table 17-4 / 17.9.3 fast PWM non-inverting: set at BOTTOM, cleared on
    /// the match, duty (OCR + 1) / (TOP + 1); inverting is the complement.
    /// Table 17-5 / 17.9.4 phase correct: duty OCR / TOP. Pads (13.3.2,
    /// 13.3.5, 13.3.8, Port L): OC1C = PB7, OC3A = PE3, OC4C = PH5,
    /// OC5B = PL4.
    #[test]
    fn pwm_reaches_the_oc_pads_of_timers_1_3_4_5() {
        let (mut cpu, mut bus) = rig();
        // Timer1 WGM 15 (fast PWM, TOP = OCR1A = 199), OC1C 150 / 200.
        w16(&mut cpu, &mut bus, 0x88, 199);
        w16(&mut cpu, &mut bus, 0x8C, 149);
        w(&mut cpu, &mut bus, 0x80, 0b0000_1011); // COM1C = 2, WGM11:10
        w(&mut cpu, &mut bus, 0x81, 0x18 | 0x01);
        // Timer3 WGM 14 (fast PWM, TOP = ICR3 = 99), OC3A 25 / 100.
        w16(&mut cpu, &mut bus, 0x96, 99);
        w16(&mut cpu, &mut bus, 0x98, 24);
        w(&mut cpu, &mut bus, 0x90, 0b1000_0010); // COM3A = 2, WGM31
        w(&mut cpu, &mut bus, 0x91, 0x18 | 0x01);
        // Timer4 WGM 5 (fast PWM 8-bit), OC4C inverting: 1 - 64 / 256.
        w16(&mut cpu, &mut bus, 0xAC, 63);
        w(&mut cpu, &mut bus, 0xA0, 0b0000_1101); // COM4C = 3, WGM40
        w(&mut cpu, &mut bus, 0xA1, 0x08 | 0x01);
        // Timer5 WGM 1 (phase correct 8-bit), OC5B 51 / 255.
        w16(&mut cpu, &mut bus, 0x12A, 51);
        w(&mut cpu, &mut bus, 0x120, 0b0010_0001); // COM5B = 2, WGM50
        w(&mut cpu, &mut bus, 0x121, 0x01);
        // The pads are outputs (DDRB7, DDRE3, DDRH5, DDRL4): 13.3, "The pin
        // has to be configured as an output ... to serve this function".
        w(&mut cpu, &mut bus, PORTB - 1, 0x80);
        w(&mut cpu, &mut bus, PORTE - 1, 0x08);
        w(&mut cpu, &mut bus, PORTH - 1, 0x20);
        w(&mut cpu, &mut bus, PORTL - 1, 0x10);
        let pads = [(PORTB, 7), (PORTE, 3), (PORTH, 5), (PORTL, 4)];
        let got = duty(&mut cpu, &mut bus, &pads, 51_000);
        let want = [0.75, 0.25, 0.75, 0.2];
        for ((g, w), name) in got.iter().zip(want).zip(["OC1C", "OC3A", "OC4C", "OC5B"]) {
            assert!((g - w).abs() < 0.01, "{name} duty {g}, want {w}");
        }
    }

    /// Section 19, Output Compare Modulator: with OC0A and OC1C both
    /// enabled, PB7 carries the two combined; PORTB7 selects the logic. The
    /// timing example (19.2.1) quantizes the *high* time of OC1C to the
    /// carrier at PORTB7 = 0, so 0 is AND; 1 is OR.
    #[test]
    fn oc0a_and_oc1c_are_modulated_on_pb7_by_portb7() {
        // Exact AND / OR high-time over one common period (lcm 256, 1000):
        // OC0A fast PWM, OCR0A = 127 (high for TCNT0 <= 127); OC1C fast PWM
        // TOP 999, OCR1C = 499 (high for TCNT1 <= 499).
        let span = 32_000u64;
        let both = (0..span)
            .filter(|t| t % 256 <= 127 && t % 1000 <= 499)
            .count();
        let either = (0..span)
            .filter(|t| t % 256 <= 127 || t % 1000 <= 499)
            .count();
        for (portb7, want) in [(0u8, both), (0x80, either)] {
            let (mut cpu, mut bus) = rig();
            w(&mut cpu, &mut bus, PORTB - 1, 0x80); // DDRB7
            w(&mut cpu, &mut bus, PORTB, portb7);
            w(&mut cpu, &mut bus, 0x47, 127); // OCR0A
            w16(&mut cpu, &mut bus, 0x88, 999);
            w16(&mut cpu, &mut bus, 0x8C, 499);
            w(&mut cpu, &mut bus, 0x44, 0x83); // COM0A = 2, fast PWM
            w(&mut cpu, &mut bus, 0x80, 0b0000_1011);
            w(&mut cpu, &mut bus, 0x45, 0x01);
            w(&mut cpu, &mut bus, 0x81, 0x18 | 0x01);
            let got = duty(&mut cpu, &mut bus, &[(PORTB, 7)], 4 * span)[0];
            let want = want as f64 / span as f64;
            assert!(
                (got - want).abs() < 0.01,
                "PORTB7 = {portb7:#x}: PB7 duty {got}, want {want}"
            );
        }
    }

    /// Drive the external level of a `PINx` pad (input) on the bus side.
    fn drive(bus: &mut PadBus, pin_addr: u16, bit: u8, high: bool) {
        let key = AVR_IO_MIRROR_BASE + u64::from(pin_addr);
        let v = bus.mem.get(&key).copied().unwrap_or(0);
        let v = if high {
            v | (1 << bit)
        } else {
            v & !(1 << bit)
        };
        bus.mem.insert(key, v);
    }

    /// 17.6 Input Capture Unit: an edge of the `ICESn` polarity on ICPn
    /// copies TCNTn into ICRn and sets ICFn; ICIEn routes it to the CAPT
    /// vector (Table 14-1: TIMER5 CAPT 47, TIMER3 CAPT 32). Pads (13.3):
    /// ICP5 = PL1, ICP3 = PE7. ICES5 = 1 (TCCR5B bit 6): rising edges only;
    /// ICES3 = 0: falling edges only.
    #[test]
    fn icp_pads_capture_tcnt_into_icr_and_enter_the_capture_vector() {
        let (mut cpu, mut bus) = rig();
        w(&mut cpu, &mut bus, 0x73, 0x20); // ICIE5
        w(&mut cpu, &mut bus, 0x71, 0x20); // ICIE3
        w(&mut cpu, &mut bus, 0x121, 0x40 | 0x01); // ICES5, clk/1, normal
        w(&mut cpu, &mut bus, 0x91, 0x01); // ICES3 = 0, clk/1, normal
        drive(&mut bus, 0x2C, 7, true); // PE7 idles high
        sei(&mut cpu);
        run_to(&mut cpu, &mut bus, 1_000);
        // Both counters started at cycle 0 at clk/1, so TCNTn = cycles.
        let at = cpu.cycles;
        drive(&mut bus, 0x109, 1, true); // PL1 rises
        run_to(&mut cpu, &mut bus, at + 20);
        assert_eq!(cpu.r[15], 1, "TIMER5 CAPT entered");
        assert_eq!(
            r16(&cpu, &bus, 0x126),
            at as u16,
            "ICR5 = TCNT5 at the edge"
        );
        assert_eq!(r(&cpu, &bus, 0x3A) & 0x20, 0, "entry cleared ICF5");
        assert_eq!(cpu.r[5], 0, "PE7 has not moved");
        let fall5 = cpu.cycles;
        drive(&mut bus, 0x109, 1, false); // PL1 falls: ignored with ICES5
        run_to(&mut cpu, &mut bus, fall5 + 20);
        assert_eq!(cpu.r[15], 1, "falling edge ignored");
        let at3 = cpu.cycles;
        drive(&mut bus, 0x2C, 7, false); // PE7 falls
        run_to(&mut cpu, &mut bus, at3 + 20);
        assert_eq!(cpu.r[5], 1, "TIMER3 CAPT entered");
        assert_eq!(
            r16(&cpu, &bus, 0x96),
            at3 as u16,
            "ICR3 = TCNT3 at the edge"
        );
        assert_eq!(cpu.r[STRAY], 0);
    }

    /// ICP1 is PD4 on the 2560 (13.3.4, "ICP1 – Port D, Bit 4"), not PB0 as
    /// on the ATmega328P. ICF1 is TIFR1 bit 5 (17.11.37), polled here with
    /// ICIE1 clear: the flag rises either way.
    #[test]
    fn timer1_input_capture_listens_on_pd4() {
        let (mut cpu, mut bus) = rig();
        w(&mut cpu, &mut bus, 0x81, 0x40 | 0x01); // ICES1, clk/1
        run_to(&mut cpu, &mut bus, 200);
        drive(&mut bus, 0x23, 0, true); // PB0 rises: not ICP1 here
        run_to(&mut cpu, &mut bus, 220);
        assert_eq!(r(&cpu, &bus, 0x36) & 0x20, 0, "PB0 is not ICP1 on the 2560");
        let at = cpu.cycles;
        drive(&mut bus, 0x29, 4, true); // PD4 rises
        run_to(&mut cpu, &mut bus, at + 4);
        assert_eq!(r(&cpu, &bus, 0x36) & 0x20, 0x20, "ICF1 set by PD4");
        assert_eq!(r16(&cpu, &bus, 0x86), at as u16, "ICR1 = TCNT1 at the edge");
        assert_eq!(cpu.r[STRAY], 0, "ICIE1 clear: no vector");
    }

    /// 7.8 / Table 14-1: "the lower the address the higher is the priority
    /// level". With TIMER5 COMPA (48), TIMER3 COMPA (33) and TIMER1 COMPC
    /// (20) all pending when I is set, COMPC is entered first, then 33,
    /// then 48. Flags are raised by matches while interrupts are off.
    #[test]
    fn pending_timer_vectors_are_taken_in_vector_number_order() {
        let (mut cpu, mut bus) = rig();
        w16(&mut cpu, &mut bus, 0x128, 10); // OCR5A
        w16(&mut cpu, &mut bus, 0x98, 10); // OCR3A
        w16(&mut cpu, &mut bus, 0x8C, 10); // OCR1C
        w(&mut cpu, &mut bus, 0x73, 0x02); // OCIE5A
        w(&mut cpu, &mut bus, 0x71, 0x02); // OCIE3A
        w(&mut cpu, &mut bus, 0x6F, 0x08); // OCIE1C
        for tccrb in [0x121, 0x91, 0x81] {
            w(&mut cpu, &mut bus, tccrb, 0x01);
        }
        run_to(&mut cpu, &mut bus, 20);
        assert_eq!(r(&cpu, &bus, 0x36), 0x08, "OCF1C");
        assert_eq!(r(&cpu, &bus, 0x38), 0x02, "OCF3A");
        assert_eq!(r(&cpu, &bus, 0x3A), 0x02, "OCF5A");
        sei(&mut cpu);
        let cfg = SimulationConfig::default();
        let mut order = Vec::new();
        while order.len() < 3 && cpu.cycles < 200 {
            cpu.step(&mut bus, &[], &cfg).unwrap();
            let slot = cpu.pc / 4 + 1;
            if [20, 33, 48].contains(&slot) && cpu.pc % 4 == 0 && order.last() != Some(&slot) {
                order.push(slot);
            }
        }
        assert_eq!(order, [20, 33, 48]);
    }

    /// Table 17-2: phase correct (WGM 10) and phase and frequency correct
    /// (WGM 8) with ICRn as TOP count 0..TOP..0, TOVn at BOTTOM; 17.11.39:
    /// "When the ICRn is set to be used as the TOP value, the ICFn Flag is
    /// set when the counter reaches the TOP value", on the clock that takes
    /// the counter from TOP to TOP - 1 (Figure 17-12). Flags polled with
    /// interrupts off; TIFR bits are set whether or not TIMSK enables them.
    #[test]
    fn dual_slope_with_icr_top_sets_icf_at_top_and_tov_at_bottom() {
        let (mut cpu, mut bus) = rig();
        w16(&mut cpu, &mut bus, 0x96, 100); // ICR3
        w16(&mut cpu, &mut bus, 0xA6, 100); // ICR4
        w(&mut cpu, &mut bus, 0x90, 0x02); // WGM31
        w(&mut cpu, &mut bus, 0x91, 0x10 | 0x01); // WGM33 -> 10, clk/1
        w(&mut cpu, &mut bus, 0xA0, 0x00);
        w(&mut cpu, &mut bus, 0xA1, 0x10 | 0x01); // WGM43 -> 8, clk/1
        run_to(&mut cpu, &mut bus, 100);
        for (tifr, tcnt) in [(0x38, 0x94), (0x39, 0xA4)] {
            assert_eq!(r16(&cpu, &bus, tcnt), 100, "at TOP");
            assert_eq!(r(&cpu, &bus, tifr), 0, "ICF rises on the clock after TOP");
        }
        run_to(&mut cpu, &mut bus, 101);
        // RJMP is 2 clocks: 102 clocks, two past TOP on the way down.
        assert_eq!(cpu.cycles, 102);
        for (tifr, tcnt) in [(0x38, 0x94), (0x39, 0xA4)] {
            assert_eq!(r(&cpu, &bus, tifr), 0x20, "ICF at TOP, no TOV yet");
            assert_eq!(r16(&cpu, &bus, tcnt), 98, "counting down");
            w(&mut cpu, &mut bus, tifr, 0x20);
        }
        run_to(&mut cpu, &mut bus, 201);
        for (tifr, tcnt) in [(0x38, 0x94), (0x39, 0xA4)] {
            // OCRnA/B/C are 0: BOTTOM is also their compare match.
            assert_eq!(r(&cpu, &bus, tifr), 0x0F, "TOV and OCFnA..C at BOTTOM");
            assert_eq!(r16(&cpu, &bus, tcnt), 2, "counting up again");
        }
    }

    /// 17.11.33..40: a flag is set whether or not its enable is, an enable
    /// written while its flag is set fires at once, writing 1 clears a flag,
    /// and only ICIEn, OCIEnC..A and TOIEn exist (TIMSKn bits 5, 3..0).
    /// TIMER4 COMPB is vector 44.
    #[test]
    fn enabling_an_interrupt_with_its_flag_set_enters_it_at_once() {
        let (mut cpu, mut bus) = rig();
        w16(&mut cpu, &mut bus, 0xAA, 30); // OCR4B
        w(&mut cpu, &mut bus, 0xA1, 0x01); // clk/1, normal
        sei(&mut cpu);
        run_to(&mut cpu, &mut bus, 100);
        assert_eq!(r(&cpu, &bus, 0x39), 0x04, "OCF4B set, OCIE4B clear");
        assert_eq!(cpu.r[12], 0);
        w(&mut cpu, &mut bus, 0x72, 0xFF);
        assert_eq!(r(&cpu, &bus, 0x72), 0x2F, "TIMSK4 writable bits");
        w(&mut cpu, &mut bus, 0x72, 0x04); // OCIE4B
        let now = cpu.cycles;
        run_to(&mut cpu, &mut bus, now + 12);
        assert_eq!(cpu.r[12], 1, "TIMER4 COMPB entered on the enable");
        assert_eq!(r(&cpu, &bus, 0x39) & 0x04, 0);
        assert_eq!(cpu.r[STRAY], 0);
    }

    /// Fast PWM 10-bit (WGM 7) sets TOVn once per 1024 clocks (Table 17-2,
    /// 17.9.3); OC5A (PL3) non-inverting at OCR5A = 255 is high for
    /// 256 / 1024 of the period.
    #[test]
    fn fast_pwm_10_bit_on_oc5a() {
        let (mut cpu, mut bus) = rig();
        w16(&mut cpu, &mut bus, 0x128, 255);
        w(&mut cpu, &mut bus, PORTL - 1, 0x08); // DDRL3
        w(&mut cpu, &mut bus, 0x73, 0x01); // TOIE5
        w(&mut cpu, &mut bus, 0x120, 0x80 | 0x03); // COM5A = 2, WGM51:50
        w(&mut cpu, &mut bus, 0x121, 0x08 | 0x01); // WGM52 -> 7, clk/1
        sei(&mut cpu);
        let got = duty(&mut cpu, &mut bus, &[(PORTL, 3)], 10 * 1024)[0];
        assert!((got - 0.25).abs() < 0.01, "OC5A duty {got}");
        assert_eq!(cpu.r[19], 9, "TIMER5 OVF: 10 * 1024 clocks");
    }
}
