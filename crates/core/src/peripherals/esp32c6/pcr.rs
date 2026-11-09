// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C6 PCR — Peripheral Clock and Reset.
//!
//! Base = `DR_REG_PCR_BASE` = `0x6009_6000` (esp-idf v5.3 `soc/reg_base.h`).
//! PCR is the C6's clock/reset block: one `xxx_CONF` register per peripheral
//! carrying `CLK_EN` (bit 0) and `RST_EN` (bit 1), plus function-clock divider
//! registers (`xxx_SCLK_CONF` / `xxx_FUNC_CLK_CONF`) and the system clock
//! selection (`SYSCLK_CONF`).
//!
//! ## What this model does
//!
//! * Register-backed storage for the **whole SVD-documented PCR map**
//!   ([`PCR_REGISTERS`], generated from
//!   `tests/fixtures/real_world/esp32c6.svd`), seeded with the SVD reset
//!   values. Reads return the stored word; writes store and read back, so
//!   firmware RMW sequences see their own writes.
//! * Implements [`Peripheral::clock_gate_reg_offset`], which resolves the
//!   symbolic `reg:` names used by chip-yaml `clock:` declarations
//!   (e.g. `UART0_CONF`, `TIMERGROUP0_CONF`, `GDMA_CONF`) to this block's
//!   offsets. The generic bus gate
//!   (`crates/core/src/bus/device_hooks.rs::is_peripheral_clocked`) then makes
//!   a gated peripheral's reads return 0 and drop its writes while the gate
//!   bit is clear — exactly the observable a programming manual documents.
//!
//! ## What this model deliberately does NOT claim
//!
//! * No clock tree: `SYSCLK_CONF`, the divider registers and `PLL_DIV_CLK_EN`
//!   are stored, not interpreted. Nothing is paced from them; the chip
//!   descriptor's `cpu_hz` remains the single clock fact.
//! * `RST_EN` is stored and readable but **not enforced**. The generic gate
//!   requires listed bits to read 1, and `xxx_CONF.RST_EN` resets to 0 while
//!   the C6 comes out of power-on reset with the peripheral usable, so
//!   wiring it as a second gate bit would gate the console at boot. Reset
//!   semantics are a documented limitation (see
//!   `examples/esp32c6-devkitc/KNOWN_LIMITATIONS.md`).
//! * Only HP PCR is modelled. The LP clock registers (`LP_*`, elsewhere in
//!   the C6 map) are out of scope.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// PCR register table: `(offset, name, SVD reset value)`.
/// Generated from `tests/fixtures/real_world/esp32c6.svd` (PCR @ 0x6009_6000).
/// Registers whose SVD entry has no `resetValue` are seeded 0 — the SVD
/// leaves them unspecified; they are round-trip storage only.
pub const PCR_REGISTERS: &[(u32, &str, u32)] = &[
    (0x000, "UART0_CONF", 0x00000001),
    (0x004, "UART0_SCLK_CONF", 0x00700000),
    (0x008, "UART0_PD_CTRL", 0x00000002),
    (0x00C, "UART1_CONF", 0x00000001),
    (0x010, "UART1_SCLK_CONF", 0x00700000),
    (0x014, "UART1_PD_CTRL", 0x00000002),
    (0x018, "MSPI_CONF", 0x00000005),
    (0x01C, "MSPI_CLK_CONF", 0x00000300),
    (0x020, "I2C0_CONF", 0x00000001),
    (0x024, "I2C_SCLK_CONF", 0x00400000),
    (0x028, "UHCI_CONF", 0x00000001),
    (0x02C, "RMT_CONF", 0x00000001),
    (0x030, "RMT_SCLK_CONF", 0x00501000),
    (0x034, "LEDC_CONF", 0x00000001),
    (0x038, "LEDC_SCLK_CONF", 0x00400000),
    (0x03C, "TIMERGROUP0_CONF", 0x00000001),
    (0x040, "TIMERGROUP0_TIMER_CLK_CONF", 0x00400000),
    (0x044, "TIMERGROUP0_WDT_CLK_CONF", 0x00400000),
    (0x048, "TIMERGROUP1_CONF", 0x00000001),
    (0x04C, "TIMERGROUP1_TIMER_CLK_CONF", 0x00400000),
    (0x050, "TIMERGROUP1_WDT_CLK_CONF", 0x00400000),
    (0x054, "SYSTIMER_CONF", 0x00000001),
    (0x058, "SYSTIMER_FUNC_CLK_CONF", 0x00400000),
    (0x05C, "TWAI0_CONF", 0x00000001),
    (0x060, "TWAI0_FUNC_CLK_CONF", 0x00400000),
    (0x064, "TWAI1_CONF", 0x00000001),
    (0x068, "TWAI1_FUNC_CLK_CONF", 0x00400000),
    (0x06C, "I2S_CONF", 0x00000001),
    (0x070, "I2S_TX_CLKM_CONF", 0x00402000),
    (0x074, "I2S_TX_CLKM_DIV_CONF", 0x00000200),
    (0x078, "I2S_RX_CLKM_CONF", 0x00402000),
    (0x07C, "I2S_RX_CLKM_DIV_CONF", 0x00000200),
    (0x080, "SARADC_CONF", 0x00000005),
    (0x084, "SARADC_CLKM_CONF", 0x00404000),
    (0x088, "TSENS_CLK_CONF", 0x00400000),
    (0x08C, "USB_DEVICE_CONF", 0x00000001),
    (0x090, "INTMTX_CONF", 0x00000001),
    (0x094, "PCNT_CONF", 0x00000001),
    (0x098, "ETM_CONF", 0x00000001),
    (0x09C, "PWM_CONF", 0x00000001),
    (0x0A0, "PWM_CLK_CONF", 0x00404000),
    (0x0A4, "PARL_IO_CONF", 0x00000001),
    (0x0A8, "PARL_CLK_RX_CONF", 0x00040000),
    (0x0AC, "PARL_CLK_TX_CONF", 0x00040000),
    (0x0B0, "SDIO_SLAVE_CONF", 0x00000001),
    (0x0B4, "PVT_MONITOR_CONF", 0x0000001D),
    (0x0B8, "PVT_MONITOR_FUNC_CLK_CONF", 0x00400000),
    (0x0BC, "GDMA_CONF", 0x00000001),
    (0x0C0, "SPI2_CONF", 0x00000001),
    (0x0C4, "SPI2_CLKM_CONF", 0x00400000),
    (0x0C8, "AES_CONF", 0x00000001),
    (0x0CC, "SHA_CONF", 0x00000001),
    (0x0D0, "RSA_CONF", 0x00000001),
    (0x0D4, "RSA_PD_CTRL", 0x00000002),
    (0x0D8, "ECC_CONF", 0x00000001),
    (0x0DC, "ECC_PD_CTRL", 0x00000002),
    (0x0E0, "DS_CONF", 0x00000001),
    (0x0E4, "HMAC_CONF", 0x00000001),
    (0x0E8, "IOMUX_CONF", 0x00000001),
    (0x0EC, "IOMUX_CLK_CONF", 0x00700000),
    (0x0F0, "MEM_MONITOR_CONF", 0x00000001),
    (0x0F4, "REGDMA_CONF", 0x00000000),
    (0x0F8, "RETENTION_CONF", 0x00000000),
    (0x0FC, "TRACE_CONF", 0x00000001),
    (0x100, "ASSIST_CONF", 0x00000001),
    (0x104, "CACHE_CONF", 0x00000001),
    (0x108, "MODEM_APB_CONF", 0x00000001),
    (0x10C, "TIMEOUT_CONF", 0x00000000),
    (0x110, "SYSCLK_CONF", 0x28000200),
    (0x114, "CPU_WAITI_CONF", 0x0000000D),
    (0x118, "CPU_FREQ_CONF", 0x00000000),
    (0x11C, "AHB_FREQ_CONF", 0x00000300),
    (0x120, "APB_FREQ_CONF", 0x00000000),
    (0x124, "SYSCLK_FREQ_QUERY_0", 0x0001E014),
    (0x128, "PLL_DIV_CLK_EN", 0x0000007F),
    (0x12C, "CTRL_CLK_OUT_EN", 0x000007FF),
    (0x130, "CTRL_TICK_CONF", 0x00010727),
    (0x134, "CTRL_32K_CONF", 0x00000000),
    (0x138, "SRAM_POWER_CONF", 0x0000700F),
    (0xFF0, "RESET_EVENT_BYPASS", 0x00000002),
    (0xFF4, "FPGA_DEBUG", 0xFFFFFFFF),
    (0xFF8, "CLOCK_GATE", 0x00000000),
    (0xFFC, "DATE", 0x02206150),
];

/// ESP32-C6 Peripheral Clock and Reset block (HP PCR).
#[derive(Debug)]
pub struct Esp32c6Pcr {
    /// Word-aligned register backing store, seeded with the SVD reset values.
    regs: crate::FastMap<u64, u32>,
    /// Live mirrors handed out by `clock_select_cell` (offset, cell), refreshed
    /// on every write so a consumer reads the source select firmware set.
    mirrors: Vec<(u64, Arc<AtomicU32>)>,
}

impl Default for Esp32c6Pcr {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32c6Pcr {
    pub fn new() -> Self {
        let mut regs = crate::FastMap::default();
        for &(offset, _name, reset) in PCR_REGISTERS {
            if reset != 0 {
                regs.insert(u64::from(offset), reset);
            }
        }
        Self {
            regs,
            mirrors: Vec::new(),
        }
    }

    fn sync_mirror(&self, word_off: u64) {
        let v = self.word(word_off);
        for (off, cell) in &self.mirrors {
            if *off == word_off {
                cell.store(v, Ordering::Relaxed);
            }
        }
    }

    /// SVD register name at `offset`, if the PCR map has one.
    pub fn register_name(offset: u64) -> Option<&'static str> {
        PCR_REGISTERS
            .iter()
            .find(|(o, _, _)| u64::from(*o) == offset)
            .map(|(_, name, _)| *name)
    }

    /// Resolve an SVD register name (case-insensitive) to its byte offset.
    pub fn gate_reg_offset(name: &str) -> Option<u64> {
        PCR_REGISTERS
            .iter()
            .find(|(_, n, _)| n.eq_ignore_ascii_case(name))
            .map(|(o, _, _)| u64::from(*o))
    }

    fn word(&self, offset: u64) -> u32 {
        self.regs.get(&offset).copied().unwrap_or(0)
    }
}

impl Peripheral for Esp32c6Pcr {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        Ok(((self.word(word_off) >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let entry = self.regs.entry(word_off).or_insert(0);
        *entry &= !(0xFFu32 << byte_off);
        *entry |= (u32::from(value)) << byte_off;
        self.sync_mirror(word_off);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.regs.insert(offset & !3, value);
        self.sync_mirror(offset & !3);
        Ok(())
    }

    /// Side-effect-free probe: PCR is pure storage, so `read` cannot disturb
    /// anything and inspect can show live gate state.
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    /// Resolve a chip-yaml `clock:` gate's symbolic register name to this
    /// block's offset. The bus reads the live bit at access time, so a gate
    /// closed mid-run silences the peripheral immediately.
    fn clock_gate_reg_offset(&self, name: &str) -> Option<u64> {
        Self::gate_reg_offset(name)
    }

    /// A live mirror of a named PCR register: the C6 LEDC takes its source
    /// clock from `LEDC_SCLK_CONF.LEDC_SCLK_SEL` here, not from its own window.
    fn clock_select_cell(&mut self, name: &str) -> Option<Arc<AtomicU32>> {
        let off = Self::gate_reg_offset(name)?;
        if let Some((_, cell)) = self.mirrors.iter().find(|(o, _)| *o == off) {
            return Some(Arc::clone(cell));
        }
        let cell = Arc::new(AtomicU32::new(self.word(off)));
        self.mirrors.push((off, Arc::clone(&cell)));
        Some(cell)
    }

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
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
    fn reset_values_match_the_svd_for_the_gate_registers() {
        let p = Esp32c6Pcr::new();
        // Every gate register we actually declare on the C6 chip yaml.
        assert_eq!(p.read_u32(0x00).unwrap(), 1, "UART0_CONF reset");
        assert_eq!(p.read_u32(0x0C).unwrap(), 1, "UART1_CONF reset");
        assert_eq!(p.read_u32(0x3C).unwrap(), 1, "TIMERGROUP0_CONF reset");
        assert_eq!(p.read_u32(0x48).unwrap(), 1, "TIMERGROUP1_CONF reset");
        assert_eq!(p.read_u32(0xBC).unwrap(), 1, "GDMA_CONF reset");
        assert_eq!(p.read_u32(0x110).unwrap(), 0x2800_0200, "SYSCLK_CONF reset");
        assert_eq!(p.read_u32(0xFFC).unwrap(), 0x0220_6150, "DATE reset");
        // SVD entries with no resetValue seed 0.
        assert_eq!(p.read_u32(0x118).unwrap(), 0, "CPU_FREQ_CONF seeded 0");
    }

    #[test]
    fn writes_round_trip_and_unknown_offsets_are_storage() {
        let mut p = Esp32c6Pcr::new();
        p.write_u32(0x004, 0x0050_1234).unwrap();
        assert_eq!(p.read_u32(0x004).unwrap(), 0x0050_1234);
        // Gate bit writes are plain storage.
        p.write_u32(0x00, 0).unwrap();
        assert_eq!(p.read_u32(0x00).unwrap(), 0);
        p.write_u32(0x00, 3).unwrap();
        assert_eq!(p.read_u32(0x00).unwrap(), 3);
        // Byte-granular access to the gate register.
        p.write(0x00, 0x00).unwrap();
        assert_eq!(p.read(0x00).unwrap(), 0x00);
        assert_eq!(p.read(0x01).unwrap(), 0x00);
        // Unknown offsets round-trip rather than faulting.
        p.write_u32(0x200, 0xCAFE_BABE).unwrap();
        assert_eq!(p.read_u32(0x200).unwrap(), 0xCAFE_BABE);
    }

    #[test]
    fn gate_names_resolve_case_insensitively() {
        assert_eq!(Esp32c6Pcr::gate_reg_offset("UART0_CONF"), Some(0x00));
        assert_eq!(Esp32c6Pcr::gate_reg_offset("uart0_conf"), Some(0x00));
        assert_eq!(Esp32c6Pcr::gate_reg_offset("TIMERGROUP0_CONF"), Some(0x3C));
        assert_eq!(Esp32c6Pcr::gate_reg_offset("GDMA_CONF"), Some(0xBC));
        assert_eq!(Esp32c6Pcr::gate_reg_offset("NO_SUCH_REG"), None);
    }

    #[test]
    fn every_table_entry_has_a_unique_offset_and_name() {
        let mut offsets: Vec<u32> = PCR_REGISTERS.iter().map(|(o, _, _)| *o).collect();
        let mut names: Vec<&str> = PCR_REGISTERS.iter().map(|(_, n, _)| *n).collect();
        offsets.sort_unstable();
        names.sort_unstable();
        let n = offsets.len();
        offsets.dedup();
        names.dedup();
        assert_eq!(offsets.len(), n, "duplicate PCR offset in the table");
        assert_eq!(names.len(), n, "duplicate PCR register name in the table");
    }
}
