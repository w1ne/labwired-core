// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

pub mod shm;

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;

/// A peripheral that proxies its operations to an external process via IPC.
/// This is used for high-performance co-simulation with RTL models (e.g. Verilator).
#[derive(Debug)]
pub struct CosimPeripheral {
    pub name: String,
    // Add IPC transport (e.g. SharedMemory)
}

impl Peripheral for CosimPeripheral {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        // TODO: Propose transaction over IPC and wait for result
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // TODO: Propose transaction over IPC
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // TODO: Sync simulation time with external process
        PeripheralTickResult::default()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}
