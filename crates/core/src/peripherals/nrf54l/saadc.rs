// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Functional single-ended SAADC model for the Nordic nRF54L family.
//!
//! Source: the vendored nRF54L15 application-core SVD,
//! `GLOBAL_SAADC_S`. The task, event, channel, resolution and EasyDMA register
//! offsets match the Nordic SAADC layout, but the channel selectors and analog
//! reference do not match nRF52: nRF54L PSELP encodes a GPIO port/pin plus a
//! connection mode, its internal reference is 0.9 V, and GAIN encodings differ.
//!
//! LabWired's explicit `adc.saadc.<channel>_volts` route supplies the voltage
//! to a logical CH[n] input. PSELP must select an external analog input before
//! that channel converts. This model does not infer a board pad alias from
//! PSELP; a board descriptor must opt into pad-level routing separately.
//!
//! One TASKS_SAMPLE writes one ascending-channel scan to the EasyDMA result
//! buffer. This models deterministic conversion and routing; TACQ/TCONV timing,
//! oversampling, burst mode, limits, calibration trims and DPPI are not modeled.

use crate::{Bus, Peripheral, SimResult};

const INTERNAL_REFERENCE_MV: i64 = 900;
const EXTERNAL_REFERENCE_MV: i64 = 1200;

const OFF_TASKS_START: u64 = 0x000;
const OFF_TASKS_SAMPLE: u64 = 0x004;
const OFF_TASKS_STOP: u64 = 0x008;
const OFF_TASKS_CALIBRATEOFFSET: u64 = 0x00C;

const OFF_EVENTS_STARTED: u64 = 0x100;
const OFF_EVENTS_END: u64 = 0x104;
const OFF_EVENTS_DONE: u64 = 0x108;
const OFF_EVENTS_RESULTDONE: u64 = 0x10C;
const OFF_EVENTS_CALIBRATEDONE: u64 = 0x110;
const OFF_EVENTS_STOPPED: u64 = 0x114;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_STATUS: u64 = 0x400;
const OFF_ENABLE: u64 = 0x500;
const OFF_CH_FIRST: u64 = 0x510;
const OFF_CH_LAST: u64 = 0x58C;
const OFF_RESOLUTION: u64 = 0x5F0;
const OFF_OVERSAMPLE: u64 = 0x5F4;
const OFF_SAMPLERATE: u64 = 0x5F8;
const OFF_RESULT_PTR: u64 = 0x62C;
const OFF_RESULT_MAXCNT: u64 = 0x630;
const OFF_RESULT_AMOUNT: u64 = 0x634;

const PENDING_NONE: bool = false;
const PENDING_SAMPLE: bool = true;
const PSELP_CONNECT_MASK: u32 = 0b11 << 30;
const PSELP_CONNECT_ANALOG_INPUT: u32 = 0b01 << 30;

fn resolution_bits(resolution: u32) -> u32 {
    match resolution & 0x7 {
        0 => 8,
        1 => 10,
        2 => 12,
        _ => 14,
    }
}

/// nRF54L CH.CONFIG.GAIN encoding → (gain numerator, denominator).
fn gain_ratio(config: u32) -> (i64, i64) {
    match (config >> 8) & 0x7 {
        0 => (2, 1),
        1 => (1, 1),
        2 => (2, 3),
        3 => (1, 2),
        4 => (2, 5),
        5 => (1, 3),
        6 => (2, 7),
        _ => (1, 4),
    }
}

#[derive(Debug, Default)]
pub struct Nrf54lSaadc {
    events_started: u32,
    events_end: u32,
    events_done: u32,
    events_resultdone: u32,
    events_calibratedone: u32,
    events_stopped: u32,
    inten: u32,
    enable: u32,
    ch: [u32; 32],
    resolution: u32,
    oversample: u32,
    samplerate: u32,
    result_ptr: u32,
    result_maxcnt: u32,
    result_amount: u32,
    armed: bool,
    active_ptr: u32,
    active_maxcnt: u32,
    generation: u32,
    scheduled: bool,
    pending: bool,
    /// Logical CH[0..7] voltages supplied through the explicit ADC route.
    inputs_mv: [Option<u16>; 8],
}

impl Nrf54lSaadc {
    pub fn new() -> Self {
        Self::default()
    }

    fn enabled(&self) -> bool {
        self.enable & 1 != 0
    }

    fn channel_sample(&self, channel: usize) -> Option<i16> {
        let base = channel * 4;
        let pselp = self.ch[base];
        if pselp & PSELP_CONNECT_MASK != PSELP_CONNECT_ANALOG_INPUT {
            return None;
        }

        let config = self.ch[base + 2];
        let differential = config & (1 << 15) != 0;
        // Differential CH input routing needs an independent negative-input
        // voltage source; the current explicit route supplies one voltage.
        if differential {
            return None;
        }

        let voltage_mv = i64::from(self.inputs_mv[channel].unwrap_or(0));
        let reference_mv = if config & (1 << 12) != 0 {
            EXTERNAL_REFERENCE_MV
        } else {
            INTERNAL_REFERENCE_MV
        };
        let (gain_num, gain_den) = gain_ratio(config);
        let scale = 1i64 << resolution_bits(self.resolution);
        let code = voltage_mv * gain_num * scale / (gain_den * reference_mv);
        Some(code.clamp(0, scale - 1) as i16)
    }

    fn event_bits(&self) -> u32 {
        self.events_started
            | (self.events_end << 1)
            | (self.events_done << 2)
            | (self.events_resultdone << 3)
            | (self.events_calibratedone << 4)
            | (self.events_stopped << 5)
    }

    fn do_easydma_sample(&mut self, bus: &mut dyn Bus) {
        if !self.pending {
            return;
        }
        self.pending = PENDING_NONE;
        self.scheduled = false;

        let samples: Vec<i16> = (0..8)
            .filter_map(|channel| self.channel_sample(channel))
            .collect();
        if samples.is_empty() {
            return;
        }
        let remaining = self.active_maxcnt.saturating_sub(self.result_amount);
        if remaining == 0 {
            return;
        }

        crate::with_device_write_bypass(bus, |bus| {
            for sample in samples.iter().take(remaining as usize) {
                let bytes = sample.to_le_bytes();
                let address = self.active_ptr as u64 + u64::from(self.result_amount) * 2;
                let _ = bus.write_u8(address, bytes[0]);
                let _ = bus.write_u8(address + 1, bytes[1]);
                self.result_amount += 1;
            }
        });
        self.events_done = 1;
        self.events_resultdone = 1;
        if self.result_amount == self.active_maxcnt {
            self.events_end = 1;
            self.armed = false;
        }
    }
}

impl Peripheral for Nrf54lSaadc {
    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        let Some(input) = self.inputs_mv.get_mut(channel as usize) else {
            return false;
        };
        *input = Some(millivolts);
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        let Some(input) = self.inputs_mv.get_mut(channel as usize) else {
            return false;
        };
        *input = None;
        true
    }

    fn adc_channel_count(&self) -> Option<u8> {
        Some(8)
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.event_bits() & self.inten != 0)
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_START | OFF_TASKS_SAMPLE | OFF_TASKS_STOP | OFF_TASKS_CALIBRATEOFFSET => 0,
            OFF_EVENTS_STARTED => self.events_started,
            OFF_EVENTS_END => self.events_end,
            OFF_EVENTS_DONE => self.events_done,
            OFF_EVENTS_RESULTDONE => self.events_resultdone,
            OFF_EVENTS_CALIBRATEDONE => self.events_calibratedone,
            OFF_EVENTS_STOPPED => self.events_stopped,
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_STATUS => 0,
            OFF_ENABLE => self.enable & 1,
            OFF_CH_FIRST..=OFF_CH_LAST if offset.is_multiple_of(4) => {
                self.ch[((offset - OFF_CH_FIRST) / 4) as usize]
            }
            OFF_RESOLUTION => self.resolution & 0x7,
            OFF_OVERSAMPLE => self.oversample & 0xF,
            OFF_SAMPLERATE => self.samplerate,
            OFF_RESULT_PTR => self.result_ptr,
            OFF_RESULT_MAXCNT => self.result_maxcnt & 0x7FFF,
            OFF_RESULT_AMOUNT => self.result_amount & 0x7FFF,
            _ => {
                crate::census_reg!("nrf54l.saadc:Nrf54lSaadc", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START if value != 0 && self.enabled() => {
                self.pending = PENDING_NONE;
                self.scheduled = false;
                self.armed = true;
                self.active_ptr = self.result_ptr;
                self.active_maxcnt = self.result_maxcnt;
                self.result_amount = 0;
                self.events_started = 1;
            }
            OFF_TASKS_SAMPLE if value != 0 && self.enabled() && self.armed && !self.pending => {
                self.generation = self.generation.wrapping_add(1);
                self.scheduled = false;
                self.pending = PENDING_SAMPLE;
            }
            OFF_TASKS_STOP if value != 0 && self.enabled() => {
                self.pending = PENDING_NONE;
                self.armed = false;
                self.events_stopped = 1;
            }
            OFF_TASKS_CALIBRATEOFFSET if value != 0 && self.enabled() => {
                self.events_calibratedone = 1;
            }
            OFF_TASKS_START | OFF_TASKS_SAMPLE | OFF_TASKS_STOP | OFF_TASKS_CALIBRATEOFFSET => {}
            OFF_EVENTS_STARTED if value == 0 => self.events_started = 0,
            OFF_EVENTS_END if value == 0 => self.events_end = 0,
            OFF_EVENTS_DONE if value == 0 => self.events_done = 0,
            OFF_EVENTS_RESULTDONE if value == 0 => self.events_resultdone = 0,
            OFF_EVENTS_CALIBRATEDONE if value == 0 => self.events_calibratedone = 0,
            OFF_EVENTS_STOPPED if value == 0 => self.events_stopped = 0,
            OFF_INTEN => self.inten = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            OFF_ENABLE => {
                self.enable = value & 1;
                if !self.enabled() {
                    self.pending = PENDING_NONE;
                    self.armed = false;
                }
            }
            OFF_CH_FIRST..=OFF_CH_LAST if offset.is_multiple_of(4) => {
                self.ch[((offset - OFF_CH_FIRST) / 4) as usize] = value;
            }
            OFF_RESOLUTION => self.resolution = value & 0x7,
            OFF_OVERSAMPLE => self.oversample = value & 0xF,
            OFF_SAMPLERATE => self.samplerate = value,
            OFF_RESULT_PTR => self.result_ptr = value,
            OFF_RESULT_MAXCNT => self.result_maxcnt = value & 0x7FFF,
            OFF_STATUS | OFF_RESULT_AMOUNT => {}
            _ => crate::census_reg!("nrf54l.saadc:Nrf54lSaadc", offset, "write"),
        }
        Ok(())
    }

    fn needs_bus_tick(&self) -> bool {
        self.pending
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.do_easydma_sample(bus);
    }

    fn uses_scheduler(&self) -> bool {
        true
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.pending && !self.scheduled {
            self.scheduled = true;
            vec![(0, self.generation)]
        } else {
            Vec::new()
        }
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _scheduler: &mut crate::sched::EventScheduler,
        bus: &mut dyn Bus,
    ) -> crate::sched::EventResult {
        if event_token == self.generation && self.pending {
            self.do_easydma_sample(bus);
        }
        crate::sched::EventResult {
            raise_own_irq: self.irq_line_level() == Some(true),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{gain_ratio, resolution_bits, EXTERNAL_REFERENCE_MV, INTERNAL_REFERENCE_MV};

    #[test]
    fn nrf54l_saadc_uses_its_own_gain_and_reference_encoding() {
        assert_eq!(gain_ratio(7 << 8), (1, 4));
        assert_eq!(gain_ratio(3 << 8), (1, 2));
        assert_eq!(INTERNAL_REFERENCE_MV, 900);
        assert_eq!(EXTERNAL_REFERENCE_MV, 1200);
    }

    #[test]
    fn nrf54l_saadc_has_8_to_14_bit_resolution_encodings() {
        assert_eq!(resolution_bits(0), 8);
        assert_eq!(resolution_bits(1), 10);
        assert_eq!(resolution_bits(2), 12);
        assert_eq!(resolution_bits(3), 14);
    }
}
