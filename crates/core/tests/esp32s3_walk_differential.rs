// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential gate for the ESP32-S3 SYSTIMER → event-scheduler migration, in
//! the `esp32c3_walk_differential` / `stm32_timer_walk_differential` style: the
//! SAME hand-built ESP32-S3 machine and the same hand-assembled ISR
//! (`intmatrix_alarm`'s SYSTIMER-alarm → intmatrix → CPU dispatch → GPIO chain)
//! run twice — once with the SYSTIMER pinned back onto the per-cycle walk
//! (`force_legacy_walk`, the reference) and once scheduler-driven — and the
//! GPIO-toggle observable is compared.
//!
//! This is the fidelity contract that licenses un-pinning the S3 SYSTIMER from
//! the walk: the scheduler-driven alarm must be routed through the Xtensa
//! interrupt matrix (`pending_cpu_irqs` + INTR_STATUS mirror) at EXACTLY the
//! cycle the legacy walk would have delivered it.
//!
//! 1. `alarm_isr_is_byte_identical_at_interval_1` — at tick interval 1 the walk
//!    ticks every cycle and the scheduler fires every event at its exact
//!    deadline, so both deliver the alarm on the same cycle. EVERY
//!    instruction-boundary interrupt observable is compared: PC (ISR-entry
//!    cycle), `pending_cpu_irqs` for core 0 (routed level set/clear +
//!    de-assert-after-INT_CLR timing), the intmatrix `INTR_STATUS` word the ISR
//!    reads (source-discovery content), total_cycles, and the full GPIO2
//!    transition trace — all byte-identical. If the scheduler fires even one
//!    cycle late, mis-sets the routed level, or mirrors the wrong source into
//!    INTR_STATUS, the vectors diverge.
//!
//! 2. `alarm_toggle_count_is_exact_at_interval_8` — scheduler @ interval 8 vs
//!    the walk-on interval-1 golden reference. At interval > 1 the walk
//!    quantises alarm *detection* up to the tick grid while the scheduler still
//!    fires at the exact deadline, so per-cycle stamps legitimately differ by
//!    < one interval (the same bounded quantisation the write-path `sync_to`
//!    and the STM32 gate-5 document). But a level-latched alarm can never be
//!    MISSED, so the number of ISR entries (GPIO2 toggle pairs) over a fixed
//!    instruction window is EXACT — asserted, with the window edge verified to
//!    sit more than one interval + dispatch lag away from the last toggle in
//!    the reference so quantisation cannot move a toggle across the edge.

#![cfg(feature = "event-scheduler")]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::XtensaLx7;
use labwired_core::peripherals::esp32s3::gpio::GpioObserver;
use labwired_core::peripherals::esp32s3::systimer::Systimer;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::{Bus, Cpu, DebugControl, Machine};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct RecordingObserver {
    events: Mutex<Vec<(u8, bool, bool, u64)>>,
}

impl GpioObserver for RecordingObserver {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64) {
        self.events.lock().unwrap().push((pin, from, to, sim_cycle));
    }
}

/// Hand-assembled ISR (identical to `intmatrix_alarm`): GPIO2 0→1 (W1TS),
/// GPIO2 1→0 (W1TC), SYSTIMER_INT_CLR ack, `rfe`. Each entry emits a
/// rising+falling pair on pin 2.
const ISR_BYTES: &[u8] = &[
    0x69, 0x03, // s32i.n  a6, a3, 0   (GPIO_OUT_W1TS: pin 2 0→1)
    0x69, 0x04, // s32i.n  a6, a4, 0   (GPIO_OUT_W1TC: pin 2 1→0)
    0x79, 0x05, // s32i.n  a7, a5, 0   (SYSTIMER_INT_CLR: ack alarm 0)
    0x00, 0x30, 0x00, // rfe
];

/// `j 0` — jump-to-self spin loop, 3 bytes.
const SPIN_BYTES: &[u8] = &[0x06, 0xff, 0xff];

const IRAM_BASE: u32 = 0x4037_0000;
const ISR_PC: u32 = IRAM_BASE + 0x1000;
const VECBASE_VALUE: u32 = ISR_PC - 0x300;
const SYSTIMER_BASE: u32 = 0x6002_3000;
const INTMATRIX_BASE: u32 = 0x600C_2000;
const SYSTIMER_TARGET0_SOURCE: u32 = 57;
const CPU_IRQ_SLOT: u8 = 12;

/// Build the shared S3 alarm machine (period-mode alarm every 20 SYSTIMER
/// ticks, routed source 57 → CPU slot 12 → the GPIO-toggle ISR), exactly as
/// `intmatrix_alarm_full_irq_chain` does. `scheduler = false` pins the SYSTIMER
/// back onto the legacy per-cycle walk (`force_legacy_walk`) to form the
/// reference lane from the same assembly.
fn build_alarm_machine(
    scheduler: bool,
    tick_interval: u32,
) -> (Machine<XtensaLx7>, Arc<RecordingObserver>) {
    let mut bus = SystemBus::new();
    let opts = Esp32s3Opts::default();
    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);

    let obs = Arc::new(RecordingObserver::default());
    wiring.add_gpio_observer(&mut bus, obs.clone());

    if !scheduler {
        let systimer = bus
            .peripherals
            .iter_mut()
            .find_map(|p| {
                p.dev
                    .as_any_mut()
                    .and_then(|a| a.downcast_mut::<Systimer>())
            })
            .expect("S3 bus registers a Systimer");
        systimer.force_legacy_walk();
    }

    let mut cpu = wiring.cpu;

    for (i, &b) in SPIN_BYTES.iter().enumerate() {
        bus.write_u8((IRAM_BASE + i as u32) as u64, b).unwrap();
    }
    for (i, &b) in ISR_BYTES.iter().enumerate() {
        bus.write_u8((ISR_PC + i as u32) as u64, b).unwrap();
    }

    use labwired_core::cpu::xtensa_sr::{INTENABLE, VECBASE};
    cpu.sr.write(VECBASE, VECBASE_VALUE);
    cpu.regs.write_logical(3, 0x6000_4008); // GPIO_OUT_W1TS
    cpu.regs.write_logical(4, 0x6000_400C); // GPIO_OUT_W1TC
    cpu.regs.write_logical(5, SYSTIMER_BASE + 0x6C); // SYSTIMER_INT_CLR
    cpu.regs.write_logical(6, 0x0000_0004); // bit 2 mask
    cpu.regs.write_logical(7, 0x0000_0001); // alarm 0 clear bit

    let intmatrix_off = INTMATRIX_BASE + SYSTIMER_TARGET0_SOURCE * 4;
    bus.write_u32(intmatrix_off as u64, CPU_IRQ_SLOT as u32)
        .unwrap();

    // SYSTIMER ALARM0: PERIOD mode, period 20 SYSTIMER ticks.
    bus.write_u32((SYSTIMER_BASE + 0x1C) as u64, 0).unwrap();
    bus.write_u32((SYSTIMER_BASE + 0x20) as u64, 20).unwrap();
    bus.write_u32((SYSTIMER_BASE + 0x34) as u64, (1u32 << 30) | 20)
        .unwrap();
    bus.write_u32((SYSTIMER_BASE + 0x50) as u64, 1).unwrap();
    let conf = bus.read_u32(SYSTIMER_BASE as u64).unwrap();
    bus.write_u32(SYSTIMER_BASE as u64, conf | (1u32 << 24))
        .unwrap();
    bus.write_u32((SYSTIMER_BASE + 0x64) as u64, 1).unwrap(); // INT_ENA bit 0

    cpu.sr.write(INTENABLE, 1u32 << CPU_IRQ_SLOT);
    cpu.ps.set_intlevel(0);
    cpu.ps.set_excm(false);
    cpu.set_pc(IRAM_BASE);

    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = tick_interval;
    machine.bus.config.peripheral_tick_interval = tick_interval;
    (machine, obs)
}

/// The GPIO2 rising edges (0→1) recorded by the observer, as (sim_cycle).
fn pin2_rising_cycles(obs: &RecordingObserver) -> Vec<u64> {
    obs.events
        .lock()
        .unwrap()
        .iter()
        .filter(|&&(p, from, to, _)| p == 2 && !from && to)
        .map(|&(_, _, _, cyc)| cyc)
        .collect()
}

/// Run `steps` instructions through the batched `Machine::run` path (the
/// production path the scheduler timing convention is calibrated to).
fn run_steps(machine: &mut Machine<XtensaLx7>, steps: u64) {
    const CHUNK: u32 = 100_000;
    let mut done = 0u64;
    while done < steps {
        let n = CHUNK.min((steps - done) as u32);
        machine.run(Some(n)).expect("run S3 alarm machine");
        done += n as u64;
    }
}

/// PRO_INTR_STATUS_REG_1 (source 57 lives in word 1, bit 25): base 0x18C +
/// 1*4. The register esp-hal's `__level_*_interrupt` reads to discover which
/// matrix source asserted.
const INTR_STATUS_REG1: u32 = INTMATRIX_BASE + 0x190;

/// The exact interrupt-delivery state the coordinator's fidelity checklist
/// names: CPU PC (→ ISR-entry cycle), the routed `pending_cpu_irqs` level for
/// core 0 (set/clear timing), and the intmatrix `INTR_STATUS` word the ISR
/// reads (source-discovery content). Captured after EVERY instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IrqProbe {
    step: u64,
    total_cycles: u64,
    pc: u32,
    pending_cpu_irqs0: u32,
    intr_status_reg1: u32,
}

fn irq_probe(machine: &Machine<XtensaLx7>, step: u64) -> IrqProbe {
    IrqProbe {
        step,
        total_cycles: machine.total_cycles,
        pc: machine.cpu.get_pc(),
        pending_cpu_irqs0: machine.bus.pending_cpu_irqs[0],
        intr_status_reg1: machine.bus.read_u32(INTR_STATUS_REG1 as u64).unwrap(),
    }
}

fn run_irq_probed(machine: &mut Machine<XtensaLx7>, steps: u64) -> Vec<IrqProbe> {
    let mut probes = Vec::with_capacity(steps as usize);
    for s in 0..steps {
        machine.run(Some(1)).expect("run S3 alarm machine");
        probes.push(irq_probe(machine, s + 1));
    }
    probes
}

/// Gate 1: walk-on vs scheduler at tick interval 1. The alarm is delivered on
/// the same cycle by both lanes, so EVERY instruction-boundary interrupt
/// observable is byte-identical — directly pinning the coordinator's fidelity
/// checklist: the ISR-entry cycle (PC trace), the `pending_cpu_irqs` level
/// set/clear + de-assert-after-INT_CLR timing (per-instruction), the
/// `INTR_STATUS` content the ISR reads, and the full GPIO2 transition trace.
/// If the scheduler fired even one cycle late, mis-set the routed level, or
/// mirrored the wrong source into INTR_STATUS, these vectors diverge.
#[test]
fn alarm_isr_is_byte_identical_at_interval_1() {
    const STEPS: u64 = 20_000;

    let (mut walk, walk_obs) = build_alarm_machine(false, 1);
    let walk_probes = run_irq_probed(&mut walk, STEPS);
    let walk_events = walk_obs.events.lock().unwrap().clone();

    let (mut sched, sched_obs) = build_alarm_machine(true, 1);
    let sched_probes = run_irq_probed(&mut sched, STEPS);
    let sched_events = sched_obs.events.lock().unwrap().clone();

    let rising = pin2_rising_cycles(&walk_obs);
    assert!(
        rising.len() >= 5,
        "reference (SYSTIMER walk) must take repeated alarm ISRs (got {} GPIO2 toggles)",
        rising.len()
    );
    // The reference must actually assert the routed level and mirror source 57
    // (bit 25) into INTR_STATUS_REG_1 at some point — otherwise the probe would
    // be vacuously identical (both all-zero).
    assert!(
        walk_probes
            .iter()
            .any(|p| p.pending_cpu_irqs0 == (1 << CPU_IRQ_SLOT)),
        "reference must route the alarm to CPU slot {CPU_IRQ_SLOT} in pending_cpu_irqs"
    );
    assert!(
        walk_probes
            .iter()
            .any(|p| p.intr_status_reg1 & (1 << 25) != 0),
        "reference must mirror SYSTIMER source 57 into INTR_STATUS_REG_1"
    );
    // The whole point: every interrupt observable identical at every
    // instruction — find the first divergence if any.
    for (w, s) in walk_probes.iter().zip(sched_probes.iter()) {
        assert_eq!(
            w, s,
            "IRQ delivery diverged at step {} (SYSTIMER walk-reference vs scheduler): \
             pending_cpu_irqs / INTR_STATUS / PC / total_cycles must be byte-identical \
             every cycle at interval 1",
            w.step
        );
    }
    assert_eq!(walk_probes.len(), sched_probes.len());
    assert_eq!(
        walk_events, sched_events,
        "GPIO2 transition trace (pin, edge, sim-cycle) must be byte-identical \
         (SYSTIMER walk vs scheduler) at interval 1"
    );
}

/// Gate 2: scheduler @ interval 8 vs the walk-on interval-1 golden reference.
/// Per-cycle stamps quantise (< one interval, documented), so the trace is not
/// compared — but a level-latched alarm can never be missed, so the ISR-entry
/// COUNT (GPIO2 rising edges) over the fixed window is exact, and total_cycles
/// (one per instruction) matches. Window edge verified clear of the last
/// reference toggle by > interval + dispatch lag.
#[test]
fn alarm_toggle_count_is_exact_at_interval_8() {
    const STEPS: u64 = 20_000;
    const INTERVAL: u32 = 8;

    let (mut walk, walk_obs) = build_alarm_machine(false, 1);
    run_steps(&mut walk, STEPS);
    let walk_rising = pin2_rising_cycles(&walk_obs);

    let (mut sched, sched_obs) = build_alarm_machine(true, INTERVAL);
    run_steps(&mut sched, STEPS);
    let sched_rising = pin2_rising_cycles(&sched_obs);

    assert!(
        walk_rising.len() >= 10,
        "reference must observe repeated alarm ISRs (got {})",
        walk_rising.len()
    );
    // Quantisation can only shift a toggle by < one interval + the dispatch
    // lag; keep the window edge well clear of the last reference toggle so a
    // shift cannot move it across the edge (alarm period is ~100 CPU cycles).
    let last_toggle_cycle = *walk_rising.last().unwrap();
    assert!(
        walk.total_cycles - last_toggle_cycle > (INTERVAL as u64 + 50),
        "fixture must keep the window edge > interval + dispatch lag from the last \
         reference toggle (last at cycle {last_toggle_cycle}, window end {})",
        walk.total_cycles
    );
    assert_eq!(
        sched_rising.len(),
        walk_rising.len(),
        "alarm ISR-entry count over the fixed window must be exact at interval {INTERVAL}"
    );
    assert_eq!(
        sched.total_cycles, walk.total_cycles,
        "total_cycles (one per instruction) must match the interval-1 reference"
    );
}
