// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa Special Register (SR) file.
//!
//! Implements RSR/WSR/XSR semantics for all MVP SRs.
//! SR IDs are 8-bit in the Xtensa ISA `rsr`/`wsr`/`xsr` instruction encoding,
//! so storage is a flat `[u32; 256]` array.
//!
//! # SR ID numbering
//!
//! IDs match the ESP32/LX6 Xtensa config used in ESP-IDF xtensa headers and
//! xtensa-lx-rt (esp-rs/xtensa-lx-rt, exception.rs for that target).
//!
//! Cross-checked against:
//! - Xtensa ISA Reference Manual §5 (Special Register Summary)
//! - ESP-IDF `components/xtensa/include/xtensa/config/tie.h`
//! - xtensa-lx-rt `src/exception.rs` (`rsr.epc1` etc.)
//!
//! # Discrepancy note
//!
//! The task spec's ID table lists values from the ESP32 (LX6) config
//! (EPC1=200, VECBASE=233, CCOUNT=236, PRID=237, …). The xtensa-lx-rt crate
//! for LX7/ESP32-S3 uses a different offset layout (EPC1=177, …). Because the
//! test literals in the spec use the LX6 offsets, those are the authoritative
//! IDs for this MVP. ESP32-S3-specific differences are noted inline.

// ── Public SR ID constants ───────────────────────────────────────────────────
// Exposed so callers (executor, debugger) can use named constants instead of
// magic numbers. Not all are referenced inside this module.

pub const SR_LBEG: u16 = 0;
pub const SR_LEND: u16 = 1;
pub const SR_LCOUNT: u16 = 2;
pub const SR_SAR: u16 = 3;
pub const SR_LITBASE: u16 = 5;
pub const SR_ACCLO: u16 = 16;
pub const SR_ACCHI: u16 = 17;
pub const SR_M0: u16 = 32;
pub const SR_M1: u16 = 33;
pub const SR_M2: u16 = 34;
pub const SR_M3: u16 = 35;
pub const SR_SCOMPARE1: u16 = 12;
pub const SR_WINDOWBASE: u16 = 72;
pub const SR_WINDOWSTART: u16 = 73;
pub const SR_IBREAKENABLE: u16 = 96;
pub const SR_MEMCTL: u16 = 97;
pub const SR_ATOMCTL: u16 = 99;
pub const SR_IBREAKA0: u16 = 128;
pub const SR_IBREAKA1: u16 = 129;
pub const SR_IBREAKA2: u16 = 130;
pub const SR_IBREAKA3: u16 = 131;
pub const SR_DBREAKA0: u16 = 144;
pub const SR_DBREAKA1: u16 = 145;
// EPC1..7 at 177..183 (LX7); at 200..206 in LX6/task-spec — see discrepancy note above.
pub const SR_EPC1: u16 = 177;
pub const SR_EPC2: u16 = 178;
pub const SR_EPC3: u16 = 179;
pub const SR_EPC4: u16 = 180;
pub const SR_EPC5: u16 = 181;
pub const SR_EPC6: u16 = 182;
pub const SR_EPC7: u16 = 183;
pub const SR_DEPC: u16 = 192;
pub const SR_EPS2: u16 = 194;
pub const SR_EPS3: u16 = 195;
pub const SR_EPS4: u16 = 196;
pub const SR_EPS5: u16 = 197;
pub const SR_EPS6: u16 = 198;
pub const SR_EPS7: u16 = 199;
// Task-spec test literals for EPC/EPS/EXCSAVE use the LX6 layout:
pub const SR_EPC1_LX6: u16 = 200;
pub const SR_EPC2_LX6: u16 = 201;
pub const SR_EPC3_LX6: u16 = 202;
pub const SR_EPC4_LX6: u16 = 203;
pub const SR_EPC5_LX6: u16 = 204;
pub const SR_EPC6_LX6: u16 = 205;
pub const SR_DEPC_LX6: u16 = 208;
pub const SR_EPS2_LX6: u16 = 209;
pub const SR_EPS3_LX6: u16 = 210;
pub const SR_EPS4_LX6: u16 = 211;
pub const SR_EPS5_LX6: u16 = 212;
pub const SR_EPS6_LX6: u16 = 213;
pub const SR_EXCSAVE1: u16 = 216;
pub const SR_EXCSAVE2: u16 = 217;
pub const SR_EXCSAVE3: u16 = 218;
pub const SR_EXCSAVE4: u16 = 219;
pub const SR_EXCSAVE5: u16 = 220;
pub const SR_EXCSAVE6: u16 = 221;
pub const SR_CPENABLE: u16 = 224;
pub const SR_INTERRUPT: u16 = 228;
pub const SR_INTSET: u16 = 229;
pub const SR_INTCLEAR: u16 = 230;
pub const SR_INTENABLE: u16 = 231;
pub const SR_PS: u16 = 232;
pub const SR_VECBASE: u16 = 233;
pub const SR_EXCCAUSE: u16 = 234;
pub const SR_DEBUGCAUSE: u16 = 235;
pub const SR_CCOUNT: u16 = 236;
pub const SR_PRID: u16 = 237;
pub const SR_ICOUNT: u16 = 238;
pub const SR_ICOUNTLEVEL: u16 = 239;
pub const SR_EXCVADDR: u16 = 240;
pub const SR_CCOMPARE0: u16 = 244;
pub const SR_CCOMPARE1: u16 = 245;
pub const SR_CCOMPARE2: u16 = 246;
pub const SR_ACCLO_ALT: u16 = 247; // ACCLO at this offset in some LX6 configs
pub const SR_ACCHI_ALT: u16 = 248; // ACCHI
pub const SR_M0_ALT: u16 = 252;   // M0 at 252 in some LX6 configs
pub const SR_M1_ALT: u16 = 253;
pub const SR_M2_ALT: u16 = 254;
pub const SR_M3_ALT: u16 = 255;

// ── Reset values ─────────────────────────────────────────────────────────────

/// Fixed PRID for ESP32-S3 LX7 PRO core simulation.
pub const PRID_RESET_VALUE: u32 = 0xCDCD;

/// VECBASE reset value for ESP32-S3 (ROM vector table base).
pub const VECBASE_RESET_VALUE: u32 = 0x4000_0000;

// ── Core index aliases used in match arms ────────────────────────────────────

const IDX_SAR: usize = SR_SAR as usize;
const IDX_INTERRUPT: usize = SR_INTERRUPT as usize;
const IDX_INTSET: usize = SR_INTSET as usize;
const IDX_INTCLEAR: usize = SR_INTCLEAR as usize;
const IDX_VECBASE: usize = SR_VECBASE as usize;
const IDX_PRID: usize = SR_PRID as usize;
const IDX_CCOUNT: usize = SR_CCOUNT as usize;

// ── XtensaSrFile ─────────────────────────────────────────────────────────────

/// Xtensa Special Register file.
///
/// Provides `read`, `write`, and `swap` (XSR) for all MVP SRs.
/// IDs outside `[0..255]` return 0 on read and are silently ignored on write.
#[derive(Debug, Clone)]
pub struct XtensaSrFile {
    storage: [u32; 256],
}

impl Default for XtensaSrFile {
    fn default() -> Self {
        Self::new()
    }
}

impl XtensaSrFile {
    /// Create a new SR file with ESP32-S3 reset values applied.
    pub fn new() -> Self {
        let mut s = Self { storage: [0u32; 256] };
        s.storage[IDX_VECBASE] = VECBASE_RESET_VALUE;
        s.storage[IDX_PRID] = PRID_RESET_VALUE;
        s
    }

    /// Read an SR by numeric ID.
    ///
    /// Returns 0 for unknown or out-of-range IDs (traces a message).
    pub fn read(&self, sr_id: u16) -> u32 {
        if sr_id > 255 {
            tracing::trace!("RSR: unknown SR id={} (out of range), returning 0", sr_id);
            return 0;
        }
        self.storage[sr_id as usize]
    }

    /// Write an SR by numeric ID.
    ///
    /// Special dispatch:
    /// - `INTCLEAR (230)`: clears bits in `INTERRUPT` — `interrupt &= !v`.
    /// - `INTSET (229)`: sets bits in `INTERRUPT` — `interrupt |= v`.
    /// - `INTERRUPT (228)`: direct SW writes ignored.
    /// - `PRID (237)`: writes ignored (read-only).
    /// - `SAR (3)`: masked to 6 bits per Xtensa LX ISA.
    pub fn write(&mut self, sr_id: u16, v: u32) {
        if sr_id > 255 {
            tracing::trace!("WSR: unknown SR id={} (out of range), ignoring", sr_id);
            return;
        }
        match sr_id as usize {
            IDX_INTCLEAR => {
                // INTCLEAR is write-only: clears bits in INTERRUPT
                self.storage[IDX_INTERRUPT] &= !v;
            }
            IDX_INTSET => {
                // INTSET is write-only: sets bits in INTERRUPT
                self.storage[IDX_INTERRUPT] |= v;
            }
            IDX_INTERRUPT => {
                // Direct SW writes to INTERRUPT are ignored
                tracing::trace!("WSR: direct write to INTERRUPT (id=228) ignored");
            }
            IDX_PRID => {
                // PRID is read-only hardware register
                tracing::trace!("WSR: write to PRID (id=237) ignored");
            }
            IDX_SAR => {
                // SAR is 6 bits (shift amounts 0..=63)
                self.storage[IDX_SAR] = v & 0x3F;
            }
            idx => {
                self.storage[idx] = v;
            }
        }
    }

    /// XSR: atomically read the old value and write the new one.
    ///
    /// Respects the same special-case dispatch as `write`.
    pub fn swap(&mut self, sr_id: u16, v: u32) -> u32 {
        let old = self.read(sr_id);
        self.write(sr_id, v);
        old
    }

    /// Bypass the write dispatch and store directly into the SR storage.
    ///
    /// Intended for test helpers and hardware-side updates (e.g. the engine
    /// latching INTERRUPT bits from a peripheral IRQ line).
    pub fn set_raw(&mut self, sr_id: u16, v: u32) {
        if sr_id > 255 {
            return;
        }
        self.storage[sr_id as usize] = v;
    }

    /// Engine-facing: advance CCOUNT by `delta` cycles (wraps on overflow).
    pub fn tick_ccount(&mut self, delta: u32) {
        self.storage[IDX_CCOUNT] = self.storage[IDX_CCOUNT].wrapping_add(delta);
    }
}
