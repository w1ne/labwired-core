// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::decoder::riscv::{decode_rv32, Instruction};
use crate::{Bus, Cpu, SimResult, SimulationObserver};
use std::sync::Arc;

/// Estimated CPU clocks per interpreted instruction, used to scale the
/// free-running cycle/perf-counter CSRs (0x802/0x7E2/0xC00). Firmware busy-wait
/// delays compute a target in CPU clocks (e.g. `us * cpu_freq_mhz`); reporting
/// the cycle counter as `mtime * CYCLE_SCALE` lets those delays elapse in
/// ~1/CYCLE_SCALE the interpreted instructions instead of one-per-clock. Sim
/// delays only need to complete, not match wall-clock, so a coarse value is
/// correct here. Kept a power of two; tuned so the C3 bootloader's entropy fill
/// drops from ~380M instructions to a few million.
const CYCLE_SCALE: u64 = 256;

/// Chunk H: land on a basic-block entry PC this many times before compiling
/// it to wasm. Matches the framework default; keeps one-shot init/boot code
/// on the interpreter and only pays translation cost for genuinely hot loops.
#[cfg(feature = "jit")]
const RISCV_JIT_HOT_THRESHOLD: u32 = 50;

#[derive(Debug, Clone, Copy)]
pub struct RiscVDecodeCacheEntry {
    pub tag: u32,
    pub opcode: u32,
    pub instruction: Instruction,
    pub inst_len: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiscVCoreProfile {
    /// ESP32-C3's machine performance-counter block. It does not implement
    /// the standard unprivileged `cycle` CSR at 0xC00.
    Esp32C3,
    /// Baseline RV32 profile used by generic RISC-V targets.
    StandardRv32,
}

#[derive(Debug)]
pub struct RiscV {
    pub core_profile: RiscVCoreProfile,
    pub x: [u32; 32], // x0..x31. x0 is correctly hardwired to 0 in logic.
    pub pc: u32,

    // CSRs
    pub mstatus: u32,
    pub mie: u32,
    pub mip: u32,
    pub mtvec: u32,
    pub mscratch: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,

    // CLINT-like internal state (minimal)
    pub mtime: u64,
    pub mtimecmp: u64,

    /// Active LR/SC reservation address. `None` means no outstanding
    /// reservation; the next SC.W to any address will fail. On a single
    /// hart any intervening store (including any AMO*) invalidates the
    /// reservation per RISC-V ISA §8.2.
    pub reservation: Option<u32>,

    waiting_for_interrupt: bool,
    decode_cache: Box<[Option<RiscVDecodeCacheEntry>; 4096]>,

    /// Side-effect-free instruction-fetch window over flash-XIP (and linear
    /// code memories). Avoids per-instruction `find_peripheral_index` + dyn
    /// dispatch — the post-XIP-opt profile hotspot on C3 OLED. Only filled
    /// from read-only code paths (FlashXIP / RAM / flash linear); MMIO never
    /// enters the window, so FIFO clear-on-read and other side effects stay
    /// on the normal bus path for data accesses.
    fetch_base: u32,
    fetch_len: u16,
    fetch_bytes: [u8; FETCH_WINDOW_BYTES],

    /// Chunk H: opt-in RV32IMC wasm-JIT fast path. Mirrors Xtensa's
    /// `self.jit_enabled`; synced from [`crate::SimulationConfig::riscv_jit_enabled`]
    /// on each `step_batch` entry. Off by default — the interpreter is the
    /// behavioral oracle.
    #[cfg(feature = "jit")]
    jit_enabled: bool,

    /// Lazily-created JIT engine (block cache + `wasmtime` executor). `None`
    /// until the first JIT-enabled batch, then reused (and its block cache
    /// warmed) across batches.
    #[cfg(feature = "jit")]
    jit_engine: Option<crate::cpu::jit_framework::riscv::RiscvJitEngine>,
}

/// Bytes of guest code held in the interpreter fetch window (power of two).
const FETCH_WINDOW_BYTES: usize = 256;

impl Default for RiscV {
    fn default() -> Self {
        Self::new_for(RiscVCoreProfile::Esp32C3)
    }
}

impl RiscV {
    pub fn new_for(core_profile: RiscVCoreProfile) -> Self {
        Self {
            core_profile,
            x: [0; 32],
            pc: 0,
            mstatus: 0,
            mie: 0,
            mip: 0,
            mtvec: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            mtime: 0,
            mtimecmp: 0,
            reservation: None,
            waiting_for_interrupt: false,
            decode_cache: Box::new([None; 4096]),
            fetch_base: 0,
            fetch_len: 0,
            fetch_bytes: [0; FETCH_WINDOW_BYTES],
            #[cfg(feature = "jit")]
            jit_enabled: false,
            #[cfg(feature = "jit")]
            jit_engine: None,
        }
    }

    /// Fetch a little-endian u32 instruction word at `self.pc`, preferring the
    /// local code window (same bytes as `bus.read_u32(pc)` for XIP/RAM/flash).
    fn fetch_opcode_u32(&mut self, bus: &mut dyn Bus) -> SimResult<u32> {
        let pc = self.pc;
        let off = pc.wrapping_sub(self.fetch_base);
        if (off as u64) < self.fetch_len as u64 && (off as u64) + 4 <= self.fetch_len as u64 {
            let i = off as usize;
            return Ok(u32::from_le_bytes([
                self.fetch_bytes[i],
                self.fetch_bytes[i + 1],
                self.fetch_bytes[i + 2],
                self.fetch_bytes[i + 3],
            ]));
        }
        self.refill_fetch_window(bus, pc);
        let off = pc.wrapping_sub(self.fetch_base);
        if (off as u64) < self.fetch_len as u64 && (off as u64) + 4 <= self.fetch_len as u64 {
            let i = off as usize;
            return Ok(u32::from_le_bytes([
                self.fetch_bytes[i],
                self.fetch_bytes[i + 1],
                self.fetch_bytes[i + 2],
                self.fetch_bytes[i + 3],
            ]));
        }
        // Window could not cover `pc` (non-code memory) — fall back to the bus.
        bus.read_u32(pc as u64)
    }

    /// Fill [`fetch_bytes`] from side-effect-free code memory starting near `pc`.
    /// On failure / non-code, sets `fetch_len = 0` so the caller uses `bus.read_u32`.
    fn refill_fetch_window(&mut self, bus: &mut dyn Bus, pc: u32) {
        self.fetch_len = 0;
        // Align down so sequential execution reuses the window across branches
        // within the same 256-byte line when possible.
        let base = pc & !((FETCH_WINDOW_BYTES as u32) - 1);

        let Some(sb) = bus
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::bus::SystemBus>())
        else {
            return;
        };

        // Flash-XIP only (ESP32-C3 app at 0x4200_0000). We deliberately do
        // **not** window linear RAM/flash: unit tests and self-modifying sequences
        // patch guest code under the PC and expect the next `step` to see the
        // new bytes; a RAM window would go stale. XIP flash is immutable for
        // the current SPI model (program/erase only touch status regs), so a
        // window is byte-identical to repeated `bus.read_u32` there.
        if let Some(idx) = sb.find_peripheral_index(base as u64) {
            let p = &sb.peripherals[idx];
            if let Some(xip) = p.dev.as_any().and_then(|a| {
                a.downcast_ref::<crate::peripherals::esp32s3::flash_xip::FlashXipPeripheral>()
            }) {
                let end = p.base.saturating_add(p.size);
                if (base as u64) >= p.base && (base as u64) + 4 <= end {
                    let max = ((end - base as u64) as usize).min(FETCH_WINDOW_BYTES);
                    let mut buf = [0u8; FETCH_WINDOW_BYTES];
                    xip.read_span((base as u64) - p.base, &mut buf[..max]);
                    self.fetch_base = base;
                    self.fetch_bytes = buf;
                    self.fetch_len = max as u16;
                }
            }
        }
    }

    pub fn new() -> Self {
        Self::new_for(RiscVCoreProfile::Esp32C3)
    }

    fn update_mtime_after_elapsed_cycles(&mut self, cycles: u64) {
        self.mtime = self.mtime.wrapping_add(cycles);
        if self.mtime >= self.mtimecmp {
            self.mip |= 1 << 7; // MTIP
        } else {
            self.mip &= !(1 << 7);
        }
    }

    fn read_reg(&self, n: u8) -> u32 {
        if n == 0 {
            0
        } else {
            self.x[n as usize]
        }
    }

    fn write_reg(&mut self, n: u8, val: u32) {
        if n != 0 {
            self.x[n as usize] = val;
        }
    }

    fn read_csr(&self, csr: u16) -> Option<u32> {
        Some(match csr {
            0x300 => self.mstatus,
            0x304 => self.mie,
            0x344 => self.mip,
            0x305 => self.mtvec,
            0x340 => self.mscratch,
            0x341 => self.mepc,
            0x342 => self.mcause,
            0x343 => self.mtval,
            // Physical Memory Protection (PMP) configuration and address registers (stubs)
            0x3A0..=0x3A3 | 0x3B0..=0x3BF => 0,
            // Timer CSR stubs (Standard RISC-V shadow non-privileged? No, these are machine mode)
            0xB00 => (self.mtime & 0xFFFFFFFF) as u32,
            0xB80 => (self.mtime >> 32) as u32,
            // The ESP32-C3 exposes its free-running counter at the custom
            // machine PCCR CSR 0x7E2, not at standard RISC-V cycle CSR 0xC00.
            //
            // These are reported as mtime * CYCLE_SCALE so that cycle-budget
            // delays (which the firmware computes in CPU clocks, e.g. µs*freq)
            // elapse in ~1/CYCLE_SCALE as many interpreted instructions. Without
            // it the bootloader's entropy fill alone burns ~380M instructions of
            // pure delay. Delay loops only need to *complete*, not match wall
            // time, so a coarse cycle estimate is correct in sim; the real-time
            // CLINT timer (mtime vs mtimecmp, CSR 0xB00) stays unscaled.
            0x7E0..=0x7E2 | 0x802 if self.core_profile == RiscVCoreProfile::Esp32C3 => {
                (self.mtime.wrapping_mul(CYCLE_SCALE) & 0xFFFFFFFF) as u32
            }
            0xC00 if self.core_profile == RiscVCoreProfile::StandardRv32 => {
                (self.mtime.wrapping_mul(CYCLE_SCALE) & 0xFFFFFFFF) as u32
            }
            0xC80 if self.core_profile == RiscVCoreProfile::StandardRv32 => {
                (self.mtime.wrapping_mul(CYCLE_SCALE) >> 32) as u32
            }
            _ => return None,
        })
    }

    fn write_csr(&mut self, csr: u16, val: u32) -> bool {
        match csr {
            0x300 => self.mstatus = val & 0x0000_1888, // Minimal mstatus (MIE, MPP)
            0x304 => self.mie = val,
            0x344 => self.mip = val,
            0x305 => self.mtvec = val,
            0x340 => self.mscratch = val,
            0x341 => self.mepc = val,
            0x342 => self.mcause = val,
            0x343 => self.mtval = val,
            // Physical Memory Protection (PMP) configuration and address registers (stubs)
            0x3A0..=0x3A3 | 0x3B0..=0x3BF => {}
            0x7E0..=0x7E2 | 0x802 if self.core_profile == RiscVCoreProfile::Esp32C3 => {}
            _ => return false,
        }
        true
    }

    fn csr_read_or_trap(&mut self, csr: u16, opcode: u32) -> Option<u32> {
        let value = self.read_csr(csr);
        if value.is_none() {
            self.mtval = opcode;
            self.handle_trap(2, self.pc);
        }
        value
    }

    fn csr_write_or_trap(&mut self, csr: u16, value: u32, opcode: u32) -> bool {
        if self.write_csr(csr, value) {
            true
        } else {
            self.mtval = opcode;
            self.handle_trap(2, self.pc);
            false
        }
    }

    fn handle_trap(&mut self, cause: u32, epc: u32) {
        if std::env::var("LABWIRED_TRAP_DEBUG").is_ok() {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            if N.fetch_add(1, Ordering::Relaxed) < 60 {
                eprintln!(
                    "[trap] cause={cause:#010x} epc={epc:#010x} mtvec={:#010x} ra={:#010x} sp={:#010x} a0={:#010x}",
                    self.mtvec, self.x[1], self.x[2], self.x[10]
                );
            }
        }
        self.mepc = epc;
        self.mcause = cause;
        // mtvec handling (Direct vs Vectored)
        let mode = self.mtvec & 3;
        let base = self.mtvec & !3;
        if mode == 1 && (cause & 0x80000000) != 0 {
            // Vectored interrupt
            let irq = cause & 0x7FFFFFFF;
            self.pc = base + irq * 4;
        } else {
            self.pc = base;
        }
        // Update mstatus per the privileged spec on trap entry:
        //   MPIE <- MIE, MIE <- 0, MPP <- current privilege (M-mode = 0b11).
        // The IDF trap handler saves/restores mstatus around nested interrupts
        // and ends with MRET, which relies on MPIE carrying the pre-trap MIE.
        let mie = (self.mstatus >> 3) & 1;
        self.mstatus &= !(1 << 7); // clear MPIE
        self.mstatus |= mie << 7; // MPIE <- MIE
        self.mstatus &= !(1 << 3); // MIE <- 0
        self.mstatus |= 0b11 << 11; // MPP <- M-mode
    }
}

/// Chunk H: RV32IMC wasm-JIT dispatch helpers for `Machine<RiscV>`.
///
/// These drive the [`RiscvJitEngine`](crate::cpu::jit_framework::riscv::RiscvJitEngine)
/// directly over the machine's raw register file and guest-RAM window
/// (reached by downcasting the `&mut dyn Bus` back to the concrete
/// [`SystemBus`](crate::bus::SystemBus)), retiring compiled blocks atomically
/// while keeping timer/interrupt timing exact.
#[cfg(feature = "jit")]
impl RiscV {
    /// The production correctness gate. The JIT may run only when nothing
    /// needs per-instruction visibility. Poll-mode logic probes, breakpoints,
    /// cycle-accurate peripherals, and the next scheduled event already pin
    /// `Machine::run`'s batch to a single instruction (so the JIT never
    /// engages for them — see the `max_count > 1` guard in `step_batch`);
    /// this checks the two rails that do NOT clamp the batch: per-instruction
    /// observers and push-mode logic taps.
    fn jit_gate_allows(&self, bus: &dyn Bus, observers: &[Arc<dyn SimulationObserver>]) -> bool {
        if !observers.is_empty() {
            return false;
        }
        if bus.logic_tap().is_some_and(|t| t.push_armed()) {
            return false;
        }
        // Belt-and-suspenders: a cycle-accurate bus already forces batch == 1
        // (so we would not be here), but check explicitly in case the caller
        // ever relaxes that clamp.
        if let Some(sb) = bus
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::bus::SystemBus>())
        {
            if sb.requires_cycle_accurate() {
                return false;
            }
        }
        true
    }

    /// Would a compiled block that retires `n` instructions from the current
    /// state step *across* an interrupt the interpreter would have taken
    /// mid-block? If so the caller interprets the stretch instead, so the
    /// trap lands at exactly the instruction the interpreter would trap on.
    ///
    /// The interpreter's interrupt check ([`RiscV::step`] tail) fires only
    /// when global `mstatus.MIE` is set. Within one `Machine::run` batch the
    /// external IRQ lines are stable (peripherals tick only *between* batches),
    /// so the sole source that can *become* pending mid-block is the internal
    /// CLINT timer: the block advances `mtime` by exactly `n`, so if that
    /// crosses `mtimecmp` (with the timer unmasked in `mie`) the MTIP edge —
    /// and its trap — must be observed inside the block.
    fn block_would_cross_irq(&self, bus: &dyn Bus, n: u32) -> bool {
        // Interrupts globally disabled: no trap is taken regardless.
        if (self.mstatus & (1 << 3)) == 0 {
            return false;
        }
        // Something is already pending (or an external line is asserted): the
        // interpreter would trap within one instruction. Do not batch past it.
        if (self.mip & self.mie) != 0 || bus.external_irq_lines() != 0 {
            return true;
        }
        // Internal timer edge inside the block's mtime span.
        let timer_unmasked = (self.mie & (1 << 7)) != 0;
        timer_unmasked
            && self.mtime < self.mtimecmp
            && self.mtimecmp <= self.mtime.wrapping_add(n as u64)
    }

    /// Drive one batch through the JIT engine, returning the true retired
    /// instruction count (never past `max_count`). The engine is moved out of
    /// `self` for the duration so its methods can borrow `self.x` / `self.pc`
    /// and the bus independently, then it is restored.
    fn step_batch_jit(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &crate::SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        let mut engine = self.jit_engine.take().unwrap_or_else(|| {
            crate::cpu::jit_framework::riscv::RiscvJitEngine::new(RISCV_JIT_HOT_THRESHOLD)
        });
        let out = self.run_jit_loop(&mut engine, bus, observers, config, max_count);
        self.jit_engine = Some(engine);
        out
    }

    /// The JIT dispatch loop body. `engine` is a borrowed handle (moved out of
    /// `self` by the caller) so `engine.run_ready(pc, &mut self.x, …)` does not
    /// alias `self`.
    fn run_jit_loop(
        &mut self,
        engine: &mut crate::cpu::jit_framework::riscv::RiscvJitEngine,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &crate::SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        use crate::bus::SystemBus;
        use crate::cpu::jit_framework::block_cache::Lookup;

        // Exact-cycle clock (see the interpreter `step_batch`): republish
        // `batch_start + retired` before each dispatch so an interpreted MMIO
        // read sees the cycle-exact clock. A compiled block never reads the bus
        // clock (it touches only registers + RAM), so republishing before it is
        // harmless, and the arming/reading store is ALWAYS interpreted — hence
        // JIT-on observes the identical clock at every bus access as JIT-off,
        // preserving byte-identity while making counter reads exact.
        #[cfg(feature = "event-scheduler")]
        let exact_clock = config.peripheral_tick_interval > 1;
        #[cfg(feature = "event-scheduler")]
        let batch_start = if exact_clock { bus.current_cycle() } else { 0 };

        let mut retired: u32 = 0;
        while retired < max_count {
            #[cfg(feature = "event-scheduler")]
            if exact_clock {
                bus.publish_cycle(batch_start + retired as u64);
            }
            let pc = self.pc as u64;
            match engine.observe(pc) {
                Lookup::Ready => {
                    let n = engine.ready_instr_count(pc).unwrap_or(0);
                    // Never retire past the batch budget (preserves the
                    // event/IRQ clamp `Machine::run` already applied), and
                    // never let a block cross a mid-block interrupt deadline.
                    let must_interpret =
                        n == 0 || retired + n > max_count || self.block_would_cross_irq(bus, n);
                    if must_interpret {
                        self.step(bus, observers, config)?;
                        engine.note_interpreted();
                        retired += 1;
                    } else if let Some(sb) =
                        bus.as_any_mut().and_then(|a| a.downcast_mut::<SystemBus>())
                    {
                        let (actual_n, next_pc, clear_reservation, needs_interp) =
                            engine.run_ready(pc, &mut self.x, &mut sb.ram.data);
                        if clear_reservation {
                            self.reservation = None;
                        }
                        self.pc = next_pc as u32;
                        // ── THE MTIME FIXUP ──────────────────────────────
                        // A compiled block retires `actual_n` instructions
                        // without calling `step`, so it never advanced the
                        // CLINT `mtime` (the interpreter bumps it 1/instr).
                        // Advance it by exactly `actual_n` here so the cycle
                        // CSRs (0xC00/0x802/0x7E2 = mtime*CYCLE_SCALE) and the
                        // MTIP timer edge stay identical to a per-instruction
                        // run. This is the analogue of Xtensa's CCOUNT += N-1.
                        self.update_mtime_after_elapsed_cycles(actual_n as u64);
                        retired += actual_n;
                        // Entry-instruction memory fault: nothing retired and
                        // the PC did not move — interpret one for progress.
                        if actual_n == 0 && needs_interp {
                            self.step(bus, observers, config)?;
                            engine.note_interpreted();
                            retired += 1;
                        }
                    } else {
                        // Not a `SystemBus` (never happens in production): fall
                        // back to the interpreter for this instruction.
                        self.step(bus, observers, config)?;
                        engine.note_interpreted();
                        retired += 1;
                    }
                }
                Lookup::Interpret { promote } => {
                    if promote {
                        if let Some(sb) = bus.as_any().and_then(|a| a.downcast_ref::<SystemBus>()) {
                            engine.try_compile_from_bus(pc, sb);
                        }
                    }
                    self.step(bus, observers, config)?;
                    engine.note_interpreted();
                    retired += 1;
                }
            }
            // Mirror the interpreter batch's idle fast-forward early-exit so
            // enabling the JIT never changes when a batch returns short.
            if config.idle_fast_forward_enabled && self.idle_fast_forward_budget(bus).is_some() {
                return Ok(retired);
            }
            // Mirror the interpreter's Gap #1 mid-batch-arming early-exit (see
            // `step_batch`). A compiled block never executes an MMIO store, so
            // `pending_schedule` can only have been populated by an interpreted
            // instruction (`self.step` above) — the SAME instruction the JIT-off
            // interpreter would break on. Ending the batch at that identical
            // point keeps JIT-on byte-identical to JIT-off while making the
            // just-armed event deliverable at its exact cycle by the next batch.
            #[cfg(feature = "event-scheduler")]
            if config.peripheral_tick_interval > 1 && bus.has_pending_schedule() {
                return Ok(retired);
            }
        }
        Ok(retired)
    }

    /// Accumulated JIT engine stats — `None` if the engine was never created
    /// (the JIT never ran). Used by the differential merge-gate test to assert
    /// the compiled path was non-vacuously exercised.
    pub fn jit_stats(&self) -> Option<crate::cpu::jit_framework::riscv::EngineStats> {
        self.jit_engine.as_ref().map(|e| e.stats())
    }
}

impl Cpu for RiscV {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        self.pc = 0;
        Ok(())
    }

    /// Mirror the RV32IMC JIT engine's counters into the feature-agnostic
    /// [`crate::CpuJitStats`] so generic callers can prove non-vacuity. Only
    /// present under `jit`; without it the trait default (`None`) applies.
    #[cfg(feature = "jit")]
    fn jit_engine_stats(&self) -> Option<crate::CpuJitStats> {
        self.jit_stats().map(|s| crate::CpuJitStats {
            compiled: s.compiled,
            block_runs: s.block_runs,
            block_instrs: s.block_instrs,
            interpreted: s.interpreted,
        })
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        _config: &crate::SimulationConfig,
    ) -> SimResult<()> {
        self.waiting_for_interrupt = false;
        let opcode = self.fetch_opcode_u32(bus)?;

        let retired_pc = self.pc;
        for observer in observers {
            observer.on_step_start(self.pc, opcode);
        }

        let cache_idx = ((self.pc >> 1) & 0xFFF) as usize;
        let cached = if _config.decode_cache_enabled {
            self.decode_cache[cache_idx]
                .filter(|entry| entry.tag == self.pc && entry.opcode == opcode)
        } else {
            None
        };
        let (instruction, inst_len) = if let Some(entry) = cached {
            (entry.instruction, entry.inst_len as u32)
        } else {
            let instruction = decode_rv32(opcode);
            let inst_len = if (opcode & 0x3) == 0x3 { 4 } else { 2 };
            if _config.decode_cache_enabled {
                self.decode_cache[cache_idx] = Some(RiscVDecodeCacheEntry {
                    tag: self.pc,
                    opcode,
                    instruction,
                    inst_len: inst_len as u8,
                });
            }
            (instruction, inst_len)
        };
        tracing::debug!(
            "PC={:#x}, Op={:#08x}, Instr={:?}, Len={}",
            self.pc,
            opcode,
            instruction,
            inst_len
        );

        let mut next_pc = self.pc.wrapping_add(inst_len);

        match instruction {
            Instruction::Lui { rd, imm } => {
                self.write_reg(rd, imm);
            }
            Instruction::Auipc { rd, imm } => {
                let val = self.pc.wrapping_add(imm);
                self.write_reg(rd, val);
            }
            Instruction::Jal { rd, imm } => {
                let target = self.pc.wrapping_add(imm as u32);
                // Link address is the NEXT instruction: pc + inst_len. The
                // decoder maps the 2-byte C.JAL to Jal, so a hardcoded +4 would
                // set ra 2 bytes too far and corrupt every compressed call's
                // return — use inst_len so c.jal links pc+2 and jal links pc+4.
                self.write_reg(rd, self.pc.wrapping_add(inst_len));
                next_pc = target;
            }
            Instruction::Jalr { rd, rs1, imm } => {
                let base = self.read_reg(rs1);
                let target = base.wrapping_add(imm as u32) & !1;
                self.write_reg(rd, self.pc.wrapping_add(inst_len));
                next_pc = target;
            }
            Instruction::Beq { rs1, rs2, imm } => {
                if self.read_reg(rs1) == self.read_reg(rs2) {
                    next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bne { rs1, rs2, imm } => {
                if self.read_reg(rs1) != self.read_reg(rs2) {
                    next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Blt { rs1, rs2, imm } => {
                if (self.read_reg(rs1) as i32) < (self.read_reg(rs2) as i32) {
                    next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bge { rs1, rs2, imm } => {
                if (self.read_reg(rs1) as i32) >= (self.read_reg(rs2) as i32) {
                    next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bltu { rs1, rs2, imm } => {
                if self.read_reg(rs1) < self.read_reg(rs2) {
                    next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bgeu { rs1, rs2, imm } => {
                if self.read_reg(rs1) >= self.read_reg(rs2) {
                    next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Lb { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u8(addr as u64)? as i8;
                self.write_reg(rd, val as i32 as u32);
            }
            Instruction::Lh { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u16(addr as u64)? as i16;
                self.write_reg(rd, val as i32 as u32);
            }
            Instruction::Lw { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u32(addr as u64)?;
                self.write_reg(rd, val);
            }
            Instruction::Lbu { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u8(addr as u64)?;
                self.write_reg(rd, val as u32);
            }
            Instruction::Lhu { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u16(addr as u64)?;
                self.write_reg(rd, val as u32);
            }
            Instruction::Sb { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2) as u8;
                bus.write_u8(addr as u64, val)?;
                self.reservation = None;
            }
            Instruction::Sh { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2) as u16;
                bus.write_u16(addr as u64, val)?;
                self.reservation = None;
            }
            Instruction::Sw { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2);
                bus.write_u32(addr as u64, val)?;
                self.reservation = None;
            }
            Instruction::Addi { rd, rs1, imm } => {
                let res = self.read_reg(rs1).wrapping_add(imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::Slti { rd, rs1, imm } => {
                let val = if (self.read_reg(rs1) as i32) < imm {
                    1
                } else {
                    0
                };
                self.write_reg(rd, val);
            }
            Instruction::Sltiu { rd, rs1, imm } => {
                let val = if self.read_reg(rs1) < (imm as u32) {
                    1
                } else {
                    0
                };
                self.write_reg(rd, val);
            }
            Instruction::Xori { rd, rs1, imm } => {
                let res = self.read_reg(rs1) ^ (imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::Ori { rd, rs1, imm } => {
                let res = self.read_reg(rs1) | (imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::Andi { rd, rs1, imm } => {
                let res = self.read_reg(rs1) & (imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::Slli { rd, rs1, shamt } => {
                let res = self.read_reg(rs1) << shamt;
                self.write_reg(rd, res);
            }
            Instruction::Srli { rd, rs1, shamt } => {
                let res = self.read_reg(rs1) >> shamt;
                self.write_reg(rd, res);
            }
            Instruction::Srai { rd, rs1, shamt } => {
                let res = (self.read_reg(rs1) as i32) >> shamt;
                self.write_reg(rd, res as u32);
            }
            Instruction::Add { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1).wrapping_add(self.read_reg(rs2));
                self.write_reg(rd, res);
            }
            Instruction::Sub { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1).wrapping_sub(self.read_reg(rs2));
                self.write_reg(rd, res);
            }
            Instruction::Sll { rd, rs1, rs2 } => {
                let shamt = self.read_reg(rs2) & 0x1F;
                let res = self.read_reg(rs1) << shamt;
                self.write_reg(rd, res);
            }
            Instruction::Slt { rd, rs1, rs2 } => {
                let val = if (self.read_reg(rs1) as i32) < (self.read_reg(rs2) as i32) {
                    1
                } else {
                    0
                };
                self.write_reg(rd, val);
            }
            Instruction::Sltu { rd, rs1, rs2 } => {
                let val = if self.read_reg(rs1) < self.read_reg(rs2) {
                    1
                } else {
                    0
                };
                self.write_reg(rd, val);
            }
            Instruction::Xor { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1) ^ self.read_reg(rs2);
                self.write_reg(rd, res);
            }
            Instruction::Srl { rd, rs1, rs2 } => {
                let shamt = self.read_reg(rs2) & 0x1F;
                let res = self.read_reg(rs1) >> shamt;
                self.write_reg(rd, res);
            }
            Instruction::Sra { rd, rs1, rs2 } => {
                let shamt = self.read_reg(rs2) & 0x1F;
                let res = (self.read_reg(rs1) as i32) >> shamt;
                self.write_reg(rd, res as u32);
            }
            Instruction::Or { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1) | self.read_reg(rs2);
                self.write_reg(rd, res);
            }
            Instruction::And { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1) & self.read_reg(rs2);
                self.write_reg(rd, res);
            }
            Instruction::Fence => {
                // No-op in single threaded core model
            }
            Instruction::Wfi => {
                // Wait-for-interrupt: implemented as a no-op busy-wait. The step
                // loop already polls pending interrupts every instruction, so
                // the idle task's WFI spin wakes as soon as a line asserts.
                self.waiting_for_interrupt = true;
            }
            Instruction::Ecall | Instruction::Ebreak => {
                // Should trap. For now, we can just log or halt.
                tracing::warn!("ECALL/EBREAK encountered at {:#x}", self.pc);
                self.handle_trap(
                    if instruction == Instruction::Ecall {
                        11
                    } else {
                        3
                    },
                    self.pc,
                );
                return Ok(());
            }
            Instruction::Mret => {
                // Return from trap. Per the privileged spec:
                //   MIE <- MPIE, MPIE <- 1 (privilege <- MPP, but we stay M-mode).
                self.pc = self.mepc;
                let mpie = (self.mstatus >> 7) & 1;
                self.mstatus &= !(1 << 3); // clear MIE
                self.mstatus |= mpie << 3; // MIE <- MPIE
                self.mstatus |= 1 << 7; // MPIE <- 1
                return Ok(());
            }
            Instruction::Csrrw { rd, rs1, csr } => {
                let Some(old) = self.csr_read_or_trap(csr, opcode) else {
                    return Ok(());
                };
                let val = self.read_reg(rs1);
                if !self.csr_write_or_trap(csr, val, opcode) {
                    return Ok(());
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrs { rd, rs1, csr } => {
                let Some(old) = self.csr_read_or_trap(csr, opcode) else {
                    return Ok(());
                };
                if rs1 != 0 {
                    let val = self.read_reg(rs1);
                    if !self.csr_write_or_trap(csr, old | val, opcode) {
                        return Ok(());
                    }
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrc { rd, rs1, csr } => {
                let Some(old) = self.csr_read_or_trap(csr, opcode) else {
                    return Ok(());
                };
                if rs1 != 0 {
                    let val = self.read_reg(rs1);
                    if !self.csr_write_or_trap(csr, old & !val, opcode) {
                        return Ok(());
                    }
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrwi { rd, imm, csr } => {
                let Some(old) = self.csr_read_or_trap(csr, opcode) else {
                    return Ok(());
                };
                if !self.csr_write_or_trap(csr, imm as u32, opcode) {
                    return Ok(());
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrsi { rd, imm, csr } => {
                let Some(old) = self.csr_read_or_trap(csr, opcode) else {
                    return Ok(());
                };
                if imm != 0 && !self.csr_write_or_trap(csr, old | (imm as u32), opcode) {
                    return Ok(());
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrci { rd, imm, csr } => {
                let Some(old) = self.csr_read_or_trap(csr, opcode) else {
                    return Ok(());
                };
                if imm != 0 && !self.csr_write_or_trap(csr, old & !(imm as u32), opcode) {
                    return Ok(());
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            // RV32M Extension
            Instruction::Mul { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1).wrapping_mul(self.read_reg(rs2));
                self.write_reg(rd, res);
            }
            Instruction::Mulh { rd, rs1, rs2 } => {
                let res = (self.read_reg(rs1) as i32 as i64)
                    .wrapping_mul(self.read_reg(rs2) as i32 as i64);
                self.write_reg(rd, (res >> 32) as u32);
            }
            Instruction::Mulhsu { rd, rs1, rs2 } => {
                let res = (self.read_reg(rs1) as i32 as i64)
                    .wrapping_mul(self.read_reg(rs2) as u64 as i64);
                self.write_reg(rd, (res >> 32) as u32);
            }
            Instruction::Mulhu { rd, rs1, rs2 } => {
                let res = (self.read_reg(rs1) as u64).wrapping_mul(self.read_reg(rs2) as u64);
                self.write_reg(rd, (res >> 32) as u32);
            }
            Instruction::Div { rd, rs1, rs2 } => {
                let dividend = self.read_reg(rs1) as i32;
                let divisor = self.read_reg(rs2) as i32;
                let res = if divisor == 0 {
                    -1
                } else if dividend == i32::MIN && divisor == -1 {
                    dividend
                } else {
                    dividend / divisor
                };
                self.write_reg(rd, res as u32);
            }
            Instruction::Divu { rd, rs1, rs2 } => {
                let dividend = self.read_reg(rs1);
                let divisor = self.read_reg(rs2);
                let res = dividend.checked_div(divisor).unwrap_or(u32::MAX);
                self.write_reg(rd, res);
            }
            Instruction::Rem { rd, rs1, rs2 } => {
                let dividend = self.read_reg(rs1) as i32;
                let divisor = self.read_reg(rs2) as i32;
                let res = if divisor == 0 {
                    dividend
                } else if dividend == i32::MIN && divisor == -1 {
                    0
                } else {
                    dividend % divisor
                };
                self.write_reg(rd, res as u32);
            }
            Instruction::Remu { rd, rs1, rs2 } => {
                let dividend = self.read_reg(rs1);
                let divisor = self.read_reg(rs2);
                let res = if divisor == 0 {
                    dividend
                } else {
                    dividend % divisor
                };
                self.write_reg(rd, res);
            }
            // RV32C Extension
            Instruction::CAddi { rd, imm } => {
                if rd != 0 {
                    let res = self.read_reg(rd).wrapping_add(imm as u32);
                    self.write_reg(rd, res);
                }
            }
            Instruction::CLi { rd, imm } => {
                if rd != 0 {
                    self.write_reg(rd, imm as u32);
                }
            }
            Instruction::CMv { rd, rs2 } => {
                if rd != 0 {
                    let val = self.read_reg(rs2);
                    self.write_reg(rd, val);
                }
            }
            Instruction::CAddi16sp { imm } => {
                let sp = self.read_reg(2);
                self.write_reg(2, sp.wrapping_add(imm as u32));
            }
            Instruction::CAddi4spn { rd, imm } => {
                let sp = self.read_reg(2);
                self.write_reg(rd, sp.wrapping_add(imm));
            }
            Instruction::CLw { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm);
                let val = bus.read_u32(addr as u64)?;
                self.write_reg(rd, val);
            }
            Instruction::CSw { rs2, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm);
                let val = self.read_reg(rs2);
                bus.write_u32(addr as u64, val)?;
                self.reservation = None;
            }
            Instruction::CLwsp { rd, imm } => {
                let sp = self.read_reg(2);
                let addr = sp.wrapping_add(imm);
                let val = bus.read_u32(addr as u64)?;
                self.write_reg(rd, val);
            }
            Instruction::CSwsp { rs2, imm } => {
                let sp = self.read_reg(2);
                let addr = sp.wrapping_add(imm);
                let val = self.read_reg(rs2);
                bus.write_u32(addr as u64, val)?;
                self.reservation = None;
            }
            Instruction::CJr { rs1 } => {
                next_pc = self.read_reg(rs1) & !1;
            }
            Instruction::CJalr { rs1 } => {
                let target = self.read_reg(rs1) & !1;
                self.write_reg(1, self.pc.wrapping_add(2));
                next_pc = target;
            }
            Instruction::CJ { imm } => {
                next_pc = self.pc.wrapping_add(imm as u32);
            }
            Instruction::CBeqz { rs1, imm } => {
                if self.read_reg(rs1) == 0 {
                    next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::CBnez { rs1, imm } => {
                if self.read_reg(rs1) != 0 {
                    next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::CSli { rd, shamt } => {
                if rd != 0 {
                    let res = self.read_reg(rd) << shamt;
                    self.write_reg(rd, res);
                }
            }

            // ---- RV32A: atomic memory operations (word) ----
            //
            // Single-hart semantics: aq/rl are ignored. LR.W records a
            // reservation on the effective address; SC.W succeeds iff the
            // current reservation matches its effective address. Any store
            // (including any AMO*) invalidates the reservation per §8.2.
            Instruction::LrW { rd, rs1 } => {
                let addr = self.read_reg(rs1);
                let val = bus.read_u32(addr as u64)?;
                self.write_reg(rd, val);
                self.reservation = Some(addr);
            }
            Instruction::ScW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let store_ok = self.reservation == Some(addr);
                if store_ok {
                    bus.write_u32(addr as u64, self.read_reg(rs2))?;
                    self.write_reg(rd, 0); // success
                } else {
                    self.write_reg(rd, 1); // failure
                }
                self.reservation = None;
            }
            Instruction::AmoSwapW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                bus.write_u32(addr as u64, self.read_reg(rs2))?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoAddW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                bus.write_u32(addr as u64, old.wrapping_add(self.read_reg(rs2)))?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoXorW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                bus.write_u32(addr as u64, old ^ self.read_reg(rs2))?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoOrW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                bus.write_u32(addr as u64, old | self.read_reg(rs2))?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoAndW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                bus.write_u32(addr as u64, old & self.read_reg(rs2))?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMinW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                let rhs = self.read_reg(rs2);
                let new = (old as i32).min(rhs as i32) as u32;
                bus.write_u32(addr as u64, new)?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMaxW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                let rhs = self.read_reg(rs2);
                let new = (old as i32).max(rhs as i32) as u32;
                bus.write_u32(addr as u64, new)?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMinuW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                let new = old.min(self.read_reg(rs2));
                bus.write_u32(addr as u64, new)?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMaxuW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                let new = old.max(self.read_reg(rs2));
                bus.write_u32(addr as u64, new)?;
                self.write_reg(rd, old);
                self.reservation = None;
            }

            Instruction::Unknown(inst) => {
                tracing::error!("Unknown instruction {:#x} at {:#x}", inst, self.pc);
                return Err(crate::SimulationError::DecodeError(self.pc as u64));
            }
        }

        // Timer update (Internal minimal CLINT)
        self.update_mtime_after_elapsed_cycles(1);

        // Check for interrupts. On the ESP32-C3 the custom interrupt controller
        // exposes its 31 sources as CPU interrupt lines 1..31 directly in
        // mip/mie (no standard MEIP/MSIP/MTIP semantics); the bus drives those
        // lines level-sensitively via `external_irq_lines()` after routing
        // asserted sources through the interrupt matrix. OR them into the local
        // mip view so a line stays asserted only while its source does.
        if (self.mstatus & (1 << 3)) != 0 {
            // Standard machine sources are masked by `mie`; ESP32-C3 external
            // lines arrive already gated (enable + priority/threshold) by the
            // bus, so they bypass `mie` (which the C3 firmware leaves at 0).
            let pending = (self.mip & self.mie) | bus.external_irq_lines();
            if pending != 0 {
                // Standard machine interrupts keep their spec priority
                // (External > Software > Timer); any other set bit is an ESP
                // interrupt-matrix line, taken highest-line-first.
                let irq = if (pending & (1 << 11)) != 0 {
                    11
                } else if (pending & (1 << 3)) != 0 {
                    3
                } else if (pending & (1 << 7)) != 0 {
                    7
                } else {
                    31 - pending.leading_zeros()
                };

                if irq != 0xFFFFFFFF {
                    // Per RISC-V privileged spec §3.1.17, an async interrupt
                    // must save the address of the *next* instruction into
                    // mepc so MRET resumes forward. self.pc is still pointing
                    // at the instruction we just finished executing — passing
                    // it would cause MRET to re-execute, doubling side effects
                    // (ADDI counted twice, stores applied twice, etc).
                    if !observers.is_empty() {
                        crate::emit_trace_event(
                            observers,
                            labwired_hw_trace::TraceEvent::InstructionRetired {
                                pc: retired_pc,
                                opcode,
                            },
                        );
                    }
                    self.handle_trap(0x80000000 | irq, next_pc);
                    // Trap taken, next instruction will be handled in trap handler
                    return Ok(());
                }
            }
        }

        self.pc = next_pc;

        // Building the register snapshot is pure waste when nothing observes it,
        // and this runs on every instruction. Gate it on having observers.
        if !observers.is_empty() {
            let mut registers = [0u32; 33];
            registers[..32].copy_from_slice(&self.x);
            registers[32] = self.pc;

            crate::emit_trace_event(
                observers,
                labwired_hw_trace::TraceEvent::InstructionRetired {
                    pc: retired_pc,
                    opcode,
                },
            );
            for obs in observers {
                obs.on_step_end(inst_len, &registers);
            }
        }

        Ok(())
    }

    fn step_batch(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &crate::SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        // ── Chunk H JIT fast-path ─────────────────────────────────────────
        // When the RV32IMC wasm-JIT is opted in AND nothing needs
        // per-instruction visibility, drive this batch through the JIT
        // engine (compiled blocks + interpreter fallback). Off by default,
        // and only compiled under `jit`; the interpreter loop below is the
        // reference path (byte-identical to a non-`jit` build when the flag
        // is off). `max_count <= 1` batches skip the JIT: `Machine::run`
        // clamps the batch to one instruction precisely when it needs a
        // per-instruction boundary (breakpoint set, poll-mode logic probe,
        // cycle-accurate peripheral, next scheduled event), so restricting
        // the JIT to multi-instruction batches folds all those correctness
        // rails into one cheap check.
        #[cfg(feature = "jit")]
        {
            self.jit_enabled = config.riscv_jit_enabled;
            if self.jit_enabled && max_count > 1 && self.jit_gate_allows(bus, observers) {
                return self.step_batch_jit(bus, observers, config, max_count);
            }
        }

        // Push-mode logic capture: advance the tap clock once per retired
        // instruction while armed, so MMIO pad writes stamp with the cycle
        // boundary they become observable at (see `crate::logic_capture`).
        let tap = bus.logic_tap().filter(|t| t.push_armed());
        // Exact-cycle clock (event-scheduler, widened interval): the bus mirror
        // is seeded once per batch by `Machine::run`, so a mid-batch MMIO read of
        // a lazily-derived counter would see the STALE batch-start cycle. Capture
        // the batch-start cycle here and republish `batch_start + i` before each
        // instruction so those reads are cycle-EXACT — identical to interval-1
        // (where the batch is one instruction). This is what removes the last
        // cpu_state divergence: a firmware busy-waiting on a lazy counter now
        // exits its poll on the same instruction at any tick interval. Skipped at
        // interval 1 (already exact) so that hot path is byte-unchanged.
        #[cfg(feature = "event-scheduler")]
        let exact_clock = config.peripheral_tick_interval > 1;
        #[cfg(feature = "event-scheduler")]
        let batch_start = if exact_clock { bus.current_cycle() } else { 0 };
        for i in 0..max_count {
            if let Some(tap) = &tap {
                tap.bump_clock();
            }
            #[cfg(feature = "event-scheduler")]
            if exact_clock {
                bus.publish_cycle(batch_start + i as u64);
            }
            self.step(bus, observers, config)?;
            if config.idle_fast_forward_enabled && self.idle_fast_forward_budget(bus).is_some() {
                return Ok(i + 1);
            }
            // Event-scheduler Gap #1 (mid-batch arming): if this instruction was
            // an MMIO write that armed a peripheral event (now sitting in the
            // bus `pending_schedule`, not yet in the scheduler heap), END the
            // batch here so `Machine::run`'s post-batch drain enqueues it and the
            // NEXT batch's `next_event_deadline` clamp delivers it at its exact
            // absolute cycle — instead of this widened batch overrunning the
            // deadline and the event firing a batch late (which shifts the ISR
            // entry instruction and diverges `cpu_state`). Gated on interval > 1:
            // at interval 1 the batch is already one instruction so this never
            // fires, keeping the interval-1 hot path (and every walk-identity
            // gate) byte-unchanged and cost-free. Because compiled JIT blocks
            // never execute MMIO stores (they touch only registers + RAM), the
            // arming store is ALWAYS interpreted in both JIT-on and JIT-off, so
            // mirroring this check in `run_jit_loop` breaks both arms at the same
            // instruction — the JIT-on/off byte-identity gate is preserved.
            #[cfg(feature = "event-scheduler")]
            if config.peripheral_tick_interval > 1 && bus.has_pending_schedule() {
                return Ok(i + 1);
            }
        }
        Ok(max_count)
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }
    fn get_pc(&self) -> u32 {
        self.pc
    }
    fn set_sp(&mut self, val: u32) {
        self.write_reg(2, val); // x2 is SP
    }
    fn set_exception_pending(&mut self, _exception_num: u32) {
        // For RISC-V Machine mode, external interrupts are routed to MEIP (bit 11).
        // The specific 'exception_num' (IRQ) would be tracked by a PLIC.
        // Since we don't have a PLIC yet, we pend a generic external interrupt.
        self.mip |= 1 << 11;
    }

    fn idle_fast_forward_budget(&self, bus: &dyn Bus) -> Option<u64> {
        if !self.waiting_for_interrupt {
            return None;
        }
        if ((self.mip & self.mie) | bus.external_irq_lines()) != 0 {
            return None;
        }
        if self.mtimecmp == u64::MAX {
            return Some(u64::MAX);
        }
        if self.mtime + 1 >= self.mtimecmp {
            return None;
        }
        Some(self.mtimecmp - self.mtime - 1)
    }

    fn fast_forward_idle_cycles(&mut self, cycles: u64) {
        self.update_mtime_after_elapsed_cycles(cycles);
    }

    fn get_register(&self, id: u8) -> u32 {
        if id < 32 {
            self.read_reg(id)
        } else if id == 32 {
            self.pc
        } else {
            0
        }
    }
    fn set_register(&mut self, id: u8, val: u32) {
        if id < 32 {
            self.write_reg(id, val);
        } else if id == 32 {
            self.pc = val;
        }
    }

    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        crate::snapshot::CpuSnapshot::RiscV(crate::snapshot::RiscVCpuSnapshot {
            registers: self.x.to_vec(),
            pc: self.pc,
            mstatus: self.mstatus,
            mie: self.mie,
            mip: self.mip,
            mtvec: self.mtvec,
            mscratch: self.mscratch,
            mepc: self.mepc,
            mcause: self.mcause,
            mtval: self.mtval,
            mtime: self.mtime,
            mtimecmp: self.mtimecmp,
        })
    }

    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        if let crate::snapshot::CpuSnapshot::RiscV(s) = snapshot {
            for (i, &val) in s.registers.iter().enumerate().take(32) {
                self.x[i] = val;
            }
            self.pc = s.pc;
            self.mstatus = s.mstatus;
            self.mie = s.mie;
            self.mip = s.mip;
            self.mtvec = s.mtvec;
            self.mscratch = s.mscratch;
            self.mepc = s.mepc;
            self.mcause = s.mcause;
            self.mtval = s.mtval;
            self.mtime = s.mtime;
            self.mtimecmp = s.mtimecmp;
        }
    }

    fn runtime_snapshot(&self) -> (crate::runtime_snapshot::CpuKind, Vec<u8>) {
        use crate::runtime_snapshot::RiscVRuntimeSnapshot;
        let snap = RiscVRuntimeSnapshot {
            x: self.x,
            pc: self.pc,
            mstatus: self.mstatus,
            mie: self.mie,
            mip: self.mip,
            mtvec: self.mtvec,
            mscratch: self.mscratch,
            mepc: self.mepc,
            mcause: self.mcause,
            mtval: self.mtval,
            mtime: self.mtime,
            mtimecmp: self.mtimecmp,
            reservation: self.reservation,
        };
        let bytes = bincode::serialize(&snap).expect("bincode serialize RiscVRuntimeSnapshot");
        (crate::runtime_snapshot::CpuKind::RiscV, bytes)
    }

    fn apply_runtime_snapshot(
        &mut self,
        kind: crate::runtime_snapshot::CpuKind,
        bytes: &[u8],
    ) -> SimResult<()> {
        use crate::runtime_snapshot::{CpuKind, RiscVRuntimeSnapshot};
        if kind != CpuKind::RiscV {
            return Err(crate::SimulationError::NotImplemented(format!(
                "apply_runtime_snapshot: kind {kind:?} given to RiscV"
            )));
        }
        let snap: RiscVRuntimeSnapshot = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("RiscV snapshot decode: {e}"))
        })?;
        self.x = snap.x;
        self.x[0] = 0; // x0 is hardwired to zero regardless of the blob.
        self.pc = snap.pc;
        self.mstatus = snap.mstatus;
        self.mie = snap.mie;
        self.mip = snap.mip;
        self.mtvec = snap.mtvec;
        self.mscratch = snap.mscratch;
        self.mepc = snap.mepc;
        self.mcause = snap.mcause;
        self.mtval = snap.mtval;
        self.mtime = snap.mtime;
        self.mtimecmp = snap.mtimecmp;
        self.reservation = snap.reservation;
        Ok(())
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for i in 0..32 {
            names.push(format!("x{}", i));
        }
        names.push("pc".to_string());
        names
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        if let Some(stripped) = name.strip_prefix('x') {
            stripped.parse().ok()
        } else if name.to_lowercase() == "pc" {
            Some(32)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::DebugControl;
    use crate::Machine;

    #[test]
    fn test_riscv_addi() {
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        // ADDI x1, x0, 5  (x1 = 0 + 5)
        // Op=0x13, rd=1, funct3=0, rs1=0, imm=5
        // 000000000101 00000 000 00001 0010011 -> 0x00500093
        bus.flash.data = vec![
            0x93, 0x00, 0x50, 0x00, // ADDI x1, x0, 5
        ];

        cpu.pc = 0x0000_0000;
        let mut machine = Machine::new(cpu, bus);
        machine.step().unwrap();

        assert_eq!(machine.cpu.read_reg(1), 5);
        assert_eq!(machine.cpu.pc, 4);
    }

    #[test]
    fn esp32c3_rejects_standard_cycle_csr_but_exposes_pccr_machine() {
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        // CSRRS x5, x0, cycle (0xC00): standard RISC-V cycle CSR is not
        // implemented by the ESP32-C3 core; it must raise illegal instruction.
        let read_standard_cycle = (0xC00u32 << 20) | (5 << 7) | (0b010 << 12) | 0x73;
        bus.flash.data = read_standard_cycle.to_le_bytes().to_vec();
        cpu.pc = 0;
        cpu.mtvec = 0x100;
        let mut machine = Machine::new(cpu, bus);
        machine.step().unwrap();

        assert_eq!(
            machine.cpu.mcause, 2,
            "unsupported CSR must trap as illegal instruction"
        );
        assert_eq!(machine.cpu.mtval, read_standard_cycle);
        assert_eq!(machine.cpu.pc, 0x100);
        assert_eq!(
            machine.cpu.read_csr(0x7E2),
            Some(0),
            "C3 PCCR_MACHINE remains implemented"
        );
    }

    #[test]
    fn standard_rv32_profile_exposes_cycle_csr() {
        let cpu = RiscV::new_for(RiscVCoreProfile::StandardRv32);
        assert_eq!(cpu.read_csr(0xC00), Some(0));
        assert_eq!(cpu.read_csr(0x7E2), None);
    }

    #[test]
    fn test_riscv_beq_taken() {
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        // 1. ADDI x1, x0, 10
        // 2. ADDI x2, x0, 10
        // 3. BEQ x1, x2, +8 (skip next instruction)
        // 4. ADDI x3, x0, 1 (should be skipped)
        // 5. ADDI x4, x0, 1 (target)

        // imm for BEQ +8:
        // 0x00000063 (BEQ x0, x0, 0)
        // imm[12]=0, imm[10:5]=0, imm[4:1]=4 (bit 3), imm[11]=0
        // offset = 8. binary: 1000.
        // imm[12] = 0
        // imm[11] = 0
        // imm[10:5] = 000000
        // imm[4:1] = 0100 (4)
        // opcode = 1100011 (0x63)
        // funct3 = 000
        // rs1 = 1, rs2 = 2

        // BEQ x1, x2, 8 -> 0x00208463
        // 0000 0000 0010 0000 1000 0100 0110 0011 -> 0x00208463 ?
        // imm[12]=0, imm[10:5]=000000.
        // imm[4:1]=0100. bit 3 is set.
        // imm[11]=0.
        // Verify encoding: https://luplab.gitlab.io/rvcodecjs/#q=beq%20x1,x2,8
        // 00208463

        bus.flash.data = vec![
            0x93, 0x00, 0xA0, 0x00, // ADDI x1, x0, 10 (0x00A00093)
            0x13, 0x01, 0xA0, 0x00, // ADDI x2, x0, 10 (0x00A00113) - wait, rs1=0.
            // ADDI x2, x0, 10: imm=10, rs1=0, funct3=0, rd=2, opcode=0x13
            // 000000001010 00000 000 00010 0010011 -> 0x00A00113. Correct.

            // BEQ x1, x2, 8
            // 0000000 00010 00001 000 01000 1100011 -> 0x00208463
            // imm[12]=0, imm[10:5]=0, rs2=2, rs1=1, funct3=0, imm[4:1]=0100 (+8?), imm[11]=0, opcode=0x63.
            // imm[4:1]=4 -> bit 3 is 1? No, imm[4:1] bits are at positions 11-8.
            // imm[4:1] = 0100 means bit 3 is 1. Yes 1<<3 = 8.
            0x63, 0x84, 0x20, 0x00,
            // Should be skipped (PC+4 from BEQ = 12. BEQ target = 8 + 8 = 16. Wait. PC of BEQ is 8. Target = 8+8=16.)
            // Offset is from current PC.
            // 0: ADDI x1
            // 4: ADDI x2
            // 8: BEQ
            // 12: ADDI x3 (skipped)
            // 16: ADDI x4 (target)
            0x13, 0x01, 0x10,
            0x00, // ADDI x3, x0, 1 (0x00100193) - wait this is ADDI x3, x0, 1.
            0x13, 0x02, 0x10, 0x00, // ADDI x4, x0, 1 (0x00100213).
        ];

        cpu.pc = 0x0000_0000;
        let mut machine = Machine::new(cpu, bus);

        // Step 1: x1 = 10
        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(1), 10);

        // Step 2: x2 = 10
        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(2), 10);

        // Step 3: BEQ taken -> PC = 8 + 8 = 16
        assert_eq!(machine.cpu.pc, 8);
        machine.step().unwrap();
        assert_eq!(machine.cpu.pc, 16);

        // Step 4: ADDI x4, x0, 1
        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(4), 1);

        // Ensure x3 is still 0
        assert_eq!(machine.cpu.read_reg(3), 0);
    }

    #[test]
    fn test_riscv_timer_interrupt() {
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();

        cpu.mtvec = 0x2000;
        cpu.mie = 1 << 7; // MTIE
        cpu.mstatus = 1 << 3; // MIE
        cpu.mtimecmp = 5;

        // Reset memory to hold our test program
        bus.flash.data = vec![0; 0x3000];
        // 0x0: JAL x0, 0 (Infinite loop)
        bus.write_u32(0x0, 0x0000006F).unwrap();
        // 0x2000: ADDI x10, x10, 1
        bus.write_u32(0x2000, 0x00150513).unwrap();
        // 0x2004: MRET
        bus.write_u32(0x2004, 0x30200073).unwrap();

        cpu.pc = 0x0;
        let mut machine = Machine::new(cpu, bus);

        // Step 1-4: mtime increases from 0->1, 1->2, 2->3, 3->4. No interrupt yet.
        for i in 0..4 {
            machine.step().unwrap();
            assert_eq!(machine.cpu.pc, 0, "Should be in loop at step {}", i);
        }

        // Step 5: mtime becomes 5, which equals mtimecmp. Trap should be taken.
        machine.step().unwrap();
        assert_eq!(machine.cpu.pc, 0x2000, "Trap should jump to mtvec");

        // Step 6: Execute ISR ADDI x10, x10, 1
        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(10), 1);
        assert_eq!(machine.cpu.pc, 0x2004);

        // Step 7: Execute MRET
        machine.step().unwrap();
        assert_eq!(machine.cpu.pc, 0, "MRET should return to 0x0");
        assert!(
            (machine.cpu.mstatus & (1 << 3)) != 0,
            "MIE should be re-enabled"
        );
    }

    #[test]
    fn test_riscv_async_irq_mepc_points_to_next_instruction() {
        // Regression test for the "doubled side effect" bug. Prior to fix,
        // an async IRQ taken at a non-branch instruction stored self.pc
        // (= the just-executed PC) into mepc. MRET then re-executed that
        // instruction, incrementing the ADDI counter twice. Branch
        // instructions (like JAL-to-self) accidentally masked the bug
        // because next_pc == self.pc for self-loops.
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();

        cpu.mtvec = 0x2000;
        cpu.mie = 1 << 7; // MTIE
        cpu.mstatus = 1 << 3; // MIE
        cpu.mtimecmp = 3;

        bus.flash.data = vec![0; 0x3000];
        // Straight-line code: every step advances PC by 4, next_pc != pc.
        // 0x0:  ADDI x10, x10, 1     ; x10 = 1
        // 0x4:  ADDI x10, x10, 1     ; x10 = 2, mtime hits mtimecmp here
        // 0x8:  ADDI x10, x10, 1     ; would make x10 = 3 but interrupt intervenes
        bus.write_u32(0x0, 0x00150513).unwrap();
        bus.write_u32(0x4, 0x00150513).unwrap();
        bus.write_u32(0x8, 0x00150513).unwrap();
        // ISR at 0x2000: just MRET so we can observe mepc on return.
        bus.write_u32(0x2000, 0x30200073).unwrap();

        cpu.pc = 0x0;
        let mut machine = Machine::new(cpu, bus);

        machine.step().unwrap(); // PC 0x0 -> 0x4, x10 = 1, mtime = 1
        machine.step().unwrap(); // PC 0x4 -> 0x8, x10 = 2, mtime = 2
                                 // This step executes 0x8 (x10 = 3, next_pc = 0xC). Then mtime -> 3
                                 // hits mtimecmp and the trap fires. mepc must be saved as 0xC,
                                 // not 0x8, so MRET doesn't re-execute the ADDI.
        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(10), 3, "third ADDI executed once");
        assert_eq!(machine.cpu.pc, 0x2000, "trapped into ISR");
        assert_eq!(
            machine.cpu.mepc, 0xC,
            "mepc must be the address of the next instruction, not the one we just finished"
        );

        // Clear MTIP so the next step doesn't re-trap.
        machine.cpu.mip &= !(1 << 7);
        machine.cpu.mtimecmp = u64::MAX;

        machine.step().unwrap(); // MRET: jump to mepc
        assert_eq!(machine.cpu.pc, 0xC, "MRET returned to mepc");
        // Not re-executing the ADDI at 0x8 is what we really care about:
        assert_eq!(
            machine.cpu.read_reg(10),
            3,
            "ADDI at 0x8 must not be re-executed (would read 4 if bug present)"
        );
    }

    #[test]
    fn test_riscv_external_irq_line_vectored_and_mret_restores_mie() {
        // ESP32-C3-style external interrupt: the bus drives a CPU interrupt
        // line (1..31) via `external_irq_lines()`; with vectored mtvec the core
        // traps to base + line*4, and MRET restores MIE from MPIE.
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        cpu.mtvec = 0x2000 | 1; // vectored
        cpu.mstatus = 1 << 3; // MIE
        cpu.mtimecmp = u64::MAX; // no CLINT timer interference

        bus.flash.data = vec![0; 0x3000];
        // NOP loop at 0x0 (ADDI x0,x0,0) and the per-line handlers are NOPs+MRET.
        bus.write_u32(0x0, 0x00000013).unwrap();
        bus.write_u32(0x4, 0x00000013).unwrap();
        // Line 5 vector (0x2000 + 5*4 = 0x2014): MRET.
        bus.write_u32(0x2014, 0x30200073).unwrap();
        // Assert external line 5 (esp32c3_irq_routing stays false, so the C3
        // aggregation leaves riscv_irq_lines untouched between ticks).
        bus.riscv_irq_lines = 1 << 5;

        cpu.pc = 0x0;
        let mut machine = Machine::new(cpu, bus);
        // First step executes the NOP at 0x0, then takes the pending line-5 trap.
        machine.step().unwrap();
        assert_eq!(
            machine.cpu.pc, 0x2014,
            "vectored trap must jump to mtvec base + line*4"
        );
        assert_eq!(
            machine.cpu.mcause,
            0x8000_0000 | 5,
            "mcause = interrupt|line"
        );
        assert_eq!(machine.cpu.mstatus & (1 << 3), 0, "MIE cleared on trap");
        assert_ne!(machine.cpu.mstatus & (1 << 7), 0, "MPIE holds prior MIE");
        // Drop the line so MRET doesn't immediately re-trap, then MRET.
        machine.bus.riscv_irq_lines = 0;
        machine.step().unwrap();
        assert_ne!(
            machine.cpu.mstatus & (1 << 3),
            0,
            "MRET restores MIE from MPIE"
        );
    }

    #[test]
    fn test_riscv_wfi_is_nop() {
        // WFI must decode and execute as a no-op (PC advances by 4); the idle
        // task's WFI spin relies on this.
        assert_eq!(
            crate::decoder::riscv::decode_rv32(0x1050_0073),
            crate::decoder::riscv::Instruction::Wfi
        );
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        bus.flash.data = vec![0; 0x100];
        bus.write_u32(0x0, 0x1050_0073).unwrap(); // WFI
        cpu.pc = 0x0;
        let mut machine = Machine::new(cpu, bus);
        machine.step().unwrap();
        assert_eq!(machine.cpu.pc, 0x4, "WFI advances PC like a NOP");
    }

    #[test]
    fn test_riscv_wfi_fast_forward_is_off_by_default() {
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        bus.flash.data = vec![0; 0x100];
        bus.write_u32(0x0, 0x1050_0073).unwrap(); // WFI
        bus.write_u32(0x4, 0xffdf_f06f).unwrap(); // JAL x0, -4
        cpu.pc = 0x0;
        cpu.mtimecmp = u64::MAX;

        let mut machine = Machine::new(cpu, bus);
        machine.bus.legacy_walk_disabled = true;
        machine.run(Some(10)).unwrap();

        assert_eq!(machine.total_cycles, 10);
        assert_eq!(machine.step_profile().cpu_instructions, 10);
    }

    #[test]
    fn test_riscv_wfi_fast_forward_skips_cpu_work_when_enabled() {
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        bus.flash.data = vec![0; 0x100];
        bus.write_u32(0x0, 0x1050_0073).unwrap(); // WFI
        bus.write_u32(0x4, 0xffdf_f06f).unwrap(); // JAL x0, -4
        cpu.pc = 0x0;
        cpu.mtimecmp = u64::MAX;

        let mut machine = Machine::new(cpu, bus);
        machine.config.idle_fast_forward_enabled = true;
        machine.bus.legacy_walk_disabled = true;
        machine.run(Some(10)).unwrap();

        assert_eq!(machine.total_cycles, 10);
        assert!(
            machine.idle_fast_forward_cycles_skipped > 0,
            "idle FF counter must rise when WFI skip fires"
        );
        assert!(
            machine.step_profile().cpu_instructions < 10,
            "fast-forwarded cycles should not retire CPU instructions"
        );
    }

    #[test]
    fn boxed_riscv_batch_preserves_idle_fast_forward_escape() {
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        bus.flash.data = vec![0; 0x100];
        bus.write_u32(0x0, 0x1050_0073).unwrap(); // WFI
        bus.write_u32(0x4, 0xffdf_f06f).unwrap(); // JAL x0, -4
        cpu.pc = 0x0;
        cpu.mtimecmp = u64::MAX;

        let mut machine = Machine::new(Box::new(cpu) as Box<dyn Cpu>, bus);
        machine.config.idle_fast_forward_enabled = true;
        machine.bus.legacy_walk_disabled = true;
        machine.run(Some(10)).unwrap();

        assert_eq!(machine.total_cycles, 10);
        assert!(
            machine.step_profile().cpu_instructions < 10,
            "boxed C3 CPU path should still leave the batch loop at WFI so Machine can fast-forward"
        );
    }

    #[test]
    fn test_riscv_wfi_fast_forward_wakes_on_systimer_event() {
        let mut bus = SystemBus::empty();
        let mut cpu = RiscV::new();
        bus.flash.data = vec![0; 0x3000];
        bus.write_u32(0x0, 0x1050_0073).unwrap(); // WFI
        bus.write_u32(0x4, 0xffdf_f06f).unwrap(); // JAL x0, -4
        bus.write_u32(0x2000 + 11 * 4, 0x3020_0073).unwrap(); // MRET at machine external IRQ vector

        bus.add_peripheral(
            "systimer",
            0x6002_3000,
            0x100,
            None,
            Box::new(
                crate::peripherals::esp32s3::systimer::Systimer::new_with_source(160_000_000, 11),
            ),
        );
        bus.write_u32(0x6002_3064, 1).unwrap(); // INT_ENA TARGET0
        bus.write_u32(0x6002_301C, 0).unwrap(); // TARGET0_HI
        bus.write_u32(0x6002_3020, 3).unwrap(); // TARGET0_LO: 3 SYSTIMER ticks
        bus.write_u32(0x6002_3050, 1).unwrap(); // COMP0_LOAD
        let conf = bus.read_u32(0x6002_3000).unwrap();
        bus.write_u32(0x6002_3000, conf | (1 << 24)).unwrap(); // TARGET0_WORK_EN

        cpu.pc = 0x0;
        cpu.mtvec = 0x2000 | 1; // vectored machine interrupts
        cpu.mie = 1 << 11; // MEIE
        cpu.mstatus = 1 << 3; // MIE
        cpu.mtimecmp = u64::MAX;

        let mut machine = Machine::new(cpu, bus);
        machine.config.idle_fast_forward_enabled = true;
        machine.run(Some(40)).unwrap();

        assert!(
            machine.step_profile().cpu_instructions < machine.total_cycles,
            "WFI should skip idle cycles until the SYSTIMER event; retired {} over {} cycles",
            machine.step_profile().cpu_instructions,
            machine.total_cycles
        );
        assert_ne!(
            machine.cpu.mcause & 0x8000_0000,
            0,
            "the scheduled SYSTIMER event should become an observable interrupt"
        );
    }

    #[test]
    fn test_riscv_rv32a_atomics() {
        // Smoke-test the RV32A word atomics. Build a small program in RAM and
        // run instructions by placing them in flash one at a time via step().
        //
        // Layout: put the memory cell we'll operate on at 0x2000_0010 in RAM.
        // Put the program at flash 0x0.
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        bus.flash.data = vec![0; 0x200];

        // Helper: encode an R-type (opcode=0x2F) atomic. All RV32A word
        // atomics share funct3 = 0b010.
        fn amo(funct5: u32, rs2: u32, rs1: u32, rd: u32) -> u32 {
            (funct5 << 27) | (rs2 << 20) | (rs1 << 15) | (0b010 << 12) | (rd << 7) | 0x2F
        }

        // Program layout in flash:
        // 0x00: LUI   x5, 0x20000       ; x5 = 0x20000000
        // 0x04: ADDI  x5, x5, 0x10      ; x5 = 0x20000010   (our atomic cell)
        // 0x08: ADDI  x6, x0, 7         ; x6 = 7
        // 0x0C: ADDI  x7, x0, 42        ; x7 = 42
        // 0x10: SW    x6, 0(x5)         ; mem[x5] = 7
        // 0x14: LR.W  x8, (x5)          ; x8 = 7 (reservation on x5)
        // 0x18: SC.W  x9, x7, (x5)      ; x9 = 0 (success), mem[x5] = 42
        // 0x1C: AMOADD.W x10, x7, (x5)  ; x10 = 42, mem[x5] = 42 + 42 = 84
        // 0x20: AMOMAX.W x11, x0, (x5)  ; x11 = 84, mem[x5] = max(84, 0) = 84
        // 0x24: SC.W  x12, x7, (x5)     ; x12 = 1 (failure — reservation cleared)
        //
        // LUI rd=5, imm[31:12]=0x20000 — rd = 0x20000000.
        let lui = 0x20000000u32 | (5 << 7) | 0x37;
        // ADDI x5, x5, 0x10 -> imm=0x10 rs1=5 funct3=0 rd=5 op=0x13
        let addi_x5 = (0x10 << 20) | (5 << 15) | (5 << 7) | 0x13;
        // ADDI x6, x0, 7
        let addi_x6_7 = (7 << 20) | (6 << 7) | 0x13;
        // ADDI x7, x0, 42
        let addi_x7_42 = (42 << 20) | (7 << 7) | 0x13;
        // SW x6, 0(x5) -> imm=0, funct3=0b010, rs1=5, rs2=6, op=0x23
        let sw = (6 << 20) | (5 << 15) | (0b010 << 12) | 0x23;
        let lr_w = amo(0x02, 0, 5, 8);
        let sc_w = amo(0x03, 7, 5, 9);
        let amoadd = amo(0x00, 7, 5, 10);
        let amomax = amo(0x14, 0, 5, 11);
        let sc_w_fail = amo(0x03, 7, 5, 12);

        let prog = [
            lui, addi_x5, addi_x6_7, addi_x7_42, sw, lr_w, sc_w, amoadd, amomax, sc_w_fail,
        ];
        for (i, w) in prog.iter().enumerate() {
            bus.write_u32((i as u64) * 4, *w).unwrap();
        }

        cpu.pc = 0;
        let mut machine = Machine::new(cpu, bus);

        for _ in 0..prog.len() {
            machine.step().unwrap();
        }

        // After the whole program:
        assert_eq!(machine.cpu.read_reg(5), 0x20000010, "x5 = cell address");
        assert_eq!(machine.cpu.read_reg(8), 7, "LR.W loaded initial value 7");
        assert_eq!(machine.cpu.read_reg(9), 0, "first SC.W succeeded");
        assert_eq!(
            machine.cpu.read_reg(10),
            42,
            "AMOADD.W returned pre-add value"
        );
        assert_eq!(
            machine.cpu.read_reg(11),
            84,
            "AMOMAX.W returned pre-op value (post-AMOADD result)"
        );
        assert_eq!(
            machine.cpu.read_reg(12),
            1,
            "second SC.W fails: AMOADD invalidated the reservation"
        );
        // Final memory state: 84.
        let final_val = machine.bus.read_u32(0x20000010).unwrap();
        assert_eq!(final_val, 84);
    }

    #[test]
    fn test_riscv_mul() {
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        // ADDI x1, x0, 10
        // ADDI x2, x0, 5
        // MUL x3, x1, x2 (x3 = 10 * 5 = 50)
        // MUL Opcode: 0x33, funct3: 0, funct7: 0x01, rs1: 1, rs2: 2, rd: 3
        // 0000001 00010 00001 000 00011 0110011 -> 0x022081B3
        bus.flash.data = vec![
            0x93, 0x00, 0xA0, 0x00, // ADDI x1, x0, 10
            0x13, 0x01, 0x50, 0x00, // ADDI x2, x0, 5
            0xB3, 0x81, 0x20, 0x02, // MUL x3, x1, x2
        ];

        cpu.pc = 0x0;
        let mut machine = Machine::new(cpu, bus);
        machine.step().unwrap();
        machine.step().unwrap();
        machine.step().unwrap();

        assert_eq!(machine.cpu.read_reg(3), 50);
        assert_eq!(machine.cpu.pc, 12);
    }

    #[test]
    fn test_riscv_compressed_addi() {
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        // C.ADDI x1, 5 (x1 = 0 + 5)
        // Op: 01, funct3: 000, rd: 1, imm: 5
        // 000 0 00001 00101 01 -> 0x0085 (Wait, C.ADDI imm is split)
        // inst[15:13]=000, inst[12]=imm[5]=0, inst[11:7]=rd=1, inst[6:2]=imm[4:0]=5
        // 000 0   00001   00101   01 -> 0x0085 (Wait, bitwise: 0000 0000 1001 0101 -> 0x0095?)
        // Let's use rvcodecjs: C.ADDI x1, 5 -> 0x0095
        bus.flash.data = vec![
            0x95, 0x00, // C.ADDI x1, 5
            0x13, 0x02, 0x50, 0x00, // ADDI x4, x0, 5 (for alignment check)
        ];

        cpu.pc = 0x0;
        let mut machine = Machine::new(cpu, bus);
        machine.step().unwrap();

        assert_eq!(machine.cpu.read_reg(1), 5);
        assert_eq!(machine.cpu.pc, 2); // PC should increment by 2

        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(4), 5);
        assert_eq!(machine.cpu.pc, 6);
    }

    #[test]
    fn test_riscv_compressed_lw_sw() {
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        // x8 is used for C.LW/SW (s0/fp).
        // 1. ADDI x8, x0, 0x20000000 (RAM start)
        // 2. ADDI x9, x0, 42
        // 3. C.SW x9, 4(x8)
        // 4. C.LW x10, 4(x8)

        // C.SW x9, 4(x8) -> 0xC044 (Little Endian: 44 C0)
        // C.LW x10, 4(x8) -> 0x4048 (Little Endian: 48 40)

        bus.flash.data = vec![
            0x37, 0x04, 0x00, 0x20, // LUI x8, 0x20000 (x8 = 0x20000000)
            0x93, 0x04, 0xA0, 0x02, // ADDI x9, x0, 42
            0x44, 0xC0, // C.SW x9, 4(x8)
            0x48, 0x40, // C.LW x10, 4(x8)
            0x00, 0x00, 0x00, 0x00, // Padding
        ];

        cpu.pc = 0x0;
        cpu.write_reg(2, 0x20001000); // Initialize SP just in case
        let mut machine = Machine::new(cpu, bus);
        machine.step().unwrap(); // LUI
        machine.step().unwrap(); // ADDI
        machine.step().unwrap(); // C.SW
        machine.step().unwrap(); // C.LW

        assert_eq!(machine.cpu.read_reg(10), 42);
        assert_eq!(machine.cpu.pc, 12); // 4 + 4 + 2 + 2 = 12
    }

    #[test]
    fn riscv_decode_cache_uses_opcode_tag_for_same_pc_changes() {
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        bus.flash.data = vec![
            0x93, 0x00, 0x10, 0x00, // ADDI x1, x0, 1
        ];

        cpu.pc = 0;
        let mut machine = Machine::new(cpu, bus);
        machine.config.decode_cache_enabled = true;
        let initial_pc = machine.cpu.pc;
        machine.step().unwrap();

        let cache_idx = ((initial_pc >> 1) & 0xFFF) as usize;
        let entry = machine.cpu.decode_cache[cache_idx].expect("first step caches decode");
        assert_eq!(entry.tag, 0);
        assert_eq!(entry.opcode, 0x0010_0093);
        assert_eq!(machine.cpu.read_reg(1), 1);

        machine.bus.flash.data = vec![
            0x93, 0x00, 0x20, 0x00, // ADDI x1, x0, 2
        ];
        machine.cpu.pc = 0;
        machine.step().unwrap();

        let entry = machine.cpu.decode_cache[cache_idx].expect("second step refreshes decode");
        assert_eq!(entry.opcode, 0x0020_0093);
        assert_eq!(machine.cpu.read_reg(1), 2);
    }
}
