//! CH32X035 USART1_TX is PB10 at the reset mapping (`pad_map: ch32x03x`),
//! not the STM32F1 PA9, and AFIO_PCFR1 USART1_RM [6:5] moves it off PB10.
//! Without this route the F1 console gate stays shut and WCH's Arduino
//! core prints nothing.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, Bus};
use std::path::Path;

const RCC_APB2PCENR: u64 = 0x4002_1018;
const AFIO_PCFR1: u64 = 0x4001_0004;
const GPIOB_CFGHR: u64 = 0x4001_0C04;

#[test]
fn x035_usart1_tx_is_pb10_until_usart1_rm_moves_it() {
    let chip = ChipDescriptor::from_file(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/ch32x035g8u6.yaml"),
    )
    .unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        "name: x035-pad\nchip: ch32x035g8u6.yaml\nexternal_devices: []\nboard_io: []\n",
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let gpiob = bus.find_peripheral_index_by_name("gpiob").unwrap();
    let gpioa = bus.find_peripheral_index_by_name("gpioa").unwrap();
    let func = |bus: &SystemBus, idx: usize, pin: u8| {
        bus.peripherals[idx]
            .dev
            .gpio_routing(pin)
            .and_then(|route| route.func)
    };

    // AFIO, IOPB and USART1 clocks; PB10 = 50 MHz AF push-pull (0xB).
    bus.write_u32(RCC_APB2PCENR, 1 | (1 << 3) | (1 << 14))
        .unwrap();
    bus.write_u32(GPIOB_CFGHR, 0xB << 8).unwrap();
    assert_eq!(func(&bus, gpiob, 10).as_deref(), Some("USART1_TX"));
    assert_ne!(func(&bus, gpioa, 9).as_deref(), Some("USART1_TX"));

    // USART1_RM = 1 (GPIO_PartialRemap1_USART1): PB10 is no longer TX.
    bus.write_u32(AFIO_PCFR1, 1 << 5).unwrap();
    assert_ne!(func(&bus, gpiob, 10).as_deref(), Some("USART1_TX"));
}
