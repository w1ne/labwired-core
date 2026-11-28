// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use super::qingke_pfic::{PficFrame, QingKePfic, HPE_REGS, HPE_REGS_V4, INTSYSCR_HWSTKEN};
use super::riscv64_plic::{Plic, PlicParams};
use crate::decoder::riscv::{decode_qingke_xw, decode_rv32, Instruction};
use crate::{Bus, Cpu, SimResult, SimulationObserver};
use std::sync::Arc;

#[path = "riscv_trigger.rs"]
pub mod trigger;

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
    /// RV32E profile: the integer register file contains x0..x15 only.
    Rv32e,
    /// WCH QingKe V4 (CH32V20x/V30x, `core: qingke-v4*`): the standard RV32
    /// machine plus the vendor core configuration CSR CORECFGR (0xBC0) that
    /// WCH's startup and ch32fun write before `main` (stored; it tunes the
    /// pipeline and has no architectural effect here), and a PFIC whose
    /// hardware prologue saves the 16-register V4 set.
    QingkeV4,
}

impl RiscVCoreProfile {
    /// Select the profile a chip descriptor runs.
    ///
    /// The engine models two profiles: the standard RV32 machine and the
    /// ESP32-C3's vendor CSR set. An `esp32*` chip keeps the vendor profile —
    /// the C3 is the part whose ROM and performance-counter block the engine
    /// models, and the C6 shares that register surface today. Any other chip
    /// that declares a generic non-E `rv32*` core (e.g. `core: "rv32im"` on a
    /// VexRiscv SoC) runs the standard profile. A chip with no `core:` keeps
    /// the historical ESP32-C3 default, so descriptors written before this
    /// selection existed behave exactly as they did.
    pub fn for_chip(chip: &labwired_config::ChipDescriptor) -> Self {
        let core = chip
            .core
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if matches!(core.as_str(), "rv32e" | "rv32ec" | "rv32emc" | "rv32em") {
            return Self::Rv32e;
        }
        if core.starts_with("qingke-v4") {
            return Self::QingkeV4;
        }
        if chip.name.to_ascii_lowercase().contains("esp32") {
            return Self::Esp32C3;
        }
        if core.starts_with("rv32") {
            Self::StandardRv32
        } else {
            Self::Esp32C3
        }
    }
}

impl RiscVCoreProfile {
    /// The registers a QingKe PFIC's hardware prologue saves on this core.
    fn hpe_regs(self) -> &'static [usize] {
        if self == Self::QingkeV4 {
            &HPE_REGS_V4
        } else {
            &HPE_REGS
        }
    }
}

const CLINT_SIZE: u32 = 0x1_0000;
const CLINT_MTIMECMP: u32 = 0x4000;
const CLINT_MTIME: u32 = 0xBFF8;

/// Imported Renode systems built on `platforms/cpus/miv.repl` (Microchip
/// Mi-V RV32), which share its CLINT and PLIC.
fn is_miv(name: &str) -> bool {
    matches!(
        name,
        "miv" | "miv_rv32" | "miv-board" | "miv-board-additional-uarts"
    )
}

/// Memory-mapped core-local interruptor of a single-hart RV32 chip, in the
/// SiFive `clint0` layout that RISC-V `riscv,machine-timer` drivers use:
/// `msip` at +0x0, `mtimecmp` at +0x4000, `mtime` at +0xBFF8. `msip` drives
/// `mip.MSIP`, `mtime >= mtimecmp` drives `mip.MTIP`. The window is decoded by
/// the core, as on the RV64 core, so it never reaches the bus.
///
/// `mtime` counts the chip's timer reference, not CPU clocks: it is derived
/// from the core's retired-cycle count as `cycles * mtime_hz / cpu_hz`, so a
/// reference that does not divide the CPU clock (FE310: 32 768 Hz from
/// 62 MHz) keeps its exact long-run rate. A firmware write to `mtime` moves
/// `mtime_offset`.
///
/// Not modelled: AMO and LR/SC accesses to the window (they reach the bus),
/// other harts' registers (read zero, writes dropped), and `msip` /
/// `mtime_offset` in CPU snapshots (`mtimecmp` is saved).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rv32Clint {
    pub base: u32,
    pub mtime_hz: u64,
    pub cpu_hz: u64,
    pub msip: bool,
    pub mtime_offset: u64,
}

impl Rv32Clint {
    pub fn new(base: u32, mtime_hz: u64, cpu_hz: u64) -> Self {
        Self {
            base,
            mtime_hz: mtime_hz.max(1),
            cpu_hz: cpu_hz.max(1),
            msip: false,
            mtime_offset: 0,
        }
    }

    /// The CLINT a chip descriptor's core complex carries, if any.
    ///
    /// Keyed by chip name, as [`crate::cpu::riscv64::Rv64Params`] is: the
    /// block belongs to the core complex, not to a board.
    pub fn for_chip(chip: &labwired_config::ChipDescriptor) -> Option<Self> {
        let name = chip.name.to_ascii_lowercase();
        if name == "sifive-fe310" && chip.cpu_hz != 0 {
            // FE310-G002: CLINT at 0x0200_0000 (Zephyr `riscv32-fe310.dtsi`
            // `clint@2000000`, Renode `sifive-fe310.repl`). `mtime` runs from
            // the 32 768 Hz lfclk (Zephyr `soc/sifive/sifive_freedom/fe300`
            // Kconfig.defconfig). Renode clocks it at 62 MHz instead.
            return Some(Self::new(0x0200_0000, 32_768, chip.cpu_hz));
        }
        if is_miv(&name) && chip.cpu_hz != 0 {
            // Mi-V: CoreLevelInterruptor at 0x4400_0000, clocked at the
            // 66 MHz CPU clock (Renode `platforms/cpus/miv.repl`
            // `clint ... frequency: 66000000`, `[0, 1] -> cpu@[3, 7]`).
            return Some(Self::new(0x4400_0000, chip.cpu_hz, chip.cpu_hz));
        }
        None
    }

    /// The PLIC a chip descriptor's core complex carries, if any. Keyed by
    /// chip name, as [`Self::for_chip`] is. The bus forwards a peripheral's
    /// raw `irq:` number, which on these chips is its PLIC source ID.
    pub fn plic_for_chip(chip: &labwired_config::ChipDescriptor) -> Option<PlicParams> {
        let name = chip.name.to_ascii_lowercase();
        if name == "sifive-fe310" {
            // FE310-G002: PLIC at 0x0C00_0000 with sources 1..52, 7 priority
            // levels (3-bit fields) and one context, hart 0 M-mode (FE310-G002
            // Manual v1p1 Ch. 8; Zephyr `riscv32-fe310.dtsi`
            // `interrupt-controller@c000000` `riscv,ndev = <52>`; Renode
            // `sifive-fe310.repl` `numberOfSources: 52`, `numberOfContexts: 1`,
            // uart0 `IRQ -> plic@3`).
            return Some(PlicParams {
                base: 0x0C00_0000,
                num_sources: 53, // Plic stores the one-past-last ID.
                num_contexts: 1,
                priority_bits: 3,
                context: 0,
            });
        }
        if is_miv(&name) {
            // Mi-V: PLIC at 0x4000_0000 with sources 1..31, no priority
            // levels and one context (Renode `platforms/cpus/miv.repl`
            // `numberOfSources: 31`, `numberOfContexts: 1`,
            // `prioritiesEnabled: false`, `0 -> cpu@11`; timer0 and timer1
            // drive sources 30 and 31).
            return Some(PlicParams {
                base: 0x4000_0000,
                num_sources: 32, // Plic stores the one-past-last ID.
                num_contexts: 1,
                priority_bits: 0,
                context: 0,
            });
        }
        None
    }

    fn mtime(&self, cycles: u64) -> u64 {
        let ticks = u128::from(cycles) * u128::from(self.mtime_hz) / u128::from(self.cpu_hz);
        (ticks as u64).wrapping_add(self.mtime_offset)
    }

    /// First retired-cycle count at which `mtime >= cmp`.
    fn first_cycle_at(&self, cmp: u64) -> u64 {
        let Some(ticks) = cmp.checked_sub(self.mtime_offset) else {
            return 0;
        };
        let cycles =
            (u128::from(ticks) * u128::from(self.cpu_hz)).div_ceil(u128::from(self.mtime_hz));
        u64::try_from(cycles).unwrap_or(u64::MAX)
    }
}

/// Architectural integer-register operands used by an instruction. The
/// implicit stack-pointer operands in the compressed formats are x2, which is
/// legal in RV32E; compressed prime-register formats are already decoded to
/// x8..x15 by `decode_rv32`.
fn rv32e_reg_operands(instruction: &Instruction) -> [Option<u8>; 3] {
    use Instruction::*;
    match *instruction {
        Lui { rd, .. } | Auipc { rd, .. } | Jal { rd, .. } => [Some(rd), None, None],
        Jalr { rd, rs1, .. } => [Some(rd), Some(rs1), None],
        Beq { rs1, rs2, .. }
        | Bne { rs1, rs2, .. }
        | Blt { rs1, rs2, .. }
        | Bge { rs1, rs2, .. }
        | Bltu { rs1, rs2, .. }
        | Bgeu { rs1, rs2, .. }
        | Sb { rs1, rs2, .. }
        | Sh { rs1, rs2, .. }
        | Sw { rs1, rs2, .. } => [Some(rs1), Some(rs2), None],
        Lb { rd, rs1, .. }
        | Lh { rd, rs1, .. }
        | Lw { rd, rs1, .. }
        | Lbu { rd, rs1, .. }
        | Lhu { rd, rs1, .. }
        | Addi { rd, rs1, .. }
        | Slti { rd, rs1, .. }
        | Sltiu { rd, rs1, .. }
        | Xori { rd, rs1, .. }
        | Ori { rd, rs1, .. }
        | Andi { rd, rs1, .. }
        | Slli { rd, rs1, .. }
        | Srli { rd, rs1, .. }
        | Srai { rd, rs1, .. }
        | Csrrw { rd, rs1, .. }
        | Csrrs { rd, rs1, .. }
        | Csrrc { rd, rs1, .. }
        | LrW { rd, rs1 } => [Some(rd), Some(rs1), None],
        Add { rd, rs1, rs2 }
        | Sub { rd, rs1, rs2 }
        | Sll { rd, rs1, rs2 }
        | Slt { rd, rs1, rs2 }
        | Sltu { rd, rs1, rs2 }
        | Xor { rd, rs1, rs2 }
        | Srl { rd, rs1, rs2 }
        | Sra { rd, rs1, rs2 }
        | Or { rd, rs1, rs2 }
        | And { rd, rs1, rs2 }
        | Mul { rd, rs1, rs2 }
        | Mulh { rd, rs1, rs2 }
        | Mulhsu { rd, rs1, rs2 }
        | Mulhu { rd, rs1, rs2 }
        | Div { rd, rs1, rs2 }
        | Divu { rd, rs1, rs2 }
        | Rem { rd, rs1, rs2 }
        | Remu { rd, rs1, rs2 }
        | ScW { rd, rs1, rs2 }
        | AmoSwapW { rd, rs1, rs2 }
        | AmoAddW { rd, rs1, rs2 }
        | AmoXorW { rd, rs1, rs2 }
        | AmoOrW { rd, rs1, rs2 }
        | AmoAndW { rd, rs1, rs2 }
        | AmoMinW { rd, rs1, rs2 }
        | AmoMaxW { rd, rs1, rs2 }
        | AmoMinuW { rd, rs1, rs2 }
        | AmoMaxuW { rd, rs1, rs2 } => [Some(rd), Some(rs1), Some(rs2)],
        Csrrwi { rd, .. } | Csrrsi { rd, .. } | Csrrci { rd, .. } => [Some(rd), None, None],
        CAddi { rd, .. } | CLi { rd, .. } | CSli { rd, .. } => [Some(rd), None, None],
        CLw { rd, rs1, .. } => [Some(rd), Some(rs1), None],
        CSw { rs2, rs1, .. } => [Some(rs1), Some(rs2), None],
        CJr { rs1 } | CJalr { rs1 } | CBeqz { rs1, .. } | CBnez { rs1, .. } => {
            [Some(rs1), None, None]
        }
        CMv { rd, rs2 } => [Some(rd), Some(rs2), None],
        CAddi16sp { .. } => [Some(2), None, None],
        CAddi4spn { rd, .. } | CLwsp { rd, .. } => [Some(rd), Some(2), None],
        CSwsp { rs2, .. } => [Some(2), Some(rs2), None],
        Fence | Ecall | Ebreak | Mret | Wfi | CJ { .. } | Unknown(_) => [None, None, None],
    }
}

pub(crate) fn rv32e_uses_high_register(instruction: &Instruction) -> bool {
    rv32e_reg_operands(instruction)
        .into_iter()
        .flatten()
        .any(|register| register >= 16)
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

    // CLINT-like internal state (minimal). `mtime` is the retired-cycle
    // count; with a memory-mapped [`Rv32Clint`] `mtimecmp` compares against
    // the CLINT's own `mtime` instead.
    pub mtime: u64,
    pub mtimecmp: u64,
    /// Memory-mapped CLINT of a chip that declares one ([`Rv32Clint::for_chip`]).
    pub clint: Option<Rv32Clint>,
    /// Memory-mapped PLIC of a chip that declares one
    /// ([`Rv32Clint::plic_for_chip`]). It drives `mip.MEIP`; peripheral IRQs
    /// pend its sources through [`Cpu::set_exception_pending`].
    pub plic: Option<Plic>,
    /// WCH QingKe V2 PFIC of a CH32V00x part ([`QingKePfic::for_chip`]).
    /// Peripheral IRQs pend its lines through [`Cpu::set_exception_pending`];
    /// the core takes them itself, through the mtvec vector table.
    pub pfic: Option<QingKePfic>,
    /// QingKe INTSYSCR (CSR 0x804). Stored only while a PFIC is attached;
    /// `HWSTKEN` enables the hardware prologue/epilogue (HPE).
    pub intsyscr: u32,
    /// QingKe V4 CORECFGR (CSR 0xBC0), stored only.
    pub corecfgr: u32,

    /// Active LR/SC reservation address. `None` means no outstanding
    /// reservation; the next SC.W to any address will fail. On a single
    /// hart any intervening store (including any AMO*) invalidates the
    /// reservation per RISC-V ISA §8.2.
    pub reservation: Option<u32>,

    /// Debug trigger module (`tselect`/`tdata1`/`tdata2`/`tinfo`/`tcontrol`):
    /// hardware breakpoints and watchpoints. ESP32-C3 profile only.
    pub triggers: trigger::TriggerModule,

    waiting_for_interrupt: bool,
    decode_cache: Box<[Option<RiscVDecodeCacheEntry>; 4096]>,
    block_cache: super::riscv_block_cache::Rv32BlockCache,

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
            clint: None,
            plic: None,
            pfic: None,
            intsyscr: 0,
            corecfgr: 0,
            reservation: None,
            triggers: trigger::TriggerModule::default(),
            waiting_for_interrupt: false,
            decode_cache: Box::new([None; 4096]),
            block_cache: super::riscv_block_cache::Rv32BlockCache::new(),
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
        // `off + 4 <= fetch_len` already proves that `off` starts inside the
        // window. Keep the arithmetic widened so a wrapped `u32` offset can
        // never turn a miss into a hit.
        if (off as u64) + 4 <= self.fetch_len as u64 {
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
        if (off as u64) + 4 <= self.fetch_len as u64 {
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
        self.block_cache.invalidate_if_store_overlaps(addr, size);
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

        // 2. Plain flash. Declarative ESP32-C3 boards map IROM directly through
        // `SystemBus::flash` rather than a FlashXipPeripheral. It is just as
        // safe to window: guest stores already invalidate an overlapping live
        // window, while a peripheral mapped over the same address must retain
        // routing precedence and is therefore deliberately refused here.
        // Keep the synthetic base-zero flash used by debugger/host-patching
        // tests on the uncached path. Real mapped flash has a non-zero base;
        // after load it is immutable except through guest bus stores, which
        // are covered by `invalidate_fetch_if_store_overlaps`.
        if sb.flash.base_addr != 0 && sb.find_peripheral_index(base as u64).is_none() {
            if let Some(offset) = (base as u64).checked_sub(sb.flash.base_addr) {
                let off = offset as usize;
                if off < sb.flash.data.len() {
                    let max = (sb.flash.data.len() - off).min(FETCH_WINDOW_BYTES);
                    if max >= 4 {
                        let mut buf = [0u8; FETCH_WINDOW_BYTES];
                        buf[..max].copy_from_slice(&sb.flash.data[off..off + max]);
                        self.fetch_base = base;
                        self.fetch_bytes = buf;
                        self.fetch_len = max as u16;
                        return;
                    }
                }
            }
        }

        // 3. extra_mem: ESP32-C3 IRAM (0x4037_0000) + mask ROM (0x4000_0000).
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

    /// Map a CLINT at `clint.base`. `mtimecmp` resets to all-ones, which keeps
    /// MTIP low until firmware arms it, as SiFive-derived drivers assume.
    pub fn attach_clint(&mut self, clint: Rv32Clint) {
        self.clint = Some(clint);
        self.mtimecmp = u64::MAX;
        self.refresh_timer_lines();
    }

    /// Map a PLIC. Every source resets disabled at priority 0, so MEIP stays
    /// low until firmware enables a source.
    pub fn attach_plic(&mut self, params: PlicParams) {
        self.plic = Some(Plic::new(params));
        self.refresh_meip();
    }

    /// Map a QingKe PFIC at 0xE000E000 (V2, or V4 on a `QingkeV4` core).
    /// Every line resets disabled.
    pub fn attach_pfic(&mut self) {
        self.pfic = Some(QingKePfic::new());
    }

    /// Take PFIC interrupt `irq`, with `mepc = next_pc`. QingKe mtvec bit 0
    /// selects the vector table; bit 1 makes each entry an absolute handler
    /// address (vendor startup: `_start | 3`) instead of a jump instruction.
    fn enter_pfic_interrupt(&mut self, bus: &mut dyn Bus, irq: u32, next_pc: u32) -> SimResult<()> {
        if let Some(pfic) = self.pfic.as_mut() {
            pfic.take(irq);
        }
        self.handle_trap(0x8000_0000 | irq, next_pc);
        if self.mtvec & 1 != 0 {
            let slot = (self.mtvec & !3).wrapping_add(irq.wrapping_mul(4));
            self.pc = if self.mtvec & 2 != 0 {
                bus.read_u32(u64::from(slot))? & !1
            } else {
                slot
            };
        }
        Ok(())
    }

    /// True when the attached PFIC has an interrupt the core would take.
    #[inline]
    fn pfic_ready(&self) -> bool {
        self.pfic.as_ref().is_some_and(QingKePfic::has_ready)
    }

    /// Drive `mip.MEIP` from the PLIC's notification to this hart's context.
    fn refresh_meip(&mut self) {
        if let Some(plic) = &self.plic {
            if plic.meip() {
                self.mip |= 1 << 11;
            } else {
                self.mip &= !(1 << 11);
            }
        }
    }

    /// True when the core decodes a memory-mapped window itself (CLINT or
    /// PLIC), so bus-only fast paths would miss its accesses.
    #[inline]
    fn decodes_core_mmio(&self) -> bool {
        self.clint.is_some() || self.plic.is_some() || self.pfic.is_some()
    }

    #[inline]
    fn plic_offset(&self, addr: u32) -> Option<u64> {
        self.plic.as_ref()?.offset(u64::from(addr))
    }

    /// PLIC load: its registers are 32-bit, so a narrower load returns the
    /// addressed lanes of the containing word. A claim read is a side effect.
    fn plic_read(&mut self, off: u64, size: u32) -> u32 {
        let Some(plic) = self.plic.as_mut() else {
            return 0;
        };
        let word = plic.read_u32(off & !3);
        self.refresh_meip();
        let v = word >> ((off & 3) * 8);
        if size >= 4 {
            v
        } else {
            v & ((1u32 << (size * 8)) - 1)
        }
    }

    /// PLIC store. Narrower stores write the zero-extended value to the
    /// containing word, as SiFive's 32-bit-only register file does.
    fn plic_write(&mut self, off: u64, size: u32, value: u32) {
        let Some(plic) = self.plic.as_mut() else {
            return;
        };
        let v = if size >= 4 {
            value
        } else {
            (value & ((1u32 << (size * 8)) - 1)) << ((off & 3) * 8)
        };
        plic.write_u32(off & !3, v);
        self.refresh_meip();
    }

    fn update_mtime_after_elapsed_cycles(&mut self, cycles: u64) {
        self.mtime = self.mtime.wrapping_add(cycles);
        self.refresh_timer_lines();
    }

    fn refresh_timer_lines(&mut self) {
        let mtip = match &self.clint {
            None => self.mtime >= self.mtimecmp,
            Some(c) => {
                if c.msip {
                    self.mip |= 1 << 3; // MSIP
                } else {
                    self.mip &= !(1 << 3);
                }
                c.mtime(self.mtime) >= self.mtimecmp
            }
        };
        if mtip {
            self.mip |= 1 << 7; // MTIP
        } else {
            self.mip &= !(1 << 7);
        }
    }

    /// Retired cycles until MTIP rises: `None` when it is already pending,
    /// `Some(u64::MAX)` when `mtimecmp` is disarmed.
    fn cycles_until_mtip(&self) -> Option<u64> {
        if self.mtimecmp == u64::MAX {
            return Some(u64::MAX);
        }
        let target = match &self.clint {
            None => self.mtimecmp,
            Some(c) => c.first_cycle_at(self.mtimecmp),
        };
        target.checked_sub(self.mtime).filter(|n| *n != 0)
    }

    #[inline]
    fn clint_offset(&self, addr: u32) -> Option<u32> {
        let c = self.clint.as_ref()?;
        let off = addr.wrapping_sub(c.base);
        (off < CLINT_SIZE).then_some(off)
    }

    /// The 64-bit CLINT register containing `off` and its offset.
    fn clint_reg(&self, off: u32) -> Option<(u32, u64)> {
        let c = self.clint.as_ref()?;
        if off < 4 {
            Some((0, u64::from(c.msip)))
        } else if (CLINT_MTIMECMP..CLINT_MTIMECMP + 8).contains(&off) {
            Some((CLINT_MTIMECMP, self.mtimecmp))
        } else if (CLINT_MTIME..CLINT_MTIME + 8).contains(&off) {
            Some((CLINT_MTIME, c.mtime(self.mtime)))
        } else {
            None
        }
    }

    fn clint_read(&self, off: u32, size: u32) -> u32 {
        let Some((reg_off, value)) = self.clint_reg(off) else {
            return 0;
        };
        let v = (value >> ((off - reg_off) * 8)) as u32;
        if size >= 4 {
            v
        } else {
            v & ((1u32 << (size * 8)) - 1)
        }
    }

    fn clint_write(&mut self, off: u32, size: u32, value: u32) {
        let Some((reg_off, old)) = self.clint_reg(off) else {
            return;
        };
        let shift = (off - reg_off) * 8;
        let mask = if size >= 4 {
            u64::from(u32::MAX) << shift
        } else {
            u64::from((1u32 << (size * 8)) - 1) << shift
        };
        let new = (old & !mask) | ((u64::from(value) << shift) & mask);
        let cycles = self.mtime;
        let Some(c) = self.clint.as_mut() else {
            return;
        };
        match reg_off {
            0 => c.msip = new & 1 != 0,
            CLINT_MTIMECMP => self.mtimecmp = new,
            _ => {
                let base = c.mtime(cycles).wrapping_sub(c.mtime_offset);
                c.mtime_offset = new.wrapping_sub(base);
            }
        }
        self.refresh_timer_lines();
    }

    /// Data load of `size` bytes, decoding the CLINT and PLIC windows before
    /// the bus.
    #[inline]
    fn load(&mut self, bus: &mut dyn Bus, addr: u32, size: u32) -> SimResult<u32> {
        if let Some(off) = self.clint_offset(addr) {
            return Ok(self.clint_read(off, size));
        }
        if let Some(off) = self.plic_offset(addr) {
            return Ok(self.plic_read(off, size));
        }
        if let Some(pfic) = self.pfic.as_ref() {
            if let Some(off) = QingKePfic::offset(addr) {
                return Ok(pfic.read(off, size));
            }
        }
        Ok(match size {
            1 => u32::from(bus.read_u8(u64::from(addr))?),
            2 => u32::from(bus.read_u16(u64::from(addr))?),
            _ => bus.read_u32(u64::from(addr))?,
        })
    }

    /// Data store of `size` bytes, decoding the CLINT and PLIC windows before
    /// the bus.
    #[inline]
    fn store(&mut self, bus: &mut dyn Bus, addr: u32, size: u32, value: u32) -> SimResult<()> {
        if let Some(off) = self.clint_offset(addr) {
            self.clint_write(off, size, value);
            return Ok(());
        }
        if let Some(off) = self.plic_offset(addr) {
            self.plic_write(off, size, value);
            return Ok(());
        }
        if let Some(pfic) = self.pfic.as_mut() {
            if let Some(off) = QingKePfic::offset(addr) {
                pfic.write(off, size, value);
                return Ok(());
            }
        }
        match size {
            1 => bus.write_u8(u64::from(addr), value as u8),
            2 => bus.write_u16(u64::from(addr), value as u16),
            _ => bus.write_u32(u64::from(addr), value),
        }
    }

    /// Aggregate an ADDI/back-branch loop only inside an already-vetted fetch
    /// window. Decode anew each batch: no promoted block can outlive a guest
    /// store or restored code. Cold/non-code/cross-window PCs stay interpreted.
    fn try_spin_window(&mut self, bus: &dyn Bus, budget: u32) -> u32 {
        if budget < 8
            // An armed trigger must see every access and fetch individually.
            || self.triggers.armed()
            || self.waiting_for_interrupt
            || (self.mstatus & (1 << 3)) != 0
            || bus.external_irq_lines() != 0
            || bus.requires_cycle_accurate()
        {
            return 0;
        }
        let offset = self.pc.wrapping_sub(self.fetch_base) as usize;
        let Some(bytes) = self.fetch_bytes[..usize::from(self.fetch_len)].get(offset..) else {
            return 0;
        };
        // Four bytes per decode match the interpreter, including compressed
        // instructions. Never speculate an MMIO read to recognize a loop.
        let Some(first) = bytes.get(..4) else {
            return 0;
        };
        let word = u32::from_le_bytes(first.try_into().unwrap());
        let length = if word & 3 == 3 { 4 } else { 2 };
        let (reg, addend) = match decode_rv32(word) {
            Instruction::Addi { rd, rs1, imm } if rd != 0 && rd == rs1 => (rd, imm),
            Instruction::CAddi { rd, imm } if rd != 0 => (rd, imm),
            _ => return 0,
        };
        let Some(second) = bytes.get(length..length + 4) else {
            return 0;
        };
        let branch = u32::from_le_bytes(second.try_into().unwrap());
        let displacement = match decode_rv32(branch) {
            Instruction::Jal { rd: 0, imm } | Instruction::CJ { imm } => imm,
            _ => return 0,
        };
        if (length as u32).wrapping_add(displacement as u32) != 0 {
            return 0;
        }
        let retired = budget & !1;
        let delta = addend.wrapping_mul((retired / 2) as i32) as u32;
        self.write_reg(reg, self.read_reg(reg).wrapping_add(delta));
        self.update_mtime_after_elapsed_cycles(u64::from(retired));
        retired
    }

    /// Execute the compiler/ROM-emitted MMIO poll shape
    /// `lw; srli; andi; bnez(back)` as one small decoded loop.
    ///
    /// Unlike a wait-state fast-forward, this deliberately performs every
    /// load. Register-visible results, read side effects, memory accounting,
    /// and the exact cycle observed by lazy peripherals therefore remain the
    /// same as four calls through [`Self::step`]. The win is only removing
    /// repeated fetch/decode/dispatch and batch-policy work around those four
    /// instructions. The ESP32-C3 mask ROM spends most of early boot in this
    /// exact status-poll loop.
    fn try_masked_poll_window(
        &mut self,
        bus: &mut dyn Bus,
        budget: u32,
        exact_cycle: Option<u64>,
    ) -> SimResult<u32> {
        const LOOP_INSNS: u32 = 4;
        if budget < LOOP_INSNS
            // An armed trigger must see every access and fetch individually.
            // CLINT and PLIC accesses are decoded by the core, not by the
            // bus reads and writes these windows replay.
            || self.decodes_core_mmio()
            || self.triggers.armed()
            || self.waiting_for_interrupt
            || (self.mstatus & (1 << 3)) != 0
            || bus.external_irq_lines() != 0
            || bus.requires_cycle_accurate()
        {
            return Ok(0);
        }

        let head_off = self.pc.wrapping_sub(self.fetch_base) as usize;
        let bytes = &self.fetch_bytes[..usize::from(self.fetch_len)];
        if head_off >= bytes.len() {
            return Ok(0);
        }
        let decode_at = |off: usize| -> Option<(Instruction, usize)> {
            let low = u16::from_le_bytes(bytes.get(off..off + 2)?.try_into().ok()?);
            let len = if low & 3 == 3 { 4 } else { 2 };
            let mut raw = [0u8; 4];
            raw[..len].copy_from_slice(bytes.get(off..off + len)?);
            Some((decode_rv32(u32::from_le_bytes(raw)), len))
        };

        let (load, load_len) = match decode_at(head_off) {
            Some(v) => v,
            None => return Ok(0),
        };
        let (value_reg, base_reg, load_imm) = match load {
            Instruction::Lw { rd, rs1, imm } => (rd, rs1, imm as u32),
            Instruction::CLw { rd, rs1, imm } => (rd, rs1, imm),
            Instruction::CLwsp { rd, imm } => (rd, 2, imm),
            _ => return Ok(0),
        };
        // The address must stay invariant across iterations.
        if value_reg == 0 || value_reg == base_reg {
            return Ok(0);
        }

        let shift_off = head_off + load_len;
        let (shift, shift_len) = match decode_at(shift_off) {
            Some(v) => v,
            None => return Ok(0),
        };
        let shamt = match shift {
            Instruction::Srli { rd, rs1, shamt } if rd == value_reg && rs1 == value_reg => shamt,
            _ => return Ok(0),
        };

        let mask_off = shift_off + shift_len;
        let (mask, mask_len) = match decode_at(mask_off) {
            Some(v) => v,
            None => return Ok(0),
        };
        let mask = match mask {
            Instruction::Andi { rd, rs1, imm } if rd == value_reg && rs1 == value_reg => imm as u32,
            _ => return Ok(0),
        };

        let branch_off = mask_off + mask_len;
        let (branch, branch_len) = match decode_at(branch_off) {
            Some(v) => v,
            None => return Ok(0),
        };
        let displacement = match branch {
            Instruction::Bne { rs1, rs2: 0, imm } if rs1 == value_reg => imm,
            Instruction::Bne { rs1: 0, rs2, imm } if rs2 == value_reg => imm,
            Instruction::CBnez { rs1, imm } if rs1 == value_reg => imm,
            _ => return Ok(0),
        };
        if (branch_off as u32).wrapping_add(displacement as u32) != head_off as u32 {
            return Ok(0);
        }

        let fallthrough = self.pc.wrapping_add(
            (load_len + shift_len + mask_len + branch_len)
                .try_into()
                .unwrap(),
        );
        let addr = self.read_reg(base_reg).wrapping_add(load_imm);
        let iterations = budget / LOOP_INSNS;
        let mut retired = 0u32;
        // Always perform the first read through the ordinary bus path. Besides
        // preserving its exact error/side-effect behavior, this proves whether
        // the loop exits immediately before asking the peripheral to coalesce.
        for iteration in 0..iterations {
            if let Some(batch_start) = exact_cycle {
                bus.publish_cycle(batch_start + u64::from(retired));
            }
            let value = bus.read_u32(u64::from(addr))?;
            let tested = value.wrapping_shr(u32::from(shamt)) & mask;
            self.write_reg(value_reg, tested);
            retired += LOOP_INSNS;
            self.update_mtime_after_elapsed_cycles(u64::from(LOOP_INSNS));
            if tested == 0 {
                self.pc = fallthrough;
                return Ok(retired);
            }
            let remaining = iterations - iteration - 1;
            if remaining != 0 {
                if let Some(repeated) = bus.repeat_stable_read_u32(u64::from(addr), remaining) {
                    let repeated = repeated?;
                    debug_assert_eq!(repeated, value, "stable repeated read changed value");
                    let skipped = remaining * LOOP_INSNS;
                    retired += skipped;
                    self.update_mtime_after_elapsed_cycles(u64::from(skipped));
                    break;
                }
            }
        }
        // The final branch was taken, so execution remains at the loop head.
        self.pc = self.fetch_base.wrapping_add(head_off as u32);
        Ok(retired)
    }

    /// Aggregate the compiler-emitted `store; addi; back-edge` form of a hot
    /// arithmetic loop. Unlike [`Self::try_spin_window`], this loop has a RAM
    /// side effect: `black_box` and volatile accumulator loops commonly spill
    /// the current value to the stack on every trip.
    ///
    /// The optimization is deliberately narrower than a generic trace JIT.
    /// It only accepts one fixed ordinary-RAM address, refuses every observer,
    /// permission gate, interrupt, and cycle-accurate bus, preserves the RAM
    /// access count, and commits the exact value the final interpreted store
    /// would leave behind. Any uncertainty returns zero before mutating state.
    fn try_store_spin_window(&mut self, bus: &mut dyn Bus, budget: u32) -> u32 {
        if budget < 6
            // An armed trigger must see every access and fetch individually.
            // CLINT and PLIC accesses are decoded by the core, not by the
            // bus reads and writes these windows replay.
            || self.decodes_core_mmio()
            || self.triggers.armed()
            || self.waiting_for_interrupt
            || (self.mstatus & (1 << 3)) != 0
            || bus.external_irq_lines() != 0
            || bus.requires_cycle_accurate()
        {
            return 0;
        }
        let offset = self.pc.wrapping_sub(self.fetch_base) as usize;
        let bytes = &self.fetch_bytes[..usize::from(self.fetch_len)];
        if offset >= bytes.len() {
            return 0;
        }
        let decode_at = |off: usize| -> Option<(Instruction, usize)> {
            let low = u16::from_le_bytes(bytes.get(off..off + 2)?.try_into().ok()?);
            let len = if low & 3 == 3 { 4 } else { 2 };
            let mut raw = [0u8; 4];
            raw[..len].copy_from_slice(bytes.get(off..off + len)?);
            Some((decode_rv32(u32::from_le_bytes(raw)), len))
        };

        // A machine window can begin on any of the three loop instructions
        // (512 is not divisible by three). Search backwards within the maximum
        // two RV32 instructions preceding the current PC and accept the unique
        // store/add/back-edge block containing it. `prefix` is the number of
        // instructions from this phase to the store-headed phase: add needs
        // add+branch (2), branch needs branch (1), store needs none.
        let mut recognized = None;
        for back in [0usize, 2, 4, 6, 8] {
            let Some(head_off) = offset.checked_sub(back) else {
                continue;
            };
            let Some((store, store_len)) = decode_at(head_off) else {
                continue;
            };
            let (base_reg, store_reg, store_imm) = match store {
                Instruction::Sw { rs1, rs2, imm } => (rs1, rs2, imm as u32),
                Instruction::CSw { rs1, rs2, imm } => (rs1, rs2, imm),
                Instruction::CSwsp { rs2, imm } => (2, rs2, imm),
                _ => continue,
            };
            let add_off = head_off + store_len;
            let Some((add, add_len)) = decode_at(add_off) else {
                continue;
            };
            let (add_reg, addend) = match add {
                Instruction::Addi { rd, rs1, imm } if rd != 0 && rd == rs1 => (rd, imm),
                Instruction::CAddi { rd, imm } if rd != 0 => (rd, imm),
                _ => continue,
            };
            if add_reg == base_reg {
                continue;
            }
            let branch_off = add_off + add_len;
            let Some((branch, _)) = decode_at(branch_off) else {
                continue;
            };
            let displacement = match branch {
                Instruction::Jal { rd: 0, imm } | Instruction::CJ { imm } => imm,
                _ => continue,
            };
            if (branch_off as u32).wrapping_add(displacement as u32) != head_off as u32 {
                continue;
            }
            let prefix = if offset == head_off {
                0
            } else if offset == add_off {
                2
            } else if offset == branch_off {
                1
            } else {
                continue;
            };
            recognized = Some((
                head_off, base_reg, store_reg, store_imm, add_reg, addend, prefix,
            ));
            break;
        }
        let Some((head_off, base_reg, store_reg, store_imm, add_reg, addend, prefix)) = recognized
        else {
            return 0;
        };
        if budget < prefix + 3 {
            return 0;
        }

        let addr = self.read_reg(base_reg).wrapping_add(store_imm);
        let iterations = (budget - prefix) / 3;
        let retired = prefix + iterations * 3;
        let prefix_adds = u32::from(prefix == 2);
        let initial_add = self.read_reg(add_reg);
        let final_store = if store_reg == add_reg {
            initial_add
                .wrapping_add(addend.wrapping_mul((prefix_adds + iterations - 1) as i32) as u32)
        } else {
            self.read_reg(store_reg)
        };
        // The bus decides whether the loop's stores may collapse into one
        // (PMS, observers, flat-RAM window) and, only if so, commits the final
        // value and the full RAM write count. A capability, not a downcast to
        // `SystemBus`; a refusal has changed nothing on either side.
        if !bus.commit_ram_store_spin(addr, final_store, u64::from(iterations)) {
            return 0;
        }
        self.write_reg(
            add_reg,
            initial_add.wrapping_add(addend.wrapping_mul((prefix_adds + iterations) as i32) as u32),
        );
        self.pc = self.fetch_base.wrapping_add(head_off as u32);
        self.reservation = None;
        self.update_mtime_after_elapsed_cycles(u64::from(retired));
        retired
    }

    fn invalidate_fetch_window(&mut self) {
        self.fetch_len = 0;
        self.fetch_refill_failed_base = None;
        self.block_cache.invalidate();
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
            // WCH QingKe V2 INTSYSCR (CH32V003/V005/V006 are the RV32E chips):
            // the vendor startup writes HWSTKEN|INESTEN before `mret` to main.
            // With a PFIC attached, HWSTKEN drives the hardware prologue and
            // INESTEN is stored only. Without one it reads as zero.
            0x804 if self.pfic.is_some() => self.intsyscr,
            0x804 if self.core_profile == RiscVCoreProfile::Rv32e => 0,
            // This emulator exposes one hart, whose architectural ID is zero.
            0xF14 => 0,
            // Debug trigger module (hardware breakpoints / watchpoints).
            0x7A0..=0x7A5 if self.core_profile == RiscVCoreProfile::Esp32C3 => {
                return self.triggers.read_csr(csr)
            }
            0x300 => self.mstatus,
            // medeleg / mideleg — M-mode only emulator; esp_intr_alloc on C6
            // does `csrrc mideleg` when binding CPU ints. Accept as RAZ/WI.
            0x302 | 0x303 if self.core_profile == RiscVCoreProfile::Esp32C3 => 0,
            0x304 => self.mie,
            0x344 => self.mip,
            0x305 => self.mtvec,
            0x340 => self.mscratch,
            0x341 => self.mepc,
            0x342 => self.mcause,
            0x343 => self.mtval,
            // Physical Memory Protection (PMP) configuration and address registers (stubs)
            0x3A0..=0x3A3 | 0x3B0..=0x3BF => 0,
            // Espressif Physical Memory Attributes (PMA) — CSR_PMACFG0=0xBC0,
            // CSR_PMAADDR0=0xBD0 (riscv/csr.h). esp_cpu_configure_invalid_regions
            // on C6 writes these during early init; side effects are not modelled.
            0xBC0..=0xBDF if self.core_profile == RiscVCoreProfile::Esp32C3 => 0,
            // QingKe V4 CORECFGR: see `RiscVCoreProfile::QingkeV4`.
            0xBC0 if self.core_profile == RiscVCoreProfile::QingkeV4 => self.corecfgr,
            // Timer CSR stubs (Standard RISC-V shadow non-privileged? No, these are machine mode)
            0xB00 => (self.mtime & 0xFFFFFFFF) as u32,
            0xB80 => (self.mtime >> 32) as u32,
            // The ESP32-C3 exposes its free-running counter at the custom
            // machine PCCR CSR 0x7E2, not at standard RISC-V cycle CSR 0xC00.
            //
            // The cycle counters count CPU clocks. One retired instruction is
            // one simulated CPU clock (`mtime` advances once per clock, the
            // same clock the SYSTIMER/`esp_timer` and peripherals are paced
            // by), so they read `mtime` directly. Firmware converts them with
            // the chip clock (e.g. Arduino `pulseIn` timeouts, `ets_delay_us`),
            // so any other rate would skew those against `micros()`.
            0x7E0..=0x7E2 | 0x802 if self.core_profile == RiscVCoreProfile::Esp32C3 => {
                (self.mtime & 0xFFFFFFFF) as u32
            }
            // The requested RV32E profile shares the baseline profile's cycle
            // CSR behavior; this profile choice is separate from register count.
            0xC00
                if matches!(
                    self.core_profile,
                    RiscVCoreProfile::StandardRv32
                        | RiscVCoreProfile::Rv32e
                        | RiscVCoreProfile::QingkeV4
                ) =>
            {
                (self.mtime & 0xFFFFFFFF) as u32
            }
            0xC80
                if matches!(
                    self.core_profile,
                    RiscVCoreProfile::StandardRv32
                        | RiscVCoreProfile::Rv32e
                        | RiscVCoreProfile::QingkeV4
                ) =>
            {
                (self.mtime >> 32) as u32
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
            // See read_csr: QingKe V2 INTSYSCR (HWSTKEN, INESTEN).
            0x804 if self.pfic.is_some() => self.intsyscr = val & 0x3,
            0x804 if self.core_profile == RiscVCoreProfile::Rv32e => {}
            0x7A0..=0x7A5 if self.core_profile == RiscVCoreProfile::Esp32C3 => {
                return self.triggers.write_csr(csr, val)
            }
            0x300 => self.mstatus = val & 0x0000_1888, // Minimal mstatus (MIE, MPP)
            // See read_csr: medeleg/mideleg RAZ/WI on Esp32C3 profile.
            0x302 | 0x303 if self.core_profile == RiscVCoreProfile::Esp32C3 => {}
            0x304 => self.mie = val,
            0x344 => {
                // MEIP is read-only: the PLIC drives it.
                self.mip = val;
                self.refresh_meip();
            }
            0x305 => self.mtvec = val,
            0x340 => self.mscratch = val,
            0x341 => self.mepc = val,
            0x342 => self.mcause = val,
            0x343 => self.mtval = val,
            // Physical Memory Protection (PMP) configuration and address registers (stubs)
            0x3A0..=0x3A3 | 0x3B0..=0x3BF => {}
            // See read_csr: accept PMA cfg/addr writes as no-ops.
            0xBC0..=0xBDF if self.core_profile == RiscVCoreProfile::Esp32C3 => {}
            0xBC0 if self.core_profile == RiscVCoreProfile::QingkeV4 => self.corecfgr = val,
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
        if let Some(pfic) = self.pfic.as_mut() {
            // QingKe HPE: with HWSTKEN the hardware saves the caller-saved
            // registers on entry and `mret` restores them, so vendor
            // `WCH-Interrupt-fast` handlers save nothing themselves.
            let saved = (self.intsyscr & INTSYSCR_HWSTKEN != 0).then(|| {
                let mut saved = [0; 16];
                for (slot, &r) in saved.iter_mut().zip(self.core_profile.hpe_regs()) {
                    *slot = self.x[r];
                }
                saved
            });
            let irq = (cause & 0x8000_0000 != 0).then_some(cause & 0x7FFF_FFFF);
            pfic.frames.push(PficFrame { irq, saved });
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
                                    // tcontrol: mpte <- mte, mte <- 0, so no trigger fires in the handler.
        self.triggers.on_trap_entry();
    }

    /// The trigger-module check for the instruction about to execute
    /// (`timing = 0`: before it has any effect). Execute triggers match the
    /// instruction's own bytes; load/store triggers the effective address the
    /// instruction is about to access. Only called while a trigger is armed.
    fn trigger_check(&mut self, instruction: &Instruction, inst_len: u32) -> Option<u32> {
        use trigger::AccessKind::{Load, LoadStore, Store};
        if let Some(hit) = self.triggers.check_execute(self.pc, inst_len) {
            return Some(hit.address);
        }
        let ea = |base: u32, off: u32| base.wrapping_add(off);
        let (addr, len, kind) = match *instruction {
            Instruction::Lb { rs1, imm, .. } | Instruction::Lbu { rs1, imm, .. } => {
                (ea(self.read_reg(rs1), imm as u32), 1, Load)
            }
            Instruction::Lh { rs1, imm, .. } | Instruction::Lhu { rs1, imm, .. } => {
                (ea(self.read_reg(rs1), imm as u32), 2, Load)
            }
            Instruction::Lw { rs1, imm, .. } => (ea(self.read_reg(rs1), imm as u32), 4, Load),
            Instruction::CLw { rs1, imm, .. } => (ea(self.read_reg(rs1), imm), 4, Load),
            Instruction::CLwsp { imm, .. } => (ea(self.read_reg(2), imm), 4, Load),
            Instruction::Sb { rs1, imm, .. } => (ea(self.read_reg(rs1), imm as u32), 1, Store),
            Instruction::Sh { rs1, imm, .. } => (ea(self.read_reg(rs1), imm as u32), 2, Store),
            Instruction::Sw { rs1, imm, .. } => (ea(self.read_reg(rs1), imm as u32), 4, Store),
            Instruction::CSw { rs1, imm, .. } => (ea(self.read_reg(rs1), imm), 4, Store),
            Instruction::CSwsp { imm, .. } => (ea(self.read_reg(2), imm), 4, Store),
            Instruction::LrW { rs1, .. } => (self.read_reg(rs1), 4, Load),
            Instruction::ScW { rs1, .. } => (self.read_reg(rs1), 4, Store),
            Instruction::AmoSwapW { rs1, .. }
            | Instruction::AmoAddW { rs1, .. }
            | Instruction::AmoXorW { rs1, .. }
            | Instruction::AmoOrW { rs1, .. }
            | Instruction::AmoAndW { rs1, .. }
            | Instruction::AmoMinW { rs1, .. }
            | Instruction::AmoMaxW { rs1, .. }
            | Instruction::AmoMinuW { rs1, .. }
            | Instruction::AmoMaxuW { rs1, .. } => (self.read_reg(rs1), 4, LoadStore),
            _ => return None,
        };
        self.triggers
            .check_data(addr, len, kind)
            .map(|hit| hit.address)
    }
}

#[inline]
fn pending_schedule_deadline(_bus: &dyn Bus) -> Option<u64> {
    #[cfg(feature = "event-scheduler")]
    {
        if _bus.has_pending_schedule() {
            return _bus.earliest_pending_deadline();
        }
    }
    None
}

impl RiscV {
    /// Would a compiled or cached block that retires `n` instructions from the
    /// current state step *across* an interrupt the interpreter would have
    /// taken mid-block? If so the caller interprets the stretch instead, so
    /// the trap lands at exactly the instruction the interpreter would trap on.
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
        if (self.mip & self.mie) != 0 || bus.external_irq_lines() != 0 || self.pfic_ready() {
            return true;
        }
        // Internal timer edge inside the block's mtime span.
        let timer_unmasked = (self.mie & (1 << 7)) != 0;
        timer_unmasked && self.cycles_until_mtip().is_some_and(|c| c <= u64::from(n))
    }

    /// Execute one or more cached decoded basic blocks via [`Rv32BlockCache`]
    /// (MIT design adapted from `mikroverk/esp32sim` EX265), returning the
    /// number of instructions retired (`0` if the current PC is not eligible
    /// for block execution and should fall back to [`Self::step`]).
    fn try_step_cached_blocks(
        &mut self,
        bus: &mut dyn Bus,
        max_count: u32,
        exact_clock: bool,
        batch_start: u64,
        already_retired: u32,
    ) -> SimResult<u32> {
        use super::riscv_block_cache::{
            FLAG_HAS_MEM, FLAG_HAS_STORE, FLAG_SELF_LOOP, FLAG_VERIFY_ENTRY, MAX_BLOCK_LEN,
        };

        if max_count == 0 || self.waiting_for_interrupt || self.triggers.armed() {
            return Ok(0);
        }

        // Fast guard against cycle-accurate buses and active debug watches.
        if !bus.block_cache_eligible() {
            return Ok(0);
        }

        let mut total_retired: u32 = 0;

        loop {
            let remaining = max_count - total_retired;
            if remaining == 0 {
                break;
            }
            if exact_clock {
                if let Some(dl) = pending_schedule_deadline(bus) {
                    let now = batch_start.wrapping_add(u64::from(already_retired + total_retired));
                    if dl <= now {
                        break;
                    }
                }
            }

            let pc = self.pc;
            let meta = if let Some(m) = self.block_cache.lookup(pc) {
                // Re-verify synthetic base-zero flash entry word so debugger
                // host-patching tests see patched words immediately.
                if (m.flags & FLAG_VERIFY_ENTRY) != 0
                    && bus.read_flash_entry_word(pc as u64) != Some(m.entry_word)
                {
                    self.block_cache.invalidate();
                    break;
                }
                m
            } else {
                // Compile a fresh decoded block from side-effect-free code
                // memory. Warm the fetch window first so spin/poll fast paths
                // (`try_spin_window`) also see the live window bytes.
                if bus.check_fetch_permission(pc as u64) != crate::FetchPermission::Allowed {
                    break;
                }
                self.refill_fetch_window(bus, pc);
                let Some((code_bytes, writable_code, verify_entry)) =
                    bus.read_block_cache_code(pc as u64, MAX_BLOCK_LEN * 4)
                else {
                    break;
                };

                // Clamp `code_bytes` before any 256-byte window whose fetch
                // permission is not `Allowed` (preserves PMS violation traps).
                let mut allowed_len = code_bytes.len();
                let start_win = pc & !0xFF;
                let end_pc = pc.wrapping_add(allowed_len as u32);
                let mut win = start_win.wrapping_add(0x100);
                while win > start_win && win < end_pc {
                    if bus.check_fetch_permission(win as u64) != crate::FetchPermission::Allowed {
                        allowed_len = (win - pc) as usize;
                        break;
                    }
                    win = win.wrapping_add(0x100);
                }

                let Some(m) = self.block_cache.compile_block(
                    pc,
                    &code_bytes[..allowed_len],
                    self.core_profile,
                    writable_code,
                    verify_entry,
                ) else {
                    break;
                };
                m
            };

            let block_len = meta.len as u32;
            if block_len > remaining {
                break;
            }
            if self.block_would_cross_irq(bus, block_len) {
                break;
            }
            if exact_clock {
                if let Some(dl) = pending_schedule_deadline(bus) {
                    let now = batch_start.wrapping_add(u64::from(already_retired + total_retired));
                    if dl < now.saturating_add(u64::from(block_len)) {
                        break;
                    }
                }
            }

            let has_mem = (meta.flags & FLAG_HAS_MEM) != 0;
            let has_store = (meta.flags & FLAG_HAS_STORE) != 0;
            let is_self_loop = (meta.flags & FLAG_SELF_LOOP) != 0;

            // Temporarily move `block_cache` out or copy the tiny block slice
            // (<= 32 steps) onto the stack so `self` is free for `load`/`store`.
            let mut local_steps = [super::riscv_block_cache::DecodedStep {
                inst: Instruction::Fence,
                pc: 0,
                next_pc: 0,
            }; MAX_BLOCK_LEN];
            local_steps[..meta.len].copy_from_slice(self.block_cache.steps(meta));
            let steps = &local_steps[..meta.len];

            // Self-loop tight execution (from `mikroverk/esp32sim` EX265):
            // repeat the decoded block without re-indexing the cache table
            // while it jumps back to `meta.tag_pc`, budget permits, and no
            // interrupt deadline is crossed.
            let mut streak_retired: u32 = 0;
            loop {
                let mut next_pc = meta.tag_pc;
                let mut broke_early = false;
                for (idx, step) in steps.iter().enumerate() {
                    self.pc = step.pc;
                    next_pc = step.next_pc;

                    if exact_clock && has_mem {
                        bus.publish_cycle(batch_start.wrapping_add(u64::from(
                            already_retired + total_retired + streak_retired + idx as u32,
                        )));
                    }

                    match step.inst {
                        Instruction::Lui { rd, imm } => self.write_reg(rd, imm),
                        Instruction::Auipc { rd, imm } => {
                            self.write_reg(rd, step.pc.wrapping_add(imm));
                        }
                        Instruction::Addi { rd, rs1, imm } => {
                            let val = self.read_reg(rs1).wrapping_add(imm as u32);
                            self.write_reg(rd, val);
                        }
                        Instruction::Slti { rd, rs1, imm } => {
                            let val = u32::from((self.read_reg(rs1) as i32) < imm);
                            self.write_reg(rd, val);
                        }
                        Instruction::Sltiu { rd, rs1, imm } => {
                            let val = u32::from(self.read_reg(rs1) < imm as u32);
                            self.write_reg(rd, val);
                        }
                        Instruction::Xori { rd, rs1, imm } => {
                            self.write_reg(rd, self.read_reg(rs1) ^ (imm as u32));
                        }
                        Instruction::Ori { rd, rs1, imm } => {
                            self.write_reg(rd, self.read_reg(rs1) | (imm as u32));
                        }
                        Instruction::Andi { rd, rs1, imm } => {
                            self.write_reg(rd, self.read_reg(rs1) & (imm as u32));
                        }
                        Instruction::Slli { rd, rs1, shamt } => {
                            self.write_reg(rd, self.read_reg(rs1) << (shamt & 0x1F));
                        }
                        Instruction::Srli { rd, rs1, shamt } => {
                            self.write_reg(rd, self.read_reg(rs1) >> (shamt & 0x1F));
                        }
                        Instruction::Srai { rd, rs1, shamt } => {
                            self.write_reg(
                                rd,
                                ((self.read_reg(rs1) as i32) >> (shamt & 0x1F)) as u32,
                            );
                        }
                        Instruction::Add { rd, rs1, rs2 } => {
                            self.write_reg(rd, self.read_reg(rs1).wrapping_add(self.read_reg(rs2)));
                        }
                        Instruction::Sub { rd, rs1, rs2 } => {
                            self.write_reg(rd, self.read_reg(rs1).wrapping_sub(self.read_reg(rs2)));
                        }
                        Instruction::Sll { rd, rs1, rs2 } => {
                            let shamt = self.read_reg(rs2) & 0x1F;
                            self.write_reg(rd, self.read_reg(rs1) << shamt);
                        }
                        Instruction::Slt { rd, rs1, rs2 } => {
                            let val = u32::from(
                                (self.read_reg(rs1) as i32) < (self.read_reg(rs2) as i32),
                            );
                            self.write_reg(rd, val);
                        }
                        Instruction::Sltu { rd, rs1, rs2 } => {
                            let val = u32::from(self.read_reg(rs1) < self.read_reg(rs2));
                            self.write_reg(rd, val);
                        }
                        Instruction::Xor { rd, rs1, rs2 } => {
                            self.write_reg(rd, self.read_reg(rs1) ^ self.read_reg(rs2));
                        }
                        Instruction::Srl { rd, rs1, rs2 } => {
                            let shamt = self.read_reg(rs2) & 0x1F;
                            self.write_reg(rd, self.read_reg(rs1) >> shamt);
                        }
                        Instruction::Sra { rd, rs1, rs2 } => {
                            let shamt = self.read_reg(rs2) & 0x1F;
                            self.write_reg(rd, ((self.read_reg(rs1) as i32) >> shamt) as u32);
                        }
                        Instruction::Or { rd, rs1, rs2 } => {
                            self.write_reg(rd, self.read_reg(rs1) | self.read_reg(rs2));
                        }
                        Instruction::And { rd, rs1, rs2 } => {
                            self.write_reg(rd, self.read_reg(rs1) & self.read_reg(rs2));
                        }
                        Instruction::Fence => {}
                        Instruction::Mul { rd, rs1, rs2 } => {
                            self.write_reg(rd, self.read_reg(rs1).wrapping_mul(self.read_reg(rs2)));
                        }
                        Instruction::Mulh { rd, rs1, rs2 } => {
                            let a = self.read_reg(rs1) as i32 as i64;
                            let b = self.read_reg(rs2) as i32 as i64;
                            self.write_reg(rd, ((a.wrapping_mul(b)) >> 32) as u32);
                        }
                        Instruction::Mulhsu { rd, rs1, rs2 } => {
                            let a = self.read_reg(rs1) as i32 as i64 as u64;
                            let b = self.read_reg(rs2) as u64;
                            self.write_reg(rd, ((a.wrapping_mul(b)) >> 32) as u32);
                        }
                        Instruction::Mulhu { rd, rs1, rs2 } => {
                            let a = self.read_reg(rs1) as u64;
                            let b = self.read_reg(rs2) as u64;
                            self.write_reg(rd, ((a.wrapping_mul(b)) >> 32) as u32);
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
                            let res = self
                                .read_reg(rs1)
                                .checked_div(self.read_reg(rs2))
                                .unwrap_or(u32::MAX);
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
                        Instruction::CSli { rd, shamt } => {
                            if rd != 0 {
                                let res = self.read_reg(rd).wrapping_shl(u32::from(shamt));
                                self.write_reg(rd, res);
                            }
                        }
                        Instruction::Lb { rd, rs1, imm } => {
                            let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                            match self.load(bus, addr, 1) {
                                Ok(v) => self.write_reg(rd, (v as u8 as i8) as i32 as u32),
                                Err(e) => {
                                    self.update_mtime_after_elapsed_cycles(u64::from(
                                        streak_retired + idx as u32,
                                    ));
                                    return Err(e);
                                }
                            }
                        }
                        Instruction::Lh { rd, rs1, imm } => {
                            let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                            match self.load(bus, addr, 2) {
                                Ok(v) => self.write_reg(rd, (v as u16 as i16) as i32 as u32),
                                Err(e) => {
                                    self.update_mtime_after_elapsed_cycles(u64::from(
                                        streak_retired + idx as u32,
                                    ));
                                    return Err(e);
                                }
                            }
                        }
                        Instruction::Lw { rd, rs1, imm } => {
                            let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                            match self.load(bus, addr, 4) {
                                Ok(v) => self.write_reg(rd, v),
                                Err(e) => {
                                    self.update_mtime_after_elapsed_cycles(u64::from(
                                        streak_retired + idx as u32,
                                    ));
                                    return Err(e);
                                }
                            }
                        }
                        Instruction::Lbu { rd, rs1, imm } => {
                            let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                            match self.load(bus, addr, 1) {
                                Ok(v) => self.write_reg(rd, v),
                                Err(e) => {
                                    self.update_mtime_after_elapsed_cycles(u64::from(
                                        streak_retired + idx as u32,
                                    ));
                                    return Err(e);
                                }
                            }
                        }
                        Instruction::Lhu { rd, rs1, imm } => {
                            let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                            match self.load(bus, addr, 2) {
                                Ok(v) => self.write_reg(rd, v),
                                Err(e) => {
                                    self.update_mtime_after_elapsed_cycles(u64::from(
                                        streak_retired + idx as u32,
                                    ));
                                    return Err(e);
                                }
                            }
                        }
                        Instruction::CLw { rd, rs1, imm } => {
                            let addr = self.read_reg(rs1).wrapping_add(imm);
                            match self.load(bus, addr, 4) {
                                Ok(v) => self.write_reg(rd, v),
                                Err(e) => {
                                    self.update_mtime_after_elapsed_cycles(u64::from(
                                        streak_retired + idx as u32,
                                    ));
                                    return Err(e);
                                }
                            }
                        }
                        Instruction::CLwsp { rd, imm } => {
                            let addr = self.read_reg(2).wrapping_add(imm);
                            match self.load(bus, addr, 4) {
                                Ok(v) => self.write_reg(rd, v),
                                Err(e) => {
                                    self.update_mtime_after_elapsed_cycles(u64::from(
                                        streak_retired + idx as u32,
                                    ));
                                    return Err(e);
                                }
                            }
                        }
                        Instruction::Sb { rs1, rs2, imm } => {
                            let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                            let val = self.read_reg(rs2);
                            if let Err(e) = self.store(bus, addr, 1, val) {
                                self.update_mtime_after_elapsed_cycles(u64::from(
                                    streak_retired + idx as u32,
                                ));
                                return Err(e);
                            }
                            self.invalidate_fetch_if_store_overlaps(addr, 1);
                            self.reservation = None;
                            if exact_clock && pending_schedule_deadline(bus).is_some() {
                                self.pc = next_pc;
                                streak_retired += idx as u32 + 1;
                                broke_early = true;
                                break;
                            }
                        }
                        Instruction::Sh { rs1, rs2, imm } => {
                            let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                            let val = self.read_reg(rs2);
                            if let Err(e) = self.store(bus, addr, 2, val) {
                                self.update_mtime_after_elapsed_cycles(u64::from(
                                    streak_retired + idx as u32,
                                ));
                                return Err(e);
                            }
                            self.invalidate_fetch_if_store_overlaps(addr, 2);
                            self.reservation = None;
                            if exact_clock && pending_schedule_deadline(bus).is_some() {
                                self.pc = next_pc;
                                streak_retired += idx as u32 + 1;
                                broke_early = true;
                                break;
                            }
                        }
                        Instruction::Sw { rs1, rs2, imm } => {
                            let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                            let val = self.read_reg(rs2);
                            if let Err(e) = self.store(bus, addr, 4, val) {
                                self.update_mtime_after_elapsed_cycles(u64::from(
                                    streak_retired + idx as u32,
                                ));
                                return Err(e);
                            }
                            self.invalidate_fetch_if_store_overlaps(addr, 4);
                            self.reservation = None;
                            if exact_clock && pending_schedule_deadline(bus).is_some() {
                                self.pc = next_pc;
                                streak_retired += idx as u32 + 1;
                                broke_early = true;
                                break;
                            }
                        }
                        Instruction::CSw { rs2, rs1, imm } => {
                            let addr = self.read_reg(rs1).wrapping_add(imm);
                            let val = self.read_reg(rs2);
                            if let Err(e) = self.store(bus, addr, 4, val) {
                                self.update_mtime_after_elapsed_cycles(u64::from(
                                    streak_retired + idx as u32,
                                ));
                                return Err(e);
                            }
                            self.invalidate_fetch_if_store_overlaps(addr, 4);
                            self.reservation = None;
                            if exact_clock && pending_schedule_deadline(bus).is_some() {
                                self.pc = next_pc;
                                streak_retired += idx as u32 + 1;
                                broke_early = true;
                                break;
                            }
                        }
                        Instruction::CSwsp { rs2, imm } => {
                            let addr = self.read_reg(2).wrapping_add(imm);
                            let val = self.read_reg(rs2);
                            if let Err(e) = self.store(bus, addr, 4, val) {
                                self.update_mtime_after_elapsed_cycles(u64::from(
                                    streak_retired + idx as u32,
                                ));
                                return Err(e);
                            }
                            self.invalidate_fetch_if_store_overlaps(addr, 4);
                            self.reservation = None;
                            if exact_clock && pending_schedule_deadline(bus).is_some() {
                                self.pc = next_pc;
                                streak_retired += idx as u32 + 1;
                                broke_early = true;
                                break;
                            }
                        }
                        Instruction::Jal { rd, imm } => {
                            let target = step.pc.wrapping_add(imm as u32);
                            self.write_reg(rd, step.next_pc);
                            next_pc = target;
                        }
                        Instruction::Jalr { rd, rs1, imm } => {
                            let target = self.read_reg(rs1).wrapping_add(imm as u32) & !1;
                            self.write_reg(rd, step.next_pc);
                            next_pc = target;
                        }
                        Instruction::Beq { rs1, rs2, imm } => {
                            if self.read_reg(rs1) == self.read_reg(rs2) {
                                next_pc = step.pc.wrapping_add(imm as u32);
                            }
                        }
                        Instruction::Bne { rs1, rs2, imm } => {
                            if self.read_reg(rs1) != self.read_reg(rs2) {
                                next_pc = step.pc.wrapping_add(imm as u32);
                            }
                        }
                        Instruction::Blt { rs1, rs2, imm } => {
                            if (self.read_reg(rs1) as i32) < (self.read_reg(rs2) as i32) {
                                next_pc = step.pc.wrapping_add(imm as u32);
                            }
                        }
                        Instruction::Bge { rs1, rs2, imm } => {
                            if (self.read_reg(rs1) as i32) >= (self.read_reg(rs2) as i32) {
                                next_pc = step.pc.wrapping_add(imm as u32);
                            }
                        }
                        Instruction::Bltu { rs1, rs2, imm } => {
                            if self.read_reg(rs1) < self.read_reg(rs2) {
                                next_pc = step.pc.wrapping_add(imm as u32);
                            }
                        }
                        Instruction::Bgeu { rs1, rs2, imm } => {
                            if self.read_reg(rs1) >= self.read_reg(rs2) {
                                next_pc = step.pc.wrapping_add(imm as u32);
                            }
                        }
                        Instruction::CJr { rs1 } => {
                            next_pc = self.read_reg(rs1) & !1;
                        }
                        Instruction::CJalr { rs1 } => {
                            let target = self.read_reg(rs1) & !1;
                            self.write_reg(1, step.pc.wrapping_add(2));
                            next_pc = target;
                        }
                        Instruction::CJ { imm } => {
                            next_pc = step.pc.wrapping_add(imm as u32);
                        }
                        Instruction::CBeqz { rs1, imm } => {
                            if self.read_reg(rs1) == 0 {
                                next_pc = step.pc.wrapping_add(imm as u32);
                            }
                        }
                        Instruction::CBnez { rs1, imm } => {
                            if self.read_reg(rs1) != 0 {
                                next_pc = step.pc.wrapping_add(imm as u32);
                            }
                        }
                        _ => unreachable!("unsupported instruction in Rv32BlockCache"),
                    }
                }

                if !broke_early {
                    self.pc = next_pc;
                    streak_retired += block_len;
                }

                // Stay inside the decoded self-loop if the branch jumped back
                // to `meta.tag_pc` and no store/deadline/IRQ forces an exit.
                if !broke_early
                    && is_self_loop
                    && !has_store
                    && next_pc == meta.tag_pc
                    && (max_count - total_retired - streak_retired) >= block_len
                    && !self.block_would_cross_irq(bus, streak_retired + block_len)
                    && !(exact_clock && pending_schedule_deadline(bus).is_some())
                {
                    continue;
                }
                break;
            }

            self.update_mtime_after_elapsed_cycles(u64::from(streak_retired));
            total_retired += streak_retired;
        }

        Ok(total_retired)
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
        // Compiled blocks access memory through the bus; they would miss the
        // core-decoded CLINT and PLIC windows.
        if !observers.is_empty() || self.decodes_core_mmio() {
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
            // A non-default RAM `cpu_writes` policy is enforced by the
            // interpreter's store gate; compiled blocks write `bus.ram.data`
            // directly, so they must not run while it is in force.
            if sb.ram_cpu_writes_gated() {
                return false;
            }
            // Same reasoning, one level up: compiled blocks also bypass the
            // accessors that latch a debug watch, and they run whole blocks
            // without the machine getting a window boundary to stop at. Refuse
            // while ANY watch is armed — store-only watches included, because
            // the gate's job is to keep raw-RAM writes off the path, and the
            // per-batch test is cheaper than reasoning about which block might
            // touch which range. `plan_cpu_window` already clamps to one
            // instruction while a watch is armed; this refusal is
            // defence-in-depth for direct `step_batch` callers.
            if !sb.debug_watches.is_empty() {
                return false;
            }
        }
        true
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
        let exact_clock = cfg!(feature = "event-scheduler") && config.peripheral_tick_interval > 1;
        let batch_start = if exact_clock { bus.current_cycle() } else { 0 };

        let flush_jit_ram = |engine: &mut crate::cpu::jit_framework::riscv::RiscvJitEngine,
                             bus: &mut dyn Bus| {
            if let Some(ram) = bus.ram_data_mut() {
                engine.flush_ram(ram);
            }
        };

        let mut retired: u32 = 0;
        while retired < max_count {
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
                    // Compiled blocks bypass the trigger check, so an armed
                    // trigger (possibly armed mid-batch) forces interpretation.
                    let must_interpret = n == 0
                        || retired + n > max_count
                        || self.triggers.armed()
                        || self.block_would_cross_irq(bus, n);
                    if must_interpret {
                        flush_jit_ram(engine, bus);
                        self.step(bus, observers, config)?;
                        engine.note_interpreted();
                        retired += 1;
                    } else if let Some(sb) =
                        bus.as_any_mut().and_then(|a| a.downcast_mut::<SystemBus>())
                    {
                        let (actual_n, next_pc, clear_reservation, needs_interp) =
                            engine.run_ready_batched(pc, &mut self.x, &mut sb.ram.data);
                        if clear_reservation {
                            self.reservation = None;
                        }
                        self.pc = next_pc as u32;
                        // ── THE MTIME FIXUP ──────────────────────────────
                        // A compiled block retires `actual_n` instructions
                        // without calling `step`, so it never advanced the
                        // CLINT `mtime` (the interpreter bumps it 1/instr).
                        // Advance it by exactly `actual_n` here so the cycle
                        // CSRs (0xC00/0x802/0x7E2 = mtime) and the
                        // MTIP timer edge stay identical to a per-instruction
                        // run. This is the analogue of Xtensa's CCOUNT += N-1.
                        self.update_mtime_after_elapsed_cycles(actual_n as u64);
                        retired += actual_n;
                        // Entry-instruction memory fault: nothing retired and
                        // the PC did not move — interpret one for progress.
                        if actual_n == 0 && needs_interp {
                            flush_jit_ram(engine, bus);
                            self.step(bus, observers, config)?;
                            engine.note_interpreted();
                            retired += 1;
                        }
                    } else {
                        // Not a `SystemBus` (never happens in production): fall
                        // back to the interpreter for this instruction.
                        flush_jit_ram(engine, bus);
                        self.step(bus, observers, config)?;
                        engine.note_interpreted();
                        retired += 1;
                    }
                }
                Lookup::Interpret { promote } => {
                    flush_jit_ram(engine, bus);
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
            if config.idle_fast_forward_enabled
                && self.waiting_for_interrupt
                && self.idle_fast_forward_budget(bus).is_some()
            {
                flush_jit_ram(engine, bus);
                return Ok(retired);
            }
            // Mirror the interpreter's Gap #1 deadline clamp (see `step_batch`).
            if config.peripheral_tick_interval > 1 {
                if let Some(dl) = pending_schedule_deadline(bus) {
                    let max_ret = dl.saturating_sub(batch_start);
                    if (retired as u64) >= max_ret {
                        flush_jit_ram(engine, bus);
                        return Ok(retired);
                    }
                }
            }
        }
        flush_jit_ram(engine, bus);
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
        self.invalidate_fetch_window();
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
            let mut instruction = decode_rv32(opcode);
            let inst_len = if (opcode & 0x3) == 0x3 { 4 } else { 2 };
            // QingKe V2 (the RV32E chips) also runs WCH's XW compressed
            // byte/halfword loads and stores; see `decode_qingke_xw`.
            if inst_len == 2
                && self.core_profile == RiscVCoreProfile::Rv32e
                && matches!(instruction, Instruction::Unknown(_))
            {
                if let Some(xw) = decode_qingke_xw(opcode as u16) {
                    instruction = xw;
                }
            }
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

        // RV32E reserves every integer register field above x15. Check all
        // operands before trigger handling or instruction execution so an
        // illegal register use cannot perform a partial memory/CSR effect.
        if self.core_profile == RiscVCoreProfile::Rv32e && rv32e_uses_high_register(&instruction) {
            self.mtval = 0;
            self.handle_trap(2, self.pc);
            return Ok(());
        }

        let mut next_pc = self.pc.wrapping_add(inst_len);

        // Debug trigger module: a matching breakpoint/watchpoint raises a
        // breakpoint exception before the instruction has any effect.
        if self.triggers.armed() {
            if let Some(addr) = self.trigger_check(&instruction, inst_len) {
                self.mtval = addr;
                self.handle_trap(3, self.pc);
                return Ok(());
            }
        }

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
                let val = self.load(bus, addr, 1)? as u8 as i8;
                self.write_reg(rd, val as i32 as u32);
            }
            Instruction::Lh { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.load(bus, addr, 2)? as u16 as i16;
                self.write_reg(rd, val as i32 as u32);
            }
            Instruction::Lw { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.load(bus, addr, 4)?;
                self.write_reg(rd, val);
            }
            Instruction::Lbu { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.load(bus, addr, 1)?;
                self.write_reg(rd, val);
            }
            Instruction::Lhu { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.load(bus, addr, 2)?;
                self.write_reg(rd, val);
            }
            Instruction::Sb { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2) as u8;
                self.store(bus, addr, 1, u32::from(val))?;
                self.invalidate_fetch_if_store_overlaps(addr, 1);
                self.reservation = None;
            }
            Instruction::Sh { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2) as u16;
                self.store(bus, addr, 2, u32::from(val))?;
                self.invalidate_fetch_if_store_overlaps(addr, 2);
                self.reservation = None;
            }
            Instruction::Sw { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2);
                self.store(bus, addr, 4, val)?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
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
                let res = self.read_reg(rs1).wrapping_shl(shamt as u32);
                self.write_reg(rd, res);
            }
            Instruction::Srli { rd, rs1, shamt } => {
                let res = self.read_reg(rs1).wrapping_shr(shamt as u32);
                self.write_reg(rd, res);
            }
            Instruction::Srai { rd, rs1, shamt } => {
                let res = (self.read_reg(rs1) as i32).wrapping_shr(shamt as u32);
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
                if let Some(frame) = self.pfic.as_mut().and_then(QingKePfic::complete) {
                    if let Some(saved) = frame.saved {
                        for (&r, v) in self.core_profile.hpe_regs().iter().zip(saved) {
                            self.x[r] = v;
                        }
                    }
                }
                let mpie = (self.mstatus >> 7) & 1;
                self.mstatus &= !(1 << 3); // clear MIE
                self.mstatus |= mpie << 3; // MIE <- MPIE
                self.mstatus |= 1 << 7; // MPIE <- 1
                self.triggers.on_mret();
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
                let val = self.load(bus, addr, 4)?;
                self.write_reg(rd, val);
            }
            Instruction::CSw { rs2, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm);
                let val = self.read_reg(rs2);
                self.store(bus, addr, 4, val)?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.reservation = None;
            }
            Instruction::CLwsp { rd, imm } => {
                let sp = self.read_reg(2);
                let addr = sp.wrapping_add(imm);
                let val = self.load(bus, addr, 4)?;
                self.write_reg(rd, val);
            }
            Instruction::CSwsp { rs2, imm } => {
                let sp = self.read_reg(2);
                let addr = sp.wrapping_add(imm);
                let val = self.read_reg(rs2);
                self.store(bus, addr, 4, val)?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
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
                    let res = self.read_reg(rd).wrapping_shl(shamt as u32);
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
                    self.invalidate_fetch_if_store_overlaps(addr, 4);
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
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoAddW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                bus.write_u32(addr as u64, old.wrapping_add(self.read_reg(rs2)))?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoXorW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                bus.write_u32(addr as u64, old ^ self.read_reg(rs2))?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoOrW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                bus.write_u32(addr as u64, old | self.read_reg(rs2))?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoAndW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                bus.write_u32(addr as u64, old & self.read_reg(rs2))?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMinW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                let rhs = self.read_reg(rs2);
                let new = (old as i32).min(rhs as i32) as u32;
                bus.write_u32(addr as u64, new)?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMaxW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                let rhs = self.read_reg(rs2);
                let new = (old as i32).max(rhs as i32) as u32;
                bus.write_u32(addr as u64, new)?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMinuW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                let new = old.min(self.read_reg(rs2));
                bus.write_u32(addr as u64, new)?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMaxuW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                let new = old.max(self.read_reg(rs2));
                bus.write_u32(addr as u64, new)?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
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
            if let Some(irq) = self.pfic.as_ref().and_then(QingKePfic::next_irq) {
                if !observers.is_empty() {
                    crate::emit_trace_event(
                        observers,
                        labwired_hw_trace::TraceEvent::InstructionRetired {
                            pc: retired_pc,
                            opcode,
                        },
                    );
                }
                // As below: mepc is the next instruction.
                return self.enter_pfic_interrupt(bus, irq, next_pc);
            }
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
            if self.jit_enabled
                && max_count > 1
                && !self.triggers.armed()
                && self.jit_gate_allows(bus, observers)
            {
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
        let (exact_clock, batch_start) = {
            let exact_clock =
                cfg!(feature = "event-scheduler") && config.peripheral_tick_interval > 1;
            (
                exact_clock,
                if exact_clock { bus.current_cycle() } else { 0 },
            )
        };
        // Gap #1: mid-batch arming. Clamp remaining work to the earliest pending
        // absolute deadline instead of ending on the first arm — far-future
        // timers (FreeRTOS tick, etc.) used to force ~60-insn batches and tank
        // host MIPS. We still never retire past the deadline without a drain
        // (same delivery cycle as the immediate-end policy).
        // Only scheduler builds tighten this bound. Keep one declaration so
        // adding a fast path does not multiply feature-conditioned code paths.
        #[allow(unused_mut)]
        let mut limit = max_count;
        let mut i = 0u32;
        if config.decode_cache_enabled && observers.is_empty() && tap.is_none() {
            #[cfg(feature = "event-scheduler")]
            if exact_clock {
                if let Some(deadline) = bus.earliest_pending_deadline() {
                    // Preserve the interpreter's minimum one-instruction boundary
                    // for already-due events; Machine normally drains these first.
                    limit = limit.min(
                        deadline
                            .saturating_sub(batch_start)
                            .max(1)
                            .min(u64::from(u32::MAX)) as u32,
                    );
                }
            }
            i = self.try_spin_window(bus, limit);
            if i == 0 {
                i = self.try_store_spin_window(bus, limit);
            }
            if i == 0 {
                let exact_cycle = exact_clock.then_some(batch_start);
                i = self.try_masked_poll_window(bus, limit, exact_cycle)?;
            }
            if exact_clock && i > 0 {
                // Match the last pre-instruction clock published by the loop below.
                bus.publish_cycle(batch_start + u64::from(i - 1));
            }
        }
        // A batch boundary can land on any of the four poll instructions. If
        // the entry was not the load-headed phase recognized above, interpret
        // at most three instructions and retry as each following PC comes into
        // view. Once the loop head is reached the fast path consumes the rest
        // of the batch; non-poll code pays only three cold decode probes.
        let mut masked_poll_probes = 3u8;
        let block_cache_ok = config.decode_cache_enabled && observers.is_empty() && tap.is_none();
        while i < limit {
            if i != 0 && masked_poll_probes != 0 && limit - i >= 4 {
                masked_poll_probes -= 1;
                let exact_cycle = exact_clock.then_some(batch_start + u64::from(i));
                let retired = self.try_masked_poll_window(bus, limit - i, exact_cycle)?;
                if retired != 0 {
                    i += retired;
                    if exact_clock {
                        bus.publish_cycle(batch_start + u64::from(i - 1));
                    }
                    continue;
                }
            }
            if block_cache_ok && limit - i >= 2 {
                let retired =
                    self.try_step_cached_blocks(bus, limit - i, exact_clock, batch_start, i)?;
                if retired != 0 {
                    i += retired;
                    if exact_clock {
                        bus.publish_cycle(batch_start + u64::from(i - 1));
                        if let Some(dl) = pending_schedule_deadline(bus) {
                            let max_ret = dl.saturating_sub(batch_start);
                            if (i as u64) >= max_ret {
                                return Ok(i);
                            }
                            if max_ret < u32::MAX as u64 {
                                limit = limit.min(max_ret as u32);
                            }
                        }
                    }
                    continue;
                }
            }
            if let Some(tap) = &tap {
                tap.bump_clock();
            }
            if exact_clock {
                bus.publish_cycle(batch_start + i as u64);
            }
            self.step(bus, observers, config)?;
            i += 1;
            if config.idle_fast_forward_enabled
                && self.waiting_for_interrupt
                && self.idle_fast_forward_budget(bus).is_some()
            {
                return Ok(i);
            }
            // A world's net pad changed drive: end the batch here
            // (`LogicTap::set_batch_stop_channels`).
            if tap.as_ref().is_some_and(|t| t.batch_stop_hit()) {
                return Ok(i);
            }
            if exact_clock {
                if let Some(dl) = pending_schedule_deadline(bus) {
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
    fn set_exception_pending(&mut self, exception_num: u32) {
        // Without an NVIC the bus forwards a peripheral's raw IRQ number,
        // which on a chip with a PFIC is its interrupt number and on a chip
        // with a PLIC is its source ID. Without either the
        // line pends a generic, sticky machine external interrupt.
        if let Some(pfic) = self.pfic.as_mut() {
            pfic.raise(exception_num);
        } else if let Some(plic) = self.plic.as_mut() {
            plic.raise(exception_num);
            self.refresh_meip();
        } else {
            self.mip |= 1 << 11;
        }
    }

    fn idle_fast_forward_budget(&self, bus: &dyn Bus) -> Option<u64> {
        if !self.waiting_for_interrupt {
            return None;
        }
        if ((self.mip & self.mie) | bus.external_irq_lines()) != 0 || self.pfic_ready() {
            return None;
        }
        match self.cycles_until_mtip()? {
            u64::MAX => Some(u64::MAX),
            0 | 1 => None,
            n => Some(n - 1),
        }
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
    fn invalidate_code_caches(&mut self) {
        self.invalidate_fetch_window();
        self.decode_cache.fill(None);
        #[cfg(feature = "jit")]
        if let Some(jit) = self.jit_engine.as_mut() {
            jit.invalidate_blocks();
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
            triggers: Some(self.triggers.snapshot()),
        })
    }

    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        if let crate::snapshot::CpuSnapshot::RiscV(s) = snapshot {
            self.invalidate_fetch_window();
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
            // A snapshot from before triggers were captured carries none:
            // the module comes back at reset, so the result does not depend
            // on what the machine it is applied to had programmed.
            match &s.triggers {
                Some(t) => self.triggers.restore(t),
                None => self.triggers = trigger::TriggerModule::default(),
            }
        }
    }

    fn runtime_snapshot(&self) -> Option<(crate::runtime_snapshot::CpuKind, Vec<u8>)> {
        use crate::runtime_snapshot::{RiscVRuntimeSnapshot, RiscVRuntimeSnapshotV2};
        let base = RiscVRuntimeSnapshot {
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
        let snap = RiscVRuntimeSnapshotV2 {
            base,
            triggers: self.triggers.snapshot(),
        };
        let bytes = bincode::serialize(&snap).expect("bincode serialize RiscVRuntimeSnapshot");
        Some((crate::runtime_snapshot::CpuKind::RiscV, bytes))
    }

    fn apply_runtime_snapshot(
        &mut self,
        kind: crate::runtime_snapshot::CpuKind,
        bytes: &[u8],
    ) -> SimResult<()> {
        use crate::runtime_snapshot::{CpuKind, RiscVRuntimeSnapshot, RiscVRuntimeSnapshotV2};
        if kind != CpuKind::RiscV {
            return Err(crate::SimulationError::NotImplemented(format!(
                "apply_runtime_snapshot: kind {kind:?} given to RiscV"
            )));
        }
        // Newest layout first: an older, shorter blob cannot decode as it, but
        // it decodes exactly as the bare V1 layout. A V1 blob carries no
        // trigger state, so the module comes back at reset — what a machine
        // that never programmed a trigger has.
        let (snap, triggers) = match bincode::deserialize::<RiscVRuntimeSnapshotV2>(bytes) {
            Ok(v2) => (v2.base, Some(v2.triggers)),
            Err(_) => {
                let v1: RiscVRuntimeSnapshot = bincode::deserialize(bytes).map_err(|e| {
                    crate::SimulationError::NotImplemented(format!("RiscV snapshot decode: {e}"))
                })?;
                (v1, None)
            }
        };
        match &triggers {
            Some(t) => self.triggers.restore(t),
            None => self.triggers = trigger::TriggerModule::default(),
        }
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
        self.invalidate_fetch_window();
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
#[path = "riscv_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "riscv_rv32e_tests.rs"]
mod rv32e_tests;

#[cfg(test)]
#[path = "riscv_ch32v003_tests.rs"]
mod ch32v003_tests;
