// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Walk≡scheduler fidelity gates for nRF52 TIMER / RTC / RADIO at tick 512.
//!
//! Nrf52Timer / Nrf52Rtc have no `force_legacy_walk` knob (scheduler mode is
//! clock-presence). Dual lanes therefore use the production walk-free
//! nRF52840 DK bus with:
//!
//! - **Lane A** — `peripheral_tick_interval = 1` (every-cycle drain + bus_tick)
//! - **Lane B** — `peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL` (512)
//!
//! Both ride the event scheduler under Machine + walk-free; completion is
//! observed with single-cycle advance batches so absolute cycle identity is
//! measurable (not quantised to a 512-batch boundary).
//!
//! Surfaces:
//! 1. TIMER0 COMPARE[0] — program CC/START/INTEN, assert same fire cycle
//!    (within 1) and EVENTS_COMPARE[0] on both lanes.
//! 2. RTC0 COMPARE[0] — EVTEN+INTEN compare path.
//! 3. RTC0 COUNTER poll-only — no INTEN/EVTEN; read-side CycleClock sync must
//!    advance COUNTER under tick-512 batching and match walk@1 within one tick.
//! 4. RADIO TXEN → START → END — walk@1≡sched@512 cycle identity.
//! 5. RADIO Ble_1Mbit bit-time — END at model air cycles (±1) and length
//!    scaling; other MODEs remain interim on the scoreboard.
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

/// Advance ≤ `max_cycles` one cycle at a time; return total_cycles when
/// `done` returns true, or None if the budget is exhausted.
fn advance_until<F>(machine: &mut Machine<CycleCpu>, max_cycles: u64, mut done: F) -> Option<u64>
where
    F: FnMut(&Machine<CycleCpu>) -> bool,
{
    while machine.total_cycles < max_cycles {
        machine
            .advance(AdvanceRequest::run(Some(1)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("Machine::advance");
        if done(machine) {
            return Some(machine.total_cycles);
        }
    }
    None
}

fn assert_cycle_identity(at_a: u64, at_b: u64, what: &str) {
    let delta = at_a.abs_diff(at_b);
    assert!(
        delta <= 1,
        "{what}: completion cycle must agree within 1 \
         (interval=1 at={at_a}, interval={RECOMMENDED_TICK_INTERVAL} at={at_b}, delta={delta})"
    );
}

// ── TIMER0 COMPARE ──────────────────────────────────────────────────────────

const TIMER0: u64 = 0x4000_8000;
const TIMER_TASKS_START: u64 = TIMER0;
const TIMER_TASKS_CLEAR: u64 = TIMER0 + 0x00C;
const TIMER_EVENTS_COMPARE0: u64 = TIMER0 + 0x140;
const TIMER_INTENSET: u64 = TIMER0 + 0x304;
const TIMER_BITMODE: u64 = TIMER0 + 0x508;
const TIMER_PRESCALER: u64 = TIMER0 + 0x510;
const TIMER_CC0: u64 = TIMER0 + 0x540;

fn arm_timer0_compare(machine: &mut Machine<CycleCpu>, cc: u32) {
    machine.bus.write_u32(TIMER_BITMODE, 3).unwrap(); // 32-bit
    machine.bus.write_u32(TIMER_PRESCALER, 0).unwrap();
    machine.bus.write_u32(TIMER_CC0, cc).unwrap();
    machine.bus.write_u32(TIMER_INTENSET, 1 << 16).unwrap(); // COMPARE[0]
    machine.bus.write_u32(TIMER_EVENTS_COMPARE0, 0).unwrap();
    machine.bus.write_u32(TIMER_TASKS_CLEAR, 1).unwrap();
    machine.bus.write_u32(TIMER_TASKS_START, 1).unwrap();
}

fn timer_compare_done(m: &Machine<CycleCpu>) -> bool {
    m.bus.read_u32(TIMER_EVENTS_COMPARE0).unwrap_or(0) != 0
}

/// TIMER0 COMPARE[0]: walk-free interval 1 vs interval 512 — same fire cycle
/// (within 1) and same EVENTS_COMPARE[0] latch.
#[test]
fn timer0_compare_walk1_vs_sched512_cycle_identity() {
    const CC: u32 = 8;
    // CC=8 at PRESCALER=0 → match after 8 base ticks; headroom for arm lag.
    const BUDGET: u64 = 64;

    let mut lane_a = machine_at_interval(1);
    arm_timer0_compare(&mut lane_a, CC);
    let at_a = advance_until(&mut lane_a, BUDGET, timer_compare_done)
        .expect("lane A (interval=1) must fire TIMER0 COMPARE[0]");

    let mut lane_b = machine_at_interval(RECOMMENDED_TICK_INTERVAL);
    arm_timer0_compare(&mut lane_b, CC);
    let at_b = advance_until(&mut lane_b, BUDGET, timer_compare_done)
        .expect("lane B (interval=512) must fire TIMER0 COMPARE[0] via scheduler");

    assert!(
        timer_compare_done(&lane_a) && timer_compare_done(&lane_b),
        "both lanes must latch EVENTS_COMPARE[0]"
    );
    assert_cycle_identity(at_a, at_b, "TIMER0 COMPARE[0]");
}

// ── RTC0 COMPARE (EVTEN + INTEN path) ───────────────────────────────────────

const RTC0: u64 = 0x4000_B000;
const RTC_TASKS_START: u64 = RTC0;
const RTC_TASKS_CLEAR: u64 = RTC0 + 0x008;
const RTC_EVENTS_COMPARE0: u64 = RTC0 + 0x140;
const RTC_INTENSET: u64 = RTC0 + 0x304;
const RTC_EVTENSET: u64 = RTC0 + 0x344;
const RTC_PRESCALER: u64 = RTC0 + 0x508;
const RTC_CC0: u64 = RTC0 + 0x540;

fn arm_rtc0_compare(machine: &mut Machine<CycleCpu>, cc: u32) {
    // PRESCALER must be written while stopped.
    machine.bus.write_u32(RTC_PRESCALER, 0).unwrap();
    machine.bus.write_u32(RTC_CC0, cc).unwrap();
    // EVTEN required for EVENTS_COMPARE latch; INTEN for IRQ surface.
    machine.bus.write_u32(RTC_EVTENSET, 1 << 16).unwrap();
    machine.bus.write_u32(RTC_INTENSET, 1 << 16).unwrap();
    machine.bus.write_u32(RTC_EVENTS_COMPARE0, 0).unwrap();
    machine.bus.write_u32(RTC_TASKS_CLEAR, 1).unwrap();
    machine.bus.write_u32(RTC_TASKS_START, 1).unwrap();
}

fn rtc_compare_done(m: &Machine<CycleCpu>) -> bool {
    m.bus.read_u32(RTC_EVENTS_COMPARE0).unwrap_or(0) != 0
}

/// RTC0 COMPARE[0] (EVTEN+INTEN): interval 1 vs 512 — same fire cycle within 1.
///
/// Real LFCLK ratio (~1953 CPU cycles per RTC tick) with CC[0]=2 needs
/// ~3906 cycles; budget keeps clear of batch-edge ambiguity.
#[test]
fn rtc0_compare_walk1_vs_sched512_cycle_identity() {
    const CC: u32 = 2;
    // 2 × ~1953 ≈ 3906; generous headroom for scheduler arm / LFCLK fraction.
    const BUDGET: u64 = 12_000;

    let mut lane_a = machine_at_interval(1);
    arm_rtc0_compare(&mut lane_a, CC);
    let at_a = advance_until(&mut lane_a, BUDGET, rtc_compare_done)
        .expect("lane A (interval=1) must fire RTC0 COMPARE[0]");

    let mut lane_b = machine_at_interval(RECOMMENDED_TICK_INTERVAL);
    arm_rtc0_compare(&mut lane_b, CC);
    let at_b = advance_until(&mut lane_b, BUDGET, rtc_compare_done)
        .expect("lane B (interval=512) must fire RTC0 COMPARE[0] via scheduler");

    assert!(
        rtc_compare_done(&lane_a) && rtc_compare_done(&lane_b),
        "both lanes must latch EVENTS_COMPARE[0] (EVTEN compare path)"
    );
    assert_cycle_identity(at_a, at_b, "RTC0 COMPARE[0]");
}

// ── RTC0 COUNTER poll-only (no INTEN/EVTEN) ──────────────────────────────────

const RTC_COUNTER: u64 = RTC0 + 0x504;

fn arm_rtc0_counter_poll(machine: &mut Machine<CycleCpu>) {
    // PRESCALER must be written while stopped. No INTEN / EVTEN — pure COUNTER
    // poll fidelity under walk-free batching.
    machine.bus.write_u32(RTC_PRESCALER, 0).unwrap();
    machine.bus.write_u32(RTC_TASKS_CLEAR, 1).unwrap();
    machine.bus.write_u32(RTC_TASKS_START, 1).unwrap();
}

/// Real LFCLK: ~1953.125 CPU cycles per COUNTER increment at PRESCALER=0.
const RTC_CYCLES_PER_TICK: u64 = 1954;

/// RTC0 COUNTER poll under Machine@512: after many cycles the free-running
/// COUNTER must advance proportionally (not stick at 0 mid-batch). Read-side
/// CycleClock sync supplies batch-boundary freshness.
#[test]
fn rtc_counter_poll_advances_under_sched_tick512() {
    // Budget: enough for ≥ 4 COUNTER ticks at real LFCLK (~7816 cycles).
    const BUDGET: u64 = 10_000;
    let mut m = machine_at_interval(RECOMMENDED_TICK_INTERVAL);
    arm_rtc0_counter_poll(&mut m);

    m.advance(AdvanceRequest::run(Some(BUDGET)).with_breakpoints(BreakpointPolicy::Ignore))
        .expect("Machine::advance");

    let counter = m.bus.read_u32(RTC_COUNTER).unwrap_or(0);
    let expected_min = (BUDGET / RTC_CYCLES_PER_TICK).saturating_sub(1) as u32;
    assert!(
        counter >= expected_min && counter > 0,
        "COUNTER must advance under tick-512 batching with no INTEN/EVTEN \
         (got {counter}, expected ≥ {expected_min} after {BUDGET} cycles)"
    );
}

/// RTC0 COUNTER poll: interval 1 vs 512 after the same cycle budget must agree
/// within documented batch-boundary freshness (≤ one interval of quantisation).
/// Single-cycle advance steps make the published clock exact on both lanes;
/// a coarser advance of 512 on lane B may trail by ≤ RECOMMENDED_TICK_INTERVAL
/// CPU cycles of LFCLK quantisation (≤ 1 COUNTER tick at PRESCALER=0).
#[test]
fn rtc_counter_poll_walk1_vs_sched512_identity() {
    // Same budget as the advance gate — several COUNTER ticks of headroom.
    const BUDGET: u64 = 10_000;

    let mut lane_a = machine_at_interval(1);
    arm_rtc0_counter_poll(&mut lane_a);
    // Single-cycle steps: published clock is exact at every boundary.
    for _ in 0..BUDGET {
        lane_a
            .advance(AdvanceRequest::run(Some(1)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("lane A advance");
    }
    let counter_a = lane_a.bus.read_u32(RTC_COUNTER).unwrap_or(0);

    let mut lane_b = machine_at_interval(RECOMMENDED_TICK_INTERVAL);
    arm_rtc0_counter_poll(&mut lane_b);
    // Same absolute cycle budget; batch size = interval for the sched lane.
    lane_b
        .advance(AdvanceRequest::run(Some(BUDGET)).with_breakpoints(BreakpointPolicy::Ignore))
        .expect("lane B advance");
    let counter_b = lane_b.bus.read_u32(RTC_COUNTER).unwrap_or(0);

    assert!(
        counter_a > 0 && counter_b > 0,
        "both lanes must observe a non-zero COUNTER (a={counter_a}, b={counter_b})"
    );
    // Batch-boundary freshness: at most one COUNTER tick of LFCLK quantisation
    // (interval 512 << ~1953 cycles/tick → typically exact COUNTER match).
    let delta = counter_a.abs_diff(counter_b);
    assert!(
        delta <= 1,
        "COUNTER poll walk@1 vs sched@512 must agree within ≤1 tick \
         (batch-boundary freshness); got a={counter_a} b={counter_b} delta={delta}"
    );
}

// ── RADIO TXEN → START → END (minimal) ──────────────────────────────────────

const RADIO: u64 = 0x4000_1000;
const RADIO_TASKS_TXEN: u64 = RADIO;
const RADIO_TASKS_START: u64 = RADIO + 0x008;
const RADIO_EVENTS_READY: u64 = RADIO + 0x100;
const RADIO_EVENTS_END: u64 = RADIO + 0x10C;
const RADIO_SHORTS: u64 = RADIO + 0x200;
const RADIO_PACKETPTR: u64 = RADIO + 0x504;
const RADIO_FREQUENCY: u64 = RADIO + 0x508;
const RADIO_MODE: u64 = RADIO + 0x510;
const RADIO_PCNF0: u64 = RADIO + 0x514;
const RADIO_PCNF1: u64 = RADIO + 0x518;
const RADIO_BASE0: u64 = RADIO + 0x51C;
const RADIO_PREFIX0: u64 = RADIO + 0x524;
// SHORTS bit 0 = READY_START (PS table 224).
const SHORT_READY_START: u32 = 1 << 0;

/// Model constants (see `Nrf52Radio::cycles_for_packet`): BLE_1Mbit = MODE 3
/// → 8 cycles/byte; air time uses payload LENGTH + 3 CRC bytes.
const RADIO_MODE_BLE_1MBIT: u32 = 0x3;
const BLE_1MBIT_CYCLES_PER_BYTE: u64 = 8;
/// Fixed overhead: TXEN→READY (1) + EasyDMA arm (1) before the air countdown.
const RADIO_TX_CHAIN_OVERHEAD: u64 = 2;

fn plant_radio_tx_buf_len(bus: &mut SystemBus, base: u64, payload_len: u8) {
    // S0 + LENGTH + payload (BLE-like layout; matches unit-test PCNF0).
    bus.write_u8(base, 0x40).unwrap(); // S0
    bus.write_u8(base + 1, payload_len).unwrap();
    for i in 0..payload_len {
        bus.write_u8(base + 2 + u64::from(i), 0xA5u8.wrapping_add(i))
            .unwrap();
    }
}

fn arm_radio_tx_with_len(machine: &mut Machine<CycleCpu>, buf: u64, payload_len: u8) {
    plant_radio_tx_buf_len(&mut machine.bus, buf, payload_len);
    machine.bus.write_u32(RADIO_FREQUENCY, 0x4E).unwrap(); // BLE adv ch 37
    machine
        .bus
        .write_u32(RADIO_MODE, RADIO_MODE_BLE_1MBIT)
        .unwrap();
    // LFLEN=8, S0LEN=1 (same encoding as `nrf52/radio.rs` unit tests).
    machine.bus.write_u32(RADIO_PCNF0, 0x0000_0108).unwrap();
    // MAXLEN=0xFF, STATLEN=0 — air bytes = LENGTH + 3 CRC.
    machine.bus.write_u32(RADIO_PCNF1, 0x0000_00FF).unwrap();
    machine.bus.write_u32(RADIO_BASE0, 0xCAFE_BABE).unwrap();
    machine.bus.write_u32(RADIO_PREFIX0, 0xDEAD).unwrap();
    machine.bus.write_u32(RADIO_PACKETPTR, buf as u32).unwrap();
    machine
        .bus
        .write_u32(RADIO_SHORTS, SHORT_READY_START)
        .unwrap();
    machine.bus.write_u32(RADIO_EVENTS_READY, 0).unwrap();
    machine.bus.write_u32(RADIO_EVENTS_END, 0).unwrap();
    machine.bus.write_u32(RADIO_TASKS_TXEN, 1).unwrap();
}

fn arm_radio_tx(machine: &mut Machine<CycleCpu>, buf: u64) {
    arm_radio_tx_with_len(machine, buf, 1);
}

/// Model air time for BLE_1Mbit: `(payload_len + 3 CRC) × 8` cycles.
fn ble_1mbit_air_cycles(payload_len: u8) -> u64 {
    (u64::from(payload_len) + 3) * BLE_1MBIT_CYCLES_PER_BYTE
}

fn radio_end_done(m: &Machine<CycleCpu>) -> bool {
    m.bus.read_u32(RADIO_EVENTS_END).unwrap_or(0) != 0
}

/// RADIO TX start chain: TXEN + READY_START short → EVENTS_END.
/// Interval 1 vs 512 must raise END and agree on completion cycle within 1.
/// Bit-rate scaling for fixed MODE is certified separately below.
#[test]
fn radio_tx_end_walk1_vs_sched512_cycle_identity() {
    // READY + DMA + BLE_1Mbit air for L=1 → (1+3)*8 = 32 + overhead.
    const BUDGET: u64 = 128;
    let buf = 0x2000_2000u64;

    let mut lane_a = machine_at_interval(1);
    arm_radio_tx(&mut lane_a, buf);
    // Explicit START is redundant with SHORTS READY_START but harmless if
    // READY has not yet fired; TXEN arms the scheduler chain.
    let _ = lane_a.bus.write_u32(RADIO_TASKS_START, 1);
    let at_a = advance_until(&mut lane_a, BUDGET, radio_end_done)
        .expect("lane A (interval=1) must raise RADIO EVENTS_END");

    let mut lane_b = machine_at_interval(RECOMMENDED_TICK_INTERVAL);
    arm_radio_tx(&mut lane_b, buf);
    let _ = lane_b.bus.write_u32(RADIO_TASKS_START, 1);
    let at_b = advance_until(&mut lane_b, BUDGET, radio_end_done)
        .expect("lane B (interval=512) must raise RADIO EVENTS_END via scheduler");

    assert!(
        radio_end_done(&lane_a) && radio_end_done(&lane_b),
        "both lanes must latch EVENTS_END"
    );
    assert_cycle_identity(at_a, at_b, "RADIO TX→END");
}

/// Bit-time fidelity for **fixed MODE = Ble_1Mbit** only:
/// 1. EVENTS_END at model air time ±1 cycle (plus fixed TXEN chain overhead).
/// 2. Two payload lengths: Δcycles proportional to Δlength × 8 (±1).
///
/// Does **not** claim the full MODE matrix (2Mbit / LR / 802.15.4 remain interim).
#[test]
fn radio_ble_1mbit_bit_time_scales_with_length() {
    const L1: u8 = 1;
    const L2: u8 = 8;
    // Overhead + air for L2: 2 + (8+3)*8 = 90; keep headroom.
    const BUDGET: u64 = 256;
    let buf = 0x2000_2000u64;

    let run = |payload_len: u8, interval: u32| -> u64 {
        let mut m = machine_at_interval(interval);
        arm_radio_tx_with_len(&mut m, buf, payload_len);
        let _ = m.bus.write_u32(RADIO_TASKS_START, 1);
        advance_until(&mut m, BUDGET, radio_end_done).unwrap_or_else(|| {
            panic!("EVENTS_END missing for L={payload_len} interval={interval} within {BUDGET}")
        })
    };

    // Absolute model: END ≈ overhead + (L+3)*8, within ±1.
    let at_l1_i1 = run(L1, 1);
    let expected_l1 = RADIO_TX_CHAIN_OVERHEAD + ble_1mbit_air_cycles(L1);
    let delta_abs = at_l1_i1.abs_diff(expected_l1);
    assert!(
        delta_abs <= 1,
        "Ble_1Mbit L={L1}: END at {at_l1_i1}, model expected {expected_l1} (±1); \
         air=(L+3)*{BLE_1MBIT_CYCLES_PER_BYTE}, overhead={RADIO_TX_CHAIN_OVERHEAD}"
    );

    // Walk@1 ≡ sched@512 for the same length (bit-time path, not just short).
    let at_l1_i512 = run(L1, RECOMMENDED_TICK_INTERVAL);
    assert_cycle_identity(at_l1_i1, at_l1_i512, "RADIO Ble_1Mbit L1 bit-time");

    // Length scaling: ΔEND = (L2−L1)*8 within ±1.
    let at_l2_i1 = run(L2, 1);
    let expected_l2 = RADIO_TX_CHAIN_OVERHEAD + ble_1mbit_air_cycles(L2);
    assert!(
        at_l2_i1.abs_diff(expected_l2) <= 1,
        "Ble_1Mbit L={L2}: END at {at_l2_i1}, model expected {expected_l2} (±1)"
    );
    let delta = at_l2_i1 as i64 - at_l1_i1 as i64;
    let expected_delta = ((L2 - L1) as i64) * BLE_1MBIT_CYCLES_PER_BYTE as i64;
    assert!(
        (delta - expected_delta).unsigned_abs() <= 1,
        "Ble_1Mbit length scaling: Δcycles L{L2}−L{L1} = {delta}, \
         expected {expected_delta} (±1); at_l1={at_l1_i1} at_l2={at_l2_i1}"
    );

    // Sched@512 must scale the same way.
    let at_l2_i512 = run(L2, RECOMMENDED_TICK_INTERVAL);
    assert_cycle_identity(at_l2_i1, at_l2_i512, "RADIO Ble_1Mbit L2 bit-time");
}
