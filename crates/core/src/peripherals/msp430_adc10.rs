// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Texas Instruments MSP430x2xx ADC10 (family user's guide SLAU144,
//! chapter 22).
//!
//! [`Msp430Adc10`] (`msp430_adc10`): `ADC10CTL0` at offset 0, `ADC10CTL1`
//! at 2, `ADC10MEM` at 4 (0x01B0 on every x2xx part). `ADC10AE0`/`ADC10AE1`
//! (0x004A/0x004B) only disconnect the port buffers, so a descriptor maps
//! them as `msp430_bytes` storage.
//!
//! Conversion (22.2.1, 22.2.5, 22.2.6):
//!
//! * `NADC = 1023 * (VIN - VR-) / (VR+ - VR-)`, rounded to the nearest
//!   code, 0x3FF at or above `VR+` and 0 at or below `VR-`. `ADC10DF` = 1
//!   gives the result left-justified in two's complement.
//! * `SREFx` selects `VR+`: VCC (0, 4), the internal reference (1, 5;
//!   1.5 V, or 2.5 V with `REF2_5V`) or VeREF+ (2, 3, 6, 7, the A4 input
//!   level). `SREFx` 4 to 7 take `VR-` from the A3 input level, else VSS.
//! * `INCHx` 0-7 are the A0-A7 inputs (an input the descriptor does not
//!   bond, or nobody drives, is 0 mV); 8 is VeREF+ (A4), 9 is `VR-`, 10 is
//!   the temperature sensor, `0.00355 * T + 0.986` V (Figure 22-13, `T`
//!   the ambient temperature, 25 °C until a board sets one), 11-15 are
//!   VCC / 2.
//! * A conversion starts on `ADC10SC` (`SHSx` = 0) with `ENC` and
//!   `ADC10ON` set; `ADC10SC` reads 0. It takes the sample time (`ADC10SHTx`:
//!   4, 8, 16 or 64 ADC10CLK) plus 13 ADC10CLK, then writes `ADC10MEM` and
//!   sets `ADC10IFG`. `ADC10BUSY` reads 1 from the start to the result.
//! * `CONSEQx` 1 converts `INCHx` down to A0, 2 repeats `INCHx`, 3 repeats
//!   the `INCHx`..A0 sequence. With `MSC` the next conversion follows the
//!   result at once; without it each waits for the next `ADC10SC`.
//! * Clearing `ENC` stops `CONSEQx` 0 at once with no result, `CONSEQx` 2
//!   after the conversion in progress and `CONSEQx` 1 and 3 at the end of
//!   the sequence (22.2.6.6). `CONSEQx` 0 with `ENC` clear stops any mode
//!   at once.
//! * Writes to the `ENC`-protected bits (`ADC10CTL0` 15-4 and 7,
//!   `ADC10CTL1`) are ignored while `ENC` is set.
//!
//! Clocks: `ADC10SSELx` 0 is ADC10OSC (`adc10osc_hz`), 1 ACLK (`aclk_hz`),
//! 2 MCLK and 3 SMCLK (one CPU cycle per clock, as Timer_A), divided by
//! `ADC10DIVx` + 1. ADC10OSC and ACLK count against `cpu_hz` CPU cycles per
//! second; with no `cpu_hz` they also take one CPU cycle per clock.
//!
//! Interrupt: `ADC10IFG` is the descriptor's `irq_source` on line
//! `irq_line`, alone on its vector, so acceptance clears it; `ADC10IE` is
//! its enable.
//!
//! Limits:
//!
//! * The data transfer controller is not modelled (`ADC10DTC0`,
//!   `ADC10DTC1`, `ADC10SA` sit outside this window).
//! * `SHSx` 1-3 (Timer_A output units) never trigger: the Timer_A output
//!   units drive no signal. `ISSH` is storage.
//! * The reference has no settling time and does not depend on `REFON`;
//!   `REFOUT`, `REFBURST` and `ADC10SR` are storage. The synchronisation
//!   time `tsync` is not added. The conversion has no noise and no offset,
//!   gain or linearity error.

use crate::peripherals::msp430::{Msp430Irq, SOURCES};
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::sync::Arc;

const CTL0: u64 = 0x00;
const CTL1: u64 = 0x02;
const MEM: u64 = 0x04;

const ADC10SC: u16 = 0x0001;
const ENC: u16 = 0x0002;
const ADC10IFG: u16 = 0x0004;
const ADC10IE: u16 = 0x0008;
const ADC10ON: u16 = 0x0010;
const REF2_5V: u16 = 0x0040;
const MSC: u16 = 0x0080;
/// `ADC10CTL0` bits that only change while `ENC` is clear.
const CTL0_PROTECTED: u16 = 0xFFF0;
/// `ADC10CTL1` bits stored: all but `ADC10BUSY`.
const CTL1_STORED: u16 = 0xFFFE;
const ADC10DF: u16 = 0x0200;
const ADC10BUSY: u16 = 0x0001;

/// Sample-and-hold times, in ADC10CLK, by `ADC10SHTx`.
const SAMPLE_CLOCKS: [u64; 4] = [4, 8, 16, 64];
/// ADC10CLK cycles from the end of sampling to the result.
const CONVERT_CLOCKS: u64 = 13;

/// External inputs A0-A7.
pub const EXTERNAL_INPUTS: u8 = 8;

/// The interrupt source and request line of `ADC10IFG`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Adc10Request {
    pub source: u32,
    pub line: u32,
}

/// The descriptor's analog and clock facts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adc10Config {
    /// Bonded external inputs: A0 .. A(`inputs` - 1), at most 8.
    pub inputs: u8,
    /// Supply voltage, millivolts.
    pub vcc_mv: u32,
    pub adc10osc_hz: u64,
    pub aclk_hz: u64,
    /// CPU cycles per second; 0 counts every clock as one CPU cycle.
    pub cpu_hz: u64,
}

#[derive(Debug)]
pub struct Msp430Adc10 {
    request: Option<Adc10Request>,
    config: Adc10Config,
    irq: Arc<Msp430Irq>,
    /// `ADC10CTL0` without `ADC10SC` and `ADC10IFG`.
    ctl0: u16,
    /// `ADC10CTL1` without `ADC10BUSY`.
    ctl1: u16,
    mem: u16,
    /// `ADC10IFG` while no interrupt source is routed.
    ifg: bool,
    inputs: [u16; EXTERNAL_INPUTS as usize],
    temperature_c: f64,
    /// CPU cycles left to the result of the conversion in progress.
    converting: Option<u64>,
    /// The channel of the conversion in progress, or the next one.
    channel: u8,
    /// A sequence or repeat mode has more conversions to make.
    active: bool,
}

impl Msp430Adc10 {
    pub fn new(request: Option<Adc10Request>, config: Adc10Config) -> Self {
        Self {
            request: request.filter(|r| r.source < SOURCES && r.line < 64),
            config: Adc10Config {
                inputs: config.inputs.min(EXTERNAL_INPUTS),
                ..config
            },
            irq: Msp430Irq::new(),
            ctl0: 0,
            ctl1: 0,
            mem: 0,
            ifg: false,
            inputs: [0; EXTERNAL_INPUTS as usize],
            temperature_c: 25.0,
            converting: None,
            channel: 0,
            active: false,
        }
    }

    fn flag(&self) -> bool {
        match self.request {
            Some(r) => self.irq.flag(r.source),
            None => self.ifg,
        }
    }

    fn set_flag(&mut self, on: bool) {
        match self.request {
            Some(r) => self.irq.set_flag(r.source, on),
            None => self.ifg = on,
        }
    }

    fn sync_enable(&self) {
        if let Some(r) = self.request {
            self.irq.set_enable(r.source, self.ctl0 & ADC10IE != 0);
        }
    }

    fn inch(&self) -> u8 {
        (self.ctl1 >> 12) as u8
    }

    fn conseq(&self) -> u16 {
        (self.ctl1 >> 1) & 3
    }

    fn input_mv(&self, x: u8) -> f64 {
        if x < self.config.inputs {
            f64::from(self.inputs[x as usize])
        } else {
            0.0
        }
    }

    /// `VR+` and `VR-`, millivolts.
    fn references(&self) -> (f64, f64) {
        let vcc = f64::from(self.config.vcc_mv);
        let sref = self.ctl0 >> 13;
        let high = match sref & 3 {
            0 => vcc,
            1 if self.ctl0 & REF2_5V != 0 => 2500.0,
            1 => 1500.0,
            _ => self.input_mv(4),
        };
        let low = if sref & 4 != 0 { self.input_mv(3) } else { 0.0 };
        (high, low)
    }

    fn channel_mv(&self, channel: u8) -> f64 {
        match channel {
            0..=7 => self.input_mv(channel),
            8 => self.input_mv(4),
            9 => self.references().1,
            10 => 3.55 * self.temperature_c + 986.0,
            _ => f64::from(self.config.vcc_mv) / 2.0,
        }
    }

    /// The `ADC10MEM` value of a conversion of `channel` now.
    fn convert(&self, channel: u8) -> u16 {
        let (high, low) = self.references();
        let vin = self.channel_mv(channel);
        let code = if vin >= high {
            1023
        } else if vin <= low {
            0
        } else {
            (1023.0 * (vin - low) / (high - low)).round() as i32
        };
        if self.ctl1 & ADC10DF != 0 {
            ((code - 512) << 6) as u16
        } else {
            code as u16
        }
    }

    /// CPU cycles from the start of a conversion to its result.
    fn conversion_cycles(&self) -> u64 {
        let clocks = SAMPLE_CLOCKS[((self.ctl0 >> 11) & 3) as usize] + CONVERT_CLOCKS;
        let clocks = clocks * (u64::from((self.ctl1 >> 5) & 7) + 1);
        let source_hz = match (self.ctl1 >> 3) & 3 {
            0 => self.config.adc10osc_hz,
            1 => self.config.aclk_hz,
            _ => 0,
        };
        if source_hz == 0 || self.config.cpu_hz == 0 {
            clocks
        } else {
            (clocks * self.config.cpu_hz).div_ceil(source_hz).max(1)
        }
    }

    fn start(&mut self) {
        self.converting = Some(self.conversion_cycles());
    }

    fn stop(&mut self) {
        self.converting = None;
        self.active = false;
    }

    /// A rising edge of the sample input (`ADC10SC`).
    fn trigger(&mut self) {
        let ready = self.ctl0 & (ENC | ADC10ON) == (ENC | ADC10ON);
        if !ready || (self.ctl1 >> 10) & 3 != 0 || self.converting.is_some() {
            return;
        }
        let msc_running = self.ctl0 & MSC != 0 && self.conseq() != 0 && self.active;
        if msc_running {
            return;
        }
        if !self.active {
            self.channel = self.inch();
            self.active = self.conseq() != 0;
        }
        self.start();
    }

    /// The conversion in progress has its result.
    fn complete(&mut self) {
        self.converting = None;
        self.mem = self.convert(self.channel);
        self.set_flag(true);
        let enc = self.ctl0 & ENC != 0;
        let msc = self.ctl0 & MSC != 0;
        match self.conseq() {
            0 => self.active = false,
            2 => {
                if !enc {
                    self.active = false;
                } else if msc {
                    self.start();
                }
            }
            seq => {
                if self.channel == 0 {
                    if seq == 1 || !enc {
                        self.active = false;
                        return;
                    }
                    self.channel = self.inch();
                } else {
                    self.channel -= 1;
                }
                if msc {
                    self.start();
                } else if !enc {
                    // No trigger can arrive with `ENC` clear.
                    self.active = false;
                }
            }
        }
    }

    fn word(&self, offset: u64) -> u16 {
        match offset & !1 {
            CTL0 => self.ctl0 | if self.flag() { ADC10IFG } else { 0 },
            CTL1 if self.converting.is_some() => self.ctl1 | ADC10BUSY,
            CTL1 => self.ctl1,
            MEM => self.mem,
            _ => 0,
        }
    }

    fn set_word(&mut self, offset: u64, value: u16) {
        match offset & !1 {
            CTL0 => {
                let was_enc = self.ctl0 & ENC != 0;
                let kept = if was_enc { CTL0_PROTECTED } else { 0 };
                self.ctl0 = (self.ctl0 & kept) | (value & !kept & !(ADC10SC | ADC10IFG));
                self.set_flag(value & ADC10IFG != 0);
                self.sync_enable();
                if was_enc && self.ctl0 & ENC == 0 {
                    match self.conseq() {
                        0 => self.stop(),
                        // Modes 1-3 finish below; with no conversion in
                        // progress and no `MSC`, nothing can trigger again.
                        _ if self.converting.is_none() => self.active = false,
                        _ => {}
                    }
                }
                if self.ctl0 & ADC10ON == 0 {
                    self.stop();
                }
                if value & ADC10SC != 0 {
                    self.trigger();
                }
            }
            CTL1 if self.ctl0 & ENC == 0 => {
                self.ctl1 = value & CTL1_STORED;
                if self.conseq() == 0 {
                    self.stop();
                }
            }
            _ => {}
        }
    }
}

impl Peripheral for Msp430Adc10 {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.word(offset) >> (8 * (offset & 1))) as u8)
    }

    /// A byte store writes the low byte and clears the upper one.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset & 1 == 0 {
            self.set_word(offset, u16::from(value));
        }
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok(self.word(offset))
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.set_word(offset, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn legacy_tick_active(&self) -> bool {
        self.converting.is_some()
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, mut cycles: u64) -> PeripheralTickResult {
        while let Some(left) = self.converting {
            if cycles < left {
                self.converting = Some(left - cycles);
                break;
            }
            cycles -= left;
            self.complete();
        }
        PeripheralTickResult::default()
    }

    fn attach_msp430_irq(&mut self, irq: &Arc<Msp430Irq>) {
        if let Some(r) = self.request {
            irq.route(r.source, r.line, true);
        }
        self.irq = irq.clone();
        self.on_system_reset();
    }

    fn on_system_reset(&mut self) {
        self.ctl0 = 0;
        self.ctl1 = 0;
        self.converting = None;
        self.channel = 0;
        self.active = false;
        self.set_flag(false);
        self.sync_enable();
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        if channel >= self.config.inputs {
            return false;
        }
        self.inputs[channel as usize] = millivolts;
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        self.set_adc_channel_input(channel, 0)
    }

    fn adc_channel_count(&self) -> Option<u8> {
        Some(self.config.inputs)
    }

    fn set_ambient_temperature_c(&mut self, celsius: f64) -> bool {
        if !celsius.is_finite() {
            return false;
        }
        self.temperature_c = celsius;
        true
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "adc10ctl0": self.word(CTL0),
            "adc10ctl1": self.word(CTL1),
            "adc10mem": self.mem,
            "channel": self.channel,
            "active": self.active,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: u32 = 34;
    const LINE: u32 = 10;

    fn adc() -> Msp430Adc10 {
        let mut adc = Msp430Adc10::new(
            Some(Adc10Request {
                source: SOURCE,
                line: LINE,
            }),
            Adc10Config {
                inputs: 8,
                vcc_mv: 3000,
                adc10osc_hz: 5_000_000,
                aclk_hz: 32_768,
                cpu_hz: 1_000_000,
            },
        );
        adc.attach_msp430_irq(&Msp430Irq::new());
        adc
    }

    fn w(adc: &mut Msp430Adc10, offset: u64, value: u16) {
        adc.write_u16(offset, value).unwrap();
    }

    fn r(adc: &Msp430Adc10, offset: u64) -> u16 {
        adc.read_u16(offset).unwrap()
    }

    /// Run until `ADC10BUSY` clears; the CPU cycles it took.
    fn run(adc: &mut Msp430Adc10) -> u64 {
        let mut n = 0;
        while r(adc, CTL1) & ADC10BUSY != 0 {
            adc.tick_elapsed(1);
            n += 1;
            assert!(n < 100_000, "conversion never ends");
        }
        n
    }

    #[test]
    fn single_conversion_against_vcc_takes_sample_plus_13_clocks() {
        let mut adc = adc();
        assert!(adc.set_adc_channel_input(1, 1500));
        // INCH = A1, SMCLK (one CPU cycle per clock), single conversion.
        w(&mut adc, CTL1, 0x1000 | 3 << 3);
        w(&mut adc, CTL0, ADC10ON | ADC10IE);
        w(&mut adc, CTL0, ADC10ON | ADC10IE | ENC | ADC10SC);
        assert_eq!(r(&adc, CTL0) & ADC10SC, 0, "ADC10SC resets itself");
        assert_eq!(run(&mut adc), 4 + 13);
        // 1023 * 1500 / 3000 = 511.5, rounded.
        assert_eq!(r(&adc, MEM), 512);
        assert_ne!(r(&adc, CTL0) & ADC10IFG, 0);
        assert_eq!(adc.irq.pending(), 1 << LINE);
    }

    #[test]
    fn adc10osc_divider_and_sample_time_set_the_duration() {
        let mut adc = adc();
        // ADC10OSC 5 MHz against a 1 MHz CPU, /2, 64-clock sample.
        w(&mut adc, CTL1, 1 << 5);
        w(&mut adc, CTL0, 3 << 11 | ADC10ON | ENC | ADC10SC);
        // (64 + 13) * 2 = 154 clocks = 30.8 us -> 31 CPU cycles.
        assert_eq!(run(&mut adc), 31);
    }

    #[test]
    fn internal_reference_temperature_and_half_vcc() {
        let mut adc = adc();
        assert!(adc.set_ambient_temperature_c(30.0));
        // SREF = 1, REFON, 1.5 V; INCH = 10.
        w(&mut adc, CTL1, 0xA000 | 3 << 3);
        w(&mut adc, CTL0, 1 << 13 | 0x0020 | ADC10ON | ENC | ADC10SC);
        run(&mut adc);
        // (3.55 * 30 + 986) mV of 1500 mV = 745.1 -> 745.
        assert_eq!(r(&adc, MEM), 745);
        w(&mut adc, CTL0, 1 << 13 | 0x0020 | ADC10ON);
        // INCH = 11: VCC / 2 = 1500 mV against 2.5 V.
        w(&mut adc, CTL1, 0xB000 | 3 << 3);
        w(&mut adc, CTL0, 1 << 13 | 0x0060 | ADC10ON | ENC | ADC10SC);
        run(&mut adc);
        assert_eq!(r(&adc, MEM), 614);
        // At or above VR+ reads full scale.
        w(&mut adc, CTL0, 1 << 13 | 0x0020 | ADC10ON);
        w(&mut adc, CTL0, 1 << 13 | 0x0020 | ADC10ON | ENC | ADC10SC);
        run(&mut adc);
        assert_eq!(r(&adc, MEM), 0x3FF);
    }

    #[test]
    fn twos_complement_is_left_justified() {
        let mut adc = adc();
        adc.set_adc_channel_input(0, 0);
        w(&mut adc, CTL1, ADC10DF | 3 << 3);
        w(&mut adc, CTL0, ADC10ON | ENC | ADC10SC);
        run(&mut adc);
        assert_eq!(r(&adc, MEM), 0x8000);
    }

    #[test]
    fn sequence_converts_inch_down_to_a0_with_msc() {
        let mut adc = adc();
        for x in 0..3u8 {
            adc.set_adc_channel_input(x, 1000 * u16::from(x));
        }
        // INCH = A2, sequence (CONSEQ 1), SMCLK.
        w(&mut adc, CTL1, 0x2000 | 3 << 3 | 1 << 1);
        w(&mut adc, CTL0, MSC | ADC10ON | ENC | ADC10SC);
        let mut results = Vec::new();
        for _ in 0..3 * 17 {
            adc.tick_elapsed(1);
            if r(&adc, CTL0) & ADC10IFG != 0 {
                results.push(r(&adc, MEM));
                w(&mut adc, CTL0, MSC | ADC10ON | ENC);
            }
        }
        assert_eq!(results, vec![682, 341, 0]);
        assert_eq!(r(&adc, CTL1) & ADC10BUSY, 0);
        assert!(!adc.active);
    }

    #[test]
    fn sequence_without_msc_waits_for_each_trigger() {
        let mut adc = adc();
        w(&mut adc, CTL1, 0x1000 | 3 << 3 | 1 << 1);
        w(&mut adc, CTL0, ADC10ON | ENC | ADC10SC);
        run(&mut adc);
        assert_eq!(adc.channel, 0);
        assert!(adc.active);
        adc.tick_elapsed(100);
        assert_eq!(
            r(&adc, CTL1) & ADC10BUSY,
            0,
            "no conversion without a trigger"
        );
        w(&mut adc, CTL0, ADC10ON | ENC | ADC10SC);
        assert_eq!(run(&mut adc), 17);
        assert!(!adc.active);
    }

    #[test]
    fn enc_rules_protect_bits_and_stop_conversions() {
        let mut adc = adc();
        w(&mut adc, CTL1, 0x5000 | 3 << 3);
        w(&mut adc, CTL0, ADC10ON | ENC);
        // ADC10CTL1 and the protected ADC10CTL0 bits hold while ENC is set.
        w(&mut adc, CTL1, 0x7000);
        assert_eq!(r(&adc, CTL1) >> 12, 5);
        w(&mut adc, CTL0, 3 << 11 | ADC10ON | ENC);
        assert_eq!(r(&adc, CTL0) & (3 << 11), 0);
        // Single conversion: clearing ENC stops it with no result.
        w(&mut adc, CTL0, ADC10ON | ENC | ADC10SC);
        adc.tick_elapsed(3);
        w(&mut adc, CTL0, ADC10ON);
        assert_eq!(r(&adc, CTL1) & ADC10BUSY, 0);
        adc.tick_elapsed(100);
        assert_eq!(r(&adc, CTL0) & ADC10IFG, 0);
        // Repeat single channel: clearing ENC ends after the conversion.
        w(&mut adc, CTL1, 0x5000 | 3 << 3 | 2 << 1);
        w(&mut adc, CTL0, MSC | ADC10ON | ENC | ADC10SC);
        adc.tick_elapsed(17 * 3 + 5);
        w(&mut adc, CTL0, MSC | ADC10ON);
        assert_ne!(r(&adc, CTL1) & ADC10BUSY, 0);
        assert_eq!(run(&mut adc), 12);
        adc.tick_elapsed(100);
        assert_eq!(r(&adc, CTL1) & ADC10BUSY, 0);
    }

    #[test]
    fn no_trigger_without_adc10on_or_from_timer_sources() {
        let mut adc = adc();
        w(&mut adc, CTL0, ENC | ADC10SC);
        assert_eq!(r(&adc, CTL1) & ADC10BUSY, 0);
        w(&mut adc, CTL0, 0);
        w(&mut adc, CTL1, 1 << 10);
        w(&mut adc, CTL0, ADC10ON | ENC | ADC10SC);
        assert_eq!(r(&adc, CTL1) & ADC10BUSY, 0);
    }

    #[test]
    fn unbonded_inputs_are_refused_and_read_zero() {
        let mut adc = Msp430Adc10::new(
            None,
            Adc10Config {
                inputs: 2,
                vcc_mv: 3000,
                adc10osc_hz: 0,
                aclk_hz: 0,
                cpu_hz: 0,
            },
        );
        assert_eq!(adc.adc_channel_count(), Some(2));
        assert!(!adc.set_adc_channel_input(2, 3000));
        w(&mut adc, CTL1, 0x2000);
        w(&mut adc, CTL0, ADC10ON | ENC | ADC10SC);
        assert_eq!(run(&mut adc), 17);
        assert_eq!(r(&adc, MEM), 0);
        // With no routed source the flag is local and firmware clears it.
        assert_ne!(r(&adc, CTL0) & ADC10IFG, 0);
        w(&mut adc, CTL0, ADC10ON);
        assert_eq!(r(&adc, CTL0) & ADC10IFG, 0);
    }
}
