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
//! IDs are the LX7 / ESP32-S3 authoritative encoding, verified empirically via
//! `xtensa-esp-elf-as`: each `rsr.<name>` instruction emits a 3-byte word
//! `20 XX 03` where `XX` is the SR ID. All constants below reflect this ground
//! truth and supersede any earlier LX6-derived or doc-only tables.

// ── Public SR ID constants ───────────────────────────────────────────────────
// Verified against `xtensa-esp-elf-as` emitted `rsr.<name>` instruction bytes.
// The middle byte of the 0x20XX03 encoding IS the SR ID.

pub const SAR:          u16 = 3;    // 0x03
pub const LITBASE:      u16 = 5;    // 0x05
pub const SCOMPARE1:    u16 = 12;   // 0x0C
pub const ACCLO:        u16 = 16;   // 0x10 MAC16 accumulator — stub
pub const ACCHI:        u16 = 17;
pub const M0:           u16 = 32;   // 0x20 MAC16 — stub
pub const M1:           u16 = 33;
pub const M2:           u16 = 34;
pub const M3:           u16 = 35;
pub const WINDOWBASE:   u16 = 72;   // 0x48
pub const WINDOWSTART:  u16 = 73;   // 0x49
pub const PTEVADDR:     u16 = 83;   // 0x53 (not MVP-critical, placeholder)
pub const IBREAKA0:     u16 = 128;  // 0x80
pub const IBREAKA1:     u16 = 129;
pub const DBREAKA0:     u16 = 144;  // 0x90
pub const DBREAKA1:     u16 = 145;
pub const DBREAKC0:     u16 = 160;  // 0xA0
pub const DBREAKC1:     u16 = 161;
pub const EPC1:         u16 = 177;  // 0xB1
pub const EPC2:         u16 = 178;  // 0xB2
pub const EPC3:         u16 = 179;
pub const EPC4:         u16 = 180;
pub const EPC5:         u16 = 181;
pub const EPC6:         u16 = 182;
pub const EPC7:         u16 = 183;
pub const DEPC:         u16 = 192;  // 0xC0
pub const EPS2:         u16 = 194;  // 0xC2
pub const EPS3:         u16 = 195;
pub const EPS4:         u16 = 196;
pub const EPS5:         u16 = 197;
pub const EPS6:         u16 = 198;
pub const EPS7:         u16 = 199;
pub const EXCSAVE1:     u16 = 209;  // 0xD1
pub const EXCSAVE2:     u16 = 210;
pub const EXCSAVE3:     u16 = 211;
pub const EXCSAVE4:     u16 = 212;
pub const EXCSAVE5:     u16 = 213;
pub const EXCSAVE6:     u16 = 214;
pub const EXCSAVE7:     u16 = 215;
pub const CPENABLE:     u16 = 224;  // 0xE0
pub const INTERRUPT:    u16 = 226;  // 0xE2 (read-only from SW)
pub const INTCLEAR:     u16 = 227;  // 0xE3 (write-clears INTERRUPT bits)
pub const INTENABLE:    u16 = 228;  // 0xE4
pub const PS:           u16 = 230;  // 0xE6
pub const VECBASE:      u16 = 231;  // 0xE7
pub const EXCCAUSE:     u16 = 232;  // 0xE8
pub const DEBUGCAUSE:   u16 = 233;  // 0xE9
pub const CCOUNT:       u16 = 234;  // 0xEA (SW-writable, engine also updates)
pub const PRID:         u16 = 235;  // 0xEB (read-only)
pub const ICOUNT:       u16 = 236;  // 0xEC
pub const ICOUNTLEVEL:  u16 = 237;  // 0xED
pub const EXCVADDR:     u16 = 238;  // 0xEE
pub const CCOMPARE0:    u16 = 240;  // 0xF0
pub const CCOMPARE1:    u16 = 241;
pub const CCOMPARE2:    u16 = 242;

// ── Reset values ─────────────────────────────────────────────────────────────

/// Fixed PRID for ESP32-S3 LX7 PRO core simulation.
pub const PRID_RESET_VALUE: u32 = 0xCDCD;

/// VECBASE reset value for ESP32-S3 (ROM vector table base).
pub const VECBASE_RESET_VALUE: u32 = 0x4000_0000;

// ── Core index aliases used in match arms ────────────────────────────────────

const IDX_SAR:       usize = SAR       as usize;
const IDX_INTERRUPT: usize = INTERRUPT as usize;
const IDX_INTCLEAR:  usize = INTCLEAR  as usize;
const IDX_VECBASE:   usize = VECBASE   as usize;
const IDX_PRID:      usize = PRID      as usize;
const IDX_CCOUNT:    usize = CCOUNT    as usize;

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
    /// - `INTCLEAR (227)`: clears bits in `INTERRUPT` — `interrupt &= !v`.
    /// - `INTERRUPT (226)`: direct SW writes ignored (hardware-latched).
    /// - `PRID (235)`: writes ignored (read-only).
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
            IDX_INTERRUPT => {
                // Direct SW writes to INTERRUPT are ignored (hardware-latched)
                tracing::trace!("WSR: direct write to INTERRUPT (id=226) ignored");
            }
            IDX_PRID => {
                // PRID is read-only hardware register
                tracing::trace!("WSR: write to PRID (id=235) ignored");
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
