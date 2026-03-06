use crate::bus::SystemBus;
use crate::cpu::cortex_m::CortexM;
use crate::{Bus, Cpu, Machine};
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[test]
fn test_rp2040_full_smoke() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/rp2040.yaml");

    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/rp2040-pico.yaml");

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

    // Thumb-1 Code for Cortex-M0+ (RP2040)
    // ldr r0, [pc, #8]  -> 4802 (loads 0x40034000 into r0)
    // movs r1, #79 ('O') -> 214F
    // str r1, [r0, #0]   -> 6001
    // movs r1, #75 ('K') -> 214B
    // str r1, [r0, #0]   -> 6001
    // b .                -> E7FE
    // .word 0x40034000   -> 40034000
    let code = vec![
        0x02, 0x48, 0x4F, 0x21, 0x01, 0x60, 0x4B, 0x21, 0x01, 0x60, 0xFE, 0xE7, 0x00, 0x40, 0x03,
        0x40,
    ];

    let load_addr = 0x10000000;
    for (i, byte) in code.iter().enumerate() {
        bus.write_u8(load_addr + i as u64, *byte).unwrap();
    }

    let mut cpu = CortexM::new(); // RP2040 is Cortex-M0+
    cpu.set_pc(load_addr as u32);

    let mut machine = Machine::new(cpu, bus);

    // Execute 20 steps
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
