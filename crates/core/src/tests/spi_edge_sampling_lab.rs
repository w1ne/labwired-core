// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end proof of the opt-in edge-sampling switch on a REAL part, through
//! the production path: `system.yaml` → kit attach → traced device wrapper →
//! STM32 bit engine.
//!
//! The lab is the shipped max31855-thermocouple-lab wiring (MAX31855 on SPI1 of
//! an F103, CS = PA4). The MAX31855 is a mode-0/mode-1-tolerant read-only part
//! on silicon; here it is STRAPPED by the manifest (`config: { spi_mode: N }`)
//! so a controller programmed for a different CPOL/CPHA is a genuine mismatch.
//!
//! Three arms, and all three matter:
//!   * no `spi_mode`  → the byte-level default: clean 32-bit frame (unchanged).
//!   * `spi_mode: 0`  → edge-accurate, master also mode 0: byte-identical.
//!   * `spi_mode: 1`  → edge-accurate, master mode 0: the master latches on the
//!     same edge the slave changes MISO on, so every byte arrives shifted one
//!     bit and the decoded temperature is wrong.

#[cfg(test)]
mod spi_edge_sampling_lab_tests {
    use crate::bus::SystemBus;
    use crate::peripherals::spi::Spi;
    use crate::Peripheral;
    use labwired_config::{
        Arch, ChipDescriptor, ExternalDevice, MemoryRange, PeripheralConfig, SystemManifest,
    };
    use std::collections::HashMap;

    const SPI1_BASE: u64 = 0x4001_3000;

    fn chip() -> ChipDescriptor {
        ChipDescriptor {
            schema_version: "1.0".to_string(),
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            memory_regions: Vec::new(),
            name: "stm32f103-test".to_string(),
            arch: Arch::Arm,
            core: None,
            flash: MemoryRange {
                base: 0x0800_0000,
                size: "64KB".to_string(),
            },
            ram: MemoryRange {
                base: 0x2000_0000,
                size: "20KB".to_string(),
            },
            peripherals: vec![PeripheralConfig {
                id: "spi1".to_string(),
                r#type: "spi".to_string(),
                base_address: SPI1_BASE,
                size: Some("1KB".to_string()),
                irq: Some(35),
                clock: None,
                config: HashMap::new(),
            }],
            pins: Default::default(),
        }
    }

    /// The lab manifest, optionally straps the part for edge sampling.
    fn manifest(spi_mode: Option<i64>) -> SystemManifest {
        let mut config = HashMap::new();
        config.insert(
            "cs_pin".to_string(),
            serde_yaml::Value::String("PA4".to_string()),
        );
        if let Some(mode) = spi_mode {
            config.insert(
                "spi_mode".to_string(),
                serde_yaml::Value::Number(mode.into()),
            );
        }
        SystemManifest {
            parts: Vec::new(),
            cosim_models: Vec::new(),
            motor_models: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "max31855-edge-sampling".to_string(),
            chip: "../chips/stm32f103.yaml".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: vec![ExternalDevice {
                id: "tc1".to_string(),
                r#type: "max31855".to_string(),
                connection: "spi1".to_string(),
                channel: None,
                route: Default::default(),
                config,
            }],
            board_io: Vec::new(),
            debug_uart: None,
            wifi_ap: None,
            peripherals: Vec::new(),
        }
    }

    /// Clock four dummy bytes out of SPI1 in `master_mode` and return what the
    /// firmware would have read back — the MAX31855's 32-bit frame.
    fn read_frame(spi_mode: Option<i64>, master_mode: u8) -> [u8; 4] {
        let chip = chip();
        let mut bus = SystemBus::from_config(&chip, &manifest(spi_mode)).unwrap();
        let idx = bus.find_peripheral_index_by_name("spi1").unwrap();
        let spi = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Spi>()
            .unwrap();
        let cpol = u16::from(master_mode & 0b10 != 0);
        let cpha = u16::from(master_mode & 0b01 != 0);
        // SPE | MSTR | BR=/4 | CPOL | CPHA — 8-bit frames, MSB first.
        spi.write_u16(0x00, (1 << 6) | (1 << 2) | (1 << 3) | (cpol << 1) | cpha)
            .unwrap();
        let mut out = [0u8; 4];
        for byte in out.iter_mut() {
            spi.write(0x0C, 0x00).unwrap();
            for _ in 0..4096 {
                if !spi.transfer_active() {
                    break;
                }
                spi.tick_elapsed(1);
            }
            assert!(!spi.transfer_active(), "frame never completed");
            *byte = spi.read(0x0C).unwrap();
        }
        out
    }

    /// MAX31855 frame decode (datasheet): bits [31:18] = thermocouple °C at
    /// 0.25 °C/LSB.
    fn thermocouple_c(frame: [u8; 4]) -> f64 {
        let word = u32::from_be_bytes(frame);
        let raw = ((word >> 18) & 0x3FFF) as i32;
        let signed = if raw & 0x2000 != 0 { raw - 0x4000 } else { raw };
        f64::from(signed) / 4.0
    }

    /// The part's defaults: 25.0 °C hot junction, 22.0 °C cold junction
    /// (`configs/devices/max31855.yaml`), i.e. word 0x01901600.
    const CLEAN: [u8; 4] = [0x01, 0x90, 0x16, 0x00];

    #[test]
    fn byte_level_default_reads_the_clean_frame() {
        let frame = read_frame(None, 0);
        assert_eq!(frame, CLEAN, "default path must be untouched");
        assert_eq!(thermocouple_c(frame), 25.0);
    }

    #[test]
    fn edge_sampling_with_matching_modes_is_byte_identical() {
        assert_eq!(
            read_frame(Some(0), 0),
            CLEAN,
            "an edge-sampled slave in the master's own mode must read the same"
        );
    }

    /// The point of the feature. Master mode 0, slave strapped mode 1: the
    /// slave presents each MISO bit on the rising edge the master latches on,
    /// so byte k arrives as `(last bit of byte k-1) << 7 | byte k >> 1`.
    #[test]
    fn edge_sampling_with_a_mode_mismatch_corrupts_the_reading() {
        let frame = read_frame(Some(1), 0);
        assert_eq!(
            frame,
            [0x00, 0xC8, 0x0B, 0x00],
            "every byte shifted one bit late"
        );
        assert_ne!(frame, CLEAN);
        assert_eq!(
            thermocouple_c(frame),
            12.5,
            "the firmware reads a plausible-looking WRONG temperature"
        );
    }

    /// Same mismatch, no opt-in: proves the corruption is the switch and not
    /// the wiring. (Vacuity guard for the test above.)
    #[test]
    fn the_same_mismatch_without_the_opt_in_reads_clean() {
        assert_eq!(read_frame(None, 1), CLEAN);
        assert_eq!(read_frame(None, 3), CLEAN);
    }

    /// A controller with no bit-level engine must REFUSE an edge-sampled
    /// device at config time, naming itself, rather than accepting it and
    /// quietly exchanging bytes — the failure mode where a lab author watches
    /// a mode-mismatch lesson not reproduce and has nothing to read.
    #[test]
    fn a_byte_level_controller_refuses_an_edge_sampled_device() {
        use crate::peripherals::spi::{SpiDevice, SpiSampling};

        struct EdgeDev;
        impl SpiDevice for EdgeDev {
            fn sampling(&self) -> SpiSampling {
                SpiSampling::edge_mode(1)
            }
            fn transfer(&mut self, _mosi: u8) -> u8 {
                0
            }
            fn cs_pin(&self) -> &str {
                "PA4"
            }
        }
        struct ByteDev;
        impl SpiDevice for ByteDev {
            fn transfer(&mut self, _mosi: u8) -> u8 {
                0
            }
            fn cs_pin(&self) -> &str {
                "PA4"
            }
        }

        // Named so clippy::type_complexity does not fire on the tuple below:
        // a boxed factory is the only way to hold differently-typed controllers
        // in one table, and the table is the point of this guard.
        type ControllerFactory = Box<dyn Fn() -> Box<dyn crate::Peripheral>>;

        // Same Rust type as the STM32 bit engine, but the H5 "SPI v3" register
        // file completes a frame whole — the guard must see the LAYOUT.
        let cases: Vec<(&str, ControllerFactory, &str)> = vec![
            (
                "spi_h5",
                Box::new(|| {
                    Box::new(Spi::new_with_layout(
                        crate::peripherals::spi::SpiRegisterLayout::Stm32H5,
                    ))
                }),
                "STM32H5",
            ),
            (
                "spi_esp32",
                Box::new(|| Box::new(crate::peripherals::esp32::spi::Esp32Spi::new())),
                "ESP32 classic",
            ),
        ];
        for (name, make, expect) in cases {
            let mut bus = SystemBus::new();
            bus.add_peripheral(name, 0x4000_0000, 0x400, None, make());
            // The byte-level device attaches happily...
            bus.attach_spi_device(name, Box::new(ByteDev))
                .unwrap_or_else(|e| panic!("{name}: byte-level attach must work: {e:#}"));
            // ...the edge-sampled one is refused, by name.
            let err = match bus.attach_spi_device(name, Box::new(EdgeDev)) {
                Ok(()) => panic!("{name} must refuse an edge-sampled device"),
                Err(e) => format!("{e:#}"),
            };
            assert!(
                err.contains(expect) && err.contains("spi_mode"),
                "{name}: unhelpful refusal: {err}"
            );
        }
    }

    /// The SAME part, the SAME mismatch, on a DIFFERENT controller — through a
    /// real manifest, so the kit → attach → controller chain is exercised, not
    /// just the engine. The C3 GP-SPI reaches the same bytes as the STM32 bit
    /// engine because both call one edge model; if they ever fork, this test
    /// and `edge_sampling_with_a_mode_mismatch_corrupts_the_reading` disagree.
    #[test]
    fn the_esp32c3_controller_reaches_the_same_corruption() {
        use labwired_config::ChipDescriptor;

        fn c3_frame(spi_mode: Option<&str>) -> [u8; 4] {
            let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
                .expect("load esp32c3.yaml");
            let manifest: labwired_config::SystemManifest = serde_yaml::from_str(&format!(
                r#"
name: c3-max31855-edge
chip: "../chips/esp32c3.yaml"
cpu_hz: 160_000_000
external_devices:
  - id: "tc1"
    type: "max31855"
    connection: "spi2"
    config:
      cs_pin: "GPIO7"
{}
"#,
                spi_mode
                    .map(|m| format!("      spi_mode: {m}"))
                    .unwrap_or_default()
            ))
            .expect("parse manifest");
            let mut bus = SystemBus::from_config(&chip, &manifest).expect("build C3 bus");
            let idx = bus.find_peripheral_index_by_name("spi2").unwrap();
            let spi = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<crate::peripherals::esp32c3::spi::Esp32c3Spi>()
                .unwrap();
            // Reset MISC/USER already mean mode 0; clock four 8-bit frames
            // through the W buffer the way the IDF driver does.
            let mut out = [0u8; 4];
            for byte in out.iter_mut() {
                spi.write_u32(0x98, 0).unwrap(); // W0 = MOSI 0x00
                spi.write_u32(0x1C, 8 - 1).unwrap(); // MS_DLEN
                spi.write_u32(0x00, 1 << 24).unwrap(); // CMD.USR
                *byte = (spi.read_u32(0x98).unwrap() & 0xFF) as u8;
            }
            out
        }

        assert_eq!(c3_frame(None), CLEAN, "C3 default path must be untouched");
        assert_eq!(
            c3_frame(Some("0")),
            CLEAN,
            "C3, matching modes: byte-identical"
        );
        assert_eq!(
            c3_frame(Some("1")),
            [0x00, 0xC8, 0x0B, 0x00],
            "C3 mismatch must corrupt exactly as the STM32 engine does"
        );
    }

    /// A typo in the manifest must fail the build, not silently pick a mode.
    #[test]
    fn an_out_of_range_spi_mode_is_rejected() {
        let err = match SystemBus::from_config(&chip(), &manifest(Some(7))) {
            Ok(_) => panic!("spi_mode 7 must be rejected"),
            Err(e) => e,
        };
        assert!(
            format!("{err:#}").contains("spi_mode"),
            "unhelpful error: {err:#}"
        );
    }
}
