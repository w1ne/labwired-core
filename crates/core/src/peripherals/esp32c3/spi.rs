// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 GP-SPI2 master controller (`SPI2` / FSPI, `0x6002_4000`) — the
//! general-purpose SPI master the firmware drives via the `spi_master` /
//! Arduino `SPI` drivers, DISTINCT from the flash controllers SPI0/SPI1.
//!
//! The ESP32-C3 GP-SPI is the SAME Espressif IP family as the ESP32-S3's SPI2
//! (see [`crate::peripherals::esp32s3::gpspi::Esp32s3Spi`]). The register map,
//! the `SPI_CMD.USR` launch handshake, the `SPI_MS_DLEN` bit-length field and
//! the `SPI_DMA_INT_*` block are byte-for-byte identical — verified against
//! `configs/peripherals/esp32c3/spi2.yaml` (SVD-sourced): `TRANS_DONE` is bit
//! 12, the 21 architected interrupt bits span [20:0], `SLAVE.SOFT_RESET` is bit
//! 27 and `DMA_CONF` AFIFO-reset strobes are bits 29/30/31 on both parts.
//!
//! ## C3-vs-S3 differences (all from the C3 `spi2.yaml`)
//!
//!   * Interrupt-matrix source: the C3 routes GP-SPI2 through interrupt-matrix
//!     source **19** (`SPI_INTR_2_MAP` lives at register offset 76 = `4 * 19`
//!     in `interrupt_core0.yaml`), NOT the S3's Xtensa ordinal 21.
//!   * `DMA_CONF` reset value is `0x0000_0000` (S3 resets it to `0x0000_0003`).
//!   * `DATE` reset value is `0x0200_7220` (S3 = `0x0210_1190`).
//!   * The C3 SVD omits the `SPI_DMA_INT_SET` register at 0x44 (W1S software-set
//!     of raw interrupt bits) that the S3 declares — so 0x44 is a hole here.
//!
//! ## Transaction model (CPU / W-buffer full-duplex)
//!
//! A master programs the transfer bit length (`SPI_MS_DLEN`) and the MOSI
//! payload into `W0..W15`, then sets `SPI_CMD.USR` (bit 24) to launch and
//! busy-polls `!(CMD & USR)` for completion. We run the transaction
//! IMMEDIATELY on the CPU data path: each MOSI byte read out of the `W0..W15`
//! window (little-endian within each word) is exchanged with the attached
//! [`SpiDevice`]s and the returned MISO byte is written back into the same
//! window (genuine full-duplex). With no device on the bus the MISO line floats
//! pulled-high, so every byte reads `0xFF` — exactly the all-ones a real
//! controller shifts in from an idle bus. Then `USR` auto-clears (so the
//! firmware's poll exits) and `SPI_TRANS_DONE` latches in `SPI_DMA_INT_RAW`.
//!
//! This is the CPU data path only; GDMA-coupled (descriptor-driven) transfers
//! are not modeled here.
//!
//! `tick()` emits the controller's intr-matrix source while `INT_ST != 0`,
//! mirroring the I²C / UART pattern; the bus routes it through the per-core
//! interrupt matrix.

use crate::peripherals::spi::SpiDevice;
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

pub const SPI2_BASE: u32 = 0x6002_4000;
pub const SPI2_SIZE: u64 = 0x1000;

/// ESP32-C3 GP-SPI2 interrupt-matrix source number.
///
/// On the C3 (RISC-V) the firmware programs `SPI_INTR_2_MAP` in the interrupt
/// matrix at offset `4 * source`; the C3 `interrupt_core0.yaml` places that
/// register at offset 76 = `4 * 19`, so the source index is 19 — NOT the S3's
/// 21 (which is the Xtensa `ets_isr_source_t` ordinal).
pub const SPI2_INTR_SOURCE_ID: u32 = 19;

const CMD: u64 = 0x00;
const CTRL: u64 = 0x08;
const CLOCK: u64 = 0x0C;
const USER: u64 = 0x10;
const USER1: u64 = 0x14;
const USER2: u64 = 0x18;
const MS_DLEN: u64 = 0x1C;
const MISC: u64 = 0x20;
const DMA_CONF: u64 = 0x30;
const DMA_INT_ENA: u64 = 0x34;
const DMA_INT_CLR: u64 = 0x38;
const DMA_INT_RAW: u64 = 0x3C;
const DMA_INT_ST: u64 = 0x40;
const W0: u64 = 0x98;
/// `SPI_W15` (0xD4) — last word of the 16-word data buffer.
const W15: u64 = 0xD4;
const SLAVE: u64 = 0xE0;
const CLK_GATE: u64 = 0xE8;
const DATE: u64 = 0xF0;

/// `SPI_USR` launch bit in `SPI_CMD` (bitpos 24).
const USR_BIT: u32 = 1 << 24;
/// `SPI_UPDATE` config-sync bit in `SPI_CMD` (bitpos 23) — self-clearing;
/// `spi_master` writes it and polls for it to clear before launch.
const UPDATE_BIT: u32 = 1 << 23;
/// `SPI_TRANS_DONE_INT` bit in the `SPI_DMA_INT_*` block (bitpos 12).
pub const TRANS_DONE: u32 = 1 << 12;
/// The 21 architected interrupt bits in the `SPI_DMA_INT_*` block ([20:0]).
const INT_MASK: u32 = 0x001F_FFFF;
/// `SPI_MS_DATA_BITLEN` mask in `SPI_MS_DLEN` (bits[17:0]).
const MS_DATA_BITLEN: u32 = 0x0003_FFFF;
/// `SPI_DMA_AFIFO_RST`/`SPI_BUF_AFIFO_RST`/`SPI_RX_AFIFO_RST` in `DMA_CONF`
/// (bits 31/30/29) — self-clearing FIFO reset strobes.
const AFIFO_RST_BITS: u32 = 0xE000_0000;
/// `SPI_SOFT_RESET` in `SLAVE` (bitpos 27, WT) — self-clearing.
const SOFT_RESET_BIT: u32 = 1 << 27;

/// One word past the last architected register (`DATE` @ 0xF0).
const NWORDS: usize = 0xF4 / 4;

/// `(reset value, writable-bit mask)` for the architected register at word
/// index `word` (offset `word * 4`), per the ESP32-C3 `spi2.yaml`; `None` = a
/// hole in the register map (reads 0, ignores writes). The writable masks match
/// the S3's same-IP `SPI2` block; the C3-specific reset values (DMA_CONF, DATE)
/// and the missing `DMA_INT_SET` register are the documented deltas.
const fn spec(word: usize) -> Option<(u32, u32)> {
    match (word as u64) * 4 {
        CMD => Some((0x0000_0000, 0x0183_FFFF)),
        0x04 => Some((0x0000_0000, 0xFFFF_FFFF)), // ADDR
        CTRL => Some((0x003C_0000, 0x07BD_C7E8)),
        CLOCK => Some((0x8000_3043, 0x803F_FFFF)),
        USER => Some((0x8000_00C0, 0xFF02_F3F9)),
        USER1 => Some((0xB841_0007, 0xFFFF_00FF)),
        USER2 => Some((0x7800_0000, 0xF800_FFFF)),
        MS_DLEN => Some((0x0000_0000, MS_DATA_BITLEN)),
        MISC => Some((0x0000_003E, 0xE18F_1FFF)),
        0x24 => Some((0x0000_0000, 0x0001_FFFF)), // DIN_MODE
        0x28 => Some((0x0000_0000, 0x0000_FFFF)), // DIN_NUM
        0x2C => Some((0x0000_0000, 0x0000_01FF)), // DOUT_MODE
        DMA_CONF => Some((0x0000_0000, 0xF83C_0000)),
        DMA_INT_ENA => Some((0x0000_0000, INT_MASK)),
        DMA_INT_CLR => Some((0x0000_0000, INT_MASK)), // WO, W1C
        DMA_INT_RAW => Some((0x0000_0000, INT_MASK)), // R/WTC
        DMA_INT_ST => Some((0x0000_0000, 0x0000_0000)), // RO
        W0..=W15 => Some((0x0000_0000, 0xFFFF_FFFF)),
        SLAVE => Some((0x0280_0000, 0x1FC0_0F0F)),
        0xE4 => Some((0x0000_0000, 0xFFFF_FFFF)), // SLAVE1
        CLK_GATE => Some((0x0000_0000, 0x0000_0007)),
        DATE => Some((0x0200_7220, 0x0FFF_FFFF)),
        _ => None,
    }
}

pub struct Esp32c3Spi {
    /// Interrupt-matrix source ID (GP-SPI2 = 19 on the C3).
    source_id: u32,
    /// Register file for the architected map (word-indexed; holes stay 0 and
    /// are never read back — `spec()` gates both directions).
    regs: [u32; NWORDS],
    /// Latched raw interrupt bits (`SPI_DMA_INT_RAW`); W1C via INT_CLR /
    /// write-1-to-clear on INT_RAW itself.
    int_raw: u32,
    /// Devices on this controller's bus: MOSI bytes broadcast to every device,
    /// first non-zero MISO byte wins (single-device labs in practice).
    pub(crate) attached_devices: Vec<Box<dyn SpiDevice>>,
    /// Bus-published cycle clock (walk-free plan). `Some` once
    /// `SystemBus::push_peripheral`/`add_peripheral` attaches it. Its presence
    /// (under the `event-scheduler` feature) flips the model onto the event
    /// scheduler: the per-cycle walk skips this peripheral and the bus derives
    /// its level-sensitive matrix IRQ from [`Self::matrix_irq_sources`] instead
    /// of the walk's `explicit_irqs`. This controller has NO free-running
    /// counter — `int_raw` is write-armed by the transaction launch — so there
    /// are no scheduled events: the migration is level-export only. `None`
    /// (feature off, a hand-built bus, or the differential's `force_legacy_walk`)
    /// keeps the legacy per-cycle walk. Not serialized — re-attached by the bus.
    clock: Option<CycleClock>,
}

impl std::fmt::Debug for Esp32c3Spi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32c3Spi(src={}, int_raw=0x{:08x}, int_ena=0x{:08x}, attached={})",
            self.source_id,
            self.int_raw,
            self.reg(DMA_INT_ENA),
            self.attached_devices.len(),
        )
    }
}

impl Esp32c3Spi {
    /// A GP-SPI2 controller instance asserting interrupt-matrix source
    /// `source_id` (19 for the C3 GP-SPI2).
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
            attached_devices: Vec::new(),
            clock: None,
        }
    }

    /// Test/differential knob: detach the cycle clock, pinning the model to the
    /// legacy per-cycle walk (`uses_scheduler() == false`). Used by the
    /// walk-on-vs-scheduler differential gate to build the reference config from
    /// the same bus assembly (mirrors `Esp32c3I2c::force_legacy_walk`).
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    /// Raw device push — does NOT wrap for tracing. The only production caller
    /// is the bus choke point [`crate::bus::SystemBus::attach_spi_device`],
    /// which wraps first.
    pub(crate) fn push_device(&mut self, device: Box<dyn SpiDevice>) {
        self.attached_devices.push(device);
    }

    pub fn attached_devices(&self) -> &[Box<dyn SpiDevice>] {
        &self.attached_devices
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

    /// Raw store (full word) — used by the transaction engine for the W buffer
    /// and CMD bookkeeping where the mask has already been applied.
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

    /// Number of bytes in the transfer, from `SPI_MS_DLEN`
    /// (`SPI_MS_DATA_BITLEN` is bits-1). Capped at the 64-byte W buffer.
    fn transfer_bytes(&self) -> usize {
        let bits = (self.reg(MS_DLEN) & MS_DATA_BITLEN) as usize + 1;
        (bits.div_ceil(8)).min(64)
    }

    /// Exchange one MOSI byte with every attached device; the FIRST non-zero
    /// MISO response wins (per the `SpiDevice` v1 contract). With no device the
    /// line floats high → 0xFF.
    fn exchange_byte(&mut self, mosi: u8) -> u8 {
        if self.attached_devices.is_empty() {
            return 0xFF;
        }
        let mut winner = 0u8;
        for dev in &mut self.attached_devices {
            let resp = dev.transfer(mosi);
            if winner == 0 {
                winner = resp;
            }
        }
        winner
    }

    /// Launch the user transaction on the CPU (W-buffer) data path: shift each
    /// MOSI byte out of `W0..W15` (little-endian within each word) through the
    /// attached devices, write the MISO byte back into the same window, then
    /// clear `USR` and latch `SPI_TRANS_DONE`.
    fn launch_transaction(&mut self) {
        let bytes = self.transfer_bytes();
        for i in 0..bytes {
            let off = W0 + (i as u64 / 4) * 4;
            let shift = (i % 4) * 8;
            let mosi = ((self.reg(off) >> shift) & 0xFF) as u8;
            let miso = self.exchange_byte(mosi);
            let word = (self.reg(off) & !(0xFFu32 << shift)) | ((miso as u32) << shift);
            self.set_reg_raw(off, word);
        }
        // Clear the USR start bit (auto-clear on done) so `!(CMD & USR)` exits.
        let cmd = self.reg(CMD) & !USR_BIT;
        self.set_reg_raw(CMD, cmd);
        // Latch transaction-done.
        self.int_raw |= TRANS_DONE;
    }
}

impl Peripheral for Esp32c3Spi {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !3)?;
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset & !3 {
            DMA_INT_RAW => self.int_raw,
            DMA_INT_ST => self.int_st(),
            // Write-only register reads as zero.
            DMA_INT_CLR => 0,
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
            // INT_ST is read-only; ignore writes.
            DMA_INT_ST => {}
            CMD => {
                self.set_reg_masked(CMD, value);
                // SPI_UPDATE (bit 23) is a self-clearing config-sync strobe.
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
                    // Soft-reset returns the launch state machine to idle.
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

    /// LEGACY per-cycle walk path: re-assert the level interrupt source while
    /// any enabled INT bit is set. In scheduler mode ([`Self::uses_scheduler`]
    /// true) the walk skips this peripheral entirely and the bus re-derives the
    /// level from [`Self::matrix_irq_sources`] instead; this reporter is a pure
    /// no-op on state, so a stray call is harmless.
    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult {
            explicit_irqs: if self.int_st() != 0 {
                Some(vec![self.source_id])
            } else {
                None
            },
            ..PeripheralTickResult::default()
        }
    }

    fn legacy_tick_active(&self) -> bool {
        self.int_st() != 0
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    /// Walk-free plan: driven by the event scheduler once the bus has attached
    /// its cycle clock (production `push_peripheral`/`add_peripheral` always do,
    /// under the `event-scheduler` feature). The per-cycle walk then skips this
    /// peripheral; its `int_raw` is write-armed by the transaction launch (no
    /// free-running counter), so there is nothing to advance and no event to
    /// schedule — only the level export via `matrix_irq_sources` is needed.
    /// Without a clock (feature off, a hand-built bus, or `force_legacy_walk`)
    /// it stays on the legacy walk so those callers keep the old exact
    /// semantics.
    fn uses_scheduler(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    /// C3 interrupt-matrix level: the GP-SPI2 source while any enabled INT bit
    /// is set — the exact condition `tick` pushes on the legacy walk. In
    /// scheduler mode the walk no longer re-emits it, so the bus re-derives the
    /// level from here (`refresh_esp32c3_sched_sources`, polled on the event
    /// path and the walk-tick aggregation) so the level-sensitive IRQ stays
    /// routed and de-asserts the tick after firmware writes INT_CLR.
    fn matrix_irq_sources(&self) -> Vec<u32> {
        if self.int_st() != 0 {
            vec![self.source_id]
        } else {
            Vec::new()
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

    #[test]
    fn spi2_interrupt_source_is_19_not_21() {
        // C3-vs-S3 difference: the C3 routes GP-SPI2 through interrupt-matrix
        // source 19 (SPI_INTR_2_MAP at offset 76 = 4*19), NOT the S3's 21.
        assert_eq!(SPI2_INTR_SOURCE_ID, 19);
    }

    #[test]
    fn reset_defaults_match_c3_yaml() {
        let s = Esp32c3Spi::new(SPI2_INTR_SOURCE_ID);
        assert_eq!(s.read_u32(CLOCK).unwrap(), 0x8000_3043, "CLOCK");
        assert_eq!(s.read_u32(CTRL).unwrap(), 0x003C_0000, "CTRL");
        assert_eq!(s.read_u32(USER).unwrap(), 0x8000_00C0, "USER");
        assert_eq!(s.read_u32(USER1).unwrap(), 0xB841_0007, "USER1");
        assert_eq!(s.read_u32(USER2).unwrap(), 0x7800_0000, "USER2");
        assert_eq!(s.read_u32(MISC).unwrap(), 0x0000_003E, "MISC");
        // C3-specific resets (differ from the S3).
        assert_eq!(s.read_u32(DMA_CONF).unwrap(), 0x0000_0000, "DMA_CONF (C3)");
        assert_eq!(s.read_u32(SLAVE).unwrap(), 0x0280_0000, "SLAVE");
        assert_eq!(s.read_u32(DATE).unwrap(), 0x0200_7220, "DATE (C3)");
    }

    #[test]
    fn config_registers_store_under_write_mask() {
        let mut s = Esp32c3Spi::new(SPI2_INTR_SOURCE_ID);
        // USER1 bits 8..15 are reserved: the 0x58 byte is dropped.
        s.write_u32(USER1, 0x0000_5817).unwrap();
        assert_eq!(s.read_u32(USER1).unwrap(), 0x0000_0017);
        // MS_DLEN is 18 bits wide: upper bits never store.
        s.write_u32(MS_DLEN, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(MS_DLEN).unwrap(), MS_DATA_BITLEN);
    }

    #[test]
    fn dma_int_set_offset_is_a_hole_on_c3() {
        // The C3 SVD omits SPI_DMA_INT_SET (0x44); it must NOT round-trip.
        let mut s = Esp32c3Spi::new(SPI2_INTR_SOURCE_ID);
        s.write_u32(0x44, 0xDEAD_BEEF).unwrap();
        assert_eq!(s.read_u32(0x44).unwrap(), 0, "0x44 is a hole on the C3");
    }

    #[test]
    fn unmapped_offsets_read_zero_and_ignore_writes() {
        let mut s = Esp32c3Spi::new(SPI2_INTR_SOURCE_ID);
        for off in [0x48u64, 0x90, 0xD8, 0xEC, 0xF4, 0xFFC] {
            s.write_u32(off, 0xDEAD_BEEF).unwrap();
            assert_eq!(s.read_u32(off).unwrap(), 0, "hole at {off:#x}");
        }
    }

    #[test]
    fn cmd_update_bit_self_clears() {
        let mut s = Esp32c3Spi::new(SPI2_INTR_SOURCE_ID);
        s.write_u32(CMD, UPDATE_BIT).unwrap();
        assert_eq!(
            s.read_u32(CMD).unwrap() & UPDATE_BIT,
            0,
            "UPDATE self-clears"
        );
    }

    #[test]
    fn dma_conf_afifo_rst_strobes_self_clear() {
        let mut s = Esp32c3Spi::new(SPI2_INTR_SOURCE_ID);
        s.write_u32(DMA_CONF, AFIFO_RST_BITS).unwrap();
        assert_eq!(
            s.read_u32(DMA_CONF).unwrap() & AFIFO_RST_BITS,
            0,
            "AFIFO reset strobes self-clear"
        );
    }

    #[test]
    fn slave_soft_reset_self_clears() {
        let mut s = Esp32c3Spi::new(SPI2_INTR_SOURCE_ID);
        s.write_u32(SLAVE, SOFT_RESET_BIT).unwrap();
        assert_eq!(
            s.read_u32(SLAVE).unwrap() & SOFT_RESET_BIT,
            0,
            "SOFT_RESET (WT) self-clears"
        );
    }

    #[test]
    fn w_buffer_round_trips() {
        let mut s = Esp32c3Spi::new(SPI2_INTR_SOURCE_ID);
        for w in 0..16u64 {
            s.write_u32(W0 + w * 4, 0xA000_0000 | (w as u32)).unwrap();
        }
        for w in 0..16u64 {
            assert_eq!(s.read_u32(W0 + w * 4).unwrap(), 0xA000_0000 | (w as u32));
        }
        assert_eq!(W15, W0 + 15 * 4, "W15 offset");
    }

    #[test]
    fn usr_start_auto_clears_and_latches_trans_done() {
        let mut s = Esp32c3Spi::new(SPI2_INTR_SOURCE_ID);
        s.write_u32(MS_DLEN, 32 - 1).unwrap(); // 32-bit transfer
        s.write_u32(CMD, USR_BIT).unwrap();
        assert_eq!(s.read_u32(CMD).unwrap() & USR_BIT, 0, "USR auto-clears");
        assert_eq!(
            s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE,
            TRANS_DONE,
            "TRANS_DONE latched"
        );
    }

    #[test]
    fn miso_fills_0xff_for_no_device() {
        let mut s = Esp32c3Spi::new(SPI2_INTR_SOURCE_ID);
        s.write_u32(W0, 0x1234_5678).unwrap();
        s.write_u32(W0 + 4, 0x0000_0000).unwrap();
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
    fn full_duplex_routes_mosi_through_attached_device() {
        // A device that returns mosi ^ 0xA5 proves real full-duplex: the MISO
        // bytes written back to W0 are a function of the MOSI bytes shifted out,
        // which a declarative register file could never produce.
        struct Xor(Vec<u8>);
        impl SpiDevice for Xor {
            fn transfer(&mut self, mosi: u8) -> u8 {
                self.0.push(mosi);
                mosi ^ 0xA5
            }
            fn cs_pin(&self) -> &str {
                "GPIO10"
            }
        }
        let mut s = Esp32c3Spi::new(SPI2_INTR_SOURCE_ID);
        s.push_device(Box::new(Xor(Vec::new())));
        // MOSI = 0xEFBE_ADDE little-endian → bytes DE AD BE EF.
        s.write_u32(W0, 0xEFBE_ADDE).unwrap();
        s.write_u32(MS_DLEN, 32 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        // MISO = each byte ^ 0xA5 → 7B 08 1B 4A, packed little-endian.
        let expect = u32::from_le_bytes([0xDE ^ 0xA5, 0xAD ^ 0xA5, 0xBE ^ 0xA5, 0xEF ^ 0xA5]);
        assert_eq!(s.read_u32(W0).unwrap(), expect, "full-duplex MISO in W0");
    }

    #[test]
    fn int_clr_is_w1c() {
        let mut s = Esp32c3Spi::new(SPI2_INTR_SOURCE_ID);
        s.write_u32(MS_DLEN, 8 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        assert_eq!(s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE, TRANS_DONE);
        s.write_u32(DMA_INT_CLR, TRANS_DONE).unwrap();
        assert_eq!(
            s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE,
            0,
            "W1C cleared"
        );
    }

    #[test]
    fn int_st_masks_with_ena_and_emits_source() {
        let mut s = Esp32c3Spi::new(SPI2_INTR_SOURCE_ID);
        assert!(
            !s.legacy_tick_active(),
            "idle level-IRQ SPI must stay out of the legacy tick walk"
        );
        assert!(
            s.legacy_tick_dynamic(),
            "writes that assert/clear INT_ST must refresh tick membership"
        );
        s.write_u32(MS_DLEN, 8 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        // No enable yet → ST gated, no source emitted.
        assert_eq!(s.read_u32(DMA_INT_ST).unwrap(), 0, "ST gated by ENA");
        assert_eq!(s.tick().explicit_irqs, None, "no source while ST==0");
        // Enable TRANS_DONE → ST asserts and the source is emitted.
        s.write_u32(DMA_INT_ENA, TRANS_DONE).unwrap();
        assert_eq!(s.read_u32(DMA_INT_ST).unwrap() & TRANS_DONE, TRANS_DONE);
        assert!(s.legacy_tick_active(), "asserted INT_ST needs level ticks");
        assert_eq!(s.tick().explicit_irqs, Some(vec![SPI2_INTR_SOURCE_ID]));
        // Clear the raw bit → ST drops, source stops.
        s.write_u32(DMA_INT_CLR, TRANS_DONE).unwrap();
        assert!(
            !s.legacy_tick_active(),
            "cleared INT_ST can leave tick walk"
        );
        assert_eq!(s.tick().explicit_irqs, None);
    }

    #[test]
    fn trans_done_not_set_before_launch() {
        let s = Esp32c3Spi::new(SPI2_INTR_SOURCE_ID);
        assert_eq!(
            s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE,
            0,
            "TRANS_DONE must not be set before USR"
        );
    }
}
