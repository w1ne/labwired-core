// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! TIMER_IRQ_0 must wake a real Cortex-M0+ from WFE through the RP2040
//! scheduler path (the MicroPython `best_effort_wfe_or_timeout` shape).
//!
//! Complements `rp2040_timer_machine_gate`, which only observes `INTR` on a
//! cycle-stepping stand-in CPU, and `cortex_m_event_sleep`, which never
//! attaches the RP2040 timer. This drives `Machine::advance` at
//! `RECOMMENDED_TICK_INTERVAL` with idle fast-forward on — the same
//! acceleration MicroPython labs use — and asserts the core leaves WFE and
//! takes `TIMER_IRQ_0` when ALARM0 matches.

#![cfg(feature = "event-scheduler")]

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::{SystemBus, RECOMMENDED_TICK_INTERVAL};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, BreakpointPolicy, Bus, Cpu, Machine};

const TIMER: u64 = 0x4005_4000;
const ALARM0: u64 = TIMER + 0x10;
const INTE: u64 = TIMER + 0x38;
const INTR: u64 = TIMER + 0x34;
const NVIC_ISER: u64 = 0xE000_E100;

const SRAM: u32 = 0x2000_0000;
const CODE: u32 = 0x2000_0100;
const IRQ0: u32 = 0x2000_0200;
const STACK: u32 = 0x2000_4000;

fn machine_rp2040() -> Machine<CortexM> {
    std::env::set_var("LABWIRED_RP2040_BOOTROM", "");
    let chip = ChipDescriptor::from_file(root("configs/chips/rp2040.yaml")).expect("load rp2040");
    let system_path = root("configs/systems/rp2040-pico.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load rp2040-pico system");
    let anchored = system_path
        .parent()
        .expect("system parent")
        .join(&manifest.chip);
    manifest.chip = anchored.to_string_lossy().into_owned();
    manifest.walk_deleted = None;
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build rp2040 bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    Machine::new(cpu, bus)
}

fn plant_wfe_and_irq0_stub(machine: &mut Machine<CortexM>) {
    // Vector table in SRAM so taking TIMER_IRQ_0 does not fetch bootrom.
    for i in 0..32u32 {
        let entry = if i == 0 {
            STACK
        } else if i == 16 {
            IRQ0 | 1
        } else {
            (CODE + 4) | 1
        };
        machine
            .bus
            .write_u32(u64::from(SRAM + i * 4), entry)
            .expect("plant vector");
    }
    machine.cpu.set_vtor(SRAM);
    // Reset/thread stub: WFE, then an infinite loop we must not sit in once
    // the alarm is armed (the IRQ handler is the success observe).
    machine.bus.write_u16(u64::from(CODE), 0xBF20).unwrap(); // WFE
    machine.bus.write_u16(u64::from(CODE + 2), 0xE7FE).unwrap(); // B .
    // IRQ0 stub: B .
    machine.bus.write_u16(u64::from(IRQ0), 0xE7FE).unwrap();
}

/// Alarm0 + WFE + idle-FF must deliver TIMER_IRQ_0 and leave the WFE park.
#[test]
fn rp2040_timer_alarm0_wakes_cortex_m_wfe() {
    let mut machine = machine_rp2040();
    plant_wfe_and_irq0_stub(&mut machine);
    machine.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.bus.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.config.idle_fast_forward_enabled = true;

    machine.cpu.set_pc(CODE);
    machine.cpu.set_sp(STACK);
    machine.bus.write_u32(NVIC_ISER, 1).unwrap();

    machine.step().expect("execute WFE");
    assert!(
        machine.cpu.waiting_for_event(),
        "WFE must park before the alarm is programmed (pc {:#x})",
        machine.cpu.get_pc()
    );

    // 8 µs → 1000 CPU cycles at 125 MHz, same target as the machine gate.
    machine.bus.write_u32(INTE, 1).unwrap();
    machine.bus.write_u32(ALARM0, 8).unwrap();

    const CYCLE_BUDGET: u64 = 4_096;
    while machine.total_cycles < CYCLE_BUDGET {
        machine
            .advance(
                AdvanceRequest::run(Some(RECOMMENDED_TICK_INTERVAL.into()))
                    .with_breakpoints(BreakpointPolicy::Ignore),
            )
            .expect("Machine::advance");
        if machine.cpu.get_pc() == IRQ0 {
            break;
        }
    }

    assert_eq!(
        machine.cpu.get_pc(),
        IRQ0,
        "TIMER_IRQ_0 handler never entered within {CYCLE_BUDGET} cycles \
         (pc {:#x}, waiting_for_event={}, INTR={:#x}, total_cycles={})",
        machine.cpu.get_pc(),
        machine.cpu.waiting_for_event(),
        machine.bus.read_u32(INTR).unwrap_or(0),
        machine.total_cycles,
    );
    assert!(
        !machine.cpu.waiting_for_event(),
        "core must leave WFE when TIMER_IRQ_0 is taken"
    );
}
