use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::riscv::configure_riscv, Machine};

#[test]
fn ch32v003_compiled_rv32e_firmware_toggles_pc0() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip_path = root.join("configs/chips/ch32v003f4p6.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("CH32V003 descriptor");
    let mut bus = SystemBus::from_config(&chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let image = labwired_loader::load_elf(&root.join("tests/fixtures/ch32v003f4p6-blink.elf"))
        .expect("compiled RV32E firmware ELF");

    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32V003 ELF");
    let gpioc = machine
        .bus
        .find_peripheral_index_by_name("gpioc")
        .expect("GPIOC registered from descriptor");

    let mut previous = None;
    let mut edges = 0;
    for _ in 0..200_000 {
        machine.step().expect("compiled RV32E firmware instruction");
        if let Some(state) = machine.bus.peripherals[gpioc].dev.read_gpio_output(0) {
            if previous.is_some_and(|prior| prior != state) {
                edges += 1;
            }
            previous = Some(state);
            if edges >= 2 {
                break;
            }
        }
    }

    assert!(
        edges >= 2,
        "compiled firmware produced only {edges} PC0 edges"
    );
    assert_eq!(machine.cpu.mcause, 0, "compiled firmware must not trap");
}
