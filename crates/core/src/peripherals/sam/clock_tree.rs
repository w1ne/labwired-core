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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bus, DmaRequest, SimulationConfig};
    use std::collections::HashMap;

    struct FlatRam {
        mem: HashMap<u64, u8>,
        config: SimulationConfig,
    }

    impl FlatRam {
        fn new() -> Self {
            Self {
                mem: HashMap::new(),
                config: SimulationConfig::default(),
            }
        }
        fn poke_u32(&mut self, addr: u64, value: u32) {
            self.write_u32(addr, value).unwrap();
        }
        fn poke_u8(&mut self, addr: u64, value: u8) {
            self.write_u8(addr, value).unwrap();
        }
        /// GENCTRL: SRC | GENEN | optional DIVSEL | DIV in the upper half.
        fn enable_gen(&mut self, generator: u64, src: u32, div: u32, divsel: bool) {
            let mut genctrl = src | (1 << 8) | (div << 16);
            if divsel {
                genctrl |= 1 << 12;
            }
            self.poke_u32(GCLK + 0x20 + 4 * generator, genctrl);
        }
        fn enable_pch(&mut self, channel: u64, generator: u32) {
            self.poke_u32(GCLK + 0x80 + 4 * channel, generator | (1 << 6));
        }
        fn enable_dfll(&mut self) {
            self.poke_u8(OSCCTRL + 0x1c, 0x82);
        }
    }

    impl Bus for FlatRam {
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

    #[test]
    fn dfll_source_and_integer_div_reach_a_peripheral_channel() {
        let mut bus = FlatRam::new();
        bus.enable_dfll();
        // GEN1: DFLL48M / 4 → 12 MHz; PCHCTRL[9] (TC0/1) selects GEN1.
        bus.enable_gen(1, 6, 4, false);
        bus.enable_pch(9, 1);
        let hz = peripheral_hz(&bus, 9).expect("enabled DFLL channel");
        assert!(hz > 0, "resolved a positive frequency");
        let undivided = {
            let mut b = FlatRam::new();
            b.enable_dfll();
            b.enable_gen(1, 6, 1, false);
            b.enable_pch(9, 1);
            peripheral_hz(&b, 9).unwrap()
        };
        assert_eq!(
            undivided / hz,
            4,
            "integer DIV scales the DFLL into the peripheral channel"
        );
    }

    #[test]
    fn divsel_power_of_two_scales_the_generator() {
        let mut bus = FlatRam::new();
        bus.enable_dfll();
        // DIVSEL with DIV=1 → divide by 2^(1+1) = 4.
        bus.enable_gen(0, 6, 1, true);
        let hz = generator_hz(&bus, 0).expect("DIVSEL generator");
        let base = {
            let mut b = FlatRam::new();
            b.enable_dfll();
            b.enable_gen(0, 6, 1, false);
            generator_hz(&b, 0).unwrap()
        };
        assert_eq!(base / hz, 4, "DIVSEL uses 2^(DIV+1)");
    }

    #[test]
    fn mclk_cpudiv_gates_cpu_hz() {
        let mut bus = FlatRam::new();
        bus.enable_dfll();
        bus.enable_gen(0, 6, 1, false);
        bus.poke_u8(MCLK_CPUDIV, 1);
        let full = cpu_hz(&bus).expect("CPU with CPUDIV=1");
        bus.poke_u8(MCLK_CPUDIV, 2);
        let halved = cpu_hz(&bus).expect("CPU with CPUDIV=2");
        assert_eq!(full / halved, 2, "MCLK.CPUDIV halves f_CPU");
        assert_eq!(
            cpu_hz(&bus),
            generator_hz(&bus, 0).map(|g| g / 2),
            "cpu_hz is generator 0 divided by the MCLK mask/div"
        );
    }

    #[test]
    fn disabled_generator_or_channel_resolves_to_none() {
        let mut bus = FlatRam::new();
        bus.enable_dfll();
        // GENEN clear: generator stays None even with a DFLL source field.
        bus.poke_u32(GCLK + 0x20, 6); // SRC=DFLL, GENEN=0
        assert_eq!(generator_hz(&bus, 0), None);

        bus.enable_gen(0, 6, 1, false);
        // CHEN clear: channel stays None even when its generator is live.
        bus.poke_u32(GCLK + 0x80 + 4 * 9, 0); // GEN0, CHEN=0
        assert_eq!(peripheral_hz(&bus, 9), None);

        bus.enable_pch(9, 0);
        assert!(peripheral_hz(&bus, 9).is_some());

        // Out-of-range indices never guess.
        assert_eq!(generator_hz(&bus, 12), None);
        assert_eq!(peripheral_hz(&bus, 48), None);
    }

    #[test]
    fn osculp32k_source_and_missing_dfll_do_not_guess() {
        let mut bus = FlatRam::new();
        bus.enable_gen(2, 4, 1, false); // OSCULP32K
        let hz = generator_hz(&bus, 2).expect("ULP32K generator");
        assert!(hz > 0 && hz < 100_000, "ULP32K is a low-kHz source");

        // DFLL selected but ENABLE clear → None (never a guessed 48 MHz).
        bus.poke_u8(OSCCTRL + 0x1c, 0); // DFLL disabled
        bus.enable_gen(3, 6, 1, false);
        assert_eq!(generator_hz(&bus, 3), None);
    }
}
