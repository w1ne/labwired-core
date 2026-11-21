// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Zynq-7000 PS-XADC interface (`XADCIF`, DEVCFG + 0x100 = `0xF800_7100`)
//! and the XADC behind it.
//!
//! Two sources:
//! - Renode `Peripherals/Analog/Xilinx_XADC.cs` (renode-infrastructure
//!   @cdf0cd3d, MIT): the interface map (`CFG` @0x00, `INT_STS` @0x04,
//!   `INT_MASK` @0x08, `MSTS` @0x0C, `CMDFIFO` @0x10, `RDFIFO` @0x14, `MCTL`
//!   @0x18), the `CFG`/`INT_STS`/`INT_MASK`/`MSTS` reset values, the command
//!   word (`CMD` [29:26]: 0 no-op, 1 DRP read, 2 DRP write; `ADDR` [25:16];
//!   `DATA` [15:0]), the 15-entry data FIFO, the DRP status addresses (equal
//!   to the channel numbers), and the 12-bit result MSB-justified in 16 bits.
//! - Xilinx embeddedsw `XilinxProcessorIPLib/drivers/xadcps` @97f2baf7
//!   (MIT): `xadcps_hw.h` gives every interface bit (`CFG.ENABLE` "enable
//!   access from PS", `CFIFOTH`/`DFIFOTH`, `MSTS` levels, `MCTL.RESET` bit 4
//!   and `FLUSH` bit 0), the DRP map (status 0x00..0x1F, `CFR0..2`
//!   0x40..0x42, `SEQ00..07` 0x48..0x4F, alarm thresholds 0x50..0x5F), and
//!   the `CFR0.BU`, `CFR1.SEQ` and `SEQ04/05` bipolar bits. `xadcps.c`
//!   gives the bring-up (`CFG |= ENABLE`, then `MCTL = 0` "release xadc
//!   from reset") and the pipelined read: `XAdcPs_ReadInternalReg` writes a
//!   read command, discards one `RDFIFO` word, writes a no-op, and takes the
//!   next word as the result.
//!
//! Modelled:
//! - Each command shifts out the result of the previous one into the data
//!   FIFO (the DRP is a JTAG-style shift chain), then latches its own: a
//!   read latches the DRP register, a write stores it and latches 0, a no-op
//!   latches 0. So read-read-no-op returns both registers, in order.
//!   Renode pushes a 0 for every read, which loses the first of two
//!   back-to-back reads; `XAdcPs_ReadInternalReg` sees the same values.
//! - Commands run only with `CFG.ENABLE` set and `MCTL.RESET` clear.
//!   `MCTL` resets with `RESET` set: the driver's bring-up "releases" it.
//!   Renode resets `MCTL` to 0 and ignores both bits.
//! - Status registers convert the input at the DRP read (inputs held from
//!   the last routing update, like a continuously sequencing XADC):
//!   VP/VN (3) and VAUX0..15 (0x10..0x1F) unipolar `V * 4096 / 1 V` clamped to
//!   0xFFF, or bipolar `V * 4096 / 1 V` clamped to -2048..2047 in two's
//!   complement when selected (single-channel mode: `CFR0.BU` on the
//!   `CFR0.CH` channel; sequencer modes: `SEQ04` bit 11 for VP/VN, `SEQ05`
//!   bit n for VAUXn). The supply channels (VCCINT, VCCAUX, VREFP, VREFN,
//!   VCCBRAM, VCCPINT, VCCPAUX, VCCO_DDR) use `V * 4096 / 3 V`. Renode
//!   scales by channel *index* instead of address, so its VCCPINT/VCCPAUX/
//!   VCCO_DDR read against 1 V; this follows the address.
//! - Config and alarm registers 0x40..0x5F store 16 bits and read back.
//! - `INT_STS.DFIFO_GTH` latches when the data FIFO level exceeds `DFIFOTH`,
//!   `CFIFO_LTH` (set at reset) re-latches on each command while `CFIFOTH`
//!   is non-zero, since the command FIFO drains at once; both are write 1 to
//!   clear. The IRQ line is `INT_STS & !INT_MASK` over bits [9:0].
//! - `MCTL.FLUSH` empties the data FIFO and the shift latch. `MCTL.RESET`
//!   also clears the DRP registers. Routed inputs survive both.
//!
//! Not modelled: the die temperature sensor and undriven supplies (read 0,
//! Renode's value with no sample source), alarms and `OT` (never set),
//! min/max tracking (0x20..0x3F read 0), averaging, calibration, conversion
//! time, and the `VN`/`VAUXN` pads (held at 0 V).

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

// ── Interface registers (Renode Xilinx_XADC.cs, xadcps_hw.h) ────────────────
const CFG: u64 = 0x00;
const INT_STS: u64 = 0x04;
const INT_MASK: u64 = 0x08;
const MSTS: u64 = 0x0C;
const CMDFIFO: u64 = 0x10;
const RDFIFO: u64 = 0x14;
const MCTL: u64 = 0x18;

const CFG_RESET: u32 = 0x0000_1114;
/// ENABLE [31], CFIFOTH [23:20], DFIFOTH [19:16], WEDGE, REDGE, TCKRATE
/// [9:8], IGAP [4:0].
const CFG_MASK: u32 = 0x80FF_331F;
const CFG_ENABLE: u32 = 1 << 31;

const INT_CFIFO_LTH: u32 = 1 << 9;
const INT_DFIFO_GTH: u32 = 1 << 8;
const INT_ALL: u32 = 0x3FF;

const MSTS_DFIFOE: u32 = 1 << 8;
const MSTS_DFIFOF: u32 = 1 << 9;
const MSTS_CFIFOE: u32 = 1 << 10;

const MCTL_FLUSH: u32 = 1 << 0;
const MCTL_RESET: u32 = 1 << 4;

const DATA_FIFO_DEPTH: usize = 15;

// ── DRP (xadcps_hw.h) ───────────────────────────────────────────────────────
const CMD_NOP: u32 = 0;
const CMD_READ: u32 = 1;
const CMD_WRITE: u32 = 2;

const CH_VPVN: usize = 0x03;
const CH_AUX0: usize = 0x10;
/// Status registers 0x00..0x1F are the 32 channel results.
const CHANNELS: usize = 32;
const CFR0: usize = 0x40;
const CFR1: usize = 0x41;
const SEQ04: usize = 0x4C;
const SEQ05: usize = 0x4D;
/// Config, sequence and alarm registers 0x40..0x5F.
const CONFIG_BASE: usize = 0x40;
const CONFIG_REGS: usize = 0x20;

const CFR0_BU: u16 = 1 << 10;
const CFR0_CH_MASK: u16 = 0x1F;
const CFR1_SEQ_SHIFT: u16 = 12;
const SEQ_SINGLE_CHANNEL: u16 = 3;
const SEQ_SAFE: u16 = 0;

const UNIPOLAR_UV: i64 = 1_000_000;
const SUPPLY_UV: i64 = 3_000_000;

/// The channels the supply sensor converts against 3 V.
fn is_supply(channel: usize) -> bool {
    matches!(
        channel,
        0x01 | 0x02 | 0x04 | 0x05 | 0x06 | 0x0D | 0x0E | 0x0F
    )
}

/// The channels with an external pad pair.
fn is_external(channel: usize) -> bool {
    channel == CH_VPVN || (CH_AUX0..CHANNELS).contains(&channel)
}

/// Zynq-7000 XADC through the PS-XADC command/data FIFOs.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct XilinxXadc {
    cfg: u32,
    /// `INT_STS` [9:0]; `RDFIFO` reads re-evaluate it.
    int_sts: Cell<u32>,
    int_mask: u32,
    mctl: u32,
    /// Shift-chain result the next command pushes into the data FIFO.
    latch: u16,
    data_fifo: RefCell<VecDeque<u32>>,
    /// Bytes of a `CMDFIFO` word written lane by lane.
    cmd_stage: u32,
    /// The `RDFIFO` word popped by a lane 0 byte read.
    rd_latch: Cell<u32>,
    config: [u16; CONFIG_REGS],
    /// Routed levels per DRP channel, millivolts. `u16::MAX` = undriven.
    inputs_mv: [u16; CHANNELS],
}

impl Default for XilinxXadc {
    fn default() -> Self {
        Self::new()
    }
}

impl XilinxXadc {
    pub fn new() -> Self {
        Self {
            cfg: CFG_RESET,
            int_sts: Cell::new(INT_CFIFO_LTH),
            int_mask: u32::MAX,
            mctl: MCTL_RESET,
            latch: 0,
            data_fifo: RefCell::new(VecDeque::with_capacity(DATA_FIFO_DEPTH)),
            cmd_stage: 0,
            rd_latch: Cell::new(0),
            config: [0; CONFIG_REGS],
            inputs_mv: [u16::MAX; CHANNELS],
        }
    }

    fn reset_registers(&mut self) {
        let inputs = self.inputs_mv;
        *self = Self::new();
        self.inputs_mv = inputs;
    }

    fn flush(&mut self) {
        self.data_fifo.borrow_mut().clear();
        self.latch = 0;
    }

    fn input_uv(&self, channel: usize) -> i64 {
        match self.inputs_mv[channel] {
            u16::MAX => 0,
            mv => i64::from(mv) * 1000,
        }
    }

    fn bipolar(&self, channel: usize) -> bool {
        let cfr0 = self.config[CFR0 - CONFIG_BASE];
        match self.config[CFR1 - CONFIG_BASE] >> CFR1_SEQ_SHIFT & 0xF {
            SEQ_SAFE => false,
            SEQ_SINGLE_CHANNEL => {
                cfr0 & CFR0_BU != 0 && usize::from(cfr0 & CFR0_CH_MASK) == channel
            }
            _ if channel == CH_VPVN => self.config[SEQ04 - CONFIG_BASE] & (1 << 11) != 0,
            _ => self.config[SEQ05 - CONFIG_BASE] & (1 << (channel - CH_AUX0)) != 0,
        }
    }

    /// The 16-bit status register: 12-bit code MSB-justified.
    fn convert(&self, channel: usize) -> u16 {
        let uv = self.input_uv(channel);
        let code = if is_supply(channel) {
            (uv * 4096 / SUPPLY_UV).min(0xFFF)
        } else if !is_external(channel) {
            0
        } else if self.bipolar(channel) {
            (uv * 4096 / UNIPOLAR_UV).clamp(-2048, 2047) & 0xFFF
        } else {
            (uv * 4096 / UNIPOLAR_UV).min(0xFFF)
        };
        (code as u16) << 4
    }

    fn drp_read(&self, address: usize) -> u16 {
        match address {
            a if a < CHANNELS => self.convert(a),
            a if (CONFIG_BASE..CONFIG_BASE + CONFIG_REGS).contains(&a) => {
                self.config[a - CONFIG_BASE]
            }
            _ => 0,
        }
    }

    fn drp_write(&mut self, address: usize, data: u16) {
        if (CONFIG_BASE..CONFIG_BASE + CONFIG_REGS).contains(&address) {
            self.config[address - CONFIG_BASE] = data;
        }
    }

    fn command(&mut self, word: u32) {
        if self.cfg & CFG_ENABLE == 0 || self.mctl & MCTL_RESET != 0 {
            return;
        }
        let address = (word >> 16 & 0x3FF) as usize;
        let data = word as u16;
        let next = match word >> 26 & 0xF {
            CMD_READ => self.drp_read(address),
            CMD_WRITE => {
                self.drp_write(address, data);
                0
            }
            CMD_NOP => 0,
            _ => return,
        };
        let shifted = std::mem::replace(&mut self.latch, next);
        {
            let mut fifo = self.data_fifo.borrow_mut();
            if fifo.len() < DATA_FIFO_DEPTH {
                fifo.push_back(u32::from(shifted));
            }
        }
        let mut status = self.int_sts.get();
        if self.cfg >> 20 & 0xF != 0 {
            status |= INT_CFIFO_LTH;
        }
        self.int_sts.set(status);
        self.update_dfifo_gth();
    }

    fn update_dfifo_gth(&self) {
        let threshold = (self.cfg >> 16 & 0xF) as usize;
        if self.data_fifo.borrow().len() > threshold {
            self.int_sts.set(self.int_sts.get() | INT_DFIFO_GTH);
        }
    }

    fn pop_data(&self) -> u32 {
        let value = self.data_fifo.borrow_mut().pop_front().unwrap_or(0);
        self.update_dfifo_gth();
        value
    }

    fn msts(&self) -> u32 {
        let level = self.data_fifo.borrow().len() as u32;
        let mut value = MSTS_CFIFOE | (level << 12);
        if level == 0 {
            value |= MSTS_DFIFOE;
        }
        if level as usize == DATA_FIFO_DEPTH {
            value |= MSTS_DFIFOF;
        }
        value
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset & !3 {
            CFG => self.cfg,
            INT_STS => self.int_sts.get(),
            INT_MASK => self.int_mask,
            MSTS => self.msts(),
            MCTL => self.mctl,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset & !3 {
            CFG => self.cfg = value & CFG_MASK,
            INT_STS => self.int_sts.set(self.int_sts.get() & !(value & INT_ALL)),
            INT_MASK => self.int_mask = value,
            CMDFIFO => self.command(value),
            MCTL => {
                if value & MCTL_RESET != 0 {
                    self.config = [0; CONFIG_REGS];
                    self.flush();
                }
                if value & MCTL_FLUSH != 0 {
                    self.flush();
                }
                self.mctl = value & MCTL_RESET;
            }
            _ => {}
        }
    }

    fn irq_asserted(&self) -> bool {
        self.int_sts.get() & !self.int_mask & INT_ALL != 0
    }
}

impl Peripheral for XilinxXadc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        let channel = channel as usize;
        if channel >= CHANNELS || !(is_external(channel) || is_supply(channel)) {
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
        self.reset_registers();
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let shift = (offset & 3) * 8;
        if offset & !3 == RDFIFO {
            if offset & 3 == 0 {
                self.rd_latch.set(self.pop_data());
            }
            return Ok((self.rd_latch.get() >> shift) as u8);
        }
        Ok((self.read_word(offset) >> shift) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        if aligned == CMDFIFO {
            // The command runs when its top byte lands.
            self.cmd_stage = (self.cmd_stage & !(0xFF << shift)) | (u32::from(value) << shift);
            if offset & 3 == 3 {
                let word = std::mem::take(&mut self.cmd_stage);
                self.command(word);
            }
            return Ok(());
        }
        if aligned == INT_STS {
            self.write_word(aligned, u32::from(value) << shift);
            return Ok(());
        }
        let current = self.read_word(aligned);
        let next = (current & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_word(aligned, next);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if offset & !3 == RDFIFO {
            return Ok(self.pop_data());
        }
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
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

    const ENABLED: u32 = CFG_RESET | CFG_ENABLE | (0xF << 20) | (0xF << 16);

    fn cmd(op: u32, address: u32, data: u16) -> u32 {
        (op << 26) | (address << 16) | u32::from(data)
    }

    /// xadcps.c bring-up: `CFG |= ENABLE | CFIFOTH | DFIFOTH`, `MCTL = 0`.
    fn ready(inputs: &[(u8, u16)]) -> XilinxXadc {
        let mut adc = XilinxXadc::new();
        for &(channel, mv) in inputs {
            assert!(adc.set_adc_channel_input(channel, mv));
        }
        adc.write_u32(CFG, ENABLED).unwrap();
        adc.write_u32(MCTL, 0).unwrap();
        adc
    }

    /// `XAdcPs_ReadInternalReg`: read, dummy pop, no-op, pop the result.
    fn read_internal(adc: &mut XilinxXadc, address: u32) -> u32 {
        adc.write_u32(CMDFIFO, cmd(CMD_READ, address, 0)).unwrap();
        let dummy = adc.read_u32(RDFIFO).unwrap();
        adc.write_u32(CMDFIFO, dummy).unwrap();
        adc.read_u32(RDFIFO).unwrap()
    }

    /// `XAdcPs_WriteInternalReg`: write, then pop one word.
    fn write_internal(adc: &mut XilinxXadc, address: u32, data: u16) {
        adc.write_u32(CMDFIFO, cmd(CMD_WRITE, address, data))
            .unwrap();
        adc.read_u32(RDFIFO).unwrap();
    }

    #[test]
    fn unipolar_and_supply_transfers_msb_justify_twelve_bits() {
        let mut adc = ready(&[
            (0x03, 500),
            (0x15, 250),
            (0x1F, 1500),
            (0x01, 1000),
            (0x0E, 1800),
        ]);
        // VP/VN: 0.5 V * 4096 / 1 V = 0x800.
        assert_eq!(read_internal(&mut adc, 0x03), 0x800 << 4);
        // VAUX5: 0.25 V = 0x400.
        assert_eq!(read_internal(&mut adc, 0x15), 0x400 << 4);
        // VAUX15 above 1 V clamps at 0xFFF.
        assert_eq!(read_internal(&mut adc, 0x1F), 0xFFF0);
        // VCCINT 1.0 V and VCCPAUX 1.8 V against 3 V (by address, not index).
        assert_eq!(read_internal(&mut adc, 0x01), (1000 * 4096 / 3000) << 4);
        assert_eq!(read_internal(&mut adc, 0x0E), (1800 * 4096 / 3000) << 4);
        // Undriven VAUX0, the temperature sensor, min/max and an unmapped
        // address read 0.
        for address in [0x10, 0x00, 0x20, 0x3F, 0x7F] {
            assert_eq!(read_internal(&mut adc, address), 0, "DRP 0x{address:x}");
        }
        // Only pads and supply channels take a routed level.
        assert!(!adc.set_adc_channel_input(0x00, 100));
        assert!(!adc.set_adc_channel_input(0x08, 100));
        assert!(!adc.set_adc_channel_input(32, 100));
        assert!(adc.clear_adc_channel_input(0x15));
        assert_eq!(read_internal(&mut adc, 0x15), 0);
    }

    #[test]
    fn bipolar_follows_single_channel_bu_and_sequencer_bits() {
        let mut adc = ready(&[(0x03, 300), (0x18, 700)]);
        // Single-channel mode, CFR0.BU on VAUX8 (0x18): 0.7 V clamps at
        // +2047; VP/VN stays unipolar (0.3 V = 1228).
        write_internal(&mut adc, CFR1 as u32, SEQ_SINGLE_CHANNEL << CFR1_SEQ_SHIFT);
        write_internal(&mut adc, CFR0 as u32, CFR0_BU | 0x18);
        assert_eq!(read_internal(&mut adc, 0x18), 0x7FF << 4);
        assert_eq!(read_internal(&mut adc, 0x03), (300 * 4096 / 1000) << 4);
        // Continuous sequence: SEQ04 bit 11 makes VP/VN bipolar, so 0.3 V
        // is still 1228 (inside +-0.5 V); SEQ05 bit 8 off makes VAUX8
        // unipolar again (0.7 V = 2867).
        write_internal(&mut adc, CFR1 as u32, 2 << CFR1_SEQ_SHIFT);
        write_internal(&mut adc, SEQ04 as u32, 1 << 11);
        assert_eq!(read_internal(&mut adc, 0x03), (300 * 4096 / 1000) << 4);
        assert_eq!(read_internal(&mut adc, 0x18), (700 * 4096 / 1000) << 4);
        write_internal(&mut adc, SEQ05 as u32, 1 << 8);
        assert_eq!(read_internal(&mut adc, 0x18), 0x7FF << 4);
        // Safe mode ignores the sequencer bits.
        write_internal(&mut adc, CFR1 as u32, 0);
        assert_eq!(read_internal(&mut adc, 0x18), (700 * 4096 / 1000) << 4);
        // Config registers read back all 16 bits.
        write_internal(&mut adc, 0x42, 0xABCD);
        assert_eq!(read_internal(&mut adc, 0x42), 0xABCD);
    }

    #[test]
    fn commands_shift_out_the_previous_result() {
        let mut adc = ready(&[(0x10, 100), (0x11, 200)]);
        adc.write_u32(CMDFIFO, cmd(CMD_READ, 0x10, 0)).unwrap();
        adc.write_u32(CMDFIFO, cmd(CMD_READ, 0x11, 0)).unwrap();
        adc.write_u32(CMDFIFO, cmd(CMD_NOP, 0, 0)).unwrap();
        assert_eq!(adc.read_u32(MSTS).unwrap() >> 12 & 0xF, 3);
        assert_eq!(
            adc.read_u32(RDFIFO).unwrap(),
            0,
            "first word is the reset latch"
        );
        assert_eq!(adc.read_u32(RDFIFO).unwrap(), (100 * 4096 / 1000) << 4);
        assert_eq!(adc.read_u32(RDFIFO).unwrap(), (200 * 4096 / 1000) << 4);
        assert_eq!(adc.read_u32(MSTS).unwrap(), MSTS_CFIFOE | MSTS_DFIFOE);
        // A write latches 0, so a following no-op pushes 0.
        adc.write_u32(CMDFIFO, cmd(CMD_READ, 0x10, 0)).unwrap();
        adc.write_u32(CMDFIFO, cmd(CMD_WRITE, 0x41, 0x2000))
            .unwrap();
        adc.write_u32(CMDFIFO, cmd(CMD_NOP, 0, 0)).unwrap();
        let words: Vec<u32> = (0..3).map(|_| adc.read_u32(RDFIFO).unwrap()).collect();
        assert_eq!(words, vec![0, (100 * 4096 / 1000) << 4, 0]);
        // Byte-wide command writes run on the top byte; RDFIFO lane 0 pops.
        for (lane, byte) in cmd(CMD_READ, 0x11, 0).to_le_bytes().iter().enumerate() {
            adc.write(CMDFIFO + lane as u64, *byte).unwrap();
        }
        adc.write_u32(CMDFIFO, 0).unwrap();
        adc.read_u32(RDFIFO).unwrap();
        let lo = adc.read(RDFIFO).unwrap();
        let hi = adc.read(RDFIFO + 1).unwrap();
        assert_eq!(u32::from(lo) | u32::from(hi) << 8, (200 * 4096 / 1000) << 4);
        assert_eq!(adc.read_u32(MSTS).unwrap() & MSTS_DFIFOE, MSTS_DFIFOE);
    }

    #[test]
    fn enable_and_mctl_reset_gate_commands() {
        let mut adc = XilinxXadc::new();
        adc.set_adc_channel_input(0x10, 100);
        assert_eq!(adc.read_u32(MCTL).unwrap(), MCTL_RESET, "held in reset");
        // ENABLE clear: dropped.
        adc.write_u32(MCTL, 0).unwrap();
        adc.write_u32(CMDFIFO, cmd(CMD_READ, 0x10, 0)).unwrap();
        assert_eq!(adc.read_u32(MSTS).unwrap() & MSTS_DFIFOE, MSTS_DFIFOE);
        // ENABLE set, RESET set: dropped.
        adc.write_u32(CFG, ENABLED).unwrap();
        adc.write_u32(MCTL, MCTL_RESET).unwrap();
        adc.write_u32(CMDFIFO, cmd(CMD_READ, 0x10, 0)).unwrap();
        assert_eq!(adc.read_u32(MSTS).unwrap() & MSTS_DFIFOE, MSTS_DFIFOE);
        // Released: served.
        adc.write_u32(MCTL, 0).unwrap();
        assert_eq!(read_internal(&mut adc, 0x10), (100 * 4096 / 1000) << 4);
        // RESET clears the DRP config, keeps the routed input.
        write_internal(&mut adc, 0x40, 0x1234);
        adc.write_u32(MCTL, MCTL_RESET).unwrap();
        adc.write_u32(MCTL, 0).unwrap();
        assert_eq!(read_internal(&mut adc, 0x40), 0);
        assert_eq!(read_internal(&mut adc, 0x10), (100 * 4096 / 1000) << 4);
        // FLUSH empties the FIFO and the latch.
        adc.write_u32(CMDFIFO, cmd(CMD_READ, 0x10, 0)).unwrap();
        adc.write_u32(CMDFIFO, cmd(CMD_READ, 0x10, 0)).unwrap();
        adc.write_u32(MCTL, MCTL_FLUSH).unwrap();
        assert_eq!(adc.read_u32(MSTS).unwrap() & MSTS_DFIFOE, MSTS_DFIFOE);
        adc.write_u32(CMDFIFO, cmd(CMD_NOP, 0, 0)).unwrap();
        assert_eq!(adc.read_u32(RDFIFO).unwrap(), 0);
        // System reset restores every reset value, inputs kept.
        adc.on_system_reset();
        assert_eq!(adc.read_u32(CFG).unwrap(), CFG_RESET);
        assert_eq!(adc.read_u32(INT_STS).unwrap(), INT_CFIFO_LTH);
        assert_eq!(adc.read_u32(INT_MASK).unwrap(), u32::MAX);
        assert_eq!(adc.read_u32(MSTS).unwrap(), 0x500);
        adc.write_u32(CFG, ENABLED).unwrap();
        adc.write_u32(MCTL, 0).unwrap();
        assert_eq!(read_internal(&mut adc, 0x10), (100 * 4096 / 1000) << 4);
    }

    #[test]
    fn fifo_threshold_status_and_irq() {
        // DFIFOTH = 1: GTH latches at two words.
        let mut adc = ready(&[]);
        adc.write_u32(CFG, CFG_ENABLE | (1 << 16)).unwrap();
        adc.write_u32(INT_STS, INT_ALL).unwrap();
        assert_eq!(adc.read_u32(INT_STS).unwrap(), 0);
        adc.write_u32(CMDFIFO, 0).unwrap();
        assert_eq!(
            adc.read_u32(INT_STS).unwrap(),
            0,
            "CFIFOTH 0 never re-latches LTH"
        );
        adc.write_u32(CMDFIFO, 0).unwrap();
        assert_eq!(adc.read_u32(INT_STS).unwrap(), INT_DFIFO_GTH);
        assert!(!adc.irq_asserted(), "reset mask hides everything");
        adc.write_u32(INT_MASK, !INT_DFIFO_GTH).unwrap();
        assert!(adc.irq_asserted());
        // Latched: draining does not clear it, W1C does.
        adc.read_u32(RDFIFO).unwrap();
        adc.read_u32(RDFIFO).unwrap();
        assert!(adc.irq_asserted());
        adc.write(INT_STS + 1, 0x01).unwrap();
        assert!(!adc.irq_asserted());
        // CFIFOTH set: each command re-latches LTH.
        adc.write_u32(CFG, CFG_ENABLE | (1 << 20) | (0xF << 16))
            .unwrap();
        adc.write_u32(CMDFIFO, 0).unwrap();
        assert_eq!(adc.read_u32(INT_STS).unwrap(), INT_CFIFO_LTH);
        // The data FIFO holds 15 words and reports full.
        for _ in 0..20 {
            adc.write_u32(CMDFIFO, 0).unwrap();
        }
        let msts = adc.read_u32(MSTS).unwrap();
        assert_eq!(msts >> 12 & 0xF, 15);
        assert_ne!(msts & MSTS_DFIFOF, 0);
        // CFG keeps only its defined bits.
        adc.write_u32(CFG, u32::MAX).unwrap();
        assert_eq!(adc.read_u32(CFG).unwrap(), CFG_MASK);
    }
}
