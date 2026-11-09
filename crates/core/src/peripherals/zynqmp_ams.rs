// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Zynq UltraScale+ MPSoC AMS (analog monitoring system) at `0xFFA5_0000`:
//! the AMS control block, the PS SYSMON at +0x800 and the PL SYSMON at
//! +0xC00, whose VP/VN and VAUX0..15 pads are the device's external analog
//! inputs.
//!
//! Three sources:
//! - Xilinx embeddedsw `XilinxProcessorIPLib/drivers/sysmonpsu` @97f2baf7
//!   (MIT): `xsysmonpsu_hw.h` gives the block base (`XSYSMONPSU_BASEADDR`
//!   0xFFA50000), the control map (`ISR_0` 0x10, `ISR_1` 0x14 with `EOC`
//!   bit 3 and `EOS` bit 4 and mask 0xE000001F, `IMR_0`/`IMR_1` 0x18/0x1C
//!   resetting to 0xFFFFFFFF/0xE000001F, `IER_x` 0x20/0x24, `IDR_x`
//!   0x28/0x2C, `PS_SYSMON_CSTS` 0x40 with `XSYSMONPSU_PS_SYSMON_READY`
//!   0x08010000, `PL_SYSMON_CSTS` 0x44 with `ACESBLE` bit 0) and every
//!   SYSMON register (`VP_VN` 0x0C, `VAUX00..15` 0x40..0x7C, `CFG_REG0`
//!   0x100 with `BU` bit 10 and `MUX_CH` [5:0], `CFG_REG1` 0x104 with
//!   `SEQ_MDE` [15:12], `SEQ_CH0` 0x120 with `VP_VN` bit 11, `SEQ_CH1`
//!   0x124 with `VAUXn` bit n, `SEQ_INPUT_MDE0/1` 0x130/0x134).
//!   `xsysmonpsu.h` gives `XPS_BA_OFFSET` 0x800 and `XPL_BA_OFFSET` 0xC00,
//!   the sequencer modes (0 safe/default, 1 one pass, 2 continuous, 3
//!   single channel; the default mode monitors only the on-chip sensors),
//!   the 10-bit resolution ("the 10 MSB bits of the 16 data read") and
//!   `XSysMonPsu_VpVnRawToVoltage` = raw * 1 V / 65536.
//! - Linux `drivers/iio/adc/xilinx-ams.c` (v6.12, GPL-2.0, used as a
//!   register reference only): the same offsets, `ams_init_device` polling
//!   `PS_CSTS` for `BIT(27) | BIT(16)` and treating `PL_CSTS` = 0 as "no PL
//!   SYSMON", `ams_enable_channel_sequence` (`CFG_REG1.SEQ` = default,
//!   `SEQ_CH0/1/2`, then continuous), `ams_get_pl_scale` (VP/VN and VAUXn
//!   against 1 V, raw over 2^16), and the `IER`/`IDR` write pattern.
//! - Renode `platforms/cpus/zynqmp.repl` @master: the import's only AMS
//!   entry is a `PythonPeripheral` at 0xFFA50040 returning 0x08010000, the
//!   same `PS_CSTS` value; its tags `AMS_CTRL` 0xFFA50000 (0xA0),
//!   `AMS_PS_SYSMON` 0xFFA50800 (0x2B8) and `AMS_PL_SYSMON` 0xFFA50C00
//!   (0x2B4) fix the 4 KB window.
//!
//! Modelled:
//! - The SYSMON registers sit at `DRP address * 4` inside each block, the
//!   same DRP map as the Zynq-7000 XADC (`xilinx_xadc`): status 0x00..0x3F
//!   and 0x80..0xFF are read-only, config 0x40..0x7F store 16 bits.
//! - Each tick the PL sequencer converts what its mode selects into the
//!   latched status registers: continuous (2) every channel enabled in
//!   `SEQ_CH0` (VP/VN) and `SEQ_CH1` (VAUXn); one pass (1) the same once
//!   per `CFG_REG1` write; single channel (3) the `CFG_REG0.MUX_CH`
//!   channel. Safe mode (0) converts only the on-chip sensors, so a routed
//!   pad does not reach its status register until firmware sequences it.
//! - Codes are 10 bits MSB-justified in 16: unipolar `V * 1024 / 1 V`
//!   clamped at 0x3FF; bipolar `V * 1024 / 1 V` clamped to -512..511 in
//!   two's complement, selected by `SEQ_INPUT_MDE0` bit 11 / `MDE1` bit n
//!   in sequencer modes and by `CFG_REG0.BU` in single-channel mode (the
//!   XADC rule, same DRP bits).
//! - `ISR_1.EOC` latches whenever either block converts and `EOS` when a
//!   sequence pass ends (not in single-channel mode); the PS SYSMON always
//!   sequences its on-chip sensors. `ISR_x` are write 1 to clear, `IER_x`
//!   clear and `IDR_x` set `IMR_x` bits, and the IRQ level is
//!   `ISR & !IMR`.
//!
//! Not modelled: the on-chip temperature and supply sensors (read 0), the
//! PS SYSMON's own pads (it has none), alarms and their thresholds,
//! averaging, min/max tracking, acquisition time, conversion time and the
//! `VN`/`VAUXN` pads (held at 0 V). No GIC line is declared for the block:
//! no peripheral in the imported descriptor maps one.

use crate::{Peripheral, PeripheralTickResult, SimResult};

// ── AMS control (xsysmonpsu_hw.h) ───────────────────────────────────────────
const MISC_CTRL: u64 = 0x00;
const ISR_0: u64 = 0x10;
const ISR_1: u64 = 0x14;
const IMR_0: u64 = 0x18;
const IMR_1: u64 = 0x1C;
const IER_0: u64 = 0x20;
const IER_1: u64 = 0x24;
const IDR_0: u64 = 0x28;
const IDR_1: u64 = 0x2C;
const PS_CSTS: u64 = 0x40;
const PL_CSTS: u64 = 0x44;

const ISR_1_MASK: u32 = 0xE000_001F;
const ISR_1_EOC: u32 = 1 << 3;
const ISR_1_EOS: u32 = 1 << 4;
/// `XSYSMONPSU_PS_SYSMON_READY`: `STRTUP_STE` = 8, `STRTUP_DNE`.
const PS_SYSMON_READY: u32 = 0x0801_0000;
const PL_CSTS_ACESBLE: u32 = 1;

const PS_BASE: u64 = 0x800;
const PL_BASE: u64 = 0xC00;
const BLOCK_SIZE: u64 = 0x400;

// ── SYSMON DRP map (xsysmonpsu_hw.h offsets / 4) ────────────────────────────
const DRP_REGS: usize = 0x100;
const CONFIG_BASE: usize = 0x40;
const CONFIG_END: usize = 0x80;
const CFG_REG0: usize = 0x40;
const CFG_REG1: usize = 0x41;
const SEQ_CH0: usize = 0x48;
const SEQ_CH1: usize = 0x49;
const SEQ_INPUT_MDE0: usize = 0x4C;
const SEQ_INPUT_MDE1: usize = 0x4D;

const CFG_REG0_BU: u16 = 1 << 10;
const CFG_REG0_MUX_CH: u16 = 0x3F;
const SEQ_VP_VN: u16 = 1 << 11;

const SEQ_SAFE: u16 = 0;
const SEQ_ONEPASS: u16 = 1;
const SEQ_CONTINUOUS: u16 = 2;
const SEQ_SINGLE_CHANNEL: u16 = 3;

/// `XSM_CH_VPVN` and `XSM_CH_AUX_MIN..=XSM_CH_AUX_MAX` (xsysmonpsu.h).
const CH_VPVN: usize = 0x03;
const CH_AUX0: usize = 0x10;
const CHANNELS: usize = 0x20;

const FULL_SCALE_UV: i64 = 1_000_000;

fn is_external(channel: usize) -> bool {
    channel == CH_VPVN || (CH_AUX0..CHANNELS).contains(&channel)
}

/// One SYSMON's DRP register file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Sysmon {
    regs: Vec<u16>,
    /// A one-pass sequence ran since the last `CFG_REG1` write.
    pass_done: bool,
}

impl Sysmon {
    fn new() -> Self {
        Self {
            regs: vec![0; DRP_REGS],
            pass_done: false,
        }
    }

    fn mode(&self) -> u16 {
        self.regs[CFG_REG1] >> 12 & 0xF
    }

    fn write(&mut self, drp: usize, value: u16) {
        if (CONFIG_BASE..CONFIG_END).contains(&drp) {
            self.regs[drp] = value;
            if drp == CFG_REG1 {
                self.pass_done = false;
            }
        }
    }

    /// The external channels this tick converts, with their bipolar flag,
    /// and whether the pass ends a sequence. `None`: nothing converts.
    fn schedule(&mut self) -> Option<(Vec<(usize, bool)>, bool)> {
        let sequenced = |regs: &[u16]| {
            let mut list = Vec::new();
            if regs[SEQ_CH0] & SEQ_VP_VN != 0 {
                list.push((CH_VPVN, regs[SEQ_INPUT_MDE0] & SEQ_VP_VN != 0));
            }
            for n in 0..16 {
                if regs[SEQ_CH1] >> n & 1 != 0 {
                    list.push((CH_AUX0 + n, regs[SEQ_INPUT_MDE1] >> n & 1 != 0));
                }
            }
            list
        };
        match self.mode() {
            SEQ_SAFE => Some((Vec::new(), true)),
            SEQ_ONEPASS if self.pass_done => None,
            SEQ_ONEPASS => {
                self.pass_done = true;
                Some((sequenced(&self.regs), true))
            }
            SEQ_CONTINUOUS => Some((sequenced(&self.regs), true)),
            SEQ_SINGLE_CHANNEL => {
                let cfg0 = self.regs[CFG_REG0];
                let channel = usize::from(cfg0 & CFG_REG0_MUX_CH);
                Some((vec![(channel, cfg0 & CFG_REG0_BU != 0)], false))
            }
            _ => None,
        }
    }
}

/// Zynq UltraScale+ AMS: control block plus PS and PL SYSMON.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ZynqmpAms {
    misc_ctrl: u32,
    isr: [u32; 2],
    imr: [u32; 2],
    ps: Sysmon,
    pl: Sysmon,
    /// Routed levels per PL channel, millivolts. `u16::MAX` = undriven.
    inputs_mv: [u16; CHANNELS],
}

impl Default for ZynqmpAms {
    fn default() -> Self {
        Self::new()
    }
}

impl ZynqmpAms {
    pub fn new() -> Self {
        Self {
            misc_ctrl: 0,
            isr: [0; 2],
            imr: [u32::MAX, ISR_1_MASK],
            ps: Sysmon::new(),
            pl: Sysmon::new(),
            inputs_mv: [u16::MAX; CHANNELS],
        }
    }

    /// The 16-bit status word: 10-bit code MSB-justified.
    fn code(&self, channel: usize, bipolar: bool) -> u16 {
        let uv = match self.inputs_mv.get(channel) {
            Some(&mv) if mv != u16::MAX && is_external(channel) => i64::from(mv) * 1000,
            _ => 0,
        };
        let code = if bipolar {
            (uv * 1024 / FULL_SCALE_UV).clamp(-512, 511) & 0x3FF
        } else {
            (uv * 1024 / FULL_SCALE_UV).min(0x3FF)
        };
        (code as u16) << 6
    }

    fn sequence(&mut self) {
        let mut status = 0;
        if let Some((_, eos)) = self.ps.schedule() {
            status |= ISR_1_EOC | if eos { ISR_1_EOS } else { 0 };
        }
        if let Some((channels, eos)) = self.pl.schedule() {
            for (channel, bipolar) in channels {
                if is_external(channel) {
                    self.pl.regs[channel] = self.code(channel, bipolar);
                }
            }
            status |= ISR_1_EOC | if eos { ISR_1_EOS } else { 0 };
        }
        self.isr[1] |= status;
    }

    fn block(&self, offset: u64) -> Option<(&Sysmon, usize)> {
        match offset {
            o if (PS_BASE..PS_BASE + BLOCK_SIZE).contains(&o) => {
                Some((&self.ps, ((o - PS_BASE) / 4) as usize))
            }
            o if (PL_BASE..PL_BASE + BLOCK_SIZE).contains(&o) => {
                Some((&self.pl, ((o - PL_BASE) / 4) as usize))
            }
            _ => None,
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        let offset = offset & !3;
        if let Some((block, drp)) = self.block(offset) {
            return u32::from(block.regs[drp]);
        }
        match offset {
            MISC_CTRL => self.misc_ctrl,
            ISR_0 => self.isr[0],
            ISR_1 => self.isr[1],
            IMR_0 => self.imr[0],
            IMR_1 => self.imr[1],
            PS_CSTS => PS_SYSMON_READY,
            PL_CSTS => PL_CSTS_ACESBLE,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let offset = offset & !3;
        if (PS_BASE..PS_BASE + BLOCK_SIZE).contains(&offset) {
            self.ps
                .write(((offset - PS_BASE) / 4) as usize, value as u16);
            return;
        }
        if (PL_BASE..PL_BASE + BLOCK_SIZE).contains(&offset) {
            self.pl
                .write(((offset - PL_BASE) / 4) as usize, value as u16);
            return;
        }
        match offset {
            MISC_CTRL => self.misc_ctrl = value,
            ISR_0 => self.isr[0] &= !value,
            ISR_1 => self.isr[1] &= !(value & ISR_1_MASK),
            IER_0 => self.imr[0] &= !value,
            IER_1 => self.imr[1] &= !(value & ISR_1_MASK),
            IDR_0 => self.imr[0] |= value,
            IDR_1 => self.imr[1] |= value & ISR_1_MASK,
            _ => {}
        }
    }

    /// Registers where a byte write must not merge the other lanes.
    fn lane_write_only(offset: u64) -> bool {
        matches!(offset & !3, ISR_0 | ISR_1 | IER_0 | IER_1 | IDR_0 | IDR_1)
    }

    fn irq_asserted(&self) -> bool {
        self.isr[0] & !self.imr[0] != 0 || self.isr[1] & !self.imr[1] & ISR_1_MASK != 0
    }
}

impl Peripheral for ZynqmpAms {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        let channel = channel as usize;
        if !is_external(channel) {
            return false;
        }
        self.inputs_mv[channel] = millivolts;
        true
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

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_asserted())
    }

    fn on_system_reset(&mut self) {
        let inputs = self.inputs_mv;
        *self = Self::new();
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        let lane = u32::from(value) << shift;
        if Self::lane_write_only(aligned) {
            self.write_word(aligned, lane);
            return Ok(());
        }
        let current = self.read_word(aligned);
        self.write_word(aligned, (current & !(0xFF << shift)) | lane);
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
        self.sequence();
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

    const PL: u64 = PL_BASE;

    fn pl(drp: usize) -> u64 {
        PL + drp as u64 * 4
    }

    /// `ams_enable_channel_sequence` for the PL SYSMON: default mode,
    /// `SEQ_CH0/1`, then continuous.
    fn sequence(ams: &mut ZynqmpAms, ch0: u16, ch1: u16, mode: u16) {
        ams.write_u32(pl(CFG_REG1), 0).unwrap();
        ams.write_u32(pl(SEQ_CH0), u32::from(ch0)).unwrap();
        ams.write_u32(pl(SEQ_CH1), u32::from(ch1)).unwrap();
        ams.write_u32(pl(CFG_REG1), u32::from(mode) << 12).unwrap();
    }

    #[test]
    fn control_reset_values_follow_sysmonpsu() {
        let ams = ZynqmpAms::new();
        assert_eq!(ams.read_u32(PS_CSTS).unwrap(), 0x0801_0000);
        assert_eq!(ams.read_u32(PL_CSTS).unwrap(), 1);
        assert_eq!(ams.read_u32(IMR_0).unwrap(), 0xFFFF_FFFF);
        assert_eq!(ams.read_u32(IMR_1).unwrap(), 0xE000_001F);
        assert_eq!(ams.read_u32(ISR_1).unwrap(), 0);
        // The VP/VN status registers are 0x0C into each block.
        assert_eq!(ams.read_u32(PL + 0x0C).unwrap(), 0);
    }

    #[test]
    fn safe_mode_never_converts_a_routed_pad() {
        let mut ams = ZynqmpAms::new();
        assert!(ams.set_adc_channel_input(0x15, 500));
        for _ in 0..4 {
            ams.tick();
        }
        assert_eq!(ams.read_u32(pl(0x15)).unwrap(), 0, "VAUX5 not sequenced");
        // The on-chip sequence still ends conversions.
        assert_eq!(ams.read_u32(ISR_1).unwrap(), ISR_1_EOC | ISR_1_EOS);
    }

    #[test]
    fn continuous_sequence_converts_enabled_channels_at_ten_bits() {
        let mut ams = ZynqmpAms::new();
        assert!(ams.set_adc_channel_input(0x03, 250));
        assert!(ams.set_adc_channel_input(0x15, 500));
        assert!(ams.set_adc_channel_input(0x16, 1500));
        assert!(ams.set_adc_channel_input(0x17, 700));
        sequence(&mut ams, SEQ_VP_VN, (1 << 5) | (1 << 6), SEQ_CONTINUOUS);
        ams.tick();
        // VP/VN 0.25 V * 1024 = 256; VAUX5 0.5 V = 512; VAUX6 clamps.
        assert_eq!(ams.read_u32(PL + 0x0C).unwrap(), 256 << 6);
        assert_eq!(ams.read_u32(pl(0x15)).unwrap(), 512 << 6);
        assert_eq!(ams.read_u32(pl(0x16)).unwrap(), 0xFFC0);
        // VAUX7 is driven but not in SEQ_CH1.
        assert_eq!(ams.read_u32(pl(0x17)).unwrap(), 0);
        // Held until the next conversion.
        assert!(ams.set_adc_channel_input(0x15, 100));
        assert_eq!(ams.read_u32(pl(0x15)).unwrap(), 512 << 6);
        ams.tick();
        assert_eq!(ams.read_u32(pl(0x15)).unwrap(), (100 * 1024 / 1000) << 6);
    }

    #[test]
    fn one_pass_converts_once_per_cfg_reg1_write() {
        let mut ams = ZynqmpAms::new();
        assert!(ams.set_adc_channel_input(0x15, 400));
        sequence(&mut ams, 0, 1 << 5, SEQ_ONEPASS);
        ams.tick();
        assert_eq!(ams.read_u32(pl(0x15)).unwrap(), (400 * 1024 / 1000) << 6);
        assert!(ams.set_adc_channel_input(0x15, 800));
        ams.tick();
        assert_eq!(ams.read_u32(pl(0x15)).unwrap(), (400 * 1024 / 1000) << 6);
        ams.write_u32(pl(CFG_REG1), u32::from(SEQ_ONEPASS) << 12)
            .unwrap();
        ams.tick();
        assert_eq!(ams.read_u32(pl(0x15)).unwrap(), (800 * 1024 / 1000) << 6);
    }

    #[test]
    fn bipolar_selects_follow_mode() {
        let mut ams = ZynqmpAms::new();
        assert!(ams.set_adc_channel_input(0x18, 300));
        assert!(ams.set_adc_channel_input(0x19, 900));
        // Sequencer: SEQ_INPUT_MDE1 bit n makes VAUXn bipolar.
        ams.write_u32(pl(SEQ_INPUT_MDE1), 1 << 9).unwrap();
        sequence(&mut ams, 0, (1 << 8) | (1 << 9), SEQ_CONTINUOUS);
        ams.tick();
        assert_eq!(ams.read_u32(pl(0x18)).unwrap(), 307 << 6);
        // 0.9 V bipolar clamps at +511.
        assert_eq!(ams.read_u32(pl(0x19)).unwrap(), 511 << 6);
        // Single channel: CFG_REG0.BU with MUX_CH.
        ams.write_u32(pl(CFG_REG0), u32::from(CFG_REG0_BU) | 0x18)
            .unwrap();
        ams.write_u32(pl(CFG_REG1), u32::from(SEQ_SINGLE_CHANNEL) << 12)
            .unwrap();
        assert!(ams.set_adc_channel_input(0x18, 600));
        // The PS SYSMON's default sequence would end passes of its own.
        ams.write_u32(
            PS_BASE + CFG_REG1 as u64 * 4,
            u32::from(SEQ_SINGLE_CHANNEL) << 12,
        )
        .unwrap();
        ams.write_u32(ISR_1, ISR_1_MASK).unwrap();
        ams.tick();
        assert_eq!(ams.read_u32(pl(0x18)).unwrap(), 511 << 6);
        // EOC but no EOS: both sequencers are off.
        assert_eq!(ams.read_u32(ISR_1).unwrap(), ISR_1_EOC);
    }

    #[test]
    fn interrupt_masks_follow_ier_and_idr() {
        let mut ams = ZynqmpAms::new();
        ams.tick();
        assert!(!ams.irq_asserted(), "IMR_1 masks EOC at reset");
        ams.write_u32(IER_1, ISR_1_EOC).unwrap();
        assert_eq!(ams.read_u32(IMR_1).unwrap(), ISR_1_MASK & !ISR_1_EOC);
        assert!(ams.irq_asserted());
        ams.write_u32(ISR_1, ISR_1_EOC | ISR_1_EOS).unwrap();
        assert!(!ams.irq_asserted(), "ISR_1 is write 1 to clear");
        ams.tick();
        assert!(ams.irq_asserted());
        ams.write_u32(IDR_1, ISR_1_EOC).unwrap();
        assert!(!ams.irq_asserted());
    }

    #[test]
    fn status_registers_ignore_writes_and_reset_keeps_routing() {
        let mut ams = ZynqmpAms::new();
        assert!(ams.set_adc_channel_input(0x03, 500));
        assert!(!ams.set_adc_channel_input(0x04, 500), "VREFP is not a pad");
        // ams_init_device writes 0xFFFF to VP_VN.
        ams.write_u32(PL + 0x0C, 0xFFFF).unwrap();
        assert_eq!(ams.read_u32(PL + 0x0C).unwrap(), 0);
        ams.on_system_reset();
        sequence(&mut ams, SEQ_VP_VN, 0, SEQ_CONTINUOUS);
        ams.tick();
        assert_eq!(ams.read_u32(PL + 0x0C).unwrap(), 512 << 6);
    }
}
