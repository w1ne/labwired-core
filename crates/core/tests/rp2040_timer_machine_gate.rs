// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Machine-driven TIMER ALARM0 proof under walk-free RP2040 at `rec_tick=512`.
//!
//! Complements the inventory gate (`rp2040_pico_is_walk_free_and_tick_512`)
//! which only asserts forcer emptiness / `max_safe`. This test exercises the
//! real RP2040 TIMER model through `Machine::advance` (scheduler drain path)
//! with `peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL`, **not**
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

fn bus_rp2040_walk_free() -> SystemBus {
    // Opt out of in-tree bootrom so the peripheral set matches inventory /
    // production assembly (bootrom is not a walk forcer).
    std::env::set_var("LABWIRED_RP2040_BOOTROM", "");
    let chip = ChipDescriptor::from_file(root("configs/chips/rp2040.yaml")).expect("load rp2040");
    let system_path = root("configs/systems/rp2040-pico.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load rp2040-pico system");
    let anchored = system_path
        .parent()
        .expect("system parent")
        .join(&manifest.chip);
    manifest.chip = anchored.to_string_lossy().into_owned();
    // Auto-derive walk deletion (never honor a hand walk_deleted hatch).
    manifest.walk_deleted = None;
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build rp2040 bus");
    let _ = configure_cortex_m(&mut bus);
    bus
}

/// PR-C behavioral gate: TIMER ALARM0 fires through the Machine scheduler
/// path at `peripheral_tick_interval = 512` on a walk-free RP2040 Pico bus.
#[test]
fn rp2040_machine_timer_alarm0_fires_at_tick_512() {
    let bus = bus_rp2040_walk_free();
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

    // RP2040 TIMER base (datasheet §4.6).
    const TIMER: u64 = 0x4005_4000;
    const ALARM0: u64 = TIMER + 0x10;
    const INTR: u64 = TIMER + 0x34;
    const INTE: u64 = TIMER + 0x38;
    const TIMERAWL: u64 = TIMER + 0x28;

    // Short alarm: target low==8. Counter starts at 0 and advances 1 per CPU
    // cycle under the scheduler path. INTE arms alarm-0 → TIMER_IRQ_0; CycleCpu
    // does not model NVIC dispatch, so INTR is the observe surface.
    machine.bus.write_u32(INTE, 1).unwrap();
    machine.bus.write_u32(ALARM0, 8).unwrap();

    const CYCLE_BUDGET: u64 = 4_096;
    let mut alarm_fired = false;
    let mut cycles_at_fire: Option<u64> = None;

    while machine.total_cycles < CYCLE_BUDGET {
        machine
            .advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("Machine::advance");

        if machine.bus.read_u32(INTR).unwrap_or(0) & 1 != 0 {
            alarm_fired = true;
            cycles_at_fire = Some(machine.total_cycles);
            break;
        }
    }

    assert!(
        alarm_fired,
        "TIMER ALARM0 INTR never latched within {CYCLE_BUDGET} cycles \
         under Machine + peripheral_tick_interval={RECOMMENDED_TICK_INTERVAL} \
         (total_cycles={}, TIMERAWL={:#x}, legacy_walk_disabled={}). \
         Scheduler path must deliver the alarm without forced walk.",
        machine.total_cycles,
        machine.bus.read_u32(TIMERAWL).unwrap_or(0),
        machine.bus.legacy_walk_disabled,
    );
    let at = cycles_at_fire.expect("cycles_at_fire set when alarm_fired");
    assert!(
        at <= CYCLE_BUDGET,
        "ALARM0 fired at cycle {at}, beyond budget {CYCLE_BUDGET}"
    );
    // Sanity: short alarm must not need the full budget.
    assert!(
        at <= 1_024,
        "ALARM0 with target=8 should fire well before 1024 cycles at \
         interval 512 (got total_cycles={at})"
    );
}
