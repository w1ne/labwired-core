// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::any::Any;
use std::str::FromStr;

/// Trait implemented by simulated SPI devices (peripherals attached to an SPI bus).
///
/// For v1, CS-pin-aware routing is not implemented: all transfers are broadcast
/// to every attached device and the first non-zero MISO byte wins.  This is
/// correct for single-device labs (MAX31855 alone).  CS-aware routing is noted
/// as a Phase 2 follow-up.
pub trait SpiDevice: Send {
    /// Called when the CS line goes low (chip is selected).
    fn cs_select(&mut self) {}
    /// Called when the CS line goes high (chip is released — flush state).
    fn cs_release(&mut self) {}
    /// SPI is full-duplex: master sends `mosi_byte`, device returns its current MISO byte.
    /// On read-only devices like MAX31855, `mosi_byte` is ignored.
    fn transfer(&mut self, mosi_byte: u8) -> u8;
    /// CS pin label this device is wired to (e.g. "PA4" or numeric pin ID). Used by the bus
    /// dispatcher to pick which device responds when the firmware drives a particular CS line.
    fn cs_pin(&self) -> &str;
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
    /// Binary mid-flight snapshot for runtime resume. Default empty;
    /// override for stateful devices (e-paper panels with framebuffers,
    /// thermocouples with cached temperatures, etc.).
    fn runtime_snapshot(&self) -> Vec<u8> {
        Vec::new()
    }
    fn restore_runtime_snapshot(&mut self, _bytes: &[u8]) -> crate::SimResult<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpiRegisterLayout {
    #[default]
    Stm32,
    Nrf52Spim,
}

impl FromStr for SpiRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32" | "stm32f1" | "stm32v2" => Ok(Self::Stm32),
            "nrf52" | "nrf52_spim" | "nrf_spim" | "nordic" => Ok(Self::Nrf52Spim),
            _ => Err(format!(
                "unsupported SPI register layout '{}'; supported: stm32, nrf52",
                value
            )),
        }
    }
}

/// STM32F1 compatible SPI peripheral
#[derive(Default, serde::Serialize)]
pub struct Spi {
    layout: SpiRegisterLayout,
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

    nrf_events_end: u32,
    nrf_events_stopped: u32,
    nrf_enable: u32,
    nrf_psel_sck: u32,
    nrf_psel_mosi: u32,
    nrf_psel_miso: u32,
    nrf_frequency: u32,
    nrf_config: u32,
    nrf_rxd_ptr: u32,
    nrf_rxd_maxcnt: u32,
    nrf_rxd_amount: u32,
    nrf_txd_ptr: u32,
    nrf_txd_maxcnt: u32,
    nrf_txd_amount: u32,

    #[serde(skip)]
    pub attached_devices: Vec<Box<dyn SpiDevice>>,
}

impl core::fmt::Debug for Spi {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Spi")
            .field("layout", &self.layout)
            .field("cr1", &self.cr1)
            .field("sr", &self.sr)
            .field("transfer_in_progress", &self.transfer_in_progress)
            .field("loopback", &self.loopback)
            .field("attached_devices", &self.attached_devices.len())
            .finish()
    }
}

impl Spi {
    pub fn new() -> Self {
        Self::new_with_layout(SpiRegisterLayout::Stm32)
    }

    pub fn new_with_layout(layout: SpiRegisterLayout) -> Self {
        Self {
            layout,
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

    pub fn attach(&mut self, device: Box<dyn SpiDevice>) {
        self.attached_devices.push(device);
    }

    fn read_reg(&self, offset: u64) -> u16 {
        if matches!(self.layout, SpiRegisterLayout::Nrf52Spim) {
            return self.read_nrf_reg(offset) as u16;
        }
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
        if matches!(self.layout, SpiRegisterLayout::Nrf52Spim) {
            self.write_nrf_reg(offset, value as u32);
            return;
        }
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

                    // v1 routing: broadcast to all attached devices, use last non-zero response.
                    // CS-pin-aware routing is Phase 2 (see concerns in commit).
                    if !self.attached_devices.is_empty() {
                        let mosi = self.transfer_buffer;
                        let mut miso: u8 = 0;
                        for dev in &mut self.attached_devices {
                            let resp = dev.transfer(mosi);
                            if resp != 0 {
                                miso = resp;
                            }
                        }
                        self.transfer_buffer = miso;
                    }
                }
            _ => {}
        }
    }

    fn read_nrf_reg(&self, offset: u64) -> u32 {
        match offset {
            0x104 => self.nrf_events_stopped,
            0x118 => self.nrf_events_end,
            0x500 => self.nrf_enable,
            0x508 => self.nrf_psel_sck,
            0x50C => self.nrf_psel_mosi,
            0x510 => self.nrf_psel_miso,
            0x524 => self.nrf_frequency,
            0x534 => self.nrf_rxd_ptr,
            0x538 => self.nrf_rxd_maxcnt,
            0x53C => self.nrf_rxd_amount,
            0x544 => self.nrf_txd_ptr,
            0x548 => self.nrf_txd_maxcnt,
            0x54C => self.nrf_txd_amount,
            0x554 => self.nrf_config,
            _ => 0,
        }
    }

    fn write_nrf_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x010 if value != 0 => {
                self.nrf_events_end = 1;
                self.nrf_txd_amount = self.nrf_txd_maxcnt;
                self.nrf_rxd_amount = self.nrf_rxd_maxcnt;
            }
            0x014 if value != 0 => {
                self.nrf_events_stopped = 1;
            }
            0x104 => self.nrf_events_stopped = value,
            0x118 => self.nrf_events_end = value,
            0x500 => self.nrf_enable = value,
            0x508 => self.nrf_psel_sck = value,
            0x50C => self.nrf_psel_mosi = value,
            0x510 => self.nrf_psel_miso = value,
            0x524 => self.nrf_frequency = value,
            0x534 => self.nrf_rxd_ptr = value,
            0x538 => self.nrf_rxd_maxcnt = value,
            0x53C => self.nrf_rxd_amount = value,
            0x544 => self.nrf_txd_ptr = value,
            0x548 => self.nrf_txd_maxcnt = value,
            0x54C => self.nrf_txd_amount = value,
            0x554 => self.nrf_config = value,
            _ => {}
        }
    }
}

impl crate::Peripheral for Spi {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        if matches!(self.layout, SpiRegisterLayout::Nrf52Spim) {
            let reg_val = self.read_nrf_reg(reg_offset);
            return Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8);
        }
        // Widen to u32 before the shift: SPI registers are u16 but byte
        // accesses at offsets 2 and 3 read the upper byte of the next
        // halfword. The CI release profile has overflow checks on, so
        // `(u16 >> 16)` would panic; the `as u32` widen avoids that.
        // (Result is identical to short-circuiting offsets 2..3 to 0,
        // since `(u16 as u32) >> 16` is 0.)
        let reg_val = self.read_reg(reg_offset) as u32;
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        if matches!(self.layout, SpiRegisterLayout::Nrf52Spim) {
            let mut reg_val = self.read_nrf_reg(reg_offset);
            let mask: u32 = 0xFF << (byte_offset * 8);
            reg_val &= !mask;
            reg_val |= (value as u32) << (byte_offset * 8);
            self.write_nrf_reg(reg_offset, reg_val);
            return Ok(());
        }

        // Same widen-then-shift dance as read() to avoid u16 shift overflow.
        // Writes to bytes 2..3 are naturally discarded because the final
        // `write_reg(reg_offset, reg_val as u16)` truncates back to u16.
        let mut reg_val = self.read_reg(reg_offset) as u32;
        let mask: u32 = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val as u16);
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        if matches!(self.layout, SpiRegisterLayout::Nrf52Spim) {
            self.write(offset, (value & 0xFF) as u8)?;
            self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
            return Ok(());
        }
        // SPI DR (offset 0x0C) MUST be atomic — a Thumb `strh` from firmware
        // is one bus access, kicking off a single SPI transfer. The default
        // trait impl byte-splits, which would start two transfers back-to-back
        // and broadcast a spurious upper-byte (typically 0x00) to attached
        // devices — devastating for protocol state machines that interpret
        // every byte (SSD1680 e-paper command set). For 8-bit DFF (the default
        // and only mode we support) only the low byte goes on the wire; the
        // upper byte is discarded by silicon. For 16-bit DFF this would need
        // a 16-cycle transfer; not modeled, low byte only is fine for now.
        if offset == 0x0C {
            self.write_reg(0x0C, value);
            return Ok(());
        }
        // Other registers: byte-split is fine (no transfer side-effects).
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
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

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
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
