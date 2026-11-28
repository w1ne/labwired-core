use labwired_core::peripherals::exti::{Exti, ExtiRegisterLayout};
use labwired_core::peripherals::gpio::GpioPort;
use labwired_core::Peripheral;
use labwired_core::{bus::SystemBus, Bus};
use std::str::FromStr;

#[test]
fn g0_registers_group_gpio_pending_into_the_correct_irqs() {
    let mut exti = Exti::new_with_layout(ExtiRegisterLayout::from_str("stm32g0").unwrap());
    exti.write_u32(0x04, 1).unwrap();
    exti.write_u32(0x60, 1).unwrap();
    assert_eq!(exti.read_u32(0x04).unwrap(), 1);
    assert_eq!(exti.read_u32(0x60).unwrap(), 1);
    let pending = 1 | (1 << 3) | (1 << 9);
    exti.write_u32(0x80, pending).unwrap();
    for line in [0, 3, 9] {
        exti.trigger_line(line);
    }
    assert_eq!(exti.read_u32(0x0c).unwrap(), pending);
    assert_eq!(exti.tick().explicit_irqs, Some(vec![5, 6, 7]));
    exti.write_u32(0x0c, 1 << 3).unwrap();
    assert_eq!(exti.tick().explicit_irqs, Some(vec![5, 7]));
    exti.write(0x0c, 1).unwrap();
    assert_eq!(exti.read_u32(0x0c).unwrap(), 1 << 9);
    assert_eq!(exti.tick().explicit_irqs, Some(vec![7]));
    exti.write_u32(0x0c, 1 << 9).unwrap();
    assert_eq!(exti.tick().explicit_irqs, None);
}

#[test]
fn g0_masked_edges_latch_and_rising_falling_clear_independently() {
    let mut exti = Exti::new_with_layout(ExtiRegisterLayout::Stm32G0);
    exti.write_u32(0x60, 0xffffffff).unwrap();
    assert_eq!(exti.read_u32(0x60).unwrap(), 0x07070707);
    exti.write_u32(0x60, 1).unwrap();
    exti.write_u32(0, 1).unwrap();
    exti.write_u32(4, 1).unwrap();
    assert!(exti.gpio_edge(1, 0, true, false));
    assert!(exti.gpio_edge(1, 0, false, true));
    assert_eq!(exti.tick().explicit_irqs, None);
    assert_eq!(exti.read_u32(0xc).unwrap(), 1);
    assert_eq!(exti.read_u32(0x10).unwrap(), 1);
    exti.write_u32(0x80, 1).unwrap();
    assert_eq!(exti.tick().explicit_irqs, Some(vec![5]));
    exti.write_u32(0xc, 1).unwrap();
    assert_eq!(exti.read_u32(0x10).unwrap(), 1);
    assert_eq!(exti.tick().explicit_irqs, Some(vec![5]));
    exti.write_u32(0x10, 1).unwrap();
    assert_eq!(exti.tick().explicit_irqs, None);
}

#[test]
fn external_wake_pad_uses_configured_port_and_falling_edge() {
    let mut bus = SystemBus::empty();
    bus.add_peripheral(
        "gpioa",
        0x50000000,
        1024,
        None,
        Box::new(GpioPort::new_stm32v2_with_resets(0, 0, 0)),
    );
    bus.add_peripheral(
        "gpiob",
        0x50000400,
        1024,
        None,
        Box::new(GpioPort::new_stm32v2_with_resets(0, 0, 0)),
    );
    bus.add_peripheral(
        "exti",
        0x40021800,
        1024,
        None,
        Box::new(Exti::new_with_layout(
            ExtiRegisterLayout::from_str("stm32g0").unwrap(),
        )),
    );
    bus.set_peripheral_gpio_input(0, 0, true);
    bus.set_peripheral_gpio_input(1, 0, true);
    bus.write_u32(0x40021860, 1).unwrap(); // EXTI0 = PB0
    bus.write_u32(0x40021804, 1).unwrap(); // falling edge
    bus.write_u32(0x40021880, 1).unwrap(); // interrupt mask
    assert_eq!(bus.read_u32(0x50000410).unwrap() & 1, 1);
    assert_eq!(bus.read_u32(0x40021860).unwrap(), 1);
    assert_eq!(bus.read_u32(0x40021804).unwrap(), 1);
    bus.pending_schedule.clear();
    bus.set_peripheral_gpio_input(0, 0, false); // wrong port
    assert_eq!(bus.read_u32(0x40021810).unwrap(), 0);
    bus.set_peripheral_gpio_input(1, 0, false);
    assert_eq!(bus.read_u32(0x50000410).unwrap() & 1, 0);
    assert_eq!(bus.read_u32(0x40021810).unwrap(), 1);
    assert_eq!(bus.read_u32(0x4002180c).unwrap(), 0);
    assert_eq!(bus.tick_peripherals_fully_forced().0, vec![5]);
    if bus.peripherals[2].dev.uses_scheduler() {
        assert!(bus
            .pending_schedule
            .iter()
            .any(|&(idx, deadline, _)| idx == 2 && deadline == bus.current_cycle + 1));
    } else {
        assert!(bus.pending_schedule.is_empty());
    }
    bus.write_u32(0x40021810, 1).unwrap();
    bus.set_peripheral_gpio_input(1, 0, true); // wrong polarity
    assert_eq!(bus.read_u32(0x4002180c).unwrap(), 0);
    assert_eq!(bus.read_u32(0x40021810).unwrap(), 0);
    bus.set_peripheral_gpio_input(1, 0, false);
    assert_eq!(bus.read_u32(0x40021810).unwrap(), 1);
}
