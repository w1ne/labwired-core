// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential gates for walk-free batch **B1** (SysTick + SCB → event
//! scheduler), in the `esp32c3_walk_differential` style: the SAME hand-built
//! Cortex-M machine and hand-assembled Thumb firmware run twice — once with
//! SysTick/SCB pinned back onto the per-cycle walk (`force_legacy_walk`, the
//! reference) and once scheduler-driven — and every observable is compared.
//!
//! 1. `systick_irq_firmware_is_byte_identical_at_interval_1` — a delay-loop +
//!    SysTick-ISR-counter firmware that ARMS SysTick (RVR/CVR/CSR writes) and
//!    polls `SYST_CVR` from its main loop. Probed after EVERY instruction via
//!    `Machine::run(Some(1))`: total_cycles, PC, all 16 core registers and
//!    both RAM counters must be byte-identical at tick interval 1 — which
//!    pins the IRQ delivery cycles and the ISR execution count exactly.
//!
//! 2. `systick_isr_count_is_exact_at_interval_64` — the scheduler lane at tick
//!    interval 64 vs the walk-on interval-1 golden reference: exception
//!    delivery is quantised to the batch grid (documented, ≤ one interval),
//!    but the ISR execution count over a fixed instruction window is EXACT
//!    (event deadlines are absolute — no cumulative drift), provided no fire
//!    lands within one interval of the window edge (asserted from the
//!    reference's own delivery trace).
//!
//! 3. `pendsv_firmware_is_byte_identical_at_interval_1` — the SCB leg: an
//!    ICSR.PENDSVSET write must deliver PendSV (exception 14) on the same
//!    cycle in both lanes (the write-armed delay-0 event vs the walk drain).
//!
//! 4. `wfi_sleep_wakes_from_scheduled_systick_event` — the canonical
//!    `__disable_irq(); wfi()` idle pattern + a periodic wfi/ISR loop: with
//!    idle fast-forward enabled the machine must skip to the SCHEDULED
//!    SysTick expiry (the FF budget clamps to `next_event_deadline`) and wake
//!    with byte-identical architectural state vs the FF-off run, while
//!    retiring far fewer instructions.

#![cfg(feature = "event-scheduler")]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::peripherals::scb::Scb;
use labwired_core::peripherals::systick::Systick;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use labwired_core::{DebugControl, Machine, Peripheral};

/// ISR increment target.
const ISR_COUNT_ADDR: u64 = 0x2000_0000;
/// Main-loop increment target.
const MAIN_COUNT_ADDR: u64 = 0x2000_0004;
const INITIAL_SP: u32 = 0x2000_8000;

/// Assemble `halfwords` into flash at 0x0 via the bus.
fn load_thumb(bus: &mut SystemBus, base: u64, halfwords: &[u16]) {
    for (i, hw) in halfwords.iter().enumerate() {
        bus.write_u16(base + (i as u64) * 2, *hw).unwrap();
    }
}

fn write_word(bus: &mut SystemBus, addr: u64, word: u32) {
    bus.write_u32(addr, word).unwrap();
}

/// Build the shared machine assembly: `SystemBus::new()` (which registers the
/// native SysTick at 0xE000_E010) + `configure_cortex_m` (which installs the
/// real SCB/NVIC/DWT — the runtime-faithful Cortex-M bus). The SysTick gets
/// the bus cycle clock exactly as the production `from_config` choke would
/// attach it; `scheduler = false` then pins BOTH migrated models back onto
/// the legacy walk (`force_legacy_walk`) to form the reference lane from the
/// same assembly.
fn build_machine(scheduler: bool, tick_interval: u32) -> Machine<CortexM> {
    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);

    // Mirror the production attach choke for the hand-built SysTick entry.
    let clock = bus.cycle_clock.clone();
    let systick_idx = bus.find_peripheral_index_by_name("systick").unwrap();
    bus.peripherals[systick_idx]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<Systick>()
        .unwrap()
        .attach_cycle_clock(clock);

    if !scheduler {
        bus.peripherals[systick_idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Systick>()
            .unwrap()
            .force_legacy_walk();
        let scb_idx = bus.find_peripheral_index_by_name("scb").unwrap();
        bus.peripherals[scb_idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Scb>()
            .unwrap()
            .force_legacy_walk();
    }

    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = tick_interval;
    machine.bus.config.peripheral_tick_interval = tick_interval;
    machine.cpu.sp = INITIAL_SP;
    machine
}

/// The shared ISR at 0x80: increment the word at `ISR_COUNT_ADDR`, return.
///
///   0x80: 4802  ldr r0, [pc, #8]    ; = ISR_COUNT_ADDR (pool at 0x8C)
///   0x82: 6801  ldr r1, [r0]
///   0x84: 3101  adds r1, #1
///   0x86: 6001  str r1, [r0]
///   0x88: 4770  bx lr
///   0x8A: bf00  nop (pool align)
///   0x8C: .word 0x2000_0000
const ISR_BASE: u64 = 0x80;
fn load_isr(bus: &mut SystemBus) {
    load_thumb(
        bus,
        ISR_BASE,
        &[0x4802, 0x6801, 0x3101, 0x6001, 0x4770, 0xBF00],
    );
    write_word(bus, ISR_BASE + 0xC, ISR_COUNT_ADDR as u32);
}

/// Firmware A — arm SysTick (RVR=99 → period 100), then spin incrementing the
/// main counter while polling SYST_CVR (the lazy-read surface). The SysTick
/// ISR (exception 15, vector at 0x3C) increments the ISR counter.
///
///   0x40: 4806  ldr r0, [pc, #24]   ; = 0xE000_E010 (pool at 0x5C)
///   0x42: 2163  movs r1, #99
///   0x44: 6041  str r1, [r0, #4]    ; SYST_RVR = 99
///   0x46: 2100  movs r1, #0
///   0x48: 6081  str r1, [r0, #8]    ; SYST_CVR write → counter cleared
///   0x4A: 2107  movs r1, #7
///   0x4C: 6001  str r1, [r0, #0]    ; SYST_CSR = ENABLE|TICKINT|CLKSOURCE
///   0x4E: 4A05  ldr r2, [pc, #20]   ; = MAIN_COUNT_ADDR (pool at 0x64)
///   0x50: 2300  movs r3, #0
///   loop:
///   0x52: 3301  adds r3, #1
///   0x54: 6013  str r3, [r2]
///   0x56: 6884  ldr r4, [r0, #8]    ; poll SYST_CVR (lazy read)
///   0x58: E7FB  b loop
///   0x5A: BF00  nop (pool align)
///   0x5C: .word 0xE000_E010
///   0x60: .word 0 (pad)
///   0x64: .word 0x2000_0004
fn load_firmware_systick_counter(bus: &mut SystemBus) {
    write_word(bus, 0x3C, (ISR_BASE as u32) | 1); // vector 15 → ISR
    load_thumb(
        bus,
        0x40,
        &[
            0x4806, 0x2163, 0x6041, 0x2100, 0x6081, 0x2107, 0x6001, 0x4A05, 0x2300, 0x3301, 0x6013,
            0x6884, 0xE7FB, 0xBF00,
        ],
    );
    write_word(bus, 0x5C, 0xE000_E010);
    write_word(bus, 0x64, MAIN_COUNT_ADDR as u32);
    load_isr(bus);
}

/// Firmware B — the SCB leg: pend PendSV via ICSR.PENDSVSET, then spin. The
/// PendSV ISR (exception 14, vector at 0x38) increments the ISR counter.
///
///   0x40: 4802  ldr r0, [pc, #8]    ; = 0xE000_ED04 (pool at 0x4C)
///   0x42: 4903  ldr r1, [pc, #12]   ; = 0x1000_0000  (pool at 0x50)
///   0x44: 6001  str r1, [r0]        ; ICSR = PENDSVSET
///   0x46: 2300  movs r3, #0
///   loop:
///   0x48: 3301  adds r3, #1
///   0x4A: E7FD  b loop
///   0x4C: .word 0xE000_ED04
///   0x50: .word 0x1000_0000
fn load_firmware_pendsv(bus: &mut SystemBus) {
    write_word(bus, 0x38, (ISR_BASE as u32) | 1); // vector 14 → ISR
    load_thumb(bus, 0x40, &[0x4802, 0x4903, 0x6001, 0x2300, 0x3301, 0xE7FD]);
    write_word(bus, 0x4C, 0xE000_ED04);
    write_word(bus, 0x50, 0x1000_0000);
    load_isr(bus);
}

/// Firmware C — the canonical idle pattern: `cpsid i` (PRIMASK set while
/// arming, the `__disable_irq(); wfi()` shape), arm SysTick (RVR=199 → period
/// 200), `cpsie i`, then loop { wfi; main_count++ }. Each wake is driven by a
/// SCHEDULED SysTick expiry: the pended exception 15 is taken on wake
/// (PRIMASK clear), the ISR increments its counter, and the loop re-sleeps.
///
///   0x40: B672  cpsid i
///   0x42: 4807  ldr r0, [pc, #28]   ; = 0xE000_E010 (pool at 0x60)
///   0x44: 21C7  movs r1, #199
///   0x46: 6041  str r1, [r0, #4]    ; RVR
///   0x48: 2100  movs r1, #0
///   0x4A: 6081  str r1, [r0, #8]    ; CVR
///   0x4C: 2107  movs r1, #7
///   0x4E: 6001  str r1, [r0, #0]    ; CSR
///   0x50: 4A04  ldr r2, [pc, #16]   ; = MAIN_COUNT_ADDR (pool at 0x64)
///   0x52: 2300  movs r3, #0
///   0x54: B662  cpsie i
///   loop:
///   0x56: BF30  wfi
///   0x58: 3301  adds r3, #1
///   0x5A: 6013  str r3, [r2]
///   0x5C: E7FB  b loop
///   0x5E: BF00  nop (pool align)
///   0x60: .word 0xE000_E010
///   0x64: .word 0x2000_0004
fn load_firmware_wfi(bus: &mut SystemBus) {
    write_word(bus, 0x3C, (ISR_BASE as u32) | 1); // vector 15 → ISR
    load_thumb(
        bus,
        0x40,
        &[
            0xB672, 0x4807, 0x21C7, 0x6041, 0x2100, 0x6081, 0x2107, 0x6001, 0x4A04, 0x2300, 0xB662,
            0xBF30, 0x3301, 0x6013, 0xE7FB, 0xBF00,
        ],
    );
    write_word(bus, 0x60, 0xE000_E010);
    write_word(bus, 0x64, MAIN_COUNT_ADDR as u32);
    load_isr(bus);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Probe {
    step: u64,
    total_cycles: u64,
    pc: u32,
    regs: [u32; 16],
    isr_count: u32,
    main_count: u32,
}

fn probe(machine: &Machine<CortexM>, step: u64) -> Probe {
    let mut regs = [0u32; 16];
    for (i, r) in regs.iter_mut().enumerate() {
        *r = machine.read_core_reg(i as u8);
    }
    Probe {
        step,
        total_cycles: machine.total_cycles,
        pc: machine.get_pc(),
        regs,
        isr_count: machine.bus.read_u32(ISR_COUNT_ADDR).unwrap(),
        main_count: machine.bus.read_u32(MAIN_COUNT_ADDR).unwrap(),
    }
}

/// Run `steps` instructions one at a time through the batched `Machine::run`
/// path (the production path the scheduler timing convention is calibrated
/// to), probing the full architectural state after every instruction.
fn run_probed(machine: &mut Machine<CortexM>, entry: u32, steps: u64) -> Vec<Probe> {
    machine.cpu.pc = entry;
    let mut probes = Vec::with_capacity(steps as usize);
    for s in 0..steps {
        machine.run(Some(1)).unwrap();
        probes.push(probe(machine, s + 1));
    }
    probes
}

/// First divergence pretty-printer: comparing two 3000-element probe vectors
/// with assert_eq! produces an unreadable dump; find the first mismatch.
fn assert_probes_identical(reference: &[Probe], candidate: &[Probe], what: &str) {
    assert_eq!(reference.len(), candidate.len());
    for (r, c) in reference.iter().zip(candidate.iter()) {
        assert_eq!(
            r, c,
            "{what}: first divergence at step {} (walk-reference vs scheduler)",
            r.step
        );
    }
}

/// Gate 1: the delay-loop + SysTick-IRQ-counter firmware, walk-on vs
/// scheduler at tick interval 1 — every instruction-boundary observable
/// byte-identical (IRQ delivery cycles, ISR execution count, total_cycles,
/// registers — including r4, the raw SYST_CVR poll).
#[test]
fn systick_irq_firmware_is_byte_identical_at_interval_1() {
    const STEPS: u64 = 3_000;

    let mut walk = build_machine(false, 1);
    load_firmware_systick_counter(&mut walk.bus);
    let walk_probes = run_probed(&mut walk, 0x40, STEPS);

    let mut sched = build_machine(true, 1);
    load_firmware_systick_counter(&mut sched.bus);
    let sched_probes = run_probed(&mut sched, 0x40, STEPS);

    // The firmware must actually exercise the surface: multiple ISR entries
    // and a live main loop.
    let last = walk_probes.last().unwrap();
    assert!(
        last.isr_count >= 25,
        "reference must take repeated SysTick ISRs (got {})",
        last.isr_count
    );
    assert!(last.main_count > 500, "main loop must run");

    assert_probes_identical(&walk_probes, &sched_probes, "systick counter firmware");
}

/// Gate 2: scheduler @ interval 64 vs the walk-on interval-1 golden
/// reference. Exception delivery quantises to the batch grid (≤ one interval,
/// documented), so per-instruction state is NOT compared — but the ISR count
/// over the fixed window must be EXACT (absolute deadlines, no drift), and
/// the window edge is verified to be more than one interval away from any
/// delivery in the reference.
#[test]
fn systick_isr_count_is_exact_at_interval_64() {
    const STEPS: u64 = 6_000;

    let mut walk = build_machine(false, 1);
    load_firmware_systick_counter(&mut walk.bus);
    let walk_probes = run_probed(&mut walk, 0x40, STEPS);

    let mut sched = build_machine(true, 64);
    load_firmware_systick_counter(&mut sched.bus);
    sched.cpu.pc = 0x40;
    sched.run(Some(STEPS as u32)).unwrap();

    let reference = walk_probes.last().unwrap();
    let sched_isr = sched.bus.read_u32(ISR_COUNT_ADDR).unwrap();

    // The quantisation carve-out only smears delivery by < one interval, so
    // the count can only differ if a fire lands within one interval of the
    // window edge. Verify the fixture stays clear of the edge using the
    // reference's own delivery trace (the last ISR-count increment).
    // Quantisation can only DELAY a delivery (event deadlines are clamped
    // `max(now)`, never early), by < one interval (64) plus the bounded
    // exception-entry lag (< 16 cycles here). So the count can only differ if
    // the last expiry sits within ~80 cycles of the window edge.
    let last_delivery_step = walk_probes
        .iter()
        .zip(walk_probes.iter().skip(1))
        .filter(|(a, b)| b.isr_count > a.isr_count)
        .map(|(_, b)| b.step)
        .next_back()
        .expect("reference delivers at least one ISR");
    assert!(
        STEPS - last_delivery_step > 80,
        "fixture must keep the window edge > interval + entry lag from the last \
         delivery (last at step {last_delivery_step})"
    );

    assert!(
        reference.isr_count >= 25,
        "reference must take repeated ISRs"
    );
    assert_eq!(
        sched_isr, reference.isr_count,
        "ISR execution count over the fixed window must be exact at interval 64"
    );
    // total_cycles advances one per instruction in both lanes (B1 removed the
    // legacy SysTick tick-cost artifact), so the window length itself matches.
    assert_eq!(sched.total_cycles, reference.total_cycles);
}

/// Gate 3: the SCB leg — ICSR.PENDSVSET → PendSV (exception 14) delivered on
/// the SAME cycle in both lanes at interval 1, per-instruction byte-identity.
#[test]
fn pendsv_firmware_is_byte_identical_at_interval_1() {
    const STEPS: u64 = 40;

    let mut walk = build_machine(false, 1);
    load_firmware_pendsv(&mut walk.bus);
    let walk_probes = run_probed(&mut walk, 0x40, STEPS);

    let mut sched = build_machine(true, 1);
    load_firmware_pendsv(&mut sched.bus);
    let sched_probes = run_probed(&mut sched, 0x40, STEPS);

    assert_eq!(
        walk_probes.last().unwrap().isr_count,
        1,
        "PendSV ISR must run exactly once in the reference"
    );
    assert_probes_identical(&walk_probes, &sched_probes, "pendsv firmware");
}

/// Gate 4: WFI idle fast-forward + SysTick. The sleeping firmware must
/// fast-forward to the SCHEDULED expiry (`next_event_deadline` clamps the FF
/// budget) and wake correctly: identical architectural end state vs the
/// FF-off run at the same step budget, with far fewer retired instructions.
#[test]
fn wfi_sleep_wakes_from_scheduled_systick_event() {
    const STEPS: u32 = 4_000;

    let build = |ff: bool| -> Machine<CortexM> {
        let mut machine = build_machine(true, 1);
        load_firmware_wfi(&mut machine.bus);
        machine.config.idle_fast_forward_enabled = ff;
        // FF is only legal when nothing depends on the legacy walk;
        // SysTick+SCB are scheduler-driven now, so delete the walk exactly
        // like the existing WFI FF fixtures do.
        machine.bus.legacy_walk_disabled = true;
        machine.cpu.pc = 0x40;
        machine
    };

    let mut ff_off = build(false);
    ff_off.run(Some(STEPS)).unwrap();
    let off_probe = probe(&ff_off, STEPS as u64);

    let mut ff_on = build(true);
    ff_on.reset_step_profile();
    ff_on.run(Some(STEPS)).unwrap();
    let on_probe = probe(&ff_on, STEPS as u64);

    // The pattern must actually sleep-and-wake repeatedly, driven by the
    // scheduled SysTick expiries (period 200 → ~20 wakes in 4000 cycles).
    assert!(
        off_probe.isr_count >= 10,
        "WFI loop must take repeated SysTick ISRs (got {})",
        off_probe.isr_count
    );

    // Fast-forward must not change WHEN things happen or HOW OFTEN the ISR
    // runs — only how many instructions the CPU retires getting there. (The
    // FF-off lane treats WFI as a spin — the platform's documented no-FF
    // semantics — so thread-loop progress counters legitimately differ; the
    // event-driven observables must not.)
    assert_eq!(on_probe.total_cycles, off_probe.total_cycles);
    assert_eq!(
        on_probe.isr_count, off_probe.isr_count,
        "every scheduled SysTick expiry must wake the sleeping core and run the ISR"
    );

    // Each wake resumes the interrupted thread: exactly one loop pass (one
    // main_count increment) per wake before the next sleep.
    let wakes = on_probe.isr_count as i64;
    let passes = on_probe.main_count as i64;
    assert!(
        (wakes - passes).abs() <= 1,
        "FF lane must run one thread pass per wake (isr {wakes}, main {passes})"
    );

    let retired = ff_on.step_profile().cpu_instructions;
    assert!(
        retired < (STEPS as u64) / 4,
        "idle fast-forward must skip the sleeping cycles ({} retired of {})",
        retired,
        STEPS
    );
}
