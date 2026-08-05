// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 RMT TX_END must reach the CPU on the SHIPPED (walk-deleted) bus.
//!
//! ## What this asserts, and why it is not the existing RMT test
//!
//! `Esp32c3Rmt`'s own unit test (`logic_tap_sees_tx_start_pulse`) and the
//! matrix L2 oracle both observe `tx_start_count` / the LogicTap edge. Both of
//! those are produced *inside the `CONF0` write*, so they are walk-independent
//! by construction: they are green whether or not the model is ever ticked.
//! No test in this repo asserted RMT **interrupt delivery**, which was the one
//! thing that depended on the walk.
//!
//! The observable here is therefore `SystemBus::riscv_irq_lines` — the routed
//! CPU interrupt-line mask the RISC-V core reads at its instruction boundary.
//! Not `legacy_walk_disabled`, not `matrix_irq_sources()`, not the model's own
//! `INT_ST`: the bit that actually reaches the CPU.
//!
//! ## The defect this pins
//!
//! `Esp32c3Rmt` declared `needs_legacy_walk() == false` while its `tick()`
//! emitted `explicit_irqs: [28]` whenever `INT_ST != 0`, and it exported its
//! level through the *returning* `matrix_irq_sources()` without ever declaring
//! `uses_scheduler()`. On the shipped C3 bus every peripheral qualifies, so
//! `derive_walk_deletable()` deletes the legacy walk — and then:
//!
//!   * the walk never calls `tick()`, so `explicit_irqs` never fires, and
//!   * `poll_scheduler_matrix_sources()` only polls `uses_scheduler()` models,
//!     so the level export is never read either.
//!
//! Both delivery channels closed. `rgbLedWrite` →
//! `rmtWrite(..., RMT_WAIT_FOR_EVER)` blocks on a semaphore given from that
//! ISR, so the firmware livelocks — the same shape as the classic-ESP32 UART
//! TX-FIFO defect.
//!
//! Runs in every feature configuration: with the walk ON (no
//! `event-scheduler`) delivery must work through `tick()`, and with the walk
//! DELETED it must work through the scheduler matrix export. A fix that only
//! satisfies one of the two is not a fix.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

/// ETS_RMT_INTR_SOURCE on ESP32-C3.
const RMT_SOURCE: u32 = 28;
/// CPU interrupt line firmware binds RMT to (esp-idf `intr_alloc` picks a free
/// one; the number is arbitrary, only the routing matters).
const LINE: u32 = 1;
const INTC: u64 = 0x600C_2000;

// RMT register offsets (C3 map — see `peripherals::esp32c3::rmt`).
const RMT_CH0_TX_CONF0: u64 = 0x10;
const RMT_INT_ST: u64 = 0x3C;
const RMT_INT_ENA: u64 = 0x40;
const RMT_INT_CLR: u64 = 0x44;
const TX_START: u32 = 1 << 0;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// The shipped `esp32c3-devkit` bus with walk-deletion AUTO-DERIVED (never the
/// hand `walk_deleted` hatch), plus the C3 interrupt-matrix routing the browser
/// rom-boot entry turns on.
fn bus_esp32c3_devkit() -> SystemBus {
    let chip = ChipDescriptor::from_file(root("configs/chips/esp32c3.yaml")).expect("chip yaml");
    let system_path = root("configs/systems/esp32c3-devkit.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("system yaml");
    let anchored = system_path.parent().expect("parent").join(&manifest.chip);
    manifest.chip = anchored.to_string_lossy().into_owned();
    manifest.walk_deleted = None;
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build c3 bus");
    let _ = labwired_core::system::riscv::configure_riscv(&mut bus);
    // `build_rom_boot_machine` (the browser C3 entry) enables matrix routing and
    // then re-derives walk deletion over the final peripheral set. Mirror both.
    bus.esp32c3_irq_routing = true;
    bus.recompute_walk_deletable();
    bus
}

/// Bind matrix source `RMT_SOURCE` to CPU line `LINE` at priority 15 and enable
/// it — exactly what esp-idf's `intr_alloc` writes.
fn route_rmt_to_cpu(bus: &mut SystemBus) {
    bus.write_u32(INTC + u64::from(RMT_SOURCE) * 4, LINE)
        .unwrap();
    bus.write_u32(INTC + 0x114 + u64::from(LINE) * 4, 15)
        .unwrap();
    bus.write_u32(INTC + 0x104, 1 << LINE).unwrap();
    bus.write_u32(INTC + 0x194, 1).unwrap();
}

fn rmt_base(bus: &SystemBus) -> u64 {
    let idx = bus
        .find_peripheral_index_by_name("rmt")
        .expect("rmt present");
    bus.peripherals[idx].base
}

/// TX_END must raise the routed CPU line under the ORDINARY production tick.
#[test]
fn rmt_tx_end_reaches_the_cpu_on_the_shipped_c3_bus() {
    let mut bus = bus_esp32c3_devkit();
    let base = rmt_base(&bus);
    route_rmt_to_cpu(&mut bus);

    // Arm CH0/CH1 TX_END, then kick a transmission. The model completes TX
    // inside the CONF0 write and latches INT_RAW bit 0.
    bus.write_u32(base + RMT_INT_ENA, 0x3).unwrap();
    bus.write_u32(base + RMT_CH0_TX_CONF0, TX_START).unwrap();
    assert_ne!(
        bus.read_u32(base + RMT_INT_ST).unwrap(),
        0,
        "precondition: RMT TX_END must be asserting after TX_START"
    );

    // Whatever the browser runs: the ordinary production tick. 64 of them is
    // three orders of magnitude more than any delivery path needs.
    for _ in 0..64 {
        let _ = bus.tick_peripherals_with_costs();
    }

    assert_ne!(
        bus.riscv_irq_lines & (1 << LINE),
        0,
        "STARVED: RMT TX_END (source {RMT_SOURCE}) never reached CPU line {LINE}. \
         riscv_irq_lines={:#x}, legacy_walk_disabled={}, INT_ST={:#x}. \
         rgbLedWrite blocks forever on the semaphore this IRQ gives.",
        bus.riscv_irq_lines,
        bus.legacy_walk_disabled,
        bus.read_u32(base + RMT_INT_ST).unwrap(),
    );
}

/// The level must DE-ASSERT when firmware acknowledges via `INT_CLR`.
///
/// A fix that latches the line forever swaps a livelock for an ISR storm, so
/// the de-assert direction is asserted too — a level export is only correct if
/// it is re-derived in both directions.
#[test]
fn rmt_tx_end_de_asserts_after_int_clr() {
    let mut bus = bus_esp32c3_devkit();
    let base = rmt_base(&bus);
    route_rmt_to_cpu(&mut bus);

    bus.write_u32(base + RMT_INT_ENA, 0x3).unwrap();
    bus.write_u32(base + RMT_CH0_TX_CONF0, TX_START).unwrap();
    for _ in 0..8 {
        let _ = bus.tick_peripherals_with_costs();
    }
    assert_ne!(
        bus.riscv_irq_lines & (1 << LINE),
        0,
        "precondition: line must be asserted before the acknowledge"
    );

    // The ISR acknowledges CH0 TX_END.
    bus.write_u32(base + RMT_INT_CLR, 0x3).unwrap();
    assert_eq!(
        bus.read_u32(base + RMT_INT_ST).unwrap(),
        0,
        "precondition: INT_CLR must clear the model's own status"
    );
    for _ in 0..8 {
        let _ = bus.tick_peripherals_with_costs();
    }

    assert_eq!(
        bus.riscv_irq_lines & (1 << LINE),
        0,
        "LATCHED: RMT line {LINE} stayed asserted after INT_CLR \
         (riscv_irq_lines={:#x}) — the ISR would re-enter forever",
        bus.riscv_irq_lines,
    );
}

/// The C3 walk must STILL be deleted after the fix.
///
/// This is the throughput half of the contract: the honest-but-wrong repair for
/// the starvation above is `needs_legacy_walk() -> true`, which un-deletes the
/// whole C3 walk and costs the 512x peripheral-tick batching every shipped C3
/// lab depends on. Pinning walk-deletion here makes that repair fail its own
/// test instead of silently shipping as a slowdown.
#[cfg(feature = "event-scheduler")]
#[test]
fn c3_devkit_walk_stays_deleted() {
    let bus = bus_esp32c3_devkit();
    let forcers: Vec<&str> = bus
        .peripherals
        .iter()
        .filter(|p| !p.dev.uses_scheduler() && p.dev.needs_legacy_walk())
        .map(|p| p.name.as_str())
        .collect();
    assert!(
        bus.legacy_walk_disabled,
        "esp32c3-devkit must keep auto-deriving walk deletion; forcers: {forcers:?}"
    );
}
