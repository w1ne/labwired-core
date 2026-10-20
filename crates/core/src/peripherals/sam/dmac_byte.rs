// SPDX-License-Identifier: MIT
//! Opt-in SAMD51 software-byte and SERCOM4 TX-byte profiles, not full DMAC.
//! Primary register facts: DS60001507F sections 22.6, 22.7, 22.8 and 22.10.
//! No ASF implementation/header is copied or imported. Unsupported descriptors latch an explicit
//! simulator refusal; they never masquerade as hardware completion/error flags.

use crate::{Bus, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;

const ENABLE: u32 = 2;
const TERR: u8 = 1;
const TCMPL: u8 = 2;
const SERCOM4_DATA: u64 = 0x43000028;
const TX_TRIGGER: u32 = (0x0d << 8) | (2 << 20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Profile {
    SoftwareBlock,
    Sercom4Tx,
}

#[derive(Clone, Copy, Debug, Default)]
struct Descriptor {
    control: u16,
    remaining: u16,
    source: u32,
    destination: u32,
    next: u32,
}

impl Descriptor {
    fn decode(raw: [u8; 16]) -> Self {
        Self {
            control: u16::from_le_bytes(raw[0..2].try_into().unwrap()),
            remaining: u16::from_le_bytes(raw[2..4].try_into().unwrap()),
            source: u32::from_le_bytes(raw[4..8].try_into().unwrap()),
            destination: u32::from_le_bytes(raw[8..12].try_into().unwrap()),
            next: u32::from_le_bytes(raw[12..16].try_into().unwrap()),
        }
    }

    fn encode(self) -> [u8; 16] {
        let mut raw = [0; 16];
        raw[0..2].copy_from_slice(&self.control.to_le_bytes());
        raw[2..4].copy_from_slice(&self.remaining.to_le_bytes());
        raw[4..8].copy_from_slice(&self.source.to_le_bytes());
        raw[8..12].copy_from_slice(&self.destination.to_le_bytes());
        raw[12..16].copy_from_slice(&self.next.to_le_bytes());
        raw
    }

    fn refusal(self) -> Option<&'static str> {
        if self.control & 1 == 0 {
            return Some("invalid VALID interrupt classification is not admitted");
        }
        // VALID, BLOCKACT=NOACT/INT, SRCINC/DSTINC; all other modes excluded.
        if self.control & !0x0c09 != 0 || self.next != 0 || self.remaining == 0 {
            return Some("requires byte beats, unit steps, nonzero count, NOACT/INT and no link");
        }
        None
    }

    fn address(self, source: bool) -> u64 {
        let (end, increment) = if source {
            (self.source, self.control & (1 << 10) != 0)
        } else {
            (self.destination, self.control & (1 << 11) != 0)
        };
        u64::from(if increment {
            end.wrapping_sub(u32::from(self.remaining))
        } else {
            end
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    control: u32,
    interrupt_enable: u8,
    flags: u8,
    pending: bool,
    descriptor: Option<Descriptor>,
}

#[derive(Debug)]
pub struct SamDmacByte {
    mmio_base: u64,
    profile: Profile,
    control: u16,
    base: u32,
    writeback: u32,
    channels: [Channel; 32],
    refusal: Option<&'static str>,
}

impl SamDmacByte {
    pub fn new(mmio_base: u64) -> Self {
        Self::with_profile(mmio_base, Profile::SoftwareBlock)
    }

    pub fn new_sercom4_tx(mmio_base: u64) -> Self {
        Self::with_profile(mmio_base, Profile::Sercom4Tx)
    }

    fn with_profile(mmio_base: u64, profile: Profile) -> Self {
        Self {
            mmio_base,
            profile,
            control: 0,
            base: 0,
            writeback: 0,
            channels: [Channel::default(); 32],
            refusal: None,
        }
    }

    fn lines(&self) -> [bool; 5] {
        let mut lines = [false; 5];
        for (index, channel) in self.channels.iter().enumerate() {
            lines[index.min(4)] |= channel.flags & channel.interrupt_enable != 0;
        }
        lines
    }

    fn reg(&self, offset: u64) -> u32 {
        match offset {
            0 => u32::from(self.control),
            0x20 => self
                .channels
                .iter()
                .position(|c| c.flags & c.interrupt_enable != 0)
                .map(|i| {
                    let c = self.channels[i];
                    i as u32
                        | (u32::from(c.flags & c.interrupt_enable & 3) << 8)
                        | (u32::from(c.pending) << 15)
                        | (u32::from(c.descriptor.is_some() && c.control & ENABLE != 0) << 14)
                })
                .unwrap_or(0),
            0x24 => self.channels.iter().enumerate().fold(0, |bits, (i, c)| {
                bits | (u32::from(c.flags & c.interrupt_enable != 0) << i)
            }),
            0x28 | 0x2c => self.channels.iter().enumerate().fold(0, |bits, (i, c)| {
                let active = if offset == 0x28 {
                    c.descriptor.is_some() && c.control & ENABLE != 0
                } else {
                    c.pending
                };
                bits | (u32::from(active) << i)
            }),
            0x34 => self.base,
            0x38 => self.writeback,
            0x40..=0x23f => {
                let c = &self.channels[((offset - 0x40) / 16) as usize];
                match (offset - 0x40) % 16 {
                    0..=3 => c.control,
                    12 | 13 => u32::from(c.interrupt_enable),
                    14 => u32::from(c.flags),
                    15 => {
                        u32::from(c.pending)
                            | (u32::from(c.descriptor.is_some() && c.control & ENABLE != 0) << 1)
                    }
                    _ => 0,
                }
            }
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32, mask: u32) -> SimResult<()> {
        let requested = value & mask;
        match offset {
            0 => {
                if requested & 1 != 0 {
                    if self.control & ENABLE as u16 != 0 {
                        return Err(SimulationError::Other(
                            "SAM DMAC reset requires disabled controller".into(),
                        ));
                    }
                    *self = Self::with_profile(self.mmio_base, self.profile);
                } else {
                    if requested & !0x0f02 != 0 {
                        return Err(SimulationError::NotImplemented(
                            "sam_dmac_byte: CRC and other control modes excluded".into(),
                        ));
                    }
                    let mut new = ((u32::from(self.control) & !mask) | requested) as u16;
                    if self.control & ENABLE as u16 != 0 {
                        // LVLEN fields are enable-protected; DMAENABLE is not.
                        new = (new & ENABLE as u16) | (self.control & 0x0f00);
                    }
                    self.control = new;
                }
            }
            0x10 => {
                if self.profile == Profile::Sercom4Tx && requested != 0 {
                    return Err(SimulationError::NotImplemented(
                        "SERCOM4 TX profile excludes software/event trigger injection".into(),
                    ));
                }
                for (i, c) in self.channels.iter_mut().enumerate() {
                    if requested & (1 << i) != 0 && c.control & ENABLE != 0 {
                        c.pending = true;
                    }
                }
            }
            0x34 | 0x38 if self.control & ENABLE as u16 == 0 => {
                let old = if offset == 0x34 {
                    self.base
                } else {
                    self.writeback
                };
                let new = (old & !mask) | requested;
                if new & 15 != 0 {
                    return Err(SimulationError::Other(
                        "SAM DMAC table must be 16-byte aligned".into(),
                    ));
                }
                if offset == 0x34 {
                    self.base = new;
                } else {
                    self.writeback = new;
                }
            }
            0x34 | 0x38 => {} // Enable-protected tables, not live retargeting.
            0x40..=0x23f => {
                let c = &mut self.channels[((offset - 0x40) / 16) as usize];
                match (offset - 0x40) % 16 {
                    0 => {
                        if requested & 1 != 0 {
                            if c.control & ENABLE == 0 {
                                *c = Channel::default();
                            }
                        } else {
                            let mut new = (c.control & !mask) | requested;
                            if self.profile == Profile::SoftwareBlock && new & !ENABLE != 0 {
                                return Err(SimulationError::NotImplemented("sam_dmac_byte: only software BLOCK/single-beat trigger configuration".into()));
                            }
                            if self.profile == Profile::Sercom4Tx {
                                // TRIGSRC is enable-protected; TRIGACT is not
                                // (DS60001507F p393). Disabled partial setup is
                                // permitted, but an enabled channel must select
                                // exactly SERCOM4 TX/BURST/SINGLE.
                                if c.control & ENABLE != 0 {
                                    new = (new & !0xff00) | (c.control & 0xff00);
                                }
                                let source = (new >> 8) & 0xff;
                                let action = (new >> 20) & 3;
                                if new & !(ENABLE | 0xff00 | (3 << 20)) != 0
                                    || !matches!(source, 0 | 0x0d)
                                    || !matches!(action, 0 | 2)
                                    || (new & ENABLE != 0 && new & !ENABLE != TX_TRIGGER)
                                {
                                    return Err(SimulationError::NotImplemented(
                                        "SERCOM4 TX requires TX13/BURST/SINGLE without other channel modes".into(),
                                    ));
                                }
                            }
                            if new & ENABLE == 0 || c.control & ENABLE == 0 {
                                c.pending = false;
                                c.descriptor = None;
                            }
                            c.control = new;
                        }
                    }
                    4..=6 if requested != 0 => {
                        return Err(SimulationError::NotImplemented(
                            "sam_dmac_byte: commands, priority and events excluded".into(),
                        ));
                    }
                    12 => c.interrupt_enable &= !(requested as u8 & 7),
                    13 => c.interrupt_enable |= requested as u8 & 7,
                    14 => c.flags &= !(requested as u8 & 7),
                    _ => {}
                }
            }
            _ if requested != 0 => {
                return Err(SimulationError::NotImplemented(
                    "sam_dmac_byte: register outside admitted slice".into(),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn writeback_descriptor(
        &self,
        bus: &mut dyn Bus,
        index: usize,
        d: Descriptor,
    ) -> SimResult<()> {
        let start = u64::from(self.writeback) + index as u64 * 16;
        crate::with_device_write_bypass(bus, |bus| {
            for (byte, value) in d.encode().into_iter().enumerate() {
                bus.write_u8(start + byte as u64, value)?;
            }
            Ok(())
        })
    }

    fn error(&mut self, bus: &mut dyn Bus, index: usize) {
        if let Some(mut d) = self.channels[index].descriptor {
            d.control &= !1;
            self.channels[index].descriptor = Some(d);
            // A failing writeback is retained as failure, never completion.
            let _ = self.writeback_descriptor(bus, index, d);
        }
        let c = &mut self.channels[index];
        c.control &= !ENABLE;
        c.pending = false;
        c.flags |= TERR;
    }

    fn service(&mut self, bus: &mut dyn Bus, index: usize) {
        // Only the primary SystemBus RAM mapping is admitted. Unmapped addresses
        // still reach the real bus to exercise its actual AHB-error boundary.
        // In particular a stub/gated MMIO write returning Ok is NOT RAM writeback.
        let table = u64::from(self.base) + index as u64 * 16;
        let writeback = u64::from(self.writeback) + index as u64 * 16;
        if !bus.admits_ram_only_dma_extent(table, 16)
            || !bus.admits_ram_only_dma_extent(writeback, 16)
        {
            self.refuse_memory(index);
            return;
        }
        if self.channels[index].descriptor.is_none() {
            let start = table;
            let mut raw = [0; 16];
            crate::with_device_write_bypass(bus, |bus| {
                for (byte, value) in raw.iter_mut().enumerate() {
                    match bus.read_u8(start + byte as u64) {
                        Ok(read) => *value = read,
                        Err(_) => {
                            self.error(bus, index);
                            return;
                        }
                    }
                }
            });
            let d = Descriptor::decode(raw);
            if let Some(reason) = d.refusal() {
                self.refusal = Some(reason);
                self.channels[index].control &= !ENABLE;
                self.channels[index].pending = false;
                return;
            }
            self.channels[index].descriptor = Some(d);
            self.channels[index].pending = false;
        }
        let mut d = self.channels[index].descriptor.unwrap();
        let source = d.address(true);
        let destination = d.address(false);
        if !bus.admits_ram_only_dma_extent(source, 1)
            || (self.profile == Profile::SoftwareBlock
                && !bus.admits_ram_only_dma_extent(destination, 1))
        {
            self.refuse_memory(index);
            return;
        }
        if self.profile == Profile::Sercom4Tx {
            if destination != SERCOM4_DATA || d.control & 0x0c00 != 0x0400 {
                self.refuse(
                    index,
                    "SERCOM4 TX requires unit SRCINC and fixed canonical DATA destination",
                );
                return;
            }
            match bus.dma_tx_request(destination) {
                Some(true) => {}
                Some(false) => return, // No data read, count or writeback progress.
                None => {
                    self.refuse(index, "SERCOM4 TX endpoint is not admitted by the bus");
                    return;
                }
            }
            let value = match crate::with_device_write_bypass(bus, |bus| bus.read_u8(source)) {
                Ok(value) => value,
                Err(_) => {
                    self.error(bus, index);
                    return;
                }
            };
            match bus.try_dma_write_tx_u8(destination, value) {
                Ok(true) => {}
                Ok(false) => return, // Rechecked readiness is not a bus fault.
                Err(SimulationError::NotImplemented(_)) => {
                    self.refuse(index, "SERCOM4 TX endpoint lost bus admission");
                    return;
                }
                Err(_) => {
                    self.error(bus, index);
                    return;
                }
            }
        } else {
            let transferred = crate::with_device_write_bypass(bus, |bus| {
                bus.read_u8(source)
                    .and_then(|value| bus.write_u8(destination, value))
            });
            if transferred.is_err() {
                self.error(bus, index);
                return;
            }
        }
        // Only the successful byte write commits this beat's count.
        d.remaining -= 1;
        let completed = d.remaining == 0;
        if completed {
            d.control &= !1;
        }
        self.channels[index].descriptor = Some(d);
        if self.writeback_descriptor(bus, index, d).is_err() {
            self.error(bus, index);
            return;
        }
        if completed {
            let c = &mut self.channels[index];
            c.control &= !ENABLE;
            c.pending = false;
            if d.control & 8 != 0 {
                c.flags |= TCMPL;
            }
        }
    }

    fn refuse_memory(&mut self, index: usize) {
        self.refuse(
            index,
            "mapped targets other than primary RAM are not admitted",
        );
    }

    fn refuse(&mut self, index: usize, reason: &'static str) {
        self.refusal = Some(reason);
        self.channels[index].control &= !ENABLE;
        self.channels[index].pending = false;
    }
}

impl Peripheral for SamDmacByte {
    fn read(&self, offset: u64) -> SimResult<u8> {
        if let Some(reason) = self.refusal {
            return Err(SimulationError::NotImplemented(format!(
                "sam_dmac_byte: {reason}"
            )));
        }
        let aligned = if offset < 0x40 || (offset - 0x40) % 16 < 4 {
            offset & !3
        } else {
            offset
        };
        let shift = if aligned != offset {
            (offset & 3) * 8
        } else {
            0
        };
        Ok((self.reg(aligned) >> shift) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = if offset < 0x40 || (offset - 0x40) % 16 < 4 {
            offset & !3
        } else {
            offset
        };
        let shift = if aligned != offset {
            (offset & 3) * 8
        } else {
            0
        };
        self.write_reg(aligned, u32::from(value) << shift, 0xff << shift)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 != 0 {
            return Err(SimulationError::Other(
                "unaligned SAM DMAC word write".into(),
            ));
        }
        if offset < 0x40 || (offset - 0x40) % 16 == 0 {
            self.write_reg(offset, value, u32::MAX)
        } else {
            for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
                self.write(offset + i as u64, byte)?;
            }
            Ok(())
        }
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn needs_bus_tick(&self) -> bool {
        self.refusal.is_none()
            && self.control & 0x102 == 0x102
            && self.channels.iter().any(|c| {
                c.control & ENABLE != 0
                    && (self.profile == Profile::Sercom4Tx || c.pending || c.descriptor.is_some())
            })
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if !self.needs_bus_tick() {
            return;
        }
        for i in 0..32 {
            let c = self.channels[i];
            if c.control & ENABLE != 0
                && (self.profile == Profile::Sercom4Tx || c.pending || c.descriptor.is_some())
            {
                self.service(bus, i);
                if self.refusal.is_some() {
                    break;
                }
            }
        }
    }
    fn legacy_tick_active(&self) -> bool {
        self.lines().into_iter().any(|level| level)
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        for (i, level) in self.lines().into_iter().enumerate() {
            report(31 + i as u32, level);
        }
    }
    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        PeripheralTickResult {
            explicit_irqs: Some(
                self.lines()
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, level)| level.then_some(31 + i as u32))
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
        let name = match self.profile {
            Profile::SoftwareBlock => "sam_dmac_byte",
            Profile::Sercom4Tx => "sam_dmac_sercom4_tx_byte",
        };
        serde_json::json!({"peripheral":name, "refusal":self.refusal,
            "channels":self.channels.iter().map(|c| serde_json::json!({
                "control":c.control,"flags":c.flags,"pending":c.pending,
                "remaining":c.descriptor.map(|d|d.remaining)
            })).collect::<Vec<_>>()})
    }
}
