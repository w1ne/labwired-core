#![allow(clippy::manual_is_multiple_of)]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

pub mod bus;
pub mod config;
pub mod cosim;
pub mod cpu;
pub mod decoder;
pub mod interrupt;
pub mod memory;
pub mod metrics;
pub mod multi_core;
pub mod network;
pub mod peripherals;
pub mod physics;
pub mod signals;
pub mod snapshot;
pub mod system;
pub mod vfi;
pub mod world;

pub use config::SimulationConfig;

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
    #[error("Simulation halted")]
    Halt,
    #[error("Simulation error: {0}")]
    Other(String),
}

pub type SimResult<T> = Result<T, SimulationError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaDirection {
    Read,
    Write,
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaRequest {
    pub src_addr: u64,
    pub addr: u64,
    pub val: u8,
    pub direction: DmaDirection,
}

#[derive(Debug, Clone, Default)]
pub struct PeripheralTickResult {
    pub irq: bool,
    pub cycles: u32,
    pub dma_requests: Option<Vec<DmaRequest>>,
    pub explicit_irqs: Option<Vec<u32>>,
    pub dma_signals: Option<Vec<u32>>,
    pub ticks_until_next: Option<u64>,
}

impl PeripheralTickResult {
    pub fn with_irq(irq: bool) -> Self {
        Self {
            irq,
            ..Default::default()
        }
    }
}

/// Trait for observing simulation events in a modular way.
pub trait SimulationObserver: std::fmt::Debug + Send + Sync {
    fn on_simulation_start(&self) {}
    fn on_simulation_stop(&self) {}
    fn on_step_start(&self, _pc: u32, _opcode: u32) {}
    fn on_step_end(&self, _cycles: u32) {}
    fn on_memory_write(&self, _addr: u64, _old: u8, _new: u8) {}
    fn on_peripheral_tick(&self, _name: &str, _cycles: u32) {}
}

/// Trait representing a CPU architecture
pub trait Cpu: Send {
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()>;
    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()>;
    fn step_batch(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        for _ in 0..max_count {
            self.step(bus, observers, config)?;
        }
        Ok(max_count)
    }
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
    fn index_of_register(&self, name: &str) -> Option<u8>;

    // Security & Physical Extensions
    fn inject_fault(&mut self, _target: &str) -> SimResult<()> {
        Ok(())
    }
    fn get_energy_consumption(&self) -> f64 {
        0.0
    }
}

/// Trait representing a memory-mapped peripheral
pub trait Peripheral: std::fmt::Debug + Send {
    fn read(&self, offset: u64) -> SimResult<u8>;
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()>;
    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        let b0 = self.read(offset)? as u16;
        let b1 = self.read(offset + 1)? as u16;
        Ok(b0 | (b1 << 8))
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let b0 = self.read(offset)? as u32;
        let b1 = self.read(offset + 1)? as u32;
        let b2 = self.read(offset + 2)? as u32;
        let b3 = self.read(offset + 3)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
        Ok(())
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write(offset + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write(offset + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
    }
    /// Side-effect-free value probe used for debug/observer bookkeeping.
    /// Implementations should return `None` when such probing is not supported.
    fn peek(&self, _offset: u64) -> Option<u8> {
        None
    }
    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
    }
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
    fn dma_request(&mut self, _request_id: u32) {}
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
    fn config(&self) -> &SimulationConfig;
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }

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
    fn get_peripherals(&self) -> Vec<(String, u64, u64)>;
    fn get_peripheral_descriptor(
        &self,
        name: &str,
    ) -> Option<labwired_config::PeripheralDescriptor>;
    fn reset(&mut self) -> SimResult<()>;

    // State Management
    fn snapshot(&self) -> snapshot::MachineSnapshot;
    fn restore(&mut self, snapshot: &snapshot::MachineSnapshot) -> SimResult<()>;
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
    pub breakpoints: std::collections::HashSet<u32>,
    pub last_breakpoint: Option<u32>,
    pub total_cycles: u64,
    pub config: SimulationConfig,
}

impl<C: Cpu> Machine<C> {
    pub fn new(cpu: C, bus: bus::SystemBus) -> Self {
        Self {
            cpu,
            bus,
            observers: Vec::new(),
            breakpoints: HashSet::new(),
            last_breakpoint: None,
            total_cycles: 0,
            config: SimulationConfig::default(),
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
        self.total_cycles += 1;
        self.cpu
            .step(&mut self.bus, &self.observers, &self.config)?;

        if self.total_cycles % (self.config.peripheral_tick_interval as u64) == 0 {
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
        }

        Ok(())
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
            // Check breakpoints BEFORE batch
            let pc = self.cpu.get_pc();
            let pc_aligned = pc & !1;

            if self.breakpoints.contains(&pc_aligned) && self.last_breakpoint != Some(pc_aligned) {
                self.last_breakpoint = Some(pc_aligned);
                return Ok(StopReason::Breakpoint(pc));
            }

            // We are executing, so clear the "last hit" sticky BP
            self.last_breakpoint = None;

            if let Some(limit) = max_steps {
                if steps >= limit {
                    return Ok(StopReason::MaxStepsReached);
                }
            }

            // Execute in batch until next peripheral tick or breakpoint/limit
            let current_cycles = self.total_cycles;
            let tick_interval = self.config.peripheral_tick_interval as u64;
            let remaining_until_tick = (tick_interval - (current_cycles % tick_interval)) as u32;

            let current_batch = if let Some(limit) = max_steps {
                remaining_until_tick.min(limit - steps)
            } else {
                remaining_until_tick
            };

            let executed =
                self.cpu
                    .step_batch(&mut self.bus, &self.observers, &self.config, current_batch)?;

            steps += executed;
            self.total_cycles += executed as u64;

            if self.total_cycles % (self.config.peripheral_tick_interval as u64) == 0 {
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
            }

            // If we executed less than requested, it means the CPU wanted to exit early (e.g. branch/exception)
            // or we just finished the batch naturally. The loop will continue and check breakpoints/limits.
            if executed == 0 && current_batch > 0 {
                // If the CPU makes no progress but says Ok(0), we might need to investigate.
                // For now, assume it's valid (e.g. waiting for something).
                break;
            }
        }
        Ok(StopReason::StepDone)
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

    fn get_peripherals(&self) -> Vec<(String, u64, u64)> {
        self.bus
            .peripherals
            .iter()
            .map(|p| (p.name.clone(), p.base, p.size))
            .collect()
    }

    fn get_peripheral_descriptor(
        &self,
        name: &str,
    ) -> Option<labwired_config::PeripheralDescriptor> {
        use crate::peripherals::declarative::GenericPeripheral;
        let entry = self.bus.peripherals.iter().find(|p| p.name == name)?;
        let gen_p = entry.dev.as_any()?.downcast_ref::<GenericPeripheral>()?;
        // We need a way to get the descriptor from GenericPeripheral.
        // It's private currently. Let's make it public or add a getter.
        Some(gen_p.get_descriptor().clone())
    }

    fn reset(&mut self) -> SimResult<()> {
        self.cpu.reset(&mut self.bus)
    }

    fn snapshot(&self) -> snapshot::MachineSnapshot {
        self.snapshot()
    }

    fn restore(&mut self, snapshot: &snapshot::MachineSnapshot) -> SimResult<()> {
        self.apply_snapshot(snapshot.clone())
    }
}
