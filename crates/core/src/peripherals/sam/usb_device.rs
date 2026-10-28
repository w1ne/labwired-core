// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Explicit SAMD51 device-bank slice, independently authored from DS60001507F.
//! No USB descriptor/class replies are manufactured. Host tokens transfer real
//! SRAM bytes only. Wire timing, PID/retry/isochronous and host modes are excluded.
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;
use std::collections::VecDeque;

const ENDPOINTS: usize = 8;
const QUEUE_CAP: usize = 64;
const TRANSFER_CAP: u32 = 4096;
const EORST: u16 = 1 << 3;
const RXSTP: u8 = 1 << 4;
const BK0RDY: u8 = 1 << 6;
const BK1RDY: u8 = 1 << 7;

/// Typed simulator-host requests; these are inputs, never expected responses.
#[derive(Clone, Debug)]
pub enum HostToken {
    Reset,
    Setup {
        address: u8,
        packet: [u8; 8],
    },
    Out {
        address: u8,
        endpoint: u8,
        data: Vec<u8>,
    },
    In {
        address: u8,
        endpoint: u8,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostReply {
    Reset,
    SetupAccepted,
    OutAccepted { endpoint: u8, bytes: usize },
    In { endpoint: u8, data: Vec<u8> },
    Stall { endpoint: u8, input: bool },
}
#[derive(Clone, Copy, Debug, Default)]
struct Endpoint {
    config: u8,
    status: u8,
    flags: u8,
    enables: u8,
}
#[derive(Debug)]
pub struct Samd51UsbDeviceBanks {
    control: u8,
    device_control: u16,
    address: u8,
    descriptor: u32,
    calibration: u16,
    flags: u16,
    enables: u16,
    endpoints: [Endpoint; ENDPOINTS],
    host: VecDeque<HostToken>,
    replies: VecDeque<HostReply>,
    refusal: Option<String>,
    transactions: u64,
}
impl Default for Samd51UsbDeviceBanks {
    fn default() -> Self {
        Self {
            control: 0,
            device_control: 1,
            address: 0,
            descriptor: 0,
            calibration: 0,
            flags: 0,
            enables: 0,
            endpoints: [Endpoint::default(); ENDPOINTS],
            host: VecDeque::new(),
            replies: VecDeque::new(),
            refusal: None,
            transactions: 0,
        }
    }
}
impl Samd51UsbDeviceBanks {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn queue_host(&mut self, token: HostToken) -> SimResult<()> {
        if self.host.len() >= QUEUE_CAP || self.refusal.is_some() {
            return Self::refuse("host queue full or controller refused");
        }
        match &token {
            HostToken::Reset => {}
            HostToken::Setup { address, .. } if *address <= 127 => {}
            HostToken::In { address, endpoint }
                if *address <= 127 && usize::from(*endpoint) < ENDPOINTS => {}
            HostToken::Out {
                address,
                endpoint,
                data,
            } if *address <= 127 && usize::from(*endpoint) < ENDPOINTS && data.len() <= 64 => {}
            _ => return Self::refuse("host token outside admitted endpoint/address/packet bounds"),
        }
        self.host.push_back(token);
        Ok(())
    }
    pub fn take_host_reply(&mut self) -> Option<HostReply> {
        self.replies.pop_front()
    }
    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }
    pub fn transactions(&self) -> u64 {
        self.transactions
    }
    fn refuse<T>(reason: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "samd51_usb_device_banks: {reason}"
        )))
    }
    fn live(&self) -> bool {
        self.control & 2 != 0 && self.device_control & 1 == 0
    }
    fn lines(&self) -> [bool; 4] {
        if !self.live() || self.refusal.is_some() {
            return [false; 4];
        }
        let common = self.flags & self.enables;
        let endpoints = self
            .endpoints
            .iter()
            .map(|e| e.flags & e.enables)
            .fold(0, |a, b| a | b);
        [
            common & !4 != 0 || endpoints & !3 != 0,
            common & 4 != 0,
            endpoints & 1 != 0,
            endpoints & 2 != 0,
        ]
    }
    fn endpoint(offset: u64) -> Option<(usize, u64)> {
        let relative = offset.checked_sub(0x100)?;
        let index = usize::try_from(relative / 0x20).ok()?;
        (index < ENDPOINTS).then_some((index, relative % 0x20))
    }
    fn check(&self) -> SimResult<()> {
        if let Some(reason) = &self.refusal {
            Self::refuse(reason)
        } else {
            Ok(())
        }
    }
    fn descriptor_bank(
        &self,
        bus: &mut dyn Bus,
        ep: usize,
        input: bool,
    ) -> SimResult<(u64, u32, u32)> {
        let table = u64::from(self.descriptor);
        if table & 3 != 0 || !bus.admits_ram_only_dma_extent(table, ENDPOINTS as u64 * 32) {
            return Self::refuse("descriptor table must be four-byte aligned primary RAM");
        }
        let bank = table + ep as u64 * 32 + if input { 16 } else { 0 };
        let address = bus.read_u32(bank)?;
        let packet = bus.read_u32(bank + 4)?;
        if address & 3 != 0 || packet >> 31 != 0 || (packet >> 28) & 7 > 3 {
            return Self::refuse("requires aligned buffers, <=64-byte endpoints and software ZLP");
        }
        // EXTREG and STATUS_BK plus reserved bytes are not an admitted mode.
        if bus.read_u32(bank + 8)? != 0 || bus.read_u32(bank + 12)? != 0 {
            return Self::refuse("extended/error/isochronous descriptor fields are not admitted");
        }
        Ok((bank, address, packet))
    }
    fn ram(bus: &dyn Bus, address: u64, length: u64) -> SimResult<()> {
        if length > u64::from(TRANSFER_CAP)
            || !bus.admits_ram_only_dma_extent(address, length.max(1))
        {
            return Self::refuse("buffer extent must be bounded primary RAM");
        }
        Ok(())
    }
    fn clocks_and_pins(bus: &mut dyn Bus) -> SimResult<bool> {
        Ok(bus.read_u32(0x40000810)? & (1 << 10) != 0
            && bus.read_u32(0x40000818)? & 1 != 0
            && bus.read_u32(0x40001ca8)? & (1 << 6) != 0
            && bus.read_u8(0x41008058)? & 1 != 0
            && bus.read_u8(0x41008059)? & 1 != 0
            && bus.read_u8(0x4100803c)? == 0x77)
    }
    fn service(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        let Some(token) = self.host.front().cloned() else {
            return Ok(());
        };
        if !self.live() || self.replies.len() >= QUEUE_CAP || !Self::clocks_and_pins(bus)? {
            return Ok(());
        }
        if matches!(token, HostToken::Reset) {
            self.address = 0;
            for (i, e) in self.endpoints.iter_mut().enumerate() {
                let config = if i == 0 { e.config } else { 0 };
                *e = Endpoint {
                    config,
                    ..Default::default()
                };
            }
            self.flags |= EORST;
            self.replies.push_back(HostReply::Reset);
        } else {
            let (address, ep, input) = match &token {
                HostToken::Setup { address, .. } => (*address, 0, false),
                HostToken::Out {
                    address, endpoint, ..
                } => (*address, usize::from(*endpoint), false),
                HostToken::In { address, endpoint } => (*address, usize::from(*endpoint), true),
                HostToken::Reset => unreachable!(),
            };
            if address != self.address & 127 {
                return Ok(());
            }
            let endpoint = self.endpoints[ep];
            let kind = if input {
                endpoint.config >> 4
            } else {
                endpoint.config & 7
            };
            if kind == 0 {
                return Ok(());
            }
            if matches!(token, HostToken::Setup { .. }) && (ep != 0 || kind != 1) {
                return Self::refuse("SETUP requires enabled control endpoint zero");
            }
            let setup = matches!(token, HostToken::Setup { .. });
            let stall = if input { 1 << 5 } else { 1 << 4 };
            if !setup && endpoint.status & stall != 0 {
                self.endpoints[ep].flags |= if input { 1 << 6 } else { 1 << 5 };
                self.replies.push_back(HostReply::Stall {
                    endpoint: ep as u8,
                    input,
                });
            } else {
                let ready = if setup {
                    // DS60001507F38.6.2.6: SETUP ignores BK0RDY; an unread
                    // RXSTP still owns the previous setup buffer.
                    endpoint.flags & RXSTP == 0
                } else if input {
                    endpoint.status & BK1RDY != 0
                } else {
                    endpoint.status & BK0RDY == 0
                };
                if !ready {
                    return Ok(());
                }
                let (bank, buffer, packet) = self.descriptor_bank(bus, ep, input)?;
                let size = 1u32 << (((packet >> 28) & 7) + 3);
                if input {
                    let total = packet & 0x3fff;
                    let sent = (packet >> 14) & 0x3fff;
                    if total > TRANSFER_CAP || sent > total {
                        return Self::refuse("invalid IN total/progress extent");
                    }
                    Self::ram(bus, u64::from(buffer), u64::from(total))?;
                    let count = (total - sent).min(size);
                    let mut data = Vec::with_capacity(count as usize);
                    for i in 0..count {
                        data.push(bus.read_u8(u64::from(buffer) + u64::from(sent + i))?);
                    }
                    let progress = sent + count;
                    bus.write_u32(bank + 4, (packet & !(0x3fff << 14)) | (progress << 14))?;
                    if progress == total {
                        self.endpoints[ep].status &= !BK1RDY;
                        self.endpoints[ep].flags |= 2;
                    }
                    self.endpoints[ep].status ^= 2; // successful host ACK toggles DTGLIN
                    self.replies.push_back(HostReply::In {
                        endpoint: ep as u8,
                        data,
                    });
                } else {
                    let (data, setup) = match &token {
                        HostToken::Setup { packet, .. } => (packet.as_slice(), true),
                        HostToken::Out { data, .. } => (data.as_slice(), false),
                        _ => unreachable!(),
                    };
                    if data.len() > size as usize || (packet >> 14) & 0x3fff != 0 {
                        return Self::refuse(
                            "OUT supports one bounded packet, no multipacket aggregation",
                        );
                    }
                    Self::ram(bus, u64::from(buffer), u64::from(size))?;
                    for (i, value) in data.iter().enumerate() {
                        bus.write_u8(u64::from(buffer) + i as u64, *value)?;
                    }
                    bus.write_u32(bank + 4, (packet & !0x3fff) | data.len() as u32)?;
                    self.endpoints[ep].status |= BK0RDY;
                    if setup {
                        self.endpoints[ep].status =
                            (self.endpoints[ep].status & !(BK1RDY | 0x30)) | 7;
                        self.endpoints[ep].flags |= RXSTP;
                        self.replies.push_back(HostReply::SetupAccepted);
                    } else {
                        self.endpoints[ep].flags |= 1;
                        self.endpoints[ep].status ^= 1; // successful OUT toggles DTGLOUT
                        self.replies.push_back(HostReply::OutAccepted {
                            endpoint: ep as u8,
                            bytes: data.len(),
                        });
                    }
                }
            }
        }
        self.host.pop_front();
        self.transactions += 1;
        Ok(())
    }
}
impl Peripheral for Samd51UsbDeviceBanks {
    fn on_system_reset(&mut self) {
        // A reset device leaves the bus: the host's queued transactions and
        // unread replies die with the connection. The count is evidence.
        let transactions = self.transactions;
        *self = Self {
            transactions,
            ..Self::default()
        };
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        self.check()?;
        if let Some((ep, reg)) = Self::endpoint(offset) {
            let e = self.endpoints[ep];
            return match reg {
                0 => Ok(e.config),
                4..=6 => Ok(e.status),
                7 => Ok(e.flags),
                8..=9 => Ok(e.enables),
                _ => Self::refuse("endpoint register outside admitted slice"),
            };
        }
        match offset {
            0 => Ok(self.control),
            2 => Ok(0),
            8..=9 => Ok((self.device_control >> ((offset - 8) * 8)) as u8),
            10 => Ok(self.address),
            0x14..=0x15 | 0x18..=0x19 => Ok((self.enables >> ((offset & 1) * 8)) as u8),
            0x1c..=0x1d => Ok((self.flags >> ((offset - 0x1c) * 8)) as u8),
            0x20..=0x21 => {
                let summary = self.endpoints.iter().enumerate().fold(0u16, |sum, (i, e)| {
                    sum | if e.flags & e.enables != 0 { 1 << i } else { 0 }
                });
                Ok((summary >> ((offset - 0x20) * 8)) as u8)
            }
            0x24..=0x27 => Ok((self.descriptor >> ((offset - 0x24) * 8)) as u8),
            0x28..=0x29 => Ok((self.calibration >> ((offset - 0x28) * 8)) as u8),
            _ => Self::refuse("register outside admitted slice"),
        }
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.check()?;
        if let Some((ep, reg)) = Self::endpoint(offset) {
            let e = &mut self.endpoints[ep];
            match reg {
                0 => {
                    let out = value & 7;
                    let input = (value >> 4) & 7;
                    let valid = |kind| {
                        if ep == 0 {
                            kind == 0 || kind == 1
                        } else {
                            kind == 0 || kind == 3 || kind == 4
                        }
                    };
                    if value & 0x88 != 0 || !valid(out) || !valid(input) {
                        return Self::refuse(
                            "only control EP0 and bulk/interrupt endpoints admitted",
                        );
                    }
                    e.config = value;
                }
                4 => {
                    if value & 8 != 0 {
                        return Self::refuse("reserved endpoint status bit");
                    }
                    e.status &= !value;
                }
                5 => {
                    if value & 8 != 0 {
                        return Self::refuse("reserved endpoint status bit");
                    }
                    e.status |= value;
                }
                7 => e.flags &= !value,
                8 => e.enables &= !value,
                9 => {
                    if value & 0x80 != 0 {
                        return Self::refuse("reserved endpoint interrupt bit");
                    }
                    e.enables |= value;
                }
                _ => return Self::refuse("read-only/unsupported endpoint write"),
            }
            return Ok(());
        }
        match offset {
            0 => {
                // SWRST (0), ENABLE (1), RUNSTDBY (2) per the ATDF; host MODE (7) and
                // reserved bits are refused.
                if value & !0x07 != 0 {
                    return Self::refuse("host/test/unsupported CTRLA mode");
                }
                if value & 1 != 0 {
                    let host = std::mem::take(&mut self.host);
                    let replies = std::mem::take(&mut self.replies);
                    *self = Self::default();
                    self.host = host;
                    self.replies = replies;
                } else {
                    self.control = value;
                }
            }
            8 => {
                if value & !1 != 0 {
                    return Self::refuse("only full-speed attach/detach admitted");
                }
                self.device_control = u16::from(value);
            }
            9 if value == 0 => {}
            10 => self.address = value,
            0x14..=0x15 => self.enables &= !(u16::from(value) << ((offset & 1) * 8)),
            0x18..=0x19 => {
                let bits = u16::from(value) << ((offset & 1) * 8);
                if bits & !EORST != 0 {
                    return Self::refuse("only EORST common interrupt enabled by this slice");
                }
                self.enables |= bits;
            }
            0x1c..=0x1d => self.flags &= !(u16::from(value) << ((offset & 1) * 8)),
            0x24..=0x27 => {
                let shift = (offset - 0x24) * 8;
                self.descriptor = (self.descriptor & !(255 << shift)) | (u32::from(value) << shift);
            }
            0x28..=0x29 => {
                let shift = (offset - 0x28) * 8;
                self.calibration =
                    (self.calibration & !(255 << shift)) | (u16::from(value) << shift);
            }
            _ => return Self::refuse("read-only/unsupported common write"),
        }
        Ok(())
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.check()?;
        match offset {
            8 if value & !1 == 0 => self.device_control = value,
            0x14 if value & !0x7ff == 0 => self.enables &= !value,
            0x18 if value & !EORST == 0 => self.enables |= value,
            0x1c if value & !0x7ff == 0 => self.flags &= !value,
            0x28 if value & !0x77df == 0 => self.calibration = value,
            _ => return Self::refuse("unsupported halfword register/value; no partial store"),
        }
        Ok(())
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.check()?;
        if offset != 0x24 {
            return Self::refuse("only descriptor address accepts a word store");
        }
        self.descriptor = value;
        Ok(())
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn needs_bus_tick(&self) -> bool {
        // External host tokens can arrive after the current queue drains. Keep
        // an enabled controller in the cached bus pass; service() is inert
        // without a queued token, and never invents a response or completion.
        self.live() && self.refusal.is_none()
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if let Err(error) = self.service(bus) {
            self.refusal = Some(error.to_string());
        }
    }
    fn idle_poll_bus_tick(&self) -> bool {
        self.needs_bus_tick()
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn legacy_tick_active(&self) -> bool {
        self.lines().into_iter().any(|x| x)
    }
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        for (i, level) in self.lines().into_iter().enumerate() {
            report(80 + i as u32, level);
        }
    }
    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        PeripheralTickResult {
            explicit_irqs: Some(
                self.lines()
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, v)| v.then_some(80 + i as u32))
                    .collect(),
            ),
            ..Default::default()
        }
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"peripheral":"samd51_usb_device_banks","address":self.address,"descriptor":self.descriptor,
            "transactions":self.transactions,"queued_host_tokens":self.host.len(),"pending_replies":self.replies.len(),"refusal":self.refusal})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DmaRequest, SimulationConfig};
    use std::collections::BTreeMap;
    const TABLE: u32 = 0x20000004;
    const BUFFER: u32 = 0x20001004;
    struct Memory {
        bytes: BTreeMap<u64, u8>,
        config: SimulationConfig,
    }
    impl Memory {
        fn new() -> Self {
            let mut m = Self {
                bytes: BTreeMap::new(),
                config: SimulationConfig::default(),
            };
            for (address, value) in [(0x40000810, 1 << 10), (0x40000818, 1), (0x40001ca8, 0x41)] {
                m.word(address, value);
            }
            for (address, value) in [(0x41008058, 1), (0x41008059, 1), (0x4100803c, 0x77)] {
                m.bytes.insert(address, value);
            }
            m
        }
        fn word(&mut self, address: u64, value: u32) {
            for (i, b) in value.to_le_bytes().into_iter().enumerate() {
                self.bytes.insert(address + i as u64, b);
            }
        }
    }
    impl Bus for Memory {
        fn read_u8(&self, address: u64) -> SimResult<u8> {
            if let Some(value) = self.bytes.get(&address) {
                return Ok(*value);
            }
            if (0x20000000..0x20002000).contains(&address) {
                return Ok(0);
            }
            Err(SimulationError::NotImplemented(
                "test unmapped byte read".into(),
            ))
        }
        fn write_u8(&mut self, address: u64, value: u8) -> SimResult<()> {
            // Mapped MMIO can accept ordinary stores. DMA admission must
            // still refuse it; a downstream write error cannot prove the gate.
            if !(0x20000000..0x20002000).contains(&address) && !self.bytes.contains_key(&address) {
                return Err(SimulationError::NotImplemented(
                    "test unmapped store".into(),
                ));
            }
            self.bytes.insert(address, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            vec![]
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
            Err(SimulationError::NotImplemented("unused test DMA".into()))
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
        fn admits_ram_only_dma_extent(&self, start: u64, length: u64) -> bool {
            start >= 0x20000000
                && start
                    .checked_add(length)
                    .is_some_and(|end| end <= 0x20002000)
        }
    }
    fn controller() -> Samd51UsbDeviceBanks {
        let mut c = Samd51UsbDeviceBanks::new();
        c.write(0, 2).unwrap();
        c.write_u16(8, 0).unwrap();
        c.write_u32(0x24, TABLE).unwrap();
        c
    }
    fn bank(m: &mut Memory, ep: usize, input: bool, count: u32) {
        let address = u64::from(TABLE) + ep as u64 * 32 + if input { 16 } else { 0 };
        m.word(address, BUFFER);
        m.word(address + 4, (3 << 28) | count);
    }
    #[test]
    fn setup_uses_four_byte_aligned_real_ram_and_general_irq() {
        let mut c = controller();
        let mut m = Memory::new();
        bank(&mut m, 0, false, 0);
        c.write(0x100, 0x11).unwrap();
        c.write(0x109, RXSTP).unwrap();
        let packet = [0x80, 6, 0, 1, 0, 0, 18, 0];
        c.queue_host(HostToken::Setup { address: 0, packet })
            .unwrap();
        c.tick_with_bus(&mut m);
        assert_eq!(c.take_host_reply(), Some(HostReply::SetupAccepted));
        for (i, b) in packet.into_iter().enumerate() {
            assert_eq!(m.read_u8(u64::from(BUFFER) + i as u64).unwrap(), b);
        }
        assert_eq!(m.read_u32(u64::from(TABLE) + 4).unwrap() & 0x3fff, 8);
        assert_eq!(c.read(0x106).unwrap() & BK0RDY, BK0RDY);
        assert_eq!(c.lines(), [true, false, false, false]);
        c.write(0x107, RXSTP).unwrap();
        assert_eq!(c.lines(), [false; 4]);
    }
    #[test]
    fn setup_overwrites_full_status_bank_and_clears_stall_but_preserves_unread_setup() {
        let mut c = controller();
        let mut m = Memory::new();
        bank(&mut m, 0, false, 0);
        c.write(0x100, 0x11).unwrap();
        c.queue_host(HostToken::Out {
            address: 0,
            endpoint: 0,
            data: vec![],
        })
        .unwrap();
        c.tick_with_bus(&mut m);
        assert_eq!(
            c.take_host_reply(),
            Some(HostReply::OutAccepted {
                endpoint: 0,
                bytes: 0
            })
        );
        c.write(0x105, 0x30).unwrap();
        c.queue_host(HostToken::Setup {
            address: 0,
            packet: [19; 8],
        })
        .unwrap();
        c.tick_with_bus(&mut m);
        assert_eq!(c.take_host_reply(), Some(HostReply::SetupAccepted));
        assert_eq!(c.read(0x106).unwrap(), BK0RDY | 7);
        assert_eq!(
            c.read(0x107).unwrap(),
            RXSTP | 1,
            "SETUP does not invent a TRCPT0 clear"
        );
        let count = c.transactions();
        c.queue_host(HostToken::Setup {
            address: 0,
            packet: [37; 8],
        })
        .unwrap();
        c.tick_with_bus(&mut m);
        assert_eq!(c.take_host_reply(), None);
        assert_eq!(c.transactions(), count);
        assert_eq!(m.read_u8(u64::from(BUFFER)).unwrap(), 19);
        c.write(0x107, RXSTP).unwrap();
        c.tick_with_bus(&mut m);
        assert_eq!(c.take_host_reply(), Some(HostReply::SetupAccepted));
        assert_eq!(m.read_u8(u64::from(BUFFER)).unwrap(), 37);
    }
    #[test]
    fn in_requires_host_and_bank_readiness_then_preserves_multipacket_progress() {
        let mut c = controller();
        let mut m = Memory::new();
        bank(&mut m, 1, true, 65);
        for i in 0..65u64 {
            m.write_u8(u64::from(BUFFER) + i, i as u8).unwrap();
        }
        c.write(0x120, 0x30).unwrap();
        c.write(0x129, 2).unwrap();
        c.queue_host(HostToken::In {
            address: 0,
            endpoint: 1,
        })
        .unwrap();
        c.tick_with_bus(&mut m);
        assert_eq!(c.transactions(), 0);
        assert_eq!(c.take_host_reply(), None);
        c.write(0x125, BK1RDY).unwrap();
        c.tick_with_bus(&mut m);
        assert_eq!(
            c.take_host_reply(),
            Some(HostReply::In {
                endpoint: 1,
                data: (0..64).collect()
            })
        );
        assert_eq!(
            (m.read_u32(u64::from(TABLE) + 32 + 16 + 4).unwrap() >> 14) & 0x3fff,
            64
        );
        assert_eq!(c.lines(), [false; 4]);
        assert_ne!(c.read(0x126).unwrap() & BK1RDY, 0);
        c.tick_with_bus(&mut m);
        assert_eq!(
            c.transactions(),
            1,
            "no host token means no invented completion"
        );
        c.queue_host(HostToken::In {
            address: 0,
            endpoint: 1,
        })
        .unwrap();
        c.tick_with_bus(&mut m);
        assert_eq!(
            c.take_host_reply(),
            Some(HostReply::In {
                endpoint: 1,
                data: vec![64]
            })
        );
        assert_eq!(c.lines(), [false, false, false, true]);
        assert_eq!(c.read(0x126).unwrap() & BK1RDY, 0);
        c.write(0x127, 2).unwrap();
        assert_eq!(c.lines(), [false; 4]);
    }
    #[test]
    fn enabled_poll_survives_empty_queue_and_services_later_host_arrival() {
        let mut c = controller();
        let mut m = Memory::new();
        // The bus caches this eligibility after every pass. No token means
        // no event, but must not remove the live external-input controller.
        let mut cached = c.needs_bus_tick();
        assert!(cached && c.idle_poll_bus_tick());
        if cached {
            c.tick_with_bus(&mut m);
            cached = c.needs_bus_tick();
        }
        assert_eq!(c.transactions(), 0);
        assert_eq!(c.take_host_reply(), None);
        c.queue_host(HostToken::Reset).unwrap();
        if cached {
            c.tick_with_bus(&mut m);
            cached = c.needs_bus_tick();
        }
        assert_eq!(c.take_host_reply(), Some(HostReply::Reset));
        assert_eq!(c.transactions(), 1);
        assert!(cached && c.idle_poll_bus_tick());
        c.tick_with_bus(&mut m);
        assert_eq!(c.take_host_reply(), None);
        assert_eq!(c.transactions(), 1);
        c.write(0, 0).unwrap();
        assert!(!c.needs_bus_tick() && !c.idle_poll_bus_tick());
    }
    #[test]
    fn ordinary_full_packet_ack_completes_without_automatic_zlp() {
        let mut c = controller();
        let mut m = Memory::new();
        bank(&mut m, 1, true, 64);
        c.write(0x120, 0x30).unwrap();
        c.write(0x125, BK1RDY).unwrap();
        c.queue_host(HostToken::In {
            address: 0,
            endpoint: 1,
        })
        .unwrap();
        c.tick_with_bus(&mut m);
        assert!(matches!(c.take_host_reply(),Some(HostReply::In {data,..}) if data.len()==64));
        assert_eq!(c.read(0x127).unwrap(), 2);
        assert_eq!(c.read(0x126).unwrap() & BK1RDY, 0);
    }
    #[test]
    fn out_readiness_and_completion_do_not_become_setup_or_in_irqs() {
        let mut c = controller();
        let mut m = Memory::new();
        bank(&mut m, 2, false, 0);
        c.write(0x140, 3).unwrap();
        c.write(0x149, 1).unwrap();
        c.write(0x145, BK0RDY).unwrap();
        c.queue_host(HostToken::Out {
            address: 0,
            endpoint: 2,
            data: vec![19, 37],
        })
        .unwrap();
        c.tick_with_bus(&mut m);
        assert_eq!(c.take_host_reply(), None);
        assert_eq!(c.transactions(), 0);
        c.write(0x144, BK0RDY).unwrap();
        c.tick_with_bus(&mut m);
        assert_eq!(
            c.take_host_reply(),
            Some(HostReply::OutAccepted {
                endpoint: 2,
                bytes: 2
            })
        );
        assert_eq!(m.read_u8(u64::from(BUFFER)).unwrap(), 19);
        assert_eq!(m.read_u8(u64::from(BUFFER) + 1).unwrap(), 37);
        assert_eq!(c.lines(), [false, false, true, false]);
        c.write(0x147, 1).unwrap();
        assert_eq!(c.lines(), [false; 4]);
    }
    #[test]
    fn clock_and_pinmux_inputs_gate_actual_host_progress() {
        for address in [
            0x40000811, 0x40000818, 0x40001ca8, 0x41008058, 0x41008059, 0x4100803c,
        ] {
            let mut c = controller();
            let mut m = Memory::new();
            bank(&mut m, 0, false, 0);
            c.write(0x100, 0x11).unwrap();
            let original = m.bytes.insert(address, 0).unwrap();
            c.queue_host(HostToken::Setup {
                address: 0,
                packet: [0; 8],
            })
            .unwrap();
            c.tick_with_bus(&mut m);
            assert_eq!(c.transactions(), 0);
            assert_eq!(c.take_host_reply(), None);
            m.bytes.insert(address, original);
            c.tick_with_bus(&mut m);
            assert_eq!(c.take_host_reply(), Some(HostReply::SetupAccepted));
        }
    }
    #[test]
    fn malformed_dma_and_unsupported_descriptors_refuse_without_completion() {
        for kind in 0..7 {
            let mut c = controller();
            let mut m = Memory::new();
            bank(&mut m, 1, true, 1);
            c.write(0x120, 0x30).unwrap();
            c.write(0x125, BK1RDY).unwrap();
            match kind {
                0 => {
                    c.write_u32(0x24, TABLE + 2).unwrap();
                    m.word(u64::from(TABLE + 2) + 48, BUFFER);
                    m.word(u64::from(TABLE + 2) + 52, (3 << 28) | 1);
                }
                1 => {
                    c.write_u32(0x24, 0x41000000).unwrap();
                    for (offset, value) in [(48, BUFFER), (52, (3 << 28) | 1), (56, 0), (60, 0)] {
                        m.word(0x41000000 + offset, value);
                    }
                }
                2 => m.word(u64::from(TABLE) + 48, BUFFER + 2),
                3 => m.word(u64::from(TABLE) + 48, 0x4100803c),
                4 => m.word(u64::from(TABLE) + 52, (3 << 28) | 4097),
                5 => m.word(u64::from(TABLE) + 52, (3 << 28) | 1 | (1 << 31)),
                6 => m.word(u64::from(TABLE) + 56, 1),
                _ => unreachable!(),
            }
            c.queue_host(HostToken::In {
                address: 0,
                endpoint: 1,
            })
            .unwrap();
            c.tick_with_bus(&mut m);
            assert!(c.refusal().is_some(), "negative {kind}");
            assert_eq!(c.transactions(), 0);
            assert_eq!(c.take_host_reply(), None);
            assert_eq!(c.endpoints[1].flags, 0);
            assert!(c.read(0x127).is_err());
        }
    }
    #[test]
    fn reset_and_register_width_controls_preserve_explicit_irq_routes() {
        let mut c = controller();
        let mut m = Memory::new();
        c.write(0x100, 0x11).unwrap();
        c.write(0x120, 0x33).unwrap();
        c.write_u16(0x18, EORST).unwrap();
        c.queue_host(HostToken::Reset).unwrap();
        c.tick_with_bus(&mut m);
        assert_eq!(c.take_host_reply(), Some(HostReply::Reset));
        assert_eq!(c.read(0x100).unwrap(), 0x11);
        assert_eq!(c.read(0x120).unwrap(), 0);
        assert_eq!(c.lines(), [true, false, false, false]);
        c.write_u16(0x1c, EORST).unwrap();
        assert_eq!(c.lines(), [false; 4]);
        c.write_u16(0x14, EORST).unwrap();
        assert!(c.write_u16(0x18, 0x1008).is_err());
        assert_eq!(c.enables, 0);
        assert!(c.write_u32(0, 2).is_err());
        assert_eq!(c.control, 2);
        assert!(c.write(0x100, 2).is_err());
        assert_eq!(c.read(0x100).unwrap(), 0x11);
        assert!(c.write(0x106, 0).is_err());
        assert!(c.read(0x80).is_err());
    }
    #[test]
    fn external_queue_is_bounded_and_rejects_out_of_profile_tokens() {
        let mut c = controller();
        assert!(c
            .queue_host(HostToken::In {
                address: 0,
                endpoint: 8
            })
            .is_err());
        assert!(c
            .queue_host(HostToken::In {
                address: 128,
                endpoint: 0
            })
            .is_err());
        assert!(c
            .queue_host(HostToken::Out {
                address: 0,
                endpoint: 1,
                data: vec![0; 65]
            })
            .is_err());
        for _ in 0..QUEUE_CAP {
            c.queue_host(HostToken::Reset).unwrap();
        }
        assert!(c.queue_host(HostToken::Reset).is_err());
        assert_eq!(c.host.len(), QUEUE_CAP);
    }
    #[test]
    fn ctrla_admits_run_in_standby_at_bit_two_only() {
        let mut c = Samd51UsbDeviceBanks::new();
        c.write(0, 0x04).unwrap();
        assert_eq!(c.read(0).unwrap(), 0x04);
        c.write(0, 0x06).unwrap();
        assert!(c.live() || c.device_control & 1 != 0);
        let mut d = Samd51UsbDeviceBanks::new();
        assert!(d.write(0, 0x40).is_err(), "bit 6 is reserved in USB CTRLA");
        let mut h = Samd51UsbDeviceBanks::new();
        assert!(h.write(0, 0x80).is_err(), "host mode is not admitted");
    }
}
