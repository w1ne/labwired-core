// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 GP-SPI master controller (SPI2 / FSPI and SPI3) — digital twin.
//!
//! This is the *general-purpose* SPI master the firmware drives via the
//! `spi_master` / `esp_lcd` / Arduino `SPI` drivers — DISTINCT from the flash
//! controller modeled in `spi_mem_flash.rs` (SPIMEM1 @ `0x6000_2000`). One
//! `Esp32s3Spi` instance models one controller; the parent registers two:
//!   * SPI2 / FSPI @ `0x6002_4000`, intr-matrix source `ETS_SPI2_INTR_SOURCE` = 21
//!   * SPI3        @ `0x6002_5000`, intr-matrix source `ETS_SPI3_INTR_SOURCE` = 22
//!
//! (sources verified from `soc/esp32s3/include/soc/interrupts.h`: the enum
//! anchors `ETS_GPIO_INTR_SOURCE = 16`, then GPIO_NMI=17, GPIO2=18, GPIO_NMI2=19,
//! SPI1=20, SPI2=21, SPI3=22, with `ETS_LCD_CAM_INTR_SOURCE = 24` confirming the
//! count.)
//!
//! ## Register file
//!
//! All 38 architected registers of the ESP32-S3 SVD `SPI2` block are modeled
//! as a fixed register file: each register is seeded with its silicon reset
//! value and a write applies the register's writable-bit mask
//! (`stored = (stored & !wmask) | (value & wmask)`) — reserved bits read back
//! their reset value, never arbitrary written data. Offsets outside the
//! architected map (the 0x48..0x94 hole, 0xD8/0xDC, 0xEC, and everything
//! above 0xF0) read as zero and ignore writes, NOT round-trip, so the SVD
//! behavioral coverage probe cannot mistake this model for generic storage.
//!
//! Registers with side effects (offsets per `soc/esp32s3/register/soc/spi_reg.h`):
//!
//! | offset | reg          | behavior                                            |
//! |--------|--------------|-----------------------------------------------------|
//! | 0x00   | CMD          | bit24 `SPI_USR` start: set → launch, auto-clears;   |
//! |        |              | bit23 `SPI_UPDATE` self-clears (config sync)        |
//! | 0x1C   | MS_DLEN      | `SPI_MS_DATA_BITLEN`[17:0] = transfer bits-1        |
//! | 0x30   | DMA_CONF     | AFIFO reset bits 29/30/31 self-clear; bit 27       |
//! |        |              | `SPI_DMA_RX_ENA` / bit 28 `SPI_DMA_TX_ENA` select  |
//! |        |              | the GDMA-coupled transaction path (see below)       |
//! | 0x34   | DMA_INT_ENA  | interrupt enable mask                               |
//! | 0x38   | DMA_INT_CLR  | WO, W1C — clears latched raw bits                   |
//! | 0x3C   | DMA_INT_RAW  | R/WTC — write-1-to-clear latched raw bits           |
//! | 0x40   | DMA_INT_ST   | RO: INT_RAW & INT_ENA                               |
//! | 0x44   | DMA_INT_SET  | WO, W1S — software-sets raw bits                    |
//! | 0x98.. | W0..W15      | 16-word data buffer (MOSI out / MISO in)            |
//! | 0xE0   | SLAVE        | bit27 `SPI_SOFT_RESET` (WT) self-clears             |
//!
//! Note: the GP-SPI controller has a *single* `SPI_MS_DLEN` (0x1C) — there are
//! no separate MISO_DLEN / MOSI_DLEN registers as on SPIMEM. All transaction
//! interrupts (including `SPI_TRANS_DONE`, bit 12) live in the `SPI_DMA_INT_*`
//! register block (0x34/0x38/0x3C/0x40/0x44), per the header.
//!
//! ## Transaction model
//!
//! A master programs the phases (USER/USER1/USER2), the bit length
//! (MS_DLEN), and the MOSI payload into `W0..W15`, then sets `SPI_CMD.USR`
//! (bit 24) to launch. Firmware then busy-polls `!(CMD & USR)` for completion.
//! We complete IMMEDIATELY: clear the `USR` bit (so the poll exits) and latch
//! `SPI_TRANS_DONE` in `SPI_DMA_INT_RAW`. With no device attached in scope,
//! the faithful "no device on the bus" behavior is an idle (pulled-high) MISO
//! line, so we fill the MISO data region of `W0..W15` with `0xFF` per byte —
//! exactly the all-ones a real controller shifts in when MISO floats high.
//! The MISO region length is taken from `SPI_MS_DLEN` (`SPI_MS_DATA_BITLEN`+1
//! bits, rounded up to whole bytes, capped at the 64-byte W buffer).
//!
//! ## DMA-mode transactions (GDMA-coupled)
//!
//! When `SPI_CMD.USR` is kicked while `SPI_DMA_TX_ENA` (DMA_CONF bit 28)
//! and/or `SPI_DMA_RX_ENA` (bit 27) are set — the ESP-IDF `spi_master`
//! driver's DMA path — the controller does NOT complete immediately:
//! `USR` stays set and a [`SpiDmaPending`] records the transaction. The
//! GDMA peripheral's SPI pump (gdma.rs) then supplies MOSI bytes from its
//! OUT descriptor chain and collects MISO bytes into its IN chain via
//! [`Esp32s3Spi::dma_transfer`] (attached-device response, or 0xFF with no
//! device), and finally calls [`Esp32s3Spi::dma_complete`] which clears
//! `USR` and latches `SPI_TRANS_DONE`. The W0..W15 buffer is untouched in
//! DMA mode; the non-DMA CPU path below is byte-identical to before.
//!
//! `tick()` emits the controller's intr-matrix source while `INT_ST != 0`,
//! mirroring the UART/systimer pattern; the bus routes it through the per-core
//! interrupt matrix.

use crate::peripherals::esp_gpspi_wire::EspSpiWire;
use crate::peripherals::spi::SpiDevice;
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

/// ESP32-S3 core clock. The GP-SPI divisors count APB ticks and the engine's
/// cycle axis is CPU cycles, so the narration scales by `CPU_CLOCK_HZ /
/// APB_CLK_HZ` — the same conversion `esp_uart` applies to its baud divisor.
/// The S3 runs its core at 240 MHz; the C3's copy of this model says 160.
const CPU_CLOCK_HZ: u64 = 240_000_000;

const CMD: u64 = 0x00;
const ADDR: u64 = 0x04;
const CTRL: u64 = 0x08;
const CLOCK: u64 = 0x0C;
const USER: u64 = 0x10;
const USER1: u64 = 0x14;
const USER2: u64 = 0x18;
const MS_DLEN: u64 = 0x1C;
const MISC: u64 = 0x20;
const DIN_MODE: u64 = 0x24;
const DIN_NUM: u64 = 0x28;
const DOUT_MODE: u64 = 0x2C;
const DMA_CONF: u64 = 0x30;
const DMA_INT_ENA: u64 = 0x34;
const DMA_INT_CLR: u64 = 0x38;
const DMA_INT_RAW: u64 = 0x3C;
const DMA_INT_ST: u64 = 0x40;
const DMA_INT_SET: u64 = 0x44;
const W0: u64 = 0x98;
/// `SPI_W15` (0xD4) — last word of the 16-word data buffer.
const W15: u64 = 0xD4;
const SLAVE: u64 = 0xE0;
const SLAVE1: u64 = 0xE4;
const CLK_GATE: u64 = 0xE8;
const DATE: u64 = 0xF0;

/// `SPI_USR` launch bit in `SPI_CMD` (bitpos 24).
const USR_BIT: u32 = 1 << 24;
/// `SPI_UPDATE` config-sync bit in `SPI_CMD` (bitpos 23) — self-clearing;
/// ESP-IDF's `spi_master` writes it and polls for it to clear before launch.
const UPDATE_BIT: u32 = 1 << 23;
/// `SPI_TRANS_DONE_INT` bit in the `SPI_DMA_INT_*` block (bitpos 12).
const TRANS_DONE: u32 = 1 << 12;
/// All 21 architected interrupt bits in the `SPI_DMA_INT_*` block.
const INT_MASK: u32 = 0x001F_FFFF;
/// `SPI_MS_DATA_BITLEN` mask in `SPI_MS_DLEN` (bits[17:0]).
const MS_DATA_BITLEN: u32 = 0x0003_FFFF;
/// `SPI_DMA_AFIFO_RST`/`SPI_BUF_AFIFO_RST`/`SPI_RX_AFIFO_RST` in `DMA_CONF`
/// (bits 31/30/29) — self-clearing FIFO reset strobes.
const AFIFO_RST_BITS: u32 = 0xE000_0000;
/// `SPI_DMA_TX_ENA` in `DMA_CONF` — "Set this bit to enable SPI DMA
/// controlled send data mode." Verified against ESP-IDF
/// `soc/esp32s3/register/soc/spi_reg.h`: `SPI_DMA_TX_ENA : R/W ;bitpos:[28]`.
const SPI_DMA_TX_ENA: u32 = 1 << 28;
/// `SPI_DMA_RX_ENA` in `DMA_CONF` — "Set this bit to enable SPI DMA
/// controlled receive data mode." Verified against ESP-IDF
/// `soc/esp32s3/register/soc/spi_reg.h`: `SPI_DMA_RX_ENA : R/W ;bitpos:[27]`.
/// (Bits 18..21 are the slave seg-trans fields, NOT the DMA enables.)
const SPI_DMA_RX_ENA: u32 = 1 << 27;
/// `SPI_SOFT_RESET` in `SLAVE` (bitpos 27, WT) — self-clearing.
const SOFT_RESET_BIT: u32 = 1 << 27;

/// One word past the last architected register (`DATE` @ 0xF0).
const NWORDS: usize = 0xF4 / 4;

/// `(reset value, writable-bit mask)` for the architected register at word
/// index `word` (offset `word * 4`), exactly per the ESP32-S3 SVD `SPI2`
/// block; `None` = hole in the register map (reads 0, ignores writes).
const fn spec(word: usize) -> Option<(u32, u32)> {
    match (word as u64) * 4 {
        CMD => Some((0x0000_0000, 0x0183_FFFF)),
        ADDR => Some((0x0000_0000, 0xFFFF_FFFF)),
        CTRL => Some((0x003C_0000, 0x07BD_C7E8)),
        CLOCK => Some((0x8000_3043, 0x803F_FFFF)),
        USER => Some((0x8000_00C0, 0xFF02_F3F9)),
        USER1 => Some((0xB841_0007, 0xFFFF_00FF)),
        USER2 => Some((0x7800_0000, 0xF800_FFFF)),
        MS_DLEN => Some((0x0000_0000, MS_DATA_BITLEN)),
        MISC => Some((0x0000_003E, 0xE18F_1FFF)),
        DIN_MODE => Some((0x0000_0000, 0x0001_FFFF)),
        DIN_NUM => Some((0x0000_0000, 0x0000_FFFF)),
        DOUT_MODE => Some((0x0000_0000, 0x0000_01FF)),
        DMA_CONF => Some((0x0000_0003, 0xF83C_0000)),
        DMA_INT_ENA => Some((0x0000_0000, INT_MASK)),
        DMA_INT_CLR => Some((0x0000_0000, INT_MASK)), // WO, W1C
        DMA_INT_RAW => Some((0x0000_0000, INT_MASK)), // R/WTC
        DMA_INT_ST => Some((0x0000_0000, 0x0000_0000)), // RO
        DMA_INT_SET => Some((0x0000_0000, INT_MASK)), // WO, W1S
        W0..=W15 => Some((0x0000_0000, 0xFFFF_FFFF)),
        SLAVE => Some((0x0280_0000, 0x1FC0_0F0F)),
        SLAVE1 => Some((0x0000_0000, 0xFFFF_FFFF)),
        CLK_GATE => Some((0x0000_0000, 0x0000_0007)),
        DATE => Some((0x0210_1190, 0x0FFF_FFFF)),
        _ => None,
    }
}

/// Pending DMA transaction state, set when `SPI_CMD.USR` is written with
/// `SPI_DMA_TX_ENA` or `SPI_DMA_RX_ENA` set in `DMA_CONF`. GDMA's coupled
/// pump peeks this each tick to know how many wire bytes remain, exchanges
/// up to a per-tick budget via [`Esp32s3Spi::dma_transfer`], and calls
/// [`Esp32s3Spi::dma_complete`] once `transferred` reaches `total_bytes`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SpiDmaPending {
    /// Total wire bytes in the transaction (`SPI_MS_DLEN` bits, rounded up
    /// to whole bytes — NOT capped at the 64-byte W buffer: that cap is a
    /// CPU-path artefact and DMA exists precisely to exceed it).
    pub(crate) total_bytes: usize,
    /// Wire bytes already exchanged (advanced by `dma_transfer`).
    pub(crate) transferred: usize,
    /// `SPI_DMA_TX_ENA` was set: GDMA's OUT chain supplies MOSI bytes.
    pub(crate) tx_ena: bool,
    /// `SPI_DMA_RX_ENA` was set: GDMA's IN chain receives MISO bytes.
    pub(crate) rx_ena: bool,
}

pub struct Esp32s3Spi {
    /// Interrupt-matrix source ID (SPI2=21, SPI3=22).
    source_id: u32,
    /// Register file for the architected map (word-indexed; holes stay 0 and
    /// are never read back — `spec()` gates both directions).
    regs: [u32; NWORDS],
    /// Latched raw interrupt bits (`SPI_DMA_INT_RAW`); W1C via INT_CLR /
    /// write-1-to-clear on INT_RAW itself; W1S via INT_SET.
    int_raw: u32,
    /// Set by `launch_transaction` when DMA mode is active; cleared by
    /// `dma_complete`. GDMA's SPI pump peeks this each tick.
    pending_dma: Option<SpiDmaPending>,
    /// Devices on this controller's bus (same model as `Esp32Spi` /
    /// the shared `Spi`): transfers broadcast to every device, first
    /// non-zero MISO byte wins (single-device labs in practice).
    pub(crate) attached_devices: Vec<Box<dyn SpiDevice>>,

    /// Bus-published cycle clock (walk-free level export).
    clock: Option<CycleClock>,
    /// Narration state for the SCK/MOSI/CS pads the output matrix can route this
    /// controller to. Inert until `SystemBus::wire_esp32s3_spi_pads` binds it;
    /// see [`crate::peripherals::esp_gpspi_wire`], shared with the C3's copy of
    /// this same IP.
    wire: EspSpiWire,
    external_can_poll_scheduled: bool,
}

impl std::fmt::Debug for Esp32s3Spi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32s3Spi(src={}, int_raw=0x{:08x}, int_ena=0x{:08x}, pending_dma={:?}, attached={})",
            self.source_id,
            self.int_raw,
            self.reg(DMA_INT_ENA),
            self.pending_dma,
            self.attached_devices.len(),
        )
    }
}

impl Esp32s3Spi {
    /// A GP-SPI controller instance. `source_id` is the intr-matrix source
    /// (`ETS_SPI2_INTR_SOURCE` = 21 for SPI2/FSPI, `ETS_SPI3_INTR_SOURCE` = 22
    /// for SPI3).
    pub fn new(source_id: u32) -> Self {
        let mut regs = [0u32; NWORDS];
        let mut w = 0;
        while w < NWORDS {
            if let Some((reset, _)) = spec(w) {
                regs[w] = reset;
            }
            w += 1;
        }
        Self {
            source_id,
            regs,
            int_raw: 0,
            pending_dma: None,
            attached_devices: Vec::new(),

            clock: None,
            wire: EspSpiWire::default(),
            external_can_poll_scheduled: false,
        }
    }

    /// The shared pad-line cell for this controller's SCK/MOSI/CS, created on
    /// first use at bus wiring time.
    ///
    /// ⚠️ Creating the cell turns narration ON. `SystemBus::wire_esp32s3_spi_pads`
    /// resolves the GPIO port FIRST for that reason: a controller owning a cell
    /// no route reaches still buffers every launched transaction, arms a wakeup
    /// per burst, and narrates into a wire nothing reads.
    pub(crate) fn pad_lines_arc(
        &mut self,
    ) -> std::sync::Arc<crate::peripherals::pad_lines::PadLines> {
        let cpol = crate::peripherals::esp_gpspi_wire::framing(self.reg(MISC), self.reg(USER)).cpol;
        self.wire.pad_lines_arc(cpol)
    }

    /// Engine cycles per SCK period, from this controller's own `SPI_CLOCK`.
    fn bit_time_cycles(&self) -> Option<u64> {
        crate::peripherals::esp_gpspi_wire::bit_time_cycles(self.reg(CLOCK), CPU_CLOCK_HZ)
    }

    /// Queue the bytes a transaction put on the wire, with the framing and rate
    /// its registers were configured with.
    fn wire_push_bytes(&mut self, bytes: &[u8]) {
        if !self.wire.is_bound() {
            return;
        }
        let framing = crate::peripherals::esp_gpspi_wire::framing(self.reg(MISC), self.reg(USER));
        let bit_time = self.bit_time_cycles();
        let now = self.clock.as_ref().map_or(0, |c| c.now());
        for &b in bytes {
            self.wire.push(b, framing, bit_time, now);
        }
    }

    /// Publish a held burst once the wire has carried it. The cycle axis comes
    /// from the bus clock; with none attached (a hand-built test bus) there is
    /// no axis to place a waveform on and nothing is published.
    fn wire_flush(&mut self, force: bool) {
        let Some(now) = self.clock.as_ref().map(|c| c.now()) else {
            return;
        };
        self.wire.flush(now, force);
    }

    /// Raw device push — does NOT wrap for tracing. The only production caller
    /// is the bus choke point [`crate::bus::SystemBus::attach_spi_device`],
    /// which wraps first. Reachable from config-level external-device wiring via
    /// `attach_esp32_external_devices` in `system/xtensa.rs`, which downcasts to
    /// either SPI model.
    pub(crate) fn push_device(&mut self, device: Box<dyn SpiDevice>) {
        self.attached_devices.push(device);
    }

    pub(crate) fn attached_devices_mut(&mut self) -> &mut [Box<dyn SpiDevice>] {
        &mut self.attached_devices
    }

    /// Number of attached devices (used by the system-wiring tests; test-only,
    /// so it is compiled out of non-test builds).
    #[cfg(test)]
    pub(crate) fn attached_device_count(&self) -> usize {
        self.attached_devices.len()
    }

    fn reg(&self, off: u64) -> u32 {
        let w = (off / 4) as usize;
        if w < NWORDS && spec(w).is_some() {
            self.regs[w]
        } else {
            0
        }
    }

    /// Masked store into an architected register; no-op on holes.
    fn set_reg_masked(&mut self, off: u64, value: u32) {
        let w = (off / 4) as usize;
        if w < NWORDS {
            if let Some((_, wmask)) = spec(w) {
                self.regs[w] = (self.regs[w] & !wmask) | (value & wmask);
            }
        }
    }

    /// Raw store (full word) — used by the transaction engine for the W
    /// buffer and CMD bookkeeping where the mask has already been applied.
    fn set_reg_raw(&mut self, off: u64, value: u32) {
        let w = (off / 4) as usize;
        if w < NWORDS {
            self.regs[w] = value;
        }
    }

    /// INT_ST = INT_RAW & INT_ENA.
    fn int_st(&self) -> u32 {
        self.int_raw & self.reg(DMA_INT_ENA)
    }

    /// Number of MISO bytes the transaction reads, from `SPI_MS_DLEN`
    /// (`SPI_MS_DATA_BITLEN` is bits-1). Capped at the 64-byte W buffer.
    fn miso_bytes(&self) -> usize {
        let bits = (self.reg(MS_DLEN) & MS_DATA_BITLEN) as usize + 1;
        (bits.div_ceil(8)).min(64)
    }

    fn exchange_byte(&mut self, mosi: u8) -> u8 {
        if self.attached_devices.is_empty() {
            return 0xFF;
        }
        let mut winner = 0u8;
        for device in &mut self.attached_devices {
            let response = device.transfer(mosi);
            if winner == 0 {
                winner = response;
            }
        }
        winner
    }

    /// Launch the user transaction.
    ///
    /// In DMA mode (`SPI_DMA_TX_ENA` or `SPI_DMA_RX_ENA` set in `DMA_CONF`):
    /// defer byte movement to GDMA. Keep `USR` set (in-progress) and record
    /// the pending transaction; GDMA's SPI pump exchanges bytes via
    /// `dma_transfer` and calls `dma_complete` when the wire count is done.
    ///
    /// In non-DMA (W-buffer) mode: fill the MISO region of W0..W15 with 0xFF
    /// per byte (idle pulled-high MISO), clear the `USR` start bit so the
    /// firmware's completion poll exits, and latch `SPI_TRANS_DONE`.
    fn launch_transaction(&mut self) {
        let dma_conf = self.reg(DMA_CONF);
        let tx_ena = dma_conf & SPI_DMA_TX_ENA != 0;
        let rx_ena = dma_conf & SPI_DMA_RX_ENA != 0;
        if tx_ena || rx_ena {
            // DMA mode: defer byte movement to GDMA. Keep USR set
            // (in-progress) so the firmware's completion poll spins until
            // GDMA finishes the descriptor walk — exactly the silicon
            // ordering the ESP-IDF spi_master ISR depends on. The W buffer
            // is NOT touched in this mode (it is the CPU data path).
            let bits = (self.reg(MS_DLEN) & MS_DATA_BITLEN) as usize + 1;
            self.pending_dma = Some(SpiDmaPending {
                total_bytes: bits.div_ceil(8),
                transferred: 0,
                tx_ena,
                rx_ena,
            });
            return;
        }
        // Non-DMA W-buffer mode is full-duplex: send MOSI to attached devices
        // and replace the transmitted bytes with their MISO responses.
        let bytes = self.miso_bytes();
        // Narrate the MOSI payload FIRST. It is still sitting in W0..W15 at this
        // point and the exchange below overwrites it with MISO, so
        // reading it after would report a bus that only ever carried ones.
        if self.wire.is_bound() {
            let mosi: Vec<u8> = (0..bytes)
                .map(|i| {
                    let off = W0 + (i as u64 / 4) * 4;
                    ((self.reg(off) >> ((i % 4) * 8)) & 0xFF) as u8
                })
                .collect();
            self.wire_push_bytes(&mosi);
        }
        for word_index in 0..bytes.div_ceil(4) {
            let off = W0 + (word_index as u64) * 4;
            let mosi_word = self.reg(off);
            let mut miso_word = 0u32;
            for byte_index in 0..4 {
                let absolute = word_index * 4 + byte_index;
                if absolute >= bytes {
                    break;
                }
                let shift = byte_index * 8;
                let mosi = ((mosi_word >> shift) & 0xFF) as u8;
                miso_word |= (self.exchange_byte(mosi) as u32) << shift;
            }
            self.set_reg_raw(off, miso_word);
        }
        // Clear the USR start bit (auto-clear on done) so `!(CMD & USR)` exits.
        let cmd = self.reg(CMD) & !USR_BIT;
        self.set_reg_raw(CMD, cmd);
        // Latch transaction-done.
        self.int_raw |= TRANS_DONE;
    }

    /// Peek the pending DMA transaction, if any (GDMA's pump polls this).
    pub(crate) fn dma_pending(&self) -> Option<SpiDmaPending> {
        self.pending_dma
    }

    /// Exchange one burst of wire bytes for the in-flight DMA transaction:
    /// each MOSI byte is broadcast to the attached devices and the FIRST
    /// non-zero MISO byte wins, per the `SpiDevice` v1 trait contract
    /// (see the trait doc in `peripherals/spi.rs`). Note: the shared STM32
    /// `Spi` transfer engine diverges from that doc and keeps the LAST
    /// non-zero response; the two are indistinguishable for the
    /// single-device labs v1 targets, and neither behaviour is changed.
    /// With no device attached the MISO line floats high, so every byte
    /// reads 0xFF. Advances `pending_dma.transferred` by the burst length.
    pub(crate) fn dma_transfer(&mut self, mosi: &[u8]) -> Vec<u8> {
        // GDMA-coupled transfers are the path esp_lcd drives a display over, so
        // this is where most real S3 SPI traffic reaches the wire.
        self.wire_push_bytes(mosi);
        let mut miso = Vec::with_capacity(mosi.len());
        for &m in mosi {
            let byte = if self.attached_devices.is_empty() {
                0xFF
            } else {
                let mut winner = 0u8;
                for dev in &mut self.attached_devices {
                    let resp = dev.transfer(m);
                    if winner == 0 {
                        winner = resp;
                    }
                }
                winner
            };
            miso.push(byte);
        }
        if let Some(p) = &mut self.pending_dma {
            p.transferred += mosi.len();
        }
        miso
    }

    /// Complete a DMA-mode transaction: clear the `USR` bit (firmware's
    /// completion poll exits) and latch `SPI_TRANS_DONE`. The W buffer is
    /// deliberately untouched — DMA-mode data lives in the descriptor
    /// buffers, not the CPU-path W0..W15 window.
    pub(crate) fn dma_complete(&mut self) {
        let cmd = self.reg(CMD) & !USR_BIT;
        self.set_reg_raw(CMD, cmd);
        self.int_raw |= TRANS_DONE;
        self.pending_dma = None;
    }
}

impl Peripheral for Esp32s3Spi {
    fn line_names(&self) -> &'static [&'static str] {
        crate::peripherals::esp_gpspi_wire::SPI_LINES
    }

    fn wire_lines(&self) -> Option<&crate::peripherals::pad_lines::PadLines> {
        self.wire.wire_lines()
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !3)?;
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset & !3 {
            DMA_INT_RAW => self.int_raw,
            DMA_INT_ST => self.int_st(),
            // Write-only registers read as zero.
            DMA_INT_CLR | DMA_INT_SET => 0,
            o => self.reg(o),
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let shift = (offset & 3) * 8;
        // Read-modify-write the affected word, then re-dispatch through the
        // u32 path so launch/W1C side-effects fire consistently.
        let base = match word_off {
            DMA_INT_RAW => self.int_raw,
            _ => self.reg(word_off),
        };
        let merged = (base & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_u32(word_off, merged)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset & !3 {
            // WO, W1C: clear latched raw bits where INT_CLR has a 1.
            DMA_INT_CLR => {
                self.int_raw &= !(value & INT_MASK);
            }
            // R/WTC: writing 1s to INT_RAW clears those latched bits.
            DMA_INT_RAW => {
                self.int_raw &= !(value & INT_MASK);
            }
            // WO, W1S: software-set raw bits (self-test / software interrupts).
            DMA_INT_SET => {
                self.int_raw |= value & INT_MASK;
            }
            // INT_ST is read-only; ignore writes.
            DMA_INT_ST => {}
            CMD => {
                self.set_reg_masked(CMD, value);
                // SPI_UPDATE (bit 23) is a self-clearing config-sync strobe:
                // ESP-IDF's spi_master writes it and polls for it to clear.
                let cmd = self.reg(CMD) & !UPDATE_BIT;
                self.set_reg_raw(CMD, cmd);
                if value & USR_BIT != 0 {
                    self.launch_transaction();
                }
            }
            DMA_CONF => {
                self.set_reg_masked(DMA_CONF, value);
                // AFIFO reset strobes (bits 29/30/31) self-clear.
                let v = self.reg(DMA_CONF) & !AFIFO_RST_BITS;
                self.set_reg_raw(DMA_CONF, v);
            }
            SLAVE => {
                self.set_reg_masked(SLAVE, value);
                // SPI_SOFT_RESET (bit 27, WT) self-clears.
                let v = self.reg(SLAVE) & !SOFT_RESET_BIT;
                self.set_reg_raw(SLAVE, v);
                if value & SOFT_RESET_BIT != 0 {
                    // SOFT_RESET aborts any in-flight DMA transaction —
                    // firmware timeout-recovery resets the block and re-kicks;
                    // a stale `pending_dma` would resurrect the dead
                    // transaction on the next GDMA tick. Also drop USR so the
                    // launch state machine returns to idle. No TRANS_DONE:
                    // the transaction was aborted, not completed.
                    self.pending_dma = None;
                    let cmd = self.reg(CMD) & !USR_BIT;
                    self.set_reg_raw(CMD, cmd);
                }
            }
            // Everything else: masked store into the architected register;
            // holes ignore writes entirely.
            o => self.set_reg_masked(o, value),
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Legacy-walk publication point. Under `event-scheduler` the walk skips
        // this model and the chain in `take_scheduled_events` owns it; without
        // the feature this is where a held burst reaches the pads. Inert — one
        // `is_empty` — on every bus that never routed an SPI pad.
        self.wire_flush(false);
        for device in &mut self.attached_devices {
            device.poll_external_bus();
        }
        PeripheralTickResult {
            explicit_irqs: if self.int_st() != 0 {
                Some(vec![self.source_id])
            } else {
                None
            },
            ..PeripheralTickResult::default()
        }
    }

    /// Walk-free: once the bus attaches a cycle clock under `event-scheduler`,
    /// level IRQs export via [`Self::matrix_irq_sources_into`] and the per-cycle
    /// walk is unnecessary (work settles on MMIO writes / `tick_with_bus`).
    fn uses_scheduler(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.uses_scheduler()
    }

    /// Publish a held burst under the scheduler, where the walk skips this model.
    ///
    /// # Why an event chain and not `needs_legacy_walk() -> true`
    ///
    /// `SystemBus::derive_walk_deletable` is all-or-nothing: one model forcing
    /// the walk back on drops walk-deletion for the WHOLE bus, including every
    /// S3 lab that never routes an SPI pad. This chain costs wakeups only while
    /// a burst is genuinely held — which needs BOTH routed pads and a launched
    /// transaction — and it arms at the exact cycle the wire finishes
    /// (`ready_in`), so a 64-byte burst costs ONE wakeup rather than 150 000.
    /// `uses_scheduler`/`needs_legacy_walk` are deliberately UNCHANGED above.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        const EXTERNAL_CAN_POLL_EVENT: u32 = u32::MAX;
        if self.clock.is_none() {
            return Vec::new();
        }
        let mut events = Vec::new();
        if self.wire.is_pending() && !self.wire.scheduled {
            let now = self.clock.as_ref().map_or(0, |c| c.now());
            self.wire.arm_seq = self.wire.arm_seq.wrapping_add(1);
            if self.wire.arm_seq == EXTERNAL_CAN_POLL_EVENT {
                self.wire.arm_seq = 0;
            }
            self.wire.scheduled = true;
            events.push((self.wire.ready_in(now), self.wire.arm_seq));
        }
        let has_external_can = self
            .attached_devices
            .iter()
            .any(|device| device.needs_external_bus_poll());
        if has_external_can && !self.external_can_poll_scheduled {
            self.external_can_poll_scheduled = true;
            events.push((0, EXTERNAL_CAN_POLL_EVENT));
        }
        events
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        const EXTERNAL_CAN_POLL_EVENT: u32 = u32::MAX;
        if event_token == EXTERNAL_CAN_POLL_EVENT {
            for device in &mut self.attached_devices {
                device.poll_external_bus();
            }
            return crate::sched::EventResult {
                reschedule_delay: Some(1),
                ..Default::default()
            };
        }
        if event_token != self.wire.arm_seq {
            // Stale token from a superseded arm; the live chain owns publication.
            return crate::sched::EventResult::default();
        }
        self.wire_flush(false);
        // A flush that reported LevelsOnly keeps its bytes; `ready_in` is then 0,
        // so `max(1)` retries next cycle and converges as the run grows.
        let now = self.clock.as_ref().map_or(0, |c| c.now());
        let pending = self.wire.is_pending();
        self.wire.scheduled = pending;
        crate::sched::EventResult {
            reschedule_delay: pending.then(|| self.wire.ready_in(now).max(1)),
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        if self.int_st() != 0 {
            out.push(self.source_id);
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for dev in self.attached_devices.iter_mut() {
            if let Some(si) = dev.as_sim_input_mut() {
                if f(si) {
                    return true;
                }
            }
        }
        false
    }

    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        for dev in &self.attached_devices {
            crate::inspect::visit_spi_device(&**dev, f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct ExternalCanPoller(Arc<AtomicUsize>);
    impl SpiDevice for ExternalCanPoller {
        fn needs_external_bus_poll(&self) -> bool {
            true
        }
        fn component_id(&self) -> Option<&str> {
            Some("external-can")
        }
        fn poll_external_bus(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        fn transfer(&mut self, _mosi: u8) -> u8 {
            0
        }
        fn cs_pin(&self) -> &str {
            "GPIO10"
        }
    }

    #[test]
    fn scheduler_arms_recurring_external_can_poll() {
        let polls = Arc::new(AtomicUsize::new(0));
        let mut spi = Esp32s3Spi::new(SPI2_SOURCE);
        spi.push_device(Box::new(ExternalCanPoller(polls.clone())));
        spi.attach_cycle_clock(crate::CycleClock::default());

        let events = spi.take_scheduled_events();
        assert_eq!(
            events.len(),
            1,
            "scheduler mode must arm external CAN polling"
        );
        let mut scheduler = crate::sched::EventScheduler::new();
        let mut bus = crate::bus::SystemBus::new();
        let result = spi.on_event(events[0].1, &mut scheduler, &mut bus);
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        assert_eq!(result.reschedule_delay, Some(1));

        let legacy_polls = Arc::new(AtomicUsize::new(0));
        let mut legacy = Esp32s3Spi::new(SPI2_SOURCE);
        legacy.push_device(Box::new(ExternalCanPoller(legacy_polls.clone())));
        assert!(legacy.take_scheduled_events().is_empty());
        legacy.tick();
        assert_eq!(legacy_polls.load(Ordering::SeqCst), 1);
    }

    const SPI2_SOURCE: u32 = 21;
    const SPI3_SOURCE: u32 = 22;

    #[test]
    fn config_registers_store_under_write_mask() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        s.write_u32(USER, 0x9000_00C0).unwrap();
        s.write_u32(USER1, 0x0000_5817).unwrap();
        s.write_u32(USER2, 0x7000_0006).unwrap();
        s.write_u32(CLOCK, 0x0000_1001).unwrap();
        s.write_u32(CTRL, 0x003C_0000).unwrap();
        s.write_u32(MISC, 0x0000_003E).unwrap();
        s.write_u32(MS_DLEN, 0x0000_001F).unwrap();
        assert_eq!(s.read_u32(USER).unwrap(), 0x9000_00C0);
        // USER1 bits 8..15 are reserved (not in the SVD write mask): the 0x58
        // byte is dropped and the reserved bits keep their reset value (0).
        assert_eq!(s.read_u32(USER1).unwrap(), 0x0000_0017);
        assert_eq!(s.read_u32(USER2).unwrap(), 0x7000_0006);
        assert_eq!(s.read_u32(CLOCK).unwrap(), 0x0000_1001);
        assert_eq!(s.read_u32(CTRL).unwrap(), 0x003C_0000);
        assert_eq!(s.read_u32(MISC).unwrap(), 0x0000_003E);
        assert_eq!(s.read_u32(MS_DLEN).unwrap(), 0x0000_001F);
        // MS_DLEN is 18 bits wide: upper bits never store.
        s.write_u32(MS_DLEN, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(MS_DLEN).unwrap(), MS_DATA_BITLEN);
    }

    #[test]
    fn reset_defaults_seeded() {
        let s = Esp32s3Spi::new(SPI2_SOURCE);
        assert_eq!(s.read_u32(CLOCK).unwrap(), 0x8000_3043, "CLOCK");
        assert_eq!(s.read_u32(CTRL).unwrap(), 0x003C_0000, "CTRL");
        assert_eq!(s.read_u32(USER).unwrap(), 0x8000_00C0, "USER");
        assert_eq!(s.read_u32(USER1).unwrap(), 0xB841_0007, "USER1");
        assert_eq!(s.read_u32(USER2).unwrap(), 0x7800_0000, "USER2");
        assert_eq!(s.read_u32(MISC).unwrap(), 0x0000_003E, "MISC");
        assert_eq!(s.read_u32(DMA_CONF).unwrap(), 0x0000_0003, "DMA_CONF");
        assert_eq!(s.read_u32(SLAVE).unwrap(), 0x0280_0000, "SLAVE");
        assert_eq!(s.read_u32(DATE).unwrap(), 0x0210_1190, "DATE");
    }

    #[test]
    fn unmapped_offsets_read_zero_and_ignore_writes() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        // Holes inside the map and offsets above DATE must NOT round-trip —
        // the coverage probe's baseline depends on it.
        for off in [0x48u64, 0x90, 0xD8, 0xEC, 0xF4, 0xFFC] {
            s.write_u32(off, 0xDEAD_BEEF).unwrap();
            assert_eq!(s.read_u32(off).unwrap(), 0, "hole at {off:#x}");
        }
    }

    #[test]
    fn cmd_update_bit_self_clears() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        // ESP-IDF spi_master writes UPDATE then polls for it to clear; a
        // round-tripping CMD would wedge the driver.
        s.write_u32(CMD, UPDATE_BIT).unwrap();
        assert_eq!(
            s.read_u32(CMD).unwrap() & UPDATE_BIT,
            0,
            "UPDATE self-clears"
        );
    }

    #[test]
    fn dma_conf_afifo_rst_strobes_self_clear() {
        let mut s = Esp32s3Spi::new(SPI3_SOURCE);
        s.write_u32(DMA_CONF, AFIFO_RST_BITS).unwrap();
        assert_eq!(
            s.read_u32(DMA_CONF).unwrap() & AFIFO_RST_BITS,
            0,
            "AFIFO reset strobes self-clear"
        );
    }

    #[test]
    fn slave_soft_reset_self_clears() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        s.write_u32(SLAVE, SOFT_RESET_BIT).unwrap();
        assert_eq!(
            s.read_u32(SLAVE).unwrap() & SOFT_RESET_BIT,
            0,
            "SOFT_RESET (WT) self-clears"
        );
    }

    #[test]
    fn int_set_latches_and_int_raw_write_clears() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        // W1S via INT_SET.
        s.write_u32(DMA_INT_SET, TRANS_DONE).unwrap();
        assert_eq!(s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE, TRANS_DONE);
        // R/WTC: writing the set bit to INT_RAW clears it.
        s.write_u32(DMA_INT_RAW, TRANS_DONE).unwrap();
        assert_eq!(s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE, 0);
        // Write-only registers read as zero.
        assert_eq!(s.read_u32(DMA_INT_CLR).unwrap(), 0);
        assert_eq!(s.read_u32(DMA_INT_SET).unwrap(), 0);
    }

    #[test]
    fn w_buffer_round_trips() {
        let mut s = Esp32s3Spi::new(SPI3_SOURCE);
        for w in 0..16u64 {
            let off = W0 + w * 4;
            let val = 0xA000_0000 | (w as u32);
            s.write_u32(off, val).unwrap();
        }
        for w in 0..16u64 {
            let off = W0 + w * 4;
            assert_eq!(s.read_u32(off).unwrap(), 0xA000_0000 | (w as u32));
        }
        assert_eq!(W15, W0 + 15 * 4, "W15 offset");
    }

    #[test]
    fn w_buffer_byte_round_trip() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        // Byte-wise MOSI fill into W0.
        s.write(W0, 0xDE).unwrap();
        s.write(W0 + 1, 0xAD).unwrap();
        s.write(W0 + 2, 0xBE).unwrap();
        s.write(W0 + 3, 0xEF).unwrap();
        assert_eq!(s.read_u32(W0).unwrap(), 0xEFBE_ADDE);
        assert_eq!(s.read(W0).unwrap(), 0xDE);
        assert_eq!(s.read(W0 + 3).unwrap(), 0xEF);
    }

    #[test]
    fn usr_start_auto_clears_and_latches_trans_done() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        // 32-bit transfer.
        s.write_u32(MS_DLEN, 32 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        // USR auto-cleared so `!(CMD & USR)` poll exits.
        assert_eq!(s.read_u32(CMD).unwrap() & USR_BIT, 0, "USR auto-clears");
        // TRANS_DONE latched in INT_RAW.
        assert_eq!(
            s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE,
            TRANS_DONE,
            "TRANS_DONE latched"
        );
    }

    #[test]
    fn miso_fills_0xff_for_no_device() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        // Pre-load W0 with MOSI data; the launch overwrites the MISO region.
        s.write_u32(W0, 0x0000_0000).unwrap();
        s.write_u32(W0 + 4, 0x1234_5678).unwrap();
        // 5-byte (40-bit) transfer → W0 fully 0xFF, W1 low byte 0xFF only.
        s.write_u32(MS_DLEN, 40 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        assert_eq!(
            s.read_u32(W0).unwrap(),
            0xFFFF_FFFF,
            "first 4 MISO bytes 0xFF"
        );
        assert_eq!(
            s.read_u32(W0 + 4).unwrap(),
            0x0000_00FF,
            "5th byte 0xFF, rest untouched"
        );
    }

    #[test]
    fn cpu_w_buffer_transaction_streams_mosi_to_attached_device() {
        struct Spy(Arc<Mutex<Vec<u8>>>);
        impl SpiDevice for Spy {
            fn transfer(&mut self, mosi: u8) -> u8 {
                self.0.lock().unwrap().push(mosi);
                mosi ^ 0xA5
            }

            fn cs_pin(&self) -> &str {
                "GPIO10"
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        s.push_device(Box::new(Spy(seen.clone())));
        s.write_u32(W0, 0x0403_0201).unwrap();
        s.write_u32(MS_DLEN, 4 * 8 - 1).unwrap();

        s.write_u32(CMD, USR_BIT).unwrap();

        assert_eq!(&*seen.lock().unwrap(), &[1, 2, 3, 4]);
        assert_eq!(s.read_u32(W0).unwrap(), 0xA1A6_A7A4);
    }

    #[test]
    fn int_clr_is_w1c() {
        let mut s = Esp32s3Spi::new(SPI3_SOURCE);
        s.write_u32(MS_DLEN, 8 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        assert_eq!(s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE, TRANS_DONE);
        // W1C: writing the bit clears it; writing 0 leaves others intact.
        s.write_u32(DMA_INT_CLR, TRANS_DONE).unwrap();
        assert_eq!(
            s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE,
            0,
            "W1C cleared"
        );
    }

    #[test]
    fn int_st_masks_with_ena_and_emits_source() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        // No enable yet → no source emitted even after a transaction.
        s.write_u32(MS_DLEN, 8 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        assert_eq!(s.read_u32(DMA_INT_ST).unwrap(), 0, "ST gated by ENA");
        assert_eq!(s.tick().explicit_irqs, None, "no source while ST==0");
        // Enable TRANS_DONE → ST asserts and the source is emitted while ST!=0.
        s.write_u32(DMA_INT_ENA, TRANS_DONE).unwrap();
        assert_eq!(s.read_u32(DMA_INT_ST).unwrap() & TRANS_DONE, TRANS_DONE);
        assert_eq!(s.tick().explicit_irqs, Some(vec![SPI2_SOURCE]));
        // Clear the raw bit → ST drops, source stops.
        s.write_u32(DMA_INT_CLR, TRANS_DONE).unwrap();
        assert_eq!(s.tick().explicit_irqs, None);
    }

    #[test]
    fn int_st_is_read_only() {
        let mut s = Esp32s3Spi::new(SPI3_SOURCE);
        s.write_u32(DMA_INT_ST, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(DMA_INT_ST).unwrap(), 0, "INT_ST ignores writes");
    }

    // ── DMA-mode (GDMA-coupled) transaction tests ─────────────────────────

    #[test]
    fn dma_mode_usr_defers_completion_and_leaves_w_buffer_untouched() {
        let mut s = Esp32s3Spi::new(SPI3_SOURCE);
        s.write_u32(W0, 0x1234_5678).unwrap();
        // 200-byte transaction with both DMA directions enabled.
        s.write_u32(DMA_CONF, SPI_DMA_TX_ENA | SPI_DMA_RX_ENA)
            .unwrap();
        s.write_u32(MS_DLEN, 200 * 8 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        // USR stays set (in-progress) and TRANS_DONE is NOT latched yet.
        assert_ne!(
            s.read_u32(CMD).unwrap() & USR_BIT,
            0,
            "USR held in DMA mode"
        );
        assert_eq!(
            s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE,
            0,
            "TRANS_DONE deferred to GDMA"
        );
        // The pending record carries the FULL (uncapped) byte count.
        let p = s.dma_pending().expect("pending_dma set");
        assert_eq!(p.total_bytes, 200);
        assert_eq!(p.transferred, 0);
        assert!(p.tx_ena && p.rx_ena);
        // W buffer untouched — DMA mode bypasses the CPU data path.
        assert_eq!(s.read_u32(W0).unwrap(), 0x1234_5678, "W0 untouched");
    }

    #[test]
    fn dma_transfer_advances_and_dma_complete_finishes() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        s.write_u32(DMA_CONF, SPI_DMA_TX_ENA).unwrap();
        s.write_u32(MS_DLEN, 8 * 8 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        // No device attached → MISO floats high (0xFF per byte).
        let miso = s.dma_transfer(&[0xAA, 0x55, 0x00]);
        assert_eq!(miso, vec![0xFF, 0xFF, 0xFF]);
        assert_eq!(s.dma_pending().unwrap().transferred, 3);
        s.dma_complete();
        assert_eq!(s.read_u32(CMD).unwrap() & USR_BIT, 0, "USR cleared");
        assert_ne!(
            s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE,
            0,
            "TRANS_DONE latched"
        );
        assert!(s.dma_pending().is_none(), "pending cleared");
    }

    /// `SLAVE.SOFT_RESET` mid-DMA-transaction aborts the pending transfer:
    /// firmware timeout-recovery must not leave a stale `pending_dma` that
    /// the GDMA pump would resurrect after the reset.
    #[test]
    fn slave_soft_reset_aborts_pending_dma() {
        let mut s = Esp32s3Spi::new(SPI3_SOURCE);
        s.write_u32(DMA_CONF, SPI_DMA_TX_ENA | SPI_DMA_RX_ENA)
            .unwrap();
        s.write_u32(MS_DLEN, 32 * 8 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        assert!(s.dma_pending().is_some(), "DMA transaction pending");
        assert_ne!(s.read_u32(CMD).unwrap() & USR_BIT, 0, "USR held");

        // Firmware timeout-recovery: soft-reset the block.
        s.write_u32(SLAVE, SOFT_RESET_BIT).unwrap();

        assert!(s.dma_pending().is_none(), "SOFT_RESET aborts pending DMA");
        assert_eq!(s.read_u32(CMD).unwrap() & USR_BIT, 0, "USR cleared");
        assert_eq!(
            s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE,
            0,
            "aborted transaction must NOT latch TRANS_DONE"
        );
    }

    #[test]
    fn dma_transfer_routes_bytes_through_attached_device() {
        struct Xor(Vec<u8>);
        impl crate::peripherals::spi::SpiDevice for Xor {
            fn transfer(&mut self, mosi: u8) -> u8 {
                self.0.push(mosi);
                mosi ^ 0xA5
            }
            fn cs_pin(&self) -> &str {
                "GPIO10"
            }
        }
        let mut s = Esp32s3Spi::new(SPI3_SOURCE);
        s.push_device(Box::new(Xor(Vec::new())));
        s.write_u32(DMA_CONF, SPI_DMA_TX_ENA | SPI_DMA_RX_ENA)
            .unwrap();
        s.write_u32(MS_DLEN, 4 * 8 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        let miso = s.dma_transfer(&[0x01, 0x02, 0x03, 0x04]);
        assert_eq!(
            miso,
            vec![0x01 ^ 0xA5, 0x02 ^ 0xA5, 0x03 ^ 0xA5, 0x04 ^ 0xA5]
        );
    }
}
