use crate::bus::SystemBus;
use crate::cpu::cortex_m::CortexM;
use crate::{Bus, Cpu, Machine};
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[test]
fn test_rp2040_full_smoke() {
    // Opt out of in-tree bootrom so bare-metal code at flash base is not
    // shadowed by mask ROM at address 0 (same as `rp2040_bus()`).
    std::env::set_var("LABWIRED_RP2040_BOOTROM", "");

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
    // UART0 is a PL011: the TX data register (UARTDR) is at offset 0x00.
    // ldr r0, [pc, #8]     -> 4802 (loads 0x40034000 into r0)
    // movs r1, #79 ('O')   -> 214F
    // str r1, [r0, #0x00]  -> 6001
    // movs r1, #75 ('K')   -> 214B
    // str r1, [r0, #0x00]  -> 6001
    // b .                  -> E7FE
    // .word 0x40034000     -> 40034000
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

/// Build a SystemBus for the rp2040-pico target, mirroring `test_rp2040_full_smoke`.
fn rp2040_bus() -> SystemBus {
    // Bare-metal unit fixtures map vectors at flash base / use the Cortex-M
    // boot alias at 0. Empty env opts out of the in-tree mask ROM so that
    // alias is not shadowed (see `from_config` LABWIRED_RP2040_BOOTROM).
    std::env::set_var("LABWIRED_RP2040_BOOTROM", "");

    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/rp2040.yaml");
    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/rp2040-pico.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).expect("load rp2040 chip");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load rp2040 system");
    let anchored_chip = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored_chip.to_str().unwrap().to_string();
    SystemBus::from_config(&chip, &manifest).expect("Failed to build bus")
}

/// The RP2040 bootrom runs a 256-byte stage-2 (boot2) blob from the top of
/// flash and only then enters the application vector table at
/// `flash_base + 0x100`. LabWired does not execute boot2 (flash is directly
/// mapped), so `load_firmware` must relocate the reset vector past the stage-2
/// blob — otherwise reset reads boot2's first words as a bogus (SP, PC) and
/// faults at step 0. This is exactly what a real Zephyr/pico-sdk image looks
/// like.
#[test]
fn test_rp2040_boot2_vector_relocation() {
    use crate::system::cortex_m::configure_cortex_m;

    let mut bus = rp2040_bus();

    // Flash image: a boot2 stage-2 blob whose first words are NOT a valid
    // (SP, PC) pair, followed by the real vector table at +0x100.
    let mut image = vec![0u8; 0x108];
    image[0..4].copy_from_slice(&0x4b32_b500u32.to_le_bytes()); // bogus "SP" (boot2 code)
    image[4..8].copy_from_slice(&0x6058_2021u32.to_le_bytes()); // bogus "PC"
    let real_sp = 0x2000_1000u32;
    let real_reset = 0x1000_0401u32; // thumb bit set
    image[0x100..0x104].copy_from_slice(&real_sp.to_le_bytes());
    image[0x104..0x108].copy_from_slice(&real_reset.to_le_bytes());

    let mut img = crate::memory::ProgramImage::new(real_reset as u64, crate::Arch::Arm);
    img.add_segment(0x1000_0000, image);

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&img).expect("load firmware");

    assert_eq!(
        machine.cpu.get_pc(),
        0x1000_0400,
        "PC must be the post-boot2 reset handler, not boot2 garbage"
    );
    assert_eq!(
        machine.cpu.get_register(13),
        real_sp,
        "SP must come from the post-boot2 vector table"
    );
    assert_eq!(
        machine.bus.read_u32(0xE000_ED08).unwrap(),
        0x1000_0100,
        "VTOR must be relocated past the stage-2 bootloader"
    );
}

/// A bare-metal RP2040 image whose vector table already sits at the flash base
/// (no stage-2 blob) must NOT be relocated — the primary vectors are valid and
/// must be honored verbatim, matching the repo's own `rp2040-demo` fixture.
#[test]
fn test_rp2040_no_relocation_when_vectors_at_flash_base() {
    use crate::system::cortex_m::configure_cortex_m;

    let mut bus = rp2040_bus();

    let mut image = vec![0u8; 0x10];
    let sp = 0x2000_2000u32;
    let reset = 0x1000_0201u32; // thumb bit set
    image[0..4].copy_from_slice(&sp.to_le_bytes());
    image[4..8].copy_from_slice(&reset.to_le_bytes());

    let mut img = crate::memory::ProgramImage::new(reset as u64, crate::Arch::Arm);
    img.add_segment(0x1000_0000, image);

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&img).expect("load firmware");

    assert_eq!(
        machine.cpu.get_pc(),
        0x1000_0200,
        "PC from flash-base vectors"
    );
    assert_eq!(
        machine.bus.read_u32(0xE000_ED08).unwrap(),
        0,
        "VTOR must stay at 0 when no stage-2 relocation is needed"
    );
}
