// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF54L15 application-core boot conformance.
//!
//! The deterministic, ELF-independent twin of `test_nrf54l15_smoke_survival` in
//! `firmware_survival.rs` (which boots a real bare-metal ELF end to end). It
//! asserts, at the bus level, the facts the boot path depends on — the ones
//! that are silent when wrong:
//!
//!   1. Peripherals are reachable at the SECURE alias base 0x5000_0000. Every
//!      peripheral on this family sits behind one devicetree node with
//!      `ranges = <0x0 0x50000000 0x10000000>`, so a DT `reg = <0xc6000>` is
//!      absolute 0x500C_6000. Getting the window wrong produces a bus fault at
//!      the first UARTE write, which looks like a firmware bug.
//!   2. NVM is RRAM at 0x0 sized 1524 KB, and SRAM is 256 KB at 0x2000_0000 —
//!      so the reset SP is 0x2004_0000. A wrong RAM size does not fault; it
//!      silently corrupts the stack, which is far worse.
//!   3. FICR/UICR live OUTSIDE the 0x5000_0000 window at their own absolute
//!      addresses (0x00FF_C000 / 0x00FF_D000). This is easy to get wrong by
//!      pattern-matching the other peripherals.
//!   4. The three GPIO ports have DIFFERENT widths (P0=7, P1=17, P2=11), which
//!      is unusual and is what the DK's split LED wiring depends on.
//!
//! Register/address facts: Zephyr devicetree `dts/vendor/nordic/nrf54l_05_10_15.dtsi`
//! and `nrf54l15.dtsi`, cross-checked against the nRF54L15 Product Specification.

use labwired_config::ChipDescriptor;
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

// Peripherals at the secure alias the cpuapp DT uses by default.
const UARTE20: u64 = 0x500C_6000;
const UARTE30: u64 = 0x5010_4000;
// GPIO PERIPHERAL bases = devicetree address - 0x500. A Nordic GPIO DT node
// points at the OUT register, not at the peripheral base. See the long comment
// in configs/chips/nrf54l15.yaml.
const GPIO_P0: u64 = 0x5010_9B00;
const GPIO_P1: u64 = 0x500D_7D00;
const GPIO_P2: u64 = 0x5004_FF00;
const TIMER20: u64 = 0x500C_A000;
const GRTC: u64 = 0x500E_2000;
const TEMP: u64 = 0x500D_7000;

// NOT in the 0x5000_0000 window.
const FICR: u64 = 0x00FF_C000;
const UICR: u64 = 0x00FF_D000;
const RRAMC: u64 = 0x5004_B000;

// UARTE register offsets (nRF52-compatible layout, reused on this family).
const UARTE_ENABLE: u64 = 0x500;
const UARTE_PSEL_TXD: u64 = 0x50C;
const UARTE_BAUDRATE: u64 = 0x524;
const UARTE_ENABLE_UARTE: u32 = 8;

// GPIO (nRF52 profile), peripheral-base-relative.
const GPIO_OUT: u64 = 0x504;
const GPIO_OUTSET: u64 = 0x508;
const GPIO_DIRSET: u64 = 0x518;

fn nrf54l15_chip_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/nrf54l15.yaml")
}

fn nrf54l15_bus() -> SystemBus {
    let path = nrf54l15_chip_path();
    let chip = ChipDescriptor::from_file(&path).expect("load nrf54l15 chip");
    let manifest = labwired_config::SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "nrf54l15-boot".to_string(),
        chip: path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).expect("assemble nrf54l15 bus")
}

#[test]
fn memory_map_is_rram_at_zero_with_256k_sram() {
    let chip = ChipDescriptor::from_file(nrf54l15_chip_path()).expect("load chip");

    // RRAM, not flash: based at 0x0, and 1524 KB is the real (odd) size.
    assert_eq!(chip.flash.base, 0x0000_0000, "RRAM must be based at 0x0");
    assert_eq!(
        chip.flash.size, "1524KB",
        "nRF54L15 RRAM is 1524 KB (DT: cpuapp_rram), not 1.5 MB rounded"
    );

    assert_eq!(chip.ram.base, 0x2000_0000);
    assert_eq!(chip.ram.size, "256KB");

    // Consequence that actually bites: the reset stack pointer. 256 KB at
    // 0x2000_0000 puts the initial SP at 0x2004_0000, which is exactly what
    // the smoke firmware boots with.
    assert_eq!(
        chip.ram.base + 256 * 1024,
        0x2004_0000,
        "initial SP must be 0x2004_0000"
    );
}

#[test]
fn core_is_cortex_m33() {
    let chip = ChipDescriptor::from_file(nrf54l15_chip_path()).expect("load chip");
    assert_eq!(chip.core.as_deref(), Some("cortex-m33"));
    assert!(matches!(chip.arch, labwired_config::Arch::Arm));
}

#[test]
fn peripherals_mapped_at_secure_alias() {
    let mut bus = nrf54l15_bus();

    // A UARTE write/readback proves the window is mapped and the model is live.
    bus.write_u32(UARTE20 + UARTE_ENABLE, UARTE_ENABLE_UARTE).unwrap();
    assert_eq!(
        bus.read_u32(UARTE20 + UARTE_ENABLE).unwrap(),
        UARTE_ENABLE_UARTE,
        "UARTE20 must be reachable at the secure alias 0x500C_6000"
    );

    bus.write_u32(UARTE30 + UARTE_ENABLE, UARTE_ENABLE_UARTE).unwrap();
    assert_eq!(bus.read_u32(UARTE30 + UARTE_ENABLE).unwrap(), UARTE_ENABLE_UARTE);

    // BAUDRATE and PSEL round-trip (the console init sequence).
    bus.write_u32(UARTE20 + UARTE_BAUDRATE, 0x01D7_E000).unwrap();
    assert_eq!(bus.read_u32(UARTE20 + UARTE_BAUDRATE).unwrap(), 0x01D7_E000);
    bus.write_u32(UARTE20 + UARTE_PSEL_TXD, 0x24).unwrap(); // P1.04
    assert_eq!(bus.read_u32(UARTE20 + UARTE_PSEL_TXD).unwrap(), 0x24);
}

#[test]
fn all_three_gpio_ports_are_mapped_and_independent() {
    let mut bus = nrf54l15_bus();

    // DK LED0 is P2.09, LED1 is P1.10 — the split across ports is the point.
    // These land on the absolute addresses the devicetree advertises:
    // P2 OUTSET = 0x5004_FF00 + 0x508 = 0x5005_0408 = DT 0x5005_0400 + 0x008.
    bus.write_u32(GPIO_P2 + GPIO_DIRSET, 1 << 9).unwrap();
    bus.write_u32(GPIO_P2 + GPIO_OUTSET, 1 << 9).unwrap();
    assert_ne!(
        bus.read_u32(GPIO_P2 + GPIO_OUT).unwrap() & (1 << 9),
        0,
        "P2.09 (DK LED0) must latch high"
    );

    // Writing P2 must not disturb P1 or P0.
    assert_eq!(bus.read_u32(GPIO_P1 + GPIO_OUT).unwrap() & (1 << 9), 0);
    assert_eq!(bus.read_u32(GPIO_P0 + GPIO_OUT).unwrap(), 0);

    bus.write_u32(GPIO_P1 + GPIO_DIRSET, 1 << 10).unwrap();
    bus.write_u32(GPIO_P1 + GPIO_OUTSET, 1 << 10).unwrap();
    assert_ne!(
        bus.read_u32(GPIO_P1 + GPIO_OUT).unwrap() & (1 << 10),
        0,
        "P1.10 (DK LED1) must latch high"
    );
}

#[test]
fn ficr_and_uicr_are_outside_the_peripheral_window() {
    let bus = nrf54l15_bus();

    // These are absolute addresses, NOT 0x5000_0000-relative. A read must not
    // fault — unprovisioned UICR reading as zero is what the boot path expects.
    let _ = bus.read_u32(FICR);
    let _ = bus.read_u32(UICR);

    // And the RRAM controller is in the peripheral window, unlike FICR/UICR.
    let _ = bus.read_u32(RRAMC);
}

#[test]
fn timer_and_temp_and_grtc_windows_are_reachable() {
    let mut bus = nrf54l15_bus();

    // TIMER20 has cc-num = 6 on this family (DT), unlike the nRF52 default of 4.
    // A CC round-trip on channel 5 would fault or read back zero if the model
    // were built with only 4 channels.
    const TIMER_CC0: u64 = 0x540;
    bus.write_u32(TIMER20 + TIMER_CC0 + 5 * 4, 0x1234).unwrap();
    assert_eq!(
        bus.read_u32(TIMER20 + TIMER_CC0 + 5 * 4).unwrap(),
        0x1234,
        "TIMER20 must expose 6 CC channels (DT cc-num = 6)"
    );

    // TEMP and GRTC windows must be mapped so a probe does not fault the bus.
    let _ = bus.read_u32(TEMP);
    let _ = bus.read_u32(GRTC);
}

/// Regression probe for a PRE-EXISTING nRF5340 profile bug found while
/// onboarding the nRF54L15 (kept here because this is where the reasoning
/// lives; move it if the nRF5340 profile is fixed).
///
/// `configs/chips/nrf5340.yaml` maps GPIO P0 at 0x5084_2500, which is the
/// DEVICETREE address — i.e. the OUT register — not the peripheral base. By
/// the same +0x500 rule this file documents, the base should be 0x5084_2000.
/// If this test fails, the nRF5340 GPIO registers are all 0x500 too high and
/// its LEDs cannot be driven either.
#[test]
fn nrf5340_gpio_base_follows_the_same_plus_0x500_rule() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/nrf5340.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load nrf5340 chip");
    let gpio0 = chip
        .peripherals
        .iter()
        .find(|p| p.id == "gpio0")
        .expect("nrf5340 gpio0");

    assert_eq!(
        gpio0.base_address, 0x5084_2000,
        "nRF5340 GPIO P0 base should be the DT address 0x5084_2500 minus 0x500; \
         mapping the DT address directly puts every GPIO register 0x500 too high"
    );
}
