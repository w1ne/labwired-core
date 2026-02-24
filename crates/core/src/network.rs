// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;

/// Trait for virtual interconnects between machines.
pub trait Interconnect: Send {
    /// Advance the interconnect state.
    fn tick(&mut self) -> SimResult<()>;
}

/// A simple cross-link between two UART peripherals.
pub struct UartCrossLink {
    pub node_a: String,
    pub node_b: String,
    // Add buffers and peripheral references
}

impl Interconnect for UartCrossLink {
    fn tick(&mut self) -> SimResult<()> {
        // TODO: Move bytes between node buffers
        Ok(())
    }
}
