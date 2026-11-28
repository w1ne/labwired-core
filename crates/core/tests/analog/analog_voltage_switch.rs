// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Voltage-controlled switches: the contact that a relay coil closes.

use labwired_core::analog::{parse_netlist, AnalogError, Integration, Solver};

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

#[test]
fn signed_logic_switch_does_not_enable_on_negative_voltage() {
    for signed in [false, true] {
        for invert in [false, true] {
            let deck = format!("* signed logic versus relay control\nVctrl ctrl ref dc -5\nVref ref 0 dc 10\nVdd vdd 0 dc 5\nRload vdd out 1k\nS1 out 0 ctrl ref vt=2.5 vh=1.7 signed={} invert={} ron=1 roff=1meg\n.end\n", u8::from(signed), u8::from(invert));
            let circuit = parse_netlist(&deck).expect("signed control parses");
            let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
            let out = solver.circuit().node("out").expect("out");
            run_to(&mut solver, 1e-6, 20);
            let expected_closed = !signed ^ invert;
            assert_eq!(
                solver.node_voltage(out) < 0.05,
                expected_closed,
                "negative control: signed={signed} invert={invert}"
            );
            solver.set_voltage_source(0, 3.3);
            run_to(&mut solver, 1e-6, 20);
            assert_eq!(solver.node_voltage(out) < 0.05, !invert, "positive control");
            solver.set_voltage_source(0, 1.5);
            run_to(&mut solver, 1e-6, 20);
            assert_eq!(solver.node_voltage(out) < 0.05, !invert, "hysteresis holds");
            solver.set_voltage_source(0, 0.2);
            run_to(&mut solver, 1e-6, 20);
            assert_eq!(solver.node_voltage(out) < 0.05, invert, "drops below 0.8 V");
        }
    }
}

#[test]
fn signed_switch_rejects_non_boolean_mode() {
    let err =
        parse_netlist("* invalid\nS1 a 0 c 0 vt=1 signed=2 ron=1 roff=1meg\n.end\n").unwrap_err();
    assert!(err.to_string().contains("signed must be 0 or 1"), "{err}");
}

#[test]
fn ngspice_sw_model_switches_on_the_signed_band_around_vt() {
    // ngspice closes at V > VT + VH and opens at V < VT − VH. With VT = 0
    // the opening edge is below 0 V, which a magnitude switch cannot express.
    let circuit = parse_netlist(
        "* ngspice switch\n\
         Vctrl ctrl 0 dc -1\n\
         Vdd vdd 0 dc 5\n\
         Rload vdd out 1k\n\
         S1 out 0 ctrl 0 SMOD\n\
         .model SMOD SW(VT=0 VH=0.5 RON=1 ROFF=1meg)\n\
         .end\n",
    )
    .expect("ngspice switch parses");
    let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
    let out = solver.circuit().node("out").expect("out");
    let closed = |solver: &Solver| solver.node_voltage(out) < 0.05;

    run_to(&mut solver, 1e-6, 20);
    assert!(!closed(&solver), "open below the band");
    solver.set_voltage_source(0, 0.4);
    run_to(&mut solver, 1e-6, 20);
    assert!(
        !closed(&solver),
        "inside the band an open switch stays open"
    );
    solver.set_voltage_source(0, 0.6);
    run_to(&mut solver, 1e-6, 20);
    assert!(closed(&solver), "closes above VT + VH");
    solver.set_voltage_source(0, -0.4);
    run_to(&mut solver, 1e-6, 20);
    assert!(
        closed(&solver),
        "a closed switch stays closed down to VT − VH"
    );
    solver.set_voltage_source(0, -0.6);
    run_to(&mut solver, 1e-6, 20);
    assert!(!closed(&solver), "opens below VT − VH");
}

#[test]
fn ngspice_sw_model_uses_ngspice_defaults_and_checks_the_card_kind() {
    let circuit = parse_netlist("* defaults\nS1 a 0 c 0 SMOD\n.model SMOD SW\n.end\n")
        .expect("a bare SW card parses");
    let switch = &circuit.switches[0];
    assert_eq!((switch.ron, switch.roff), (1.0, 1e12));
    let control = switch.voltage.as_ref().expect("voltage controlled");
    assert_eq!((control.vt, control.vh), (0.0, 0.0));
    assert!(control.signed && !control.invert);

    let err = parse_netlist("* wrong card\nS1 a 0 c 0 DMOD\n.model DMOD D\n.end\n").unwrap_err();
    assert!(err.to_string().contains("is a S element"), "{err}");
    let err =
        parse_netlist("* bad card\nS1 a 0 c 0 SMOD\n.model SMOD SW(VH=-1)\n.end\n").unwrap_err();
    assert!(matches!(&err, AnalogError::Unsupported { .. }), "{err}");
    assert!(err.to_string().contains("needs ngspice"), "{err}");
}

#[test]
fn ngspice_sw_model_holds_its_state_exactly_on_the_band_edges() {
    // ngspice compares strictly: V == VT + VH does not close an open switch
    // and V == VT − VH does not open a closed one.
    let circuit = parse_netlist(
        "* band edges\n\
         Vctrl ctrl 0 dc 1.5\n\
         Vdd vdd 0 dc 5\n\
         Rload vdd out 1k\n\
         S1 out 0 ctrl 0 SMOD\n\
         .model SMOD SW(VT=1 VH=0.5 RON=1 ROFF=1meg)\n\
         .end\n",
    )
    .expect("netlist parses");
    let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
    let out = solver.circuit().node("out").expect("out");
    let closed = |solver: &Solver| solver.node_voltage(out) < 0.05;

    run_to(&mut solver, 1e-6, 20);
    assert!(
        !closed(&solver),
        "exactly VT + VH leaves an open switch open"
    );
    solver.set_voltage_source(0, 1.6);
    run_to(&mut solver, 1e-6, 20);
    assert!(closed(&solver), "closes above VT + VH");
    solver.set_voltage_source(0, 0.5);
    run_to(&mut solver, 1e-6, 20);
    assert!(
        closed(&solver),
        "exactly VT − VH leaves a closed switch closed"
    );
    solver.set_voltage_source(0, 0.4);
    run_to(&mut solver, 1e-6, 20);
    assert!(!closed(&solver), "opens below VT − VH");
}

#[test]
fn ngspice_sw_default_card_stays_open_at_zero_control() {
    // VT = VH = 0 puts both edges at 0 V. A 0 V control must leave the switch
    // open on every step, not toggle it.
    let circuit = parse_netlist(
        "* default card at 0 V\n\
         Vctrl ctrl 0 dc 0\n\
         Vdd vdd 0 dc 5\n\
         Rload vdd out 1k\n\
         S1 out 0 ctrl 0 SMOD\n\
         .model SMOD SW\n\
         .end\n",
    )
    .expect("netlist parses");
    let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
    let out = solver.circuit().node("out").expect("out");
    for step in 0..20 {
        solver.advance(1e-6).expect("step solves");
        assert!(
            solver.node_voltage(out) > 4.9,
            "step {step}: {} V",
            solver.node_voltage(out)
        );
    }
}

#[test]
fn ngspice_sw_model_rejects_an_unknown_parameter() {
    let err = parse_netlist("* typo\nS1 a 0 c 0 SMOD\n.model SMOD SW(VTH=2.5 RON=1)\n.end\n")
        .unwrap_err();
    assert!(err.to_string().contains("`VTH` is not one of"), "{err}");
}

#[test]
fn ngspice_sw_initial_state_keyword_seeds_the_operating_point() {
    // ngspice `ON` starts the operating point closed; inside the `VT ± VH`
    // band the switch keeps that state, outside it the control decides.
    // `OFF` is the default. The control 1 V sits inside [0.5, 1.5].
    let deck = |state: &str, control: f64| {
        format!(
            "* initial state\n\
             Vctrl ctrl 0 dc {control}\n\
             Vdd vdd 0 dc 5\n\
             Rload vdd out 1k\n\
             S1 out 0 ctrl 0 SMOD {state}\n\
             .model SMOD SW(VT=1 VH=0.5 RON=1 ROFF=1meg)\n\
             .end\n"
        )
    };
    let out_after = |state: &str, control: f64| {
        let circuit = parse_netlist(&deck(state, control)).expect("netlist parses");
        let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
        let out = solver.circuit().node("out").expect("out");
        let at_op = solver.node_voltage(out);
        run_to(&mut solver, 1e-6, 20);
        assert_eq!(at_op, solver.node_voltage(out), "{state} at {control} V");
        at_op
    };
    assert!(
        out_after("ON", 1.0) < 0.05,
        "ON holds closed inside the band"
    );
    assert!(
        out_after("on", 1.0) < 0.05,
        "the keyword is case-insensitive"
    );
    assert!(
        out_after("OFF", 1.0) > 4.9,
        "OFF holds open inside the band"
    );
    assert!(out_after("", 1.0) > 4.9, "the default is OFF");
    assert!(out_after("ON", 0.2) > 4.9, "ON opens below VT − VH");
    assert!(out_after("OFF", 1.8) < 0.05, "OFF closes above VT + VH");

    let circuit = parse_netlist(&deck("ON", 1.0)).expect("netlist parses");
    let control = circuit.switches[0].voltage.as_ref().expect("voltage");
    assert!(control.initially_closed);
}

#[test]
fn ngspice_sw_on_at_lower_edge_stays_closed_in_op_and_transient() {
    let circuit = parse_netlist(
        "* ON at lower hysteresis edge\n\
         Vctrl ctrl 0 dc 0.5\n\
         Vdd vdd 0 dc 5\n\
         Rload vdd out 1k\n\
         S1 out 0 ctrl 0 SMOD ON\n\
         .model SMOD SW(VT=1 VH=0.5 RON=1 ROFF=1meg)\n\
         .end\n",
    )
    .expect("netlist parses");
    let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
    let out = solver.circuit().node("out").expect("out");
    assert!(
        solver.node_voltage(out) < 0.05,
        "ON remains closed at exactly VT − VH in the operating point: {} V",
        solver.node_voltage(out)
    );

    run_to(&mut solver, 1e-6, 10);
    assert!(
        solver.node_voltage(out) < 0.05,
        "ON remains closed at exactly VT − VH in transient: {} V",
        solver.node_voltage(out)
    );

    solver.set_voltage_source(0, 1.6);
    run_to(&mut solver, 1e-6, 10);
    assert!(
        solver.node_voltage(out) < 0.05,
        "control above VT + VH closes the switch: {} V",
        solver.node_voltage(out)
    );

    solver.set_voltage_source(0, 0.5);
    run_to(&mut solver, 1e-6, 10);
    assert!(
        solver.node_voltage(out) < 0.05,
        "transient hysteresis holds closed at exactly VT − VH: {} V",
        solver.node_voltage(out)
    );
}

#[test]
fn ngspice_sw_lower_edge_is_exactly_vt_minus_vh() {
    // 0.5 + 0.3 − 2·0.3 is 0.20000000000000007, not 0.2. The lower edge
    // must be ngspice's own VT − VH, or an ON switch held at exactly 0.2 V
    // opens where ngspice keeps it closed.
    let circuit = parse_netlist(
        "* inexact band arithmetic\n\
         Vctrl ctrl 0 dc 0.2\n\
         Vdd vdd 0 dc 5\n\
         Rload vdd out 1k\n\
         S1 out 0 ctrl 0 SMOD ON\n\
         .model SMOD SW(VT=0.5 VH=0.3 RON=1 ROFF=1meg)\n\
         .end\n",
    )
    .expect("netlist parses");
    let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
    let ctrl = solver.circuit().node("ctrl").expect("ctrl");
    let out = solver.circuit().node("out").expect("out");
    assert_eq!(
        solver.node_voltage(ctrl),
        0.2,
        "the control sits on the edge"
    );
    assert!(
        solver.node_voltage(out) < 0.05,
        "ON stays closed at exactly VT − VH = 0.2 V: {} V",
        solver.node_voltage(out)
    );
    run_to(&mut solver, 1e-6, 10);
    assert!(
        solver.node_voltage(out) < 0.05,
        "the transient keeps it closed at 0.2 V: {} V",
        solver.node_voltage(out)
    );
    solver.set_voltage_source(0, 0.19);
    run_to(&mut solver, 1e-6, 10);
    assert!(
        solver.node_voltage(out) > 4.9,
        "below VT − VH the switch opens: {} V",
        solver.node_voltage(out)
    );
}

#[test]
fn ngspice_sw_rejects_an_unknown_trailing_keyword() {
    let err = parse_netlist("* keyword\nS1 a 0 c 0 SMOD MAYBE\n.model SMOD SW(VT=1)\n.end\n")
        .unwrap_err();
    assert!(!matches!(&err, AnalogError::Unsupported { .. }), "{err}");
    assert!(err.to_string().contains("must be ON or OFF"), "{err}");
}

#[test]
fn ngspice_sw_on_that_opens_at_the_operating_point_stays_open_in_the_band() {
    // The switch pulls its own control down: closed, the control is 0.1 V,
    // below the band, so the OP opens it; open, the control is 1 V, inside
    // [0.5, 1.5], so it stays open. ngspice-42 `op` agrees (open).
    let circuit = parse_netlist(
        "* ON with feedback\n\
         Vdd vdd 0 dc 2\n\
         Rtop vdd out 1k\n\
         S1 out 0 ctrl 0 SMOD ON\n\
         Rbot ctrl 0 1meg\n\
         E1 ctrl 0 out 0 0.5\n\
         .model SMOD SW(VT=1 VH=0.5 RON=100 ROFF=1g)\n\
         .end\n",
    )
    .expect("netlist parses");
    let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
    let out = solver.circuit().node("out").expect("out");
    assert!(
        solver.node_voltage(out) > 1.99,
        "{} V",
        solver.node_voltage(out)
    );
    run_to(&mut solver, 1e-6, 20);
    assert!(
        solver.node_voltage(out) > 1.99,
        "{} V",
        solver.node_voltage(out)
    );
}

/// Pull-up on `out`, shunted by an ngspice `W` switch that senses the branch
/// current of `Vsense`, driven by `Vctrl` through 1 kΩ (1 mA per volt).
const CURRENT_SWITCH: &str = "* csw\n\
     Vctrl ctrl 0 dc 0\n\
     Rs ctrl s 1k\n\
     Vsense s 0 dc 0\n\
     Vdd vdd 0 dc 5\n\
     R1 vdd out 1k\n\
     W1 out 0 Vsense WMOD\n\
     C1 out 0 1p\n\
     .model WMOD CSW(IT=1m IH=0.5m RON=1 ROFF=1meg)\n\
     .end\n";

#[test]
fn ngspice_csw_switch_follows_the_signed_branch_current_band() {
    let circuit = parse_netlist(CURRENT_SWITCH).expect("netlist parses");
    let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
    let out = solver.circuit().node("out").expect("out");
    let level = |solver: &mut Solver, volts: f64| {
        solver.set_voltage_source(0, volts);
        run_to(solver, 1e-6, 20);
        solver.node_voltage(out)
    };
    // The band is [IT − IH, IT + IH] = [0.5 mA, 1.5 mA].
    assert!(level(&mut solver, 1.4) > 4.9, "below IT + IH stays open");
    assert!(level(&mut solver, 1.6) < 0.05, "above IT + IH closes");
    assert!(
        level(&mut solver, 0.6) < 0.05,
        "inside the band holds closed"
    );
    assert!(level(&mut solver, 0.4) > 4.9, "below IT − IH opens");
    // A magnitude rule would close at −2 mA; the signed rule stays open.
    assert!(
        level(&mut solver, -2.0) > 4.9,
        "negative current keeps it open"
    );
}

#[test]
fn ngspice_csw_switch_on_keyword_seeds_the_operating_point() {
    let deck = CURRENT_SWITCH
        .replace("Vctrl ctrl 0 dc 0", "Vctrl ctrl 0 dc 1")
        .replace("W1 out 0 Vsense WMOD", "W1 out 0 Vsense WMOD ON");
    let circuit = parse_netlist(&deck).expect("netlist parses");
    let solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver");
    let out = solver.circuit().node("out").expect("out");
    // 1 mA is inside the band, so only `ON` closes the switch.
    assert!(
        solver.node_voltage(out) < 0.05,
        "{} V",
        solver.node_voltage(out)
    );

    let off = parse_netlist(&CURRENT_SWITCH.replace("Vctrl ctrl 0 dc 0", "Vctrl ctrl 0 dc 1"))
        .expect("parses");
    let solver = Solver::new(off, Integration::BackwardEuler).expect("solver");
    assert!(
        solver.node_voltage(out) > 4.9,
        "{} V",
        solver.node_voltage(out)
    );
}

#[test]
fn ngspice_csw_switch_rejects_bad_controls_and_models() {
    let err = parse_netlist("* w\nW1 a 0 Rx WMOD\nRx a 0 1k\n.model WMOD CSW(IT=1m)\n.end\n")
        .unwrap_err();
    assert!(
        err.to_string().contains("carries no branch current"),
        "{err}"
    );

    let err =
        parse_netlist("* w\nV1 a 0 1\nW1 a 0 V1 WMOD\n.model WMOD SW(VT=1)\n.end\n").unwrap_err();
    assert!(err.to_string().contains("is a SW model"), "{err}");

    let err =
        parse_netlist("* w\nV1 a 0 1\nW1 a 0 V1 WMOD\n.model WMOD CSW(VT=1)\n.end\n").unwrap_err();
    assert!(
        err.to_string().contains("`IT`, `IH`, `RON`, `ROFF`"),
        "{err}"
    );

    let err = parse_netlist("* w\nV1 a 0 1\nW1 a 0 V1 WMOD MAYBE\n.model WMOD CSW(IT=1)\n.end\n")
        .unwrap_err();
    assert!(err.to_string().contains("must be ON or OFF"), "{err}");
}
