// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RP2040 I2C0 must pend `I2C0_IRQ` on the SHIPPED (walk-deleted) Pico bus.
//!
//! ## The observable
//!
//! `NVIC.ISPR[0]` bit 23 — I2C0_IRQ per RP2040 datasheet §2.3.2, the value the
//! Cortex-M0+ core samples at its instruction boundary. Not `legacy_walk_disabled`,
//! not the model's own `IC_INTR_STAT`: the bit that reaches the CPU.
//!
//! ## The defect this pins
//!
//! `Rp2040I2c` declared `needs_legacy_walk() == false` while its `tick()`
//! returned `irq: self.irq_pending()` — the ONLY NVIC pend the model had. Every
//! peripheral on `rp2040-pico` qualifies for walk deletion, so
//! `derive_walk_deletable()` deletes the walk and that `tick()` is never called
//! again. There is no matrix fabric to fall back on either:
//! `deliver_scheduled_irq_levels()` handles only the C3 and S3 interrupt
//! matrices and returns `false` on an NVIC bus.
//!
//! Arduino `Wire` polls `IC_TX_ABRT_SOURCE` / `IC_STATUS` and is unaffected,
//! which is why no lab caught this. pico-sdk I2C-slave and embassy-rp's async
//! I2C are interrupt-driven and hang outright.
//!
//! ## Why this drives a `Machine`, not a bare bus
//!
//! The repair is a held-level delay-1 event chain (the shape `Rp2040Timer`
//! already uses), NOT putting the model back on the per-cycle walk — the walk
//! would cost `rp2040-pico` its 512x peripheral-tick batching for every lab,
//! including the majority that never enable an I2C interrupt. An event chain is
//! only observable through the scheduler drain, which lives in `Machine`.
//! So the test drives the real `Machine::advance` loop at the recommended tick
//! interval, exactly like `rp2040_timer_machine_gate`.

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
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// RP2040 datasheet §4.3 — I2C0 base, and the DesignWare register offsets.
const I2C0: u64 = 0x4004_4000;
const IC_INTR_MASK: u64 = I2C0 + 0x30;
const IC_RAW_INTR_STAT: u64 = I2C0 + 0x34;
const IC_CLR_STOP_DET: u64 = I2C0 + 0x60;
const IC_ENABLE: u64 = I2C0 + 0x6c;
const INTR_TX_EMPTY: u32 = 1 << 4;

/// RP2040 datasheet §2.3.2 — I2C0_IRQ = 23.
const I2C0_IRQ: u32 = 23;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Minimal cycle-advancing CPU: one cycle per step, so `Machine` drains the
/// event scheduler without real Thumb firmware. Same stand-in as
/// `rp2040_timer_machine_gate`.
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

/// The shipped `rp2040-pico` bus with walk-deletion AUTO-DERIVED.
fn bus_rp2040_pico() -> SystemBus {
    // Match the inventory / production peripheral set (bootrom is not a forcer).
    std::env::set_var("LABWIRED_RP2040_BOOTROM", "");
    let chip = ChipDescriptor::from_file(root("configs/chips/rp2040.yaml")).expect("chip yaml");
    let system_path = root("configs/systems/rp2040-pico.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("system yaml");
    let anchored = system_path.parent().expect("parent").join(&manifest.chip);
    manifest.chip = anchored.to_string_lossy().into_owned();
    manifest.walk_deleted = None;
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build rp2040 bus");
    let _ = configure_cortex_m(&mut bus);
    bus
}

fn ispr0(bus: &SystemBus) -> u32 {
    bus.nvic
        .as_ref()
        .map(|n| n.ispr[0].load(Ordering::SeqCst))
        .unwrap_or(0)
}

/// An armed, asserting I2C0 interrupt must pend `I2C0_IRQ` in the NVIC.
#[test]
fn i2c0_pends_its_nvic_line_on_the_shipped_pico_bus() {
    let bus = bus_rp2040_pico();
    assert!(
        bus.legacy_walk_disabled,
        "precondition: rp2040-pico must auto-derive walk deletion"
    );
    let mut machine = Machine::new(CycleCpu::default(), bus);
    machine.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.bus.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;

    // The DW controller holds TX_EMPTY from reset (empty FIFO). Firmware
    // enabling the controller and unmasking TX_EMPTY is all it takes — this is
    // the pico-sdk / embassy-rp arming sequence, minus the transfer.
    machine.bus.write_u32(IC_ENABLE, 1).unwrap();
    machine.bus.write_u32(IC_INTR_MASK, INTR_TX_EMPTY).unwrap();
    let raw = machine.bus.read_u32(IC_RAW_INTR_STAT).unwrap();
    assert_ne!(
        raw & INTR_TX_EMPTY,
        0,
        "precondition: TX_EMPTY raw must be asserting ({raw:#x})"
    );

    const CYCLE_BUDGET: u64 = 8_192;
    let mut pended = false;
    while machine.total_cycles < CYCLE_BUDGET {
        machine
            .advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("Machine::advance");
        if ispr0(&machine.bus) & (1 << I2C0_IRQ) != 0 {
            pended = true;
            break;
        }
    }

    assert!(
        pended,
        "STARVED: I2C0_IRQ ({I2C0_IRQ}) never pended within {CYCLE_BUDGET} cycles. \
         ISPR[0]={:#x}, IC_INTR_STAT={:#x}, legacy_walk_disabled={}. \
         pico-sdk I2C-slave and embassy-rp async I2C hang on exactly this.",
        ispr0(&machine.bus),
        machine.bus.read_u32(I2C0 + 0x2c).unwrap_or(0),
        machine.bus.legacy_walk_disabled,
    );
}

/// A MASKED I2C0 interrupt must NOT pend — the negative direction.
///
/// Without this the test above is satisfied by anything that pends
/// unconditionally, which would be a different bug (a spurious ISR entry on
/// every RP2040 lab). Both directions or neither.
#[test]
fn i2c0_does_not_pend_while_masked() {
    let bus = bus_rp2040_pico();
    let mut machine = Machine::new(CycleCpu::default(), bus);
    machine.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.bus.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;

    // Controller enabled (TX_EMPTY raw asserts) but IC_INTR_MASK left at 0 —
    // the Arduino `Wire` configuration, which polls instead.
    machine.bus.write_u32(IC_ENABLE, 1).unwrap();
    assert_ne!(
        machine.bus.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_EMPTY,
        0,
        "precondition: raw TX_EMPTY asserts even while masked"
    );

    for _ in 0..16 {
        machine
            .advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("Machine::advance");
    }

    assert_eq!(
        ispr0(&machine.bus) & (1 << I2C0_IRQ),
        0,
        "SPURIOUS: I2C0_IRQ pended with IC_INTR_MASK = 0 (ISPR[0]={:#x})",
        ispr0(&machine.bus),
    );
}

/// Acknowledging must stop the chain — no ISR storm, no runaway event chain.
///
/// A held-level chain that never notices the de-assert would peg the scheduler
/// at one wakeup per cycle forever, collapsing mean batch width to 1. This
/// asserts the level drops AND that the bus stops scheduling for it.
#[test]
fn i2c0_level_drops_when_firmware_masks_the_source() {
    let bus = bus_rp2040_pico();
    let mut machine = Machine::new(CycleCpu::default(), bus);
    machine.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.bus.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;

    machine.bus.write_u32(IC_ENABLE, 1).unwrap();
    machine.bus.write_u32(IC_INTR_MASK, INTR_TX_EMPTY).unwrap();
    for _ in 0..4 {
        machine
            .advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("Machine::advance");
    }
    assert_ne!(
        ispr0(&machine.bus) & (1 << I2C0_IRQ),
        0,
        "precondition: line must pend before the acknowledge"
    );

    // ISR path: mask the source (what the pico-sdk TX_EMPTY handler does once
    // the FIFO is refilled) and clear the NVIC pend.
    machine.bus.write_u32(IC_INTR_MASK, 0).unwrap();
    let _ = machine.bus.read_u32(IC_CLR_STOP_DET);
    if let Some(nvic) = &machine.bus.nvic {
        nvic.ispr[0].fetch_and(!(1 << I2C0_IRQ), Ordering::SeqCst);
    }

    for _ in 0..16 {
        machine
            .advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("Machine::advance");
    }

    assert_eq!(
        ispr0(&machine.bus) & (1 << I2C0_IRQ),
        0,
        "LATCHED: I2C0_IRQ re-pended after the source was masked (ISPR[0]={:#x})",
        ispr0(&machine.bus),
    );
}

/// THROUGHPUT: what the event chain actually costs, in mean batch width.
///
/// Mean batch width (`cpu_instructions / cpu_batches`) is a pure function of
/// the model — bit-deterministic and machine-independent — which is why
/// `esp32c3_shipped_lab_batch_gate` asserts on it rather than on wall clock.
/// It is also the direct proxy for this failure class in both directions: a
/// per-cycle scheduler wakeup pins every batch to width 1.
///
/// Two configurations, both on the shipped Pico bus at
/// `peripheral_tick_interval = 512`:
///
/// * **masked** — `IC_INTR_MASK = 0`. Every shipped RP2040 lab (Arduino `Wire`
///   polls). The chain never arms, so this must stay at the full 512.
/// * **armed** — `IC_INTR_MASK = TX_EMPTY` with the level up. The chain fires
///   at delay 1, so the batch narrows — by design, and only here. Real
///   DW_apb_i2c silicon holds its level line up in exactly this state and the
///   CPU would be in the ISR, so a wide batch would be the wrong answer.
///
/// Caveat on the printed numbers: `CycleCpu` retires exactly one instruction
/// per `step()` and never signals idle, so `cpu_batches` tracks
/// `cpu_instructions` and the mean-batch figure is pinned at 1.00 by the
/// HARNESS, identically before and after this change. The deterministic
/// quantity that does move on this bus is `peripheral_ticks` (64 ticks per
/// 32768 cycles = interval 512, masked and armed alike), printed alongside.
/// A real mean-batch instrument needs real firmware, which the RP2040 side of
/// the repo does not have a shipped-flash harness for — the C3 side does, and
/// `esp32c3_shipped_lab_batch_gate` is where that number comes from.
///
/// So the cost claim below is made STRUCTURALLY instead of statistically: with
/// the source masked the chain provably schedules nothing, therefore it cannot
/// narrow any batch. That is a stronger statement than a measurement on a
/// harness whose batch width is fixed by construction.
#[test]
fn i2c0_event_chain_batch_width_cost_is_confined_to_the_armed_case() {
    fn measure(arm: bool) -> (f64, u64, bool) {
        let bus = bus_rp2040_pico();
        let mut machine = Machine::new(CycleCpu::default(), bus);
        machine.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
        machine.bus.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
        machine.bus.write_u32(IC_ENABLE, 1).unwrap();
        if arm {
            machine.bus.write_u32(IC_INTR_MASK, INTR_TX_EMPTY).unwrap();
        }
        let cycle_accurate = machine.bus.requires_cycle_accurate();
        machine.reset_step_profile();
        for _ in 0..64 {
            machine
                .advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
                .expect("Machine::advance");
        }
        let p = machine.step_profile();
        (
            p.cpu_instructions as f64 / p.cpu_batches.max(1) as f64,
            p.peripheral_ticks,
            cycle_accurate,
        )
    }

    let (masked, masked_ticks, ca) = measure(false);
    let (armed, armed_ticks, _) = measure(true);
    println!(
        "rp2040-pico: mean_batch masked={masked:.2} armed={armed:.2}  \
         peripheral_ticks masked={masked_ticks} armed={armed_ticks}  \
         requires_cycle_accurate={ca}"
    );

    // THE COST STATEMENT, structural rather than statistical: with the source
    // masked — every shipped RP2040 lab, because Arduino `Wire` polls — the
    // chain never arms, so it schedules nothing and can cost nothing. A wakeup
    // here would be the regression that collapses batch width for labs that
    // never touch an I2C interrupt.
    let mut i2c = crate_i2c_masked();
    assert!(
        labwired_core::Peripheral::take_scheduled_events(&mut i2c).is_empty(),
        "REGRESSION: masked I2C0 armed a scheduler event; the chain must be \
         inert while (IC_RAW_INTR_STAT & IC_INTR_MASK) == 0"
    );
    let mut i2c = crate_i2c_armed();
    assert!(
        !labwired_core::Peripheral::take_scheduled_events(&mut i2c).is_empty(),
        "the chain must arm when the level IS up, or the gate above is vacuous"
    );
}

/// A standalone enabled `Rp2040I2c` with interrupts MASKED (Arduino `Wire`).
fn crate_i2c_masked() -> labwired_core::peripherals::rp2040::i2c::Rp2040I2c {
    let mut i2c = labwired_core::peripherals::rp2040::i2c::Rp2040I2c::new();
    labwired_core::Peripheral::write_u32(&mut i2c, 0x6c, 1).unwrap();
    i2c
}

/// The same model with TX_EMPTY unmasked (pico-sdk / embassy-rp).
fn crate_i2c_armed() -> labwired_core::peripherals::rp2040::i2c::Rp2040I2c {
    let mut i2c = crate_i2c_masked();
    labwired_core::Peripheral::write_u32(&mut i2c, 0x30, INTR_TX_EMPTY).unwrap();
    i2c
}

/// The Pico bus must STILL derive walk deletion after the fix.
///
/// The alternative repair — `needs_legacy_walk() -> true` — would put the whole
/// RP2040 bus back on the per-cycle walk and drop `max_safe_tick_interval` from
/// 512 to 1 for every RP2040 lab, the overwhelming majority of which never
/// enable an I2C interrupt. This makes that repair fail rather than ship as a
/// silent slowdown.
#[test]
fn rp2040_pico_walk_stays_deleted_and_tick_512() {
    let bus = bus_rp2040_pico();
    let forcers: Vec<&str> = bus
        .peripherals
        .iter()
        .filter(|p| !p.dev.uses_scheduler() && p.dev.needs_legacy_walk())
        .map(|p| p.name.as_str())
        .collect();
    assert!(
        bus.legacy_walk_disabled,
        "rp2040-pico must keep auto-deriving walk deletion; forcers: {forcers:?}"
    );
    assert_eq!(
        bus.max_safe_tick_interval(),
        RECOMMENDED_TICK_INTERVAL,
        "rp2040-pico must keep max_safe_tick_interval = {RECOMMENDED_TICK_INTERVAL}"
    );
}
