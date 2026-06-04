// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! PCNT (Pulse Counter) peripheral for ESP32-S3.
//!
//! Four independent units (U0..U3), each with two input channels (CH0/CH1),
//! a signed 16-bit count register, and a set of comparators (high/low limit,
//! two arbitrary thresholds, and zero) that raise a single per-unit
//! "threshold/limit event" interrupt. This module is a *self-contained*
//! `Peripheral` impl mirroring `peripherals/uart.rs`: round-trip config
//! registers, faithful reset defaults, a `tick()` that emits the PCNT
//! interrupt-matrix source while the masked status is non-zero, and a
//! `#[cfg(test)]` block.
//!
//! ## Register map (ESP32-S3 TRM §11; verified against IDF `soc/pcnt_reg.h`)
//!
//! PCNT base = `DR_REG_PCNT_BASE = 0x6001_7000`. All offsets below are
//! relative to that base (this peripheral's MMIO window).
//!
//! | Offset | Name           | Notes |
//! |-------:|----------------|-------|
//! | 0x00   | U0_CONF0       | edge/level modes, filter, comparator enables |
//! | 0x04   | U0_CONF1       | THRES0 [15:0], THRES1 [31:16] |
//! | 0x08   | U0_CONF2       | H_LIM [15:0], L_LIM [31:16] |
//! | 0x0C   | U1_CONF0       | (+0x0C per unit for CONF0/1/2 triplet) |
//! | 0x10   | U1_CONF1       | |
//! | 0x14   | U1_CONF2       | |
//! | 0x18   | U2_CONF0       | |
//! | 0x1C   | U2_CONF1       | |
//! | 0x20   | U2_CONF2       | |
//! | 0x24   | U3_CONF0       | |
//! | 0x28   | U3_CONF1       | |
//! | 0x2C   | U3_CONF2       | |
//! | 0x30   | U0_CNT         | RO, PULSE_CNT [15:0] (signed) |
//! | 0x34   | U1_CNT         | RO |
//! | 0x38   | U2_CNT         | RO |
//! | 0x3C   | U3_CNT         | RO |
//! | 0x40   | INT_RAW        | RO, bits[3:0] = per-unit THR_EVENT raw |
//! | 0x44   | INT_ST         | RO, INT_RAW & INT_ENA |
//! | 0x48   | INT_ENA        | R/W, bits[3:0] per-unit enable |
//! | 0x4C   | INT_CLR        | WO, W1C of INT_RAW bits |
//! | 0x50   | U0_STATUS      | RO, latched event flags + zero-mode |
//! | 0x54   | U1_STATUS      | RO |
//! | 0x58   | U2_STATUS      | RO |
//! | 0x5C   | U3_STATUS      | RO |
//! | 0x60   | CTRL           | per-unit RST(bit 2n)/PAUSE(bit 2n+1), CLK_EN(bit16) |
//! | 0xFC   | DATE           | version, default 0x1908_2001 |
//!
//! ## CONF0 bitfields (per unit)
//!
//! | Bits   | Field            | Meaning |
//! |--------|------------------|---------|
//! | [9:0]  | FILTER_THRES     | filter width (APB cycles); reset 16 |
//! | [10]   | FILTER_EN        | input filter enable; reset 1 |
//! | [11]   | THR_ZERO_EN      | zero comparator enable; reset 1 |
//! | [12]   | THR_H_LIM_EN     | high-limit comparator enable; reset 1 |
//! | [13]   | THR_L_LIM_EN     | low-limit comparator enable; reset 1 |
//! | [14]   | THR_THRES0_EN    | thres0 comparator enable; reset 0 |
//! | [15]   | THR_THRES1_EN    | thres1 comparator enable; reset 0 |
//! | [17:16]| CH0_NEG_MODE     | 1=inc, 2=dec, 0/3=none on CH0 neg edge |
//! | [19:18]| CH0_POS_MODE     | as above for CH0 pos edge |
//! | [21:20]| CH0_HCTRL_MODE   | control-high modifier |
//! | [23:22]| CH0_LCTRL_MODE   | control-low modifier |
//! | [25:24]| CH1_NEG_MODE     | CH1 neg edge |
//! | [27:26]| CH1_POS_MODE     | CH1 pos edge |
//! | [29:28]| CH1_HCTRL_MODE   | |
//! | [31:30]| CH1_LCTRL_MODE   | |
//!
//! ## Source ID (IDF `soc/interrupts.h`; verified vs esp32s3-pac 0.35.2)
//!
//! `ETS_PCNT_INTR_SOURCE` is interrupt-matrix source **41** (esp32s3-pac
//! `Interrupt::PCNT = 41`). There is a *single* PCNT source for all four
//! units; firmware reads INT_ST to learn which unit(s) fired. We emit it via
//! `PeripheralTickResult.explicit_irqs` while any masked status bit is set
//! (level-sensitive — held until firmware writes INT_CLR), matching the
//! systimer peripheral's IRQ-delivery model.

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// PCNT interrupt-matrix source ID (`ETS_PCNT_INTR_SOURCE`, esp32s3-pac
/// `Interrupt::PCNT = 41`). Single source shared by all four units.
pub const PCNT_INTR_SOURCE: u32 = 41;

/// Number of pulse-counter units.
const NUM_UNITS: usize = 4;

// ── CONF0 bit positions (per unit) ──
const CONF0_THR_ZERO_EN: u32 = 1 << 11;
const CONF0_THR_H_LIM_EN: u32 = 1 << 12;
const CONF0_THR_L_LIM_EN: u32 = 1 << 13;
const CONF0_THR_THRES0_EN: u32 = 1 << 14;
const CONF0_THR_THRES1_EN: u32 = 1 << 15;

/// CONF0 reset value: FILTER_THRES=16, FILTER_EN=1, THR_ZERO_EN=1,
/// THR_H_LIM_EN=1, THR_L_LIM_EN=1 (THRES0/1_EN default 0, mode fields 0).
/// = 16 | (1<<10) | (1<<11) | (1<<12) | (1<<13) = 0x3C10.
const CONF0_RESET: u32 = 16 | (1 << 10) | (1 << 11) | (1 << 12) | (1 << 13);

// ── CTRL bit positions ──
/// Per-unit reset bit lives at 2*unit; pause at 2*unit+1.
const fn ctrl_rst_bit(unit: usize) -> u32 {
    1 << (2 * unit)
}
const fn ctrl_pause_bit(unit: usize) -> u32 {
    1 << (2 * unit + 1)
}
/// CTRL bit 16 — register clock-gate enable. Documented for completeness;
/// the sim always permits register access, so the gate isn't enforced.
#[allow(dead_code)]
const CTRL_CLK_EN: u32 = 1 << 16;
/// CTRL reset value: all four RST bits set (bits 0/2/4/6), CLK_EN clear.
/// Real silicon powers up with every unit held in reset (RST default 1).
const CTRL_RESET: u32 = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6);

/// PCNT version register default (`PCNT_DATE` = 419898881 = 0x1908_2001).
const DATE_RESET: u32 = 0x1908_2001;

// ── STATUS (U_n) latched-event bit positions ──
const STATUS_THRES1_LAT: u32 = 1 << 2;
const STATUS_THRES0_LAT: u32 = 1 << 3;
const STATUS_L_LIM_LAT: u32 = 1 << 4;
const STATUS_H_LIM_LAT: u32 = 1 << 5;
const STATUS_ZERO_LAT: u32 = 1 << 6;
// bits[1:0] = ZERO_MODE (0..3); reflects direction across zero.

/// Per-unit state. Config registers round-trip verbatim; `count` is the live
/// signed 16-bit counter (RO from firmware's view); `status` mirrors the
/// latched event flags exposed in `U_n_STATUS`.
#[derive(Debug, Default, Clone, Copy)]
struct UnitState {
    /// CONF0 — edge/level modes, filter, comparator enables. Round-tripped.
    conf0: u32,
    /// CONF1 — THRES0 [15:0], THRES1 [31:16]. Round-tripped.
    conf1: u32,
    /// CONF2 — H_LIM [15:0], L_LIM [31:16]. Round-tripped.
    conf2: u32,
    /// Live signed 16-bit pulse count. No real edge source is wired in the
    /// sim, so this stays 0 unless `inject_pulses` drives it or CTRL.rst
    /// clears it.
    count: i16,
    /// INT_RAW pending bit for this unit (sticky until INT_CLR).
    int_raw: bool,
    /// Latched STATUS event flags (THRES/LIM/ZERO_LAT + ZERO_MODE in [1:0]).
    /// Latched at the moment an event fires; reflects the most recent event.
    status: u32,
}

impl UnitState {
    fn reset() -> Self {
        Self {
            conf0: CONF0_RESET,
            conf1: 0,
            conf2: 0,
            count: 0,
            int_raw: false,
            status: 0,
        }
    }

    fn thres0(&self) -> i16 {
        (self.conf1 & 0xFFFF) as u16 as i16
    }
    fn thres1(&self) -> i16 {
        ((self.conf1 >> 16) & 0xFFFF) as u16 as i16
    }
    fn h_lim(&self) -> i16 {
        (self.conf2 & 0xFFFF) as u16 as i16
    }
    fn l_lim(&self) -> i16 {
        ((self.conf2 >> 16) & 0xFFFF) as u16 as i16
    }
}

/// ESP32-S3 PCNT — four pulse-counter units with per-unit event interrupts.
pub struct Esp32s3Pcnt {
    units: [UnitState; NUM_UNITS],
    /// INT_ENA (0x48) — bits[3:0] gate IRQ delivery (and INT_ST visibility).
    int_ena: u32,
    /// CTRL (0x60) — per-unit RST/PAUSE bits + CLK_EN. Round-tripped; the
    /// RST bits additionally clear the matching unit's count while set.
    ctrl: u32,
    /// Interrupt-matrix source id this peripheral pends (constructor arg,
    /// like `Uart`/`Systimer` take their wiring) — `PCNT_INTR_SOURCE` (41).
    source_id: u32,
}

impl core::fmt::Debug for Esp32s3Pcnt {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Esp32s3Pcnt")
            .field("counts", &self.units.map(|u| u.count))
            .field("int_raw", &self.int_raw_word())
            .field("int_ena", &self.int_ena)
            .field("ctrl", &format_args!("{:#06x}", self.ctrl))
            .finish()
    }
}

impl Default for Esp32s3Pcnt {
    fn default() -> Self {
        Self::new(PCNT_INTR_SOURCE)
    }
}

impl Esp32s3Pcnt {
    /// Construct a PCNT bound to interrupt-matrix `source_id`. Production
    /// wiring passes `PCNT_INTR_SOURCE` (41); the parameter mirrors
    /// `Uart`/`Systimer` taking their wiring so tests can assert the exact
    /// source emitted on the IRQ path.
    pub fn new(source_id: u32) -> Self {
        Self {
            units: [UnitState::reset(); NUM_UNITS],
            int_ena: 0,
            ctrl: CTRL_RESET,
            source_id,
        }
    }

    /// INT_RAW (0x40): bits[3:0] = per-unit pending event.
    fn int_raw_word(&self) -> u32 {
        let mut v = 0u32;
        for (i, u) in self.units.iter().enumerate() {
            if u.int_raw {
                v |= 1 << i;
            }
        }
        v
    }

    /// INT_ST (0x44): masked status = INT_RAW & INT_ENA.
    fn int_st_word(&self) -> u32 {
        self.int_raw_word() & self.int_ena & 0xF
    }

    /// Whether CTRL holds unit `i` in reset (RST bit set).
    fn unit_in_reset(&self, i: usize) -> bool {
        self.ctrl & ctrl_rst_bit(i) != 0
    }

    /// 32-bit register read (word granularity). Reads are side-effect-free.
    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            // ── CONFn triplet, 0x0C bytes per unit ──
            0x00..=0x2F => {
                let unit = (offset / 0x0C) as usize;
                let which = (offset % 0x0C) / 4;
                match which {
                    0 => self.units[unit].conf0,
                    1 => self.units[unit].conf1,
                    _ => self.units[unit].conf2,
                }
            }
            // ── U_n_CNT (RO) — sign-extend the 16-bit count into [15:0] ──
            0x30..=0x3F => {
                let unit = ((offset - 0x30) / 4) as usize;
                // While held in reset the readback is 0.
                if self.unit_in_reset(unit) {
                    0
                } else {
                    self.units[unit].count as u16 as u32
                }
            }
            0x40 => self.int_raw_word(),
            0x44 => self.int_st_word(),
            0x48 => self.int_ena & 0xF,
            // 0x4C INT_CLR is write-only; reads as 0 on silicon.
            0x4C => 0,
            // ── U_n_STATUS (RO) ──
            0x50..=0x5F => {
                let unit = ((offset - 0x50) / 4) as usize;
                self.units[unit].status
            }
            0x60 => self.ctrl,
            0xFC => DATE_RESET,
            _ => 0,
        }
    }

    /// 32-bit register write (word granularity). Applies side effects
    /// (count reset on CTRL.rst, W1C on INT_CLR).
    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            // ── CONFn triplet — round-trip verbatim ──
            0x00..=0x2F => {
                let unit = (offset / 0x0C) as usize;
                let which = (offset % 0x0C) / 4;
                match which {
                    0 => self.units[unit].conf0 = value,
                    1 => self.units[unit].conf1 = value,
                    _ => self.units[unit].conf2 = value,
                }
            }
            // 0x30..0x3F U_n_CNT are read-only; ignore writes.
            0x30..=0x3F => {}
            // 0x40 INT_RAW is read-only.
            0x40 => {}
            // 0x44 INT_ST is read-only.
            0x44 => {}
            0x48 => self.int_ena = value & 0xF,
            0x4C => {
                // INT_CLR: write-1-to-clear the matching INT_RAW bit.
                for (i, u) in self.units.iter_mut().enumerate() {
                    if value & (1 << i) != 0 {
                        u.int_raw = false;
                    }
                }
            }
            // 0x50..0x5F U_n_STATUS are read-only.
            0x50..=0x5F => {}
            0x60 => {
                self.ctrl = value;
                // Any unit whose RST bit is set has its count cleared. Real
                // silicon holds the counter at 0 while RST is high; we clear
                // on write and the CNT readback also returns 0 while held.
                for i in 0..NUM_UNITS {
                    if self.unit_in_reset(i) {
                        self.units[i].count = 0;
                    }
                }
            }
            _ => {}
        }
    }

    /// Inject `n` pulses (positive count steps) into `unit`, honouring that
    /// unit's CONF comparator setup — the sim has no real edge source wired
    /// in, so this is the test/future-input hook (analogous to the UART's
    /// `push_rx`). Each step advances the live count by one, then evaluates
    /// the high/low-limit, threshold and zero comparators:
    ///
    /// * **H_LIM**: when the count reaches `h_lim` (and the comparator is
    ///   enabled) the counter *wraps to 0* and a high-limit event fires.
    /// * **L_LIM**: symmetric for the low limit (negative steps).
    /// * **THRES0/THRES1**: when the count equals the threshold (comparator
    ///   enabled) a thres event fires; the counter keeps going.
    /// * **ZERO**: when the count crosses to 0 a zero event fires.
    ///
    /// Any fired event latches `STATUS` and sets the unit's `INT_RAW` bit.
    /// A paused (CTRL.pause) or reset (CTRL.rst) unit ignores injection,
    /// matching the frozen-counter hardware semantics.
    ///
    /// `n` may be negative to drive the counter down (e.g. a CH set to
    /// "decrease"); positive drives up.
    pub fn inject_pulses(&mut self, unit: usize, n: i32) {
        if unit >= NUM_UNITS {
            return;
        }
        if self.unit_in_reset(unit) || self.ctrl & ctrl_pause_bit(unit) != 0 {
            return;
        }
        let step: i16 = if n >= 0 { 1 } else { -1 };
        for _ in 0..n.unsigned_abs() {
            self.step_once(unit, step);
        }
    }

    /// Advance `unit`'s counter by a single `+1`/`-1` step and run the
    /// comparators. Factored out of `inject_pulses` so the per-edge event
    /// logic is exercised one pulse at a time.
    fn step_once(&mut self, unit: usize, step: i16) {
        let h_lim = self.units[unit].h_lim();
        let l_lim = self.units[unit].l_lim();
        let thres0 = self.units[unit].thres0();
        let thres1 = self.units[unit].thres1();
        let conf0 = self.units[unit].conf0;

        let next = self.units[unit].count.wrapping_add(step);
        let mut event = false;
        let mut status = 0u32;

        // High-limit: count reached h_lim → wrap to 0 + H_LIM event.
        if conf0 & CONF0_THR_H_LIM_EN != 0 && step > 0 && h_lim != 0 && next >= h_lim {
            self.units[unit].count = 0;
            status |= STATUS_H_LIM_LAT | STATUS_ZERO_LAT;
            // ZERO_MODE 1 = increased from negative to 0 / wrapped through.
            status |= 1;
            event = true;
        }
        // Low-limit: count reached l_lim → wrap to 0 + L_LIM event.
        else if conf0 & CONF0_THR_L_LIM_EN != 0 && step < 0 && l_lim != 0 && next <= l_lim {
            self.units[unit].count = 0;
            status |= STATUS_L_LIM_LAT | STATUS_ZERO_LAT;
            event = true;
        } else {
            self.units[unit].count = next;
            // Threshold comparators: fire when the count equals the threshold.
            if conf0 & CONF0_THR_THRES0_EN != 0 && next == thres0 {
                status |= STATUS_THRES0_LAT;
                event = true;
            }
            if conf0 & CONF0_THR_THRES1_EN != 0 && next == thres1 {
                status |= STATUS_THRES1_LAT;
                event = true;
            }
            // Zero comparator: fire when the count lands on 0.
            if conf0 & CONF0_THR_ZERO_EN != 0 && next == 0 {
                status |= STATUS_ZERO_LAT;
                event = true;
            }
        }

        if event {
            self.units[unit].status = status;
            self.units[unit].int_raw = true;
        }
    }

    /// Current signed count for `unit` (test/inspection helper).
    pub fn count(&self, unit: usize) -> i16 {
        self.units.get(unit).map(|u| u.count).unwrap_or(0)
    }
}

impl Peripheral for Esp32s3Pcnt {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // Read-modify-write the enclosing word so byte-granular HAL writes
        // compose into a coherent 32-bit value (matches `Systimer`).
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset & !3, value);
        Ok(())
    }

    /// Level-sensitive IRQ delivery: while any masked status bit is set,
    /// emit the single PCNT source every tick so the bus aggregator keeps
    /// the CPU's pending bit asserted until firmware ACKs via INT_CLR
    /// (same model as the systimer peripheral). The counter itself has no
    /// internal edge source, so `tick()` never advances it — pulses arrive
    /// only via `inject_pulses`.
    fn tick(&mut self) -> PeripheralTickResult {
        if self.int_st_word() != 0 {
            PeripheralTickResult {
                explicit_irqs: Some(vec![self.source_id]),
                ..Default::default()
            }
        } else {
            PeripheralTickResult::default()
        }
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Some(((word >> byte_off) & 0xFF) as u8)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_defaults() {
        let p = Esp32s3Pcnt::new(PCNT_INTR_SOURCE);
        // CONF0 reset = 0x3C10 for every unit.
        for u in 0..NUM_UNITS {
            assert_eq!(p.read_word((u as u64) * 0x0C), CONF0_RESET);
        }
        // CTRL powers up with all four RST bits set, CLK_EN clear.
        assert_eq!(p.read_word(0x60), CTRL_RESET);
        // DATE version register.
        assert_eq!(p.read_word(0xFC), DATE_RESET);
        // No interrupts pending.
        assert_eq!(p.read_word(0x40), 0);
        assert_eq!(p.read_word(0x44), 0);
    }

    #[test]
    fn conf_registers_round_trip() {
        let mut p = Esp32s3Pcnt::new(PCNT_INTR_SOURCE);
        // Unit 2 CONFn live at 0x18 / 0x1C / 0x20.
        p.write_u32(0x18, 0xDEAD_BEEF).unwrap();
        p.write_u32(0x1C, 0x1234_5678).unwrap();
        p.write_u32(0x20, 0x0BAD_F00D).unwrap();
        assert_eq!(p.read_u32(0x18).unwrap(), 0xDEAD_BEEF);
        assert_eq!(p.read_u32(0x1C).unwrap(), 0x1234_5678);
        assert_eq!(p.read_u32(0x20).unwrap(), 0x0BAD_F00D);
        // Other units untouched.
        assert_eq!(p.read_u32(0x00).unwrap(), CONF0_RESET);

        // Byte-granular write composes into the word, too.
        p.write(0x00, 0x99).unwrap();
        assert_eq!(p.read(0x00).unwrap(), 0x99);
        assert_eq!(p.read_u32(0x00).unwrap(), (CONF0_RESET & !0xFF) | 0x99);
    }

    #[test]
    fn cnt_pause_round_trips() {
        let mut p = Esp32s3Pcnt::new(PCNT_INTR_SOURCE);
        // Clear all RST bits, set PAUSE for unit 1 (bit 3).
        p.write_u32(0x60, ctrl_pause_bit(1)).unwrap();
        assert_eq!(p.read_u32(0x60).unwrap(), ctrl_pause_bit(1));
    }

    #[test]
    fn cnt_rst_clears_count() {
        let mut p = Esp32s3Pcnt::new(PCNT_INTR_SOURCE);
        // Take unit 0 out of reset (clear all RST bits) and drive some pulses.
        p.write_u32(0x60, 0).unwrap();
        p.inject_pulses(0, 5);
        assert_eq!(p.count(0), 5);
        assert_eq!(p.read_u32(0x30).unwrap(), 5, "U0_CNT reads back live count");

        // Assert unit 0's RST bit (bit 0): count clears to 0.
        p.write_u32(0x60, ctrl_rst_bit(0)).unwrap();
        assert_eq!(p.count(0), 0);
        assert_eq!(
            p.read_u32(0x30).unwrap(),
            0,
            "CNT reads 0 while held in reset"
        );
        // Injection is ignored while held in reset.
        p.inject_pulses(0, 3);
        assert_eq!(p.count(0), 0);
    }

    #[test]
    fn inject_pulses_increments_when_running() {
        let mut p = Esp32s3Pcnt::new(PCNT_INTR_SOURCE);
        p.write_u32(0x60, 0).unwrap(); // release all units from reset
        p.inject_pulses(1, 10);
        assert_eq!(p.count(1), 10);
        // A paused unit ignores injection.
        p.write_u32(0x60, ctrl_pause_bit(1)).unwrap();
        p.inject_pulses(1, 4);
        assert_eq!(p.count(1), 10, "paused unit does not count");
    }

    #[test]
    fn threshold_event_sets_int_and_emits_source() {
        let mut p = Esp32s3Pcnt::new(PCNT_INTR_SOURCE);
        p.write_u32(0x60, 0).unwrap(); // release from reset
                                       // Unit 0 CONF1: THRES0 = 3 (bits[15:0]).
        p.write_u32(0x04, 3).unwrap();
        // CONF0: enable THRES0 comparator (keep defaults).
        p.write_u32(0x00, CONF0_RESET | CONF0_THR_THRES0_EN)
            .unwrap();
        // Enable IRQ delivery for unit 0.
        p.write_u32(0x48, 1).unwrap();

        // Before the threshold: no event, no IRQ emitted.
        p.inject_pulses(0, 2);
        assert_eq!(p.read_word(0x40), 0, "no event before threshold");
        assert!(p.tick().explicit_irqs.is_none());

        // Reaching count == 3 fires THRES0.
        p.inject_pulses(0, 1);
        assert_eq!(p.count(0), 3);
        assert_eq!(p.read_word(0x40) & 1, 1, "INT_RAW unit0 set");
        assert_eq!(p.read_word(0x44) & 1, 1, "INT_ST masked-set");
        // STATUS latched the THRES0 flag.
        assert_ne!(p.read_word(0x50) & STATUS_THRES0_LAT, 0);

        // tick() emits the single PCNT source while masked status is set.
        let r = p.tick();
        assert_eq!(r.explicit_irqs.as_deref(), Some(&[PCNT_INTR_SOURCE][..]));
    }

    #[test]
    fn high_limit_event_wraps_and_fires() {
        let mut p = Esp32s3Pcnt::new(PCNT_INTR_SOURCE);
        p.write_u32(0x60, 0).unwrap();
        // CONF2: H_LIM = 4 (bits[15:0]). H_LIM_EN is on by default.
        p.write_u32(0x08, 4).unwrap();
        p.write_u32(0x48, 1).unwrap(); // enable IRQ

        // Drive 4 pulses: on the 4th the counter hits the limit and wraps.
        p.inject_pulses(0, 4);
        assert_eq!(p.count(0), 0, "counter wrapped to 0 at high limit");
        assert_ne!(p.read_word(0x50) & STATUS_H_LIM_LAT, 0, "H_LIM latched");
        assert_eq!(p.read_word(0x40) & 1, 1, "INT_RAW set on limit event");
        let r = p.tick();
        assert_eq!(r.explicit_irqs.as_deref(), Some(&[PCNT_INTR_SOURCE][..]));
    }

    #[test]
    fn int_clr_is_w1c_and_stops_irq() {
        let mut p = Esp32s3Pcnt::new(PCNT_INTR_SOURCE);
        p.write_u32(0x60, 0).unwrap();
        p.write_u32(0x08, 3).unwrap(); // H_LIM = 3
        p.write_u32(0x48, 0xF).unwrap(); // enable all units' IRQ
        p.inject_pulses(0, 3); // fire H_LIM on unit 0
        assert_eq!(p.read_word(0x40) & 1, 1);
        assert!(!p.tick().explicit_irqs.as_deref().unwrap().is_empty());

        // INT_CLR is write-1-to-clear: clearing a *different* bit leaves
        // unit 0 pending.
        p.write_u32(0x4C, 0b0010).unwrap();
        assert_eq!(p.read_word(0x40) & 1, 1, "unrelated W1C bit is a no-op");

        // Clear unit 0's bit → INT_RAW/INT_ST drop and tick() stops emitting.
        p.write_u32(0x4C, 0b0001).unwrap();
        assert_eq!(p.read_word(0x40) & 1, 0, "INT_RAW cleared by W1C");
        assert_eq!(p.read_word(0x44) & 1, 0, "INT_ST cleared");
        assert!(p.tick().explicit_irqs.is_none(), "no IRQ after clear");
    }

    #[test]
    fn int_ena_gates_irq_but_not_raw() {
        let mut p = Esp32s3Pcnt::new(PCNT_INTR_SOURCE);
        p.write_u32(0x60, 0).unwrap();
        p.write_u32(0x08, 2).unwrap(); // H_LIM = 2
                                       // INT_ENA stays 0 — event fires but no IRQ delivered.
        p.inject_pulses(0, 2);
        assert_eq!(p.read_word(0x40) & 1, 1, "raw pending set without enable");
        assert_eq!(p.read_word(0x44) & 1, 0, "INT_ST masked off");
        assert!(p.tick().explicit_irqs.is_none(), "no IRQ while masked");
        // Enabling now exposes it via INT_ST and the IRQ path.
        p.write_u32(0x48, 1).unwrap();
        assert_eq!(p.read_word(0x44) & 1, 1);
        assert_eq!(
            p.tick().explicit_irqs.as_deref(),
            Some(&[PCNT_INTR_SOURCE][..])
        );
    }
}
