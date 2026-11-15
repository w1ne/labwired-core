//! STM32U575 GPIO EXTI register/IRQ contract from pinned STM32U575xx header.
use labwired_core::peripherals::exti::{Exti, ExtiRegisterLayout};
use labwired_core::Peripheral;
use std::str::FromStr;

#[test]
fn u5_gpio_mux_split_pending_and_individual_irqs() {
    let mut exti = Exti::new_with_layout(ExtiRegisterLayout::from_str("stm32u5").unwrap());
    exti.write_u32(0x60, 0xffff_ffff).unwrap();
    assert_eq!(exti.read_u32(0x60).unwrap(), 0x0f0f0f0f);
    exti.write_u32(0x60, 0x08000100).unwrap(); // PB1 and PI3 (U5 has GPIOI)
    exti.write_u32(0x6c, 0x08000000).unwrap(); // PI15
    let mask = (1 << 1) | (1 << 3) | (1 << 15);
    exti.write_u32(0, mask).unwrap();
    exti.write_u32(4, mask).unwrap();
    assert!(!exti.gpio_edge(0, 1, true, false));
    assert!(exti.gpio_edge(1, 1, true, false));
    assert!(exti.gpio_edge(8, 3, false, true));
    assert!(exti.gpio_edge(8, 15, true, false));
    assert_eq!(exti.tick().explicit_irqs, None, "masked flags latch");
    exti.write_u32(0x80, mask).unwrap();
    assert_eq!(exti.tick().explicit_irqs, Some(vec![12, 14, 26]));
    exti.write(0x10, 2).unwrap(); // byte W1C must preserve line15
    assert_eq!(exti.read_u32(0x10).unwrap(), 1 << 15);
    exti.write_u32(0x0c, 1 << 3).unwrap();
    assert_eq!(exti.tick().explicit_irqs, Some(vec![26]));
    exti.write(0x11, 0x80).unwrap();
    assert_eq!(exti.tick().explicit_irqs, None);
}
