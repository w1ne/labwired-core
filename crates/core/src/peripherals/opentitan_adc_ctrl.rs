// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! OpenTitan `adc_ctrl`: the always-on controller for the two-channel AST
//! ADC (Earl Grey `adc_ctrl_aon` at 0x4044_0000, pads CC1/CC2). 10-bit
//! samples, eight filter pairs, debounce, oneshot, normal-power and
//! low-power scanning.
//!
//! Sources (lowRISC/opentitan master):
//! - `hw/ip/adc_ctrl/data/adc_ctrl.hjson` and `doc/registers.md`: every
//!   offset, field, reset value and access (`INTR_STATE` 0x00 to
//!   `adc_fsm_state` 0x7C); `filter_status` and `adc_intr_status` are
//!   rw1c; 2.148 mV per step over 0..2.2 V.
//! - `doc/theory_of_operation.md`: the sampling sequence, filter `cond`
//!   (inside / outside `[min, max]`), the debounce counters, the
//!   `adc_chn_value_intr` latch.
//! - `rtl/adc_ctrl_fsm.sv`: the state machine (`PWRDN`, `PWRUP`, `ONEST_*`,
//!   `LP_*`, `NP_*`), `trigger_l2h`/`trigger_h2l` on `adc_enable`,
//!   `pwrup_time` and `wakeup_time` counters, `stay_match` (non-zero and
//!   equal to the previous set, or the previous set empty), the
//!   `lp_sample_cnt - 1` / `np_sample_cnt - 1` thresholds, `NP_DONE`
//!   once per new match, `adc_fsm_rst` clears state, counters, the match
//!   set and the internal channel values.
//! - `rtl/adc_ctrl_core.sv`: per-pair `match` (an enabled channel filter
//!   must hit, a disabled one is ignored, both disabled never match),
//!   `filter_status` set by `match_pulse` and the lp->np transition, the
//!   `chn_val_intr` latch on `oneshot_done` (oneshot mode) or a match pulse.
//! - `rtl/adc_ctrl_intr.sv`: `adc_intr_status.match` takes match pulses of
//!   filters enabled in `adc_intr_ctl`, `trans` and `oneshot` their own
//!   enables; `INTR_STATE` is a status interrupt = OR of
//!   `adc_intr_status`; the line is `INTR_STATE & INTR_ENABLE`.
//! - `hw/top_earlgrey/ip/ast/rtl/adc_ana.sv`: the behavioural converter,
//!   `code = (V / vref) * 0x3FF` truncated.
//! - `hw/top_earlgrey/rtl/autogen/chip_earlgrey_asic.sv`:
//!   `ast_adc_a0_a = CC1`, `ast_adc_a1_a = CC2`.
//! - `sw/device/lib/dif/dif_adc_ctrl.c`: `dif_adc_ctrl_reset` writes
//!   `adc_fsm_rst` 1 then 0 with no wait, so the reset acts at the write.
//!
//! Modelled: one FSM state per tick, with the `*_021` hand-over states
//! folded into the sample states (a sample is taken in the tick of its
//! `*_0`/`*_1` state). `pwrup_time` and `wakeup_time` count ticks. A
//! sample is `mV * 1023 / vref_mv`, truncated, clamped at 0x3FF; `vref_mv`
//! defaults to 2200 (the IP doc's 0..2.2 V range).
//!
//! Not sourced, kept minimal: the clamp (`adc_ana.sv` would wrap in its
//! 10-bit port); the aon/bus clock-domain crossing is instant; the
//! `wkup_req` output and `INTR_TEST` are not wired (`INTR_TEST` sets
//! `INTR_STATE` until the next tick); `ALERT_TEST` is ignored.

use crate::{Peripheral, PeripheralTickResult, SimResult};

const CHANNELS: usize = 2;
const FILTERS: usize = 8;
const CODE_MAX: u32 = 0x3FF;

const INTR_STATE: u64 = 0x00;
const INTR_ENABLE: u64 = 0x04;
const INTR_TEST: u64 = 0x08;
const ADC_EN_CTL: u64 = 0x10;
const ADC_PD_CTL: u64 = 0x14;
const ADC_LP_SAMPLE_CTL: u64 = 0x18;
const ADC_SAMPLE_CTL: u64 = 0x1C;
const ADC_FSM_RST: u64 = 0x20;
const ADC_CHN0_FILTER_CTL: u64 = 0x24;
const ADC_CHN1_FILTER_CTL: u64 = 0x44;
const ADC_CHN_VAL: u64 = 0x64;
const ADC_WAKEUP_CTL: u64 = 0x6C;
const FILTER_STATUS: u64 = 0x70;
const ADC_INTR_CTL: u64 = 0x74;
const ADC_INTR_STATUS: u64 = 0x78;
const ADC_FSM_STATE: u64 = 0x7C;

const ADC_ENABLE: u32 = 1 << 0;
const ONESHOT_MODE: u32 = 1 << 1;
const LP_MODE: u32 = 1 << 0;
/// `lp_mode` 0, `pwrup_time` [7:4], `wakeup_time` [31:8].
const PD_CTL_MASK: u32 = 0xFFFF_FFF1;
const PD_CTL_RESET: u32 = (1600 << 8) | (7 << 4);
/// `min_v` [11:2], `cond` 12, `max_v` [27:18], `EN` 31.
const FILTER_MASK: u32 = 0x8FFC_1FFC;
const FILTER_COND: u32 = 1 << 12;
const FILTER_EN: u32 = 1 << 31;
const TRANS: u32 = 1 << FILTERS;
const ONESHOT: u32 = 1 << (FILTERS + 1);

/// `adc_fsm_state` encodings (`adc_ctrl_pkg::fsm_state_e`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum State {
    Pwrdn = 0,
    Pwrup = 1,
    Onest0 = 2,
    Onest1 = 4,
    OnestDone = 5,
    Lp0 = 6,
    Lp1 = 8,
    LpEval = 9,
    LpSlp = 10,
    LpPwrup = 11,
    Np0 = 12,
    Np1 = 14,
    NpEval = 15,
    NpDone = 16,
}

/// OpenTitan `adc_ctrl` with its AST ADC.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct OpentitanAdcCtrl {
    intr_enable: u32,
    intr_test: bool,
    en_ctl: u32,
    pd_ctl: u32,
    lp_sample_ctl: u32,
    sample_ctl: u32,
    fsm_rst: u32,
    filter_ctl: [[u32; FILTERS]; CHANNELS],
    /// `adc_chn_val[n]`: latest value [11:2], interrupt value [27:18].
    chn_val: [u32; CHANNELS],
    wakeup_ctl: u32,
    filter_status: u32,
    intr_ctl: u32,
    intr_status: u32,
    state: State,
    trigger_q: bool,
    pwrup_cnt: u32,
    wakeup_cnt: u32,
    lp_cnt: u8,
    np_cnt: u16,
    match_q: u8,
    /// The FSM's own channel values, which the filters compare.
    fsm_val: [u32; CHANNELS],
    vref_mv: u32,
    /// Routed levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; CHANNELS],
}

impl Default for OpentitanAdcCtrl {
    fn default() -> Self {
        Self::new(2200)
    }
}

impl OpentitanAdcCtrl {
    /// `vref_mv`: the AST ADC full scale (2200 mV per the IP doc).
    pub fn new(vref_mv: u32) -> Self {
        Self {
            intr_enable: 0,
            intr_test: false,
            en_ctl: 0,
            pd_ctl: PD_CTL_RESET,
            lp_sample_ctl: 4,
            sample_ctl: 155,
            fsm_rst: 0,
            filter_ctl: [[0; FILTERS]; CHANNELS],
            chn_val: [0; CHANNELS],
            wakeup_ctl: 0,
            filter_status: 0,
            intr_ctl: 0,
            intr_status: 0,
            state: State::Pwrdn,
            trigger_q: false,
            pwrup_cnt: 0,
            wakeup_cnt: 0,
            lp_cnt: 0,
            np_cnt: 0,
            match_q: 0,
            fsm_val: [0; CHANNELS],
            vref_mv: vref_mv.max(1),
            inputs_mv: [u16::MAX; CHANNELS],
        }
    }

    fn code(&self, channel: usize) -> u32 {
        let mv = match self.inputs_mv[channel] {
            u16::MAX => 0,
            mv => u32::from(mv),
        };
        (mv * CODE_MAX / self.vref_mv).min(CODE_MAX)
    }

    fn sample(&mut self, channel: usize) {
        let code = self.code(channel);
        self.fsm_val[channel] = code;
        self.chn_val[channel] = (self.chn_val[channel] & !(CODE_MAX << 2)) | (code << 2);
    }

    /// `adc_ctrl_core.sv` `match[k]` over the FSM's channel values.
    fn matches(&self) -> u8 {
        let mut set = 0;
        for k in 0..FILTERS {
            let mut any = false;
            let mut hit = true;
            for (ch, &val) in self.fsm_val.iter().enumerate() {
                let ctl = self.filter_ctl[ch][k];
                if ctl & FILTER_EN == 0 {
                    continue;
                }
                any = true;
                let min = (ctl >> 2) & CODE_MAX;
                let max = (ctl >> 18) & CODE_MAX;
                let inside = min <= val && val <= max;
                hit &= inside != (ctl & FILTER_COND != 0);
            }
            if any && hit {
                set |= 1 << k;
            }
        }
        set
    }

    /// `ld_match`: compare against the previous set, then load it.
    fn load_match(&mut self) -> (u8, bool) {
        let set = self.matches();
        let stay = set != 0 && (set == self.match_q || self.match_q == 0);
        self.match_q = set;
        (set, stay)
    }

    fn latch_intr_values(&mut self) {
        for ch in 0..CHANNELS {
            self.chn_val[ch] = (self.chn_val[ch] & !(CODE_MAX << 18)) | (self.fsm_val[ch] << 18);
        }
    }

    fn fsm_reset(&mut self) {
        self.state = State::Pwrdn;
        self.trigger_q = false;
        self.pwrup_cnt = 0;
        self.wakeup_cnt = 0;
        self.lp_cnt = 0;
        self.np_cnt = 0;
        self.match_q = 0;
        self.fsm_val = [0; CHANNELS];
    }

    fn pwrup_time(&self) -> u32 {
        (self.pd_ctl >> 4) & 0xF
    }

    fn wakeup_time(&self) -> u32 {
        self.pd_ctl >> 8
    }

    fn lp_mode(&self) -> bool {
        self.pd_ctl & LP_MODE != 0
    }

    /// One `clk_aon` step of `adc_ctrl_fsm.sv`.
    fn step(&mut self) {
        if self.fsm_rst & 1 != 0 {
            self.fsm_reset();
            return;
        }
        let enable = self.en_ctl & ADC_ENABLE != 0;
        let l2h = !self.trigger_q && enable;
        let h2l = self.trigger_q && !enable;
        self.trigger_q = enable;
        if h2l {
            self.state = State::Pwrdn;
            self.pwrup_cnt = 0;
            self.lp_cnt = 0;
            self.np_cnt = 0;
            return;
        }
        self.state = match self.state {
            State::Pwrdn if l2h => State::Pwrup,
            State::Pwrdn => State::Pwrdn,
            State::Pwrup | State::LpPwrup if self.pwrup_cnt != self.pwrup_time() => {
                self.pwrup_cnt += 1;
                self.state
            }
            State::Pwrup => {
                self.pwrup_cnt = 0;
                if self.en_ctl & ONESHOT_MODE != 0 {
                    State::Onest0
                } else if self.lp_mode() {
                    State::Lp0
                } else {
                    State::Np0
                }
            }
            State::LpPwrup => {
                self.pwrup_cnt = 0;
                State::Lp0
            }
            State::Onest0 | State::Lp0 | State::Np0 => {
                self.sample(0);
                match self.state {
                    State::Onest0 => State::Onest1,
                    State::Lp0 => State::Lp1,
                    _ => State::Np1,
                }
            }
            State::Onest1 | State::Lp1 | State::Np1 => {
                self.sample(1);
                match self.state {
                    State::Onest1 => State::OnestDone,
                    State::Lp1 => State::LpEval,
                    _ => State::NpEval,
                }
            }
            State::OnestDone => {
                // oneshot_done: latch, and flag if ONESHOT_EN.
                self.latch_intr_values();
                if self.intr_ctl & ONESHOT != 0 {
                    self.intr_status |= ONESHOT;
                }
                State::Pwrdn
            }
            State::LpEval => {
                let (_, stay) = self.load_match();
                let thresh = (self.lp_sample_ctl as u8).wrapping_sub(1);
                if !stay {
                    self.lp_cnt = 0;
                    State::LpSlp
                } else if self.lp_cnt < thresh {
                    self.lp_cnt += 1;
                    State::LpSlp
                } else {
                    // lp -> np transition pulse.
                    self.lp_cnt = 0;
                    self.filter_status |= TRANS;
                    if self.intr_ctl & TRANS != 0 {
                        self.intr_status |= TRANS;
                    }
                    State::Np0
                }
            }
            State::LpSlp if self.wakeup_cnt != self.wakeup_time() => {
                self.wakeup_cnt += 1;
                State::LpSlp
            }
            State::LpSlp => {
                self.wakeup_cnt = 0;
                State::LpPwrup
            }
            State::NpEval => {
                let (_, stay) = self.load_match();
                let thresh = (self.sample_ctl as u16).wrapping_sub(1);
                if !stay {
                    self.np_cnt = 0;
                    if self.lp_mode() {
                        State::Lp0
                    } else {
                        State::Np0
                    }
                } else if self.np_cnt < thresh {
                    self.np_cnt += 1;
                    State::Np0
                } else if self.np_cnt == thresh {
                    self.np_cnt = self.np_cnt.wrapping_add(1);
                    State::NpDone
                } else {
                    State::Np0
                }
            }
            State::NpDone => {
                // `adc_ctrl_done`: `match_pulse = done & match` over the
                // values the evaluation used.
                let set = self.matches();
                self.match_pulse(set);
                State::Np0
            }
        };
    }

    fn match_pulse(&mut self, set: u8) {
        let set = u32::from(set);
        self.filter_status |= set;
        if self.en_ctl & ONESHOT_MODE == 0 && self.en_ctl & ADC_ENABLE != 0 {
            self.latch_intr_values();
        }
        self.intr_status |= set & self.intr_ctl & 0xFF;
    }

    fn intr_state(&self) -> u32 {
        u32::from(self.intr_status != 0 || self.intr_test)
    }

    fn irq_level(&self) -> bool {
        self.intr_state() & self.intr_enable & 1 != 0
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            INTR_STATE => self.intr_state(),
            INTR_ENABLE => self.intr_enable,
            ADC_EN_CTL => self.en_ctl,
            ADC_PD_CTL => self.pd_ctl,
            ADC_LP_SAMPLE_CTL => self.lp_sample_ctl,
            ADC_SAMPLE_CTL => self.sample_ctl,
            ADC_FSM_RST => self.fsm_rst,
            ADC_CHN0_FILTER_CTL..=0x40 => {
                self.filter_ctl[0][((offset - ADC_CHN0_FILTER_CTL) / 4) as usize]
            }
            ADC_CHN1_FILTER_CTL..=0x60 => {
                self.filter_ctl[1][((offset - ADC_CHN1_FILTER_CTL) / 4) as usize]
            }
            0x64 | 0x68 => self.chn_val[((offset - ADC_CHN_VAL) / 4) as usize],
            ADC_WAKEUP_CTL => self.wakeup_ctl,
            FILTER_STATUS => self.filter_status,
            ADC_INTR_CTL => self.intr_ctl,
            ADC_INTR_STATUS => self.intr_status,
            ADC_FSM_STATE => self.state as u32,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            INTR_ENABLE => self.intr_enable = value & 1,
            INTR_TEST => self.intr_test |= value & 1 != 0,
            ADC_EN_CTL => self.en_ctl = value & 0x3,
            ADC_PD_CTL => self.pd_ctl = value & PD_CTL_MASK,
            ADC_LP_SAMPLE_CTL => self.lp_sample_ctl = value & 0xFF,
            ADC_SAMPLE_CTL => self.sample_ctl = value & 0xFFFF,
            ADC_FSM_RST => {
                self.fsm_rst = value & 1;
                if self.fsm_rst != 0 {
                    self.fsm_reset();
                }
            }
            ADC_CHN0_FILTER_CTL..=0x40 => {
                self.filter_ctl[0][((offset - ADC_CHN0_FILTER_CTL) / 4) as usize] =
                    value & FILTER_MASK;
            }
            ADC_CHN1_FILTER_CTL..=0x60 => {
                self.filter_ctl[1][((offset - ADC_CHN1_FILTER_CTL) / 4) as usize] =
                    value & FILTER_MASK;
            }
            ADC_WAKEUP_CTL => self.wakeup_ctl = value & 0x1FF,
            FILTER_STATUS => self.filter_status &= !(value & 0x1FF),
            ADC_INTR_CTL => self.intr_ctl = value & 0x3FF,
            ADC_INTR_STATUS => self.intr_status &= !(value & 0x3FF),
            _ => {}
        }
    }
}

impl Peripheral for OpentitanAdcCtrl {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        match self.inputs_mv.get_mut(usize::from(channel)) {
            Some(input) => {
                *input = millivolts;
                true
            }
            None => false,
        }
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        match self.inputs_mv.get_mut(usize::from(channel)) {
            Some(input) => {
                *input = u16::MAX;
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
        Some(self.irq_level())
    }

    fn on_system_reset(&mut self) {
        let inputs = self.inputs_mv;
        *self = Self::new(self.vref_mv);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        if matches!(aligned, FILTER_STATUS | ADC_INTR_STATUS | INTR_TEST) {
            // Write-one-to-clear and pulse registers: only this byte acts.
            self.write_word(aligned, u32::from(value) << shift);
            return Ok(());
        }
        let current = self.read_word(aligned);
        let next = (current & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_word(aligned, next);
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
        self.intr_test = false;
        self.step();
        PeripheralTickResult {
            irq: self.irq_level(),
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

    /// `min <= v <= max` (or outside it with `outside`), enabled.
    fn filter(min: u32, max: u32, outside: bool) -> u32 {
        FILTER_EN | (max << 18) | (u32::from(outside) << 12) | (min << 2)
    }

    fn value(adc: &OpentitanAdcCtrl, ch: u64) -> u32 {
        (adc.read_u32(ADC_CHN_VAL + 4 * ch).unwrap() >> 2) & CODE_MAX
    }

    fn intr_value(adc: &OpentitanAdcCtrl, ch: u64) -> u32 {
        (adc.read_u32(ADC_CHN_VAL + 4 * ch).unwrap() >> 18) & CODE_MAX
    }

    /// Tick until `adc_intr_status & bits`, or panic after `limit`.
    fn run_until(adc: &mut OpentitanAdcCtrl, bits: u32, limit: usize) -> usize {
        for n in 1..=limit {
            adc.tick();
            if adc.read_u32(ADC_INTR_STATUS).unwrap() & bits != 0 {
                return n;
            }
        }
        panic!("adc_intr_status {bits:#x} not set in {limit} ticks");
    }

    /// The DIF oneshot flow: pwrup_time 0, ONESHOT_EN, enable.
    fn oneshot(adc: &mut OpentitanAdcCtrl) -> usize {
        adc.write_u32(ADC_PD_CTL, PD_CTL_RESET & !0xF0).unwrap();
        adc.write_u32(ADC_INTR_CTL, ONESHOT).unwrap();
        adc.write_u32(ADC_EN_CTL, ONESHOT_MODE | ADC_ENABLE)
            .unwrap();
        run_until(adc, ONESHOT, 16)
    }

    #[test]
    fn reset_values_and_masks() {
        let mut adc = OpentitanAdcCtrl::default();
        assert_eq!(adc.read_u32(ADC_PD_CTL).unwrap(), 0x0006_4070);
        assert_eq!(adc.read_u32(ADC_LP_SAMPLE_CTL).unwrap(), 4);
        assert_eq!(adc.read_u32(ADC_SAMPLE_CTL).unwrap(), 155);
        assert_eq!(adc.read_u32(ADC_FSM_STATE).unwrap(), 0, "PWRDN");
        adc.write_u32(ADC_CHN0_FILTER_CTL + 4 * 7, !0).unwrap();
        assert_eq!(adc.read_u32(0x40).unwrap(), FILTER_MASK);
        adc.write_u32(ADC_PD_CTL, !0).unwrap();
        assert_eq!(adc.read_u32(ADC_PD_CTL).unwrap(), PD_CTL_MASK);
        adc.write_u32(ADC_CHN_VAL, !0).unwrap();
        assert_eq!(adc.read_u32(ADC_CHN_VAL).unwrap(), 0, "read-only");
        assert_eq!(adc.adc_channel_count(), Some(2));
        assert!(!adc.set_adc_channel_input(2, 100));
    }

    #[test]
    fn oneshot_samples_both_channels_after_pwrup_and_latches() {
        let mut adc = OpentitanAdcCtrl::default();
        assert!(adc.set_adc_channel_input(0, 1100));
        assert!(adc.set_adc_channel_input(1, 3000));
        adc.write_u32(ADC_INTR_CTL, ONESHOT).unwrap();
        adc.write_u32(INTR_ENABLE, 1).unwrap();
        // Reset pwrup_time 7: PWRUP holds 8 ticks, then 0, 1, DONE.
        adc.write_u32(ADC_EN_CTL, ONESHOT_MODE | ADC_ENABLE)
            .unwrap();
        let ticks = run_until(&mut adc, ONESHOT, 32);
        assert_eq!(ticks, 1 + 8 + 2 + 1);
        assert_eq!(value(&adc, 0), 1100 * 1023 / 2200);
        assert_eq!(value(&adc, 1), 1023, "clamped above 2.2 V");
        assert_eq!(intr_value(&adc, 0), value(&adc, 0), "latched");
        assert!(adc.irq_level());
        assert_eq!(adc.read_u32(INTR_STATE).unwrap(), 1);
        assert_eq!(adc.read_u32(ADC_FSM_STATE).unwrap(), 0, "back to PWRDN");
        // rw1c clears the status and drops the line.
        adc.write_u32(ADC_INTR_STATUS, ONESHOT).unwrap();
        assert!(!adc.irq_level());
        // Still enabled: no new oneshot until a fresh rising edge.
        assert!(adc.set_adc_channel_input(0, 550));
        for _ in 0..32 {
            adc.tick();
        }
        assert_eq!(adc.read_u32(ADC_INTR_STATUS).unwrap(), 0);
        // dif_adc_ctrl_reset (1 then 0, no wait) re-arms the edge.
        adc.write_u32(ADC_FSM_RST, 1).unwrap();
        adc.write_u32(ADC_FSM_RST, 0).unwrap();
        run_until(&mut adc, ONESHOT, 32);
        assert_eq!(value(&adc, 0), 550 * 1023 / 2200);
    }

    #[test]
    fn oneshot_without_oneshot_en_sets_no_status() {
        let mut adc = OpentitanAdcCtrl::default();
        assert!(adc.set_adc_channel_input(0, 1000));
        adc.write_u32(ADC_PD_CTL, 0).unwrap();
        adc.write_u32(ADC_EN_CTL, ONESHOT_MODE | ADC_ENABLE)
            .unwrap();
        for _ in 0..8 {
            adc.tick();
        }
        assert_eq!(value(&adc, 0), 1000 * 1023 / 2200);
        assert_eq!(adc.read_u32(ADC_INTR_STATUS).unwrap(), 0);
        // Undriven channel 1 samples zero.
        assert_eq!(value(&adc, 1), 0);
    }

    #[test]
    fn np_debounce_raises_one_match_after_np_sample_cnt() {
        let mut adc = OpentitanAdcCtrl::default();
        assert!(adc.set_adc_channel_input(0, 500)); // code 232
        assert!(adc.set_adc_channel_input(1, 2000));
        // Filter 2: chn0 inside [200, 300]; chn1 filter disabled.
        adc.write_u32(ADC_CHN0_FILTER_CTL + 8, filter(200, 300, false))
            .unwrap();
        adc.write_u32(ADC_SAMPLE_CTL, 3).unwrap();
        adc.write_u32(ADC_INTR_CTL, 1 << 2).unwrap();
        adc.write_u32(INTR_ENABLE, 1).unwrap();
        adc.write_u32(ADC_PD_CTL, 0).unwrap();
        adc.write_u32(ADC_EN_CTL, ADC_ENABLE).unwrap();
        // 2 ticks to NP_0, 3 ticks per pair, 3 matching pairs, NP_DONE.
        let ticks = run_until(&mut adc, 0xFF, 64);
        assert_eq!(ticks, 2 + 3 * 3 + 1);
        assert_eq!(adc.read_u32(ADC_INTR_STATUS).unwrap(), 1 << 2);
        assert_eq!(adc.read_u32(FILTER_STATUS).unwrap(), 1 << 2);
        assert_eq!(intr_value(&adc, 0), 232);
        assert!(adc.irq_level());
        adc.write_u32(ADC_INTR_STATUS, !0).unwrap();
        adc.write_u32(FILTER_STATUS, !0).unwrap();
        // A stable match does not repeat, and the latch holds.
        assert!(adc.set_adc_channel_input(0, 600)); // code 279, still inside
        for _ in 0..30 {
            adc.tick();
        }
        assert_eq!(adc.read_u32(ADC_INTR_STATUS).unwrap(), 0);
        assert_eq!(value(&adc, 0), 279);
        assert_eq!(intr_value(&adc, 0), 232);
        // Leave and re-enter: a fresh debounce and a new latch.
        assert!(adc.set_adc_channel_input(0, 1000));
        for _ in 0..6 {
            adc.tick();
        }
        assert!(adc.set_adc_channel_input(0, 600));
        run_until(&mut adc, 0xFF, 64);
        assert_eq!(intr_value(&adc, 0), 279);
    }

    #[test]
    fn filter_conditions_and_channel_pairs() {
        let mut adc = OpentitanAdcCtrl::default();
        assert!(adc.set_adc_channel_input(0, 500)); // 232
        assert!(adc.set_adc_channel_input(1, 1500)); // 697
        let cases = [
            // Both enabled: both must hit.
            (filter(200, 300, false), filter(600, 700, false), true),
            (filter(200, 300, false), filter(0, 600, false), false),
            // Outside: hit below min or above max.
            (filter(300, 1023, true), 0, true),
            (filter(0, 1023, true), 0, false),
            // Only chn1 enabled.
            (0, filter(697, 697, false), true),
            // Neither enabled never matches.
            (0, 0, false),
        ];
        for (k, &(c0, c1, _)) in cases.iter().enumerate() {
            adc.write_u32(ADC_CHN0_FILTER_CTL + 4 * k as u64, c0)
                .unwrap();
            adc.write_u32(ADC_CHN1_FILTER_CTL + 4 * k as u64, c1)
                .unwrap();
        }
        adc.write_u32(ADC_SAMPLE_CTL, 1).unwrap();
        adc.write_u32(ADC_PD_CTL, 0).unwrap();
        adc.write_u32(ADC_EN_CTL, ADC_ENABLE).unwrap();
        // PWRUP, NP_0, NP_1, NP_EVAL, NP_DONE.
        for _ in 0..6 {
            adc.tick();
        }
        let expected = cases
            .iter()
            .enumerate()
            .filter(|(_, c)| c.2)
            .fold(0, |m, (k, _)| m | (1 << k));
        assert_eq!(adc.read_u32(FILTER_STATUS).unwrap(), expected);
        assert_eq!(
            adc.read_u32(ADC_INTR_STATUS).unwrap(),
            0,
            "no filter enabled in adc_intr_ctl"
        );
    }

    #[test]
    fn lp_mode_counts_lp_samples_then_transitions_to_np() {
        let mut adc = OpentitanAdcCtrl::default();
        assert!(adc.set_adc_channel_input(0, 500));
        adc.write_u32(ADC_CHN0_FILTER_CTL, filter(200, 300, false))
            .unwrap();
        adc.write_u32(ADC_LP_SAMPLE_CTL, 2).unwrap();
        adc.write_u32(ADC_SAMPLE_CTL, 2).unwrap();
        adc.write_u32(ADC_INTR_CTL, TRANS | 1).unwrap();
        // lp_mode, pwrup_time 1, wakeup_time 4.
        adc.write_u32(ADC_PD_CTL, (4 << 8) | (1 << 4) | LP_MODE)
            .unwrap();
        adc.write_u32(ADC_EN_CTL, ADC_ENABLE).unwrap();
        // PWRDN->PWRUP (1), PWRUP 2, LP pair 3, SLP 5, LP_PWRUP 2, pair 3.
        let ticks = run_until(&mut adc, TRANS, 64);
        assert_eq!(ticks, 1 + 2 + 3 + 5 + 2 + 3);
        assert_eq!(adc.read_u32(FILTER_STATUS).unwrap() & TRANS, TRANS);
        assert_eq!(adc.read_u32(ADC_FSM_STATE).unwrap(), State::Np0 as u32);
        // Two NP pairs debounce the match, then NP_DONE.
        let ticks = run_until(&mut adc, 1, 64);
        assert_eq!(ticks, 3 * 2 + 1);
        // Losing the match in NP with lp_mode goes back to LP_0.
        assert!(adc.set_adc_channel_input(0, 2000));
        for _ in 0..3 {
            adc.tick();
        }
        assert_eq!(adc.read_u32(ADC_FSM_STATE).unwrap(), State::Lp0 as u32);
    }

    #[test]
    fn disable_and_fsm_reset_return_to_pwrdn() {
        let mut adc = OpentitanAdcCtrl::default();
        adc.write_u32(ADC_PD_CTL, 0).unwrap();
        adc.write_u32(ADC_EN_CTL, ADC_ENABLE).unwrap();
        for _ in 0..3 {
            adc.tick();
        }
        assert_ne!(adc.read_u32(ADC_FSM_STATE).unwrap(), 0);
        adc.write_u32(ADC_EN_CTL, 0).unwrap();
        adc.tick();
        assert_eq!(adc.read_u32(ADC_FSM_STATE).unwrap(), 0, "h2l");
        adc.write_u32(ADC_EN_CTL, ADC_ENABLE).unwrap();
        adc.write_u32(ADC_FSM_RST, 1).unwrap();
        for _ in 0..4 {
            adc.tick();
        }
        assert_eq!(adc.read_u32(ADC_FSM_STATE).unwrap(), 0, "held in reset");
        adc.write_u32(ADC_FSM_RST, 0).unwrap();
        adc.tick();
        assert_eq!(adc.read_u32(ADC_FSM_STATE).unwrap(), 1, "PWRUP");
    }

    #[test]
    fn system_reset_keeps_routed_inputs_and_vref() {
        let mut adc = OpentitanAdcCtrl::new(2300);
        assert!(adc.set_adc_channel_input(0, 1150));
        oneshot(&mut adc);
        assert_eq!(value(&adc, 0), 1150 * 1023 / 2300);
        adc.on_system_reset();
        assert_eq!(adc.read_u32(ADC_CHN_VAL).unwrap(), 0);
        assert_eq!(adc.read_u32(ADC_PD_CTL).unwrap(), PD_CTL_RESET);
        assert!(adc.set_adc_reference_voltage_v(2.2));
        oneshot(&mut adc);
        assert_eq!(value(&adc, 0), 1150 * 1023 / 2200);
    }
}
