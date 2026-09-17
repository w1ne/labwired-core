// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Native (`wasmtime`) executor for Cortex-M compiled blocks.

use wasmtime::{Caller, Engine, Func, Instance, Memory, MemoryType, Module, Store, TypedFunc};

use crate::cpu::CortexM;
use crate::Machine;

use super::super::block_cache::{BlockCache, Lookup};
use super::super::frontend::BlockPlan;
use super::super::side_exit::{BailReason, SideExit};
use super::super::{CodeView, Pc};
use super::emit::{
    MemBinding, FAULT_PC_SLOT, FAULT_RETIRED_SLOT, IT_STATE_SLOT, NEXT_PC_SLOT, RES_FLAG_SLOT,
    WIRE_CHAIN_DYNAMIC, WIRE_FALL_THROUGH, WIRE_MEM_FAULT, WIRE_UNSUPPORTED,
};
use super::host::{pack_regs, unpack_regs};
use super::CortexMFrontend;
use crate::bus::SystemBus;

const REG_SYNC_BYTES: usize = NEXT_PC_SLOT as usize + 4;

struct RamHost {
    ptr: *mut u8,
    len: usize,
    fpu: *mut u32,
}

unsafe impl Send for RamHost {}
unsafe impl Sync for RamHost {}

fn host_ram_load(caller: Caller<'_, RamHost>, off: i32, width: i32, signed: i32) -> i32 {
    let host = caller.data();
    let off = off as u32 as usize;
    let width = width as u32 as usize;
    if host.ptr.is_null() || width == 0 || off.saturating_add(width) > host.len {
        return 0;
    }
    unsafe {
        let p = host.ptr.add(off);
        match (width, signed) {
            (1, 0) => i32::from(*p),
            (1, _) => i32::from(*p as i8),
            (2, 0) => i32::from(u16::from_le_bytes([*p, *p.add(1)])),
            (2, _) => i32::from(i16::from_le_bytes([*p, *p.add(1)])),
            (4, _) => i32::from_le_bytes([*p, *p.add(1), *p.add(2), *p.add(3)]),
            _ => 0,
        }
    }
}

fn host_vfp_get(caller: Caller<'_, RamHost>, sn: i32) -> i32 {
    let host = caller.data();
    let i = sn as u32 as usize;
    if host.fpu.is_null() || i >= 32 {
        return 0;
    }
    unsafe { *host.fpu.add(i) as i32 }
}

fn host_vfp_set(caller: Caller<'_, RamHost>, sd: i32, bits: i32) {
    let host = caller.data();
    let i = sd as u32 as usize;
    if host.fpu.is_null() || i >= 32 {
        return;
    }
    unsafe {
        *host.fpu.add(i) = bits as u32;
    }
}

fn host_ram_store(caller: Caller<'_, RamHost>, off: i32, val: i32, width: i32) {
    let host = caller.data();
    let off = off as u32 as usize;
    let width = width as u32 as usize;
    if host.ptr.is_null() || width == 0 || off.saturating_add(width) > host.len {
        return;
    }
    let v = val as u32;
    unsafe {
        let p = host.ptr.add(off);
        match width {
            1 => *p = v as u8,
            2 => {
                let b = (v as u16).to_le_bytes();
                *p = b[0];
                *p.add(1) = b[1];
            }
            4 => {
                let b = v.to_le_bytes();
                *p = b[0];
                *p.add(1) = b[1];
                *p.add(2) = b[2];
                *p.add(3) = b[3];
            }
            _ => {}
        }
    }
}

pub struct CompiledBlock {
    store: Store<RamHost>,
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

    /// Run to the next side-exit. The fourth tuple element is the IT state to
    /// reinstall in the core when control returns to the interpreter: `Some`
    /// exactly on `WIRE_MEM_FAULT` / `WIRE_UNSUPPORTED`, where the block may
    /// have stopped mid-IT (the emitter wrote the pre-fault state), and
    /// `None` on chain/fall-through exits, which never carry one.
    pub fn run(
        &mut self,
        x: &mut [u32; 16],
        ram: &mut [u8],
        fpu: &mut [u32; 32],
    ) -> (SideExit, u32, bool, Option<u8>) {
        let mut bytes = [0u8; REG_SYNC_BYTES];
        for (i, w) in x.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        self.regs
            .write(&mut self.store, 0, &bytes)
            .expect("register-file memory write");

        self.store.data_mut().ptr = ram.as_mut_ptr();
        self.store.data_mut().len = ram.len();
        self.store.data_mut().fpu = fpu.as_mut_ptr();
        if self.ram_len > 0 {
            self.regs
                .write(&mut self.store, RES_FLAG_SLOT as usize, &[0u8; 4])
                .expect("reservation-flag clear");
        }

        let wire = self
            .run
            .call(&mut self.store, ())
            .expect("compiled block never traps");
        self.store.data_mut().ptr = std::ptr::null_mut();
        self.store.data_mut().len = 0;
        self.store.data_mut().fpu = std::ptr::null_mut();

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
        if self.has_store {
            clear_exclusive = self.read_slot(RES_FLAG_SLOT) != 0;
        }

        let (exit, n, it_state) = match wire {
            WIRE_FALL_THROUGH => (
                SideExit::Chain {
                    next_pc: self.end_pc,
                },
                self.instr_count,
                None,
            ),
            WIRE_CHAIN_DYNAMIC => {
                let s = NEXT_PC_SLOT as usize;
                let next_pc =
                    u32::from_le_bytes([bytes[s], bytes[s + 1], bytes[s + 2], bytes[s + 3]]) as Pc;
                (SideExit::Chain { next_pc }, self.instr_count, None)
            }
            WIRE_MEM_FAULT | WIRE_UNSUPPORTED => {
                let resume_pc = self.read_slot(FAULT_PC_SLOT) as Pc;
                let retired = self.read_slot(FAULT_RETIRED_SLOT);
                let reason = if wire == WIRE_MEM_FAULT {
                    BailReason::MemoryFault
                } else {
                    BailReason::UnsupportedInstruction
                };
                (
                    SideExit::EnterInterpreter { resume_pc, reason },
                    retired,
                    Some(self.read_slot(IT_STATE_SLOT) as u8),
                )
            }
            _ => (
                SideExit::EnterInterpreter {
                    resume_pc: self.end_pc,
                    reason: BailReason::PartialBlock,
                },
                self.instr_count,
                None,
            ),
        };
        (exit, n, clear_exclusive, it_state)
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
        let mut store = Store::new(
            &self.engine,
            RamHost {
                ptr: std::ptr::null_mut(),
                len: 0,
                fpu: std::ptr::null_mut(),
            },
        );
        let (ram_len, has_store) = match binding {
            Some(b) => (b.ram_len, b.has_store),
            None => (0usize, false),
        };
        let regs = Memory::new(&mut store, MemoryType::new(1, None)).ok()?;
        let instance = if binding.is_some() {
            let load = Func::wrap(&mut store, host_ram_load);
            let store_fn = Func::wrap(&mut store, host_ram_store);
            let vget = Func::wrap(&mut store, host_vfp_get);
            let vset = Func::wrap(&mut store, host_vfp_set);
            Instance::new(
                &mut store,
                &module,
                &[
                    regs.into(),
                    load.into(),
                    store_fn.into(),
                    vget.into(),
                    vset.into(),
                ],
            )
            .ok()?
        } else {
            Instance::new(&mut store, &module, &[regs.into()]).ok()?
        };
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

pub const MIN_PROFITABLE_BLOCK_INSTRS: u32 = 4;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineStats {
    pub compiled: u64,
    pub block_runs: u64,
    pub block_instrs: u64,
    pub interpreted: u64,
    /// Bytes memcpy'd between wasm linear memory and guest RAM. Zero-copy
    /// host load/store keeps this at 0 even for mem blocks.
    pub ram_bytes_synced: u64,
    /// Compiled block→block transitions that skipped `observe()`.
    pub chained: u64,
}

pub struct CortexMJitEngine {
    frontend: CortexMFrontend,
    jit: CortexMWasmJit,
    cache: BlockCache<CompiledBlock>,
    stats: EngineStats,
    min_profitable: u32,
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
            min_profitable: MIN_PROFITABLE_BLOCK_INSTRS,
        }
    }

    pub fn set_min_profitable(&mut self, n: u32) {
        self.min_profitable = n.max(1);
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
        let (exit, n, clear_exclusive, it_state) = block.run(&mut x, ram, &mut cpu.fpu_s);
        unpack_regs(cpu, &x);
        if let Some(it) = it_state {
            cpu.it_state = it;
        }
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

    pub fn note_chained(&mut self) {
        self.stats.chained += 1;
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
        if plan.instr_count < self.min_profitable {
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
        // A mid-IT entry must go to the interpreter: the compiled block at
        // that PC models its body as unconditional (its `it_state` starts at
        // 0), so predication lives in `cpu.it_state` alone. `run_jit_loop`
        // applies the same gate per batch.
        if machine.cpu.it_state != 0 {
            return self.interpret_one(machine);
        }
        let pc = machine.cpu.pc as Pc;
        match self.cache.observe(pc) {
            Lookup::Ready => {
                let Some(block) = self.cache.run_artifact(pc) else {
                    return self.interpret_one(machine);
                };
                let mut x = [0u32; 16];
                pack_regs(&machine.cpu, &mut x);
                let (exit, n, clear_exclusive, it_state) =
                    block.run(&mut x, &mut machine.bus.ram.data, &mut machine.cpu.fpu_s);
                unpack_regs(&mut machine.cpu, &x);
                if let Some(it) = it_state {
                    machine.cpu.it_state = it;
                }
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
