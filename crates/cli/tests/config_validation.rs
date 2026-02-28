use labwired_config::{ChipDescriptor, SystemManifest};
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

    // Try writing to I2C1_OAR1 (offset 0x08) which is a simple data register
    bus.write_u32(0x40005408, 0x55AA)
        .expect("Failed to write to I2C1_OAR1");
    let oar1 = bus
        .read_u32(0x40005408)
        .expect("Failed to read from I2C1_OAR1");
    assert_eq!(oar1 & 0xFFFF, 0x55AA);

    println!("SUCCESS: Chip configuration validated for GPIOC and I2C1");
}
