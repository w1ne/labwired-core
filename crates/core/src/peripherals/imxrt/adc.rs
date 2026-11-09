// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! NXP 12-bit SAR ADC with the `HCn`/`HS`/`Rn`/`CFG`/`GC`/`GS` register set,
//! in two layouts of the same IP:
//!
//! - [`AdcLayout::Imxrt`]: i.MX RT1060 (ADC1 `0x400C_4000`, ADC2
//!   `0x400C_8000`, IMXRT1060RM §66): `HC0..HC7` @0x00, `HS` @0x20,
//!   `R0..R7` @0x24, `CFG` @0x44, `GC` @0x48, `GS` @0x4C, `CV` @0x50,
//!   `OFS` @0x54, `CAL` @0x58.
//! - [`AdcLayout::Vf610`]: Vybrid VF6xx (ADC0 `0x4003_B000`, ADC1
//!   `0x400B_B000`, Linux v6.12 `arch/arm/boot/dts/nxp/vf/vfxxx.dtsi`):
//!   `HC0` @0x00, `HC1` @0x04, `HS` @0x08, `R0` @0x0C, `R1` @0x10, `CFG`
//!   @0x14, `GC` @0x18, `GS` @0x1C, `CV` @0x20, `OFS` @0x24, `CAL` @0x28,
//!   `PCTL` @0x30 (Linux v6.12 `drivers/iio/adc/vf610_adc.c`
//!   `VF610_REG_ADC_*`). The fields are the RT ones bit for bit: the driver's
//!   `VF610_ADC_MODE_*` (CFG[3:2]), `ADLSMP`/`ADSTS`/`ADLPC`/`ADHSC`/`REFSEL`/
//!   `ADTRG`/`AVGS`/`OVWREN` (CFG[16:4]), `ADACKEN`..`CAL` (GC[7:0]),
//!   `ADCHC` (HC[4:0], 0x1F = disabled), `AIEN` (HC bit 7), `HS_COCO0`
//!   (bit 0) and `CALF` (GS bit 1). `PCTL` is stored only.
//!
//! Software-triggered conversions (`CFG.ADTRG` = 0): a write to `HC0` with
//! `ADCH` != 0x1F starts a conversion of that channel; after the conversion
//! time `R0` holds the result and `HS.COCO0` is set (cleared by reading `R0`
//! or writing `HC0`). `GC.ADCO` repeats. `GC.CAL` runs the calibration
//! sequence: `GS.CALF` is cleared, and after the calibration time `GC.CAL`
//! self-clears and `HS.COCO0` is set (RM §66.5.6; the Linux VF610 driver
//! writes `HC0 = AIEN | 0x1F`, sets `GC.CAL` and waits for that COCO0
//! interrupt). The interrupt line is `COCO0 & HC0.AIEN`. `HC1..` (hardware
//! triggers) are stored only.
//!
//! Channel voltages are board inputs (`Peripheral::set_adc_channel_input`,
//! millivolts, channels 0..15); the reference is VREFH = 3.3 V (the
//! Colibri VF61 `vref-supply` is the 3.3 V module AVDD, vf-colibri.dtsi).

use super::{byte_of, Timebase};
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::cell::Cell;
use std::collections::BTreeMap;

const GC_ADCO: u32 = 1 << 6;
const GC_CAL: u32 = 1 << 7;
const GS_ADACT: u32 = 1 << 0;
const GS_CALF: u32 = 1 << 1;
const GS_AWKST: u32 = 1 << 2;
const HC_AIEN: u32 = 1 << 7;
const CFG_ADTRG: u32 = 1 << 13;

/// Conversion time: with the SDK defaults (ADACK, long sample off,
/// hardware average 4) one result takes a few µs.
const CONVERSION_US: u64 = 3;
/// Calibration: several hundred ADC clocks (RM §66.5.6).
const CALIBRATION_US: u64 = 50;
const VREF_MV: u32 = 3300;

/// Register map of one instance of the IP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AdcLayout {
    /// i.MX RT10xx: eight HC/R pairs.
    #[default]
    Imxrt,
    /// Vybrid VF6xx: two HC/R pairs and `PCTL`.
    Vf610,
}

struct Offsets {
    /// Number of `HCn` (and `Rn`) registers, from offset 0.
    hc_count: u32,
    hs: u32,
    r0: u32,
    cfg: u32,
    gc: u32,
    gs: u32,
    cv: u32,
    ofs: u32,
    cal: u32,
    pctl: Option<u32>,
}

impl AdcLayout {
    const fn offsets(self) -> Offsets {
        match self {
            AdcLayout::Imxrt => Offsets {
                hc_count: 8,
                hs: 0x20,
                r0: 0x24,
                cfg: 0x44,
                gc: 0x48,
                gs: 0x4C,
                cv: 0x50,
                ofs: 0x54,
                cal: 0x58,
                pctl: None,
            },
            AdcLayout::Vf610 => Offsets {
                hc_count: 2,
                hs: 0x08,
                r0: 0x0C,
                cfg: 0x14,
                gc: 0x18,
                gs: 0x1C,
                cv: 0x20,
                ofs: 0x24,
                cal: 0x28,
                pctl: Some(0x30),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reg {
    Hc(usize),
    Hs,
    R(usize),
    Cfg,
    Gc,
    Gs,
    Cv,
    Ofs,
    Cal,
    Pctl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Busy {
    Idle,
    Convert { channel: u8, done_at: u64 },
    Calibrate { done_at: u64 },
}

#[derive(Debug)]
pub struct ImxrtAdc {
    layout: AdcLayout,
    hc: [u32; 8],
    r: [Cell<u32>; 8],
    cfg: u32,
    gc: Cell<u32>,
    gs: Cell<u32>,
    cv: u32,
    ofs: u32,
    cal: u32,
    pctl: u32,
    coco: Cell<bool>,
    busy: Cell<Busy>,
    inputs: BTreeMap<u8, u16>,
    time: Timebase,
}

impl Default for ImxrtAdc {
    fn default() -> Self {
        Self::new()
    }
}

impl ImxrtAdc {
    pub fn new() -> Self {
        Self::with_layout(AdcLayout::Imxrt)
    }

    pub fn with_layout(layout: AdcLayout) -> Self {
        Self {
            layout,
            hc: [0x1F; 8],
            r: Default::default(),
            cfg: 0x0000_0200,
            gc: Cell::new(0),
            gs: Cell::new(0),
            cv: 0,
            ofs: 0,
            cal: 0,
            pctl: 0,
            coco: Cell::new(false),
            busy: Cell::new(Busy::Idle),
            inputs: BTreeMap::new(),
            time: Timebase::default(),
        }
    }

    fn decode(&self, off: u32) -> Option<Reg> {
        let o = self.layout.offsets();
        let off = off & !3;
        let span = 4 * o.hc_count;
        Some(match off {
            _ if off < span => Reg::Hc((off / 4) as usize),
            _ if off == o.hs => Reg::Hs,
            _ if (o.r0..o.r0 + span).contains(&off) => Reg::R(((off - o.r0) / 4) as usize),
            _ if off == o.cfg => Reg::Cfg,
            _ if off == o.gc => Reg::Gc,
            _ if off == o.gs => Reg::Gs,
            _ if off == o.cv => Reg::Cv,
            _ if off == o.ofs => Reg::Ofs,
            _ if off == o.cal => Reg::Cal,
            _ if Some(off) == o.pctl => Reg::Pctl,
            _ => return None,
        })
    }

    fn resolution_bits(&self) -> u32 {
        match (self.cfg >> 2) & 3 {
            0 => 8,
            1 => 10,
            _ => 12,
        }
    }

    fn sample(&self, channel: u8) -> u32 {
        let mv = self.inputs.get(&channel).copied().unwrap_or(0) as u32;
        let full = (1u32 << self.resolution_bits()) - 1;
        (mv.min(VREF_MV) * full + VREF_MV / 2) / VREF_MV
    }

    /// Bring the conversion/calibration state machine up to "now".
    fn settle(&self) {
        let now = self.time.now();
        match self.busy.get() {
            Busy::Convert { channel, done_at } if now >= done_at => {
                self.r[0].set(self.sample(channel));
                self.coco.set(true);
                if self.gc.get() & GC_ADCO != 0 {
                    self.busy.set(Busy::Convert {
                        channel,
                        done_at: done_at + self.time.us(CONVERSION_US),
                    });
                } else {
                    self.busy.set(Busy::Idle);
                }
            }
            Busy::Calibrate { done_at } if now >= done_at => {
                self.gc.set(self.gc.get() & !GC_CAL);
                self.coco.set(true);
                self.busy.set(Busy::Idle);
            }
            _ => {}
        }
    }

    pub fn read_reg(&self, off: u32) -> u32 {
        self.settle();
        match self.decode(off) {
            Some(Reg::Hc(i)) => self.hc[i],
            Some(Reg::Hs) => self.coco.get() as u32,
            Some(Reg::R(i)) => {
                if i == 0 {
                    // Reading R0 clears COCO0.
                    self.coco.set(false);
                }
                self.r[i].get()
            }
            Some(Reg::Cfg) => self.cfg,
            Some(Reg::Gc) => self.gc.get(),
            Some(Reg::Gs) => {
                let active = !matches!(self.busy.get(), Busy::Idle);
                (self.gs.get() & !GS_ADACT) | if active { GS_ADACT } else { 0 }
            }
            Some(Reg::Cv) => self.cv,
            Some(Reg::Ofs) => self.ofs,
            Some(Reg::Cal) => self.cal,
            Some(Reg::Pctl) => self.pctl,
            None => 0,
        }
    }

    pub fn write_reg(&mut self, off: u32, value: u32, mask: u32) {
        self.settle();
        let merge = |old: u32| (old & !mask) | (value & mask);
        match self.decode(off) {
            Some(Reg::Hc(0)) => {
                self.hc[0] = merge(self.hc[0]);
                self.coco.set(false);
                let ch = (self.hc[0] & 0x1F) as u8;
                if ch == 0x1F {
                    self.busy.set(Busy::Idle);
                } else if self.cfg & CFG_ADTRG == 0 {
                    self.busy.set(Busy::Convert {
                        channel: ch,
                        done_at: self.time.now() + self.time.us(CONVERSION_US),
                    });
                }
            }
            Some(Reg::Hc(i)) => self.hc[i] = merge(self.hc[i]),
            Some(Reg::Cfg) => self.cfg = merge(self.cfg) & 0x0001_FFFF,
            Some(Reg::Gc) => {
                let new = merge(self.gc.get()) & 0xFF;
                self.gc.set(new);
                if new & GC_CAL != 0 && !matches!(self.busy.get(), Busy::Calibrate { .. }) {
                    // Starting calibration clears CALF and any result.
                    self.gs.set(self.gs.get() & !GS_CALF);
                    self.coco.set(false);
                    self.busy.set(Busy::Calibrate {
                        done_at: self.time.now() + self.time.us(CALIBRATION_US),
                    });
                }
            }
            Some(Reg::Gs) => {
                // CALF and AWKST are write-1-to-clear.
                let clr = value & mask & (GS_CALF | GS_AWKST);
                self.gs.set(self.gs.get() & !clr);
            }
            Some(Reg::Cv) => self.cv = merge(self.cv),
            Some(Reg::Ofs) => self.ofs = merge(self.ofs) & 0x1FFF,
            Some(Reg::Cal) => self.cal = merge(self.cal) & 0xF,
            Some(Reg::Pctl) => self.pctl = merge(self.pctl),
            Some(Reg::Hs | Reg::R(_)) | None => {}
        }
    }

    fn irq_level(&self) -> bool {
        self.settle();
        self.coco.get() && self.hc[0] & HC_AIEN != 0
    }
}

impl ImxrtAdc {
    /// Recompute the cached interrupt line (see `Timebase::level`).
    fn refresh_irq(&self) {
        self.time.set_level(self.irq_level());
    }

    fn tick_inner(&mut self, cycles: u64) -> PeripheralTickResult {
        self.time.advance(cycles);
        self.settle();
        let until = match self.busy.get() {
            Busy::Convert { done_at, .. } | Busy::Calibrate { done_at } => Some(done_at),
            Busy::Idle => None,
        };
        super::wake_hint(self.time.now(), until)
    }
}

impl Peripheral for ImxrtAdc {
    /// Walked only while timed work is in flight or the interrupt line is
    /// asserted (so its deassert is reconciled); MMIO re-arms it.
    fn legacy_tick_active(&self) -> bool {
        (!matches!(self.busy.get(), Busy::Idle)) || self.time.level()
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let v = byte_of(self.read_reg(offset as u32), offset);
        self.refresh_irq();
        Ok(v)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset & 3) * 8;
        self.write_reg(offset as u32, (value as u32) << shift, 0xFF << shift);
        self.refresh_irq();
        Ok(())
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let v = self.read_reg(offset as u32);
        self.refresh_irq();
        Ok(v)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset as u32, value, u32::MAX);
        self.refresh_irq();
        Ok(())
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        // No side effects: R0 reads here must not clear COCO0.
        let off = offset as u32 & !3;
        let v = match self.decode(off) {
            Some(Reg::R(0)) => self.r[0].get(),
            Some(Reg::Hs) => self.coco.get() as u32,
            _ => self.read_reg(off),
        };
        Some(byte_of(v, offset))
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        let r = self.tick_inner(cycles);
        self.refresh_irq();
        r
    }
    fn irq_line_level(&self) -> Option<bool> {
        Some(self.time.level())
    }
    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        if channel < 16 {
            self.inputs.insert(channel, millivolts);
            true
        } else {
            false
        }
    }
    fn adc_channel_count(&self) -> Option<u8> {
        Some(16)
    }
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.time.attach_clock(clock);
    }
    fn attach_cpu_hz(&mut self, hz: u64) {
        self.time.attach_cpu_hz(hz);
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
    use crate::CycleClock;

    // i.MX RT1060 offsets (IMXRT1060RM §66.8).
    const HC0: u32 = 0x00;
    const HS: u32 = 0x20;
    const R0: u32 = 0x24;
    const CFG: u32 = 0x44;
    const GC: u32 = 0x48;
    const GS: u32 = 0x4C;

    fn adc() -> (ImxrtAdc, CycleClock) {
        let mut a = ImxrtAdc::new();
        let c = CycleClock::default();
        a.attach_cycle_clock(c.clone());
        (a, c)
    }

    #[test]
    fn calibration_completes_and_sets_coco() {
        let (mut a, c) = adc();
        // SDK ADC_DoAutoCalibration: GS = CALF (w1c), GC |= CAL, poll CAL.
        a.write_reg(GS, GS_CALF, u32::MAX);
        a.write_reg(GC, GC_CAL, u32::MAX);
        assert_ne!(a.read_reg(GC) & GC_CAL, 0, "calibration is not instant");
        c.publish(a.time.us(CALIBRATION_US));
        assert_eq!(a.read_reg(GC) & GC_CAL, 0);
        assert_eq!(a.read_reg(GS) & GS_CALF, 0);
        assert_eq!(a.read_reg(HS) & 1, 1);
    }

    #[test]
    fn conversion_reads_the_channel_voltage() {
        let (mut a, c) = adc();
        a.write_reg(CFG, 2 << 2, u32::MAX); // 12-bit
        a.set_adc_channel_input(9, 2500);
        a.write_reg(HC0, 9, u32::MAX);
        assert_eq!(a.read_reg(HS), 0);
        c.publish(a.time.us(CONVERSION_US));
        assert_eq!(a.read_reg(HS), 1);
        assert_eq!(a.read_reg(R0), (2500 * 4095 + 1650) / 3300);
        assert_eq!(a.read_reg(HS), 0, "reading R0 clears COCO0");
    }

    /// Linux v6.12 vf610_adc.c `VF610_REG_ADC_*`.
    mod vf610 {
        pub const HC0: u32 = 0x00;
        pub const HC1: u32 = 0x04;
        pub const HS: u32 = 0x08;
        pub const R0: u32 = 0x0C;
        pub const CFG: u32 = 0x14;
        pub const GC: u32 = 0x18;
        pub const GS: u32 = 0x1C;
        pub const CV: u32 = 0x20;
        pub const PCTL: u32 = 0x30;
        pub const MODE_BIT8: u32 = 0x00;
        pub const MODE_BIT10: u32 = 0x04;
        pub const MODE_BIT12: u32 = 0x08;
        pub const CLK_DIV8: u32 = 0x60;
        pub const ADLPC_EN: u32 = 0x80;
        pub const ADHSC_EN: u32 = 0x400;
        pub const OVWREN: u32 = 0x10000;
        pub const ADCON: u32 = 0x40;
        pub const CAL: u32 = 0x80;
        pub const AIEN: u32 = 1 << 7;
        pub const CONV_DISABLE: u32 = 0x1F;
        pub const COCO0: u32 = 0x1;
        pub const CALF: u32 = 0x2;
    }

    fn vf610_adc() -> (ImxrtAdc, CycleClock) {
        let mut a = ImxrtAdc::with_layout(AdcLayout::Vf610);
        let c = CycleClock::default();
        a.attach_cycle_clock(c.clone());
        (a, c)
    }

    /// Let `us` microseconds pass on the published bus clock.
    fn wait(a: &ImxrtAdc, c: &CycleClock, us: u64) {
        c.publish(a.time.now() + a.time.us(us));
    }

    /// The driver ISR: on COCO0, read R0 masked to the resolution.
    fn vf610_isr(a: &ImxrtAdc, bits: u32) -> Option<u32> {
        (a.read_reg(vf610::HS) & vf610::COCO0 != 0)
            .then(|| a.read_reg(vf610::R0) & ((1 << bits) - 1))
    }

    fn code(mv: u32, bits: u32) -> u32 {
        let full = (1 << bits) - 1;
        (mv.min(3300) * full + 1650) / 3300
    }

    #[test]
    fn vf610_layout_resets_and_places_every_register() {
        use vf610::*;
        let (mut a, _) = vf610_adc();
        assert_eq!(a.read_reg(HC0), 0x1F, "HC0 resets to conversion disabled");
        assert_eq!(a.read_reg(HC1), 0x1F);
        assert_eq!(a.read_reg(CFG), 0x200);
        assert_eq!(a.read_reg(HS), 0);
        for (reg, value) in [(CV, 0x0ABC_0123), (PCTL, 0x00FF_00FF), (HC1, 0x85)] {
            a.write_reg(reg, value, u32::MAX);
            assert_eq!(a.read_reg(reg), value, "{reg:#x} is stored");
        }
        // HC1 is a hardware-trigger slot: writing it starts nothing.
        assert_eq!(a.read_reg(GS) & 1, 0, "HC1 leaves the converter idle");
        // The RT offsets past PCTL are not registers on the VF610.
        assert_eq!(a.read_reg(0x44), 0);
        a.write_reg(0x48, CAL, u32::MAX);
        assert_eq!(a.read_reg(GC), 0, "0x48 is not GC on the VF610");
    }

    #[test]
    fn vf610_linux_probe_calibrates_then_reads_each_resolution() {
        use vf610::*;
        let (mut a, c) = vf610_adc();
        a.set_adc_channel_input(5, 1650);
        a.set_adc_channel_input(9, 3000);
        // vf610_adc_cfg_post_set + vf610_adc_sample_set (12 bit, /8).
        a.write_reg(CFG, ADLPC_EN | ADHSC_EN | OVWREN, u32::MAX);
        a.write_reg(GC, 0, u32::MAX);
        let cfg = a.read_reg(CFG);
        a.write_reg(CFG, (cfg & !0x6C) | MODE_BIT12 | CLK_DIV8, u32::MAX);
        // vf610_adc_calibration: AIEN with conversions disabled, GC |= CAL,
        // wait for the COCO0 interrupt, then check GS.CALF.
        a.write_reg(HC0, AIEN | CONV_DISABLE, u32::MAX);
        let gc = a.read_reg(GC);
        a.write_reg(GC, gc | CAL, u32::MAX);
        assert!(!a.irq_level(), "calibration takes time");
        assert_eq!(a.read_reg(GS) & 1, 1, "ADACT while calibrating");
        wait(&a, &c, CALIBRATION_US);
        assert!(a.irq_level(), "calibration end interrupts through AIEN");
        assert_eq!(a.read_reg(GC) & CAL, 0, "GC.CAL self-clears");
        assert_eq!(a.read_reg(GS) & CALF, 0, "calibration passed");
        assert!(vf610_isr(&a, 12).is_some());
        assert!(!a.irq_level(), "reading R0 drops COCO0");

        // vf610_read_sample: HC0 = ADCHC(ch) | AIEN, wait for the ISR.
        for (mode, bits) in [(MODE_BIT12, 12), (MODE_BIT10, 10), (MODE_BIT8, 8)] {
            let cfg = a.read_reg(CFG);
            a.write_reg(CFG, (cfg & !0xC) | mode, u32::MAX);
            for (ch, mv) in [(5u32, 1650u32), (9, 3000), (7, 0)] {
                a.write_reg(HC0, ch | AIEN, u32::MAX);
                assert_eq!(vf610_isr(&a, bits), None, "CH{ch}: not converted yet");
                wait(&a, &c, CONVERSION_US);
                assert!(a.irq_level(), "CH{ch}: COCO0 & AIEN");
                assert_eq!(
                    vf610_isr(&a, bits),
                    Some(code(mv, bits)),
                    "CH{ch} {bits}-bit"
                );
            }
        }
    }

    #[test]
    fn vf610_buffer_mode_reconverts_until_predisable() {
        use vf610::*;
        let (mut a, c) = vf610_adc();
        a.write_reg(CFG, MODE_BIT12, u32::MAX);
        a.set_adc_channel_input(6, 500);
        // vf610_adc_buffer_postenable: GC |= ADCON, HC0 = ch | AIEN.
        let gc = a.read_reg(GC);
        a.write_reg(GC, gc | ADCON, u32::MAX);
        a.write_reg(HC0, 6 | AIEN, u32::MAX);
        wait(&a, &c, CONVERSION_US);
        assert_eq!(vf610_isr(&a, 12), Some(code(500, 12)));
        a.set_adc_channel_input(6, 2500);
        wait(&a, &c, CONVERSION_US);
        assert_eq!(
            vf610_isr(&a, 12),
            Some(code(2500, 12)),
            "next result follows the pad"
        );
        // vf610_adc_buffer_predisable: GC &= ~ADCON, HC0 = CONV_DISABLE.
        let gc = a.read_reg(GC);
        a.write_reg(GC, gc & !ADCON, u32::MAX);
        a.write_reg(HC0, CONV_DISABLE, u32::MAX);
        wait(&a, &c, 10 * CONVERSION_US);
        assert_eq!(vf610_isr(&a, 12), None, "no conversion after predisable");
    }
}
