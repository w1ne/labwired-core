//! Actual customer ELF startup/WAKE prerequisite; no electrical-link claim.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::cortex_m::configure_cortex_m, Bus, Machine};
use std::path::Path;

#[test]
#[ignore = "requires the built customer G0 ELF in IOLINKI_G0_ELF"]
fn customer_g0_firmware_initializes_and_handles_external_wake() {
    let elf = std::env::var("IOLINKI_G0_ELF").expect("IOLINKI_G0_ELF required");
    let chip = ChipDescriptor::from_file(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32g0b1re.yaml"),
    )
    .unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        "name: g0-reference\nchip: stm32g0b1re.yaml\nexternal_devices: []\nboard_io: []\n",
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let gpio_b = bus.find_peripheral_index_by_name("gpiob").unwrap();
    bus.set_peripheral_gpio_input(gpio_b, 0, true);
    bus.set_peripheral_gpio_input(gpio_b, 1, true);
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine
        .load_firmware(&labwired_loader::load_elf(Path::new(&elf)).unwrap())
        .unwrap();
    for _ in 0..500_000 {
        machine.step().unwrap();
    }
    assert_eq!(machine.bus.read_u32(0x40021860).unwrap() & 7, 1);
    assert_eq!(machine.bus.read_u32(0x40021804).unwrap() & 1, 1);
    assert_eq!(machine.bus.read_u32(0x40021880).unwrap() & 1, 1);
    assert_eq!(
        machine.bus.read_u32(0x40000028).unwrap(),
        15,
        "TIM2 1MHz prescaler"
    );
    machine.bus.set_peripheral_gpio_input(gpio_b, 0, false);
    assert_eq!(machine.bus.read_u32(0x40021810).unwrap() & 1, 1);
    for _ in 0..100_000 {
        machine.step().unwrap();
    }
    assert_eq!(
        machine.bus.read_u32(0x40021810).unwrap() & 1,
        0,
        "actual EXTI handler cleared wake"
    );
    assert_eq!(
        machine.bus.read_u32(0x4001380c).unwrap(),
        417,
        "wake configured actual UART at COM2"
    );
}
