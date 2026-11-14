// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Observers see **every** committed store, at every access width, and an
//! observer registered on the `Machine` reaches the bus.
//!
//! Two gaps used to make a `--vcd` trace record `pc` and nothing else, and
//! `--trace` carry no memory writes at all:
//!
//! 1. `write_u16` / `write_u32` short-circuit into the peripheral before
//!    reaching the `write_u8` fallback that notifies, so every wide MMIO
//!    store (GPIO BSRR, USART TDR) was invisible to observers.
//! 2. `Machine` and `SystemBus` keep separate observer lists and the CLI
//!    registered only on the `Machine`, so even RAM-path writes never
//!    arrived.
//!
//! The width case is the one that bites hardest: a fix that hand-writes the
//! byte count at the call site can emit word-shaped events for a half-word
//! store, which on this workspace (`overflow-checks = true` in **both**
//! profiles, `panic = "abort"`) aborts the simulator on the first 16-bit
//! peripheral store rather than merely mis-tracing. `no_phantom_bytes`
//! below is what holds that shut.

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::timer::Timer;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Machine, SimulationObserver};
use std::sync::{Arc, Mutex};

const TIM_BASE: u64 = 0x4000_0000;
/// TIMx_ARR — an ordinary read-write register, no side effects on access.
const TIM_ARR: u64 = TIM_BASE + 0x2C;
const RAM_ADDR: u64 = 0x2000_0100;

#[derive(Debug, Default)]
struct Recorder {
    writes: Mutex<Vec<(u64, u8)>>,
}

impl Recorder {
    fn taken(&self) -> Vec<(u64, u8)> {
        std::mem::take(&mut *self.writes.lock().unwrap())
    }
}

impl SimulationObserver for Recorder {
    fn on_memory_write(&self, addr: u64, _old: u8, new: u8) {
        self.writes.lock().unwrap().push((addr, new));
    }
}

fn bus_with_timer() -> SystemBus {
    let mut bus = SystemBus::new();
    bus.add_peripheral("tim2", TIM_BASE, 0x400, Some(28), Box::new(Timer::new()));
    bus
}

/// A half-word MMIO store publishes exactly TWO byte events, little-endian.
///
/// The regression guarded here is not cosmetic: deriving the count from
/// anything but the stored type shifts a `u16` by 16 and 24, which this
/// workspace turns into an abort on a perfectly ordinary firmware store.
#[test]
fn u16_peripheral_store_publishes_two_le_bytes_and_no_phantom_bytes() {
    let mut bus = bus_with_timer();
    let rec = Arc::new(Recorder::default());
    bus.add_observer(rec.clone());

    bus.write_u16(TIM_ARR, 0xBEEF).unwrap();

    assert_eq!(
        rec.taken(),
        vec![(TIM_ARR, 0xEF), (TIM_ARR + 1, 0xBE)],
        "a 16-bit store must publish two bytes, LE, and nothing at +2/+3"
    );
}

/// A word MMIO store publishes exactly FOUR byte events, little-endian.
#[test]
fn u32_peripheral_store_publishes_four_le_bytes() {
    let mut bus = bus_with_timer();
    let rec = Arc::new(Recorder::default());
    bus.add_observer(rec.clone());

    bus.write_u32(TIM_ARR, 0xDEAD_BEEF).unwrap();

    assert_eq!(
        rec.taken(),
        vec![
            (TIM_ARR, 0xEF),
            (TIM_ARR + 1, 0xBE),
            (TIM_ARR + 2, 0xAD),
            (TIM_ARR + 3, 0xDE),
        ],
    );
}

/// The byte stream is the same shape whatever width the firmware used —
/// that is the property a trace consumer actually depends on.
#[test]
fn wide_and_narrow_stores_agree_byte_for_byte() {
    let mut wide = bus_with_timer();
    let wide_rec = Arc::new(Recorder::default());
    wide.add_observer(wide_rec.clone());
    wide.write_u32(TIM_ARR, 0x1234_5678).unwrap();

    let mut narrow = bus_with_timer();
    let narrow_rec = Arc::new(Recorder::default());
    narrow.add_observer(narrow_rec.clone());
    for (i, byte) in 0x1234_5678u32.to_le_bytes().iter().enumerate() {
        narrow.write_u8(TIM_ARR + i as u64, *byte).unwrap();
    }

    assert_eq!(wide_rec.taken(), narrow_rec.taken());
}

/// An unmapped store commits nothing, so it publishes nothing.
#[test]
fn rejected_store_publishes_nothing() {
    let mut bus = bus_with_timer();
    let rec = Arc::new(Recorder::default());
    bus.add_observer(rec.clone());

    assert!(bus.write_u32(0x9999_0000, 0xDEAD_BEEF).is_err());

    assert!(
        rec.taken().is_empty(),
        "a violated store must not be traced"
    );
}

/// `Machine::add_observer` reaches the bus: the registration choke point is
/// what makes an observer whole, and it must work without a run having to
/// start first.
#[test]
fn machine_registered_observer_sees_bus_stores() {
    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    bus.add_peripheral("tim2", TIM_BASE, 0x400, Some(28), Box::new(Timer::new()));
    let mut machine = Machine::new(cpu, bus);

    let rec = Arc::new(Recorder::default());
    machine.add_observer(rec.clone());

    machine.bus.write_u32(TIM_ARR, 0x0000_0001).unwrap();
    machine.bus.write_u8(RAM_ADDR, 0x5A).unwrap();

    let seen = rec.taken();
    assert!(
        seen.contains(&(TIM_ARR, 0x01)),
        "machine-registered observer missed an MMIO store: {seen:?}"
    );
    assert!(
        seen.contains(&(RAM_ADDR, 0x5A)),
        "machine-registered observer missed a RAM store: {seen:?}"
    );
}

/// Registering on the `Machine` must not evict an observer already on the
/// bus. The previous shape mirrored the lists by comparing their LENGTHS and
/// assigning over the bus list, which silently dropped a bus-only observer
/// (the DAP memory tracker is exactly that).
#[test]
fn machine_registration_does_not_evict_a_bus_only_observer() {
    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    bus.add_peripheral("tim2", TIM_BASE, 0x400, Some(28), Box::new(Timer::new()));

    let bus_only = Arc::new(Recorder::default());
    bus.add_observer(bus_only.clone());

    let mut machine = Machine::new(cpu, bus);
    let via_machine = Arc::new(Recorder::default());
    machine.add_observer(via_machine.clone());

    machine.bus.write_u32(TIM_ARR, 0x0000_0002).unwrap();

    assert!(
        bus_only.taken().contains(&(TIM_ARR, 0x02)),
        "a bus-only observer was evicted by machine registration"
    );
    assert!(via_machine.taken().contains(&(TIM_ARR, 0x02)));
}

/// Each emitter publishes its own event kinds, so an observer sitting in
/// both lists sees a store exactly once — no duplicates.
#[test]
fn observer_in_both_lists_sees_each_store_once() {
    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    bus.add_peripheral("tim2", TIM_BASE, 0x400, Some(28), Box::new(Timer::new()));
    let mut machine = Machine::new(cpu, bus);

    let rec = Arc::new(Recorder::default());
    machine.add_observer(rec.clone());

    machine.bus.write_u16(TIM_ARR, 0x00FF).unwrap();

    assert_eq!(rec.taken(), vec![(TIM_ARR, 0xFF), (TIM_ARR + 1, 0x00)]);
}
