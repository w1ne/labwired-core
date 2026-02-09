use std::path::PathBuf;
use svd_ingestor::process_peripheral;
use svd_parser::svd::{Device, Peripheral};

fn get_fixture_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../../tests/fixtures");
    path.push(name);
    path
}

#[test]
fn test_parse_dummy_stm32() {
    let path = get_fixture_path("dummy_stm32.svd");
    let xml = std::fs::read_to_string(path).expect("Failed to read fixture");

    // Parse SVD using svd-parser
    let device = svd_parser::parse(&xml).expect("Failed to parse SVD");

    // Check device properties
    assert_eq!(device.name, "STM32F103");

    // Find peripheral
    let peripheral = device
        .peripherals
        .iter()
        .find(|p| p.name == "USART1")
        .expect("USART1 not found");

    // Process peripheral
    let descriptor = process_peripheral(&device, peripheral).expect("Failed to process peripheral");

    assert_eq!(descriptor.peripheral, "USART1");
    assert_eq!(descriptor.registers.len(), 3);

    // Verify SR register
    let sr = descriptor
        .registers
        .iter()
        .find(|r| r.id == "SR")
        .expect("SR not found");
    assert_eq!(sr.address_offset, 0x00);
    assert_eq!(sr.fields.len(), 2);

    // Verify fields in SR
    let txe = sr
        .fields
        .iter()
        .find(|f| f.name == "TXE")
        .expect("TXE not found");
    assert_eq!(txe.bit_range, [7, 7]); // bitRange [7:7]

    let tc = sr
        .fields
        .iter()
        .find(|f| f.name == "TC")
        .expect("TC not found");
    assert_eq!(tc.bit_range, [6, 6]); // bitRange [6:6]

    // Verify DR register
    let dr = descriptor
        .registers
        .iter()
        .find(|r| r.id == "DR")
        .expect("DR not found");
    // <bitRange>[8:0]</bitRange> -> msb=8, lsb=0
    // But DR field name is DR?
    let dr_field = dr
        .fields
        .iter()
        .find(|f| f.name == "DR")
        .expect("DR field not found");
    assert_eq!(dr_field.bit_range, [8, 0]);

    // Verify CR register (Side Effects)
    let cr = descriptor
        .registers
        .iter()
        .find(|r| r.id == "CR")
        .expect("CR not found");

    let side_effects = cr
        .side_effects
        .as_ref()
        .expect("Side effects missing for CR");
    assert_eq!(
        side_effects.write_action,
        Some(labwired_config::WriteAction::WriteOneToClear)
    );
}

#[test]
fn test_invalid_svd_structure() {
    // Manually construct a broken peripheral to test error handling
    // This mocks a scenario where logic might fail or return error,
    // although svd-parser types are strong, logic might have valid checks.

    // For example, if we wanted to test validation logic in svd-ingestor (if added).
    // Currently svd-ingestor doesn't have much validation, it relies on svd-parser.
    // Let's test a scenario where we pass a peripheral with no registers.

    let device = Device::builder()
        .name("TEST".to_string())
        .peripherals(vec![])
        .build(svd_parser::ValidateLevel::Disabled)
        .unwrap();

    let peripheral = svd_parser::svd::PeripheralInfo::builder()
        .name("EMPTY".to_string())
        .base_address(0x0)
        .build(svd_parser::ValidateLevel::Disabled)
        .unwrap();

    let peripheral_enum = Peripheral::Single(peripheral);

    let descriptor = process_peripheral(&device, &peripheral_enum).unwrap();
    assert!(descriptor.registers.is_empty());
}

#[test]
fn test_parse_derived_peripheral() {
    let path = get_fixture_path("advanced_stm32.svd");
    let xml = std::fs::read_to_string(path).expect("Failed to read fixture");
    let device = svd_parser::parse(&xml).expect("Failed to parse SVD");

    let timer1 = device
        .peripherals
        .iter()
        .find(|p| p.name == "TIMER1")
        .expect("TIMER1 not found");

    let descriptor = process_peripheral(&device, timer1).expect("Failed to process peripheral");

    // TIMER1 is derived from TIMER_BASE (which has CR) and adds SR.
    // So it should have 2 registers.
    assert_eq!(descriptor.registers.len(), 2);
    assert!(descriptor.registers.iter().any(|r| r.id == "CR"));
    assert!(descriptor.registers.iter().any(|r| r.id == "SR"));
}

#[test]
fn test_parse_clusters() {
    let path = get_fixture_path("advanced_stm32.svd");
    let xml = std::fs::read_to_string(path).expect("Failed to read fixture");
    let device = svd_parser::parse(&xml).expect("Failed to parse SVD");

    let clusters = device
        .peripherals
        .iter()
        .find(|p| p.name == "CLUSTERS")
        .expect("CLUSTERS not found");

    let descriptor = process_peripheral(&device, clusters).expect("Failed to process peripheral");

    // GRP[0] and GRP[1] clusters, each has REG and REGS[0..1].
    // Total 6 registers.
    assert_eq!(descriptor.registers.len(), 6);

    // Check offsets and potentially prefixes (if implemented)
    let r0 = descriptor
        .registers
        .iter()
        .find(|r| r.id == "GRP0_REG")
        .unwrap();
    assert_eq!(r0.address_offset, 0x00);

    let rs0 = descriptor
        .registers
        .iter()
        .find(|r| r.id == "GRP0_REGS0")
        .unwrap();
    assert_eq!(rs0.address_offset, 0x04);

    let rs1 = descriptor
        .registers
        .iter()
        .find(|r| r.id == "GRP0_REGS1")
        .unwrap();
    assert_eq!(rs1.address_offset, 0x08);

    let r1 = descriptor
        .registers
        .iter()
        .find(|r| r.id == "GRP1_REG")
        .unwrap();
    assert_eq!(r1.address_offset, 0x10);

    let rs1_0 = descriptor
        .registers
        .iter()
        .find(|r| r.id == "GRP1_REGS0")
        .unwrap();
    assert_eq!(rs1_0.address_offset, 0x14);
}
