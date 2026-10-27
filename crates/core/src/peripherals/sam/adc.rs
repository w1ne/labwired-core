// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Opt-in SAMD51 ADC slice (DS60001507F 45): single-ended software-triggered
//! conversions of AINn against GND with the VDDANA references (REFSEL INTVCC1
//! = VDDANA, INTVCC0 = VDDANA / 2) at 8/10/12-bit resolution. A conversion
//! started by SWTRIG.START completes on the next bus tick, and only while the
//! ADC is enabled, its bus clock and generic clock run, and the input's pad is
//! handed to the ADC (PMUX function B). The code is floor(V * 2^n / Vref),
//! clamped. Input voltages are set by the host; nothing is sampled from a
//! modelled analog front end (no settling, noise, gain/offset correction,
//! averaging, windowing, events, DMA sequencing or differential mode).
use crate::{Bus, Peripheral, SimResult, SimulationError};
use std::any::Any;

const MCLK_APBDMASK: u64 = 0x40000820;
const START: u8 = 1 << 1;
const RESRDY: u8 = 1;

#[derive(Clone, Copy, Debug)]
pub struct AdcInstance {
    pub base: u64,
    pub gclk_channel: u64,
    pub apbd_bit: u32,
    /// (AIN, port, pin) for function B, from the ATSAMD51J19A ATDF.
    pub inputs: &'static [(u8, u8, u8)],
}

pub const ADC_INSTANCES: [AdcInstance; 2] = [
    AdcInstance {
        base: 0x43001c00,
        gclk_channel: 40,
        apbd_bit: 7,
        inputs: &[
            (0, 0, 2),
            (1, 0, 3),
            (2, 1, 8),
            (3, 1, 9),
            (4, 0, 4),
            (5, 0, 5),
            (6, 0, 6),
            (7, 0, 7),
            (8, 0, 8),
            (9, 0, 9),
            (10, 0, 10),
            (11, 0, 11),
            (12, 1, 0),
            (13, 1, 1),
            (14, 1, 2),
            (15, 1, 3),
        ],
    },
    AdcInstance {
        base: 0x43002000,
        gclk_channel: 41,
        apbd_bit: 8,
        inputs: &[
            (0, 1, 8),
            (1, 1, 9),
            (2, 0, 8),
            (3, 0, 9),
            (6, 1, 4),
            (7, 1, 5),
            (8, 1, 6),
            (9, 1, 7),
        ],
    },
];

#[derive(Debug)]
pub struct Samd51Adc {
    instance: AdcInstance,
    ctrla: u16,
    inputctrl: u16,
    ctrlb: u16,
    refctrl: u8,
    avgctrl: u8,
    sampctrl: u8,
    calib: u16,
    inten: u8,
    intflag: u8,
    result: u16,
    pending: bool,
    /// Host-set input voltages per AIN, millivolts.
    inputs: [u32; 16],
    vddana_mv: u32,
    conversions: u64,
}

impl Samd51Adc {
    pub fn new(instance: AdcInstance) -> Self {
        Self {
            instance,
            ctrla: 0,
            inputctrl: 0,
            ctrlb: 0,
            refctrl: 0,
            avgctrl: 0,
            sampctrl: 0,
            calib: 0,
            inten: 0,
            intflag: 0,
            result: 0,
            pending: false,
            inputs: [0; 16],
            vddana_mv: 3300,
            conversions: 0,
        }
    }
    /// Drive AIN `ain` with `millivolts` (the outside world, not a register).
    pub fn set_input_millivolts(&mut self, ain: usize, millivolts: u32) {
        if ain < 16 {
            self.inputs[ain] = millivolts;
        }
    }
    pub fn conversions(&self) -> u64 {
        self.conversions
    }
    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "samd51 ADC slice: {what}"
        )))
    }
    fn enabled(&self) -> bool {
        self.ctrla & 2 != 0
    }
    fn bits(&self) -> SimResult<u32> {
        match (self.ctrlb >> 3) & 3 {
            0 => Ok(12),
            2 => Ok(10),
            3 => Ok(8),
            _ => Self::refuse("16-bit (averaging) resolution"),
        }
    }
    fn convert(&mut self, bus: &dyn Bus) -> SimResult<bool> {
        let clocked = bus
            .read_u32(MCLK_APBDMASK)
            .is_ok_and(|m| m & (1 << self.instance.apbd_bit) != 0)
            && bus
                .read_u32(0x40001c80 + 4 * self.instance.gclk_channel)
                .is_ok_and(|c| c & (1 << 6) != 0);
        if !self.enabled() || !clocked {
            return Ok(false); // the conversion stays pending
        }
        let ain = u8::try_from(self.inputctrl & 0x1f).unwrap_or(0xff);
        let Some(&(_, port, pin)) = self.instance.inputs.iter().find(|i| i.0 == ain) else {
            return Self::refuse("MUXPOS outside the pad inputs");
        };
        let group = 0x41008000 + 0x80 * u64::from(port);
        let pincfg = bus.read_u8(group + 0x40 + u64::from(pin)).unwrap_or(0);
        let pmux = bus.read_u8(group + 0x30 + u64::from(pin / 2)).unwrap_or(0);
        let function = if pin & 1 != 0 { pmux >> 4 } else { pmux & 0xf };
        let vref = match self.refctrl & 0xf {
            3 => self.vddana_mv,
            2 => self.vddana_mv / 2,
            _ => return Self::refuse("references other than INTVCC0/INTVCC1"),
        };
        let bits = self.bits()?;
        // A pad not handed to the ADC carries no signal to the input.
        let mv = if pincfg & 1 != 0 && function == 1 {
            self.inputs[usize::from(ain)]
        } else {
            0
        };
        let full = (1u64 << bits) - 1;
        self.result = (u64::from(mv) * (1 << bits) / u64::from(vref)).min(full) as u16;
        self.intflag |= RESRDY;
        self.conversions += 1;
        Ok(true)
    }
    fn read_reg(&self, offset: u64) -> SimResult<u16> {
        Ok(match offset {
            0x00 => self.ctrla,
            0x02 | 0x03 | 0x14 | 0x2f => 0,
            0x04 => self.inputctrl,
            0x06 => self.ctrlb,
            0x08 => u16::from(self.refctrl),
            0x0a => u16::from(self.avgctrl),
            0x0b => u16::from(self.sampctrl),
            0x0c | 0x0e | 0x10 | 0x12 => 0,
            0x2c | 0x2d => u16::from(self.inten),
            0x2e => u16::from(self.intflag),
            0x30 | 0x32 => 0, // SYNCBUSY: writes complete at once
            0x40 => self.result,
            0x48 => self.calib,
            _ => return Self::refuse("register"),
        })
    }
    fn write_reg(&mut self, offset: u64, value: u16) -> SimResult<()> {
        match offset {
            0x00 if value & 1 != 0 => {
                let (inputs, vddana_mv, conversions) =
                    (self.inputs, self.vddana_mv, self.conversions);
                *self = Self {
                    inputs,
                    vddana_mv,
                    conversions,
                    ..Self::new(self.instance)
                };
            }
            // ENABLE, RUNSTDBY, ONDEMAND, PRESCALER; DUALSEL/SLAVEEN refused.
            0x00 if value & !0x07c2 == 0 => self.ctrla = value,
            0x02 | 0x03 | 0x0c | 0x0e | 0x10 | 0x12 if value == 0 => {}
            // MUXPOS (single-ended against GND only).
            0x04 if value >> 8 == 0x18 && value & 0x1f < 16 => self.inputctrl = value,
            // RESSEL only (no left adjust, freerun, correction or window).
            0x06 if value & !0x18 == 0 => self.ctrlb = value,
            0x08 if value & !0x8f == 0 => self.refctrl = value as u8,
            0x0a if value == 0 => self.avgctrl = 0,
            0x0b => self.sampctrl = value as u8,
            0x14 if value as u8 & !START == 0 => {
                if value as u8 & START != 0 {
                    self.intflag &= !RESRDY;
                    self.pending = true;
                }
            }
            0x2c => self.inten &= !(value as u8),
            0x2d => self.inten |= value as u8 & 7,
            0x2e => self.intflag &= !(value as u8),
            0x48 => self.calib = value,
            _ => return Self::refuse("store outside the conversion slice"),
        }
        Ok(())
    }
}

impl Peripheral for Samd51Adc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let wide = matches!(offset, 0x00..=0x01 | 0x04..=0x07 | 0x40..=0x41 | 0x48..=0x49);
        let base = if wide { offset & !1 } else { offset };
        let base = if (0x30..=0x33).contains(&offset) {
            0x30
        } else {
            base
        };
        Ok((self.read_reg(base)? >> (8 * (offset - base))) as u8)
    }
    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        self.read_reg(offset)
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(u32::from(self.read_reg(offset)?))
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_reg(offset, u16::from(value))
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write_reg(offset, value)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match u16::try_from(value) {
            Ok(v) => self.write_reg(offset, v),
            Err(_) => Self::refuse("word store"),
        }
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn needs_bus_tick(&self) -> bool {
        self.pending
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if let Ok(true) = self.convert(bus) {
            self.pending = false;
        }
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"peripheral": "samd51_adc", "base": self.instance.base, "ctrla": self.ctrla,
            "inputctrl": self.inputctrl, "refctrl": self.refctrl, "result": self.result,
            "conversions": self.conversions})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_input_over_reference_and_inputs_are_checked() {
        let mut a = Samd51Adc::new(ADC_INSTANCES[0]);
        a.write_u16(0x06, 2 << 3).unwrap(); // 10-bit
        a.write(0x08, 3).unwrap(); // INTVCC1
        a.write_u16(0x04, 0x1805).unwrap(); // AIN5 vs GND
        assert!(a.write_u16(0x04, 0x0005).is_err(), "differential");
        a.write_u16(0x06, 1 << 3).unwrap();
        assert!(
            a.bits().is_err(),
            "16-bit (averaging) is refused at conversion"
        );
        a.write_u16(0x06, 2 << 3).unwrap();
        a.set_input_millivolts(5, 1650);
        assert_eq!(a.bits().unwrap(), 10);
        let full = 1u64 << 10;
        assert_eq!(1650 * full / 3300, 512);
    }
}
