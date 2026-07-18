// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! STM32F401 cold-reset conformance: the simulator's reset values for RCC,
//! FLASH, PWR and the debug-port GPIO banks must match the STM32F401 register
//! ground truth (issue #576).
//!
//! Ground truth: the STM32F401 CMSIS SVD (`tests/fixtures/real_world/
//! stm32f401.svd`) — the same oracle the `register_coverage` gate probes —
//! cross-read against RM0368 Rev 5. Where RM0368's printed reset value is a
//! known documentation quirk (GPIOA_MODER/OSPEEDR, PWR_CR, FLASH_OPTCR), the
//! SVD carries the architectural reset value and wins; those cases are called
//! out inline.
//!
//! Before this fix the F401 reused F1/L4 peripheral models whose family-specific
//! reset words leaked in: RCC_CR read 0x03 (no HSITRIM), FLASH_ACR read 0x30
//! (F1 prefetch default), PWR_CR read 0x200 (L4 VOS), and the debug GPIO banks
//! read 0.

use labwired_config::ChipDescriptor;
use labwired_core::Bus;

fn f401_bus() -> labwired_core::bus::SystemBus {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips/stm32f401.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load chip");
    let manifest = labwired_config::SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "stm32f401-conformance".to_string(),
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
fn rcc_reset_state_matches_rm0368() {
    let bus = f401_bus();
    let rd = |off: u64| bus.read_u32(0x4002_3800 + off).unwrap();
    // RCC_CR reset = 0x0000_0083: HSION(0) + HSIRDY(1) + HSITRIM=0x10 (bits 7:3).
    // RM0368 §6.3.1 and the SVD agree; the old model dropped HSITRIM (read 0x03).
    assert_eq!(rd(0x00), 0x0000_0083, "RCC_CR");
    // RCC_PLLCFGR reset = 0x2400_3010 (RM0368 §6.3.2) — already correct.
    assert_eq!(rd(0x04), 0x2400_3010, "RCC_PLLCFGR");
    assert_eq!(rd(0x08), 0x0000_0000, "RCC_CFGR");
    assert_eq!(rd(0x40), 0x0000_0000, "RCC_APB1ENR");
    assert_eq!(rd(0x44), 0x0000_0000, "RCC_APB2ENR");
}

#[test]
fn flash_reset_state_matches_svd() {
    let bus = f401_bus();
    let rd = |off: u64| bus.read_u32(0x4002_3C00 + off).unwrap();
    // FLASH_ACR reset = 0 (RM0368 §3.8.1) — no F1 prefetch default (was 0x30).
    assert_eq!(rd(0x00), 0x0000_0000, "FLASH_ACR");
    assert_eq!(rd(0x0C), 0x0000_0000, "FLASH_SR");
    // FLASH_CR reset = 0x8000_0000 (LOCK at bit 31, RM0368 §3.8.5).
    assert_eq!(rd(0x10), 0x8000_0000, "FLASH_CR");
    // FLASH_OPTCR: SVD architectural default (RM0368 §3.8.6 prints the
    // option-byte-loaded 0x0FFF_AAED).
    assert_eq!(rd(0x14), 0x0000_0014, "FLASH_OPTCR");
}

#[test]
fn pwr_reset_state_matches_svd() {
    let bus = f401_bus();
    let rd = |off: u64| bus.read_u32(0x4000_7000 + off).unwrap();
    // PWR_CR/PWR_CSR reset = 0 (STM32F401 SVD). The old L4 model read CR=0x200.
    assert_eq!(rd(0x00), 0x0000_0000, "PWR_CR");
    assert_eq!(rd(0x04), 0x0000_0000, "PWR_CSR");
    // The L4 CR3 / SR2 / PUCRx surface must not exist on F4.
    assert_eq!(rd(0x08), 0x0000_0000, "no PWR reg at 0x08");
    assert_eq!(rd(0x14), 0x0000_0000, "no PWR reg at 0x14");
}

#[test]
fn gpio_reset_state_matches_svd() {
    let bus = f401_bus();
    // (base, MODER, OSPEEDR, PUPDR) per port. A/B carry the SWD/JTAG pin
    // defaults (RM0368 §8.4 / STM32F401 SVD); C..H reset all-zero.
    let ports: [(u64, u32, u32, u32); 3] = [
        (0x4002_0000, 0xA800_0000, 0x0000_0000, 0x6400_0000), // A
        (0x4002_0400, 0x0000_0280, 0x0000_00C0, 0x0000_0100), // B
        (0x4002_0800, 0x0000_0000, 0x0000_0000, 0x0000_0000), // C
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
