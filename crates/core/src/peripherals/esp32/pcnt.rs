// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-classic pulse counter (PCNT, TRM §17) at `DR_REG_PCNT_BASE`
//! 0x3FF5_7000 — what a quadrature rotary encoder or a flow / tacho sensor
//! counts on (`ESP32Encoder`, esp-idf `driver/pcnt.h`).
//!
//! Register map and reset values from esp-idf `soc/esp32/register/soc/
//! pcnt_reg.h` + `pcnt_struct.h`; action encodings from
//! `hal/esp32/include/hal/pcnt_ll.h` / `hal/pcnt_types.h`:
//!
//! | offset            | register                                          |
//! |-------------------|---------------------------------------------------|
//! | 0x00 + 0x0C·u     | `Un_CONF0` filter, event enables, ch0/ch1 modes   |
//! | 0x04 + 0x0C·u     | `Un_CONF1` thres0 [15:0], thres1 [31:16]          |
//! | 0x08 + 0x0C·u     | `Un_CONF2` h_lim [15:0], l_lim [31:16]            |
//! | 0x60 + 4·u        | `Un_CNT` 16-bit two's-complement count (RO)       |
//! | 0x80/84/88/8C     | `INT_RAW` / `INT_ST` / `INT_ENA` / `INT_CLR`      |
//! | 0x90 + 4·u        | `Un_STATUS` zero mode [1:0] + event latches [6:2] |
//! | 0xB0              | `CTRL` cnt_rst (bit 2u, reset 1) / pause (2u+1)   |
//! | 0xFC              | `DATE` 0x1412_2600                                |
//!
//! Each of the 8 units has two channels. A channel counts on its SIGNAL
//! input's edges (`POS_MODE` / `NEG_MODE`: 1 = +1, 2 = −1, else hold) and its
//! CONTROL input's level modifies that step (`HCTRL_MODE` / `LCTRL_MODE`:
//! 0 = keep, 1 = invert, 2 = inhibit). Full-quadrature decoding — what
//! `ESP32Encoder::attachFullQuad` programs — is ch0 = (sig A, ctrl B) and
//! ch1 = (sig B, ctrl A), four counts per Gray cycle.
//!
//! The inputs arrive through the GPIO input matrix: the classic GPIO port
//! resolves `GPIO_FUNCn_IN_SEL_CFG` for the 32 PCNT signals and hands every
//! level change to [`Peripheral::timer_input_edge`] at the engine cycle it
//! happened (`SystemBus::wire_esp32_pcnt_inputs`). Input `ti` is
//! `4·unit + k`, `k` in the `gpio_sig_map.h` order SIG_CH0, SIG_CH1,
//! CTRL_CH0, CTRL_CH1 ([`PCNT_SIGNALS`]).
//!
//! Limits: with `THR_H_LIM_EN` / `THR_L_LIM_EN` set (both are, at reset)
//! reaching `H_LIM`
//! counting up or `L_LIM` counting down clears the counter. A zero limit never
//! matches in its own direction, so a unit with both limits at 0 free-runs and
//! wraps at 16 bits. Clearing only with the event enabled is the convention
//! the ESP32-S3 model (`esp32s3::pcnt`) already uses; what silicon does with
//! the limit event disabled is not pinned down here. Each step that lands on
//! an enabled event (zero, h_lim, l_lim, thres0, thres1) latches it in
//! `Un_STATUS` and sets the unit's `INT_RAW` bit; the level interrupt is
//! `ETS_PCNT_INTR_SOURCE` = 48 through the DPORT matrix.
//!
//! NOT modelled: the glitch filter. `FILTER_EN` / `FILTER_THRES` read back as
//! written, but every edge counts. The filter rejects pulses narrower than
//! `FILTER_THRES` APB clocks (≤ 1023 / 80 MHz ≈ 12.8 µs); a mechanical
//! encoder or any stimulus slower than that counts the same either way, a
//! faster glitch train counts more here than on silicon.

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// `ETS_PCNT_INTR_SOURCE` (`soc/esp32/include/soc/interrupts.h`: LEDC = 43,
/// EFUSE, TWAI, RTC_CORE, RMT, then PCNT = 48, I2C_EXT0 = 49).
pub const PCNT_INTR_SOURCE_ID: u32 = 48;

/// GPIO-matrix input signal indices of the PCNT, per unit, in input order
/// `[SIG_CH0, SIG_CH1, CTRL_CH0, CTRL_CH1]` — esp-idf
/// `soc/esp32/include/soc/gpio_sig_map.h` (`PCNT_SIG_CH0_IN0_IDX` = 39 …
/// `PCNT_CTRL_CH1_IN4_IDX` = 58, then units 5..7 resume at 71 after the
/// 59..70 block of other signals).
pub const PCNT_SIGNALS: [[u32; 4]; UNITS] = [
    [39, 40, 41, 42],
    [43, 44, 45, 46],
    [47, 48, 49, 50],
    [51, 52, 53, 54],
    [55, 56, 57, 58],
    [71, 72, 73, 74],
    [75, 76, 77, 78],
    [79, 80, 81, 82],
];

const UNITS: usize = 8;

const REG_CNT: u64 = 0x60;
const REG_INT_RAW: u64 = 0x80;
const REG_INT_ST: u64 = 0x84;
const REG_INT_ENA: u64 = 0x88;
const REG_INT_CLR: u64 = 0x8C;
const REG_STATUS: u64 = 0x90;
const REG_CTRL: u64 = 0xB0;
const REG_DATE: u64 = 0xFC;

/// `Un_CONF0` reset: FILTER_THRES 0x10, FILTER_EN, THR_ZERO_EN, THR_H_LIM_EN,
/// THR_L_LIM_EN set (`pcnt_reg.h` field defaults).
const CONF0_RESET: u32 = 0x0000_3C10;
/// `PCNT_CTRL_REG` reset: every unit's `PLUS_CNT_RST` (bit 2u) set — a unit
/// counts only after the driver releases it.
const CTRL_RESET: u32 = 0x5555;
const DATE_RESET: u32 = 0x1412_2600;
const INT_MASK: u32 = (1 << UNITS) - 1;

const THR_ZERO_EN: u32 = 1 << 11;
const THR_H_LIM_EN: u32 = 1 << 12;
const THR_L_LIM_EN: u32 = 1 << 13;
const THR_THRES0_EN: u32 = 1 << 14;
const THR_THRES1_EN: u32 = 1 << 15;

/// `Un_STATUS` event latches (`pcnt_struct.h`: thres1_lat bit 2, thres0_lat
/// 3, l_lim_lat 4, h_lim_lat 5, zero_lat 6).
const LAT_THRES1: u32 = 1 << 2;
const LAT_THRES0: u32 = 1 << 3;
const LAT_L_LIM: u32 = 1 << 4;
const LAT_H_LIM: u32 = 1 << 5;
const LAT_ZERO: u32 = 1 << 6;

/// `pcnt_unit_zero_cross_mode_t` in `Un_STATUS[1:0]`.
const ZERO_FROM_POSITIVE: u32 = 0;
const ZERO_FROM_NEGATIVE: u32 = 1;
const ZERO_NEGATIVE: u32 = 2;
const ZERO_POSITIVE: u32 = 3;

/// Scheduler wake: an input edge or an MMIO write moved the IRQ level. The
/// event carries no work; firing it re-polls `matrix_irq_sources_into`.
const PCNT_LEVEL_WAKE_TOKEN: u32 = 0x9C47_0001;

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct Unit {
    conf0: u32,
    conf1: u32,
    conf2: u32,
    cnt: i16,
    status: u32,
    /// Input levels `[SIG_CH0, SIG_CH1, CTRL_CH0, CTRL_CH1]` as the matrix
    /// last delivered them.
    inputs: [bool; 4],
}

/// The classic-ESP32 PCNT block.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Esp32Pcnt {
    units: [Unit; UNITS],
    int_raw: u32,
    int_ena: u32,
    ctrl: u32,
    date: u32,
    #[serde(skip)]
    clock: Option<crate::cycle_clock::CycleClock>,
    #[serde(skip)]
    level_wake: bool,
}

impl Default for Esp32Pcnt {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32Pcnt {
    crate::cycle_clock::scheduler_mode!();

    pub fn new() -> Self {
        Self {
            units: [Unit {
                conf0: CONF0_RESET,
                ..Default::default()
            }; UNITS],
            int_raw: 0,
            int_ena: 0,
            ctrl: CTRL_RESET,
            date: DATE_RESET,
            clock: None,
            level_wake: false,
        }
    }

    /// The signed 16-bit count of `unit` (what `pcnt_ll_get_count` returns).
    pub fn count(&self, unit: usize) -> i16 {
        self.units[unit].cnt
    }

    fn irq_level(&self) -> bool {
        self.int_raw & self.int_ena != 0
    }

    fn held(&self, unit: usize) -> bool {
        self.ctrl & (0b11 << (2 * unit)) != 0
    }

    /// One edge on the SIGNAL input of `ch`: the step the channel's modes
    /// give it, `0` when it holds or is inhibited.
    fn channel_step(u: &Unit, ch: usize, rising: bool) -> i32 {
        let shift = 16 + 8 * ch as u32;
        let neg = (u.conf0 >> shift) & 3;
        let pos = (u.conf0 >> (shift + 2)) & 3;
        let hctrl = (u.conf0 >> (shift + 4)) & 3;
        let lctrl = (u.conf0 >> (shift + 6)) & 3;
        let step = match if rising { pos } else { neg } {
            1 => 1,
            2 => -1,
            _ => return 0,
        };
        let ctrl_high = u.inputs[2 + ch];
        match if ctrl_high { hctrl } else { lctrl } {
            1 => -step,
            2 => 0,
            _ => step,
        }
    }

    /// Apply one counting step to `unit`, then raise the events it lands on.
    fn count_step(&mut self, unit: usize, step: i32) {
        let u = &mut self.units[unit];
        let before = u.cnt;
        let mut cnt = before.wrapping_add(step as i16);
        let h_lim = (u.conf2 & 0xFFFF) as u16 as i16;
        let l_lim = (u.conf2 >> 16) as u16 as i16;
        let thres0 = (u.conf1 & 0xFFFF) as u16 as i16;
        let thres1 = (u.conf1 >> 16) as u16 as i16;
        let mut events = 0;
        if u.conf0 & THR_H_LIM_EN != 0 && step > 0 && h_lim != 0 && cnt >= h_lim {
            events |= LAT_H_LIM;
            cnt = 0;
        } else if u.conf0 & THR_L_LIM_EN != 0 && step < 0 && l_lim != 0 && cnt <= l_lim {
            events |= LAT_L_LIM;
            cnt = 0;
        }
        if cnt == 0 && before != 0 {
            events |= LAT_ZERO & mask_if(u.conf0 & THR_ZERO_EN);
        }
        if cnt == thres0 {
            events |= LAT_THRES0 & mask_if(u.conf0 & THR_THRES0_EN);
        }
        if cnt == thres1 {
            events |= LAT_THRES1 & mask_if(u.conf0 & THR_THRES1_EN);
        }
        let mode = match cnt {
            0 if before < 0 => ZERO_FROM_NEGATIVE,
            0 => ZERO_FROM_POSITIVE,
            c if c < 0 => ZERO_NEGATIVE,
            _ => ZERO_POSITIVE,
        };
        u.cnt = cnt;
        let lat = if events != 0 { events } else { u.status & !3 };
        u.status = lat | mode;
        if events != 0 {
            self.int_raw |= 1 << unit;
        }
    }

    fn reg(&self, off: u64) -> u32 {
        match off {
            o if o < REG_CNT => {
                let u = &self.units[(o / 12) as usize];
                match o % 12 {
                    0 => u.conf0,
                    4 => u.conf1,
                    _ => u.conf2,
                }
            }
            o if o < REG_INT_RAW => self.units[((o - REG_CNT) / 4) as usize].cnt as u16 as u32,
            REG_INT_RAW => self.int_raw,
            REG_INT_ST => self.int_raw & self.int_ena,
            REG_INT_ENA => self.int_ena,
            REG_INT_CLR => 0,
            o if (REG_STATUS..REG_CTRL).contains(&o) => {
                self.units[((o - REG_STATUS) / 4) as usize].status
            }
            REG_CTRL => self.ctrl,
            REG_DATE => self.date,
            _ => 0,
        }
    }

    fn set_reg(&mut self, off: u64, value: u32) {
        match off {
            o if o < REG_CNT => {
                let u = &mut self.units[(o / 12) as usize];
                match o % 12 {
                    0 => u.conf0 = value,
                    4 => u.conf1 = value,
                    _ => u.conf2 = value,
                }
            }
            REG_INT_ENA => self.int_ena = value & INT_MASK,
            REG_INT_CLR => self.int_raw &= !value,
            REG_CTRL => {
                self.ctrl = value & 0xFFFF;
                for unit in 0..UNITS {
                    if self.ctrl & (1 << (2 * unit)) != 0 {
                        self.units[unit].cnt = 0;
                    }
                }
            }
            REG_DATE => self.date = value,
            // Un_CNT, INT_RAW, INT_ST and Un_STATUS are read-only.
            _ => {}
        }
    }

    fn write_word(&mut self, off: u64, value: u32) {
        let before = self.irq_level();
        self.set_reg(off, value);
        if self.scheduler_mode() && self.irq_level() != before {
            self.level_wake = true;
        }
    }
}

#[inline]
fn mask_if(bit: u32) -> u32 {
    if bit != 0 {
        u32::MAX
    } else {
        0
    }
}

impl Peripheral for Esp32Pcnt {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.reg(offset & !3);
        Ok((word >> ((offset % 4) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let off = offset & !3;
        let shift = (offset % 4) * 8;
        // INT_CLR is write-1-to-clear: merging into a read-back would clear
        // nothing the byte did not name.
        let base = if off == REG_INT_CLR { 0 } else { self.reg(off) };
        let word = (base & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_word(off, word);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset & !3, value);
        Ok(())
    }

    /// Input `ti` = `4·unit + k` (`k`: SIG_CH0, SIG_CH1, CTRL_CH0, CTRL_CH1)
    /// moved to `level`. A signal edge counts per the channel's modes and the
    /// control input's current level; a control edge only changes that level.
    fn timer_input_edge(&mut self, ti: u8, level: bool, _cycle: u64) -> bool {
        let (unit, k) = ((ti / 4) as usize, (ti % 4) as usize);
        if unit >= UNITS || self.units[unit].inputs[k] == level {
            return false;
        }
        self.units[unit].inputs[k] = level;
        if k >= 2 || self.held(unit) {
            return false;
        }
        let step = Self::channel_step(&self.units[unit], k, level);
        if step == 0 {
            return false;
        }
        let before = self.irq_level();
        self.count_step(unit, step);
        if self.irq_level() != before && self.scheduler_mode() {
            self.level_wake = true;
        }
        self.level_wake
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if std::mem::take(&mut self.level_wake) {
            vec![(0, PCNT_LEVEL_WAKE_TOKEN)]
        } else {
            Vec::new()
        }
    }

    /// LEVEL interrupt on the walk: assert source 48 every tick while an
    /// enabled `INT_RAW` bit is set (the ISR clears it through `INT_CLR`).
    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult {
            explicit_irqs: self.irq_level().then(|| vec![PCNT_INTR_SOURCE_ID]),
            ..Default::default()
        }
    }

    /// The same level for the DPORT scheduler arm.
    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        if self.irq_level() {
            out.push(PCNT_INTR_SOURCE_ID);
        }
    }

    fn attach_cycle_clock(&mut self, clock: crate::cycle_clock::CycleClock) {
        self.clock = Some(clock);
        self.level_wake |= self.scheduler_mode() && self.irq_level();
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    /// Conditional for the same reason as the classic I²C model's: without a
    /// clock there is no scheduler, and the walk's `tick()` is what delivers
    /// the IRQ level.
    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if let Ok(s) = serde_json::from_value::<Esp32Pcnt>(state) {
            let clock = self.clock.take();
            *self = s;
            self.clock = clock;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ESP32Encoder::attachFullQuad` on unit `u`: ch0 counts A's edges
    /// (rise −1, fall +1) reversed while B is low; ch1 counts B's edges
    /// (rise +1, fall −1) reversed while A is low. Limits ±32766, released
    /// from reset.
    fn full_quad(p: &mut Esp32Pcnt, u: u64) {
        // ch0: neg=INC(1) pos=DEC(2) hctrl=KEEP(0) lctrl=INVERSE(1);
        // ch1: neg=DEC(2) pos=INC(1) hctrl=KEEP(0) lctrl=INVERSE(1).
        let ch0 = 1 | (2 << 2) | (1 << 6);
        let ch1 = 2 | (1 << 2) | (1 << 6);
        let conf0 = (CONF0_RESET & 0xFFFF) | (ch0 << 16) | (ch1 << 24);
        p.write_u32(u * 12, conf0).unwrap();
        p.write_u32(u * 12 + 8, (32766u32) | ((-32766i16 as u16 as u32) << 16))
            .unwrap();
        let ctrl = p.read_u32(REG_CTRL).unwrap() & !(0b11 << (2 * u));
        p.write_u32(REG_CTRL, ctrl).unwrap();
    }

    /// Walk the Gray sequence `steps` phases (A leads B for positive).
    fn quad(p: &mut Esp32Pcnt, u: u8, phase: &mut i32, steps: i32) {
        for _ in 0..steps.abs() {
            *phase += steps.signum();
            let ph = phase.rem_euclid(4);
            let a = ph == 1 || ph == 2;
            let b = ph == 2 || ph == 3;
            p.timer_input_edge(4 * u, a, 0);
            p.timer_input_edge(4 * u + 1, b, 0);
            p.timer_input_edge(4 * u + 2, b, 0);
            p.timer_input_edge(4 * u + 3, a, 0);
        }
    }

    #[test]
    fn reset_values_match_pcnt_reg_h() {
        let p = Esp32Pcnt::new();
        for u in 0..UNITS as u64 {
            assert_eq!(p.read_u32(u * 12).unwrap(), 0x3C10);
            assert_eq!(p.read_u32(REG_CNT + 4 * u).unwrap(), 0);
        }
        assert_eq!(p.read_u32(REG_CTRL).unwrap(), 0x5555);
        assert_eq!(p.read_u32(REG_DATE).unwrap(), 0x1412_2600);
    }

    #[test]
    fn unit_held_in_reset_does_not_count() {
        let mut p = Esp32Pcnt::new();
        // ch0 pos = INC, but CTRL still holds unit 0 in reset.
        p.write_u32(0, CONF0_RESET | (1 << 18)).unwrap();
        p.timer_input_edge(0, true, 0);
        assert_eq!(p.count(0), 0);
        p.write_u32(REG_CTRL, CTRL_RESET & !1).unwrap();
        p.timer_input_edge(0, false, 0);
        p.timer_input_edge(0, true, 0);
        assert_eq!(p.count(0), 1);
        // Pause holds the count; clearing pause resumes it.
        p.write_u32(REG_CTRL, (CTRL_RESET & !1) | 2).unwrap();
        p.timer_input_edge(0, false, 0);
        p.timer_input_edge(0, true, 0);
        assert_eq!(p.count(0), 1);
        // A cnt_rst pulse clears it.
        p.write_u32(REG_CTRL, CTRL_RESET).unwrap();
        p.write_u32(REG_CTRL, CTRL_RESET & !1).unwrap();
        assert_eq!(p.count(0), 0);
    }

    #[test]
    fn full_quadrature_counts_four_per_cycle_both_ways() {
        let mut p = Esp32Pcnt::new();
        full_quad(&mut p, 3);
        let mut phase = 0;
        quad(&mut p, 3, &mut phase, 8);
        let up = p.count(3);
        assert_eq!(up.abs(), 8, "8 Gray phases = 8 counts in full quad");
        quad(&mut p, 3, &mut phase, -12);
        assert_eq!(p.count(3), up - 12 * up.signum());
        // Other units never moved.
        assert_eq!(p.count(0), 0);
    }

    #[test]
    fn inhibit_and_invert_control_modes() {
        let mut p = Esp32Pcnt::new();
        // ch0: pos = INC, hctrl = HOLD (2), lctrl = INVERSE (1).
        p.write_u32(0, CONF0_RESET | (1 << 18) | (2 << 20) | (1 << 22))
            .unwrap();
        p.write_u32(REG_CTRL, CTRL_RESET & !1).unwrap();
        p.timer_input_edge(2, true, 0); // ctrl high: inhibit
        p.timer_input_edge(0, true, 0);
        p.timer_input_edge(0, false, 0);
        assert_eq!(p.count(0), 0);
        p.timer_input_edge(2, false, 0); // ctrl low: invert
        p.timer_input_edge(0, true, 0);
        assert_eq!(p.count(0), -1);
    }

    #[test]
    fn high_limit_clears_counter_and_raises_interrupt() {
        let mut p = Esp32Pcnt::new();
        p.write_u32(0, CONF0_RESET | (1 << 18)).unwrap();
        p.write_u32(8, 3).unwrap(); // h_lim = 3
        p.write_u32(REG_INT_ENA, 1).unwrap();
        p.write_u32(REG_CTRL, CTRL_RESET & !1).unwrap();
        for _ in 0..2 {
            p.timer_input_edge(0, true, 0);
            p.timer_input_edge(0, false, 0);
        }
        assert_eq!(p.count(0), 2);
        assert_eq!(p.read_u32(REG_INT_ST).unwrap(), 0);
        p.timer_input_edge(0, true, 0);
        assert_eq!(p.count(0), 0, "reaching h_lim clears the counter");
        let status = p.read_u32(REG_STATUS).unwrap();
        assert_ne!(status & LAT_H_LIM, 0);
        assert_ne!(status & LAT_ZERO, 0);
        assert_eq!(p.read_u32(REG_INT_ST).unwrap(), 1);
        assert!(p.tick().explicit_irqs == Some(vec![PCNT_INTR_SOURCE_ID]));
        p.write_u32(REG_INT_CLR, 1).unwrap();
        assert_eq!(p.read_u32(REG_INT_RAW).unwrap(), 0);
        assert!(p.tick().explicit_irqs.is_none());
    }

    #[test]
    fn threshold_event_only_when_enabled() {
        let mut p = Esp32Pcnt::new();
        p.write_u32(0, CONF0_RESET | (1 << 18)).unwrap();
        p.write_u32(4, 2).unwrap(); // thres0 = 2
        p.write_u32(REG_CTRL, CTRL_RESET & !1).unwrap();
        for _ in 0..2 {
            p.timer_input_edge(0, true, 0);
            p.timer_input_edge(0, false, 0);
        }
        assert_eq!(p.read_u32(REG_INT_RAW).unwrap(), 0);
        p.write_u32(0, CONF0_RESET | (1 << 18) | THR_THRES0_EN)
            .unwrap();
        p.write_u32(REG_CTRL, CTRL_RESET).unwrap();
        p.write_u32(REG_CTRL, CTRL_RESET & !1).unwrap();
        for _ in 0..2 {
            p.timer_input_edge(0, true, 0);
            p.timer_input_edge(0, false, 0);
        }
        assert_eq!(p.read_u32(REG_INT_RAW).unwrap(), 1);
        assert_ne!(p.read_u32(REG_STATUS).unwrap() & LAT_THRES0, 0);
    }

    #[test]
    fn negative_count_reads_as_16_bit_twos_complement() {
        let mut p = Esp32Pcnt::new();
        p.write_u32(0, CONF0_RESET | (2 << 18)).unwrap(); // pos = DEC
        p.write_u32(REG_CTRL, CTRL_RESET & !1).unwrap();
        p.timer_input_edge(0, true, 0);
        assert_eq!(p.read_u32(REG_CNT).unwrap(), 0xFFFF);
        assert_eq!(p.read_u32(REG_STATUS).unwrap() & 3, ZERO_NEGATIVE);
    }
}
