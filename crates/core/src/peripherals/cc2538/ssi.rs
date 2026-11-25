// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CC2538 synchronous serial interface (`cc2538_pl022_ssi`): SSI0 at 0x4000_8000
//! and SSI1 at 0x4000_9000 (SWRU319C Table 4-1), the block Renode maps as
//! `SPI.CC2538_SSI` in `cc2538.repl`. Interrupts SSI0 = 7, SSI1 = 34 in the
//! regular map (Table 5-2).
//!
//! Register map (SWRU319C §19.9, "SSI Registers"):
//!
//! | Offset | Name | Access | Meaning |
//! | --- | --- | --- | --- |
//! | 0x000 | SSI_CR0 | R/W | SCR \[15:8\], SPH 7, SPO 6, FRF \[5:4\], DSS \[3:0\] (0x3..0xF = 4..16-bit frames) |
//! | 0x004 | SSI_CR1 | R/W | SOD 3, MS 2 (slave), SSE 1 (enable), LBM 0 (loopback) |
//! | 0x008 | SSI_DR | R/W | write queues a TX frame, read pops the RX FIFO (0 when empty) |
//! | 0x00C | SSI_SR | R | BSY 4, RFF 3, RNE 2, TNF 1, TFE 0; 0x3 at reset |
//! | 0x010 | SSI_CPSR | R/W | CPSDVSR \[7:0\] |
//! | 0x014 | SSI_IM | R/W | TXIM 3, RXIM 2, RTIM 1, RORIM 0 |
//! | 0x018 | SSI_RIS | R | TXRIS 3 (TX FIFO ≤ half), RXRIS 2 (RX FIFO ≥ half), RTRIS 1, RORRIS 0; 0x8 at reset |
//! | 0x01C | SSI_MIS | R | RIS & IM |
//! | 0x020 | SSI_ICR | W1C | RTIC 1, RORIC 0 |
//! | 0x024 | SSI_DMACTL | R/W | TXDMAE 1, RXDMAE 0 (stored only) |
//! | 0xFC8 | SSI_CC | R/W | CS 0 (baud clock source, stored only) |
//!
//! Both FIFOs are eight 16-bit frames deep. In master mode with SSE set a
//! frame leaves the TX FIFO at once: with LBM it is shifted straight back
//! into the RX FIFO, otherwise it is clocked through the devices attached by
//! `external_devices:` (frames wider than 8 bits as MSB-first bytes; the
//! first non-zero MISO byte wins, the [`SpiDevice`] v1 convention). With no
//! device MISO reads the idle level 0, as in the RP2040 PL022 and STM32 SPI
//! models: the RX FIFO still gains the frame, because every SCLK edge shifts
//! one bit in. A frame arriving at a full RX FIFO is lost and sets RORRIS.
//! Frames written while SSE is clear wait in the TX FIFO (a ninth is dropped)
//! and leave when SSE is set. The IRQ line is MIS != 0.
//!
//! Differences from Renode `CC2538_SSI`: FIFOs are 8 deep (Renode: 16);
//! frames of 4..16 bits are carried (Renode only 8); an unconnected master
//! still receives the idle frame (Renode receives nothing); LBM loops back
//! (Renode tags it).
//!
//! Not modelled: slave mode (MS = 1 frames stay queued, as nothing clocks
//! them), the RX timeout (RTRIS stays 0), TI SSF and MICROWIRE frame formats
//! (FRF is stored; frames move as Motorola SPI), clock polarity/phase, the
//! Fss pin and devices' chip select (firmware drives CS on a GPIO), the bit
//! rate and transfer time (BSY is set only while frames wait in the TX FIFO),
//! and µDMA requests.

use crate::peripherals::device::SpiDevice;
use crate::{Peripheral, SimResult};
use std::cell::RefCell;
use std::collections::VecDeque;

const CR0: u64 = 0x000;
const CR1: u64 = 0x004;
const DR: u64 = 0x008;
const SR: u64 = 0x00C;
const CPSR: u64 = 0x010;
const IM: u64 = 0x014;
const RIS: u64 = 0x018;
const MIS: u64 = 0x01C;
const ICR: u64 = 0x020;
const DMACTL: u64 = 0x024;
const CC: u64 = 0xFC8;

const CR0_MASK: u32 = 0xFFFF;
const CR0_DSS: u32 = 0xF;
const CR1_MASK: u32 = 0xF;
const CR1_LBM: u32 = 1 << 0;
const CR1_SSE: u32 = 1 << 1;
const CR1_MS: u32 = 1 << 2;

const SR_TFE: u32 = 1 << 0;
const SR_TNF: u32 = 1 << 1;
const SR_RNE: u32 = 1 << 2;
const SR_RFF: u32 = 1 << 3;
const SR_BSY: u32 = 1 << 4;

const INT_ROR: u32 = 1 << 0;
const INT_RX: u32 = 1 << 2;
const INT_TX: u32 = 1 << 3;
const IM_MASK: u32 = 0xF;

const FIFO_DEPTH: usize = 8;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Cc2538Ssi {
    cr0: u32,
    cr1: u32,
    cpsr: u32,
    im: u32,
    dmactl: u32,
    cc: u32,
    overrun: bool,
    tx: VecDeque<u16>,
    rx: RefCell<VecDeque<u16>>,
    #[serde(skip)]
    devices: Vec<Box<dyn SpiDevice>>,
}

impl std::fmt::Debug for Cc2538Ssi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cc2538Ssi")
            .field("cr0", &self.cr0)
            .field("cr1", &self.cr1)
            .field("tx", &self.tx.len())
            .field("rx", &self.rx.borrow().len())
            .field("devices", &self.devices.len())
            .finish_non_exhaustive()
    }
}

impl Default for Cc2538Ssi {
    fn default() -> Self {
        Self::new()
    }
}

impl Cc2538Ssi {
    pub fn new() -> Self {
        Self {
            cr0: 0,
            cr1: 0,
            cpsr: 0,
            im: 0,
            dmactl: 0,
            cc: 0,
            overrun: false,
            tx: VecDeque::new(),
            rx: RefCell::new(VecDeque::new()),
            devices: Vec::new(),
        }
    }

    fn reset_registers(&mut self) {
        let devices = std::mem::take(&mut self.devices);
        *self = Self::new();
        self.devices = devices;
    }

    /// Frame width in bits. DSS values 0..2 are reserved; they are treated as
    /// the smallest legal width, 4 bits.
    fn frame_bits(&self) -> u32 {
        (self.cr0 & CR0_DSS).max(3) + 1
    }

    fn frame_mask(&self) -> u16 {
        (((1u32 << self.frame_bits()) - 1) & 0xFFFF) as u16
    }

    fn raw(&self) -> u32 {
        let rx = self.rx.borrow().len();
        let mut r = 0;
        if self.overrun {
            r |= INT_ROR;
        }
        if rx >= FIFO_DEPTH / 2 {
            r |= INT_RX;
        }
        if self.tx.len() <= FIFO_DEPTH / 2 {
            r |= INT_TX;
        }
        r
    }

    fn irq_active(&self) -> bool {
        self.raw() & self.im != 0
    }

    fn status(&self) -> u32 {
        let rx = self.rx.borrow().len();
        let mut s = 0;
        if self.tx.is_empty() {
            s |= SR_TFE;
        } else {
            s |= SR_BSY;
        }
        if self.tx.len() < FIFO_DEPTH {
            s |= SR_TNF;
        }
        if rx != 0 {
            s |= SR_RNE;
        }
        if rx == FIFO_DEPTH {
            s |= SR_RFF;
        }
        s
    }

    fn shifting(&self) -> bool {
        self.cr1 & CR1_SSE != 0 && self.cr1 & CR1_MS == 0
    }

    /// One frame on the wire; returns the frame shifted in on MISO.
    fn exchange(&mut self, mosi: u16) -> u16 {
        if self.cr1 & CR1_LBM != 0 {
            return mosi;
        }
        let bits = self.frame_bits();
        let bytes = bits.div_ceil(8);
        let mut miso = 0u32;
        for i in (0..bytes).rev() {
            let out = (mosi >> (i * 8)) as u8;
            let mut back = 0u8;
            for dev in &mut self.devices {
                let r = dev.transfer(out);
                if back == 0 {
                    back = r;
                }
            }
            miso = (miso << 8) | u32::from(back);
        }
        miso as u16 & self.frame_mask()
    }

    fn drain_tx(&mut self) {
        if !self.shifting() {
            return;
        }
        while let Some(frame) = self.tx.pop_front() {
            let miso = self.exchange(frame);
            let mut rx = self.rx.borrow_mut();
            if rx.len() < FIFO_DEPTH {
                rx.push_back(miso);
            } else {
                self.overrun = true;
            }
        }
    }

    fn read_reg(&self, reg: u64) -> u32 {
        match reg {
            DR => self.rx.borrow_mut().pop_front().map_or(0, u32::from),
            _ => self.peek(reg),
        }
    }

    /// The register value without the RX-FIFO pop a DR read performs.
    fn peek(&self, reg: u64) -> u32 {
        match reg {
            CR0 => self.cr0,
            CR1 => self.cr1,
            DR => self.rx.borrow().front().copied().map_or(0, u32::from),
            SR => self.status(),
            CPSR => self.cpsr,
            IM => self.im,
            RIS => self.raw(),
            MIS => self.raw() & self.im,
            DMACTL => self.dmactl,
            CC => self.cc,
            _ => 0,
        }
    }

    fn write_reg(&mut self, reg: u64, value: u32) {
        match reg {
            CR0 => self.cr0 = value & CR0_MASK,
            CR1 => {
                self.cr1 = value & CR1_MASK;
                self.drain_tx();
            }
            DR => {
                if self.tx.len() < FIFO_DEPTH {
                    self.tx.push_back(value as u16 & self.frame_mask());
                }
                self.drain_tx();
            }
            // CPSDVSR must be even, 2..254: bit 0 reads back 0.
            CPSR => self.cpsr = value & 0xFE,
            IM => self.im = value & IM_MASK,
            ICR if value & INT_ROR != 0 => self.overrun = false,
            DMACTL => self.dmactl = value & 0x3,
            CC => self.cc = value & 0x1,
            _ => {}
        }
    }
}

impl Peripheral for Cc2538Ssi {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let lane = (offset & 3) * 8;
        let value = if offset & 3 == 0 {
            self.read_reg(reg)
        } else {
            self.peek(reg)
        };
        Ok((value >> lane) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    /// Byte stores merge into the register; DR queues a frame on its low byte
    /// only and ICR acts on the written byte.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let lane = (offset & 3) * 8;
        match reg {
            DR | ICR => {
                if offset & 3 == 0 {
                    self.write_reg(reg, value as u32);
                }
            }
            _ => {
                let merged = (self.peek(reg) & !(0xFF << lane)) | ((value as u32) << lane);
                self.write_reg(reg, merged);
            }
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    /// A DR read can take the RX FIFO below half full and drop RXRIS.
    fn reads_can_deassert_irq(&self) -> bool {
        true
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn spi_attached_devices(&self) -> Option<&Vec<Box<dyn SpiDevice>>> {
        Some(&self.devices)
    }

    fn spi_attached_devices_mut(&mut self) -> Option<&mut Vec<Box<dyn SpiDevice>>> {
        Some(&mut self.devices)
    }

    fn on_system_reset(&mut self) {
        self.reset_registers();
    }

    fn on_node_reset(&mut self) {
        self.reset_registers();
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_active())
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> crate::PeripheralTickResult {
        crate::PeripheralTickResult {
            irq: self.irq_active(),
            ..Default::default()
        }
    }

    /// The line changes only through this block's own MMIO, after which the
    /// bus re-checks this predicate, so an idle controller leaves the walk.
    fn legacy_tick_active(&self) -> bool {
        self.irq_active()
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("Cc2538Ssi snapshot decode: {error}"))
        })?;
        let devices = std::mem::take(&mut self.devices);
        *self = restored;
        self.devices = devices;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Answers each MOSI byte with its complement and logs it.
    struct Inverter(Arc<Mutex<Vec<u8>>>);

    impl SpiDevice for Inverter {
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.0.lock().unwrap().push(mosi);
            !mosi
        }
        fn cs_pin(&self) -> &str {
            "pa7"
        }
    }

    fn master_8bit(ssi: &mut Cc2538Ssi, cr1_extra: u32) {
        ssi.write_u32(CR0, 0x07).unwrap();
        ssi.write_u32(CPSR, 2).unwrap();
        ssi.write_u32(CR1, CR1_SSE | cr1_extra).unwrap();
    }

    #[test]
    fn reset_values() {
        let ssi = Cc2538Ssi::new();
        assert_eq!(ssi.read_u32(SR).unwrap(), SR_TFE | SR_TNF);
        assert_eq!(ssi.read_u32(RIS).unwrap(), INT_TX);
        assert_eq!(ssi.read_u32(MIS).unwrap(), 0);
        assert_eq!(ssi.read_u32(CR0).unwrap(), 0);
        assert_eq!(ssi.read_u32(DR).unwrap(), 0);
        assert!(!ssi.irq_active());
    }

    #[test]
    fn loopback_returns_each_frame() {
        let mut ssi = Cc2538Ssi::new();
        master_8bit(&mut ssi, CR1_LBM);
        ssi.write_u32(DR, 0xA5).unwrap();
        ssi.write_u32(DR, 0x5A).unwrap();
        assert_eq!(ssi.read_u32(SR).unwrap(), SR_TFE | SR_TNF | SR_RNE);
        assert_eq!(ssi.read_u32(DR).unwrap(), 0xA5);
        assert_eq!(ssi.read_u32(DR).unwrap(), 0x5A);
        assert_eq!(ssi.read_u32(SR).unwrap(), SR_TFE | SR_TNF);
    }

    #[test]
    fn frames_are_masked_to_the_data_size() {
        let mut ssi = Cc2538Ssi::new();
        ssi.write_u32(CR0, 0x0F).unwrap(); // 16-bit
        ssi.write_u32(CR1, CR1_SSE | CR1_LBM).unwrap();
        ssi.write_u32(DR, 0x1_BEEF).unwrap();
        assert_eq!(ssi.read_u32(DR).unwrap(), 0xBEEF);
        ssi.write_u32(CR0, 0x03).unwrap(); // 4-bit
        ssi.write_u32(DR, 0xAB).unwrap();
        assert_eq!(ssi.read_u32(DR).unwrap(), 0xB);
    }

    #[test]
    fn devices_answer_msb_first_for_wide_frames() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut ssi = Cc2538Ssi::new();
        ssi.devices.push(Box::new(Inverter(log.clone())));
        ssi.write_u32(CR0, 0x0F).unwrap();
        ssi.write_u32(CR1, CR1_SSE).unwrap();
        ssi.write_u32(DR, 0x1234).unwrap();
        assert_eq!(*log.lock().unwrap(), vec![0x12, 0x34]);
        assert_eq!(ssi.read_u32(DR).unwrap(), 0xEDCB);
    }

    #[test]
    fn unconnected_master_receives_the_idle_frame() {
        let mut ssi = Cc2538Ssi::new();
        master_8bit(&mut ssi, 0);
        ssi.write_u32(DR, 0x9F).unwrap();
        assert_ne!(ssi.read_u32(SR).unwrap() & SR_RNE, 0);
        assert_eq!(ssi.read_u32(DR).unwrap(), 0);
        assert_eq!(ssi.read_u32(SR).unwrap() & SR_RNE, 0);
    }

    #[test]
    fn frames_wait_in_the_tx_fifo_until_enabled() {
        let mut ssi = Cc2538Ssi::new();
        ssi.write_u32(CR0, 0x07).unwrap();
        for b in 0..9u32 {
            ssi.write_u32(DR, b).unwrap();
        }
        // Eight queued, the ninth dropped; FIFO full and above half.
        assert_eq!(ssi.read_u32(SR).unwrap(), SR_BSY);
        assert_eq!(ssi.read_u32(RIS).unwrap(), 0);
        ssi.write_u32(CR1, CR1_SSE | CR1_LBM).unwrap();
        assert_eq!(ssi.read_u32(SR).unwrap(), SR_TFE | SR_TNF | SR_RNE | SR_RFF);
        let got: Vec<u32> = (0..8).map(|_| ssi.read_u32(DR).unwrap()).collect();
        assert_eq!(got, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn slave_mode_does_not_clock_frames() {
        let mut ssi = Cc2538Ssi::new();
        master_8bit(&mut ssi, CR1_MS | CR1_LBM);
        ssi.write_u32(DR, 0x42).unwrap();
        assert_eq!(ssi.read_u32(SR).unwrap() & (SR_TFE | SR_RNE), 0);
    }

    #[test]
    fn overrun_sets_rorris_and_icr_clears_it() {
        let mut ssi = Cc2538Ssi::new();
        master_8bit(&mut ssi, CR1_LBM);
        for b in 0..9u32 {
            ssi.write_u32(DR, b).unwrap();
        }
        assert_eq!(ssi.read_u32(RIS).unwrap(), INT_TX | INT_RX | INT_ROR);
        ssi.write_u32(IM, INT_ROR).unwrap();
        assert!(ssi.irq_active());
        ssi.write_u32(ICR, INT_ROR).unwrap();
        assert_eq!(ssi.read_u32(MIS).unwrap(), 0);
        assert!(!ssi.irq_active());
        // The ninth frame was lost: the eighth is the last in the FIFO.
        let got: Vec<u32> = (0..8).map(|_| ssi.read_u32(DR).unwrap()).collect();
        assert_eq!(got, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn rx_interrupt_follows_the_half_full_level() {
        let mut ssi = Cc2538Ssi::new();
        master_8bit(&mut ssi, CR1_LBM);
        ssi.write_u32(IM, INT_RX).unwrap();
        for b in 0..3u32 {
            ssi.write_u32(DR, b).unwrap();
        }
        assert!(!ssi.irq_active());
        ssi.write_u32(DR, 3).unwrap();
        assert!(ssi.irq_active());
        ssi.read_u32(DR).unwrap();
        assert!(!ssi.irq_active());
    }

    #[test]
    fn cpsr_is_even_and_snapshot_round_trips() {
        let mut ssi = Cc2538Ssi::new();
        ssi.write_u32(CPSR, 0x13).unwrap();
        assert_eq!(ssi.read_u32(CPSR).unwrap(), 0x12);
        master_8bit(&mut ssi, CR1_LBM);
        ssi.write_u32(DR, 0x77).unwrap();
        let snap = ssi.snapshot();
        let mut other = Cc2538Ssi::new();
        other.restore(snap).unwrap();
        assert_eq!(other.read_u32(CR1).unwrap(), CR1_SSE | CR1_LBM);
        assert_eq!(other.read_u32(DR).unwrap(), 0x77);
    }
}
