// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! EasyDMA completion fidelity under walk-free nRF52840 at `rec_tick=512`.
//!
//! Before dual-path scheduler promotion, UARTE/SAADC/PWM (and nRF SPIM)
//! completed only via `bus_tick_indices` / `tick_with_bus`. At
//! `peripheral_tick_interval = 512` that lag could reach ~511 instructions —
//! a fidelity defect for busy-wait drivers.
//!
//! After promotion these models schedule delay-0 events on STARTTX / SAMPLE /
//! SEQSTART / TASKS_START so completion lands on the **next cycle** under
//! Machine + walk-free + tick 512 (not at the 512-cycle peripheral tick
//! quantum).
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
    let _ = configure_cortex_m(&mut bus);
    bus
}

fn machine_at_interval(interval: u32) -> Machine<CycleCpu> {
    let bus = bus_nrf52840_walk_free();
    assert!(
        bus.legacy_walk_disabled,
        "precondition: walk-free auto-derive failed"
    );
    assert_eq!(
        bus.max_safe_tick_interval(),
        RECOMMENDED_TICK_INTERVAL,
        "precondition: max_safe must be {RECOMMENDED_TICK_INTERVAL}"
    );
    let mut machine = Machine::new(CycleCpu::default(), bus);
    machine.config.peripheral_tick_interval = interval;
    machine.bus.config.peripheral_tick_interval = interval;
    machine
}

/// Plant a small TX buffer in RAM and return its base address.
fn plant_tx_buf(bus: &mut SystemBus, base: u64, bytes: &[u8]) {
    for (i, &b) in bytes.iter().enumerate() {
        bus.write_u8(base + i as u64, b)
            .expect("write TX buffer byte into RAM");
    }
}

/// Advance ≤ `max_cycles` one small batch at a time; return total_cycles when
/// `done` returns true, or None if the budget is exhausted.
fn advance_until<F>(
    machine: &mut Machine<CycleCpu>,
    max_cycles: u64,
    batch: u64,
    mut done: F,
) -> Option<u64>
where
    F: FnMut(&Machine<CycleCpu>) -> bool,
{
    while machine.total_cycles < max_cycles {
        machine
            .advance(AdvanceRequest::run(Some(batch)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("Machine::advance");
        if done(machine) {
            return Some(machine.total_cycles);
        }
    }
    None
}

// ── UARTE0 EasyDMA TX @ tick 512 ────────────────────────────────────────────

const UARTE0: u64 = 0x4000_2000;
const UARTE_TASKS_STARTTX: u64 = UARTE0 + 0x008;
const UARTE_EVENTS_ENDTX: u64 = UARTE0 + 0x120;
const UARTE_EVENTS_TXSTOPPED: u64 = UARTE0 + 0x158;
const UARTE_ENABLE: u64 = UARTE0 + 0x500;
const UARTE_TXD_PTR: u64 = UARTE0 + 0x544;
const UARTE_TXD_MAXCNT: u64 = UARTE0 + 0x548;
const UARTE_TXD_AMOUNT: u64 = UARTE0 + 0x54C;

fn arm_uarte_tx(machine: &mut Machine<CycleCpu>, buf: u64, bytes: &[u8]) {
    plant_tx_buf(&mut machine.bus, buf, bytes);
    machine.bus.write_u32(UARTE_ENABLE, 8).unwrap(); // UARTE EasyDMA
    machine.bus.write_u32(UARTE_TXD_PTR, buf as u32).unwrap();
    machine
        .bus
        .write_u32(UARTE_TXD_MAXCNT, bytes.len() as u32)
        .unwrap();
    // Clear any stale completion (write-0).
    machine.bus.write_u32(UARTE_EVENTS_ENDTX, 0).unwrap();
    machine.bus.write_u32(UARTE_EVENTS_TXSTOPPED, 0).unwrap();
    machine.bus.write_u32(UARTE_TASKS_STARTTX, 1).unwrap();
}

fn uarte_complete(m: &Machine<CycleCpu>) -> bool {
    m.bus.read_u32(UARTE_EVENTS_ENDTX).unwrap_or(0) != 0
        && m.bus.read_u32(UARTE_EVENTS_TXSTOPPED).unwrap_or(0) != 0
}

/// Machine + walk-free + interval 512: UARTE EasyDMA TX must complete within
/// a handful of device cycles (delay-0 scheduler), not after a 512-cycle
/// bus_tick quantum.
#[test]
fn uarte_easydma_completes_within_8_cycles_at_tick_512() {
    let mut machine = machine_at_interval(RECOMMENDED_TICK_INTERVAL);
    let buf = 0x2000_1000u64;
    let payload = b"Hi";
    arm_uarte_tx(&mut machine, buf, payload);

    // Without the scheduler path, completion waited for the next 512-cycle
    // peripheral tick. With delay-0, ≤ 8 cycles (one small batch) is ample.
    const CYCLE_BUDGET: u64 = 8;
    let at = advance_until(&mut machine, CYCLE_BUDGET, 1, uarte_complete);
    assert!(
        at.is_some(),
        "UARTE ENDTX/TXSTOPPED never set within {CYCLE_BUDGET} cycles under \
         Machine + peripheral_tick_interval={RECOMMENDED_TICK_INTERVAL} \
         (total_cycles={}, legacy_walk_disabled={}). Delay-0 scheduler path \
         must complete EasyDMA without waiting for the bus_tick quantum.",
        machine.total_cycles,
        machine.bus.legacy_walk_disabled,
    );
    let at = at.unwrap();
    assert!(
        at <= CYCLE_BUDGET,
        "UARTE TX completed at cycle {at}, beyond budget {CYCLE_BUDGET}"
    );
    assert_eq!(
        machine.bus.read_u32(UARTE_TXD_AMOUNT).unwrap(),
        payload.len() as u32
    );
}

// ── SAADC EasyDMA SAMPLE @ tick 512 ─────────────────────────────────────────

const SAADC: u64 = 0x4000_7000;
const SAADC_TASKS_SAMPLE: u64 = SAADC + 0x004;
const SAADC_EVENTS_END: u64 = SAADC + 0x104;
const SAADC_EVENTS_RESULTDONE: u64 = SAADC + 0x10C;
const SAADC_ENABLE: u64 = SAADC + 0x500;
const SAADC_RESOLUTION: u64 = SAADC + 0x5F0;
const SAADC_RESULT_PTR: u64 = SAADC + 0x62C;
const SAADC_RESULT_MAXCNT: u64 = SAADC + 0x630;
const SAADC_RESULT_AMOUNT: u64 = SAADC + 0x634;

#[test]
fn saadc_easydma_completes_within_8_cycles_at_tick_512() {
    let mut machine = machine_at_interval(RECOMMENDED_TICK_INTERVAL);
    let buf = 0x2000_1100u64;

    machine.bus.write_u32(SAADC_ENABLE, 1).unwrap();
    machine.bus.write_u32(SAADC_RESOLUTION, 2).unwrap(); // 12-bit
    machine.bus.write_u32(SAADC_RESULT_PTR, buf as u32).unwrap();
    machine.bus.write_u32(SAADC_RESULT_MAXCNT, 2).unwrap();
    machine.bus.write_u32(SAADC_EVENTS_END, 0).unwrap();
    machine.bus.write_u32(SAADC_EVENTS_RESULTDONE, 0).unwrap();
    machine.bus.write_u32(SAADC_TASKS_SAMPLE, 1).unwrap();

    const CYCLE_BUDGET: u64 = 8;
    let at = advance_until(&mut machine, CYCLE_BUDGET, 1, |m| {
        m.bus.read_u32(SAADC_EVENTS_END).unwrap_or(0) != 0
            && m.bus.read_u32(SAADC_EVENTS_RESULTDONE).unwrap_or(0) != 0
    });
    assert!(
        at.is_some(),
        "SAADC END/RESULTDONE never set within {CYCLE_BUDGET} cycles at tick \
         {RECOMMENDED_TICK_INTERVAL} (total_cycles={})",
        machine.total_cycles,
    );
    assert_eq!(machine.bus.read_u32(SAADC_RESULT_AMOUNT).unwrap(), 2);
}

// ── PWM0 SEQSTART0 @ tick 512 ───────────────────────────────────────────────

const PWM0: u64 = 0x4001_C000;
const PWM_TASKS_SEQSTART0: u64 = PWM0 + 0x008;
const PWM_EVENTS_SEQEND0: u64 = PWM0 + 0x110;
const PWM_EVENTS_PWMPERIODEND: u64 = PWM0 + 0x118;
const PWM_ENABLE: u64 = PWM0 + 0x500;
const PWM_SEQ0_PTR: u64 = PWM0 + 0x520;
const PWM_SEQ0_CNT: u64 = PWM0 + 0x524;

#[test]
fn pwm_easydma_completes_within_8_cycles_at_tick_512() {
    let mut machine = machine_at_interval(RECOMMENDED_TICK_INTERVAL);
    let buf = 0x2000_1200u64;
    // Two 16-bit duty samples.
    plant_tx_buf(&mut machine.bus, buf, &[0x10, 0x80, 0x20, 0x80]);

    machine.bus.write_u32(PWM_ENABLE, 1).unwrap();
    machine.bus.write_u32(PWM_SEQ0_PTR, buf as u32).unwrap();
    machine.bus.write_u32(PWM_SEQ0_CNT, 2).unwrap();
    machine.bus.write_u32(PWM_EVENTS_SEQEND0, 0).unwrap();
    machine.bus.write_u32(PWM_EVENTS_PWMPERIODEND, 0).unwrap();
    machine.bus.write_u32(PWM_TASKS_SEQSTART0, 1).unwrap();

    const CYCLE_BUDGET: u64 = 8;
    let at = advance_until(&mut machine, CYCLE_BUDGET, 1, |m| {
        m.bus.read_u32(PWM_EVENTS_SEQEND0).unwrap_or(0) != 0
            && m.bus.read_u32(PWM_EVENTS_PWMPERIODEND).unwrap_or(0) != 0
    });
    assert!(
        at.is_some(),
        "PWM SEQEND0/PWMPERIODEND never set within {CYCLE_BUDGET} cycles at \
         tick {RECOMMENDED_TICK_INTERVAL} (total_cycles={})",
        machine.total_cycles,
    );
}

// ── Walk@1 vs sched@512 UARTE TX completion identity ────────────────────────

/// Lane A: tick_interval=1 (bus_tick every cycle + scheduler).
/// Lane B: walk-free + interval 512 + scheduler delay-0.
/// Same STARTTX / buffer; ENDTX+TXSTOPPED must raise and completion cycles
/// must agree within 1 absolute cycle.
#[test]
fn uarte_tx_walk1_vs_sched512_completion_cycle_identity() {
    let payload = b"ID";
    let buf = 0x2000_1300u64;

    let mut lane_a = machine_at_interval(1);
    arm_uarte_tx(&mut lane_a, buf, payload);
    let at_a = advance_until(&mut lane_a, 16, 1, uarte_complete)
        .expect("lane A (interval=1) must complete UARTE TX");

    let mut lane_b = machine_at_interval(RECOMMENDED_TICK_INTERVAL);
    arm_uarte_tx(&mut lane_b, buf, payload);
    let at_b = advance_until(&mut lane_b, 16, 1, uarte_complete)
        .expect("lane B (interval=512) must complete UARTE TX via scheduler");

    assert!(
        uarte_complete(&lane_a) && uarte_complete(&lane_b),
        "both lanes must raise ENDTX+TXSTOPPED"
    );
    assert_eq!(
        lane_a.bus.read_u32(UARTE_TXD_AMOUNT).unwrap(),
        payload.len() as u32
    );
    assert_eq!(
        lane_b.bus.read_u32(UARTE_TXD_AMOUNT).unwrap(),
        payload.len() as u32
    );

    let delta = at_a.abs_diff(at_b);
    assert!(
        delta <= 1,
        "UARTE TX completion cycle must agree within 1: \
         walk@1 at={at_a}, sched@512 at={at_b}, delta={delta}"
    );
}
