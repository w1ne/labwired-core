// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Renesas RA 12-bit successive-approximation A/D converter, ADC12.
//!
//! Register layout from FSP `R7FA8M1AH.h` (`R_ADC0_Type`, 0x1E4 bytes): `ADCSR`
//! (u16) +0x00, `ADANSA0`/`ADANSA1` +0x04/+0x06, `ADADS0`/`ADADS1`
//! +0x08/+0x0A, `ADADC` (u8) +0x0C, `ADCER` (u16) +0x0E, `ADDR0`-`ADDR28`
//! (u16) +0x20-+0x58. One unit per 0x200 window (ADC0 0x4033_2000, ADC1
//! 0x4033_2200 on the RA8M1). Behaviour from FSP `r_adc.c`, which Zephyr's
//! `adc_renesas_ra.c` drives: `R_ADC_Open` stores `ADCSR` without `ADST`,
//! `R_ADC_ScanStart` writes it back with `ADST` set (software trigger),
//! `R_ADC_ScanStatusGet`/`R_ADC_ScanStart` read `ADST` as "scan in
//! progress", and the scan-end ISR reads `ADDRn`.
//!
//! Modelled:
//! - single scan (`ADCSR.ADCS` = 0) and continuous scan (`ADCS` = 2) of
//!   the group-A channels in `ADANSA0` (channels 0-15) / `ADANSA1`
//!   (16-28), started by writing `ADST` = 1; writing `ADST` = 0 aborts;
//! - each scan takes `conversion_cycles` bus cycles per selected channel
//!   (a nominal 1 µs at the configured `clock_hz`, not the hardware
//!   manual's state count), then every selected channel's `ADDRn` is
//!   loaded, `ADST` clears (single scan) and the scan-end event fires;
//! - the conversion code is `input_mv * (2^bits - 1) / vref_mv`, clamped,
//!   with `bits` from `ADCER.ADPRC` (12/10/8) and left-justified when
//!   `ADCER.ADRFMT` = 1 (the value shifted to bit 15);
//! - A/D-converted value addition / averaging (`ADADS0/1` channel bits,
//!   `ADADC.ADC` count 1/2/3/4/16, `ADADC.AVEE`): addition sums the
//!   (identical) samples, averaging leaves the single-sample code;
//! - channel levels come from `input_mv` in the chip config or
//!   [`Peripheral::set_adc_channel_input`]; an undriven channel reads 0.
//!
//! The scan-end interrupt is an ELC event (`ELC_EVENT_ADC0_SCAN_END` 0x1AE,
//! `ELC_EVENT_ADC1_SCAN_END` 0x1B4 on the RA8M1, FSP `bsp_elc.h`). It leaves
//! through `explicit_irqs` with [`Peripheral::explicit_irqs_are_link_events`]
//! so the bus links it through the ICU `IELSRn` ([`super::ra_icu`]).
//!
//! NOT modelled: group scan mode (`ADCS` = 1, group B / `ADANSB`, the
//! group-B scan-end event), double-trigger mode, synchronous/asynchronous
//! hardware triggers (`ADCSR.TRGE`/`EXTRG`, `ADSTRGR`: only the software
//! `ADST` write starts a scan), the temperature sensor and internal reference
//! (`ADEXICR`, `ADTSDR`, `ADOCDR`), self-diagnosis, the window comparator,
//! the data buffer (`ADBUF*`), sample-and-hold, clear-after-read
//! (`ADCER.ACE`: reads are side-effect free here, so `ADDRn` keeps its
//! value until the next conversion), and sampling-time registers
//! (`ADSSTR*`, stored only). Every other register in the window is plain
//! storage.

use std::collections::BTreeMap;

use crate::{CycleClock, Peripheral, SimResult};

/// Size of one unit's window.
const WINDOW: usize = 0x200;

const ADCSR: u64 = 0x00;
const ADANSA0: u64 = 0x04;
const ADANSA1: u64 = 0x06;
const ADADS0: u64 = 0x08;
const ADADS1: u64 = 0x0A;
const ADADC: u64 = 0x0C;
const ADCER: u64 = 0x0E;
const ADDR0: u64 = 0x20;
/// `ADDR0`-`ADDR28`.
const CHANNELS: usize = 29;
const ADDR_END: u64 = ADDR0 + 2 * CHANNELS as u64;

const ADST: u16 = 1 << 15;
const ADCS_SHIFT: u16 = 13;
const ADCS_MASK: u16 = 3;
const ADCS_SINGLE: u16 = 0;
const ADCS_CONTINUOUS: u16 = 2;

const ADRFMT: u16 = 1 << 15;
const ADPRC_SHIFT: u16 = 1;
const ADPRC_MASK: u16 = 3;

const AVEE: u8 = 1 << 7;
const ADADC_COUNT_MASK: u8 = 7;

/// Zephyr's EK-RA8M1 ICLK, the rate the bus cycles run at.
pub const DEFAULT_CLOCK_HZ: u64 = 480_000_000;
/// The nominal conversion time per channel, 1 µs.
pub const DEFAULT_CONVERSION_NS: u64 = 1_000;
/// `ELC_EVENT_ADC0_SCAN_END` on the RA8M1.
pub const DEFAULT_ELC_EVENT: u32 = 0x1AE;
/// AVCC0/VREFH0 on the EK-RA8M1 (Zephyr `vref-mv = <3300>`).
pub const DEFAULT_VREF_MV: u16 = 3300;

/// One RA ADC12 unit.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RaAdc12 {
    elc_event: u32,
    vref_mv: u16,
    conversion_cycles: u64,
    /// Plain register storage for the whole window; `ADDRn` lives in `data`.
    regs: Vec<u8>,
    data: [u16; CHANNELS],
    inputs: BTreeMap<u8, u16>,
    /// Cycle the running scan completes at.
    armed: Option<u64>,
    /// Bumped whenever `armed` changes; a stale scheduled event whose token
    /// no longer matches is dropped.
    generation: u32,
    #[serde(skip)]
    pending_events: Vec<(u64, u32)>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl RaAdc12 {
    pub fn new(clock_hz: u64, elc_event: u32, vref_mv: u16) -> Self {
        let conversion_cycles = (u128::from(clock_hz.max(1)) * u128::from(DEFAULT_CONVERSION_NS)
            / 1_000_000_000)
            .max(1) as u64;
        Self {
            elc_event,
            vref_mv: vref_mv.max(1),
            conversion_cycles,
            regs: vec![0; WINDOW],
            data: [0; CHANNELS],
            inputs: BTreeMap::new(),
            armed: None,
            generation: 0,
            pending_events: Vec::new(),
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn now_cycle(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    fn reg16(&self, offset: u64) -> u16 {
        let o = offset as usize;
        u16::from_le_bytes([self.regs[o], self.regs[o + 1]])
    }

    fn set_reg16(&mut self, offset: u64, value: u16) {
        let o = offset as usize;
        self.regs[o..o + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn adcsr(&self) -> u16 {
        self.reg16(ADCSR)
    }

    fn mode(&self) -> u16 {
        (self.adcsr() >> ADCS_SHIFT) & ADCS_MASK
    }

    /// Group-A channel mask, `ADANSA1:ADANSA0`.
    fn scan_mask(&self) -> u32 {
        let m = u32::from(self.reg16(ADANSA0)) | (u32::from(self.reg16(ADANSA1)) << 16);
        m & ((1 << CHANNELS) - 1)
    }

    fn addition_mask(&self) -> u32 {
        u32::from(self.reg16(ADADS0)) | (u32::from(self.reg16(ADADS1)) << 16)
    }

    /// Conversions summed per `ADADC.ADC`: 1, 2, 3, 4 or 16.
    fn addition_count(&self) -> u32 {
        match self.regs[ADADC as usize] & ADADC_COUNT_MASK {
            1 => 2,
            2 => 3,
            3 => 4,
            5 => 16,
            _ => 1,
        }
    }

    fn resolution_bits(&self) -> u32 {
        match (self.reg16(ADCER) >> ADPRC_SHIFT) & ADPRC_MASK {
            1 => 10,
            2 => 8,
            _ => 12,
        }
    }

    /// The `ADDRn` value one conversion of `channel` produces now.
    fn convert(&self, channel: usize) -> u16 {
        let bits = self.resolution_bits();
        let full = (1u32 << bits) - 1;
        let mv = u32::from(*self.inputs.get(&(channel as u8)).unwrap_or(&0));
        let code = (mv * full / u32::from(self.vref_mv)).min(full);
        let adadc = self.regs[ADADC as usize];
        let n = self.addition_count();
        let added = if self.addition_mask() & (1 << channel) != 0 && n > 1 && adadc & AVEE == 0 {
            code * n
        } else {
            code
        };
        if self.reg16(ADCER) & ADRFMT != 0 {
            // Left-justified: a single conversion's MSB at bit 15.
            ((added << (16 - bits)) & 0xFFFF) as u16
        } else {
            added.min(0xFFFF) as u16
        }
    }

    /// Arm the end of a scan of the current channel set from now.
    fn start_scan(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.armed = None;
        let channels = u64::from(self.scan_mask().count_ones());
        if channels == 0 || self.clock.is_none() {
            return;
        }
        let delay = channels * self.conversion_cycles;
        self.armed = Some(self.now_cycle() + delay);
        if self.scheduler_mode() {
            self.pending_events.push((delay, self.generation));
        }
    }

    fn abort_scan(&mut self) {
        if self.armed.take().is_some() {
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Complete the armed scan: load `ADDRn`, clear `ADST` (single scan) or
    /// start the next pass (continuous). Returns the ELC events raised.
    fn fire(&mut self) -> Vec<u32> {
        if self.armed.take().is_none() {
            return Vec::new();
        }
        let mask = self.scan_mask();
        for ch in 0..CHANNELS {
            if mask & (1 << ch) != 0 {
                self.data[ch] = self.convert(ch);
            }
        }
        if self.mode() == ADCS_CONTINUOUS {
            self.start_scan();
        } else {
            let adcsr = self.adcsr() & !ADST;
            self.set_reg16(ADCSR, adcsr);
        }
        vec![self.elc_event]
    }

    fn after_adcsr_write(&mut self, before: u16) {
        let after = self.adcsr();
        let started = after & ADST != 0 && before & ADST == 0;
        if after & ADST == 0 {
            self.abort_scan();
        } else if started {
            // A software `ADST` write starts the scan whether or not `TRGE`
            // also enables hardware triggers (FSP sets `TRGE` for software
            // scans too, r_adc.c `r_adc_open_sub`). Group scans are not
            // modelled: the start bit stays set and nothing converts.
            let mode = self.mode();
            if mode == ADCS_SINGLE || mode == ADCS_CONTINUOUS {
                self.start_scan();
            }
        }
    }

    pub fn set_channel_input(&mut self, channel: u8, millivolts: u16) {
        self.inputs.insert(channel, millivolts);
    }
}

impl Default for RaAdc12 {
    fn default() -> Self {
        Self::new(DEFAULT_CLOCK_HZ, DEFAULT_ELC_EVENT, DEFAULT_VREF_MV)
    }
}

impl Peripheral for RaAdc12 {
    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode() && self.armed.is_some()
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn explicit_irqs_are_link_events(&self) -> bool {
        true
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> crate::PeripheralTickResult {
        if self.scheduler_mode() {
            return crate::PeripheralTickResult::default();
        }
        let mut events = Vec::new();
        if self.armed.is_some_and(|at| self.now_cycle() >= at) {
            events.extend(self.fire());
        }
        crate::PeripheralTickResult {
            explicit_irqs: (!events.is_empty()).then_some(events),
            ..Default::default()
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let offset = offset % WINDOW as u64;
        if (ADDR0..ADDR_END).contains(&offset) {
            let v = self.data[((offset - ADDR0) / 2) as usize];
            return Ok(v.to_le_bytes()[((offset - ADDR0) % 2) as usize]);
        }
        Ok(self.regs[offset as usize])
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let offset = offset % WINDOW as u64;
        if (ADDR0..ADDR_END).contains(&offset) {
            // ADDRn are read-only.
            return Ok(());
        }
        let before = self.adcsr();
        self.regs[offset as usize] = value;
        if offset == ADCSR + 1 {
            self.after_adcsr_write(before);
        }
        Ok(())
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        std::mem::take(&mut self.pending_events)
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if event_token != self.generation || self.armed.is_none() {
            return crate::sched::EventResult::default();
        }
        let events = self.fire();
        // A continuous scan armed its next pass in `fire`; it goes back as
        // a reschedule of this token (the drain does not harvest
        // `take_scheduled_events` after an event).
        let reschedule_delay = self.pending_events.pop().map(|(delay, _)| {
            self.generation = event_token;
            delay
        });
        self.pending_events.clear();
        crate::sched::EventResult {
            explicit_irqs: events,
            reschedule_delay,
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn on_node_reset(&mut self) {
        let clock = self.clock.take();
        let inputs = std::mem::take(&mut self.inputs);
        let conversion_cycles = self.conversion_cycles;
        *self = Self::new(DEFAULT_CLOCK_HZ, self.elc_event, self.vref_mv);
        self.conversion_cycles = conversion_cycles;
        self.inputs = inputs;
        self.clock = clock;
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        if usize::from(channel) >= CHANNELS {
            return false;
        }
        self.set_channel_input(channel, millivolts);
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        if usize::from(channel) >= CHANNELS {
            return false;
        }
        self.inputs.remove(&channel);
        true
    }

    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let mut restored: RaAdc12 = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("RaAdc12 snapshot decode: {error}"))
        })?;
        restored.clock = self.clock.take();
        *self = restored;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 MHz bus so one channel converts in exactly one cycle.
    const HZ: u64 = 1_000_000;
    const EVENT: u32 = 0x1B4;
    const TRGE: u16 = 1 << 9;

    fn adc_at(cycle: u64) -> (RaAdc12, CycleClock) {
        let clock = CycleClock::default();
        clock.publish(cycle);
        let mut a = RaAdc12::new(HZ, EVENT, 3300);
        a.attach_cycle_clock(clock.clone());
        (a, clock)
    }

    fn w16(a: &mut RaAdc12, off: u64, v: u16) {
        a.write(off, v as u8).unwrap();
        a.write(off + 1, (v >> 8) as u8).unwrap();
    }

    fn r16(a: &RaAdc12, off: u64) -> u16 {
        u16::from(a.read(off).unwrap()) | (u16::from(a.read(off + 1).unwrap()) << 8)
    }

    fn addr(ch: u64) -> u64 {
        ADDR0 + 2 * ch
    }

    /// Deliver the armed scan end the way the build's time base does.
    fn finish(a: &mut RaAdc12, clock: &CycleClock) -> Vec<u32> {
        let at = a.armed.expect("a scan is armed");
        clock.publish(at);
        if a.scheduler_mode() {
            a.fire()
        } else {
            a.tick_elapsed(1).explicit_irqs.unwrap_or_default()
        }
    }

    #[test]
    fn single_scan_converts_the_selected_channels_and_clears_adst() {
        let (mut a, clock) = adc_at(100);
        a.set_channel_input(0, 1650);
        a.set_channel_input(2, 3300);
        a.set_channel_input(1, 999);
        w16(&mut a, ADANSA0, 0b101);
        w16(&mut a, ADCSR, ADST);
        assert_eq!(r16(&a, ADCSR) & ADST, ADST, "scan in progress");
        assert_eq!(a.armed, Some(102), "two channels, one cycle each");
        assert_eq!(finish(&mut a, &clock), vec![EVENT]);
        assert_eq!(r16(&a, ADCSR) & ADST, 0);
        assert_eq!(r16(&a, addr(0)), 2047, "1650 mV of 3300 mV, 12-bit");
        assert_eq!(r16(&a, addr(2)), 4095);
        assert_eq!(r16(&a, addr(1)), 0, "unselected channel untouched");
        assert!(a.armed.is_none());
    }

    #[test]
    fn adansa1_selects_channels_from_sixteen() {
        let (mut a, clock) = adc_at(0);
        a.set_channel_input(16, 3300);
        w16(&mut a, ADANSA1, 1);
        w16(&mut a, ADCSR, ADST);
        assert_eq!(finish(&mut a, &clock), vec![EVENT]);
        assert_eq!(r16(&a, addr(16)), 4095);
    }

    #[test]
    fn resolution_justification_and_addition() {
        let (mut a, clock) = adc_at(0);
        a.set_channel_input(0, 3300);
        w16(&mut a, ADANSA0, 1);
        // 10-bit, left-justified.
        w16(&mut a, ADCER, ADRFMT | (1 << ADPRC_SHIFT));
        w16(&mut a, ADCSR, ADST);
        finish(&mut a, &clock);
        assert_eq!(r16(&a, addr(0)), 0x3FF << 6);
        // 12-bit right-justified, four-time addition on channel 0.
        w16(&mut a, ADCER, 0);
        w16(&mut a, ADADS0, 1);
        a.write(ADADC, 3).unwrap();
        w16(&mut a, ADCSR, ADST);
        finish(&mut a, &clock);
        assert_eq!(r16(&a, addr(0)), 4 * 4095);
        // Averaging leaves the single-sample code.
        a.write(ADADC, AVEE | 3).unwrap();
        w16(&mut a, ADCSR, ADST);
        finish(&mut a, &clock);
        assert_eq!(r16(&a, addr(0)), 4095);
    }

    #[test]
    fn clearing_adst_aborts_and_the_stale_event_is_dropped() {
        let (mut a, _clock) = adc_at(0);
        w16(&mut a, ADANSA0, 1);
        w16(&mut a, ADCSR, ADST);
        let token = a.generation;
        w16(&mut a, ADCSR, 0);
        assert!(a.armed.is_none());
        assert_ne!(a.generation, token);
        assert!(!a.legacy_tick_active());
    }

    #[test]
    fn software_start_converts_with_trge_set() {
        // FSP R_ADC_ScanStart writes GBADIE | TRGE | ADST.
        let (mut a, clock) = adc_at(0);
        a.set_channel_input(0, 3300);
        w16(&mut a, ADANSA0, 1);
        w16(&mut a, ADCSR, ADST | TRGE | (1 << 6));
        assert_eq!(finish(&mut a, &clock), vec![EVENT]);
        assert_eq!(r16(&a, addr(0)), 4095);
    }

    #[test]
    fn group_scan_start_converts_nothing() {
        let (mut a, _clock) = adc_at(0);
        w16(&mut a, ADANSA0, 1);
        w16(&mut a, ADCSR, ADST | (1 << ADCS_SHIFT));
        assert!(a.armed.is_none());
    }

    #[test]
    fn continuous_scan_rearms_and_keeps_adst() {
        let (mut a, clock) = adc_at(0);
        w16(&mut a, ADANSA0, 1);
        w16(&mut a, ADCSR, ADST | (ADCS_CONTINUOUS << ADCS_SHIFT));
        assert_eq!(finish(&mut a, &clock), vec![EVENT]);
        assert_eq!(r16(&a, ADCSR) & ADST, ADST);
        assert_eq!(a.armed, Some(2));
        assert_eq!(finish(&mut a, &clock), vec![EVENT]);
    }

    #[test]
    fn scheduler_mode_queues_one_wake_per_scan() {
        let (mut a, _clock) = adc_at(0);
        w16(&mut a, ADANSA0, 0b11);
        w16(&mut a, ADCSR, ADST);
        let events = a.take_scheduled_events();
        if a.scheduler_mode() {
            assert_eq!(events, vec![(2, a.generation)]);
        } else {
            assert!(events.is_empty());
            assert!(a.legacy_tick_active());
        }
    }

    #[test]
    fn addr_registers_are_read_only_and_inputs_seam() {
        let (mut a, _clock) = adc_at(0);
        w16(&mut a, addr(3), 0x1234);
        assert_eq!(r16(&a, addr(3)), 0);
        assert!(a.set_adc_channel_input(28, 100));
        assert!(!a.set_adc_channel_input(29, 100));
        assert_eq!(a.adc_channel_count(), Some(29));
    }
}
