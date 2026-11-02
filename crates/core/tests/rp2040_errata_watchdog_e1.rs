// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Executable model check for RP2040 erratum E1.
//!
//! Datasheet Appendix B, RP2040-E1: *"There is a logic error which means the
//! counter is decremented twice per tick, instead of once per tick."* The chip
//! descriptor is the corpus entry point for that erratum, and this test is what
//! makes it executable rather than a footnote:
//!
//! 1. the descriptor really declares `RP2040-E1` on the watchdog instance, and
//!    the `model_check` it names really exists in the tree;
//! 2. the watchdog, driven over its real MMIO registers, really counts down in
//!    TWOS — i.e. the model reproduces the vendor defect and not the
//!    datasheet's idealised "once per tick".
//!
//! If a future edit makes the model count down once per tick, step 2 fails and
//! the `proven` claim for this erratum has to be withdrawn. That is the point:
//! the corpus row and the behaviour cannot drift apart silently.
//!
//! Deliberately NOT behind the `event-scheduler` feature, and driven through
//! `Machine::advance` rather than `SystemBus::tick_peripherals_fully_forced`.
//! With the scheduler the watchdog has a cycle clock attached and its legacy
//! `tick()` is a no-op (the countdown rides scheduled events), so a forced
//! walk leaves the counter at 100. `Machine::advance` walks the peripherals in
//! the feature-off build and delivers the scheduled events in the scheduler
//! build, so the same ten-cycle run exercises whichever path the build uses.

mod common;

use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::snapshot::{ArmCpuSnapshot, CpuSnapshot};
use labwired_core::{
    AdvanceRequest, BreakpointPolicy, Bus, Cpu, Machine, SimResult, SimulationConfig,
    SimulationObserver,
};
use std::sync::Arc;

const WATCHDOG_BASE: u64 = 0x4005_8000;
const CTRL: u64 = WATCHDOG_BASE;
const LOAD: u64 = WATCHDOG_BASE + 0x04;
const TICK: u64 = WATCHDOG_BASE + 0x2C;
const CTRL_ENABLE: u32 = 1 << 30;
const TICK_ENABLE: u32 = 1 << 9;
const MODEL_CHECK: &str =
    "crates/core/src/peripherals/rp2040/watchdog.rs::tests::counter_decrements_twice_per_tick";

/// Minimal cycle-advancing CPU: `Machine::advance` needs a core to step, and
/// this one does nothing but burn one cycle per step.
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
            waiting_for_event: false,
            event_register: false,
        })
    }

    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::Arm(snapshot) = snapshot {
            self.steps = snapshot.registers.first().copied().unwrap_or(0);
            self.pc = snapshot.pc;
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

fn rp2040_bus() -> SystemBus {
    let system_path = root("configs/systems/rp2040-pico.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load rp2040-pico");
    let chip_path = system_path
        .parent()
        .expect("system manifest parent")
        .join(&manifest.chip);
    manifest.chip = chip_path.to_string_lossy().into_owned();
    let chip = ChipDescriptor::from_file(chip_path).expect("load rp2040 descriptor");
    SystemBus::from_config(&chip, &manifest).expect("build rp2040 bus")
}

/// The descriptor declares the erratum, and the test it names exists.
#[test]
fn rp2040_e1_is_declared_on_the_watchdog_descriptor() {
    let chip = ChipDescriptor::from_file(root("configs/chips/rp2040.yaml"))
        .expect("load real rp2040 descriptor");
    let watchdog = chip
        .peripherals
        .iter()
        .find(|peripheral| peripheral.id == "watchdog")
        .expect("rp2040 descriptor declares watchdog");
    let errata = watchdog
        .config
        .get("errata")
        .and_then(serde_yaml::Value::as_sequence)
        .expect("watchdog errata block survives descriptor parsing");
    let e1 = errata
        .iter()
        .find(|entry| entry.get("id").and_then(serde_yaml::Value::as_str) == Some("RP2040-E1"))
        .expect("watchdog declares RP2040-E1");
    assert_eq!(
        e1.get("model_check").and_then(serde_yaml::Value::as_str),
        Some(MODEL_CHECK),
        "descriptor must point at this stable model check"
    );

    let (source_path, test_path) = MODEL_CHECK.split_once("::").expect("model check path");
    let source = std::fs::read_to_string(root(source_path)).expect("model-check source exists");
    let test_name = test_path.rsplit("::").next().expect("test name");
    assert!(
        source.contains("mod tests") && source.contains(&format!("fn {test_name}")),
        "model-check test does not exist at {MODEL_CHECK}"
    );
}

/// The model reproduces the defect: 100 loaded counts, ten ticks, 80 left.
#[test]
fn the_watchdog_counts_down_twice_per_tick() {
    let mut bus = rp2040_bus();
    // RP2040 datasheet §4.7: TICK @ +0x2c, LOAD @ +0x04, CTRL @ +0x00.
    // A divide ratio of one produces one watchdog tick per peripheral tick.
    bus.write_u32(TICK, 1 | TICK_ENABLE).expect("program TICK");
    bus.write_u32(LOAD, 100).expect("load watchdog counter");
    bus.write_u32(CTRL, CTRL_ENABLE).expect("enable watchdog");

    let mut machine = Machine::new(CycleCpu::default(), bus);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    machine
        .advance(AdvanceRequest::run(Some(10_u64)).with_breakpoints(BreakpointPolicy::Ignore))
        .expect("advance watchdog model");

    let remaining = machine.bus.read_u32(CTRL).expect("read CTRL") & 0x00FF_FFFF;
    assert_eq!(
        remaining, 80,
        "RP2040-E1 model check: after ten ticks, 100 loaded counts must decrement by two each tick"
    );
}
