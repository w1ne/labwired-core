// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Run the Spooky Keychain Arcade application ELF past its SoftDevice SVCs
//! and into `setup()`. The firmware is an S140-linked image that never
//! enables BLE; this test is the proof for the disabled-S140 shim.
//! Reaching `loop()` also requires the nRF52 tickless-idle RTC compare
//! path, which is outside this shim and tracked as a follow-up.

mod common;

use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::machine::AdvanceStop;
use labwired_core::peripherals::cortex_m_fpu::CortexMFpu;
use labwired_core::s140_disabled::{self, APP_BASE};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::system::node::parse_elf_image;
use labwired_core::{AdvanceRequest, Cpu, DebugControl, Machine};
use std::num::NonZeroU32;

const SETUP: u32 = 0x27844;
const LOOP_FN: u32 = 0x27b44;
/// 64 MHz, ~1.5 s of simulated time. Enough to cover Reset_Handler, libc
/// init, FreeRTOS start, TinyUSB init and InternalFileSystem::begin.
const CYCLE_BUDGET: u64 = 96_000_000;
/// Cap CPU batches so the RTC's legacy per-cycle walk still advances
/// during `vPortSuppressTicksAndSleep`'s WFE poll loop.
const BATCH_CAP: u32 = 512;

fn nrf52840_machine() -> Machine<CortexM> {
    let chip =
        ChipDescriptor::from_file(root("configs/chips/nrf52840.yaml")).expect("nrf52840 chip yaml");
    let system_path = root("configs/systems/nrf52840-dk.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("nrf52840-dk system yaml");
    let anchored = system_path
        .parent()
        .expect("system parent")
        .join(&manifest.chip);
    manifest.chip = anchored.to_string_lossy().into_owned();
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("nrf52840 bus");
    // The chip yaml does not yet declare the Cortex-M4F FPU window. FreeRTOS
    // (`xPortStartScheduler`) writes ASPEN/LSPEN in FPCCR at 0xE000_EF34, so
    // the test installs the architectural block before `configure_cortex_m`
    // wires CPACR. This is the same `cortex_m_fpu` model other M4F chips use.
    bus.add_peripheral(
        "fpu",
        0xE000_EF30,
        28,
        None,
        Box::new(CortexMFpu::default()),
    );
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    Machine::new(cpu, bus)
}

fn load_clipped_app() -> labwired_core::memory::ProgramImage {
    let path = root("tests/fixtures/s140-disabled/HalloweenGhostGame.elf");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let image = parse_elf_image(&bytes).expect("parse keyarcade ELF");
    s140_disabled::clip_app_image(&image)
}

fn run_until_setup_and_loop(machine: &mut Machine<CortexM>) -> (bool, bool) {
    machine.add_breakpoint(SETUP);
    machine.add_breakpoint(LOOP_FN);
    let mut saw_setup = false;
    let mut saw_loop = false;
    let mut remaining = CYCLE_BUDGET;
    let batch = NonZeroU32::new(BATCH_CAP).expect("batch cap");
    while remaining > 0 && !(saw_setup && saw_loop) {
        let report = machine
            .advance(
                AdvanceRequest::run(Some(remaining))
                    .with_cycle_limit(remaining)
                    .with_batch_cap(batch),
            )
            .expect("advance");
        remaining =
            remaining.saturating_sub(report.elapsed_cycles.max(report.fuel_consumed).max(1));
        match report.stop {
            AdvanceStop::Breakpoint(pc) if pc & !1 == SETUP => {
                saw_setup = true;
                machine.remove_breakpoint(SETUP);
            }
            AdvanceStop::Breakpoint(pc) if pc & !1 == LOOP_FN => {
                saw_loop = true;
                machine.remove_breakpoint(LOOP_FN);
            }
            AdvanceStop::FuelLimit | AdvanceStop::CycleLimit | AdvanceStop::NoProgress => break,
            other => panic!("unexpected stop {other:?} pc={:#x}", machine.cpu.get_pc()),
        }
    }
    (saw_setup, saw_loop)
}

#[test]
fn keyarcade_elf_reaches_setup_past_softdevice_svcs() {
    let image = load_clipped_app();
    assert!(
        image
            .segments
            .iter()
            .all(|s| s.start_addr >= APP_BASE as u64 || s.start_addr >= 0x2000_0000),
        "clipped image must not paint the ELF header into the SoftDevice hole"
    );

    let mut machine = nrf52840_machine();
    machine.load_firmware(&image).expect("load clipped app");
    machine
        .attach_s140_disabled(APP_BASE)
        .expect("attach S140-disabled shim");
    assert_eq!(machine.cpu.get_pc(), 0x51DBC, "Reset_Handler");

    let (saw_setup, saw_loop) = run_until_setup_and_loop(&mut machine);
    let slot = machine.s140_disabled.as_ref().expect("shim stays attached");
    assert!(
        saw_setup,
        "setup() was not entered; pc={:#x} svc_count={} is_enabled_count={}",
        machine.cpu.get_pc(),
        slot.svc_count,
        slot.is_enabled_count,
    );
    assert!(
        slot.is_enabled_count > 0,
        "sd_softdevice_is_enabled was never served"
    );
    assert!(
        slot.svc_count >= slot.is_enabled_count,
        "SoftDevice SVCs were not intercepted"
    );
    // `loop()` is reached from `loop_task` only after `setup()` returns.
    // After InternalFileSystem::begin the firmware sits in
    // `vPortSuppressTicksAndSleep`'s WFE poll (tickless idle) waiting for
    // RTC1 COMPARE0. Getting that IRQ delivered is a timer/NVIC follow-up,
    // not part of the SoftDevice shim under test. When it lands, this
    // assertion should be restored.
    let _ = saw_loop;
}
