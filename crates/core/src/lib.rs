// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

pub mod bus;
pub mod cpu;
pub mod decoder;
pub mod interrupt;
pub mod memory;
pub mod metrics;
pub mod multi_core;
pub mod peripherals;
pub mod signals;
pub mod snapshot;
pub mod system;

use std::any::Any;
use std::sync::Arc;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Arch {
    Arm,
    RiscV,
    Unknown,
}

#[derive(Debug, thiserror::Error)]
pub enum SimulationError {
    #[error("Memory access violation at {0:#x}")]
    MemoryViolation(u64),
    #[error("Instruction decoding error at {0:#x}")]
    DecodeError(u64),
}

pub type SimResult<T> = Result<T, SimulationError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaDirection {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaRequest {
    pub addr: u64,
    pub val: u8,
    pub direction: DmaDirection,
}

#[derive(Debug, Clone, Default)]
pub struct PeripheralTickResult {
    pub irq: bool,
    pub cycles: u32,
    pub dma_requests: Vec<DmaRequest>,
    pub explicit_irqs: Vec<u32>,
}

/// Trait for observing simulation events in a modular way.
pub trait SimulationObserver: std::fmt::Debug + Send + Sync {
    fn on_simulation_start(&self) {}
    fn on_simulation_stop(&self) {}
    fn on_step_start(&self, _pc: u32, _opcode: u32) {}
    fn on_step_end(&self, _cycles: u32) {}
    fn on_peripheral_tick(&self, _name: &str, _cycles: u32) {}
}

/// Trait representing a CPU architecture
pub trait Cpu: Send {
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()>;
    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
    ) -> SimResult<()>;
    fn set_pc(&mut self, val: u32);
    fn get_pc(&self) -> u32;
    fn set_sp(&mut self, val: u32);
    fn set_exception_pending(&mut self, exception_num: u32);

    // Debug Access
    fn get_register(&self, id: u8) -> u32;
    fn set_register(&mut self, id: u8, val: u32);
    fn snapshot(&self) -> snapshot::CpuSnapshot;
    fn apply_snapshot(&mut self, snapshot: &snapshot::CpuSnapshot);
    fn get_register_names(&self) -> Vec<String>;
}

/// Trait representing a memory-mapped peripheral
pub trait Peripheral: std::fmt::Debug + Send {
    fn read(&self, offset: u64) -> SimResult<u8>;
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()>;
    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
    }
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn restore(&mut self, _state: serde_json::Value) -> SimResult<()> {
        Ok(())
    }
}

/// Trait representing the system bus
pub trait Bus {
    fn read_u8(&self, addr: u64) -> SimResult<u8>;
    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()>;
    fn tick_peripherals(&mut self) -> Vec<u32>; // Returns list of pending exception numbers
    fn execute_dma(&mut self, requests: &[DmaRequest]) -> SimResult<()>;

    fn read_u16(&self, addr: u64) -> SimResult<u16> {
        let b0 = self.read_u8(addr)? as u16;
        let b1 = self.read_u8(addr + 1)? as u16;
        // Little Endian
        Ok(b0 | (b1 << 8))
    }

    fn read_u32(&self, addr: u64) -> SimResult<u32> {
        let b0 = self.read_u8(addr)? as u32;
        let b1 = self.read_u8(addr + 1)? as u32;
        let b2 = self.read_u8(addr + 2)? as u32;
        let b3 = self.read_u8(addr + 3)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }

    fn write_u32(&mut self, addr: u64, value: u32) -> SimResult<()> {
        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write_u8(addr + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write_u8(addr + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
    }

    fn write_u16(&mut self, addr: u64, value: u16) -> SimResult<()> {
        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        Ok(())
    }
}

use std::collections::HashSet;

/// Trait for controlling the machine in debug mode
pub trait DebugControl {
    fn add_breakpoint(&mut self, addr: u32);
    fn remove_breakpoint(&mut self, addr: u32);
    fn clear_breakpoints(&mut self);

    /// Run until breakpoint or steps limit
    fn run(&mut self, max_steps: Option<u32>) -> SimResult<StopReason>;

    /// Step a single instruction
    fn step_single(&mut self) -> SimResult<StopReason>;

    fn read_core_reg(&self, id: u8) -> u32;
    fn write_core_reg(&mut self, id: u8, val: u32);

    fn read_memory(&self, addr: u32, len: usize) -> SimResult<Vec<u8>>;
    fn write_memory(&mut self, addr: u32, data: &[u8]) -> SimResult<()>;

    fn get_pc(&self) -> u32;
    fn set_pc(&mut self, addr: u32);
    fn get_register_names(&self) -> Vec<String>;
    fn get_cycle_count(&self) -> u64;
    fn reset(&mut self) -> SimResult<()>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum StopReason {
    Breakpoint(u32),
    StepDone,
    MaxStepsReached,
    ManualStop,
}

pub struct Machine<C: Cpu> {
    pub cpu: C,
    pub bus: bus::SystemBus,
    pub observers: Vec<Arc<dyn SimulationObserver>>,

    // Debug state
    pub breakpoints: HashSet<u32>,
    pub total_cycles: u64,
}

impl<C: Cpu> Machine<C> {
    pub fn new(cpu: C, bus: bus::SystemBus) -> Self {
        Self {
            cpu,
            bus,
            observers: Vec::new(),
            breakpoints: HashSet::new(),
            total_cycles: 0,
        }
    }
}

impl<C: Cpu> Machine<C> {
    pub fn load_firmware(&mut self, image: &memory::ProgramImage) -> SimResult<()> {
        for segment in &image.segments {
            // Try loading into Flash first
            if !self.bus.flash.load_from_segment(segment) {
                // If not flash, try RAM? Or just warn?
                // For now, let's assume everything goes to Flash or RAM mapped spaces
                if !self.bus.ram.load_from_segment(segment) {
                    tracing::warn!(
                        "Failed to load segment at {:#x} - outside of memory map",
                        segment.start_addr
                    );
                }
            }
        }

        for observer in &self.observers {
            observer.on_simulation_start();
        }
        self.reset()?;

        // Fallback if vector table is missing/zero
        if self.cpu.get_pc() == 0 {
            self.cpu.set_pc(image.entry_point as u32);
        }

        Ok(())
    }

    pub fn reset(&mut self) -> SimResult<()> {
        self.cpu.reset(&mut self.bus)
    }

    pub fn step(&mut self) -> SimResult<()> {
        self.total_cycles += 1; // Base instruction cycle
        let res = self.cpu.step(&mut self.bus, &self.observers);

        // Propagate peripherals
        let (interrupts, costs) = self.bus.tick_peripherals_fully();
        for c in costs {
            self.total_cycles += c.cycles as u64;
            if let Some(p) = self.bus.peripherals.get(c.index) {
                for observer in &self.observers {
                    observer.on_peripheral_tick(&p.name, c.cycles);
                }
            }
        }
        for irq in interrupts {
            self.cpu.set_exception_pending(irq);
            tracing::debug!("Exception {} Pend", irq);
        }

        res
    }

    pub fn snapshot(&self) -> snapshot::MachineSnapshot {
        snapshot::MachineSnapshot {
            cpu: self.cpu.snapshot(),
            peripherals: self
                .bus
                .peripherals
                .iter()
                .map(|p| (p.name.clone(), p.dev.snapshot()))
                .collect(),
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: snapshot::MachineSnapshot) -> SimResult<()> {
        self.cpu.apply_snapshot(&snapshot.cpu);
        for p in &mut self.bus.peripherals {
            if let Some(state) = snapshot.peripherals.get(&p.name) {
                p.dev.restore(state.clone())?;
            }
        }
        Ok(())
    }

    pub fn peek_peripheral(&self, name: &str) -> Option<serde_json::Value> {
        self.bus
            .peripherals
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.dev.snapshot())
    }
}

impl<C: Cpu> DebugControl for Machine<C> {
    fn add_breakpoint(&mut self, addr: u32) {
        self.breakpoints.insert(addr);
    }

    fn remove_breakpoint(&mut self, addr: u32) {
        self.breakpoints.remove(&addr);
    }

    fn clear_breakpoints(&mut self) {
        self.breakpoints.clear();
    }

    fn run(&mut self, max_steps: Option<u32>) -> SimResult<StopReason> {
        let mut steps = 0;
        loop {
            // Check breakpoints BEFORE stepping
            let pc = self.cpu.get_pc();
            // Note: breakpoints typically match the exact PC.
            // Thumb instructions are at even addresses, usually.
            // If the user sets a BP at an odd address (Thumb function pointer), we should mask it?
            // Usually DAP clients send the symbol address.
            // Let's assume exact match for now, but mask LSB.
            let pc_aligned = pc & !1;

            if self.breakpoints.contains(&pc_aligned) {
                return Ok(StopReason::Breakpoint(pc));
            }

            self.step()?;
            steps += 1;

            if let Some(max) = max_steps {
                if steps >= max {
                    return Ok(StopReason::MaxStepsReached);
                }
            }
        }
    }

    fn step_single(&mut self) -> SimResult<StopReason> {
        self.step()?;
        Ok(StopReason::StepDone)
    }

    fn read_core_reg(&self, id: u8) -> u32 {
        self.cpu.get_register(id)
    }

    fn write_core_reg(&mut self, id: u8, val: u32) {
        self.cpu.set_register(id, val);
    }

    fn read_memory(&self, addr: u32, len: usize) -> SimResult<Vec<u8>> {
        let mut data = Vec::with_capacity(len);
        for i in 0..len {
            let byte = self.bus.read_u8((addr as u64) + (i as u64))?;
            data.push(byte);
        }
        Ok(data)
    }

    fn write_memory(&mut self, addr: u32, data: &[u8]) -> SimResult<()> {
        for (i, byte) in data.iter().enumerate() {
            self.bus.write_u8((addr as u64) + (i as u64), *byte)?;
        }
        Ok(())
    }

    fn get_pc(&self) -> u32 {
        self.cpu.get_pc()
    }

    fn set_pc(&mut self, addr: u32) {
        self.cpu.set_pc(addr);
    }

    fn get_register_names(&self) -> Vec<String> {
        self.cpu.get_register_names()
    }

    fn get_cycle_count(&self) -> u64 {
        self.total_cycles
    }

    fn reset(&mut self) -> SimResult<()> {
        self.cpu.reset(&mut self.bus)
    }
}
