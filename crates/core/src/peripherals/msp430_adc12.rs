// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! MSP430 ADC12 (F1xx/F2xx, `msp430_adc12`) and ADC12_A (F5xx,
//! `msp430_adc12_a`) 12-bit SAR converters.
//!
//! Two sources:
//! - mspsim `se/sics/mspsim/core/ADC12.java` (github.com/contiki-ng/mspsim
//!   @c8169c98, BSD-3-Clause): the F1/F2 register windows (`ADC12MCTL0..15`
//!   bytes @0x080, `ADC12MEM0..15` words @0x140, `ADC12CTL0/CTL1/IFG/IE/IV`
//!   @0x1A0..0x1A8, mapped by `MSP430f1611Config.java`), the ENC lock (`CTL0` keeps [15:4], `CTL1`
//!   keeps all but `CONSEQ`, `MCTLx` frozen while `ENC` is set), the
//!   `CONSEQ` stepping (`EOS` returns to `CSTARTADD`, single sequence stops
//!   there, without `EOS` the pointer wraps 15 → 0), `BUSY` in `CTL1` bit 0,
//!   a `MEMx` read clearing `IFGx`, and the `IV` value `6 + 2x`.
//! - TI msp430-gcc support headers (via pftbest/msp430_svd @41cd91d3:
//!   msp430f149/f1611/f2617/f2619/f5437 `.h` and `_symbols.ld`): every
//!   address above for the four F1/F2 dies, the ADC12_A block at 0x0700
//!   (`CTL0` @0x00, `CTL1` @0x02, `CTL2` @0x04, `IFG` @0x0A, `IE` @0x0C,
//!   `IV` @0x0E, `MCTL` @0x10, `MEM` @0x20), the bit names (`ADC12SC`,
//!   `ENC`, `ADC12TOVIE`, `ADC12OVIE`, `ADC12ON`, `REFON`, `REF2_5V`, `MSC`,
//!   `SHP`, `ISSH`, `CONSEQ`, `CSTARTADD`, `EOS`, `SREF`, `INCH`), the F5
//!   `ADC12CTL2` bits (`ADC12DF` 0x08, `ADC12RES` [5:4] = 8/10/12 bits) and
//!   the `ADC12IV` encoding (0x02 overflow, 0x04 conversion-time overflow,
//!   0x06 + 2x for `ADC12IFGx`). msp430f5437.h has no `REFCTL0`, so on that
//!   (non-A) die `ADC12REFON`/`ADC12REF2_5V` drive the reference as on F1/F2.
//!
//! Modelled:
//! - Conversion start: `ADC12SC` with `SHS` = 0, `ADC12ON` and `ENC`. In
//!   pulse mode (`SHP` = 1) the conversion runs at the write and `ADC12SC`
//!   clears; in extended-sample mode (`SHP` = 0) sampling runs while
//!   `ADC12SC` is held and the conversion runs when firmware clears it.
//!   Conversions are instantaneous, inputs held at the trigger.
//! - `CONSEQ` 0 converts `MCTL[CSTARTADD]` once; 1 runs a sequence to the
//!   first `EOS`; 2 and 3 repeat the single channel / sequence once per tick
//!   while `ENC` stays set. With `MSC` = 0 each trigger converts ONE
//!   sequence step; with `MSC` = 1 one trigger runs the whole sequence.
//! - `N = full * (Vin - VR-) / (VR+ - VR-)`, truncated, clamped to
//!   0..full. `SREF` picks VR+ from AVCC (3.3 V), the internal reference
//!   (2.5 V with `REF2_5V`, else 1.5 V; 0 V while `REFON` is clear, so the
//!   code saturates) or VeREF+ (the level routed to INCH 8), and VR- from
//!   AVSS or VREF-/VeREF- (the level routed to INCH 9).
//! - `INCH` 11 converts (AVCC - AVSS) / 2, as do 12..15 on F1/F2; on
//!   ADC12_A 12..15 are the external A12..A15. Routable inputs are the
//!   external channels only: A0..A7, VeREF+ (8), VeREF- (9) and on ADC12_A
//!   A12..A15. An undriven input converts as 0 V.
//! - ADC12_A `ADC12RES` (8/10/12 bits; reset 12) and `ADC12DF` (signed,
//!   left aligned: the offset code shifted to bit 15 with the MSB flipped).
//! - `IFGx` rises per conversion and a `MEMx` read clears it. Converting into
//!   a `MEMx` whose `IFGx` is still set raises the overflow condition.
//!   `IV` reports overflow (0x02, with `ADC12OVIE`) ahead of the lowest
//!   enabled `IFGx` (0x06 + 2x); any `IV` access clears the overflow it
//!   reports. The IRQ line is `(overflow && OVIE) || IFG & IE`.
//!
//! Not modelled: conversion time and the `SHT`/`ADC12DIV`/`ADC12SSEL`
//! clocking (stored), timer triggers (`SHS` != 0 never converts), the
//! conversion-time overflow (`TOV`, never set), the temperature sensor
//! (`INCH` 10 converts 0 V), and the ADC12_A `ADC12SR`/`ADC12PDIV`/
//! `ADC12REFOUT`/`ADC12REFBURST` bits (stored).

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::Cell;

const CHANNELS: usize = 16;
const AVCC_MV: u32 = 3300;

// ADC12CTL0
const SC: u16 = 1 << 0;
const ENC: u16 = 1 << 1;
const OVIE: u16 = 1 << 3;
const ADC12ON: u16 = 1 << 4;
const REFON: u16 = 1 << 5;
const REF2_5V: u16 = 1 << 6;
const MSC: u16 = 1 << 7;
/// Bits that stay writable while `ENC` is set (mspsim: `& 0xf` merge).
const CTL0_ENC_WRITABLE: u16 = 0x000F;

// ADC12CTL1
const BUSY: u16 = 1 << 0;
const CONSEQ_MASK: u16 = 0x0006;
const SHP: u16 = 1 << 9;
/// `SHS`: 0 = `ADC12SC`; timer sources never trigger here.
const SHS_MASK: u16 = 0x0C00;
/// `CONSEQ` is the only `CTL1` field writable while `ENC` is set.
const CTL1_ENC_WRITABLE: u16 = CONSEQ_MASK;

// ADC12CTL2 (ADC12_A only): reset value ADC12RES_2 (12 bits).
const CTL2_RESET: u16 = 0x0020;
const CTL2_MASK: u16 = 0x01BF;
const DF: u16 = 1 << 3;

// ADC12MCTLx
const EOS: u8 = 1 << 7;

const VEREF_PLUS: usize = 8;
const VEREF_MINUS: usize = 9;
const TEMP_SENSOR: usize = 10;
const HALF_AVCC: usize = 11;

/// Which register map the block uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Adc12Layout {
    /// F1xx/F2xx: the model's window starts at `ADC12MCTL0` (0x0080) and
    /// runs to `ADC12IV` (0x01A9); the gaps belong to other peripherals,
    /// which win their addresses on the bus because they start later.
    Adc12,
    /// F5xx ADC12_A: one contiguous block (0x0700 on the F543x).
    Adc12A,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reg {
    Ctl0,
    Ctl1,
    Ctl2,
    Ifg,
    Ie,
    Iv,
    Mctl(usize),
    Mem(usize),
}

impl Adc12Layout {
    /// Bytes the chip descriptor must map from the base address.
    pub fn window_size(self) -> u64 {
        match self {
            Self::Adc12 => 0x12A,
            Self::Adc12A => 0x40,
        }
    }

    fn decode(self, offset: u64) -> Option<Reg> {
        match self {
            Self::Adc12 => match offset {
                0x000..=0x00F => Some(Reg::Mctl(offset as usize)),
                0x0C0..=0x0DF => Some(Reg::Mem(((offset - 0x0C0) / 2) as usize)),
                0x120 | 0x121 => Some(Reg::Ctl0),
                0x122 | 0x123 => Some(Reg::Ctl1),
                0x124 | 0x125 => Some(Reg::Ifg),
                0x126 | 0x127 => Some(Reg::Ie),
                0x128 | 0x129 => Some(Reg::Iv),
                _ => None,
            },
            Self::Adc12A => match offset {
                0x00 | 0x01 => Some(Reg::Ctl0),
                0x02 | 0x03 => Some(Reg::Ctl1),
                0x04 | 0x05 => Some(Reg::Ctl2),
                0x0A | 0x0B => Some(Reg::Ifg),
                0x0C | 0x0D => Some(Reg::Ie),
                0x0E | 0x0F => Some(Reg::Iv),
                0x10..=0x1F => Some(Reg::Mctl((offset - 0x10) as usize)),
                0x20..=0x3F => Some(Reg::Mem(((offset - 0x20) / 2) as usize)),
                _ => None,
            },
        }
    }

    /// Whether `INCH` = `channel` is an external pin a netlist can drive.
    fn is_external(self, channel: usize) -> bool {
        match channel {
            0..=9 => true,
            12..=15 => self == Self::Adc12A,
            _ => false,
        }
    }
}

/// MSP430 ADC12 / ADC12_A.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Msp430Adc12 {
    layout: Adc12Layout,
    ctl0: u16,
    /// `CTL1` without the read-only `BUSY` bit.
    ctl1: u16,
    ctl2: u16,
    ifg: Cell<u16>,
    ie: u16,
    overflow: Cell<bool>,
    mctl: [u8; CHANNELS],
    mem: [u16; CHANNELS],
    /// Next `MCTL` index of the running sequence.
    pos: usize,
    /// A sequence or repeat run is armed (`BUSY`).
    active: bool,
    /// Extended-sample mode: `ADC12SC` is held high and sampling.
    sampling: bool,
    /// Routed levels in millivolts by `INCH`. `u16::MAX` means undriven.
    inputs_mv: [u16; CHANNELS],
}

impl Msp430Adc12 {
    pub fn new(layout: Adc12Layout) -> Self {
        Self {
            layout,
            ctl0: 0,
            ctl1: 0,
            ctl2: CTL2_RESET,
            ifg: Cell::new(0),
            ie: 0,
            overflow: Cell::new(false),
            mctl: [0; CHANNELS],
            mem: [0; CHANNELS],
            pos: 0,
            active: false,
            sampling: false,
            inputs_mv: [u16::MAX; CHANNELS],
        }
    }

    fn reset_registers(&mut self) {
        let inputs = self.inputs_mv;
        let mem = self.mem;
        *self = Self::new(self.layout);
        // ADC12MEMx is "unchanged" across reset (mspsim, TI register tables).
        self.mem = mem;
        self.inputs_mv = inputs;
    }

    fn conseq(&self) -> u16 {
        (self.ctl1 & CONSEQ_MASK) >> 1
    }

    fn cstartadd(&self) -> usize {
        usize::from(self.ctl1 >> 12)
    }

    fn enabled(&self) -> bool {
        self.ctl0 & (ENC | ADC12ON) == ENC | ADC12ON
    }

    fn level_mv(&self, channel: usize) -> u32 {
        match self.inputs_mv[channel] {
            u16::MAX => 0,
            mv => u32::from(mv),
        }
    }

    fn input_mv(&self, inch: usize) -> u32 {
        match inch {
            TEMP_SENSOR => 0,
            HALF_AVCC => AVCC_MV / 2,
            12..=15 if self.layout == Adc12Layout::Adc12 => AVCC_MV / 2,
            _ => self.level_mv(inch),
        }
    }

    /// `(VR+, VR-)` in millivolts for `SREF` (TI `SREF_0..7`).
    fn references_mv(&self, sref: u8) -> (u32, u32) {
        let internal = match (self.ctl0 & REFON != 0, self.ctl0 & REF2_5V != 0) {
            (false, _) => 0,
            (true, true) => 2500,
            (true, false) => 1500,
        };
        let positive = match sref & 3 {
            0 => AVCC_MV,
            1 => internal,
            _ => self.level_mv(VEREF_PLUS),
        };
        let negative = if sref & 4 != 0 {
            self.level_mv(VEREF_MINUS)
        } else {
            0
        };
        (positive, negative)
    }

    fn resolution_bits(&self) -> u32 {
        match self.layout {
            Adc12Layout::Adc12 => 12,
            Adc12Layout::Adc12A => match (self.ctl2 >> 4) & 3 {
                0 => 8,
                1 => 10,
                _ => 12,
            },
        }
    }

    /// One conversion of `MCTL[index]` into `MEM[index]`.
    fn convert_one(&mut self, index: usize) {
        let mctl = self.mctl[index];
        let vin = self.input_mv(usize::from(mctl & 0x0F));
        let (vr_pos, vr_neg) = self.references_mv((mctl >> 4) & 7);
        let bits = self.resolution_bits();
        let full = (1u32 << bits) - 1;
        let code = if vin <= vr_neg {
            0
        } else if vin >= vr_pos {
            full
        } else {
            (vin - vr_neg) * full / (vr_pos - vr_neg)
        };
        let result = if self.layout == Adc12Layout::Adc12A && self.ctl2 & DF != 0 {
            ((code << (16 - bits)) as u16) ^ 0x8000
        } else {
            code as u16
        };
        let ifg = self.ifg.get();
        if ifg & (1 << index) != 0 {
            self.overflow.set(true);
        }
        self.mem[index] = result;
        self.ifg.set(ifg | (1 << index));
    }

    /// Convert from `pos`; returns whether the step ended a sequence.
    fn step(&mut self) -> bool {
        let index = self.pos;
        self.convert_one(index);
        if self.conseq() & 1 == 0 {
            self.pos = self.cstartadd();
            return true;
        }
        if self.mctl[index] & EOS != 0 {
            self.pos = self.cstartadd();
            true
        } else {
            self.pos = (index + 1) % CHANNELS;
            false
        }
    }

    /// Run one pass: the whole sequence with `MSC` (bounded to 16 steps when
    /// no `EOS` is set), else one step. Returns whether the pass ended the
    /// sequence (`EOS`, or any single-channel conversion).
    fn run_pass(&mut self) -> bool {
        let whole = self.conseq() & 1 != 0 && self.ctl0 & MSC != 0;
        for _ in 0..CHANNELS {
            let ended = self.step();
            if ended || !whole {
                return ended;
            }
        }
        false
    }

    /// One sample-and-hold trigger.
    fn trigger(&mut self) {
        if !self.enabled() || self.ctl1 & SHS_MASK != 0 {
            return;
        }
        if !self.active {
            self.pos = self.cstartadd();
        }
        let ended = self.run_pass();
        // A single conversion, or a single sequence that reached EOS, is
        // done. Repeat modes stay armed (each tick with MSC, else each
        // trigger); an unfinished MSC = 0 sequence waits for its next
        // trigger.
        self.active = match self.conseq() {
            0 => false,
            1 => !ended,
            _ => true,
        };
    }

    fn write_ctl0(&mut self, value: u16) {
        let was_enc = self.ctl0 & ENC != 0;
        let old_sc = self.ctl0 & SC != 0;
        self.ctl0 = if was_enc {
            (self.ctl0 & !CTL0_ENC_WRITABLE) | (value & CTL0_ENC_WRITABLE)
        } else {
            value
        };
        if self.ctl0 & ENC == 0 {
            // Clearing ENC ends repeat runs and abandons a waiting sequence.
            self.active = false;
        }
        let new_sc = self.ctl0 & SC != 0;
        if self.ctl1 & SHP != 0 {
            if new_sc {
                self.trigger();
                self.ctl0 &= !SC;
            }
            self.sampling = false;
        } else if new_sc && !old_sc {
            self.sampling = self.enabled();
        } else if !new_sc && old_sc && self.sampling {
            self.sampling = false;
            self.trigger();
        }
    }

    fn write_ctl1(&mut self, value: u16) {
        let value = value & !BUSY;
        self.ctl1 = if self.ctl0 & ENC != 0 {
            (self.ctl1 & !CTL1_ENC_WRITABLE) | (value & CTL1_ENC_WRITABLE)
        } else {
            value
        };
        if self.conseq() == 0 {
            self.active = false;
        }
    }

    /// `ADC12IV`: overflow first, then the lowest enabled `IFGx`.
    fn vector(&self) -> u16 {
        if self.overflow.get() && self.ctl0 & OVIE != 0 {
            return 0x02;
        }
        let pending = self.ifg.get() & self.ie;
        if pending == 0 {
            0
        } else {
            6 + 2 * pending.trailing_zeros() as u16
        }
    }

    /// Any `IV` access clears the overflow condition when it is the one
    /// reported.
    fn touch_iv(&self) {
        if self.vector() == 0x02 {
            self.overflow.set(false);
        }
    }

    fn read_reg(&self, reg: Reg) -> u16 {
        match reg {
            Reg::Ctl0 => self.ctl0,
            Reg::Ctl1 => {
                self.ctl1
                    | if self.active || self.sampling {
                        BUSY
                    } else {
                        0
                    }
            }
            Reg::Ctl2 => self.ctl2,
            Reg::Ifg => self.ifg.get(),
            Reg::Ie => self.ie,
            Reg::Iv => self.vector(),
            Reg::Mctl(i) => u16::from(self.mctl[i]),
            Reg::Mem(i) => self.mem[i],
        }
    }

    fn after_read(&self, reg: Reg) {
        match reg {
            Reg::Mem(i) => self.ifg.set(self.ifg.get() & !(1 << i)),
            Reg::Iv => self.touch_iv(),
            _ => {}
        }
    }

    fn write_reg(&mut self, reg: Reg, value: u16) {
        match reg {
            Reg::Ctl0 => self.write_ctl0(value),
            Reg::Ctl1 => self.write_ctl1(value),
            Reg::Ctl2 => self.ctl2 = value & CTL2_MASK,
            Reg::Ifg => self.ifg.set(value),
            Reg::Ie => self.ie = value,
            Reg::Iv => self.touch_iv(),
            Reg::Mctl(i) => {
                if self.ctl0 & ENC == 0 {
                    self.mctl[i] = value as u8;
                }
            }
            Reg::Mem(_) => {}
        }
    }

    fn irq_asserted(&self) -> bool {
        (self.overflow.get() && self.ctl0 & OVIE != 0) || self.ifg.get() & self.ie != 0
    }

    /// Byte lane of `offset` inside its 16-bit register (0 for MCTL bytes).
    fn lane(&self, reg: Reg, offset: u64) -> u32 {
        match reg {
            Reg::Mctl(_) => 0,
            _ => (offset & 1) as u32,
        }
    }
}

impl Peripheral for Msp430Adc12 {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        let channel = usize::from(channel);
        if channel >= CHANNELS || !self.layout.is_external(channel) {
            return false;
        }
        self.inputs_mv[channel] = millivolts;
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        let channel = usize::from(channel);
        if channel >= CHANNELS || !self.layout.is_external(channel) {
            return false;
        }
        self.inputs_mv[channel] = u16::MAX;
        true
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_asserted())
    }

    fn on_system_reset(&mut self) {
        self.reset_registers();
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let Some(reg) = self.layout.decode(offset) else {
            return Ok(0);
        };
        let value = (self.read_reg(reg) >> (8 * self.lane(reg, offset))) as u8;
        self.after_read(reg);
        Ok(value)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let Some(reg) = self.layout.decode(offset) else {
            return Ok(());
        };
        let shift = 8 * self.lane(reg, offset);
        let current = match reg {
            // A byte store to IV or a MEM word has no merge to preserve.
            Reg::Iv | Reg::Mem(_) => 0,
            Reg::Ctl1 => self.ctl1,
            _ => self.read_reg(reg),
        };
        // Merging a byte must not re-assert a self-cleared ADC12SC.
        let current = if reg == Reg::Ctl0 && shift == 8 {
            current & !SC
        } else {
            current
        };
        let next = (current & !(0xFF << shift)) | (u16::from(value) << shift);
        self.write_reg(reg, next);
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        let Some(reg) = self.layout.decode(offset) else {
            return Ok(0);
        };
        if let Reg::Mctl(_) = reg {
            return Ok(u16::from(self.read(offset)?) | (u16::from(self.read(offset + 1)?) << 8));
        }
        let value = self.read_reg(reg);
        self.after_read(reg);
        Ok(value)
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        let Some(reg) = self.layout.decode(offset) else {
            return Ok(());
        };
        if let Reg::Mctl(_) = reg {
            self.write(offset, value as u8)?;
            return self.write(offset + 1, (value >> 8) as u8);
        }
        self.write_reg(reg, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Repeat modes with MSC convert again every tick while ENC holds.
        if self.active && self.conseq() >= 2 && self.ctl0 & MSC != 0 && self.enabled() {
            self.run_pass();
        }
        PeripheralTickResult {
            irq: self.irq_asserted(),
            ..PeripheralTickResult::default()
        }
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

    // F1/F2 offsets from the 0x0080 base.
    const CTL0: u64 = 0x120;
    const CTL1: u64 = 0x122;
    const IFG: u64 = 0x124;
    const IE: u64 = 0x126;
    const IV: u64 = 0x128;
    const fn mem(x: u64) -> u64 {
        0x0C0 + 2 * x
    }

    /// `4095 * Vin / VR`, truncated (SREF with VR- = AVSS).
    fn code(mv: u32, vr_mv: u32) -> u16 {
        (mv * 4095 / vr_mv) as u16
    }

    fn adc12(inputs: &[(u8, u16)]) -> Msp430Adc12 {
        let mut adc = Msp430Adc12::new(Adc12Layout::Adc12);
        for &(channel, mv) in inputs {
            assert!(adc.set_adc_channel_input(channel, mv));
        }
        adc
    }

    /// The SLAU049 single-conversion example: `ADC12ON + SHT0_2`, `SHP`,
    /// `MCTLx = INCH`, then `ENC + ADC12SC`.
    fn single(adc: &mut Msp430Adc12, mctl: u8, start: u16) -> u16 {
        adc.write_u16(CTL0, 0).unwrap();
        adc.write_u16(CTL0, 0x0200 | ADC12ON | REFON | REF2_5V)
            .unwrap();
        adc.write_u16(CTL1, (start << 12) | SHP).unwrap();
        adc.write(u64::from(start), mctl).unwrap();
        adc.write_u16(CTL0, adc.ctl0 | ENC | SC).unwrap();
        adc.read_u16(mem(u64::from(start))).unwrap()
    }

    #[test]
    fn transfer_function_and_reference_selection() {
        let mut adc = adc12(&[(0, 1650), (1, 3300), (2, 1000), (8, 2000), (9, 500)]);
        assert_eq!(single(&mut adc, 0, 0), code(1650, 3300), "SREF_0: AVCC");
        assert_eq!(single(&mut adc, 1, 0), 4095, "AVCC is full scale");
        // SREF_1 with REF2_5V: 1000 mV against 2.5 V; 1650 mV against it too.
        assert_eq!(single(&mut adc, 0x12, 0), code(1000, 2500));
        // SREF_2: VeREF+ = the level routed to INCH 8.
        assert_eq!(single(&mut adc, 0x22, 0), code(1000, 2000));
        // SREF_6: VeREF+ / VeREF-: (1000 - 500) / (2000 - 500).
        assert_eq!(single(&mut adc, 0x62, 0), code(500, 1500));
        // Below VR- clamps to zero.
        assert!(adc.set_adc_channel_input(2, 400));
        assert_eq!(single(&mut adc, 0x62, 0), 0);
        // INCH 11 and the F1/F2 12..15: (AVCC - AVSS) / 2.
        for inch in 11..16 {
            assert_eq!(single(&mut adc, inch, 3), code(1650, 3300), "INCH {inch}");
        }
        assert_eq!(
            single(&mut adc, 10, 3),
            0,
            "temperature sensor not modelled"
        );
        // An undriven pin converts as 0 V.
        assert_eq!(single(&mut adc, 5, 0), 0);
    }

    #[test]
    fn internal_reference_follows_refon_and_ref2_5v() {
        let mut adc = adc12(&[(0, 1000)]);
        adc.write_u16(CTL0, ADC12ON | REFON).unwrap();
        adc.write_u16(CTL1, SHP).unwrap();
        adc.write(0, 0x10).unwrap();
        adc.write_u16(CTL0, ADC12ON | REFON | ENC | SC).unwrap();
        assert_eq!(adc.read_u16(mem(0)).unwrap(), code(1000, 1500), "1.5 V");
        adc.write_u16(CTL0, 0).unwrap();
        adc.write_u16(CTL0, ADC12ON | ENC | SC).unwrap();
        assert_eq!(
            adc.read_u16(mem(0)).unwrap(),
            4095,
            "REFON clear: VREF+ collapses"
        );
    }

    #[test]
    fn start_needs_adc12on_enc_and_the_sc_source() {
        let mut adc = adc12(&[(0, 1000)]);
        adc.write_u16(CTL1, SHP).unwrap();
        adc.write_u16(CTL0, ENC | SC).unwrap();
        assert_eq!(adc.read_u16(IFG).unwrap(), 0, "ADC12ON clear");
        adc.write_u16(CTL0, ADC12ON | SC).unwrap();
        assert_eq!(adc.read_u16(IFG).unwrap(), 0, "ENC clear");
        // SHS = 1 (Timer_A.OUT1): ADC12SC does not trigger.
        adc.write_u16(CTL0, 0).unwrap();
        adc.write_u16(CTL1, SHP | 0x0400).unwrap();
        adc.write_u16(CTL0, ADC12ON | ENC | SC).unwrap();
        assert_eq!(adc.read_u16(IFG).unwrap(), 0, "timer trigger source");
        adc.write_u16(CTL0, ADC12ON).unwrap();
        adc.write_u16(CTL1, SHP).unwrap();
        adc.write_u16(CTL0, ADC12ON | ENC | SC).unwrap();
        assert_eq!(adc.read_u16(IFG).unwrap(), 1);
        assert_eq!(adc.read_u16(CTL0).unwrap() & SC, 0, "pulse mode clears SC");
        assert_eq!(adc.read_u16(CTL1).unwrap() & BUSY, 0, "single is done");
    }

    #[test]
    fn extended_sample_mode_converts_when_sc_drops() {
        let mut adc = adc12(&[(0, 1200)]);
        adc.write_u16(CTL0, ADC12ON).unwrap();
        adc.write_u16(CTL1, 0).unwrap(); // SHP = 0
        adc.write_u16(CTL0, ADC12ON | ENC | SC).unwrap();
        assert_eq!(adc.read_u16(IFG).unwrap(), 0, "still sampling");
        assert_ne!(adc.read_u16(CTL1).unwrap() & BUSY, 0);
        assert!(adc.set_adc_channel_input(0, 900));
        adc.write_u16(CTL0, ADC12ON | ENC).unwrap();
        assert_eq!(adc.read_u16(IFG).unwrap(), 1);
        assert_eq!(
            adc.read_u16(mem(0)).unwrap(),
            code(900, 3300),
            "held at SC fall"
        );
        assert_eq!(adc.read_u16(CTL1).unwrap() & BUSY, 0);
    }

    #[test]
    fn sequence_with_msc_runs_to_eos_and_without_msc_steps_per_trigger() {
        let mut adc = adc12(&[(1, 330), (2, 660), (3, 990), (4, 1320)]);
        adc.write_u16(CTL0, ADC12ON | MSC).unwrap();
        // CSTARTADD 1, CONSEQ_1. MCTL1..3 = A1..A3, EOS on MCTL3.
        adc.write_u16(CTL1, (1 << 12) | SHP | 0x0002).unwrap();
        for (i, inch) in [(1, 1), (2, 2), (3, 3 | EOS)] {
            adc.write(i, inch).unwrap();
        }
        adc.write(4, 0).unwrap();
        adc.write_u16(CTL0, ADC12ON | MSC | ENC | SC).unwrap();
        assert_eq!(adc.read_u16(IFG).unwrap(), 0b1110, "MEM1..3, not MEM4");
        for x in 1..=3u64 {
            assert_eq!(adc.read_u16(mem(x)).unwrap(), code(330 * x as u32, 3300));
        }
        assert_eq!(adc.read_u16(IFG).unwrap(), 0, "MEMx reads clear IFGx");
        assert_eq!(adc.read_u16(CTL1).unwrap() & BUSY, 0, "sequence done");

        // MSC = 0: one step per trigger, BUSY until EOS.
        adc.write_u16(CTL0, ADC12ON).unwrap();
        adc.write_u16(CTL0, ADC12ON | ENC | SC).unwrap();
        assert_eq!(adc.read_u16(IFG).unwrap(), 0b0010);
        assert_ne!(adc.read_u16(CTL1).unwrap() & BUSY, 0);
        adc.write_u16(CTL0, ADC12ON | ENC | SC).unwrap();
        assert_eq!(adc.read_u16(IFG).unwrap(), 0b0110);
        adc.write_u16(CTL0, ADC12ON | ENC | SC).unwrap();
        assert_eq!(adc.read_u16(IFG).unwrap(), 0b1110);
        assert_eq!(adc.read_u16(CTL1).unwrap() & BUSY, 0);
        // The next trigger starts over at CSTARTADD.
        adc.write_u16(IFG, 0).unwrap();
        adc.write_u16(CTL0, ADC12ON | ENC | SC).unwrap();
        assert_eq!(adc.read_u16(IFG).unwrap(), 0b0010);
    }

    #[test]
    fn sequence_without_eos_wraps_and_stops_after_sixteen() {
        let mut adc = adc12(&[]);
        adc.write_u16(CTL0, ADC12ON | MSC).unwrap();
        adc.write_u16(CTL1, (14 << 12) | SHP | 0x0002).unwrap();
        adc.write_u16(CTL0, ADC12ON | MSC | ENC | SC).unwrap();
        assert_eq!(adc.read_u16(IFG).unwrap(), 0xFFFF, "15 -> 0 wrap");
    }

    #[test]
    fn repeat_modes_reconvert_each_tick_until_enc_clears() {
        let mut adc = adc12(&[(4, 1000)]);
        adc.write_u16(CTL0, ADC12ON | MSC).unwrap();
        adc.write_u16(CTL1, (2 << 12) | SHP | 0x0004).unwrap(); // CONSEQ_2
        adc.write(2, 4).unwrap();
        adc.write_u16(CTL0, ADC12ON | MSC | ENC | SC).unwrap();
        assert_eq!(adc.read_u16(mem(2)).unwrap(), code(1000, 3300));
        assert!(adc.set_adc_channel_input(4, 2000));
        adc.tick();
        assert_eq!(adc.read_u16(mem(2)).unwrap(), code(2000, 3300), "repeat");
        assert_ne!(adc.read_u16(CTL1).unwrap() & BUSY, 0);
        adc.write_u16(CTL0, ADC12ON | MSC).unwrap();
        assert!(adc.set_adc_channel_input(4, 3000));
        adc.tick();
        assert_eq!(
            adc.read_u16(mem(2)).unwrap(),
            code(2000, 3300),
            "ENC stops it"
        );
        assert_eq!(adc.read_u16(CTL1).unwrap() & BUSY, 0);
    }

    #[test]
    fn enc_locks_control_and_memory_control() {
        let mut adc = adc12(&[]);
        adc.write_u16(CTL0, 0x1200 | ADC12ON).unwrap();
        adc.write_u16(CTL1, (3 << 12) | SHP).unwrap();
        adc.write(3, 0x05).unwrap();
        adc.write_u16(CTL0, 0x1200 | ADC12ON | ENC).unwrap();
        adc.write_u16(CTL0, 0xF000 | ENC | OVIE).unwrap();
        assert_eq!(adc.read_u16(CTL0).unwrap(), 0x1200 | ADC12ON | ENC | OVIE);
        adc.write_u16(CTL1, (9 << 12) | 0x0006).unwrap();
        assert_eq!(
            adc.read_u16(CTL1).unwrap(),
            (3 << 12) | SHP | 0x0006,
            "only CONSEQ"
        );
        adc.write(3, 0x07).unwrap();
        assert_eq!(adc.read(3).unwrap(), 0x05, "MCTLx frozen");
        adc.write_u16(CTL0, 0).unwrap();
        adc.write(3, 0x87).unwrap();
        assert_eq!(adc.read(3).unwrap(), 0x87);
    }

    #[test]
    fn interrupt_vector_and_overflow() {
        let mut adc = adc12(&[(0, 100)]);
        single(&mut adc, 0, 2);
        // `single` read MEM2; convert again without reading.
        adc.write_u16(CTL0, adc.ctl0 | SC).unwrap();
        assert_eq!(adc.read_u16(IFG).unwrap(), 1 << 2);
        assert!(!adc.irq_asserted(), "no IE");
        assert_eq!(adc.read_u16(IV).unwrap(), 0);
        adc.write_u16(IE, 1 << 2).unwrap();
        assert!(adc.tick().irq);
        assert_eq!(adc.read_u16(IV).unwrap(), 0x06 + 2 * 2);
        assert_eq!(adc.read_u16(IFG).unwrap(), 1 << 2, "IV read keeps IFGx");
        // A second conversion into the unread MEM2 overflows.
        adc.write_u16(CTL0, adc.ctl0 | SC).unwrap();
        assert!(adc.overflow.get());
        assert_eq!(adc.read_u16(IV).unwrap(), 0x0A, "OVIE clear: IFG2 reported");
        adc.write_u16(CTL0, 0).unwrap();
        adc.write_u16(CTL0, 0x0200 | ADC12ON | REFON | REF2_5V | OVIE)
            .unwrap();
        assert_eq!(adc.read_u16(IV).unwrap(), 0x02, "overflow first");
        assert_eq!(adc.read_u16(IV).unwrap(), 0x0A, "the IV access cleared it");
        adc.read_u16(mem(2)).unwrap();
        assert_eq!(adc.read_u16(IV).unwrap(), 0);
        assert!(!adc.irq_asserted());
    }

    #[test]
    fn byte_access_merges_without_reasserting_sc() {
        let mut adc = adc12(&[(0, 1650)]);
        adc.write_u16(CTL1, SHP).unwrap();
        adc.write(CTL0, (ADC12ON | ENC | SC) as u8).unwrap();
        assert_eq!(adc.read(IFG).unwrap(), 1, "low-byte start");
        assert_eq!(adc.read(mem(0)).unwrap(), (2047u16 & 0xFF) as u8);
        adc.write(CTL0 + 1, 0x00).unwrap();
        assert_eq!(
            adc.read(IFG).unwrap(),
            0,
            "high-byte store does not restart"
        );
    }

    #[test]
    fn routable_inputs_are_external_pins_only() {
        let mut adc = Msp430Adc12::new(Adc12Layout::Adc12);
        for channel in 0..=9 {
            assert!(adc.set_adc_channel_input(channel, 1));
        }
        for channel in 10..=16 {
            assert!(!adc.set_adc_channel_input(channel, 1), "INCH {channel}");
        }
        let mut adc = Msp430Adc12::new(Adc12Layout::Adc12A);
        for channel in [0, 7, 8, 9, 12, 15] {
            assert!(adc.set_adc_channel_input(channel, 1));
        }
        assert!(!adc.set_adc_channel_input(10, 1));
        assert!(!adc.set_adc_channel_input(11, 1));
        assert_eq!(adc.adc_channel_count(), Some(16));
    }

    // ADC12_A offsets from the 0x0700 base.
    const A_CTL0: u64 = 0x00;
    const A_CTL1: u64 = 0x02;
    const A_CTL2: u64 = 0x04;
    const A_IFG: u64 = 0x0A;
    const A_IV: u64 = 0x0E;
    const A_MCTL: u64 = 0x10;
    const fn a_mem(x: u64) -> u64 {
        0x20 + 2 * x
    }

    #[test]
    fn adc12_a_map_resolution_and_signed_format() {
        let mut adc = Msp430Adc12::new(Adc12Layout::Adc12A);
        assert!(adc.set_adc_channel_input(12, 1650));
        assert_eq!(adc.read_u16(A_CTL2).unwrap(), 0x0020, "ADC12RES_2 at reset");
        let convert = |adc: &mut Msp430Adc12, ctl2: u16| {
            adc.write_u16(A_CTL0, ADC12ON).unwrap();
            adc.write_u16(A_CTL2, ctl2).unwrap();
            adc.write_u16(A_CTL1, (5 << 12) | SHP).unwrap();
            adc.write(A_MCTL + 5, 12).unwrap();
            adc.write_u16(A_CTL0, ADC12ON | ENC | SC).unwrap();
            assert_eq!(adc.read_u16(A_IFG).unwrap(), 1 << 5);
            assert_eq!(adc.read_u16(A_IV).unwrap(), 0, "IE clear");
            adc.read_u16(a_mem(5)).unwrap()
        };
        assert_eq!(convert(&mut adc, 0x0020), 2047, "12 bits, A12 external");
        assert_eq!(convert(&mut adc, 0x0010), 511, "10 bits");
        assert_eq!(convert(&mut adc, 0x0000), 127, "8 bits");
        // ADC12DF: 2047 << 4 = 0x7FF0, MSB flipped = 0xFFF0 (-16).
        assert_eq!(convert(&mut adc, 0x0028), 0xFFF0);
        assert!(adc.set_adc_channel_input(12, 3300));
        assert_eq!(convert(&mut adc, 0x0028), 0x7FF0, "full scale signed");
        assert!(adc.set_adc_channel_input(12, 0));
        assert_eq!(convert(&mut adc, 0x0028), 0x8000, "zero signed");
        assert_eq!(adc.read_u16(A_IFG).unwrap(), 0);
        assert_eq!(Adc12Layout::Adc12A.window_size(), 0x40);
    }

    #[test]
    fn reset_clears_control_but_keeps_results() {
        let mut adc = adc12(&[(0, 1000)]);
        let held = single(&mut adc, 0, 4);
        adc.write_u16(IE, 0xFFFF).unwrap();
        adc.on_system_reset();
        assert_eq!(adc.read_u16(CTL0).unwrap(), 0);
        assert_eq!(adc.read_u16(CTL1).unwrap(), 0);
        assert_eq!(adc.read_u16(IE).unwrap(), 0);
        assert_eq!(adc.read(4).unwrap(), 0, "MCTLx reset");
        assert_eq!(adc.read_u16(mem(4)).unwrap(), held, "MEMx unchanged");
        assert_eq!(
            single(&mut adc, 0, 0),
            code(1000, 3300),
            "inputs stay routed"
        );
    }
}
