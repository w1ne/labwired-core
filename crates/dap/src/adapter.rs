// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::trace::{InstructionTrace, TraceBuffer};
use anyhow::{anyhow, Result};
use labwired_core::{DebugControl, Machine};
use labwired_loader::SymbolProvider;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const GPIO_F1_IDR_OFFSET: u64 = 0x08;
const GPIO_F1_ODR_OFFSET: u64 = 0x0C;
const GPIO_V2_IDR_OFFSET: u64 = 0x10;
const GPIO_V2_ODR_OFFSET: u64 = 0x14;

#[derive(Clone, Serialize)]
pub struct TelemetryData {
    pub pc: u32,
    pub cycles: u64,
    pub mips: f64,
    pub registers: HashMap<String, u32>,
    #[serde(default)]
    pub board_io: Vec<BoardIoState>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BreakpointResolution {
    pub requested_line: i64,
    pub verified: bool,
    pub resolved_line: Option<u32>,
    pub address: Option<u32>,
    pub message: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct BoardIoState {
    pub id: String,
    pub kind: String,
    pub signal: String,
    pub active: bool,
}

#[derive(Clone)]
struct ResolvedBoardIoBinding {
    id: String,
    kind: labwired_config::BoardIoKind,
    signal: labwired_config::BoardIoSignal,
    active_high: bool,
    register_addr: u64,
    pin_mask: u32,
}

#[derive(Clone, Copy)]
struct GpioOffsets {
    idr_offset: u64,
    odr_offset: u64,
}

#[derive(Clone)]
pub struct LabwiredAdapter {
    pub machine: Arc<Mutex<Option<Box<dyn DebugControl + Send>>>>,
    pub symbols: Arc<Mutex<Option<SymbolProvider>>>,
    pub uart_sink: Arc<Mutex<Vec<u8>>>,
    pub last_telemetry: Arc<Mutex<(u64, Instant)>>, // cycles, time
    pub trace_buffer: Arc<Mutex<TraceBuffer>>,
    pub cycle_count: Arc<AtomicU64>,
    board_io_bindings: Arc<Mutex<Vec<ResolvedBoardIoBinding>>>,
    mem_tracker: Arc<MemoryTracker>,
}

#[derive(Debug, Default)]
struct MemoryTracker {
    writes: Mutex<Vec<crate::trace::MemoryWrite>>,
}

impl labwired_core::SimulationObserver for MemoryTracker {
    fn on_simulation_start(&self) {}
    fn on_simulation_stop(&self) {}
    fn on_step_start(&self, _pc: u32, _opcode: u32) {
        self.writes.lock().unwrap().clear();
    }
    fn on_memory_write(&self, addr: u64, old: u8, new: u8) {
        self.writes.lock().unwrap().push(crate::trace::MemoryWrite {
            address: addr,
            old_value: old,
            new_value: new,
        });
    }
}

impl Default for LabwiredAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LabwiredAdapter {
    fn resolve_arch(arch: labwired_core::Arch) -> Result<labwired_core::Arch> {
        match arch {
            labwired_core::Arch::Arm => Ok(labwired_core::Arch::Arm),
            labwired_core::Arch::RiscV => Ok(labwired_core::Arch::RiscV),
            other => Err(anyhow!(
                "Unsupported or unknown firmware architecture: {:?}",
                other
            )),
        }
    }

    pub fn new() -> Self {
        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        Self {
            machine: Arc::new(Mutex::new(None)),
            symbols: Arc::new(Mutex::new(None)),
            uart_sink,
            last_telemetry: Arc::new(Mutex::new((0, Instant::now()))),
            trace_buffer: Arc::new(Mutex::new(TraceBuffer::new(100_000))),
            cycle_count: Arc::new(AtomicU64::new(0)),
            board_io_bindings: Arc::new(Mutex::new(Vec::new())),
            mem_tracker: Arc::new(MemoryTracker::default()),
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

        let mut registers = HashMap::with_capacity(16);
        let names = machine.get_register_names();
        for (i, name) in names.iter().enumerate().take(16) {
            let val = machine.read_core_reg(i as u8);
            registers.insert(name.clone(), val);
        }

        let board_io = {
            let bindings = self.board_io_bindings.lock().unwrap();
            bindings
                .iter()
                .filter_map(|binding| {
                    let active = self.read_board_io_state(machine.as_ref(), binding)?;
                    Some(BoardIoState {
                        id: binding.id.clone(),
                        kind: board_io_kind_str(binding.kind).to_string(),
                        signal: board_io_signal_str(binding.signal).to_string(),
                        active,
                    })
                })
                .collect::<Vec<_>>()
        };

        Some(TelemetryData {
            pc,
            cycles,
            mips,
            registers,
            board_io,
        })
    }

    pub fn load_firmware(
        &self,
        firmware_path: PathBuf,
        system_path: Option<PathBuf>,
    ) -> Result<()> {
        self.board_io_bindings.lock().unwrap().clear();
        let image = labwired_loader::load_elf(&firmware_path)?;

        let mut resolved_board_io_bindings = Vec::new();
        let mut bus = if let Some(sys_path) = &system_path {
            let manifest = labwired_config::SystemManifest::from_file(sys_path)?;
            let chip_path = sys_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(&manifest.chip);
            let chip = labwired_config::ChipDescriptor::from_file(&chip_path)?;
            resolved_board_io_bindings = resolve_board_io_bindings(&chip, &manifest);
            labwired_core::bus::SystemBus::from_config(&chip, &manifest)?
        } else {
            labwired_core::bus::SystemBus::new()
        };
        *self.board_io_bindings.lock().unwrap() = resolved_board_io_bindings;

        let arch = Self::resolve_arch(image.arch)?;

        match arch {
            labwired_core::Arch::Arm => {
                let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
                bus.attach_uart_tx_sink(self.uart_sink.clone(), false);
                bus.observers.push(self.mem_tracker.clone()); // Attach memory tracker
                let mut machine = Machine::new(cpu, bus);
                machine
                    .load_firmware(&image)
                    .map_err(|e| anyhow!("Failed to load firmware: {:?}", e))?;
                *self.machine.lock().unwrap() = Some(Box::new(machine));
            }
            labwired_core::Arch::RiscV => {
                let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
                bus.attach_uart_tx_sink(self.uart_sink.clone(), false);
                bus.observers.push(self.mem_tracker.clone()); // Attach memory tracker
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

    fn read_board_io_state(
        &self,
        machine: &dyn DebugControl,
        binding: &ResolvedBoardIoBinding,
    ) -> Option<bool> {
        let register_value = read_u32_le(machine, binding.register_addr)?;
        let pin_is_set = (register_value & binding.pin_mask) != 0;
        Some(if binding.active_high {
            pin_is_set
        } else {
            !pin_is_set
        })
    }

    pub fn lookup_source(&self, addr: u64) -> Option<labwired_loader::SourceLocation> {
        self.symbols.lock().unwrap().as_ref()?.lookup(addr)
    }

    pub fn resolve_symbol(&self, name: &str) -> Option<u64> {
        self.symbols.lock().unwrap().as_ref()?.resolve_symbol(name)
    }

    pub fn get_locals(&self, pc: u32) -> Vec<labwired_loader::LocalVariable> {
        self.symbols
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.find_locals(pc as u64))
            .unwrap_or_default()
    }

    pub fn lookup_source_reverse(&self, path: &str, line: u32) -> Option<u64> {
        self.symbols
            .lock()
            .unwrap()
            .as_ref()?
            .location_to_pc(path, line)
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
            machine
                .reset()
                .map_err(|e| anyhow!("Reset failed: {:?}", e))
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
        std::mem::take(&mut *sink)
    }

    pub fn step(&self) -> Result<labwired_core::StopReason> {
        let mut guard = self.machine.lock().unwrap();
        if let Some(machine) = guard.as_mut() {
            // Capture state BEFORE execution
            let pc_before = machine.get_pc();
            let registers_before: Vec<u32> = (0..16).map(|i| machine.read_core_reg(i)).collect();

            // Read the instruction bytes at the current PC
            let instr_bytes = machine
                .read_memory(pc_before, 4)
                .unwrap_or(vec![0, 0, 0, 0]);
            let opcode = (instr_bytes[0] as u32) | ((instr_bytes[1] as u32) << 8);

            // Execute instruction
            let reason = machine
                .step_single()
                .map_err(|e| anyhow!("Step failed: {:?}", e))?;

            // Capture state AFTER execution
            let registers_after: Vec<u32> = (0..16).map(|i| machine.read_core_reg(i)).collect();

            // Calculate register delta (only changed registers: ID -> (old, new))
            let mut register_delta = std::collections::HashMap::new();
            for i in 0..16 {
                if registers_before[i] != registers_after[i] {
                    register_delta.insert(i as u8, (registers_before[i], registers_after[i]));
                }
            }

            // Capture memory writes recorded during the step
            let memory_writes = {
                let mut writes = self.mem_tracker.writes.lock().unwrap();
                let captured = writes.clone();
                writes.clear();
                captured
            };

            // Increment cycle count
            let cycle = self.cycle_count.fetch_add(1, Ordering::SeqCst);

            // Disassemble mnemonic (for the trace log)
            let mnemonic = {
                let instr = labwired_core::decoder::decode_thumb_16(opcode as u16);
                Some(format!("{:?}", instr))
            };

            // Resolve function name from symbols
            let function = {
                let symbol_guard = self.symbols.lock().unwrap();
                symbol_guard
                    .as_ref()
                    .and_then(|s| s.lookup(pc_before as u64))
                    .and_then(|loc| loc.function)
            };

            // Record trace
            let trace = InstructionTrace {
                pc: pc_before,
                instruction: opcode,
                cycle,
                function,
                register_delta,
                memory_writes,
                stack_depth: machine.read_core_reg(13),
                mnemonic,
            };

            let mut trace_buffer = self.trace_buffer.lock().unwrap();
            trace_buffer.record(trace);

            Ok(reason)
        } else {
            Err(anyhow!("Machine not initialized"))
        }
    }

    pub fn step_over_source_line(
        &self,
        max_instructions: u32,
    ) -> Result<labwired_core::StopReason> {
        let start_pc = self.get_pc()?;
        let start_loc = self.lookup_source(start_pc as u64);

        // If we don't have source mapping, fall back to single-instruction stepping.
        let Some(start_loc) = start_loc else {
            return self.step();
        };

        let max_instructions = max_instructions.max(1);
        let mut last_reason = labwired_core::StopReason::StepDone;

        for _ in 0..max_instructions {
            let reason = self.step()?;
            last_reason = reason.clone();

            match reason {
                labwired_core::StopReason::Breakpoint(_)
                | labwired_core::StopReason::ManualStop
                | labwired_core::StopReason::MaxStepsReached => return Ok(reason),
                _ => {}
            }

            let pc = self.get_pc()?;
            let Some(curr_loc) = self.lookup_source(pc as u64) else {
                if pc != start_pc {
                    return Ok(reason);
                }
                continue;
            };

            let changed_line = curr_loc.file != start_loc.file || curr_loc.line != start_loc.line;
            if changed_line {
                return Ok(reason);
            }
        }

        Ok(last_reason)
    }

    pub fn get_peripherals_json(&self) -> serde_json::Value {
        use serde_json::json;
        let mut peripherals = Vec::new();
        let machine_guard = self.machine.lock().unwrap();
        if let Some(machine) = machine_guard.as_ref() {
            for (name, base, size) in machine.get_peripherals() {
                let mut registers = Vec::new();
                if let Some(desc) = machine.get_peripheral_descriptor(&name) {
                    for reg in desc.registers {
                        let mut val = 0u32;
                        if let Ok(data) = machine
                            .read_memory((base + reg.address_offset) as u32, reg.size as usize / 8)
                        {
                            for (i, byte) in data.iter().enumerate() {
                                val |= (*byte as u32) << (i * 8);
                            }
                        }

                        let fields = reg
                            .fields
                            .iter()
                            .map(|f| {
                                let msb = f.bit_range[0];
                                let lsb = f.bit_range[1];
                                let bit_width = msb - lsb + 1;
                                let mask = if bit_width >= 32 {
                                    0xFFFFFFFFu32
                                } else {
                                    (1u32 << bit_width) - 1
                                };
                                let f_val = (val >> lsb) & mask;
                                json!({
                                    "name": f.name,
                                    "bitOffset": lsb,
                                    "bitWidth": bit_width,
                                    "value": f_val,
                                    "description": f.description
                                })
                            })
                            .collect::<Vec<_>>();

                        registers.push(json!({
                            "name": reg.id,
                            "offset": reg.address_offset,
                            "size": reg.size,
                            "value": val,
                            "fields": fields
                        }));
                    }
                }

                peripherals.push(json!({
                    "name": name,
                    "base": base,
                    "size": size,
                    "registers": registers
                }));
            }
        }
        json!({ "peripherals": peripherals })
    }

    pub fn get_rtos_state_json(&self) -> serde_json::Value {
        use serde_json::json;
        // Mock RTOS state for now, but centralized here for testing
        json!({
            "tasks": [
                { "name": "Idle", "state": "Running", "stackUsage": 11, "priority": 0 },
                { "name": "MainTask", "state": "Ready", "stackUsage": 42, "priority": 1 },
                { "name": "SensorPoller", "state": "Blocked", "stackUsage": 28, "priority": 2 },
            ]
        })
    }

    /// Get instruction trace for a specific cycle range
    pub fn get_instruction_trace(&self, start_cycle: u64, end_cycle: u64) -> Vec<InstructionTrace> {
        let trace_buffer = self.trace_buffer.lock().unwrap();
        trace_buffer
            .get_range(start_cycle, end_cycle)
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

    pub fn step_back(&self) -> Result<labwired_core::StopReason> {
        let mut guard = self.machine.lock().unwrap();
        if let Some(machine) = guard.as_mut() {
            let mut trace_buffer = self.trace_buffer.lock().unwrap();
            if let Some(trace) = trace_buffer.pop_trace() {
                // 1. Undo memory writes (in reverse order)
                for write in trace.memory_writes.iter().rev() {
                    machine
                        .write_memory(write.address as u32, &[write.old_value])
                        .map_err(|e| anyhow!("Failed to undo memory write: {:?}", e))?;
                }

                // 2. Undo register changes
                for (reg_id, (old_val, _new_val)) in trace.register_delta {
                    machine.write_core_reg(reg_id, old_val);
                }

                // 3. Revert PC
                machine.set_pc(trace.pc);

                // 4. Update cycle count
                self.cycle_count.fetch_sub(1, Ordering::SeqCst);

                Ok(labwired_core::StopReason::StepDone)
            } else {
                Err(anyhow!("No history available for reverse stepping"))
            }
        } else {
            Err(anyhow!("Machine not initialized"))
        }
    }

    pub fn step_out(&self) -> Result<labwired_core::StopReason> {
        let mut guard = self.machine.lock().unwrap();
        if let Some(machine) = guard.as_mut() {
            // Record initial stack pointer (SP is R13 in ARM)
            let initial_sp = machine.read_core_reg(13);

            // Drop the lock before stepping to avoid deadlock
            drop(guard);

            // Continue stepping until SP >= initial_sp (function has returned)
            // We use a maximum step count to prevent infinite loops
            const MAX_STEPS: u32 = 100_000;
            let mut steps = 0;

            loop {
                // Step once
                let reason = self.step()?;
                steps += 1;

                // Check if we've exceeded max steps
                if steps >= MAX_STEPS {
                    return Err(anyhow!(
                        "Step-out exceeded maximum steps (possible infinite loop)"
                    ));
                }

                // Re-acquire lock to check SP
                let guard = self.machine.lock().unwrap();
                if let Some(machine) = guard.as_ref() {
                    let current_sp = machine.read_core_reg(13);

                    // If SP has increased (or stayed same), we've returned from the function
                    // In ARM, SP grows downward, so returning means SP increases
                    if current_sp >= initial_sp {
                        return Ok(labwired_core::StopReason::StepDone);
                    }
                }
                drop(guard);

                // Check for other stop reasons (breakpoints, etc.)
                match reason {
                    labwired_core::StopReason::Breakpoint(_) => return Ok(reason),
                    labwired_core::StopReason::MaxStepsReached => {
                        return Err(anyhow!("Step-out reached max steps"));
                    }
                    _ => {} // Continue stepping
                }
            }
        } else {
            Err(anyhow!("Machine not initialized"))
        }
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

    pub fn set_breakpoints(
        &self,
        path: String,
        lines: Vec<i64>,
    ) -> Result<Vec<BreakpointResolution>> {
        let mut resolutions = Vec::with_capacity(lines.len());
        let mut addresses = Vec::new();

        let syms_guard = self.symbols.lock().unwrap();
        let syms = syms_guard.as_ref();

        for requested_line in lines {
            if requested_line <= 0 {
                resolutions.push(BreakpointResolution {
                    requested_line,
                    verified: false,
                    resolved_line: None,
                    address: None,
                    message: Some("Invalid line number".to_string()),
                });
                continue;
            }

            if let Some(syms) = syms {
                if let Some((addr, resolved_line)) =
                    syms.location_to_pc_nearest(&path, requested_line as u32)
                {
                    let addr32 = addr as u32;
                    addresses.push(addr32);
                    resolutions.push(BreakpointResolution {
                        requested_line,
                        verified: true,
                        resolved_line: Some(resolved_line),
                        address: Some(addr32),
                        message: if resolved_line != requested_line as u32 {
                            Some(format!(
                                "Mapped to nearest executable line {}",
                                resolved_line
                            ))
                        } else {
                            None
                        },
                    });
                } else {
                    tracing::warn!(
                        "Could not resolve breakpoint at {}:{}",
                        path,
                        requested_line
                    );
                    resolutions.push(BreakpointResolution {
                        requested_line,
                        verified: false,
                        resolved_line: None,
                        address: None,
                        message: Some("No executable location for this line".to_string()),
                    });
                }
            } else {
                resolutions.push(BreakpointResolution {
                    requested_line,
                    verified: false,
                    resolved_line: None,
                    address: None,
                    message: Some("Debug symbols unavailable".to_string()),
                });
            }
        }

        addresses.sort_unstable();
        addresses.dedup();

        let mut machine_guard = self.machine.lock().unwrap();
        if let Some(machine) = machine_guard.as_mut() {
            machine.clear_breakpoints();
            for addr in addresses {
                machine.add_breakpoint(addr);
                tracing::info!("Breakpoint set at {:#x}", addr);
            }
        }

        Ok(resolutions)
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

fn read_u32_le(machine: &dyn DebugControl, addr: u64) -> Option<u32> {
    let bytes = machine.read_memory(addr as u32, 4).ok()?;
    if bytes.len() < 4 {
        return None;
    }
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn board_io_kind_str(kind: labwired_config::BoardIoKind) -> &'static str {
    match kind {
        labwired_config::BoardIoKind::Led => "led",
        labwired_config::BoardIoKind::Button => "button",
    }
}

fn board_io_signal_str(signal: labwired_config::BoardIoSignal) -> &'static str {
    match signal {
        labwired_config::BoardIoSignal::Output => "output",
        labwired_config::BoardIoSignal::Input => "input",
    }
}

fn profile_name(peripheral: &labwired_config::PeripheralConfig) -> Option<&str> {
    if let Some(value) = peripheral.config.get("profile") {
        if let Some(name) = value.as_str() {
            return Some(name);
        }
        tracing::warn!(
            "Peripheral '{}' has non-string profile; ignoring profile override",
            peripheral.id
        );
        return None;
    }

    if let Some(value) = peripheral.config.get("register_layout") {
        if let Some(name) = value.as_str() {
            return Some(name);
        }
        tracing::warn!(
            "Peripheral '{}' has non-string register_layout; ignoring layout override",
            peripheral.id
        );
    }
    None
}

fn gpio_offsets_for_peripheral(
    peripheral: &labwired_config::PeripheralConfig,
) -> Option<GpioOffsets> {
    if peripheral.r#type != "gpio" {
        return None;
    }

    let default_offsets = GpioOffsets {
        idr_offset: GPIO_F1_IDR_OFFSET,
        odr_offset: GPIO_F1_ODR_OFFSET,
    };

    let Some(layout_name) = profile_name(peripheral) else {
        return Some(default_offsets);
    };

    let layout = match labwired_core::peripherals::gpio::GpioRegisterLayout::from_str(layout_name) {
        Ok(layout) => layout,
        Err(err) => {
            tracing::warn!(
                "GPIO peripheral '{}' has invalid profile '{}': {}; defaulting to stm32f1 offsets",
                peripheral.id,
                layout_name,
                err
            );
            return Some(default_offsets);
        }
    };

    match layout {
        labwired_core::peripherals::gpio::GpioRegisterLayout::Stm32F1 => Some(default_offsets),
        labwired_core::peripherals::gpio::GpioRegisterLayout::Stm32V2 => Some(GpioOffsets {
            idr_offset: GPIO_V2_IDR_OFFSET,
            odr_offset: GPIO_V2_ODR_OFFSET,
        }),
    }
}

fn resolve_board_io_bindings(
    chip: &labwired_config::ChipDescriptor,
    manifest: &labwired_config::SystemManifest,
) -> Vec<ResolvedBoardIoBinding> {
    let peripheral_gpio_windows: HashMap<String, (u64, GpioOffsets)> = chip
        .peripherals
        .iter()
        .filter_map(|p| {
            let offsets = gpio_offsets_for_peripheral(p)?;
            Some((p.id.to_ascii_lowercase(), (p.base_address, offsets)))
        })
        .collect();

    let mut resolved = Vec::new();
    for binding in &manifest.board_io {
        if binding.pin > 31 {
            tracing::warn!(
                "Skipping board_io binding '{}' with invalid pin {}",
                binding.id,
                binding.pin
            );
            continue;
        }

        let peripheral_key = binding.peripheral.to_ascii_lowercase();
        let Some((base_addr, gpio_offsets)) = peripheral_gpio_windows.get(&peripheral_key).copied()
        else {
            tracing::warn!(
                "Skipping board_io binding '{}' because GPIO peripheral '{}' was not found",
                binding.id,
                binding.peripheral
            );
            continue;
        };

        let register_offset = match binding.signal {
            labwired_config::BoardIoSignal::Input => gpio_offsets.idr_offset,
            labwired_config::BoardIoSignal::Output => gpio_offsets.odr_offset,
        };

        resolved.push(ResolvedBoardIoBinding {
            id: binding.id.clone(),
            kind: binding.kind,
            signal: binding.signal,
            active_high: binding.active_high,
            register_addr: base_addr + register_offset,
            pin_mask: 1u32 << binding.pin,
        });
    }

    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[test]
    fn test_resolve_board_io_bindings_uses_default_gpio_offsets() {
        let chip = labwired_config::ChipDescriptor {
            schema_version: "1.0".to_string(),
            name: "test".to_string(),
            arch: labwired_config::Arch::Arm,
            flash: labwired_config::MemoryRange {
                base: 0x0800_0000,
                size: "128KB".to_string(),
            },
            ram: labwired_config::MemoryRange {
                base: 0x2000_0000,
                size: "32KB".to_string(),
            },
            peripherals: vec![labwired_config::PeripheralConfig {
                id: "gpioa".to_string(),
                r#type: "gpio".to_string(),
                base_address: 0x4001_0800,
                size: None,
                irq: None,
                config: HashMap::new(),
            }],
        };

        let manifest = labwired_config::SystemManifest {
            schema_version: "1.0".to_string(),
            name: "test-system".to_string(),
            chip: "test-chip".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: Vec::new(),
            board_io: vec![labwired_config::BoardIoBinding {
                id: "led".to_string(),
                kind: labwired_config::BoardIoKind::Led,
                peripheral: "gpioa".to_string(),
                pin: 5,
                signal: labwired_config::BoardIoSignal::Output,
                active_high: true,
            }],
            peripherals: Vec::new(),
        };

        let resolved = resolve_board_io_bindings(&chip, &manifest);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].register_addr, 0x4001_0800 + GPIO_F1_ODR_OFFSET);
    }

    #[test]
    fn test_resolve_board_io_bindings_uses_stm32v2_gpio_offsets() {
        let mut gpio_config = HashMap::new();
        gpio_config.insert("profile".to_string(), "stm32v2".into());
        let chip = labwired_config::ChipDescriptor {
            schema_version: "1.0".to_string(),
            name: "test".to_string(),
            arch: labwired_config::Arch::Arm,
            flash: labwired_config::MemoryRange {
                base: 0x0800_0000,
                size: "128KB".to_string(),
            },
            ram: labwired_config::MemoryRange {
                base: 0x2000_0000,
                size: "32KB".to_string(),
            },
            peripherals: vec![labwired_config::PeripheralConfig {
                id: "gpiob".to_string(),
                r#type: "gpio".to_string(),
                base_address: 0x4202_0400,
                size: None,
                irq: None,
                config: gpio_config,
            }],
        };

        let manifest = labwired_config::SystemManifest {
            schema_version: "1.0".to_string(),
            name: "test-system".to_string(),
            chip: "test-chip".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: Vec::new(),
            board_io: vec![
                labwired_config::BoardIoBinding {
                    id: "led".to_string(),
                    kind: labwired_config::BoardIoKind::Led,
                    peripheral: "gpiob".to_string(),
                    pin: 0,
                    signal: labwired_config::BoardIoSignal::Output,
                    active_high: true,
                },
                labwired_config::BoardIoBinding {
                    id: "button".to_string(),
                    kind: labwired_config::BoardIoKind::Button,
                    peripheral: "gpiob".to_string(),
                    pin: 13,
                    signal: labwired_config::BoardIoSignal::Input,
                    active_high: true,
                },
            ],
            peripherals: Vec::new(),
        };

        let resolved = resolve_board_io_bindings(&chip, &manifest);
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].register_addr, 0x4202_0400 + GPIO_V2_ODR_OFFSET);
        assert_eq!(resolved[1].register_addr, 0x4202_0400 + GPIO_V2_IDR_OFFSET);
    }

    #[test]
    fn test_resolve_board_io_bindings_register_layout_alias_still_supported() {
        let mut gpio_config = HashMap::new();
        gpio_config.insert("register_layout".to_string(), "stm32v2".into());
        let chip = labwired_config::ChipDescriptor {
            schema_version: "1.0".to_string(),
            name: "test".to_string(),
            arch: labwired_config::Arch::Arm,
            flash: labwired_config::MemoryRange {
                base: 0x0800_0000,
                size: "128KB".to_string(),
            },
            ram: labwired_config::MemoryRange {
                base: 0x2000_0000,
                size: "32KB".to_string(),
            },
            peripherals: vec![labwired_config::PeripheralConfig {
                id: "gpiob".to_string(),
                r#type: "gpio".to_string(),
                base_address: 0x4202_0400,
                size: None,
                irq: None,
                config: gpio_config,
            }],
        };

        let manifest = labwired_config::SystemManifest {
            schema_version: "1.0".to_string(),
            name: "test-system".to_string(),
            chip: "test-chip".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: Vec::new(),
            board_io: vec![labwired_config::BoardIoBinding {
                id: "led".to_string(),
                kind: labwired_config::BoardIoKind::Led,
                peripheral: "gpiob".to_string(),
                pin: 0,
                signal: labwired_config::BoardIoSignal::Output,
                active_high: true,
            }],
            peripherals: Vec::new(),
        };

        let resolved = resolve_board_io_bindings(&chip, &manifest);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].register_addr, 0x4202_0400 + GPIO_V2_ODR_OFFSET);
    }

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
        let _ = adapter
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

    #[test]
    fn test_step_back_registers_and_memory() {
        let adapter = LabwiredAdapter::new();
        // Setup a simple machine manually for testing
        let mut bus = labwired_core::bus::SystemBus::new();
        let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
        bus.observers.push(adapter.mem_tracker.clone());

        let mut machine = labwired_core::Machine::new(cpu, bus);
        machine.set_pc(0x100);
        machine.write_core_reg(0, 42);

        *adapter.machine.lock().unwrap() = Some(Box::new(machine));

        // 1. Perform a step that changes state
        // We'll mock the trace recording since we can't easily run a "real" instruction here without firmware
        let trace = crate::trace::InstructionTrace {
            pc: 0x100,
            instruction: 0xBF00, // NOP
            cycle: 0,
            function: None,
            register_delta: {
                let mut map = std::collections::HashMap::new();
                map.insert(0, (42, 100)); // R0 changed from 42 to 100
                map
            },
            memory_writes: vec![crate::trace::MemoryWrite {
                address: 0x20000000,
                old_value: 0xAA,
                new_value: 0xBB,
            }],
            stack_depth: 0,
            mnemonic: None,
        };

        adapter.trace_buffer.lock().unwrap().record(trace);

        // Apply the "new" state to the machine to simulate forward progress
        {
            let mut guard = adapter.machine.lock().unwrap();
            let m = guard.as_mut().unwrap();
            m.write_core_reg(0, 100);
            m.write_memory(0x20000000, &[0xBB]).unwrap();
            m.set_pc(0x102);
            adapter.cycle_count.fetch_add(1, Ordering::SeqCst);
        }

        // 2. Perform step_back
        adapter.step_back().expect("Step back failed");

        // 3. Verify state is restored
        let m_guard = adapter.machine.lock().unwrap();
        let m = m_guard.as_ref().unwrap();
        assert_eq!(m.get_pc(), 0x100);
        assert_eq!(m.read_core_reg(0), 42);
        let mem = m.read_memory(0x20000000, 1).unwrap();
        assert_eq!(mem[0], 0xAA);
        assert_eq!(adapter.get_cycle_count(), 0);
    }

    #[test]
    fn test_get_peripherals_json() {
        let adapter = LabwiredAdapter::new();
        let mut bus = labwired_core::bus::SystemBus::new();
        let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
        let machine = labwired_core::Machine::new(cpu, bus);
        *adapter.machine.lock().unwrap() = Some(Box::new(machine));

        let json = adapter.get_peripherals_json();
        let peripherals = json["peripherals"].as_array().unwrap();

        // SystemBus::new() adds several peripherals (uart1, afio, gpioa, etc.)
        assert!(peripherals.len() >= 5);

        let uart1 = peripherals.iter().find(|p| p["name"] == "uart1").unwrap();
        assert_eq!(uart1["base"], 0x4000_C000);

        // uart1 is a mock, it has no declarative registers by default
        // But the JSON structure should still be correct
        assert!(uart1["registers"].is_array());
    }

    #[test]
    fn test_get_rtos_state_json() {
        let adapter = LabwiredAdapter::new();
        let json = adapter.get_rtos_state_json();
        let tasks = json["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0]["name"], "Idle");
        assert_eq!(tasks[0]["state"], "Running");
    }

    #[test]
    fn test_reverse_step_stress_test() {
        let adapter = LabwiredAdapter::new();
        let mut bus = labwired_core::bus::SystemBus::new();
        let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
        bus.observers.push(adapter.mem_tracker.clone());

        let mut machine = labwired_core::Machine::new(cpu, bus);
        machine.set_pc(0x100);
        *adapter.machine.lock().unwrap() = Some(Box::new(machine));

        // Let's capture the INITIAL state (snapshot)
        let initial_registers: Vec<u32> = (0..16)
            .map(|i| {
                let guard = adapter.machine.lock().unwrap();
                guard.as_ref().unwrap().read_core_reg(i as u8)
            })
            .collect();

        // Simulate 100 random state transitions (mocked for speed/simplicity)
        for i in 0u32..100 {
            let trace = crate::trace::InstructionTrace {
                pc: 0x100 + (i * 2),
                instruction: 0xBF00,
                cycle: i as u64,
                function: None,
                register_delta: {
                    let mut map = std::collections::HashMap::new();
                    map.insert(0, (i, i + 1));
                    map
                },
                memory_writes: vec![crate::trace::MemoryWrite {
                    address: (0x20000000 + (i * 4)) as u64,
                    old_value: 0,
                    new_value: 0xFF,
                }],
                stack_depth: 0,
                mnemonic: None,
            };
            adapter.trace_buffer.lock().unwrap().record(trace);

            // Advance machine
            let mut guard = adapter.machine.lock().unwrap();
            let m = guard.as_mut().unwrap();
            m.write_core_reg(0, i + 1);
            m.write_memory(0x20000000 + (i * 4), &[0xFF]).unwrap();
            m.set_pc(0x100 + (i * 2) + 2);
            adapter.cycle_count.fetch_add(1, Ordering::SeqCst);
        }

        // Now reverse all 100 steps
        for _ in 0..100 {
            adapter.step_back().unwrap();
        }

        // Verify state matches initial
        let guard = adapter.machine.lock().unwrap();
        let m = guard.as_ref().unwrap();
        assert_eq!(m.get_pc(), 0x100);
        for (i, expected) in initial_registers.iter().enumerate().take(16) {
            assert_eq!(m.read_core_reg(i as u8), *expected);
        }
        assert_eq!(adapter.get_cycle_count(), 0);
    }

    #[test]
    fn test_write_memory() {
        let adapter = LabwiredAdapter::new();
        {
            let mut bus = labwired_core::bus::SystemBus::new();
            let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
            let machine = labwired_core::Machine::new(cpu, bus);
            *adapter.machine.lock().unwrap() = Some(Box::new(machine));
        }

        let addr = 0x2000_1000;
        let data = vec![0xDE, 0xAD, 0xBE, 0xEF];

        adapter.write_memory(addr, &data).unwrap();

        let read_data = adapter.read_memory(addr, 4).unwrap();
        assert_eq!(read_data, data);
    }

    #[test]
    fn test_resolve_arch_rejects_unknown() {
        let err = LabwiredAdapter::resolve_arch(labwired_core::Arch::Unknown)
            .expect_err("Unknown architecture should be rejected");
        assert!(err
            .to_string()
            .contains("Unsupported or unknown firmware architecture"));
    }
}
