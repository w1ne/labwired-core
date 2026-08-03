// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every nRF52840 peripheral named in `RULE_A_ALLOWLIST` must reach the NVIC on
//! the SHIPPED (walk-deleted) `nrf52840-dk` bus.
//!
//! ## The observable
//!
//! `NVIC.ISPR[n]` — the bit the Cortex-M4 core samples at its instruction
//! boundary. Not `legacy_walk_disabled`, not the model's own `EVENTS_*`
//! register: the bit that reaches the CPU. Same instrument as
//! `rp2040_i2c_irq_delivery`.
//!
//! ## Why these tests exist even though they pass on `main`
//!
//! The seven nRF52 rule-A entries were recorded as live instances of the
//! RMT / RP2040-I2C defect. They are NOT. Every one of them already declares
//! `uses_scheduler() -> true` plus a real `take_scheduled_events` / `on_event`
//! chain, and `needs_legacy_walk()` has exactly ONE consumer in the tree —
//! `SystemBus::derive_walk_deletable`, where it is OR-ed with
//! `uses_scheduler()`:
//!
//! ```ignore
//! self.peripherals.iter().all(|p| p.dev.uses_scheduler() || !p.dev.needs_legacy_walk())
//! ```
//!
//! An unconditional `uses_scheduler() -> true` short-circuits that OR, so the
//! `needs_legacy_walk() -> false` override on these models is dead code: it
//! cannot change walk deletion, and it cannot change the walk skip (which keys
//! on `uses_scheduler()` alone). What it CAN do is state something false about
//! the model — `tick()` does work — which is what the static contract rejects
//! and what makes the next reader believe delivery rides the walk.
//!
//! So the repair is deleting the dead override, and the risk that repair
//! carries is a REGRESSION, not a fix: it must not disturb delivery. These
//! tests are the regression instrument, and they are pinned as non-vacuous by
//! [`the_delivery_probes_go_red_when_the_event_chain_is_cut`], which severs the
//! arming hook on a bare model and watches every probe's precondition fail.

#![cfg(feature = "event-scheduler")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::{SystemBus, RECOMMENDED_TICK_INTERVAL};
use labwired_core::snapshot::{ArmCpuSnapshot, CpuSnapshot};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{
    AdvanceRequest, BreakpointPolicy, Bus, Cpu, Machine, Peripheral, SimResult, SimulationConfig,
    SimulationObserver,
};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

// ── nRF52840 PS rev 1.7 §4.2 (instantiation) — bases and NVIC lines ─────────
const CLOCK: u64 = 0x4000_0000;
const CLOCK_IRQ: u32 = 0; // POWER_CLOCK
const RADIO: u64 = 0x4000_1000;
const RADIO_IRQ: u32 = 1;
const SERIAL0: u64 = 0x4000_3000; // SPIM0/SPIS0/TWIM0/TWIS0 mux ("i2c0")
const SERIAL0_IRQ: u32 = 3;
const TWI1: u64 = 0x4000_4000; // TWIM1 ("twi1")
const TWI1_IRQ: u32 = 4;
const GPIOTE: u64 = 0x4000_6000;
const GPIOTE_IRQ: u32 = 6;
const ECB: u64 = 0x4000_E000;
const ECB_IRQ: u32 = 14;
const EGU0: u64 = 0x4001_4000;
const EGU0_IRQ: u32 = 20;

/// GPIO P0. The chip descriptor remaps P1 to 0x50001000 (see the comment in
/// `configs/chips/nrf52840.yaml`); every probe here stays on P0.
const GPIO0: u64 = 0x5000_0000;
/// Nordic GPIO `IN` — 0x510. NOT `OUT` (0x504). Reading `IN` returns
/// `(OUT & DIR) | (IN & !DIR)`, so an input pin (DIR=0, the reset state)
/// reflects the written pad level and the bus edge scan sees it.
const GPIO_IN: u64 = 0x510;

/// nRF52840 RAM origin — EasyDMA pointers must land in real memory.
const RAM: u32 = 0x2000_0000;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Minimal cycle-advancing CPU: one cycle per step, so `Machine` drains the
/// event scheduler without real Thumb firmware. Same stand-in as
/// `rp2040_i2c_irq_delivery`.
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

/// The shipped `nrf52840-dk` bus with walk-deletion AUTO-DERIVED.
fn bus_nrf52840_dk() -> SystemBus {
    let chip = ChipDescriptor::from_file(root("configs/chips/nrf52840.yaml")).expect("chip yaml");
    let system_path = root("configs/systems/nrf52840-dk.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("system yaml");
    let anchored = system_path.parent().expect("parent").join(&manifest.chip);
    manifest.chip = anchored.to_string_lossy().into_owned();
    manifest.walk_deleted = None;
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build nrf52840 bus");
    let _ = configure_cortex_m(&mut bus);
    bus
}

fn machine_nrf52840_dk() -> Machine<CycleCpu> {
    let bus = bus_nrf52840_dk();
    assert!(
        bus.legacy_walk_disabled,
        "precondition: nrf52840-dk must auto-derive walk deletion — every probe \
         in this file is about delivery WITHOUT the per-cycle walk"
    );
    let mut machine = Machine::new(CycleCpu::default(), bus);
    machine.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.bus.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine
}

fn ispr(bus: &SystemBus, irq: u32) -> u32 {
    bus.nvic
        .as_ref()
        .map(|n| n.ispr[(irq / 32) as usize].load(Ordering::SeqCst))
        .unwrap_or(0)
}

fn pended(bus: &SystemBus, irq: u32) -> bool {
    ispr(bus, irq) & (1 << (irq % 32)) != 0
}

/// Run the real `Machine::advance` loop at the recommended tick interval until
/// `irq` pends, and return the total cycle count at which it FIRST pended.
///
/// Steps ONE cycle at a time so the returned cycle is exact — the radio probe
/// asserts on it, and a batch-granular answer would hide a deadline that is off
/// by up to 512 cycles.
fn cycles_until_pend(machine: &mut Machine<CycleCpu>, irq: u32, budget: u64) -> Option<u64> {
    while machine.total_cycles < budget {
        machine
            .advance(AdvanceRequest::run(Some(1)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("Machine::advance");
        if pended(&machine.bus, irq) {
            return Some(machine.total_cycles);
        }
    }
    None
}

/// Coarse twin for probes that only care THAT the line pends.
fn run_until_pend(machine: &mut Machine<CycleCpu>, irq: u32, budget: u64) -> bool {
    while machine.total_cycles < budget {
        machine
            .advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("Machine::advance");
        if pended(&machine.bus, irq) {
            return true;
        }
    }
    false
}

const BUDGET: u64 = 8_192;

macro_rules! starved {
    ($m:expr, $name:literal, $irq:expr, $extra:expr) => {
        format!(
            "STARVED: {} ({}) never pended within {} cycles, ISPR[{}]={:#x}. {}",
            $name,
            $irq,
            BUDGET,
            $irq / 32,
            ispr(&$m.bus, $irq),
            $extra
        )
    };
}

// ─────────────────────────────────────────────────────────────────────────────
// Nrf52Clock — "tick() re-pends a held HFCLKSTARTED/LFCLKSTARTED level"
// ─────────────────────────────────────────────────────────────────────────────

/// The Zephyr / nRF-SDK clock-start sequence must pend POWER_CLOCK.
#[test]
fn clock_hfclkstarted_pends_power_clock() {
    let mut m = machine_nrf52840_dk();
    m.bus.write_u32(CLOCK + 0x304, 1 << 0).unwrap(); // INTENSET.HFCLKSTARTED
    m.bus.write_u32(CLOCK, 1).unwrap(); // TASKS_HFCLKSTART
    assert_eq!(
        m.bus.read_u32(CLOCK + 0x100).unwrap(),
        1,
        "precondition: EVENTS_HFCLKSTARTED must latch on the task write"
    );
    assert!(
        run_until_pend(&mut m, CLOCK_IRQ, BUDGET),
        "{}",
        starved!(
            m,
            "POWER_CLOCK",
            CLOCK_IRQ,
            "Zephyr's nrf clock driver waits on this before the kernel starts."
        )
    );
}

/// The reverse arming order (task first, INTENSET second) must also pend.
///
/// The chain arms at the MMIO write choke, so ordering decides which write
/// observes the level. Both orders occur in real drivers.
#[test]
fn clock_pends_when_intenset_comes_after_the_task() {
    let mut m = machine_nrf52840_dk();
    m.bus.write_u32(CLOCK, 1).unwrap(); // TASKS_HFCLKSTART
    m.bus.write_u32(CLOCK + 0x304, 1 << 0).unwrap(); // INTENSET.HFCLKSTARTED
    assert!(
        run_until_pend(&mut m, CLOCK_IRQ, BUDGET),
        "{}",
        starved!(
            m,
            "POWER_CLOCK",
            CLOCK_IRQ,
            "arming order task-then-INTENSET"
        )
    );
}

/// Negative direction: no INTENSET, no pend.
#[test]
fn clock_does_not_pend_while_the_interrupt_is_disabled() {
    let mut m = machine_nrf52840_dk();
    m.bus.write_u32(CLOCK, 1).unwrap(); // TASKS_HFCLKSTART, INTEN left at 0
    assert_eq!(
        m.bus.read_u32(CLOCK + 0x100).unwrap(),
        1,
        "precondition: the event latches even with the interrupt disabled"
    );
    for _ in 0..16 {
        m.advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("advance");
    }
    assert!(
        !pended(&m.bus, CLOCK_IRQ),
        "SPURIOUS: POWER_CLOCK pended with INTEN = 0 (ISPR[0]={:#x})",
        ispr(&m.bus, CLOCK_IRQ)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Nrf52Egu — "tick() drains software-triggered EGU events into IRQs"
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn egu0_trigger_pends_its_line() {
    let mut m = machine_nrf52840_dk();
    m.bus.write_u32(EGU0 + 0x304, 1 << 0).unwrap(); // INTENSET ch0
    m.bus.write_u32(EGU0, 1).unwrap(); // TASKS_TRIGGER[0]
    assert_eq!(
        m.bus.read_u32(EGU0 + 0x100).unwrap(),
        1,
        "precondition: EVENTS_TRIGGERED[0] must latch on the task write"
    );
    assert!(
        run_until_pend(&mut m, EGU0_IRQ, BUDGET),
        "{}",
        starved!(m, "SWI0_EGU0", EGU0_IRQ, "software-triggered EGU drain")
    );
}

#[test]
fn egu0_does_not_pend_on_an_unmasked_channel() {
    let mut m = machine_nrf52840_dk();
    m.bus.write_u32(EGU0 + 0x304, 1 << 1).unwrap(); // INTENSET ch1 only
    m.bus.write_u32(EGU0, 1).unwrap(); // TASKS_TRIGGER[0]
    for _ in 0..16 {
        m.advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("advance");
    }
    assert!(
        !pended(&m.bus, EGU0_IRQ),
        "SPURIOUS: SWI0_EGU0 pended for channel 0 with only channel 1 unmasked"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Nrf52Gpiote — "tick() drains pending PORT/IN events into IRQs"
// ─────────────────────────────────────────────────────────────────────────────

/// A GPIO input edge — a CROSS-peripheral wake the MMIO write choke never sees.
///
/// This is the one member of the seven whose arming stimulus is not a write to
/// itself: `observe_gpio_change` is driven from the bus's GPIO edge scan in
/// `tick_peripherals_phase1`. The scan survives walk deletion only because
/// `per_cycle_tick_is_trivial()` is gated on `!self.nordic_gpio_service`, and
/// the freshly-latched chain is armed by the explicit
/// `collect_scheduled_events` sweep that follows the scan.
#[test]
fn gpiote_input_edge_pends_its_line() {
    let mut m = machine_nrf52840_dk();
    // CONFIG[0]: MODE=Event(1), PSEL=11, PORT=0, POLARITY=LoToHi(1).
    let cfg = 1 | (11 << 8) | (1 << 16);
    m.bus.write_u32(GPIOTE + 0x510, cfg).unwrap();
    m.bus.write_u32(GPIOTE + 0x304, 1 << 0).unwrap(); // INTENSET IN[0]

    // Settle the edge detector at the low level before the rising edge.
    for _ in 0..2 {
        m.advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("advance");
    }
    // Drive P0.11 high at GPIO0.IN (0x510) — NOT OUT (0x504).
    m.bus.write_u32(GPIO0 + GPIO_IN, 1 << 11).unwrap();

    assert!(
        run_until_pend(&mut m, GPIOTE_IRQ, BUDGET),
        "{}",
        starved!(
            m,
            "GPIOTE",
            GPIOTE_IRQ,
            "a button/IRQ-driven input edge never reached the CPU."
        )
    );
    assert_eq!(
        m.bus.read_u32(GPIOTE + 0x100).unwrap(),
        1,
        "EVENTS_IN[0] must be latched alongside the pend"
    );
}

/// Wrong-polarity edge must not pend.
#[test]
fn gpiote_does_not_pend_on_the_opposite_edge() {
    let mut m = machine_nrf52840_dk();
    // POLARITY = HiToLo(2) but we drive a RISING edge.
    let cfg = 1 | (11 << 8) | (2 << 16);
    m.bus.write_u32(GPIOTE + 0x510, cfg).unwrap();
    m.bus.write_u32(GPIOTE + 0x304, 1 << 0).unwrap();
    for _ in 0..2 {
        m.advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("advance");
    }
    m.bus.write_u32(GPIO0 + GPIO_IN, 1 << 11).unwrap();
    for _ in 0..16 {
        m.advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("advance");
    }
    assert!(
        !pended(&m.bus, GPIOTE_IRQ),
        "SPURIOUS: GPIOTE pended on a rising edge with POLARITY = HiToLo"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Nrf52Ecb — "tick() latches ENDECB and pends the AES ECB line"
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn ecb_startecb_pends_its_line() {
    let mut m = machine_nrf52840_dk();
    m.bus.write_u32(ECB + 0x504, RAM).unwrap(); // ECBDATAPTR → RAM
    m.bus.write_u32(ECB + 0x304, 1 << 0).unwrap(); // INTENSET.ENDECB
    m.bus.write_u32(ECB, 1).unwrap(); // TASKS_STARTECB
    assert!(
        run_until_pend(&mut m, ECB_IRQ, BUDGET),
        "{}",
        starved!(
            m,
            "ECB",
            ECB_IRQ,
            "the AES-ECB block completion never reached the CPU."
        )
    );
    assert_eq!(
        m.bus.read_u32(ECB + 0x100).unwrap(),
        1,
        "EVENTS_ENDECB must be latched alongside the pend"
    );
}

#[test]
fn ecb_does_not_pend_without_startecb() {
    let mut m = machine_nrf52840_dk();
    m.bus.write_u32(ECB + 0x504, RAM).unwrap();
    m.bus.write_u32(ECB + 0x304, 1 << 0).unwrap(); // armed, but never started
    for _ in 0..16 {
        m.advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("advance");
    }
    assert!(
        !pended(&m.bus, ECB_IRQ),
        "SPURIOUS: ECB pended without TASKS_STARTECB"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Nrf52Twim — "tick() converts latched TWIM events into the instance IRQ"
// ─────────────────────────────────────────────────────────────────────────────

/// `twi1` is the bare `Nrf52Twim` model (chip yaml type `nrf52840_i2c`).
#[test]
fn twim1_transfer_pends_its_line() {
    let mut m = machine_nrf52840_dk();
    m.bus.write_u32(TWI1 + 0x500, 6).unwrap(); // ENABLE = TWIM
    m.bus.write_u32(TWI1 + 0x588, 0x50).unwrap(); // ADDRESS
    m.bus.write_u32(TWI1 + 0x544, RAM).unwrap(); // TXD.PTR
    m.bus.write_u32(TWI1 + 0x548, 1).unwrap(); // TXD.MAXCNT
                                               // STOPPED | ERROR | LASTTX — a completed OR aborted transfer must pend.
    m.bus
        .write_u32(TWI1 + 0x304, (1 << 1) | (1 << 9) | (1 << 24))
        .unwrap();
    m.bus.write_u32(TWI1 + 0x008, 1).unwrap(); // TASKS_STARTTX
    assert!(
        run_until_pend(&mut m, TWI1_IRQ, BUDGET),
        "{}",
        starved!(
            m,
            "TWIM1",
            TWI1_IRQ,
            "an interrupt-driven (non-polling) TWIM transfer never completed."
        )
    );
}

#[test]
fn twim1_does_not_pend_while_inten_is_clear() {
    let mut m = machine_nrf52840_dk();
    m.bus.write_u32(TWI1 + 0x500, 6).unwrap();
    m.bus.write_u32(TWI1 + 0x588, 0x50).unwrap();
    m.bus.write_u32(TWI1 + 0x544, RAM).unwrap();
    m.bus.write_u32(TWI1 + 0x548, 1).unwrap();
    m.bus.write_u32(TWI1 + 0x008, 1).unwrap(); // STARTTX, INTEN = 0
    for _ in 0..16 {
        m.advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("advance");
    }
    assert!(
        !pended(&m.bus, TWI1_IRQ),
        "SPURIOUS: TWIM1 pended with INTEN = 0 (the polling driver configuration)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Nrf52SerialInstance — "tick() delegates to the active TWIM/SPIM sub-model"
// ─────────────────────────────────────────────────────────────────────────────

/// `i2c0` is the SPIM0/TWIM0 mux (chip yaml type `nrf52840_serial`).
///
/// The mux forwards `take_scheduled_events` / `on_event` / `sync_to` to the
/// ENABLE-selected sub-model, so this probe answers the open question in the
/// allowlist note: fixing the sub-models does NOT automatically fix the mux —
/// the mux needs its own forwarding, and this asserts it has it.
#[test]
fn serial_instance_in_twim_mode_pends_its_line() {
    let mut m = machine_nrf52840_dk();
    m.bus.write_u32(SERIAL0 + 0x500, 6).unwrap(); // ENABLE = TWIM
    m.bus.write_u32(SERIAL0 + 0x588, 0x50).unwrap(); // ADDRESS
    m.bus.write_u32(SERIAL0 + 0x544, RAM).unwrap(); // TXD.PTR
    m.bus.write_u32(SERIAL0 + 0x548, 1).unwrap(); // TXD.MAXCNT
    m.bus
        .write_u32(SERIAL0 + 0x304, (1 << 1) | (1 << 9) | (1 << 24))
        .unwrap();
    m.bus.write_u32(SERIAL0 + 0x008, 1).unwrap(); // TASKS_STARTTX
    assert!(
        run_until_pend(&mut m, SERIAL0_IRQ, BUDGET),
        "{}",
        starved!(
            m,
            "SPIM0/TWIM0",
            SERIAL0_IRQ,
            "the serial-instance mux dropped its sub-model's delivery hook."
        )
    );
}

/// With ENABLE = 0 the mux is a pure config surface and must stay silent.
#[test]
fn serial_instance_does_not_pend_while_disabled() {
    let mut m = machine_nrf52840_dk();
    m.bus.write_u32(SERIAL0 + 0x588, 0x50).unwrap();
    m.bus.write_u32(SERIAL0 + 0x544, RAM).unwrap();
    m.bus.write_u32(SERIAL0 + 0x548, 1).unwrap();
    m.bus
        .write_u32(SERIAL0 + 0x304, (1 << 1) | (1 << 9) | (1 << 24))
        .unwrap();
    m.bus.write_u32(SERIAL0 + 0x008, 1).unwrap(); // STARTTX with ENABLE = 0
    for _ in 0..16 {
        m.advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("advance");
    }
    assert!(
        !pended(&m.bus, SERIAL0_IRQ),
        "SPURIOUS: the serial-instance mux pended with ENABLE = 0"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Nrf52Radio — "tick() advances the TX/RX cycle countdown and fires ADDRESS/END"
// ─────────────────────────────────────────────────────────────────────────────

/// RADIO END must pend at the BIT-RATE cycle, not merely eventually.
///
/// This is the only member of the seven that is time-driven rather than
/// write-driven, so "it fires" is not the interesting claim — "it fires when
/// the air time says" is. `Nrf52Radio::cycles_for_packet` is the model's own
/// air-time law (1 sim cycle ≈ 1 µs): `(total_bytes.max(1)) * cycles_per_byte`,
/// with `total_bytes = LENGTH + 3` for the CRC and `cycles_per_byte = 8` at
/// `MODE = Ble_1Mbit`. The deadline is derived HERE from LENGTH and MODE
/// independently of the model's own countdown bookkeeping, so a countdown that
/// collapses to "next cycle" (which is what a scheduler hand-off gets wrong)
/// fails this even though it still pends.
#[test]
fn radio_end_pends_at_the_bitrate_deadline() {
    const LENGTH: u32 = 16;
    const MODE_BLE_1MBIT: u32 = 3;
    const CYCLES_PER_BYTE: u64 = 8;
    // The model's air-time law, restated from the PS/MODE table rather than
    // read out of the model.
    let air_cycles = (LENGTH as u64 + 3) * CYCLES_PER_BYTE;

    let mut m = machine_nrf52840_dk();
    // A BLE-shaped packet in RAM: S0 = 0, LENGTH = 16, then payload.
    m.bus.write_u32(RAM as u64, LENGTH << 8).unwrap();
    for i in 0..8u64 {
        m.bus
            .write_u32(RAM as u64 + 4 + i * 4, 0xA5A5_A5A5)
            .unwrap();
    }
    m.bus.write_u32(RADIO + 0x510, MODE_BLE_1MBIT).unwrap(); // MODE
    m.bus.write_u32(RADIO + 0x504, RAM).unwrap(); // PACKETPTR
                                                  // PCNF0: LFLEN = 8 bits at [3:0], S0LEN = 1 at [8].
    m.bus.write_u32(RADIO + 0x514, 8 | (1 << 8)).unwrap();
    m.bus.write_u32(RADIO + 0x518, 0xFF).unwrap(); // PCNF1.MAXLEN
    m.bus.write_u32(RADIO + 0x304, 1 << 3).unwrap(); // INTENSET.END

    // TXEN → READY, then START → air time → END.
    m.bus.write_u32(RADIO, 1).unwrap(); // TASKS_TXEN
    let mut ready_cycles = 0;
    while m.bus.read_u32(RADIO + 0x100).unwrap() == 0 && ready_cycles < 64 {
        m.advance(AdvanceRequest::run(Some(1)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("advance");
        ready_cycles += 1;
    }
    assert_eq!(
        m.bus.read_u32(RADIO + 0x100).unwrap(),
        1,
        "precondition: EVENTS_READY must fire after TASKS_TXEN"
    );

    let start_cycle = m.total_cycles;
    m.bus.write_u32(RADIO + 0x008, 1).unwrap(); // TASKS_START

    let end_cycle =
        cycles_until_pend(&mut m, RADIO_IRQ, start_cycle + BUDGET).unwrap_or_else(|| {
            panic!(
                "STARVED: RADIO END ({RADIO_IRQ}) never pended within {BUDGET} cycles \
             of TASKS_START, ISPR[0]={:#x}, EVENTS_END={:#x}, STATE={:#x}. \
             A BLE stack blocks on this forever.",
                ispr(&m.bus, RADIO_IRQ),
                m.bus.read_u32(RADIO + 0x10C).unwrap_or(0),
                m.bus.read_u32(RADIO + 0x550).unwrap_or(0),
            )
        });

    let elapsed = end_cycle - start_cycle;
    println!(
        "radio: LENGTH={LENGTH} MODE={MODE_BLE_1MBIT} air_cycles={air_cycles} \
         start={start_cycle} end={end_cycle} elapsed={elapsed}"
    );
    assert_eq!(
        elapsed, air_cycles,
        "WRONG CYCLE: RADIO END pended {elapsed} cycles after TASKS_START, but a \
         {LENGTH}-byte MODE={MODE_BLE_1MBIT} packet is {air_cycles} cycles of air \
         time ((LENGTH + 3 CRC) x {CYCLES_PER_BYTE} cycles/byte). An END at the \
         wrong cycle is worse than one that never fires, because a BLE timing \
         test passes on it."
    );
}

/// A LONGER packet must take proportionally longer — the air-time law is a
/// function of LENGTH, not a constant that happens to match one case.
///
/// Without this, `radio_end_pends_at_the_bitrate_deadline` is satisfiable by
/// any fixed delay that coincides with 152 cycles.
#[test]
fn radio_air_time_scales_with_packet_length() {
    fn end_delay(length: u32) -> u64 {
        const MODE_BLE_1MBIT: u32 = 3;
        let mut m = machine_nrf52840_dk();
        m.bus.write_u32(RAM as u64, length << 8).unwrap();
        for i in 0..16u64 {
            m.bus
                .write_u32(RAM as u64 + 4 + i * 4, 0xA5A5_A5A5)
                .unwrap();
        }
        m.bus.write_u32(RADIO + 0x510, MODE_BLE_1MBIT).unwrap();
        m.bus.write_u32(RADIO + 0x504, RAM).unwrap();
        m.bus.write_u32(RADIO + 0x514, 8 | (1 << 8)).unwrap();
        m.bus.write_u32(RADIO + 0x518, 0xFF).unwrap();
        m.bus.write_u32(RADIO + 0x304, 1 << 3).unwrap();
        m.bus.write_u32(RADIO, 1).unwrap();
        for _ in 0..64 {
            if m.bus.read_u32(RADIO + 0x100).unwrap() != 0 {
                break;
            }
            m.advance(AdvanceRequest::run(Some(1)).with_breakpoints(BreakpointPolicy::Ignore))
                .expect("advance");
        }
        let start = m.total_cycles;
        m.bus.write_u32(RADIO + 0x008, 1).unwrap();
        let end = cycles_until_pend(&mut m, RADIO_IRQ, start + BUDGET)
            .unwrap_or_else(|| panic!("STARVED: RADIO END never pended for LENGTH={length}"));
        end - start
    }

    let short = end_delay(8);
    let long = end_delay(32);
    println!("radio air time: LENGTH=8 -> {short} cycles, LENGTH=32 -> {long} cycles");
    assert_eq!(short, (8 + 3) * 8, "LENGTH=8 air time");
    assert_eq!(long, (32 + 3) * 8, "LENGTH=32 air time");
    assert!(
        long > short,
        "CONSTANT DELAY: air time did not scale with LENGTH ({short} vs {long}) — \
         the countdown collapsed to a fixed hand-off delay"
    );
}

/// Negative direction: END must not pend with INTENSET.END clear.
#[test]
fn radio_does_not_pend_end_while_masked() {
    const MODE_BLE_1MBIT: u32 = 3;
    let mut m = machine_nrf52840_dk();
    m.bus.write_u32(RAM as u64, 16 << 8).unwrap();
    m.bus.write_u32(RADIO + 0x510, MODE_BLE_1MBIT).unwrap();
    m.bus.write_u32(RADIO + 0x504, RAM).unwrap();
    m.bus.write_u32(RADIO + 0x514, 8 | (1 << 8)).unwrap();
    m.bus.write_u32(RADIO + 0x518, 0xFF).unwrap();
    // INTEN deliberately left at 0.
    m.bus.write_u32(RADIO, 1).unwrap();
    for _ in 0..64 {
        m.advance(AdvanceRequest::run(Some(1)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("advance");
    }
    m.bus.write_u32(RADIO + 0x008, 1).unwrap();
    for _ in 0..8 {
        m.advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("advance");
    }
    assert_ne!(
        m.bus.read_u32(RADIO + 0x10C).unwrap(),
        0,
        "precondition: EVENTS_END must still latch while the interrupt is masked"
    );
    assert!(
        !pended(&m.bus, RADIO_IRQ),
        "SPURIOUS: RADIO pended with INTEN = 0 (ISPR[0]={:#x})",
        ispr(&m.bus, RADIO_IRQ)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Controls
// ─────────────────────────────────────────────────────────────────────────────

/// The nRF52840-DK bus must STILL derive walk deletion.
///
/// The forbidden repair — `needs_legacy_walk() -> true` — would put the whole
/// nRF52 bus back on the per-cycle walk and drop `max_safe_tick_interval` from
/// 512 to 1 for every nRF52 lab. This makes that repair fail rather than ship
/// as a silent slowdown. Twin of `rp2040_pico_walk_stays_deleted_and_tick_512`.
#[test]
fn nrf52840_dk_walk_stays_deleted_and_tick_512() {
    let bus = bus_nrf52840_dk();
    let forcers: Vec<&str> = bus
        .peripherals
        .iter()
        .filter(|p| !p.dev.uses_scheduler() && p.dev.needs_legacy_walk())
        .map(|p| p.name.as_str())
        .collect();
    assert!(
        bus.legacy_walk_disabled,
        "nrf52840-dk must keep auto-deriving walk deletion; forcers: {forcers:?}"
    );
    assert_eq!(
        bus.max_safe_tick_interval(),
        RECOMMENDED_TICK_INTERVAL,
        "nrf52840-dk must keep max_safe_tick_interval = {RECOMMENDED_TICK_INTERVAL}"
    );
}

/// Every one of the seven must keep `uses_scheduler() == true`.
///
/// This is the load-bearing half of the repair. Deleting the dead
/// `needs_legacy_walk() -> false` override is only safe BECAUSE
/// `uses_scheduler()` already short-circuits `derive_walk_deletable`'s OR. If a
/// later edit drops `uses_scheduler()` from any of these, the default
/// `needs_legacy_walk() -> true` turns that model into a walk forcer and the
/// whole nRF52 bus loses its batching — silently, since nothing else observes
/// it. Named per model so the failure says which one.
#[test]
fn the_seven_stay_scheduler_driven() {
    let bus = bus_nrf52840_dk();
    for name in [
        "clock", "ecb", "egu0", "egu1", "egu2", "egu3", "egu4", "egu5", "gpiote", "radio", "i2c0",
        "twi1",
    ] {
        let p = bus
            .peripherals
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("nrf52840-dk must instantiate `{name}`"));
        assert!(
            p.dev.uses_scheduler(),
            "`{name}` dropped uses_scheduler(); with the dead \
             needs_legacy_walk() override removed it is now a WALK FORCER and \
             the whole nRF52 bus falls back to tick interval 1"
        );
    }
}

/// The delivery probes above must be capable of failing.
///
/// Every probe passes on unmodified `main` — the seven models were already
/// migrated — so a reader has no evidence the probes measure anything. This
/// severs the arming hook the way a regression would and shows the
/// consequence at the model level: with `take_scheduled_events` returning
/// nothing, no chain arms, and the `on_event` that carries `raise_own_irq`
/// never runs. Each model is checked in the exact state its probe above puts it
/// in, so a model whose probe is vacuous shows up HERE as "armed nothing to
/// begin with".
#[test]
fn the_delivery_probes_go_red_when_the_event_chain_is_cut() {
    use labwired_core::peripherals::nrf52;

    // CLOCK, armed exactly as `clock_hfclkstarted_pends_power_clock` arms it.
    let mut clock = nrf52::clock::Nrf52Clock::new();
    Peripheral::write_u32(&mut clock, 0x304, 1).unwrap();
    Peripheral::write_u32(&mut clock, 0x000, 1).unwrap();
    assert!(
        !Peripheral::take_scheduled_events(&mut clock).is_empty(),
        "Nrf52Clock armed no event in the state its delivery probe uses — that \
         probe is vacuous"
    );

    // EGU0.
    let mut egu = nrf52::egu::Nrf52Egu::new();
    Peripheral::write_u32(&mut egu, 0x304, 1).unwrap();
    Peripheral::write_u32(&mut egu, 0x000, 1).unwrap();
    assert!(
        !Peripheral::take_scheduled_events(&mut egu).is_empty(),
        "Nrf52Egu armed no event — its delivery probe is vacuous"
    );

    // GPIOTE, woken by an input edge rather than a write.
    let mut gpiote = nrf52::gpiote::Nrf52Gpiote::new();
    Peripheral::write_u32(&mut gpiote, 0x510, 1 | (11 << 8) | (1 << 16)).unwrap();
    Peripheral::write_u32(&mut gpiote, 0x304, 1).unwrap();
    Peripheral::observe_gpio_change(&mut gpiote, &[(0, 11, 1)]);
    assert!(
        !Peripheral::take_scheduled_events(&mut gpiote).is_empty(),
        "Nrf52Gpiote armed no event after an input edge — its delivery probe is \
         vacuous, and a real GPIOTE input IRQ would be lost"
    );

    // ECB.
    let mut ecb = nrf52::ecb::Nrf52Ecb::new();
    Peripheral::write_u32(&mut ecb, 0x504, RAM).unwrap();
    Peripheral::write_u32(&mut ecb, 0x304, 1).unwrap();
    Peripheral::write_u32(&mut ecb, 0x000, 1).unwrap();
    assert!(
        !Peripheral::take_scheduled_events(&mut ecb).is_empty(),
        "Nrf52Ecb armed no event — its delivery probe is vacuous"
    );

    // TWIM.
    let mut twim = nrf52::twim::Nrf52Twim::new();
    Peripheral::write_u32(&mut twim, 0x500, 6).unwrap();
    Peripheral::write_u32(&mut twim, 0x588, 0x50).unwrap();
    Peripheral::write_u32(&mut twim, 0x544, RAM).unwrap();
    Peripheral::write_u32(&mut twim, 0x548, 1).unwrap();
    Peripheral::write_u32(&mut twim, 0x304, (1 << 1) | (1 << 9) | (1 << 24)).unwrap();
    Peripheral::write_u32(&mut twim, 0x008, 1).unwrap();
    assert!(
        !Peripheral::take_scheduled_events(&mut twim).is_empty(),
        "Nrf52Twim armed no event — its delivery probe is vacuous"
    );

    // The serial-instance mux must FORWARD the same arm; a mux that dropped the
    // hook would be silent here while the bare TWIM above still armed.
    let mut inst = nrf52::serial_instance::Nrf52SerialInstance::new();
    Peripheral::write_u32(&mut inst, 0x500, 6).unwrap();
    Peripheral::write_u32(&mut inst, 0x588, 0x50).unwrap();
    Peripheral::write_u32(&mut inst, 0x544, RAM).unwrap();
    Peripheral::write_u32(&mut inst, 0x548, 1).unwrap();
    Peripheral::write_u32(&mut inst, 0x304, (1 << 1) | (1 << 9) | (1 << 24)).unwrap();
    Peripheral::write_u32(&mut inst, 0x008, 1).unwrap();
    assert!(
        !Peripheral::take_scheduled_events(&mut inst).is_empty(),
        "Nrf52SerialInstance did not forward take_scheduled_events to its active \
         TWIM sub-model — the mux drops delivery even when the sub-model is correct"
    );

    // RADIO.
    let mut radio = nrf52::radio::Nrf52Radio::new();
    Peripheral::write_u32(&mut radio, 0x510, 3).unwrap();
    Peripheral::write_u32(&mut radio, 0x504, RAM).unwrap();
    Peripheral::write_u32(&mut radio, 0x304, 1 << 3).unwrap();
    Peripheral::write_u32(&mut radio, 0x000, 1).unwrap(); // TASKS_TXEN
    assert!(
        !Peripheral::take_scheduled_events(&mut radio).is_empty(),
        "Nrf52Radio armed no event — its delivery probe is vacuous"
    );

    // And the negative direction of the same instrument: an UNARMED model must
    // schedule nothing, or "armed something" above proves nothing.
    for (name, mut inert) in [
        (
            "Nrf52Clock",
            Box::new(nrf52::clock::Nrf52Clock::new()) as Box<dyn Peripheral>,
        ),
        ("Nrf52Egu", Box::new(nrf52::egu::Nrf52Egu::new())),
        ("Nrf52Gpiote", Box::new(nrf52::gpiote::Nrf52Gpiote::new())),
        ("Nrf52Ecb", Box::new(nrf52::ecb::Nrf52Ecb::new())),
        ("Nrf52Twim", Box::new(nrf52::twim::Nrf52Twim::new())),
        (
            "Nrf52SerialInstance",
            Box::new(nrf52::serial_instance::Nrf52SerialInstance::new()),
        ),
        ("Nrf52Radio", Box::new(nrf52::radio::Nrf52Radio::new())),
    ] {
        assert!(
            inert.take_scheduled_events().is_empty(),
            "{name} armed a scheduler event at reset — a per-cycle wakeup on every \
             idle nRF52 lab"
        );
    }
}
