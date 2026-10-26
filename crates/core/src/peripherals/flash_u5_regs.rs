// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! STM32U5 FLASH register offsets + bitfields (RM0456 §7).
//!
//! Offset/field/reset values cross-checked against the vendored SVD
//! `tests/fixtures/real_world/stm32u575.svd` (FLASH @ 0x4002_2000):
//! ACR=0x00 (LATENCY[3:0], PRFTEN[8], LPM[11], PDREQ1/2[12:13],
//! SLEEP_PD[14]), NSKEYR=0x08, SECKEYR=0x0C, OPTKEYR=0x10,
//! PDKEY1R=0x18, PDKEY2R=0x1C, NSSR=0x20, SECSR=0x24, NSCR=0x28
//! (reset 0xC000_0000 = LOCK|OPTLOCK), SECCR=0x2C, ECCR=0x30,
//! OPSR=0x34, OPTR=0x40 (reset 0x0000_0000).
//!
//! The U5 register map is NOT the H5 map (H5: NSKEYR@0x04, NSSR@0x20 with
//! EOP at bit 16 and a separate NSCCR@0x30 clear register, NSCR@0x28 with
//! SER/SNB/BKSEL) and NOT the L4 map either (L4: OPTR@0x20, `l4` profile's
//! CR program/erase semantics are not bank/page-faithful for 8 KiB pages).
//! U5 keys are the architectural FLASH_KEY1/KEY2 and OPTKEY1/OPTKEY2
//! sequences (same constants as L4/H5; RM0456 §7.3.5/§7.3.6).
//!
//! Geometry (RM0456 §7.1, DS13737): 2 MiB main flash in two 1 MiB banks,
//! 8 KiB pages (128 per bank). Programming granule is a 128-bit quad-word
//! (16 bytes) programmed as 4 successive 32-bit word accesses; page erase
//! fills the 8 KiB page with 0xFF.

#![allow(dead_code)]

// ── Register offsets (relative to FLASH base 0x4002_2000) ───────────────────

pub const ACR_OFF: u64 = 0x00;
pub const NSKEYR_OFF: u64 = 0x08;
pub const SECKEYR_OFF: u64 = 0x0C;
pub const OPTKEYR_OFF: u64 = 0x10;
pub const NSSR_OFF: u64 = 0x20;
pub const NSCR_OFF: u64 = 0x28;
pub const OPSR_OFF: u64 = 0x34;
pub const OPTR_OFF: u64 = 0x40;

// ── FLASH_ACR bitfields (SVD FLASH_ACR) ─────────────────────────────────────

/// Bits [3:0] — LATENCY: AHB wait states. CubeU5 writes then reads this back
/// in `HAL_RCC_ClockConfig`; it is the one field the U5 bring-up polls.
pub const ACR_LATENCY_MASK: u32 = 0xF;
/// Bit 8 — PRFTEN: prefetch enable.
pub const ACR_PRFTEN: u32 = 1 << 8;
/// Writable ACR bits: LATENCY[3:0], PRFTEN[8], LPM[11], PDREQ1[12],
/// PDREQ2[13], SLEEP_PD[14].
pub const ACR_WRITABLE_MASK: u32 =
    ACR_LATENCY_MASK | ACR_PRFTEN | (1 << 11) | (1 << 12) | (1 << 13) | (1 << 14);

// ── FLASH_NSCR bitfields (RM0456 §7.9 / SVD) ────────────────────────────────

/// Bit 0 — PG: non-secure programming enable.
pub const NSCR_PG: u32 = 1 << 0;
/// Bit 1 — PER: non-secure page erase.
pub const NSCR_PER: u32 = 1 << 1;
/// Bit 2 — MER1: non-secure bank 1 mass erase.
pub const NSCR_MER1: u32 = 1 << 2;
/// Bits [9:3] — PNB: page number (0..127 within the selected bank).
pub const NSCR_PNB_SHIFT: u32 = 3;
pub const NSCR_PNB_MASK: u32 = 0x7F << NSCR_PNB_SHIFT;
/// Bit 11 — BKER: bank select for page erase (0 = bank 1, 1 = bank 2).
pub const NSCR_BKER: u32 = 1 << 11;
/// Bit 15 — MER2: non-secure bank 2 mass erase.
pub const NSCR_MER2: u32 = 1 << 15;
/// Bit 16 — STRT: start erase. Self-clearing: cleared by hardware when BSY
/// clears (the model completes the op at the write, so it never reads back 1).
pub const NSCR_STRT: u32 = 1 << 16;
/// Bit 17 — OPTSTRT: options modification start (needs OPTKEYR unlock).
pub const NSCR_OPTSTRT: u32 = 1 << 17;
/// Bit 24 — EOPIE: end-of-operation interrupt and status enable.
pub const NSCR_EOPIE: u32 = 1 << 24;
/// Bit 25 — ERRIE: error interrupt enable.
pub const NSCR_ERRIE: u32 = 1 << 25;
/// Bit 27 — OBL_LAUNCH: force option-byte loading.
pub const NSCR_OBL_LAUNCH: u32 = 1 << 27;
/// Bit 30 — OPTLOCK: option lock. Set-only; cleared by the OPTKEYR sequence.
pub const NSCR_OPTLOCK: u32 = 1 << 30;
/// Bit 31 — LOCK: non-secure lock. Set-only; cleared by the NSKEYR sequence.
pub const NSCR_LOCK: u32 = 1 << 31;

/// NSCR reset value (SVD): LOCK + OPTLOCK both held out of reset.
pub const NSCR_RESET: u32 = NSCR_LOCK | NSCR_OPTLOCK;

// ── FLASH_NSSR bitfields (RM0456 §7.9 / SVD) ────────────────────────────────

/// Bit 0 — EOP: end of successful operation.
pub const NSSR_EOP: u32 = 1 << 0;
/// Bit 1 — OPERR: operation error.
pub const NSSR_OPERR: u32 = 1 << 1;
/// Bit 3 — PROGERR: programming error (target not erased). NOT raised by this
/// model: program-over-not-erased commits the bitwise AND instead.
pub const NSSR_PROGERR: u32 = 1 << 3;
/// Bit 4 — WRPERR: write-protection error (locked / protected region).
pub const NSSR_WRPERR: u32 = 1 << 4;
/// Bit 5 — PGAERR: programming alignment error.
pub const NSSR_PGAERR: u32 = 1 << 5;
/// Bit 6 — SIZERR: size error (byte/half-word access during programming).
pub const NSSR_SIZERR: u32 = 1 << 6;
/// Bit 7 — PGSERR: programming sequence error.
pub const NSSR_PGSERR: u32 = 1 << 7;
/// Bit 13 — OPTWERR: option write error.
pub const NSSR_OPTWERR: u32 = 1 << 13;
/// Bit 16 — BSY: busy, read-only live status (never left set by the model).
pub const NSSR_BSY: u32 = 1 << 16;
/// Bit 17 — WDW: wait data to write (write buffer partially filled). Live
/// status, not W1C: set while a quad-word program is partial, cleared on
/// commit.
pub const NSSR_WDW: u32 = 1 << 17;

/// Sticky NSSR flags cleared by writing 1 to the matching NSSR bit. BSY/WDW
/// are live status and are NOT in this mask.
pub const NSSR_W1C_MASK: u32 = NSSR_EOP
    | NSSR_OPERR
    | NSSR_PROGERR
    | NSSR_WRPERR
    | NSSR_PGAERR
    | NSSR_SIZERR
    | NSSR_PGSERR
    | NSSR_OPTWERR;

// ── FLASH_OPTR bitfield used by the model ───────────────────────────────────

/// Bit 20 — SWAP_BANK: option-bit bank swap (read/storage only; no swap is
/// applied to the backing store on U5 — see the board doc).
pub const OPTR_SWAP_BANK: u32 = 1 << 20;

// ── Address / geometry constants (RM0456 §7.1, DS13737) ─────────────────────

/// Flash base address (both banks start here before any bank swap).
pub const FLASH_BASE: u64 = 0x0800_0000;
/// Per-bank size: 1 MiB (2 MiB total in two banks).
pub const BANK_SIZE: u64 = 0x10_0000;
/// Page size: 8 KiB (128 pages per 1 MiB bank).
pub const PAGE_SIZE: u64 = 0x2000;
/// Pages per bank: 1 MiB / 8 KiB = 128 (PNB is 7 bits wide).
pub const PAGES_PER_BANK: u32 = 128;
/// Quad-word (128-bit) programming granule: 4 successive 32-bit word writes.
pub const PROG_GRANULARITY: u64 = 16;
