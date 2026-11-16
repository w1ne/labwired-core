// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! ATmega640/1280/1281/2560/2561 ADC converter (Microchip DS2549, "ADC").
//! The input voltages come from the bus-side `avr_adc` model
//! (`peripherals/avr_adc.rs`); this is the converter and its registers.
//!
//! | Register | Addr | Bits |
//! |----------|------|------|
//! | `ADCL`/`ADCH` | 0x78/0x79 | result, right- or left-adjusted (ADLAR) |
//! | `ADCSRA` | 0x7A | ADEN ADSC ADATE ADIF ADIE ADPS2:0 |
//! | `ADCSRB` | 0x7B | ACME, MUX5, ADTS2:0 |
//! | `ADMUX`  | 0x7C | REFS1:0, ADLAR, MUX4:0 |
//!
//! Timing (section "Prescaling and Conversion Timing", Table 26-1): the ADC
//! clock is the CPU clock over the ADPS factor (2, 2, 4, 8, 16, 32, 64, 128).
//! The prescaler runs from the moment ADEN is set; a conversion started by
//! ADSC begins at the following ADC clock edge and takes 13 ADC clocks, the
//! first one after ADEN 25. An auto-triggered conversion resets the prescaler
//! at the trigger and takes 13.5 clocks. When it completes the result is
//! written, ADIF is set and, in single conversion mode, ADSC clears.
//!
//! Auto triggering (ADATE, ADTS Table 26-6): 000 free running (a new
//! conversion starts as each completes, ADSC stays one), 010 INTF0, 011 OCF0A,
//! 100 TOV0, 101 OCF1B, 110 TOV1, 111 ICF1. A conversion starts on the rising
//! edge of the selected flag; an edge during a conversion is ignored, and the
//! flag must be cleared before it can trigger again. 001 (analog comparator)
//! never triggers: the comparator is not modelled.
//!
//! Channel and reference (Tables 26-3, 26-4): MUX5 (ADCSRB bit 3) with
//! MUX4:0 selects ADC0..7 (0x00..0x07) or ADC8..15 (0x20..0x27); 0x1E is the
//! 1.1 V bandgap, 0x1F is GND; differential and gain codes read 0. REFS1:0 =
//! 01 AVCC, 10 internal 1.1 V, 11 internal 2.56 V; 00 (AREF) is taken as AVCC,
//! as AREF is not a modelled pin. Selection and input are latched when a
//! conversion starts (the sample-and-hold point is not resolved further).
//!
//! Data registers: "Once ADCL is read, ADC access to Data Registers is
//! blocked" until ADCH is read; a conversion that completes in between is
//! lost. ADLAR takes effect on the presented result at once.
//!
//! The conversion-complete interrupt is the 328P-numbered vector 22 (2560
//! vector 30, `ADC_vect`); entering it clears ADIF.
//!
//! Not modelled: the ADC clock stopping in the deeper sleep modes, the
//! conversion start on entering ADC noise reduction sleep, and the analog
//! comparator trigger.

use super::{timer1, Avr, AVR_AVCC_MV, AVR_BANDGAP_MV};
use crate::{Bus, SimResult};
use std::cell::Cell;

pub(super) const ADDR_ADCL: u16 = 0x78;
pub(super) const ADDR_ADCH: u16 = 0x79;
pub(super) const ADDR_ADCSRA: u16 = 0x7A;
pub(super) const ADDR_ADCSRB: u16 = 0x7B;
pub(super) const ADDR_ADMUX: u16 = 0x7C;

const ADEN: u8 = 1 << 7;
const ADSC: u8 = 1 << 6;
const ADATE: u8 = 1 << 5;
const ADIF: u8 = 1 << 4;
const ADIE: u8 = 1 << 3;
const ADLAR: u8 = 1 << 5;
const MUX5: u8 = 1 << 3;

/// `ADC_vect` in this core's 328P numbering.
pub(super) const VEC_ADC: u32 = 22;
/// Internal 2.56 V reference.
const AVR_REF_2V56_MV: u32 = 2560;

#[derive(Debug, Default)]
pub(super) struct MegaAdc {
    adcsrb: u8,
    /// Cycle the prescaler last started from (ADEN rising or a trigger).
    origin: u64,
    /// A conversion is running and completes at `done_at`.
    busy: bool,
    done_at: u64,
    /// The next conversion is the first after ADEN (25 ADC clocks).
    first: bool,
    /// Result of the running conversion (latched at its start).
    sample: u16,
    /// What the data registers hold (10 bits, right-adjusted).
    data: u16,
    /// ADCL read and ADCH not yet.
    locked: Cell<bool>,
    /// Level of the auto-trigger flag at the last boundary.
    trig_last: bool,
}

impl MegaAdc {
    /// Something the boundary poll must look at.
    #[inline]
    pub(super) fn active(&self, adcsra: u8) -> bool {
        self.busy || adcsra & (ADEN | ADATE) == ADEN | ADATE
    }
}

pub(super) fn handles(addr: u16) -> bool {
    (ADDR_ADCL..=ADDR_ADMUX).contains(&addr)
}

impl Avr {
    fn adc_div(&self) -> u64 {
        match self.adcsra & 7 {
            0 | 1 => 2,
            n => 1 << n,
        }
    }

    /// One 10-bit conversion of the selected channel against the selected
    /// reference.
    fn mega_adc_sample(&self, bus: &dyn Bus) -> SimResult<u16> {
        let mux = (self.admux & 0x1F)
            | if self.madc.adcsrb & MUX5 != 0 {
                0x20
            } else {
                0
            };
        let input_mv = match mux {
            0x00..=0x07 | 0x20..=0x27 => {
                let ch = if mux >= 0x20 { mux - 0x18 } else { mux };
                let base = self.model.adc_input_base() + u64::from(ch) * 2;
                u32::from(u16::from_le_bytes([
                    bus.read_u8(base)?,
                    bus.read_u8(base + 1)?,
                ]))
            }
            0x1E => AVR_BANDGAP_MV,
            _ => 0,
        };
        let reference_mv = match self.admux >> 6 {
            0b10 => AVR_BANDGAP_MV,
            0b11 => AVR_REF_2V56_MV,
            _ => AVR_AVCC_MV,
        };
        Ok((input_mv * 1024 / reference_mv).min(1023) as u16)
    }

    /// Start a conversion at cycle `start`, taking `half_clocks / 2` ADC
    /// clocks (the first after ADEN always takes 25).
    fn mega_adc_start(&mut self, bus: &dyn Bus, start: u64, half_clocks: u64) -> SimResult<()> {
        let half = if self.madc.first { 50 } else { half_clocks };
        self.madc.first = false;
        self.madc.sample = self.mega_adc_sample(bus)?;
        self.madc.busy = true;
        self.madc.done_at = start + half * self.adc_div() / 2;
        self.adcsra |= ADSC;
        Ok(())
    }

    /// The level of the ADTS trigger flag.
    fn mega_adc_trigger_level(&self) -> bool {
        let tifr1 = || self.timer1_read(timer1::ADDR_TIFR1).unwrap_or(0);
        match self.madc.adcsrb & 7 {
            0b010 => self.eifr & 1 != 0,
            0b011 => self.tifr0 & (1 << 1) != 0,
            0b100 => self.tifr0 & 1 != 0,
            0b101 => tifr1() & (1 << 2) != 0,
            0b110 => tifr1() & 1 != 0,
            0b111 => tifr1() & (1 << 5) != 0,
            _ => false,
        }
    }

    fn mega_adc_sync_irq(&mut self) {
        let bit = 1u64 << VEC_ADC;
        if self.adcsra & (ADIE | ADIF) == ADIE | ADIF {
            self.pending_irq |= bit;
        } else {
            self.pending_irq &= !bit;
        }
    }

    /// The data registers as firmware reads them, per the current ADLAR.
    fn mega_adc_present(&self) -> (u8, u8) {
        let d = self.madc.data;
        if self.admux & ADLAR != 0 {
            (((d & 3) << 6) as u8, (d >> 2) as u8)
        } else {
            (d as u8, (d >> 8) as u8)
        }
    }

    /// Instruction-boundary work: completion, free running, auto trigger.
    pub(super) fn mega_adc_poll(&mut self, bus: &dyn Bus) -> SimResult<()> {
        if self.madc.busy && self.cycles >= self.madc.done_at {
            let done = self.madc.done_at;
            self.madc.busy = false;
            if !self.madc.locked.get() {
                self.madc.data = self.madc.sample;
                (self.adcl, self.adch) = self.mega_adc_present();
            }
            self.adcsra = (self.adcsra & !ADSC) | ADIF;
            if self.adcsra & ADATE != 0 && self.madc.adcsrb & 7 == 0 {
                self.mega_adc_start(bus, done, 26)?;
            }
            self.mega_adc_sync_irq();
        }
        if self.adcsra & (ADEN | ADATE) == ADEN | ADATE && self.madc.adcsrb & 7 != 0 {
            let level = self.mega_adc_trigger_level();
            if level && !self.madc.trig_last && !self.madc.busy {
                // "the prescaler is reset when the trigger event occurs"
                self.madc.origin = self.cycles;
                self.mega_adc_start(bus, self.cycles, 27)?;
            }
            self.madc.trig_last = level;
        }
        Ok(())
    }

    /// Cycles until the running conversion completes, for the idle skip.
    pub(super) fn mega_adc_wake_cycles(&self) -> Option<u64> {
        if self.madc.busy {
            Some(self.madc.done_at.saturating_sub(self.cycles).max(1))
        } else if self.adcsra & (ADEN | ADATE) == ADEN | ADATE && self.madc.adcsrb & 7 != 0 {
            // A trigger flag may rise on any cycle: no skip.
            Some(1)
        } else {
            None
        }
    }

    pub(super) fn mega_adc_vector_entered(&mut self) {
        self.adcsra &= !ADIF;
    }

    pub(super) fn mega_adc_read(&self, addr: u16) -> u8 {
        match addr {
            ADDR_ADCL => {
                self.madc.locked.set(true);
                self.mega_adc_present().0
            }
            ADDR_ADCH => {
                self.madc.locked.set(false);
                self.mega_adc_present().1
            }
            ADDR_ADCSRA => self.adcsra,
            ADDR_ADCSRB => self.madc.adcsrb,
            _ => self.admux,
        }
    }

    pub(super) fn mega_adc_write(&mut self, addr: u16, value: u8, bus: &dyn Bus) -> SimResult<()> {
        match addr {
            ADDR_ADCSRA => {
                let was = self.adcsra;
                // ADIF is cleared by writing one; ADSC cannot be cleared by
                // writing zero and reads one while a conversion runs.
                let adif = was & ADIF & !(value & ADIF);
                self.adcsra = (value & !(ADIF | ADSC)) | adif | (was & ADSC);
                if value & ADEN == 0 {
                    // "Writing zero to ADEN while a conversion is in
                    // progress will terminate this conversion."
                    self.madc.busy = false;
                    self.adcsra &= !ADSC;
                } else if was & ADEN == 0 {
                    self.madc.origin = self.cycles;
                    self.madc.first = true;
                }
                if value & (ADEN | ADSC) == ADEN | ADSC && !self.madc.busy {
                    // Starts at the next ADC clock edge after this write.
                    let div = self.adc_div();
                    let k = self.cycles.saturating_sub(self.madc.origin) / div + 1;
                    let start = self.madc.origin + k * div;
                    self.mega_adc_start(bus, start, 26)?;
                }
                if self.adcsra & ADATE != 0 && was & ADATE == 0 {
                    self.madc.trig_last = self.mega_adc_trigger_level();
                }
                self.io[usize::from(ADDR_ADCSRA - 0x20)] = self.adcsra;
                self.mega_adc_sync_irq();
            }
            ADDR_ADCSRB => {
                self.madc.adcsrb = value & 0x4F;
                self.io[usize::from(ADDR_ADCSRB - 0x20)] = self.madc.adcsrb;
                self.madc.trig_last = self.mega_adc_trigger_level();
            }
            ADDR_ADMUX => {
                self.admux = value;
                if !self.madc.locked.get() {
                    (self.adcl, self.adch) = self.mega_adc_present();
                }
            }
            // ADCL/ADCH are read-only.
            _ => {}
        }
        Ok(())
    }

    pub(super) fn mega_adc_reset(&mut self) {
        self.madc = MegaAdc::default();
        self.admux = 0;
        self.adcsra = 0;
        self.adcl = 0;
        self.adch = 0;
    }
}
