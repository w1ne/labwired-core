// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! PolarFire SoC MSS SPI (`mpfs_spi`) — master mode of the CoreSPI-family
//! controller Renode maps as `SPI.MPFS_SPI` at 0x20108000 (SPI0, PLIC 54)
//! and 0x20109000 (SPI1, PLIC 55) in `polarfire-soc.repl`.
//!
//! Register map (32-bit registers; Renode `MPFS_SPI`, Microchip
//! `mss_spi_regs.h`):
//!
//! | Offset | Name | Access | Meaning |
//! | --- | --- | --- | --- |
//! | 0x00 | CONTROL | R/W | bit0 ENABLE, bit1 MASTER, bit4..7 INTRXDATA/INTTXDATA/INTRCVOVFLOW/INTTXTURUN, bits8..23 FRAMECNT, bit29 BIGFIFO, bit31 RESET; 0x80000102 at reset |
//! | 0x04 | FRAMESIZE | R/W | bits0..5, 4 at reset |
//! | 0x08 | STATUS | R | bit0 TXDATASENT, bit1 RXDATARCVD, bit2 RXOVERFLOW, bit3 TXUNDERRUN, bits4..7 RX FIFO full/full-next/empty/empty-next, bits8..11 TX FIFO likewise, bit13 SSEL pin (high = idle) |
//! | 0x0C | INT_CLEAR | W1C | raw bits 0..5 |
//! | 0x10 | RX_DATA | R | pops the RX FIFO, clears RXDATARCVD |
//! | 0x14 | TX_DATA | W | starts one frame (master, ENABLE) |
//! | 0x18 | CLK_GEN | R/W | stored only |
//! | 0x1C | SLAVE_SELECT | R/W | bits0..7, one per SS line |
//! | 0x20 | INTMASK | R | raw & enable |
//! | 0x24 | INTRAW | R | bit0 TXDONE, bit1 RXDONE, bit2 RXOVERFLOW, bit3 TXUNDERRUN, bit4 CMDINT, bit5 SSEND |
//! | 0x28 | CONTROL2 | R/W | bit2 DISFRMCNT, bit4 INTEN_CMD, bit5 INTEN_SSEND |
//! | 0x2C | COMMAND | W | bit2 RXFIFORST, bit3 TXFIFORST (no-op), bit4 CLRFRAMECNT; reads 0 |
//! | 0x30 / 0x34 | PKTSIZE / CMDSIZE | R/W | stored only |
//! | 0x38 | HWSTATUS | R | bit0 RX FIFO not empty, bit1 TX FIFO empty |
//! | 0x3C | STAT8 | R | bit1 DONE, bit2 RXEMPTY, bit3 TXFULL, bit4 RXOVFLOW, bit5 TXUNDERRUN, bit6 SSEL |
//! | 0x40..0x4C | CTRL0..CTRL3 | R/W | byte aliases of CONTROL |
//!
//! A TX_DATA write clocks the frame through the attached devices at once and
//! queues the MISO frame (RXOVERFLOW once the FIFO holds its depth: 4 frames,
//! or with BIGFIFO 32/16/8 for frames of up to 8/16/32 bits). Frames wider
//! than 8 bits go out MSB-first as whole bytes. When the frames received
//! since the last SLAVE_SELECT/FRAMECNT write reach FRAMECNT (and DISFRMCNT is
//! clear), TXDONE/RXDONE and TXDATASENT/RXDATARCVD set, which is what
//! `MSS_SPI_transfer_frame` (FRAMECNT 1, then poll RXDATARCVD) waits for.
//! The interrupt line is INTMASK != 0.
//!
//! Devices are selected by SLAVE_SELECT: a device whose `cs_pin` is `N` or
//! `ssN` follows bit N, any other device follows "any bit set". Clearing
//! SLAVE_SELECT releases them. With several devices selected the first
//! non-zero MISO byte wins (the [`SpiDevice`] v1 convention); with none MISO
//! reads 0.
//!
//! Differences from Renode `MPFS_SPI`: INTMASK pairs TXDONE with INTTXDATA
//! and RXDONE with INTRXDATA (Renode crosses them); the RX FIFO depth counts
//! frames, and is 4 rather than 0 before BIGFIFO is first written; reaching
//! FRAMECNT does not release the slave, SLAVE_SELECT does.
//!
//! Not modelled: slave mode (TX_DATA writes outside master mode are
//! ignored), the TX FIFO (master frames complete at once, so it always reads
//! empty), AUTOFILL/AUTOEMPTY/AUTOSTALL/TXNOW, SPI modes and clock polarity,
//! the clock divider and transfer timing.

use crate::peripherals::device::SpiDevice;
use crate::{Peripheral, SimResult};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

const CONTROL: u64 = 0x00;
const FRAMESIZE: u64 = 0x04;
const STATUS: u64 = 0x08;
const INT_CLEAR: u64 = 0x0C;
const RX_DATA: u64 = 0x10;
const TX_DATA: u64 = 0x14;
const CLK_GEN: u64 = 0x18;
const SLAVE_SELECT: u64 = 0x1C;
const INTMASK: u64 = 0x20;
const INTRAW: u64 = 0x24;
const CONTROL2: u64 = 0x28;
const COMMAND: u64 = 0x2C;
const PKTSIZE: u64 = 0x30;
const CMDSIZE: u64 = 0x34;
const HWSTATUS: u64 = 0x38;
const STAT8: u64 = 0x3C;
const CTRL0: u64 = 0x40;
const CTRL3: u64 = 0x4C;

const CONTROL_RESET: u32 = 0x8000_0102;
const ENABLE: u32 = 1 << 0;
const MASTER: u32 = 1 << 1;
const INTRXDATA: u32 = 1 << 4;
const INTTXDATA: u32 = 1 << 5;
const INTRCVOVFLOW: u32 = 1 << 6;
const INTTXTURUN: u32 = 1 << 7;
const FRAMECNT_SHIFT: u32 = 8;
const FRAMECNT_MASK: u32 = 0xFFFF << FRAMECNT_SHIFT;
const BIGFIFO: u32 = 1 << 29;
const RESET: u32 = 1 << 31;

const TXDONE: u8 = 1 << 0;
const RXDONE: u8 = 1 << 1;
const RXOVERFLOW: u8 = 1 << 2;
const TXUNDERRUN: u8 = 1 << 3;
const CMDINT: u8 = 1 << 4;
const SSEND: u8 = 1 << 5;

const DISFRMCNT: u32 = 1 << 2;
const INTEN_CMD: u32 = 1 << 4;
const INTEN_SSEND: u32 = 1 << 5;

const RXFIFORST: u32 = 1 << 2;
const CLRFRAMECNT: u32 = 1 << 4;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct MpfsSpi {
    control: u32,
    framesize: u32,
    clk_gen: u32,
    slave_select: u32,
    control2: u32,
    pktsize: u32,
    cmdsize: u32,
    raw: u8,
    tx_sent: bool,
    rx_received: Cell<bool>,
    rx: RefCell<VecDeque<u32>>,
    /// The frame the last RX_DATA byte-0 read popped, for its upper bytes.
    rx_latch: Cell<u32>,
    frames: u32,
    /// Per attached device: currently selected.
    #[serde(skip)]
    selected: Vec<bool>,
    #[serde(skip)]
    devices: Vec<Box<dyn SpiDevice>>,
}

impl std::fmt::Debug for MpfsSpi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MpfsSpi")
            .field("control", &self.control)
            .field("slave_select", &self.slave_select)
            .field("raw", &self.raw)
            .field("rx", &self.rx.borrow().len())
            .field("devices", &self.devices.len())
            .finish_non_exhaustive()
    }
}

impl Default for MpfsSpi {
    fn default() -> Self {
        Self::new()
    }
}

impl MpfsSpi {
    pub fn new() -> Self {
        Self {
            control: CONTROL_RESET,
            framesize: 4,
            clk_gen: 0,
            slave_select: 0,
            control2: 0,
            pktsize: 0,
            cmdsize: 0,
            raw: 0,
            tx_sent: false,
            rx_received: Cell::new(false),
            rx: RefCell::new(VecDeque::new()),
            rx_latch: Cell::new(0),
            frames: 0,
            selected: Vec::new(),
            devices: Vec::new(),
        }
    }

    fn reset_registers(&mut self) {
        self.write_slave_select(0);
        let devices = std::mem::take(&mut self.devices);
        *self = Self::new();
        self.devices = devices;
    }

    fn frame_bits(&self) -> u32 {
        self.framesize.clamp(1, 32)
    }

    fn fifo_depth(&self) -> usize {
        if self.control & BIGFIFO == 0 {
            4
        } else {
            match self.frame_bits() {
                1..=8 => 32,
                9..=16 => 16,
                _ => 8,
            }
        }
    }

    fn frame_count_limit(&self) -> u32 {
        (self.control & FRAMECNT_MASK) >> FRAMECNT_SHIFT
    }

    fn masked(&self) -> u8 {
        let enabled = [
            (TXDONE, self.control & INTTXDATA != 0),
            (RXDONE, self.control & INTRXDATA != 0),
            (RXOVERFLOW, self.control & INTRCVOVFLOW != 0),
            (TXUNDERRUN, self.control & INTTXTURUN != 0),
            (CMDINT, self.control2 & INTEN_CMD != 0),
            (SSEND, self.control2 & INTEN_SSEND != 0),
        ];
        enabled
            .into_iter()
            .filter(|(_, on)| *on)
            .fold(0, |acc, (bit, _)| acc | bit)
            & self.raw
    }

    fn irq_active(&self) -> bool {
        self.masked() != 0
    }

    fn status(&self) -> u32 {
        let rx = self.rx.borrow().len();
        let depth = self.fifo_depth();
        let mut s = 0u32;
        s |= self.tx_sent as u32;
        s |= (self.rx_received.get() as u32) << 1;
        s |= ((self.raw & RXOVERFLOW != 0) as u32) << 2;
        s |= ((self.raw & TXUNDERRUN != 0) as u32) << 3;
        s |= ((rx == depth) as u32) << 4;
        s |= ((rx + 1 == depth) as u32) << 5;
        s |= ((rx == 0) as u32) << 6;
        s |= ((rx == 1) as u32) << 7;
        // TX FIFO: master frames leave at once, so it is always empty.
        s |= 1 << 10;
        // SSEL follows the active-low SS pin: high while nothing is selected
        // (Renode's reset value 0x2440).
        s |= ((self.slave_select == 0) as u32) << 13;
        s
    }

    fn stat8(&self) -> u32 {
        let mut s = 0u32;
        s |= ((self.tx_sent && self.rx_received.get()) as u32) << 1;
        s |= ((self.rx.borrow().is_empty()) as u32) << 2;
        s |= ((self.raw & RXOVERFLOW != 0) as u32) << 4;
        s |= ((self.raw & TXUNDERRUN != 0) as u32) << 5;
        s |= ((self.slave_select != 0) as u32) << 6;
        s
    }

    fn device_wants(cs_pin: &str, slave_select: u32) -> bool {
        let line = cs_pin.strip_prefix("ss").unwrap_or(cs_pin);
        match line.parse::<u32>() {
            Ok(n) if n < 8 => slave_select & (1 << n) != 0,
            _ => slave_select != 0,
        }
    }

    fn write_slave_select(&mut self, value: u32) {
        let value = value & 0xFF;
        if value != 0 {
            self.frames = 0;
        }
        self.slave_select = value;
        self.selected.resize(self.devices.len(), false);
        for (dev, selected) in self.devices.iter_mut().zip(&mut self.selected) {
            let next = Self::device_wants(dev.cs_pin(), value);
            if next != *selected {
                if next {
                    dev.cs_select();
                } else {
                    dev.cs_release();
                }
            }
            *selected = next;
        }
    }

    fn write_control(&mut self, value: u32) {
        if value & RESET != 0 && self.control & RESET == 0 {
            self.reset_registers();
            return;
        }
        let was_enabled = self.control & ENABLE != 0;
        let old_count = self.control & FRAMECNT_MASK;
        self.control = value;
        if value & FRAMECNT_MASK != old_count {
            self.frames = 0;
        }
        if was_enabled && value & ENABLE == 0 {
            self.control &= !FRAMECNT_MASK;
        }
    }

    fn transmit(&mut self, value: u32) {
        if self.control & (ENABLE | MASTER) != ENABLE | MASTER || self.control & RESET != 0 {
            return;
        }
        self.tx_sent = false;
        let bits = self.frame_bits();
        let mask = if bits == 32 {
            u32::MAX
        } else {
            (1 << bits) - 1
        };
        let bytes = bits.div_ceil(8);
        self.selected.resize(self.devices.len(), false);
        let mut miso = 0u32;
        for i in (0..bytes).rev() {
            let mosi = (value >> (i * 8)) as u8;
            let mut byte = 0u8;
            for (dev, selected) in self.devices.iter_mut().zip(&self.selected) {
                if *selected {
                    let out = dev.transfer(mosi);
                    if byte == 0 {
                        byte = out;
                    }
                }
            }
            miso = (miso << 8) | byte as u32;
        }
        self.receive(miso & mask);
    }

    fn receive(&mut self, frame: u32) {
        let mut rx = self.rx.borrow_mut();
        if rx.len() >= self.fifo_depth() {
            self.raw |= RXOVERFLOW;
            return;
        }
        rx.push_back(frame);
        drop(rx);
        self.frames += 1;
        if self.control2 & DISFRMCNT == 0 && self.frames == self.frame_count_limit() {
            self.tx_sent = true;
            self.rx_received.set(true);
            self.raw |= TXDONE | RXDONE;
        }
    }

    fn pop_rx(&self) -> u32 {
        self.rx_received.set(false);
        let frame = self.rx.borrow_mut().pop_front().unwrap_or(0);
        self.rx_latch.set(frame);
        frame
    }

    fn command(&mut self, value: u32) {
        if value & RXFIFORST != 0 {
            self.rx.borrow_mut().clear();
        }
        // TXFIFORST needs no action: the modelled TX FIFO is always empty.
        if value & CLRFRAMECNT != 0 {
            self.control &= !FRAMECNT_MASK;
            self.frames = 0;
        }
    }

    /// Register value without side effects (RX_DATA reads its latch).
    fn peek(&self, reg: u64) -> u32 {
        match reg {
            CONTROL => self.control,
            FRAMESIZE => self.framesize,
            STATUS => self.status(),
            RX_DATA => self.rx_latch.get(),
            CLK_GEN => self.clk_gen,
            SLAVE_SELECT => self.slave_select,
            INTMASK => self.masked() as u32,
            INTRAW => self.raw as u32,
            CONTROL2 => self.control2,
            PKTSIZE => self.pktsize,
            CMDSIZE => self.cmdsize,
            HWSTATUS => {
                (!self.rx.borrow().is_empty()) as u32 | 1 << 1 // TX FIFO empty
            }
            STAT8 => self.stat8(),
            CTRL0..=CTRL3 if reg % 4 == 0 => (self.control >> ((reg - CTRL0) * 2)) & 0xFF,
            _ => 0,
        }
    }

    fn read_reg(&self, reg: u64) -> u32 {
        if reg == RX_DATA {
            self.pop_rx()
        } else {
            self.peek(reg)
        }
    }

    fn write_reg(&mut self, reg: u64, value: u32) {
        match reg {
            CONTROL => self.write_control(value),
            FRAMESIZE => self.framesize = value & 0x3F,
            INT_CLEAR => self.raw &= !(value as u8 & 0x3F),
            TX_DATA => self.transmit(value),
            CLK_GEN => self.clk_gen = value & 0xFF,
            SLAVE_SELECT => self.write_slave_select(value),
            CONTROL2 => self.control2 = value & 0x3F,
            COMMAND => self.command(value),
            PKTSIZE => self.pktsize = value & 0xFF,
            CMDSIZE => self.cmdsize = value & 0x3F,
            CTRL0..=CTRL3 if reg % 4 == 0 => {
                let shift = (reg - CTRL0) * 2;
                let merged = (self.control & !(0xFF << shift)) | ((value & 0xFF) << shift);
                self.write_control(merged);
            }
            _ => {}
        }
    }
}

impl Peripheral for MpfsSpi {
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

    /// Byte stores merge into the register; TX_DATA starts a frame on its
    /// low byte only, and W1C/command registers act on the written byte.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let lane = (offset & 3) * 8;
        match reg {
            TX_DATA | INT_CLEAR | COMMAND => {
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

    fn reads_can_deassert_irq(&self) -> bool {
        false // RX_DATA pops data but leaves INTRAW to INT_CLEAR.
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
            crate::SimulationError::NotImplemented(format!("MpfsSpi snapshot decode: {error}"))
        })?;
        let devices = std::mem::take(&mut self.devices);
        let selected = std::mem::take(&mut self.selected);
        *self = restored;
        self.devices = devices;
        self.selected = selected;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Echoes each MOSI byte plus one and logs CS edges.
    struct Echo {
        cs: &'static str,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl SpiDevice for Echo {
        fn cs_select(&mut self) {
            self.events.lock().unwrap().push(format!("{} sel", self.cs));
        }
        fn cs_release(&mut self) {
            self.events.lock().unwrap().push(format!("{} rel", self.cs));
        }
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.events
                .lock()
                .unwrap()
                .push(format!("{} {mosi:02x}", self.cs));
            mosi.wrapping_add(1)
        }
        fn cs_pin(&self) -> &str {
            self.cs
        }
    }

    fn with(cs: &[&'static str]) -> (MpfsSpi, Arc<Mutex<Vec<String>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut spi = MpfsSpi::new();
        for cs in cs {
            spi.devices.push(Box::new(Echo {
                cs,
                events: events.clone(),
            }));
        }
        (spi, events)
    }

    fn master(spi: &mut MpfsSpi, framecnt: u32) {
        spi.write_u32(CONTROL, CONTROL_RESET & !RESET).unwrap();
        spi.write_u32(FRAMESIZE, 8).unwrap();
        spi.write_u32(CONTROL, ENABLE | MASTER | (framecnt << FRAMECNT_SHIFT))
            .unwrap();
    }

    #[test]
    fn reset_values() {
        let spi = MpfsSpi::new();
        assert_eq!(spi.read_u32(CONTROL).unwrap(), 0x8000_0102);
        assert_eq!(spi.read_u32(FRAMESIZE).unwrap(), 4);
        assert_eq!(spi.read_u32(STATUS).unwrap(), 0x2440);
        assert_eq!(spi.read_u32(INTRAW).unwrap(), 0);
        assert!(!spi.irq_active());
    }

    #[test]
    fn single_frame_transfer_completes_frame_count() {
        let (mut spi, events) = with(&[""]);
        master(&mut spi, 1);
        spi.write_u32(SLAVE_SELECT, 1).unwrap();
        spi.write_u32(TX_DATA, 0x9F).unwrap();
        let status = spi.read_u32(STATUS).unwrap();
        assert_eq!(status & 0b11, 0b11, "TXDATASENT and RXDATARCVD");
        assert_eq!(status & (1 << 6), 0, "RX FIFO not empty");
        assert_eq!(spi.read_u32(INTRAW).unwrap(), (TXDONE | RXDONE) as u32);
        assert_eq!(spi.read_u32(STAT8).unwrap() & 0b10, 0b10, "DONE");
        assert_eq!(spi.read_u32(RX_DATA).unwrap(), 0xA0);
        assert_eq!(
            spi.read_u32(STATUS).unwrap() & 0b10,
            0,
            "RXDATARCVD cleared"
        );
        spi.write_u32(SLAVE_SELECT, 0).unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec![" sel".to_string(), " 9f".into(), " rel".into()]
        );
    }

    #[test]
    fn frame_count_reaches_limit_only_on_last_frame() {
        let (mut spi, _) = with(&[""]);
        master(&mut spi, 3);
        spi.write_u32(SLAVE_SELECT, 1).unwrap();
        spi.write_u32(TX_DATA, 1).unwrap();
        spi.write_u32(TX_DATA, 2).unwrap();
        assert_eq!(spi.read_u32(INTRAW).unwrap(), 0);
        spi.write_u32(TX_DATA, 3).unwrap();
        assert_eq!(spi.read_u32(INTRAW).unwrap(), (TXDONE | RXDONE) as u32);
        let got: Vec<u32> = (0..3).map(|_| spi.read_u32(RX_DATA).unwrap()).collect();
        assert_eq!(got, vec![2, 3, 4]);
    }

    #[test]
    fn rx_overflow_after_fifo_depth() {
        let (mut spi, _) = with(&[""]);
        master(&mut spi, 0);
        spi.write_u32(SLAVE_SELECT, 1).unwrap();
        for i in 0..4 {
            spi.write_u32(TX_DATA, i).unwrap();
        }
        assert_eq!(
            spi.read_u32(STATUS).unwrap() & (1 << 4),
            1 << 4,
            "RXFIFOFULL"
        );
        assert_eq!(spi.read_u32(INTRAW).unwrap() & RXOVERFLOW as u32, 0);
        spi.write_u32(TX_DATA, 9).unwrap();
        assert_eq!(
            spi.read_u32(INTRAW).unwrap() & RXOVERFLOW as u32,
            RXOVERFLOW as u32
        );
        spi.write_u32(INT_CLEAR, RXOVERFLOW as u32).unwrap();
        assert_eq!(spi.read_u32(INTRAW).unwrap(), 0);
        spi.write_u32(COMMAND, RXFIFORST).unwrap();
        assert_eq!(
            spi.read_u32(STATUS).unwrap() & (1 << 6),
            1 << 6,
            "RXFIFOEMPTY"
        );
    }

    #[test]
    fn irq_pairs_done_with_its_own_enable() {
        let (mut spi, _) = with(&[""]);
        master(&mut spi, 1);
        spi.write_u32(SLAVE_SELECT, 1).unwrap();
        spi.write_u32(TX_DATA, 0).unwrap();
        assert!(!spi.irq_active());
        spi.write(CTRL0, (ENABLE | MASTER | INTTXDATA) as u8)
            .unwrap();
        assert_eq!(spi.read_u32(INTMASK).unwrap(), TXDONE as u32);
        assert!(spi.irq_active());
        spi.write_u32(INT_CLEAR, TXDONE as u32).unwrap();
        assert!(!spi.irq_active());
        assert_eq!(spi.read_u32(INTRAW).unwrap(), RXDONE as u32);
    }

    #[test]
    fn slave_select_routes_by_line() {
        let (mut spi, events) = with(&["ss0", "1"]);
        master(&mut spi, 0);
        spi.write_u32(SLAVE_SELECT, 0b10).unwrap();
        spi.write_u32(TX_DATA, 0x55).unwrap();
        spi.write_u32(SLAVE_SELECT, 0).unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec!["1 sel".to_string(), "1 55".into(), "1 rel".into()]
        );
    }

    #[test]
    fn no_device_reads_zero_and_disabled_ignores_tx() {
        let (mut spi, _) = with(&[]);
        spi.write_u32(TX_DATA, 0x12).unwrap();
        assert_eq!(
            spi.read_u32(STATUS).unwrap() & (1 << 6),
            1 << 6,
            "disabled: no frame"
        );
        master(&mut spi, 1);
        spi.write_u32(SLAVE_SELECT, 1).unwrap();
        spi.write_u32(TX_DATA, 0x12).unwrap();
        assert_eq!(spi.read_u32(RX_DATA).unwrap(), 0);
    }

    #[test]
    fn wide_frames_go_msb_first() {
        let (mut spi, events) = with(&[""]);
        master(&mut spi, 1);
        spi.write_u32(FRAMESIZE, 16).unwrap();
        spi.write_u32(SLAVE_SELECT, 1).unwrap();
        spi.write_u32(TX_DATA, 0x1234).unwrap();
        assert_eq!(spi.read_u32(RX_DATA).unwrap(), 0x1335);
        assert_eq!(
            events.lock().unwrap()[1..3],
            [" 12".to_string(), " 34".into()]
        );
    }

    #[test]
    fn reset_bit_and_disable() {
        let (mut spi, _) = with(&[]);
        master(&mut spi, 5);
        spi.write_u32(CONTROL, MASTER | (5 << FRAMECNT_SHIFT))
            .unwrap();
        assert_eq!(
            spi.read_u32(CONTROL).unwrap() & FRAMECNT_MASK,
            0,
            "disable clears FRAMECNT"
        );
        spi.write_u32(FRAMESIZE, 8).unwrap();
        spi.write_u32(CONTROL, RESET).unwrap();
        assert_eq!(spi.read_u32(CONTROL).unwrap(), CONTROL_RESET);
        assert_eq!(spi.read_u32(FRAMESIZE).unwrap(), 4);
    }

    #[test]
    fn byte_reads_of_rx_data_pop_once() {
        let (mut spi, _) = with(&[""]);
        master(&mut spi, 0);
        spi.write_u32(FRAMESIZE, 16).unwrap();
        spi.write_u32(SLAVE_SELECT, 1).unwrap();
        spi.write_u32(TX_DATA, 0x0102).unwrap();
        spi.write_u32(TX_DATA, 0x0304).unwrap();
        assert_eq!(Peripheral::read_u16(&spi, RX_DATA).unwrap(), 0x0203);
        assert_eq!(spi.read_u32(RX_DATA).unwrap(), 0x0405);
    }
}
