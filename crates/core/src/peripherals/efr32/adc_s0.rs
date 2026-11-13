// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Silicon Labs EFM32/EZR32 Series-0 **ADC0**: the 12-bit SAR ADC with a
//! fixed eight-channel input mux, on the Gecko (G), Giant (GG), Leopard
//! (LG), Wonder (WG), Tiny (TG), Zero (ZG) and Happy (HG) dies and the
//! EZR32 HG/LG/WG parts built on them.
//!
//! # Sources
//!
//! * Register offsets are walked from `ADC_TypeDef` in gecko_sdk tag
//!   `gsdk_4.4` `efm32gg_adc.h`. Field positions, enumerations and reset
//!   values are the `_ADC_<REG>_<FIELD>_*` defines in the same header.
//!   `efm32g_adc.h`, `efm32tg_adc.h`, `efm32zg_adc.h`, `efm32hg_adc.h`,
//!   `efm32lg_adc.h`, `efm32wg_adc.h` and `ezr32{hg,lg,wg}_adc.h` declare the
//!   same struct and the same CMD/STATUS/IF/SINGLECTRL fields. Only the
//!   width of `SINGLECTRL.PRSSEL` differs, and nothing here reads it.
//! * ADC0 sits at `ADC0_BASE` = 0x4000_2000 on every Series-0 die.
//!   `ADC0_IRQn` is 6 on G and TG, 7 on GG, LG and WG (and EZR32LG/WG),
//!   4 on ZG and HG (and EZR32HG) (the `efm32*f*.h` device headers).
//! * Channel `CHn` is `SINGLECTRL.INPUTSEL` = n. The vendor's
//!   `peripheral_examples_gsdk` `series0/adc` readmes put ADC0 CH7 on PD7
//!   on EFM32G890, GG990, HG322, LG990, TG840, WG990 and ZG222, and CH6 on
//!   PD6 (`adc_single_diff_interrupt`). Pads live in the chip descriptor,
//!   not here: this model keys its inputs by channel.
//! * `SINGLEDATA` has `readAction` = modifyExternal in the cmsis-svd-data
//!   `EFM32GG995F1024.svd` and `SINGLEDATAP` is the "Peek Register": the
//!   `SINGLEDATA` read clears `STATUS.SINGLEDV`, the peek does not.
//! * The firmware sequence is emlib `em_adc.c`/`em_adc.h` (gsdk_4.4):
//!   `ADC_InitSingle` writes `SINGLECTRL`, `ADC_Start(adcStartSingle)`
//!   writes `CMD.SINGLESTART`, the vendor example then polls
//!   `STATUS.SINGLEDV` and reads `SINGLEDATA` through `ADC_DataSingleGet`.
//!
//! # What is modelled
//!
//! * `INPUTSEL` CH0..CH7 converts the level routed to that channel.
//!   `VDD` converts the supply, `VDDDIV3` a third of it, `VSS` 0, and
//!   `VREFDIV2` half the selected reference.
//! * `REF` = 1V25 / 2V5 / VDD sets full scale. `RES` = 12, 8 or 6 bits. A
//!   code is `round-down(mV * 2^bits / Vref)`, saturating at all-ones.
//! * One result register. A finished conversion sets `STATUS.SINGLEDV` and
//!   `IF.SINGLE`; one that lands while `SINGLEDV` is still set also
//!   latches `IF.SINGLEOF`. `IF` is read-only; `IFS` sets and `IFC` clears;
//!   `IEN & IF` holds the IRQ line.
//!
//! # Idealised — present, but not physical
//!
//! * **Conversion is instantaneous**: the result lands on the tick after
//!   `SINGLESTART`. No warm-up, prescaler, timebase or acquisition time,
//!   and `STATUS.WARM` stays 0 (emlib never polls it).
//! * On `SINGLEOF` the new code replaces the unread one. No source in hand
//!   says whether silicon keeps the old or the new word.
//! * **No clock gate.** `CMU_HFPERCLKEN0.ADC0` is not checked: the
//!   onboarding CMU is a stub.
//! * **Single-ended, right-adjusted, one-shot only.** `DIFF`, `ADJ` = LEFT,
//!   `RES` = OVS, the `REF` values other than 1V25/2V5/VDD and the `TEMP`
//!   and `DAC0OUTn` inputs are stored, and a conversion under them produces
//!   NO result, so polling firmware hangs loudly instead of reading a
//!   plausible wrong code. `REP` is stored and ignored.
//! * **No scan sequence.** `SCANCTRL` stores and `SCANSTART` converts
//!   nothing. **No calibration**: `CAL` stores; codes are not corrected.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::sync::atomic::{AtomicBool, Ordering};

// ── Register offsets, walked from `ADC_TypeDef` ────────────────────────────
const OFF_CTRL: u64 = 0x00;
const OFF_CMD: u64 = 0x04;
const OFF_STATUS: u64 = 0x08;
const OFF_SINGLECTRL: u64 = 0x0C;
const OFF_SCANCTRL: u64 = 0x10;
const OFF_IEN: u64 = 0x14;
const OFF_IF: u64 = 0x18;
const OFF_IFS: u64 = 0x1C;
const OFF_IFC: u64 = 0x20;
const OFF_SINGLEDATA: u64 = 0x24;
const OFF_SCANDATA: u64 = 0x28;
const OFF_SINGLEDATAP: u64 = 0x2C;
const OFF_SCANDATAP: u64 = 0x30;
const OFF_CAL: u64 = 0x34;
/// After `RESERVED0[1]`, the last word of `ADC_TypeDef`.
const OFF_BIASPROG: u64 = 0x3C;

// ── Reset values (`_ADC_<REG>_RESETVALUE`) ─────────────────────────────────
const CTRL_RESET: u32 = 0x001F_0000;
const CAL_RESET: u32 = 0x3F00_3F00;
const BIASPROG_RESET: u32 = 0x0000_0747;

// ── Field positions ────────────────────────────────────────────────────────
const CMD_SINGLESTART: u32 = 1 << 0;
const CMD_SINGLESTOP: u32 = 1 << 1;
const STATUS_SINGLEACT: u32 = 1 << 0;
const STATUS_SINGLEDV: u32 = 1 << 16;
const IF_SINGLE: u32 = 1 << 0;
const IF_SINGLEOF: u32 = 1 << 8;
/// `_ADC_IF_MASK`, the same on every Series-0 die.
const IF_MASK: u32 = 0x0000_0303;

const SINGLECTRL_DIFF: u32 = 1 << 1;
const SINGLECTRL_ADJ_LEFT: u32 = 1 << 2;
const SINGLECTRL_RES_SHIFT: u32 = 4;
const SINGLECTRL_RES_MASK: u32 = 0x3;
const SINGLECTRL_INPUTSEL_SHIFT: u32 = 8;
const SINGLECTRL_INPUTSEL_MASK: u32 = 0xF;
const SINGLECTRL_REF_SHIFT: u32 = 16;
const SINGLECTRL_REF_MASK: u32 = 0x7;

/// `_ADC_SINGLECTRL_RES_*`.
const RES_12BIT: u32 = 0;
const RES_8BIT: u32 = 1;
const RES_6BIT: u32 = 2;
/// `_ADC_SINGLECTRL_REF_*`.
const REF_1V25: u32 = 0;
const REF_2V5: u32 = 1;
const REF_VDD: u32 = 2;
/// `_ADC_SINGLECTRL_INPUTSEL_*` single-ended internal inputs.
const INPUTSEL_VDDDIV3: u32 = 9;
const INPUTSEL_VDD: u32 = 10;
const INPUTSEL_VSS: u32 = 11;
const INPUTSEL_VREFDIV2: u32 = 12;

/// CH0..CH7, the external channels.
const CHANNELS: u8 = 8;

/// VDD on the Starter Kits, in millivolts.
const DEFAULT_VDD_MV: u32 = 3300;

/// The EFM32/EZR32 Series-0 ADC0.
#[derive(Debug)]
pub struct Efm32s0Adc {
    ctrl: u32,
    singlectrl: u32,
    scanctrl: u32,
    ien: u32,
    iflag: u32,
    cal: u32,
    biasprog: u32,

    /// Level on each channel in millivolts, CH0..CH7.
    inputs: [u16; CHANNELS as usize],
    vdd_mv: u32,

    singledata: u32,
    /// `STATUS.SINGLEDV`. Atomic because the `SINGLEDATA` read clears it
    /// and the read path is `&self`.
    single_dv: AtomicBool,
    conversion_pending: bool,

    clock: Option<CycleClock>,
    chain_live: bool,
}

impl Default for Efm32s0Adc {
    fn default() -> Self {
        Self::new()
    }
}

impl Efm32s0Adc {
    pub fn new() -> Self {
        Self {
            ctrl: CTRL_RESET,
            singlectrl: 0,
            scanctrl: 0,
            ien: 0,
            iflag: 0,
            cal: CAL_RESET,
            biasprog: BIASPROG_RESET,
            inputs: [0; CHANNELS as usize],
            vdd_mv: DEFAULT_VDD_MV,
            singledata: 0,
            single_dv: AtomicBool::new(false),
            conversion_pending: false,
            clock: None,
            chain_live: false,
        }
    }

    /// Set VDD, the `REF` = VDD full scale and the `VDD`/`VDDDIV3` level.
    pub fn set_vdd_mv(&mut self, mv: u32) {
        self.vdd_mv = mv.max(1);
    }

    pub fn set_channel_input(&mut self, channel: u8, millivolts: u16) {
        if let Some(slot) = self.inputs.get_mut(usize::from(channel)) {
            *slot = millivolts;
        }
    }

    fn vref_mv(&self) -> Option<u32> {
        match (self.singlectrl >> SINGLECTRL_REF_SHIFT) & SINGLECTRL_REF_MASK {
            REF_1V25 => Some(1250),
            REF_2V5 => Some(2500),
            REF_VDD => Some(self.vdd_mv),
            _ => None,
        }
    }

    /// Millivolts at the selected input, or `None` when `INPUTSEL` names
    /// an input this model does not convert (`TEMP`, `DAC0OUTn`).
    fn selected_millivolts(&self, vref: u32) -> Option<u32> {
        match (self.singlectrl >> SINGLECTRL_INPUTSEL_SHIFT) & SINGLECTRL_INPUTSEL_MASK {
            ch @ 0..=7 => Some(u32::from(self.inputs[ch as usize])),
            INPUTSEL_VDDDIV3 => Some(self.vdd_mv / 3),
            INPUTSEL_VDD => Some(self.vdd_mv),
            INPUTSEL_VSS => Some(0),
            INPUTSEL_VREFDIV2 => Some(vref / 2),
            _ => None,
        }
    }

    fn convert(&self) -> Option<u32> {
        if self.singlectrl & (SINGLECTRL_DIFF | SINGLECTRL_ADJ_LEFT) != 0 {
            return None;
        }
        let vref = self.vref_mv()?;
        let bits = match (self.singlectrl >> SINGLECTRL_RES_SHIFT) & SINGLECTRL_RES_MASK {
            RES_12BIT => 12,
            RES_8BIT => 8,
            RES_6BIT => 6,
            _ => return None,
        };
        let mv = self.selected_millivolts(vref)?;
        let code = (u64::from(mv) << bits) / u64::from(vref);
        Some((code as u32).min((1 << bits) - 1))
    }

    fn land_result(&mut self, code: u32) {
        if self.single_dv.swap(true, Ordering::Relaxed) {
            self.iflag |= IF_SINGLEOF;
        }
        self.singledata = code;
        self.iflag |= IF_SINGLE;
    }

    fn status(&self) -> u32 {
        let mut s = 0;
        if self.conversion_pending {
            s |= STATUS_SINGLEACT;
        }
        if self.single_dv.load(Ordering::Relaxed) {
            s |= STATUS_SINGLEDV;
        }
        s
    }

    /// Non-destructive view of every register.
    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            OFF_CTRL => self.ctrl,
            OFF_STATUS => self.status(),
            OFF_SINGLECTRL => self.singlectrl,
            OFF_SCANCTRL => self.scanctrl,
            OFF_IEN => self.ien,
            OFF_IF => self.iflag,
            OFF_SINGLEDATA | OFF_SINGLEDATAP => self.singledata,
            OFF_SCANDATA | OFF_SCANDATAP => 0,
            OFF_CAL => self.cal,
            OFF_BIASPROG => self.biasprog,
            _ => 0,
        }
    }

    /// The `SINGLEDATA` read side effect: data valid drops.
    fn read_singledata(&self) -> u32 {
        self.single_dv.store(false, Ordering::Relaxed);
        self.singledata
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            OFF_CTRL => self.ctrl = value,
            OFF_CMD => {
                if value & CMD_SINGLESTOP != 0 {
                    self.conversion_pending = false;
                }
                if value & CMD_SINGLESTART != 0 {
                    self.conversion_pending = true;
                }
            }
            OFF_SINGLECTRL => self.singlectrl = value,
            OFF_SCANCTRL => self.scanctrl = value,
            OFF_IEN => self.ien = value,
            // IF is read-only on Series 0; IFS/IFC are the set/clear views.
            OFF_IFS => self.iflag |= value & IF_MASK,
            OFF_IFC => self.iflag &= !value,
            OFF_CAL => self.cal = value,
            OFF_BIASPROG => self.biasprog = value,
            _ => {}
        }
    }

    crate::cycle_clock::scheduler_mode!();

    /// Test/differential knob: detach the clock, pinning the model to the
    /// legacy walk.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
        self.chain_live = false;
    }

    /// One legacy walk tick: settle a pending conversion, then report the
    /// held IRQ level.
    fn service_tick(&mut self) -> PeripheralTickResult {
        let mut result = PeripheralTickResult::default();
        if self.conversion_pending {
            self.conversion_pending = false;
            // `None`: SINGLECTRL names something unmodelled. No result and
            // no flag, so firmware polling SINGLEDV hangs.
            if let Some(code) = self.convert() {
                self.land_result(code);
            }
        }
        if self.ien & self.iflag != 0 {
            result.irq = true;
        }
        result
    }

    fn active(&self) -> bool {
        self.conversion_pending || (self.ien & self.iflag != 0)
    }
}

impl Peripheral for Efm32s0Adc {
    /// CH0..CH7.
    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS)
    }

    /// A byte read of `SINGLEDATA` clears `SINGLEDV` on lane 0 only, so a
    /// byte-wise read of the word still sees the code in every lane.
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let word = if reg == OFF_SINGLEDATA && offset % 4 == 0 {
            self.read_singledata()
        } else {
            self.read_word(reg)
        };
        Ok(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    /// Side-effect-free: never clears `SINGLEDV`.
    fn peek(&self, offset: u64) -> Option<u8> {
        let word = self.read_word(offset & !3);
        Some(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) * 8;
        // CMD/IFS/IFC are command views: merge into 0 so a byte store does
        // not replay the bits it did not write.
        let base = match reg {
            OFF_CMD | OFF_IFS | OFF_IFC => 0,
            r => self.read_word(r),
        };
        let merged = (base & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_word(reg, merged);
        Ok(())
    }

    /// The path a 32-bit CPU load takes. `SINGLEDATA` clears `SINGLEDV`.
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if offset == OFF_SINGLEDATA {
            return Ok(self.read_singledata());
        }
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    /// Same contract as the Series-1 ADC0: the conversion and the held IRQ
    /// level are both armed by an MMIO write and settled by the event chain
    /// on the walk's cycle; nothing accumulates with elapsed cycles.
    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn sync_to(&mut self, _now_cycle: u64) {}

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.scheduler_mode() && self.active() && !self.chain_live {
            self.chain_live = true;
            vec![(0u64, 0u32)]
        } else {
            Vec::new()
        }
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() {
            return crate::sched::EventResult::default();
        }
        let irq = self.service_tick().irq;
        let active = self.active();
        self.chain_live = active;
        crate::sched::EventResult {
            raise_own_irq: irq,
            reschedule_delay: active.then_some(1),
            ..Default::default()
        }
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        self.service_tick()
    }

    fn tick_elapsed_forced(&mut self, _cycles: u64) -> PeripheralTickResult {
        self.service_tick()
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        if channel >= CHANNELS {
            return false;
        }
        self.set_channel_input(channel, millivolts);
        true
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

    fn single(adc: &mut Efm32s0Adc, inputsel: u32, reff: u32, res: u32) {
        adc.write_word(
            OFF_SINGLECTRL,
            (inputsel << SINGLECTRL_INPUTSEL_SHIFT)
                | (reff << SINGLECTRL_REF_SHIFT)
                | (res << SINGLECTRL_RES_SHIFT),
        );
    }

    /// `ADC_Start(adcStartSingle)`, one tick, then `SINGLEDATA` if
    /// `STATUS.SINGLEDV` rose.
    fn convert(adc: &mut Efm32s0Adc) -> Option<u32> {
        adc.write_word(OFF_CMD, CMD_SINGLESTART);
        adc.tick_elapsed(1);
        (adc.read_word(OFF_STATUS) & STATUS_SINGLEDV != 0)
            .then(|| adc.read_u32(OFF_SINGLEDATA).unwrap())
    }

    #[test]
    fn the_register_map_matches_adc_typedef() {
        // 14 words CTRL..CAL, RESERVED0[1], then BIASPROG.
        assert_eq!(OFF_CAL, 13 * 4);
        assert_eq!(OFF_BIASPROG, 15 * 4);
        assert_eq!(OFF_SINGLEDATAP, 11 * 4);
    }

    #[test]
    fn registers_reset_to_the_header_values() {
        let adc = Efm32s0Adc::new();
        assert_eq!(adc.read_word(OFF_CTRL), 0x001F_0000);
        assert_eq!(adc.read_word(OFF_SINGLECTRL), 0);
        assert_eq!(adc.read_word(OFF_CAL), 0x3F00_3F00);
        assert_eq!(adc.read_word(OFF_BIASPROG), 0x0000_0747);
        assert_eq!(adc.read_word(OFF_STATUS), 0);
        assert_eq!(adc.read_word(OFF_IF), 0);
    }

    #[test]
    fn inputsel_picks_the_channel() {
        let mut adc = Efm32s0Adc::new();
        adc.set_channel_input(6, 1250);
        adc.set_channel_input(7, 600);
        single(&mut adc, 6, REF_2V5, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(2048));
        single(&mut adc, 7, REF_2V5, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(600 * 4096 / 2500));
        // An undriven channel reads 0.
        single(&mut adc, 5, REF_2V5, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(0));
        // There is no CH8: the bus cannot drive it.
        assert!(!adc.set_adc_channel_input(8, 1000));
    }

    #[test]
    fn ref_and_res_set_the_code() {
        let mut adc = Efm32s0Adc::new();
        adc.set_channel_input(7, 1000);
        single(&mut adc, 7, REF_1V25, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(1000 * 4096 / 1250));
        single(&mut adc, 7, REF_VDD, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(1000 * 4096 / 3300));
        single(&mut adc, 7, REF_VDD, RES_8BIT);
        assert_eq!(convert(&mut adc), Some(1000 * 256 / 3300));
        single(&mut adc, 7, REF_VDD, RES_6BIT);
        assert_eq!(convert(&mut adc), Some(1000 * 64 / 3300));
        adc.set_channel_input(7, 2000);
        single(&mut adc, 7, REF_1V25, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(4095));
    }

    #[test]
    fn internal_inputs_follow_vdd_and_vref() {
        let mut adc = Efm32s0Adc::new();
        single(&mut adc, INPUTSEL_VDD, REF_VDD, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(4095));
        single(&mut adc, INPUTSEL_VSS, REF_VDD, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(0));
        single(&mut adc, INPUTSEL_VDDDIV3, REF_2V5, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(1100 * 4096 / 2500));
        single(&mut adc, INPUTSEL_VREFDIV2, REF_2V5, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(2048));
        adc.set_vdd_mv(1800);
        single(&mut adc, INPUTSEL_VDDDIV3, REF_1V25, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(600 * 4096 / 1250));
    }

    #[test]
    fn unmodelled_settings_produce_no_result() {
        let mut adc = Efm32s0Adc::new();
        for (extra, inputsel, reff, res) in [
            (SINGLECTRL_DIFF, INPUTSEL_VDD, REF_VDD, RES_12BIT),
            (SINGLECTRL_ADJ_LEFT, INPUTSEL_VDD, REF_VDD, RES_12BIT),
            (0, INPUTSEL_VDD, 3, RES_12BIT),
            (0, INPUTSEL_VDD, REF_VDD, 3),
            // TEMP and DAC0OUT0/1.
            (0, 8, REF_VDD, RES_12BIT),
            (0, 13, REF_VDD, RES_12BIT),
            (0, 14, REF_VDD, RES_12BIT),
        ] {
            single(&mut adc, inputsel, reff, res);
            let v = adc.read_word(OFF_SINGLECTRL) | extra;
            adc.write_word(OFF_SINGLECTRL, v);
            assert_eq!(convert(&mut adc), None, "SINGLECTRL {v:#x}");
            assert_eq!(adc.read_word(OFF_IF), 0, "SINGLECTRL {v:#x}");
        }
    }

    #[test]
    fn singledata_clears_singledv_and_singledatap_does_not() {
        let mut adc = Efm32s0Adc::new();
        single(&mut adc, INPUTSEL_VDD, REF_VDD, RES_8BIT);
        adc.write_word(OFF_CMD, CMD_SINGLESTART);
        assert_ne!(adc.read_word(OFF_STATUS) & STATUS_SINGLEACT, 0);
        adc.tick_elapsed(1);
        assert_eq!(adc.read_word(OFF_STATUS), STATUS_SINGLEDV);
        assert_eq!(adc.read_u32(OFF_SINGLEDATAP).unwrap(), 255);
        assert_eq!(adc.read_word(OFF_STATUS), STATUS_SINGLEDV);
        assert_eq!(adc.read_u32(OFF_SINGLEDATA).unwrap(), 255);
        assert_eq!(adc.read_word(OFF_STATUS), 0);
        // The data register keeps the code after the read.
        assert_eq!(adc.read_u32(OFF_SINGLEDATAP).unwrap(), 255);
        // IF.SINGLE is not cleared by the read; IFC clears it.
        assert_ne!(adc.read_word(OFF_IF) & IF_SINGLE, 0);
    }

    #[test]
    fn a_byte_read_clears_singledv_once() {
        let mut adc = Efm32s0Adc::new();
        adc.set_channel_input(7, 3000);
        single(&mut adc, 7, REF_VDD, RES_12BIT);
        adc.write_word(OFF_CMD, CMD_SINGLESTART);
        adc.tick_elapsed(1);
        let code = 3000 * 4096 / 3300;
        let lo = adc.read(OFF_SINGLEDATA).unwrap();
        let hi = adc.read(OFF_SINGLEDATA + 1).unwrap();
        assert_eq!(u32::from(lo) | (u32::from(hi) << 8), code);
        assert_eq!(adc.read_word(OFF_STATUS) & STATUS_SINGLEDV, 0);
        // A peek never clears it.
        adc.write_word(OFF_CMD, CMD_SINGLESTART);
        adc.tick_elapsed(1);
        assert!(adc.peek(OFF_SINGLEDATA).is_some());
        assert_ne!(adc.read_word(OFF_STATUS) & STATUS_SINGLEDV, 0);
    }

    #[test]
    fn an_unread_result_overflows() {
        let mut adc = Efm32s0Adc::new();
        adc.set_channel_input(6, 100);
        single(&mut adc, 6, REF_VDD, RES_12BIT);
        convert(&mut adc).unwrap();
        assert_eq!(adc.read_word(OFF_IF) & IF_SINGLEOF, 0, "read in time");
        adc.write_word(OFF_CMD, CMD_SINGLESTART);
        adc.tick_elapsed(1);
        adc.set_channel_input(6, 200);
        adc.write_word(OFF_CMD, CMD_SINGLESTART);
        adc.tick_elapsed(1);
        assert_ne!(adc.read_word(OFF_IF) & IF_SINGLEOF, 0);
        assert_eq!(adc.read_u32(OFF_SINGLEDATA).unwrap(), 200 * 4096 / 3300);
    }

    #[test]
    fn if_is_read_only_ifs_sets_ifc_clears_and_ien_raises_the_irq() {
        let mut adc = Efm32s0Adc::new();
        adc.write_word(OFF_IF, IF_SINGLE);
        assert_eq!(adc.read_word(OFF_IF), 0);
        single(&mut adc, INPUTSEL_VSS, REF_VDD, RES_12BIT);
        adc.write_word(OFF_CMD, CMD_SINGLESTART);
        assert!(!adc.tick_elapsed(1).irq);
        assert_ne!(adc.read_word(OFF_IF) & IF_SINGLE, 0);
        adc.write_word(OFF_IEN, IF_SINGLE);
        assert!(adc.tick_elapsed(1).irq);
        adc.write_word(OFF_IFC, IF_SINGLE);
        assert!(!adc.tick_elapsed(1).irq);
        adc.write_word(OFF_IFS, 0xFFFF_FFFF);
        assert_eq!(adc.read_word(OFF_IF), IF_MASK);
    }

    #[test]
    fn singlestop_cancels_and_scanstart_converts_nothing() {
        let mut adc = Efm32s0Adc::new();
        single(&mut adc, INPUTSEL_VDD, REF_VDD, RES_12BIT);
        adc.write_word(OFF_CMD, CMD_SINGLESTART);
        adc.write_word(OFF_CMD, CMD_SINGLESTOP);
        adc.tick_elapsed(1);
        assert_eq!(adc.read_word(OFF_STATUS), 0);
        adc.write_word(OFF_SCANCTRL, 0xFF << 8);
        adc.write_word(OFF_CMD, 1 << 2);
        adc.tick_elapsed(1);
        assert_eq!(adc.read_word(OFF_STATUS), 0);
        assert_eq!(adc.read_word(OFF_IF), 0);
    }
}
