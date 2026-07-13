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
use super::super::frontend::BlockPlan;
use super::super::side_exit::{BailReason, SideExit};
use super::super::{CodeView, Pc};
use super::emit::{
    MemBinding, FAULT_PC_SLOT, FAULT_RETIRED_SLOT, NEXT_PC_SLOT, RAM_WINDOW_OFF, RES_FLAG_SLOT,
    WIRE_CHAIN_DYNAMIC, WIRE_FALL_THROUGH, WIRE_MEM_FAULT,
};
use super::RiscVFrontend;

/// Bytes synced between the guest register file and the imported memory each
/// block run: `x0..x31` (`128`) plus the one-word dynamic next-PC slot
/// ([`NEXT_PC_SLOT`]) a [`WIRE_CHAIN_DYNAMIC`] block writes its resolved
/// continuation address to.
const REG_SYNC_BYTES: usize = NEXT_PC_SLOT as usize + 4;

/// One instantiated, runnable compiled block: its own `wasmtime` store, the
/// imported memory (register file + optional guest-RAM window), and the
/// exported `run` entry.
pub struct CompiledBlock {
    store: Store<()>,
    run: TypedFunc<(), i32>,
    regs: Memory,
    end_pc: Pc,
    instr_count: u32,
    /// Guest-RAM bytes this block syncs in/out around a run (`0` for a
    /// pure-ALU block, which never touches the RAM window).
    ram_len: usize,
    /// Whether the block contains a store (so a set reservation-flag slot is
    /// worth reading back to clear `cpu.reservation`).
    has_store: bool,
}

impl CompiledBlock {
    /// Read a 4-byte little-endian control slot from the imported memory.
    fn read_slot(&self, off: u32) -> u32 {
        let mut b = [0u8; 4];
        self.regs
            .read(&self.store, off as usize, &mut b)
            .expect("control-slot read");
        u32::from_le_bytes(b)
    }

    /// Run the block against the guest register file `x` and the guest RAM
    /// bytes `ram` (the machine's `bus.ram.data`), mutating both in place.
    /// Returns the resolved [`SideExit`], the number of guest instructions the
    /// block retired, and whether the caller must clear `cpu.reservation`
    /// (a store executed inline). `ram` is ignored for pure-ALU blocks.
    ///
    /// The `bytes` buffer syncs the union of the register file (`x0..x31`) and
    /// the one-word dynamic next-PC slot ([`NEXT_PC_SLOT`], word 32) a
    /// [`WIRE_CHAIN_DYNAMIC`] block writes its resolved continuation to; the
    /// memory-fault control slots (words 33/34/35) and the guest-RAM window
    /// (byte [`RAM_WINDOW_OFF`]) are synced separately below.
    pub fn run(&mut self, x: &mut [u32; 32], ram: &mut [u8]) -> (SideExit, u32, bool) {
        let mut bytes = [0u8; REG_SYNC_BYTES];
        for (i, w) in x.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        self.regs
            .write(&mut self.store, 0, &bytes)
            .expect("register-file memory write");

        // Sync the guest-RAM window in + clear the reservation flag slot, but
        // only for blocks that touch memory (pure-ALU blocks skip this).
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
        x[0] = 0; // x0 is hardwired to zero

        let mut clear_reservation = false;
        if self.ram_len > 0 {
            self.regs
                .read(&self.store, RAM_WINDOW_OFF as usize, &mut ram[..ram_n])
                .expect("guest-RAM writeback");
            if self.has_store {
                clear_reservation = self.read_slot(RES_FLAG_SLOT) != 0;
            }
        }

        let (exit, n) = match wire {
            // Straight-line prefix (ALU + in-window loads/stores) ran through:
            // chain to the static end PC.
            WIRE_FALL_THROUGH => (
                SideExit::Chain {
                    next_pc: self.end_pc,
                },
                self.instr_count,
            ),
            // A branch/jump terminator resolved its continuation in wasm and
            // wrote it to the next-PC slot (word 32); chain there. The
            // terminator itself is retired, so the whole block count applies.
            WIRE_CHAIN_DYNAMIC => {
                let s = NEXT_PC_SLOT as usize;
                let next_pc =
                    u32::from_le_bytes([bytes[s], bytes[s + 1], bytes[s + 2], bytes[s + 3]]) as Pc;
                (SideExit::Chain { next_pc }, self.instr_count)
            }
            // Memory fault mid-block: the faulting load/store published its own
            // PC and the count of instructions retired before it. The
            // interpreter resumes there to perform the real (MMIO) access.
            WIRE_MEM_FAULT => {
                let resume_pc = self.read_slot(FAULT_PC_SLOT) as Pc;
                let retired = self.read_slot(FAULT_RETIRED_SLOT);
                (
                    SideExit::EnterInterpreter {
                        resume_pc,
                        reason: BailReason::MemoryFault,
                    },
                    retired,
                )
            }
            // No other edge exists; treat an unknown wire as a conservative
            // bail so a future emit bug can never silently corrupt state.
            _ => (
                SideExit::EnterInterpreter {
                    resume_pc: self.end_pc,
                    reason: BailReason::PartialBlock,
                },
                self.instr_count,
            ),
        };
        (exit, n, clear_reservation)
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

    /// Instantiate `plan`'s wasm into a runnable block with its own imported
    /// memory. `binding` (from
    /// [`RiscVFrontend::translate_block_riscv`](super::RiscVFrontend::translate_block_riscv))
    /// is `Some` iff the block contains a load/store, in which case the memory
    /// is sized to cover the guest-RAM window mapped at [`RAM_WINDOW_OFF`].
    /// Returns `None` if the plan is a body-less stub or the bytes fail to
    /// validate / instantiate — the caller keeps the PC on the interpreter.
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
                // `cpu.x` and `bus.ram.data` are disjoint fields of `machine`.
                let (exit, n, clear_reservation) =
                    block.run(&mut machine.cpu.x, &mut machine.bus.ram.data);
                if clear_reservation {
                    machine.cpu.reservation = None;
                }
                self.stats.block_runs += 1;
                self.stats.block_instrs += n as u64;
                machine.cpu.pc = exit.continuation_pc() as u32;
                // A memory fault on the block's *entry* instruction retires
                // nothing (`n == 0`) and leaves the PC unchanged. Interpret one
                // instruction to guarantee forward progress — otherwise the
                // dispatcher would re-run the same always-faulting block
                // forever (and a bare `0` reads as a halt).
                if n == 0 && exit.needs_interpreter() {
                    return self.interpret_one(machine);
                }
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
        // Bind the machine's current guest-RAM window so loads/stores can take
        // the inline fast path (out-of-window accesses side-exit to the
        // interpreter's bus, which owns all MMIO).
        self.frontend.set_ram_window(
            machine.bus.ram.base_addr as u32,
            machine.bus.ram.data.len() as u32,
        );
        let view = CodeView::new(base, &flash.data);
        let Ok((plan, binding)) = self.frontend.translate_block_riscv(pc, &view) else {
            return;
        };
        if let Some(block) = self.jit.compile(&plan, binding) {
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
            RiscVFrontend::new()
                .translate_block_riscv(0, &view)
                .unwrap()
                .0
        };
        assert!(!plan.is_stub());
        let jit = RiscvWasmJit::new();
        let mut block = jit.compile(&plan, None).expect("compile");
        let mut x = [0u32; 32];
        let (exit, n, _clear) = block.run(&mut x, &mut []);
        assert_eq!(n, 2, "two addi retired");
        assert_eq!(x[1], 7);
        assert_eq!(x[2], 10);
        assert_eq!(x[0], 0, "x0 stays zero");
        assert_eq!(exit, SideExit::Chain { next_pc: 8 });
    }
}
