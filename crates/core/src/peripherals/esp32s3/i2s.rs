// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 I2S controller (I2S0 + I2S1) — configuration + control digital twin.
//!
//! The S3 has two identical I2S peripherals supporting standard (Philips),
//! TDM and PDM modes, each with independent TX and RX paths:
//!
//! * I2S0 @ `0x6000_F000`
//! * I2S1 @ `0x6002_D000`
//!
//! Both share the same register layout, so a single [`Esp32s3I2s`] type models
//! either one. The parent constructs two instances and passes the matching
//! interrupt-source id to [`Esp32s3I2s::new`].
//!
//! ## Scope
//!
//! This twin faithfully round-trips every configuration register the esp-hal /
//! ESP-IDF I2S drivers program (TX/RX `CONF`/`CONF1`, the clock dividers,
//! timing, TDM/slot control, EOF count, PCM-to-PDM conversion config). It also
//! models the control semantics the polling driver paths depend on:
//!
//! * `TX_RESET` / `RX_RESET` / `TX_FIFO_RESET` / `RX_FIFO_RESET` are
//!   write-pulse bits in `TX_CONF` / `RX_CONF`. Real silicon self-clears them
//!   once the reset completes; we accept the write and immediately clear the
//!   bit so a subsequent read sees it deasserted (matching the driver's
//!   "write 1, then poll for 0 / write 0" idiom).
//! * Setting `TX_START` / `RX_START` latches an internal running flag (which
//!   reads back through `TX_CONF` / `RX_CONF`) and raises the corresponding
//!   `TX_DONE` / `RX_DONE` raw interrupt, so firmware that polls `INT_RAW`
//!   (or waits on the IRQ) for "first descriptor done" proceeds instead of
//!   hanging.
//!
//! Actual audio-sample streaming on the S3 flows entirely through GDMA
//! (the I2S core has no CPU-visible sample FIFO register on this chip) and is
//! therefore **out of scope** for this peripheral — we model the MMIO control
//! surface, not the DMA-fed bit clock.
//!
//! ## Register map (ESP32-S3 TRM §38; `soc/i2s_reg.h`)
//!
//! | Offset | Name                | Notes                                       |
//! |-------:|---------------------|---------------------------------------------|
//! | 0x000C | INT_RAW             | RX_DONE=b0 TX_DONE=b1 RX_HUNG=b2 TX_HUNG=b3  |
//! | 0x0010 | INT_ST              | INT_RAW & INT_ENA (read-only)               |
//! | 0x0014 | INT_ENA             | enable mask                                 |
//! | 0x0018 | INT_CLR             | W1C against INT_RAW                         |
//! | 0x0020 | RX_CONF             | RX_RESET=b0 RX_FIFO_RESET=b1 RX_START=b2     |
//! | 0x0024 | TX_CONF             | TX_RESET=b0 TX_FIFO_RESET=b1 TX_START=b2     |
//! | 0x0028 | RX_CONF1            | RX bit/slot widths                          |
//! | 0x002C | TX_CONF1            | TX bit/slot widths                          |
//! | 0x0030 | RX_CLKM_CONF        | RX master-clock source/divider              |
//! | 0x0034 | TX_CLKM_CONF        | TX master-clock source/divider              |
//! | 0x0038 | RX_CLKM_DIV_CONF    | RX fractional divider                       |
//! | 0x003C | TX_CLKM_DIV_CONF    | TX fractional divider                       |
//! | 0x0040 | TX_PCM2PDM_CONF     | PCM→PDM conversion config                   |
//! | 0x0044 | TX_PCM2PDM_CONF1    | PCM→PDM conversion config (cont.)           |
//! | 0x0050 | RX_TDM_CTRL         | RX TDM slot enables                         |
//! | 0x0054 | TX_TDM_CTRL         | TX TDM slot enables                         |
//! | 0x0058 | RX_TIMING           | RX data/clock edge timing                   |
//! | 0x005C | TX_TIMING           | TX data/clock edge timing                   |
//! | 0x0064 | RXEOF_NUM           | RX EOF byte count (DMA in-link EOF)          |
//! | 0x0068 | CONF_SIGLE_DATA     | constant data driven on idle channels       |
//!
//! Any other offset accepts writes silently and reads 0.

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// I2S0 MMIO base address.
pub const I2S0_BASE: u32 = 0x6000_F000;
/// I2S1 MMIO base address.
pub const I2S1_BASE: u32 = 0x6002_D000;
/// MMIO window size (4 KiB) for either controller.
pub const I2S_SIZE: u64 = 0x1000;

/// `ETS_I2S0_INTR_SOURCE` — verified position in the ESP32-S3 interrupt-source
/// enum (`soc/interrupts.h`): WIFI_MAC=0 … GPIO=16 … LCD_CAM=24, **I2S0=25**.
pub const I2S0_INTR_SOURCE_ID: u32 = 25;
/// `ETS_I2S1_INTR_SOURCE` — immediately follows I2S0, so **I2S1=26**.
pub const I2S1_INTR_SOURCE_ID: u32 = 26;

// --- register offsets ---
const REG_INT_RAW: u64 = 0x0C;
const REG_INT_ST: u64 = 0x10;
const REG_INT_ENA: u64 = 0x14;
const REG_INT_CLR: u64 = 0x18;
const REG_RX_CONF: u64 = 0x20;
const REG_TX_CONF: u64 = 0x24;
const REG_RX_CONF1: u64 = 0x28;
const REG_TX_CONF1: u64 = 0x2C;
const REG_RX_CLKM_CONF: u64 = 0x30;
const REG_TX_CLKM_CONF: u64 = 0x34;
const REG_RX_CLKM_DIV_CONF: u64 = 0x38;
const REG_TX_CLKM_DIV_CONF: u64 = 0x3C;
const REG_TX_PCM2PDM_CONF: u64 = 0x40;
const REG_TX_PCM2PDM_CONF1: u64 = 0x44;
const REG_RX_TDM_CTRL: u64 = 0x50;
const REG_TX_TDM_CTRL: u64 = 0x54;
const REG_RX_TIMING: u64 = 0x58;
const REG_TX_TIMING: u64 = 0x5C;
const REG_RXEOF_NUM: u64 = 0x64;
const REG_CONF_SIGLE_DATA: u64 = 0x68;

// --- TX_CONF / RX_CONF control bits (TRM §38; soc/i2s_reg.h) ---
// RX_CONF and TX_CONF use the same low bits.
const RESET_BIT: u32 = 1 << 0; // TX_RESET / RX_RESET
const FIFO_RESET_BIT: u32 = 1 << 1; // TX_FIFO_RESET / RX_FIFO_RESET
const START_BIT: u32 = 1 << 2; // TX_START / RX_START

// --- INT_* bit positions (RAW/ST/ENA/CLR share the layout) ---
/// RX path finished (descriptor done) — bit 0.
pub const INT_RX_DONE: u32 = 1 << 0;
/// TX path finished (descriptor done) — bit 1.
pub const INT_TX_DONE: u32 = 1 << 1;
/// RX FIFO hung — bit 2.
pub const INT_RX_HUNG: u32 = 1 << 2;
/// TX FIFO hung — bit 3.
pub const INT_TX_HUNG: u32 = 1 << 3;

/// Bits that are write-pulses in TX_CONF / RX_CONF and must self-clear.
const PULSE_BITS: u32 = RESET_BIT | FIFO_RESET_BIT;

pub struct Esp32s3I2s {
    /// Interrupt-matrix source id for this controller (25=I2S0, 26=I2S1).
    source_id: u32,

    // Configuration registers — pure round-trip storage (minus the
    // self-clearing pulse bits in tx_conf / rx_conf, handled on write).
    rx_conf: u32,
    tx_conf: u32,
    rx_conf1: u32,
    tx_conf1: u32,
    rx_clkm_conf: u32,
    tx_clkm_conf: u32,
    rx_clkm_div_conf: u32,
    tx_clkm_div_conf: u32,
    tx_pcm2pdm_conf: u32,
    tx_pcm2pdm_conf1: u32,
    rx_tdm_ctrl: u32,
    tx_tdm_ctrl: u32,
    rx_timing: u32,
    tx_timing: u32,
    rxeof_num: u32,
    conf_sigle_data: u32,

    // Interrupt state.
    int_raw: u32,
    int_ena: u32,

    /// Set while TX_START is asserted (readable via TX_CONF bit 2).
    tx_running: bool,
    /// Set while RX_START is asserted (readable via RX_CONF bit 2).
    rx_running: bool,
}

impl Esp32s3I2s {
    /// Construct one controller. `source_id` is the interrupt-matrix source
    /// ([`I2S0_INTR_SOURCE_ID`] or [`I2S1_INTR_SOURCE_ID`]).
    pub fn new(source_id: u32) -> Self {
        Self {
            source_id,
            // Reset defaults: the S3 I2S config registers come out of reset
            // all-zero (POR value 0x0000_0000 for every register modeled here
            // per the TRM reset column); seed them explicitly for clarity.
            rx_conf: 0,
            tx_conf: 0,
            rx_conf1: 0,
            tx_conf1: 0,
            rx_clkm_conf: 0,
            tx_clkm_conf: 0,
            rx_clkm_div_conf: 0,
            tx_clkm_div_conf: 0,
            tx_pcm2pdm_conf: 0,
            tx_pcm2pdm_conf1: 0,
            rx_tdm_ctrl: 0,
            tx_tdm_ctrl: 0,
            rx_timing: 0,
            tx_timing: 0,
            rxeof_num: 0,
            conf_sigle_data: 0,
            int_raw: 0,
            int_ena: 0,
            tx_running: false,
            rx_running: false,
        }
    }
}

impl Default for Esp32s3I2s {
    fn default() -> Self {
        Self::new(I2S0_INTR_SOURCE_ID)
    }
}

impl std::fmt::Debug for Esp32s3I2s {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Esp32s3I2s")
            .field("source_id", &self.source_id)
            .field("tx_conf", &format_args!("{:#010x}", self.tx_conf))
            .field("rx_conf", &format_args!("{:#010x}", self.rx_conf))
            .field("tx_running", &self.tx_running)
            .field("rx_running", &self.rx_running)
            .field("int_raw", &format_args!("{:#010x}", self.int_raw))
            .field("int_ena", &format_args!("{:#010x}", self.int_ena))
            .finish()
    }
}

impl Esp32s3I2s {
    /// Apply a write to TX_CONF: latch config bits, honor the write-pulse
    /// reset bits (accept + self-clear), and act on TX_START.
    fn write_tx_conf(&mut self, value: u32) {
        if value & FIFO_RESET_BIT != 0 {
            // TX FIFO reset pulse: nothing CPU-visible to clear here (the FIFO
            // is GDMA-fed and not modeled), but we accept it.
        }
        if value & START_BIT != 0 {
            self.tx_running = true;
            // Latch TX_DONE so a driver polling INT_RAW (or waiting on the
            // IRQ) for first-descriptor completion makes progress. Real
            // hardware would set this when GDMA signals EOF on the out-link.
            self.int_raw |= INT_TX_DONE;
        } else {
            self.tx_running = false;
        }
        // Store the value but strip the self-clearing pulse bits and keep
        // START reflected by tx_running (rebuilt on read).
        self.tx_conf = value & !PULSE_BITS;
    }

    /// Apply a write to RX_CONF: mirror of [`Self::write_tx_conf`].
    fn write_rx_conf(&mut self, value: u32) {
        if value & FIFO_RESET_BIT != 0 {
            // RX FIFO reset pulse — accepted, no modeled FIFO to clear.
        }
        if value & START_BIT != 0 {
            self.rx_running = true;
            self.int_raw |= INT_RX_DONE;
        } else {
            self.rx_running = false;
        }
        self.rx_conf = value & !PULSE_BITS;
    }
}

impl Peripheral for Esp32s3I2s {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        // The esp-hal / ESP-IDF I2S drivers use 32-bit accesses exclusively;
        // stray byte reads return 0 harmlessly.
        Ok(0)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let v = match offset {
            REG_INT_RAW => self.int_raw,
            REG_INT_ST => self.int_raw & self.int_ena,
            REG_INT_ENA => self.int_ena,
            REG_INT_CLR => 0, // write-only W1C; reads as 0
            // TX/RX_CONF: stored value OR the live running (START) bit. The
            // pulse reset bits were stripped on write, so they read back 0.
            REG_TX_CONF => self.tx_conf | if self.tx_running { START_BIT } else { 0 },
            REG_RX_CONF => self.rx_conf | if self.rx_running { START_BIT } else { 0 },
            REG_TX_CONF1 => self.tx_conf1,
            REG_RX_CONF1 => self.rx_conf1,
            REG_TX_CLKM_CONF => self.tx_clkm_conf,
            REG_RX_CLKM_CONF => self.rx_clkm_conf,
            REG_TX_CLKM_DIV_CONF => self.tx_clkm_div_conf,
            REG_RX_CLKM_DIV_CONF => self.rx_clkm_div_conf,
            REG_TX_PCM2PDM_CONF => self.tx_pcm2pdm_conf,
            REG_TX_PCM2PDM_CONF1 => self.tx_pcm2pdm_conf1,
            REG_RX_TDM_CTRL => self.rx_tdm_ctrl,
            REG_TX_TDM_CTRL => self.tx_tdm_ctrl,
            REG_RX_TIMING => self.rx_timing,
            REG_TX_TIMING => self.tx_timing,
            REG_RXEOF_NUM => self.rxeof_num,
            REG_CONF_SIGLE_DATA => self.conf_sigle_data,
            _ => 0,
        };
        Ok(v)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // Byte writes ignored — driver writes whole words.
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            REG_TX_CONF => self.write_tx_conf(value),
            REG_RX_CONF => self.write_rx_conf(value),
            REG_INT_ENA => self.int_ena = value,
            // INT_RAW is technically read-only on hardware; accept writes
            // (driver never writes it) but only let INT_CLR mutate it.
            REG_INT_RAW => self.int_raw = value,
            REG_INT_CLR => self.int_raw &= !value, // W1C
            REG_TX_CONF1 => self.tx_conf1 = value,
            REG_RX_CONF1 => self.rx_conf1 = value,
            REG_TX_CLKM_CONF => self.tx_clkm_conf = value,
            REG_RX_CLKM_CONF => self.rx_clkm_conf = value,
            REG_TX_CLKM_DIV_CONF => self.tx_clkm_div_conf = value,
            REG_RX_CLKM_DIV_CONF => self.rx_clkm_div_conf = value,
            REG_TX_PCM2PDM_CONF => self.tx_pcm2pdm_conf = value,
            REG_TX_PCM2PDM_CONF1 => self.tx_pcm2pdm_conf1 = value,
            REG_RX_TDM_CTRL => self.rx_tdm_ctrl = value,
            REG_TX_TDM_CTRL => self.tx_tdm_ctrl = value,
            REG_RX_TIMING => self.rx_timing = value,
            REG_TX_TIMING => self.tx_timing = value,
            REG_RXEOF_NUM => self.rxeof_num = value,
            REG_CONF_SIGLE_DATA => self.conf_sigle_data = value,
            _ => {} // Accept-and-ignore other offsets.
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Emit our interrupt source while any enabled raw bit is set (level
        // semantics — I2S sources are level-triggered per soc/interrupts.h).
        let explicit = if self.int_raw & self.int_ena != 0 {
            Some(vec![self.source_id])
        } else {
            None
        };
        PeripheralTickResult {
            explicit_irqs: explicit,
            ..Default::default()
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
    fn conf_registers_round_trip() {
        let mut p = Esp32s3I2s::new(I2S0_INTR_SOURCE_ID);
        // CONF1 / clock / timing / TDM / EOF all behave as plain storage.
        p.write_u32(REG_TX_CONF1, 0x1234_5678).unwrap();
        p.write_u32(REG_RX_CONF1, 0x0BAD_F00D).unwrap();
        p.write_u32(REG_TX_TDM_CTRL, 0x0000_FFFF).unwrap();
        p.write_u32(REG_RX_TDM_CTRL, 0x0000_00FF).unwrap();
        p.write_u32(REG_RXEOF_NUM, 0x0000_0040).unwrap();
        p.write_u32(REG_CONF_SIGLE_DATA, 0xDEAD_BEEF).unwrap();
        assert_eq!(p.read_u32(REG_TX_CONF1).unwrap(), 0x1234_5678);
        assert_eq!(p.read_u32(REG_RX_CONF1).unwrap(), 0x0BAD_F00D);
        assert_eq!(p.read_u32(REG_TX_TDM_CTRL).unwrap(), 0x0000_FFFF);
        assert_eq!(p.read_u32(REG_RX_TDM_CTRL).unwrap(), 0x0000_00FF);
        assert_eq!(p.read_u32(REG_RXEOF_NUM).unwrap(), 0x0000_0040);
        assert_eq!(p.read_u32(REG_CONF_SIGLE_DATA).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn clock_dividers_round_trip() {
        let mut p = Esp32s3I2s::new(I2S0_INTR_SOURCE_ID);
        p.write_u32(REG_TX_CLKM_CONF, 0x0000_2102).unwrap();
        p.write_u32(REG_RX_CLKM_CONF, 0x0000_2103).unwrap();
        p.write_u32(REG_TX_CLKM_DIV_CONF, 0x0000_0500).unwrap();
        p.write_u32(REG_RX_CLKM_DIV_CONF, 0x0000_0501).unwrap();
        assert_eq!(p.read_u32(REG_TX_CLKM_CONF).unwrap(), 0x0000_2102);
        assert_eq!(p.read_u32(REG_RX_CLKM_CONF).unwrap(), 0x0000_2103);
        assert_eq!(p.read_u32(REG_TX_CLKM_DIV_CONF).unwrap(), 0x0000_0500);
        assert_eq!(p.read_u32(REG_RX_CLKM_DIV_CONF).unwrap(), 0x0000_0501);
    }

    #[test]
    fn timing_and_pcm2pdm_round_trip() {
        let mut p = Esp32s3I2s::new(I2S1_INTR_SOURCE_ID);
        p.write_u32(REG_TX_TIMING, 0x0000_0011).unwrap();
        p.write_u32(REG_RX_TIMING, 0x0000_0022).unwrap();
        p.write_u32(REG_TX_PCM2PDM_CONF, 0xA5A5_A5A5).unwrap();
        p.write_u32(REG_TX_PCM2PDM_CONF1, 0x5A5A_5A5A).unwrap();
        assert_eq!(p.read_u32(REG_TX_TIMING).unwrap(), 0x0000_0011);
        assert_eq!(p.read_u32(REG_RX_TIMING).unwrap(), 0x0000_0022);
        assert_eq!(p.read_u32(REG_TX_PCM2PDM_CONF).unwrap(), 0xA5A5_A5A5);
        assert_eq!(p.read_u32(REG_TX_PCM2PDM_CONF1).unwrap(), 0x5A5A_5A5A);
    }

    #[test]
    fn reset_pulse_bits_self_clear() {
        let mut p = Esp32s3I2s::new(I2S0_INTR_SOURCE_ID);
        // Write TX_RESET | TX_FIFO_RESET pulses (bits 0,1). They must read
        // back as 0 — real silicon self-clears once reset completes.
        p.write_u32(REG_TX_CONF, RESET_BIT | FIFO_RESET_BIT)
            .unwrap();
        assert_eq!(p.read_u32(REG_TX_CONF).unwrap() & PULSE_BITS, 0);

        p.write_u32(REG_RX_CONF, RESET_BIT | FIFO_RESET_BIT)
            .unwrap();
        assert_eq!(p.read_u32(REG_RX_CONF).unwrap() & PULSE_BITS, 0);
    }

    #[test]
    fn tx_conf_non_pulse_bits_round_trip() {
        let mut p = Esp32s3I2s::new(I2S0_INTR_SOURCE_ID);
        // Bit 9 (TX_CHAN_EQUAL or similar config) should persist; pulse bits
        // should not.
        let cfg = (1 << 9) | RESET_BIT;
        p.write_u32(REG_TX_CONF, cfg).unwrap();
        assert_eq!(p.read_u32(REG_TX_CONF).unwrap(), 1 << 9);
    }

    #[test]
    fn tx_start_sets_running_and_latches_tx_done() {
        let mut p = Esp32s3I2s::new(I2S0_INTR_SOURCE_ID);
        p.write_u32(REG_TX_CONF, START_BIT).unwrap();
        // START reads back through TX_CONF.
        assert_eq!(p.read_u32(REG_TX_CONF).unwrap() & START_BIT, START_BIT);
        // TX_DONE latched into INT_RAW so polling firmware proceeds.
        assert_eq!(p.read_u32(REG_INT_RAW).unwrap() & INT_TX_DONE, INT_TX_DONE);
    }

    #[test]
    fn rx_start_sets_running_and_latches_rx_done() {
        let mut p = Esp32s3I2s::new(I2S0_INTR_SOURCE_ID);
        p.write_u32(REG_RX_CONF, START_BIT).unwrap();
        assert_eq!(p.read_u32(REG_RX_CONF).unwrap() & START_BIT, START_BIT);
        assert_eq!(p.read_u32(REG_INT_RAW).unwrap() & INT_RX_DONE, INT_RX_DONE);
    }

    #[test]
    fn clearing_start_stops_running() {
        let mut p = Esp32s3I2s::new(I2S0_INTR_SOURCE_ID);
        p.write_u32(REG_TX_CONF, START_BIT).unwrap();
        assert_eq!(p.read_u32(REG_TX_CONF).unwrap() & START_BIT, START_BIT);
        p.write_u32(REG_TX_CONF, 0).unwrap();
        assert_eq!(p.read_u32(REG_TX_CONF).unwrap() & START_BIT, 0);
    }

    #[test]
    fn int_clr_is_write_one_to_clear() {
        let mut p = Esp32s3I2s::new(I2S0_INTR_SOURCE_ID);
        p.write_u32(REG_INT_RAW, INT_TX_DONE | INT_RX_DONE | INT_TX_HUNG)
            .unwrap();
        // Clear only TX_DONE; the others remain.
        p.write_u32(REG_INT_CLR, INT_TX_DONE).unwrap();
        assert_eq!(p.read_u32(REG_INT_RAW).unwrap(), INT_RX_DONE | INT_TX_HUNG);
        // INT_CLR reads back as 0.
        assert_eq!(p.read_u32(REG_INT_CLR).unwrap(), 0);
    }

    #[test]
    fn int_st_masks_with_int_ena() {
        let mut p = Esp32s3I2s::new(I2S0_INTR_SOURCE_ID);
        p.write_u32(REG_INT_RAW, INT_TX_DONE | INT_RX_DONE).unwrap();
        p.write_u32(REG_INT_ENA, INT_TX_DONE).unwrap();
        assert_eq!(p.read_u32(REG_INT_ST).unwrap(), INT_TX_DONE);
    }

    #[test]
    fn tick_emits_source_while_int_st_set() {
        let mut p = Esp32s3I2s::new(I2S1_INTR_SOURCE_ID);
        // No enable → no emission even with raw set.
        p.write_u32(REG_INT_RAW, INT_TX_DONE).unwrap();
        assert!(p.tick().explicit_irqs.is_none());

        // Enable TX_DONE → source emitted (level: stays asserted on re-tick).
        p.write_u32(REG_INT_ENA, INT_TX_DONE).unwrap();
        assert_eq!(
            p.tick().explicit_irqs.as_deref(),
            Some(&[I2S1_INTR_SOURCE_ID][..])
        );
        assert_eq!(
            p.tick().explicit_irqs.as_deref(),
            Some(&[I2S1_INTR_SOURCE_ID][..]),
            "level-triggered: re-asserts while INT_ST != 0"
        );

        // Clear the raw bit → emission stops.
        p.write_u32(REG_INT_CLR, INT_TX_DONE).unwrap();
        assert!(p.tick().explicit_irqs.is_none());
    }

    #[test]
    fn two_instances_are_independent() {
        let mut i2s0 = Esp32s3I2s::new(I2S0_INTR_SOURCE_ID);
        let mut i2s1 = Esp32s3I2s::new(I2S1_INTR_SOURCE_ID);

        // Configure I2S0 only.
        i2s0.write_u32(REG_TX_CLKM_CONF, 0x0000_1234).unwrap();
        i2s0.write_u32(REG_TX_CONF, START_BIT).unwrap();
        i2s0.write_u32(REG_INT_ENA, INT_TX_DONE).unwrap();

        // I2S1 untouched.
        assert_eq!(i2s1.read_u32(REG_TX_CLKM_CONF).unwrap(), 0);
        assert_eq!(i2s1.read_u32(REG_TX_CONF).unwrap() & START_BIT, 0);
        assert_eq!(i2s1.read_u32(REG_INT_RAW).unwrap(), 0);

        // Each emits its own source id.
        assert_eq!(
            i2s0.tick().explicit_irqs.as_deref(),
            Some(&[I2S0_INTR_SOURCE_ID][..])
        );
        assert!(i2s1.tick().explicit_irqs.is_none());
    }

    #[test]
    fn reset_defaults_are_zero() {
        let p = Esp32s3I2s::new(I2S0_INTR_SOURCE_ID);
        for off in [
            REG_TX_CONF,
            REG_RX_CONF,
            REG_TX_CONF1,
            REG_RX_CONF1,
            REG_TX_CLKM_CONF,
            REG_RX_CLKM_CONF,
            REG_TX_CLKM_DIV_CONF,
            REG_RX_CLKM_DIV_CONF,
            REG_TX_TIMING,
            REG_RX_TIMING,
            REG_TX_TDM_CTRL,
            REG_RX_TDM_CTRL,
            REG_RXEOF_NUM,
            REG_INT_RAW,
            REG_INT_ENA,
        ] {
            assert_eq!(p.read_u32(off).unwrap(), 0, "offset {off:#x} not zero");
        }
    }

    #[test]
    fn unmapped_offsets_read_zero_and_accept_writes() {
        let mut p = Esp32s3I2s::new(I2S0_INTR_SOURCE_ID);
        p.write_u32(0xFFC, 0xDEAD_BEEF).unwrap();
        assert_eq!(p.read_u32(0xFFC).unwrap(), 0);
    }
}
