// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The chaining dispatch loop.
//!
//! [`DispatchLoop`] is the engine that ties the framework together. Given
//! a [`JitHost`] (the machine), it repeatedly:
//!
//!   1. Polls the flash-dirty flag; on a flash write it drops the whole
//!      block cache ([`BlockCache::invalidate_all`]).
//!   2. Checks the [`SafetyGate`]. If anything needs per-instruction
//!      visibility, it interprets one instruction and loops — the JIT
//!      never runs while a probe / observer / breakpoint / cycle-accurate
//!      mode is active.
//!   3. Looks up the current PC in the block cache:
//!      * **Ready** (hot + compiled): runs the artifact, then follows the
//!        [`SideExit`]. On [`SideExit::Chain`] it loops straight to the
//!        next PC — if that PC is also hot the next iteration runs its
//!        block with no interpreter round-trip (the chaining fast path).
//!      * **Interpret**: interprets one instruction. If this hit crossed
//!        the promotion threshold, it asks the frontend to translate the
//!        block and installs the artifact for next time.
//!
//! The passthrough frontend + interpreter runtime make every "Ready" run
//! side-exit immediately, so with the scaffold the loop is functionally
//! "interpret everything" — but it exercises every seam (cache promotion,
//! instantiate, run, side-exit, fallback, chaining) end to end.

use super::block_cache::{BlockCache, Lookup};
use super::fallback::{HostStep, JitHost};
use super::frontend::IsaFrontend;
use super::runtime::{JitRuntime, MemoryBinding};
use super::side_exit::SideExit;
use super::CodeView;

/// Run statistics for one [`DispatchLoop::run`] invocation. Feeds the
/// merge-bar measurement (compiled-block coverage) and telemetry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunStats {
    /// Instructions retired on the interpreter fallback path.
    pub interpreted: u64,
    /// Compiled-block invocations.
    pub block_runs: u64,
    /// Chained block→block transitions that skipped the interpreter.
    pub chained: u64,
    /// Blocks compiled (frontend translate + runtime instantiate).
    pub compiled: u64,
    /// Frontend refusals / instantiate failures (stayed on interpreter).
    pub compile_refusals: u64,
    /// Cache invalidations from flash writes.
    pub flash_invalidations: u64,
}

/// The universal dispatch engine, generic over the ISA frontend and the
/// execution runtime.
pub struct DispatchLoop<F: IsaFrontend, R: JitRuntime> {
    frontend: F,
    runtime: R,
    cache: BlockCache<R::Artifact>,
    /// The directly-addressable RAM/flash window emitted blocks import
    /// (see [`MemoryBinding`]). Set once at construction — memory does not
    /// move after config.
    mem: MemoryBinding,
    stats: RunStats,
}

impl<F: IsaFrontend, R: JitRuntime> DispatchLoop<F, R> {
    /// Build a dispatch loop with the default hot threshold.
    pub fn new(frontend: F, runtime: R, mem: MemoryBinding) -> Self {
        Self {
            frontend,
            runtime,
            cache: BlockCache::default(),
            mem,
            stats: RunStats::default(),
        }
    }

    /// Override the promotion threshold.
    pub fn with_hot_threshold(mut self, threshold: u32) -> Self {
        self.cache = BlockCache::new(threshold);
        self
    }

    /// Accumulated statistics.
    pub fn stats(&self) -> RunStats {
        self.stats
    }

    /// Borrow the block cache (telemetry / tests).
    pub fn cache(&self) -> &BlockCache<R::Artifact> {
        &self.cache
    }

    /// Run the guest for up to `budget` dispatcher iterations, or until the
    /// host halts. Returns the number of iterations performed.
    pub fn run<H: JitHost>(&mut self, host: &mut H, budget: u64) -> u64 {
        let mut iters = 0;
        while iters < budget {
            iters += 1;

            // (1) Flash write => drop the whole cache (correctness rail).
            if host.take_flash_dirty() {
                self.cache.invalidate_all();
                self.stats.flash_invalidations += 1;
            }

            // (2) Observation surface forces the interpreter.
            if !host.safety().jit_allowed() {
                if self.interpret(host) == HostStep::Halted {
                    break;
                }
                continue;
            }

            let pc = host.pc();
            match self.cache.observe(pc) {
                // (3a) Hot block ready — run it and follow the side-exit.
                Lookup::Ready => {
                    let exit = match self.cache.run_artifact(pc) {
                        Some(art) => {
                            self.stats.block_runs += 1;
                            self.runtime.run(art)
                        }
                        // Race: evicted between observe and run. Treat as a
                        // miss and interpret.
                        None => SideExit::unsupported(pc),
                    };
                    if self.follow(host, exit) == HostStep::Halted {
                        break;
                    }
                }
                // (3b) Cold — interpret one; maybe promote+compile.
                Lookup::Interpret { promote } => {
                    if promote {
                        self.try_compile(host, pc);
                    }
                    if self.interpret(host) == HostStep::Halted {
                        break;
                    }
                }
            }
        }
        iters
    }

    /// Act on a [`SideExit`]. `Chain` to a hot PC loops without the
    /// interpreter; every other exit interprets one instruction at the
    /// resume PC.
    fn follow<H: JitHost>(&mut self, host: &mut H, exit: SideExit) -> HostStep {
        match exit {
            SideExit::Chain { next_pc } => {
                host.resume_at(next_pc);
                if self.cache.is_hot(next_pc) {
                    self.stats.chained += 1;
                }
                HostStep::Advanced
            }
            SideExit::EnterInterpreter { resume_pc, .. } => {
                host.resume_at(resume_pc);
                self.interpret(host)
            }
            SideExit::Exception { resume_pc, .. } => {
                // The machine's exception path is driven by the
                // interpreter; resume there and let it vector.
                host.resume_at(resume_pc);
                self.interpret(host)
            }
        }
    }

    /// Interpret exactly one instruction and count it.
    fn interpret<H: JitHost>(&mut self, host: &mut H) -> HostStep {
        let step = host.interpret_one();
        self.stats.interpreted += 1;
        step
    }

    /// Ask the frontend to translate the block at `pc` and, on success,
    /// instantiate + install it. Any refusal/failure leaves `pc` on the
    /// interpreter — never an error.
    fn try_compile<H: JitHost>(&mut self, host: &mut H, pc: u64) {
        let Some(bytes) = host.code_bytes(pc) else {
            self.stats.compile_refusals += 1;
            return;
        };
        if bytes.len() < 2 {
            self.stats.compile_refusals += 1;
            return;
        }
        let view = CodeView::new(pc, &bytes);
        let plan = match self.frontend.translate_block(pc, &view) {
            Ok(plan) => plan,
            Err(_) => {
                self.stats.compile_refusals += 1;
                return;
            }
        };
        match self.runtime.instantiate(&plan, &self.mem) {
            Ok(art) => {
                self.cache.install(pc, art);
                self.stats.compiled += 1;
            }
            Err(_) => self.stats.compile_refusals += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::jit_framework::fallback::SafetyGate;
    use crate::cpu::jit_framework::frontend::PassthroughFrontend;
    use crate::cpu::jit_framework::runtime::InterpreterRuntime;
    use crate::cpu::jit_framework::{Pc, StateVec};

    /// Minimal looping machine: a `loop_len`-instruction hot loop at
    /// `flash_base`, executed until `total` instructions retire. Each
    /// interpreted step advances the PC to the next slot in the loop
    /// (wrapping at the top), so the same handful of PCs recur and cross
    /// the hot threshold — exercising promotion + block dispatch. The
    /// passthrough JIT never actually executes anything, so this proves
    /// the loop + fallback drive a program to completion.
    struct FakeMachine {
        pc: Pc,
        loop_len: u64,
        total: u64,
        flash_base: Pc,
        flash: Vec<u8>,
        safety: SafetyGate,
        flash_dirty: bool,
        steps: u64,
    }

    impl FakeMachine {
        fn new(total: u64) -> Self {
            let flash_base = 0x0800_0000;
            let loop_len = 5;
            Self {
                pc: flash_base,
                loop_len,
                total,
                flash_base,
                flash: vec![0u8; (loop_len as usize + 4) * 4],
                safety: SafetyGate::default(),
                flash_dirty: false,
                steps: 0,
            }
        }
    }

    impl JitHost for FakeMachine {
        fn pc(&self) -> Pc {
            self.pc
        }
        fn interpret_one(&mut self) -> HostStep {
            self.steps += 1;
            if self.steps >= self.total {
                HostStep::Halted
            } else {
                // Walk to the next slot in the hot loop (wrapping).
                self.pc = self.flash_base + (self.steps % self.loop_len) * 4;
                HostStep::Advanced
            }
        }
        fn resume_at(&mut self, pc: Pc) {
            self.pc = pc;
        }
        fn code_bytes(&self, pc: Pc) -> Option<Vec<u8>> {
            if pc >= self.flash_base && (pc - self.flash_base) < self.flash.len() as u64 {
                Some(self.flash[(pc - self.flash_base) as usize..].to_vec())
            } else {
                None
            }
        }
        fn safety(&self) -> SafetyGate {
            self.safety
        }
        fn snapshot_state(&self) -> StateVec {
            vec![self.pc as u32]
        }
        fn take_flash_dirty(&mut self) -> bool {
            std::mem::take(&mut self.flash_dirty)
        }
    }

    fn mem() -> MemoryBinding {
        MemoryBinding::NativeLinear {
            guest_base: 0x0800_0000,
            len: 0x10_0000,
        }
    }

    #[test]
    fn passthrough_loop_runs_program_to_completion_via_fallback() {
        let mut machine = FakeMachine::new(1000);
        let mut loop_ = DispatchLoop::new(PassthroughFrontend, InterpreterRuntime, mem())
            .with_hot_threshold(10);
        let iters = loop_.run(&mut machine, 100_000);

        // The program halts once `total` instructions retire.
        assert!(machine.steps >= machine.total);
        assert!(iters < 100_000, "must halt before budget");
        let stats = loop_.stats();
        // Every advance was interpreted (passthrough never executes).
        assert!(stats.interpreted >= 1000);
        // The recurring hot-loop PCs got "compiled" into side-exit stubs...
        assert!(stats.compiled > 0, "some PC crossed the hot threshold");
        // ...one per distinct loop slot, and no more (installed once each).
        assert!(stats.compiled <= machine.loop_len);
        // ...and running those stubs side-exits straight back to interp.
        assert!(stats.block_runs > 0, "compiled stubs were dispatched");
        // Passthrough stubs never chain (they always bail).
        assert_eq!(stats.chained, 0);
    }

    #[test]
    fn safety_gate_forces_pure_interpreter_no_compiles() {
        let mut machine = FakeMachine::new(100);
        machine.safety.breakpoints_active = true; // debugger attached
        let mut loop_ =
            DispatchLoop::new(PassthroughFrontend, InterpreterRuntime, mem()).with_hot_threshold(1);
        loop_.run(&mut machine, 100_000);
        let stats = loop_.stats();
        // With the gate closed the cache is never consulted, so nothing
        // is ever compiled or dispatched.
        assert_eq!(stats.compiled, 0);
        assert_eq!(stats.block_runs, 0);
        assert!(stats.interpreted >= 100);
    }

    #[test]
    fn flash_write_invalidates_cache_mid_run() {
        let mut machine = FakeMachine::new(50);
        machine.flash_dirty = true; // pending flash write at first poll
        let mut loop_ =
            DispatchLoop::new(PassthroughFrontend, InterpreterRuntime, mem()).with_hot_threshold(1);
        loop_.run(&mut machine, 100_000);
        assert_eq!(loop_.stats().flash_invalidations, 1);
        // The cache re-warmed after the invalidation (fresh generation).
        assert_eq!(loop_.cache().generation(), 1);
    }
}
