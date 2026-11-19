// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Nuvoton NPCX memory DMA receive channel, based on Renode's `NPCX_MDMA`.
//!
//! Each Renode MDMA instance has two internal contexts in a 0x100-byte window.
//! This profile models context 0 (UART receive) and its registers at offsets
//! 0x00..0x18. Context 1 (memory-to-peripheral), its registers, the MDMA
//! interrupt output and MDMA5's I3C routing remain outside the profile. The
//! descriptor must provide the fixed UART URBUF source and the routed UART RX
//! request name, plus the destination memory address.
//!
//! Context 0 copies one byte from its fixed source to
//! `destination_address + current_count` per enable/request. `MDMAEN` starts
//! one byte immediately, matching Renode's `TryPerformCopy`; UART ERD
//! requests drive later bytes. The 13-bit transfer count is capped at 4096.
//! `TC` (bit 18) is sticky and write-zero-to-clear. There is no direct IRQ
//! output because Renode combines each MDMA IRQ with its UART IRQ, while the
//! current bus descriptor has no shared-level interrupt combiner.

use crate::{
    CycleClock, DmaDirection, DmaRequest, DmaRoute, Peripheral, PeripheralTickResult, SimResult,
};
use std::any::Any;

const CTRL: u64 = 0x00;
const SOURCE: u64 = 0x04;
const DESTINATION: u64 = 0x08;
const TRANSFER_COUNT: u64 = 0x0C;
const CURRENT_DESTINATION: u64 = 0x14;
const CURRENT_COUNT: u64 = 0x18;

const MDMAEN: u32 = 1;
const SIEN: u32 = 1 << 8;
const TC: u32 = 1 << 18;
const MAX_TRANSFER_COUNT: u16 = 4096;
const ADDRESS_MASK: u32 = 0x000F_FFFF;
const EVENT_TOKEN: u32 = 0;

/// NPCX MDMA context 0 (UART receive).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct NpcxMdma {
    source_address: u32,
    destination_address: u32,
    default_destination_address: u32,
    rx_request: Option<String>,
    transfer_count: u16,
    current_count: u16,
    enabled: bool,
    interrupt_enabled: bool,
    terminal_count: bool,
    pending_requests: u16,
    #[serde(skip)]
    in_flight: bool,
    #[serde(skip)]
    event_live: bool,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl NpcxMdma {
    pub fn new(source_address: u32, destination_address: u32, rx_request: Option<String>) -> Self {
        Self {
            source_address,
            destination_address,
            default_destination_address: destination_address,
            rx_request,
            transfer_count: 0,
            current_count: 0,
            enabled: false,
            interrupt_enabled: false,
            terminal_count: false,
            pending_requests: 0,
            in_flight: false,
            event_live: false,
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn control(&self) -> u32 {
        (u32::from(self.enabled) * MDMAEN)
            | (u32::from(self.interrupt_enabled) * SIEN)
            | (u32::from(self.terminal_count) * TC)
    }

    fn has_work(&self) -> bool {
        self.enabled && self.transfer_count > 0 && self.pending_requests > 0 && !self.in_flight
    }

    fn arm_initial_copy(&mut self) {
        if self.enabled && self.transfer_count > 0 {
            self.pending_requests = self.pending_requests.saturating_add(1);
        }
    }

    fn next_request(&mut self) -> Option<DmaRequest> {
        if !self.has_work() {
            return None;
        }
        self.pending_requests -= 1;
        self.in_flight = true;
        Some(DmaRequest {
            src_addr: u64::from(self.source_address),
            addr: u64::from(
                self.destination_address
                    .wrapping_add(u32::from(self.current_count)),
            ),
            val: 0,
            direction: DmaDirection::Copy,
            transform: None,
            remaining: Some(u32::from(self.transfer_count - 1)),
        })
    }

    fn write_control(&mut self, value: u32) {
        self.interrupt_enabled = value & SIEN != 0;
        if value & TC == 0 {
            // Renode models TC as WriteZeroToClear: write one preserves it.
            self.terminal_count = false;
        }

        if self.enabled && value & MDMAEN == 0 && self.transfer_count > 0 {
            // Renode refuses to stop an active copy.
            return;
        }
        let was_enabled = self.enabled;
        self.enabled = value & MDMAEN != 0;
        if !was_enabled && self.enabled {
            self.current_count = 0;
            self.arm_initial_copy();
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            CTRL => self.control(),
            SOURCE => self.source_address,
            DESTINATION => self.destination_address,
            TRANSFER_COUNT => u32::from(self.transfer_count),
            CURRENT_DESTINATION => self
                .destination_address
                .wrapping_add(u32::from(self.current_count)),
            CURRENT_COUNT => u32::from(self.current_count),
            _ => 0, // context 1 and reserved words are outside this profile
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            CTRL => self.write_control(value),
            DESTINATION => {
                self.destination_address =
                    (self.destination_address & !ADDRESS_MASK) | (value & ADDRESS_MASK);
            }
            TRANSFER_COUNT => self.transfer_count = (value as u16).min(MAX_TRANSFER_COUNT),
            _ => {} // source/current registers and context 1 are read-only here
        }
    }

    fn reset_registers(&mut self) {
        self.destination_address = self.default_destination_address;
        self.transfer_count = 0;
        self.current_count = 0;
        self.enabled = false;
        self.interrupt_enabled = false;
        self.terminal_count = false;
        self.pending_requests = 0;
        self.in_flight = false;
        self.event_live = false;
    }
}

impl Peripheral for NpcxMdma {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let aligned = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        Ok((self.read_word(aligned) >> shift) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        let old = self.read_word(aligned);
        self.write_word(
            aligned,
            (old & !(0xFF << shift)) | (u32::from(value) << shift),
        );
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        let lo = u16::from(self.read(offset)?);
        let hi = u16::from(self.read(offset + 1)?);
        Ok(lo | (hi << 8))
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write(offset, value as u8)?;
        self.write(offset + 1, (value >> 8) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        Some((self.read_word(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn dma_route_request(&self, request: &str, out: &mut Vec<DmaRoute>) {
        if self.enabled && self.rx_request.as_deref() == Some(request) {
            out.push(DmaRoute {
                controller: None,
                channel: 1, // bus-facing channels are numbered from one
            });
        }
    }

    fn dma_request(&mut self, channel: u32) {
        if channel == 1 && self.enabled && self.transfer_count > 0 {
            self.pending_requests = self.pending_requests.saturating_add(1);
        }
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        PeripheralTickResult {
            dma_requests: self.next_request().map(|request| vec![request]),
            ..Default::default()
        }
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.scheduler_mode() && self.has_work() && !self.event_live {
            self.event_live = true;
            vec![(0, EVENT_TOKEN)]
        } else {
            Vec::new()
        }
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() || event_token != EVENT_TOKEN {
            return crate::sched::EventResult::default();
        }
        self.event_live = false;
        crate::sched::EventResult {
            dma_requests: self.next_request().into_iter().collect(),
            ..Default::default()
        }
    }

    fn dma_transfer_complete(&mut self, _item: &DmaRequest) -> bool {
        if !self.in_flight {
            return false;
        }
        self.in_flight = false;
        self.transfer_count = self.transfer_count.saturating_sub(1);
        self.current_count = self.current_count.saturating_add(1);
        if self.transfer_count == 0 {
            self.enabled = false;
            self.current_count = 0;
            self.terminal_count = true;
            self.pending_requests = 0;
        }
        true
    }

    fn dma_transfer_error(&mut self, _item: &DmaRequest) {
        self.in_flight = false;
        self.enabled = false;
        self.pending_requests = 0;
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn on_system_reset(&mut self) {
        self.reset_registers();
    }

    fn on_node_reset(&mut self) {
        self.reset_registers();
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uart_rx_enable_copies_one_byte_and_latches_terminal_count() {
        let mut mdma = NpcxMdma::new(0x400E_0002, 0x1000_0000, Some("cr_uart1_rx".into()));
        mdma.write_u32(DESTINATION, 0x1005_F000).unwrap();
        mdma.write_u32(TRANSFER_COUNT, 1).unwrap();
        mdma.write_u32(CTRL, MDMAEN | SIEN).unwrap();

        let request = mdma.tick().dma_requests.unwrap().remove(0);
        assert_eq!(request.src_addr, 0x400E_0002);
        assert_eq!(request.addr, 0x1005_F000);
        assert_eq!(request.direction, DmaDirection::Copy);
        assert!(mdma.dma_transfer_complete(&request));
        assert_eq!(mdma.read_u32(CTRL).unwrap(), SIEN | TC);
        assert_eq!(mdma.read_u32(TRANSFER_COUNT).unwrap(), 0);
        assert_eq!(mdma.read_u32(CURRENT_COUNT).unwrap(), 0);
        assert_eq!(mdma.read_u32(CURRENT_DESTINATION).unwrap(), 0x1005_F000);
    }

    #[test]
    fn uart_rx_request_only_routes_while_enabled() {
        let mut mdma = NpcxMdma::new(0x400E_0002, 0x1000_0000, Some("cr_uart1_rx".into()));
        let mut routes = Vec::new();
        mdma.dma_route_request("cr_uart1_rx", &mut routes);
        assert!(routes.is_empty());

        mdma.write_u32(TRANSFER_COUNT, 2).unwrap();
        mdma.write_u32(CTRL, MDMAEN).unwrap();
        mdma.dma_route_request("cr_uart1_rx", &mut routes);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].channel, 1);
    }
}
