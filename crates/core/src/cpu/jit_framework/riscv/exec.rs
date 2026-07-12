// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Native (`wasmtime`) executor for RV32IMC compiled blocks (JIT chunk C).
//!
//! This is the runtime that fills the register-marshalling slot the
//! framework scaffold deliberately elided
//! ([`JitRuntime::run`](super::super::runtime::JitRuntime::run)'s doc: *"the
//! real runtimes take a register-file handle here"*). Because that handle
//! is not yet in the generic trait, chunk C keeps the executor **RISC-V-
//! local** rather than perturbing the generic `JitRuntime` contract: it runs
//! an emitted [`BlockPlan`] directly against a `Machine<RiscV>`'s register
//! file.
//!
//! ## Register marshalling
//!
//! Each compiled module imports one `wasmtime::Memory` — the guest register
//! file (word `i` = `xi` at byte `i*4`; see
//! [`super::emit`]). Running one block is:
//!
//!   1. copy `cpu.x[0..32]` into the memory (`128` bytes),
//!   2. call the exported `run` (which loads the touched regs into locals,
//!      computes, stores them back — all inside wasm),
//!   3. copy the memory back into `cpu.x`, forcing `x0 = 0`,
//!   4. map the returned `i32` wire code to a [`SideExit`].
//!
//! Only the touched registers actually move through the memory (the
//! prologue/epilogue in the emitted body load/store just those), so the
//! host-side copy is a fixed `128`-byte sync regardless of block length.
//!
//! ## Wire-code → side-exit protocol
//!
//! Chunk C emits exactly one edge:
//! [`WIRE_FALL_THROUGH`](super::emit::WIRE_FALL_THROUGH) → [`SideExit::Chain`]
//! to `end_pc` (the block ran its ALU prefix straight through; control flows
//! on to the next instruction, which the interpreter or a chained block
//! handles). Chunks D/E add non-zero wire codes for taken branches and
//! memory faults; the `match` in [`CompiledBlock::run`] is the single place
//! they extend.

use wasmtime::{Engine, Instance, Memory, MemoryType, Module, Store, TypedFunc};

use crate::cpu::RiscV;
use crate::Machine;

use super::super::block_cache::{BlockCache, Lookup};
use super::super::frontend::{BlockPlan, IsaFrontend};
use super::super::side_exit::{BailReason, SideExit};
use super::super::{CodeView, Pc};
use super::emit::WIRE_FALL_THROUGH;
use super::RiscVFrontend;

/// One instantiated, runnable compiled block: its own `wasmtime` store, the
/// imported register-file memory, and the exported `run` entry.
pub struct CompiledBlock {
    store: Store<()>,
    run: TypedFunc<(), i32>,
    regs: Memory,
    end_pc: Pc,
    instr_count: u32,
}

impl CompiledBlock {
    /// Run the block against the guest register file `x`, mutating it in
    /// place. Returns the resolved [`SideExit`] and the number of guest
    /// instructions the block retired.
    pub fn run(&mut self, x: &mut [u32; 32]) -> (SideExit, u32) {
        let mut bytes = [0u8; 128];
        for (i, w) in x.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        self.regs
            .write(&mut self.store, 0, &bytes)
            .expect("register-file memory write");

        let wire = self
            .run
            .call(&mut self.store, ())
            .expect("compiled ALU block never traps");

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
        x[0] = 0; // x0 is hardwired to zero

        let exit = match wire {
            WIRE_FALL_THROUGH => SideExit::Chain {
                next_pc: self.end_pc,
            },
            // No other edge exists in chunk C; treat an unknown wire as a
            // conservative bail so a future emit bug can never silently
            // corrupt state.
            _ => SideExit::EnterInterpreter {
                resume_pc: self.end_pc,
                reason: BailReason::PartialBlock,
            },
        };
        (exit, self.instr_count)
    }
}

/// Compiles [`BlockPlan`]s into runnable [`CompiledBlock`]s on a shared
/// `wasmtime` engine.
pub struct RiscvWasmJit {
    engine: Engine,
}

impl Default for RiscvWasmJit {
    fn default() -> Self {
        Self::new()
    }
}

impl RiscvWasmJit {
    /// New JIT over a fresh `wasmtime` engine (module compilation amortises
    /// across every block).
    pub fn new() -> Self {
        Self {
            engine: Engine::default(),
        }
    }

    /// Instantiate `plan`'s wasm into a runnable block with its own register
    /// memory. Returns `None` if the plan is a body-less stub (nothing to
    /// run) or the bytes fail to validate / instantiate — the caller keeps
    /// the PC on the interpreter.
    pub fn compile(&self, plan: &BlockPlan) -> Option<CompiledBlock> {
        if plan.is_stub() {
            return None;
        }
        let module = Module::new(&self.engine, &plan.code).ok()?;
        let mut store = Store::new(&self.engine, ());
        let regs = Memory::new(&mut store, MemoryType::new(1, None)).ok()?;
        let instance = Instance::new(&mut store, &module, &[regs.into()]).ok()?;
        let run = instance.get_typed_func::<(), i32>(&mut store, "run").ok()?;
        Some(CompiledBlock {
            store,
            run,
            regs,
            end_pc: plan.end_pc,
            instr_count: plan.instr_count,
        })
    }
}

/// Run counters for a [`RiscvJitEngine`] session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineStats {
    /// Blocks that crossed the hot threshold and were compiled + installed.
    pub compiled: u64,
    /// Compiled-block invocations.
    pub block_runs: u64,
    /// Guest instructions retired inside compiled blocks.
    pub block_instrs: u64,
    /// Guest instructions retired on the interpreter fallback path.
    pub interpreted: u64,
}

/// A minimal RISC-V dispatch engine: block-cache promotion + the
/// [`RiscVFrontend`] emitter + the `wasmtime` executor, driving a
/// `Machine<RiscV>` one *unit* (a compiled-block run or a single interpreted
/// instruction) at a time.
///
/// It intentionally does **not** go through the generic
/// [`DispatchLoop`](super::super::dispatch::DispatchLoop): that loop's
/// `JitRuntime::run` cannot reach the guest register file (the elided
/// handle), and the differential harness needs the per-unit *retired
/// instruction count* to keep the interpreter reference aligned across a
/// batched block. Both concerns are served by [`step_unit`](Self::step_unit).
pub struct RiscvJitEngine {
    frontend: RiscVFrontend,
    jit: RiscvWasmJit,
    cache: BlockCache<CompiledBlock>,
    stats: EngineStats,
}

impl RiscvJitEngine {
    /// New engine with the given hot-promotion threshold.
    pub fn new(hot_threshold: u32) -> Self {
        Self {
            frontend: RiscVFrontend::new(),
            jit: RiscvWasmJit::new(),
            cache: BlockCache::new(hot_threshold),
            stats: EngineStats::default(),
        }
    }

    /// Accumulated statistics.
    pub fn stats(&self) -> EngineStats {
        self.stats
    }

    /// Advance `machine` by exactly one dispatch unit. Returns the number of
    /// guest instructions retired (`0` == the machine halted).
    ///
    /// A unit is either one compiled-block run (retiring `instr_count`
    /// instructions atomically) or a single interpreted instruction.
    pub fn step_unit(&mut self, machine: &mut Machine<RiscV>) -> u32 {
        let pc = machine.cpu.pc as Pc;
        match self.cache.observe(pc) {
            Lookup::Ready => {
                let Some(block) = self.cache.run_artifact(pc) else {
                    return self.interpret_one(machine);
                };
                let (exit, n) = block.run(&mut machine.cpu.x);
                self.stats.block_runs += 1;
                self.stats.block_instrs += n as u64;
                machine.cpu.pc = exit.continuation_pc() as u32;
                n
            }
            Lookup::Interpret { promote } => {
                if promote {
                    self.try_compile(machine, pc);
                }
                self.interpret_one(machine)
            }
        }
    }

    /// Interpret one instruction, counting it. Returns `1`, or `0` on halt.
    fn interpret_one(&mut self, machine: &mut Machine<RiscV>) -> u32 {
        match machine.step() {
            Ok(()) => {
                self.stats.interpreted += 1;
                1
            }
            Err(_) => 0,
        }
    }

    /// Translate + instantiate the block at `pc`, installing it on success.
    /// Any refusal (non-ALU entry → stub, out-of-flash PC, instantiate
    /// failure) leaves the PC on the interpreter — never an error.
    fn try_compile(&mut self, machine: &Machine<RiscV>, pc: Pc) {
        let flash = &machine.bus.flash;
        let base = flash.base_addr;
        let len = flash.data.len() as u64;
        if pc < base || (pc - base) >= len {
            return;
        }
        let view = CodeView::new(base, &flash.data);
        let Ok(plan) = self.frontend.translate_block(pc, &view) else {
            return;
        };
        if let Some(block) = self.jit.compile(&plan) {
            self.cache.install(pc, block);
            self.stats.compiled += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(ws: &[u32]) -> Vec<u8> {
        let mut b = Vec::new();
        for w in ws {
            b.extend_from_slice(&w.to_le_bytes());
        }
        b
    }

    fn enc_addi(rd: u32, rs1: u32, imm: i32) -> u32 {
        ((imm as u32 & 0xFFF) << 20) | (rs1 << 15) | (rd << 7) | 0x13
    }

    #[test]
    fn compiled_block_executes_and_mutates_registers() {
        // addi x1,x0,7 ; addi x2,x1,3 ; ecall(terminator)
        let prog = words(&[enc_addi(1, 0, 7), enc_addi(2, 1, 3), 0x0000_0073]);
        let plan = {
            let view = CodeView::new(0, &prog);
            RiscVFrontend::new().translate_block(0, &view).unwrap()
        };
        assert!(!plan.is_stub());
        let jit = RiscvWasmJit::new();
        let mut block = jit.compile(&plan).expect("compile");
        let mut x = [0u32; 32];
        let (exit, n) = block.run(&mut x);
        assert_eq!(n, 2, "two addi retired");
        assert_eq!(x[1], 7);
        assert_eq!(x[2], 10);
        assert_eq!(x[0], 0, "x0 stays zero");
        assert_eq!(exit, SideExit::Chain { next_pc: 8 });
    }
}
