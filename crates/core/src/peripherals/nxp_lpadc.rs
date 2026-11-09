// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NXP LPADC, the command/trigger-queue SAR ADC of the i.MX RT500 (`ADC0` at
//! `0x4013_A000`, secure alias `0x5013_A000`, `ADC0_IRQn` 22 on the
//! MIMXRT595S).
//!
//! Sources:
//! - mcux-sdk `devices/MIMXRT595S/MIMXRT595S_cm33.h` (`ADC_Type`: `VERID`
//!   @0x00, `PARAM` @0x04, `CTRL` @0x10, `STAT` @0x14, `IE` @0x18, `DE`
//!   @0x1C, `CFG` @0x20, `PAUSE` @0x24, `SWTRIG` @0x34, `TSTAT` @0x38,
//!   `TCTRL[16]` @0xA0, `FCTRL[2]` @0xE0, `CMD[15]` `CMDL/CMDH` @0x100
//!   step 8, `CV[4]` @0x200, `RESFIFO[2]` @0x300) and its bit fields;
//!   `MIMXRT595S_cm33_features.h` (two result FIFOs, `CMDL.DIFF` and
//!   `CMDL.CSCALE`, no `CMDL.MODE`, no `TCTRL.FIFO_SEL_B`).
//! - mcux-soc-svd `MIMXRT595S/MIMXRT595S_cm33.xml`: reset values (`VERID`
//!   0x0200_201A, `PARAM` 0x0F04_1010 = 16 triggers, 16-entry FIFOs, 4 CV,
//!   15 commands; `CFG` 0x0080_0000; `CMDLn` 0x2000) and the `STAT.FOFn`,
//!   `TEXC_INT`, `TCOMP_INT` and `TSTAT` write-1-to-clear fields.
//! - mcux-sdk `drivers/lpadc/fsl_lpadc.{c,h}`: `LPADC_Init` (`CTRL.RST`
//!   pulse, `RSTFIFOn`, `CFG`, `FCTRLn.FWMARK`, `ADCEN`), the command and
//!   trigger set-up, `LPADC_DoSoftwareTrigger`, the `RESFIFO.VALID` check,
//!   and "the ready flag sets when the FIFO holds more words than FWMARK".
//! - mcux-sdk-examples `evkmimxrt595/driver_examples/lpadc/polling`: a
//!   12-bit single-ended result is `RESFIFO.D >> 3` against a 4096 range.
//! - Zephyr v4.0.0 `drivers/adc/adc_mcux_lpadc.c`: one command per Zephyr
//!   channel chained by `CMDH.NEXT`, trigger 0 by software, the FIFO0
//!   watermark interrupt, and the 13-bit differential result with its sign
//!   in bit 15 (`(D >> 3) & 0xFFF`, minus 0x1000 when bit 15 is set).
//!
//! Modelled:
//! - Software triggers (`SWTRIG.SWTn`) while `CTRL.ADCEN` is set: trigger n
//!   runs `TCTRL[n].TCMD` (0 = no command) and its `CMDH.NEXT` chain. Each
//!   command converts `LOOP + 1` times; with `LWI` the channel increments on
//!   each loop. A chained command with `WAIT_TRIG` set waits for its trigger
//!   to be asserted again. A chain that comes back to a command it already
//!   ran in this pass is free-running: the rest of it runs one command per
//!   tick until `ADCEN` clears or `CTRL.RST` is set (`STAT.ADC_ACTIVE`,
//!   `TRGACT` and `CMDACT` show it).
//! - Conversions: `CMDL.ADCH` with `ABSEL` picks the A-side or B-side pad;
//!   `DIFF` converts `CHnA - CHnB` (`ABSEL` = 0) or `CHnB - CHnA`. Results
//!   are 12-bit single-ended in `D[14:3]` or 13-bit two's complement in
//!   `D[15:3]`, 1 LSB = VREF / 4096, truncated and clamped. `CSCALE` = 0
//!   scales the input by 30/64 first.
//! - Result FIFOs: `TCTRL.FIFO_SEL_A` picks FIFO0 or FIFO1 (this IP has no
//!   `FIFO_SEL_B`, so both sides use it). Each entry carries `TSRC`,
//!   `LOOPCNT`, `CMDSRC` and `VALID`; a read of `RESFIFOn` pops it, and an
//!   empty FIFO reads 0. A 17th entry sets `STAT.FOFn` and is dropped.
//!   `FCTRLn.FCOUNT`, `STAT.RDYn` (`FCOUNT > FWMARK`), `CTRL.RSTFIFOn`.
//! - Trigger completion: when a chain ends and `IE.TCOMP_IE` has the
//!   trigger's bit, `TSTAT.TCOMP_FLAG` and `STAT.TCOMP_INT` set (both
//!   write 1 to clear).
//! - The IRQ line: `FWMIEn`/`RDYn`, `FOFIEn`/`FOFn`, and `TCOMP_IE` against
//!   `TSTAT.TCOMP_FLAG`.
//! - `CTRL.RST` holds every register except `CTRL` in reset while set.
//!
//! Not modelled: hardware triggers (`TCTRL.HTEN`; nothing drives them),
//! trigger priority and pre-emption (`TPRI`, `CFG.TPRICTRL/TRES/TCMDRES`,
//! `TSTAT.TEXC_NUM`, which stays 0), the compare function (`CMDH.CMPEN`,
//! `CVn`: every result is stored), conversion and power-up time (`STS`,
//! `AVGS` - the average of a held sample is that sample - `CFG.PUDLY`,
//! `PAUSE`, `TDLY`), DMA (`DE` stored only), and `CFG.REFSEL`: every option
//! converts against the configured `vref_mv`, the VREFH the board ties the
//! reference to (1.8 V on the MIMXRT595-EVK per the Zephyr board overlay).

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::RefCell;
use std::collections::VecDeque;

/// Routable inputs: `ABSEL * 32 + ADCH`, so 0..=31 are the A-side
/// channels and 32..=63 the B side.
pub const INPUTS: usize = 64;
pub const B_SIDE: usize = 32;

const DEFAULT_VREF_MV: u32 = 1800;
const TRIGGERS: usize = 16;
const COMMANDS: usize = 15;
const FIFOS: usize = 2;
const FIFO_DEPTH: usize = 16;
const CV_COUNT: usize = 4;

// ── Register map (MIMXRT595S_cm33.h ADC_Type) ───────────────────────────────
const VERID: u64 = 0x00;
const PARAM: u64 = 0x04;
const CTRL: u64 = 0x10;
const STAT: u64 = 0x14;
const IE: u64 = 0x18;
const DE: u64 = 0x1C;
const CFG: u64 = 0x20;
const PAUSE: u64 = 0x24;
const SWTRIG: u64 = 0x34;
const TSTAT: u64 = 0x38;
const TCTRL0: u64 = 0xA0;
const FCTRL0: u64 = 0xE0;
const CMD1: u64 = 0x100;
const CV0: u64 = 0x200;
const RESFIFO0: u64 = 0x300;

/// SVD reset values.
const VERID_RESET: u32 = 0x0200_201A;
const PARAM_RESET: u32 = 0x0F04_1010;
const CFG_RESET: u32 = 0x0080_0000;
const CMDL_RESET: u32 = 0x2000;

const CTRL_ADCEN: u32 = 1 << 0;
const CTRL_RST: u32 = 1 << 1;
const CTRL_DOZEN: u32 = 1 << 2;
const CTRL_RSTFIFO0: u32 = 1 << 8;
const CTRL_RSTFIFO1: u32 = 1 << 9;
const CTRL_STORED: u32 = CTRL_ADCEN | CTRL_RST | CTRL_DOZEN;

const STAT_RDY0: u32 = 1 << 0;
const STAT_FOF0: u32 = 1 << 1;
const STAT_RDY1: u32 = 1 << 2;
const STAT_FOF1: u32 = 1 << 3;
const STAT_TEXC_INT: u32 = 1 << 8;
const STAT_TCOMP_INT: u32 = 1 << 9;
const STAT_ADC_ACTIVE: u32 = 1 << 11;
const STAT_TRGACT_SHIFT: u32 = 16;
const STAT_CMDACT_SHIFT: u32 = 24;
const STAT_W1C: u32 = STAT_FOF0 | STAT_FOF1 | STAT_TEXC_INT | STAT_TCOMP_INT;

const IE_FWMIE0: u32 = 1 << 0;
const IE_FOFIE0: u32 = 1 << 1;
const IE_FWMIE1: u32 = 1 << 2;
const IE_FOFIE1: u32 = 1 << 3;
const IE_MASK: u32 = 0xFFFF_010F;
const TCOMP_SHIFT: u32 = 16;

const TCTRL_FIFO_SEL_A: u32 = 1 << 1;
const TCTRL_MASK: u32 = 0x0F0F_8F03;
const TCTRL_TCMD_SHIFT: u32 = 24;
const FCTRL_FWMARK_SHIFT: u32 = 16;
const FCTRL_FWMARK_MASK: u32 = 0xF << FCTRL_FWMARK_SHIFT;

const CMDL_ADCH: u32 = 0x1F;
const CMDL_ABSEL: u32 = 1 << 5;
const CMDL_DIFF: u32 = 1 << 6;
const CMDL_CSCALE: u32 = 1 << 13;
const CMDL_MASK: u32 = CMDL_ADCH | CMDL_ABSEL | CMDL_DIFF | CMDL_CSCALE;
const CMDH_WAIT_TRIG: u32 = 1 << 2;
const CMDH_LWI: u32 = 1 << 7;
const CMDH_LOOP_SHIFT: u32 = 16;
const CMDH_NEXT_SHIFT: u32 = 24;
const CMDH_MASK: u32 = 0x0F0F_7787;

const RESFIFO_TSRC_SHIFT: u32 = 16;
const RESFIFO_LOOPCNT_SHIFT: u32 = 20;
const RESFIFO_CMDSRC_SHIFT: u32 = 24;
const RESFIFO_VALID: u32 = 1 << 31;

/// A chain that loops back on itself without `WAIT_TRIG`.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct FreeRun {
    trigger: usize,
    /// The 1-based command the next tick runs.
    command: usize,
}

/// NXP LPADC with two result FIFOs and fifteen command buffers.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct NxpLpadc {
    ctrl: u32,
    /// `FOFn`, `TEXC_INT`, `TCOMP_INT`; `RDYn` and the activity fields are
    /// derived.
    stat: u32,
    ie: u32,
    de: u32,
    cfg: u32,
    pause: u32,
    /// `TCOMP_FLAG` in bits 31:16.
    tstat: u32,
    tctrl: [u32; TRIGGERS],
    fwmark: [u32; FIFOS],
    cmdl: [u32; COMMANDS],
    cmdh: [u32; COMMANDS],
    cv: [u32; CV_COUNT],
    /// `RefCell` because a firmware read of `RESFIFOn` pops an entry.
    fifo: [RefCell<VecDeque<u32>>; FIFOS],
    /// The 1-based command a chain stopped at on `WAIT_TRIG`, per trigger.
    waiting: [u8; TRIGGERS],
    free_run: Option<FreeRun>,
    /// Routed pad levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: Vec<u16>,
    vref_mv: u32,
}

impl Default for NxpLpadc {
    fn default() -> Self {
        Self::new(DEFAULT_VREF_MV)
    }
}

impl NxpLpadc {
    pub fn new(vref_mv: u32) -> Self {
        Self {
            ctrl: 0,
            stat: 0,
            ie: 0,
            de: 0,
            cfg: CFG_RESET,
            pause: 0,
            tstat: 0,
            tctrl: [0; TRIGGERS],
            fwmark: [0; FIFOS],
            cmdl: [CMDL_RESET; COMMANDS],
            cmdh: [0; COMMANDS],
            cv: [0; CV_COUNT],
            fifo: [RefCell::new(VecDeque::new()), RefCell::new(VecDeque::new())],
            waiting: [0; TRIGGERS],
            free_run: None,
            inputs_mv: vec![u16::MAX; INPUTS],
            vref_mv: vref_mv.max(1),
        }
    }

    /// Every register except `CTRL` back to its reset value.
    fn reset_logic(&mut self) {
        let ctrl = self.ctrl;
        let inputs = std::mem::take(&mut self.inputs_mv);
        *self = Self::new(self.vref_mv);
        self.ctrl = ctrl;
        self.inputs_mv = inputs;
    }

    fn input_mv(&self, index: usize) -> i64 {
        self.inputs_mv
            .get(index)
            .copied()
            .filter(|mv| *mv != u16::MAX)
            .map_or(0, i64::from)
    }

    /// `RESFIFO.D` for one conversion of `cmdl` on channel `adch`.
    fn convert(&self, cmdl: u32, adch: usize) -> u16 {
        let a = self.input_mv(adch);
        let b = self.input_mv(B_SIDE + adch);
        let scale = |mv: i64| {
            if cmdl & CMDL_CSCALE != 0 {
                mv
            } else {
                mv * 30 / 64
            }
        };
        let vref = i64::from(self.vref_mv);
        if cmdl & CMDL_DIFF != 0 {
            let (plus, minus) = if cmdl & CMDL_ABSEL == 0 {
                (a, b)
            } else {
                (b, a)
            };
            let code = ((scale(plus) - scale(minus)) * 4096 / vref).clamp(-4096, 4095);
            ((code as i16) << 3) as u16
        } else {
            let side = if cmdl & CMDL_ABSEL == 0 { a } else { b };
            let code = (scale(side) * 4096 / vref).clamp(0, 4095);
            (code as u16) << 3
        }
    }

    fn push(&mut self, fifo: usize, entry: u32) {
        let mut queue = self.fifo[fifo].borrow_mut();
        if queue.len() >= FIFO_DEPTH {
            self.stat |= if fifo == 0 { STAT_FOF0 } else { STAT_FOF1 };
        } else {
            queue.push_back(entry);
        }
    }

    /// Run 1-based `command` for `trigger`; returns its `NEXT`.
    fn run_command(&mut self, trigger: usize, command: usize) -> usize {
        let cmdl = self.cmdl[command - 1];
        let cmdh = self.cmdh[command - 1];
        let fifo = usize::from(self.tctrl[trigger] & TCTRL_FIFO_SEL_A != 0);
        let loops = ((cmdh >> CMDH_LOOP_SHIFT) & 0xF) as usize;
        for pass in 0..=loops {
            let step = if cmdh & CMDH_LWI != 0 { pass } else { 0 };
            let adch = ((cmdl & CMDL_ADCH) as usize + step) & CMDL_ADCH as usize;
            let entry = u32::from(self.convert(cmdl, adch))
                | ((trigger as u32) << RESFIFO_TSRC_SHIFT)
                | ((pass as u32) << RESFIFO_LOOPCNT_SHIFT)
                | ((command as u32) << RESFIFO_CMDSRC_SHIFT)
                | RESFIFO_VALID;
            self.push(fifo, entry);
        }
        ((cmdh >> CMDH_NEXT_SHIFT) & 0xF) as usize
    }

    /// The chain has ended: flag completion if its interrupt is enabled.
    fn complete(&mut self, trigger: usize) {
        let bit = 1u32 << (TCOMP_SHIFT + trigger as u32);
        if self.ie & bit != 0 {
            self.tstat |= bit;
            self.stat |= STAT_TCOMP_INT;
        }
    }

    fn software_trigger(&mut self, trigger: usize) {
        if self.ctrl & CTRL_ADCEN == 0 || self.ctrl & CTRL_RST != 0 {
            return;
        }
        let mut command = match std::mem::take(&mut self.waiting[trigger]) {
            0 => (self.tctrl[trigger] >> TCTRL_TCMD_SHIFT) as usize & 0xF,
            resume => resume as usize,
        };
        let mut ran = 0u32;
        while (1..=COMMANDS).contains(&command) {
            ran |= 1 << command;
            let next = self.run_command(trigger, command);
            if !(1..=COMMANDS).contains(&next) {
                break;
            }
            if self.cmdh[next - 1] & CMDH_WAIT_TRIG != 0 {
                self.waiting[trigger] = next as u8;
                return;
            }
            if ran & (1 << next) != 0 {
                self.free_run = Some(FreeRun {
                    trigger,
                    command: next,
                });
                return;
            }
            command = next;
        }
        self.complete(trigger);
    }

    fn rdy(&self, fifo: usize) -> bool {
        self.fifo[fifo].borrow().len() as u32 > self.fwmark[fifo]
    }

    fn stat_value(&self) -> u32 {
        let mut value = self.stat;
        if self.rdy(0) {
            value |= STAT_RDY0;
        }
        if self.rdy(1) {
            value |= STAT_RDY1;
        }
        if let Some(run) = self.free_run {
            value |= STAT_ADC_ACTIVE
                | ((run.trigger as u32) << STAT_TRGACT_SHIFT)
                | ((run.command as u32) << STAT_CMDACT_SHIFT);
        }
        value
    }

    fn irq_asserted(&self) -> bool {
        let stat = self.stat_value();
        (self.ie & IE_FWMIE0 != 0 && stat & STAT_RDY0 != 0)
            || (self.ie & IE_FOFIE0 != 0 && stat & STAT_FOF0 != 0)
            || (self.ie & IE_FWMIE1 != 0 && stat & STAT_RDY1 != 0)
            || (self.ie & IE_FOFIE1 != 0 && stat & STAT_FOF1 != 0)
            || self.ie & self.tstat & (0xFFFF << TCOMP_SHIFT) != 0
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            VERID => VERID_RESET,
            PARAM => PARAM_RESET,
            CTRL => self.ctrl,
            STAT => self.stat_value(),
            IE => self.ie,
            DE => self.de,
            CFG => self.cfg,
            PAUSE => self.pause,
            TSTAT => self.tstat,
            o if (TCTRL0..TCTRL0 + 4 * TRIGGERS as u64).contains(&o) => {
                self.tctrl[((o - TCTRL0) / 4) as usize]
            }
            o if (FCTRL0..FCTRL0 + 4 * FIFOS as u64).contains(&o) => {
                let n = ((o - FCTRL0) / 4) as usize;
                self.fifo[n].borrow().len() as u32 | (self.fwmark[n] << FCTRL_FWMARK_SHIFT)
            }
            o if (CMD1..CMD1 + 8 * COMMANDS as u64).contains(&o) => {
                let n = ((o - CMD1) / 8) as usize;
                if o & 4 == 0 {
                    self.cmdl[n]
                } else {
                    self.cmdh[n]
                }
            }
            o if (CV0..CV0 + 4 * CV_COUNT as u64).contains(&o) => self.cv[((o - CV0) / 4) as usize],
            o if (RESFIFO0..RESFIFO0 + 4 * FIFOS as u64).contains(&o) => {
                let n = ((o - RESFIFO0) / 4) as usize;
                self.fifo[n].borrow_mut().pop_front().unwrap_or(0)
            }
            // SWTRIG self-clears once the trigger is taken; gaps read 0.
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        if self.ctrl & CTRL_RST != 0 && offset != CTRL {
            return;
        }
        match offset {
            CTRL => {
                self.ctrl = value & CTRL_STORED;
                if value & CTRL_RST != 0 {
                    self.reset_logic();
                }
                if value & CTRL_RSTFIFO0 != 0 {
                    self.fifo[0].borrow_mut().clear();
                }
                if value & CTRL_RSTFIFO1 != 0 {
                    self.fifo[1].borrow_mut().clear();
                }
                if value & CTRL_ADCEN == 0 {
                    self.free_run = None;
                }
            }
            STAT => self.stat &= !(value & STAT_W1C),
            IE => self.ie = value & IE_MASK,
            DE => self.de = value & 0xF,
            CFG => self.cfg = value,
            PAUSE => self.pause = value,
            SWTRIG => {
                for trigger in 0..TRIGGERS {
                    if value & (1 << trigger) != 0 {
                        self.software_trigger(trigger);
                    }
                }
            }
            TSTAT => {
                self.tstat &= !value;
                if self.tstat & (0xFFFF << TCOMP_SHIFT) == 0 {
                    self.stat &= !STAT_TCOMP_INT;
                }
            }
            o if (TCTRL0..TCTRL0 + 4 * TRIGGERS as u64).contains(&o) => {
                self.tctrl[((o - TCTRL0) / 4) as usize] = value & TCTRL_MASK;
            }
            o if (FCTRL0..FCTRL0 + 4 * FIFOS as u64).contains(&o) => {
                self.fwmark[((o - FCTRL0) / 4) as usize] =
                    (value & FCTRL_FWMARK_MASK) >> FCTRL_FWMARK_SHIFT;
            }
            o if (CMD1..CMD1 + 8 * COMMANDS as u64).contains(&o) => {
                let n = ((o - CMD1) / 8) as usize;
                if o & 4 == 0 {
                    self.cmdl[n] = value & CMDL_MASK;
                } else {
                    self.cmdh[n] = value & CMDH_MASK;
                }
            }
            o if (CV0..CV0 + 4 * CV_COUNT as u64).contains(&o) => {
                self.cv[((o - CV0) / 4) as usize] = value;
            }
            // VERID, PARAM, RESFIFOn are read-only.
            _ => {}
        }
    }

    /// The word a byte write merges into, without read side effects.
    fn stored_word(&self, offset: u64) -> u32 {
        match offset {
            STAT | SWTRIG | TSTAT => 0,
            o if (RESFIFO0..RESFIFO0 + 4 * FIFOS as u64).contains(&o) => 0,
            o => self.read_word(o),
        }
    }
}

impl Peripheral for NxpLpadc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(INPUTS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        let Some(input) = self.inputs_mv.get_mut(channel as usize) else {
            return false;
        };
        *input = millivolts;
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        let Some(input) = self.inputs_mv.get_mut(channel as usize) else {
            return false;
        };
        *input = u16::MAX;
        true
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_asserted())
    }

    fn on_system_reset(&mut self) {
        let inputs = std::mem::take(&mut self.inputs_mv);
        *self = Self::new(self.vref_mv);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        let current = self.stored_word(aligned);
        let next = (current & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_word(aligned, next);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset & !3, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if let Some(run) = self.free_run {
            let next = self.run_command(run.trigger, run.command);
            self.free_run = if (1..=COMMANDS).contains(&next) {
                Some(FreeRun {
                    trigger: run.trigger,
                    command: next,
                })
            } else {
                self.complete(run.trigger);
                None
            };
        }
        PeripheralTickResult {
            irq: self.irq_asserted(),
            ..PeripheralTickResult::default()
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

    const ENTRY_D: u32 = 0xFFFF;

    fn lpadc_with(inputs: &[(usize, u16)]) -> NxpLpadc {
        let mut adc = NxpLpadc::default();
        for &(index, mv) in inputs {
            assert!(adc.set_adc_channel_input(index as u8, mv));
        }
        adc
    }

    /// fsl_lpadc LPADC_Init with the default config (FWMARK 0) plus
    /// Zephyr's `enableAnalogPreliminary`.
    fn init(adc: &mut NxpLpadc) {
        adc.write_u32(CTRL, CTRL_RST).unwrap();
        adc.write_u32(CTRL, 0).unwrap();
        adc.write_u32(CTRL, CTRL_RSTFIFO0).unwrap();
        adc.write_u32(CTRL, CTRL_RSTFIFO1).unwrap();
        adc.write_u32(CTRL, CTRL_DOZEN).unwrap();
        adc.write_u32(CFG, (1 << 28) | (0x80 << 16)).unwrap();
        adc.write_u32(PAUSE, 0).unwrap();
        adc.write_u32(FCTRL0, 0).unwrap();
        adc.write_u32(FCTRL0 + 4, 0).unwrap();
        adc.write_u32(CTRL, CTRL_DOZEN | CTRL_ADCEN).unwrap();
    }

    fn command(adc: &mut NxpLpadc, id: u64, cmdl: u32, cmdh: u32) {
        adc.write_u32(CMD1 + (id - 1) * 8, cmdl).unwrap();
        adc.write_u32(CMD1 + (id - 1) * 8 + 4, cmdh).unwrap();
    }

    fn se12(mv: u32, vref: u32) -> u32 {
        (mv * 4096 / vref).min(4095) << 3
    }

    #[test]
    fn reset_values_match_the_svd() {
        let adc = NxpLpadc::default();
        assert_eq!(adc.read_u32(VERID).unwrap(), 0x0200_201A);
        assert_eq!(adc.read_u32(PARAM).unwrap(), 0x0F04_1010);
        assert_eq!(adc.read_u32(CFG).unwrap(), 0x0080_0000);
        assert_eq!(adc.read_u32(CMD1).unwrap(), 0x2000);
        assert_eq!(adc.read_u32(CMD1 + 14 * 8).unwrap(), 0x2000, "CMDL15");
        assert_eq!(adc.read_u32(RESFIFO0).unwrap(), 0, "empty FIFO: VALID 0");
    }

    #[test]
    fn sdk_polling_flow_converts_a_side_and_b_side_pads() {
        // CH0A = 900 mV, CH0B = 1500 mV, VREF 1.8 V.
        let mut adc = lpadc_with(&[(0, 900), (B_SIDE, 1500)]);
        init(&mut adc);
        // lpadc_polling: command 15 = CH0A, full scale; trigger 0 -> CMD 15.
        command(&mut adc, 15, CMDL_CSCALE, 0);
        adc.write_u32(TCTRL0, 15 << TCTRL_TCMD_SHIFT).unwrap();
        // A trigger with ADCEN clear converts nothing.
        adc.write_u32(CTRL, CTRL_DOZEN).unwrap();
        adc.write_u32(SWTRIG, 1).unwrap();
        assert_eq!(adc.read_u32(FCTRL0).unwrap() & 0x1F, 0);
        adc.write_u32(CTRL, CTRL_DOZEN | CTRL_ADCEN).unwrap();
        adc.write_u32(SWTRIG, 1).unwrap();
        assert_eq!(adc.read_u32(SWTRIG).unwrap(), 0, "SWTRIG self-clears");
        assert_ne!(adc.read_u32(STAT).unwrap() & STAT_RDY0, 0);
        let entry = adc.read_u32(RESFIFO0).unwrap();
        assert_ne!(entry & RESFIFO_VALID, 0);
        assert_eq!(entry & ENTRY_D, se12(900, 1800));
        assert_eq!((entry & ENTRY_D) >> 3, 2048, "example: D >> 3, /4096");
        assert_eq!((entry >> RESFIFO_CMDSRC_SHIFT) & 0xF, 15);
        assert_eq!((entry >> RESFIFO_TSRC_SHIFT) & 0xF, 0);
        assert_eq!(adc.read_u32(STAT).unwrap() & STAT_RDY0, 0, "drained");
        // ABSEL selects the B-side pad of the same channel.
        command(&mut adc, 15, CMDL_CSCALE | CMDL_ABSEL, 0);
        adc.write_u32(SWTRIG, 1).unwrap();
        assert_eq!(adc.read_u32(RESFIFO0).unwrap() & ENTRY_D, se12(1500, 1800));
        // CSCALE = 0 scales by 30/64 first.
        command(&mut adc, 15, 0, 0);
        adc.write_u32(SWTRIG, 1).unwrap();
        assert_eq!(
            adc.read_u32(RESFIFO0).unwrap() & ENTRY_D,
            se12(900 * 30 / 64, 1800)
        );
        // Over-range clamps, undriven reads 0.
        adc.set_adc_channel_input(0, 2500);
        command(&mut adc, 15, CMDL_CSCALE, 0);
        adc.write_u32(SWTRIG, 1).unwrap();
        assert_eq!(adc.read_u32(RESFIFO0).unwrap() & ENTRY_D, 4095 << 3);
        command(&mut adc, 15, CMDL_CSCALE | 3, 0);
        adc.write_u32(SWTRIG, 1).unwrap();
        assert_eq!(adc.read_u32(RESFIFO0).unwrap() & ENTRY_D, 0);
    }

    #[test]
    fn differential_results_are_13_bit_with_the_sign_in_bit_15() {
        let mut adc = lpadc_with(&[(2, 400), (B_SIDE + 2, 1000)]);
        init(&mut adc);
        adc.write_u32(TCTRL0, 1 << TCTRL_TCMD_SHIFT).unwrap();
        let zephyr = |d: u32| -> i32 {
            let mut r = ((d >> 3) & 0xFFF) as i32;
            if d & 0x8000 != 0 {
                r -= 0x1000;
            }
            r
        };
        // DIFF, ABSEL = 0: CH2A - CH2B = -600 mV.
        command(&mut adc, 1, CMDL_CSCALE | CMDL_DIFF | 2, 0);
        adc.write_u32(SWTRIG, 1).unwrap();
        let d = adc.read_u32(RESFIFO0).unwrap() & ENTRY_D;
        assert_eq!(zephyr(d), -600 * 4096 / 1800);
        // ABSEL = 1: CH2B - CH2A = +600 mV.
        command(&mut adc, 1, CMDL_CSCALE | CMDL_DIFF | CMDL_ABSEL | 2, 0);
        adc.write_u32(SWTRIG, 1).unwrap();
        let d = adc.read_u32(RESFIFO0).unwrap() & ENTRY_D;
        assert_eq!(zephyr(d), 600 * 4096 / 1800);
        assert_eq!(d & 0x8000, 0);
    }

    #[test]
    fn next_chain_loop_lwi_and_wait_trig() {
        let mut adc = lpadc_with(&[(0, 100), (1, 200), (2, 300), (5, 600)]);
        init(&mut adc);
        // CMD1: CH0, LOOP 2 with LWI -> CH0, CH1, CH2; NEXT 2.
        command(
            &mut adc,
            1,
            CMDL_CSCALE,
            CMDH_LWI | (2 << CMDH_LOOP_SHIFT) | (2 << CMDH_NEXT_SHIFT),
        );
        // CMD2: CH5, NEXT 3; CMD3 waits for the trigger again.
        command(&mut adc, 2, CMDL_CSCALE | 5, 3 << CMDH_NEXT_SHIFT);
        command(&mut adc, 3, CMDL_CSCALE | 1, CMDH_WAIT_TRIG);
        // Trigger 4 into FIFO1, completion interrupt on trigger 4.
        adc.write_u32(TCTRL0 + 16, TCTRL_FIFO_SEL_A | (1 << TCTRL_TCMD_SHIFT))
            .unwrap();
        adc.write_u32(IE, 1 << (TCOMP_SHIFT + 4)).unwrap();
        adc.write_u32(SWTRIG, 1 << 4).unwrap();
        assert_eq!(adc.read_u32(FCTRL0).unwrap() & 0x1F, 0, "FIFO0 unused");
        assert_eq!(adc.read_u32(FCTRL0 + 4).unwrap() & 0x1F, 4);
        let entries: Vec<u32> = (0..4)
            .map(|_| adc.read_u32(RESFIFO0 + 4).unwrap())
            .collect();
        let d: Vec<u32> = entries.iter().map(|e| e & ENTRY_D).collect();
        assert_eq!(
            d,
            [
                se12(100, 1800),
                se12(200, 1800),
                se12(300, 1800),
                se12(600, 1800)
            ]
        );
        let loopcnt: Vec<u32> = entries.iter().map(|e| (e >> 20) & 0xF).collect();
        assert_eq!(loopcnt, [0, 1, 2, 0]);
        assert!(entries.iter().all(|e| (e >> 16) & 0xF == 4), "TSRC 4");
        // CMD3 is waiting: no completion yet.
        assert_eq!(adc.read_u32(TSTAT).unwrap(), 0);
        assert!(!adc.tick().irq);
        adc.write_u32(SWTRIG, 1 << 4).unwrap();
        let resumed = adc.read_u32(RESFIFO0 + 4).unwrap();
        assert_eq!((resumed >> 24) & 0xF, 3, "the second trigger runs CMD3");
        assert_eq!(resumed & ENTRY_D, se12(200, 1800));
        assert_eq!(adc.read_u32(TSTAT).unwrap(), 1 << 20);
        assert_ne!(adc.read_u32(STAT).unwrap() & STAT_TCOMP_INT, 0);
        assert!(adc.tick().irq);
        adc.write_u32(TSTAT, 1 << 20).unwrap();
        assert_eq!(adc.read_u32(STAT).unwrap() & STAT_TCOMP_INT, 0);
        assert!(!adc.tick().irq);
    }

    #[test]
    fn watermark_overflow_and_irq() {
        let mut adc = lpadc_with(&[(3, 1800)]);
        init(&mut adc);
        command(&mut adc, 1, CMDL_CSCALE | 3, 0);
        adc.write_u32(TCTRL0, 1 << TCTRL_TCMD_SHIFT).unwrap();
        adc.write_u32(FCTRL0, 2 << FCTRL_FWMARK_SHIFT).unwrap();
        adc.write_u32(IE, IE_FWMIE0 | IE_FOFIE0).unwrap();
        for n in 1..=3u32 {
            adc.write_u32(SWTRIG, 1).unwrap();
            let rdy = adc.read_u32(STAT).unwrap() & STAT_RDY0 != 0;
            assert_eq!(rdy, n > 2, "RDY0 once FCOUNT ({n}) > FWMARK (2)");
            assert_eq!(adc.tick().irq, n > 2);
        }
        for _ in 0..14 {
            adc.write_u32(SWTRIG, 1).unwrap();
        }
        assert_eq!(adc.read_u32(FCTRL0).unwrap(), 16 | (2 << 16));
        assert_ne!(adc.read_u32(STAT).unwrap() & STAT_FOF0, 0, "17th entry");
        adc.write_u32(CTRL, CTRL_ADCEN | CTRL_RSTFIFO0).unwrap();
        assert_eq!(adc.read_u32(FCTRL0).unwrap() & 0x1F, 0);
        assert!(adc.tick().irq, "FOF0 is still set");
        adc.write_u32(STAT, STAT_FOF0).unwrap();
        assert!(!adc.tick().irq);
    }

    #[test]
    fn a_looping_chain_free_runs_one_command_per_tick() {
        let mut adc = lpadc_with(&[(0, 450)]);
        init(&mut adc);
        // CMD1 -> CMD1: continuous conversion.
        command(&mut adc, 1, CMDL_CSCALE, 1 << CMDH_NEXT_SHIFT);
        adc.write_u32(TCTRL0, 1 << TCTRL_TCMD_SHIFT).unwrap();
        adc.write_u32(SWTRIG, 1).unwrap();
        let stat = adc.read_u32(STAT).unwrap();
        assert_ne!(stat & STAT_ADC_ACTIVE, 0);
        assert_eq!((stat >> 24) & 0xF, 1, "CMDACT");
        assert_eq!(adc.read_u32(RESFIFO0).unwrap() & ENTRY_D, se12(450, 1800));
        adc.set_adc_channel_input(0, 1350);
        adc.tick();
        assert_eq!(adc.read_u32(RESFIFO0).unwrap() & ENTRY_D, se12(1350, 1800));
        // Clearing ADCEN stops it.
        adc.write_u32(CTRL, 0).unwrap();
        adc.tick();
        assert_eq!(adc.read_u32(RESFIFO0).unwrap(), 0);
        assert_eq!(adc.read_u32(STAT).unwrap() & STAT_ADC_ACTIVE, 0);
    }

    #[test]
    fn ctrl_rst_holds_the_logic_in_reset() {
        let mut adc = lpadc_with(&[(0, 900)]);
        init(&mut adc);
        command(&mut adc, 1, CMDL_CSCALE, 0);
        adc.write_u32(TCTRL0, 1 << TCTRL_TCMD_SHIFT).unwrap();
        adc.write_u32(SWTRIG, 1).unwrap();
        adc.write_u32(CTRL, CTRL_RST | CTRL_ADCEN).unwrap();
        assert_eq!(adc.read_u32(TCTRL0).unwrap(), 0);
        assert_eq!(adc.read_u32(CMD1).unwrap(), CMDL_RESET);
        assert_eq!(adc.read_u32(FCTRL0).unwrap(), 0);
        adc.write_u32(TCTRL0, 1 << TCTRL_TCMD_SHIFT).unwrap();
        assert_eq!(adc.read_u32(TCTRL0).unwrap(), 0, "writes ignored in reset");
        adc.write_u32(CTRL, CTRL_ADCEN).unwrap();
        // The routed input survives the reset.
        adc.write_u32(TCTRL0, 1 << TCTRL_TCMD_SHIFT).unwrap();
        adc.write_u32(SWTRIG, 1).unwrap();
        // CMDL1 is back at its 0x2000 reset value: CSCALE = full scale.
        assert_eq!(adc.read_u32(RESFIFO0).unwrap() & ENTRY_D, se12(900, 1800));
        assert!(!adc.set_adc_channel_input(64, 1));
    }
}
