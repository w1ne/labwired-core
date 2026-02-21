// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{bus::SystemBus, Bus, Cpu, SimulationObserver};
use std::sync::Arc;

/// A machine that supports multiple CPU cores.
///
/// This structure evolves the standard `Machine` to support modern multi-core
/// MCUs like the ESP32 (dual Xtensa/RISC-V) or RP2040 (dual Cortex-M0+).
pub struct MultiCoreMachine {
    pub cores: Vec<Box<dyn Cpu>>,
    pub bus: SystemBus,
    pub observers: Vec<Arc<dyn SimulationObserver>>,
}

impl MultiCoreMachine {
    pub fn new(bus: SystemBus) -> Self {
        Self {
            cores: Vec::new(),
            bus,
            observers: Vec::new(),
        }
    }

    pub fn add_core(&mut self, core: Box<dyn Cpu>) {
        self.cores.push(core);
    }

    /// Step all cores in the machine.
    /// In a more advanced simulation, this would handle cycle-accurate synchronization.
    pub fn step_all(&mut self) -> Vec<crate::SimResult<()>> {
        let mut results = Vec::new();
        for core in &mut self.cores {
            results.push(core.step(&mut self.bus, &self.observers));
        }

        // Tick peripherals once after all cores have stepped
        let interrupts = self.bus.tick_peripherals();
        // Distribute interrupts to cores. For now, route all to core 0.
        // In fully mature models, an external interrupt controller (like GIC/PLIC) handles routing.
        if !interrupts.is_empty() && !self.cores.is_empty() {
            for irq in interrupts {
                self.cores[0].set_exception_pending(irq);
            }
        }

        results
    }
}
