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

    /// Side-effect-free instruction-fetch window over flash-XIP and linear
    /// code memories (`extra_mem` IRAM/ROM). Avoids per-instruction
    /// `find_peripheral_index` + dyn dispatch — the post-XIP-opt profile
    /// hotspot on C3 OLED (app text is XIP; FreeRTOS / ISR text is IRAM, ~35%
    /// of the busy path). Only filled from side-effect-free code paths
    /// (FlashXIP / extra_mem); MMIO never enters the window. Guest stores that
    /// overlap the live window invalidate it (self-modifying IRAM stays
    /// byte-identical to unwindowed `bus.read_u32`). Plain `ram` is deliberately
    /// not windowed: unit tests host-patch RAM under the PC between steps and
    /// expect the next fetch to see the new bytes without a CPU store.
    fetch_base: u32,
    fetch_len: u16,
    fetch_bytes: [u8; FETCH_WINDOW_BYTES],

    /// Aligned window base that [`RiscV::refill_fetch_window`] last failed to
    /// fill, so the same failure is not re-derived on the next instruction.
    ///
    /// The window only fills from a `FlashXipPeripheral` or from `extra_mem`.
    /// A chip whose code sits in a plain `flash` region — which is what
    /// `configs/chips/esp32c3.yaml` declares at 0x4200_0000 — matches neither,
    /// so `fetch_len` stays 0 and the refill is attempted again on the very
    /// next instruction: a `find_peripheral_index` call plus a downcast plus a
    /// walk of every `extra_mem` window, measured at ~20 Ir per instruction of
    /// routing alone (docs/performance/2026-09-17-bus-scheduler-pass.md).
    ///
    /// Remembering the failing base costs one compare and cannot change what
    /// is fetched: the skipped call only reads side-effect-free memory, and
    /// the fetch still falls through to `bus.read_u32` exactly as it did
    /// before. It is cleared the moment a refill succeeds, and the base is
    /// window-aligned, so crossing into a new 256-byte line always re-asks.
    fetch_refill_failed_base: Option<u32>,

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

/// `addi x0, x0, 0` — retired in place of a fetch a memory-protection unit
/// blocked, so the raised interrupt is taken at the next instruction boundary
/// instead of the core executing bytes the hardware never delivered.
const NOP_OPCODE: u32 = 0x0000_0013;

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
            fetch_refill_failed_base: None,
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
        // Window miss: this is the one place a fetch can enter a region the
        // window has not already vetted, so the memory-protection check goes
        // here. Splits are 512-byte aligned and the window is 256-byte aligned
        // and at most 256 bytes long, so one check per refill covers every
        // fetch the window then serves.
        match bus.check_fetch_permission(pc as u64) {
            crate::FetchPermission::Allowed => {}
            crate::FetchPermission::DeniedFaultRaised => {
                // Silicon blocks the fetch and raises the violation through the
                // interrupt matrix; the trap lands at the next instruction
                // boundary. Retire a NOP so the interrupt check at the tail of
                // `step` takes it — reproducing the PC skid that makes real IDF
                // read the faulting address out of the PMS status registers.
                self.fetch_len = 0;
                return Ok(NOP_OPCODE);
            }
            crate::FetchPermission::DeniedUndeliverable => {
                // Nothing routes the violation to the firmware, so executing
                // whatever bytes are there would be a silent lie. Fail loud.
                self.fetch_len = 0;
                return Err(crate::SimulationError::MemoryViolation(pc as u64));
            }
        }
        // Skip a refill that just failed for this same window line — see
        // `fetch_refill_failed_base`. Byte-identical: the call only reads
        // side-effect-free memory, and the fall-through below is unchanged.
        let window_base = pc & !((FETCH_WINDOW_BYTES as u32) - 1);
        // The memo is only ever set by a refill that left the window empty,
        // and any later successful refill clears it, so a remembered failure
        // cannot let a stale window serve bytes from a different line.
        debug_assert!(
            self.fetch_refill_failed_base.is_none() || self.fetch_len == 0,
            "remembered refill failure with a live fetch window"
        );
        if self.fetch_refill_failed_base != Some(window_base) {
            self.refill_fetch_window(bus, pc);
            self.fetch_refill_failed_base = if self.fetch_len == 0 {
                Some(window_base)
            } else {
                None
            };
        }
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

    /// Drop the fetch window if a guest store of `size` bytes at `addr`
    /// overlaps it. Keeps IRAM self-modifying sequences byte-identical to the
    /// unwindowed bus path. No-op when the window is empty or the store is
    /// outside it (the common case: stack/data stores while fetching code).
    #[inline]
    fn invalidate_fetch_if_store_overlaps(&mut self, addr: u32, size: u32) {
        if self.fetch_len == 0 || size == 0 {
            return;
        }
        let win_lo = self.fetch_base;
        let win_hi = win_lo.wrapping_add(self.fetch_len as u32);
        let store_hi = addr.wrapping_add(size);
        // Half-open ranges [addr, store_hi) and [win_lo, win_hi).
        if addr < win_hi && store_hi > win_lo {
            self.fetch_len = 0;
        }
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

        // 1. Flash-XIP (ESP32-C3 app at 0x4200_0000). Immutable under the current
        // SPI model (program/erase only touch status regs), so a window is
        // byte-identical to repeated `bus.read_u32` there.
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
                    return;
                }
            }
        }

        // 2. extra_mem: ESP32-C3 IRAM (0x4037_0000) + mask ROM (0x4000_0000).
        // IRAM holds FreeRTOS/ISR text (~35% of C3 OLED busy instructions).
        // Side-effect free; guest stores that overlap the window invalidate it
        // via [`invalidate_fetch_if_store_overlaps`]. Host-side `bus.write_*`
        // patches of IRAM mid-run are not observed until the next refill —
        // production firmware does not host-patch IRAM under a live PC.
        for mem in &sb.extra_mem {
            let Some(offset) = (base as u64).checked_sub(mem.base_addr) else {
                continue;
            };
            let off = offset as usize;
            if off >= mem.data.len() {
                continue;
            }
            let max = (mem.data.len() - off).min(FETCH_WINDOW_BYTES);
            if max < 4 {
                continue;
            }
            let mut buf = [0u8; FETCH_WINDOW_BYTES];
            buf[..max].copy_from_slice(&mem.data[off..off + max]);
            self.fetch_base = base;
            self.fetch_bytes = buf;
            self.fetch_len = max as u16;
            return;
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
            // The ESP32-C3 ROM clears ustatus during reset. This emulator runs
            // only in machine mode, so no user-status bits can become active.
            0x000 if self.core_profile == RiscVCoreProfile::Esp32C3 => 0,
            // Undocumented vendor ROM-init CSRs observed in the ESP32-C3 mask
            // ROM. The observed writes discard the old values; any silicon
            // side effects are not modeled.
            0x800 | 0x801 if self.core_profile == RiscVCoreProfile::Esp32C3 => 0,
            // This emulator exposes one hart, whose architectural ID is zero.
            0xF14 => 0,
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
            // See read_csr: accept the ROM's reset-time clear as a no-op.
            0x000 if self.core_profile == RiscVCoreProfile::Esp32C3 => {}
            // See read_csr: accept the ROM-init writes and discard their values.
            0x800 | 0x801 if self.core_profile == RiscVCoreProfile::Esp32C3 => {}
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
            self.pc = base.wrapping_add(irq.wrapping_mul(4));
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
            // Mirror the interpreter's Gap #1 deadline clamp (see `step_batch`).
            #[cfg(feature = "event-scheduler")]
            if config.peripheral_tick_interval > 1 {
                if let Some(dl) = bus.earliest_pending_deadline() {
                    let max_ret = dl.saturating_sub(batch_start);
                    if (retired as u64) >= max_ret {
                        return Ok(retired);
                    }
                }
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
            Instruction::Lui { .. }
            | Instruction::Auipc { .. }
            | Instruction::Addi { .. }
            | Instruction::Slti { .. }
            | Instruction::Sltiu { .. }
            | Instruction::Xori { .. }
            | Instruction::Ori { .. }
            | Instruction::Andi { .. }
            | Instruction::Slli { .. }
            | Instruction::Srli { .. }
            | Instruction::Srai { .. }
            | Instruction::Add { .. }
            | Instruction::Sub { .. }
            | Instruction::Sll { .. }
            | Instruction::Slt { .. }
            | Instruction::Sltu { .. }
            | Instruction::Xor { .. }
            | Instruction::Srl { .. }
            | Instruction::Sra { .. }
            | Instruction::Or { .. }
            | Instruction::And { .. }
            | Instruction::Fence
            | Instruction::CAddi { .. }
            | Instruction::CLi { .. }
            | Instruction::CMv { .. }
            | Instruction::CAddi16sp { .. }
            | Instruction::CAddi4spn { .. }
            | Instruction::CSli { .. } => {
                self.exec_alu(instruction)?;
            }
            Instruction::Jal { .. }
            | Instruction::Jalr { .. }
            | Instruction::Beq { .. }
            | Instruction::Bne { .. }
            | Instruction::Blt { .. }
            | Instruction::Bge { .. }
            | Instruction::Bltu { .. }
            | Instruction::Bgeu { .. }
            | Instruction::CJr { .. }
            | Instruction::CJalr { .. }
            | Instruction::CJ { .. }
            | Instruction::CBeqz { .. }
            | Instruction::CBnez { .. } => {
                self.exec_branch(instruction, inst_len, &mut next_pc)?;
            }
            Instruction::Lb { .. }
            | Instruction::Lh { .. }
            | Instruction::Lw { .. }
            | Instruction::Lbu { .. }
            | Instruction::Lhu { .. }
            | Instruction::Sb { .. }
            | Instruction::Sh { .. }
            | Instruction::Sw { .. }
            | Instruction::CLw { .. }
            | Instruction::CSw { .. }
            | Instruction::CLwsp { .. }
            | Instruction::CSwsp { .. } => {
                self.exec_load_store(bus, instruction)?;
            }
            Instruction::Mul { .. }
            | Instruction::Mulh { .. }
            | Instruction::Mulhsu { .. }
            | Instruction::Mulhu { .. }
            | Instruction::Div { .. }
            | Instruction::Divu { .. }
            | Instruction::Rem { .. }
            | Instruction::Remu { .. } => {
                self.exec_muldiv(instruction)?;
            }
            Instruction::LrW { .. }
            | Instruction::ScW { .. }
            | Instruction::AmoSwapW { .. }
            | Instruction::AmoAddW { .. }
            | Instruction::AmoXorW { .. }
            | Instruction::AmoOrW { .. }
            | Instruction::AmoAndW { .. }
            | Instruction::AmoMinW { .. }
            | Instruction::AmoMaxW { .. }
            | Instruction::AmoMinuW { .. }
            | Instruction::AmoMaxuW { .. } => {
                self.exec_atomic(bus, instruction)?;
            }
            Instruction::Wfi
            | Instruction::Ecall
            | Instruction::Ebreak
            | Instruction::Mret
            | Instruction::Csrrw { .. }
            | Instruction::Csrrs { .. }
            | Instruction::Csrrc { .. }
            | Instruction::Csrrwi { .. }
            | Instruction::Csrrsi { .. }
            | Instruction::Csrrci { .. } => {
                if self.exec_system(instruction, opcode)? {
                    return Ok(());
                }
            }
            Instruction::Unknown(_) => {
                self.exec_misc(instruction)?;
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
            let mut registers = [0u32; 34];
            registers[..32].copy_from_slice(&self.x);
            // Standard trailer (see `SimulationObserver`): SP then PC. On
            // RISC-V the stack pointer is x2 by ABI convention.
            registers[32] = self.x[2];
            registers[33] = self.pc;

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
        // Gap #1: mid-batch arming. Clamp remaining work to the earliest pending
        // absolute deadline instead of ending on the first arm — far-future
        // timers (FreeRTOS tick, etc.) used to force ~60-insn batches and tank
        // host MIPS. We still never retire past the deadline without a drain
        // (same delivery cycle as the immediate-end policy).
        #[cfg(feature = "event-scheduler")]
        let mut limit = max_count;
        #[cfg(not(feature = "event-scheduler"))]
        let limit = max_count;
        let mut i = 0u32;
        while i < limit {
            if let Some(tap) = &tap {
                tap.bump_clock();
            }
            #[cfg(feature = "event-scheduler")]
            if exact_clock {
                bus.publish_cycle(batch_start + i as u64);
            }
            self.step(bus, observers, config)?;
            i += 1;
            if config.idle_fast_forward_enabled && self.idle_fast_forward_budget(bus).is_some() {
                return Ok(i);
            }
            #[cfg(feature = "event-scheduler")]
            if exact_clock {
                if let Some(dl) = bus.earliest_pending_deadline() {
                    // After `i` instructions, total_cycles will be batch_start+i.
                    // Do not advance total_cycles past `dl` before drain enqueues.
                    let max_ret = dl.saturating_sub(batch_start);
                    if (i as u64) >= max_ret {
                        return Ok(i);
                    }
                    if max_ret < u32::MAX as u64 {
                        limit = limit.min(max_ret as u32);
                    }
                }
            }
        }
        Ok(i)
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

    fn runtime_snapshot(&self) -> Option<(crate::runtime_snapshot::CpuKind, Vec<u8>)> {
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
        Some((crate::runtime_snapshot::CpuKind::RiscV, bytes))
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

#[path = "riscv/exec/mod.rs"]
mod exec;

#[cfg(test)]
#[path = "riscv_tests.rs"]
mod tests;
