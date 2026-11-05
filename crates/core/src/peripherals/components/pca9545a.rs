// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NXP PCA9545A four-channel I²C switch with interrupt status.
//!
//! The low four control bits select channels after STOP. The upper four bits
//! report the active-low interrupt inputs and ignore writes. Bus routing and
//! open-drain collisions share the existing TCA9548A implementation. Children
//! keep advancing while their channels are isolated. RESET clears the mask.
//!
//! Source https://www.nxp.com/docs/en/data-sheet/PCA9545A_45B_45C.pdf
//! The byte protocol is modeled. Pass-gate resistance, pull-ups, line rise time,
//! and external GPIO wiring of RESET/INT require a board-level connection.

use crate::peripherals::device::I2cBusSwitch;
use crate::peripherals::i2c::I2cDevice;

use super::tca9548a::Tca9548a;

pub const PCA9545A_CHANNELS: usize = 4;
pub const PCA9545A_BASE_ADDRESS: u8 = 0x70;

#[derive(Debug)]
pub struct Pca9545a {
    routing: Tca9548a,
    selected_control: bool,
    pending_control: Option<u8>,
    interrupt_mask: u8,
    reset_asserted: bool,
}

impl Pca9545a {
    pub fn new(address: u8) -> Self {
        Self {
            routing: Tca9548a::new(PCA9545A_BASE_ADDRESS | (address & 0x03)),
            selected_control: false,
            pending_control: None,
            interrupt_mask: 0,
            reset_asserted: false,
        }
    }

    pub fn attach(&mut self, channel: u8, device: Box<dyn I2cDevice>) -> anyhow::Result<()> {
        anyhow::ensure!(
            usize::from(channel) < PCA9545A_CHANNELS,
            "PCA9545A channel {channel} is outside 0..=3"
        );
        self.routing.attach(channel, device)
    }

    pub fn channel_enabled(&self, channel: u8) -> bool {
        self.routing.channel_enabled(channel)
    }

    pub fn channel_devices(&self, channel: u8) -> &[Box<dyn I2cDevice>] {
        self.routing.channel_devices(channel)
    }

    pub fn control_register(&self) -> u8 {
        self.pending_control
            .unwrap_or_else(|| self.routing.control_register())
            | (self.interrupt_mask << 4)
    }

    pub fn set_interrupt_asserted(&mut self, channel: u8, asserted: bool) -> anyhow::Result<()> {
        anyhow::ensure!(
            usize::from(channel) < PCA9545A_CHANNELS,
            "invalid interrupt channel"
        );
        let bit = 1 << channel;
        self.interrupt_mask = (self.interrupt_mask & !bit) | (u8::from(asserted) << channel);
        Ok(())
    }

    pub fn interrupt_asserted(&self) -> bool {
        self.interrupt_mask != 0
    }

    pub fn set_reset_asserted(&mut self, asserted: bool) {
        self.reset_asserted = asserted;
        if asserted {
            self.pending_control = None;
            self.selected_control = false;
            self.routing.set_control_register(0);
            self.routing.select_address(0xff);
        }
    }
}

impl Default for Pca9545a {
    fn default() -> Self {
        Self::new(PCA9545A_BASE_ADDRESS)
    }
}

impl I2cBusSwitch for Pca9545a {
    fn channel_count(&self) -> usize {
        PCA9545A_CHANNELS
    }

    fn attach(&mut self, channel: u8, device: Box<dyn I2cDevice>) -> anyhow::Result<()> {
        Pca9545a::attach(self, channel, device)
    }
}

impl I2cDevice for Pca9545a {
    fn as_bus_switch(&self) -> Option<&dyn I2cBusSwitch> {
        Some(self)
    }

    fn as_bus_switch_mut(&mut self) -> Option<&mut dyn I2cBusSwitch> {
        Some(self)
    }

    fn address(&self) -> u8 {
        self.routing.address()
    }

    fn claims_address(&self, address: u8) -> bool {
        !self.reset_asserted && self.routing.claims_address(address)
    }

    fn select_address(&mut self, address: u8) {
        self.selected_control = address == self.address() && !self.reset_asserted;
        self.routing
            .select_address(if self.reset_asserted { 0xff } else { address });
    }

    fn start(&mut self) {
        self.routing.start();
    }

    fn stop(&mut self) {
        self.routing.stop();
        if let Some(control) = self.pending_control.take() {
            self.routing.set_control_register(control);
        }
    }

    fn write(&mut self, data: u8) {
        if self.selected_control {
            self.pending_control = Some(data & 0x0f);
        }
        self.routing.write_downstream(data);
    }

    fn read(&mut self) -> u8 {
        self.routing
            .read_with_control_value(self.control_register())
    }

    fn advance_time_us(&mut self, us: u64) {
        self.routing.advance_time_us(us);
    }

    fn for_each_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        self.routing.for_each_sim_input(f)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}
