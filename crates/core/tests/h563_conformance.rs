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
use labwired_core::Bus;

fn h563_bus() -> labwired_core::bus::SystemBus {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips/stm32h563.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load chip");
    let manifest = labwired_config::SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "h563-conformance".to_string(),
        chip: path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
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
fn peripheral_estate_reset_state_matches_silicon() {
    let mut bus = h563_bus();
    // These are CLOCKED reset values: TIM1/TIM2/I2C1 are clock-gated out of
    // reset (yaml `clock:`), so the warm silicon capture enabled their bus
    // clocks before sampling. Mirror that — enable TIM2EN/I2C1EN (APB1LENR)
    // and TIM1EN (APB2ENR) so the gated peripherals present their real reset
    // state rather than the dead-while-gated 0.
    const RCC: u64 = 0x4402_0C00;
    bus.write_u32(RCC + 0x9C, (1 << 21) | 1).unwrap(); // APB1LENR: I2C1EN|TIM2EN
    bus.write_u32(RCC + 0xA4, 1 << 11).unwrap(); // APB2ENR: TIM1EN
    let rd = |addr: u64| bus.read_u32(addr).unwrap();
    // Timers: ARR resets full-scale per counter width; everything else 0.
    assert_eq!(rd(0x4001_2C00 + 0x2C), 0xFFFF, "TIM1_ARR");
    assert_eq!(rd(0x4000_0000 + 0x2C), 0xFFFF_FFFF, "TIM2_ARR (32-bit)");
    assert_eq!(rd(0x4000_0400 + 0x2C), 0xFFFF, "TIM3_ARR");
    assert_eq!(rd(0x4000_1000 + 0x2C), 0xFFFF, "TIM6_ARR");
    for (base, name) in [
        (0x4001_2C00u64, "TIM1"),
        (0x4000_0000, "TIM2"),
        (0x4000_0400, "TIM3"),
        (0x4000_1000, "TIM6"),
    ] {
        assert_eq!(rd(base), 0, "{name}_CR1");
        assert_eq!(rd(base + 0x10), 0, "{name}_SR");
        assert_eq!(rd(base + 0x24), 0, "{name}_CNT");
        assert_eq!(rd(base + 0x28), 0, "{name}_PSC");
    }
    // I2C (v2 IP): ISR resets to TXE.
    assert_eq!(rd(0x4000_5400 + 0x18), 0x0001, "I2C1_ISR");
    assert_eq!(rd(0x4000_5800 + 0x18), 0x0001, "I2C2_ISR");
    assert_eq!(rd(0x4000_5400 + 0x08), 0, "I2C1_OAR1");
    // Extra UARTs: ISR resets to TXE|TC like USART3.
    assert_eq!(rd(0x4001_3800 + 0x1C), 0xC0, "USART1_ISR");
    assert_eq!(rd(0x4000_4400 + 0x1C), 0xC0, "USART2_ISR");
    assert_eq!(rd(0x4400_2400 + 0x1C), 0xC0, "LPUART1_ISR");
    // Watchdogs. WWDG CR is not pinned: on silicon the T counter decrements
    // as soon as the APB clock is on (captured mid-count at 0x4D with WDGA
    // clear) — only CFR is a stable reset value.
    assert_eq!(rd(0x4000_2C00 + 0x04), 0x7F, "WWDG_CFR");
    assert_eq!(rd(0x4000_3000), 0, "IWDG_KR");
    assert_eq!(rd(0x4000_3000 + 0x08), 0xFFF, "IWDG_RLR");
    // CRC.
    assert_eq!(rd(0x4002_3000), 0xFFFF_FFFF, "CRC_DR");
    assert_eq!(rd(0x4002_3000 + 0x04), 0, "CRC_IDR");
    // RNG: SR only — silicon resets CR to 0x00800D00 (NIST-config default),
    // which the generic model does not carry; documented in the chip yaml.
    assert_eq!(rd(0x420C_0800 + 0x04), 0, "RNG_SR");
    // LPTIM1: whole captured block is zero at reset.
    for off in [0u64, 0x0C, 0x10, 0x14] {
        assert_eq!(rd(0x4400_4400 + off), 0, "LPTIM1 @ +{off:#X}");
    }
}

#[test]
fn crc_compute_matches_silicon() {
    // Bench oracle: feeding 0x12345678 to the H563's CRC unit (reset state)
    // returned 0xDF8A8A2B over SWD. The sim must compute the same word.
    let mut bus = h563_bus();
    bus.write_u32(0x4002_3008, 1).unwrap(); // CR.RESET
    bus.write_u32(0x4002_3000, 0x1234_5678).unwrap();
    assert_eq!(bus.read_u32(0x4002_3000).unwrap(), 0xDF8A_8A2B, "CRC_DR");
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

#[test]
fn class_model_reset_state_matches_silicon() {
    // SPI1/SPI2 (stm32h5 profile), ADC1 (stm32l4 layout), RTC (rtc_v3) and
    // GPDMA1 reset values, all pinned to the 2026-06-11 NUCLEO-H563ZI
    // capture (validation corpus probe-20260611).
    //
    // These are CLOCKED reset values: SPI1/ADC1/RTC/GPDMA1 are clock-gated out
    // of reset (yaml `clock:`), so the warm capture enabled their bus clocks
    // first. Mirror that before sampling — otherwise the gated peripherals read
    // the dead-while-gated 0. (SPI2 is left ungated, so it reads either way.)
    let mut bus = h563_bus();
    const RCC: u64 = 0x4402_0C00;
    bus.write_u32(RCC + 0xA4, 1 << 12).unwrap(); // APB2ENR: SPI1EN
    bus.write_u32(RCC + 0x8C, 1 << 10).unwrap(); // AHB2ENR: ADCEN
    bus.write_u32(RCC + 0xA8, 1 << 21).unwrap(); // APB3ENR: RTCAPBEN
    bus.write_u32(RCC + 0x88, 1).unwrap(); // AHB1ENR: GPDMA1EN
    let rd = |addr: u64| bus.read_u32(addr).unwrap();

    // SPI1 @ 0x4001_3000 / SPI2 @ 0x4000_3800.
    for base in [0x4001_3000u64, 0x4000_3800] {
        assert_eq!(rd(base), 0, "SPI CR1 @ {base:#X}");
        assert_eq!(rd(base + 0x08), 0x0007_0007, "SPI CFG1 @ {base:#X}");
        assert_eq!(rd(base + 0x0C), 0, "SPI CFG2 @ {base:#X}");
        assert_eq!(rd(base + 0x14), 0x0000_1002, "SPI SR @ {base:#X}");
        assert_eq!(rd(base + 0x40), 0x0000_0107, "SPI CRCPOLY @ {base:#X}");
    }

    // ADC1 @ 0x4202_8000: deep power-down + JQDIS out of reset.
    assert_eq!(rd(0x4202_8000), 0, "ADC1_ISR");
    assert_eq!(rd(0x4202_8008), 0x2000_0000, "ADC1_CR");
    assert_eq!(rd(0x4202_800C), 0x8000_0000, "ADC1_CFGR");

    // RTC @ 0x4400_7800 (fresh backup domain).
    assert_eq!(rd(0x4400_7800), 0, "RTC_TR");
    assert_eq!(rd(0x4400_7804), 0x0000_2101, "RTC_DR");
    assert_eq!(rd(0x4400_780C), 0x0000_0007, "RTC_ICSR");
    assert_eq!(rd(0x4400_7810), 0x007F_00FF, "RTC_PRER");
    assert_eq!(rd(0x4400_7814), 0x0000_FFFF, "RTC_WUTR");
    assert_eq!(rd(0x4400_7818), 0, "RTC_CR");

    // GPDMA1 @ 0x4002_0000: every channel idles with IDLEF.
    assert_eq!(rd(0x4002_0000), 0, "GPDMA1_SECCFGR");
    for ch in 0..8u64 {
        let csr = 0x4002_0000 + 0x60 + ch * 0x80;
        assert_eq!(rd(csr), 0x1, "GPDMA1_C{ch}SR");
    }
}
