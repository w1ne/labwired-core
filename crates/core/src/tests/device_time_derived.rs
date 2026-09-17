// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **Device time on every chip** — the derived-clock arm of the central device
//! time drive.
//!
//! `i2c_central_time_drive.rs` proves the ESP32 arm: a peripheral that models a
//! genuine absolute-µs counter (the SYSTIMER) is the source, and the machine
//! fans its deltas out to every opted-in controller. That file also pinned the
//! hole this one closes — its `no_source_means_no_central_advance` recorded that
//! a chip WITHOUT such a counter left every timed declarative device
//! "effectively always-ready".
//!
//! Always-ready is a silent thunk. `data_ready` and `delay_us` exist so a
//! firmware that skips the datasheet's conversion poll reads a result that is
//! not ready — the failure it would hit on silicon. On STM32, nRF52, RP2040 and
//! SAMD that check could not fail, because nothing ever moved the device's
//! clock. The machine now derives the clock from the executed cycle count and
//! the system's declared `cpu_hz`, and says so in the fidelity census.
//!
//! What each test here holds down:
//!  * `stm32l476_*` — the NEGATIVE CONTROL. A real `configs/systems` manifest, a
//!    real shipping descriptor, and a ready bit that must be CLEAR before the
//!    datasheet conversion time and SET after. This assertion fails on
//!    `origin/main`: with `i2c_time_source_index == None` the drive returned
//!    before touching anything, the device's `elapsed_us` stayed 0 forever, and
//!    `prox_data_rdy` never set — the "ready after" arm of the test is the one
//!    that goes red.
//!  * `spi_*` — the SPI fan-out (A1.3): a counting `SpiDevice` stub attached to
//!    an SPI controller must be handed totals equal to the elapsed µs.
//!  * `fidelity_*` — the approximation note: exactly once, and never on a chip
//!    whose time is measured rather than derived.

use crate::peripherals::components::declarative_i2c::GenericI2cDevice;
use crate::peripherals::esp32c3::i2c::Esp32c3I2c;
use crate::peripherals::esp32s3::systimer::Systimer;
use crate::peripherals::i2c::I2cDevice;
use crate::peripherals::spi::{Spi, SpiDevice, SpiRegisterLayout};
use crate::tests::machine_advance::CountingCpu;
use crate::{AdvanceRequest, Machine};
use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Build a machine from a REAL shipped system manifest, with one extra
/// declarative I²C device bolted onto the named controller.
///
/// The manifest is read from disk rather than written inline on purpose: the
/// chip reference, the peripheral map and — the part that matters here — the
/// board's `cpu_hz` are the ones the product ships, so a change to any of them
/// moves this test rather than sliding past it.
fn machine_from_system(
    system_rel: &str,
    device_type: &str,
    connection: &str,
) -> Machine<CountingCpu> {
    let system_path = repo_root().join(system_rel);
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load {system_rel}: {e}"));
    let chip_path = system_path
        .parent()
        .expect("system dir")
        .join(&manifest.chip);
    manifest.external_devices.push(ExternalDevice {
        id: format!("{device_type}_under_test"),
        r#type: device_type.to_string(),
        connection: connection.to_string(),
        channel: None,
        route: Default::default(),
        config: Default::default(),
    });
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let bus = crate::bus::SystemBus::from_config(&chip, &manifest)
        .unwrap_or_else(|e| panic!("build bus for {system_rel}: {e}"));
    Machine::new(CountingCpu::default(), bus)
}

/// Reach the one attached declarative device on a generic `I2c` controller.
fn attached_generic_device(machine: &mut Machine<CountingCpu>) -> &mut GenericI2cDevice {
    let ctrl = machine
        .bus
        .peripherals
        .iter_mut()
        .filter_map(|p| {
            p.dev
                .as_any_mut()?
                .downcast_mut::<crate::peripherals::i2c::I2c>()
        })
        .find(|i| !i.attached_devices().is_empty())
        .expect("a generic I2c controller hosting the device under test");
    // The bus trace-wraps attached slaves; `as_any_mut` is forwarded through
    // the wrapper, so the downcast still lands on the real model.
    let cell = &ctrl.attached_devices()[0];
    let ptr: *mut GenericI2cDevice = cell
        .borrow_mut()
        .as_any_mut()
        .and_then(|a| a.downcast_mut::<GenericI2cDevice>())
        .expect("attached declarative device") as *mut _;
    // SAFETY: the RefCell borrow ends with the statement above, but the box it
    // points at is owned by `ctrl` and is not moved or dropped while the
    // returned reference lives — `ctrl`'s borrow of `machine` outlives it.
    unsafe { &mut *ptr }
}

// ─── A1 negative control: STM32L476 ────────────────────────────────────────

/// `Adafruit_VCNL4010::readProximity()` writes COMMAND = 0x08 (prox_od) and then
/// spins on `read8(COMMAND) & 0x20`. On the NUCLEO-L476RG that poll must be
/// REAL: the flag stays clear for the datasheet's 570 µs of simulated time.
///
/// ⚠️ NEGATIVE CONTROL. On `origin/main` the final assertion fails — no
/// peripheral on an STM32L476 answers `Peripheral::sim_time_us`, so
/// `advance_central_i2c_time` returned immediately, the device was never
/// advanced, and `prox_data_rdy` stayed 0 no matter how long the machine ran.
#[test]
fn stm32l476_data_ready_is_gated_by_the_derived_clock() {
    const COMMAND: u8 = 0x80;
    const PROX_RDY: u8 = 0x20;

    fn read_command(dev: &mut GenericI2cDevice) -> u8 {
        dev.start();
        dev.write(COMMAND);
        dev.start();
        let b = dev.read();
        dev.stop();
        b
    }

    let mut machine = machine_from_system("configs/systems/nucleo-l476rg.yaml", "vcnl4010", "i2c1");
    // The board manifest declares the MSI reset rate the firmware actually runs
    // at; the derived clock is only honest if it uses that number.
    let cpu_hz = machine.bus.cpu_hz;
    assert_eq!(cpu_hz, 4_000_000, "nucleo-l476rg.yaml declares 4 MHz MSI");
    let cycles_per_us = cpu_hz / 1_000_000;

    // Anchor the drive, then start an on-demand proximity measurement.
    machine
        .advance(AdvanceRequest::run(Some(100)))
        .expect("advance");
    let dev = attached_generic_device(&mut machine);
    dev.start();
    dev.write(COMMAND);
    dev.write(0x08);
    dev.stop();
    assert_eq!(
        read_command(attached_generic_device(&mut machine)) & PROX_RDY,
        0,
        "the result cannot be ready in the instant the measurement started"
    );

    // 250 µs of simulated time — well inside the 570 µs conversion.
    machine
        .advance(AdvanceRequest::run(Some(250 * cycles_per_us)))
        .expect("advance");
    assert_eq!(
        read_command(attached_generic_device(&mut machine)) & PROX_RDY,
        0,
        "a too-short data-ready poll must still see the flag CLEAR — this is the \
         assertion that has no teeth without a device clock"
    );

    // Past it.
    machine
        .advance(AdvanceRequest::run(Some(600 * cycles_per_us)))
        .expect("advance");
    assert_eq!(
        read_command(attached_generic_device(&mut machine)) & PROX_RDY,
        PROX_RDY,
        "⚠️ NEGATIVE CONTROL: this is the arm that FAILS on origin/main — an \
         STM32L476 had no device clock at all, so the conversion never completed"
    );
}

/// The derived clock is `cycles * 1_000_000 / cpu_hz`, and that is checkable
/// against the device's own accounting rather than inferred from a flag.
#[test]
fn derived_clock_matches_cycles_over_cpu_hz() {
    let mut machine = machine_from_system("configs/systems/nucleo-l476rg.yaml", "sht31", "i2c1");
    let cpu_hz = machine.bus.cpu_hz;
    // A 15 ms delayed measurement on the shipping SHT31 descriptor.
    let dev = attached_generic_device(&mut machine);
    dev.start();
    dev.write(0x24);
    dev.write(0x00);
    dev.stop();

    // Just short of the deadline at 4 MHz: 14 ms == 56_000 cycles.
    machine
        .advance(AdvanceRequest::run(Some(14_000 * (cpu_hz / 1_000_000))))
        .expect("advance");
    let early = {
        let d = attached_generic_device(&mut machine);
        d.start();
        let out = [d.read(), d.read(), d.read()];
        d.stop();
        out
    };
    assert_eq!(early, [0xFF, 0xFF, 0xFF], "not ready at 14 ms");

    machine
        .advance(AdvanceRequest::run(Some(2_000 * (cpu_hz / 1_000_000))))
        .expect("advance");
    let late = {
        let d = attached_generic_device(&mut machine);
        d.start();
        let out = [d.read(), d.read(), d.read()];
        d.stop();
        out
    };
    assert_ne!(
        late,
        [0xFF, 0xFF, 0xFF],
        "the 15 ms measurement completes once the derived clock crosses it"
    );
}

// ─── A1.3: the same drive reaches SPI ──────────────────────────────────────

/// A `SpiDevice` that records nothing but the microseconds it is handed.
#[derive(Default)]
struct TimeCounter {
    total_us: Arc<Mutex<u64>>,
    calls: Arc<Mutex<u32>>,
}

impl SpiDevice for TimeCounter {
    fn transfer(&mut self, _mosi: u8) -> u8 {
        0
    }
    fn cs_pin(&self) -> &str {
        "PA4"
    }
    fn advance_time_us(&mut self, us: u64) {
        *self.total_us.lock().unwrap() += us;
        *self.calls.lock().unwrap() += 1;
    }
}

/// The elapsed µs an SPI device is handed must equal the elapsed µs of the run
/// — the SAME number an I²C slave on the same machine gets, because it comes
/// from the same drive.
#[test]
fn spi_devices_are_handed_the_elapsed_microseconds() {
    const CPU_HZ: u64 = 4_000_000;
    const CYCLES: u64 = 4_000_000; // exactly 1 s at 4 MHz

    let total = Arc::new(Mutex::new(0u64));
    let calls = Arc::new(Mutex::new(0u32));
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Stm32);
    spi.push_device(Box::new(TimeCounter {
        total_us: total.clone(),
        calls: calls.clone(),
    }));

    let mut bus = crate::bus::SystemBus::new();
    bus.cpu_hz = CPU_HZ;
    bus.add_peripheral("spi1", 0x4001_3000, 0x400, None, Box::new(spi));
    let mut machine = Machine::new(CountingCpu::default(), bus);

    machine
        .advance(AdvanceRequest::run(Some(CYCLES)))
        .expect("advance");

    let elapsed_cycles = machine.total_cycles;
    let expected_us = elapsed_cycles * 1_000_000 / CPU_HZ;
    assert_eq!(
        *total.lock().unwrap(),
        expected_us,
        "the SPI device's summed advance_time_us must equal the run's derived µs \
         ({elapsed_cycles} cycles at {CPU_HZ} Hz)"
    );
    assert!(
        *calls.lock().unwrap() > 0,
        "the drive must actually reach the SPI device, not merely total to zero"
    );
    assert!(expected_us > 0, "the run must have covered real time");
}

/// The declarative SPI engine records the same elapsed time, so Phase B's
/// timers have a clock to read. No behaviour rides on it yet — that is what
/// `declarative_device_byte_parity` pins.
#[test]
fn declarative_spi_device_records_elapsed_time() {
    use crate::peripherals::components::declarative_spi::GenericSpiDevice;

    let yaml = labwired_config::embedded_device_yaml("max31855").expect("embedded");
    let mut dev = GenericSpiDevice::from_yaml(yaml, "PA4").expect("parse");
    assert_eq!(dev.elapsed_us(), 0);
    SpiDevice::advance_time_us(&mut dev, 1_500);
    SpiDevice::advance_time_us(&mut dev, 500);
    assert_eq!(dev.elapsed_us(), 2_000);
}

// ─── A1.2: the census note ─────────────────────────────────────────────────

/// Build the ESP32-class machine of `i2c_central_time_drive`: SYSTIMER source,
/// ESP32-C3 controller, one delay device.
fn esp32_machine() -> Machine<CountingCpu> {
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
    let mut controller = Esp32c3I2c::new();
    controller.push_slave(Box::new(
        GenericI2cDevice::from_yaml(DELAY_DEVICE_YAML, 0).unwrap(),
    ));
    let mut bus = crate::bus::SystemBus::new();
    bus.cpu_hz = 160_000_000;
    bus.add_peripheral(
        "systimer",
        0x6002_3000,
        0x100,
        None,
        Box::new(Systimer::new_with_source(160_000_000, 37)),
    );
    bus.add_peripheral("i2c0", 0x6001_3000, 0x100, None, Box::new(controller));
    Machine::new(CountingCpu::default(), bus)
}

fn derived_notes() -> Vec<crate::fidelity::FidelityGap> {
    crate::fidelity::report()
        .to_gaps()
        .into_iter()
        .filter(|g| g.kind == crate::fidelity::DERIVED_DEVICE_TIME)
        .collect()
}

/// The approximation is recorded EXACTLY once on a derived-clock chip, no
/// matter how many slices run — and never on a chip whose µs are measured.
///
/// Both arms live in one test because the census is a thread-local the test
/// harness shares across threads only by luck; `reset()` here scopes it.
#[test]
fn fidelity_note_is_recorded_once_on_a_derived_chip_and_never_on_esp32() {
    if std::env::var_os("LABWIRED_STRICT_FIDELITY").is_some() {
        // The note does not panic under strict mode, but the ESP32 arm below
        // runs real peripherals that may. Nothing to prove here in that lane.
        return;
    }

    // ── ESP32: a modelled absolute-µs counter. No note, ever. ──
    crate::fidelity::reset();
    let mut esp = esp32_machine();
    esp.advance(AdvanceRequest::run(Some(2_000_000)))
        .expect("advance");
    assert!(
        derived_notes().is_empty(),
        "an ESP32 reads a real SYSTIMER; claiming a derived clock there would be a lie"
    );

    // ── STM32L476: derived. Exactly one note, across many slices. ──
    crate::fidelity::reset();
    let mut stm = machine_from_system("configs/systems/nucleo-l476rg.yaml", "vcnl4010", "i2c1");
    for _ in 0..8 {
        stm.advance(AdvanceRequest::run(Some(10_000)))
            .expect("advance");
    }
    let notes = derived_notes();
    assert_eq!(
        notes.len(),
        1,
        "the derived-clock approximation is a note, not a counter: {notes:?}"
    );
    assert_eq!(notes[0].count, 1);
    assert!(
        notes[0].detail.contains("derived from cpu_hz")
            && notes[0]
                .detail
                .contains("PLL reconfiguration is not tracked"),
        "the note must say what the reader is looking at: {}",
        notes[0].detail
    );
    // It rides the existing record shape — no new fields, no schema change.
    assert_eq!(notes[0].address.as_deref(), Some("0x0"));
    assert_eq!(notes[0].opcode, None);
    assert_eq!(notes[0].first_pc, "0x0");
    crate::fidelity::reset();
}
