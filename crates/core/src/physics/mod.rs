// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

pub mod fmi;

/// Interface for physical environmental models.
pub trait PhysicalModel: Send {
    /// Advance the physical state by a time step.
    fn step(&mut self, dt_ns: u64);
}
