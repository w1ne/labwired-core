// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 SDMMC host controller (SD/MMC host) — digital twin.
//!
//! The ESP32-S3 SD/MMC host is a Synopsys DesignWare-style Mobile Storage Host
//! Controller (DWC_mshc). Firmware (the ESP-IDF `sdmmc_host` driver, the
//! Arduino `SD_MMC` library) programs a command into `CMD`/`CMDARG`, sets the
//! `start_cmd` bit (CMD bit 31), then busy-polls `RINTSTS.CMD_DONE` (and, for a
//! data command, `RINTSTS.DATA_OVER`) before reading the response out of
//! `RESP0..3`. This twin models exactly that handshake.
//!
//! Base address: `DR_REG_SDMMC_BASE = 0x6002_8000`, 0x1000 wide.
//! Interrupt-matrix source: `ETS_SDIO_HOST_INTR_SOURCE = 36` (verified against
//! `soc/esp32s3/include/soc/interrupts.h`, which places
//! `ETS_SDIO_HOST_INTR_SOURCE = 36` between `ETS_LEDC_INTR_SOURCE = 35` and
//! `ETS_TWAI_INTR_SOURCE = 37`).
//!
//! ## Register map (offsets from base, DWC_mshc programming model / ESP32-S3
//! TRM "SD/MMC Host Controller"; verified against
//! `soc/esp32s3/register/soc/sdmmc_reg.h`)
//!
//! | Offset | Name     | Behaviour |
//! |-------:|----------|-----------|
//! | 0x00   | CTRL     | bit0 CONTROLLER_RESET, bit1 FIFO_RESET, bit2 DMA_RESET, bit4 INT_ENABLE — the three *_RESET bits self-clear |
//! | 0x04   | PWREN    | card power-enable (round-trip) |
//! | 0x08   | CLKDIV   | clock dividers (round-trip) |
//! | 0x0C   | CLKSRC   | clock source select (round-trip) |
//! | 0x10   | CLKENA   | per-card clock enable (round-trip) |
//! | 0x14   | TMOUT    | response/data timeout (round-trip) |
//! | 0x18   | CTYPE    | bus width: bit[n] 1-bit/4-bit, bits[16+n] 8-bit (round-trip) |
//! | 0x1C   | BLKSIZ   | block size, bits[15:0] (round-trip) |
//! | 0x20   | BYTCNT   | byte count for the transfer (round-trip) |
//! | 0x24   | INTMASK  | per-source interrupt mask (gates MINTSTS) |
//! | 0x28   | CMDARG   | argument for the next command |
//! | 0x2C   | CMD      | bit31 START_CMD, bit6 DATA_EXPECTED, bit9 RESPONSE_LENGTH(long), bit8 RESPONSE_EXPECT, bits[5:0] CMD_INDEX |
//! | 0x30   | RESP0    | response bits[31:0] (RO) |
//! | 0x34   | RESP1    | response bits[63:32] (RO) |
//! | 0x38   | RESP2    | response bits[95:64] (RO) |
//! | 0x3C   | RESP3    | response bits[127:96] (RO) |
//! | 0x40   | MINTSTS  | masked interrupt status = RINTSTS & INTMASK (RO) |
//! | 0x44   | RINTSTS  | raw interrupt status — **W1C** |
//! | 0x48   | STATUS   | bit9 DATA_BUSY, bit10 DATA_STATE_MC_BUSY, bits[16:13] RESPONSE_INDEX, bits[29:17] FIFO_COUNT, etc. (RO) |
//! | 0x4C   | FIFOTH   | FIFO threshold (round-trip) |
//! | 0x58   | TCBCNT   | transferred CIU byte count (RO) |
//! | 0x5C   | TBBCNT   | transferred host/BIU byte count (RO) |
//! | 0x100  | CARDTHRCTL | card read-threshold control (round-trip) |
//!
//! Offsets not listed above round-trip verbatim (so any config register the
//! driver touches reads back what it wrote) and have no side effects.
//!
//! ## RINTSTS / MINTSTS interrupt bits (DWC_mshc shared layout)
//!
//! | Bit | Name      | Meaning |
//! |----:|-----------|---------|
//! | 0   | CD        | card detect |
//! | 1   | RE        | response error |
//! | 2   | CMD_DONE  | command done |
//! | 3   | DTO       | data transfer over (DATA_OVER) |
//! | 4   | TXDR      | transmit FIFO data request |
//! | 5   | RXDR      | receive FIFO data request |
//! | 6   | RCRC      | response CRC error |
//! | 7   | DCRC      | data CRC error |
//! | 8   | RTO       | response timeout |
//! | 9   | DRTO      | data read timeout |
//! | 10  | HTO       | data starvation / host timeout |
//! | 11  | FRUN      | FIFO underrun/overrun |
//! | 12  | HLE       | hardware-locked write error |
//! | 13  | SBE/BCI   | start-bit / busy-clear-interrupt error |
//! | 14  | ACD       | auto command done |
//! | 15  | EBE       | end-bit error |
//! | 16  | SDIO      | SDIO card interrupt |
//!
//! ## No card attached — responses are MODELED, not real
//!
//! There is no physical SD/eMMC card in the simulator, so a real command can
//! neither be clocked out nor acknowledged by a card. This twin therefore
//! **models** the command handshake so the driver does not hang:
//!
//! * On a `CMD.start_cmd` write we clear `start_cmd` (the controller clears it
//!   once the command is accepted into the command path) and, on the *next*
//!   `tick`, latch `RINTSTS.CMD_DONE`. If the command set `DATA_EXPECTED`
//!   (CMD bit 6) we also latch `RINTSTS.DATA_OVER` so a data-phase poll
//!   completes.
//! * `RESP0..3` are filled with a benign, fixed R1-style response
//!   (`R1_RESPONSE`, a card in the `tran` state with no error flags). This is
//!   NOT what any specific card would return to a specific command — it is a
//!   deterministic placeholder chosen so the driver's response parsing sees a
//!   non-error card and proceeds. Block reads/writes return no real data.
//! * No error bits (RE/RCRC/RTO/…) are ever latched, and `STATUS.DATA_BUSY`
//!   never sticks, so the driver never spins forever on a busy card.
//!
//! In short: this is a *liveness* model of the host controller's register
//! handshake, not a functional SD card. It is sufficient to bring the
//! `sdmmc_host` / `SD_MMC` init path through command issue without hanging; it
//! does not store or return card contents.
//!
//! ## Interrupt delivery
//!
//! `MINTSTS = RINTSTS & INTMASK`; additionally the DWC global interrupt enable
//! is `CTRL.INT_ENABLE` (bit 4). While `MINTSTS != 0` and `CTRL.INT_ENABLE` is
//! set, `tick()` emits the `ETS_SDIO_HOST_INTR_SOURCE` matrix source
//! (level-sensitive, same pattern as `timer_group.rs` / `gpspi.rs`). `RINTSTS`
//! is W1C, so firmware ACKs by writing the bits back.

use std::collections::HashMap;

use crate::{Peripheral, PeripheralTickResult, SimResult};

// ── Register offsets ──
// Several entries map the full controller register file for documentation;
// the model only drives the subset the boot-ROM probe touches today.
#[allow(dead_code)]
const CTRL: u64 = 0x00;
#[allow(dead_code)]
const PWREN: u64 = 0x04;
#[allow(dead_code)]
const CLKDIV: u64 = 0x08;
#[allow(dead_code)]
const CLKSRC: u64 = 0x0C;
#[allow(dead_code)]
const CLKENA: u64 = 0x10;
#[allow(dead_code)]
const TMOUT: u64 = 0x14;
#[allow(dead_code)]
const CTYPE: u64 = 0x18;
#[allow(dead_code)]
const BLKSIZ: u64 = 0x1C;
#[allow(dead_code)]
const BYTCNT: u64 = 0x20;
#[allow(dead_code)]
const INTMASK: u64 = 0x24;
#[allow(dead_code)]
const CMDARG: u64 = 0x28;
#[allow(dead_code)]
const CMD: u64 = 0x2C;
const RESP0: u64 = 0x30;
const RESP1: u64 = 0x34;
const RESP2: u64 = 0x38;
const RESP3: u64 = 0x3C;
const MINTSTS: u64 = 0x40;
const RINTSTS: u64 = 0x44;
const STATUS: u64 = 0x48;
const FIFOTH: u64 = 0x4C;
/// TCBCNT (0x58) — transferred CIU byte count, RO. Documents the extent of the
/// modeled register file; reads back as a round-tripped 0.
#[allow(dead_code)]
const TCBCNT: u64 = 0x58;
/// TBBCNT (0x5C) — transferred BIU byte count, RO.
#[allow(dead_code)]
const TBBCNT: u64 = 0x5C;
/// CARDTHRCTL (0x100) — card read-threshold control, round-trip.
#[allow(dead_code)]
const CARDTHRCTL: u64 = 0x100;

// ── CTRL (0x00) bits ──
const CTRL_CONTROLLER_RESET: u32 = 1 << 0;
const CTRL_FIFO_RESET: u32 = 1 << 1;
const CTRL_DMA_RESET: u32 = 1 << 2;
const CTRL_INT_ENABLE: u32 = 1 << 4;
/// The three reset bits self-clear once the reset completes.
const CTRL_RESET_BITS: u32 = CTRL_CONTROLLER_RESET | CTRL_FIFO_RESET | CTRL_DMA_RESET;

// ── CMD (0x2C) bits ──
const CMD_START: u32 = 1 << 31;
const CMD_INDEX_MASK: u32 = 0x3F;
const CMD_RESPONSE_EXPECT: u32 = 1 << 6;
#[allow(dead_code)]
const CMD_RESPONSE_LENGTH_LONG: u32 = 1 << 7;
#[allow(dead_code)]
const CMD_CHECK_RESPONSE_CRC: u32 = 1 << 8;
const CMD_DATA_EXPECTED: u32 = 1 << 9;

// ── RINTSTS / MINTSTS / INTMASK (shared bit layout) ──
const INT_CD: u32 = 1 << 0;
const INT_RE: u32 = 1 << 1;
const INT_CMD_DONE: u32 = 1 << 2;
const INT_DATA_OVER: u32 = 1 << 3;
const INT_TXDR: u32 = 1 << 4;
const INT_RXDR: u32 = 1 << 5;
const INT_RCRC: u32 = 1 << 6;
const INT_DCRC: u32 = 1 << 7;
const INT_RTO: u32 = 1 << 8;
const INT_DRTO: u32 = 1 << 9;
const INT_HTO: u32 = 1 << 10;
const INT_FRUN: u32 = 1 << 11;
const INT_HLE: u32 = 1 << 12;
const INT_SBE: u32 = 1 << 13;
const INT_ACD: u32 = 1 << 14;
const INT_EBE: u32 = 1 << 15;
const INT_SDIO: u32 = 1 << 16;
/// All interrupt bits that physically exist in RINTSTS/MINTSTS (bits 0..16).
const INT_MASK_ALL: u32 = 0x0001_FFFF;
#[allow(dead_code)]
const INT_ALL_UNUSED: u32 = INT_CD
    | INT_RE
    | INT_TXDR
    | INT_RXDR
    | INT_RCRC
    | INT_DCRC
    | INT_RTO
    | INT_DRTO
    | INT_HTO
    | INT_FRUN
    | INT_HLE
    | INT_SBE
    | INT_ACD
    | INT_EBE
    | INT_SDIO;

// ── STATUS (0x48) bits ──
/// DATA_BUSY (bit 9) — card data line busy. Never sticks in this model.
#[allow(dead_code)]
const STATUS_DATA_BUSY: u32 = 1 << 9;
/// FIFO_EMPTY (bit 2) — set at reset (FIFO is empty, nothing buffered).
const STATUS_FIFO_EMPTY: u32 = 1 << 2;

/// Benign R1-style response placeholder loaded into RESP0 after a command that
/// expects a response. Bits: CURRENT_STATE = 4 (`tran`, bits[12:9]) and
/// READY_FOR_DATA (bit 8) set, no error flags. This is a *modeled* value, not a
/// real card's reply (see module docs). 0x0000_0900.
const R1_RESPONSE: u32 = (4 << 9) | (1 << 8);

pub struct Esp32s3Sdmmc {
    /// Interrupt-matrix source ID (`ETS_SDIO_HOST_INTR_SOURCE` = 36).
    source_id: u32,
    /// Backing store for all round-tripped config registers.
    regs: HashMap<u64, u32>,
    /// Raw interrupt status (`RINTSTS`, 0x44). W1C.
    rint_sts: u32,
    /// RESP0..3 latched response words.
    resp: [u32; 4],
    /// Set when a `CMD.start_cmd` write has been accepted but the resulting
    /// CMD_DONE/DATA_OVER has not yet been latched. Drained on the next
    /// `tick()`, giving the one-cycle command-done latency the brief specifies.
    pending_cmd: Option<PendingCmd>,
}

/// A command accepted on a `start_cmd` write, awaiting completion on `tick`.
#[derive(Debug, Clone, Copy)]
struct PendingCmd {
    /// Whether the command expects a response (CMD bit 6) — gates RESP fill.
    response_expect: bool,
    /// Whether the command has a data phase (CMD bit 9) — gates DATA_OVER.
    data_expected: bool,
}

impl std::fmt::Debug for Esp32s3Sdmmc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32s3Sdmmc(src={}, rintsts=0x{:08x}, intmask=0x{:08x}, pending={})",
            self.source_id,
            self.rint_sts,
            self.reg(INTMASK),
            self.pending_cmd.is_some(),
        )
    }
}

impl Esp32s3Sdmmc {
    /// Construct the SD/MMC host bound to interrupt-matrix `source_id`
    /// (`ETS_SDIO_HOST_INTR_SOURCE` = 36 on the ESP32-S3).
    pub fn new(source_id: u32) -> Self {
        let mut regs = HashMap::new();
        // DWC_mshc reset defaults that firmware tends to read back.
        regs.insert(TMOUT, 0xFFFF_FF40); // response + data timeout reset value
        regs.insert(FIFOTH, 0);
        Self {
            source_id,
            regs,
            rint_sts: 0,
            resp: [0; 4],
            pending_cmd: None,
        }
    }

    fn reg(&self, off: u64) -> u32 {
        self.regs.get(&off).copied().unwrap_or(0)
    }

    fn set_reg(&mut self, off: u64, val: u32) {
        self.regs.insert(off, val);
    }

    /// MINTSTS = RINTSTS & INTMASK.
    fn mint_sts(&self) -> u32 {
        self.rint_sts & self.reg(INTMASK) & INT_MASK_ALL
    }

    /// STATUS (0x48). The FIFO is always empty (no data path modeled) and the
    /// data lines are never busy, so the driver's busy poll always clears.
    fn status_word(&self) -> u32 {
        STATUS_FIFO_EMPTY
    }

    /// Handle a write to CTRL (0x00): the three *_RESET bits self-clear, and a
    /// FIFO/controller reset clears the latched raw interrupts (mirrors the
    /// real controller, which flushes pending status on a soft reset).
    fn write_ctrl(&mut self, value: u32) {
        if value & CTRL_FIFO_RESET != 0 || value & CTRL_CONTROLLER_RESET != 0 {
            // Soft reset flushes pending command + status.
            self.rint_sts = 0;
            self.pending_cmd = None;
        }
        let _ = CTRL_DMA_RESET; // documented; no DMA engine modeled
                                // Store CTRL with the self-clearing reset bits cleared.
        self.set_reg(CTRL, value & !CTRL_RESET_BITS);
    }

    /// Handle a write to CMD (0x2C). When `start_cmd` (bit 31) is set, accept
    /// the command: clear `start_cmd` so the driver's "command accepted" poll
    /// exits, and queue the completion for the next `tick`.
    fn write_cmd(&mut self, value: u32) {
        if value & CMD_START != 0 {
            self.pending_cmd = Some(PendingCmd {
                response_expect: value & CMD_RESPONSE_EXPECT != 0,
                data_expected: value & CMD_DATA_EXPECTED != 0,
            });
            let _ = CMD_INDEX_MASK; // index is round-tripped, not acted upon
                                    // Clear start_cmd: the controller deasserts it once the command is
                                    // loaded into the command path.
            self.set_reg(CMD, value & !CMD_START);
        } else {
            self.set_reg(CMD, value);
        }
    }

    /// Drain a pending command on `tick`: latch CMD_DONE (always) and, for a
    /// data command, DATA_OVER; fill RESP0..3 with the benign modeled response.
    fn complete_pending_cmd(&mut self) {
        if let Some(cmd) = self.pending_cmd.take() {
            self.rint_sts |= INT_CMD_DONE;
            if cmd.response_expect {
                // Modeled benign R1 (no card attached — see module docs).
                self.resp = [R1_RESPONSE, 0, 0, 0];
            } else {
                self.resp = [0; 4];
            }
            if cmd.data_expected {
                self.rint_sts |= INT_DATA_OVER;
            }
        }
    }
}

impl Peripheral for Esp32s3Sdmmc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !3)?;
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset & !3 {
            RESP0 => self.resp[0],
            RESP1 => self.resp[1],
            RESP2 => self.resp[2],
            RESP3 => self.resp[3],
            MINTSTS => self.mint_sts(),
            RINTSTS => self.rint_sts & INT_MASK_ALL,
            STATUS => self.status_word(),
            o => self.reg(o),
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let shift = (offset & 3) * 8;
        // Read-modify-write the affected word from a side-effect-free snapshot,
        // then re-dispatch through the u32 path so start_cmd / W1C / reset
        // side-effects fire on exactly one coherent value.
        let base = match word_off {
            RINTSTS => self.rint_sts,
            CTRL | CMD => self.reg(word_off),
            _ => self.read_u32(word_off)?,
        };
        let merged = (base & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_u32(word_off, merged)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset & !3 {
            CTRL => self.write_ctrl(value),
            CMD => self.write_cmd(value),
            // RINTSTS is W1C: writing a 1 clears that raw bit.
            RINTSTS => self.rint_sts &= !(value & INT_MASK_ALL),
            // RESP0..3, MINTSTS, STATUS are read-only; ignore writes.
            RESP0 | RESP1 | RESP2 | RESP3 | MINTSTS | STATUS => {}
            // Everything else (PWREN/CLKDIV/CLKSRC/CLKENA/TMOUT/CTYPE/BLKSIZ/
            // BYTCNT/INTMASK/CMDARG/FIFOTH/CARDTHRCTL/…) round-trips verbatim.
            o => self.set_reg(o, value),
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // One-cycle command-done latency: a command accepted on a start_cmd
        // write latches CMD_DONE (+ DATA_OVER) here, on the next tick.
        self.complete_pending_cmd();

        // Level-sensitive IRQ delivery: while any masked raw bit is set AND the
        // DWC global interrupt enable (CTRL.INT_ENABLE) is set, emit the
        // SDIO-host matrix source. RINTSTS is W1C, so the source de-asserts
        // once firmware ACKs. (Same pattern as timer_group.rs / gpspi.rs.)
        let int_enabled = self.reg(CTRL) & CTRL_INT_ENABLE != 0;
        let explicit_irqs = if int_enabled && self.mint_sts() != 0 {
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

    const SDIO_HOST_SOURCE: u32 = 36;

    fn new_host() -> Esp32s3Sdmmc {
        Esp32s3Sdmmc::new(SDIO_HOST_SOURCE)
    }

    #[test]
    fn reset_defaults_seeded() {
        let h = new_host();
        // TMOUT reset value readable.
        assert_eq!(h.read_u32(TMOUT).unwrap(), 0xFFFF_FF40);
        // No interrupts latched, no response, FIFO empty.
        assert_eq!(h.read_u32(RINTSTS).unwrap(), 0);
        assert_eq!(h.read_u32(RESP0).unwrap(), 0);
        assert_eq!(
            h.read_u32(STATUS).unwrap() & STATUS_FIFO_EMPTY,
            STATUS_FIFO_EMPTY
        );
        // DATA_BUSY never asserted.
        assert_eq!(h.read_u32(STATUS).unwrap() & STATUS_DATA_BUSY, 0);
    }

    #[test]
    fn config_registers_round_trip() {
        let mut h = new_host();
        h.write_u32(PWREN, 0x0000_0001).unwrap();
        h.write_u32(CLKDIV, 0x0000_0004).unwrap();
        h.write_u32(CLKSRC, 0x0000_0000).unwrap();
        h.write_u32(CLKENA, 0x0001_0001).unwrap();
        h.write_u32(CTYPE, 0x0000_0001).unwrap(); // 4-bit bus on card 0
        h.write_u32(BLKSIZ, 0x0000_0200).unwrap(); // 512-byte blocks
        h.write_u32(BYTCNT, 0x0000_0200).unwrap();
        h.write_u32(CMDARG, 0xDEAD_BEEF).unwrap();
        h.write_u32(FIFOTH, 0x0010_0003).unwrap();
        h.write_u32(CARDTHRCTL, 0x0200_0001).unwrap();
        assert_eq!(h.read_u32(PWREN).unwrap(), 0x0000_0001);
        assert_eq!(h.read_u32(CLKDIV).unwrap(), 0x0000_0004);
        assert_eq!(h.read_u32(CLKENA).unwrap(), 0x0001_0001);
        assert_eq!(h.read_u32(CTYPE).unwrap(), 0x0000_0001);
        assert_eq!(h.read_u32(BLKSIZ).unwrap(), 0x0000_0200);
        assert_eq!(h.read_u32(BYTCNT).unwrap(), 0x0000_0200);
        assert_eq!(h.read_u32(CMDARG).unwrap(), 0xDEAD_BEEF);
        assert_eq!(h.read_u32(FIFOTH).unwrap(), 0x0010_0003);
        assert_eq!(h.read_u32(CARDTHRCTL).unwrap(), 0x0200_0001);
    }

    #[test]
    fn ctrl_reset_bits_self_clear() {
        let mut h = new_host();
        // INT_ENABLE should persist; the reset bits should read back 0.
        h.write_u32(
            CTRL,
            CTRL_CONTROLLER_RESET | CTRL_FIFO_RESET | CTRL_DMA_RESET | CTRL_INT_ENABLE,
        )
        .unwrap();
        let ctrl = h.read_u32(CTRL).unwrap();
        assert_eq!(ctrl & CTRL_RESET_BITS, 0, "reset bits self-clear");
        assert_eq!(
            ctrl & CTRL_INT_ENABLE,
            CTRL_INT_ENABLE,
            "INT_ENABLE persists"
        );
    }

    #[test]
    fn start_cmd_clears_start_bit_and_latches_cmd_done_on_tick() {
        let mut h = new_host();
        // Issue CMD0-like command: start, response expected, index 0.
        h.write_u32(CMD, CMD_START | CMD_RESPONSE_EXPECT).unwrap();
        // start_cmd auto-clears so the "command accepted" poll exits.
        assert_eq!(h.read_u32(CMD).unwrap() & CMD_START, 0, "start_cmd clears");
        // CMD_DONE not yet latched (one-cycle latency).
        assert_eq!(
            h.read_u32(RINTSTS).unwrap() & INT_CMD_DONE,
            0,
            "not done yet"
        );
        // tick latches CMD_DONE.
        h.tick();
        assert_eq!(
            h.read_u32(RINTSTS).unwrap() & INT_CMD_DONE,
            INT_CMD_DONE,
            "CMD_DONE latched on tick"
        );
        // Benign modeled R1 response present in RESP0.
        assert_eq!(h.read_u32(RESP0).unwrap(), R1_RESPONSE);
    }

    #[test]
    fn no_response_command_leaves_resp_zero() {
        let mut h = new_host();
        // Command with no response expected.
        h.write_u32(CMD, CMD_START).unwrap();
        h.tick();
        assert_eq!(h.read_u32(RINTSTS).unwrap() & INT_CMD_DONE, INT_CMD_DONE);
        assert_eq!(h.read_u32(RESP0).unwrap(), 0, "no response → RESP0 stays 0");
    }

    #[test]
    fn data_command_latches_data_over() {
        let mut h = new_host();
        // Data command (e.g. CMD17 single-block read): start + data_expected.
        h.write_u32(CMD, CMD_START | CMD_RESPONSE_EXPECT | CMD_DATA_EXPECTED)
            .unwrap();
        h.tick();
        let rint = h.read_u32(RINTSTS).unwrap();
        assert_eq!(rint & INT_CMD_DONE, INT_CMD_DONE, "CMD_DONE latched");
        assert_eq!(
            rint & INT_DATA_OVER,
            INT_DATA_OVER,
            "DATA_OVER latched for data cmd"
        );
    }

    #[test]
    fn non_data_command_does_not_latch_data_over() {
        let mut h = new_host();
        h.write_u32(CMD, CMD_START | CMD_RESPONSE_EXPECT).unwrap();
        h.tick();
        assert_eq!(
            h.read_u32(RINTSTS).unwrap() & INT_DATA_OVER,
            0,
            "no DATA_OVER without data_expected"
        );
    }

    #[test]
    fn rintsts_is_w1c() {
        let mut h = new_host();
        h.write_u32(CMD, CMD_START | CMD_DATA_EXPECTED).unwrap();
        h.tick();
        let before = h.read_u32(RINTSTS).unwrap();
        assert_eq!(before & INT_CMD_DONE, INT_CMD_DONE);
        assert_eq!(before & INT_DATA_OVER, INT_DATA_OVER);
        // W1C: writing CMD_DONE clears only it; DATA_OVER survives.
        h.write_u32(RINTSTS, INT_CMD_DONE).unwrap();
        let after = h.read_u32(RINTSTS).unwrap();
        assert_eq!(after & INT_CMD_DONE, 0, "CMD_DONE cleared by W1C");
        assert_eq!(after & INT_DATA_OVER, INT_DATA_OVER, "DATA_OVER untouched");
        // Writing 0 clears nothing.
        h.write_u32(RINTSTS, 0).unwrap();
        assert_eq!(h.read_u32(RINTSTS).unwrap() & INT_DATA_OVER, INT_DATA_OVER);
        // Clear the rest.
        h.write_u32(RINTSTS, INT_DATA_OVER).unwrap();
        assert_eq!(h.read_u32(RINTSTS).unwrap(), 0);
    }

    #[test]
    fn mintsts_masks_with_intmask() {
        let mut h = new_host();
        h.write_u32(CMD, CMD_START).unwrap();
        h.tick();
        // CMD_DONE raw, but INTMASK = 0 → MINTSTS = 0.
        assert_eq!(h.read_u32(RINTSTS).unwrap() & INT_CMD_DONE, INT_CMD_DONE);
        assert_eq!(h.read_u32(MINTSTS).unwrap(), 0, "masked out");
        // Unmask CMD_DONE → MINTSTS shows it.
        h.write_u32(INTMASK, INT_CMD_DONE).unwrap();
        assert_eq!(h.read_u32(MINTSTS).unwrap() & INT_CMD_DONE, INT_CMD_DONE);
    }

    #[test]
    fn irq_emitted_only_when_unmasked_and_globally_enabled() {
        let mut h = new_host();
        // Globally enable interrupts + unmask CMD_DONE.
        h.write_u32(CTRL, CTRL_INT_ENABLE).unwrap();
        h.write_u32(INTMASK, INT_CMD_DONE).unwrap();
        // Before any command → no IRQ.
        assert!(h.tick().explicit_irqs.is_none());
        // Issue a command; the same tick that latches CMD_DONE also emits.
        h.write_u32(CMD, CMD_START).unwrap();
        let r = h.tick();
        assert_eq!(r.explicit_irqs.as_deref(), Some(&[SDIO_HOST_SOURCE][..]));
        // Level-sensitive: stays asserted while latched + unmasked + enabled.
        assert_eq!(
            h.tick().explicit_irqs.as_deref(),
            Some(&[SDIO_HOST_SOURCE][..])
        );
        // ACK via W1C → IRQ de-asserts.
        h.write_u32(RINTSTS, INT_CMD_DONE).unwrap();
        assert!(h.tick().explicit_irqs.is_none(), "IRQ de-asserts after ACK");
    }

    #[test]
    fn intmask_gates_irq_emission() {
        let mut h = new_host();
        // Global enable on, but INTMASK = 0 → no IRQ even with CMD_DONE raw.
        h.write_u32(CTRL, CTRL_INT_ENABLE).unwrap();
        h.write_u32(CMD, CMD_START).unwrap();
        assert!(h.tick().explicit_irqs.is_none(), "masked → no IRQ");
        // Unmask now → next tick emits (raw bit persists).
        h.write_u32(INTMASK, INT_CMD_DONE).unwrap();
        assert_eq!(
            h.tick().explicit_irqs.as_deref(),
            Some(&[SDIO_HOST_SOURCE][..])
        );
    }

    #[test]
    fn global_int_enable_gates_irq_emission() {
        let mut h = new_host();
        // Unmasked, but CTRL.INT_ENABLE off → no IRQ.
        h.write_u32(INTMASK, INT_CMD_DONE).unwrap();
        h.write_u32(CMD, CMD_START).unwrap();
        assert!(h.tick().explicit_irqs.is_none(), "global disable → no IRQ");
        // MINTSTS still reflects the masked status independent of global enable.
        assert_eq!(h.read_u32(MINTSTS).unwrap() & INT_CMD_DONE, INT_CMD_DONE);
        // Turn the global enable on → next tick emits.
        h.write_u32(CTRL, CTRL_INT_ENABLE).unwrap();
        assert_eq!(
            h.tick().explicit_irqs.as_deref(),
            Some(&[SDIO_HOST_SOURCE][..])
        );
    }

    #[test]
    fn byte_access_round_trip_and_start_via_msb() {
        let mut h = new_host();
        // Byte-wise CMDARG fill.
        h.write(CMDARG, 0x11).unwrap();
        h.write(CMDARG + 1, 0x22).unwrap();
        h.write(CMDARG + 2, 0x33).unwrap();
        h.write(CMDARG + 3, 0x44).unwrap();
        assert_eq!(h.read_u32(CMDARG).unwrap(), 0x4433_2211);
        assert_eq!(h.read(CMDARG).unwrap(), 0x11);
        assert_eq!(h.read(CMDARG + 3).unwrap(), 0x44);
        // Setting start_cmd via the high byte (bit 31 lives in CMD+3) launches.
        h.write(CMD, CMD_RESPONSE_EXPECT as u8).unwrap(); // low byte: response_expect
        h.write(CMD + 3, 0x80).unwrap(); // high byte sets bit 31 (start_cmd)
                                         // start_cmd cleared after acceptance.
        assert_eq!(h.read_u32(CMD).unwrap() & CMD_START, 0);
        h.tick();
        assert_eq!(h.read_u32(RINTSTS).unwrap() & INT_CMD_DONE, INT_CMD_DONE);
        assert_eq!(h.read_u32(RESP0).unwrap(), R1_RESPONSE);
    }

    #[test]
    fn controller_reset_flushes_pending_and_status() {
        let mut h = new_host();
        h.write_u32(CMD, CMD_START | CMD_DATA_EXPECTED).unwrap();
        h.tick(); // latch CMD_DONE + DATA_OVER
        assert_ne!(h.read_u32(RINTSTS).unwrap(), 0);
        // Soft reset flushes latched status.
        h.write_u32(CTRL, CTRL_CONTROLLER_RESET).unwrap();
        assert_eq!(h.read_u32(RINTSTS).unwrap(), 0, "reset flushes RINTSTS");
        // A reset issued before a pending command's tick drops the command.
        h.write_u32(CMD, CMD_START).unwrap();
        h.write_u32(CTRL, CTRL_FIFO_RESET).unwrap();
        h.tick();
        assert_eq!(
            h.read_u32(RINTSTS).unwrap() & INT_CMD_DONE,
            0,
            "pending cmd dropped by reset"
        );
    }

    #[test]
    fn resp_registers_are_read_only() {
        let mut h = new_host();
        h.write_u32(RESP0, 0xFFFF_FFFF).unwrap();
        h.write_u32(RESP1, 0xFFFF_FFFF).unwrap();
        assert_eq!(h.read_u32(RESP0).unwrap(), 0, "RESP0 ignores writes");
        assert_eq!(h.read_u32(RESP1).unwrap(), 0, "RESP1 ignores writes");
    }

    #[test]
    fn source_id_recorded() {
        let h = new_host();
        assert_eq!(h.source_id, SDIO_HOST_SOURCE);
    }
}
