// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 ADC + temperature sensor (datasheet §4.9, base `0x4004C000`).
//!
//! A 12-bit SAR converter with five inputs — 0..3 are GPIO26..29, input 4 is
//! the on-die temperature sensor — an 8-entry sample FIFO, a round-robin
//! sequencer and one interrupt, `ADC_IRQ_FIFO` (NVIC 22). Nine registers:
//! `CS`, `RESULT`, `FCS`, `FIFO`, `DIV`, `INTR`, `INTE`, `INTF`, `INTS`.
//! Offsets, field positions and the interrupt number are taken from the
//! vendored SVD (`tests/fixtures/real_world/rp2040.svd`).
//!
//! ## What silicon does, and what this models
//!
//! * **`CS.EN` then `CS.READY`.** `READY` reads 0 out of reset (matching the
//!   SVD's all-zero `CS`) and asserts once the converter is powered and idle —
//!   which is what makes `pico-sdk`'s `adc_init()` poll terminate.
//! * **`START_ONCE` / `START_MANY`.** A write to `START_ONCE` (write-only,
//!   self-clearing) converts `CS.AINSEL` once. `START_MANY` free-runs, paced by
//!   `DIV`: `1 + INT + FRAC/256` clocks per sample, with `DIV = 0` meaning
//!   back-to-back. The fractional part is a real 8-bit phase accumulator, so
//!   `DIV = 2.5` genuinely alternates between 2- and 3-clock gaps.
//! * **The round-robin sequencer.** After each conversion `AINSEL` advances to
//!   the next input selected in `CS.RROBIN` (5 bits — one per input), wrapping.
//!   With `RROBIN = 0` `AINSEL` stays put.
//! * **The FIFO.** With `FCS.EN` every conversion is pushed. `LEVEL` tracks
//!   occupancy, `EMPTY`/`FULL` follow it, a push into a full FIFO sets `OVER`
//!   and drops the sample, a pop from an empty FIFO sets `UNDER` and reads 0.
//!   `FCS.SHIFT` right-shifts each entry by 4 to the byte-wide form the DMA
//!   byte path consumes. `OVER`/`UNDER`/`ERR` are write-1-to-clear.
//! * **`INTR.FIFO` is a level, not a latch.** It is read-only and reads
//!   `LEVEL >= FCS.THRESH` — there is nothing to acknowledge, which is why
//!   `pico-sdk`'s FIFO ISR drains the FIFO instead of writing an INTR register.
//!   `ADC_IRQ_FIFO` is held while `(INTR | INTF) & INTE`, matching the timer and
//!   PWM models.
//! * **The temperature sensor** (input 4, `CS.TS_EN`) converts the datasheet's
//!   transfer function rather than a magic constant:
//!   `T = 27 - (V - 0.706) / 0.001721`, i.e. 0.706 V at 27 °C. See
//!   [`temp_sensor_microvolts`] and [`code_from_microvolts`] — the ambient is a
//!   named constant and the volts-to-code conversion is the same 12-bit /
//!   3.3 V scale `pico-sdk`'s `conversion_factor` uses. With `TS_EN` clear the
//!   sensor is unpowered and converts 0, as an unbiased input does on silicon.
//!
//! ## Deliberately not modelled
//!
//! * **Inputs 0..3 have no analog source of their own.** Nothing is wired to
//!   GPIO26..29 in a bare simulation, so they convert 0 until something drives
//!   them. They *are* drivable: [`Rp2040Adc::set_channel_input`] is the same
//!   seam `SystemBus::seed_adc_channel` uses for the STM32 and ESP32-S3 ADCs,
//!   so a potentiometer or thermistor component wired in `system.yaml`, an MCP
//!   `set_input`, or a test all move a real level onto a real channel. What is
//!   *not* modelled is any invented default — a fabricated mid-scale reading
//!   would look like a working sensor that is not there.
//! * **Conversion errors.** `CS.ERR`, `CS.ERR_STICKY` and `FIFO.ERR` read 0:
//!   this model has no source of conversion error, so it never claims one.
//!   `ERR_STICKY` still honours its write-1-to-clear contract.
//! * **Conversion latency.** A conversion completes within the tick that starts
//!   it (96 ADC clocks on silicon). Firmware that polls `READY` still works;
//!   firmware measuring conversion *time* would not see the real 2 µs.
//! * **`FCS.DREQ_EN`.** Stored and read back, but no DREQ is raised — the DMA
//!   model has no paced-transfer (DREQ) path yet, so a channel wired to
//!   `DREQ_ADC` would not move. Draining the FIFO from the CPU works.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

// Register offsets (relative to the ADC base) — SVD-verified.
const CS: u64 = 0x00;
const RESULT: u64 = 0x04;
const FCS: u64 = 0x08;
const FIFO: u64 = 0x0C;
const DIV: u64 = 0x10;
const INTR: u64 = 0x14;
const INTE: u64 = 0x18;
const INTF: u64 = 0x1C;
const INTS: u64 = 0x20;

// CS fields (SVD: RROBIN[20:16], AINSEL[14:12], ERR_STICKY[10], ERR[9],
// READY[8], START_MANY[3], START_ONCE[2], TS_EN[1], EN[0]).
const CS_EN: u32 = 1 << 0;
const CS_TS_EN: u32 = 1 << 1;
const CS_START_ONCE: u32 = 1 << 2;
const CS_START_MANY: u32 = 1 << 3;
const CS_READY: u32 = 1 << 8;
const CS_ERR_STICKY: u32 = 1 << 10;
const CS_AINSEL_SHIFT: u32 = 12;
const CS_AINSEL_MASK: u32 = 0x7;
const CS_RROBIN_SHIFT: u32 = 16;
const CS_RROBIN_MASK: u32 = 0x1F;

// FCS fields (SVD: THRESH[27:24], LEVEL[19:16], OVER[11], UNDER[10], FULL[9],
// EMPTY[8], DREQ_EN[3], ERR[2], SHIFT[1], EN[0]).
const FCS_EN: u32 = 1 << 0;
const FCS_SHIFT_BIT: u32 = 1 << 1;
const FCS_ERR: u32 = 1 << 2;
const FCS_DREQ_EN: u32 = 1 << 3;
const FCS_EMPTY: u32 = 1 << 8;
const FCS_FULL: u32 = 1 << 9;
const FCS_UNDER: u32 = 1 << 10;
const FCS_OVER: u32 = 1 << 11;
const FCS_LEVEL_SHIFT: u32 = 16;
const FCS_THRESH_SHIFT: u32 = 24;
const FCS_THRESH_MASK: u32 = 0xF;

/// `ADC_IRQ_FIFO` (SVD interrupt table).
const ADC_IRQ_FIFO: u32 = 22;

/// Inputs 0..3 (GPIO26..29) plus the on-die temperature sensor on input 4.
const INPUTS: usize = 5;
const TEMP_SENSOR_INPUT: usize = 4;
/// Sample FIFO depth (datasheet §4.9.2.5).
const FIFO_DEPTH: usize = 8;

/// ADC reference voltage, in microvolts. The RP2040's `ADC_VREF` is tied to
/// 3.3 V on every Pico-class board, and it is the scale `pico-sdk`'s
/// `conversion_factor = 3.3f / (1 << 12)` assumes.
const VREF_UV: i64 = 3_300_000;
/// Full-scale code count for the 12-bit SAR.
const FULL_SCALE: i64 = 1 << 12;

// ── Temperature sensor transfer function (datasheet §4.9.5) ─────────────────
// "T = 27 - (ADC_voltage - 0.706) / 0.001721". Kept as three named constants so
// the relation stays visible instead of collapsing into a magic code.
/// Sensor output at the reference temperature, in microvolts.
const TS_V_AT_REF_UV: i64 = 706_000;
/// Reference temperature for that voltage, in millidegrees Celsius.
const TS_REF_MILLI_C: i64 = 27_000;
/// Slope, in nanovolts per degree Celsius (0.001721 V/°C). The sensor output
/// FALLS as temperature rises.
const TS_SLOPE_NV_PER_C: i64 = 1_721_000;
/// Die temperature the model reports, in millidegrees Celsius. There is no
/// thermal model, so this is a fixed ambient; drive input 4 through
/// [`Rp2040Adc::set_channel_input`] to convert some other voltage.
const TS_DIE_MILLI_C: i64 = 27_000;

/// Sensor output in microvolts for a die temperature in millidegrees Celsius,
/// inverting the datasheet relation above.
fn temp_sensor_microvolts(milli_c: i64) -> i64 {
    TS_V_AT_REF_UV - (milli_c - TS_REF_MILLI_C) * TS_SLOPE_NV_PER_C / 1_000_000
}

/// 12-bit code for a level in microvolts, clamped to the converter's range.
fn code_from_microvolts(uv: i64) -> u16 {
    (uv.clamp(0, VREF_UV) * FULL_SCALE / VREF_UV).min(FULL_SCALE - 1) as u16
}

#[derive(Debug)]
pub struct Rp2040Adc {
    /// `CS` writable bits: EN, TS_EN, START_MANY, AINSEL, RROBIN.
    /// READY/ERR are derived; START_ONCE and ERR_STICKY are handled on write.
    cs: u32,
    err_sticky: bool,
    result: u16,
    /// `FCS` writable bits: EN, SHIFT, DREQ_EN, THRESH (plus the sticky
    /// OVER/UNDER/ERR flags tracked separately).
    fcs: u32,
    over: bool,
    /// The sample FIFO is read-to-drain and `UNDER` is set by a read, so both
    /// live behind interior mutability: the bus read path is `&self` but
    /// reading `FIFO` must pop (the same shape the PL022 SPI model uses).
    under: Cell<bool>,
    fifo: RefCell<VecDeque<u16>>,
    div: u32,
    /// Free-running pacer accumulator, in 1/256 clocks.
    div_acc: u32,
    inte: u32,
    intf: u32,
    /// Per-input level in microvolts. Input 4 starts at the temperature
    /// sensor's output for [`TS_DIE_MILLI_C`]; inputs 0..3 start unbiased.
    inputs_uv: [i64; INPUTS],
    /// Bus-published cycle clock (event-scheduler). When present the model is
    /// walk-independent: free-running START_MANY rides scheduled events.
    clock: Option<CycleClock>,
    /// Bumped each arm so stale conversion events die on arrival.
    arm_seq: u32,
}

impl Default for Rp2040Adc {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2040Adc {
    pub fn new() -> Self {
        let mut inputs_uv = [0i64; INPUTS];
        inputs_uv[TEMP_SENSOR_INPUT] = temp_sensor_microvolts(TS_DIE_MILLI_C);
        Self {
            cs: 0,
            err_sticky: false,
            result: 0,
            fcs: 0,
            over: false,
            under: Cell::new(false),
            fifo: RefCell::new(VecDeque::with_capacity(FIFO_DEPTH)),
            div: 0,
            div_acc: 0,
            inte: 0,
            intf: 0,
            inputs_uv,
            clock: None,
            arm_seq: 0,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn free_running(&self) -> bool {
        self.cs & CS_EN != 0 && self.cs & CS_START_MANY != 0
    }

    fn needs_scheduler_wake(&self) -> bool {
        self.free_running() || self.ints() != 0
    }

    /// One peripheral-tick of free-running conversion + level IRQ vector.
    fn step_one(&mut self) -> PeripheralTickResult {
        if self.free_running() {
            // DIV pacing: `1 + INT + FRAC/256` clocks per sample, DIV = 0 being
            // back-to-back. Accumulate in 1/256 clocks so the fraction is real.
            let period = if self.div == 0 { 256 } else { 256 + self.div };
            self.div_acc += 256;
            if self.div_acc >= period {
                self.div_acc -= period;
                self.convert();
            }
        }
        let explicit_irqs = (self.ints() != 0).then(|| vec![ADC_IRQ_FIFO]);
        PeripheralTickResult {
            explicit_irqs,
            ..Default::default()
        }
    }

    /// Drive `channel` (0..3 = GPIO26..29, 4 = the temperature-sensor input)
    /// with an analog level. This is the seam
    /// [`crate::bus::SystemBus::seed_adc_channel`] uses, so a component in
    /// `system.yaml`, an MCP `set_input` and a unit test all reach the same
    /// state.
    pub fn set_channel_input(&mut self, channel: u8, millivolts: u16) {
        if (channel as usize) < INPUTS {
            self.inputs_uv[channel as usize] = millivolts as i64 * 1_000;
        }
    }

    /// The 12-bit code the next conversion of `channel` would produce.
    pub fn channel_input_count(&self, channel: u8) -> u16 {
        self.sample(channel as usize)
    }

    fn ainsel(&self) -> usize {
        ((self.cs >> CS_AINSEL_SHIFT) & CS_AINSEL_MASK) as usize
    }

    fn rrobin(&self) -> u32 {
        (self.cs >> CS_RROBIN_SHIFT) & CS_RROBIN_MASK
    }

    fn thresh(&self) -> u32 {
        (self.fcs >> FCS_THRESH_SHIFT) & FCS_THRESH_MASK
    }

    /// Convert one input. An input above the implemented set, or the
    /// temperature sensor with `TS_EN` clear (unpowered), converts 0.
    fn sample(&self, input: usize) -> u16 {
        if input >= INPUTS {
            return 0;
        }
        if input == TEMP_SENSOR_INPUT && self.cs & CS_TS_EN == 0 {
            return 0;
        }
        code_from_microvolts(self.inputs_uv[input])
    }

    /// Advance `AINSEL` to the next input selected in `RROBIN`, wrapping.
    fn advance_round_robin(&mut self) {
        let mask = self.rrobin();
        if mask == 0 {
            return;
        }
        let start = self.ainsel();
        for step in 1..=INPUTS {
            let next = (start + step) % INPUTS;
            if mask & (1 << next) != 0 {
                self.cs = (self.cs & !(CS_AINSEL_MASK << CS_AINSEL_SHIFT))
                    | ((next as u32) << CS_AINSEL_SHIFT);
                return;
            }
        }
    }

    /// Run one conversion of the selected input: latch `RESULT`, push to the
    /// FIFO if enabled, then step the round-robin sequencer.
    fn convert(&mut self) {
        let code = self.sample(self.ainsel());
        self.result = code;
        if self.fcs & FCS_EN != 0 {
            let entry = if self.fcs & FCS_SHIFT_BIT != 0 {
                code >> 4
            } else {
                code
            };
            let mut fifo = self.fifo.borrow_mut();
            if fifo.len() >= FIFO_DEPTH {
                // A push into a full FIFO is dropped and flagged, as on silicon.
                self.over = true;
            } else {
                fifo.push_back(entry);
            }
            drop(fifo);
        }
        self.advance_round_robin();
    }

    /// `INTR.FIFO` — a level, not a latch: the FIFO has reached its threshold.
    fn intr(&self) -> u32 {
        u32::from(self.fifo.borrow().len() as u32 >= self.thresh())
    }

    fn ints(&self) -> u32 {
        (self.intr() | self.intf) & self.inte & 0x1
    }

    fn fcs_view(&self) -> u32 {
        let len = self.fifo.borrow().len();
        let mut v = self.fcs | ((len as u32) << FCS_LEVEL_SHIFT);
        if len == 0 {
            v |= FCS_EMPTY;
        }
        if len >= FIFO_DEPTH {
            v |= FCS_FULL;
        }
        if self.under.get() {
            v |= FCS_UNDER;
        }
        if self.over {
            v |= FCS_OVER;
        }
        v
    }

    /// The `FIFO` read port: pop the head, or flag `UNDER` and read 0.
    fn pop_fifo(&self) -> u32 {
        match self.fifo.borrow_mut().pop_front() {
            Some(v) => v as u32,
            None => {
                self.under.set(true);
                0
            }
        }
    }

    fn cs_view(&self) -> u32 {
        // READY: powered and idle. Conversions complete within their tick, so
        // an enabled converter is always ready; a disabled one never is, which
        // is what makes the reset value read back as the SVD's all-zero CS.
        let ready = if self.cs & CS_EN != 0 { CS_READY } else { 0 };
        let sticky = if self.err_sticky { CS_ERR_STICKY } else { 0 };
        self.cs | ready | sticky
    }
}

impl Peripheral for Rp2040Adc {
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            CS => self.cs_view(),
            RESULT => self.result as u32,
            FCS => self.fcs_view(),
            // Reading FIFO drains it; an empty pop flags UNDER and reads 0.
            FIFO => self.pop_fifo(),
            DIV => self.div,
            INTR => self.intr(),
            INTE => self.inte,
            INTF => self.intf,
            INTS => self.ints(),
            _ => {
                crate::census_reg!("rp2040.adc:Rp2040Adc", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            CS => {
                // ERR_STICKY is write-1-to-clear; START_ONCE is write-only and
                // self-clearing; READY and ERR are read-only.
                if value & CS_ERR_STICKY != 0 {
                    self.err_sticky = false;
                }
                self.cs = value
                    & (CS_EN
                        | CS_TS_EN
                        | CS_START_MANY
                        | (CS_AINSEL_MASK << CS_AINSEL_SHIFT)
                        | (CS_RROBIN_MASK << CS_RROBIN_SHIFT));
                if value & CS_START_ONCE != 0 && self.cs & CS_EN != 0 {
                    self.convert();
                }
            }
            // RESULT is read-only.
            RESULT => {}
            FCS => {
                // OVER / UNDER / ERR are write-1-to-clear; LEVEL / EMPTY / FULL
                // are read-only views of the FIFO.
                if value & FCS_OVER != 0 {
                    self.over = false;
                }
                if value & FCS_UNDER != 0 {
                    self.under.set(false);
                }
                self.fcs = value
                    & (FCS_EN
                        | FCS_SHIFT_BIT
                        | FCS_ERR
                        | FCS_DREQ_EN
                        | (FCS_THRESH_MASK << FCS_THRESH_SHIFT));
            }
            // FIFO is read-only.
            FIFO => {}
            DIV => {
                self.div = value & 0x00FF_FFFF;
                self.div_acc = 0;
            }
            // INTR and INTS are read-only levels.
            INTR | INTS => {}
            INTE => self.inte = value & 0x1,
            INTF => self.intf = value & 0x1,
            _ => {
                crate::census_reg!("rp2040.adc:Rp2040Adc", offset, "write");
            }
        }
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !0x3)?;
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !0x3;
        if aligned == FIFO {
            return Ok(()); // read-only, and reading it here would drain it
        }
        let shift = (offset & 0x3) * 8;
        let cur = self.read_u32(aligned)?;
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, new)
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Legacy / feature-off path. Scheduler mode skips the walk.
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        self.step_one()
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
        if !self.scheduler_mode() || !self.needs_scheduler_wake() {
            return Vec::new();
        }
        self.arm_seq = self.arm_seq.wrapping_add(1);
        vec![(0, self.arm_seq)]
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() || event_token != self.arm_seq {
            return crate::sched::EventResult::default();
        }
        let res = self.step_one();
        crate::sched::EventResult {
            explicit_irqs: res.explicit_irqs.unwrap_or_default(),
            reschedule_delay: self.needs_scheduler_wake().then_some(1),
            ..Default::default()
        }
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled() -> Rp2040Adc {
        let mut a = Rp2040Adc::new();
        a.write_u32(CS, CS_EN).unwrap();
        a
    }

    /// Reset state matches the SVD (`CS` all-zero, so READY is clear) and
    /// asserts READY only once the converter is powered.
    #[test]
    fn ready_follows_enable() {
        let mut a = Rp2040Adc::new();
        assert_eq!(a.read_u32(CS).unwrap(), 0, "SVD reset value");
        a.write_u32(CS, CS_EN).unwrap();
        assert_ne!(a.read_u32(CS).unwrap() & CS_READY, 0);
        a.write_u32(CS, 0).unwrap();
        assert_eq!(a.read_u32(CS).unwrap() & CS_READY, 0);
    }

    /// The temperature sensor converts the datasheet relation: 0.706 V at
    /// 27 °C on a 3.3 V / 12-bit scale is code 876.
    #[test]
    fn temperature_sensor_matches_the_datasheet_relation() {
        assert_eq!(temp_sensor_microvolts(27_000), 706_000);
        assert_eq!(code_from_microvolts(706_000), 876);

        let mut a = enabled();
        a.write_u32(CS, CS_EN | CS_TS_EN | (4 << CS_AINSEL_SHIFT))
            .unwrap();
        a.write_u32(CS, a.read_u32(CS).unwrap() | CS_START_ONCE)
            .unwrap();
        assert_eq!(a.read_u32(RESULT).unwrap(), 876);
    }

    /// The relation is a relation, not a table: a warmer die gives a LOWER code.
    #[test]
    fn temperature_sensor_output_falls_as_the_die_warms() {
        let cold = code_from_microvolts(temp_sensor_microvolts(0));
        let hot = code_from_microvolts(temp_sensor_microvolts(85_000));
        assert!(hot < 876 && 876 < cold, "cold {cold} < 876 < hot {hot}");
    }

    /// An unpowered temperature sensor (TS_EN clear) converts 0 rather than
    /// pretending to report a temperature.
    #[test]
    fn temperature_sensor_needs_ts_en() {
        let mut a = enabled();
        a.write_u32(CS, CS_EN | (4 << CS_AINSEL_SHIFT)).unwrap();
        a.write_u32(CS, a.read_u32(CS).unwrap() | CS_START_ONCE)
            .unwrap();
        assert_eq!(a.read_u32(RESULT).unwrap(), 0);
    }

    /// A driven GPIO input converts its real level, and each input is separate.
    #[test]
    fn driven_inputs_convert_their_own_level() {
        let mut a = enabled();
        a.set_channel_input(0, 1650); // half of 3.3 V
        a.set_channel_input(2, 3300); // full scale
        assert_eq!(a.channel_input_count(0), 2048);
        assert_eq!(a.channel_input_count(2), 4095, "clamped to 12 bits");
        assert_eq!(a.channel_input_count(1), 0, "nothing wired to GPIO27");

        a.write_u32(CS, CS_EN | (2 << CS_AINSEL_SHIFT) | CS_START_ONCE)
            .unwrap();
        assert_eq!(a.read_u32(RESULT).unwrap(), 4095);
    }

    /// START_ONCE is write-only and self-clearing, and does nothing while the
    /// converter is powered down.
    #[test]
    fn start_once_is_self_clearing_and_gated_by_enable() {
        let mut a = Rp2040Adc::new();
        a.set_channel_input(0, 3300);
        // Not enabled: no conversion.
        a.write_u32(CS, CS_START_ONCE).unwrap();
        assert_eq!(a.read_u32(RESULT).unwrap(), 0);

        a.write_u32(CS, CS_EN | CS_START_ONCE).unwrap();
        assert_eq!(a.read_u32(RESULT).unwrap(), 4095);
        assert_eq!(a.read_u32(CS).unwrap() & CS_START_ONCE, 0);
    }

    /// The FIFO fills, reports LEVEL/EMPTY/FULL, and drains in order.
    #[test]
    fn fifo_fills_and_drains_in_order() {
        let mut a = enabled();
        a.write_u32(FCS, FCS_EN).unwrap();
        assert_ne!(a.read_u32(FCS).unwrap() & FCS_EMPTY, 0);

        for mv in [330u16, 660, 990] {
            a.set_channel_input(0, mv);
            a.write_u32(CS, CS_EN | CS_START_ONCE).unwrap();
        }
        assert_eq!((a.read_u32(FCS).unwrap() >> FCS_LEVEL_SHIFT) & 0xF, 3);
        assert_eq!(a.read_u32(FIFO).unwrap(), 409);
        assert_eq!(a.read_u32(FIFO).unwrap(), 819);
        assert_eq!(a.read_u32(FIFO).unwrap(), 1228);
        assert_ne!(a.read_u32(FCS).unwrap() & FCS_EMPTY, 0);
    }

    /// Overflow and underflow are flagged and write-1-clearable.
    #[test]
    fn fifo_over_and_underflow_are_flagged() {
        let mut a = enabled();
        a.write_u32(FCS, FCS_EN).unwrap();
        a.set_channel_input(0, 3300);
        for _ in 0..FIFO_DEPTH + 2 {
            a.write_u32(CS, CS_EN | CS_START_ONCE).unwrap();
        }
        let fcs = a.read_u32(FCS).unwrap();
        assert_ne!(fcs & FCS_FULL, 0);
        assert_ne!(fcs & FCS_OVER, 0);
        assert_eq!((fcs >> FCS_LEVEL_SHIFT) & 0xF, FIFO_DEPTH as u32);

        a.write_u32(FCS, FCS_EN | FCS_OVER).unwrap();
        assert_eq!(a.read_u32(FCS).unwrap() & FCS_OVER, 0);

        for _ in 0..FIFO_DEPTH {
            a.read_u32(FIFO).unwrap();
        }
        assert_eq!(a.read_u32(FIFO).unwrap(), 0, "empty pop reads 0");
        assert_ne!(a.read_u32(FCS).unwrap() & FCS_UNDER, 0);
    }

    /// FCS.SHIFT produces the byte-wide form the DMA byte path consumes.
    #[test]
    fn fcs_shift_right_shifts_samples_by_four() {
        let mut a = enabled();
        a.set_channel_input(0, 3300);
        a.write_u32(FCS, FCS_EN | FCS_SHIFT_BIT).unwrap();
        a.write_u32(CS, CS_EN | CS_START_ONCE).unwrap();
        assert_eq!(a.read_u32(FIFO).unwrap(), 4095 >> 4);
    }

    /// INTR.FIFO is a level: it asserts at the threshold and drops when the
    /// FIFO is drained back below it — there is nothing to acknowledge.
    #[test]
    fn fifo_interrupt_is_a_level_gated_by_inte() {
        let mut a = enabled();
        a.set_channel_input(0, 3300);
        a.write_u32(FCS, FCS_EN | (2 << FCS_THRESH_SHIFT)).unwrap();
        assert_eq!(a.read_u32(INTR).unwrap(), 0);

        a.write_u32(CS, CS_EN | CS_START_ONCE).unwrap();
        assert_eq!(a.read_u32(INTR).unwrap(), 0, "one sample, threshold 2");
        a.write_u32(CS, CS_EN | CS_START_ONCE).unwrap();
        assert_eq!(a.read_u32(INTR).unwrap(), 1);

        // Masked: raw level set, no NVIC line.
        assert_eq!(a.read_u32(INTS).unwrap(), 0);
        assert_eq!(a.tick().explicit_irqs, None);
        a.write_u32(INTE, 1).unwrap();
        assert_eq!(a.read_u32(INTS).unwrap(), 1);
        assert_eq!(a.tick().explicit_irqs, Some(vec![ADC_IRQ_FIFO]));

        // Draining below the threshold drops the line — no INTR write needed.
        a.read_u32(FIFO).unwrap();
        a.read_u32(FIFO).unwrap();
        assert_eq!(a.read_u32(INTR).unwrap(), 0);
        assert_eq!(a.tick().explicit_irqs, None);
    }

    /// INTF forces the line without any FIFO activity (the pico-sdk test path).
    #[test]
    fn intf_forces_the_fifo_interrupt() {
        let mut a = enabled();
        a.write_u32(FCS, FCS_EN | (4 << FCS_THRESH_SHIFT)).unwrap();
        a.write_u32(INTE, 1).unwrap();
        assert_eq!(a.tick().explicit_irqs, None);
        a.write_u32(INTF, 1).unwrap();
        assert_eq!(a.tick().explicit_irqs, Some(vec![ADC_IRQ_FIFO]));
    }

    /// START_MANY free-runs, and DIV really paces it: 3.0 clocks per sample
    /// yields one conversion every three ticks.
    #[test]
    fn start_many_is_paced_by_div() {
        let mut a = enabled();
        a.set_channel_input(0, 3300);
        a.write_u32(FCS, FCS_EN).unwrap();
        a.write_u32(DIV, 2 << 8).unwrap(); // INT = 2 -> period 3.0
        a.write_u32(CS, CS_EN | CS_START_MANY).unwrap();
        for _ in 0..9 {
            a.tick();
        }
        assert_eq!((a.read_u32(FCS).unwrap() >> FCS_LEVEL_SHIFT) & 0xF, 3);
    }

    /// A fractional divider is not rounded away: 2.5 clocks per sample gives 8
    /// conversions in 20 ticks.
    #[test]
    fn div_fraction_is_honoured() {
        let mut a = enabled();
        a.write_u32(FCS, FCS_EN | (0xF << FCS_THRESH_SHIFT))
            .unwrap();
        a.write_u32(DIV, (1 << 8) | 128).unwrap(); // 1 + 1 + 0.5 = 2.5
        a.write_u32(CS, CS_EN | CS_START_MANY).unwrap();
        let mut popped = 0;
        for _ in 0..20 {
            a.tick();
            // Drain so the 8-entry FIFO cannot overflow and hide the count.
            if a.read_u32(FCS).unwrap() & FCS_EMPTY == 0 {
                a.read_u32(FIFO).unwrap();
                popped += 1;
            }
        }
        assert_eq!(popped, 8);
    }

    /// The round-robin sequencer walks only the selected inputs and wraps.
    #[test]
    fn round_robin_advances_ainsel_over_selected_inputs() {
        let mut a = enabled();
        let ainsel = |a: &Rp2040Adc| (a.read_u32(CS).unwrap() >> CS_AINSEL_SHIFT) & CS_AINSEL_MASK;
        // Inputs 0, 2 and 3 in the sequence, starting at 0.
        a.write_u32(CS, CS_EN | (0b01101 << CS_RROBIN_SHIFT))
            .unwrap();
        let base = a.read_u32(CS).unwrap();
        a.write_u32(CS, base | CS_START_ONCE).unwrap();
        assert_eq!(ainsel(&a), 2);
        a.write_u32(CS, a.read_u32(CS).unwrap() | CS_START_ONCE)
            .unwrap();
        assert_eq!(ainsel(&a), 3);
        a.write_u32(CS, a.read_u32(CS).unwrap() | CS_START_ONCE)
            .unwrap();
        assert_eq!(ainsel(&a), 0, "wraps back to the lowest selected input");
    }

    /// With RROBIN clear, AINSEL stays where firmware put it.
    #[test]
    fn round_robin_disabled_holds_ainsel() {
        let mut a = enabled();
        a.write_u32(CS, CS_EN | (3 << CS_AINSEL_SHIFT) | CS_START_ONCE)
            .unwrap();
        assert_eq!(
            (a.read_u32(CS).unwrap() >> CS_AINSEL_SHIFT) & CS_AINSEL_MASK,
            3
        );
    }

    /// ERR_STICKY honours write-1-to-clear even though nothing sets it — the
    /// model has no conversion-error source and never claims one.
    #[test]
    fn no_conversion_errors_are_invented() {
        let mut a = enabled();
        a.set_channel_input(0, 1000);
        for _ in 0..16 {
            a.write_u32(CS, CS_EN | CS_START_ONCE).unwrap();
        }
        assert_eq!(a.read_u32(CS).unwrap() & CS_ERR_STICKY, 0);
        assert_eq!(a.read_u32(FCS).unwrap() & FCS_ERR, 0);
    }
}
