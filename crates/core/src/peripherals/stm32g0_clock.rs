// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32G0 fixed-operating-point timing guard, RM0444 §5.
//!
//! The machine's cycle frequency is fixed. Register readiness alone must not
//! make a 16MHz reference ELF into a 64MHz timing result. Derive the actual
//! programmed clocks, then require that they agree with the timebase before a
//! timed UART measurement. This is not a model of PLL locking, oscillator
//! tolerance, flash wait states, or dynamic clock transitions.

use crate::{bus::SystemBus, Bus};
use anyhow::{bail, ensure, Result};
use std::sync::atomic::{AtomicU32, Ordering};

/// Live RCC registers shared with attached transceivers. Only RCC writes
/// publish them; changing a diagram's fixed timebase cannot change these.
#[derive(Debug, Default)]
pub struct G0ClockRegisters {
    cr: AtomicU32,
    cfgr: AtomicU32,
    pllcfgr: AtomicU32,
    ccipr: AtomicU32,
}

impl G0ClockRegisters {
    pub(crate) fn publish(&self, cr: u32, cfgr: u32, pllcfgr: u32, ccipr: u32) {
        self.cr.store(cr, Ordering::Relaxed);
        self.cfgr.store(cfgr, Ordering::Relaxed);
        self.pllcfgr.store(pllcfgr, Ordering::Relaxed);
        self.ccipr.store(ccipr, Ordering::Relaxed);
    }
    pub fn validate(&self, fixed_hz: u64, usart: u8) -> Result<G0Clocks> {
        let clocks = G0Clocks::from_registers(
            self.cr.load(Ordering::Relaxed),
            self.cfgr.load(Ordering::Relaxed),
            self.pllcfgr.load(Ordering::Relaxed),
            None,
        )?;
        ensure!(
            clocks.hclk_hz == fixed_hz,
            "G0 programmed HCLK={}Hz disagrees with simulation timebase={}Hz",
            clocks.hclk_hz,
            fixed_hz
        );
        ensure!(
            clocks.pclk_hz == clocks.hclk_hz,
            "G0 TIOL112 timing requires APB prescaler /1"
        );
        let shift = match usart {
            1 => 0,
            2 => 2,
            _ => bail!("G0 TIOL112 supports USART1/2 only"),
        };
        ensure!(
            (self.ccipr.load(Ordering::Relaxed) >> shift) & 3 == 0,
            "G0 TIOL112 requires UART kernel PCLK"
        );
        Ok(clocks)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct G0Clocks {
    pub sysclk_hz: u64,
    pub hclk_hz: u64,
    pub pclk_hz: u64,
    pub timer_hz: u64,
}

impl G0Clocks {
    /// `hse_hz` is required only when firmware actually selects external HSE.
    /// HSI16 is before HSIDIV on the PLL input path. PLLR encodes R-1, unlike
    /// the 2/4/6/8 encoding on several other STM32 families.
    pub fn from_registers(cr: u32, cfgr: u32, pllcfgr: u32, hse_hz: Option<u64>) -> Result<Self> {
        let source_hz = |source| -> Result<u64> {
            match source {
                2 => {
                    ensure!(cr & (1 << 10) != 0, "G0 HSI16 is not ready");
                    Ok(16_000_000)
                }
                3 => {
                    ensure!(cr & (1 << 17) != 0, "G0 HSE is not ready");
                    hse_hz
                        .filter(|hz| *hz > 0)
                        .ok_or_else(|| anyhow::anyhow!("G0 HSE frequency must be declared"))
                }
                _ => bail!("G0 PLL source is absent or reserved"),
            }
        };
        let sysclk_hz = match (cfgr >> 3) & 7 {
            0 => {
                ensure!(cr & (1 << 10) != 0, "G0 HSISYS is not ready");
                16_000_000 >> ((cr >> 11) & 7)
            }
            1 => source_hz(3)?,
            2 => {
                ensure!(
                    cr & (1 << 25) != 0 && pllcfgr & (1 << 28) != 0,
                    "G0 PLLR output must be ready and enabled"
                );
                let input = source_hz(pllcfgr & 3)?;
                let m = u64::from(((pllcfgr >> 4) & 7) + 1);
                let n = u64::from((pllcfgr >> 8) & 0x7f);
                let r = u64::from(((pllcfgr >> 29) & 7) + 1);
                ensure!(
                    (8..=86).contains(&n) && (2..=8).contains(&r),
                    "G0 PLLN/PLLR is outside its supported encoding"
                );
                ensure!(
                    input * n % (m * r) == 0,
                    "G0 fractional PLL clock is unsupported by this integer timing guard"
                );
                input * n / (m * r)
            }
            _ => bail!("G0 low-speed/reserved system source is unsupported for timed IO-Link"),
        };
        ensure!(
            sysclk_hz <= 64_000_000,
            "G0 SYSCLK exceeds64MHz device rating"
        );
        let hpre = ((cfgr >> 8) & 15) as usize;
        let ahb_div = [1, 1, 1, 1, 1, 1, 1, 1, 2, 4, 8, 16, 64, 128, 256, 512][hpre];
        let ppre = (cfgr >> 12) & 7;
        let apb_div = if ppre < 4 { 1 } else { 1 << (ppre - 3) };
        let hclk_hz = sysclk_hz / ahb_div;
        let pclk_hz = hclk_hz / apb_div;
        Ok(Self {
            sysclk_hz,
            hclk_hz,
            pclk_hz,
            timer_hz: pclk_hz * if apb_div == 1 { 1 } else { 2 },
        })
    }
}

/// Check the exact G0 register map and the currently supported timed USART
/// clock case (PCLK = HCLK, CCIPR selects PCLK, USART PRESC reset /1).
pub fn validate_timed_uart_clock(bus: &SystemBus, uart: &str) -> Result<G0Clocks> {
    let rcc = bus
        .find_peripheral_index_by_name("rcc")
        .ok_or_else(|| anyhow::anyhow!("G0 RCC missing"))?;
    ensure!(
        bus.peripherals[rcc].dev.stm32_uart_pad_af_override() == Some(1),
        "timing guard requires the STM32G0 RCC profile"
    );
    let clocks = G0Clocks::from_registers(
        bus.read_u32(0x40021000)?,
        bus.read_u32(0x40021008)?,
        bus.read_u32(0x4002100c)?,
        None,
    )?;
    ensure!(
        clocks.hclk_hz == bus.cpu_hz,
        "G0 programmed HCLK={}Hz disagrees with simulation timebase={}Hz",
        clocks.hclk_hz,
        bus.cpu_hz
    );
    ensure!(
        clocks.pclk_hz == clocks.hclk_hz,
        "timed G0 USART currently requires APB prescaler /1"
    );
    let shift = match uart {
        "usart1" => 0,
        "usart2" => 2,
        _ => bail!("timed G0 support covers USART1 and USART2 only"),
    };
    ensure!(
        (bus.read_u32(0x40021054)? >> shift) & 3 == 0,
        "timed G0 USART requires CCIPR kernel source PCLK"
    );
    let base = if uart == "usart1" {
        0x40013800
    } else {
        0x40004400
    };
    ensure!(
        bus.read_u32(base + 0x2c)? & 15 == 0,
        "timed G0 USART PRESC must remain /1"
    );
    Ok(clocks)
}
