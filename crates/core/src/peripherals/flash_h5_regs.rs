// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! STM32H5 FLASH register offsets + bitfields (RM0481 §7).
//!
//! Register offsets and reset values cross-checked against NUCLEO-H563ZI
//! silicon via SWD (2026-06-20): a read of 0x4002_2000×24 confirmed
//! ACR=0x13, OPTCR=0x1, NSSR=0x0, NSCR=0x1, OPTSR_CUR=OPTSR_PRG=0x2D30_EDF8
//! (bit31/SWAP_BANK clear → bank 1 active), matching the model reset values.
//! Bitfield positions (SER/STRT/SNB/SWAP_BANK/OPTSTRT) are architectural
//! per RM0481 §7 (not reset-readable). SVD-confirmed against
//! `tests/fixtures/real_world/stm32h563.svd`: FLASH_NSKEYR=0x004,
//! FLASH_SECKEYR=0x008 (secure, unused here), FLASH_OPTKEYR=0x00C;
//! FLASH_OPTCR has OPTLOCK(bit0), OPTSTRT(bit1), SWAP_BANK(bit31) — there
//! is no OBL_LAUNCH on H5 (that field exists on F4/L4 only).

#![allow(dead_code)]

// ── Register offsets (relative to FLASH base 0x4002_2000) ───────────────────

pub const NSKEYR_OFF: u64 = 0x04;
pub const NSSR_OFF: u64 = 0x20;
pub const NSCR_OFF: u64 = 0x28;
/// NSCCR @ 0x30 — clear register. Writing 1 to a bit clears the corresponding
/// NSSR flag (EOP/WRPERR/PGSERR/STRBERR/INCERR). Verified on NUCLEO-H563ZI
/// silicon (2026-06-22): writing NSSR itself does NOT clear; NSCCR does.
pub const NSCCR_OFF: u64 = 0x30;
pub const OPTCR_OFF: u64 = 0x1C;
pub const OPTSR_CUR_OFF: u64 = 0x50;
pub const OPTSR_PRG_OFF: u64 = 0x54;

// ── FLASH_NSCR bitfields (RM0481 §7.9.9) ────────────────────────────────────

/// Bit 0 — LOCK: FLASH_NSCR lock bit. 1 = locked (reset state).
pub const NSCR_LOCK: u32 = 1 << 0;
/// Bit 1 — PG: non-secure programming enable.
pub const NSCR_PG: u32 = 1 << 1;
/// Bit 2 — SER: non-secure sector erase.
pub const NSCR_SER: u32 = 1 << 2;
/// Bit 3 — BER: non-secure bank erase.
pub const NSCR_BER: u32 = 1 << 3;
/// Bit 5 — STRT: non-secure start erase/program.
pub const NSCR_STRT: u32 = 1 << 5;
/// Bits [12:6] — SNB: sector number (7 bits).
pub const NSCR_SNB_SHIFT: u32 = 6;
pub const NSCR_SNB_MASK: u32 = 0x7F << NSCR_SNB_SHIFT;
/// Bit 31 — BKSEL: bank select (0 = bank 1, 1 = bank 2).
pub const NSCR_BKSEL: u32 = 1 << 31;

// ── FLASH_NSSR bitfields (RM0481 §7.9.8) ────────────────────────────────────

/// Bit 0 — BSY: non-secure busy.
pub const NSSR_BSY: u32 = 1 << 0;
/// Bit 1 — WBNE: write buffer not empty. Live status — set while a partial
/// quad-word is buffered, cleared when the quad-word commits or on reset. NOT
/// W1C. Verified on silicon (NSSR=0x2 after the 1st word of a quad-word).
pub const NSSR_WBNE: u32 = 1 << 1;
/// Bit 16 — EOP: end of (successful) operation. Set when a quad-word commits.
/// Sticky, cleared via NSCCR. Verified on silicon (NSSR=0x10000 after commit).
pub const NSSR_EOP: u32 = 1 << 16;
/// Bit 17 — WRPERR: write-protection error (program/erase of a protected or
/// locked region). RM0481 §7.9.8. Sticky, cleared via NSCCR. Out of scope here
/// (no per-region protection modeled) — the constant exists, set nowhere.
pub const NSSR_WRPERR: u32 = 1 << 17;
/// Bit 18 — PGSERR: programming-sequence error. RM0481 §7.9.8. Sticky, cleared
/// via NSCCR. NOT raised by program-over-not-erased on this part (silicon
/// allows that with the result being the bitwise AND — verified 2026-06-22).
pub const NSSR_PGSERR: u32 = 1 << 18;
/// Bit 19 — STRBERR: strobe error. RM0481 §7.9.8. Sticky, cleared via NSCCR.
pub const NSSR_STRBERR: u32 = 1 << 19;
/// Bit 20 — INCERR: inconsistency error. RM0481 §7.9.8. Raised ALONE on a
/// misaligned / inconsistent quad-word program (a 16-byte run that does not
/// align to a single quad-word); nothing is committed. Verified on silicon
/// (NSSR=0x100000 = INCERR alone after 4 writes from a non-aligned base).
/// Sticky, cleared via NSCCR.
pub const NSSR_INCERR: u32 = 1 << 20;

/// Mask of the sticky operation/error flags cleared by writing 1 to the
/// matching bit in NSCCR (0x30). WBNE/BSY are live status, NOT in this mask.
pub const NSSR_W1C_MASK: u32 = NSSR_EOP | NSSR_WRPERR | NSSR_PGSERR | NSSR_STRBERR | NSSR_INCERR;

/// Quad-word (16-byte) programming granularity. A program commits only when a
/// full 16-byte run aligned to this granularity has been buffered.
pub const PROG_GRANULARITY: u64 = 16;

// ── FLASH_OPTSR_CUR / OPTSR_PRG bitfields (RM0481 §7.9.14) ─────────────────

/// Bit 31 — SWAP_BANK: bank-swap option bit (0 = no swap, 1 = swapped).
/// OPTSR_CUR bit 31 = 0 on the captured NUCLEO-H563ZI (0x2D30_EDF8).
pub const OPTSR_SWAP_BANK: u32 = 1 << 31;

// ── FLASH_OPTCR bitfields (RM0481 §7.9.13) ──────────────────────────────────

/// Bit 1 — OPTSTRT: start option-byte programming (SVD-confirmed, H5 only).
/// Writing 1 after OPTKEYR unlock and OPTSR_PRG.SWAP_BANK programs the option
/// bytes; the swap takes effect on the next system reset.
pub const OPTCR_OPTSTRT: u32 = 1 << 1;

// ── Address / geometry constants ─────────────────────────────────────────────

/// Flash base address (both banks start here before any bank swap).
pub const FLASH_BASE: u64 = 0x0800_0000;
/// Per-bank size: 1 MB (STM32H563ZI has 2 × 1 MB).
pub const BANK_SIZE: u64 = 0x10_0000;
/// Sector size: 8 KB (RM0481 §7.1).
pub const SECTOR_SIZE: u64 = 0x2000;
