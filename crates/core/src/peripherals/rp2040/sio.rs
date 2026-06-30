// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 single-cycle IO block — SIO GPIO (datasheet §2.3.1, base
//! `0xD0000000`).
//!
//! SIO sits on the Cortex-M0+ single-cycle IO port (address `0xD0000000`),
//! *outside* the `0x40000000..0x50400000` APB/AHB peripheral window, so the
//! RP2040 atomic SET/CLR/XOR register aliases (`+0x2000` / `+0x3000` /
//! `+0x1000`) do **not** apply here. Instead SIO exposes dedicated
//! set / clear / xor registers at fixed offsets (`GPIO_OUT_SET` etc.), which
//! this model implements directly.
//!
//! Modelled behaviour: a 30-bit `GPIO_OUT` output latch and a `GPIO_OE` output
//! enable, each driven by direct / set / clear / xor registers. `GPIO_IN`
//! reads back the level a pin is *driving*: `GPIO_OUT & GPIO_OE`. With no
//! external wiring in the chip model an output pin reads back its own driven
//! level (a real, observable set-drive-readback round-trip) and an input
//! (OE=0) pin floats to 0. `CPUID` reads 0 (core 0).

use crate::{Peripheral, SimResult};

// SIO register offsets (datasheet §2.3.1.7).
const CPUID: u64 = 0x000;
const GPIO_IN: u64 = 0x004;
const GPIO_HI_IN: u64 = 0x008;
const GPIO_OUT: u64 = 0x010;
const GPIO_OUT_SET: u64 = 0x014;
const GPIO_OUT_CLR: u64 = 0x018;
const GPIO_OUT_XOR: u64 = 0x01c;
const GPIO_OE: u64 = 0x020;
const GPIO_OE_SET: u64 = 0x024;
const GPIO_OE_CLR: u64 = 0x028;
const GPIO_OE_XOR: u64 = 0x02c;

// The RP2040 exposes 30 GPIOs (0..29) on bank 0.
const GPIO_MASK: u32 = 0x3fff_ffff;

#[derive(Debug, Default)]
pub struct Rp2040Sio {
    gpio_out: u32,
    gpio_oe: u32,
}

impl Rp2040Sio {
    pub fn new() -> Self {
        Self::default()
    }

    /// Level each pin is driving onto the (unwired) pads: a pin reads back its
    /// own output when its output-enable is set, otherwise it floats to 0.
    fn gpio_in(&self) -> u32 {
        self.gpio_out & self.gpio_oe
    }
}

impl Peripheral for Rp2040Sio {
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let val = match offset {
            CPUID => 0, // single core context: always core 0
            GPIO_IN => self.gpio_in(),
            GPIO_HI_IN => 0, // QSPI bank pins — not modelled
            GPIO_OUT | GPIO_OUT_SET | GPIO_OUT_CLR | GPIO_OUT_XOR => self.gpio_out,
            GPIO_OE | GPIO_OE_SET | GPIO_OE_CLR | GPIO_OE_XOR => self.gpio_oe,
            _ => 0,
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let v = value & GPIO_MASK;
        match offset {
            GPIO_OUT => self.gpio_out = v,
            GPIO_OUT_SET => self.gpio_out |= v,
            GPIO_OUT_CLR => self.gpio_out &= !v,
            GPIO_OUT_XOR => self.gpio_out ^= v,
            GPIO_OE => self.gpio_oe = v,
            GPIO_OE_SET => self.gpio_oe |= v,
            GPIO_OE_CLR => self.gpio_oe &= !v,
            GPIO_OE_XOR => self.gpio_oe ^= v,
            _ => {}
        }
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !0x3)?;
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !0x3;
        let shift = (offset & 0x3) * 8;
        let cur = self.read_u32(aligned)?;
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIN25: u32 = 1 << 25;

    #[test]
    fn cpuid_reads_zero() {
        assert_eq!(Rp2040Sio::new().read_u32(CPUID).unwrap(), 0);
    }

    #[test]
    fn set_drive_readback_roundtrip() {
        let mut sio = Rp2040Sio::new();
        // Output disabled → driven level not visible on GPIO_IN.
        sio.write_u32(GPIO_OUT_SET, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, 0);
        // Enable output → pin reads back its driven high level.
        sio.write_u32(GPIO_OE_SET, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, PIN25);
        assert_eq!(sio.read_u32(GPIO_OUT).unwrap() & PIN25, PIN25);
        // Clear the output → reads back low.
        sio.write_u32(GPIO_OUT_CLR, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, 0);
    }

    #[test]
    fn xor_toggles_output() {
        let mut sio = Rp2040Sio::new();
        sio.write_u32(GPIO_OE_SET, PIN25).unwrap();
        sio.write_u32(GPIO_OUT_XOR, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, PIN25);
        sio.write_u32(GPIO_OUT_XOR, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, 0);
    }
}
