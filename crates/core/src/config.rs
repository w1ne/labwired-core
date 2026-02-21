// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationConfig {
    /// Enable the instruction decode cache for the CPU core.
    pub decode_cache_enabled: bool,
    /// Interval in instructions for ticking peripherals (1 = every instruction).
    pub peripheral_tick_interval: u32,
    /// Enable optimized multi-byte memory access paths in the SystemBus.
    pub optimized_bus_access: bool,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            decode_cache_enabled: true,
            peripheral_tick_interval: 16, // Optimized default
            optimized_bus_access: true,
        }
    }
}
