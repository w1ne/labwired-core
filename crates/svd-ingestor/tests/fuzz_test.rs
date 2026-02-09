use proptest::prelude::*;
use svd_ingestor::process_peripheral;
use svd_parser::svd::{Device, Peripheral, Register, RegisterCluster, RegisterInfo};

proptest! {
    #[test]
    fn test_fuzz_register_conversion(
        name in "[a-zA-Z0-9_]+",
        offset in 0u32..1000u32,
        size in prop::sample::select(vec![8u32, 16u32, 32u32, 64u32]),
        reset_value in any::<u64>(),
        dim in 0u32..10u32,
        _extra_offset in 0u64..1000u64
    ) {
        // Construct a somewhat valid SVD Register
        let mut reg_builder = RegisterInfo::builder()
            .name(name.clone())
            .address_offset(offset);

        // Random properties
        reg_builder = reg_builder
            .size(Some(size))
            .reset_value(Some(reset_value));

        let reg_info = reg_builder.build(svd_parser::ValidateLevel::Disabled).unwrap();

        let register = if dim > 0 {
             let dim_el = svd_parser::svd::DimElement::builder()
                .dim(dim)
                .dim_increment(4)
                .build(svd_parser::ValidateLevel::Disabled)
                .unwrap();
            Register::Array(reg_info, dim_el)
        } else {
            Register::Single(reg_info)
        };

        // Create minimal device/peripheral context
        let device = Device::builder()
            .name("FUZZ_DEV".to_string())
            .peripherals(vec![])
            .build(svd_parser::ValidateLevel::Disabled)
            .unwrap();

        let mut p_builder = svd_parser::svd::PeripheralInfo::builder()
            .name("FUZZ_PERIPH".to_string())
            .base_address(0x4000_0000);

        p_builder = p_builder.registers(Some(vec![
            RegisterCluster::Register(register)
        ]));

        let peripheral_info = p_builder.build(svd_parser::ValidateLevel::Disabled).unwrap();
        let peripheral = Peripheral::Single(peripheral_info);

        // Assert that processing this random register NEVER panics
        let _ = process_peripheral(&device, &peripheral);
    }
}
