// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Timer/Counter1, the 16-bit timer: the Arduino `Servo` library's clock.
//!
//! Registers (data-space addresses, datasheet section 16):
//!
//! | Register | Addr | Kept |
//! |----------|------|------|
//! | `TIFR1`  | 0x36 | ICF1, OCF1B, OCF1A, TOV1; write 1 to clear |
//! | `TIMSK1` | 0x6F | ICIE1, OCIE1B, OCIE1A, TOIE1 |
//! | `TCCR1A` | 0x80 | COM1A, COM1B, WGM11:10 |
//! | `TCCR1B` | 0x81 | ICNC1, ICES1, WGM13:12, CS12:10 |
//! | `TCCR1C` | 0x82 | strobes, read as 0 |
//! | `TCNT1`, `ICR1`, `OCR1A`, `OCR1B` | 0x84..0x8B | 16-bit, through `TEMP` |
//!
//! 16-bit access goes through the shared `TEMP` byte as on silicon: writing
//! the high byte stores it in `TEMP`, and writing the low byte commits both.
//! Reading the low byte latches the high byte into `TEMP` for the next read.
//!
//! TOP is 0xFFFF, 0x00FF/0x01FF/0x03FF, OCR1A or ICR1, depending on the
//! mode. The phase-correct modes (WGM 1-3, 8-11) count up to TOP and back
//! down, so a cycle is `2 * TOP` clocks (Arduino's `analogWrite` on D9/D10:
//! 16 MHz / 64 / 510 = 490 Hz). Every other mode counts up to TOP and wraps.
//! A counter already past a lowered TOP runs on to 0xFFFF first. OCF1A/OCF1B
//! are set when the counter reaches OCR1A/OCR1B (on either slope). TOV1 is
//! set on the wrap to 0, at BOTTOM in phase-correct, or at MAX in CTC.
//! Double-buffered OCR1x is not modelled. OC1A (PB1) and OC1B (PB2) follow
//! `oc_pins.rs` in the PWM modes.
//!
//! Input capture (datasheet 16.6): an edge on ICP1 (PB0, Arduino D8) of the
//! polarity `ICES1` selects copies `TCNT1` into `ICR1` and sets `ICF1`, and
//! `TIMER1_CAPT` fires when `ICIE1` is set. The pad is sampled at instruction
//! boundaries by `ext_int.rs`, as `PINB` reads it, so a pad the firmware drives
//! itself captures too, as on silicon. In the modes where ICR1 is TOP
//! (WGM 8, 10, 12, 14) the unit is off. Not modelled: the 4-clock delay of the
//! noise canceler (`ICNC1`), the analog-comparator source (`ACSR.ACIC`), and
//! ICP1 on the ATmega2560 (PD4). The pad is watched while `ICIE1`, `ICES1` or
//! `ICNC1` is set: a firmware that polls `ICF1` for a falling edge with all
//! three clear is not seen, because watching PB0 on every Arduino sketch
//! (Timer1 runs from `init()`) would cost every sketch a pad read per
//! instruction.

use super::oc_pins::Waveform;
use super::{dual_dist, dual_from_phase, dual_hits, dual_phase, t2_hits, Avr};

/// `TIMER1_CAPT_vect` (`__vector_10`, byte 0x28).
pub const VEC_TIMER1_CAPT: u32 = 11;
/// `TIMER1_COMPA_vect` (`__vector_11`, byte 0x2C).
pub const VEC_TIMER1_COMPA: u32 = 12;
/// `TIMER1_COMPB_vect` (`__vector_12`, byte 0x30).
pub const VEC_TIMER1_COMPB: u32 = 13;
/// `TIMER1_OVF_vect` (`__vector_13`, byte 0x34).
pub const VEC_TIMER1_OVF: u32 = 14;
const T1_IRQ_MASK: u64 = (1 << VEC_TIMER1_CAPT)
    | (1 << VEC_TIMER1_COMPA)
    | (1 << VEC_TIMER1_COMPB)
    | (1 << VEC_TIMER1_OVF);

const TOV1: u8 = 1 << 0;
const OCF1A: u8 = 1 << 1;
const OCF1B: u8 = 1 << 2;
/// `ICF1` in `TIFR1`, and `ICIE1` at the same bit of `TIMSK1`.
const ICF1: u8 = 1 << 5;
/// `TCCR1B.ICES1`: capture on the rising edge (0: falling).
const ICES1: u8 = 1 << 6;
/// `TCCR1B.ICNC1`: input noise canceler.
const ICNC1: u8 = 1 << 7;

pub(super) const ADDR_TIFR1: u16 = 0x36;
pub(super) const ADDR_TIMSK1: u16 = 0x6F;
pub(super) const ADDR_TCCR1A: u16 = 0x80;
pub(super) const ADDR_OCR1BH: u16 = 0x8B;

/// Timer1 state. `temp` is a `Cell` because a low-byte read latches it.
#[derive(Debug, Default)]
pub struct Timer1 {
    pub tcnt: u16,
    pub tccra: u8,
    pub tccrb: u8,
    pub ocra: u16,
    pub ocrb: u16,
    pub icr: u16,
    pub timsk: u8,
    pub tifr: u8,
    /// Counting down (phase-correct modes only).
    pub down: bool,
    prescale_acc: u32,
    temp: std::cell::Cell<u8>,
}

impl Timer1 {
    fn wgm(&self) -> u8 {
        (self.tccra & 0x03) | ((self.tccrb >> 1) & 0x0C)
    }

    fn is_ctc(&self) -> bool {
        matches!(self.wgm(), 4 | 12)
    }

    pub(super) fn top(&self) -> u16 {
        match self.wgm() {
            1 | 5 => 0x00FF,
            2 | 6 => 0x01FF,
            3 | 7 => 0x03FF,
            4 | 9 | 11 | 15 => self.ocra,
            8 | 10 | 12 | 14 => self.icr,
            _ => 0xFFFF,
        }
    }

    pub(super) fn waveform(&self) -> Waveform {
        match self.wgm() {
            5..=7 | 14 | 15 => Waveform::Fast,
            1..=3 | 8..=11 => Waveform::PhaseCorrect,
            _ => Waveform::NonPwm,
        }
    }

    /// clk_I/O divider; 0 when stopped or clocked from T1 (not modelled).
    pub(super) fn prescaler(&self) -> u32 {
        match self.tccrb & 0x07 {
            1 => 1,
            2 => 8,
            3 => 64,
            4 => 256,
            5 => 1024,
            _ => 0,
        }
    }

    /// The input capture unit is off where ICR1 is TOP.
    fn capture_enabled(&self) -> bool {
        !matches!(self.wgm(), 8 | 10 | 12 | 14)
    }

    /// Must ICP1 be sampled at every boundary? See the module doc for why the
    /// polled-falling-edge case with every capture bit clear is left out.
    pub(super) fn capture_watched(&self) -> bool {
        self.capture_enabled() && (self.timsk & ICF1 != 0 || self.tccrb & (ICES1 | ICNC1) != 0)
    }

    /// An ICP1 edge: `rose` is its direction.
    fn capture_edge(&mut self, rose: bool) {
        if self.capture_enabled() && rose == (self.tccrb & ICES1 != 0) {
            self.icr = self.tcnt;
            self.tifr |= ICF1;
        }
    }

    /// Dual-slope counting, with a TOP it can turn around at.
    fn dual_slope(&self) -> bool {
        self.waveform() == Waveform::PhaseCorrect && self.top() > 0
    }

    fn tov_target(&self) -> u64 {
        if self.is_ctc() {
            0xFFFF
        } else {
            0
        }
    }

    /// Advance by `ticks` timer clocks, raising the flags it passes.
    fn apply_ticks(&mut self, mut ticks: u64) {
        let (ocra, ocrb, tov_at) = (
            u64::from(self.ocra),
            u64::from(self.ocrb),
            self.tov_target(),
        );
        let top = u64::from(self.top());
        let mut tcnt = u64::from(self.tcnt);
        let mut flags = 0u8;
        let mut hits = |phase: u64, n: u64, period: u64| {
            if t2_hits(phase, n, period, ocra) {
                flags |= OCF1A;
            }
            if t2_hits(phase, n, period, ocrb) {
                flags |= OCF1B;
            }
            if t2_hits(phase, n, period, tov_at) {
                flags |= TOV1;
            }
        };
        if tcnt > top {
            // Past a lowered TOP: count on to MAX and wrap to 0 first.
            let n = ticks.min(0x1_0000 - tcnt);
            hits(tcnt, n, 0x1_0000);
            tcnt = (tcnt + n) & 0xFFFF;
            ticks -= n;
        }
        if ticks > 0 && self.dual_slope() {
            let phase = dual_phase(tcnt, self.down, top);
            for (bit, target) in [(OCF1A, ocra), (OCF1B, ocrb), (TOV1, 0)] {
                if dual_hits(phase, ticks, top, target) {
                    flags |= bit;
                }
            }
            let (t, down) = dual_from_phase((phase + ticks) % (2 * top), top);
            tcnt = t;
            self.down = down;
        } else if ticks > 0 {
            let period = top + 1;
            hits(tcnt, ticks, period);
            tcnt = (tcnt + ticks) % period;
        }
        if !self.dual_slope() {
            self.down = false;
        }
        self.tcnt = tcnt as u16;
        self.tifr |= flags;
    }

    /// The pending bits this timer requests: flag and enable both set.
    fn irq_bits(&self) -> u64 {
        let on = self.tifr & self.timsk;
        let mut want = 0u64;
        if on & ICF1 != 0 {
            want |= 1 << VEC_TIMER1_CAPT;
        }
        if on & OCF1A != 0 {
            want |= 1 << VEC_TIMER1_COMPA;
        }
        if on & OCF1B != 0 {
            want |= 1 << VEC_TIMER1_COMPB;
        }
        if on & TOV1 != 0 {
            want |= 1 << VEC_TIMER1_OVF;
        }
        want
    }

    /// Timer clocks until the next enabled flag, or `None` if none can rise.
    fn ticks_to_irq(&self) -> Option<u64> {
        let top = u64::from(self.top());
        let tcnt = u64::from(self.tcnt);
        if tcnt > top {
            // Conservative: wake at the wrap and look again from there.
            return Some(0x1_0000 - tcnt);
        }
        let mask = self.timsk & (OCF1A | OCF1B | TOV1);
        if self.dual_slope() {
            let phase = dual_phase(tcnt, self.down, top);
            return [
                (OCF1A, u64::from(self.ocra)),
                (OCF1B, u64::from(self.ocrb)),
                (TOV1, 0),
            ]
            .into_iter()
            .filter(|(bit, _)| mask & bit != 0)
            .filter_map(|(_, target)| dual_dist(phase, top, target))
            .min();
        }
        let period = top + 1;
        let dist = |target: u64| {
            (target < period).then(|| match (target + period - tcnt) % period {
                0 => period,
                d => d,
            })
        };
        [
            (OCF1A, u64::from(self.ocra)),
            (OCF1B, u64::from(self.ocrb)),
            (TOV1, self.tov_target()),
        ]
        .into_iter()
        .filter(|(bit, _)| mask & bit != 0)
        .filter_map(|(_, target)| dist(target))
        .min()
    }
}

impl Avr {
    fn sync_timer1_irq(&mut self) {
        self.pending_irq = (self.pending_irq & !T1_IRQ_MASK) | self.t1.irq_bits();
    }

    /// Advance Timer1 by CPU clocks. Called beside Timer0 and Timer2.
    pub(super) fn tick_timer1(&mut self, cpu_cycles: u32) {
        let div = self.t1.prescaler();
        if div == 0 || cpu_cycles == 0 {
            return;
        }
        let acc = u64::from(self.t1.prescale_acc) + u64::from(cpu_cycles);
        let ticks = acc / u64::from(div);
        self.t1.prescale_acc = (acc % u64::from(div)) as u32;
        if ticks > 0 {
            self.t1.apply_ticks(ticks);
            self.sync_timer1_irq();
        }
    }

    /// CPU cycles until a Timer1 interrupt can wake a sleeping core.
    pub(super) fn timer1_wake_cycles(&self) -> Option<u64> {
        let div = u64::from(self.t1.prescaler());
        if !self.flag_i() || !self.io_clock_running() || div == 0 {
            return None;
        }
        let ticks = self.t1.ticks_to_irq()?;
        Some((ticks * div - u64::from(self.t1.prescale_acc).min(div - 1)).max(1))
    }

    /// An edge on ICP1 (PB0) seen at an instruction boundary.
    pub(super) fn timer1_icp_edge(&mut self, rose: bool) {
        self.t1.capture_edge(rose);
        self.sync_timer1_irq();
    }

    /// Entering a Timer1 vector clears its flag.
    pub(super) fn timer1_vector_entered(&mut self, vec: u32) {
        let bit = match vec {
            VEC_TIMER1_CAPT => ICF1,
            VEC_TIMER1_COMPA => OCF1A,
            VEC_TIMER1_COMPB => OCF1B,
            VEC_TIMER1_OVF => TOV1,
            _ => return,
        };
        self.t1.tifr &= !bit;
        self.sync_timer1_irq();
    }

    /// `Some` for a Timer1 register address.
    pub(super) fn timer1_read(&self, addr: u16) -> Option<u8> {
        let t = &self.t1;
        let wide = |v: u16, high: bool| {
            if high {
                t.temp.get()
            } else {
                t.temp.set((v >> 8) as u8);
                v as u8
            }
        };
        Some(match addr {
            ADDR_TIFR1 => t.tifr,
            ADDR_TIMSK1 => t.timsk,
            0x80 => t.tccra,
            0x81 => t.tccrb,
            0x82 => 0,
            0x84 | 0x85 => wide(t.tcnt, addr & 1 == 1),
            0x86 | 0x87 => wide(t.icr, addr & 1 == 1),
            0x88 | 0x89 => wide(t.ocra, addr & 1 == 1),
            0x8A | 0x8B => wide(t.ocrb, addr & 1 == 1),
            _ => return None,
        })
    }

    /// `true` when `addr` is a Timer1 register (and the write is applied).
    pub(super) fn timer1_write(&mut self, addr: u16, value: u8) -> bool {
        let t = &mut self.t1;
        let wide = |reg: &mut u16, temp: &std::cell::Cell<u8>| {
            *reg = (u16::from(temp.get()) << 8) | u16::from(value);
        };
        match addr {
            ADDR_TIFR1 => t.tifr &= !(value & 0x27),
            ADDR_TIMSK1 => t.timsk = value & 0x27,
            0x80 => t.tccra = value & 0xF3,
            0x81 => t.tccrb = value & 0xDF,
            // FOC1A/FOC1B strobes: force compare is not modelled.
            0x82 => {}
            0x85 | 0x87 | 0x89 | 0x8B => t.temp.set(value),
            0x84 => wide(&mut t.tcnt, &t.temp),
            0x86 => wide(&mut t.icr, &t.temp),
            0x88 => wide(&mut t.ocra, &t.temp),
            0x8A => wide(&mut t.ocrb, &t.temp),
            _ => return false,
        }
        self.sync_timer1_irq();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t1(wgm: u8, ocra: u16) -> Timer1 {
        Timer1 {
            tccra: wgm & 0x03,
            tccrb: ((wgm & 0x0C) << 1) | 2,
            ocra,
            ..Timer1::default()
        }
    }

    #[test]
    fn normal_mode_raises_compare_a_and_wraps_with_overflow() {
        let mut t = t1(0, 1000);
        t.apply_ticks(1000);
        assert_eq!((t.tcnt, t.tifr), (1000, OCF1A));
        t.tifr = 0;
        t.apply_ticks(0x1_0000 - 1000);
        // OCR1B is 0, so the wrap to 0 is also its compare match.
        assert_eq!((t.tcnt, t.tifr), (0, TOV1 | OCF1B));
        t.tifr = 0;
        t.apply_ticks(5 * 0x1_0000);
        assert_eq!(t.tifr, TOV1 | OCF1A | OCF1B);
    }

    #[test]
    fn ctc_clears_at_ocr1a_and_a_lowered_top_runs_on_to_max() {
        let mut t = t1(4, 99);
        t.apply_ticks(250);
        assert_eq!((t.tcnt, t.tifr & OCF1A), (50, OCF1A));
        let mut t = t1(4, 99);
        t.tcnt = 500;
        t.apply_ticks(0x1_0000 - 500);
        assert_eq!((t.tcnt, t.tifr & TOV1), (0, TOV1), "CTC passes MAX once");
    }

    /// Arduino's `analogWrite` mode for D9/D10: WGM 1, 8-bit phase-correct.
    /// Up to 0xFF and back down, OCF1A on both slopes, TOV1 at BOTTOM only.
    #[test]
    fn phase_correct_counts_up_and_back_down() {
        let mut t = t1(1, 128);
        t.apply_ticks(255);
        assert_eq!((t.tcnt, t.down, t.tifr), (255, false, OCF1A));
        t.tifr = 0;
        t.apply_ticks(1);
        assert_eq!((t.tcnt, t.down, t.tifr), (254, true, 0));
        t.apply_ticks(126);
        assert_eq!((t.tcnt, t.tifr), (128, OCF1A), "down-slope match");
        t.tifr = 0;
        t.apply_ticks(127);
        assert_eq!((t.tcnt, t.down, t.tifr), (1, true, 0));
        t.apply_ticks(1);
        assert_eq!((t.tcnt, t.down, t.tifr), (0, false, TOV1 | OCF1B));
        // 510 clocks per cycle: one more cycle lands back on BOTTOM.
        t.tifr = 0;
        t.apply_ticks(509);
        assert_eq!((t.tcnt, t.tifr & TOV1), (1, 0));
        t.apply_ticks(510 * 3 + 1);
        assert_eq!((t.tcnt, t.tifr & TOV1), (0, TOV1));
    }

    #[test]
    fn phase_correct_irq_distance_covers_both_slopes() {
        let mut t = t1(1, 100);
        t.timsk = OCF1A | TOV1;
        assert_eq!(t.ticks_to_irq(), Some(100));
        t.tcnt = 150;
        assert_eq!(
            t.ticks_to_irq(),
            Some(105 + 155),
            "up to TOP, then down to 100"
        );
        t.down = true;
        t.tcnt = 50;
        assert_eq!(t.ticks_to_irq(), Some(50), "BOTTOM first");
        t.timsk = OCF1A;
        assert_eq!(t.ticks_to_irq(), Some(150));
    }

    #[test]
    fn sixteen_bit_registers_go_through_temp() {
        let mut cpu = Avr::new();
        assert!(cpu.timer1_write(0x89, 0x12));
        assert!(cpu.timer1_write(0x88, 0x34));
        assert_eq!(cpu.t1.ocra, 0x1234);
        cpu.t1.tcnt = 0xABCD;
        assert_eq!(cpu.timer1_read(0x84), Some(0xCD));
        cpu.t1.tcnt = 0;
        assert_eq!(
            cpu.timer1_read(0x85),
            Some(0xAB),
            "high byte latched at the low read"
        );
    }

    /// Datasheet 16.6: the selected edge copies TCNT1 into ICR1 and sets ICF1;
    /// the other edge does nothing.
    #[test]
    fn rising_edge_capture_copies_the_counter_into_icr1() {
        let mut t = t1(0, 0);
        t.tccrb |= ICES1;
        t.tcnt = 400;
        t.capture_edge(false);
        assert_eq!((t.icr, t.tifr & ICF1), (0, 0), "falling edge ignored");
        t.capture_edge(true);
        assert_eq!((t.icr, t.tifr & ICF1), (400, ICF1));
        t.tcnt = 900;
        t.capture_edge(true);
        assert_eq!(t.icr, 900, "a new capture overwrites ICR1");
    }

    #[test]
    fn falling_edge_is_the_default_and_icr1_as_top_turns_capture_off() {
        let mut t = t1(0, 0);
        t.tcnt = 77;
        t.capture_edge(true);
        assert_eq!(t.tifr & ICF1, 0);
        t.capture_edge(false);
        assert_eq!((t.icr, t.tifr & ICF1), (77, ICF1));
        // WGM 14, fast PWM with TOP = ICR1: the unit is disconnected.
        let mut t = t1(14, 0);
        t.icr = 39_999;
        t.tcnt = 5;
        t.capture_edge(false);
        assert_eq!((t.icr, t.tifr & ICF1), (39_999, 0));
        assert!(!t.capture_watched());
    }

    #[test]
    fn capture_interrupt_is_pending_with_icie1_and_cleared_on_entry() {
        let mut cpu = Avr::new();
        cpu.timer1_write(0x81, ICES1 | 0x02);
        cpu.timer1_icp_edge(true);
        assert_eq!(cpu.pending_irq & (1 << VEC_TIMER1_CAPT), 0, "ICIE1 clear");
        cpu.timer1_write(ADDR_TIMSK1, ICF1);
        assert_ne!(cpu.pending_irq & (1 << VEC_TIMER1_CAPT), 0);
        cpu.timer1_vector_entered(VEC_TIMER1_CAPT);
        assert_eq!(cpu.pending_irq & (1 << VEC_TIMER1_CAPT), 0);
        assert_eq!(cpu.t1.tifr & ICF1, 0);
    }

    #[test]
    fn compare_a_interrupt_is_pending_while_flag_and_enable_are_set() {
        let mut cpu = Avr::new();
        cpu.timer1_write(ADDR_TIMSK1, OCF1A);
        cpu.timer1_write(0x81, 0x02); // clk/8, normal mode
        cpu.timer1_write(0x89, 0);
        cpu.timer1_write(0x88, 10);
        cpu.tick_timer1(8 * 10);
        assert_ne!(cpu.pending_irq & (1 << VEC_TIMER1_COMPA), 0);
        cpu.timer1_vector_entered(VEC_TIMER1_COMPA);
        assert_eq!(cpu.pending_irq & (1 << VEC_TIMER1_COMPA), 0);
        assert_eq!(cpu.t1.tifr & OCF1A, 0);
    }
}
