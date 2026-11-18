// SPDX-License-Identifier: MIT
//! A routed timed USART pad follows its existing shifter, including parity and
//! STM32 CR2.TXINV. A diagnostic byte trace cannot stand in for this wire.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::{DevicePins, SystemBus},
    network::timed_uart::TimedUartNet,
    peripherals::uart::Uart,
    Bus,
};
const UART: u64 = 0x40013800;
fn setup(inverted: bool) -> SystemBus {
    let chip = ChipDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/stm32l476.yaml"
    ))
    .unwrap();
    let system: SystemManifest = serde_yaml::from_str(
        "name: timed-pad\nchip: stm32l476\ncpu_hz: 80000000\nexternal_devices: []\n",
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &system).unwrap();
    let net = TimedUartNet::new();
    let (_, port, _) = net.add_link(
        ("a", "uart1", 80_000_000),
        ("b", "uart1", 80_000_000),
        0,
        0,
        1,
    );
    let index = bus.find_peripheral_index_by_name("uart1").unwrap();
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<Uart>()
        .unwrap()
        .attach_timed_port(port)
        .unwrap();
    bus.write_u32(0x4002104c, 1).unwrap();
    bus.write_u32(0x40021060, 1 << 14).unwrap();
    bus.write_u32(0x48000000, 2 << 18).unwrap(); // PA9 AF
    bus.write_u32(0x48000024, 7 << 4).unwrap(); // USART1 AF7
    bus.write_u32(UART + 0x0c, 80).unwrap();
    bus.write_u32(UART + 4, u32::from(inverted) << 17).unwrap();
    bus.write_u32(UART, 1 | 8 | (1 << 12) | (1 << 10)).unwrap(); // 8E1, UE+TE
    bus
}
fn tick(bus: &mut SystemBus, until: u64) {
    while bus.current_cycle < until {
        bus.set_current_cycle(bus.current_cycle + 1);
        bus.tick_peripherals_fully_forced();
    }
}
#[test]
fn actual_timed_shift_bits_parity_stop_and_inverted_idle_reach_the_af_pad() {
    for inverted in [false, true] {
        let mut bus = setup(inverted);
        assert_eq!(
            bus.known_pad_bit(0x48000014, 9),
            Some(!inverted),
            "idle TXINV={inverted}"
        );
        bus.write_u32(UART + 0x28, 0xa5).unwrap();
        let bits = [
            false, true, false, true, false, false, true, false, true, false, true,
        ];
        for (index, bit) in bits.into_iter().enumerate() {
            tick(&mut bus, 1 + index as u64 * 80);
            assert_eq!(
                bus.known_pad_bit(0x48000014, 9),
                Some(bit ^ inverted),
                "bit {index}, TXINV={inverted}"
            );
        }
        tick(&mut bus, 881);
        assert_eq!(bus.read_u32(UART + 0x1c).unwrap() & 0x40, 0x40);
        assert_eq!(bus.known_pad_bit(0x48000014, 9), Some(!inverted));
        bus.write_u32(UART, 0).unwrap();
        assert_eq!(
            bus.known_pad_bit(0x48000014, 9),
            None,
            "disabled USART releases its AF pad"
        );
    }
}
