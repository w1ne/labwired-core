// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Config-build gates for the native board descriptors: every system YAML below
//! must load its chip descriptor and build through `SystemBus::from_config`.
//!
//! Six one-test files (`atsamd21_config`, `atsamd51_config`, `ra4m1_config`,
//! `imxrt1064_config`, `stm32f746_config`, `stm32g071_config`) copied the same
//! loader skeleton — `workspace_root()` + `SystemManifest::from_file` +
//! `ChipDescriptor::from_file` + `SystemBus::from_config` — differing only in
//! the board path and the per-peripheral presence asserts. The skeleton lives
//! once, on `common::system`; the six tests keep their names and every assert
//! for CI attribution.
//!
//! Provenance carried over from the retired per-board files:
//! * `imxrt1064_config` — Teensy 4.1 silicon is MIMXRT1062; this model is the
//!   RT1064-class cousin (GPIO / LPUART / CCM class). SIM-DERIVED.
//! * `stm32f746_config` — SIM-DERIVED — F4-class RCC/GPIO/USART reuse +
//!   Cortex-M7 DTCM map. Not silicon-verified.

mod common;

use common::system;
use labwired_config::ChipDescriptor;
use labwired_core::bus::SystemBus;

/// Load a board from its system YAML: manifest + chip descriptor (with `chip`
/// re-anchored to the absolute descriptor path, as `build_system_bus` does),
/// then build the bus. `what` keeps each board's build-failure message.
fn from_config(system_rel: &str, what: &str) -> (ChipDescriptor, SystemBus) {
    let (chip, manifest) = system(system_rel);
    let bus = SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("{what}: {e}"));
    (chip, bus)
}

#[test]
fn atsamd21_from_config_builds() {
    let (_chip, bus) = from_config(
        "configs/systems/nano-33-iot.yaml",
        "atsamd21 / nano-33-iot must build",
    );
    // Nano 33 IoT Serial1 is SERCOM5 on PB22/PB23 (ArduinoCore-samd variant.cpp).
    assert!(
        bus.find_peripheral_index_by_name("sercom5").is_some(),
        "bus must expose sercom5 (Serial1), not sercom2"
    );
}

#[test]
fn atsamd51_from_config_builds() {
    let (_chip, bus) = from_config(
        "configs/systems/metro-m4.yaml",
        "atsamd51 / metro-m4 must build",
    );
    // Metro M4 Serial1 is SERCOM3 on PA22/PA23 (Adafruit variants/metro_m4/variant.cpp).
    assert!(
        bus.find_peripheral_index_by_name("sercom3").is_some(),
        "bus must expose sercom3 (Serial1)"
    );
    assert!(
        bus.find_peripheral_index_by_name("mclk").is_some(),
        "bus must expose mclk"
    );
    assert!(
        bus.find_peripheral_index_by_name("gclk").is_some(),
        "bus must expose gclk"
    );
}

#[test]
fn ra4m1_from_config_builds() {
    let (_chip, bus) = from_config(
        "configs/systems/arduino-uno-r4-minima.yaml",
        "ra4m1 / arduino-uno-r4-minima must build",
    );
    // Uno R4 Minima Serial is SCI2 on P301/P302 (ArduinoCore-renesas MINIMA).
    assert!(
        bus.find_peripheral_index_by_name("sci2").is_some(),
        "bus must expose sci2 (Serial)"
    );
    assert!(
        bus.find_peripheral_index_by_name("ra_sysc").is_some()
            || bus.find_peripheral_index_by_name("sysc").is_some(),
        "bus must expose ra_sysc / sysc (HOCO/OSCSF)"
    );
    assert!(
        bus.find_peripheral_index_by_name("port1").is_some(),
        "bus must expose port1 (LED P111)"
    );
}

#[test]
fn imxrt1064_from_config_builds() {
    let (chip, bus) = from_config(
        "configs/systems/teensy-41.yaml",
        "imxrt1064 / teensy-41 must build",
    );

    assert_eq!(chip.name, "imxrt1064");
    assert_eq!(chip.core.as_deref(), Some("cortex-m7"));
    // XIP skipped: image linked in DTCM. cortex-m-rt needs non-overlapping
    // FLASH/RAM, so lower DTCM is flash and upper DTCM is ram.
    assert_eq!(chip.flash.base, 0x2000_0000);
    assert!(
        (0x2000_0000..0x2002_0000).contains(&chip.ram.base),
        "ram must sit in DTCM @ 0x20000000..0x20020000, got {:#x}",
        chip.ram.base
    );

    assert!(
        bus.find_peripheral_index_by_name("lpuart6").is_some(),
        "bus must expose lpuart6 (Teensy Serial1)"
    );
    assert!(
        bus.find_peripheral_index_by_name("gpio2").is_some(),
        "bus must expose gpio2 (LED GPIO2_IO03)"
    );
    assert!(
        bus.find_peripheral_index_by_name("ccm").is_some(),
        "bus must expose ccm (imx_ccm)"
    );
    assert!(
        bus.find_peripheral_index_by_name("iomuxc").is_some(),
        "bus must expose iomuxc (imx_iomuxc)"
    );
    assert!(
        bus.find_peripheral_index_by_name("systick").is_some(),
        "bus must expose systick"
    );
    assert!(
        bus.find_peripheral_index_by_name("flexspi").is_some(),
        "bus must expose flexspi stub"
    );
}

#[test]
fn stm32f746_from_config_builds() {
    let (chip, bus) = from_config(
        "configs/systems/stm32f7-discovery.yaml",
        "stm32f746 / stm32f7-discovery must build",
    );

    assert_eq!(chip.core.as_deref(), Some("cortex-m7"));
    assert_eq!(chip.flash.base, 0x0800_0000);
    assert_eq!(chip.ram.base, 0x2000_0000);
    assert_eq!(chip.ram.size, 64 * 1024, "DTCM primary RAM must be 64KB");

    assert!(
        bus.find_peripheral_index_by_name("rcc").is_some(),
        "bus must expose rcc (stm32f4 @ 0x40023800)"
    );
    assert!(
        bus.find_peripheral_index_by_name("gpioi").is_some(),
        "bus must expose gpioi (Discovery LED PI1)"
    );
    assert!(
        bus.find_peripheral_index_by_name("usart1").is_some(),
        "bus must expose usart1 (ST-LINK VCP)"
    );
    assert!(
        bus.find_peripheral_index_by_name("systick").is_some(),
        "bus must expose systick"
    );
    for stub in ["ltdc", "eth", "dma2d", "usb", "quadspi"] {
        assert!(
            bus.find_peripheral_index_by_name(stub).is_some(),
            "bus must expose {stub} stub"
        );
    }
}

#[test]
fn stm32g071_from_config_builds() {
    let (chip, bus) = from_config(
        "configs/systems/nucleo-g071rb.yaml",
        "stm32g071 / nucleo-g071rb must build",
    );

    // Peripheral bases the smoke path and the board wiring depend on.
    // USART2 is the ST-LINK VCP console; GPIOA carries LD4 (PA5), GPIOC
    // carries B1 (PC13).
    for id in ["rcc", "gpioa", "gpioc", "usart2", "usart1", "systick"] {
        assert!(
            bus.find_peripheral_index_by_name(id).is_some(),
            "bus must expose {id}"
        );
    }

    // DS12232: 128 KB flash @ 0x08000000, 36 KB SRAM @ 0x20000000. The RAM
    // window is larger than the CMSIS header's SRAM_SIZE_MAX (32 KB), which
    // is the family maximum for the smaller SKUs.
    assert_eq!(chip.flash.base, 0x0800_0000, "flash base");
    assert_eq!(chip.flash.size, 128 * 1024, "flash size (DS12232)");
    assert_eq!(chip.ram.base, 0x2000_0000, "SRAM base");
    assert_eq!(chip.ram.size, 36 * 1024, "SRAM size (DS12232)");

    // Every `clock:` gate in the yaml resolved through the dedicated
    // `stm32g0` RCC layout (SystemBus::from_config errors on an unresolved
    // gate), so this additionally pins the G0 enable-register offsets.
    assert_eq!(chip.cpu_hz, 64_000_000, "max core clock (DS12232)");
}
