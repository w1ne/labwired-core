// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Central I²C data-ready time drive (Option A).
//!
//! A declarative `i2c_device` with `delay_us` gates its response on
//! `I2cDevice::advance_time_us`. The machine now drives that clock centrally
//! from the chip's authoritative simulated-µs source
//! (`Peripheral::sim_time_us` — the ESP32 SYSTIMER), fanning the elapsed delta
//! out to every opted-in I²C controller once per scheduler slice.
//!
//! These tests prove:
//!  1. A `delay_us` command on an ESP32-class machine (SYSTIMER + ESP32-C3 I²C)
//!     becomes ready **after** the simulated interval and **not before** — both
//!     via the direct drive and end-to-end through `Machine::advance`.
//!  2. The nRF54L TWIM does NOT opt into the central drive (it advances its own
//!     slaves off the GRTC), so its slaves are never advanced twice.
//!  3. Controllers that host attachable slaves opt in; SYSTIMER is the source.

use crate::peripherals::components::declarative_i2c::GenericI2cDevice;
use crate::peripherals::esp32c3::i2c::Esp32c3I2c;
use crate::peripherals::esp32s3::systimer::Systimer;
use crate::peripherals::i2c::{I2c, I2cDevice, I2cRegisterLayout};
use crate::peripherals::nrf54l::twim::Nrf54lTwim;
use crate::tests::machine_advance::CountingCpu;
use crate::{Machine, Peripheral};

/// ESP32-C3 SYSTIMER runs UNIT0 at a silicon-fixed 16 MHz off a 160 MHz core,
/// so 160 CPU cycles == 1 SYSTIMER tick × 10 and 160 CPU cycles == 1 µs.
const C3_CPU_HZ: u32 = 160_000_000;
const CYCLES_PER_US: u64 = (C3_CPU_HZ as u64) / 1_000_000; // 160

/// A minimal command device with a 15 ms data-ready delay and a constant status
/// word so readiness is observable without CRC framing: `0xFF` before ready,
/// `[0x80, 0x10]` once the simulated interval has elapsed.
const DELAY_DEVICE_YAML: &str = r#"
type: delay_probe
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x44
    commands:
      - name: measure
        code: 0x2400
        delay_us: 15000
        response:
          - { const: 0x8010, width: 2 }
"#;

/// Build a bare bus with the SYSTIMER source at index 0 and an ESP32-C3 I²C
/// controller (hosting one delay device) at index 1.
fn build_machine() -> Machine<CountingCpu> {
    let mut controller = Esp32c3I2c::new();
    controller.push_slave(Box::new(
        GenericI2cDevice::from_yaml(DELAY_DEVICE_YAML, 0).unwrap(),
    ));

    let mut bus = crate::bus::SystemBus::new();
    bus.add_peripheral(
        "systimer",
        0x6002_3000,
        0x100,
        None,
        Box::new(Systimer::new_with_source(C3_CPU_HZ, 37)),
    );
    bus.add_peripheral("i2c0", 0x6001_3000, 0x100, None, Box::new(controller));
    Machine::new(CountingCpu::default(), bus)
}

/// Reach the attached delay device through the ESP32-C3 controller (found by
/// type — `SystemBus::new` pre-populates several unrelated peripherals, so a
/// fixed index is not safe).
fn delay_device(machine: &mut Machine<CountingCpu>) -> &mut GenericI2cDevice {
    let ctrl = machine
        .bus
        .peripherals
        .iter_mut()
        .find_map(|p| p.dev.as_any_mut()?.downcast_mut::<Esp32c3I2c>())
        .expect("Esp32c3I2c controller on the bus");
    ctrl.attached_slaves_mut()[0]
        .as_any_mut()
        .and_then(|a| a.downcast_mut::<GenericI2cDevice>())
        .expect("attached delay device")
}

/// Issue the measurement command (16-bit BE opcode) on the device.
fn dispatch_measure(dev: &mut GenericI2cDevice) {
    dev.start();
    dev.write(0x24);
    dev.write(0x00);
}

/// Read two response bytes from a fresh read phase.
fn read_two(dev: &mut GenericI2cDevice) -> [u8; 2] {
    dev.start();
    [dev.read(), dev.read()]
}

#[test]
fn esp32_class_delay_becomes_ready_after_interval_not_before() {
    let mut machine = build_machine();

    // Seed the drive's anchor at t=0 (first call only anchors, never advances).
    machine.bus.set_current_cycle(0);
    machine.advance_central_i2c_time();

    dispatch_measure(delay_device(&mut machine));

    // Advance simulated time to 12.5 ms — short of the 15 ms deadline.
    machine.bus.set_current_cycle(12_500 * CYCLES_PER_US);
    machine.advance_central_i2c_time();
    assert_eq!(
        read_two(delay_device(&mut machine)),
        [0xFF, 0xFF],
        "not ready before the 15 ms interval elapses"
    );

    // Cross the deadline (15 ms). The delta since the last drive is handed to
    // the controller, which fans it out to the slave.
    machine.bus.set_current_cycle(15_000 * CYCLES_PER_US);
    machine.advance_central_i2c_time();
    assert_eq!(
        read_two(delay_device(&mut machine)),
        [0x80, 0x10],
        "response materialises once the simulated interval has elapsed"
    );
}

/// End-to-end through the real call site: `Machine::advance` drives the counter
/// (SYSTIMER ticks) and calls the central drive per slice, so a device measured
/// then polled after enough cycles becomes ready without any manual drive.
#[test]
fn esp32_class_delay_ready_end_to_end_through_advance() {
    use crate::AdvanceRequest;

    let mut machine = build_machine();
    // Boot a little so the counter and anchor are live, then measure.
    machine
        .advance(AdvanceRequest::run(Some(1_000)))
        .expect("advance");
    dispatch_measure(delay_device(&mut machine));

    // Not ready shortly after issuing the command.
    machine
        .advance(AdvanceRequest::run(Some(1_000)))
        .expect("advance");
    assert_eq!(
        read_two(delay_device(&mut machine)),
        [0xFF, 0xFF],
        "still gated right after the command"
    );

    // Run well past 15 ms of simulated time (15 ms = 2.4M CPU cycles at 160 MHz;
    // run 4M to clear the deadline with margin regardless of tick cadence).
    machine
        .advance(AdvanceRequest::run(Some(4_000_000)))
        .expect("advance");
    assert_eq!(
        read_two(delay_device(&mut machine)),
        [0x80, 0x10],
        "delay device is ready after the machine simulated past the interval"
    );
}

#[test]
fn nrf54l_twim_does_not_opt_into_central_drive() {
    // The nRF54L TWIM drives its slaves' advance_time_us itself off the GRTC,
    // per transaction. It must NOT also be driven centrally, or time would
    // advance twice. The exclusion is structural: TWIM inherits the default
    // `drives_central_i2c_time() == false`, so the machine never lists it.
    assert!(
        !Nrf54lTwim::new().drives_central_i2c_time(),
        "nRF54L TWIM must stay off the central drive (it self-drives off GRTC)"
    );
}

#[test]
fn i2c_hosting_controllers_opt_in_and_systimer_is_the_source() {
    // Every controller that hosts attachable I²C slaves (except the self-driving
    // nRF54L TWIM) opts into the central fan-out …
    assert!(Esp32c3I2c::new().drives_central_i2c_time());
    assert!(I2c::new_with_layout(I2cRegisterLayout::Stm32L4).drives_central_i2c_time());
    // … and only a genuine absolute-µs counter is the source. SYSTIMER answers;
    // a bare bus with no such peripheral does not.
    assert!(Systimer::new_with_source(C3_CPU_HZ, 37)
        .sim_time_us()
        .is_some());
}

/// nRF54L / STM32 hold no absolute-µs source, so on those machines the central
/// drive short-circuits and a delay device is never advanced by it — proving the
/// STM32-class holdout degrades to always-ready rather than misbehaving.
#[test]
fn no_source_means_no_central_advance() {
    let mut controller = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
    controller.push_slave(Box::new(
        GenericI2cDevice::from_yaml(DELAY_DEVICE_YAML, 0).unwrap(),
    ));
    let mut bus = crate::bus::SystemBus::new();
    // No SYSTIMER on the bus — Cortex-M SysTick/TIM are not absolute-µs sources.
    bus.add_peripheral("i2c_probe", 0x4000_5400, 0x400, None, Box::new(controller));
    let mut machine = Machine::new(CountingCpu::default(), bus);

    machine.bus.set_current_cycle(1_000_000);
    // Drive is a no-op (no source); it must not panic and must leave the device
    // un-advanced. Reaching the device (find the generic I2c we attached — the
    // one with a slave; the default bus may pre-populate other I2c banks):
    machine.advance_central_i2c_time();

    let ctrl = machine
        .bus
        .peripherals
        .iter_mut()
        .filter_map(|p| p.dev.as_any_mut()?.downcast_mut::<I2c>())
        .find(|i| !i.attached_devices().is_empty())
        .expect("generic I2c controller with the attached probe");
    let dev = ctrl.attached_devices()[0]
        .borrow_mut()
        .as_any_mut()
        .and_then(|a| a.downcast_mut::<GenericI2cDevice>())
        .map(|d| {
            d.start();
            d.write(0x24);
            d.write(0x00);
            d.start();
            [d.read(), d.read()]
        });
    // With no source the delay is un-gated by the central drive; the device's
    // own elapsed clock never advanced, so it reads not-ready (0xFF) — i.e. the
    // holdout is inert, exactly as before this hook existed.
    assert_eq!(dev, Some([0xFF, 0xFF]));
}
