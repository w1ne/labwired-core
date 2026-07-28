// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Machine-driven TIMER0 proof under walk-free nRF52840 at `rec_tick=512`.
//!
//! Complements the inventory gate (`nrf52840_dk_is_walk_free_and_tick_512`)
//! which only asserts forcer emptiness / `max_safe`. This test exercises the
//! real TIMER model through `Machine::advance` (scheduler drain path) with
//! `peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL`, **not**
//! `tick_peripherals_fully_forced`.
//!
//! Requires `--features event-scheduler`.

#![cfg(feature = "event-scheduler")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::{SystemBus, RECOMMENDED_TICK_INTERVAL};
use labwired_core::snapshot::{ArmCpuSnapshot, CpuSnapshot};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{
    AdvanceRequest, BreakpointPolicy, Bus, Cpu, Machine, SimResult, SimulationConfig,
    SimulationObserver,
};
use std::path::PathBuf;
use std::sync::Arc;

fn root(rel: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../..");
    p.push(rel);
    p
}

/// Minimal cycle-advancing CPU: retires one cycle per step so `Machine`
/// can drain the event scheduler without needing real Thumb firmware.
#[derive(Debug, Default)]
struct CycleCpu {
    pc: u32,
    steps: u32,
}

impl Cpu for CycleCpu {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        self.pc = 0;
        self.steps = 0;
        Ok(())
    }

    fn step(
        &mut self,
        _bus: &mut dyn Bus,
        _observers: &[Arc<dyn SimulationObserver>],
        _config: &SimulationConfig,
    ) -> SimResult<()> {
        self.steps = self.steps.wrapping_add(1);
        self.pc = self.pc.wrapping_add(2);
        Ok(())
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }

    fn set_sp(&mut self, _val: u32) {}

    fn set_exception_pending(&mut self, _exception_num: u32) {}

    fn get_register(&self, id: u8) -> u32 {
        match id {
            0 => self.steps,
            15 => self.pc,
            _ => 0,
        }
    }

    fn set_register(&mut self, id: u8, val: u32) {
        match id {
            0 => self.steps = val,
            15 => self.pc = val,
            _ => {}
        }
    }

    fn snapshot(&self) -> CpuSnapshot {
        let mut registers = vec![0; 16];
        registers[0] = self.steps;
        registers[15] = self.pc;
        CpuSnapshot::Arm(ArmCpuSnapshot {
            registers,
            pc: self.pc,
            xpsr: 0,
            primask: false,
            pending_exceptions: 0,
            pending_exceptions_hi: Vec::new(),
            vtor: 0,
        })
    }

    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::Arm(s) = snapshot {
            self.steps = s.registers.first().copied().unwrap_or(0);
            self.pc = s.pc;
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        (0..=12)
            .map(|id| format!("R{id}"))
            .chain(["SP", "LR", "PC"].into_iter().map(String::from))
            .collect()
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        if name.eq_ignore_ascii_case("PC") {
            return Some(15);
        }
        let id = name
            .strip_prefix('R')
            .or_else(|| name.strip_prefix('r'))?
            .parse::<u8>()
            .ok()?;
        (id <= 12).then_some(id)
    }
}

fn bus_nrf52840_walk_free() -> SystemBus {
    let chip =
        ChipDescriptor::from_file(root("configs/chips/nrf52840.yaml")).expect("load nrf52840 chip");
    let system_path = root("configs/systems/nrf52840-dk.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load nrf52840-dk system");
    let anchored = system_path
        .parent()
        .expect("system parent")
        .join(&manifest.chip);
    manifest.chip = anchored.to_string_lossy().into_owned();
    // Auto-derive walk deletion (never honor a hand walk_deleted hatch).
    manifest.walk_deleted = None;
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build nrf52840 bus");
    // Production Cortex-M bank (SCB/NVIC/DWT); cycle clocks attach via
    // from_config under event-scheduler the same way WASM does.
    let _ = configure_cortex_m(&mut bus);
    bus
}

/// PR-B behavioral gate: TIMER0 COMPARE[0] fires through the Machine
/// scheduler path at `peripheral_tick_interval = 512` on a walk-free
/// nRF52840 DK bus.
#[test]
fn nrf52840_machine_timer0_compare_fires_at_tick_512() {
    let bus = bus_nrf52840_walk_free();
    assert!(
        bus.legacy_walk_disabled,
        "precondition: walk-free auto-derive failed (legacy_walk_disabled=false)"
    );
    assert_eq!(
        bus.max_safe_tick_interval(),
        RECOMMENDED_TICK_INTERVAL,
        "precondition: max_safe must be {RECOMMENDED_TICK_INTERVAL}"
    );

    let mut machine = Machine::new(CycleCpu::default(), bus);
    machine.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.bus.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;

    const TIMER0: u64 = 0x4000_8000;
    const TASKS_START: u64 = TIMER0;
    const TASKS_CLEAR: u64 = TIMER0 + 0x00C;
    const EVENTS_COMPARE0: u64 = TIMER0 + 0x140;
    const INTENSET: u64 = TIMER0 + 0x304;
    const BITMODE: u64 = TIMER0 + 0x508;
    const PRESCALER: u64 = TIMER0 + 0x510;
    const CC0: u64 = TIMER0 + 0x540;

    // Short compare: 32-bit, no prescaler, CC[0]=8 → match after 8 base ticks.
    // INTENSET arms COMPARE[0] IRQ (NVIC line 8); CycleCpu does not model
    // NVIC dispatch, so EVENTS_COMPARE[0] is the observe surface.
    machine.bus.write_u32(BITMODE, 3).unwrap();
    machine.bus.write_u32(PRESCALER, 0).unwrap();
    machine.bus.write_u32(CC0, 8).unwrap();
    machine.bus.write_u32(INTENSET, 1 << 16).unwrap(); // COMPARE[0]
    machine.bus.write_u32(TASKS_CLEAR, 1).unwrap();
    machine.bus.write_u32(TASKS_START, 1).unwrap();

    // Bound: a few tick batches past the compare (8 cycles + one 512-batch
    // of quantisation headroom). Must fire via scheduler drain, not forced walk.
    const CYCLE_BUDGET: u64 = 4_096;
    let mut compare_fired = false;
    let mut cycles_at_fire: Option<u64> = None;

    while machine.total_cycles < CYCLE_BUDGET {
        machine
            .advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("Machine::advance");

        if machine.bus.read_u32(EVENTS_COMPARE0).unwrap_or(0) != 0 {
            compare_fired = true;
            cycles_at_fire = Some(machine.total_cycles);
            break;
        }
    }

    assert!(
        compare_fired,
        "TIMER0 EVENTS_COMPARE[0] never fired within {CYCLE_BUDGET} cycles \
         under Machine + peripheral_tick_interval={RECOMMENDED_TICK_INTERVAL} \
         (total_cycles={}, legacy_walk_disabled={}). \
         Scheduler path must deliver the compare without forced walk.",
        machine.total_cycles, machine.bus.legacy_walk_disabled,
    );
    let at = cycles_at_fire.expect("cycles_at_fire set when compare_fired");
    assert!(
        at <= CYCLE_BUDGET,
        "COMPARE[0] fired at cycle {at}, beyond budget {CYCLE_BUDGET}"
    );
    // Sanity: short CC must not need the full budget.
    assert!(
        at <= 1_024,
        "COMPARE[0] with CC[0]=8 should fire well before 1024 cycles at \
         interval 512 (got total_cycles={at})"
    );
}
