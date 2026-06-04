// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// ── Architectural separation ────────────────────────────────────────────────
// The family-specific register STATE lives in the `SpiRegs` enum: an STM32 SPI
// instance carries ONLY the STM32 registers, an nRF SPIM carries ONLY the
// Nordic registers — neither can hold the other's state. The shared transfer
// engine, attached-device routing and event-scheduler glue stay on `Spi`
// (genuinely shared behaviour), so the public API (`attach`, `set_loopback`,
// `as_any`) is unchanged. The chip-yaml `profile` selects the variant.

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
    /// Data/Command (D/C) pin label this device observes, if any (e.g. "PB6").
    ///
    /// Displays like the Nokia 5110 (PCD8544) distinguish command bytes from
    /// pixel-data bytes by the level of a dedicated GPIO line rather than by
    /// byte semantics. When this returns `Some(pin)`, the bus latches that
    /// pin's current output level into the device via [`set_dc_level`] after
    /// each MMIO write, so the value is current by the time the firmware
    /// writes the SPI data register. Default `None` → the bus does no latching
    /// and the device infers framing from the protocol (ILI9341 / SSD1680).
    ///
    /// [`set_dc_level`]: SpiDevice::set_dc_level
    fn dc_pin(&self) -> Option<&str> {
        None
    }
    /// Latched level of the [`dc_pin`](SpiDevice::dc_pin) at transfer time,
    /// pushed by the bus. No-op for devices that do not observe a D/C line.
    fn set_dc_level(&mut self, _level: bool) {}
    /// Resolved `(ODR address, bit)` of the D/C line. The bus computes this
    /// once at install time (from [`dc_pin`](SpiDevice::dc_pin)) and records it
    /// via [`set_dc_source`]; thereafter the bus reads that GPIO output bit
    /// just before each transfer and pushes the level via [`set_dc_level`].
    /// Default `None` → no D/C latching.
    ///
    /// [`set_dc_source`]: SpiDevice::set_dc_source
    fn dc_source(&self) -> Option<(u64, u8)> {
        None
    }
    /// Bus-side setter recording the resolved D/C `(ODR address, bit)`.
    fn set_dc_source(&mut self, _odr_addr: u64, _bit: u8) {}
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
    /// STM32 families with a TX/RX FIFO + CR2.DS data-size field (L4/F7/H5/
    /// G4/…). Identical register layout to `Stm32`, but a **16-bit DR write at
    /// DS≤8 packs two frames** (RM0351 §40.4.9 data packing) — modelled so
    /// firmware that wrongly uses a 16-bit DR access at 8-bit data size
    /// mis-renders in the sim exactly as it does on silicon.
    Stm32Fifo,
    Nrf52Spim,
}

impl FromStr for SpiRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32" | "stm32f1" | "stm32f4" | "stm32v2" => Ok(Self::Stm32),
            "stm32_fifo" | "stm32l4" | "stm32f7" | "stm32h5" | "stm32g4" => Ok(Self::Stm32Fifo),
            "nrf52" | "nrf52_spim" | "nrf_spim" | "nordic" => Ok(Self::Nrf52Spim),
            _ => Err(format!(
                "unsupported SPI register layout '{}'; supported: stm32, stm32_fifo, nrf52",
                value
            )),
        }
    }
}

/// Event token for the one-shot SPI transfer-completion event (the SPI has a
/// single kind of scheduled wakeup, so the value is arbitrary).
const SPI_DONE_TOKEN: u32 = 0;

/// STM32 SPI register file (F1/F4/L0 classic and L4/F7/H5 FIFO share this map;
/// `fifo` selects the FIFO DS/data-packing behaviour).
#[derive(Debug, Clone, Default, serde::Serialize)]
struct Stm32SpiRegs {
    fifo: bool,
    cr1: u16,
    cr2: u16,
    sr: u16,
    dr: u16,
    crcpr: u16,
    rxcrcr: u16,
    txcrcr: u16,
    i2scfgr: u16,
    i2spr: u16,
}

impl Stm32SpiRegs {
    fn read_reg(&self, offset: u64) -> u16 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.sr,
            // DR read returns the RX FIFO contents (`self.dr`), which is
            // distinct from what was last written. Real silicon has separate
            // TX and RX paths; we model that with `dr` for RX.
            0x0C => self.dr,
            0x10 => self.crcpr,
            0x14 => self.rxcrcr,
            0x18 => self.txcrcr,
            0x1C => self.i2scfgr,
            0x20 => self.i2spr,
            _ => 0,
        }
    }
}

/// Nordic nRF52 SPIM (EasyDMA) register file — Nordic-only state.
#[derive(Debug, Clone, Default, serde::Serialize)]
struct Nrf52SpiRegs {
    events_end: u32,
    events_stopped: u32,
    enable: u32,
    psel_sck: u32,
    psel_mosi: u32,
    psel_miso: u32,
    frequency: u32,
    config: u32,
    rxd_ptr: u32,
    rxd_maxcnt: u32,
    rxd_amount: u32,
    txd_ptr: u32,
    txd_maxcnt: u32,
    txd_amount: u32,
}

impl Nrf52SpiRegs {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x104 => self.events_stopped,
            0x118 => self.events_end,
            0x500 => self.enable,
            0x508 => self.psel_sck,
            0x50C => self.psel_mosi,
            0x510 => self.psel_miso,
            0x524 => self.frequency,
            0x534 => self.rxd_ptr,
            0x538 => self.rxd_maxcnt,
            0x53C => self.rxd_amount,
            0x544 => self.txd_ptr,
            0x548 => self.txd_maxcnt,
            0x54C => self.txd_amount,
            0x554 => self.config,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x010 if value != 0 => {
                self.events_end = 1;
                self.txd_amount = self.txd_maxcnt;
                self.rxd_amount = self.rxd_maxcnt;
            }
            0x014 if value != 0 => {
                self.events_stopped = 1;
            }
            0x104 => self.events_stopped = value,
            0x118 => self.events_end = value,
            0x500 => self.enable = value,
            0x508 => self.psel_sck = value,
            0x50C => self.psel_mosi = value,
            0x510 => self.psel_miso = value,
            0x524 => self.frequency = value,
            0x534 => self.rxd_ptr = value,
            0x538 => self.rxd_maxcnt = value,
            0x53C => self.rxd_amount = value,
            0x544 => self.txd_ptr = value,
            0x548 => self.txd_maxcnt = value,
            0x54C => self.txd_amount = value,
            0x554 => self.config = value,
            _ => {}
        }
    }
}

/// Family-isolated SPI register state. STM32 and nRF register sets cannot
/// coexist on one instance.
#[derive(Debug, Clone, serde::Serialize)]
enum SpiRegs {
    Stm32(Stm32SpiRegs),
    Nrf52(Nrf52SpiRegs),
}

impl Default for SpiRegs {
    fn default() -> Self {
        SpiRegs::Stm32(Stm32SpiRegs::default())
    }
}

/// SPI peripheral: family-isolated registers (`regs`) + a shared transfer
/// engine and attached-device routing.
#[derive(Default, serde::Serialize)]
pub struct Spi {
    regs: SpiRegs,

    // Shared transfer-engine state (architecture-independent).
    transfer_in_progress: bool,
    transfer_cycles_remaining: u32,
    transfer_buffer: u8,
    #[serde(skip)]
    scheduled: bool,
    /// When true, completed transfers also load `transfer_buffer` into the RX
    /// path (`dr` + RXNE), as if MOSI were jumpered to MISO. Defaults false.
    loopback: bool,

    #[serde(skip)]
    pub attached_devices: Vec<Box<dyn SpiDevice>>,
}

impl core::fmt::Debug for Spi {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Spi")
            .field("regs", &self.regs)
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
        let regs = match layout {
            // CR2 reset is silicon-verified over SWD:
            //   FIFO SPI (L4/F7/H5): CR2 = 0x0700 (DS=0b0111 8-bit + FRXTH).
            //   Classic SPI (F1/F4/L0): CR2 = 0x0000 (no DS field).
            SpiRegisterLayout::Stm32 => SpiRegs::Stm32(Stm32SpiRegs {
                fifo: false,
                cr2: 0x0000,
                sr: 0x0002, // TXE = 1
                ..Default::default()
            }),
            SpiRegisterLayout::Stm32Fifo => SpiRegs::Stm32(Stm32SpiRegs {
                fifo: true,
                cr2: 0x0700,
                sr: 0x0002,
                ..Default::default()
            }),
            SpiRegisterLayout::Nrf52Spim => SpiRegs::Nrf52(Nrf52SpiRegs::default()),
        };
        Self {
            regs,
            ..Default::default()
        }
    }

    pub fn set_loopback(&mut self, on: bool) {
        self.loopback = on;
    }

    pub fn attach(&mut self, device: Box<dyn SpiDevice>) {
        self.attached_devices.push(device);
    }

    fn is_nrf(&self) -> bool {
        matches!(self.regs, SpiRegs::Nrf52(_))
    }

    /// STM32 register write with transfer-engine side effects. Only called on
    /// the STM32 variant.
    fn write_stm32_reg(&mut self, offset: u64, value: u16) {
        match offset {
            0x00 => {
                if let SpiRegs::Stm32(r) = &mut self.regs {
                    r.cr1 = value;
                }
            }
            0x04 => {
                // STM32L4/F7/H5 SPI CR2: DS[3:0] (bits 11:8) select the data
                // frame size. Values below 0b0011 are reserved and hardware
                // forces them to 0b0111 (8-bit) on FIFO parts — verified on
                // NUCLEO-L476RG (CR2=0x0000 reads back 0x0700). Classic SPI
                // has no DS field, so it stores the value verbatim.
                if let SpiRegs::Stm32(r) = &mut self.regs {
                    if r.fifo {
                        let ds = (value >> 8) & 0xF;
                        r.cr2 = if ds < 0b0011 {
                            (value & !0x0F00) | (0b0111 << 8)
                        } else {
                            value
                        };
                    } else {
                        r.cr2 = value;
                    }
                }
            }
            0x08 => {
                // SR is mostly read-only; allow clearing OVR if modelled.
                if let SpiRegs::Stm32(r) = &mut self.regs {
                    r.sr = value & 0xFFBF;
                }
            }
            0x0C => {
                // DR write goes to the TX path only. Starts a transfer iff SPE.
                let cr1 = match &self.regs {
                    SpiRegs::Stm32(r) => r.cr1,
                    _ => 0,
                };
                if (cr1 & (1 << 6)) != 0 {
                    if let SpiRegs::Stm32(r) = &mut self.regs {
                        r.sr &= !0x0002; // Clear TXE
                        r.sr |= 0x0080; // Set BSY
                    }
                    self.transfer_in_progress = true;
                    let br = (cr1 >> 3) & 0x7;
                    let divider = 1u32 << (br + 1);
                    self.transfer_cycles_remaining = 8 * divider;
                    self.transfer_buffer = (value & 0xFF) as u8;

                    // v1 routing: broadcast to all attached devices, use last
                    // non-zero response.
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
            }
            _ => {}
        }
    }
}

impl crate::Peripheral for Spi {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = match &self.regs {
            SpiRegs::Nrf52(r) => r.read_reg(reg_offset),
            // Widen u16→u32 before the shift: byte accesses at offsets 2/3 read
            // the upper byte of the next halfword; `(u16 as u32) >> 16` is 0
            // without an overflow panic under the CI release profile.
            SpiRegs::Stm32(r) => r.read_reg(reg_offset) as u32,
        };
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        if let SpiRegs::Nrf52(_) = &self.regs {
            let cur = match &self.regs {
                SpiRegs::Nrf52(r) => r.read_reg(reg_offset),
                _ => 0,
            };
            let mask: u32 = 0xFF << (byte_offset * 8);
            let new = (cur & !mask) | ((value as u32) << (byte_offset * 8));
            if let SpiRegs::Nrf52(r) = &mut self.regs {
                r.write_reg(reg_offset, new);
            }
            return Ok(());
        }

        // STM32: same widen-then-shift dance to avoid u16 shift overflow; the
        // final write truncates back to u16, discarding bytes 2..3.
        let cur = match &self.regs {
            SpiRegs::Stm32(r) => r.read_reg(reg_offset) as u32,
            _ => 0,
        };
        let mask: u32 = 0xFF << (byte_offset * 8);
        let new = (cur & !mask) | ((value as u32) << (byte_offset * 8));
        self.write_stm32_reg(reg_offset, new as u16);
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        if self.is_nrf() {
            self.write(offset, (value & 0xFF) as u8)?;
            self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
            return Ok(());
        }
        // SPI DR (offset 0x0C) MUST be atomic — a Thumb `strh` is one bus
        // access kicking off a single transfer. Byte-splitting would start two
        // transfers and broadcast a spurious upper byte to attached devices.
        if offset == 0x0C {
            let (fifo, ds) = match &self.regs {
                SpiRegs::Stm32(r) => (r.fifo, (r.cr2 >> 8) & 0xF),
                _ => (false, 0),
            };
            if fifo && ds <= 0b0111 {
                // FIFO data packing (RM0351 §40.4.9): a 16-bit DR access at
                // DS≤8 enqueues TWO frames (low byte, then high byte).
                self.write_stm32_reg(0x0C, value & 0xFF);
                self.write_stm32_reg(0x0C, (value >> 8) & 0xFF);
            } else {
                self.write_stm32_reg(0x0C, value);
            }
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
                let deliver_rx = self.loopback || !self.attached_devices.is_empty();
                let buf = self.transfer_buffer;
                if let SpiRegs::Stm32(r) = &mut self.regs {
                    r.sr &= !0x0080; // Clear BSY
                    r.sr |= 0x0002; // Set TXE
                    if deliver_rx {
                        // A wired slave drove its byte onto MISO (already in
                        // `transfer_buffer`); loopback mirrors MOSI. Either way
                        // the firmware sees RXNE high with the received byte.
                        r.dr = buf as u16;
                        r.sr |= 0x0001; // RXNE
                    }
                    if (r.cr2 & (1 << 7)) != 0 {
                        irq = true; // TXEIE
                    }
                }
            }
        }

        crate::PeripheralTickResult {
            irq,
            cycles: 0,
            ..Default::default()
        }
    }

    fn uses_scheduler(&self) -> bool {
        true
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.transfer_in_progress && !self.scheduled {
            self.scheduled = true;
            vec![(self.transfer_cycles_remaining as u64, SPI_DONE_TOKEN)]
        } else {
            Vec::new()
        }
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        self.scheduled = false;
        if !self.transfer_in_progress {
            return crate::sched::EventResult::default();
        }
        self.transfer_in_progress = false;
        self.transfer_cycles_remaining = 0;
        let deliver_rx = self.loopback || !self.attached_devices.is_empty();
        let buf = self.transfer_buffer;
        let mut res = crate::sched::EventResult::default();
        if let SpiRegs::Stm32(r) = &mut self.regs {
            r.sr &= !0x0080; // Clear BSY
            r.sr |= 0x0002; // Set TXE
            if deliver_rx {
                r.dr = buf as u16;
                r.sr |= 0x0001; // RXNE
            }
            if (r.cr2 & (1 << 7)) != 0 {
                res.raise_own_irq = true; // TXEIE
            }
        }
        res
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
    use super::{Spi, SpiDevice, SpiRegisterLayout};
    use crate::Peripheral;

    /// SPI slave that records every byte it receives.
    struct Capture {
        rx: Vec<u8>,
    }
    impl SpiDevice for Capture {
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.rx.push(mosi);
            0
        }
        fn cs_pin(&self) -> &str {
            ""
        }
        fn as_any(&self) -> Option<&dyn std::any::Any> {
            Some(self)
        }
        fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
            Some(self)
        }
    }

    fn captured(spi: &Spi) -> Vec<u8> {
        spi.attached_devices[0]
            .as_any()
            .unwrap()
            .downcast_ref::<Capture>()
            .unwrap()
            .rx
            .clone()
    }

    /// FIFO-family SPI: a 16-bit DR write at DS=8 packs TWO frames — the
    /// silicon behaviour that broke the real Nokia 5110 panel.
    #[test]
    fn fifo_packs_u16_dr_write_into_two_frames() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Stm32Fifo);
        spi.attach(Box::new(Capture { rx: Vec::new() }));
        spi.write(0x00, 0x40).unwrap(); // CR1: SPE
        spi.write_u16(0x0C, 0x00AB).unwrap(); // 16-bit DR write, DS=8 (reset 0x0700)
        assert_eq!(
            captured(&spi),
            vec![0xAB, 0x00],
            "DS≤8 + 16-bit DR ⇒ 2 frames"
        );
    }

    /// The correct 8-bit DR access sends exactly one frame, even on FIFO parts.
    #[test]
    fn fifo_u8_dr_write_is_one_frame() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Stm32Fifo);
        spi.attach(Box::new(Capture { rx: Vec::new() }));
        spi.write(0x00, 0x40).unwrap();
        spi.write(0x0C, 0xAB).unwrap(); // 8-bit DR write
        assert_eq!(captured(&spi), vec![0xAB], "8-bit DR ⇒ 1 frame");
    }

    /// Non-FIFO STM32 (F1/F4) does NOT pack: a 16-bit DR write is one frame,
    /// so the F103 ILI9341 lab (which writes DR as u16) is unaffected.
    #[test]
    fn plain_stm32_does_not_pack() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Stm32);
        spi.attach(Box::new(Capture { rx: Vec::new() }));
        spi.write(0x00, 0x40).unwrap();
        spi.write_u16(0x0C, 0x00AB).unwrap();
        assert_eq!(captured(&spi), vec![0xAB], "non-FIFO ⇒ 1 frame");
    }

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
        // No slave wired → no MISO data → RXNE stays clear, DR reads 0.
        assert_eq!(sr & 0x01, 0, "RXNE NOT set without a slave");
        assert_eq!(spi.read(0x0C).unwrap(), 0x00, "DR=0 with no MISO data");
    }
}
