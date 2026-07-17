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
//! ## The shared guest memory — one copy of RAM, no marshalling
//!
//! Every compiled module imports **the same** `wasmtime::Memory`, and that
//! memory is backed by the *machine's own guest RAM allocation*: a
//! [`GuestBuf`](crate::memory::GuestBuf) whose page-rounded buffer is handed
//! to `wasmtime` through the [`MemoryCreator`] trait. `bus.ram.data` and the
//! wasm linear memory are therefore literally the same bytes — a guest store
//! executed inside a compiled block is immediately visible to the
//! interpreter, and vice versa, with **no copy in either direction**.
//!
//! This is the whole point. The previous design gave every block its own
//! `Store` + `Memory` and copied the entire RAM window in and out around
//! every single run — ~800 KB of `memcpy` per block run on a 400 KB-RAM
//! ESP32-C3. That overhead is fixed per run and independent of block length,
//! so it made short blocks (which is nearly all of real firmware: 1–4
//! instructions) wildly unprofitable, forced
//! [`MIN_PROFITABLE_BLOCK_INSTRS`] up to 16, and left the JIT covering 0.05%
//! of retired instructions while running *slower* than the interpreter.
//!
//! The [`GuestBuf`](crate::memory::GuestBuf) layout reserves the first
//! [`JIT_PREFIX_BYTES`] (== [`RAM_WINDOW_OFF`]) of its allocation for this
//! module's control area, so the emitted ABI is unchanged: guest RAM still
//! appears to compiled code at [`RAM_WINDOW_OFF`].
//!
//! ## Register marshalling
//!
//! What remains per run is the register file, which lives in the control
//! prefix (word `i` = `xi` at byte `i*4`; see [`super::emit`]). Running one
//! block is:
//!
//!   1. copy `cpu.x[0..32]` into the memory ([`REG_SYNC_BYTES`] bytes),
//!   2. call the exported `run` (which loads the touched regs into locals,
//!      computes, stores them back — all inside wasm),
//!   3. copy the memory back into `cpu.x`, forcing `x0 = 0`,
//!   4. map the returned `i32` wire code to a [`SideExit`].
//!
//! That is a fixed `132`-byte sync regardless of block length or RAM size —
//! ~6000× less traffic than the RAM window it replaced. `cpu.x` stays the
//! interpreter's source of truth, so an interpreter↔JIT transition needs no
//! extra bookkeeping.
//!
//! ## Portability
//!
//! Nothing here is native-only in *design*: the browser backend
//! ([`MemoryBinding::BrowserSharedMemory`](super::super::runtime::MemoryBinding))
//! would import the sim's own `WebAssembly.Memory` — in which case
//! `bus.ram.data` already *is* a region of the imported memory and the same
//! "regs prefix + RAM window" ABI applies with an offset resolved at
//! instantiate time instead of a fixed 256.
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

use std::sync::Arc;

use wasmtime::{
    Config, Engine, Instance, LinearMemory, Memory, MemoryCreator, MemoryType, Module, Store,
    TypedFunc,
};

use crate::cpu::RiscV;
use crate::memory::{GuestBuf, WASM_PAGE_BYTES};
use crate::Machine;

use super::super::block_cache::{BlockCache, Lookup};
use super::super::frontend::BlockPlan;
use super::super::side_exit::{BailReason, SideExit};
use super::super::{CodeView, Pc};
use super::emit::{
    MemBinding, FAULT_PC_SLOT, FAULT_RETIRED_SLOT, NEXT_PC_SLOT, RES_FLAG_SLOT, WIRE_CHAIN_DYNAMIC,
    WIRE_FALL_THROUGH, WIRE_MEM_FAULT,
};
use super::RiscVFrontend;
use crate::bus::SystemBus;

/// Bytes synced between the guest register file and the imported memory each
/// block run: `x0..x31` (`128`) plus the one-word dynamic next-PC slot
/// ([`NEXT_PC_SLOT`]) a [`WIRE_CHAIN_DYNAMIC`] block writes its resolved
/// continuation address to.
const REG_SYNC_BYTES: usize = NEXT_PC_SLOT as usize + 4;

// ── The shared guest memory: wasm linear memory backed by `bus.ram.data` ──

/// A `wasmtime` linear memory that *borrows* a
/// [`GuestBuf`](crate::memory::GuestBuf)'s allocation instead of owning one.
///
/// # Safety
///
/// `ptr`/`size` describe the `GuestBuf`'s page-rounded, page-aligned
/// allocation, kept alive for at least this struct's lifetime by `_keepalive`.
/// The memory never grows (declared min == max), so `as_ptr` is stable and
/// wasmtime never asks us to relocate it. Interleaving (never concurrency)
/// with interpreter access is what makes the aliasing sound — see
/// [`crate::memory::guest_buf`].
struct BorrowedGuestMemory {
    ptr: *mut u8,
    size: usize,
    _keepalive: Arc<dyn Send + Sync>,
}

// SAFETY: plain bytes; the keepalive `Arc` owns the allocation. See above.
unsafe impl Send for BorrowedGuestMemory {}
unsafe impl Sync for BorrowedGuestMemory {}

// SAFETY: `as_ptr` returns a page-aligned allocation of exactly `byte_size()`
// zeroed bytes that outlives the memory, and growth is refused beyond it.
unsafe impl LinearMemory for BorrowedGuestMemory {
    fn byte_size(&self) -> usize {
        self.size
    }

    fn byte_capacity(&self) -> usize {
        self.size
    }

    fn grow_to(&mut self, new_size: usize) -> wasmtime::Result<()> {
        // The memory is declared min == max == the guest allocation, so
        // wasmtime only ever "grows" it to its existing size.
        if new_size <= self.size {
            Ok(())
        } else {
            Err(wasmtime::Error::msg(
                "guest-backed wasm memory cannot grow past the guest allocation",
            ))
        }
    }

    fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }
}

/// Hands out [`BorrowedGuestMemory`] over one specific guest allocation.
/// Installed on the `wasmtime` [`Config`], so it serves every memory created
/// in that engine — of which this module creates exactly one.
struct GuestMemoryCreator {
    ptr: usize,
    size: usize,
    keepalive: Arc<dyn Send + Sync>,
}

// SAFETY: every memory handed out points at the single, live, correctly-sized
// and page-aligned guest allocation described by `ptr`/`size`.
unsafe impl MemoryCreator for GuestMemoryCreator {
    fn new_memory(
        &self,
        _ty: MemoryType,
        minimum: usize,
        maximum: Option<usize>,
        _reserved_size_in_bytes: Option<usize>,
        _guard_size_in_bytes: usize,
    ) -> Result<Box<dyn LinearMemory>, String> {
        // Refuse anything this creator was not built for rather than hand back
        // a wrongly-sized view: a mismatch would be silent guest corruption.
        if minimum > self.size {
            return Err(format!(
                "guest allocation is {} bytes, wasm memory wants {minimum}",
                self.size
            ));
        }
        if maximum.is_some_and(|m| m > self.size) {
            return Err("guest-backed memory cannot grow".to_string());
        }
        Ok(Box::new(BorrowedGuestMemory {
            ptr: self.ptr as *mut u8,
            size: self.size,
            _keepalive: Arc::clone(&self.keepalive),
        }))
    }
}

/// One instantiated, runnable compiled block. It holds no store and no memory
/// of its own: both are shared, owned by [`RiscvWasmJit`], and passed in at
/// run time.
pub struct CompiledBlock {
    run: TypedFunc<(), i32>,
    end_pc: Pc,
    instr_count: u32,
    /// Whether the block contains a store (so a set reservation-flag slot is
    /// worth reading back to clear `cpu.reservation`).
    has_store: bool,
}

impl CompiledBlock {
    /// Guest instructions this block retires on a clean fall-through.
    pub fn instr_count(&self) -> u32 {
        self.instr_count
    }
}

impl RiscvWasmJit {
    /// Read a 4-byte little-endian control slot from the shared memory.
    fn read_slot(&self, off: u32) -> u32 {
        let mut b = [0u8; 4];
        self.memory
            .read(&self.store, off as usize, &mut b)
            .expect("control-slot read");
        u32::from_le_bytes(b)
    }

    /// Run `block` against the guest register file `x`, mutating it in place.
    /// Returns the resolved [`SideExit`], the number of guest instructions the
    /// block retired, and whether the caller must clear `cpu.reservation`
    /// (a store executed inline).
    ///
    /// Guest RAM is **not** a parameter and is **not** synced: the block writes
    /// straight into the machine's own `bus.ram.data` bytes through the shared
    /// memory (see the module docs).
    ///
    /// The `bytes` buffer syncs the union of the register file (`x0..x31`) and
    /// the one-word dynamic next-PC slot ([`NEXT_PC_SLOT`], word 32) a
    /// [`WIRE_CHAIN_DYNAMIC`] block writes its resolved continuation to; the
    /// memory-fault control slots (words 33/34/35) are read on demand below.
    pub fn run(&mut self, block: &CompiledBlock, x: &mut [u32; 32]) -> (SideExit, u32, bool) {
        let mut bytes = [0u8; REG_SYNC_BYTES];
        for (i, w) in x.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        self.memory
            .write(&mut self.store, 0, &bytes)
            .expect("register-file memory write");

        if block.has_store {
            self.memory
                .write(&mut self.store, RES_FLAG_SLOT as usize, &[0u8; 4])
                .expect("reservation-flag clear");
        }

        let wire = block
            .run
            .call(&mut self.store, ())
            .expect("compiled block never traps");

        self.memory
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

        let clear_reservation = block.has_store && self.read_slot(RES_FLAG_SLOT) != 0;

        let (exit, n) = match wire {
            // Straight-line prefix (ALU + in-window loads/stores) ran through:
            // chain to the static end PC.
            WIRE_FALL_THROUGH => (
                SideExit::Chain {
                    next_pc: block.end_pc,
                },
                block.instr_count,
            ),
            // A branch/jump terminator resolved its continuation in wasm and
            // wrote it to the next-PC slot (word 32); chain there. The
            // terminator itself is retired, so the whole block count applies.
            WIRE_CHAIN_DYNAMIC => {
                let s = NEXT_PC_SLOT as usize;
                let next_pc =
                    u32::from_le_bytes([bytes[s], bytes[s + 1], bytes[s + 2], bytes[s + 3]]) as Pc;
                (SideExit::Chain { next_pc }, block.instr_count)
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
                    resume_pc: block.end_pc,
                    reason: BailReason::PartialBlock,
                },
                block.instr_count,
            ),
        };
        (exit, n, clear_reservation)
    }
}

/// Compiles [`BlockPlan`]s into runnable [`CompiledBlock`]s and runs them
/// against **one** shared `wasmtime` store + memory.
///
/// The memory is backed by the machine's guest RAM allocation (see the module
/// docs), so it is bound to a specific [`GuestBuf`]: [`bound_alloc`] records
/// which one, and [`RiscvJitEngine`] rebuilds the whole JIT if the machine
/// ever swaps its RAM buffer out from under it.
pub struct RiscvWasmJit {
    engine: Engine,
    store: Store<()>,
    memory: Memory,
    /// Identity of the [`GuestBuf`] allocation `memory` aliases.
    bound_alloc: usize,
}

impl RiscvWasmJit {
    /// New JIT whose shared wasm memory *is* `ram`'s allocation — compiled
    /// blocks read and write the machine's guest RAM in place, with no copy.
    ///
    /// Returns `None` if `wasmtime` refuses the guest-backed configuration.
    pub fn new(ram: &GuestBuf) -> Option<Self> {
        // SAFETY: the pointer is only ever dereferenced by wasm, from inside a
        // `TypedFunc::call` — never while a Rust borrow of `ram` is live. The
        // `keepalive` Arc holds the allocation up for as long as wasmtime (and
        // every memory it hands out) can reach it.
        let (ptr, size, keepalive) = unsafe { ram.raw_shared() };
        debug_assert_eq!(size % WASM_PAGE_BYTES, 0, "guest alloc is page-rounded");

        let creator = Arc::new(GuestMemoryCreator {
            ptr: ptr as usize,
            size,
            keepalive,
        });

        let mut config = Config::new();
        // We supply the allocation ourselves, so wasmtime gets no room to
        // reserve address space or to place unmapped guard pages after it.
        // With no guard, wasmtime emits explicit (dynamic) bounds checks —
        // correct, and cheap here because `emit` already range-checks every
        // guest access against the RAM window anyway.
        config.memory_reservation(0);
        config.memory_reservation_for_growth(0);
        config.memory_guard_size(0);
        config.guard_before_linear_memory(false);
        // Copy-on-write image init would try to map over our buffer.
        config.memory_init_cow(false);
        config.with_host_memory(creator);

        let engine = Engine::new(&config).ok()?;
        let mut store = Store::new(&engine, ());
        let pages = (size / WASM_PAGE_BYTES) as u32;

        // Obtain the shared memory by instantiating a module that *declares*
        // it, rather than via `Memory::new`.
        //
        // This is load-bearing and non-obvious: despite `with_host_memory`'s
        // docs claiming it covers "host `Memory` objects", wasmtime's host
        // memory path (`trampoline::memory::create_memory`) allocates with
        // `OnDemandInstanceAllocator::default()` — which carries **no**
        // memory creator — so a `Memory::new` here would silently hand back
        // wasmtime's own buffer instead of the guest allocation, and compiled
        // blocks would read and write a private copy of RAM that nothing ever
        // syncs. Only *instance* (module-declared) memories go through the
        // engine's configured allocator. So: declare it in a holder module,
        // export it, and import that memory into every block.
        //
        // `min == max == pages` so the memory can never grow and the base
        // pointer wasmtime holds stays valid for the life of the store.
        let holder_src = format!(r#"(module (memory (export "mem") {pages} {pages}))"#);
        let holder = Module::new(&engine, holder_src).ok()?;
        let holder = Instance::new(&mut store, &holder, &[]).ok()?;
        let memory = holder.get_memory(&mut store, "mem")?;

        // The whole design rests on this identity; a silent mismatch would be
        // guest-state corruption rather than a crash, so check it eagerly.
        if memory.data_ptr(&store) != ptr || memory.data_size(&store) != size {
            return None;
        }

        Some(Self {
            engine,
            store,
            memory,
            bound_alloc: ram.alloc_id(),
        })
    }

    /// Instantiate `plan`'s wasm into a runnable block importing the shared
    /// guest memory. `binding` (from
    /// [`RiscVFrontend::translate_block_riscv`](super::RiscVFrontend::translate_block_riscv))
    /// is `Some` iff the block contains a load/store; the shared memory always
    /// covers the whole guest-RAM window mapped at [`RAM_WINDOW_OFF`], so the
    /// binding only tells us whether a reservation-flag read-back is worth it.
    /// Returns `None` if the plan is a body-less stub or the bytes fail to
    /// validate / instantiate — the caller keeps the PC on the interpreter.
    pub fn compile(
        &mut self,
        plan: &BlockPlan,
        binding: Option<MemBinding>,
    ) -> Option<CompiledBlock> {
        if plan.is_stub() {
            return None;
        }
        let module = Module::new(&self.engine, &plan.code).ok()?;
        let memory = self.memory;
        let instance = Instance::new(&mut self.store, &module, &[memory.into()]).ok()?;
        let run = instance
            .get_typed_func::<(), i32>(&mut self.store, "run")
            .ok()?;
        Some(CompiledBlock {
            run,
            end_pc: plan.end_pc,
            instr_count: plan.instr_count,
            has_store: binding.is_some_and(|b| b.has_store),
        })
    }
}

/// Minimum basic-block length (guest instructions) worth compiling.
///
/// This threshold exists to keep the JIT away from blocks whose *fixed*
/// per-run cost (register sync + the wasmtime call) would outweigh the work
/// they do. It used to be `16`, because the executor also copied the entire
/// guest RAM window in and out around every run — ~800 KB per run on the C3 —
/// which is a fixed cost so enormous that only very long blocks could ever
/// amortise it. At 16 the JIT reached just 0.05% of retired instructions on
/// real C3 firmware, i.e. it was effectively disabled.
///
/// With the shared guest memory (see the module docs) that copy is gone and
/// the fixed cost is a 132-byte register sync, so the JIT can profitably
/// compile the short blocks that real firmware overwhelmingly consists of.
/// Measured on the shipped ESP32-C3 OLED lab (`riscv_jit_c3_coverage_bench`,
/// 20M timed guest instrs), sweeping this value via `LW_JIT_MIN_BLOCK`. The
/// dominant effect is on *coverage* — the fraction of retired instructions the
/// JIT can even reach — which the old value of 16 pinned at 0.05%:
///
/// | min block | coverage | steady-state ratio* |
/// |-----------|----------|---------------------|
/// | 1         | 33.05%   | 0.75x               |
/// | 2         | 29.36%   | 0.77x               |
/// | 4         | 16.08%   | 0.74x               |
/// | 16        |  0.04%   | 0.74x               |
///
/// *steady-state = the JIT warmed (hot set compiled) *before* timing, so the
/// number reflects execution, not one-time cranelift compilation. Two things
/// this table shows, and one it does not:
///
///   * **Coverage is unlocked.** 0.04% → ~33% as the threshold drops. The old
///     `16` kept the JIT off real firmware entirely; the whole point of the
///     shared-memory change is that it no longer has to.
///   * **Short blocks stopped being catastrophic.** Adding ~29% coverage
///     nudges the ratio *up* (0.74 → 0.77), i.e. the compiled short blocks are
///     now net-positive per run — where before this change min-block=1 ran the
///     lab ~100× *slower* than the interpreter.
///   * **The end-to-end ratio is still < 1.0**, because it is floored by a
///     pre-existing per-instruction JIT-*dispatch* overhead (≈0.74× even at
///     ~0% coverage) that lives in the dispatch loop, not this ABI, and is
///     only paid down as coverage rises past the ~34% `classify()` cap (a
///     separate workstream). That floor, not marshalling, is now the next
///     bottleneck. See the task notes / module docs.
///
/// `2` is the shipped default: it has the best steady-state ratio in the
/// sweep and, unlike `1`, declines to compile single-instruction blocks (which
/// can never beat one interpreter step yet still pay a full wasmtime call) —
/// at a negligible coverage cost, since 1-instruction blocks are a tiny share
/// of retired instructions.
///
/// Fidelity-neutral at every value: short blocks stay on the interpreter; the
/// compiled ones run the same semantics (the C3 OLED differential gate boots
/// the real firmware on both arms and compares every register, CSR, the UART
/// stream and the framebuffer byte for byte).
pub const MIN_PROFITABLE_BLOCK_INSTRS: u32 = 2;

/// The effective profitability threshold: [`MIN_PROFITABLE_BLOCK_INSTRS`]
/// unless `LW_JIT_MIN_BLOCK` overrides it.
///
/// The override exists so the coverage bench can sweep the threshold without
/// a rebuild; it is read once and cached. It only ever changes *which* blocks
/// are compiled — never what they do — so it cannot affect fidelity.
fn min_profitable_block_instrs() -> u32 {
    use std::sync::OnceLock;
    static V: OnceLock<u32> = OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("LW_JIT_MIN_BLOCK")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|&n| n >= 1)
            .unwrap_or(MIN_PROFITABLE_BLOCK_INSTRS)
    })
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
    /// Built lazily on the first compile, because it can only be created once
    /// the machine's guest RAM buffer (which backs its shared wasm memory) is
    /// known — i.e. once we have a `&SystemBus`. `None` also covers "wasmtime
    /// refused the guest-backed config", in which case every PC simply stays
    /// on the interpreter.
    jit: Option<RiscvWasmJit>,
    cache: BlockCache<CompiledBlock>,
    stats: EngineStats,
}

impl std::fmt::Debug for RiscvJitEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The `wasmtime::Engine` behind `jit` is not `Debug`; surface the
        // run stats (the only interesting state) and elide the rest.
        f.debug_struct("RiscvJitEngine")
            .field("stats", &self.stats)
            .field("compiled_blocks", &self.cache.compiled_len())
            .finish_non_exhaustive()
    }
}

impl RiscvJitEngine {
    /// New engine with the given hot-promotion threshold.
    pub fn new(hot_threshold: u32) -> Self {
        Self {
            frontend: RiscVFrontend::new(),
            jit: None,
            cache: BlockCache::new(hot_threshold),
            stats: EngineStats::default(),
        }
    }

    /// Accumulated statistics.
    pub fn stats(&self) -> EngineStats {
        self.stats
    }

    // ── Production dispatch primitives (chunk H) ──────────────────────────
    //
    // The methods below let `RiscV::step_batch` drive the engine directly
    // over the machine's raw register file + guest-RAM window (via a
    // downcast `&mut SystemBus`) and its own interpreter, *without* routing
    // through a `Machine<RiscV>` (which `step_unit` above requires and which
    // the `Cpu::step_batch` signature cannot hand us). They keep the same
    // block-cache promotion, executor, and stats as `step_unit`.

    /// Record that dispatch landed on `pc` and decide what to do with it
    /// (run a ready block, or interpret + maybe promote). See
    /// [`BlockCache::observe`].
    pub fn observe(&mut self, pc: Pc) -> Lookup {
        self.cache.observe(pc)
    }

    /// The instruction count of the compiled block installed at `pc`, without
    /// counting a run. `None` if `pc` is not hot. The caller uses this to
    /// refuse a block that would retire past its instruction budget or across
    /// a pending interrupt deadline.
    pub fn ready_instr_count(&self, pc: Pc) -> Option<u32> {
        self.cache.peek(pc).map(|b| b.instr_count)
    }

    /// Run the compiled block installed at `pc` against the guest register
    /// file `x`, mutating it in place. Guest RAM is **not** passed: the block
    /// writes the machine's `bus.ram.data` bytes directly through the shared
    /// wasm memory. (Taking a `&mut [u8]` to that same RAM here would alias
    /// the memory the block is about to write — the signature is copy-free
    /// *and* borrow-correct.) Returns
    /// `(retired, next_pc, clear_reservation, needs_interpreter)`:
    ///
    /// * `retired` — guest instructions the block actually retired (`0` on an
    ///   entry-instruction memory fault).
    /// * `next_pc` — the PC the machine must continue from.
    /// * `clear_reservation` — an inline store executed; caller clears
    ///   `cpu.reservation`.
    /// * `needs_interpreter` — the exit is a bail (memory fault / partial);
    ///   with `retired == 0` the caller interprets one instruction to
    ///   guarantee forward progress.
    ///
    /// Panics only if `pc` is not a ready block (the caller guarantees it via
    /// a preceding [`observe`](Self::observe) == [`Lookup::Ready`]).
    pub fn run_ready(&mut self, pc: Pc, x: &mut [u32; 32]) -> (u32, Pc, bool, bool) {
        let block = self.cache.run_artifact(pc).expect("run_ready on a hot PC");
        let jit = self
            .jit
            .as_mut()
            .expect("a ready block implies a bound JIT");
        let (exit, n, clear_reservation) = jit.run(block, x);
        self.stats.block_runs += 1;
        self.stats.block_instrs += n as u64;
        (
            n,
            exit.continuation_pc(),
            clear_reservation,
            exit.needs_interpreter(),
        )
    }

    /// Count an interpreter-fallback instruction (the caller runs the
    /// interpreter itself over its own bus).
    pub fn note_interpreted(&mut self) {
        self.stats.interpreted += 1;
    }

    /// Translate + instantiate the block at `pc`, sourcing its bytes through
    /// the [`SystemBus`]'s MMU/XIP-aware code fetch and binding its guest-RAM
    /// window, installing the block on success. Any refusal leaves the PC on
    /// the interpreter — never an error.
    pub fn try_compile_from_bus(&mut self, pc: Pc, bus: &SystemBus) {
        // Bind (or re-bind) the shared wasm memory to the machine's guest RAM
        // allocation. Re-binding matters for correctness, not just startup: if
        // anything replaces `bus.ram.data` the old allocation is orphaned, and
        // every cached block still addressing it would silently read and write
        // freed-from-the-guest's-view bytes. Compare identity and rebuild.
        let alloc = bus.ram.data.alloc_id();
        if self.jit.as_ref().is_none_or(|j| j.bound_alloc != alloc) {
            // Blocks are instantiated against the old store/memory; none of
            // them may outlive the rebind.
            self.cache.invalidate_all();
            self.jit = RiscvWasmJit::new(&bus.ram.data);
            if self.jit.is_none() {
                return; // no guest-backed wasm memory -> stay interpreted
            }
        }
        // Bind the machine's current guest-RAM window so loads/stores can take
        // the inline fast path (out-of-window accesses side-exit to the
        // interpreter's bus, which owns all MMIO).
        self.frontend
            .set_ram_window(bus.ram.base_addr as u32, bus.ram.data.len() as u32);
        // Source block bytes through the SAME MMU/XIP-aware fetch path the
        // interpreter uses. Reading `bus.flash.data` directly would bypass the
        // ESP32-C3 XIP MMU (0x4200_0000 → physical flash pages) and compile
        // from the wrong bytes. Materialising up to one max-length block
        // (`MAX_BLOCK_INSTRS` × 4 bytes) is amortised across every run of the
        // hot block, so the per-byte fetch cost is negligible.
        let code = bus.read_code_slice(pc, super::MAX_BLOCK_INSTRS as usize * 4);
        if code.len() < 2 {
            return; // `pc` is not in fetchable code memory
        }
        let view = CodeView::new(pc, &code);
        let Ok((plan, binding)) = self.frontend.translate_block_riscv(pc, &view) else {
            return;
        };
        // Real firmware (FreeRTOS / libc / drivers) is full of 1–4 instruction
        // basic blocks. With the shared guest memory (no per-run RAM copy)
        // these are now cheap enough to compile — see
        // [`MIN_PROFITABLE_BLOCK_INSTRS`] for the measured threshold sweep.
        // Single-instruction blocks still stay interpreted (they can never
        // beat one interpreter step yet pay a full wasmtime call).
        if plan.instr_count < min_profitable_block_instrs() {
            return;
        }
        let Some(jit) = self.jit.as_mut() else {
            return;
        };
        if let Some(block) = jit.compile(&plan, binding) {
            self.cache.install(pc, block);
            self.stats.compiled += 1;
        }
    }

    /// Drop the entire block cache — the invalidate-all-on-flash-write policy
    /// (see [`BlockCache::invalidate_all`]).
    pub fn invalidate_blocks(&mut self) {
        self.cache.invalidate_all();
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
                let Some(jit) = self.jit.as_mut() else {
                    return self.interpret_one(machine);
                };
                // The block reaches `machine.bus.ram.data` through the shared
                // wasm memory, so only the register file crosses here.
                let (exit, n, clear_reservation) = jit.run(block, &mut machine.cpu.x);
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
        self.try_compile_from_bus(pc, &machine.bus);
    }
}

#[cfg(test)]
mod tests {
    use super::super::emit::RAM_WINDOW_OFF;
    use super::*;
    use crate::memory::JIT_PREFIX_BYTES;

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

    /// The `GuestBuf` control prefix and the emitted ABI's RAM-window offset
    /// are the same number by construction — guest byte 0 must land exactly
    /// where compiled code expects the window to start. If these ever drift,
    /// every compiled load/store silently addresses the wrong bytes.
    #[test]
    fn jit_prefix_matches_ram_window_off() {
        // `const` blocks so the invariant is a compile-time guarantee, not a
        // runtime check (clippy rightly rejects asserting on constants).
        const _: () = assert!(JIT_PREFIX_BYTES == RAM_WINDOW_OFF as usize);
        // The control slots must all fit inside the prefix.
        const _: () = assert!(RES_FLAG_SLOT + 4 <= RAM_WINDOW_OFF);
        const _: () = assert!(REG_SYNC_BYTES <= JIT_PREFIX_BYTES);
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
        let ram = GuestBuf::new(1024);
        let mut jit = RiscvWasmJit::new(&ram).expect("guest-backed jit");
        let block = jit.compile(&plan, None).expect("compile");
        let mut x = [0u32; 32];
        let (exit, n, _clear) = jit.run(&block, &mut x);
        assert_eq!(n, 2, "two addi retired");
        assert_eq!(x[1], 7);
        assert_eq!(x[2], 10);
        assert_eq!(x[0], 0, "x0 stays zero");
        assert_eq!(exit, SideExit::Chain { next_pc: 8 });
    }

    /// The point of the whole design: a store executed inside a compiled block
    /// lands in the machine's own guest RAM with no write-back step, and a
    /// value the interpreter puts there is visible to the block with no seed
    /// step. Proven here by running against a `GuestBuf` directly.
    #[test]
    fn compiled_block_shares_guest_ram_with_no_copy() {
        // lw x2, 0(x1) ; addi x2,x2,1 ; sw x2, 4(x1) ; ecall
        // (imm=0 for the load, imm=4 for the store: imm[4:0]=4 in bits 7..12,
        // imm[11:5]=0, so no high-immediate term is needed.)
        let lw = (1 << 15) | (0b010 << 12) | (2 << 7) | 0x03;
        let sw = (2 << 20) | (1 << 15) | (0b010 << 12) | (4 << 7) | 0x23;
        let prog = words(&[lw, enc_addi(2, 2, 1), sw, 0x0000_0073]);

        const BASE: u32 = 0x3fc8_0000;
        let mut ram = GuestBuf::new(4096);
        // Interpreter-side write, never explicitly seeded into wasm.
        ram[0..4].copy_from_slice(&41u32.to_le_bytes());

        let mut fe = RiscVFrontend::new();
        fe.set_ram_window(BASE, 4096);
        let view = CodeView::new(0, &prog);
        let (plan, binding) = fe.translate_block_riscv(0, &view).unwrap();
        assert!(binding.is_some(), "block binds the RAM window");

        let mut jit = RiscvWasmJit::new(&ram).expect("guest-backed jit");
        let block = jit.compile(&plan, binding).expect("compile");
        let mut x = [0u32; 32];
        x[1] = BASE;
        let (_exit, n, _clear) = jit.run(&block, &mut x);

        assert_eq!(n, 3, "lw + addi + sw retired");
        assert_eq!(
            x[2], 42,
            "block read the interpreter's value from guest RAM"
        );
        // The store is already in the guest buffer — nothing copied it back.
        assert_eq!(
            u32::from_le_bytes(ram[4..8].try_into().unwrap()),
            42,
            "compiled store landed directly in guest RAM"
        );
    }
}
