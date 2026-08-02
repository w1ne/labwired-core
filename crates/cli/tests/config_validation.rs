use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::peripherals::i2c::I2c;
use labwired_core::Bus;
use std::path::PathBuf;

#[test]
fn test_chip_config_validation() {
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/systems/labwired-demo-board.yaml");

    let manifest = SystemManifest::from_file(&system_path).expect("Failed to load system manifest");

    let chip_path = system_path.parent().unwrap().join(&manifest.chip);

    let chip = ChipDescriptor::from_file(&chip_path).expect("Failed to load chip descriptor");

    let mut bus =
        labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("Failed to build bus");

    // Verify key peripherals exist and are at correct addresses
    // GPIOC: 0x40011000
    // I2C1: 0x40005400

    // We can check this by trying to write/read from those addresses
    // or by inspecting the bus peripheral map if exposed.

    // Let's try writing to GPIOC_ODR
    bus.write_u32(0x4001100C, 0x1234)
        .expect("Failed to write to GPIOC");
    let val = bus.read_u32(0x4001100C).expect("Failed to read from GPIOC");
    assert_eq!(val, 0x1234);

    // I2C1 is clock-gated on STM32F103: RCC_APB1ENR.I2C1EN = bit 21.
    // RCC base is 0x40021000, APB1ENR is at offset 0x1C.
    // Enable the clock before accessing I2C1 registers, mirroring what
    // firmware does at boot.
    bus.write_u32(0x4002101C, 1 << 21)
        .expect("Failed to enable I2C1 clock in RCC_APB1ENR");

    // Try writing to I2C1_OAR1 (offset 0x08) which is a simple data register.
    // Bits 13:10 are reserved on F1 silicon and read back as 0, so the
    // silicon-faithful model echoes 0x55AA as 0x41AA.
    bus.write_u32(0x40005408, 0x55AA)
        .expect("Failed to write to I2C1_OAR1");
    let oar1 = bus
        .read_u32(0x40005408)
        .expect("Failed to read from I2C1_OAR1");
    assert_eq!(oar1 & 0xFFFF, 0x41AA);

    println!("SUCCESS: Chip configuration validated for GPIOC and I2C1");
}

/// Smoke test for the F407 chip yaml — proves the descriptor parses and
/// the I2C1 peripheral is wired at the F4-family base address with the
/// legacy (Stm32F1) register layout that the I2C model defaults to.
///
/// F407 is the hardware-oracle anchor for the STM32 I²C onboarding lane
/// (AHT20 + BMP280 over I²C1). F401 follows as a yaml delta once this
/// path is silicon-verified.
#[test]
fn test_stm32f407_chip_loads() {
    let chip_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f407.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).expect("F407 chip yaml failed to parse");

    assert_eq!(chip.name, "stm32f407vgt6");
    assert!(
        chip.peripherals
            .iter()
            .any(|p| p.id == "i2c1" && p.base_address == 0x40005400),
        "F407 must declare i2c1 at 0x40005400"
    );
    assert!(
        chip.peripherals
            .iter()
            .any(|p| p.id == "gpioa" && p.base_address == 0x40020000),
        "F407 must declare gpioa at 0x40020000"
    );
}

#[test]
fn test_stm32f401_chip_loads_with_i2c1() {
    let chip_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).expect("F401 chip yaml failed to parse");

    assert_eq!(chip.name, "stm32f401re");
    assert!(
        chip.peripherals
            .iter()
            .any(|p| p.id == "i2c1" && p.base_address == 0x40005400),
        "F401 must declare i2c1 at 0x40005400"
    );
}

#[test]
fn test_stm32f401cdu6_chip_loads_with_i2c1() {
    let chip_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401cdu6.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).expect("F401CDU6 chip yaml failed to parse");

    assert_eq!(chip.name, "stm32f401cdu6");
    assert_eq!(chip.flash.size, "384KB");
    assert_eq!(chip.ram.size, "96KB");
    assert!(
        chip.peripherals
            .iter()
            .any(|p| p.id == "i2c1" && p.base_address == 0x40005400),
        "F401CDU6 must declare i2c1 at 0x40005400"
    );
}

/// End-to-end proof that `external_devices` declared in a system manifest
/// actually attach to the corresponding I²C peripheral at bus-construction
/// time. Uses the demo-blinky fixture (STM32F103 + TMP102 on i2c1) which
/// is the canonical attach example pointed at from docs.
///
/// Before this test existed, the runtime attach was commented out in
/// `crates/core/src/bus/mod.rs` and demo-blinky's claim was aspirational.
#[test]
fn test_external_device_attaches_to_i2c() {
    let system_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-blinky/system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load demo-blinky manifest");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");

    let bus =
        labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("Failed to build bus");

    let i2c_entry = bus
        .peripherals
        .iter()
        .find(|p| p.name == "i2c1")
        .expect("bus must contain an i2c1 peripheral");

    let any = i2c_entry
        .dev
        .as_any()
        .expect("I2c must expose as_any for test downcast");
    let i2c = any
        .downcast_ref::<I2c>()
        .expect("i2c1 peripheral must be the I2c type");

    assert_eq!(
        i2c.attached_devices().len(),
        1,
        "demo-blinky declares one external_devices entry on i2c1 (tmp102)"
    );
    assert_eq!(
        i2c.attached_devices()[0].borrow().address(),
        0x48,
        "tmp102 attaches at default address 0x48"
    );
}

/// End-to-end proof that the nucleo-f407-i2c system loads cleanly, attaches
/// both AHT20 (0x38, command-stream) and BMP280 (0x76, register-bank) to
/// I²C1, and exposes them at distinct addresses. This is the simulator
/// side of the F407 onboarding lane — silicon-side proof follows once
/// hardware lands and the oracle trace is captured.
#[test]
fn test_nucleo_f407_i2c_attaches_aht20_and_bmp280() {
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/nucleo-f407-i2c/system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load f407 manifest");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load f407 chip");

    let bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest)
        .expect("Failed to build f407 bus");

    let i2c_entry = bus
        .peripherals
        .iter()
        .find(|p| p.name == "i2c1")
        .expect("bus must contain an i2c1 peripheral");

    let any = i2c_entry
        .dev
        .as_any()
        .expect("I2c must expose as_any for test downcast");
    let i2c = any
        .downcast_ref::<I2c>()
        .expect("i2c1 peripheral must be the I2c type");

    let addresses: Vec<u8> = i2c
        .attached_devices()
        .iter()
        .map(|d| d.borrow().address())
        .collect();
    assert_eq!(addresses.len(), 2, "AHT20 + BMP280 = 2 devices on i2c1");
    assert!(addresses.contains(&0x38), "AHT20 at 0x38 missing");
    assert!(addresses.contains(&0x76), "BMP280 at 0x76 missing");
}
