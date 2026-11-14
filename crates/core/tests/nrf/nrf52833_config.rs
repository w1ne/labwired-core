// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Config-build gate for the native nRF52833 / micro:bit v2 descriptors.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn nrf52833_from_config_builds() {
    let sys = workspace_root().join("configs/systems/microbit-v2.yaml");
    let mut manifest =
        SystemManifest::from_file(&sys).unwrap_or_else(|e| panic!("load microbit-v2: {e}"));
    let chip_path = sys.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip for microbit-v2: {e}"));
    manifest.chip = chip_path.to_str().expect("utf-8 chip path").to_string();
    let bus = SystemBus::from_config(&chip, &manifest).expect("nrf52833 / microbit-v2 must build");
    // micro:bit v2 UART is bridged to the interface MCU through UARTE0
    // (target TX = P0.06); the two GPIO ports carry the buttons and matrix.
    assert!(
        bus.find_peripheral_index_by_name("uart0").is_some(),
        "bus must expose uart0 (UARTE0 console)"
    );
    assert!(
        bus.find_peripheral_index_by_name("gpio0").is_some(),
        "bus must expose gpio0 (P0)"
    );
    assert!(
        bus.find_peripheral_index_by_name("gpio1").is_some(),
        "bus must expose gpio1 (P1)"
    );
    assert!(
        bus.find_peripheral_index_by_name("clock").is_some(),
        "bus must expose clock"
    );
}

/// A firmware image's UICR record must survive `Machine::load_firmware`.
///
/// Segments that miss flash and RAM are written byte by byte through the bus;
/// the UICR model used to accept byte writes and drop them, so the record the
/// micro:bit V2 MakeCode hex carries (NRFFW[0] = 0x77000, the bootloader start;
/// NRFFW[1] = 0x7E000, the MBR params page) vanished without a warning. CODAL
/// computes its flash-storage page from NRFFW[0] and hard-faulted on the
/// erased 0xFFFFFFFF.
#[test]
fn nrf52833_load_firmware_programs_the_uicr_record() {
    use labwired_core::{cpu::cortex_m::CortexM, memory::ProgramImage, Arch, Bus, Machine};

    let sys = workspace_root().join("configs/systems/microbit-v2.yaml");
    let mut manifest = SystemManifest::from_file(&sys).expect("load microbit-v2");
    let chip_path = sys.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nrf52833 chip");
    manifest.chip = chip_path.to_str().expect("utf-8 chip path").to_string();
    let bus = SystemBus::from_config(&chip, &manifest).expect("nrf52833 bus");
    let mut machine = Machine::new(CortexM::new(), bus);

    let mut image = ProgramImage::new(0x101, Arch::Arm);
    // Minimal vector table: SP, reset -> 0x100 (thumb).
    let mut flash = vec![0u8; 0x104];
    flash[0..4].copy_from_slice(&0x2000_4000u32.to_le_bytes());
    flash[4..8].copy_from_slice(&0x0000_0101u32.to_le_bytes());
    flash[0x100..0x102].copy_from_slice(&0xE7FEu16.to_le_bytes()); // b .
    image.add_segment(0, flash);
    // The hex record `:081014000070070000E0070076`.
    image.add_segment(
        0x1000_1014,
        vec![0x00, 0x70, 0x07, 0x00, 0x00, 0xE0, 0x07, 0x00],
    );
    machine.load_firmware(&image).expect("load firmware");

    assert_eq!(
        machine.bus.read_u32(0x1000_1014).unwrap(),
        0x0007_7000,
        "UICR NRFFW[0] must hold the image's bootloader address"
    );
    assert_eq!(machine.bus.read_u32(0x1000_1018).unwrap(), 0x0007_E000);
    assert_eq!(
        machine.bus.read_u32(0x1000_101C).unwrap(),
        0xFFFF_FFFF,
        "fields the image does not program stay erased"
    );
}

/// AIRCR.SYSRESETREQ is a system reset: VTOR returns to its boot value and
/// the NVIC forgets every enable and pending bit.
///
/// A CODAL micro:bit V2 image moves its vector table to SRAM, then reboots
/// itself on first boot after programming UICR. The reset used to reload only
/// the CPU, so the new boot fetched SP/PC and every handler from the stale
/// SRAM table (zeroed by the new boot's startup), and the still-enabled,
/// still-pending TIMER1 IRQ vectored to 0x00000000.
#[test]
fn nrf52833_sysresetreq_resets_vtor_and_nvic() {
    use labwired_core::{
        memory::ProgramImage, system::cortex_m::configure_cortex_m, Arch, Bus, Cpu, Machine,
    };

    let sys = workspace_root().join("configs/systems/microbit-v2.yaml");
    let mut manifest = SystemManifest::from_file(&sys).expect("load microbit-v2");
    let chip_path = sys.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nrf52833 chip");
    manifest.chip = chip_path.to_str().expect("utf-8 chip path").to_string();
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("nrf52833 bus");
    // The CLI/wasm builders install the Cortex-M core peripherals (NVIC,
    // SCB) this way; `from_config` alone does not map them.
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    let mut image = ProgramImage::new(0x101, Arch::Arm);
    let mut flash = vec![0u8; 0x104];
    flash[0..4].copy_from_slice(&0x2000_4000u32.to_le_bytes());
    flash[4..8].copy_from_slice(&0x0000_0101u32.to_le_bytes());
    flash[0x100..0x102].copy_from_slice(&0xE7FEu16.to_le_bytes()); // b .
    image.add_segment(0, flash);
    machine.load_firmware(&image).expect("load firmware");
    assert_eq!(machine.bus.read_u32(0xE000_ED08).unwrap(), 0, "boot VTOR");

    // What the previous boot left behind: VTOR on an SRAM table (all zero
    // here, as after the next boot's .bss clear), TIMER1 (IRQ 9) enabled and
    // pending at a raised priority. (This chip maps no SysTick model.)
    const TIMER1_IRQ: u32 = 9;
    machine.bus.write_u32(0xE000_ED08, 0x2000_0200).unwrap();
    machine.bus.write_u32(0xE000_E100, 1 << TIMER1_IRQ).unwrap(); // ISER0
    machine.bus.write_u32(0xE000_E200, 1 << TIMER1_IRQ).unwrap(); // ISPR0
    machine.bus.write_u32(0xE000_E408, 0x0000_E000).unwrap(); // IPR2: IRQ9

    machine.bus.write_u32(0xE000_ED0C, 0x05FA_0004).unwrap(); // SYSRESETREQ
    machine.step().expect("step into the reset boundary");

    assert_eq!(
        machine.bus.read_u32(0xE000_ED08).unwrap(),
        0,
        "VTOR must return to its boot value"
    );
    assert_eq!(
        machine.bus.read_u32(0xE000_E100).unwrap(),
        0,
        "ISER0 cleared"
    );
    assert_eq!(
        machine.bus.read_u32(0xE000_E200).unwrap(),
        0,
        "ISPR0 cleared"
    );
    for _ in 0..16 {
        machine.step().expect("step after reset");
    }
    assert_eq!(
        machine.cpu.get_pc(),
        0x100,
        "the new boot runs the flash reset handler, not a stale SRAM vector"
    );
}

/// UARTE0 (IRQ 2) EasyDMA TX completion reaches the CPU when INTEN.ENDTX is
/// set: the core leaves `main` for the UARTE0 handler.
///
/// CODAL's NRF52Serial on the micro:bit V2 sends one byte per STARTTX and
/// queues the next from this interrupt; without it `serial.writeLine("boot")`
/// printed "b" and the program waited in WFE forever.
#[test]
fn nrf52833_uarte0_endtx_interrupt_is_delivered() {
    use labwired_core::{
        memory::ProgramImage, system::cortex_m::configure_cortex_m, Arch, Bus, Cpu, Machine,
    };

    let sys = workspace_root().join("configs/systems/microbit-v2.yaml");
    let mut manifest = SystemManifest::from_file(&sys).expect("load microbit-v2");
    let chip_path = sys.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nrf52833 chip");
    manifest.chip = chip_path.to_str().expect("utf-8 chip path").to_string();
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("nrf52833 bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    // Vector table: reset -> 0x100 (`b .`), IRQ 2 (vector 18) -> 0x200 (`b .`).
    let mut flash = vec![0u8; 0x204];
    flash[0..4].copy_from_slice(&0x2000_4000u32.to_le_bytes());
    flash[4..8].copy_from_slice(&0x0000_0101u32.to_le_bytes());
    flash[18 * 4..18 * 4 + 4].copy_from_slice(&0x0000_0201u32.to_le_bytes());
    flash[0x100..0x102].copy_from_slice(&0xE7FEu16.to_le_bytes());
    flash[0x200..0x202].copy_from_slice(&0xE7FEu16.to_le_bytes());
    let mut image = ProgramImage::new(0x101, Arch::Arm);
    image.add_segment(0, flash);
    machine.load_firmware(&image).expect("load firmware");

    const UARTE0: u64 = 0x4000_2000;
    machine.bus.write_u8(0x2000_0100, b'b').unwrap();
    machine.bus.write_u32(0xE000_E100, 1 << 2).unwrap(); // NVIC ISER0: IRQ 2
    machine.bus.write_u32(UARTE0 + 0x500, 8).unwrap(); // ENABLE = UARTE
    machine.bus.write_u32(UARTE0 + 0x544, 0x2000_0100).unwrap(); // TXD.PTR
    machine.bus.write_u32(UARTE0 + 0x548, 1).unwrap(); // TXD.MAXCNT
    machine.bus.write_u32(UARTE0 + 0x304, 1 << 8).unwrap(); // INTENSET = ENDTX
    machine.bus.write_u32(UARTE0 + 0x008, 1).unwrap(); // TASKS_STARTTX

    // One byte at the reset baud (250000) is a 2560-cycle frame, then the
    // NVIC takes the exception. Feature-off completes on the next tick.
    for _ in 0..6_000 {
        machine.step().expect("step");
    }
    assert_eq!(
        machine.bus.read_u32(UARTE0 + 0x120).unwrap(),
        1,
        "EVENTS_ENDTX"
    );
    assert_eq!(
        machine.cpu.get_pc(),
        0x200,
        "the ENDTX interrupt must take the core into the UARTE0 handler"
    );
}
