// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Executing core-timer oracle for the two public DevKitC-32E profiles.
//!
//! Each exact descriptor/system builds the classic ESP32 runtime. Real Xtensa
//! instructions spin until CCOMPARE0 raises timer interrupt 6, then the handler
//! records CCOUNT in RAM. Every instruction boundary is compared with the
//! explicit simulator timer/vector contract; a disarmed-timer run is the
//! negative control. This is an engine oracle, not a silicon timing capture or
//! evidence for peripherals outside the profile's documented scope.

use labwired_config::{Arch, ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::xtensa_lx7::XtensaLx7;
use labwired_core::cpu::xtensa_sr::{CCOMPARE0, CCOUNT, INTENABLE, VECBASE};
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::{Bus, Cpu, DebugControl, Machine};
use std::path::PathBuf;

const ENTRY: u32 = 0x4008_0000;
const ISR: u32 = ENTRY + 0x1000;
const WITNESS: u64 = 0x3FFB_0000;
const UNSET: u32 = 0xDEAD_BEEF;
const DEADLINE: u32 = 32;

fn build(chip_path: &str, system_path: &str, armed: bool) -> Machine<XtensaLx7> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join(chip_path)).unwrap();
    assert_eq!(chip.arch, Arch::Xtensa);
    assert!(
        !chip.is_esp32s3(),
        "a classic profile must not select the S3 map"
    );
    assert_eq!(
        (chip.flash.base, chip.flash.size),
        (0x400D_0000, 4 * 1024 * 1024)
    );
    assert_eq!((chip.ram.base, chip.ram.size), (0x3FFB_0000, 192 * 1024));
    let mut system = SystemManifest::from_file(root.join(system_path)).unwrap();
    assert_eq!(
        root.join(system_path)
            .parent()
            .unwrap()
            .join(&system.chip)
            .canonicalize()
            .unwrap(),
        root.join(chip_path).canonicalize().unwrap(),
        "the system must select the exact profile under test",
    );
    system.chip = root.join(chip_path).to_string_lossy().into_owned();
    let mut bus = SystemBus::from_config(&chip, &system).unwrap();
    let mut cpu = configure_xtensa_esp32(&mut bus);

    // j .; the ISR executes rsr a2,ccount; s32i.n a2,a3,0; j .
    for (offset, byte) in [0x06, 0xff, 0xff].iter().enumerate() {
        bus.write_u8(u64::from(ENTRY) + offset as u64, *byte)
            .unwrap();
    }
    for (offset, byte) in [0x20, CCOUNT as u8, 0x03, 0x29, 0x03, 0x06, 0xff, 0xff]
        .iter()
        .enumerate()
    {
        bus.write_u8(u64::from(ISR) + offset as u64, *byte).unwrap();
    }
    bus.write_u32(WITNESS, UNSET).unwrap();
    cpu.sr.write(VECBASE, ISR - 0x300);
    cpu.sr.write(INTENABLE, 1 << 6);
    cpu.sr.write(CCOUNT, 0);
    cpu.sr.write(CCOMPARE0, if armed { DEADLINE } else { 0 });
    cpu.regs.write_logical(3, WITNESS as u32);
    cpu.ps.set_intlevel(0);
    cpu.ps.set_excm(false);
    cpu.set_pc(ENTRY);
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    machine
}

fn assert_timer_oracle(chip: &str, system: &str) {
    for armed in [false, true] {
        let mut machine = build(chip, system, armed);
        for step in 1..=64 {
            machine.run(Some(1)).unwrap();
            assert_eq!(
                machine.cpu.sr.read(CCOUNT),
                step,
                "{chip}: timer step {step}"
            );
            let expected_pc = if !armed || step < DEADLINE {
                ENTRY
            } else if step == DEADLINE {
                ISR
            } else if step == DEADLINE + 1 {
                ISR + 3
            } else {
                ISR + 5
            };
            assert_eq!(
                machine.get_pc(),
                expected_pc,
                "{chip}: vector trace at step {step}"
            );
            let expected_witness = if armed && step >= DEADLINE + 2 {
                DEADLINE + 1
            } else {
                UNSET
            };
            assert_eq!(
                machine.bus.read_u32(WITNESS).unwrap(),
                expected_witness,
                "{chip}: executed ISR witness at step {step}"
            );
        }
    }
}

#[test]
fn devkitc_gpio_profile_core_timer_exec_oracle() {
    assert_timer_oracle(
        "configs/chips/esp32-devkitc-32e-gpio-smoke.yaml",
        "configs/systems/esp32-devkitc-32e-gpio-smoke.yaml",
    );
}

#[test]
fn devkitc_uart1_profile_core_timer_exec_oracle() {
    assert_timer_oracle(
        "configs/chips/esp32-devkitc-32e-uart1.yaml",
        "configs/systems/esp32-devkitc-32e-uart1.yaml",
    );
}
