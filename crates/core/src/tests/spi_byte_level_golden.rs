// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Regression gate for the DEFAULT (byte-level) STM32 SPI slave path.
//!
//! The bit-level edge-sampling work (`SpiSampling::Edge`) is opt-in. Nothing
//! about a device that does NOT opt in may change: not the wire waveform, not
//! the byte a frame delivers into DR, not the order the slave is consulted in.
//!
//! The expectations below are a RECORDING, not a mirror of the implementation:
//! they were captured by running `capture()` against the tree as it stood
//! BEFORE edge sampling existed (see `print_golden`, `--ignored`), and pasted
//! here verbatim. If the default path ever drifts, this test fails with a hex
//! diff of the wire itself.

#[cfg(test)]
mod spi_byte_level_golden_tests {
    use crate::peripherals::spi::{Spi, SpiDevice, SpiRegisterLayout};
    use crate::Peripheral;

    /// Deterministic full-duplex slave: answers a pure function of the byte it
    /// is handed plus its own call count, so the recording pins the ORDER the
    /// engine consults the device in as well as the values.
    struct Responder {
        calls: u8,
    }
    impl SpiDevice for Responder {
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.calls = self.calls.wrapping_add(1);
            mosi.rotate_left(3) ^ 0x5A ^ self.calls
        }
        fn cs_pin(&self) -> &str {
            "PA4"
        }
    }

    /// One configuration to record: CR1 mode bits + the frame bytes to clock.
    struct Case {
        /// CR1 BR field (SCK half-period = 2^BR peripheral-clock cycles).
        br: u16,
        cpol: bool,
        cpha: bool,
        lsb_first: bool,
        /// FIFO (L4/F7/G4) register layout instead of classic F1/F4.
        fifo: bool,
        /// CR1.DFF (classic 16-bit frames).
        dff: bool,
        bytes: &'static [u16],
    }

    const CASES: &[Case] = &[
        // Mode 0, MSB first, BR=1 — the nokia5110/max31855 lab shape.
        Case {
            br: 1,
            cpol: false,
            cpha: false,
            lsb_first: false,
            fifo: false,
            dff: false,
            bytes: &[0xA5, 0x3C, 0x00],
        },
        // Mode 1 (CPHA=1) — different leading edge.
        Case {
            br: 0,
            cpol: false,
            cpha: true,
            lsb_first: false,
            fifo: false,
            dff: false,
            bytes: &[0xF0, 0x0F],
        },
        // Mode 2 (CPOL=1, CPHA=0).
        Case {
            br: 2,
            cpol: true,
            cpha: false,
            lsb_first: false,
            fifo: false,
            dff: false,
            bytes: &[0x81],
        },
        // Mode 3 + LSB first.
        Case {
            br: 0,
            cpol: true,
            cpha: true,
            lsb_first: true,
            fifo: false,
            dff: false,
            bytes: &[0xB4, 0x77],
        },
        // Classic 16-bit frames (CR1.DFF).
        Case {
            br: 0,
            cpol: false,
            cpha: false,
            lsb_first: false,
            fifo: false,
            dff: true,
            bytes: &[0xBEEF],
        },
        // FIFO layout, 8-bit frames from the CR2 reset (DS=0b0111).
        Case {
            br: 1,
            cpol: false,
            cpha: false,
            lsb_first: false,
            fifo: true,
            dff: false,
            bytes: &[0x12, 0x34],
        },
    ];

    /// Clock every case one peripheral-clock cycle at a time, recording the
    /// (SCK, MOSI, MISO) wire triple at every tick and the DR value each frame
    /// leaves behind.
    ///
    /// Returns `(wire_hex, dr_bytes)`.
    fn capture() -> (String, Vec<u16>) {
        let mut wire: Vec<u8> = Vec::new();
        let mut drs: Vec<u16> = Vec::new();
        for case in CASES {
            let layout = if case.fifo {
                SpiRegisterLayout::Stm32Fifo
            } else {
                SpiRegisterLayout::Stm32
            };
            let mut spi = Spi::new_with_layout(layout);
            let lines = spi.line_levels_arc();
            spi.push_device(Box::new(Responder { calls: 0 }));
            let cr1 = (1 << 6) // SPE
                | (case.br << 3)
                | (u16::from(case.cpol) << 1)
                | u16::from(case.cpha)
                | (u16::from(case.lsb_first) << 7)
                | (u16::from(case.dff) << 11);
            spi.write_u16(0x00, cr1).unwrap();
            // Idle wire, recorded too (CPOL must be on SCK before any frame).
            wire.push(triple(lines.sck(), lines.mosi(), lines.miso()));
            for &b in case.bytes {
                spi.write_u16(0x0C, b).unwrap();
                wire.push(triple(lines.sck(), lines.mosi(), lines.miso()));
                for _ in 0..4096 {
                    if !spi.transfer_active() {
                        break;
                    }
                    spi.tick_elapsed(1);
                    wire.push(triple(lines.sck(), lines.mosi(), lines.miso()));
                }
                assert!(!spi.transfer_active(), "frame never completed");
                drs.push(spi.read_u16(0x0C).unwrap());
                // SR too: BSY/TXE/RXNE placement is part of the contract.
                drs.push(spi.read_u16(0x08).unwrap());
            }
        }
        let hex = wire.iter().map(|b| format!("{b:x}")).collect::<String>();
        (hex, drs)
    }

    fn triple(sck: bool, mosi: bool, miso: bool) -> u8 {
        (u8::from(sck) << 2) | (u8::from(mosi) << 1) | u8::from(miso)
    }

    /// Recording generator. Run with:
    ///   cargo test -p labwired-core --lib spi_byte_level_golden -- --ignored --nocapture
    #[test]
    #[ignore]
    fn print_golden() {
        let (hex, drs) = capture();
        println!("WIRE = \"{hex}\";");
        println!("DR = {drs:?};");
    }

    /// Captured on pristine HEAD 0877f536 (2026-08-08), BEFORE any
    /// edge-sampling code existed in this branch — see the commit that added
    /// this file, which contains no implementation change at all.
    const WIRE: &str = "02266115533771155004433771155226621155004433773377337722660044115510044115500441155115500440044115510737362735151404004040514062626262246666222255551111444400005555111144440000555511115555111177773333740415371537371537737372604263737155026042626262626042626370426372626201155115500442266115500443377115500441155004411551155004400440044011551155337733771155226600440044004411550044115511551155115500440";
    const DR: &[u16] = &[
        118, 3, 185, 3, 89, 3, 220, 3, 32, 3, 87, 3, 254, 3, 227, 3, 36, 3, 88, 3, 94, 3,
    ];

    #[test]
    fn default_byte_level_path_is_byte_identical() {
        let (hex, drs) = capture();
        assert_eq!(hex, WIRE, "default-path SPI waveform drifted");
        assert_eq!(drs, DR, "default-path DR/SR results drifted");
    }
}
