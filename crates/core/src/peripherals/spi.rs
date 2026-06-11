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

use crate::{Bus, SimResult};
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
    /// STM32 families with a TX/RX FIFO + CR2.DS data-size field (L4/F7/G4/…).
    /// Identical register layout to `Stm32`, but a **16-bit DR write at
    /// DS≤8 packs two frames** (RM0351 §40.4.9 data packing) — modelled so
    /// firmware that wrongly uses a 16-bit DR access at 8-bit data size
    /// mis-renders in the sim exactly as it does on silicon.
    Stm32Fifo,
    /// STM32H5/H7 "SPI v3" IP (RM0481 §41) — a different peripheral from the
    /// classic/FIFO map: 32-bit registers, split CFG1/CFG2 configuration,
    /// write-1-to-clear IFCR, CR2.TSIZE frame counting with SR.CTSIZE, and a
    /// CR1.CSTART-gated transfer engine. See [`Stm32H5SpiRegs`].
    Stm32H5,
    Nrf52Spim,
}

impl FromStr for SpiRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32" | "stm32f1" | "stm32f4" | "stm32v2" => Ok(Self::Stm32),
            "stm32_fifo" | "stm32l4" | "stm32f7" | "stm32g4" => Ok(Self::Stm32Fifo),
            // H5 carries the H7-lineage "SPI v3" IP, not the L4/F7 FIFO map.
            "stm32h5" => Ok(Self::Stm32H5),
            "nrf52" | "nrf52_spim" | "nrf_spim" | "nordic" => Ok(Self::Nrf52Spim),
            _ => Err(format!(
                "unsupported SPI register layout '{}'; supported: stm32, stm32_fifo, stm32h5, nrf52",
                value
            )),
        }
    }
}

/// Event token for the one-shot SPI transfer-completion event (the SPI has a
/// single kind of scheduled wakeup, so the value is arbitrary).
const SPI_DONE_TOKEN: u32 = 0;

/// STM32 SPI register file (F1/F4/L0 classic and L4/F7/G4 FIFO share this map;
/// `fifo` selects the FIFO DS/data-packing behaviour). H5/H7 use the separate
/// "SPI v3" map in [`Stm32H5SpiRegs`].
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

/// STM32H5/H7 "SPI v3" register file (RM0481 §41) — H5-only state.
///
/// Register map (RM0481 / CMSIS stm32h563xx.h):
///   0x00 CR1, 0x04 CR2, 0x08 CFG1, 0x0C CFG2, 0x10 IER, 0x14 SR,
///   0x18 IFCR (write-only, reads 0), 0x20 TXDR (write-only),
///   0x30 RXDR (read-only), 0x40 CRCPOLY, 0x44 TXCRC, 0x48 RXCRC,
///   0x4C UDRDR, 0x50 I2SCFGR.
///
/// Reset values, write masks and the mode-fault/SPE-lock machinery are pinned
/// by silicon capture 2026-06-11 (NUCLEO-H563ZI), probed over SWD.
///
/// ── Known divergence from the bench capture ────────────────────────────────
/// The bench part had no SPI kernel clock configured, so real frames never
/// shifted: TXDR writes set TXTF but CTSIZE never moved. The sim is always
/// clocked, so with SPE+CSTART in master mode each TXDR write transmits one
/// frame and decrements CTSIZE (same class of divergence as the RNG
/// kernel-clock note in the chip yaml). RX is not modelled yet: the engine is
/// TX-only and RXDR always reads 0.
#[derive(Debug, Clone, Default, serde::Serialize)]
struct Stm32H5SpiRegs {
    cr1: u32,
    cr2: u32,
    cfg1: u32,
    cfg2: u32,
    ier: u32,
    /// SR flag bits [15:0]; the CTSIZE field [31:16] lives in `ctsize`.
    sr: u32,
    /// SR.CTSIZE — remaining-frame count, loaded from CR2.TSIZE at SPE set.
    ctsize: u32,
    crcpoly: u32,
    txcrc: u32,
    rxcrc: u32,
    udrdr: u32,
    i2scfgr: u32,
}

// ── STM32H5 SPI bit positions (RM0481 §41.4) ────────────────────────────────
/// CR1: peripheral enable.
const H5_CR1_SPE: u32 = 1 << 0;
/// CR1: master transfer start; HW-cleared when CTSIZE reaches 0.
const H5_CR1_CSTART: u32 = 1 << 9;
/// CR1: internal SS level when CFG2.SSM=1.
const H5_CR1_SSI: u32 = 1 << 12;
/// CR1 writable bits: SPE(0), MASRX(8), CSTART(9), HDDIR(11), SSI(12),
/// CRC33_17(13), RCRCINI(14), TCRCINI(15), IOLOCK(16). CSUSP(10) is a
/// write-only strobe and reads 0.
const H5_CR1_WRITABLE: u32 = 0x0001_FB01;

/// SR: TX-packet space available — always set (sim TX path is bottomless).
const H5_SR_TXP: u32 = 1 << 1;
/// SR: end of transfer (CTSIZE reached 0).
const H5_SR_EOT: u32 = 1 << 3;
/// SR: transmission of TxFIFO filled.
const H5_SR_TXTF: u32 = 1 << 4;
/// SR: mode fault.
const H5_SR_MODF: u32 = 1 << 9;
/// SR: transmission complete.
const H5_SR_TXC: u32 = 1 << 12;
/// SR reset value = TXP|TXC — silicon capture 2026-06-11 (NUCLEO-H563ZI).
const H5_SR_RESET: u32 = H5_SR_TXP | H5_SR_TXC;

/// CFG1 reserved bits, read as 0. Derived from the silicon round-trip triple
/// 0x70000007 / 0x00080008 / 0x5555AAAA→0x505582AA — capture 2026-06-11
/// (NUCLEO-H563ZI).
const H5_CFG1_RESERVED: u32 = 0x0500_2800;
/// CFG1 reset = MBR /8, CRCSIZE 8-bit, DSIZE 8-bit — silicon capture
/// 2026-06-11 (NUCLEO-H563ZI).
const H5_CFG1_RESET: u32 = 0x0007_0007;

/// CFG2: master mode select.
const H5_CFG2_MASTER: u32 = 1 << 22;
/// CFG2: software SS management.
const H5_CFG2_SSM: u32 = 1 << 26;

/// IER writable bits [10:0] (RXPIE..TSERFIE).
const H5_IER_WRITABLE: u32 = 0x0000_07FF;

/// IFCR write-1-to-clear mask: EOTC(3), TXTFC(4), UDRC(5), OVRC(6), CRCEC(7),
/// TIFREC(8), MODFC(9), SUSPC(11).
const H5_IFCR_W1C: u32 = 0x0000_0BF8;

/// CRCPOLY reset (CRC-8 x^8+x^2+x+1) — silicon capture 2026-06-11
/// (NUCLEO-H563ZI).
const H5_CRCPOLY_RESET: u32 = 0x0000_0107;

impl Stm32H5SpiRegs {
    fn reset() -> Self {
        Self {
            cfg1: H5_CFG1_RESET,
            sr: H5_SR_RESET,
            crcpoly: H5_CRCPOLY_RESET,
            ..Default::default()
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.cfg1,
            0x0C => self.cfg2,
            0x10 => self.ier,
            // SR[31:16] = CTSIZE remaining-frame count, flags below.
            0x14 => (self.ctsize << 16) | self.sr,
            // IFCR (0x18) and TXDR (0x20) are write-only and read 0; RXDR
            // (0x30) reads 0 in the TX-only model (see struct docs).
            0x18 | 0x20 | 0x30 => 0,
            0x40 => self.crcpoly,
            0x44 => self.txcrc,
            0x48 => self.rxcrc,
            0x4C => self.udrdr,
            0x50 => self.i2scfgr,
            _ => 0,
        }
    }
}

/// Nordic nRF52 SPIM (EasyDMA) register file — Nordic-only state.
///
/// Register offsets follow nRF52840 PS rev 1.7 §6.30 (SPIM).
///
/// TASKS:
///   0x010  TASKS_START  — write 1 arms EasyDMA; handled via needs_bus_tick/tick_with_bus
///   0x014  TASKS_STOP   — write 1 requests a graceful stop
///
/// EVENTS:
///   0x104  EVENTS_STOPPED  — peripheral stopped
///   0x110  EVENTS_ENDRX    — last byte clocked into RXD buffer (HW-set only)
///   0x118  EVENTS_END      — all RXD+TXD transfers complete (HW-set only)
///   0x120  EVENTS_ENDTX    — last byte clocked out of TXD buffer (HW-set only)
///
/// EVENTS write-semantics (silicon-verified for TIMER/RTC, applied uniformly):
///   SW writes of 1 are ignored — only HW sets EVENTS registers.
///   SW writes of 0 clear the event.
///
/// CONFIG:
///   0x554  CONFIG  — ORDER (bit 0), CPHA (bit 1), CPOL (bit 2)
///
/// EasyDMA:
///   0x534  RXD.PTR     — base address for received bytes
///   0x538  RXD.MAXCNT  — max bytes to receive
///   0x53C  RXD.AMOUNT  — bytes actually received (HW-updated, PS §6.30.4D0)
///   0x544  TXD.PTR     — base address for bytes to transmit
///   0x548  TXD.MAXCNT  — number of bytes to transmit
///   0x54C  TXD.AMOUNT  — bytes actually transmitted (HW-updated, PS §6.30.4D8)
///   0x5C0  ORC         — over-read character (sent when TXD exhausted but RXD still running)
#[derive(Debug, Clone, Default, serde::Serialize)]
struct Nrf52SpiRegs {
    // EVENTS — HW-set only; SW may only write 0 to clear
    events_stopped: u32,
    events_endrx: u32,
    events_end: u32,
    events_endtx: u32,

    // INTEN — bit-field enabling each event's IRQ
    inten: u32,

    // Config / pin-select / mode
    enable: u32,
    psel_sck: u32,
    psel_mosi: u32,
    psel_miso: u32,
    frequency: u32,
    config: u32,

    // EasyDMA descriptors
    rxd_ptr: u32,
    rxd_maxcnt: u32,
    rxd_amount: u32,
    txd_ptr: u32,
    txd_maxcnt: u32,
    txd_amount: u32,

    // Over-read character (low 8 bits, rest reserved)
    orc: u32,
}

/// INTEN bit positions (PS §6.30 INTEN register).
/// STOPPED=1, ENDRX=4, END=6, ENDTX=8.
const INTEN_STOPPED: u32 = 1 << 1;
const INTEN_ENDRX: u32 = 1 << 4;
const INTEN_END: u32 = 1 << 6;
const INTEN_ENDTX: u32 = 1 << 8;

impl Nrf52SpiRegs {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            // TASKS read as 0 (write-only strobes on silicon)
            0x010 | 0x014 => 0,
            // EVENTS
            0x104 => self.events_stopped,
            0x110 => self.events_endrx,
            0x118 => self.events_end,
            0x120 => self.events_endtx,
            // INTEN / INTENSET / INTENCLR all mirror the inten value
            0x300 | 0x304 | 0x308 => self.inten,
            // Config
            0x500 => self.enable,
            0x508 => self.psel_sck,
            0x50C => self.psel_mosi,
            0x510 => self.psel_miso,
            0x524 => self.frequency,
            0x554 => self.config,
            // EasyDMA descriptors
            0x534 => self.rxd_ptr,
            0x538 => self.rxd_maxcnt,
            0x53C => self.rxd_amount,
            0x544 => self.txd_ptr,
            0x548 => self.txd_maxcnt,
            0x54C => self.txd_amount,
            // ORC
            0x5C0 => self.orc & 0xFF,
            _ => 0,
        }
    }

    /// Handle MMIO writes for the nRF52 SPIM register file.
    ///
    /// Returns `true` when TASKS_START was triggered (so the caller can set
    /// `pending_start`). TASKS_STOP returns `false` (handled here).
    ///
    /// EVENTS write semantics: SW write of 1 is a no-op (only HW sets events);
    /// SW write of 0 clears the event.
    fn write_reg(&mut self, offset: u64, value: u32) -> bool {
        match offset {
            // TASKS — trigger on non-zero write
            0x010 => return value != 0, // TASKS_START: signal caller
            0x014 => {
                // TASKS_STOP: no state needed; events_stopped set by HW
            }

            // EVENTS — SW write of 1 ignored; SW write of 0 clears
            0x104 if value == 0 => self.events_stopped = 0,
            0x110 if value == 0 => self.events_endrx = 0,
            0x118 if value == 0 => self.events_end = 0,
            0x120 if value == 0 => self.events_endtx = 0,

            // INTEN (direct write)
            0x300 => self.inten = value,
            // INTENSET (set bits)
            0x304 => self.inten |= value,
            // INTENCLR (clear bits)
            0x308 => self.inten &= !value,

            // Config / pin-select
            0x500 => self.enable = value,
            0x508 => self.psel_sck = value,
            0x50C => self.psel_mosi = value,
            0x510 => self.psel_miso = value,
            0x524 => self.frequency = value,
            0x554 => self.config = value,

            // EasyDMA descriptors (AMOUNT registers are HW-written; firmware
            // should not write them, but the model accepts writes so firmware
            // that does an initialising clear doesn't get confused)
            0x534 => self.rxd_ptr = value,
            0x538 => self.rxd_maxcnt = value,
            0x53C => self.rxd_amount = value,
            0x544 => self.txd_ptr = value,
            0x548 => self.txd_maxcnt = value,
            0x54C => self.txd_amount = value,

            // ORC (only low 8 bits are meaningful)
            0x5C0 => self.orc = value & 0xFF,

            _ => {}
        }
        false
    }
}

/// Family-isolated SPI register state. STM32 and nRF register sets cannot
/// coexist on one instance.
#[derive(Debug, Clone, serde::Serialize)]
enum SpiRegs {
    Stm32(Stm32SpiRegs),
    Stm32H5(Stm32H5SpiRegs),
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

    /// nRF52 SPIM: set when TASKS_START is written; cleared after
    /// `tick_with_bus` completes the EasyDMA transfer.
    #[serde(skip)]
    nrf52_pending_start: bool,

    /// Classic-SPI CR2 writable mask — a per-part delta on the shared classic
    /// layout. F1 implements 0xE7; F4 adds bit 4 (FRF, TI-mode) → 0xF7,
    /// silicon-confirmed on the bench F103 (0xE7) and F407 (0xF7). Set from the
    /// chip config's `cr2_mask`. Ignored by the FIFO layout (its own CR2 logic).
    cr2_mask: u32,

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
        Self::new_with_layout_cr2(layout, 0x0000_00E7)
    }

    /// Like [`new_with_layout`] but with an explicit classic-SPI CR2 writable
    /// mask — the per-part delta (F1 `0xE7`, F4 `0xF7` for the FRF bit).
    pub fn new_with_layout_cr2(layout: SpiRegisterLayout, cr2_mask: u32) -> Self {
        let regs = match layout {
            // CR2 reset is silicon-verified over SWD:
            //   FIFO SPI (L4/F7): CR2 = 0x0700 (DS=0b0111 8-bit + FRXTH).
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
            SpiRegisterLayout::Stm32H5 => SpiRegs::Stm32H5(Stm32H5SpiRegs::reset()),
            SpiRegisterLayout::Nrf52Spim => SpiRegs::Nrf52(Nrf52SpiRegs::default()),
        };
        Self {
            regs,
            cr2_mask,
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
                    // Classic SPI CR1 writable mask 0xEFFF (CRCNEXT bit 12 reads
                    // 0) — silicon-confirmed on F103 SPI1. The FIFO variant
                    // (L4/F7/H5) has a different CR1 bit map, so leave it verbatim.
                    r.cr1 = if r.fifo { value } else { value & 0xEFFF };
                }
            }
            0x04 => {
                // STM32L4/F7 SPI CR2: DS[3:0] (bits 11:8) select the data
                // frame size. Values below 0b0011 are reserved and hardware
                // forces them to 0b0111 (8-bit) on FIFO parts — verified on
                // NUCLEO-L476RG (CR2=0x0000 reads back 0x0700). Classic SPI
                // has no DS field; its writable mask is the per-part `cr2_mask`
                // (F1 0xE7, F4 0xF7 for the FRF bit).
                let cr2_mask = self.cr2_mask as u16;
                if let SpiRegs::Stm32(r) = &mut self.regs {
                    if r.fifo {
                        let ds = (value >> 8) & 0xF;
                        r.cr2 = if ds < 0b0011 {
                            (value & !0x0F00) | (0b0111 << 8)
                        } else {
                            value
                        };
                    } else {
                        r.cr2 = value & cr2_mask;
                    }
                }
            }
            0x08 => {
                // SR is mostly read-only; allow clearing OVR if modelled.
                if let SpiRegs::Stm32(r) = &mut self.regs {
                    r.sr = value & 0xFFBF;
                }
            }
            0x10 => {
                // CRCPR: 16-bit CRC polynomial, plain R/W (the model previously
                // dropped writes). Silicon-confirmed writable 0xFFFF on F103.
                if let SpiRegs::Stm32(r) = &mut self.regs {
                    r.crcpr = value;
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

    /// STM32H5 ("SPI v3") register write with transfer-engine side effects.
    /// Only called on the `Stm32H5` variant. Behavioural rules pinned by
    /// silicon capture 2026-06-11 (NUCLEO-H563ZI) unless noted otherwise.
    fn write_stm32h5_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => {
                // CR1
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    let prev = r.cr1;
                    let mut v = value & H5_CR1_WRITABLE;
                    // While the mode-fault condition stands (SR.MODF latched),
                    // setting SPE is refused: CR1 = SPE|SSI after a fault
                    // reads back 0x1000.
                    if r.sr & H5_SR_MODF != 0 {
                        v &= !H5_CR1_SPE;
                    }
                    // CSTART latches while a transfer is active: SW may only
                    // set it (and only under SPE); HW clears it at EOT
                    // (RM0481 §41.4.10).
                    let cstart = (prev & H5_CR1_CSTART != 0)
                        || (value & H5_CR1_CSTART != 0 && v & H5_CR1_SPE != 0);
                    v = (v & !H5_CR1_CSTART) | if cstart { H5_CR1_CSTART } else { 0 };
                    if prev & H5_CR1_SPE == 0 && v & H5_CR1_SPE != 0 {
                        // SPE 0→1: load CTSIZE from CR2.TSIZE; a nonzero
                        // frame count is a pending transfer, so TXC drops
                        // (SR = 0x00020002 with TSIZE=2 on the bench).
                        r.ctsize = r.cr2 & 0xFFFF;
                        if r.ctsize > 0 {
                            r.sr &= !H5_SR_TXC;
                        }
                    } else if prev & H5_CR1_SPE != 0 && v & H5_CR1_SPE == 0 {
                        // SPE 1→0: TXC comes back, CTSIZE is retained
                        // (SR = 0x00021002 on the bench) and the start
                        // request is dropped.
                        r.sr |= H5_SR_TXC;
                        v &= !H5_CR1_CSTART;
                    }
                    r.cr1 = v;
                }
            }
            0x04 => {
                // CR2: TSIZE[15:0] (write 0x10 → reads 0x10 on the bench).
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    r.cr2 = value & 0xFFFF;
                }
            }
            0x08 => {
                // CFG1: ignored while SPE=1 (config lock); reserved bits
                // read as 0.
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    if r.cr1 & H5_CR1_SPE == 0 {
                        r.cfg1 = value & !H5_CFG1_RESERVED;
                    }
                }
            }
            0x0C => {
                // CFG2: ignored while SPE=1. A MASTER request while the
                // internal SS level is low (SSM=1 && CR1.SSI=0) mode-faults:
                // MASTER is refused and SR.MODF latches (CFG2 write
                // 0x04400000 with SSI=0 → reads 0x04000000, SR 0x1202).
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    if r.cr1 & H5_CR1_SPE == 0 {
                        let mut v = value;
                        if v & H5_CFG2_MASTER != 0
                            && v & H5_CFG2_SSM != 0
                            && r.cr1 & H5_CR1_SSI == 0
                        {
                            v &= !H5_CFG2_MASTER;
                            r.sr |= H5_SR_MODF;
                        }
                        r.cfg2 = v;
                    }
                }
            }
            0x10 => {
                // IER (write 0x209 → reads 0x209 on the bench).
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    r.ier = value & H5_IER_WRITABLE;
                }
            }
            0x18 => {
                // IFCR: write-1-to-clear for the clearable SR flags.
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    r.sr &= !(value & H5_IFCR_W1C);
                }
            }
            0x20 => {
                // TXDR — the TX-only data engine (spec-derived; the bench
                // part had no SPI kernel clock, see Stm32H5SpiRegs docs).
                let (spe, started, master) = match &self.regs {
                    SpiRegs::Stm32H5(r) => (
                        r.cr1 & H5_CR1_SPE != 0,
                        r.cr1 & H5_CR1_CSTART != 0,
                        r.cfg2 & H5_CFG2_MASTER != 0,
                    ),
                    _ => return,
                };
                if !spe {
                    return;
                }
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    // Any enabled TXDR write fills the TxFIFO → TXTF; TXP
                    // stays set (sim TX path is bottomless).
                    r.sr |= H5_SR_TXTF;
                }
                if started && master {
                    // One frame per TXDR access. DSIZE (CFG1[4:0]) is stored
                    // but not consumed by the TX-only engine: the low byte is
                    // broadcast, matching the v1 byte-wide device routing.
                    let mosi = (value & 0xFF) as u8;
                    for dev in &mut self.attached_devices {
                        dev.transfer(mosi);
                    }
                    if let SpiRegs::Stm32H5(r) = &mut self.regs {
                        if r.ctsize > 0 {
                            r.ctsize -= 1;
                            if r.ctsize == 0 {
                                // Frame count exhausted: EOT|TXC, start
                                // request HW-cleared. TSIZE=0 (endless mode)
                                // never reaches this — no EOT.
                                r.sr |= H5_SR_EOT | H5_SR_TXC;
                                r.cr1 &= !H5_CR1_CSTART;
                            }
                        }
                    }
                }
            }
            0x40 => {
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    r.crcpoly = value;
                }
            }
            0x4C => {
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    r.udrdr = value;
                }
            }
            0x50 => {
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    r.i2scfgr = value;
                }
            }
            // SR (0x14) is read-only (flags clear via IFCR); TXCRC/RXCRC
            // (0x44/0x48) are HW-computed and read-only.
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
            SpiRegs::Stm32H5(r) => r.read_reg(reg_offset),
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
            let start_triggered = if let SpiRegs::Nrf52(r) = &mut self.regs {
                r.write_reg(reg_offset, new)
            } else {
                false
            };
            if start_triggered {
                self.nrf52_pending_start = true;
            }
            return Ok(());
        }

        // STM32H5: 32-bit registers — read-modify-write merge the byte, then
        // hand the full word to the register handler. The write-only registers
        // (TXDR, IFCR) read back 0, so the merge degenerates to the bare byte
        // shifted into place — a byte write to TXDR is one 8-bit frame, which
        // matches RM0481 §41.4.13 (TXDR access size = frame size).
        if let SpiRegs::Stm32H5(_) = &self.regs {
            let cur = match &self.regs {
                SpiRegs::Stm32H5(r) => r.read_reg(reg_offset),
                _ => 0,
            };
            let mask: u32 = 0xFF << (byte_offset * 8);
            let new = (cur & !mask) | ((value as u32) << (byte_offset * 8));
            self.write_stm32h5_reg(reg_offset, new);
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

    /// For nRF52 SPIM, 32-bit register writes must be handled atomically so
    /// that INTENSET / INTENCLR (set/clear bitmask registers) receive the full
    /// 32-bit value rather than a read-modify-write merge of individual bytes.
    /// The byte-merge in the default `write_u32` would incorrectly OR in bits
    /// from the current register state and cause INTENCLR to clear more bits
    /// than intended. Firmware on Cortex-M always uses STR (32-bit) for
    /// nRF register accesses — this override matches that behaviour.
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if let SpiRegs::Nrf52(_) = &self.regs {
            let reg_offset = offset & !3;
            let start_triggered = if let SpiRegs::Nrf52(r) = &mut self.regs {
                r.write_reg(reg_offset, value)
            } else {
                false
            };
            if start_triggered {
                self.nrf52_pending_start = true;
            }
            return Ok(());
        }
        // STM32H5: 32-bit registers must be written atomically — a word write
        // to TXDR is ONE frame (byte-splitting would transmit four), and IFCR
        // (write-1-to-clear) must see the full mask in a single access.
        if let SpiRegs::Stm32H5(_) = &self.regs {
            self.write_stm32h5_reg(offset & !3, value);
            return Ok(());
        }
        // STM32 default: four byte writes.
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write(offset + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write(offset + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        if self.is_nrf() {
            self.write(offset, (value & 0xFF) as u8)?;
            self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
            return Ok(());
        }
        // STM32H5: a halfword TXDR access is ONE 16-bit frame (RM0481
        // §41.4.13) — byte-splitting would transmit two frames. The classic
        // 0x0C special-case below must not run either: 0x0C is CFG2 on H5.
        if let SpiRegs::Stm32H5(_) = &self.regs {
            if (offset & !3) == 0x20 {
                self.write_stm32h5_reg(0x20, value as u32);
            } else {
                self.write(offset, (value & 0xFF) as u8)?;
                self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
            }
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

    /// nRF52 SPIM EasyDMA needs bus access to read/write RAM buffers.
    fn needs_bus_tick(&self) -> bool {
        self.nrf52_pending_start
    }

    /// nRF52 SPIM EasyDMA transfer engine.
    ///
    /// Reads TXD.MAXCNT bytes from RAM at TXD.PTR, clocks each through the
    /// attached `SpiDevice` (or uses ORC when TXD is exhausted but RXD still
    /// has capacity), writes received bytes to RAM at RXD.PTR up to
    /// RXD.MAXCNT, then sets EVENTS_ENDTX / EVENTS_ENDRX / EVENTS_END and
    /// updates TXD.AMOUNT / RXD.AMOUNT.
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if !self.nrf52_pending_start {
            return;
        }
        self.nrf52_pending_start = false;

        let (txd_ptr, txd_maxcnt, rxd_ptr, rxd_maxcnt, orc) = if let SpiRegs::Nrf52(r) = &self.regs
        {
            (
                r.txd_ptr as u64,
                r.txd_maxcnt as usize,
                r.rxd_ptr as u64,
                r.rxd_maxcnt as usize,
                (r.orc & 0xFF) as u8,
            )
        } else {
            return;
        };

        // Determine the total number of byte-cycles to run: whichever
        // descriptor is larger drives the clock count; the smaller one
        // pads with ORC (TX side) or discards (RX side that is full).
        let n_clocks = txd_maxcnt.max(rxd_maxcnt);

        let mut txd_amount: u32 = 0;
        let mut rxd_amount: u32 = 0;

        for i in 0..n_clocks {
            // Read MOSI byte: TX buffer while available, else ORC.
            let mosi: u8 = if i < txd_maxcnt {
                bus.read_u8(txd_ptr + i as u64).unwrap_or(0)
            } else {
                orc
            };

            if i < txd_maxcnt {
                txd_amount += 1;
            }

            // Clock the byte through the attached device (or loopback /
            // no-device — mirrors MOSI back).
            let miso: u8 = if !self.attached_devices.is_empty() {
                let mut resp: u8 = 0;
                for dev in &mut self.attached_devices {
                    let r = dev.transfer(mosi);
                    if r != 0 {
                        resp = r;
                    }
                }
                resp
            } else if self.loopback {
                mosi
            } else {
                0
            };

            // Write MISO byte to RX buffer if there is still capacity.
            if i < rxd_maxcnt {
                let _ = bus.write_u8(rxd_ptr + i as u64, miso);
                rxd_amount += 1;
            }
        }

        // Update AMOUNT registers and fire completion events.
        if let SpiRegs::Nrf52(r) = &mut self.regs {
            r.txd_amount = txd_amount;
            r.rxd_amount = rxd_amount;
            // HW fires ENDTX, ENDRX, then END (PS §6.30 sequence).
            r.events_endtx = 1;
            r.events_endrx = 1;
            r.events_end = 1;
        }
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        let mut irq = false;
        let mut fired: Vec<u32> = Vec::new();

        // ── nRF52 SPIM: raise IRQ for any enabled+pending EVENTS ─────────────
        if let SpiRegs::Nrf52(r) = &self.regs {
            // Check each event against its INTEN bit.
            if r.events_stopped != 0 && r.inten & INTEN_STOPPED != 0 {
                irq = true;
                fired.push(0x104);
            }
            if r.events_endrx != 0 && r.inten & INTEN_ENDRX != 0 {
                irq = true;
                fired.push(0x110);
            }
            if r.events_end != 0 && r.inten & INTEN_END != 0 {
                irq = true;
                fired.push(0x118);
            }
            if r.events_endtx != 0 && r.inten & INTEN_ENDTX != 0 {
                irq = true;
                fired.push(0x120);
            }
            return crate::PeripheralTickResult {
                irq,
                fired_events: fired,
                ..Default::default()
            };
        }

        // ── STM32 SPI: cycle-counted shift-register transfer ─────────────────
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

    // ── nRF52 SPIM EasyDMA unit tests ─────────────────────────────────────────

    use crate::{Bus, DmaRequest, SimulationConfig};
    use std::collections::HashMap;

    /// Minimal flat-RAM bus for unit tests — no peripherals, just byte array.
    struct FlatRamBus {
        mem: HashMap<u64, u8>,
        config: SimulationConfig,
    }

    impl FlatRamBus {
        fn new() -> Self {
            Self {
                mem: HashMap::new(),
                config: SimulationConfig::default(),
            }
        }

        fn write_slice(&mut self, base: u64, data: &[u8]) {
            for (i, &b) in data.iter().enumerate() {
                self.mem.insert(base + i as u64, b);
            }
        }

        fn read_slice(&self, base: u64, len: usize) -> Vec<u8> {
            (0..len)
                .map(|i| *self.mem.get(&(base + i as u64)).unwrap_or(&0))
                .collect()
        }
    }

    impl Bus for FlatRamBus {
        fn read_u8(&self, addr: u64) -> crate::SimResult<u8> {
            Ok(*self.mem.get(&addr).unwrap_or(&0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> crate::SimResult<()> {
            self.mem.insert(addr, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> crate::SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
    }

    /// Helper: write a u32 to nRF SPIM registers as a single word write
    /// (matches Cortex-M STR instruction semantics used by real firmware).
    fn nrf_write_u32(spi: &mut Spi, offset: u64, value: u32) {
        spi.write_u32(offset, value).unwrap();
    }

    /// Helper: read a u32 from nRF SPIM registers via 4x byte reads.
    fn nrf_read_u32(spi: &Spi, offset: u64) -> u32 {
        let b0 = spi.read(offset).unwrap() as u32;
        let b1 = spi.read(offset + 1).unwrap() as u32;
        let b2 = spi.read(offset + 2).unwrap() as u32;
        let b3 = spi.read(offset + 3).unwrap() as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }

    /// Full EasyDMA transfer with no attached device and no loopback:
    /// TXD bytes are read from RAM, MISO is 0 everywhere.
    /// After tick_with_bus: EVENTS_END/ENDTX/ENDRX all 1,
    /// TXD.AMOUNT == TXD.MAXCNT, RXD.AMOUNT == RXD.MAXCNT,
    /// RXD RAM contains zeros (no device, no loopback).
    #[test]
    fn nrf52_spim_easydma_no_device_txd_and_rxd_amount() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        let mut bus = FlatRamBus::new();

        let tx_base: u64 = 0x2000_0000;
        let rx_base: u64 = 0x2000_0100;
        let tx_data: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];
        bus.write_slice(tx_base, &tx_data);

        // Configure SPIM: ENABLE=7, TXD.PTR/MAXCNT, RXD.PTR/MAXCNT.
        nrf_write_u32(&mut spi, 0x500, 7); // ENABLE = 7
        nrf_write_u32(&mut spi, 0x544, tx_base as u32); // TXD.PTR
        nrf_write_u32(&mut spi, 0x548, 4); // TXD.MAXCNT = 4
        nrf_write_u32(&mut spi, 0x534, rx_base as u32); // RXD.PTR
        nrf_write_u32(&mut spi, 0x538, 4); // RXD.MAXCNT = 4

        // TASKS_START — must not have fired events yet.
        nrf_write_u32(&mut spi, 0x010, 1);
        assert_eq!(
            nrf_read_u32(&spi, 0x118),
            0,
            "EVENTS_END must not be set before tick"
        );
        assert!(spi.needs_bus_tick(), "pending_start must be set");

        // Run EasyDMA.
        spi.tick_with_bus(&mut bus);

        // Completion events.
        assert_eq!(
            nrf_read_u32(&spi, 0x118),
            1,
            "EVENTS_END must be 1 after transfer"
        );
        assert_eq!(nrf_read_u32(&spi, 0x120), 1, "EVENTS_ENDTX must be 1");
        assert_eq!(nrf_read_u32(&spi, 0x110), 1, "EVENTS_ENDRX must be 1");

        // AMOUNT registers.
        assert_eq!(nrf_read_u32(&spi, 0x54C), 4, "TXD.AMOUNT must be 4");
        assert_eq!(nrf_read_u32(&spi, 0x53C), 4, "RXD.AMOUNT must be 4");

        // No device/loopback → MISO is all zeros.
        let rx = bus.read_slice(rx_base, 4);
        assert_eq!(rx, vec![0, 0, 0, 0], "RXD RAM must be zeros with no device");

        // needs_bus_tick must be clear after completion.
        assert!(
            !spi.needs_bus_tick(),
            "pending_start must be cleared after tick_with_bus"
        );
    }

    /// Full EasyDMA transfer with loopback (MOSI → MISO mirror):
    /// RXD RAM should contain the same bytes that were transmitted.
    #[test]
    fn nrf52_spim_easydma_loopback_rxd_mirrors_txd() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        spi.set_loopback(true);
        let mut bus = FlatRamBus::new();

        let tx_base: u64 = 0x2000_0200;
        let rx_base: u64 = 0x2000_0300;
        let tx_data: [u8; 5] = [0x11, 0x22, 0x33, 0x44, 0x55];
        bus.write_slice(tx_base, &tx_data);

        nrf_write_u32(&mut spi, 0x500, 7);
        nrf_write_u32(&mut spi, 0x544, tx_base as u32);
        nrf_write_u32(&mut spi, 0x548, 5);
        nrf_write_u32(&mut spi, 0x534, rx_base as u32);
        nrf_write_u32(&mut spi, 0x538, 5);

        nrf_write_u32(&mut spi, 0x010, 1); // TASKS_START
        spi.tick_with_bus(&mut bus);

        // With loopback, each MISO byte is the same as the MOSI byte.
        let rx = bus.read_slice(rx_base, 5);
        assert_eq!(rx, tx_data.to_vec(), "loopback: RXD == TXD");
        assert_eq!(nrf_read_u32(&spi, 0x54C), 5, "TXD.AMOUNT");
        assert_eq!(nrf_read_u32(&spi, 0x53C), 5, "RXD.AMOUNT");
        assert_eq!(nrf_read_u32(&spi, 0x118), 1, "EVENTS_END");
    }

    /// Attached SpiDevice (echo slave): every MOSI byte is returned as-is.
    /// RXD RAM should contain the transmitted bytes.
    #[test]
    fn nrf52_spim_easydma_echo_device_rxd_contains_mosi() {
        struct EchoSlave;
        impl SpiDevice for EchoSlave {
            fn transfer(&mut self, mosi: u8) -> u8 {
                mosi
            }
            fn cs_pin(&self) -> &str {
                ""
            }
        }

        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        spi.attach(Box::new(EchoSlave));
        let mut bus = FlatRamBus::new();

        let tx_base: u64 = 0x2000_0400;
        let rx_base: u64 = 0x2000_0500;
        let tx_data: [u8; 3] = [0xA1, 0xB2, 0xC3];
        bus.write_slice(tx_base, &tx_data);

        nrf_write_u32(&mut spi, 0x500, 7);
        nrf_write_u32(&mut spi, 0x544, tx_base as u32);
        nrf_write_u32(&mut spi, 0x548, 3);
        nrf_write_u32(&mut spi, 0x534, rx_base as u32);
        nrf_write_u32(&mut spi, 0x538, 3);
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);

        let rx = bus.read_slice(rx_base, 3);
        assert_eq!(
            rx,
            tx_data.to_vec(),
            "echo device: RXD == TXD (MISO mirrors MOSI)"
        );
        assert_eq!(nrf_read_u32(&spi, 0x118), 1, "EVENTS_END");
        assert_eq!(nrf_read_u32(&spi, 0x54C), 3, "TXD.AMOUNT == 3");
        assert_eq!(nrf_read_u32(&spi, 0x53C), 3, "RXD.AMOUNT == 3");
    }

    /// RXD.MAXCNT < TXD.MAXCNT: RXD fills up, remaining MISO bytes are discarded.
    /// TXD.AMOUNT == TXD.MAXCNT, RXD.AMOUNT == RXD.MAXCNT.
    #[test]
    fn nrf52_spim_easydma_rxd_maxcnt_limits_rxd_amount() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        spi.set_loopback(true);
        let mut bus = FlatRamBus::new();

        let tx_base: u64 = 0x2000_0600;
        let rx_base: u64 = 0x2000_0700;
        bus.write_slice(tx_base, &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);

        nrf_write_u32(&mut spi, 0x544, tx_base as u32);
        nrf_write_u32(&mut spi, 0x548, 6); // TXD.MAXCNT = 6
        nrf_write_u32(&mut spi, 0x534, rx_base as u32);
        nrf_write_u32(&mut spi, 0x538, 3); // RXD.MAXCNT = 3 (less)
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);

        assert_eq!(nrf_read_u32(&spi, 0x54C), 6, "TXD.AMOUNT == 6");
        assert_eq!(nrf_read_u32(&spi, 0x53C), 3, "RXD.AMOUNT == 3 (clamped)");
        // Only first 3 bytes written to RX buffer.
        let rx = bus.read_slice(rx_base, 3);
        assert_eq!(rx, vec![0x01, 0x02, 0x03], "first 3 bytes received");
    }

    /// ORC (over-read character): when TXD.MAXCNT < RXD.MAXCNT, the ORC byte
    /// is clocked out for the extra cycles. With loopback, those ORC bytes
    /// end up in the RXD buffer.
    #[test]
    fn nrf52_spim_easydma_orc_pads_extra_rx_cycles() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        spi.set_loopback(true);
        let mut bus = FlatRamBus::new();

        let tx_base: u64 = 0x2000_0800;
        let rx_base: u64 = 0x2000_0900;
        bus.write_slice(tx_base, &[0xAA, 0xBB]); // 2 TX bytes

        nrf_write_u32(&mut spi, 0x5C0, 0xFF); // ORC = 0xFF
        nrf_write_u32(&mut spi, 0x544, tx_base as u32);
        nrf_write_u32(&mut spi, 0x548, 2); // TXD.MAXCNT = 2
        nrf_write_u32(&mut spi, 0x534, rx_base as u32);
        nrf_write_u32(&mut spi, 0x538, 4); // RXD.MAXCNT = 4 (2 extra)
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);

        // TXD.AMOUNT counts actual TX bytes, not ORC clocks.
        assert_eq!(nrf_read_u32(&spi, 0x54C), 2, "TXD.AMOUNT == 2 (not 4)");
        assert_eq!(nrf_read_u32(&spi, 0x53C), 4, "RXD.AMOUNT == 4");
        let rx = bus.read_slice(rx_base, 4);
        // Loopback: first 2 = TXD bytes, last 2 = ORC (0xFF).
        assert_eq!(rx, vec![0xAA, 0xBB, 0xFF, 0xFF], "ORC fills extra RX slots");
    }

    /// EVENTS write semantics: SW writing 1 to an EVENTS register must NOT set it.
    /// Only SW writing 0 clears it.
    #[test]
    fn nrf52_spim_events_write_1_ignored() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        let mut bus = FlatRamBus::new();

        nrf_write_u32(&mut spi, 0x500, 7);
        nrf_write_u32(&mut spi, 0x548, 2);
        nrf_write_u32(&mut spi, 0x544, 0x2000_0000_u32);
        nrf_write_u32(&mut spi, 0x538, 2);
        nrf_write_u32(&mut spi, 0x534, 0x2000_0100_u32);

        // Arm and run transfer.
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);
        assert_eq!(nrf_read_u32(&spi, 0x118), 1, "EVENTS_END set by HW");
        assert_eq!(nrf_read_u32(&spi, 0x120), 1, "EVENTS_ENDTX set by HW");
        assert_eq!(nrf_read_u32(&spi, 0x110), 1, "EVENTS_ENDRX set by HW");

        // SW write of 1 must be ignored (silicon-verified rule).
        nrf_write_u32(&mut spi, 0x118, 1); // attempt to SET EVENTS_END — must be ignored
        assert_eq!(
            nrf_read_u32(&spi, 0x118),
            1,
            "EVENTS_END unchanged by SW write of 1"
        );

        // SW write of 0 clears it.
        nrf_write_u32(&mut spi, 0x118, 0);
        assert_eq!(
            nrf_read_u32(&spi, 0x118),
            0,
            "EVENTS_END cleared by SW write of 0"
        );
        nrf_write_u32(&mut spi, 0x120, 0);
        assert_eq!(
            nrf_read_u32(&spi, 0x120),
            0,
            "EVENTS_ENDTX cleared by SW write of 0"
        );
        nrf_write_u32(&mut spi, 0x110, 0);
        assert_eq!(
            nrf_read_u32(&spi, 0x110),
            0,
            "EVENTS_ENDRX cleared by SW write of 0"
        );
    }

    /// TASKS_START before tick_with_bus: EVENTS must not be set immediately.
    /// They should only appear after tick_with_bus runs.
    #[test]
    fn nrf52_spim_events_not_set_before_tick_with_bus() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);

        nrf_write_u32(&mut spi, 0x500, 7);
        nrf_write_u32(&mut spi, 0x548, 1);
        nrf_write_u32(&mut spi, 0x544, 0x2000_0000_u32);

        // Before TASKS_START: no events.
        assert_eq!(nrf_read_u32(&spi, 0x118), 0, "EVENTS_END initially 0");
        assert_eq!(nrf_read_u32(&spi, 0x120), 0, "EVENTS_ENDTX initially 0");
        assert_eq!(nrf_read_u32(&spi, 0x110), 0, "EVENTS_ENDRX initially 0");

        // After TASKS_START but BEFORE tick_with_bus: still 0.
        nrf_write_u32(&mut spi, 0x010, 1);
        assert_eq!(
            nrf_read_u32(&spi, 0x118),
            0,
            "EVENTS_END must not fire before tick"
        );
        assert_eq!(nrf_read_u32(&spi, 0x120), 0, "EVENTS_ENDTX before tick");
        assert_eq!(nrf_read_u32(&spi, 0x110), 0, "EVENTS_ENDRX before tick");
    }

    /// INTENSET / INTENCLR round-trip.
    #[test]
    fn nrf52_spim_intenset_intenclr_round_trip() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);

        // INTENSET: bit 6 = INTEN_END, bit 8 = INTEN_ENDTX.
        nrf_write_u32(&mut spi, 0x304, (1 << 6) | (1 << 8));
        assert_eq!(
            nrf_read_u32(&spi, 0x304),
            (1 << 6) | (1 << 8),
            "INTENSET sets bits"
        );

        // INTENCLR: clear bit 6 only.
        nrf_write_u32(&mut spi, 0x308, 1 << 6);
        assert_eq!(nrf_read_u32(&spi, 0x308), 1 << 8, "INTENCLR clears bit 6");
    }

    /// ORC register stores only the low 8 bits.
    #[test]
    fn nrf52_spim_orc_masks_to_8_bits() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        nrf_write_u32(&mut spi, 0x5C0, 0xFFFF_FFAB);
        assert_eq!(
            nrf_read_u32(&spi, 0x5C0),
            0xAB,
            "ORC retains only low 8 bits"
        );
    }

    /// ENABLE register round-trip.
    #[test]
    fn nrf52_spim_enable_round_trip() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        nrf_write_u32(&mut spi, 0x500, 7);
        assert_eq!(nrf_read_u32(&spi, 0x500), 7, "ENABLE round-trips");
    }

    /// TASKS registers read back as 0 (write-only strobes on silicon).
    #[test]
    fn nrf52_spim_tasks_read_as_zero() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        nrf_write_u32(&mut spi, 0x010, 1); // TASKS_START
        assert_eq!(nrf_read_u32(&spi, 0x010), 0, "TASKS_START reads as 0");
    }

    /// Second TASKS_START after a completed transfer re-arms the engine.
    #[test]
    fn nrf52_spim_easydma_second_start_reruns_transfer() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        spi.set_loopback(true);
        let mut bus = FlatRamBus::new();

        let tx_base: u64 = 0x2000_0A00;
        let rx_base: u64 = 0x2000_0B00;
        bus.write_slice(tx_base, &[0x01, 0x02]);

        nrf_write_u32(&mut spi, 0x544, tx_base as u32);
        nrf_write_u32(&mut spi, 0x548, 2);
        nrf_write_u32(&mut spi, 0x534, rx_base as u32);
        nrf_write_u32(&mut spi, 0x538, 2);
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);
        assert_eq!(nrf_read_u32(&spi, 0x54C), 2);

        // Update TX buffer and run a second transfer.
        bus.write_slice(tx_base, &[0x55, 0x66]);
        nrf_write_u32(&mut spi, 0x118, 0); // clear EVENTS_END
        nrf_write_u32(&mut spi, 0x120, 0); // clear EVENTS_ENDTX
        nrf_write_u32(&mut spi, 0x110, 0); // clear EVENTS_ENDRX
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);

        let rx = bus.read_slice(rx_base, 2);
        assert_eq!(rx, vec![0x55, 0x66], "second transfer sees new TX data");
        assert_eq!(
            nrf_read_u32(&spi, 0x118),
            1,
            "EVENTS_END after second transfer"
        );
    }

    /// tick_with_bus with TXD.MAXCNT == 0 and RXD.MAXCNT == 0: completes
    /// immediately with AMOUNT == 0 and all events fired.
    #[test]
    fn nrf52_spim_easydma_zero_length_transfer() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        let mut bus = FlatRamBus::new();

        nrf_write_u32(&mut spi, 0x544, 0x2000_0000);
        nrf_write_u32(&mut spi, 0x548, 0); // TXD.MAXCNT = 0
        nrf_write_u32(&mut spi, 0x534, 0x2000_0100);
        nrf_write_u32(&mut spi, 0x538, 0); // RXD.MAXCNT = 0
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);

        assert_eq!(nrf_read_u32(&spi, 0x54C), 0, "TXD.AMOUNT == 0");
        assert_eq!(nrf_read_u32(&spi, 0x53C), 0, "RXD.AMOUNT == 0");
        assert_eq!(
            nrf_read_u32(&spi, 0x118),
            1,
            "EVENTS_END fires even for zero-length"
        );
        assert_eq!(nrf_read_u32(&spi, 0x120), 1, "EVENTS_ENDTX fires");
        assert_eq!(nrf_read_u32(&spi, 0x110), 1, "EVENTS_ENDRX fires");
    }

    // ── STM32H5 ("SPI v3", RM0481) unit tests ────────────────────────────────
    // Register-level expectations pinned by silicon capture 2026-06-11
    // (NUCLEO-H563ZI), probed over SWD. The TX data engine is spec-derived
    // (the bench part had no SPI kernel clock — see Stm32H5SpiRegs docs).

    fn h5() -> Spi {
        Spi::new_with_layout(SpiRegisterLayout::Stm32H5)
    }

    fn h5_read(spi: &Spi, offset: u64) -> u32 {
        spi.read_u32(offset).unwrap()
    }

    fn h5_write(spi: &mut Spi, offset: u64, value: u32) {
        spi.write_u32(offset, value).unwrap();
    }

    /// Master-mode bring-up: CR1.SSI=1, then CFG2 = MASTER|SSM, CR2.TSIZE.
    fn h5_master(tsize: u32) -> Spi {
        let mut spi = h5();
        h5_write(&mut spi, 0x00, 1 << 12); // CR1.SSI = 1 (internal SS high)
        h5_write(&mut spi, 0x0C, (1 << 22) | (1 << 26)); // CFG2 = MASTER|SSM
        h5_write(&mut spi, 0x04, tsize); // CR2.TSIZE
        spi
    }

    /// The chip-yaml token "stm32h5" selects the v3 layout, NOT the L4/F7
    /// FIFO map it used to alias.
    #[test]
    fn stm32h5_from_str_selects_v3_layout() {
        assert_eq!(
            "stm32h5".parse::<SpiRegisterLayout>().unwrap(),
            SpiRegisterLayout::Stm32H5
        );
        assert_eq!(
            "stm32l4".parse::<SpiRegisterLayout>().unwrap(),
            SpiRegisterLayout::Stm32Fifo,
            "L4/F7/G4 stay on the FIFO layout"
        );
    }

    /// Reset values — silicon capture 2026-06-11 (NUCLEO-H563ZI).
    #[test]
    fn stm32h5_reset_values_match_silicon() {
        let spi = h5();
        assert_eq!(h5_read(&spi, 0x00), 0, "CR1");
        assert_eq!(h5_read(&spi, 0x04), 0, "CR2");
        assert_eq!(h5_read(&spi, 0x08), 0x0007_0007, "CFG1");
        assert_eq!(h5_read(&spi, 0x0C), 0, "CFG2");
        assert_eq!(h5_read(&spi, 0x10), 0, "IER");
        assert_eq!(h5_read(&spi, 0x14), 0x0000_1002, "SR = TXP|TXC");
        assert_eq!(h5_read(&spi, 0x18), 0, "IFCR is write-only, reads 0");
        assert_eq!(h5_read(&spi, 0x20), 0, "TXDR is write-only, reads 0");
        assert_eq!(h5_read(&spi, 0x30), 0, "RXDR");
        assert_eq!(h5_read(&spi, 0x40), 0x0000_0107, "CRCPOLY");
        assert_eq!(h5_read(&spi, 0x44), 0, "TXCRC");
        assert_eq!(h5_read(&spi, 0x48), 0, "RXCRC");
        assert_eq!(h5_read(&spi, 0x4C), 0, "UDRDR");
        assert_eq!(h5_read(&spi, 0x50), 0, "I2SCFGR");
    }

    /// CFG1 writable mask — all three silicon round-trip pairs.
    #[test]
    fn stm32h5_cfg1_reserved_bits_masked() {
        let mut spi = h5();
        h5_write(&mut spi, 0x08, 0x7000_0007);
        assert_eq!(h5_read(&spi, 0x08), 0x7000_0007);
        h5_write(&mut spi, 0x08, 0x0008_0008);
        assert_eq!(h5_read(&spi, 0x08), 0x0008_0008);
        h5_write(&mut spi, 0x08, 0x5555_AAAA);
        assert_eq!(
            h5_read(&spi, 0x08),
            0x5055_82AA,
            "reserved bits 0x05002800 read as 0"
        );
    }

    /// CR2.TSIZE, CRCPOLY and IER round-trip the silicon-probed values.
    #[test]
    fn stm32h5_config_round_trips() {
        let mut spi = h5();
        h5_write(&mut spi, 0x04, 0x10);
        assert_eq!(h5_read(&spi, 0x04), 0x10, "CR2.TSIZE");
        h5_write(&mut spi, 0x40, 0xA5A5);
        assert_eq!(h5_read(&spi, 0x40), 0xA5A5, "CRCPOLY");
        h5_write(&mut spi, 0x10, 0x209);
        assert_eq!(h5_read(&spi, 0x10), 0x209, "IER");
    }

    /// MASTER is accepted when the internal SS level is high (SSM=1, SSI=1).
    #[test]
    fn stm32h5_cfg2_master_accepted_when_ssi_high() {
        let mut spi = h5();
        h5_write(&mut spi, 0x00, 1 << 12); // CR1.SSI = 1 first
        h5_write(&mut spi, 0x0C, (1 << 22) | (1 << 26));
        assert_eq!(h5_read(&spi, 0x0C), 0x0440_0000);
        assert_eq!(h5_read(&spi, 0x14), 0x0000_1002, "no MODF");
    }

    /// Mode fault: MASTER requested with SSM=1 while SSI=0 → MASTER refused,
    /// SR.MODF latches, SPE is refused until IFCR clears MODF.
    #[test]
    fn stm32h5_mode_fault_refuses_master_and_blocks_spe() {
        let mut spi = h5();
        // SSI is 0 at reset: the MASTER|SSM request mode-faults.
        h5_write(&mut spi, 0x0C, 0x0440_0000);
        assert_eq!(h5_read(&spi, 0x0C), 0x0400_0000, "MASTER stored as 0");
        assert_eq!(h5_read(&spi, 0x14), 0x0000_1202, "SR = TXP|MODF|TXC");
        // SPE refused while the fault stands.
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // SPE|SSI
        assert_eq!(h5_read(&spi, 0x00), 0x0000_1000, "SPE refused, SSI kept");
        // IFCR bit 9 clears MODF; MASTER and SPE then go through.
        h5_write(&mut spi, 0x18, 1 << 9);
        assert_eq!(h5_read(&spi, 0x14), 0x0000_1002, "MODF cleared via IFCR");
        h5_write(&mut spi, 0x0C, 0x0440_0000);
        assert_eq!(h5_read(&spi, 0x0C), 0x0440_0000, "MASTER accepted (SSI=1)");
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12));
        assert_eq!(h5_read(&spi, 0x00) & 1, 1, "SPE accepted after clear");
    }

    /// While SPE=1 the configuration registers are locked: CFG1/CFG2 writes
    /// are ignored.
    #[test]
    fn stm32h5_spe_locks_cfg1_and_cfg2() {
        let mut spi = h5_master(2);
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // SPE|SSI
        h5_write(&mut spi, 0x0C, 0x0440_0000 | (1 << 29));
        assert_eq!(h5_read(&spi, 0x0C), 0x0440_0000, "CFG2 locked under SPE");
        h5_write(&mut spi, 0x08, 0x7000_0007);
        assert_eq!(h5_read(&spi, 0x08), 0x0007_0007, "CFG1 locked under SPE");
    }

    /// Setting SPE loads SR.CTSIZE from CR2.TSIZE and clears TXC (a transfer
    /// is pending).
    #[test]
    fn stm32h5_spe_loads_ctsize_and_clears_txc() {
        let mut spi = h5_master(2);
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // SPE|SSI
        assert_eq!(h5_read(&spi, 0x14), 0x0002_0002, "CTSIZE=2, TXP, TXC off");
    }

    /// CR1.CSTART latches while a transfer is active and cannot be cleared by
    /// software (HW clears it at EOT — RM0481 §41.4.10).
    #[test]
    fn stm32h5_cstart_latches_while_transfer_active() {
        let mut spi = h5_master(2);
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 9) | (1 << 12)); // SPE|CSTART|SSI
        assert_eq!(h5_read(&spi, 0x00), 0x0000_1201, "CSTART latched");
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // try to drop CSTART
        assert_eq!(h5_read(&spi, 0x00), 0x0000_1201, "CSTART not SW-clearable");
    }

    /// The bench TXDR/IFCR/SPE-clear sequence. CSTART is left clear so no
    /// frame shifts and CTSIZE stays put — exactly the unclocked-silicon
    /// behaviour captured on the bench.
    #[test]
    fn stm32h5_txdr_txtf_ifcr_and_spe_clear_sequence() {
        let mut spi = h5_master(2);
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // SPE|SSI
        h5_write(&mut spi, 0x20, 0xAB); // TXDR
        assert_eq!(h5_read(&spi, 0x14), 0x0002_0012, "TXP|TXTF, CTSIZE=2");
        h5_write(&mut spi, 0x18, 0xFFFF_FFFF); // IFCR: clear all clearables
        assert_eq!(h5_read(&spi, 0x14), 0x0002_0002, "TXTF cleared");
        h5_write(&mut spi, 0x00, 1 << 12); // SPE → 0
        assert_eq!(h5_read(&spi, 0x14), 0x0002_1002, "TXC set, CTSIZE kept");
    }

    /// Sim-side TX engine: with SPE+CSTART in master mode each TXDR write
    /// transmits one frame and decrements CTSIZE; at 0 → EOT|TXC, CSTART
    /// HW-cleared. RXDR stays 0 (TX-only model).
    #[test]
    fn stm32h5_tx_engine_transmits_and_completes() {
        let mut spi = h5_master(2);
        spi.attach(Box::new(Capture { rx: Vec::new() }));
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 9) | (1 << 12)); // SPE|CSTART|SSI
        h5_write(&mut spi, 0x20, 0x11);
        assert_eq!(h5_read(&spi, 0x14), 0x0001_0012, "CTSIZE 2→1, TXP|TXTF");
        h5_write(&mut spi, 0x20, 0x22);
        assert_eq!(captured(&spi), vec![0x11, 0x22], "both frames on the bus");
        assert_eq!(h5_read(&spi, 0x14), 0x0000_101A, "EOT|TXC at CTSIZE=0");
        assert_eq!(h5_read(&spi, 0x00), 0x0000_1001, "CSTART HW-cleared");
        assert_eq!(h5_read(&spi, 0x30), 0, "RXDR TX-only: reads 0");
    }

    /// TXDR writes are inert while SPE=0: no TXTF, nothing transmitted.
    #[test]
    fn stm32h5_txdr_ignored_when_disabled() {
        let mut spi = h5_master(2);
        spi.attach(Box::new(Capture { rx: Vec::new() }));
        h5_write(&mut spi, 0x20, 0xAB);
        assert_eq!(h5_read(&spi, 0x14), 0x0000_1002, "SR untouched");
        assert!(captured(&spi).is_empty(), "nothing transmitted");
    }

    /// TXDR byte/halfword accesses are each ONE frame (RM0481 §41.4.13:
    /// access size = frame size). TSIZE=0 = endless mode: CTSIZE stays 0,
    /// no EOT, CSTART stays latched.
    #[test]
    fn stm32h5_byte_and_halfword_txdr_access_is_one_frame() {
        let mut spi = h5_master(0); // TSIZE=0: endless
        spi.attach(Box::new(Capture { rx: Vec::new() }));
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 9) | (1 << 12));
        spi.write(0x20, 0x5A).unwrap(); // byte access → one 8-bit frame
        spi.write_u16(0x20, 0x1234).unwrap(); // halfword access → one frame
        assert_eq!(captured(&spi), vec![0x5A, 0x34], "low byte per frame");
        assert_eq!(h5_read(&spi, 0x14) >> 16, 0, "CTSIZE stays 0");
        assert_eq!(h5_read(&spi, 0x14) & (1 << 3), 0, "no EOT in endless mode");
        assert_eq!(h5_read(&spi, 0x00), 0x0000_1201, "CSTART stays latched");
    }

    /// Config registers are 32-bit with byte-merge semantics on the byte path.
    #[test]
    fn stm32h5_byte_writes_merge_into_32bit_registers() {
        let mut spi = h5();
        spi.write(0x40, 0xA5).unwrap(); // CRCPOLY low byte (reset 0x107)
        spi.write(0x41, 0x5A).unwrap(); // CRCPOLY byte 1
        assert_eq!(h5_read(&spi, 0x40), 0x0000_5AA5, "bytes merged in place");
    }
}
