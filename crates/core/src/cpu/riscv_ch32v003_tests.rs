use super::{RiscV, RiscVCoreProfile};
use crate::bus::SystemBus;
use crate::Machine;
use labwired_config::{ChipDescriptor, SystemManifest};

#[test]
fn ch32v003_descriptor_runs_rv32e_gpio_firmware_through_reset_alias() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip_path = root.join("configs/chips/ch32v003f4p6.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("CH32V003 descriptor");
    assert_eq!(RiscVCoreProfile::for_chip(&chip), RiscVCoreProfile::Rv32ec);

    let mut bus = SystemBus::from_config(&chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    // RV32E instructions: enable GPIOC on RCC APB2, configure PC0 output,
    // then set it through BSHR. Fetch starts at zero through the chip's flash
    // boot alias, while the descriptor maps the physical flash at 0x08000000.
    let firmware: [u32; 9] = [
        0x4002_12b7, // lui  x5, 0x40021       RCC
        0x0100_0313, // addi x6, x0, 16        APB2 GPIOC enable mask
        0x0062_ac23, // sw   x6, 24(x5)        APB2PCENR
        0x4001_12b7, // lui  x5, 0x40011       GPIOC
        0x0020_0313, // addi x6, x0, 2         PC0 push-pull output in CFGLR
        0x0062_a023, // sw   x6, 0(x5)         CFGLR
        0x0010_0313, // addi x6, x0, 1
        0x0062_a823, // sw   x6, 16(x5)        BSHR sets PC0
        0x0000_006f, // jal  x0, 0             firmware idle loop
    ];
    bus.flash.data = firmware.into_iter().flat_map(u32::to_le_bytes).collect();

    let mut cpu = RiscV::new_for(RiscVCoreProfile::for_chip(&chip));
    cpu.pc = 0;
    let mut machine = Machine::new(cpu, bus);
    for _ in 0..8 {
        machine.step().expect("RV32E firmware instruction");
    }

    let gpioc = machine
        .bus
        .find_peripheral_index_by_name("gpioc")
        .expect("GPIOC registered from descriptor");
    assert_eq!(
        machine.bus.peripherals[gpioc].dev.read_u32(0x04).unwrap(),
        0
    );
    assert_eq!(machine.bus.peripherals[gpioc].dev.read_gpio_input(8), None);
    assert_eq!(
        machine.bus.peripherals[gpioc].dev.read_gpio_is_output(8),
        None
    );
    assert_eq!(
        machine.bus.peripherals[gpioc].dev.read_gpio_is_output(0),
        Some(true)
    );
    assert_eq!(
        machine.bus.peripherals[gpioc].dev.read_gpio_output(0),
        Some(true)
    );
    assert_eq!(machine.cpu.mcause, 0, "firmware must not trap");
}
