// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! An NE555 astable built only from primitives the engine already solves.
//!
//! `ne555_astable.cir` is the deck the LabWired canvas lowering emits for the
//! catalog part `ne555` (its `analogMacro`) wired as the textbook astable:
//! 9 V, R1 = 10k from VCC to DIS, R2 = 68k from DIS to THR/TRIG, C = 10 uF,
//! and an LED behind 1k on OUT. The 555 itself is a 5k/5k/5k divider, two
//! signed voltage-controlled switches as the threshold and trigger
//! comparators, a self-holding switch as the latch, a discharge switch and a
//! two-diode output stage. The monorepo pins this file byte-for-byte against
//! its own lowering, so this test runs exactly what a canvas run runs.
//!
//! Datasheet arithmetic: t_high = ln2 (R1 + R2) C = 0.541 s,
//! t_low = ln2 R2 C = 0.471 s, f = 1 / (t_high + t_low) = 0.986 Hz.

use labwired_core::analog::{parse_netlist, Integration, Solver};

const DECK: &str = include_str!("ne555_astable.cir");
/// The canvas lowering's default analog step.
const STEP_S: f64 = 100e-6;

#[test]
fn ne555_astable_oscillates_at_the_datasheet_frequency() {
    let circuit = parse_netlist(DECK).expect("deck parses");
    let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
    let out = solver.circuit().node("v_net_r3_1").expect("OUT node");
    let cap = solver
        .circuit()
        .node("v_net_c1_1")
        .expect("timing capacitor node");

    let steps = (5.0 / STEP_S) as usize;
    let mut rises = Vec::new();
    let mut falls = Vec::new();
    let mut high = solver.node_voltage(out) > 4.0;
    let (mut cap_min, mut cap_max) = (f64::MAX, f64::MIN);
    for step in 1..=steps {
        solver.advance(STEP_S).expect("step solves");
        let t = step as f64 * STEP_S;
        let v = solver.node_voltage(out);
        if !high && v > 4.0 {
            rises.push(t);
        } else if high && v < 4.0 {
            falls.push(t);
        }
        high = v > 4.0;
        // Steady state only: the operating point starts the capacitor charged.
        if t > 1.5 {
            let vc = solver.node_voltage(cap);
            cap_min = cap_min.min(vc);
            cap_max = cap_max.max(vc);
        }
    }

    assert!(
        rises.len() >= 4,
        "too few cycles in 5 s: rises at {rises:?}"
    );
    for pair in rises.windows(2).skip(1) {
        let f = 1.0 / (pair[1] - pair[0]);
        assert!(
            (0.9..=1.1).contains(&f),
            "astable frequency {f:.4} Hz (rises {rises:?})"
        );
        assert!(
            (f - 0.986).abs() < 0.01,
            "frequency {f:.4} Hz is not the datasheet 0.986 Hz"
        );
    }
    let period = rises[rises.len() - 1] - rises[rises.len() - 2];
    eprintln!(
        "ne555 astable: period {period:.4} s, {:.4} Hz",
        1.0 / period
    );
    // Duty cycle: high for ln2 (R1 + R2) C of each period.
    let last = rises.len() - 1;
    let fall = falls
        .iter()
        .copied()
        .find(|&t| t > rises[last - 1])
        .expect("a fall inside the last full period");
    let duty = (fall - rises[last - 1]) / (rises[last] - rises[last - 1]);
    assert!(
        (duty - 0.534).abs() < 0.01,
        "duty {duty:.3}, datasheet 0.534"
    );
    // The capacitor swings between the 1/3 and 2/3 VCC thresholds.
    assert!(
        (cap_min - 3.0).abs() < 0.05,
        "capacitor low {cap_min:.3} V, expected 3 V"
    );
    assert!(
        (cap_max - 6.0).abs() < 0.05,
        "capacitor high {cap_max:.3} V, expected 6 V"
    );
}

#[test]
fn ne555_reset_low_holds_out_low() {
    // The deck with RESET wired to ground instead of tied to VCC.
    let deck = DECK.replace("Sreset_U1 q_U1 0 v_net_j1__plus 0", "Sreset_U1 q_U1 0 0 0");
    let circuit = parse_netlist(&deck).expect("deck parses");
    let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
    let out = solver.circuit().node("v_net_r3_1").expect("OUT node");
    for _ in 0..(2.0 / STEP_S) as usize {
        solver.advance(STEP_S).expect("step solves");
        assert!(
            solver.node_voltage(out) < 0.1,
            "OUT left low while RESET is low"
        );
    }
}
