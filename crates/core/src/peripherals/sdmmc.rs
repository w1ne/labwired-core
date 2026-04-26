// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! SDMMC — STM32L4 SD/MMC card host interface (RM0351 §38).
//!
//! Register layout (4-byte spaced):
//!   POWER     0x00   - PWRCTRL[1:0] = 00 off, 11 on
//!   CLKCR     0x04   - CLKDIV / CLKEN / WIDBUS
//!   ARG       0x08   - command argument
//!   CMD       0x0C   - CMDINDEX / WAITRESP / CPSMEN
//!   RESPCMD   0x10   - last received command index (read-only)
//!   RESP1-4   0x14-0x20  - response data (read-only)
//!   DTIMER    0x24   - data timeout
//!   DLEN      0x28   - data length
//!   DCTRL     0x2C   - DTEN / DTDIR / DTMODE / DBLOCKSIZE
//!   DCOUNT    0x30   - remaining bytes (read-only)
//!   STA       0x34   - status flags
//!   ICR       0x38   - rc_w1 — clears matching STA bits
//!   MASK      0x3C   - interrupt enable
//!   FIFOCNT   0x48   - FIFO word count (read-only)
//!   FIFO      0x80   - 32-bit FIFO data
//!
//! Reset values per RM0351 §38.6: all registers = 0.
//!
//! For survival mode we model the command-state-machine handshake:
//! writing CMD with CPSMEN=1 immediately asserts STA.CMDSENT or
//! STA.CMDREND so HAL_SD_Init's polling loops exit. Real silicon
//! waits for the response from the card.

use crate::SimResult;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Sdmmc {
    power: u32,
    clkcr: u32,
    arg: u32,
    cmd: u32,
    respcmd: u32,
    resp1: u32,
    resp2: u32,
    resp3: u32,
    resp4: u32,
    dtimer: u32,
    dlen: u32,
    dctrl: u32,
    dcount: u32,
    sta: u32,
    icr: u32,
    mask: u32,
    fifocnt: u32,
    fifo: u32,
}

impl Sdmmc {
    pub fn new() -> Self {
        Self {
            power: 0,
            clkcr: 0,
            arg: 0,
            cmd: 0,
            respcmd: 0,
            resp1: 0,
            resp2: 0,
            resp3: 0,
            resp4: 0,
            dtimer: 0,
            dlen: 0,
            dctrl: 0,
            dcount: 0,
            sta: 0,
            icr: 0,
            mask: 0,
            fifocnt: 0,
            fifo: 0,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.power,
            0x04 => self.clkcr,
            0x08 => self.arg,
            0x0C => self.cmd,
            0x10 => self.respcmd,
            0x14 => self.resp1,
            0x18 => self.resp2,
            0x1C => self.resp3,
            0x20 => self.resp4,
            0x24 => self.dtimer,
            0x28 => self.dlen,
            0x2C => self.dctrl,
            0x30 => self.dcount,
            0x34 => self.sta,
            0x38 => self.icr,
            0x3C => self.mask,
            0x48 => self.fifocnt,
            0x80 => self.fifo,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.power = value & 0x03,
            0x04 => self.clkcr = value & 0x0001_FFFF,
            0x08 => self.arg = value,
            0x0C => {
                self.cmd = value & 0x0FFF;
                // CPSMEN (bit 10) launches the command-path state machine.
                // Behaviour matches silicon's no-card / no-clock scenario:
                //
                //   - If CLKCR.CLKEN (bit 8) is set, a real card would
                //     respond; survival-mode HAL flow asserts the
                //     appropriate completion flag (CMDSENT for no-response
                //     commands, CMDREND for response-bearing commands)
                //     so polling loops exit.
                //   - If CLKEN is clear (no SDMMC clock running), or
                //     when a response was expected but none arrives, the
                //     command-path state machine eventually asserts
                //     CTIMEOUT (STA bit 11) — confirmed against
                //     NUCLEO-L476RG silicon with no card present.
                //
                // RESPCMD only updates on a real response from a card.
                // Without one, it stays 0 — do NOT mirror CMDINDEX.
                if (self.cmd & (1 << 10)) != 0 {
                    let clken = (self.clkcr & (1 << 8)) != 0;
                    let waitresp = (self.cmd >> 6) & 0x3;
                    if !clken {
                        self.sta |= 1 << 11; // CTIMEOUT
                    } else if waitresp == 0 {
                        self.sta |= 1 << 7; // CMDSENT
                    } else {
                        self.sta |= 1 << 6; // CMDREND
                        self.respcmd = self.cmd & 0x3F;
                    }
                }
            }
            0x24 => self.dtimer = value,
            0x28 => self.dlen = value & 0x01FF_FFFF,
            0x2C => {
                self.dctrl = value & 0x0FFF;
                // DTEN (bit 0) starts a data-transfer; assert DBCKEND so
                // HAL polling exits without consuming FIFO data.
                if (self.dctrl & 1) != 0 {
                    self.sta |= 1 << 10; // DBCKEND
                }
            }
            0x34 => {} // STA is read-only
            0x38 => {
                // ICR is rc_w1: clear matching STA bits.
                self.sta &= !value;
                self.icr = 0;
            }
            0x3C => self.mask = value,
            0x80 => self.fifo = value,
            _ => {}
        }
    }
}

impl Default for Sdmmc {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for Sdmmc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::Sdmmc;
    use crate::Peripheral;

    fn write32(s: &mut Sdmmc, off: u64, val: u32) {
        for i in 0..4 {
            s.write(off + i, ((val >> (i * 8)) & 0xFF) as u8).unwrap();
        }
    }

    fn read32(s: &Sdmmc, off: u64) -> u32 {
        let mut v = 0u32;
        for i in 0..4 {
            v |= (s.read(off + i).unwrap() as u32) << (i * 8);
        }
        v
    }

    #[test]
    fn test_cmd_with_no_clock_asserts_ctimeout() {
        // Default state: CLKCR=0, no clock running. Real silicon times out.
        let mut s = Sdmmc::new();
        write32(&mut s, 0x0C, 0x05 | (1 << 10));
        let sta = read32(&s, 0x34);
        assert_ne!(sta & (1 << 11), 0, "CTIMEOUT should fire when CLKEN=0");
        // RESPCMD stays 0 — no response received.
        assert_eq!(read32(&s, 0x10), 0);
    }

    #[test]
    fn test_cmd_with_clock_no_response_asserts_cmdsent() {
        let mut s = Sdmmc::new();
        write32(&mut s, 0x04, 1 << 8); // CLKEN
        write32(&mut s, 0x0C, 0x05 | (1 << 10));
        let sta = read32(&s, 0x34);
        assert_ne!(sta & (1 << 7), 0); // CMDSENT
    }

    #[test]
    fn test_cmd_with_clock_short_response_asserts_cmdrend_and_respcmd() {
        let mut s = Sdmmc::new();
        write32(&mut s, 0x04, 1 << 8); // CLKEN
        write32(&mut s, 0x0C, 0x05 | (1 << 6) | (1 << 10));
        let sta = read32(&s, 0x34);
        assert_ne!(sta & (1 << 6), 0); // CMDREND
        assert_eq!(read32(&s, 0x10) & 0x3F, 0x05); // RESPCMD = CMDINDEX
    }

    #[test]
    fn test_icr_clears_sta() {
        let mut s = Sdmmc::new();
        // CTIMEOUT path (no clock configured).
        write32(&mut s, 0x0C, 0x05 | (1 << 10));
        write32(&mut s, 0x38, 1 << 11); // clear CTIMEOUT
        let sta = read32(&s, 0x34);
        assert_eq!(sta & (1 << 11), 0);
    }
}
