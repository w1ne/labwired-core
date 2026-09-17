// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! MCP2515 CAN controller with standard-ID classical CAN transport.
//!
//! Supports RESET, READ, WRITE, READ_STATUS, RX_STATUS, BIT_MODIFY for the
//! register file used by common Arduino MCP_CAN libraries.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;
use std::sync::mpsc::{Receiver, Sender};

const INST_WRITE: u8 = 0x02;
const INST_READ: u8 = 0x03;
const INST_BITMOD: u8 = 0x05;
const INST_READ_STATUS: u8 = 0xA0;
const INST_RX_STATUS: u8 = 0xB0;
const INST_RESET: u8 = 0xC0;

const REG_CANSTAT: u8 = 0x0E;
const REG_CANCTRL: u8 = 0x0F;
const REG_CNF3: u8 = 0x28;
const REG_CNF2: u8 = 0x29;
const REG_CNF1: u8 = 0x2A;
const REG_CANINTE: u8 = 0x2B;
const REG_CANINTF: u8 = 0x2C;
const REG_EFLG: u8 = 0x2D;
const REG_TXB0CTRL: u8 = 0x30;
const REG_TXB0SIDH: u8 = 0x31;
#[cfg(test)]
const REG_TXB0D0: u8 = 0x36;
const REG_TXB1CTRL: u8 = 0x40;
const REG_TXB1SIDH: u8 = 0x41;
#[cfg(test)]
const REG_TXB1D0: u8 = 0x46;
const REG_TXB2CTRL: u8 = 0x50;
const REG_TXB2SIDH: u8 = 0x51;
#[cfg(test)]
const REG_TXB2D0: u8 = 0x56;
const REG_RXB0SIDH: u8 = 0x61;
const REG_RXB1SIDH: u8 = 0x71;
const REG_RXB0CTRL: u8 = 0x60;
const REG_RXB1CTRL: u8 = 0x70;

const TXREQ: u8 = 0x08;
const CANINTF_RX0IF: u8 = 0x01;
const CANINTF_RX1IF: u8 = 0x02;
const CANINTF_TX0IF: u8 = 0x04;
const CANINTF_ERRIF: u8 = 0x20;
const EFLG_RX1OVR: u8 = 0x80;
const EFLG_RX0OVR: u8 = 0x40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TxBuffer {
    id: u32,
    dlc: u8,
    data: [u8; 8],
    pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RxBuffer {
    id: u32,
    dlc: u8,
    data: [u8; 8],
    full: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpMode {
    Normal,
    Sleep,
    Loopback,
    ListenOnly,
    Config,
}

impl OpMode {
    fn bits(self) -> u8 {
        match self {
            Self::Normal => 0x00,
            Self::Sleep => 0x20,
            Self::Loopback => 0x40,
            Self::ListenOnly => 0x60,
            Self::Config => 0x80,
        }
    }

    fn from_request(bits: u8) -> Option<Self> {
        match bits & 0xE0 {
            0x00 => Some(Self::Normal),
            0x20 => Some(Self::Sleep),
            0x40 => Some(Self::Loopback),
            0x60 => Some(Self::ListenOnly),
            0x80 => Some(Self::Config),
            _ => None,
        }
    }
}

#[allow(dead_code)] // Task 3 will use this when placing bus frames into RX registers.
fn encode_standard_id(id: u32) -> Option<[u8; 4]> {
    (id <= 0x7FF).then_some([(id >> 3) as u8, ((id & 7) << 5) as u8, 0, 0])
}

fn decode_standard_id(bytes: [u8; 4]) -> Option<u32> {
    if bytes[1] & 0x08 != 0 || bytes[2] != 0 || bytes[3] != 0 {
        return None;
    }
    Some(((bytes[0] as u32) << 3) | ((bytes[1] as u32) >> 5))
}

fn acceptance_sid(bytes: [u8; 4]) -> u32 {
    ((bytes[0] as u32) << 3) | ((bytes[1] as u32) >> 5)
}

pub struct Mcp2515 {
    cs_pin: String,
    regs: [u8; 128],
    phase: Phase,
    inst: u8,
    addr: u8,
    bitmod_mask: u8,
    rx_read_buffer: Option<usize>,
    component_id: Option<String>,
    bus_tx: Option<Sender<crate::network::CanFrame>>,
    bus_rx: Option<Receiver<crate::network::CanFrame>>,
    irq_asserted: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Instruction,
    Address,
    Data,
    BitModMask,
    BitModData,
    Status,
    Ignore,
}

impl Mcp2515 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        let mut regs = [0u8; 128];
        // Power-on: configuration mode (REQOP = 100)
        regs[REG_CANCTRL as usize] = 0x87;
        regs[REG_CANSTAT as usize] = 0x80;
        Self {
            cs_pin: cs_pin.into(),
            regs,
            phase: Phase::Instruction,
            inst: 0,
            addr: 0,
            bitmod_mask: 0,
            rx_read_buffer: None,
            component_id: None,
            bus_tx: None,
            bus_rx: None,
            irq_asserted: false,
        }
    }

    fn reset(&mut self) {
        self.regs = [0; 128];
        self.regs[REG_CANCTRL as usize] = 0x87;
        self.regs[REG_CANSTAT as usize] = OpMode::Config.bits();
        self.recompute_irq();
    }

    /// Internal active-low INT state. Physical GPIO routing is configured by a later wiring task.
    pub fn irq_asserted(&self) -> bool {
        self.irq_asserted
    }

    fn recompute_irq(&mut self) {
        self.irq_asserted = self.regs[REG_CANINTF as usize] & self.regs[REG_CANINTE as usize] != 0;
    }

    fn timing_is_500k(&self) -> bool {
        let cnf1 = self.regs[REG_CNF1 as usize];
        let cnf2 = self.regs[REG_CNF2 as usize];
        let cnf3 = self.regs[REG_CNF3 as usize];
        let brp = u32::from(cnf1 & 0x3F) + 1;
        let sjw = u32::from((cnf1 >> 6) & 0x03) + 1;
        let prop = u32::from(cnf2 & 0x07) + 1;
        let phase1 = u32::from((cnf2 >> 3) & 0x07) + 1;
        let phase2 = if cnf2 & 0x80 != 0 {
            u32::from(cnf3 & 0x07) + 1
        } else {
            phase1.max(2)
        };
        if phase2 < 2 || sjw > phase2 || prop + phase1 < phase2 {
            return false;
        }
        let tq = 1 + prop + phase1 + phase2;
        let bitrate = 16_000_000 / (2 * brp * tq);
        bitrate.abs_diff(500_000) <= 5_000
    }

    fn write_register(&mut self, address: u8, value: u8) {
        let address = address & 0x7F;
        if address == REG_CANSTAT {
            return;
        }
        if (REG_CNF3..=REG_CNF1).contains(&address)
            && self.regs[REG_CANSTAT as usize] & 0xE0 != OpMode::Config.bits()
        {
            return;
        }
        self.regs[address as usize] = value;
        if matches!(address, REG_CANINTE | REG_CANINTF | REG_EFLG) {
            self.recompute_irq();
        }
        if address != REG_CANCTRL {
            return;
        }
        let requested = OpMode::from_request(value);
        let accepted = requested
            .filter(|mode| matches!(mode, OpMode::Config | OpMode::Sleep) || self.timing_is_500k());
        if let Some(mode) = accepted {
            self.regs[REG_CANSTAT as usize] =
                (self.regs[REG_CANSTAT as usize] & 0x1F) | mode.bits();
        }
    }

    fn tx_buffer(&self, index: usize) -> TxBuffer {
        let ctrl = [REG_TXB0CTRL, REG_TXB1CTRL, REG_TXB2CTRL][index];
        let sidh = ctrl + 1;
        let id = decode_standard_id([
            self.regs[sidh as usize],
            self.regs[sidh as usize + 1],
            self.regs[sidh as usize + 2],
            self.regs[sidh as usize + 3],
        ])
        .unwrap_or(0);
        let dlc = self.regs[sidh as usize + 4] & 0x0F;
        let mut data = [0; 8];
        data.copy_from_slice(&self.regs[sidh as usize + 5..sidh as usize + 13]);
        TxBuffer {
            id,
            dlc,
            data,
            pending: self.regs[ctrl as usize] & TXREQ != 0,
        }
    }

    fn rx_buffer(&self, index: usize) -> RxBuffer {
        let sidh = [REG_RXB0SIDH, REG_RXB1SIDH][index];
        let id = decode_standard_id([
            self.regs[sidh as usize],
            self.regs[sidh as usize + 1],
            self.regs[sidh as usize + 2],
            self.regs[sidh as usize + 3],
        ])
        .unwrap_or(0);
        let dlc = self.regs[sidh as usize + 4] & 0x0F;
        let mut data = [0; 8];
        data.copy_from_slice(&self.regs[sidh as usize + 5..sidh as usize + 13]);
        RxBuffer {
            id,
            dlc,
            data,
            full: self.regs[REG_CANINTF as usize] & (1 << index) != 0,
        }
    }

    fn read_status(&self) -> u8 {
        let intf = self.regs[REG_CANINTF as usize];
        let mut status = intf & 0x03;
        for index in 0..3 {
            let buffer = self.tx_buffer(index);
            if buffer.pending {
                status |= 1 << (2 + index * 2);
            }
            if intf & (CANINTF_TX0IF << index) != 0 {
                status |= 1 << (3 + index * 2);
            }
        }
        status
    }

    fn rx_status(&self) -> u8 {
        let rx0 = self.rx_buffer(0);
        let rx1 = self.rx_buffer(1);
        let full = (if rx0.full { 0x40 } else { 0 }) | (if rx1.full { 0x80 } else { 0 });
        let selected = if rx0.full {
            Some(REG_RXB0SIDH)
        } else if rx1.full {
            Some(REG_RXB1SIDH)
        } else {
            None
        };
        let Some(sidh) = selected else {
            return 0;
        };
        let filter_hit = self.regs[sidh as usize - 1] & 0x07;
        let sidl = self.regs[sidh as usize + 1];
        let frame_type = if sidl & 0x08 != 0 {
            0x10 | if self.regs[sidh as usize + 4] & 0x40 != 0 {
                0x08
            } else {
                0
            }
        } else if sidl & 0x10 != 0 {
            0x08
        } else {
            0
        };
        full | frame_type | filter_hit
    }

    fn current_mode(&self) -> OpMode {
        OpMode::from_request(self.regs[REG_CANSTAT as usize]).unwrap_or(OpMode::Config)
    }

    fn standard_filter_match(&self, buffer: usize, id: u32) -> Option<u8> {
        let ctrl = [REG_RXB0CTRL, REG_RXB1CTRL][buffer];
        match (self.regs[ctrl as usize] >> 5) & 3 {
            3 => return Some(0),
            2 => return None,
            _ => {}
        }
        let mask_base = [0x20u8, 0x24][buffer];
        let mask_bytes = [
            self.regs[mask_base as usize],
            self.regs[mask_base as usize + 1],
            self.regs[mask_base as usize + 2],
            self.regs[mask_base as usize + 3],
        ];
        let mask = acceptance_sid(mask_bytes);
        let mide = mask_bytes[1] & 0x08 != 0;
        let filter_bases: &[u8] = if buffer == 0 {
            &[0x00, 0x04]
        } else {
            &[0x08, 0x10, 0x14, 0x18]
        };
        filter_bases.iter().enumerate().find_map(|(offset, base)| {
            let filter_bytes = [
                self.regs[*base as usize],
                self.regs[*base as usize + 1],
                self.regs[*base as usize + 2],
                self.regs[*base as usize + 3],
            ];
            let ide_matches = !mide || filter_bytes[1] & 0x08 == 0;
            let filter = acceptance_sid(filter_bytes);
            (ide_matches && (id & mask) == (filter & mask))
                .then_some((offset + if buffer == 0 { 0 } else { 2 }) as u8)
        })
    }

    fn store_rx(&mut self, buffer: usize, filter_hit: u8, frame: &crate::network::CanFrame) {
        let ctrl = [REG_RXB0CTRL, REG_RXB1CTRL][buffer];
        let sidh = [REG_RXB0SIDH, REG_RXB1SIDH][buffer];
        let encoded = encode_standard_id(frame.id).expect("accepted standard id");
        if buffer == 0 {
            // RXB0 has only FILHIT0; preserve BUKT in bit 2.
            self.regs[ctrl as usize] = (self.regs[ctrl as usize] & !0x01) | (filter_hit & 0x01);
        } else {
            self.regs[ctrl as usize] = (self.regs[ctrl as usize] & !0x07) | (filter_hit & 0x07);
        }
        self.regs[sidh as usize..sidh as usize + 4].copy_from_slice(&encoded);
        self.regs[sidh as usize + 4] = frame.data.len() as u8;
        self.regs[sidh as usize + 5..sidh as usize + 13].fill(0);
        self.regs[sidh as usize + 5..sidh as usize + 5 + frame.data.len()]
            .copy_from_slice(&frame.data);
        self.regs[REG_CANINTF as usize] |= [CANINTF_RX0IF, CANINTF_RX1IF][buffer];
        self.recompute_irq();
    }

    fn receive_standard(&mut self, frame: crate::network::CanFrame) {
        if frame.extended
            || frame.fd
            || frame.bitrate_switch
            || frame.remote
            || frame.id > 0x7ff
            || frame.data.len() > 8
        {
            return;
        }
        let match0 = self.standard_filter_match(0, frame.id);
        let match1 = self.standard_filter_match(1, frame.id);
        let full0 = self.regs[REG_CANINTF as usize] & CANINTF_RX0IF != 0;
        let full1 = self.regs[REG_CANINTF as usize] & CANINTF_RX1IF != 0;
        if let Some(hit) = match0 {
            if !full0 {
                self.store_rx(0, hit, &frame);
                return;
            }
            if self.regs[REG_RXB0CTRL as usize] & 0x04 != 0 {
                if !full1 {
                    self.store_rx(1, hit, &frame);
                    return;
                }
                self.regs[REG_EFLG as usize] |= EFLG_RX0OVR | EFLG_RX1OVR;
                self.regs[REG_CANINTF as usize] |= CANINTF_ERRIF;
            } else {
                self.regs[REG_EFLG as usize] |= EFLG_RX0OVR;
                self.regs[REG_CANINTF as usize] |= CANINTF_ERRIF;
            }
        } else if let Some(hit) = match1 {
            if !full1 {
                self.store_rx(1, hit, &frame);
                return;
            }
            self.regs[REG_EFLG as usize] |= EFLG_RX1OVR;
            self.regs[REG_CANINTF as usize] |= CANINTF_ERRIF;
        }
        self.recompute_irq();
    }

    fn service_pending_tx(&mut self) {
        let mode = self.current_mode();
        if !matches!(mode, OpMode::Normal | OpMode::Loopback) {
            return;
        }
        for index in 0..3 {
            let tx = self.tx_buffer(index);
            if !tx.pending || tx.dlc > 8 {
                continue;
            }
            let sidh = [REG_TXB0SIDH, REG_TXB1SIDH, REG_TXB2SIDH][index];
            if self.regs[sidh as usize + 4] & 0x40 != 0 {
                continue;
            }
            if decode_standard_id([
                self.regs[sidh as usize],
                self.regs[sidh as usize + 1],
                self.regs[sidh as usize + 2],
                self.regs[sidh as usize + 3],
            ])
            .is_none()
            {
                continue;
            }
            let frame =
                crate::network::CanFrame::classic(tx.id, tx.data[..tx.dlc as usize].to_vec());
            let sent = if mode == OpMode::Loopback {
                self.receive_standard(frame);
                true
            } else {
                self.bus_tx
                    .as_ref()
                    .is_some_and(|sender| sender.send(frame).is_ok())
            };
            if sent {
                let ctrl = [REG_TXB0CTRL, REG_TXB1CTRL, REG_TXB2CTRL][index];
                self.regs[ctrl as usize] &= !TXREQ;
                self.regs[REG_CANINTF as usize] |= CANINTF_TX0IF << index;
                self.recompute_irq();
            }
        }
    }
}

impl SpiDevice for Mcp2515 {
    fn needs_external_bus_poll(&self) -> bool {
        self.bus_rx.is_some()
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn attach_can_bus(
        &mut self,
        tx: Sender<crate::network::CanFrame>,
        rx: Receiver<crate::network::CanFrame>,
    ) -> anyhow::Result<()> {
        if self.bus_tx.is_some() || self.bus_rx.is_some() {
            anyhow::bail!("MCP2515 is already attached to a CAN bus");
        }
        self.bus_tx = Some(tx);
        self.bus_rx = Some(rx);
        Ok(())
    }
    fn poll_external_bus(&mut self) {
        self.service_pending_tx();
        let receives_external = matches!(self.current_mode(), OpMode::Normal | OpMode::ListenOnly);
        let mut frames = Vec::new();
        if let Some(rx) = &self.bus_rx {
            while let Ok(frame) = rx.try_recv() {
                frames.push(frame);
            }
        }
        if receives_external {
            for frame in frames {
                self.receive_standard(frame);
            }
        }
    }
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        self.phase = Phase::Instruction;
        self.rx_read_buffer = None;
    }

    fn cs_release(&mut self) {
        if let Some(buffer) = self.rx_read_buffer.take() {
            let intf = self.regs[REG_CANINTF as usize] & !(1 << buffer);
            self.write_register(REG_CANINTF, intf);
        }
        self.phase = Phase::Instruction;
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        match self.phase {
            Phase::Instruction => {
                self.inst = mosi;
                match mosi {
                    INST_RESET => {
                        self.reset();
                        self.phase = Phase::Ignore;
                        0
                    }
                    INST_READ | INST_WRITE => {
                        self.phase = Phase::Address;
                        0
                    }
                    INST_BITMOD => {
                        self.phase = Phase::Address;
                        0
                    }
                    INST_READ_STATUS => {
                        self.phase = Phase::Status;
                        0
                    }
                    INST_RX_STATUS => {
                        self.phase = Phase::Status;
                        0
                    }
                    0x81..=0x87 => {
                        for (index, ctrl) in [REG_TXB0CTRL, REG_TXB1CTRL, REG_TXB2CTRL]
                            .into_iter()
                            .enumerate()
                        {
                            if mosi & (1 << index) != 0 {
                                self.write_register(ctrl, self.regs[ctrl as usize] | TXREQ);
                            }
                        }
                        self.phase = Phase::Ignore;
                        0
                    }
                    0x40..=0x45 => {
                        let index = ((mosi - 0x40) / 2) as usize;
                        self.addr = [REG_TXB0SIDH, REG_TXB1SIDH, REG_TXB2SIDH][index]
                            + if mosi & 1 != 0 { 5 } else { 0 };
                        self.phase = Phase::Data;
                        0
                    }
                    0x90 | 0x92 | 0x94 | 0x96 => {
                        let index = ((mosi - 0x90) / 4) as usize;
                        self.rx_read_buffer = Some(index);
                        self.addr =
                            [REG_RXB0SIDH, REG_RXB1SIDH][index] + if mosi & 2 != 0 { 5 } else { 0 };
                        self.phase = Phase::Data;
                        0
                    }
                    _ => {
                        self.phase = Phase::Ignore;
                        0
                    }
                }
            }
            Phase::Address => {
                self.addr = mosi;
                self.phase = if self.inst == INST_BITMOD {
                    Phase::BitModMask
                } else {
                    Phase::Data
                };
                0
            }
            Phase::BitModMask => {
                self.bitmod_mask = mosi;
                self.phase = Phase::BitModData;
                0
            }
            Phase::BitModData => {
                let idx = self.addr as usize % self.regs.len();
                let cur = self.regs[idx];
                self.write_register(
                    self.addr,
                    (cur & !self.bitmod_mask) | (mosi & self.bitmod_mask),
                );
                self.phase = Phase::Ignore;
                0
            }
            Phase::Data => {
                let idx = self.addr as usize % self.regs.len();
                let miso = if self.inst == INST_READ || self.rx_read_buffer.is_some() {
                    self.regs[idx]
                } else {
                    0
                };
                if self.inst == INST_WRITE || (0x40..=0x45).contains(&self.inst) {
                    self.write_register(self.addr, mosi);
                }
                self.addr = self.addr.wrapping_add(1);
                miso
            }
            Phase::Status => {
                if self.inst == INST_READ_STATUS {
                    self.read_status()
                } else {
                    self.rx_status()
                }
            }
            Phase::Ignore => 0,
        }
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Mcp2515Kit;
pub static MCP2515_KIT: Mcp2515Kit = Mcp2515Kit;

static MCP2515_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "mcp2515",
    label: "MCP2515 CAN",
    summary: "Functional SPI classical CAN controller for standard 11-bit data frames.",
    detail: "Microchip MCP2515 SPI commands, modes, timing, three transmit buffers, two receive \
             buffers, standard-ID masks/filters, rollover, overflow, interrupt flags, and shared \
             classical CAN delivery. Limitations: 11-bit data frames only; extended identifiers, \
             remote frames, CAN FD/bitrate switching, physical INT GPIO wiring, and nested-device \
             trace contribution are not modeled. Active modes currently validate only a 16 MHz \
             oscillator at 500 kbit/s (within 1%); other oscillators and bitrates are not modeled.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[ConfigKey {
        name: "cs_pin",
        ty: ConfigType::Str,
        doc: "Chip-select GPIO pin (e.g. \"PA4\"). Defaults to PA4.",
    }],
    labs: &[],
};

impl PeripheralKit for Mcp2515Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &MCP2515_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let mut dev = Mcp2515::new(cs);
        dev.component_id = Some(ctx.device_id().to_string());
        ctx.attach_spi_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "mcp2515_tests.rs"]
mod tests;
