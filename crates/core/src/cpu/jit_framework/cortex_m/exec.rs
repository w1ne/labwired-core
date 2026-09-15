// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Native (`wasmtime`) executor for Cortex-M compiled blocks.

use wasmtime::{Engine, Instance, Memory, MemoryType, Module, Store, TypedFunc};

use crate::cpu::CortexM;
use crate::Machine;

use super::super::block_cache::{BlockCache, Lookup};
use super::super::frontend::BlockPlan;
use super::super::side_exit::{BailReason, SideExit};
use super::super::{CodeView, Pc};
use super::emit::{
    MemBinding, FAULT_PC_SLOT, FAULT_RETIRED_SLOT, NEXT_PC_SLOT, RAM_WINDOW_OFF, RES_FLAG_SLOT,
    WIRE_CHAIN_DYNAMIC, WIRE_FALL_THROUGH, WIRE_MEM_FAULT, WIRE_UNSUPPORTED,
};
use super::host::{pack_regs, unpack_regs};
use super::CortexMFrontend;
use crate::bus::SystemBus;

const REG_SYNC_BYTES: usize = NEXT_PC_SLOT as usize + 4;

pub struct CompiledBlock {
    store: Store<()>,
    run: TypedFunc<(), i32>,
    regs: Memory,
    end_pc: Pc,
    instr_count: u32,
    ram_len: usize,
    has_store: bool,
}

impl CompiledBlock {
    fn read_slot(&self, off: u32) -> u32 {
        let mut b = [0u8; 4];
        self.regs
            .read(&self.store, off as usize, &mut b)
            .expect("control-slot read");
        u32::from_le_bytes(b)
    }

    pub fn run(&mut self, x: &mut [u32; 16], ram: &mut [u8]) -> (SideExit, u32, bool) {
        let mut bytes = [0u8; REG_SYNC_BYTES];
        for (i, w) in x.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        self.regs
            .write(&mut self.store, 0, &bytes)
            .expect("register-file memory write");

        let ram_n = self.ram_len.min(ram.len());
        if self.ram_len > 0 {
            self.regs
                .write(&mut self.store, RAM_WINDOW_OFF as usize, &ram[..ram_n])
                .expect("guest-RAM seed");
            self.regs
                .write(&mut self.store, RES_FLAG_SLOT as usize, &[0u8; 4])
                .expect("reservation-flag clear");
        }

        let wire = self
            .run
            .call(&mut self.store, ())
            .expect("compiled block never traps");

        self.regs
            .read(&self.store, 0, &mut bytes)
            .expect("register-file memory read");
        for (i, w) in x.iter_mut().enumerate() {
            *w = u32::from_le_bytes([
                bytes[i * 4],
                bytes[i * 4 + 1],
                bytes[i * 4 + 2],
                bytes[i * 4 + 3],
            ]);
        }

        let mut clear_exclusive = false;
        if self.ram_len > 0 {
            self.regs
                .read(&self.store, RAM_WINDOW_OFF as usize, &mut ram[..ram_n])
                .expect("guest-RAM writeback");
            if self.has_store {
                clear_exclusive = self.read_slot(RES_FLAG_SLOT) != 0;
            }
        }

        let (exit, n) = match wire {
            WIRE_FALL_THROUGH => (
                SideExit::Chain {
                    next_pc: self.end_pc,
                },
                self.instr_count,
            ),
            WIRE_CHAIN_DYNAMIC => {
                let s = NEXT_PC_SLOT as usize;
                let next_pc =
                    u32::from_le_bytes([bytes[s], bytes[s + 1], bytes[s + 2], bytes[s + 3]]) as Pc;
                (SideExit::Chain { next_pc }, self.instr_count)
            }
            WIRE_MEM_FAULT | WIRE_UNSUPPORTED => {
                let resume_pc = self.read_slot(FAULT_PC_SLOT) as Pc;
                let retired = self.read_slot(FAULT_RETIRED_SLOT);
                let reason = if wire == WIRE_MEM_FAULT {
                    BailReason::MemoryFault
                } else {
                    BailReason::UnsupportedInstruction
                };
                (SideExit::EnterInterpreter { resume_pc, reason }, retired)
            }
            _ => (
                SideExit::EnterInterpreter {
                    resume_pc: self.end_pc,
                    reason: BailReason::PartialBlock,
                },
                self.instr_count,
            ),
        };
        (exit, n, clear_exclusive)
    }
}

pub struct CortexMWasmJit {
    engine: Engine,
}

impl Default for CortexMWasmJit {
    fn default() -> Self {
        Self::new()
    }
}

impl CortexMWasmJit {
    pub fn new() -> Self {
        Self {
            engine: Engine::default(),
        }
    }

    pub fn compile(&self, plan: &BlockPlan, binding: Option<MemBinding>) -> Option<CompiledBlock> {
        if plan.is_stub() {
            return None;
        }
        let module = Module::new(&self.engine, &plan.code).ok()?;
        let mut store = Store::new(&self.engine, ());
        let (ram_len, has_store, pages) = match binding {
            Some(b) => {
                let bytes = (RAM_WINDOW_OFF as usize + b.ram_len).max(1);
                let pages = bytes.div_ceil(65536).max(1) as u32;
                (b.ram_len, b.has_store, pages)
            }
            None => (0usize, false, 1u32),
        };
        let regs = Memory::new(&mut store, MemoryType::new(pages, None)).ok()?;
        let instance = Instance::new(&mut store, &module, &[regs.into()]).ok()?;
        let run = instance.get_typed_func::<(), i32>(&mut store, "run").ok()?;
        Some(CompiledBlock {
            store,
            run,
            regs,
            end_pc: plan.end_pc,
            instr_count: plan.instr_count,
            ram_len,
            has_store,
        })
    }
}

pub const MIN_PROFITABLE_BLOCK_INSTRS: u32 = 16;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineStats {
    pub compiled: u64,
    pub block_runs: u64,
    pub block_instrs: u64,
    pub interpreted: u64,
}

pub struct CortexMJitEngine {
    frontend: CortexMFrontend,
    jit: CortexMWasmJit,
    cache: BlockCache<CompiledBlock>,
    stats: EngineStats,
}

impl std::fmt::Debug for CortexMJitEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CortexMJitEngine")
            .field("stats", &self.stats)
            .field("compiled_blocks", &self.cache.compiled_len())
            .finish_non_exhaustive()
    }
}

impl CortexMJitEngine {
    pub fn new(hot_threshold: u32) -> Self {
        Self {
            frontend: CortexMFrontend::new(),
            jit: CortexMWasmJit::new(),
            cache: BlockCache::new(hot_threshold),
            stats: EngineStats::default(),
        }
    }

    pub fn stats(&self) -> EngineStats {
        self.stats
    }

    pub fn observe(&mut self, pc: Pc) -> Lookup {
        self.cache.observe(pc)
    }

    pub fn ready_instr_count(&self, pc: Pc) -> Option<u32> {
        self.cache.peek(pc).map(|b| b.instr_count)
    }

    pub fn run_ready(
        &mut self,
        pc: Pc,
        cpu: &mut CortexM,
        ram: &mut [u8],
    ) -> (u32, Pc, bool, bool) {
        let block = self.cache.run_artifact(pc).expect("run_ready on a hot PC");
        let mut x = [0u32; 16];
        pack_regs(cpu, &mut x);
        let (exit, n, clear_exclusive) = block.run(&mut x, ram);
        unpack_regs(cpu, &x);
        self.stats.block_runs += 1;
        self.stats.block_instrs += n as u64;
        (
            n,
            exit.continuation_pc(),
            clear_exclusive,
            exit.needs_interpreter(),
        )
    }

    pub fn note_interpreted(&mut self) {
        self.stats.interpreted += 1;
    }

    pub fn try_compile_from_bus(&mut self, pc: Pc, bus: &SystemBus) {
        self.frontend
            .set_ram_window(bus.ram.base_addr as u32, bus.ram.data.len() as u32);
        let code = bus.read_code_slice(pc, super::MAX_BLOCK_INSTRS as usize * 4);
        if code.len() < 2 {
            return;
        }
        let view = CodeView::new(pc, &code);
        let Ok((plan, binding)) = self.frontend.translate_block_thumb(pc, &view) else {
            return;
        };
        if plan.instr_count < MIN_PROFITABLE_BLOCK_INSTRS {
            return;
        }
        if let Some(block) = self.jit.compile(&plan, binding) {
            self.cache.install(pc, block);
            self.stats.compiled += 1;
        }
    }

    pub fn invalidate_blocks(&mut self) {
        self.cache.invalidate_all();
    }

    pub fn step_unit(&mut self, machine: &mut Machine<CortexM>) -> u32 {
        let pc = machine.cpu.pc as Pc;
        match self.cache.observe(pc) {
            Lookup::Ready => {
                let Some(block) = self.cache.run_artifact(pc) else {
                    return self.interpret_one(machine);
                };
                let mut x = [0u32; 16];
                pack_regs(&machine.cpu, &mut x);
                let (exit, n, clear_exclusive) = block.run(&mut x, &mut machine.bus.ram.data);
                unpack_regs(&mut machine.cpu, &x);
                if clear_exclusive {
                    machine.cpu.clear_exclusive_monitor();
                }
                self.stats.block_runs += 1;
                self.stats.block_instrs += n as u64;
                machine.cpu.pc = exit.continuation_pc() as u32;
                if n == 0 && exit.needs_interpreter() {
                    return self.interpret_one(machine);
                }
                n
            }
            Lookup::Interpret { promote } => {
                if promote {
                    self.try_compile_from_bus(pc, &machine.bus);
                }
                self.interpret_one(machine)
            }
        }
    }

    fn interpret_one(&mut self, machine: &mut Machine<CortexM>) -> u32 {
        match machine.step() {
            Ok(()) => {
                self.stats.interpreted += 1;
                1
            }
            Err(_) => 0,
        }
    }
}
