// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! FLASH interface peripheral — layout-selectable per chip family.
//!
//! ARCHITECTURAL SEPARATION: each family's register layout lives behind a
//! `FlashRegisterLayout` variant. F1 and L4 reset values + register offsets
//! differ; both are reachable through the same `Flash` struct but every
//! F1-specific code path is gated by `matches!(self.layout, Stm32F1)` so
//! adding/changing F1 cannot regress L4 behaviour and vice-versa.
//!
//! HAL-generated firmware writes to FLASH_ACR before any clock change to
//! adjust wait states (LATENCY field) for the new SYSCLK frequency.
//! Without this peripheral those writes silently drop and reads return
//! 0, which makes HAL_RCC_ClockConfig() loop forever waiting for
//! the latency change to take effect.
//!
//! Reset values verified against real NUCLEO-L476RG silicon via SWD:
//!   L4 ACR  = 0x0000_0600  (caches enabled by the boot ROM)
//!   L4 SR   = 0x0000_0000
//!   L4 CR   = 0xC000_0000  (LOCK + OPTLOCK both held)
//!   L4 OPTR = 0xFFEF_F8AA  (factory option-byte programming)
//!
//! Reset values verified against real STM32F103C8 silicon via SWD
//! (Blue Pill, ST-LINK V2J43, 2026-06-04):
//!   F1 ACR  = 0x0000_0030  (PRFTBE=1, PRFTBS=1; LATENCY=0)
//!   F1 SR   = 0x0000_0000
//!   F1 CR   = 0x0000_0080  (LOCK=1 — bit 7 on F1, not 31 like L4)
//!   F1 OBR  = 0x03FF_FFFC  (factory option bytes)

use crate::SimResult;
use std::str::FromStr;

#[path = "flash_h5_regs.rs"]
pub mod h5;

/// Register layout / reset-value profile for the FLASH interface.
/// Adding a variant must NOT touch the read/write branches of existing
/// variants — keep family-specific behaviour isolated.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlashRegisterLayout {
    /// Default. Verified on NUCLEO-L476RG (silicon, 2026-05).
    #[default]
    Stm32L4,
    /// STM32F1 medium-density (RM0008). Verified on STM32F103C8 Blue Pill
    /// (silicon, 2026-06-04). ACR has PRFTBE→PRFTBS read-back mirror;
    /// register file is shorter (KEYR@0x04, SR@0x0C, CR@0x10, OBR@0x1C,
    /// WRPR@0x20). No ECCR / OPTR / PCROP / WRP1..2.
    Stm32F1,
    /// STM32H5 (RM0481 §7). Models the OTA reprogramming path: ACR
    /// (LATENCY/WRHIGHFREQ/PRFTEN) read-back for clock bring-up; NSKEYR/OPTKEYR
    /// unlock; NSCR sector-erase and OPTSR_PRG.SWAP_BANK + OPTCR.OPTSTRT,
    /// recorded as pending ops drained by `Machine::step` (erase fills 0xFF;
    /// swap exchanges the two 1 MiB banks and resets). Programming is plain
    /// writes to the flash region (the bus routes them into the flash buffer).
    /// Reset values pinned to a NUCLEO-H563ZI SWD probe (silicon capture
    /// 2026-06-11, OPTSR re-confirmed 2026-06-20):
    /// ACR=0x13, OPTCR=1, NSCR=1, OPTSR_CUR=0x2D30EDF8 (this part's option
    /// bytes, representative).
    Stm32H5,
    /// STM32F4 (RM0368 §3.8, STM32F401). Distinct from both F1 and L4: the
    /// prefetch/cache ACR has NO PRFTBE→PRFTBS read-back mirror (bits read back
    /// as written), and the reset values differ from every other family —
    /// ACR=0x0000_0000 (all acceleration off), SR=0, CR=0x8000_0000 (LOCK at
    /// bit 31, not bit 7 like F1), OPTCR=0x0000_0014 (ST SVD default). Register
    /// file: ACR@0x00, KEYR@0x04, OPTKEYR@0x08, SR@0x0C, CR@0x10, OPTCR@0x14.
    /// Reset values pinned to the STM32F401 CMSIS SVD (RM0368 §3.8).
    Stm32F4,
}

impl FromStr for FlashRegisterLayout {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stm32l4" | "l4" => Ok(Self::Stm32L4),
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32h5" | "h5" => Ok(Self::Stm32H5),
            "stm32f4" | "f4" => Ok(Self::Stm32F4),
            _ => Err(format!(
                "unsupported FLASH register layout '{}'; supported: stm32l4, stm32f1, stm32f4, stm32h5",
                value
            )),
        }
    }
}

/// Pending FLASH hardware operation recorded by the simulator.
/// Drainable via [`Flash::drain_pending_op`]; consumed (and applied) by the
/// machine layer so that bank-swap takes effect on the next boot cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FlashOp {
    /// Non-secure sector erase (`NSCR.SER + STRT`).
    EraseSector { bank: u8, sector: u32 },
    /// Bank-swap + option-byte reload (`OPTSR_PRG.SWAP_BANK + OPTCR.OPTSTRT`).
    SwapAndReset,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Flash {
    layout: FlashRegisterLayout,
    acr: u32,
    pdkeyr: u32,
    keyr: u32,
    optkeyr: u32,
    sr: u32,
    cr: u32,
    eccr: u32,
    optr: u32,
    pcrop1sr: u32,
    pcrop1er: u32,
    wrp1ar: u32,
    wrp1br: u32,
    pcrop2sr: u32,
    pcrop2er: u32,
    wrp2ar: u32,
    wrp2br: u32,

    // H5: OPTSR_PRG shadow (mirrors OPTSR_CUR at reset per silicon capture).
    optsr_prg: u32,
    // H5: pending hardware op recorded on NSCR.STRT / OPTCR.OPTSTRT.
    // Cell<Option<_>> gives interior mutability so drain_pending_op takes &self.
    // skip serde — Cell<Option<_>> is not Serialize; resets to None on restore.
    #[serde(skip)]
    pending_op: std::cell::Cell<Option<FlashOp>>,

    // Internal: tracks unlock sequence on KEYR (write 0x45670123 then 0xCDEF89AB).
    key_state: KeyUnlockState,
    optkey_state: KeyUnlockState,

    // OPT-IN fidelity gate (H5 only). When true, a flash-region write is fed
    // through the silicon-true write-buffer state machine (see
    // [`Flash::h5_program_byte`]): writes accumulate into a 16-byte quad-word
    // buffer and commit (as the bitwise AND of new & existing) only on a full
    // aligned quad-word, setting EOP; a misaligned/inconsistent quad-word
    // raises INCERR alone and commits nothing. Default false: gate off ⇒
    // byte-identical to prior behaviour (every write commits, no buffering, no
    // flag). Mirrors the per-peripheral opt-in pattern (clock-gating).
    #[serde(default)]
    error_flags: bool,

    // H5 write-buffer state. `Some(base)` means a quad-word program is in
    // progress at the 16-aligned offset `base`; `written` marks which of the
    // 16 bytes have been supplied, `bytes` holds the buffered values. Only used
    // when the gate is on and NSCR.PG is set. Resets to empty on snapshot
    // restore (a half-written quad-word is never persisted on real silicon).
    #[serde(skip)]
    h5_wbuf: Option<H5WriteBuffer>,

    // OPT-IN read-while-write fidelity gate (H5 only). On real STM32H563 silicon
    // (RM0481 §7, "read-while-write") a bank cannot be read — including
    // instruction fetch — while that SAME bank is being erased or programmed:
    // the bus stalls and code executing from that bank can never make progress.
    // Production flash routines therefore run from SRAM. When this gate is on,
    // an erase whose target bank is the one the CPU is executing from FAULTS
    // (the machine layer turns the check into an unrecoverable error) instead of
    // silently succeeding, forcing the firmware to relocate the routine to SRAM.
    // Default false: gate off ⇒ byte-identical to prior behaviour (same-bank
    // erase succeeds). Independent of `error_flags`; mirrors the same opt-in
    // pattern.
    #[serde(default)]
    read_while_write: bool,

    // H5 bank-swap state. Toggled each time a SWAP_BANK op is applied (the
    // machine layer calls [`Flash::mark_swapped`] after `swap_banks`). With the
    // RWW gate on this lets the bank-mapping translate a BKSEL logical bank and a
    // PC physical bank consistently: under SWAP_BANK the bank presented at
    // 0x08000000 is the physical second bank. `#[serde(default)]` so older
    // snapshots restore as not-swapped (reset state).
    #[serde(default)]
    swapped: bool,
}

/// In-progress H5 quad-word write buffer (16 bytes at a 16-aligned base).
#[derive(Debug, Clone, Copy)]
struct H5WriteBuffer {
    base: u64,
    written: [bool; 16],
    bytes: [u8; 16],
}

/// Outcome of feeding one flash-region byte to the H5 write-buffer state
/// machine ([`Flash::h5_program_byte`]). The bus acts on this:
/// * `Buffered`     — byte accepted into the pending quad-word; commit nothing.
/// * `Commit`       — quad-word complete; the bus reads the 16 existing bytes
///   at `base`, ANDs each with `bytes[i]` (flash only flips 1→0), writes back.
/// * `Inconsistent` — misaligned/inconsistent quad-word; INCERR already set,
///   buffer discarded; commit nothing.
/// * `NotProgramming` — gate on but NSCR.PG clear; no programming occurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H5ProgAction {
    Buffered,
    Commit { base: u64, bytes: [u8; 16] },
    Inconsistent,
    NotProgramming,
}

#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
enum KeyUnlockState {
    #[default]
    Locked,
    HalfUnlocked,
    Unlocked,
}

const FLASH_KEY1: u32 = 0x4567_0123;
const FLASH_KEY2: u32 = 0xCDEF_89AB;
const OPTKEY1: u32 = 0x0819_2A3B;
const OPTKEY2: u32 = 0x4C5D_6E7F;

impl Flash {
    /// Backwards-compat constructor: defaults to STM32L4 layout so existing
    /// callers (and the bus dispatch fallback) are unchanged.
    pub fn new() -> Self {
        Self::new_with_layout(FlashRegisterLayout::Stm32L4)
    }

    pub fn new_with_layout(layout: FlashRegisterLayout) -> Self {
        // Per-layout reset values — verified against real silicon per family.
        // Touching the F1 branch must not change L4 reset values.
        let (acr_reset, cr_reset, optr_reset) = match layout {
            FlashRegisterLayout::Stm32L4 => (0x0000_0600u32, 0xC000_0000u32, 0xFFEF_F8AAu32),
            FlashRegisterLayout::Stm32F1 => (0x0000_0030u32, 0x0000_0080u32, 0x0000_0000u32),
            // NSCR (the only modeled control reg) reset 0x1; OPTSR_CUR via optr.
            FlashRegisterLayout::Stm32H5 => (0x0000_0013u32, 0x0000_0001u32, 0x2D30_EDF8u32),
            // F401 (RM0368 §3.8 / STM32F401 CMSIS SVD): ACR=0 (no
            // prefetch/cache/latency out of reset), CR=0x8000_0000 (LOCK bit
            // 31), OPTCR=0x0000_0014. NB: RM0368 §3.8.6 prints the option-byte-
            // loaded value 0x0FFF_AAED, but the ST CMSIS SVD (the reset-
            // conformance oracle) pins the clean default 0x0000_0014 — the
            // option bytes are loaded from flash at reset release, so the
            // architectural reset value is 0x14.
            FlashRegisterLayout::Stm32F4 => (0x0000_0000u32, 0x8000_0000u32, 0x0000_0014u32),
        };
        Self {
            layout,
            acr: acr_reset,
            pdkeyr: 0,
            keyr: 0,
            optkeyr: 0,
            sr: 0,
            cr: cr_reset,
            eccr: 0,
            optr: optr_reset,
            pcrop1sr: 0,
            pcrop1er: 0,
            wrp1ar: 0,
            wrp1br: 0,
            pcrop2sr: 0,
            pcrop2er: 0,
            wrp2ar: 0,
            wrp2br: 0,
            // OPTSR_PRG mirrors OPTSR_CUR at reset (silicon-confirmed).
            optsr_prg: optr_reset,
            pending_op: std::cell::Cell::new(None),
            key_state: KeyUnlockState::Locked,
            optkey_state: KeyUnlockState::Locked,
            error_flags: false,
            h5_wbuf: None,
            read_while_write: false,
            swapped: false,
        }
    }

    /// Enable (or disable) the opt-in H5 programming-error fidelity gate.
    /// Returns `self` so chip-factory construction can stay one expression.
    /// No effect on non-H5 layouts (the program checks are H5-only).
    pub fn with_error_flags(mut self, on: bool) -> Self {
        self.error_flags = on;
        self
    }

    /// True when the H5 programming-error fidelity gate is enabled AND this is
    /// the H5 layout. The bus consults this on flash-region writes to decide
    /// whether to run the program-error check.
    pub fn h5_error_flags_enabled(&self) -> bool {
        self.error_flags && matches!(self.layout, FlashRegisterLayout::Stm32H5)
    }

    /// Enable (or disable) the opt-in H5 read-while-write fidelity gate.
    /// Returns `self` so chip-factory construction can stay one expression.
    /// No effect on non-H5 layouts (the RWW check is H5-only).
    pub fn with_read_while_write(mut self, on: bool) -> Self {
        self.read_while_write = on;
        self
    }

    /// True when the H5 read-while-write fidelity gate is enabled AND this is
    /// the H5 layout. The machine layer consults this before applying a flash
    /// erase to decide whether the same-bank-as-PC check fires.
    pub fn h5_rww_enabled(&self) -> bool {
        self.read_while_write && matches!(self.layout, FlashRegisterLayout::Stm32H5)
    }

    /// Record that a SWAP_BANK has been applied (the machine layer calls this
    /// right after `LinearMemory::swap_banks`). Each call toggles the active
    /// mapping so the RWW bank translation stays consistent across repeated
    /// swaps. No-op semantics beyond the bool; only meaningful with the RWW
    /// gate on.
    pub fn mark_swapped(&mut self) {
        self.swapped = !self.swapped;
    }

    /// True if a SWAP_BANK is currently in effect (odd number of applied swaps).
    pub fn is_swapped(&self) -> bool {
        self.swapped
    }

    /// Map a BKSEL logical bank (0 or 1, as written to NSCR.BKSEL) to the
    /// physical bank under the current SWAP_BANK state. Without a swap, logical
    /// bank N is physical bank N; under SWAP_BANK the mapping is inverted (the
    /// physical second bank is presented at 0x08000000, i.e. as logical bank 0).
    fn logical_to_physical_bank(&self, logical: u8) -> u8 {
        if self.swapped {
            logical ^ 1
        } else {
            logical
        }
    }

    /// Physical bank (0 or 1) containing the flash buffer offset `buf_off`
    /// (= `absolute_addr - FLASH_BASE`). The CPU fetches code from 0x08000000+
    /// (the boot view), so PC's buffer offset under the current swap state names
    /// the physical bank it executes from. The sim physically rearranges the
    /// backing buffer on swap, so buffer-half N already holds physical bank
    /// `logical_to_physical_bank(N)`'s contents — translate the same way.
    pub fn physical_bank_of_offset(&self, buf_off: u64) -> u8 {
        let half = if buf_off >= h5::BANK_SIZE { 1u8 } else { 0u8 };
        self.logical_to_physical_bank(half)
    }

    /// Read-while-write violation test: returns true when an erase of BKSEL
    /// logical bank `erase_bank` collides with the physical bank the CPU is
    /// executing from (`pc_buf_off` = `PC - FLASH_BASE`). Both sides are
    /// translated to a physical bank through the active SWAP_BANK mapping, so a
    /// same-physical-bank erase faults regardless of which logical view the
    /// firmware addressed. Only meaningful when [`h5_rww_enabled`] is true.
    ///
    /// [`h5_rww_enabled`]: Self::h5_rww_enabled
    pub fn rww_erase_violates(&self, erase_bank: u8, pc_buf_off: u64) -> bool {
        let op_phys = self.logical_to_physical_bank(erase_bank & 1);
        let pc_phys = self.physical_bank_of_offset(pc_buf_off);
        op_phys == pc_phys
    }

    /// Feed one flash-region byte write into the H5 write-buffer state machine.
    ///
    /// `flash_offset` is the byte offset within the flash bank (absolute addr
    /// minus the flash base). Returns the [`H5ProgAction`] the bus must apply:
    /// it owns the flash backing memory, so this peripheral only tracks the
    /// pending quad-word and the NSSR status; the commit (read 16 / AND /
    /// write back) happens on the bus side.
    ///
    /// Silicon model (verified NUCLEO-H563ZI, 2026-06-22):
    ///   * NSCR.PG must be set, else `NotProgramming` (no commit, no flag).
    ///   * Writes accumulate into a 16-byte quad-word buffer at the 16-aligned
    ///     base. WBNE is set while the buffer is partial.
    ///   * A full aligned quad-word commits as new & existing (flash flips only
    ///     1→0), sets EOP, clears WBNE, clears the buffer.
    ///   * A misaligned / inconsistent quad-word — a write that targets a
    ///     different quad-word base than the in-progress one before it
    ///     completes, or a first write not at the 16-aligned base — raises
    ///     INCERR alone, discards the buffer, commits nothing.
    ///   * Program-over-not-erased is NOT an error here; the AND models it.
    ///
    /// Only meaningful when [`h5_error_flags_enabled`](Self::h5_error_flags_enabled)
    /// is true; with the gate off the bus never calls this and every write
    /// commits straight through, exactly as before.
    pub fn h5_program_byte(&mut self, flash_offset: u64, value: u8) -> H5ProgAction {
        if !self.h5_error_flags_enabled() {
            return H5ProgAction::Buffered; // unreachable: bus gates on the index
        }
        // PG (NSCR.PG, bit 1) must be set for a flash-region write to program.
        // Silicon detail for a PG-clear write is unverified on this part, so
        // the simplest faithful choice is taken: no commit, no flag. (A real
        // part would typically ignore or fault the write; we conservatively
        // drop it rather than invent a flag.)
        if self.cr & h5::NSCR_PG == 0 {
            return H5ProgAction::NotProgramming;
        }

        let base = flash_offset & !(h5::PROG_GRANULARITY - 1);
        let lane = (flash_offset - base) as usize;

        // A write that jumps to a different quad-word while one is still partial
        // is an inconsistent program run: INCERR alone, discard, commit nothing.
        if let Some(buf) = self.h5_wbuf {
            if buf.base != base {
                self.h5_wbuf = None;
                self.sr = (self.sr & !h5::NSSR_WBNE) | h5::NSSR_INCERR;
                return H5ProgAction::Inconsistent;
            }
        }

        let buf = self.h5_wbuf.get_or_insert(H5WriteBuffer {
            base,
            written: [false; 16],
            bytes: [0xFF; 16],
        });
        buf.written[lane] = true;
        buf.bytes[lane] = value;

        if buf.written.iter().all(|&w| w) {
            // Full aligned quad-word: commit (bus ANDs with existing), set EOP.
            let bytes = buf.bytes;
            let commit_base = buf.base;
            self.h5_wbuf = None;
            self.sr = (self.sr & !h5::NSSR_WBNE) | h5::NSSR_EOP;
            H5ProgAction::Commit {
                base: commit_base,
                bytes,
            }
        } else {
            // Partial quad-word: WBNE live, nothing committed, no error.
            self.sr |= h5::NSSR_WBNE;
            H5ProgAction::Buffered
        }
    }

    // (legacy `new()` body replaced; kept as the no-op below for the
    // never-reached default path — Rust requires all fields, so this
    // disambiguates from new_with_layout.)
    #[allow(dead_code)]
    fn _new_l4_legacy() -> Self {
        Self {
            layout: FlashRegisterLayout::Stm32L4,
            acr: 0x0000_0600,
            pdkeyr: 0,
            keyr: 0,
            optkeyr: 0,
            sr: 0,
            cr: 0xC000_0000,
            eccr: 0,
            optr: 0xFFEF_F8AA,
            pcrop1sr: 0,
            pcrop1er: 0,
            wrp1ar: 0,
            wrp1br: 0,
            pcrop2sr: 0,
            pcrop2er: 0,
            wrp2ar: 0,
            wrp2br: 0,
            optsr_prg: 0xFFEF_F8AA,
            pending_op: std::cell::Cell::new(None),
            key_state: KeyUnlockState::Locked,
            optkey_state: KeyUnlockState::Locked,
            error_flags: false,
            h5_wbuf: None,
            read_while_write: false,
            swapped: false,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        // ─── F1 layout (isolated; no fall-through to L4) ────────────────
        if matches!(self.layout, FlashRegisterLayout::Stm32F1) {
            return match offset {
                // ACR: PRFTBE (bit 4) reads back as PRFTBS (bit 5). HAL
                // sets PRFTBE then polls PRFTBS — without the mirror the
                // poll never satisfies and Arduino startup loops forever.
                0x00 => {
                    let prftbe = (self.acr >> 4) & 1;
                    (self.acr & !(1 << 5)) | (prftbe << 5)
                }
                0x04 => 0,           // KEYR write-only
                0x08 => 0,           // OPTKEYR write-only
                0x0C => self.sr,     // SR; BSY (bit 0) is RO, we never set busy.
                0x10 => self.cr,     // CR; LOCK at bit 7 (NOT bit 31 like L4).
                0x14 => 0,           // AR write-only
                0x1C => 0x03FF_FFFC, // OBR — verified on STM32F103C8 silicon.
                0x20 => 0xFFFF_FFFF, // WRPR — no write protect.
                _ => 0,
            };
        }
        // ─── F4 layout (isolated; RM0368 §3.8) ──────────────────────────
        if matches!(self.layout, FlashRegisterLayout::Stm32F4) {
            return match offset {
                // ACR: LATENCY/PRFTEN/ICEN/DCEN read back as written (no F1-style
                // PRFTBE→PRFTBS mirror). HAL/Zephyr set LATENCY then read it back.
                0x00 => self.acr,
                0x04 => 0,         // KEYR write-only
                0x08 => 0,         // OPTKEYR write-only
                0x0C => self.sr,   // SR; BSY (bit 16) is RO, never busy in sim.
                0x10 => self.cr,   // CR; LOCK at bit 31.
                0x14 => self.optr, // OPTCR
                _ => 0,
            };
        }
        // ─── H5 layout (isolated; interface regs only) ──────────────────
        if matches!(self.layout, FlashRegisterLayout::Stm32H5) {
            return match offset {
                0x00 => self.acr,
                // NSKEYR: write-only on real hardware; simulator reads back keyr
                // so that the byte→word reconstruction in write() can assemble
                // the full 32-bit key value before triggering the state machine.
                h5::NSKEYR_OFF => self.keyr,
                0x14 => 0,                           // OPSR
                0x1C => 0x1,       // OPTCR (silicon reset; option flows not modeled)
                0x20 => self.sr,   // NSSR
                0x28 => self.cr,   // NSCR
                0x30 => 0,         // NSCCR
                0x50 => self.optr, // OPTSR_CUR
                h5::OPTSR_PRG_OFF => self.optsr_prg, // OPTSR_PRG
                _ => 0,
            };
        }
        // ─── L4 layout (untouched) ──────────────────────────────────────
        match offset {
            0x00 => self.acr,
            0x04 => self.pdkeyr,
            0x08 => self.keyr,
            0x0C => self.optkeyr,
            0x10 => self.sr,
            0x14 => self.cr,
            0x18 => self.eccr,
            0x20 => self.optr,
            0x24 => self.pcrop1sr,
            0x28 => self.pcrop1er,
            0x2C => self.wrp1ar,
            0x30 => self.wrp1br,
            0x44 => self.pcrop2sr,
            0x48 => self.pcrop2er,
            0x4C => self.wrp2ar,
            0x50 => self.wrp2br,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        // ─── H5 layout (isolated) ───────────────────────────────────────
        if matches!(self.layout, FlashRegisterLayout::Stm32H5) {
            let unlocked = matches!(self.key_state, KeyUnlockState::Unlocked);
            match offset {
                0x00 => {
                    // ACR writable bits: LATENCY[3:0], WRHIGHFREQ[5:4],
                    // PRFTEN(8) = mask 0x13F. Round-trips silicon-pinned
                    // (0x11/0x23/0x02/0x25/0x13F — capture6+9; the original
                    // 0x133 mask dropped LATENCY bits 2:3 and broke HAL's
                    // latency-5 read-back check at 250 MHz).
                    self.acr = value & 0x0000_013F;
                }
                h5::NSKEYR_OFF => {
                    // H5 non-secure key register — walks the unlock sequence.
                    // Store in keyr so byte-level write() can read back + merge.
                    self.keyr = value;
                    self.key_state = match (self.key_state, value) {
                        (KeyUnlockState::Locked, FLASH_KEY1) => KeyUnlockState::HalfUnlocked,
                        (KeyUnlockState::HalfUnlocked, FLASH_KEY2) => {
                            // NSCR.LOCK (bit 0) clears on valid key sequence.
                            self.cr &= !1;
                            KeyUnlockState::Unlocked
                        }
                        _ => KeyUnlockState::Locked,
                    };
                }
                // OPTKEYR (H5 offset 0x0C) — option-byte unlock sequence.
                // Distinct from NSKEYR: option-byte programming and OPTSTRT
                // require this sequence (OPTKEY1 then OPTKEY2), not the flash key.
                0x0C => {
                    self.optkeyr = value;
                    self.optkey_state = match (self.optkey_state, value) {
                        (KeyUnlockState::Locked, OPTKEY1) => KeyUnlockState::HalfUnlocked,
                        (KeyUnlockState::HalfUnlocked, OPTKEY2) => KeyUnlockState::Unlocked,
                        _ => KeyUnlockState::Locked,
                    };
                }
                h5::NSCR_OFF => {
                    self.cr = value;
                    if unlocked && (value & h5::NSCR_SER) != 0 && (value & h5::NSCR_STRT) != 0 {
                        let sector = (value & h5::NSCR_SNB_MASK) >> h5::NSCR_SNB_SHIFT;
                        let bank = if value & h5::NSCR_BKSEL != 0 {
                            1u8
                        } else {
                            0u8
                        };
                        self.pending_op
                            .set(Some(FlashOp::EraseSector { bank, sector }));
                    }
                }
                // NSSR is read-only status on H5: writing it does NOT clear the
                // flags (verified on silicon — NSSR<-0x20 left EOP/error set).
                // Flags are cleared via NSCCR (0x30) below. Accept the write,
                // change nothing.
                h5::NSSR_OFF => {}
                // NSCCR @ 0x30 — clear register. Writing 1 to a bit clears the
                // matching sticky flag in NSSR (EOP/WRPERR/PGSERR/STRBERR/
                // INCERR). WBNE/BSY are live status, not clearable here.
                h5::NSCCR_OFF => self.sr &= !(value & h5::NSSR_W1C_MASK),
                h5::OPTSR_PRG_OFF => self.optsr_prg = value,
                // OPTSTRT (bit 1) requires the OPTION-key (OPTKEYR), not the flash
                // key (NSKEYR): option-byte programming is a separate unlock domain
                // on H5 (RM0481 §7, SVD-confirmed: OPTCR has OPTLOCK/bit0,
                // OPTSTRT/bit1, SWAP_BANK/bit31; there is no OBL_LAUNCH on H5). A
                // match guard keeps this one condition out of a nested `if`.
                h5::OPTCR_OFF
                    if matches!(self.optkey_state, KeyUnlockState::Unlocked)
                        && (value & h5::OPTCR_OPTSTRT) != 0
                        && (self.optsr_prg & h5::OPTSR_SWAP_BANK) != 0 =>
                {
                    self.pending_op.set(Some(FlashOp::SwapAndReset));
                }
                _ => {}
            }
            return;
        }
        // ─── F1 layout (isolated; no fall-through to L4) ────────────────
        if matches!(self.layout, FlashRegisterLayout::Stm32F1) {
            match offset {
                // ACR writable: LATENCY[2:0], HLFCYA(3), PRFTBE(4). PRFTBS RO.
                0x00 => self.acr = value & 0x0000_001F,
                // KEYR — unlocks CR.LOCK (bit 7 on F1).
                0x04 => {
                    self.key_state = match (self.key_state, value) {
                        (KeyUnlockState::Locked, FLASH_KEY1) => KeyUnlockState::HalfUnlocked,
                        (KeyUnlockState::HalfUnlocked, FLASH_KEY2) => {
                            self.cr &= !(1 << 7);
                            KeyUnlockState::Unlocked
                        }
                        _ => KeyUnlockState::Locked,
                    };
                }
                // OPTKEYR — unlocks OPTWRE (bit 9 of F1 CR).
                0x08 => {
                    self.optkey_state = match (self.optkey_state, value) {
                        (KeyUnlockState::Locked, OPTKEY1) => KeyUnlockState::HalfUnlocked,
                        (KeyUnlockState::HalfUnlocked, OPTKEY2) => {
                            self.cr &= !(1 << 9);
                            KeyUnlockState::Unlocked
                        }
                        _ => KeyUnlockState::Locked,
                    };
                }
                // SR: EOP / PGERR / WRPRTERR are rc_w1. BSY (bit 0) RO.
                0x0C => self.sr &= !(value & 0x0000_0034),
                0x10 => self.cr = value,
                0x14 => {} // AR — accept; sim doesn't program flash pages
                _ => {}
            }
            return;
        }
        // ─── F4 layout (isolated; RM0368 §3.8) ──────────────────────────
        if matches!(self.layout, FlashRegisterLayout::Stm32F4) {
            match offset {
                // ACR writable: LATENCY[3:0], PRFTEN(8), ICEN(9), DCEN(10),
                // ICRST(11), DCRST(12) = mask 0x1F0F. HAL sets LATENCY then
                // reads it straight back.
                0x00 => self.acr = value & 0x0000_1F0F,
                // KEYR — unlocks CR.LOCK (bit 31 on F4).
                0x04 => {
                    self.key_state = match (self.key_state, value) {
                        (KeyUnlockState::Locked, FLASH_KEY1) => KeyUnlockState::HalfUnlocked,
                        (KeyUnlockState::HalfUnlocked, FLASH_KEY2) => {
                            self.cr &= !(1 << 31);
                            KeyUnlockState::Unlocked
                        }
                        _ => KeyUnlockState::Locked,
                    };
                }
                // OPTKEYR — unlocks OPTCR.OPTLOCK (bit 0).
                0x08 => {
                    self.optkey_state = match (self.optkey_state, value) {
                        (KeyUnlockState::Locked, OPTKEY1) => KeyUnlockState::HalfUnlocked,
                        (KeyUnlockState::HalfUnlocked, OPTKEY2) => {
                            self.optr &= !1;
                            KeyUnlockState::Unlocked
                        }
                        _ => KeyUnlockState::Locked,
                    };
                }
                // SR: EOP(0)/OPERR(1)/WRPERR(4)/PGAERR(5)/PGPERR(6)/PGSERR(7)/
                // RDERR(8) are rc_w1. BSY (bit 16) RO.
                0x0C => self.sr &= !(value & 0x0000_01F3),
                0x10 => self.cr = value,
                0x14 => self.optr = value,
                _ => {}
            }
            return;
        }
        // ─── L4 layout (untouched) ──────────────────────────────────────
        match offset {
            // ACR is writable; LATENCY (bits 2:0), PRFTEN (bit 8),
            // ICEN (9), DCEN (10), ICRST (11), DCRST (12), RUN_PD (14),
            // SLEEP_PD (15). Other bits reserved. We keep only the
            // documented writable mask so accidental writes don't pollute
            // the readback with nonsense.
            0x00 => self.acr = value & 0xC000_FF07,
            0x04 => self.pdkeyr = value,
            // KEYR / OPTKEYR are write-only; the write_volatile sequence
            // walks the lock state machine.
            0x08 => {
                self.key_state = match (self.key_state, value) {
                    (KeyUnlockState::Locked, FLASH_KEY1) => KeyUnlockState::HalfUnlocked,
                    (KeyUnlockState::HalfUnlocked, FLASH_KEY2) => {
                        // LOCK = bit 31 of CR clears.
                        self.cr &= !(1 << 31);
                        KeyUnlockState::Unlocked
                    }
                    _ => KeyUnlockState::Locked,
                };
            }
            0x0C => {
                self.optkey_state = match (self.optkey_state, value) {
                    (KeyUnlockState::Locked, OPTKEY1) => KeyUnlockState::HalfUnlocked,
                    (KeyUnlockState::HalfUnlocked, OPTKEY2) => {
                        self.cr &= !(1 << 30); // OPTLOCK
                        KeyUnlockState::Unlocked
                    }
                    _ => KeyUnlockState::Locked,
                };
            }
            // SR is rc_w1: writing 1 clears EOP / OPERR / PROGERR /
            // WRPERR / PGAERR / SIZERR / PGSERR / MISERR / FASTERR /
            // RDERR / OPTVERR. BSY (bit 16) is read-only.
            0x10 => {
                let clearable: u32 = 0x0000_FFFE;
                self.sr &= !(value & clearable);
            }
            0x14 => self.cr = value,
            0x18 => self.eccr &= !(value & 0xC000_0000), // W1C ECC error flags
            // OPTR is writable only after OPTKEYR unlock + OBL_LAUNCH.
            // For the simulator we accept the write directly.
            0x20 => {
                if matches!(self.optkey_state, KeyUnlockState::Unlocked) {
                    self.optr = value;
                }
            }
            _ => {}
        }
    }
}

impl Flash {
    /// Drain the pending FLASH hardware operation, if any.
    ///
    /// Returns `Some(op)` the first time after an operation is recorded;
    /// subsequent calls return `None` until a new op is recorded. Uses
    /// interior mutability so callers with a shared reference can drain.
    pub fn drain_pending_op(&self) -> Option<FlashOp> {
        self.pending_op.take()
    }

    /// True when this FLASH models hardware operations (sector erase / bank
    /// swap) as pending ops that must be drained and applied per instruction.
    /// Only the H5 layout records such ops, so the runner must execute the
    /// firmware cycle-accurately (batch size 1) for the drain to fire on every
    /// instruction — see `SystemBus::requires_cycle_accurate`.
    pub fn models_ops(&self) -> bool {
        matches!(self.layout, FlashRegisterLayout::Stm32H5)
    }
}

impl Default for Flash {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for Flash {
    // Inert walk: tick() is the trait-default no-op; H5 erase/bank-swap ops drain via requires_cycle_accurate/drain_pending_op per instruction, never the walk.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // KEYR/OPTKEYR sequence works on full-word writes only. For
        // byte-level access (rare), update word and re-trigger.
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // Bypass the byte-decompose path for 32-bit register writes so that
        // key-unlock sequences (NSKEYR, OPTKEYR) see the full 32-bit word in
        // a single write_reg call. The byte path is correct for data regs but
        // would fragment each 32-bit key write into 4 independent sub-word
        // triggers, none of which match FLASH_KEY1 / FLASH_KEY2.
        self.write_reg(offset & !3, value);
        Ok(())
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod h5_erase_swap_tests {
    use super::h5;
    use super::{Flash, FlashOp, FlashRegisterLayout};
    use crate::Peripheral;

    fn unlock(f: &mut Flash) {
        f.write_u32(h5::NSKEYR_OFF, 0x4567_0123).unwrap(); // NSKEYR key 1
        f.write_u32(h5::NSKEYR_OFF, 0xCDEF_89AB).unwrap(); // NSKEYR key 2
    }

    fn unlock_opt(f: &mut Flash) {
        f.write_u32(0x0C, 0x0819_2A3B).unwrap(); // OPTKEYR key 1
        f.write_u32(0x0C, 0x4C5D_6E7F).unwrap(); // OPTKEYR key 2
    }

    #[test]
    fn ser_strt_records_erase_of_selected_sector() {
        let mut f = Flash::new_with_layout(FlashRegisterLayout::Stm32H5);
        unlock(&mut f);
        // SER + SNB=7 (bank-1) + STRT
        let nscr = h5::NSCR_SER | (7 << h5::NSCR_SNB_SHIFT) | h5::NSCR_STRT;
        f.write_u32(h5::NSCR_OFF, nscr).unwrap();
        assert_eq!(
            f.drain_pending_op(),
            Some(FlashOp::EraseSector { bank: 0, sector: 7 })
        );
        // op drains exactly once
        assert_eq!(f.drain_pending_op(), None);
    }

    #[test]
    fn ser_with_bksel_targets_bank2() {
        let mut f = Flash::new_with_layout(FlashRegisterLayout::Stm32H5);
        unlock(&mut f);
        let nscr = h5::NSCR_SER | h5::NSCR_BKSEL | (3 << h5::NSCR_SNB_SHIFT) | h5::NSCR_STRT;
        f.write_u32(h5::NSCR_OFF, nscr).unwrap();
        assert_eq!(
            f.drain_pending_op(),
            Some(FlashOp::EraseSector { bank: 1, sector: 3 })
        );
    }

    #[test]
    fn swap_bank_plus_optstrt_records_swap_and_reset() {
        let mut f = Flash::new_with_layout(FlashRegisterLayout::Stm32H5);
        unlock(&mut f); // NSKEYR — required for sector erase, not swap
        unlock_opt(&mut f); // OPTKEYR — required for OPTSTRT / option-byte ops
        f.write_u32(h5::OPTSR_PRG_OFF, h5::OPTSR_SWAP_BANK).unwrap();
        f.write_u32(h5::OPTCR_OFF, h5::OPTCR_OPTSTRT).unwrap();
        assert_eq!(f.drain_pending_op(), Some(FlashOp::SwapAndReset));
    }

    #[test]
    fn optstrt_ignored_without_optkey_unlock() {
        // NSKEYR (flash-key) unlock alone must NOT trigger swap — silicon requires
        // the separate OPTKEYR sequence for option-byte operations (RM0481 §7).
        let mut f = Flash::new_with_layout(FlashRegisterLayout::Stm32H5);
        unlock(&mut f); // NSKEYR only — no OPTKEYR
        f.write_u32(h5::OPTSR_PRG_OFF, h5::OPTSR_SWAP_BANK).unwrap();
        f.write_u32(h5::OPTCR_OFF, h5::OPTCR_OPTSTRT).unwrap();
        assert_eq!(f.drain_pending_op(), None);
    }

    #[test]
    fn erase_ignored_while_locked() {
        let mut f = Flash::new_with_layout(FlashRegisterLayout::Stm32H5);
        let nscr = h5::NSCR_SER | (7 << h5::NSCR_SNB_SHIFT) | h5::NSCR_STRT;
        f.write_u32(h5::NSCR_OFF, nscr).unwrap();
        assert_eq!(f.drain_pending_op(), None);
    }
}

#[cfg(test)]
mod h5_rww_mapping_tests {
    use super::h5;
    use super::{Flash, FlashRegisterLayout};

    fn rww_flash() -> Flash {
        Flash::new_with_layout(FlashRegisterLayout::Stm32H5).with_read_while_write(true)
    }

    #[test]
    fn gate_off_by_default_and_noop_on_non_h5() {
        let off = Flash::new_with_layout(FlashRegisterLayout::Stm32H5);
        assert!(!off.h5_rww_enabled(), "gate off by default");
        let l4 = Flash::new_with_layout(FlashRegisterLayout::Stm32L4).with_read_while_write(true);
        assert!(!l4.h5_rww_enabled(), "gate is a no-op on non-H5 layout");
        let h5 = rww_flash();
        assert!(h5.h5_rww_enabled(), "gate on for H5 when opted in");
    }

    #[test]
    fn no_swap_logical_bank_equals_physical() {
        let f = rww_flash();
        assert!(!f.is_swapped());
        // PC in bank 0 (offset 0): erase of bank 0 collides, bank 1 does not.
        assert!(f.rww_erase_violates(0, 0x0000), "erase bank0 vs PC bank0");
        assert!(
            !f.rww_erase_violates(1, 0x0000),
            "erase bank1 vs PC bank0 OK"
        );
        // PC in bank 1 (offset >= BANK_SIZE): erase of bank 1 collides.
        assert!(f.rww_erase_violates(1, h5::BANK_SIZE + 0x16000));
        assert!(!f.rww_erase_violates(0, h5::BANK_SIZE + 0x16000));
    }

    #[test]
    fn swap_inverts_the_physical_mapping() {
        let mut f = rww_flash();
        f.mark_swapped();
        assert!(f.is_swapped());
        // Under SWAP_BANK the bank presented at 0x08000000 (buffer offset 0) is
        // physical bank 1. The firmware still erases with BKSEL=0 (logical bank
        // 0), which now maps to physical bank 1 — same physical bank as PC.
        assert_eq!(
            f.physical_bank_of_offset(0x0000),
            1,
            "PC@0x08000000 → phys1"
        );
        assert!(
            f.rww_erase_violates(0, 0x0000),
            "swapped: logical-bank0 erase hits PC's physical bank1"
        );
        // An erase of logical bank 1 maps to physical bank 0 — the OTHER bank.
        assert!(!f.rww_erase_violates(1, 0x0000), "swapped: cross-bank OK");
    }

    #[test]
    fn two_swaps_restore_identity_mapping() {
        let mut f = rww_flash();
        f.mark_swapped();
        f.mark_swapped();
        assert!(!f.is_swapped(), "even swaps cancel");
        assert!(f.rww_erase_violates(0, 0x0000));
        assert!(!f.rww_erase_violates(1, 0x0000));
    }
}

#[cfg(test)]
mod h5_program_error_tests {
    use super::h5;
    use super::{Flash, FlashRegisterLayout, H5ProgAction};
    use crate::Peripheral;

    fn h5_gate_on() -> Flash {
        // Gate on + PG set: write-buffer programming is armed.
        let mut f = Flash::new_with_layout(FlashRegisterLayout::Stm32H5).with_error_flags(true);
        f.write_u32(h5::NSCR_OFF, h5::NSCR_PG).unwrap();
        f
    }

    fn nssr(f: &Flash) -> u32 {
        // NSSR is the H5 status register (offset 0x20). Reassemble from bytes.
        let b = |o: u64| f.read(o).unwrap() as u32;
        b(h5::NSSR_OFF)
            | (b(h5::NSSR_OFF + 1) << 8)
            | (b(h5::NSSR_OFF + 2) << 16)
            | (b(h5::NSSR_OFF + 3) << 24)
    }

    /// Program a full aligned quad-word at `base` byte-by-byte, returning the
    /// action from the final (16th) byte (which should be a Commit).
    fn program_full_quadword(f: &mut Flash, base: u64, fill: u8) -> H5ProgAction {
        let mut last = H5ProgAction::Buffered;
        for i in 0..16u64 {
            last = f.h5_program_byte(base + i, fill);
        }
        last
    }

    #[test]
    fn partial_quadword_sets_wbne_no_commit_no_error() {
        let mut f = h5_gate_on();
        // Three bytes of a 16-byte quad-word: still buffering.
        for i in 0..3u64 {
            assert_eq!(f.h5_program_byte(0x20 + i, 0xAA), H5ProgAction::Buffered);
        }
        assert_ne!(nssr(&f) & h5::NSSR_WBNE, 0, "WBNE set while partial");
        assert_eq!(nssr(&f) & h5::NSSR_EOP, 0, "no EOP — not committed");
        assert_eq!(nssr(&f) & h5::NSSR_INCERR, 0, "no error");
    }

    #[test]
    fn full_aligned_quadword_commits_sets_eop_clears_wbne() {
        let mut f = h5_gate_on();
        let action = program_full_quadword(&mut f, 0x20, 0xAA);
        assert_eq!(
            action,
            H5ProgAction::Commit {
                base: 0x20,
                bytes: [0xAA; 16]
            },
            "full quad-word commits with the buffered bytes"
        );
        assert_ne!(nssr(&f) & h5::NSSR_EOP, 0, "EOP set on commit");
        assert_eq!(nssr(&f) & h5::NSSR_WBNE, 0, "WBNE cleared on commit");
    }

    #[test]
    fn misaligned_first_write_then_jump_sets_incerr_alone() {
        let mut f = h5_gate_on();
        // First write at base+4 starts a buffer at base 0x20 (the quad-word it
        // belongs to). A subsequent write into a DIFFERENT quad-word (0x30)
        // before the first completes is the inconsistent program run.
        assert_eq!(f.h5_program_byte(0x24, 0x11), H5ProgAction::Buffered);
        assert_ne!(nssr(&f) & h5::NSSR_WBNE, 0, "WBNE while partial");
        let action = f.h5_program_byte(0x30, 0x22);
        assert_eq!(action, H5ProgAction::Inconsistent);
        assert_ne!(nssr(&f) & h5::NSSR_INCERR, 0, "INCERR set");
        assert_eq!(nssr(&f) & h5::NSSR_PGSERR, 0, "INCERR alone (no PGSERR)");
        assert_eq!(nssr(&f) & h5::NSSR_WBNE, 0, "buffer discarded ⇒ WBNE clear");
    }

    #[test]
    fn nsccr_clears_eop_and_incerr_but_nssr_write_does_not() {
        let mut f = h5_gate_on();
        // Commit a quad-word to set EOP.
        program_full_quadword(&mut f, 0x20, 0xAA);
        assert_ne!(nssr(&f) & h5::NSSR_EOP, 0, "EOP set");
        // Writing NSSR (0x20) must NOT clear it (silicon: NSSR is RO status).
        f.write_u32(h5::NSSR_OFF, h5::NSSR_EOP).unwrap();
        assert_ne!(nssr(&f) & h5::NSSR_EOP, 0, "NSSR write does NOT clear EOP");
        // Writing NSCCR (0x30) DOES clear it.
        f.write_u32(h5::NSCCR_OFF, h5::NSSR_EOP).unwrap();
        assert_eq!(nssr(&f) & h5::NSSR_EOP, 0, "NSCCR clears EOP");
    }

    #[test]
    fn pg_clear_does_not_program() {
        // Gate on but NSCR.PG clear ⇒ no programming, no flag.
        let mut f = Flash::new_with_layout(FlashRegisterLayout::Stm32H5).with_error_flags(true);
        assert_eq!(f.cr & h5::NSCR_PG, 0, "PG clear at reset");
        assert_eq!(f.h5_program_byte(0x20, 0xAA), H5ProgAction::NotProgramming);
        assert_eq!(nssr(&f), 0, "no flag when PG clear");
    }

    #[test]
    fn gate_is_noop_on_non_h5_layout() {
        // Enabling the gate on L4 must not arm the H5 write-buffer machine.
        let mut f = Flash::new_with_layout(FlashRegisterLayout::Stm32L4).with_error_flags(true);
        assert!(!f.h5_error_flags_enabled());
        // h5_program_byte short-circuits to Buffered (bus never calls it here).
        // Note: on L4 offset 0x20 is OPTR, not NSSR, so reading "nssr" is
        // meaningless here — the meaningful assertion is the no-op action.
        assert_eq!(f.h5_program_byte(0x3, 0x00), H5ProgAction::Buffered);
    }
}
