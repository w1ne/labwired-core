// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;

/// STM32F1 compatible SPI peripheral
#[derive(Debug, Default, serde::Serialize)]
pub struct Spi {
    cr1: u16,
    cr2: u16,
    sr: u16,
    dr: u16,
    crcpr: u16,
    rxcrcr: u16,
    txcrcr: u16,
    i2scfgr: u16,
    i2spr: u16,

    // Internal state
    transfer_in_progress: bool,
    transfer_cycles_remaining: u32,
    transfer_buffer: u8,

    /// When true, completed transfers also load `transfer_buffer` into the
    /// RX path (`dr` + RXNE). Used by tests and integration scenarios that
    /// don't have a real slave wired but want the firmware-side `read`
    /// codepath exercised. Defaults to `false` (match real silicon with no
    /// MISO data — RXNE stays clear, DR reads as 0).
    loopback: bool,
}

impl Spi {
    pub fn new() -> Self {
        Self {
            // Reset values verified against real STM32L476RG silicon via
            // SWD register dump on a NUCLEO-L476RG:
            //   CR1 = 0x0000  CR2 = 0x0700  SR = 0x0002  DR = 0x0000
            // CR2.DS[3:0] (data size, bits 11:8) defaults to 0b0111 (8-bit)
            // on STM32L4 / F7 / H5 — newer SPI blocks. Older STM32F1
            // resets CR2 to 0x0000, but the same Spi struct serves both
            // and we go with the L4 convention since it's the one that
            // matters for DS-aware firmware.
            cr2: 0x0700,
            sr: 0x0002, // TXE = 1
            ..Default::default()
        }
    }

    /// Enable internal loopback: each completed transfer copies
    /// `transfer_buffer` into the RX path (`dr` + RXNE), as if MOSI were
    /// jumpered to MISO. Off by default; tests that exercise the read-
    /// after-write codepath without wiring a slave should enable this.
    pub fn set_loopback(&mut self, on: bool) {
        self.loopback = on;
    }

    fn read_reg(&self, offset: u64) -> u16 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.sr,
            // DR read returns the RX FIFO contents (`self.dr`), which is
            // distinct from what was last written. Real silicon has
            // separate TX and RX paths; we model that with `dr` for RX
            // and `transfer_buffer` for TX in flight.
            0x0C => self.dr,
            0x10 => self.crcpr,
            0x14 => self.rxcrcr,
            0x18 => self.txcrcr,
            0x1C => self.i2scfgr,
            0x20 => self.i2spr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u16) {
        match offset {
            0x00 => {
                self.cr1 = value;
            }
            0x04 => self.cr2 = value,
            0x08 => {
                // SR is mostly read-only; allow clearing OVR if modelled.
                self.sr = value & 0xFFBF;
            }
            0x0C
                // DR write goes to the TX path only. The TX byte ends up
                // in the shifter (transfer_buffer); `self.dr` (RX) is
                // untouched, so a subsequent DR read returns whatever
                // came in on MISO — 0 with no slave wired.
                if (self.cr1 & (1 << 6)) != 0 => {
                    // SPE set: start a transfer
                    self.sr &= !0x0002; // Clear TXE
                    self.sr |= 0x0080; // Set BSY
                    self.transfer_in_progress = true;
                    let br = (self.cr1 >> 3) & 0x7;
                    let divider = 1 << (br + 1);
                    self.transfer_cycles_remaining = 8 * divider;
                    self.transfer_buffer = (value & 0xFF) as u8;
                }
            _ => {}
        }
    }
}

impl crate::Peripheral for Spi {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        // Widen to u32 before the shift: SPI registers are u16 but byte
        // accesses at offsets 2 and 3 read the upper byte of the next
        // halfword. The CI release profile has overflow checks on, so
        // `(u16 >> 16)` would panic; the `as u32` widen avoids that.
        let reg_val = self.read_reg(reg_offset) as u32;
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        // Same widen-then-shift dance as read() to avoid u16 shift overflow.
        let mut reg_val = self.read_reg(reg_offset) as u32;
        let mask: u32 = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val as u16);
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        let mut irq = false;
        if self.transfer_in_progress {
            self.transfer_cycles_remaining = self.transfer_cycles_remaining.saturating_sub(1);
            if self.transfer_cycles_remaining == 0 {
                self.transfer_in_progress = false;
                self.sr &= !0x0080; // Clear BSY
                self.sr |= 0x0002; // Set TXE
                if self.loopback {
                    // Internal loopback: MOSI → MISO. Mirrors the data the
                    // firmware just wrote back into the RX path so a
                    // unit test (or "single-board echo" integration) sees
                    // RXNE go high with the byte it transmitted.
                    self.dr = self.transfer_buffer as u16;
                    self.sr |= 0x0001; // RXNE
                }
                // Without loopback we deliberately do NOT auto-set RXNE
                // or auto-fill DR: real STM32 silicon with no slave wired
                // (or no MISO pin AF'd) doesn't drive anything onto MISO.
                // Production smoke tests therefore see SR=0x0002 / DR=0
                // after a write — matching NUCLEO-L476RG silicon.
                if (self.cr2 & (1 << 7)) != 0 {
                    // TXEIE
                    irq = true;
                }
            }
        }

        crate::PeripheralTickResult {
            irq,
            cycles: 0,
            ..Default::default()
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::Spi;
    use crate::Peripheral;

    #[test]
    fn test_spi_transfer_timing() {
        let mut spi = Spi::new();
        // Enable SPI + BR=1 (f_pclk/4): (1<<6) | (1<<3) = 0x48.
        spi.write(0x00, 0x48).unwrap();

        // Reset SR has TXE set (bit 1).
        assert_ne!(spi.read(0x08).unwrap() & 0x02, 0);

        // Write DR -> start transfer.
        spi.write(0x0C, 0xAA).unwrap();
        let sr = spi.read(0x08).unwrap();
        assert_ne!(sr & 0x80, 0, "BSY set during transfer");
        assert_eq!(sr & 0x02, 0, "TXE cleared while shifting");

        // BR=1 -> divider=4 -> 8 bits * 4 = 32 ticks.
        for _ in 0..31 {
            spi.tick();
            assert_ne!(spi.read(0x08).unwrap() & 0x80, 0, "still busy mid-transfer");
        }

        spi.tick();
        let sr = spi.read(0x08).unwrap();
        assert_eq!(sr & 0x80, 0, "BSY cleared after transfer");
        assert_ne!(sr & 0x02, 0, "TXE set after transfer");
        // No slave wired in this test → no MISO data → RXNE stays clear
        // and DR read returns the RX register (initialised to 0). This
        // matches what real STM32L476RG hardware does in the same setup;
        // see firmware_survival's nucleo_l476rg_spi case for the trace.
        assert_eq!(sr & 0x01, 0, "RXNE NOT set without a slave");
        assert_eq!(spi.read(0x0C).unwrap(), 0x00, "DR=0 with no MISO data");
    }
}
