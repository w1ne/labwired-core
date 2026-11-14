use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, Bus};
use std::path::Path;

#[test]
fn g0_pa9_routes_uart_tx_only_through_af1() {
    let chip = ChipDescriptor::from_file(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32g0b1re.yaml"),
    )
    .unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        "name: g0-pad\nchip: stm32g0b1re.yaml\nexternal_devices: []\nboard_io: []\n",
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let gpio = bus.find_peripheral_index_by_name("gpioa").unwrap();
    bus.write_u32(0x40021034, 1).unwrap();
    bus.write_u32(0x50000000, 2 << 18).unwrap();
    bus.write_u32(0x50000024, 1 << 4).unwrap();
    assert_eq!(
        bus.peripherals[gpio]
            .dev
            .gpio_routing(9)
            .unwrap()
            .func
            .as_deref(),
        Some("USART1_TX")
    );
    bus.write_u32(0x50000024, 4 << 4).unwrap();
    assert_eq!(
        bus.peripherals[gpio]
            .dev
            .gpio_routing(9)
            .unwrap()
            .func
            .as_deref(),
        Some("AF4")
    );
}
