// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Loop traces must retire exactly the cycles, and end in exactly the
//! machine state, the untraced basic-block JIT produces for the same
//! period. The invariant chain is: traced path == untraced JIT path
//! here, and untraced JIT path == interpreter is pinned separately by
//! `cortex_m_jit_nrf_differential.rs`.
//!
//! The fixture is the shipped RP2040 demo's hot shape, hand-assembled:
//! a counted loop whose body is pure ALU (`nop; adds; subs; bne head`).
//! With the trace on, one dispatch retires many iterations; with it off,
//! one dispatch per iteration. Both must land on the same pc, flags,
//! registers, memory and cycle count.

#![cfg(all(feature = "jit", feature = "event-scheduler"))]

use labwired_core::bus::{SystemBus, RECOMMENDED_TICK_INTERVAL};
use labwired_core::cpu::jit_framework::cortex_m::snapshot_state;
use labwired_core::cpu::jit_framework::differential::compare_memory;
use labwired_core::cpu::CortexM;
use labwired_core::{DebugControl, Machine};

fn h(bytes: &mut Vec<u8>, half: u16) {
    bytes.extend_from_slice(&half.to_le_bytes());
}

/// The RP2040 demo's hot loop, without the call:
///   0x00 movs r4, #250    ; r4 = iteration counter
///   0x02 movs r0, #0      ; r0 = accumulator
///   0x04 nop              ; <- loop head (traced entry)
///   0x06 adds r0, #1
///   0x08 subs r4, #1      ; sets Z on the last iteration
///   0x0A bne  0x04        ; backedge to the head (imm8 = -5 halfwords)
///   0x0C movs r1, #1      ; done marker
///   0x0E b .              ; park
fn counted_loop_machine(jit: bool, traces: bool) -> Machine<CortexM> {
    let mut prog = Vec::new();
    h(&mut prog, 0x24FA); // movs r4, #250 (0xFA) — 250 iterations
    h(&mut prog, 0x2000); // movs r0, #0
    h(&mut prog, 0xBF00); // nop              @0x04 head
    h(&mut prog, 0x3001); // adds r0, #1
    h(&mut prog, 0x3C01); // subs r4, #1
    h(&mut prog, 0xD1FB); // bne  0x04  (imm8 = -5 halfwords)
    h(&mut prog, 0x2101); // movs r1, #1
    h(&mut prog, 0xE7FE); // b .
    let mut bus = SystemBus::new();
    let n = prog.len();
    bus.flash.data[..n].copy_from_slice(&prog);
    let mut cpu = CortexM::new();
    cpu.pc = 0;
    cpu.sp = 0x2000_1000;
    cpu.xpsr = 0x0100_0000;
    let mut machine = Machine::new(cpu, bus);
    machine.config.cortex_m_jit_enabled = jit;
    machine.config.cortex_m_jit_loop_traces = traces;
    machine.bus.config.cortex_m_jit_enabled = jit;
    machine.bus.config.cortex_m_jit_loop_traces = traces;
    machine.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.bus.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.bus.legacy_walk_disabled = true;
    machine
}

#[test]
fn a_traced_counted_loop_retires_the_same_machine() {
    // `DebugControl::run` takes a STEP budget. The loop retires ~1005 steps
    // (4 loop ops × 250 iterations, plus preamble, marker and park), so this
    // is >> the 50-arrival hot threshold and >2x margin over the loop.
    let steps = 20_000;
    let mut off = counted_loop_machine(true, false);
    let mut on = counted_loop_machine(true, true);
    off.run(Some(steps)).unwrap();
    on.run(Some(steps)).unwrap();

    assert_eq!(
        snapshot_state(&off.cpu),
        snapshot_state(&on.cpu),
        "the trace changed the machine state"
    );
    assert_eq!(
        off.total_cycles, on.total_cycles,
        "the trace changed the retired cycle count"
    );
    assert_eq!(on.cpu.r0, 250, "accumulator ran the loop once");
    assert_eq!(on.cpu.r1, 1, "the loop exited and set the marker");
    if let Some(d) = compare_memory(off.total_cycles, off.cpu.pc, &off.bus, &on.bus) {
        panic!("trace RAM mismatch: {d:?}");
    }

    // Non-vacuous: the trace path must have actually retired iterations.
    let stats = on.cpu.jit_stats().expect("jit engine present");
    assert!(stats.trace_iters > 0, "the loop trace never ran: {stats:?}");
    let off_stats = off.cpu.jit_stats().expect("jit engine present");
    assert_eq!(off_stats.trace_iters, 0, "traces off must not trace");
}

#[test]
fn a_trace_stops_at_a_partial_window_and_resumes_at_the_head() {
    // Per-slice step budget smaller than one natural run (~1005 steps): the
    // trace must stop mid-loop, report its iterations, and the next dispatch
    // must resume at the head — final state and RAM equal.
    let mut off = counted_loop_machine(true, false);
    let mut on = counted_loop_machine(true, true);
    for _ in 0..40 {
        off.run(Some(64)).unwrap();
        on.run(Some(64)).unwrap();
        assert_eq!(off.cpu.pc, on.cpu.pc, "resume pc diverged");
    }
    assert_eq!(snapshot_state(&off.cpu), snapshot_state(&on.cpu));
    assert_eq!(off.total_cycles, on.total_cycles);
    if let Some(d) = compare_memory(off.total_cycles, off.cpu.pc, &off.bus, &on.bus) {
        panic!("trace RAM mismatch: {d:?}");
    }
}
