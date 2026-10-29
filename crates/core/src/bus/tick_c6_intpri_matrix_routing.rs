// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C6 interrupt-matrix routing through the split INTPRI block.
//!
//! The C6 is the first part in the tree whose matrix enable/priority/threshold
//! registers and `CPU_INTR_FROM_CPU_n` doorbells live OUTSIDE the
//! `INTERRUPT_CORE0` MAP bank (C6: PLIC @0x2000_1000 for the gates, INTPRI
//! @0x600C_5000 for the doorbells, `interrupt_core0` @0x6001_0000 for MAPs;
//! C3: everything in one 0x600C_2000 bank). These tests pin the C6 layout the
//! bus derives from the chip descriptor:
//!
//!   * a C6 bus built from `esp32c6.yaml` arms matrix routing by itself — the
//!     presence of the C6-only INTPRI block is the gate (`routing == true`);
//!   * `CPU_INTR_FROM_CPU_0` (matrix source 22, NOT the C3's 50) routes through
//!     its MAP register to the enabled CPU line, level-sensitively at the MMIO
//!     write choke;
//!   * the PLIC enable gate and the priority/threshold gate both really mask,
//!     so a line is not asserted merely because a source is pending.
//!
//! The end-to-end trap (fixture firmware takes the RISC-V trap from the doorbell
//! and clears it in the handler) is proven by
//! `examples/tier1-fixture/esp32c6`; this module pins the bus state it depends
//! on without building a CPU.

use crate::bus::SystemBus;
use crate::Bus;
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;

const INTERRUPT_CORE0: u64 = 0x6001_0000;
const INTPRI: u64 = 0x600C_5000;
const PLIC_MX: u64 = 0x2000_1000;

/// `CPU_INTR_FROM_CPU_0` matrix source (`ETS_FROM_CPU_INTR0_SOURCE` in the
/// ESP32-C6 SVD; the C3 numbers the same doorbell 50).
const SOURCE_FROM_CPU_0: u64 = 22;
const LINE: u32 = 9;

const PLIC_ENABLE: u64 = 0x00;
const PLIC_PRI_BASE: u64 = 0x10; // PLIC_MXINT_n_PRI @ 0x10 + n*4
const PLIC_THRESH: u64 = 0x90;
const CPU_INTR_FROM_CPU_0: u64 = 0x90; // INTPRI doorbell

fn c6_bus() -> SystemBus {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = root.join("../../configs/chips/esp32c6.yaml");
    let system_path = root.join("../../configs/systems/esp32c6-devkitc.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load esp32c6 chip yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load c6 system yaml");
    // Anchor the manifest's chip path to the manifest's directory so
    // `resolve_peripheral_path` finds the chip-relative descriptor files.
    manifest.chip = system_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_string_lossy()
        .into_owned();
    SystemBus::from_config(&chip, &manifest).expect("build c6 devkit bus")
}

/// Arm the doorbell path: MAP source 22 → `LINE`, PLIC-enable `LINE`, priority 1.
fn arm_line(bus: &mut SystemBus) {
    bus.write_u32(INTERRUPT_CORE0 + SOURCE_FROM_CPU_0 * 4, LINE)
        .unwrap();
    bus.write_u32(PLIC_MX + PLIC_ENABLE, 1 << LINE).unwrap();
    bus.write_u32(PLIC_MX + PLIC_PRI_BASE + (LINE as u64) * 4, 1)
        .unwrap();
}

#[test]
fn c6_descriptor_arms_matrix_routing_and_routes_the_doorbell() {
    let mut bus = c6_bus();
    assert!(
        bus.irq_fabric.esp32c3.routing,
        "the C6-only INTPRI block must arm matrix routing from the descriptor"
    );

    arm_line(&mut bus);
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines, 0,
        "an idle doorbell must not assert a line"
    );

    // Ring CPU_INTR_FROM_CPU_0: the write choke re-routes source 22 → line 9.
    bus.write_u32(INTPRI + CPU_INTR_FROM_CPU_0, 1).unwrap();
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines,
        1 << LINE,
        "the doorbell's matrix source (22 on the C6, not 50) must reach the routed line"
    );

    // Acknowledge: clearing the doorbell de-asserts the level.
    bus.write_u32(INTPRI + CPU_INTR_FROM_CPU_0, 0).unwrap();
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines, 0,
        "clearing the doorbell must de-assert the routed line"
    );
}

#[test]
fn c6_enable_gate_masks_a_pending_doorbell() {
    let mut bus = c6_bus();
    arm_line(&mut bus);

    // Disable the line, then ring the doorbell: nothing may be routed.
    bus.write_u32(PLIC_MX + PLIC_ENABLE, 0).unwrap();
    bus.write_u32(INTPRI + CPU_INTR_FROM_CPU_0, 1).unwrap();
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines, 0,
        "PLIC enable=0 must mask the pending source"
    );

    // Enabling the line with the doorbell still pending delivers it.
    bus.write_u32(PLIC_MX + PLIC_ENABLE, 1 << LINE).unwrap();
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines,
        1 << LINE,
        "enabling the line must deliver the already-pending doorbell"
    );
}

#[test]
fn c6_priority_below_threshold_is_masked() {
    let mut bus = c6_bus();
    arm_line(&mut bus);

    // Raise the threshold above the line's priority: the level is held back.
    bus.write_u32(PLIC_MX + PLIC_THRESH, 5).unwrap();
    bus.write_u32(INTPRI + CPU_INTR_FROM_CPU_0, 1).unwrap();
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines, 0,
        "priority 1 < threshold 5 must mask the line"
    );

    // Raise the priority above the threshold: the same pending source passes.
    bus.write_u32(PLIC_MX + PLIC_PRI_BASE + (LINE as u64) * 4, 6)
        .unwrap();
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines,
        1 << LINE,
        "priority 6 >= threshold 5 must route the pending source"
    );
}

/// `ETS_UART0_INTR_SOURCE` (esp32c6.svd `INTERRUPT_CORE0` interrupt `UART0`);
/// the C3 numbers the same UART0 21.
const SOURCE_UART0: u64 = 43;
/// A second, distinct CPU line so the UART proof cannot alias the doorbell's.
const UART0_LINE: u32 = 10;

const UART0_BASE: u64 = 0x6000_0000;
const UART_INT_RAW: u64 = 0x04;
const UART_INT_ENA: u64 = 0x0C;
/// `UART_INTR_TXFIFO_EMPTY` (`uart_ll.h` bit 1) — a LIVE level bit that is high
/// whenever the TX FIFO is below its empty threshold, so enabling it needs no
/// traffic and gives a deterministic source to route.
const UART_TXFIFO_EMPTY: u32 = 1 << 1;

/// A REAL peripheral source end to end on the bus side: UART0's declared
/// matrix source (43) routes through its `interrupt_core0` MAP word to an
/// enabled INTPRI line while UART0 asserts `TXFIFO_EMPTY`, and clears the
/// routed line when the UART interrupt is disabled. The trap itself is proven
/// by the tier-1 fixture; this pins the fabric state it depends on.
#[test]
fn c6_uart0_peripheral_source_routes_and_deasserts() {
    let mut bus = c6_bus();

    // The UART twin carries the descriptor-declared C6 source id (43) on the
    // `Peripheral` capability — the same wiring the C3 I2C test reads.
    let uart_idx = bus.find_peripheral_index_by_name("uart0").unwrap();
    assert_eq!(
        bus.peripherals[uart_idx].dev.matrix_irq_source_id(),
        Some(SOURCE_UART0 as u32),
        "uart0 must declare the C6 UART0 matrix source, not the C3's 21"
    );

    // Route source 43 -> line 10, PLIC-enable the line with a passing priority.
    bus.write_u32(INTERRUPT_CORE0 + SOURCE_UART0 * 4, UART0_LINE)
        .unwrap();
    bus.write_u32(PLIC_MX + PLIC_ENABLE, 1 << UART0_LINE)
        .unwrap();
    bus.write_u32(PLIC_MX + PLIC_PRI_BASE + (UART0_LINE as u64) * 4, 1)
        .unwrap();
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines, 0,
        "an idle UART must not assert a line"
    );

    // Enable the live TXFIFO_EMPTY level bit: the empty FIFO (0 < the reset
    // empty threshold, 96) asserts source 43.
    bus.write_u32(UART0_BASE + UART_INT_ENA, UART_TXFIFO_EMPTY)
        .unwrap();
    assert_ne!(
        bus.read_u32(UART0_BASE + UART_INT_RAW).unwrap() & UART_TXFIFO_EMPTY,
        0,
        "the TXFIFO_EMPTY level must be live at reset"
    );
    // One peripheral tick aggregates the walk-emitted source (legacy-walk
    // build) and re-derives scheduler-driven levels (event-scheduler build).
    bus.tick_peripherals_fully();
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines,
        1 << UART0_LINE,
        "UART0 source 43 must reach the routed CPU line"
    );

    // INT_ENA=0 is the UART's level de-assert (TXFIFO_EMPTY is live, not
    // W1C): the routed line must drop on the next re-derivation.
    bus.write_u32(UART0_BASE + UART_INT_ENA, 0).unwrap();
    bus.tick_peripherals_fully();
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines, 0,
        "clearing UART INT_ENA must de-assert the routed line"
    );
}
