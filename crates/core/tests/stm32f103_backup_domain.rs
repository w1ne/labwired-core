// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32F103 backup domain through the shipped chip config: the BKP data
//! registers are clock-gated by RCC_APB1ENR.BKPEN, write-protected until
//! PWR_CR.DBP, and survive AIRCR.SYSRESETREQ while DBP re-locks (RM0008 §5-6).
//!
//! The stock STM32duino sketch behind this (enableBackupDomain /
//! setBackupRegister / NVIC_SystemReset) counts boots 0..3 on this model; on
//! the old read-as-zero BKP stub it reset-looped at boot 0.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, memory::ProgramImage, system::cortex_m::configure_cortex_m, Arch, Bus, Machine,
};

const RCC_APB1ENR: u64 = 0x4002_101C;
const PWR_CR: u64 = 0x4000_7000;
const BKP_DR1: u64 = 0x4000_6C04;
const BKP_DR10: u64 = 0x4000_6C28;
const BKP_DR11: u64 = 0x4000_6C40;
const BKPEN: u32 = 1 << 27;
const PWREN: u32 = 1 << 28;
const DBP: u32 = 1 << 8;

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A machine parked on a `b .` loop in flash, so the host drives MMIO.
fn idle_machine() -> Machine<labwired_core::cpu::cortex_m::CortexM> {
    let chip = ChipDescriptor::from_file(root().join("configs/chips/stm32f103.yaml")).unwrap();
    let manifest =
        SystemManifest::from_file(root().join("validation/arduino-matrix/systems/stm32f103.yaml"))
            .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let mut bytes = vec![0u8; 0x104];
    bytes[0..4].copy_from_slice(&0x2000_4000u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&0x0800_0101u32.to_le_bytes()); // Thumb entry at 0x100
    bytes[0x100..0x102].copy_from_slice(&0xe7feu16.to_le_bytes()); // b .
    let mut image = ProgramImage::new(0x0800_0101, Arch::Arm);
    image.add_segment(0x0800_0000, bytes);
    machine.load_firmware(&image).unwrap();
    machine
}

#[test]
fn bkp_is_clock_gated_and_write_protected_until_dbp() {
    let mut machine = idle_machine();
    let bus = &mut machine.bus;
    assert_eq!(bus.read_u32(PWR_CR).unwrap(), 0, "F1 PWR_CR resets to 0");

    bus.write_u32(RCC_APB1ENR, BKPEN | PWREN).unwrap();
    bus.write_u32(BKP_DR1, 0x1234).unwrap();
    assert_eq!(
        bus.read_u32(BKP_DR1).unwrap(),
        0,
        "DR1 write dropped without DBP"
    );

    bus.write_u32(PWR_CR, DBP).unwrap();
    bus.write_u32(BKP_DR1, 0xFFFF_1234).unwrap();
    bus.write_u32(BKP_DR10, 0x0A0A).unwrap();
    bus.write_u32(BKP_DR11, 0x0B0B).unwrap();
    assert_eq!(bus.read_u32(BKP_DR1).unwrap(), 0x1234, "DR1 is 16 bits");
    assert_eq!(bus.read_u32(BKP_DR10).unwrap(), 0x0A0A, "DR10");
    assert_eq!(
        bus.read_u32(BKP_DR11).unwrap(),
        0,
        "no DR11 on a medium-density F103"
    );

    bus.write_u32(RCC_APB1ENR, PWREN).unwrap();
    assert_eq!(bus.read_u32(BKP_DR1).unwrap(), 0, "unclocked BKP reads 0");
}

#[test]
fn sysresetreq_keeps_bkp_data_and_relocks_dbp() {
    let mut machine = idle_machine();
    let bus = &mut machine.bus;
    bus.write_u32(RCC_APB1ENR, BKPEN | PWREN).unwrap();
    bus.write_u32(PWR_CR, DBP).unwrap();
    bus.write_u32(BKP_DR1, 0x0003).unwrap();

    machine.bus.write_u32(0xE000_ED0C, 0x05FA_0004).unwrap(); // SYSRESETREQ
    machine.step().expect("step into the reset boundary");

    let bus = &mut machine.bus;
    bus.write_u32(RCC_APB1ENR, BKPEN | PWREN).unwrap();
    assert_eq!(
        bus.read_u32(PWR_CR).unwrap() & DBP,
        0,
        "system reset clears DBP"
    );
    assert_eq!(
        bus.read_u32(BKP_DR1).unwrap(),
        0x0003,
        "DR1 survives system reset"
    );
    bus.write_u32(BKP_DR1, 0xBEEF).unwrap();
    assert_eq!(
        bus.read_u32(BKP_DR1).unwrap(),
        0x0003,
        "DBP is locked again"
    );
}
