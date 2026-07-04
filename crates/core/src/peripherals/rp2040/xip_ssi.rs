// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 XIP_SSI — Synopsys DW_apb_ssi flash controller in SPI master mode
//! (datasheet §4.10, base `0x18000000`).
//!
//! The RP2040 bootrom runs a 256-byte second-stage bootloader (boot2) that
//! programs this controller to talk to the on-board QSPI flash, then switches
//! it into execute-in-place (XIP) mode so the CPU can fetch instructions
//! straight out of the `0x10000000` flash window. The pico-sdk C runtime keeps
//! a copy of boot2 in RAM (`flash_init_boot2_copyout` /
//! `flash_enable_xip_via_boot2`) and re-runs it whenever it needs to touch
//! flash, so even an already-relocated application drives this block during
//! start-up. Without a model the boot2 status poll faults on the first read of
//! `SR` (offset `0x28`).
//!
//! This is a real transfer engine, not an address allow-list. Each write to the
//! data register (`DR0`, offset `0x60`) clocks one 8-bit frame out to the flash
//! and simultaneously clocks one frame back into the receive FIFO (full-duplex
//! SPI). `SR` (offset `0x28`) reports the genuine FIFO state — transmit FIFO
//! empty (`TFE`), transmit FIFO not full (`TFNF`), receive FIFO not empty
//! (`RFNE`) — and `BUSY` is never asserted because a modelled transfer
//! completes within the register write. The receive bytes come from a small
//! JEDEC/Winbond W25Q-style flash command responder (RDSR/RDSR2/WREN/WRSR) so
//! boot2's "read status register / set quad-enable / poll write-in-progress"
//! sequence runs to completion exactly as it does on silicon, rather than being
//! short-circuited with constants.

use crate::{Peripheral, SimResult};
use std::cell::RefCell;
use std::collections::VecDeque;

// SSI register offsets (datasheet §4.10.13 "List of Registers").
const SSIENR: u64 = 0x08; // SSI enable
const SR: u64 = 0x28; // status register
const DR0: u64 = 0x60; // data register 0 (TX/RX FIFO port)

// SR status-flag bit positions (datasheet §4.10.13, SR register).
const SR_BUSY: u32 = 1 << 0; // SSI busy (a transfer is in progress)
const SR_TFNF: u32 = 1 << 1; // transmit FIFO not full
const SR_TFE: u32 = 1 << 2; // transmit FIFO empty
const SR_RFNE: u32 = 1 << 3; // receive FIFO not empty

// SSIENR.SSI_EN (datasheet §4.10.13, SSIENR register).
const SSIENR_EN: u32 = 1 << 0;

// Serial-flash command opcodes and status bits (Winbond W25Q080 datasheet
// §8, the flash the standard RP2040 boot2 blob targets).
const CMD_WRSR: u8 = 0x01; // write status register (SR1, SR2)
const CMD_WRDI: u8 = 0x04; // write disable
const CMD_RDSR1: u8 = 0x05; // read status register 1
const CMD_WREN: u8 = 0x06; // write enable
const CMD_RDSR2: u8 = 0x35; // read status register 2
const FLASH_SR1_WIP: u8 = 1 << 0; // write in progress
const FLASH_SR1_WEL: u8 = 1 << 1; // write enable latch

/// Config-register storage covers offsets `0x00..0x100` as 32-bit words. boot2
/// writes `CTRLR0/CTRLR1/BAUDR/SER/RX_SAMPLE_DLY/SPI_CTRLR0` and reads some of
/// them back; plain storage is the faithful behaviour for those. `SR` and `DR0`
/// are intercepted and never served from here.
const NUM_REGS: usize = 0x100 / 4;

/// Minimal serial-flash state driven over SPI by the SSI engine.
#[derive(Debug, Default)]
struct FlashModel {
    /// Status register 1: WIP (bit 0) and WEL (bit 1). WIP is never latched
    /// because a modelled program/erase completes instantly.
    sr1: u8,
    /// Status register 2: the quad-enable (QE) bit lives at bit 1.
    sr2: u8,
}

/// The SSI shift engine plus the attached flash. A DR0 write shifts one frame
/// in each direction; a DR0 read drains one received frame.
#[derive(Debug, Default)]
struct Engine {
    /// Bytes clocked back from the flash, awaiting a DR0 read (RX FIFO).
    rx: VecDeque<u8>,
    /// Opcode of the command currently being clocked out, if a transaction is
    /// in progress. `None` between transactions (chip-select deasserted).
    cur_cmd: Option<u8>,
    /// Index of the next data byte within the active command (0 = opcode).
    cmd_idx: usize,
    flash: FlashModel,
}

impl Engine {
    /// Clock one 8-bit frame (`b`) out to the flash and one frame back into the
    /// RX FIFO, interpreting the byte stream as a serial-flash command.
    //
    // FIDELITY: modeled, NOT HW-validated (2026-07-04) — SSI shift engine +
    // W25Q-style flash command responder (RDSR1/RDSR2/WREN/WRSR) per RP2040
    // datasheet §4.10 (DW_apb_ssi) and Winbond W25Q080 §8. One DR0 write ==
    // one 8-bit frame; frame size (CTRLR0.DFS_32) is not honoured. Program/erase
    // complete instantly so flash SR1.WIP always reads clear.
    fn shift(&mut self, b: u8) {
        let rx = match self.cur_cmd {
            // Command (opcode) phase: MISO carries nothing meaningful while the
            // opcode is clocked out, so the received byte is 0x00.
            None => {
                self.cur_cmd = Some(b);
                self.cmd_idx = 1;
                match b {
                    CMD_WREN => self.flash.sr1 |= FLASH_SR1_WEL,
                    CMD_WRDI => self.flash.sr1 &= !FLASH_SR1_WEL,
                    _ => {}
                }
                0x00
            }
            // Data phase: the response depends on the active opcode.
            Some(cmd) => {
                let idx = self.cmd_idx;
                self.cmd_idx += 1;
                match cmd {
                    // Read status register 1: WIP always reads clear (no
                    // outstanding program/erase in the model).
                    CMD_RDSR1 => self.flash.sr1 & !FLASH_SR1_WIP,
                    // Read status register 2: returns the live QE state.
                    CMD_RDSR2 => self.flash.sr2,
                    // Write status register: first data byte -> SR1, second ->
                    // SR2. The command self-clears WEL on completion.
                    CMD_WRSR => {
                        match idx {
                            1 => self.flash.sr1 = b & !FLASH_SR1_WIP,
                            2 => self.flash.sr2 = b,
                            _ => {}
                        }
                        self.flash.sr1 &= !FLASH_SR1_WEL;
                        0x00
                    }
                    // Any other command (e.g. the 0xEB quad-read used to arm
                    // XIP): no meaningful read data.
                    _ => 0x00,
                }
            }
        };
        self.rx.push_back(rx);
    }

    /// Pop one received frame (the DR0 read port). Draining the last frame ends
    /// the transaction — the next DR0 write starts a fresh command, matching
    /// chip-select deassertion between SSI transfers.
    fn read_dr(&mut self) -> u8 {
        let v = self.rx.pop_front().unwrap_or(0);
        if self.rx.is_empty() {
            self.cur_cmd = None;
            self.cmd_idx = 0;
        }
        v
    }

    /// Clearing SSIENR flushes both FIFOs and aborts any in-flight command
    /// (datasheet §4.10.4: disabling the SSI resets the FIFOs).
    fn flush(&mut self) {
        self.rx.clear();
        self.cur_cmd = None;
        self.cmd_idx = 0;
    }
}

/// RP2040 XIP_SSI flash controller model.
#[derive(Debug)]
pub struct Rp2040XipSsi {
    regs: [u32; NUM_REGS],
    engine: RefCell<Engine>,
}

impl Default for Rp2040XipSsi {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2040XipSsi {
    pub fn new() -> Self {
        Self {
            regs: [0; NUM_REGS],
            engine: RefCell::new(Engine::default()),
        }
    }

    /// Compute `SR` from live FIFO state. The TX path drains within the write,
    /// so it is always empty/not-full and the engine is never busy; `RFNE`
    /// tracks whether a received frame is waiting to be read.
    fn status(&self) -> u32 {
        // FIDELITY: modeled, NOT HW-validated (2026-07-04) — SR (offset 0x28)
        // per RP2040 datasheet §4.10.13. TX drains within the write so TFE/TFNF
        // are always set and BUSY never asserts; RFNE tracks the RX FIFO.
        let mut sr = SR_TFE | SR_TFNF;
        if !self.engine.borrow().rx.is_empty() {
            sr |= SR_RFNE;
        }
        let _ = SR_BUSY; // documented; never asserted by this model
        sr
    }
}

impl Peripheral for Rp2040XipSsi {
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let val = match offset {
            SR => self.status(),
            DR0 => self.engine.borrow_mut().read_dr() as u32,
            _ => {
                let idx = (offset / 4) as usize;
                self.regs.get(idx).copied().unwrap_or(0)
            }
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // SR is read-only; ignore writes.
            SR => {}
            // Writing DR0 clocks one frame out to the flash.
            DR0 => self.engine.borrow_mut().shift(value as u8),
            SSIENR => {
                if value & SSIENR_EN == 0 {
                    self.engine.borrow_mut().flush();
                }
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value;
                }
            }
            _ => {
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value;
                }
            }
        }
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        // Reading any lane of DR0 drains one received frame; return the
        // requested byte of that frame. (boot2 only ever word-accesses DR0.)
        if (offset & !0x3) == DR0 {
            let word = self.engine.borrow_mut().read_dr() as u32;
            return Ok((word >> ((offset & 0x3) * 8)) as u8);
        }
        let word = self.read_u32(offset & !0x3)?;
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !0x3;
        let shift = (offset & 0x3) * 8;
        // Don't route the read-modify-write through the draining DR0 read.
        let cur = if aligned == DR0 || aligned == SR {
            0
        } else {
            self.read_u32(aligned)?
        };
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the exact status-poll loop boot2's `wait_ssi_ready` runs: read SR,
    /// require TFE set and BUSY clear.
    #[test]
    fn sr_reports_tx_empty_and_not_busy() {
        let ssi = Rp2040XipSsi::new();
        let sr = ssi.read_u32(SR).unwrap();
        assert_ne!(sr & SR_TFE, 0, "TFE must be set (TX FIFO empty)");
        assert_eq!(sr & SR_BUSY, 0, "BUSY must be clear");
    }

    /// read_flash_sreg(0x35): write opcode + dummy, read two frames. The second
    /// frame is SR2, and RFNE is asserted while data is pending.
    #[test]
    fn rdsr2_returns_status_register_two() {
        let mut ssi = Rp2040XipSsi::new();
        // Pretend the flash already has QE set (a legitimate persistent state).
        ssi.engine.borrow_mut().flash.sr2 = 0x02;
        ssi.write_u32(DR0, CMD_RDSR2 as u32).unwrap(); // opcode
        ssi.write_u32(DR0, CMD_RDSR2 as u32).unwrap(); // dummy clocks SR2 out
        assert_ne!(ssi.read_u32(SR).unwrap() & SR_RFNE, 0, "RFNE set");
        let _junk = ssi.read_u32(DR0).unwrap(); // opcode-phase frame
        let sr2 = ssi.read_u32(DR0).unwrap();
        assert_eq!(sr2, 0x02, "second frame is SR2 (QE set)");
        // FIFO drained -> RFNE clear, transaction closed.
        assert_eq!(ssi.read_u32(SR).unwrap() & SR_RFNE, 0, "RFNE clear");
    }

    /// The full boot2 quad-enable path: WREN, WRSR(0x00, 0x02), then RDSR1 must
    /// report WIP clear so the write-in-progress poll exits, and RDSR2 must now
    /// report QE set.
    #[test]
    fn wren_wrsr_sets_quad_enable_and_clears_wip() {
        let mut ssi = Rp2040XipSsi::new();
        // WREN.
        ssi.write_u32(DR0, CMD_WREN as u32).unwrap();
        let _ = ssi.read_u32(DR0).unwrap();
        assert_ne!(
            ssi.engine.borrow().flash.sr1 & FLASH_SR1_WEL,
            0,
            "WREN sets WEL"
        );
        // WRSR 0x00, 0x02 (SR1=0, SR2=QE).
        ssi.write_u32(DR0, CMD_WRSR as u32).unwrap();
        ssi.write_u32(DR0, 0x00).unwrap();
        ssi.write_u32(DR0, 0x02).unwrap();
        for _ in 0..3 {
            let _ = ssi.read_u32(DR0).unwrap();
        }
        // RDSR1: WIP must be clear (poll exit condition).
        ssi.write_u32(DR0, CMD_RDSR1 as u32).unwrap();
        ssi.write_u32(DR0, CMD_RDSR1 as u32).unwrap();
        let _ = ssi.read_u32(DR0).unwrap();
        let sr1 = ssi.read_u32(DR0).unwrap();
        assert_eq!(sr1 & FLASH_SR1_WIP as u32, 0, "WIP clear");
        // RDSR2: QE now set.
        ssi.write_u32(DR0, CMD_RDSR2 as u32).unwrap();
        ssi.write_u32(DR0, CMD_RDSR2 as u32).unwrap();
        let _ = ssi.read_u32(DR0).unwrap();
        let sr2 = ssi.read_u32(DR0).unwrap();
        assert_eq!(sr2 & 0x02, 0x02, "QE set after WRSR");
    }

    /// Disabling the SSI flushes the FIFO and aborts the transaction.
    #[test]
    fn ssienr_clear_flushes_fifo() {
        let mut ssi = Rp2040XipSsi::new();
        ssi.write_u32(DR0, 0xEB).unwrap();
        ssi.write_u32(DR0, 0xA0).unwrap();
        assert_ne!(ssi.read_u32(SR).unwrap() & SR_RFNE, 0, "RFNE set");
        ssi.write_u32(SSIENR, 0).unwrap();
        assert_eq!(
            ssi.read_u32(SR).unwrap() & SR_RFNE,
            0,
            "FIFO flushed on disable"
        );
    }
}
