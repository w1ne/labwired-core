//! Actual customer Zephyr ELF startup/WAKE prerequisite; no electrical-link claim.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::cortex_m::configure_cortex_m, Bus, Cpu, Machine};
use std::path::Path;

#[test]
#[ignore = "requires the built customer U5 ELF in IOLINKI_U5_ELF"]
fn customer_u5_firmware_initializes_and_handles_external_wake() {
    let elf = std::env::var("IOLINKI_U5_ELF").expect("IOLINKI_U5_ELF required");
    let bytes = std::fs::read(&elf).unwrap();
    let adapter_error = labwired_loader::resolve_symbol_in_elf(&bytes, "board_adapter_error")
        .expect("customer adapter error symbol");
    let chip = ChipDescriptor::from_file(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32u575.yaml"),
    )
    .unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        "name: u5-reference\nchip: stm32u575.yaml\nexternal_devices: []\nboard_io: []\n",
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let gpio_b = bus.find_peripheral_index_by_name("gpiob").unwrap();
    bus.set_peripheral_gpio_input(gpio_b, 1, true);
    bus.set_peripheral_gpio_input(gpio_b, 2, true);
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine
        .load_firmware(&labwired_loader::load_elf(Path::new(&elf)).unwrap())
        .unwrap();
    for _ in 0..1_000_000 {
        machine.step().unwrap();
    }
    assert_eq!(
        machine.bus.read_u32(0x40000028).unwrap(),
        159,
        "TIM2 1MHz prescaler: board startup completed; PC={:#x}",
        machine.cpu.get_pc()
    );
    assert_eq!(
        machine.bus.read_u32(0x46022060).unwrap() & 0xf00,
        0x100,
        "PB1 WAKE mux"
    );
    assert_eq!(machine.bus.read_u32(0x46022004).unwrap() & 2, 2);
    assert_eq!(machine.bus.read_u32(0x46022080).unwrap() & 2, 2);
    assert_eq!(
        machine.bus.read_u32(0x42020c00).unwrap() >> 10 & 3,
        1,
        "PD5 begins as SIO GPIO output"
    );
    assert_eq!(
        machine.bus.read_u32(adapter_error as u64).unwrap(),
        0,
        "customer adapter initialized without error"
    );
    let timer_before = machine.bus.read_u32(0x40000024).unwrap();
    let cycle_before = machine.logic_now_cycle();
    for _ in 0..10_000 {
        machine.step().unwrap();
    }
    let elapsed = machine
        .bus
        .read_u32(0x40000024)
        .unwrap()
        .wrapping_sub(timer_before);
    let expected = (machine.logic_now_cycle() - cycle_before) / 160;
    assert!(
        u64::from(elapsed).abs_diff(expected) <= 1,
        "TIM2 true microseconds: elapsed={elapsed}, expected={expected}"
    );
    machine.bus.set_peripheral_gpio_input(gpio_b, 1, false);
    assert_eq!(machine.bus.read_u32(0x46022010).unwrap() & 2, 2);
    for _ in 0..100_000 {
        machine.step().unwrap();
    }
    assert_eq!(
        machine.bus.read_u32(0x46022010).unwrap() & 2,
        0,
        "actual EXTI handler cleared WAKE"
    );
    assert_eq!(
        machine.bus.read_u32(0x4000440c).unwrap(),
        4167,
        "actual UART configured at COM2"
    );
    let cr1 = machine.bus.read_u32(0x40004400).unwrap();
    assert_eq!(
        cr1 & ((1 << 28) | (1 << 12) | (1 << 10) | (1 << 9) | 0xd),
        (1 << 12) | (1 << 10) | 0xd,
        "enabled UART TX/RX, 8 data bits with even parity"
    );
    assert_eq!(
        machine.bus.read_u32(0x40004404).unwrap() & (3 << 12),
        0,
        "one stop bit"
    );
    assert_eq!(
        machine.bus.read_u32(0x42020c00).unwrap() >> 10 & 0xf,
        0xa,
        "PD5/PD6 switch to UART alternate mode"
    );
    assert_eq!(
        machine.bus.read_u32(0x42020c20).unwrap() >> 20 & 0xff,
        0x77,
        "PD5/PD6 select USART2 AF7"
    );
    assert_eq!(machine.bus.read_u32(adapter_error as u64).unwrap(), 0);
}
