// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The bus side of a world's timed CAN bus ([`crate::network::timed_can`]):
//! put an on-chip controller in timed mode, read it for the bus with its
//! clock, and apply what the bus decided.

use super::SystemBus;
use crate::network::timed_can::{CanNodePoll, CanNodeUpdate};
use crate::peripherals::bxcan::BxCan;
use crate::peripherals::fdcan::Fdcan;

impl SystemBus {
    /// `true` when `can_id` names an on-chip controller with a timed mode
    /// (bxCAN, FDCAN). A CAN device below an SPI controller (MCP2515) has
    /// none.
    pub fn can_timed_capable(&self, can_id: &str) -> bool {
        self.find_peripheral_index_by_name(can_id)
            .and_then(|idx| self.peripherals[idx].dev.as_any())
            .is_some_and(|any| any.is::<BxCan>() || any.is::<Fdcan>())
    }

    /// Put the controller `can_id` on a timed CAN bus.
    pub fn attach_timed_can_by_id(&mut self, can_id: &str) -> anyhow::Result<()> {
        let idx = self
            .find_peripheral_index_by_name(can_id)
            .ok_or_else(|| anyhow::anyhow!("no CAN endpoint '{can_id}'"))?;
        let any = self.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| anyhow::anyhow!("peripheral '{can_id}' is not a CAN controller"))?;
        if let Some(bx) = any.downcast_mut::<BxCan>() {
            bx.attach_timed()?;
        } else if let Some(fd) = any.downcast_mut::<Fdcan>() {
            fd.attach_timed()?;
        } else {
            anyhow::bail!("peripheral '{can_id}' is not a CAN controller with a timed mode");
        }
        // Timed controllers keep the walk on (their interrupt lines are
        // levels held by `tick`); see `attach_can_endpoint_by_id`.
        self.rebuild_peripheral_ranges();
        self.recompute_walk_deletable();
        Ok(())
    }

    /// The controller `can_id` as the timed bus needs it. A controller whose
    /// bus clock is gated off is off the bus.
    pub fn can_timed_poll(&self, can_id: &str) -> Option<CanNodePoll> {
        let idx = self.find_peripheral_index_by_name(can_id)?;
        let clocked = self.is_peripheral_clocked(idx);
        let any = self.peripherals[idx].dev.as_any()?;
        let mut poll = if let Some(bx) = any.downcast_ref::<BxCan>() {
            bx.timed_poll(self.apb1_hz())
        } else {
            any.downcast_ref::<Fdcan>()?.timed_poll()
        };
        poll.on_bus &= clocked;
        Some(poll)
    }

    /// Apply a timed-bus update to the controller `can_id`.
    pub fn can_timed_apply(&mut self, can_id: &str, update: &CanNodeUpdate) {
        let Some(idx) = self.find_peripheral_index_by_name(can_id) else {
            return;
        };
        let Some(any) = self.peripherals[idx].dev.as_any_mut() else {
            return;
        };
        if let Some(bx) = any.downcast_mut::<BxCan>() {
            bx.timed_apply(update);
        } else if let Some(fd) = any.downcast_mut::<Fdcan>() {
            fd.timed_apply(update);
        }
        self.collect_scheduled_events(idx);
    }

    /// PCLK1, the bxCAN clock: HCLK (the core clock) divided by the APB1
    /// prescaler in RCC_CFGR.PPRE1 — bits 10:8 on the F1 (RM0008 §7.3.2)
    /// and L4 (RM0351 §6.4.3), 12:10 on the F4 (RM0090 §7.3.3). 0xx divides
    /// by 1, 100 by 2, 101 by 4, 110 by 8, 111 by 16. Without an RCC of
    /// those families the core clock is used.
    fn apb1_hz(&self) -> u64 {
        use crate::peripherals::rcc::Rcc;
        let rcc = self
            .rcc_idx
            .and_then(|idx| self.peripherals.get(idx))
            .and_then(|p| p.dev.as_any())
            .and_then(|any| any.downcast_ref::<Rcc>());
        let Some(rcc) = rcc else {
            return self.cpu_hz;
        };
        let (offset, shift) = match rcc {
            Rcc::Stm32F1(_) => (0x04, 8),
            Rcc::Stm32L4(_) => (0x08, 8),
            Rcc::Stm32F4(_) => (0x08, 10),
            _ => return self.cpu_hz,
        };
        let cfgr = crate::Peripheral::read_u32(rcc, offset).unwrap_or(0);
        let ppre = (cfgr >> shift) & 0x7;
        let div = if ppre < 4 { 1 } else { 1u64 << (ppre - 3) };
        self.cpu_hz / div
    }
}
