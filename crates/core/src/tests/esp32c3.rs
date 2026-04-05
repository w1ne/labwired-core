use crate::bus::SystemBus;
use crate::cpu::riscv::RiscV;
use crate::{Bus, Machine};
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[test]
fn test_esp32c3_full_smoke() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/esp32c3.yaml");

    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/esp32c3-devkit.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|_| panic!("Failed to load chip config at {:?}", chip_path));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|_| panic!("Failed to load system manifest at {:?}", system_path));

    // Anchor the chip path relative to the manifest, so that resolve_peripheral_path works
    let anchored_chip = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored_chip.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("Failed to build bus");

    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);

    // RV32IMC Code:
    // lui x10, 0x60000        -> 37 05 00 60
    // addi x11, x0, 79 ('O')  -> 93 05 F0 04
    // sw x11, 0(x10)          -> 23 20 B5 00
    // addi x11, x0, 75 ('K')  -> 93 05 B0 04
    // sw x11, 0(x10)          -> 23 20 B5 00
    // j .                     -> 6F 00 00 00
    let code = vec![
        0x37, 0x05, 0x00, 0x60, 0x93, 0x05, 0xF0, 0x04, 0x23, 0x20, 0xB5, 0x00, 0x93, 0x05, 0xB0,
        0x04, 0x23, 0x20, 0xB5, 0x00, 0x6F, 0x00, 0x00, 0x00,
    ];

    let load_addr = 0x42000000;
    for (i, byte) in code.iter().enumerate() {
        bus.write_u8(load_addr + i as u64, *byte).unwrap();
    }

    let mut cpu = RiscV::new();
    cpu.pc = load_addr as u32;

    let mut machine = Machine::new(cpu, bus);

    // Execute 20 steps to be sure
    for _ in 0..20 {
        machine.step().expect("Simulation failed");
    }

    let data = sink.lock().unwrap();
    assert_eq!(
        *data.last().expect("UART output empty"),
        75,
        "UART0 should contain 'K' (75)"
    );
}
