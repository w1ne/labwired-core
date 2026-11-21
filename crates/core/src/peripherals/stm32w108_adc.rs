// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32W108 (Ember EM357 core) general-purpose ADC: a sigma-delta converter
//! whose only result path is a dedicated DMA channel into RAM. The block is at
//! `0x4000_D000`; its interrupt flag and enable registers sit in the shared
//! event block at `0x4000_A810` (`ADC_ISR`) and `0x4000_A850` (`ADC_IER`).
//! The model answers those two at offsets 0x810 and 0x850 of its own window,
//! and the chip descriptor mirrors the real addresses there (`alias_of`).
//!
//! Sources:
//! - ST DocID16252 Rev 16 (STM32W108HB/CB/CC/CZ datasheet, archived
//!   CD00248316.pdf), chapter 11: §11.1.2 Table 37 (ADC0..5 = PB5, PB6, PB7,
//!   PC1, PA4, PA5; Table 38 prints PC0 for ADC3, but Table 37, §11.3.3 and
//!   the ST HAL say PC1), §11.1.4 offset/gain correction (offset added, gain
//!   multiplied as an unsigned 1.15 value, the product saturated to a signed
//!   16-bit number with `SAT` set, then divided by two), §11.1.5 DMA (linear
//!   mode stops at `NDTR` samples and sets `DMABF`; auto wrap restarts at
//!   `MSAR`; `DMABHF` at the last location of the first half), §11.1.6
//!   input selection (8 GND, 9 VREF/2, 10 VREF, 11 VREG/2), input buffers of
//!   fixed gain 0.25 (`HVSELP`/`HVSELN`), `SMP` = 5..12 significant bits
//!   left-aligned in 16, §11.1.7 continuous conversion while `ADON`,
//!   §11.1.8 Table 41 (GND ≈ 0xE000, VREF ≈ 0x2000, VREF through the input
//!   buffer ≈ 0xF000, all against VREF/2), §11.2 (`DMAOVF`: a result was
//!   ready with the DMA inactive; ISR bits clear by writing 1; the IRQ is
//!   `ISR & IER`), §11.3.1..11.3.11 (register layout and reset values).
//! - Contiki `cpu/stm32w108/hal/micro/cortexm3/stm32w108/regs.h` (ST, 2010):
//!   the same addresses, `ADC_DATA` @0x4000D000, `ADC_DMABEG`/`ADC_DMACUR`
//!   fixed bits 0x2000_0000, `INT_ADC` = NVIC bit 11.
//! - Contiki `cpu/stm32w108/hal/micro/cortexm3/adc.c` (ST HAL): the read
//!   flow the tests replay (`ADC_DMACFG = ADC_DMARST`, `ADC_DMABEG =
//!   &adcData`, `ADC_DMASIZE = 1`, `AUTOWRAP | LOAD`, `ADC_CFG` with
//!   `MUXP`/`MUXN`, wait for `INT_ADCULDFULL`) and the calibration against
//!   GND and VREG/2 (0.9 V) that `halConvertValueToVolts` uses.
//!
//! Modelled: while `ADON` is 1, each bus tick completes one conversion of
//! `(Vp - Vn)` against VREF (1.2 V by default, or the routed reference).
//! The raw sample is `32768 * (Vp - Vn) / VREF`, floored, limited to a
//! signed 16-bit number, with the low `16 - (5 + SMP)` bits cleared. Then
//! `((raw + OFFSET) * GAIN) >> 15` (saturated, `SAT`) and `>> 1` give the
//! result, so with reset OFFSET/GAIN a single-ended input against VREF/2
//! reads `16384 * (V - 0.6) / 1.2`. The result goes to `ADC_DATA` and, with
//! the DMA active, to `MSAR + 2 * CNDTR` in RAM (little endian). An
//! undriven pad converts as 0 V.
//!
//! Not sourced, kept minimal: conversion time (one conversion per bus
//! tick; the doubled first sample and the 21 µs start-up are not timed);
//! no-connection selections (6, 7, 12..15) convert as 0 V; `DMABHF` needs
//! `NDTR >= 2`; `CNDTR` restarts at 0 when auto wrap wraps; `LOAD` with
//! `NDTR = 0` does not start the DMA; the 6 MHz clock gives the same bits as
//! 1 MHz; `ADC_DATA` is from regs.h only (DS Rev 16 does not list it); the
//! model also answers `ISR`/`IER` at `0x4000_D810`/`0x4000_D850`, which the
//! silicon does not. Not modelled: the external VREF on PB0 as an input
//! channel, the CC/CZ HV chopper, and the high-voltage buffer offset drift.

use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};

/// External inputs ADC0..ADC5 (CHSEL 0..5).
const CHANNELS: usize = 6;

const DATA: u64 = 0x00;
const CR: u64 = 0x04;
const OFFSETR: u64 = 0x08;
const GAINR: u64 = 0x0C;
const DMACR: u64 = 0x10;
const DMASR: u64 = 0x14;
const DMAMSAR: u64 = 0x18;
const DMANDTR: u64 = 0x1C;
const DMAMNAR: u64 = 0x20;
const DMACNDTR: u64 = 0x24;
/// `ADC_ISR` (0x4000A810) and `ADC_IER` (0x4000A850), mirrored here.
const ISR: u64 = 0x810;
const IER: u64 = 0x850;

const CR_RESET: u32 = 0x0000_1800;
/// SMP, HVSELP, HVSELN, CHSELP, CHSELN, CLK, ADON (bit 1 reserved).
const CR_MASK: u32 = 0xFFFD;
const ADON: u32 = 1 << 0;
const CHSELN_SHIFT: u32 = 3;
const CHSELP_SHIFT: u32 = 7;
const HVSELN: u32 = 1 << 11;
const HVSELP: u32 = 1 << 12;
const SMP_SHIFT: u32 = 13;

const GAIN_RESET: u32 = 0x8000;

const DMA_LOAD: u32 = 1 << 0;
const DMA_AUTOWRAP: u32 = 1 << 1;
const DMA_RST: u32 = 1 << 4;
const DMASR_ACT: u32 = 1 << 0;
const DMASR_OVF: u32 = 1 << 1;
const RAM_FIXED: u32 = 0x2000_0000;
const MSA_MASK: u32 = 0x1FFF;
const NDT_MASK: u32 = 0x1FFF;

const DMABHF: u32 = 1 << 1;
const DMABF: u32 = 1 << 2;
const SAT: u32 = 1 << 3;
const DMAOVF: u32 = 1 << 4;
const FLAGS: u32 = DMABHF | DMABF | SAT | DMAOVF;

/// VREG/2 (§11.1.6 Table 38: "1V8 VREG/2", 0.9 V).
const VREG_HALF_MV: i64 = 900;

/// STM32W108 ADC with its DMA channel and interrupt flags.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Stm32w108Adc {
    cr: u32,
    offset: u16,
    gain: u16,
    autowrap: bool,
    dma_active: bool,
    dma_ovf: bool,
    msa: u32,
    ndt: u32,
    cndt: u32,
    data: u16,
    isr: u32,
    ier: u32,
    vref_mv: u32,
    /// Routed levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; CHANNELS],
}

impl Default for Stm32w108Adc {
    fn default() -> Self {
        Self::new(1200)
    }
}

impl Stm32w108Adc {
    /// `vref_mv`: VREF, 1.2 V internal by default.
    pub fn new(vref_mv: u32) -> Self {
        Self {
            cr: CR_RESET,
            offset: 0,
            gain: GAIN_RESET as u16,
            autowrap: false,
            dma_active: false,
            dma_ovf: false,
            msa: 0,
            ndt: 0,
            cndt: 0,
            data: 0,
            isr: 0,
            ier: 0,
            vref_mv: vref_mv.max(1),
            inputs_mv: [u16::MAX; CHANNELS],
        }
    }

    /// Level of a CHSEL source in millivolts.
    fn source_mv(&self, sel: u32) -> i64 {
        let vref = i64::from(self.vref_mv);
        match sel {
            0..=5 => match self.inputs_mv[sel as usize] {
                u16::MAX => 0,
                mv => i64::from(mv),
            },
            9 => vref / 2,
            10 => vref,
            11 => VREG_HALF_MV,
            _ => 0,
        }
    }

    /// One conversion: the corrected 16-bit result and whether the gain
    /// product saturated.
    fn convert(&self) -> (u16, bool) {
        // In quarter millivolts, so an input buffer's 0.25 gain stays exact.
        let side = |sel_shift: u32, hv: u32| {
            let mv = self.source_mv((self.cr >> sel_shift) & 0xF);
            if self.cr & hv != 0 {
                mv
            } else {
                mv * 4
            }
        };
        let diff = side(CHSELP_SHIFT, HVSELP) - side(CHSELN_SHIFT, HVSELN);
        let raw = (32768 * diff)
            .div_euclid(4 * i64::from(self.vref_mv))
            .clamp(-32768, 32767);
        let bits = 5 + ((self.cr >> SMP_SHIFT) & 0x7);
        let raw = raw & !((1i64 << (16 - bits)) - 1);
        let product = ((raw + i64::from(self.offset as i16)) * i64::from(self.gain)) >> 15;
        let saturated = !(-32768..=32767).contains(&product);
        let corrected = product.clamp(-32768, 32767);
        (((corrected >> 1) as i16) as u16, saturated)
    }

    fn write_dmacr(&mut self, value: u32) {
        if value & DMA_RST != 0 {
            self.dma_active = false;
            self.dma_ovf = false;
            self.cndt = 0;
        }
        self.autowrap = value & DMA_AUTOWRAP != 0;
        if value & DMA_LOAD != 0 && self.ndt != 0 {
            self.dma_active = true;
            self.cndt = 0;
        }
    }

    fn msa_addr(&self) -> u32 {
        RAM_FIXED | self.msa
    }

    /// Address the DMA writes next.
    fn next_addr(&self) -> u32 {
        self.msa_addr() + 2 * self.cndt
    }

    /// One completed conversion and its DMA transfer.
    fn conversion(&mut self, bus: &mut dyn Bus) {
        let (value, saturated) = self.convert();
        self.data = value;
        if saturated {
            self.isr |= SAT;
        }
        if !self.dma_active {
            self.dma_ovf = true;
            self.isr |= DMAOVF;
            return;
        }
        let addr = u64::from(self.next_addr());
        let bytes = value.to_le_bytes();
        crate::with_device_write_bypass(bus, |bus| {
            let _ = bus.write_u8(addr, bytes[0]);
            let _ = bus.write_u8(addr + 1, bytes[1]);
        });
        self.cndt += 1;
        if self.ndt >= 2 && self.cndt == self.ndt / 2 {
            self.isr |= DMABHF;
        }
        if self.cndt == self.ndt {
            self.isr |= DMABF;
            if self.autowrap {
                self.cndt = 0;
            } else {
                self.dma_active = false;
            }
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            DATA => u32::from(self.data),
            CR => self.cr,
            OFFSETR => u32::from(self.offset),
            GAINR => u32::from(self.gain),
            DMACR => {
                if self.autowrap {
                    DMA_AUTOWRAP
                } else {
                    0
                }
            }
            DMASR => {
                (if self.dma_active { DMASR_ACT } else { 0 })
                    | (if self.dma_ovf { DMASR_OVF } else { 0 })
            }
            DMAMSAR => self.msa_addr(),
            DMANDTR => self.ndt,
            DMAMNAR => self.next_addr() & !1,
            DMACNDTR => self.cndt,
            ISR => self.isr,
            IER => self.ier,
            _ => {
                crate::census_reg!("stm32w108_adc:Stm32w108Adc", offset, "read");
                0
            }
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            CR => self.cr = value & CR_MASK,
            OFFSETR => self.offset = value as u16,
            GAINR => self.gain = value as u16,
            DMACR => self.write_dmacr(value),
            DMAMSAR => self.msa = value & MSA_MASK,
            DMANDTR => self.ndt = value & NDT_MASK,
            ISR => self.isr &= !(value & FLAGS),
            IER => self.ier = value & FLAGS,
            DATA | DMASR | DMAMNAR | DMACNDTR => {}
            _ => {
                crate::census_reg!("stm32w108_adc:Stm32w108Adc", offset, "write");
            }
        }
    }
}

impl Peripheral for Stm32w108Adc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        match self.inputs_mv.get_mut(channel as usize) {
            Some(slot) => {
                *slot = millivolts;
                true
            }
            None => false,
        }
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        match self.inputs_mv.get_mut(channel as usize) {
            Some(slot) => {
                *slot = u16::MAX;
                true
            }
            None => false,
        }
    }

    fn supports_adc_reference_voltage(&self) -> bool {
        true
    }

    fn set_adc_reference_voltage_v(&mut self, volts: f64) -> bool {
        if !volts.is_finite() || volts <= 0.0 {
            return false;
        }
        self.vref_mv = ((volts * 1000.0).round() as u32).max(1);
        true
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.isr & self.ier != 0)
    }

    fn on_system_reset(&mut self) {
        let (inputs, vref) = (self.inputs_mv, self.vref_mv);
        *self = Self::new(vref);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    /// A byte write touches its own lane only: the other lanes are written
    /// as 0, which leaves W1C flags and the DMA strobes alone, and the
    /// stored registers keep their other bytes.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        let lane = u32::from(value) << shift;
        let word = match aligned {
            ISR | DMACR => lane,
            _ => (self.read_word(aligned) & !(0xFF << shift)) | lane,
        };
        self.write_word(aligned, word);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult {
            irq: self.isr & self.ier != 0,
            ..PeripheralTickResult::default()
        }
    }

    fn needs_bus_tick(&self) -> bool {
        self.cr & ADON != 0
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if self.cr & ADON != 0 {
            self.conversion(bus);
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
    use crate::bus::SystemBus;

    const RAM: u32 = 0x2000_0100;

    fn adc_with(inputs: &[(u8, u16)]) -> Stm32w108Adc {
        let mut adc = Stm32w108Adc::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv), "channel {ch}");
        }
        adc
    }

    /// ADC_CR for CHSELP `p`, CHSELN `n`, 12 bits, 1 MHz, ADON.
    fn cr(p: u32, n: u32) -> u32 {
        (7 << SMP_SHIFT) | (p << CHSELP_SHIFT) | (n << CHSELN_SHIFT) | (1 << 2) | ADON
    }

    /// The ideal single-ended code against VREF/2 at 12 bits.
    fn ideal(mv: i64) -> u16 {
        let raw = (32768 * (mv - 600)).div_euclid(1200).clamp(-32768, 32767) & !0xF;
        ((raw >> 1) as i16) as u16
    }

    fn sample(adc: &mut Stm32w108Adc, p: u32, n: u32) -> u16 {
        adc.write_u32(CR, cr(p, n)).unwrap();
        adc.convert().0
    }

    #[test]
    fn reset_values_match_the_register_map() {
        let adc = Stm32w108Adc::default();
        let expect = [
            (DATA, 0),
            (CR, 0x1800),
            (OFFSETR, 0),
            (GAINR, 0x8000),
            (DMACR, 0),
            (DMASR, 0),
            (DMAMSAR, 0x2000_0000),
            (DMANDTR, 0),
            (DMAMNAR, 0x2000_0000),
            (DMACNDTR, 0),
            (ISR, 0),
            (IER, 0),
        ];
        for (offset, value) in expect {
            assert_eq!(adc.read_u32(offset).unwrap(), value, "offset {offset:#x}");
        }
        let mut adc = adc;
        adc.write_u32(CR, 0xFFFF_FFFF).unwrap();
        assert_eq!(
            adc.read_u32(CR).unwrap(),
            0xFFFD,
            "bit 1 and 31:16 reserved"
        );
        adc.write_u32(DMAMSAR, 0xFFFF_FFFF).unwrap();
        assert_eq!(adc.read_u32(DMAMSAR).unwrap(), 0x2000_1FFF, "MSA[12:0]");
        adc.write_u32(IER, 0xFFFF_FFFF).unwrap();
        assert_eq!(adc.read_u32(IER).unwrap(), 0x1E, "IER bits 4:1");
    }

    #[test]
    fn table_41_calibration_points_convert_to_their_nominal_codes() {
        let mut adc = Stm32w108Adc::default();
        assert_eq!(sample(&mut adc, 8, 9), 0xE000, "GND vs VREF/2");
        assert_eq!(sample(&mut adc, 9, 9), 0x0000, "VREF/2 vs VREF/2");
        assert_eq!(sample(&mut adc, 10, 9), 0x2000, "VREF vs VREF/2");
        assert_eq!(sample(&mut adc, 11, 9), 0x1000, "VREG/2 (0.9 V)");
        // VREF through the P input buffer (gain 0.25): 0.3 V - 0.6 V.
        adc.write_u32(CR, cr(10, 9) | HVSELP).unwrap();
        assert_eq!(adc.convert().0, 0xF000, "VREF, buffered");
    }

    #[test]
    fn external_pads_convert_single_ended_and_differential() {
        let mut adc = adc_with(&[(0, 250), (1, 950), (4, 1200), (5, 0)]);
        assert_eq!(sample(&mut adc, 0, 9), ideal(250), "ADC0 0.25 V");
        assert_eq!(sample(&mut adc, 1, 9), ideal(950), "ADC1 0.95 V");
        assert_eq!(sample(&mut adc, 4, 9), 0x2000, "ADC4 at VREF");
        assert_eq!(sample(&mut adc, 5, 9), 0xE000, "ADC5 at 0 V");
        assert_eq!(sample(&mut adc, 2, 9), 0xE000, "undriven ADC2 reads 0 V");
        // ADC1 - ADC0 = 0.7 V, against VREF.
        let diff = sample(&mut adc, 1, 0);
        assert_eq!(diff, ((((32768 * 700 / 1200) & !0xF) >> 1) as i16) as u16);
        // ADC0 - ADC1 = -0.7 V floors away from zero before the mask.
        let neg = ((-32768i64 * 700).div_euclid(1200) & !0xF) >> 1;
        assert_eq!(sample(&mut adc, 0, 1) as i16 as i64, neg);
        assert_eq!(neg, -9560);
    }

    #[test]
    fn smp_keeps_only_the_significant_bits() {
        let mut adc = adc_with(&[(0, 777)]);
        for smp in 0..8u32 {
            let bits = 5 + smp;
            adc.write_u32(
                CR,
                (smp << SMP_SHIFT) | (0 << CHSELP_SHIFT) | (9 << CHSELN_SHIFT) | ADON,
            )
            .unwrap();
            let raw = ((32768 * (777 - 600)) / 1200) & !((1 << (16 - bits)) - 1);
            assert_eq!(adc.convert().0, (raw >> 1) as u16, "SMP {smp}");
        }
    }

    #[test]
    fn offset_and_gain_correct_then_halve_and_saturate() {
        let mut adc = adc_with(&[(0, 1000)]);
        adc.write_u32(CR, cr(0, 9)).unwrap();
        let raw = (32768 * 400 / 1200) & !0xF;
        adc.write_u32(OFFSETR, (-1000i16) as u16 as u32).unwrap();
        adc.write_u32(GAINR, 0xC000).unwrap(); // 1.5
        let (value, sat) = adc.convert();
        assert_eq!(value as i16 as i64, (((raw - 1000) * 0xC000) >> 15) >> 1);
        assert!(!sat);
        // Table 41: gain 32768 * 16384 / (NVREF - NGND) and offset
        // 2 * (57344 - NGND) bring an ideal converter back to itself.
        adc.write_u32(OFFSETR, 0).unwrap();
        adc.write_u32(GAINR, 0xFFFF).unwrap();
        adc.write_u32(CR, cr(10, 8)).unwrap(); // VREF vs GND = +32767 raw
        let (value, sat) = adc.convert();
        assert!(sat, "1.99 * 32767 saturates");
        assert_eq!(value, 0x3FFF, "the limited product halved");
    }

    fn bus_with(adc: Stm32w108Adc) -> SystemBus {
        let mut bus = SystemBus::new();
        bus.add_peripheral("adc", 0x4000_D000, 0x1000, None, Box::new(adc));
        bus
    }

    fn adc_mut(bus: &mut SystemBus) -> &mut Stm32w108Adc {
        bus.peripherals
            .iter_mut()
            .find(|p| p.name == "adc")
            .unwrap()
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Stm32w108Adc>()
            .unwrap()
    }

    fn step(bus: &mut SystemBus) {
        let mut adc = std::mem::take(adc_mut(bus));
        adc.tick_with_bus(bus);
        *adc_mut(bus) = adc;
    }

    #[test]
    fn linear_dma_fills_the_buffer_then_overflows() {
        let mut bus = bus_with(adc_with(&[(4, 900)]));
        let adc = adc_mut(&mut bus);
        adc.write_u32(DMACR, DMA_RST).unwrap();
        adc.write_u32(DMAMSAR, RAM).unwrap();
        adc.write_u32(DMANDTR, 4).unwrap();
        adc.write_u32(DMACR, DMA_LOAD).unwrap();
        adc.write_u32(IER, DMABF).unwrap();
        assert_eq!(adc.read_u32(DMASR).unwrap(), DMASR_ACT);
        adc.write_u32(CR, cr(4, 9)).unwrap();
        for n in 1..=4u32 {
            step(&mut bus);
            let adc = adc_mut(&mut bus);
            assert_eq!(adc.read_u32(DMACNDTR).unwrap(), n);
            assert_eq!(
                adc.read_u32(ISR).unwrap() & DMABHF != 0,
                n >= 2,
                "BHF at 2/4"
            );
            assert_eq!(adc.irq_line_level(), Some(n == 4), "BF enabled");
        }
        let code = ideal(900);
        for i in 0..4u64 {
            assert_eq!(
                bus.read_u16(u64::from(RAM) + 2 * i).unwrap(),
                code,
                "sample {i}"
            );
        }
        let adc = adc_mut(&mut bus);
        assert_eq!(adc.read_u32(DMASR).unwrap(), 0, "linear mode stops");
        assert_eq!(adc.read_u32(DATA).unwrap(), u32::from(code));
        step(&mut bus);
        let adc = adc_mut(&mut bus);
        assert_eq!(adc.read_u32(DMASR).unwrap(), DMASR_OVF);
        assert_ne!(adc.read_u32(ISR).unwrap() & DMAOVF, 0);
        adc.write_u32(ISR, DMABF | DMAOVF | DMABHF).unwrap();
        assert_eq!(adc.read_u32(ISR).unwrap(), 0, "write 1 clears");
        adc.write_u32(DMACR, DMA_RST).unwrap();
        assert_eq!(adc.read_u32(DMASR).unwrap(), 0, "reset clears OVF");
    }

    #[test]
    fn autowrap_restarts_at_the_start_address() {
        let mut bus = bus_with(adc_with(&[(0, 300), (1, 1100)]));
        let adc = adc_mut(&mut bus);
        adc.write_u32(DMAMSAR, RAM).unwrap();
        adc.write_u32(DMANDTR, 2).unwrap();
        adc.write_u32(DMACR, DMA_AUTOWRAP | DMA_LOAD).unwrap();
        adc.write_u32(CR, cr(0, 9)).unwrap();
        step(&mut bus);
        step(&mut bus);
        adc_mut(&mut bus).write_u32(CR, cr(1, 9)).unwrap();
        step(&mut bus);
        let adc = adc_mut(&mut bus);
        assert_eq!(adc.read_u32(DMASR).unwrap(), DMASR_ACT, "still active");
        assert_eq!(adc.read_u32(DMAMNAR).unwrap(), RAM + 2);
        assert_eq!(
            bus.read_u16(u64::from(RAM)).unwrap(),
            ideal(1100),
            "wrapped"
        );
        assert_eq!(bus.read_u16(u64::from(RAM) + 2).unwrap(), ideal(300));
    }

    #[test]
    fn adon_off_converts_nothing_and_byte_writes_keep_flags() {
        let mut bus = bus_with(adc_with(&[(0, 300)]));
        let adc = adc_mut(&mut bus);
        assert!(!adc.needs_bus_tick(), "reset: ADON = 0");
        adc.write_u32(CR, cr(0, 9) & !ADON).unwrap();
        step(&mut bus);
        let adc = adc_mut(&mut bus);
        assert_eq!(adc.read_u32(ISR).unwrap(), 0, "no conversion, no OVF");
        adc.isr = DMABF | DMAOVF;
        adc.write(ISR, DMABF as u8).unwrap();
        assert_eq!(adc.read_u32(ISR).unwrap(), DMAOVF, "byte W1C");
        adc.write(CR + 1, 0x00).unwrap();
        assert_eq!(adc.read_u32(CR).unwrap(), cr(0, 9) & !ADON & 0xFF);
    }

    #[test]
    fn system_reset_keeps_routed_inputs_and_vref() {
        let mut adc = adc_with(&[(3, 800)]);
        assert!(adc.set_adc_reference_voltage_v(1.25));
        adc.write_u32(CR, cr(3, 9)).unwrap();
        adc.on_system_reset();
        assert_eq!(adc.read_u32(CR).unwrap(), CR_RESET);
        adc.write_u32(CR, cr(3, 9)).unwrap();
        let raw = ((32768 * (800 * 4 - 625 * 4)) / (4 * 1250)) & !0xF;
        assert_eq!(adc.convert().0, (raw >> 1) as u16);
    }
}
