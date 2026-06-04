// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 USB-OTG (DWC_otg / Synopsys DesignWare USB 2.0 OTG) controller.
//!
//! This is the *second* USB block on the S3. It is the full-speed
//! Synopsys DWC2 OTG core (the same IP behind TinyUSB's `dwc2` port and
//! esp-idf's USB-OTG / TinyUSB driver), mapped by esp-idf at
//! `0x6008_0000`. It is **distinct** from the [`UsbSerialJtag`] CDC/JTAG
//! bridge at `0x6003_8000` — do not confuse the two.
//!
//! - Base address: `0x6008_0000`
//! - Modelled window: `0x1000` bytes (core global regs + device-mode core)
//! - Interrupt source: `ETS_USB_INTR_SOURCE = 23` (DWC OTG core interrupt),
//!   delivered through [`PeripheralTickResult::explicit_irqs`].
//!
//! ## What is modelled
//!
//! The Core Global register block and the start of the device-mode core,
//! enough that a polling DWC2 driver (esp-idf `usb_phy` + TinyUSB, or a
//! bare-metal probe) can:
//!   * read the Synopsys release id ([`GSNPSID`]) and the four hardware
//!     capability words ([`GHWCFG1`]..[`GHWCFG4`]) — all read-only constants;
//!   * issue a core soft reset via `GRSTCTL.CSRST` (bit 0), which
//!     **self-clears** and always reads back `AHBIDLE` (bit 31) = 1;
//!   * enable interrupts (`GAHBCFG.GINTMSK` global bit 0 + per-source
//!     `GINTMSK`) and observe / W1-clear `GINTSTS`;
//!   * read live device status (`DSTS`) and drive `DCTL.SFTDISCON`.
//!
//! Every other offset inside the `0x1000` window round-trips losslessly
//! through a sparse map, so a driver that pokes registers we did not give
//! explicit semantics still reads back what it wrote and nothing is
//! silently dropped.
//!
//! ## Register layout (ESP32-S3 TRM "USB OTG" + Synopsys DWC2 map)
//!
//! | Offset | Name      | Dir | Behaviour |
//! |-------:|-----------|-----|-----------|
//! | 0x000  | GOTGCTL   | R/W | OTG control; round-trips, `BSESVLD|ASESVLD|CONIDSTS` forced for liveness |
//! | 0x004  | GOTGINT   | R/W1C | OTG interrupt; W1C |
//! | 0x008  | GAHBCFG   | R/W | AHB config; bit 0 = global interrupt enable (GINTMSK) |
//! | 0x00C  | GUSBCFG   | R/W | USB config; round-trips |
//! | 0x010  | GRSTCTL   | R/W | bit 0 CSRST self-clears; bit 31 AHBIDLE reads 1 |
//! | 0x014  | GINTSTS   | R/W1C | interrupt status (CURMOD/USBRST/ENUMDONE/RXFLVL...) |
//! | 0x018  | GINTMSK   | R/W | interrupt mask |
//! | 0x01C  | GRXSTSR   | RO  | receive status debug read (top of FIFO, 0 — no PHY) |
//! | 0x020  | GRXSTSP   | RO  | receive status pop (0 — no PHY) |
//! | 0x024  | GRXFSIZ   | R/W | receive FIFO size |
//! | 0x028  | GNPTXFSIZ | R/W | non-periodic TX FIFO size |
//! | 0x02C  | GNPTXSTS  | RO  | non-periodic TX FIFO/queue status (always "space available") |
//! | 0x040  | GSNPSID   | RO  | Synopsys release id constant ([`GSNPSID_VALUE`]) |
//! | 0x044  | GHWCFG1   | RO  | endpoint-direction capability word |
//! | 0x048  | GHWCFG2   | RO  | architecture / HS-PHY / #endpoints |
//! | 0x04C  | GHWCFG3   | RO  | FIFO depth / feature word |
//! | 0x050  | GHWCFG4   | RO  | misc capability word |
//! | 0x800  | DCFG      | R/W | device config |
//! | 0x804  | DCTL      | R/W | device control; bit 1 SFTDISCON |
//! | 0x808  | DSTS      | RO  | device status; ENUMSPD = full speed, not suspended |
//! | 0x818  | DAINT     | RO  | device all-endpoints interrupt (0 — no traffic) |
//! | 0x81C  | DAINTMSK  | R/W | device all-endpoints interrupt mask |
//! | 0x820  | DAENA*    | R/W | (alias kept for round-trip; see note) |
//!
//! ## Liveness / no-PHY limitation
//!
//! **There is no real USB host, cable, or PHY attached in the simulator.**
//! We therefore model only *register liveness*, not packet traffic:
//!   * `GRSTCTL.CSRST` completes instantly (`AHBIDLE` always 1) so a reset
//!     poll loop exits immediately;
//!   * `GSNPSID`/`GHWCFG*` are fixed constants so capability probing works;
//!   * `GNPTXSTS`/`DSTS` advertise idle-but-ready status so a driver never
//!     spins waiting on FIFO space or a speed-enumeration that will not come
//!     from a phantom host;
//!   * after a core soft reset we *optionally* latch a benign
//!     `USBRST → ENUMDONE` sequence into `GINTSTS` (gated on `GINTMSK`) so a
//!     driver's enumeration state machine progresses one step instead of
//!     hanging. This is **synthetic** liveness, not a modelled bus reset.
//!
//! No FIFO data path, no actual endpoint transfers, and no token/SOF timing
//! are modelled. The block never touches any other peripheral.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::collections::HashMap;

/// `ETS_USB_INTR_SOURCE` — DWC OTG core interrupt (interrupt matrix source).
pub const USB_OTG_SOURCE: u32 = 23;

// ── Core Global register offsets (Synopsys DWC2 map; S3 TRM "USB OTG") ──
const GOTGCTL: u64 = 0x000;
const GOTGINT: u64 = 0x004;
const GAHBCFG: u64 = 0x008;
const GUSBCFG: u64 = 0x00C;
const GRSTCTL: u64 = 0x010;
const GINTSTS: u64 = 0x014;
const GINTMSK: u64 = 0x018;
const GRXSTSR: u64 = 0x01C;
const GRXSTSP: u64 = 0x020;
const GRXFSIZ: u64 = 0x024;
const GNPTXFSIZ: u64 = 0x028;
const GNPTXSTS: u64 = 0x02C;
const GSNPSID: u64 = 0x040;
const GHWCFG1: u64 = 0x044;
const GHWCFG2: u64 = 0x048;
const GHWCFG3: u64 = 0x04C;
const GHWCFG4: u64 = 0x050;

// ── Device-mode core register offsets ──
const DCFG: u64 = 0x800;
const DCTL: u64 = 0x804;
const DSTS: u64 = 0x808;
const DAINT: u64 = 0x818;
const DAINTMSK: u64 = 0x81C;

/// Size of the modelled MMIO window (one 4 KiB page from the core regs).
pub const WINDOW_SIZE: u64 = 0x1000;

// ── GRSTCTL bits ──
const GRSTCTL_CSRST: u32 = 1 << 0; // core soft reset (self-clearing)
const GRSTCTL_AHBIDLE: u32 = 1 << 31; // AHB master idle (reads 1)

// ── GAHBCFG bits ──
const GAHBCFG_GINTMSK: u32 = 1 << 0; // global interrupt enable

// ── GINTSTS / GINTMSK bits (subset we name) ──
// CURMOD/RXFLVL are part of the documented register-bit map; named for the
// integrator and tests even though the no-PHY model never latches them itself.
#[allow(dead_code)]
const GINTSTS_CURMOD: u32 = 1 << 0; // current mode of operation (1 = host)
#[allow(dead_code)]
const GINTSTS_RXFLVL: u32 = 1 << 4; // RX FIFO non-empty
const GINTSTS_USBRST: u32 = 1 << 12; // USB reset detected
const GINTSTS_ENUMDONE: u32 = 1 << 13; // enumeration done

/// Synopsys release id reported by `GSNPSID`. A plausible DWC2 release
/// ("OT" + version 4.00a) matching what TinyUSB's dwc2 port accepts.
pub const GSNPSID_VALUE: u32 = 0x4F54_400A;

// Hardware-capability words. Plausible full-speed S3 OTG values: device+host
// capable internal FS PHY core. The exact bitfields are vendor-configured;
// these are stable read-back constants for capability probing, not a claim
// of bit-perfect TRM fidelity.
pub const GHWCFG1_VALUE: u32 = 0x0000_0000;
pub const GHWCFG2_VALUE: u32 = 0x224D_D930;
pub const GHWCFG3_VALUE: u32 = 0x0200_1E58;
pub const GHWCFG4_VALUE: u32 = 0xDFF0_5E08;

/// `DSTS` value: ENUMSPD = full speed (`0b01` in bits[2:1]), not suspended.
const DSTS_FULL_SPEED: u32 = 0b01 << 1;

/// `GNPTXSTS` value: report ample non-periodic TX FIFO space + free queue
/// slots so a driver never blocks waiting on a phantom host to drain.
const GNPTXSTS_READY: u32 = 0x0008_0200;

/// ESP32-S3 DWC_otg USB 2.0 OTG core (full-speed), register-level twin.
pub struct Esp32s3UsbOtg {
    /// Interrupt-matrix source raised via `explicit_irqs` (= 23).
    source_id: u32,

    // ── Named global registers with semantics ──
    gotgctl: u32,
    gotgint: u32,
    gahbcfg: u32,
    gusbcfg: u32,
    gintsts: u32,
    gintmsk: u32,
    grxfsiz: u32,
    gnptxfsiz: u32,

    // ── Named device registers with semantics ──
    dcfg: u32,
    dctl: u32,
    daintmsk: u32,

    /// Lossless round-trip store for every other in-window word offset.
    other: HashMap<u64, u32>,

    /// Synthetic-liveness flag: a core soft reset was just performed; the
    /// next `tick()` latches the benign USBRST→ENUMDONE sequence.
    pending_reset_seq: bool,
    /// Has the ENUMDONE half of the synthetic sequence been latched yet?
    enumdone_latched: bool,
}

impl std::fmt::Debug for Esp32s3UsbOtg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32s3UsbOtg(src={}, gintsts=0x{:08x}, gintmsk=0x{:08x}, gahbcfg=0x{:08x})",
            self.source_id, self.gintsts, self.gintmsk, self.gahbcfg,
        )
    }
}

impl Esp32s3UsbOtg {
    /// Construct the OTG core. `source_id` is the interrupt-matrix source the
    /// core asserts (the integrator passes `ETS_USB_INTR_SOURCE = 23`).
    pub fn new(source_id: u32) -> Self {
        Self {
            source_id,
            // Liveness defaults: force valid-session + device-mode connection
            // id so OTG capability reads look like an attached B-session.
            // CONIDSTS(bit16) | BSESVLD(bit19) | ASESVLD(bit18).
            gotgctl: (1 << 16) | (1 << 18) | (1 << 19),
            gotgint: 0,
            gahbcfg: 0,
            gusbcfg: 0,
            // CURMOD reflects current mode; default device mode (0). We keep a
            // stable value here and never spuriously toggle it.
            gintsts: 0,
            gintmsk: 0,
            grxfsiz: 0x0000_0100,
            gnptxfsiz: 0x0100_0100,
            dcfg: 0,
            dctl: 0,
            daintmsk: 0,
            other: HashMap::new(),
            pending_reset_seq: false,
            enumdone_latched: false,
        }
    }

    /// Side-effect-free 32-bit register read.
    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            GOTGCTL => self.gotgctl,
            GOTGINT => self.gotgint,
            GAHBCFG => self.gahbcfg,
            GUSBCFG => self.gusbcfg,
            // CSRST always reads back cleared (self-clearing); AHBIDLE = 1.
            GRSTCTL => GRSTCTL_AHBIDLE,
            GINTSTS => self.gintsts,
            GINTMSK => self.gintmsk,
            // No PHY: the receive-status FIFO is always empty.
            GRXSTSR | GRXSTSP => 0,
            GRXFSIZ => self.grxfsiz,
            GNPTXFSIZ => self.gnptxfsiz,
            // RO: always report TX space + queue slots available.
            GNPTXSTS => GNPTXSTS_READY,
            // RO capability constants.
            GSNPSID => GSNPSID_VALUE,
            GHWCFG1 => GHWCFG1_VALUE,
            GHWCFG2 => GHWCFG2_VALUE,
            GHWCFG3 => GHWCFG3_VALUE,
            GHWCFG4 => GHWCFG4_VALUE,
            DCFG => self.dcfg,
            DCTL => self.dctl,
            // RO: full-speed, not suspended.
            DSTS => DSTS_FULL_SPEED,
            // RO: no endpoint interrupts pending (no traffic).
            DAINT => 0,
            DAINTMSK => self.daintmsk,
            _ => self.other.get(&offset).copied().unwrap_or(0),
        }
    }

    /// 32-bit register write with the per-register semantics.
    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            // Keep the forced liveness bits asserted regardless of writes.
            GOTGCTL => {
                self.gotgctl = value | (1 << 16) | (1 << 18) | (1 << 19);
            }
            // GOTGINT is W1C.
            GOTGINT => self.gotgint &= !value,
            GAHBCFG => self.gahbcfg = value,
            GUSBCFG => self.gusbcfg = value,
            GRSTCTL => {
                // CSRST self-clears: accept the request, schedule the benign
                // USBRST→ENUMDONE liveness sequence, but never store the bit.
                if value & GRSTCTL_CSRST != 0 {
                    self.pending_reset_seq = true;
                    self.enumdone_latched = false;
                }
                // All other GRSTCTL bits are momentary/self-clearing too; we
                // intentionally do not retain them (reads return AHBIDLE only).
            }
            // GINTSTS is W1C: writing 1 clears the corresponding pending bit.
            GINTSTS => self.gintsts &= !value,
            GINTMSK => self.gintmsk = value,
            GRXFSIZ => self.grxfsiz = value,
            GNPTXFSIZ => self.gnptxfsiz = value,
            DCFG => self.dcfg = value,
            DCTL => self.dctl = value,
            DAINTMSK => self.daintmsk = value,
            // Read-only registers: ignore writes (no error, no store) so a
            // driver that does a blind read-modify-write does not corrupt them.
            GRXSTSR | GRXSTSP | GNPTXSTS | GSNPSID | GHWCFG1 | GHWCFG2 | GHWCFG3 | GHWCFG4
            | DSTS | DAINT => {}
            // Everything else in-window: lossless round-trip.
            _ => {
                self.other.insert(offset, value);
            }
        }
    }
}

impl Peripheral for Esp32s3UsbOtg {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        // Byte-granular RMW into the coherent 32-bit view: reconstruct the
        // current word, splice in this byte, re-apply word semantics. This
        // makes W1C and self-clearing bits behave correctly under both byte
        // and word access.
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Synthetic liveness: after a core soft reset, drive the enumeration
        // state machine forward one step per tick so a polling driver does not
        // hang waiting for a phantom host. USBRST first, then ENUMDONE.
        if self.pending_reset_seq {
            if !self.enumdone_latched {
                self.gintsts |= GINTSTS_USBRST;
                self.enumdone_latched = true;
            } else {
                self.gintsts |= GINTSTS_ENUMDONE;
                self.pending_reset_seq = false;
            }
        }

        // Level-sensitive IRQ: assert source 23 while any unmasked status bit
        // is pending AND the global interrupt enable (GAHBCFG.GINTMSK) is set.
        let pending = self.gintsts & self.gintmsk;
        let global_enabled = self.gahbcfg & GAHBCFG_GINTMSK != 0;
        let explicit_irqs = if pending != 0 && global_enabled {
            Some(vec![self.source_id])
        } else {
            None
        };

        PeripheralTickResult {
            explicit_irqs,
            ..PeripheralTickResult::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev() -> Esp32s3UsbOtg {
        Esp32s3UsbOtg::new(USB_OTG_SOURCE)
    }

    #[test]
    fn source_id_is_23() {
        assert_eq!(USB_OTG_SOURCE, 23);
        assert_eq!(dev().source_id, 23);
    }

    #[test]
    fn grstctl_csrst_self_clears_and_ahbidle_reads_1() {
        let mut p = dev();
        // Request core soft reset.
        p.write_word(GRSTCTL, GRSTCTL_CSRST);
        let v = p.read_word(GRSTCTL);
        // CSRST must read back 0 (self-cleared), AHBIDLE (bit31) must be 1.
        assert_eq!(v & GRSTCTL_CSRST, 0, "CSRST should self-clear");
        assert_eq!(v & GRSTCTL_AHBIDLE, GRSTCTL_AHBIDLE, "AHBIDLE must read 1");
        // And AHBIDLE is set even with no reset issued.
        assert_eq!(dev().read_word(GRSTCTL), GRSTCTL_AHBIDLE);
    }

    #[test]
    fn gsnpsid_and_ghwcfg_are_readonly_constants() {
        let mut p = dev();
        assert_eq!(p.read_word(GSNPSID), GSNPSID_VALUE);
        assert_eq!(p.read_word(GHWCFG1), GHWCFG1_VALUE);
        assert_eq!(p.read_word(GHWCFG2), GHWCFG2_VALUE);
        assert_eq!(p.read_word(GHWCFG3), GHWCFG3_VALUE);
        assert_eq!(p.read_word(GHWCFG4), GHWCFG4_VALUE);
        // Writes must not change them.
        p.write_word(GSNPSID, 0xDEAD_BEEF);
        p.write_word(GHWCFG2, 0x0);
        assert_eq!(p.read_word(GSNPSID), GSNPSID_VALUE);
        assert_eq!(p.read_word(GHWCFG2), GHWCFG2_VALUE);
    }

    #[test]
    fn gintsts_is_w1c() {
        let mut p = dev();
        // Simulate pending bits by driving the synthetic reset sequence.
        p.gintsts = GINTSTS_USBRST | GINTSTS_ENUMDONE | GINTSTS_RXFLVL;
        // Writing 1s clears only the written bits.
        p.write_word(GINTSTS, GINTSTS_USBRST);
        assert_eq!(p.read_word(GINTSTS) & GINTSTS_USBRST, 0, "USBRST cleared");
        assert_eq!(
            p.read_word(GINTSTS) & GINTSTS_ENUMDONE,
            GINTSTS_ENUMDONE,
            "ENUMDONE still set"
        );
        // Writing 0 must NOT clear a pending bit (W1C semantics).
        p.write_word(GINTSTS, 0);
        assert_eq!(p.read_word(GINTSTS) & GINTSTS_ENUMDONE, GINTSTS_ENUMDONE);
    }

    #[test]
    fn irq_only_when_globally_and_locally_unmasked() {
        let mut p = dev();
        p.gintsts = GINTSTS_ENUMDONE;

        // Neither global nor local enable: no IRQ.
        assert!(p.tick().explicit_irqs.is_none());

        // Local mask set, global enable clear: still no IRQ.
        p.gintmsk = GINTSTS_ENUMDONE;
        assert!(p.tick().explicit_irqs.is_none(), "global GINTMSK gates it");

        // Global enable set, but local mask for a different bit: no IRQ.
        p.gahbcfg = GAHBCFG_GINTMSK;
        p.gintmsk = GINTSTS_RXFLVL;
        assert!(p.tick().explicit_irqs.is_none(), "local mask gates it");

        // Both global + local unmasked and bit pending: IRQ on source 23.
        p.gintmsk = GINTSTS_ENUMDONE;
        let r = p.tick();
        assert_eq!(r.explicit_irqs.as_deref(), Some(&[USB_OTG_SOURCE][..]));

        // Clear the pending bit (W1C): IRQ deasserts.
        p.write_word(GINTSTS, GINTSTS_ENUMDONE);
        assert!(p.tick().explicit_irqs.is_none(), "deasserts after W1C ack");
    }

    #[test]
    fn enumdone_latch_on_tick_after_reset() {
        let mut p = dev();
        assert_eq!(p.read_word(GINTSTS), 0);
        // Issue a core soft reset.
        p.write_word(GRSTCTL, GRSTCTL_CSRST);
        // First tick latches USBRST.
        p.tick();
        assert_eq!(
            p.read_word(GINTSTS) & GINTSTS_USBRST,
            GINTSTS_USBRST,
            "USBRST latched on first tick after reset"
        );
        // Second tick latches ENUMDONE.
        p.tick();
        assert_eq!(
            p.read_word(GINTSTS) & GINTSTS_ENUMDONE,
            GINTSTS_ENUMDONE,
            "ENUMDONE latched on second tick"
        );
        // Sequence complete: a third tick adds nothing new.
        let before = p.read_word(GINTSTS);
        p.tick();
        assert_eq!(p.read_word(GINTSTS), before);
    }

    #[test]
    fn round_trip_unmodelled_offsets_lossless() {
        let mut p = dev();
        // A bunch of in-window offsets with no explicit semantics.
        for off in [0x100u64, 0x200, 0x500, 0x900, 0xA00, 0xFFC] {
            p.write_word(off, 0xCAFE_0000 | off as u32);
            assert_eq!(p.read_word(off), 0xCAFE_0000 | off as u32, "off 0x{off:x}");
        }
        // R/W named registers round-trip too.
        p.write_word(GUSBCFG, 0x1234_5678);
        assert_eq!(p.read_word(GUSBCFG), 0x1234_5678);
        p.write_word(DCFG, 0x0000_0A55);
        assert_eq!(p.read_word(DCFG), 0x0000_0A55);
    }

    #[test]
    fn byte_access_composes_into_word() {
        let mut p = dev();
        // Write GINTMSK one byte at a time.
        p.write(GINTMSK, 0x78).unwrap();
        p.write(GINTMSK + 1, 0x56).unwrap();
        p.write(GINTMSK + 2, 0x34).unwrap();
        p.write(GINTMSK + 3, 0x12).unwrap();
        assert_eq!(p.read_word(GINTMSK), 0x1234_5678);
        // And byte reads decompose the word (little-endian).
        assert_eq!(p.read(GINTMSK).unwrap(), 0x78);
        assert_eq!(p.read(GINTMSK + 1).unwrap(), 0x56);
        assert_eq!(p.read(GINTMSK + 2).unwrap(), 0x34);
        assert_eq!(p.read(GINTMSK + 3).unwrap(), 0x12);
        // GSNPSID read via the byte path returns the constant LE bytes.
        assert_eq!(p.read(GSNPSID).unwrap(), (GSNPSID_VALUE & 0xFF) as u8);
        assert_eq!(
            p.read(GSNPSID + 3).unwrap(),
            ((GSNPSID_VALUE >> 24) & 0xFF) as u8
        );
    }

    #[test]
    fn byte_w1c_clears_correct_bits() {
        let mut p = dev();
        p.gintsts = 0xFFFF_FFFF;
        // Clear bit 12 (USBRST) via a byte write to GINTSTS+1 (bits 8..15).
        p.write(GINTSTS + 1, 1 << (12 - 8)).unwrap();
        assert_eq!(p.read_word(GINTSTS) & GINTSTS_USBRST, 0);
        // Bit 13 (ENUMDONE) in the same byte was written 0 -> untouched.
        assert_eq!(p.read_word(GINTSTS) & GINTSTS_ENUMDONE, GINTSTS_ENUMDONE);
    }

    #[test]
    fn dsts_and_gnptxsts_report_ready() {
        let p = dev();
        // Full speed enum, not suspended.
        assert_eq!(p.read_word(DSTS), DSTS_FULL_SPEED);
        // Non-periodic TX FIFO reports space available.
        assert_eq!(p.read_word(GNPTXSTS), GNPTXSTS_READY);
    }

    #[test]
    fn no_irq_without_pending_even_if_enabled() {
        let mut p = dev();
        p.gahbcfg = GAHBCFG_GINTMSK;
        p.gintmsk = 0xFFFF_FFFF;
        // GINTSTS is 0 -> no pending -> no IRQ.
        assert!(p.tick().explicit_irqs.is_none());
    }
}
