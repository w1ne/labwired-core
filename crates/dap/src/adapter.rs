// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use anyhow::{anyhow, Result};
use labwired_core::{DebugControl, Machine};
use labwired_loader::SymbolProvider;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use crate::trace::{TraceBuffer, InstructionTrace};

#[derive(Clone, Serialize)]
pub struct TelemetryData {
    pub pc: u32,
    pub cycles: u64,
    pub mips: f64,
    pub registers: std::collections::HashMap<String, u32>,
}

#[derive(Clone)]
pub struct LabwiredAdapter {
    pub machine: Arc<Mutex<Option<Box<dyn DebugControl + Send>>>>,
    pub symbols: Arc<Mutex<Option<SymbolProvider>>>,
    pub uart_sink: Arc<Mutex<Vec<u8>>>,
    pub last_telemetry: Arc<Mutex<(u64, Instant)>>, // cycles, time
    pub trace_buffer: Arc<Mutex<TraceBuffer>>,
    pub cycle_count: Arc<AtomicU64>,
}

impl Default for LabwiredAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LabwiredAdapter {
    pub fn new() -> Self {
        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        Self {
            machine: Arc::new(Mutex::new(None)),
            symbols: Arc::new(Mutex::new(None)),
            uart_sink,
            last_telemetry: Arc::new(Mutex::new((0, Instant::now()))),
            trace_buffer: Arc::new(Mutex::new(TraceBuffer::new(100_000))),
            cycle_count: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn get_telemetry(&self) -> Option<TelemetryData> {
        let machine_guard = self.machine.lock().unwrap();
        let machine = machine_guard.as_ref()?;

        let pc = machine.read_core_reg(15);
        let cycles = machine.get_cycle_count();

        let mut last_guard = self.last_telemetry.lock().unwrap();
        let (last_cycles, last_time) = *last_guard;
        let now = Instant::now();
        let elapsed = now.duration_since(last_time).as_secs_f64();

        let mips = if elapsed > 0.0 {
            let delta = cycles.saturating_sub(last_cycles);
            (delta as f64 / elapsed) / 1_000_000.0
        } else {
            0.0
        };

        *last_guard = (cycles, now);

        let mut registers = std::collections::HashMap::new();
        let names = machine.get_register_names();
        for (i, name) in names.iter().enumerate().take(16) {
            let val = machine.read_core_reg(i as u8);
            registers.insert(name.clone(), val);
        }

        Some(TelemetryData {
            pc,
            cycles,
            mips,
            registers,
        })
    }

    pub fn load_firmware(
        &self,
        firmware_path: PathBuf,
        system_path: Option<PathBuf>,
    ) -> Result<()> {
        let image = labwired_loader::load_elf(&firmware_path)?;

        let mut bus = if let Some(sys_path) = &system_path {
            let manifest = labwired_config::SystemManifest::from_file(sys_path)?;
            let chip_path = sys_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(&manifest.chip);
            let chip = labwired_config::ChipDescriptor::from_file(&chip_path)?;
            labwired_core::bus::SystemBus::from_config(&chip, &manifest)?
        } else {
            labwired_core::bus::SystemBus::new()
        };

        let arch = match image.arch {
            labwired_core::Arch::Arm => labwired_core::Arch::Arm,
            labwired_core::Arch::RiscV => labwired_core::Arch::RiscV,
            _ => {
                // Fallback or guess from system config?
                // For now, assume Arm if unclear
                labwired_core::Arch::Arm
            }
        };

        match arch {
            labwired_core::Arch::Arm => {
                let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
                bus.attach_uart_tx_sink(self.uart_sink.clone(), false);
                let mut machine = Machine::new(cpu, bus);
                machine
                    .load_firmware(&image)
                    .map_err(|e| anyhow!("Failed to load firmware: {:?}", e))?;
                *self.machine.lock().unwrap() = Some(Box::new(machine));
            }
            labwired_core::Arch::RiscV => {
                let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
                bus.attach_uart_tx_sink(self.uart_sink.clone(), false);
                let mut machine = Machine::new(cpu, bus);
                machine
                    .load_firmware(&image)
                    .map_err(|e| anyhow!("Failed to load firmware: {:?}", e))?;
                *self.machine.lock().unwrap() = Some(Box::new(machine));
            }
            _ => return Err(anyhow!("Unsupported architecture: {:?}", arch)),
        }

        // Load symbols
        if let Ok(syms) = SymbolProvider::new(&firmware_path) {
            *self.symbols.lock().unwrap() = Some(syms);
        } else {
            tracing::warn!(
                "No debug symbols found or failed to parse: {:?}",
                firmware_path
            );
        }

        Ok(())
    }

    pub fn lookup_source(&self, addr: u64) -> Option<labwired_loader::SourceLocation> {
        self.symbols.lock().unwrap().as_ref()?.lookup(addr)
    }

    pub fn lookup_source_reverse(&self, path: &str, line: u32) -> Option<u64> {
        self.symbols.lock().unwrap().as_ref()?.location_to_pc(path, line)
    }

    pub fn get_pc(&self) -> Result<u32> {
        let guard = self.machine.lock().unwrap();
        if let Some(machine) = guard.as_ref() {
            Ok(machine.get_pc())
        } else {
            Err(anyhow!("Machine not initialized"))
        }
    }

    pub fn get_register(&self, id: u8) -> Result<u32> {
        let guard = self.machine.lock().unwrap();
        if let Some(machine) = guard.as_ref() {
            Ok(machine.read_core_reg(id))
        } else {
            Err(anyhow!("Machine not initialized"))
        }
    }

    pub fn set_register(&self, id: u8, val: u32) -> Result<()> {
        let mut guard = self.machine.lock().unwrap();
        if let Some(machine) = guard.as_mut() {
            machine.write_core_reg(id, val);
            Ok(())
        } else {
            Err(anyhow!("Machine not initialized"))
        }
    }

    pub fn set_pc(&self, addr: u32) -> Result<()> {
        let mut guard = self.machine.lock().unwrap();
        if let Some(machine) = guard.as_mut() {
            machine.set_pc(addr);
            Ok(())
        } else {
            Err(anyhow!("Machine not initialized"))
        }
    }

    pub fn reset(&self) -> Result<()> {
        let mut guard = self.machine.lock().unwrap();
        if let Some(machine) = guard.as_mut() {
            machine.reset().map_err(|e| anyhow!("Reset failed: {:?}", e))
        } else {
            Err(anyhow!("Machine not initialized"))
        }
    }

    pub fn get_register_names(&self) -> Result<Vec<String>> {
        let guard = self.machine.lock().unwrap();
        if let Some(machine) = guard.as_ref() {
            Ok(machine.get_register_names())
        } else {
            Err(anyhow!("Machine not initialized"))
        }
    }

    pub fn poll_uart(&self) -> Vec<u8> {
        let mut sink = self.uart_sink.lock().unwrap();
        let out = sink.clone();
        sink.clear();
        out
    }

    pub fn step(&self) -> Result<labwired_core::StopReason> {
        let mut guard = self.machine.lock().unwrap();
        if let Some(machine) = guard.as_mut() {
            // Capture state BEFORE execution
            let pc_before = machine.get_pc();
            let registers_before: Vec<u32> = (0..16).map(|i| machine.read_core_reg(i)).collect();
            
            // Execute instruction
            let reason = machine
                .step_single()
                .map_err(|e| anyhow!("Step failed: {:?}", e))?;
            
            // Capture state AFTER execution
            let registers_after: Vec<u32> = (0..16).map(|i| machine.read_core_reg(i)).collect();
            
            // Calculate register delta (only changed registers)
            let mut register_delta = std::collections::HashMap::new();
            for i in 0..16 {
                if registers_before[i] != registers_after[i] {
                    register_delta.insert(i as u8, registers_after[i]);
                }
            }
            
            // Increment cycle count
            let cycle = self.cycle_count.fetch_add(1, Ordering::SeqCst);
            
            // TODO: Resolve function name from symbols
            let function = None;
            
            // Record trace
            let trace = InstructionTrace {
                pc: pc_before,
                instruction: 0, // TODO: fetch actual instruction bytes
                cycle,
                function,
                register_delta,
                memory_writes: Vec::new(), // TODO: track memory writes
            };
            
            let mut trace_buffer = self.trace_buffer.lock().unwrap();
            trace_buffer.record(trace);
            
            Ok(reason)
        } else {
            Err(anyhow!("Machine not initialized"))
        }
    }

    /// Get instruction trace for a specific cycle range
    pub fn get_instruction_trace(&self, start_cycle: u64, end_cycle: u64) -> Vec<InstructionTrace> {
        let trace_buffer = self.trace_buffer.lock().unwrap();
        trace_buffer.get_range(start_cycle, end_cycle)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Get all instruction traces
    pub fn get_all_traces(&self) -> Vec<InstructionTrace> {
        let trace_buffer = self.trace_buffer.lock().unwrap();
        trace_buffer.get_all()
    }

    /// Get current cycle count
    pub fn get_cycle_count(&self) -> u64 {
        self.cycle_count.load(Ordering::SeqCst)
    }

    pub fn continue_execution(&self) -> Result<labwired_core::StopReason> {
        self.continue_execution_chunk(100_000)
    }

    pub fn continue_execution_chunk(&self, max_steps: u32) -> Result<labwired_core::StopReason> {
        let mut guard = self.machine.lock().unwrap();
        if let Some(machine) = guard.as_mut() {
            let reason = machine
                .run(Some(max_steps))
                .map_err(|e| anyhow!("Run failed: {:?}", e))?;
            Ok(reason)
        } else {
            Err(anyhow!("Machine not initialized"))
        }
    }

    pub fn set_breakpoints(&self, path: String, lines: Vec<i64>) -> Result<()> {
        let mut addresses = Vec::new();

        let syms_guard = self.symbols.lock().unwrap();
        if let Some(syms) = syms_guard.as_ref() {
            for line in lines {
                if let Some(addr) = syms.location_to_pc(&path, line as u32) {
                    let addr: u64 = addr;
                    addresses.push(addr as u32);
                } else {
                    tracing::warn!("Could not resolve breakpoint at {}:{}", path, line);
                }
            }
        }

        let mut machine_guard = self.machine.lock().unwrap();
        if let Some(machine) = machine_guard.as_mut() {
            machine.clear_breakpoints();
            for addr in addresses {
                machine.add_breakpoint(addr);
                tracing::info!("Breakpoint set at {:#x}", addr);
            }
        }

        Ok(())
    }

    pub fn add_breakpoint_addr(&self, addr: u32) -> Result<()> {
        let mut machine_guard = self.machine.lock().unwrap();
        if let Some(machine) = machine_guard.as_mut() {
            machine.add_breakpoint(addr);
            tracing::info!("Breakpoint added at {:#x}", addr);
            Ok(())
        } else {
            Err(anyhow!("Machine not initialized"))
        }
    }

    pub fn remove_breakpoint_addr(&self, addr: u32) -> Result<()> {
        let mut machine_guard = self.machine.lock().unwrap();
        if let Some(machine) = machine_guard.as_mut() {
            machine.remove_breakpoint(addr);
            tracing::info!("Breakpoint removed at {:#x}", addr);
            Ok(())
        } else {
            Err(anyhow!("Machine not initialized"))
        }
    }

    pub fn read_memory(&self, addr: u64, len: usize) -> Result<Vec<u8>> {
        let machine_guard = self.machine.lock().unwrap();
        if let Some(machine) = machine_guard.as_ref() {
            machine
                .read_memory(addr as u32, len)
                .map_err(|e| anyhow!("Memory read failed: {:?}", e))
        } else {
            Err(anyhow!("Machine not initialized"))
        }
    }

    pub fn write_memory(&self, addr: u64, data: &[u8]) -> Result<()> {
        let mut machine_guard = self.machine.lock().unwrap();
        if let Some(machine) = machine_guard.as_mut() {
            machine
                .write_memory(addr as u32, data)
                .map_err(|e| anyhow!("Memory write failed: {:?}", e))
        } else {
            Err(anyhow!("Machine not initialized"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_adapter_breakpoints() {
        let elf_path = PathBuf::from("../../target/thumbv7m-none-eabi/debug/firmware");
        if !elf_path.exists() {
            return;
        }

        let adapter = LabwiredAdapter::new();
        adapter
            .load_firmware(elf_path, None)
            .expect("Failed to load firmware");

        // Set breakpoint at main.rs:11
        adapter
            .set_breakpoints("main.rs".to_string(), vec![11])
            .expect("Failed to set breakpoints");
    }

    #[test]
    fn test_adapter_read_memory() {
        let elf_path = PathBuf::from("../../target/thumbv7m-none-eabi/debug/firmware");
        if !elf_path.exists() {
            return;
        }

        let adapter = LabwiredAdapter::new();
        adapter
            .load_firmware(elf_path, None)
            .expect("Failed to load firmware");

        // Read first few bytes of Flash (Vector Table)
        let data = adapter.read_memory(0x0, 4).expect("Failed to read memory");
        assert_eq!(data.len(), 4);
    }

    #[test]
    fn test_adapter_uart_capture() {
        let elf_path = PathBuf::from("../../target/thumbv7m-none-eabi/debug/firmware-ci-fixture");
        if !elf_path.exists() {
            // Try to find it in the current directory or parent if running from different scope
            return;
        }

        let adapter = LabwiredAdapter::new();
        adapter
            .load_firmware(elf_path, None)
            .expect("Failed to load firmware");

        // Step for a while to let it write "OK\n"
        // In firmware-ci-fixture, Reset calls main which writes 'O', 'K', '\n'
        for _ in 0..200 {
            let _ = adapter.step().unwrap();
        }

        let output = adapter.poll_uart();
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("OK"),
            "UART output should contain 'OK', got: '{}'",
            text
        );
    }
}
