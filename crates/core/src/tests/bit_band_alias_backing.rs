// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The Cortex-M bit-band alias decode must not swallow a vendor peripheral.
//!
//! ARMv7-M reserves the peripheral alias window 0x4200_0000–0x43FF_FFFF for
//! bit-banding 0x4000_0000–0x400F_FFFF, but the vendor memory map wins inside
//! it. The ATSAMD51 decodes real peripherals there (SERCOM3 @ 0x4200_1000,
//! QSPI @ 0x4200_3400; DS60001507 §7.2), so an unconditional alias decode
//! rewrote QSPI's base into the unmapped physical byte 0x4000_01A0 and every
//! access answered `MemoryViolation` — which is how the SAMD51 register-
//! compliance and chip-conformance gates went red on main.
//!
//! Fix: translate only when the aliased byte is backed by RAM, flash, an
//! `extra_mem` window or a peripheral register. These tests pin both halves:
//! the SAMD51-shaped window is ordinary MMIO, and a genuine bit-band access
//! still sets the physical bit.

#[cfg(test)]
mod tests {
    use crate::bus::SystemBus;
    use crate::peripherals::stub::StubPeripheral;
    use crate::Bus;

    /// A peripheral window inside the architectural alias range answers as a
    /// peripheral, not as an alias of an unmapped physical byte.
    #[test]
    fn peripheral_in_the_alias_window_is_read_and_written_directly() {
        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "qspi",
            0x4200_3400,
            0x400,
            None,
            Box::new(StubPeripheral::new(0)),
        );

        assert_eq!(bus.read_u32(0x4200_3400).unwrap(), 0);
        assert_eq!(bus.read_u8(0x4200_3400).unwrap(), 0);
        assert!(bus.write_u32(0x4200_3400, 0).is_ok());
        assert!(bus.write_u8(0x4200_3400, 0).is_ok());
    }

    /// SRAM bit-banding still works: an alias write sets exactly one bit of
    /// the physical byte, and the alias reads back that bit.
    #[test]
    fn sram_bit_band_still_sets_the_physical_bit() {
        let mut bus = SystemBus::new();
        let phys_byte = 0x2000_0100u64;
        let bit = 3u8;
        let alias = 0x2200_0000 + (phys_byte - 0x2000_0000) * 32 + u64::from(bit) * 4;

        bus.write_u32(alias, 1).unwrap();
        assert_eq!(bus.read_u8(phys_byte).unwrap(), 1 << bit);
        assert_eq!(bus.read_u32(alias).unwrap(), 1);

        bus.write_u32(alias, 0).unwrap();
        assert_eq!(bus.read_u8(phys_byte).unwrap(), 0);
        assert_eq!(bus.read_u32(alias).unwrap(), 0);
    }

    /// A peripheral bit-band alias still translates when its target register is
    /// backed: the alias word of bit 5 at physical 0x4000_8014 reads the bit.
    #[test]
    fn peripheral_bit_band_still_translates_when_target_is_mapped() {
        let mut bus = SystemBus::new();
        let mut stub = StubPeripheral::new(0);
        stub.values.insert(0x14, 1 << 5);
        bus.add_peripheral("aliased_uart", 0x4000_8000, 0x400, None, Box::new(stub));

        let bit = 5u8;
        let alias = 0x4200_0000 + 0x8000 * 32 + 0x14 * 32 + u64::from(bit) * 4;
        assert_eq!(bus.read_u32(alias).unwrap(), 1);
    }
}
