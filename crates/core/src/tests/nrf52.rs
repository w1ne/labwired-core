use crate::bus::SystemBus;
use crate::cpu::cortex_m::CortexM;
use crate::{Bus, Cpu, Machine};
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[test]
fn test_nrf52_full_smoke() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/nrf52832.yaml");

    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/nrf52-dk.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|_| panic!("Failed to load chip config at {:?}", chip_path));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|_| panic!("Failed to load system manifest at {:?}", system_path));

    let anchored_chip = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored_chip.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("Failed to build bus");

    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);

    // Thumb-1 Code for Cortex-M4F (nRF52)
    let code = vec![
        0x02, 0x48, // ldr r0, [pc, #8]  (loads 0x4000251C)
        0x4F, 0x21, // movs r1, #79 ('O')
        0x01, 0x60, // str r1, [r0, #0]
        0x4B, 0x21, // movs r1, #75 ('K')
        0x01, 0x60, // str r1, [r0, #0]
        0xFE, 0xE7, // b .
        0x1C, 0x25, 0x00, 0x40, // .word 0x4000251C (UART0 TXD)
    ];

    let load_addr = 0x00000000; // nRF52 flash base
    for (i, byte) in code.iter().enumerate() {
        bus.write_u8(load_addr + i as u64, *byte).unwrap();
    }

    let mut cpu = CortexM::new();
    cpu.set_pc(load_addr as u32);

    let mut machine = Machine::new(cpu, bus);

    for _ in 0..20 {
        machine.step().expect("Simulation failed");
    }

    let data = sink.lock().unwrap();
    assert_eq!(
        *data.last().expect("UART output empty"),
        75,
        "UART0 TXD should contain 'K' (75)"
    );
}
