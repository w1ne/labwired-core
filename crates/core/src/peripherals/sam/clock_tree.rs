// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! SAMD51 nominal clock-tree resolution from live GCLK/OSCCTRL/MCLK registers
//! (DS60001507F 14, 15, 28). Frequencies are nominal: no oscillator tolerance,
//! lock time or jitter. A source that is not running, a disabled generator or a
//! disabled peripheral channel resolves to `None`, never to a guessed value.
use crate::Bus;

const GCLK: u64 = 0x40001c00;
const OSCCTRL: u64 = 0x40001000;
const MCLK_CPUDIV: u64 = 0x40000805;
const DFLL_HZ: u64 = 48_000_000;
const OSCULP32K_HZ: u64 = 32_768;

/// Output frequency of generic clock generator `generator`.
pub fn generator_hz(bus: &dyn Bus, generator: u64) -> Option<u64> {
    generator_hz_depth(bus, generator, 0)
}

fn generator_hz_depth(bus: &dyn Bus, generator: u64, depth: u32) -> Option<u64> {
    if generator > 11 || depth > 4 {
        return None;
    }
    let genctrl = bus.read_u32(GCLK + 0x20 + 4 * generator).ok()?;
    if genctrl & (1 << 8) == 0 {
        return None; // GENEN
    }
    let source = match genctrl & 0xf {
        4 => OSCULP32K_HZ,
        6 => {
            let dfll = bus.read_u8(OSCCTRL + 0x1c).ok()?;
            if dfll & 2 == 0 {
                return None;
            }
            DFLL_HZ
        }
        n @ (7 | 8) => dpll_hz(bus, n as u64 - 7, depth)?,
        _ => return None, // crystals, GCLK_IN and generator chaining are not modelled
    };
    let div = u64::from(genctrl >> 16);
    let divisor = if genctrl & (1 << 12) != 0 {
        1u64 << (div.min(31) + 1)
    } else {
        div.max(1)
    };
    Some(source / divisor)
}

fn dpll_hz(bus: &dyn Bus, index: u64, depth: u32) -> Option<u64> {
    let base = OSCCTRL + 0x30 + 0x14 * index;
    if bus.read_u32(base + 0x10).ok()? & 1 == 0 {
        return None; // not locked
    }
    let ratio = bus.read_u32(base + 0x4).ok()?;
    let ctrlb = bus.read_u32(base + 0x8).ok()?;
    if (ctrlb >> 5) & 7 != 0 {
        return None; // only the GCLK reference is modelled
    }
    let reference = peripheral_hz_depth(bus, 1 + index, depth + 1)?;
    let ldr = u64::from(ratio & 0x1fff);
    let frac = u64::from((ratio >> 16) & 0x1f);
    Some(reference * ((ldr + 1) * 32 + frac) / 32)
}

/// Frequency delivered to peripheral channel `channel` (GCLK PCHCTRL index).
pub fn peripheral_hz(bus: &dyn Bus, channel: u64) -> Option<u64> {
    peripheral_hz_depth(bus, channel, 0)
}

fn peripheral_hz_depth(bus: &dyn Bus, channel: u64, depth: u32) -> Option<u64> {
    if channel > 47 {
        return None;
    }
    let pchctrl = bus.read_u32(GCLK + 0x80 + 4 * channel).ok()?;
    if pchctrl & (1 << 6) == 0 {
        return None; // CHEN
    }
    generator_hz_depth(bus, u64::from(pchctrl & 0xf), depth + 1)
}

/// CPU clock: generator 0 divided by MCLK.CPUDIV.
pub fn cpu_hz(bus: &dyn Bus) -> Option<u64> {
    let main = generator_hz(bus, 0)?;
    let cpudiv = u64::from(bus.read_u8(MCLK_CPUDIV).unwrap_or(1)).max(1);
    Some(main / cpudiv)
}
