// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The SAMD51 board models are reached through the production chip
//! descriptors (`configs/chips/atsamd51*.yaml`), not only through hand-built
//! buses: each descriptor builds, names the models, binds the TC waveform
//! outputs to their PMUX-E pads, and maps the APB-C/D peripherals in the
//! would-be bit-band alias window directly (the SAMD51 has no bit-band).

#[cfg(test)]
mod samd51_board_models_tests {
    use crate::bus::SystemBus;
    use crate::Bus;
    use labwired_config::{ChipDescriptor, SystemManifest};

    fn chip(file: &str) -> (ChipDescriptor, SystemManifest) {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("configs/chips")
            .join(file);
        let chip = ChipDescriptor::from_file(&path).expect("load chip yaml");
        let manifest = SystemManifest {
            parts: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "samd51-board-models".to_string(),
            chip: path.to_string_lossy().to_string(),
            cpu_hz: None,
            external_devices: vec![],
            cosim_models: Vec::new(),
            motor_models: Vec::new(),
            board_io: vec![],
            debug_uart: None,
            wifi_ap: None,
            peripherals: vec![],
            memory_overrides: Default::default(),
        };
        (chip, manifest)
    }

    const SAMD51_CHIPS: [&str; 2] = ["atsamd51.yaml", "atsamd51-pybadge.yaml"];

    #[test]
    fn samd51_descriptors_wire_the_board_models() {
        for file in SAMD51_CHIPS {
            let (chip, manifest) = chip(file);
            for id in [
                "pac", "oscctrl", "gclk", "eic", "tc0", "tc4", "tcc0", "adc0", "dac",
            ] {
                let p = chip
                    .peripherals
                    .iter()
                    .find(|p| p.id == id)
                    .unwrap_or_else(|| panic!("{file} declares {id}"));
                assert!(
                    p.r#type.starts_with("samd51_"),
                    "{file}: {id} uses a SAMD51 board model, not {}",
                    p.r#type
                );
            }
            let bus = SystemBus::from_config(&chip, &manifest)
                .unwrap_or_else(|e| panic!("{file} builds: {e:#}"));
            for id in ["eic", "tc5", "tcc4", "adc1"] {
                assert!(
                    bus.find_peripheral_index_by_name(id).is_some(),
                    "{file}: {id}"
                );
            }
            assert!(
                !bus.bit_band_enabled,
                "{file}: the SAMD51 does not implement bit-banding"
            );
        }
    }

    #[test]
    fn samd51_tc_waveform_outputs_are_bound_to_their_pads() {
        for file in SAMD51_CHIPS {
            let (chip, manifest) = chip(file);
            let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
            let bound = bus.bound_pad_functions();
            for want in ["TC0_WO0", "TC3_WO1", "TC4_WO0", "TC5_WO1"] {
                assert!(bound.contains(&want), "{file}: {want} bound; got {bound:?}");
            }
        }
    }

    #[test]
    fn apb_c_peripherals_in_the_alias_window_are_reached_directly() {
        let (chip, manifest) = chip("atsamd51-pybadge.yaml");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
        // TC4 CC0 (COUNT16) at 0x4200_141C: a bit-band translation would send
        // this to the PAC at 0x4000_00A0 instead.
        bus.write_u16(0x4200_141C, 0x1234).unwrap();
        assert_eq!(bus.read_u16(0x4200_141C).unwrap(), 0x1234);
    }

    #[test]
    fn other_cortex_m3_m4_parts_keep_bit_banding() {
        let (chip, manifest) = chip("stm32f103.yaml");
        let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
        assert!(bus.bit_band_enabled, "the M3 STM32F103 keeps bit-banding");
    }
}
