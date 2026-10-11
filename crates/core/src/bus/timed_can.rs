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

impl SystemBus {
    /// `true` when `can_id` names an on-chip controller with a timed mode
    /// (bxCAN, FDCAN: [`crate::Peripheral::timed_can`]). A CAN device below
    /// an SPI controller (MCP2515) has none.
    pub fn can_timed_capable(&self, can_id: &str) -> bool {
        self.find_peripheral_index_by_name(can_id)
            .is_some_and(|idx| self.peripherals[idx].dev.timed_can().is_some())
    }

    /// Put the controller `can_id` on a timed CAN bus.
    pub fn attach_timed_can_by_id(&mut self, can_id: &str) -> anyhow::Result<()> {
        let idx = self
            .find_peripheral_index_by_name(can_id)
            .ok_or_else(|| anyhow::anyhow!("no CAN endpoint '{can_id}'"))?;
        self.peripherals[idx]
            .dev
            .timed_can_mut()
            .ok_or_else(|| {
                anyhow::anyhow!("peripheral '{can_id}' is not a CAN controller with a timed mode")
            })?
            .attach_timed()?;
        // The controller's walk and scheduler needs changed with its mode;
        // see `attach_can_endpoint_by_id`.
        self.rebuild_peripheral_ranges();
        self.recompute_walk_deletable();
        Ok(())
    }

    /// The controller `can_id` as the timed bus needs it. A controller whose
    /// bus clock is gated off is off the bus.
    pub fn can_timed_poll(&self, can_id: &str) -> Option<CanNodePoll> {
        let idx = self.find_peripheral_index_by_name(can_id)?;
        let clocked = self.is_peripheral_clocked(idx);
        let mut poll = self.peripherals[idx]
            .dev
            .timed_can()?
            .timed_poll(self.apb1_hz());
        poll.on_bus &= clocked;
        Some(poll)
    }

    /// Apply a timed-bus update to the controller `can_id`.
    pub fn can_timed_apply(&mut self, can_id: &str, update: &CanNodeUpdate) {
        let Some(idx) = self.find_peripheral_index_by_name(can_id) else {
            return;
        };
        let Some(controller) = self.peripherals[idx].dev.timed_can_mut() else {
            return;
        };
        controller.timed_apply(update);
        self.collect_scheduled_events(idx);
    }

    /// PCLK1, the bxCAN clock: HCLK (the core clock) divided by the APB1
    /// prescaler in RCC_CFGR.PPRE1. The RCC family is told apart by where
    /// its APB1 enable register sits ([`crate::Peripheral::clock_gate_reg_offset`]):
    /// 0x1C on the F1 (CFGR 0x04, PPRE1 bits 10:8, RM0008 §7.3.2), 0x40 on the
    /// F4 (CFGR 0x08, bits 12:10, RM0090 §7.3.3), 0x58 on the L4 and G4 (CFGR
    /// 0x08, bits 10:8, RM0351 §6.4.3, RM0440 §7.4.3). 0xx divides by 1, 100 by
    /// 2, 101 by 4, 110 by 8, 111 by 16. Any other clock controller, or none,
    /// gives the core clock.
    fn apb1_hz(&self) -> u64 {
        let Some(rcc) = self.rcc_idx.and_then(|idx| self.peripherals.get(idx)) else {
            return self.cpu_hz;
        };
        let (cfgr, shift) = match rcc.dev.clock_gate_reg_offset("apb1enr") {
            Some(0x1C) => (0x04, 8),
            Some(0x40) => (0x08, 10),
            Some(0x58) => (0x08, 8),
            _ => return self.cpu_hz,
        };
        let ppre = (rcc.dev.read_u32(cfgr).unwrap_or(0) >> shift) & 0x7;
        let div = if ppre < 4 { 1 } else { 1u64 << (ppre - 3) };
        self.cpu_hz / div
    }
}
