// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `SessionMachine` type erasure: a `Machine<C>` behind a `Box<dyn _>` must
//! still advance, count cycles, enumerate inputs and round-trip a snapshot.

use labwired_core::machine::AdvanceRequest;
use labwired_core::session::machine::SessionMachine;
use labwired_core::{bus::SystemBus, system::cortex_m::configure_cortex_m, Machine};

fn tiny_arm_machine() -> Box<dyn SessionMachine> {
    // `SystemBus::new()` is the default test layout: 1 MB flash at 0x0 and
    // 1 MB RAM at 0x2000_0000 (NOT the STM32 0x0800_0000 alias), so the
    // vector table is written at flash base 0.
    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    // vector table: SP=0x2000_1000, PC=0x0000_0009 (thumb) -> `b .` at 0x8
    let mut img = labwired_core::memory::ProgramImage::new(0x0000_0009, labwired_core::Arch::Arm);
    let mut seg = vec![0u8; 12];
    seg[0..4].copy_from_slice(&0x2000_1000u32.to_le_bytes());
    seg[4..8].copy_from_slice(&0x0000_0009u32.to_le_bytes());
    seg[8..10].copy_from_slice(&0xE7FEu16.to_le_bytes()); // b .
    img.add_segment(0x0000_0000, seg);
    m.load_firmware(&img).unwrap();
    Box::new(m)
}

#[test]
fn erased_machine_advances_and_counts_cycles() {
    let mut m = tiny_arm_machine();
    let before = m.cycles();
    let report = m.advance(AdvanceRequest::run(Some(100))).unwrap();
    assert!(report.fuel_consumed > 0);
    assert!(m.cycles() > before);
}

#[test]
fn erased_machine_lists_inputs_and_snapshots() {
    let mut m = tiny_arm_machine();
    assert!(m.list_inputs().is_empty()); // no board IO attached
    let snap = m.snapshot();
    m.advance(AdvanceRequest::run(Some(10))).unwrap();
    m.apply_snapshot(&snap).unwrap();
    assert_eq!(m.bus_trace_events().len(), 0);
}
