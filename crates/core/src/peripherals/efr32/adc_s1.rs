// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Silicon Labs EFR32 Series-1 **ADC0** — the 12-bit SAR ADC behind the
//! Analog Port (APORT) on EFR32xG1, xG12 and xG13.
//!
//! # Sources
//!
//! * Register offsets are walked from `ADC_TypeDef` in the vendor CMSIS
//!   header `efr32mg12p_adc.h` (gecko_sdk tag `gsdk_4.4`). Field positions,
//!   enumerations and reset values are the `_ADC_<REG>_<FIELD>_*` defines in
//!   the same header. `efr32mg1p_adc.h` and `efr32mg13p_adc.h` declare the
//!   same struct and the same CMD/STATUS/IF/SINGLECTRL/SINGLEFIFO fields and
//!   enumerations. Only the STATUS/IF masks differ: xG1 has no SCANPENDING
//!   and none of the IF bits above PROGERR. Nothing here is recalled.
//! * ADC0 sits at `ADC0_BASE` = 0x4000_2000 on all three dies. `ADC0_IRQn`
//!   is 14 on EFR32MG1P and 15 on EFR32MG12P and EFR32MG13P.
//! * The APORT bus/channel to pin matrix is EFR32MG12 data sheet
//!   Table 6.12, "ADC0 Bus and Pin Mapping" (§6.6 APORT Client Maps). The
//!   pin is selected by the `POSSEL` enumeration
//!   `APORT<bus><X|Y>CH<n>` = `bus * 0x20 + n`; X/Y follows the channel's
//!   parity, so each 0x20..0x9F value names exactly one bus half:
//!
//!   | bus      | CH0..CH7 | CH8..CH11 | CH12..CH15 | CH16..CH21 | CH22..CH31  |
//!   |----------|----------|-----------|------------|------------|-------------|
//!   | APORT1/2 | PC0..PC7 | PC8..PC11 | —          | PF0..PF5   | PF6..PF15   |
//!   | APORT3/4 | PD8..PD15| PA0..PA3  | PA4..PA7   | —          | PB6..PB15   |
//!
//!   The vendor's own Series-1 example `peripheral_examples_gsdk`
//!   `series1/adc/adc_single_polled` names PC9 = `APORT2XCH9` on EFR32MG1P,
//!   EFR32MG12P and EFR32MG13P alike, and PA3 = `APORT4XCH11` on xG14. Both
//!   agree with the table.
//! * The firmware sequence is emlib `em_adc.c`/`em_adc.h` (gsdk_4.4):
//!   `ADC_InitSingle` writes `SINGLECTRL`/`SINGLECTRLX`, `ADC_Start` writes
//!   `CMD.SINGLESTART`, then firmware polls `STATUS.SINGLEDV` (the vendor
//!   example) or `IF.SINGLE` (Moddable `geckoAnalog.c`) and reads
//!   `SINGLEDATA`. `ADC_INITSINGLE_DEFAULT` is 1.25 V, 12 bits, right
//!   adjusted, discard on a full FIFO. The example converts with
//!   `mV = sample * 2500 / 4096`, so the code is `mV * 4096 / Vref`.
//!
//! # What is modelled
//!
//! * `SINGLECTRL.POSSEL` selects a real pad through the APORT matrix above,
//!   so routing a source to the wrong pin, or naming a bus half that pin is
//!   not on, reads what is really there. A `POSSEL` that names no pad
//!   (`APORT1YCH13`, PB0..PB5, the APORT0 PI pins) produces no result at all.
//! * `POSSEL` = `AVDD` converts the supply, `VSS` converts 0.
//! * `SINGLECTRL.REF` = 1V25 / 2V5 / VDD sets full scale. `RES` = 12, 8 or 6
//!   bits. A code is `round-down(mV * 2^bits / Vref)`, saturating at
//!   all-ones.
//! * The single FIFO is four deep (`SINGLECTRLX.DVL` counts up to four
//!   entries; `SINGLEFIFOCOUNT` is three bits). `SINGLEDATA` pops it and
//!   `SINGLEDATAP` peeks. `SINGLEFIFOCOUNT` reports the depth, and
//!   `SINGLEFIFOCLEAR` empties it. When the FIFO is full a new result is
//!   discarded (`FIFOOFACT` = DISCARD) or overwrites the oldest (OVERWRITE),
//!   and `IF.SINGLEOF` latches either way.
//! * `STATUS.SINGLEDV` and `IF.SINGLE` rise once the FIFO holds `DVL + 1`
//!   results. `IF` is read-only; `IFS` sets and `IFC` clears; `IEN & IF`
//!   holds the IRQ line.
//!
//! # Idealised — present, but not physical
//!
//! * **Conversion is instantaneous**: the result lands on the tick after
//!   `SINGLESTART`. No warm-up, prescaler or acquisition time.
//! * **No clock gate.** On silicon `CMU_HFPERCLKEN0.ADC0` gates the block,
//!   but the onboarding CMU is a stub, so there is nothing to gate on.
//! * **Single-ended, right-adjusted, one-shot only.** `DIFF`, `ADJ` = LEFT,
//!   `RES` = OVS and the `REF` values other than 1V25/2V5/VDD are stored,
//!   and a conversion under them produces NO result. A wrong-but-plausible
//!   code is worse than a loud hang, and no source here pins those transfer
//!   functions down. `REP` is stored and ignored.
//! * **No scan sequence.** `SCANCTRL`/`SCANMASK`/`SCANINPUTSEL` store, and
//!   `SCANSTART` converts nothing.
//! * **No calibration.** `CAL` holds its reset value and whatever firmware
//!   writes; the code is not offset or gain-corrected by it.
//! * Reading `SINGLEDATA` does not clear `IF.SINGLE`. No source in hand says
//!   it does, and the Zephyr driver clears it through `IFC` itself.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

// ── Register offsets, walked from `ADC_TypeDef` ────────────────────────────
const OFF_CTRL: u64 = 0x00;
const OFF_CMD: u64 = 0x08;
const OFF_STATUS: u64 = 0x0C;
const OFF_SINGLECTRL: u64 = 0x10;
const OFF_SINGLECTRLX: u64 = 0x14;
const OFF_SCANCTRL: u64 = 0x18;
const OFF_SCANCTRLX: u64 = 0x1C;
const OFF_SCANMASK: u64 = 0x20;
const OFF_SCANINPUTSEL: u64 = 0x24;
const OFF_SCANNEGSEL: u64 = 0x28;
const OFF_CMPTHR: u64 = 0x2C;
const OFF_BIASPROG: u64 = 0x30;
const OFF_CAL: u64 = 0x34;
const OFF_IF: u64 = 0x38;
const OFF_IFS: u64 = 0x3C;
const OFF_IFC: u64 = 0x40;
const OFF_IEN: u64 = 0x44;
const OFF_SINGLEDATA: u64 = 0x48;
const OFF_SINGLEDATAP: u64 = 0x50;
const OFF_SINGLEFIFOCOUNT: u64 = 0x84;
const OFF_SINGLEFIFOCLEAR: u64 = 0x8C;
/// Last word of `ADC_TypeDef`; the map test checks the walk ends here.
#[cfg(test)]
const OFF_APORTMASTERDIS: u64 = 0x94;

// ── Reset values (`_ADC_<REG>_RESETVALUE`) ─────────────────────────────────
const CTRL_RESET: u32 = 0x001F_0000;
/// POSSEL = NEGSEL = VSS.
const SINGLECTRL_RESET: u32 = 0x00FF_FF00;
const SCANNEGSEL_RESET: u32 = 0x0000_39E4;
const CAL_RESET: u32 = 0x4078_4078;

// ── Field positions ────────────────────────────────────────────────────────
const CMD_SINGLESTART: u32 = 1 << 0;
const CMD_SINGLESTOP: u32 = 1 << 1;
const STATUS_SINGLEACT: u32 = 1 << 0;
const STATUS_SINGLEDV: u32 = 1 << 16;
const IF_SINGLE: u32 = 1 << 0;
const IF_SINGLEOF: u32 = 1 << 8;
/// `_ADC_IF_MASK` on xG12/xG13, the widest of the three dies.
const IF_MASK: u32 = 0x3F03_0F03;
const SINGLEFIFOCLEAR_BIT: u32 = 1 << 0;

const SINGLECTRL_DIFF: u32 = 1 << 1;
const SINGLECTRL_ADJ_LEFT: u32 = 1 << 2;
const SINGLECTRL_RES_SHIFT: u32 = 3;
const SINGLECTRL_RES_MASK: u32 = 0x3;
const SINGLECTRL_REF_SHIFT: u32 = 5;
const SINGLECTRL_REF_MASK: u32 = 0x7;
const SINGLECTRL_POSSEL_SHIFT: u32 = 8;
const SINGLECTRL_POSSEL_MASK: u32 = 0xFF;

const SINGLECTRLX_DVL_SHIFT: u32 = 12;
const SINGLECTRLX_DVL_MASK: u32 = 0x3;
const SINGLECTRLX_FIFOOFACT: u32 = 1 << 14;

/// `_ADC_SINGLECTRL_RES_*`.
const RES_12BIT: u32 = 0;
const RES_8BIT: u32 = 1;
const RES_6BIT: u32 = 2;
/// `_ADC_SINGLECTRL_REF_*`.
const REF_1V25: u32 = 0;
const REF_2V5: u32 = 1;
const REF_VDD: u32 = 2;
/// `_ADC_SINGLECTRL_POSSEL_*` internal inputs.
const POSSEL_AVDD: u32 = 0xE0;
const POSSEL_VSS: u32 = 0xFF;

const SINGLE_FIFO_DEPTH: usize = 4;

/// AVDD on the Wireless Starter Kit, in millivolts.
const DEFAULT_VDD_MV: u32 = 3300;

/// GPIO port index, A..F = 0..5, as [`channel_for`] uses it.
const PORT_A: u8 = 0;
const PORT_B: u8 = 1;
const PORT_C: u8 = 2;
const PORT_D: u8 = 3;
const PORT_F: u8 = 5;

/// Analog channel index for a `(port, pin)` pair: the key
/// [`Peripheral::set_adc_channel_input`] and `adc.<id>.<ch>_volts` address
/// a pad by. Port A..F is 0..5, so channel 5 is PA5 and 0x29 is PC9. Same
/// indexing as the Series-2 IADC; the APORT has no channel number of its
/// own that is unique to a pad.
pub const fn channel_for(port: u8, pin: u8) -> u8 {
    (port << 4) | (pin & 0xF)
}

/// The pad an `APORT<bus><X|Y>CH<n>` POSSEL value reaches, per EFR32MG12
/// data sheet Table 6.12. `None` when that bus channel has no pin.
pub const fn aport_pad(possel: u8) -> Option<(u8, u8)> {
    let bus = possel >> 5;
    let ch = possel & 0x1F;
    match bus {
        1 | 2 => match ch {
            0..=11 => Some((PORT_C, ch)),
            16..=31 => Some((PORT_F, ch - 16)),
            _ => None,
        },
        3 | 4 => match ch {
            0..=7 => Some((PORT_D, ch + 8)),
            8..=15 => Some((PORT_A, ch - 8)),
            22..=31 => Some((PORT_B, ch - 16)),
            _ => None,
        },
        _ => None,
    }
}

/// The EFR32 Series-1 ADC0.
#[derive(Debug)]
pub struct Efr32s1Adc {
    ctrl: u32,
    singlectrl: u32,
    singlectrlx: u32,
    scanctrl: u32,
    scanctrlx: u32,
    scanmask: u32,
    scaninputsel: u32,
    scannegsel: u32,
    cmpthr: u32,
    biasprog: u32,
    cal: u32,
    iflag: u32,
    ien: u32,

    /// Level on each pad in millivolts, keyed by [`channel_for`].
    inputs: std::collections::HashMap<u8, u16>,
    vdd_mv: u32,

    /// Behind a `Mutex` because a `SINGLEDATA` read pops and the read path
    /// is `&self` (same shape as the Series-2 IADC).
    fifo: std::sync::Mutex<std::collections::VecDeque<u32>>,
    conversion_pending: bool,
    /// Word popped by the last byte-lane-0 read of `SINGLEDATA`, so a
    /// byte-wise read pops once.
    byte_lane_cache: std::sync::atomic::AtomicU32,

    clock: Option<CycleClock>,
    chain_live: bool,
}

impl Default for Efr32s1Adc {
    fn default() -> Self {
        Self::new()
    }
}

impl Efr32s1Adc {
    pub fn new() -> Self {
        Self {
            ctrl: CTRL_RESET,
            singlectrl: SINGLECTRL_RESET,
            singlectrlx: 0,
            scanctrl: 0,
            scanctrlx: 0,
            scanmask: 0,
            scaninputsel: 0,
            scannegsel: SCANNEGSEL_RESET,
            cmpthr: 0,
            biasprog: 0,
            cal: CAL_RESET,
            iflag: 0,
            ien: 0,
            inputs: std::collections::HashMap::new(),
            vdd_mv: DEFAULT_VDD_MV,
            fifo: std::sync::Mutex::new(std::collections::VecDeque::new()),
            conversion_pending: false,
            byte_lane_cache: std::sync::atomic::AtomicU32::new(0),
            clock: None,
            chain_live: false,
        }
    }

    /// Set AVDD, the `REF` = VDD full scale and the `POSSEL` = AVDD level.
    pub fn set_vdd_mv(&mut self, mv: u32) {
        self.vdd_mv = mv.max(1);
    }

    pub fn set_channel_input(&mut self, channel: u8, millivolts: u16) {
        self.inputs.insert(channel, millivolts);
    }

    /// Millivolts at the selected input, or `None` when `POSSEL` names
    /// nothing this model converts.
    fn selected_millivolts(&self) -> Option<u32> {
        let possel = (self.singlectrl >> SINGLECTRL_POSSEL_SHIFT) & SINGLECTRL_POSSEL_MASK;
        match possel {
            POSSEL_VSS => Some(0),
            POSSEL_AVDD => Some(self.vdd_mv),
            p => aport_pad(p as u8).map(|(port, pin)| {
                u32::from(*self.inputs.get(&channel_for(port, pin)).unwrap_or(&0))
            }),
        }
    }

    /// Full scale and width from `SINGLECTRL`, or `None` for a setting with
    /// no modelled transfer function (see the module header).
    fn transfer(&self) -> Option<(u32, u32)> {
        if self.singlectrl & (SINGLECTRL_DIFF | SINGLECTRL_ADJ_LEFT) != 0 {
            return None;
        }
        let vref = match (self.singlectrl >> SINGLECTRL_REF_SHIFT) & SINGLECTRL_REF_MASK {
            REF_1V25 => 1250,
            REF_2V5 => 2500,
            REF_VDD => self.vdd_mv,
            _ => return None,
        };
        let bits = match (self.singlectrl >> SINGLECTRL_RES_SHIFT) & SINGLECTRL_RES_MASK {
            RES_12BIT => 12,
            RES_8BIT => 8,
            RES_6BIT => 6,
            _ => return None,
        };
        Some((vref, bits))
    }

    fn convert(&self) -> Option<u32> {
        let mv = self.selected_millivolts()?;
        let (vref, bits) = self.transfer()?;
        let code = (u64::from(mv) << bits) / u64::from(vref);
        Some((code as u32).min((1 << bits) - 1))
    }

    fn dvl_entries(&self) -> usize {
        (((self.singlectrlx >> SINGLECTRLX_DVL_SHIFT) & SINGLECTRLX_DVL_MASK) + 1) as usize
    }

    fn fifo_len(&self) -> usize {
        self.fifo.lock().map(|f| f.len()).unwrap_or(0)
    }

    fn push_result(&mut self, code: u32) {
        let overwrite = self.singlectrlx & SINGLECTRLX_FIFOOFACT != 0;
        let len = if let Ok(mut f) = self.fifo.lock() {
            if f.len() >= SINGLE_FIFO_DEPTH {
                self.iflag |= IF_SINGLEOF;
                if overwrite {
                    f.pop_front();
                    f.push_back(code);
                }
            } else {
                f.push_back(code);
            }
            f.len()
        } else {
            0
        };
        if len >= self.dvl_entries() {
            self.iflag |= IF_SINGLE;
        }
    }

    fn pop_result(&self) -> u32 {
        self.fifo
            .lock()
            .ok()
            .and_then(|mut f| f.pop_front())
            .unwrap_or(0)
    }

    fn front_result(&self) -> u32 {
        self.fifo
            .lock()
            .ok()
            .and_then(|f| f.front().copied())
            .unwrap_or(0)
    }

    fn status(&self) -> u32 {
        let mut s = 0;
        if self.conversion_pending {
            s |= STATUS_SINGLEACT;
        }
        if self.fifo_len() >= self.dvl_entries() {
            s |= STATUS_SINGLEDV;
        }
        s
    }

    /// Non-destructive view of every register: `SINGLEDATA` reads the front
    /// without popping here, because `peek` and byte RMW also come through.
    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            OFF_CTRL => self.ctrl,
            OFF_CMD => 0,
            OFF_STATUS => self.status(),
            OFF_SINGLECTRL => self.singlectrl,
            OFF_SINGLECTRLX => self.singlectrlx,
            OFF_SCANCTRL => self.scanctrl,
            OFF_SCANCTRLX => self.scanctrlx,
            OFF_SCANMASK => self.scanmask,
            OFF_SCANINPUTSEL => self.scaninputsel,
            OFF_SCANNEGSEL => self.scannegsel,
            OFF_CMPTHR => self.cmpthr,
            OFF_BIASPROG => self.biasprog,
            OFF_CAL => self.cal,
            OFF_IF => self.iflag,
            OFF_IEN => self.ien,
            OFF_SINGLEDATA | OFF_SINGLEDATAP => self.front_result(),
            OFF_SINGLEFIFOCOUNT => self.fifo_len() as u32,
            _ => 0,
        }
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
            OFF_SINGLECTRLX => self.singlectrlx = value,
            OFF_SCANCTRL => self.scanctrl = value,
            OFF_SCANCTRLX => self.scanctrlx = value,
            OFF_SCANMASK => self.scanmask = value,
            OFF_SCANINPUTSEL => self.scaninputsel = value,
            OFF_SCANNEGSEL => self.scannegsel = value,
            OFF_CMPTHR => self.cmpthr = value,
            OFF_BIASPROG => self.biasprog = value,
            OFF_CAL => self.cal = value,
            // IF is read-only on Series 1; IFS/IFC are the set/clear views.
            OFF_IFS => self.iflag |= value & IF_MASK,
            OFF_IFC => self.iflag &= !value,
            OFF_IEN => self.ien = value,
            OFF_SINGLEFIFOCLEAR if value & SINGLEFIFOCLEAR_BIT != 0 => {
                if let Ok(mut f) = self.fifo.lock() {
                    f.clear();
                }
            }
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
    /// held IRQ level. Shared by the walk, the forced walk and the event
    /// chain.
    fn service_tick(&mut self) -> PeripheralTickResult {
        let mut result = PeripheralTickResult::default();
        if self.conversion_pending {
            self.conversion_pending = false;
            // `None`: POSSEL or SINGLECTRL names something unmodelled. No
            // result and no flag, so firmware polling SINGLEDV hangs.
            if let Some(code) = self.convert() {
                self.push_result(code);
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

impl Peripheral for Efr32s1Adc {
    /// Every [`channel_for`] index: ports A..F times sixteen pins.
    fn adc_channel_count(&self) -> Option<u8> {
        Some(96)
    }

    /// A byte read of `SINGLEDATA` pops once, on lane 0.
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        if reg == OFF_SINGLEDATA {
            let lane = offset % 4;
            let word = if lane == 0 {
                let popped = self.pop_result();
                self.byte_lane_cache
                    .store(popped, std::sync::atomic::Ordering::Relaxed);
                popped
            } else {
                self.byte_lane_cache
                    .load(std::sync::atomic::Ordering::Relaxed)
            };
            return Ok(((word >> (lane * 8)) & 0xFF) as u8);
        }
        let word = self.read_word(reg);
        Ok(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    /// Side-effect-free: never pops.
    fn peek(&self, offset: u64) -> Option<u8> {
        let word = self.read_word(offset & !3);
        Some(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) * 8;
        // CMD/IFS/IFC/SINGLEFIFOCLEAR are command views: merge into 0 so a
        // byte store does not replay the bits it did not write.
        let base = match reg {
            OFF_CMD | OFF_IFS | OFF_IFC | OFF_SINGLEFIFOCLEAR => 0,
            r => self.read_word(r),
        };
        let merged = (base & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_word(reg, merged);
        Ok(())
    }

    /// The path a 32-bit CPU load takes. `SINGLEDATA` pops here.
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if offset == OFF_SINGLEDATA {
            return Ok(self.pop_result());
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

    /// Same contract as the Series-2 IADC: the conversion and the held IRQ
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

    /// `APORT<bus>..CH<n>` POSSEL encoding.
    const fn aport(bus: u32, ch: u32) -> u32 {
        bus * 0x20 + ch
    }

    fn single(adc: &mut Efr32s1Adc, possel: u32, reff: u32, res: u32) {
        adc.write_word(
            OFF_SINGLECTRL,
            (0xFF << 16)
                | (possel << SINGLECTRL_POSSEL_SHIFT)
                | (reff << SINGLECTRL_REF_SHIFT)
                | (res << SINGLECTRL_RES_SHIFT),
        );
    }

    /// `ADC_Start(adcStartSingle)`, one tick, then `SINGLEDATA` if
    /// `STATUS.SINGLEDV` rose.
    fn convert(adc: &mut Efr32s1Adc) -> Option<u32> {
        adc.write_word(OFF_CMD, CMD_SINGLESTART);
        adc.tick_elapsed(1);
        (adc.read_word(OFF_STATUS) & STATUS_SINGLEDV != 0)
            .then(|| adc.read_u32(OFF_SINGLEDATA).unwrap())
    }

    #[test]
    fn the_register_map_matches_adc_typedef() {
        // 22 words CTRL..SCANDATAP, 4 reserved, SCANDATAX(P), 3 reserved,
        // then APORTREQ..APORTMASTERDIS (7 words).
        assert_eq!(OFF_SINGLEDATAP, 20 * 4);
        assert_eq!(OFF_SINGLEFIFOCOUNT, (22 + 4 + 2 + 3 + 2) * 4);
        assert_eq!(OFF_APORTMASTERDIS, (22 + 4 + 2 + 3 + 6) * 4);
    }

    #[test]
    fn registers_reset_to_the_header_values() {
        let adc = Efr32s1Adc::new();
        assert_eq!(adc.read_word(OFF_CTRL), 0x001F_0000);
        assert_eq!(adc.read_word(OFF_SINGLECTRL), 0x00FF_FF00);
        assert_eq!(adc.read_word(OFF_SCANNEGSEL), 0x0000_39E4);
        assert_eq!(adc.read_word(OFF_CAL), 0x4078_4078);
        assert_eq!(adc.read_word(OFF_STATUS), 0);
    }

    #[test]
    fn the_aport_matrix_follows_table_6_12() {
        // Vendor example: PC9 = APORT2XCH9. Moddable: PA0 = APORT3XCH8,
        // PB7 = APORT3YCH23. xG14 example: PA3 = APORT4XCH11.
        assert_eq!(aport_pad(0x49), Some((PORT_C, 9)));
        assert_eq!(aport_pad(0x68), Some((PORT_A, 0)));
        assert_eq!(aport_pad(0x77), Some((PORT_B, 7)));
        assert_eq!(aport_pad(0x8B), Some((PORT_A, 3)));
        // Both halves of a pin: PC9 is also on APORT1Y, PA0 on APORT4Y.
        assert_eq!(aport_pad(0x29), Some((PORT_C, 9)));
        assert_eq!(aport_pad(0x88), Some((PORT_A, 0)));
        // Table edges: PD8 on CH0, PF15 on CH31, PB6 on CH22.
        assert_eq!(aport_pad(0x60), Some((PORT_D, 8)));
        assert_eq!(aport_pad(0x3F), Some((PORT_F, 15)));
        assert_eq!(aport_pad(0x96), Some((PORT_B, 6)));
        // Empty cells: APORT1/2 CH12..15, APORT3/4 CH16..21 (PB0..PB5),
        // and APORT0, whose PI pins this model has no port for.
        for none in [0x2C, 0x4F, 0x70, 0x95, 0x00, 0x1F] {
            assert_eq!(aport_pad(none), None, "POSSEL {none:#x}");
        }
    }

    #[test]
    fn a_conversion_reads_the_pad_possel_reaches() {
        let mut adc = Efr32s1Adc::new();
        adc.set_channel_input(channel_for(PORT_C, 9), 1250);
        adc.set_channel_input(channel_for(PORT_A, 5), 600);
        single(&mut adc, aport(2, 9), REF_2V5, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(2048));
        // PA5 = CH13 on APORT3Y and APORT4X.
        single(&mut adc, aport(3, 13), REF_2V5, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(600 * 4096 / 2500));
        single(&mut adc, aport(4, 13), REF_2V5, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(600 * 4096 / 2500));
        // CH13 on APORT1Y has no pin: no result at all.
        single(&mut adc, aport(1, 13), REF_2V5, RES_12BIT);
        assert_eq!(convert(&mut adc), None);
        assert_eq!(adc.read_word(OFF_SINGLEFIFOCOUNT), 0);
    }

    #[test]
    fn ref_and_res_set_the_code() {
        let mut adc = Efr32s1Adc::new();
        let pa6 = channel_for(PORT_A, 6);
        adc.set_channel_input(pa6, 1000);
        let possel = aport(3, 14);
        single(&mut adc, possel, REF_1V25, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(1000 * 4096 / 1250));
        single(&mut adc, possel, REF_VDD, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(1000 * 4096 / 3300));
        single(&mut adc, possel, REF_VDD, RES_8BIT);
        assert_eq!(convert(&mut adc), Some(1000 * 256 / 3300));
        single(&mut adc, possel, REF_VDD, RES_6BIT);
        assert_eq!(convert(&mut adc), Some(1000 * 64 / 3300));
        // Above full scale saturates.
        adc.set_channel_input(pa6, 2000);
        single(&mut adc, possel, REF_1V25, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(4095));
    }

    #[test]
    fn avdd_and_vss_are_real_inputs() {
        let mut adc = Efr32s1Adc::new();
        single(&mut adc, POSSEL_AVDD, REF_VDD, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(4095));
        single(&mut adc, POSSEL_VSS, REF_VDD, RES_12BIT);
        assert_eq!(convert(&mut adc), Some(0));
    }

    #[test]
    fn unmodelled_transfer_functions_produce_no_result() {
        let mut adc = Efr32s1Adc::new();
        for (extra, reff, res) in [
            (SINGLECTRL_DIFF, REF_VDD, RES_12BIT),
            (SINGLECTRL_ADJ_LEFT, REF_VDD, RES_12BIT),
            (0, 3, RES_12BIT),
            (0, REF_VDD, 3),
        ] {
            single(&mut adc, POSSEL_AVDD, reff, res);
            let v = adc.read_word(OFF_SINGLECTRL) | extra;
            adc.write_word(OFF_SINGLECTRL, v);
            assert_eq!(convert(&mut adc), None, "SINGLECTRL {v:#x}");
            assert_eq!(adc.read_word(OFF_IF) & IF_SINGLE, 0);
        }
    }

    #[test]
    fn singledata_pops_and_singledatap_peeks() {
        let mut adc = Efr32s1Adc::new();
        single(&mut adc, POSSEL_AVDD, REF_VDD, RES_8BIT);
        adc.write_word(OFF_CMD, CMD_SINGLESTART);
        adc.tick_elapsed(1);
        assert_eq!(adc.read_word(OFF_SINGLEFIFOCOUNT), 1);
        assert_eq!(adc.read_u32(OFF_SINGLEDATAP).unwrap(), 255);
        assert_eq!(adc.read_word(OFF_SINGLEFIFOCOUNT), 1);
        assert_eq!(adc.read_u32(OFF_SINGLEDATA).unwrap(), 255);
        assert_eq!(adc.read_word(OFF_SINGLEFIFOCOUNT), 0);
        assert_eq!(adc.read_word(OFF_STATUS) & STATUS_SINGLEDV, 0);
    }

    #[test]
    fn dvl_holds_singledv_until_enough_results() {
        let mut adc = Efr32s1Adc::new();
        single(&mut adc, POSSEL_AVDD, REF_VDD, RES_12BIT);
        adc.write_word(OFF_SINGLECTRLX, 2 << SINGLECTRLX_DVL_SHIFT);
        for n in 1..=3 {
            adc.write_word(OFF_CMD, CMD_SINGLESTART);
            adc.tick_elapsed(1);
            let dv = adc.read_word(OFF_STATUS) & STATUS_SINGLEDV != 0;
            let flag = adc.read_word(OFF_IF) & IF_SINGLE != 0;
            assert_eq!((dv, flag), (n == 3, n == 3), "after {n} results");
        }
    }

    #[test]
    fn a_full_fifo_discards_or_overwrites_and_flags_overflow() {
        for (overwrite, oldest) in [(false, 1), (true, 2)] {
            let mut adc = Efr32s1Adc::new();
            let pa5 = channel_for(PORT_A, 5);
            if overwrite {
                adc.write_word(OFF_SINGLECTRLX, SINGLECTRLX_FIFOOFACT);
            }
            single(&mut adc, aport(3, 13), REF_VDD, RES_12BIT);
            for mv in 1..=5u16 {
                adc.set_channel_input(pa5, mv * 3300 / 4096 + 1);
                adc.write_word(OFF_CMD, CMD_SINGLESTART);
                adc.tick_elapsed(1);
            }
            assert_eq!(adc.read_word(OFF_SINGLEFIFOCOUNT), 4);
            assert_ne!(adc.read_word(OFF_IF) & IF_SINGLEOF, 0);
            assert_eq!(adc.read_u32(OFF_SINGLEDATA).unwrap(), oldest);
        }
    }

    #[test]
    fn if_is_read_only_ifs_sets_ifc_clears_and_ien_raises_the_irq() {
        let mut adc = Efr32s1Adc::new();
        adc.write_word(OFF_IF, IF_SINGLE);
        assert_eq!(adc.read_word(OFF_IF), 0);
        single(&mut adc, POSSEL_VSS, REF_VDD, RES_12BIT);
        adc.write_word(OFF_CMD, CMD_SINGLESTART);
        assert!(!adc.tick_elapsed(1).irq);
        assert_ne!(adc.read_word(OFF_IF) & IF_SINGLE, 0);
        adc.write_word(OFF_IEN, IF_SINGLE);
        assert!(adc.tick_elapsed(1).irq);
        adc.write_word(OFF_IFC, IF_SINGLE);
        assert!(!adc.tick_elapsed(1).irq);
        adc.write_word(OFF_IFS, IF_SINGLEOF);
        assert_eq!(adc.read_word(OFF_IF), IF_SINGLEOF);
    }

    #[test]
    fn singlefifoclear_and_singlestop() {
        let mut adc = Efr32s1Adc::new();
        single(&mut adc, POSSEL_AVDD, REF_VDD, RES_12BIT);
        adc.write_word(OFF_CMD, CMD_SINGLESTART);
        adc.tick_elapsed(1);
        adc.write_word(OFF_SINGLEFIFOCLEAR, 1);
        assert_eq!(adc.read_word(OFF_SINGLEFIFOCOUNT), 0);
        adc.write_word(OFF_CMD, CMD_SINGLESTART);
        assert_ne!(adc.read_word(OFF_STATUS) & STATUS_SINGLEACT, 0);
        adc.write_word(OFF_CMD, CMD_SINGLESTOP);
        adc.tick_elapsed(1);
        assert_eq!(adc.read_word(OFF_SINGLEFIFOCOUNT), 0);
    }

    #[test]
    fn scanstart_converts_nothing() {
        let mut adc = Efr32s1Adc::new();
        adc.write_word(OFF_SCANMASK, 1);
        adc.write_word(OFF_CMD, 1 << 2);
        adc.tick_elapsed(1);
        assert_eq!(adc.read_word(OFF_SINGLEFIFOCOUNT), 0);
        assert_eq!(adc.read_word(OFF_IF), 0);
    }
}
