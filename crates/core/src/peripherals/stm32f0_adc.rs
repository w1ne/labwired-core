// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32F0 / STM32L0 12-bit ADC — the `CHSELR` block (RM0091 §13, RM0377
//! §14; Renode models it as `STM32F0_ADC`).
//!
//! Not an `AdcRegisterLayout` variant: this block has no `SQRx` sequencer.
//! The regular sequence is the set bits of `CHSELR`, converted in ascending
//! channel order (`CFGR1.SCANDIR` = 0) or descending (1).
//!
//! Modelled:
//! * `CR`: ADEN raises `ISR.ADRDY`; ADDIS clears ADEN; ADCAL self-clears
//!   (and sets `ISR.EOCAL` on the L0); ADSTART with the software trigger
//!   (`CFGR1.EXTEN` = 0) runs the sequence; ADSTP aborts it.
//! * Each conversion takes `sample + 12.5` ADC clocks, with the ADC clock
//!   taken as the CPU clock (the prescaler/`CKMODE` is not modelled). It sets
//!   EOSMP and EOC, then EOS after the last channel. Reading `DR` clears EOC.
//!   An unread EOC at the next result sets OVR; `OVRMOD` = 0 keeps the old
//!   data, 1 overwrites it.
//! * `CONT` restarts the sequence, `DISCEN` converts one channel per
//!   ADSTART, `WAIT` holds the next conversion until `DR` is read.
//! * `RES` and `ALIGN` shape `DR`; the analog watchdog (`TR`, `AWDEN`,
//!   `AWDSGL`, `AWDCH`) sets `ISR.AWD`.
//! * `CFGR1.DMAEN` raises the `<id>_rx` DMA request after every result.
//! * The interrupt line is the level `ISR & IER`.
//!
//! Inputs: pad channels take a board-driven voltage
//! ([`Peripheral::set_adc_channel_input`]); an undriven channel converts 0.
//! Internal channels follow `CCR.TSEN` / `CCR.VREFEN`: on the F0 the
//! temperature sensor is IN16 and V_REFINT IN17 (RM0091 §13.8); on the L0
//! V_REFINT is IN17 and the sensor IN18 (RM0377 §14.9). F0 V_BAT (IN18) and
//! L0 V_LCD (IN16) read 0.
//!
//! Not modelled: hardware triggers (ADSTART with EXTEN ≠ 0 waits forever),
//! AUTOFF, oversampling (L0 `CFGR2.OVSE`), the conversion prescaler, and a
//! calibration factor (`CALFACT`/`DR` after ADCAL read 0 unless written).

use crate::cycle_clock::CycleClock;
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::cell::Cell;

const ISR: u64 = 0x00;
const IER: u64 = 0x04;
const CR: u64 = 0x08;
const CFGR1: u64 = 0x0C;
const CFGR2: u64 = 0x10;
const SMPR: u64 = 0x14;
const TR: u64 = 0x20;
const CHSELR: u64 = 0x28;
const DR: u64 = 0x40;
const CALFACT: u64 = 0xB4;
const CCR: u64 = 0x308;

const ISR_ADRDY: u32 = 1 << 0;
const ISR_EOSMP: u32 = 1 << 1;
const ISR_EOC: u32 = 1 << 2;
const ISR_EOS: u32 = 1 << 3;
const ISR_OVR: u32 = 1 << 4;
const ISR_AWD: u32 = 1 << 7;
const ISR_EOCAL: u32 = 1 << 11;

const CR_ADEN: u32 = 1 << 0;
const CR_ADDIS: u32 = 1 << 1;
const CR_ADSTART: u32 = 1 << 2;
const CR_ADSTP: u32 = 1 << 4;
const CR_ADVREGEN: u32 = 1 << 28;
const CR_ADCAL: u32 = 1 << 31;

const CFGR1_DMAEN: u32 = 1 << 0;
const CFGR1_SCANDIR: u32 = 1 << 2;
const CFGR1_ALIGN: u32 = 1 << 5;
const CFGR1_EXTEN: u32 = 0b11 << 10;
const CFGR1_OVRMOD: u32 = 1 << 12;
const CFGR1_CONT: u32 = 1 << 13;
const CFGR1_WAIT: u32 = 1 << 14;
const CFGR1_DISCEN: u32 = 1 << 16;
const CFGR1_AWDSGL: u32 = 1 << 22;
const CFGR1_AWDEN: u32 = 1 << 23;

const CCR_VREFEN: u32 = 1 << 22;
const CCR_TSEN: u32 = 1 << 23;

const CHANNELS: usize = 19;
const CHSELR_MASK: u32 = (1 << CHANNELS) - 1;
const DEFAULT_VREF_MV: f64 = 3300.0;
const DEFAULT_AMBIENT_C: f64 = 25.0;

/// Which family's internal channels and register extras apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Stm32F0AdcFamily {
    /// RM0091: TS on IN16, V_REFINT on IN17, no EOCAL/CALFACT/ADVREGEN.
    F0,
    /// RM0377: V_REFINT on IN17, TS on IN18, EOCAL, CALFACT, ADVREGEN.
    L0,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
struct Conversion {
    channel: u8,
    /// Legacy walk: cycles left, counted down by `tick_elapsed`.
    remaining: u64,
    /// Scheduler mode: the cycle the conversion completes on.
    due: u64,
}

#[derive(Debug, serde::Serialize)]
pub struct Stm32F0Adc {
    family: Stm32F0AdcFamily,
    isr: Cell<u32>,
    ier: u32,
    cr: u32,
    cfgr1: u32,
    cfgr2: u32,
    smpr: u32,
    tr: u32,
    chselr: u32,
    dr: u32,
    calfact: u32,
    ccr: u32,
    /// The conversion in progress.
    active: Option<Conversion>,
    /// Channels of the running sequence still to convert, as a CHSELR mask.
    pending: u32,
    /// `DR` holds a result nobody has read (EOC may have been cleared by a
    /// W1C, which does not count as a read).
    #[serde(skip)]
    dr_unread: Cell<bool>,
    #[serde(skip)]
    inputs_mv: [Option<u16>; CHANNELS],
    #[serde(skip)]
    reference_mv: f64,
    #[serde(skip)]
    ambient_c: f64,
    /// Bus-published cycle clock. `Some` once attached → event-schedulable;
    /// `None` keeps the legacy walk.
    #[serde(skip)]
    clock: Option<CycleClock>,
    /// Scheduler mode: `true` while the single conversion wake is queued.
    #[serde(skip)]
    chain_live: bool,
}

impl Stm32F0Adc {
    pub fn new(family: Stm32F0AdcFamily) -> Self {
        Self {
            family,
            isr: Cell::new(0),
            ier: 0,
            cr: 0,
            cfgr1: 0,
            cfgr2: 0,
            smpr: 0,
            tr: 0x0FFF_0000,
            chselr: 0,
            dr: 0,
            calfact: 0,
            ccr: 0,
            active: None,
            pending: 0,
            dr_unread: Cell::new(false),
            inputs_mv: [None; CHANNELS],
            reference_mv: DEFAULT_VREF_MV,
            ambient_c: DEFAULT_AMBIENT_C,
            clock: None,
            chain_live: false,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn now(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    fn resolution_bits(&self) -> u32 {
        match (self.cfgr1 >> 3) & 0b11 {
            0 => 12,
            1 => 10,
            2 => 8,
            _ => 6,
        }
    }

    /// Conversion time in ADC clocks, rounded up: `SMP` sampling + 12.5.
    fn conversion_cycles(&self) -> u64 {
        // Sampling time ×2 (half-cycle units), RM0091 §13.12.6 / RM0377 §14.12.6.
        const F0_SMP_X2: [u64; 8] = [3, 15, 27, 57, 83, 111, 143, 479];
        const L0_SMP_X2: [u64; 8] = [3, 7, 15, 25, 39, 79, 159, 321];
        let smp = (self.smpr & 0b111) as usize;
        let sample_x2 = match self.family {
            Stm32F0AdcFamily::F0 => F0_SMP_X2[smp],
            Stm32F0AdcFamily::L0 => L0_SMP_X2[smp],
        };
        (sample_x2 + 25).div_ceil(2)
    }

    /// Millivolts on `channel`: a driven pad, an enabled internal source, or 0.
    fn channel_mv(&self, channel: u8) -> f64 {
        if let Some(mv) = self.inputs_mv[channel as usize] {
            return f64::from(mv);
        }
        let vrefint = self.ccr & CCR_VREFEN != 0;
        let tsen = self.ccr & CCR_TSEN != 0;
        match (self.family, channel) {
            // DS9826 Table 22: V_REFINT typ 1.23 V. Table 50: V30 typ 1.43 V,
            // Avg_Slope typ 4.3 mV/°C, V_SENSE falling with temperature.
            (Stm32F0AdcFamily::F0, 16) if tsen => 1430.0 - (self.ambient_c - 30.0) * 4.3,
            (Stm32F0AdcFamily::F0, 17) if vrefint => 1230.0,
            // DS10182 (STM32L072): V_REFINT typ 1.224 V; temperature sensor
            // V130 typ 670 mV, Avg_Slope typ 1.61 mV/°C.
            (Stm32F0AdcFamily::L0, 17) if vrefint => 1224.0,
            (Stm32F0AdcFamily::L0, 18) if tsen => 670.0 - (130.0 - self.ambient_c) * 1.61,
            _ => 0.0,
        }
    }

    /// The conversion result at the configured resolution (right-aligned).
    fn convert(&self, channel: u8) -> u32 {
        let count12 = (self.channel_mv(channel) * 4095.0 / self.reference_mv)
            .floor()
            .clamp(0.0, 4095.0) as u32;
        count12 >> (12 - self.resolution_bits())
    }

    fn aligned(&self, code: u32) -> u32 {
        if self.cfgr1 & CFGR1_ALIGN == 0 {
            return code;
        }
        match self.resolution_bits() {
            // 6-bit left alignment is on the byte (RM0091 §13.6.3).
            6 => code << 2,
            bits => code << (16 - bits),
        }
    }

    fn awd_watches(&self, channel: u8) -> bool {
        if self.cfgr1 & CFGR1_AWDEN == 0 {
            return false;
        }
        self.cfgr1 & CFGR1_AWDSGL == 0 || ((self.cfgr1 >> 26) & 0x1F) as u8 == channel
    }

    /// The next channel of the sequence still pending, in scan order.
    fn next_channel(&self) -> Option<u8> {
        if self.pending == 0 {
            return None;
        }
        Some(if self.cfgr1 & CFGR1_SCANDIR == 0 {
            self.pending.trailing_zeros() as u8
        } else {
            31 - self.pending.leading_zeros() as u8
        })
    }

    fn begin_next(&mut self) {
        let now = self.now();
        self.active = self.next_channel().map(|channel| {
            self.pending &= !(1 << channel);
            let remaining = self.conversion_cycles();
            Conversion {
                channel,
                remaining,
                due: now + remaining,
            }
        });
    }

    fn start_sequence(&mut self) {
        self.pending = self.chselr & CHSELR_MASK;
        self.begin_next();
        if self.active.is_none() {
            // Empty CHSELR: nothing to convert, the command completes.
            self.cr &= !CR_ADSTART;
        }
    }

    fn stop(&mut self) {
        self.active = None;
        self.pending = 0;
        self.cr &= !(CR_ADSTART | CR_ADSTP);
    }

    /// Finish the active conversion. Returns whether to raise a DMA request.
    fn complete(&mut self, channel: u8) -> bool {
        let code = self.convert(channel);
        let mut isr = self.isr.get() | ISR_EOSMP;
        let overrun = self.dr_unread.get() && isr & ISR_EOC != 0;
        if overrun {
            isr |= ISR_OVR;
        }
        if !overrun || self.cfgr1 & CFGR1_OVRMOD != 0 {
            self.dr = self.aligned(code);
            self.dr_unread.set(true);
        }
        isr |= ISR_EOC;
        if self.awd_watches(channel) {
            let code12 = code << (12 - self.resolution_bits());
            let lt = self.tr & 0xFFF;
            let ht = (self.tr >> 16) & 0xFFF;
            if code12 < lt || code12 > ht {
                isr |= ISR_AWD;
            }
        }
        let end_of_sequence = self.pending == 0;
        if end_of_sequence {
            isr |= ISR_EOS;
        }
        self.isr.set(isr);

        let discontinuous = self.cfgr1 & CFGR1_DISCEN != 0;
        if end_of_sequence {
            if self.cfgr1 & CFGR1_CONT != 0 && !discontinuous {
                self.start_sequence();
            } else {
                self.cr &= !CR_ADSTART;
                self.active = None;
            }
        } else if discontinuous {
            // One channel per trigger; the rest of the sequence waits.
            self.cr &= !CR_ADSTART;
            self.active = None;
        } else {
            self.begin_next();
        }
        self.cfgr1 & CFGR1_DMAEN != 0 && (!overrun || self.cfgr1 & CFGR1_OVRMOD != 0)
    }

    fn apply_cr(&mut self, value: u32) {
        let mut cr = self.cr;
        if self.family == Stm32F0AdcFamily::L0 {
            cr = (cr & !CR_ADVREGEN) | (value & CR_ADVREGEN);
        }
        // ADCAL is accepted only with the ADC disabled; it completes at once.
        if value & CR_ADCAL != 0 && cr & CR_ADEN == 0 {
            if self.family == Stm32F0AdcFamily::L0 {
                self.isr.set(self.isr.get() | ISR_EOCAL);
            }
        }
        if value & CR_ADEN != 0 && cr & CR_ADEN == 0 && value & CR_ADDIS == 0 {
            cr |= CR_ADEN;
            self.isr.set(self.isr.get() | ISR_ADRDY);
        }
        self.cr = cr;
        if value & CR_ADSTP != 0 && self.cr & CR_ADSTART != 0 {
            self.stop();
        }
        if value & CR_ADDIS != 0 && self.cr & CR_ADEN != 0 {
            self.stop();
            self.cr &= !CR_ADEN;
            return;
        }
        if value & CR_ADSTART != 0 && self.cr & CR_ADEN != 0 && self.cr & CR_ADSTART == 0 {
            self.cr |= CR_ADSTART;
            if self.cfgr1 & CFGR1_EXTEN == 0 {
                if self.pending != 0 {
                    // Discontinuous mode resumes the interrupted sequence.
                    self.begin_next();
                } else {
                    self.start_sequence();
                }
            }
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            ISR => self.isr.get(),
            IER => self.ier,
            CR => self.cr,
            CFGR1 => self.cfgr1,
            CFGR2 => self.cfgr2,
            SMPR => self.smpr,
            TR => self.tr,
            CHSELR => self.chselr,
            DR => self.dr,
            CALFACT if self.family == Stm32F0AdcFamily::L0 => self.calfact,
            CCR => self.ccr,
            _ => {
                crate::census_reg!("adc:Stm32F0Adc", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        let isr_mask = match self.family {
            Stm32F0AdcFamily::F0 => 0x9F,
            Stm32F0AdcFamily::L0 => 0x89F,
        };
        match offset {
            ISR => self.isr.set(self.isr.get() & !(value & isr_mask)),
            IER => self.ier = value & isr_mask,
            CR => self.apply_cr(value),
            // RM0091 §13.12.4: CFGR1/CFGR2/SMPR/TR/CHSELR are written only
            // with ADSTART = 0; this model takes the write regardless.
            CFGR1 => self.cfgr1 = value & 0x7CC1_FDFF,
            CFGR2 => self.cfgr2 = value,
            SMPR => self.smpr = value & 0b111,
            TR => self.tr = value & 0x0FFF_0FFF,
            CHSELR => self.chselr = value & CHSELR_MASK,
            DR => {}
            CALFACT if self.family == Stm32F0AdcFamily::L0 => self.calfact = value & 0x7F,
            CCR => self.ccr = value,
            _ => {
                crate::census_reg!("adc:Stm32F0Adc", offset, "write");
            }
        }
    }

    fn read_dr(&self) {
        self.isr.set(self.isr.get() & !ISR_EOC);
        self.dr_unread.set(false);
    }
}

impl Peripheral for Stm32F0Adc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        if reg_offset == DR {
            self.read_dr();
        }
        Ok(((self.read_reg(reg_offset) >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let shift = (offset % 4) * 8;
        // ISR is W1C and CR holds commands: a byte write must not replay the
        // other bytes' read-back.
        let merged = if matches!(reg_offset, ISR | CR) {
            u32::from(value) << shift
        } else {
            (self.read_reg(reg_offset) & !(0xFF << shift)) | (u32::from(value) << shift)
        };
        if reg_offset == CR {
            // Keep ADEN/ADVREGEN state for bytes this write does not touch.
            let keep = self.cr & !(0xFF << shift) & (CR_ADVREGEN | CR_ADEN);
            self.apply_cr(merged | keep);
        } else {
            self.write_reg(reg_offset, merged);
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let reg_offset = offset & !3;
        if reg_offset == DR {
            self.read_dr();
        }
        Ok(self.read_reg(reg_offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        Some(((self.read_reg(offset & !3) >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        // One live wake: a conversion started by an MMIO write arms it; the
        // handler re-arms itself while a sequence runs. A wake left behind by
        // ADSTP/ADDIS is absorbed by the `due` check in `on_event`.
        match self.active {
            Some(conv) if self.scheduler_mode() && !self.chain_live => {
                self.chain_live = true;
                vec![(conv.due.saturating_sub(self.now()).saturating_sub(1), 0)]
            }
            _ => Vec::new(),
        }
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let mut result = crate::sched::EventResult::default();
        if !self.scheduler_mode() {
            return result;
        }
        let now = self.now();
        let Some(conv) = self.active else {
            self.chain_live = false;
            return result;
        };
        if now < conv.due {
            result.reschedule_delay = Some(conv.due - now);
            return result;
        }
        if self.cfgr1 & CFGR1_WAIT != 0 && self.dr_unread.get() {
            // WAIT: the result is held until DR is read.
            result.reschedule_delay = Some(1);
            return result;
        }
        if self.complete(conv.channel) {
            result.dma_signals = vec![crate::DMA_SIGNAL_RX];
        }
        result.raise_own_irq = self.isr.get() & self.ier != 0;
        result.reschedule_delay = self.active.map(|next| next.due.saturating_sub(now).max(1));
        self.chain_live = result.reschedule_delay.is_some();
        result
    }

    /// At most one result per call: a late tick must not overrun results
    /// the firmware (or the DMA) would have read in between.
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        let mut result = PeripheralTickResult::default();
        if self.scheduler_mode() {
            return result;
        }
        let Some(mut conv) = self.active else {
            return result;
        };
        if self.cfgr1 & CFGR1_WAIT != 0 && self.dr_unread.get() {
            return result;
        }
        if conv.remaining > cycles {
            conv.remaining -= cycles;
            self.active = Some(conv);
            return result;
        }
        if self.complete(conv.channel) {
            result.dma_signals = Some(vec![crate::DMA_SIGNAL_RX]);
        }
        result
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.isr.get() & self.ier != 0)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        match self.inputs_mv.get_mut(channel as usize) {
            Some(slot) => {
                *slot = Some(millivolts);
                true
            }
            None => false,
        }
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        match self.inputs_mv.get_mut(channel as usize) {
            Some(slot) => {
                *slot = None;
                true
            }
            None => false,
        }
    }

    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn supports_adc_reference_voltage(&self) -> bool {
        true
    }

    fn set_adc_reference_voltage_v(&mut self, volts: f64) -> bool {
        if !volts.is_finite() || volts <= 0.0 {
            return false;
        }
        self.reference_mv = volts * 1000.0;
        true
    }

    fn set_ambient_temperature_c(&mut self, celsius: f64) -> bool {
        if !celsius.is_finite() {
            return false;
        }
        self.ambient_c = celsius;
        true
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled(family: Stm32F0AdcFamily) -> Stm32F0Adc {
        let mut adc = Stm32F0Adc::new(family);
        adc.write_u32(CR, CR_ADEN).unwrap();
        adc
    }

    fn run(adc: &mut Stm32F0Adc) -> Vec<u32> {
        let mut dma = Vec::new();
        for _ in 0..10_000 {
            let r = adc.tick_elapsed(1);
            if r.dma_signals.is_some() {
                dma.push(adc.read_u32(DR).unwrap());
            }
            if adc.active.is_none() {
                break;
            }
        }
        dma
    }

    #[test]
    fn aden_raises_adrdy_and_adcal_self_clears() {
        let mut adc = Stm32F0Adc::new(Stm32F0AdcFamily::L0);
        adc.write_u32(CR, CR_ADCAL).unwrap();
        assert_eq!(adc.read_u32(CR).unwrap() & CR_ADCAL, 0);
        assert_ne!(adc.read_u32(ISR).unwrap() & ISR_EOCAL, 0);
        adc.write_u32(CR, CR_ADEN).unwrap();
        assert_ne!(adc.read_u32(ISR).unwrap() & ISR_ADRDY, 0);
        adc.write_u32(CR, CR_ADDIS).unwrap();
        assert_eq!(adc.read_u32(CR).unwrap() & CR_ADEN, 0);
    }

    #[test]
    fn single_sequence_converts_chselr_in_scan_order_with_dma() {
        let mut adc = enabled(Stm32F0AdcFamily::F0);
        adc.set_adc_channel_input(0, 3300);
        adc.set_adc_channel_input(1, 1650);
        adc.set_adc_channel_input(5, 0);
        adc.write_u32(CHSELR, 0b10_0011).unwrap();
        adc.write_u32(CFGR1, CFGR1_DMAEN).unwrap();
        adc.write_u32(CR, CR_ADEN | CR_ADSTART).unwrap();
        assert_eq!(run(&mut adc), vec![4095, 2047, 0]);
        let isr = adc.read_u32(ISR).unwrap();
        assert_ne!(isr & ISR_EOS, 0);
        assert_eq!(isr & ISR_OVR, 0);
        assert_eq!(adc.read_u32(CR).unwrap() & CR_ADSTART, 0);

        adc.write_u32(ISR, ISR_EOS).unwrap();
        adc.write_u32(CFGR1, CFGR1_DMAEN | CFGR1_SCANDIR).unwrap();
        adc.write_u32(CR, CR_ADEN | CR_ADSTART).unwrap();
        assert_eq!(run(&mut adc), vec![0, 2047, 4095]);
    }

    #[test]
    fn unread_result_overruns_and_ovrmod_picks_which_data_survives() {
        let mut adc = enabled(Stm32F0AdcFamily::F0);
        adc.set_adc_channel_input(0, 3300);
        adc.set_adc_channel_input(1, 0);
        adc.write_u32(CHSELR, 0b11).unwrap();
        adc.write_u32(CR, CR_ADEN | CR_ADSTART).unwrap();
        run(&mut adc);
        assert_ne!(adc.read_u32(ISR).unwrap() & ISR_OVR, 0);
        assert_eq!(adc.read_u32(DR).unwrap(), 4095, "OVRMOD=0 keeps old data");

        adc.write_u32(ISR, 0xFF).unwrap();
        adc.write_u32(CFGR1, CFGR1_OVRMOD).unwrap();
        adc.write_u32(CR, CR_ADEN | CR_ADSTART).unwrap();
        run(&mut adc);
        assert_eq!(adc.read_u32(DR).unwrap(), 0, "OVRMOD=1 overwrites");
    }

    #[test]
    fn resolution_alignment_and_watchdog() {
        let mut adc = enabled(Stm32F0AdcFamily::F0);
        adc.set_adc_channel_input(3, 3300);
        adc.write_u32(CHSELR, 1 << 3).unwrap();
        // 8-bit, left-aligned, watchdog on IN3 with HT 0x800.
        adc.write_u32(TR, 0x0800_0000).unwrap();
        adc.write_u32(
            CFGR1,
            (2 << 3) | CFGR1_ALIGN | CFGR1_AWDEN | CFGR1_AWDSGL | (3 << 26),
        )
        .unwrap();
        adc.write_u32(CR, CR_ADEN | CR_ADSTART).unwrap();
        run(&mut adc);
        assert_eq!(adc.read_u32(DR).unwrap(), 0xFF00);
        assert_ne!(adc.read_u32(ISR).unwrap() & ISR_AWD, 0);
    }

    #[test]
    fn internal_channels_follow_family_and_ccr() {
        let mut f0 = enabled(Stm32F0AdcFamily::F0);
        let mut l0 = enabled(Stm32F0AdcFamily::L0);
        for adc in [&mut f0, &mut l0] {
            adc.write_u32(CHSELR, 0b111 << 16).unwrap();
            adc.write_u32(CFGR1, CFGR1_DMAEN).unwrap();
            adc.write_u32(CR, CR_ADEN | CR_ADSTART).unwrap();
            assert_eq!(run(adc), vec![0, 0, 0], "disconnected until CCR enables");
            adc.write_u32(CCR, CCR_VREFEN | CCR_TSEN).unwrap();
            adc.write_u32(CR, CR_ADEN | CR_ADSTART).unwrap();
        }
        // F0: TS 1451.5 mV, VREFINT 1230 mV, VBAT 0. L0: VLCD 0, VREFINT 1224, TS 500.95.
        assert_eq!(run(&mut f0), vec![1801, 1526, 0]);
        assert_eq!(run(&mut l0), vec![0, 1518, 621]);
    }

    #[test]
    fn continuous_restarts_and_discontinuous_steps_one_channel() {
        let mut adc = enabled(Stm32F0AdcFamily::L0);
        adc.write_u32(CHSELR, 0b11).unwrap();
        adc.write_u32(CFGR1, CFGR1_DMAEN | CFGR1_CONT).unwrap();
        adc.write_u32(CR, CR_ADEN | CR_ADSTART).unwrap();
        let mut results = 0;
        for _ in 0..200 {
            if adc.tick_elapsed(1).dma_signals.is_some() {
                adc.read_u32(DR).unwrap();
                results += 1;
            }
        }
        assert!(results > 4, "CONT keeps converting ({results})");
        adc.write_u32(CR, CR_ADEN | CR_ADSTP).unwrap();
        assert_eq!(adc.read_u32(CR).unwrap() & (CR_ADSTART | CR_ADSTP), 0);

        adc.write_u32(ISR, 0xFFF).unwrap();
        adc.write_u32(CFGR1, CFGR1_DISCEN).unwrap();
        adc.write_u32(CR, CR_ADEN | CR_ADSTART).unwrap();
        run(&mut adc);
        assert_eq!(
            adc.read_u32(ISR).unwrap() & ISR_EOS,
            0,
            "first channel only"
        );
        adc.read_u32(DR).unwrap();
        adc.write_u32(CR, CR_ADEN | CR_ADSTART).unwrap();
        run(&mut adc);
        assert_ne!(adc.read_u32(ISR).unwrap() & ISR_EOS, 0);
    }

    #[test]
    fn interrupt_line_is_isr_and_ier() {
        let mut adc = enabled(Stm32F0AdcFamily::F0);
        assert_eq!(adc.irq_line_level(), Some(false));
        adc.write_u32(IER, ISR_EOC).unwrap();
        adc.write_u32(CHSELR, 1).unwrap();
        adc.write_u32(CR, CR_ADEN | CR_ADSTART).unwrap();
        run(&mut adc);
        assert_eq!(adc.irq_line_level(), Some(true));
        adc.read_u32(DR).unwrap();
        assert_eq!(adc.irq_line_level(), Some(false));
    }
}
