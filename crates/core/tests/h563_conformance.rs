// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! STM32H563 boot-state conformance: the simulator's reset values for the
//! wired peripherals must match real silicon.
//!
//! Ground truth: register capture from a NUCLEO-H563ZI (DBGMCU_IDCODE
//! 0x10016484, Cortex-M33 r0p4) at reset halt over SWD —
//! `scripts/hw-capture-stm32h563.sh`, 2026-06-10. Volatile registers (GPIO
//! IDR pin states, factory calibration variance) are intentionally not
//! asserted; HSICAL/CSICAL are pinned to the captured part as representative
//! values.
//!
//! Cross-check on hardware: `examples/nucleo-h563zi/silicon-smoke` prints
//! RCC_CR / GPIOA_MODER / SYSTICK_CALIB over USART3 — the same ELF must
//! produce identical lines on the board (VCP) and in the simulator.

use labwired_config::ChipDescriptor;

fn h563_bus() -> labwired_core::bus::SystemBus {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips/stm32h563.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load chip");
    let manifest = labwired_config::SystemManifest {
        walk_deleted: false,
        schema_version: "1.0".to_string(),
        name: "h563-conformance".to_string(),
        chip: path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus")
}

#[test]
fn rcc_reset_state_matches_silicon() {
    let bus = h563_bus();
    let rd = |off: u64| bus.read_u32(0x4402_0C00 + off).unwrap();
    assert_eq!(rd(0x00), 0x0000_002B, "RCC_CR");
    assert_eq!(rd(0x10), 0x0040_04F7, "RCC_HSICFGR");
    assert_eq!(rd(0x18), 0x0020_0087, "RCC_CSICFGR");
    assert_eq!(rd(0x1C), 0x0000_0000, "RCC_CFGR1");
    assert_eq!(rd(0x20), 0x0000_0000, "RCC_CFGR2");
    assert_eq!(rd(0x88), 0xD000_0100, "RCC_AHB1ENR");
    assert_eq!(rd(0x8C), 0xC000_0000, "RCC_AHB2ENR");
    assert_eq!(rd(0x9C), 0x0000_0000, "RCC_APB1LENR");
    assert_eq!(rd(0xA4), 0x0000_0000, "RCC_APB2ENR");
    assert_eq!(rd(0xA8), 0x0000_0000, "RCC_APB3ENR");
    assert_eq!(rd(0xF4), 0x0C00_0000, "RCC_RSR");
}

#[test]
fn gpio_reset_state_matches_silicon() {
    let bus = h563_bus();
    // (base, MODER, OSPEEDR, PUPDR) per port; debug pins shape A and B.
    let ports: [(u64, u32, u32, u32); 7] = [
        (0x4202_0000, 0xABFF_FFFF, 0x0C00_0000, 0x6400_0000), // A
        (0x4202_0400, 0xFFFF_FEBF, 0x0000_00C0, 0x0000_0100), // B
        (0x4202_0800, 0xFFFF_FFFF, 0, 0),                     // C
        (0x4202_0C00, 0xFFFF_FFFF, 0, 0),                     // D
        (0x4202_1000, 0xFFFF_FFFF, 0, 0),                     // E
        (0x4202_1400, 0xFFFF_FFFF, 0, 0),                     // F
        (0x4202_1800, 0xFFFF_FFFF, 0, 0),                     // G
    ];
    for (base, moder, ospeedr, pupdr) in ports {
        assert_eq!(bus.read_u32(base).unwrap(), moder, "MODER @ {base:#X}");
        assert_eq!(
            bus.read_u32(base + 0x08).unwrap(),
            ospeedr,
            "OSPEEDR @ {base:#X}"
        );
        assert_eq!(
            bus.read_u32(base + 0x0C).unwrap(),
            pupdr,
            "PUPDR @ {base:#X}"
        );
    }
}

#[test]
fn systick_and_usart_reset_state_match_silicon() {
    let bus = h563_bus();
    assert_eq!(
        bus.read_u32(0xE000_E01C).unwrap(),
        0x0010_03E8,
        "SYSTICK_CALIB"
    );
    // USART3 ISR reset: TXE|TC.
    assert_eq!(
        bus.read_u32(0x4000_4800 + 0x1C).unwrap(),
        0x0000_00C0,
        "USART3_ISR"
    );
}
