// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Voltage-controlled switches: the contact that a relay coil closes.

use labwired_core::analog::{parse_netlist, Integration, Solver};

fn run_to(solver: &mut Solver, h: f64, steps: usize) {
    for _ in 0..steps {
        solver.advance(h).expect("step solves");
    }
}

const RELAY: &str = "* relay coil + NO\n\
Vsup vcc 0 dc 5\n\
Sdrv vcc coil_p gate ron=0.05 roff=1meg\n\
Rcoil coil_p mid 70\n\
Lcoil mid 0 10m\n\
Dfly 0 coil_p DFLY\n\
.model DFLY D (IS=2.52e-9 N=1.752 RS=0)\n\
Vload load 0 dc 5\n\
Rload load com 1k\n\
Sno com 0 coil_p 0 vt=3.75 vh=3.25 ron=0.1 roff=1meg\n\
.end\n";

#[test]
fn voltage_switch_closes_above_vt_and_opens_below_dropout() {
    // Pull-up on `out`, shunted by a switch that senses `ctrl`.
    let circuit = parse_netlist(
        "* vcs\n\
         Vctrl ctrl 0 dc 0\n\
         Vdd vdd 0 dc 5\n\
         R1 vdd out 1k\n\
         S1 out 0 ctrl 0 vt=3.75 vh=3.25 ron=1 roff=1meg\n\
         C1 out 0 1p\n\
         .end\n",
    )
    .expect("netlist parses");
    let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
    let out = solver.circuit().node("out").expect("out");
    assert!(
        solver.switches_controlled_by("").is_empty(),
        "voltage switches are not digitally routed"
    );

    // Below pickup: contact open, out near 5 V.
    solver.set_voltage_source(0, 0.0);
    run_to(&mut solver, 1e-6, 50);
    let open = solver.node_voltage(out);
    assert!(open > 4.5, "open contact: {open} V");

    // Above pickup: contact closes.
    solver.set_voltage_source(0, 5.0);
    run_to(&mut solver, 1e-6, 50);
    let closed = solver.node_voltage(out);
    assert!(closed < 0.05, "closed contact: {closed} V");
    assert!(closed < open / 50.0, "closing must pull out down");

    // Between dropout and pickup: hysteresis holds it closed.
    solver.set_voltage_source(0, 1.0); // dropout = vt-vh = 0.5; 1.0 still above
    run_to(&mut solver, 1e-6, 50);
    let held = solver.node_voltage(out);
    assert!(held < 0.05, "hysteresis holds closed at 1 V: {held} V");

    // Below dropout: opens.
    solver.set_voltage_source(0, 0.0);
    run_to(&mut solver, 1e-6, 50);
    let released = solver.node_voltage(out);
    assert!(released > 4.5, "opens below dropout: {released} V");
}

#[test]
fn inverted_voltage_switch_is_the_nc_contact() {
    // NC: closed at rest, opens when |V| exceeds vt.
    let circuit = parse_netlist(
        "* nc\n\
         Vctrl ctrl 0 dc 0\n\
         Vdd vdd 0 dc 5\n\
         R1 vdd out 1k\n\
         S1 out 0 ctrl 0 vt=3.75 vh=3.25 invert=1 ron=1 roff=1meg\n\
         C1 out 0 1p\n\
         .end\n",
    )
    .expect("netlist parses");
    let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
    let out = solver.circuit().node("out").expect("out");

    run_to(&mut solver, 1e-6, 50);
    let at_rest = solver.node_voltage(out);
    assert!(at_rest < 0.05, "NC closed at rest: {at_rest} V");

    solver.set_voltage_source(0, 5.0);
    run_to(&mut solver, 1e-6, 50);
    let energised = solver.node_voltage(out);
    assert!(energised > 4.5, "NC opens when energised: {energised} V");
}

#[test]
fn relay_coil_current_and_contact_follow_drive_with_flyback_clamp() {
    // Songle-ish 5 V SPDT: 70 Ω / 10 mH coil, low-side digital drive switch,
    // flyback diode, and a voltage-controlled NO contact across a 5 V load.
    let circuit = parse_netlist(RELAY).expect("relay netlist parses");
    let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
    let com = solver.circuit().node("com").expect("com");
    let coil_p = solver.circuit().node("coil_p").expect("coil_p");
    let drive = solver.switches_controlled_by("gate");
    assert_eq!(drive.len(), 1, "drive switch is digitally routed");

    // Rest: no coil current, NO open, com near 5 V.
    run_to(&mut solver, 50e-6, 40);
    assert!(
        solver.inductor_current(0).abs() < 1e-4,
        "rest coil current {} A",
        solver.inductor_current(0)
    );
    assert!(
        solver.node_voltage(com) > 4.5,
        "NO open at rest: {} V",
        solver.node_voltage(com)
    );

    // Energise: coil current rises toward 5/70, NO closes.
    solver.set_switch(drive[0], true);
    run_to(&mut solver, 50e-6, 400); // 20 ms >> tau = 0.14 ms
    let coil_i = solver.inductor_current(0).abs();
    let expected = 5.0 / 70.0;
    assert!(
        (coil_i - expected).abs() / expected < 0.05,
        "settled coil {coil_i} A vs {expected} A"
    );
    assert!(
        solver.node_voltage(com) < 0.1,
        "NO closed when energised: {} V",
        solver.node_voltage(com)
    );

    // Release: flyback clamps coil_p, coil current decays, NO reopens.
    solver.set_switch(drive[0], false);
    let mut abs_peak = 0.0f64;
    let mut first_i = None;
    for step in 0..400 {
        solver.advance(50e-6).expect("step");
        abs_peak = abs_peak.max(solver.node_voltage(coil_p).abs());
        if step == 0 {
            first_i = Some(solver.inductor_current(0).abs());
        }
    }
    let first_i = first_i.unwrap();
    assert!(
        first_i > 0.02,
        "coil still carries current one step after release: {first_i} A"
    );
    assert!(
        abs_peak < 5.0 + 2.0,
        "flyback clamps |V(coil_p)|, peaked at {abs_peak} V"
    );
    assert!(
        solver.inductor_current(0).abs() < 1e-3,
        "coil current gone after release: {} A",
        solver.inductor_current(0)
    );
    assert!(
        solver.node_voltage(com) > 4.5,
        "NO reopens after release: {} V",
        solver.node_voltage(com)
    );
}

#[test]
fn spdt_no_nc_transfer_is_atomic_same_update() {
    // Lean choice vs break-before-make: contacts that share a sense node flip in
    // the same update_voltage_switches pass, so COM moves from NC to NO with no
    // observable both-open interval between advances. Pin that so a later BBM
    // change has to update this test on purpose.
    let circuit = parse_netlist(
        "* spdt atomic\n\
         Vctrl ctrl 0 dc 0\n\
         Vdd vdd 0 dc 5\n\
         Rno vdd no 1k\n\
         Rnc vdd nc 1k\n\
         Sno no 0 ctrl 0 vt=3.75 vh=3.25 ron=1 roff=1meg\n\
         Snc nc 0 ctrl 0 vt=3.75 vh=3.25 invert=1 ron=1 roff=1meg\n\
         Cno no 0 1p\n\
         Cnc nc 0 1p\n\
         .end\n",
    )
    .expect("netlist parses");
    let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
    let no = solver.circuit().node("no").expect("no");
    let nc = solver.circuit().node("nc").expect("nc");

    run_to(&mut solver, 1e-6, 50);
    let no_rest = solver.node_voltage(no);
    let nc_rest = solver.node_voltage(nc);
    assert!(no_rest > 4.5, "NO open at rest: {no_rest} V");
    assert!(nc_rest < 0.05, "NC closed at rest: {nc_rest} V");

    // Drive through pickup. Voltage switches refresh at the start of each step
    // from the previous solution, so the first step that observes a transfer is
    // the one whose update flipped both contacts together.
    solver.set_voltage_source(0, 5.0);
    let mut saw_transfer = false;
    for _ in 0..20 {
        solver.advance(1e-6).expect("step");
        let no_closed = solver.node_voltage(no) < 0.5;
        let nc_open = solver.node_voltage(nc) > 4.0;
        if no_closed || nc_open {
            assert!(
                no_closed && nc_open,
                "atomic transfer: after the update that moved either contact, \
                 NO closed={no_closed} NC open={nc_open} (no BBM gap)"
            );
            saw_transfer = true;
            break;
        }
    }
    assert!(
        saw_transfer,
        "expected NO/NC to transfer within a few steps after energising"
    );
}
