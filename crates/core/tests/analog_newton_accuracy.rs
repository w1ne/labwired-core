//! Independent passive-network witnesses around a high-gain buck startup.
use labwired_core::analog::{
    mna::{NEWTON_RELTOL, NEWTON_VNTOL},
    parse_netlist, AnalogInitialization, Integration, Solver,
};
use std::collections::BTreeMap;

const DECK: &str = "Vbat in 0 dc 16.8\n\
    .model buck BUCK(VREF=0.8 VDO=0 DMAX=0.98 ILIM=1.1 IQ=80u ROUT=0.05 EFF=0.9 AOL=10000)\n\
    Xbuck in sw 0 fb buck\n\
    Rdcr sw winding 0.585\n\
    Lwinding winding out 4.7u\n\
    Cout out 0 22u\n\
    Rtop out fb 115k\n\
    Rbottom fb 0 10k\n";

fn voltage(solver: &Solver, name: &str) -> f64 {
    solver.node_voltage(solver.circuit().node(name).unwrap())
}

#[test]
fn high_gain_buck_startup_preserves_passive_divider_and_winding_kcl() {
    let initial = AnalogInitialization::TransientState {
        capacitor_voltages_v: BTreeMap::from([("Cout".into(), 0.0)]),
        inductor_currents_a: BTreeMap::from([("Lwinding".into(), 0.0)]),
    };
    let mut solver = Solver::new_with_initialization(
        parse_netlist(DECK).unwrap(),
        Integration::BackwardEuler,
        128,
        &initial,
    )
    .unwrap();
    let winding = solver.circuit().branch_index("Lwinding").unwrap();

    // The capacitor explicitly starts at zero. The buck senses the divider
    // without drawing input current, so its midpoint is independently zero.
    assert_eq!(voltage(&solver, "out"), 0.0);
    assert!(voltage(&solver, "fb").abs() <= 1e-12);
    assert_eq!(solver.branch_current(winding), 0.0);

    for step in 1..=100 {
        solver.advance(1e-8).unwrap();
        let out = voltage(&solver, "out");
        let feedback = voltage(&solver, "fb");
        let current = solver.branch_current(winding);
        // Propagate the declared node-voltage accuracy through the passive
        // divider. The exact zero-state check above keeps its absolute floor.
        let feedback_tolerance = NEWTON_VNTOL + NEWTON_RELTOL * feedback.abs();
        let output_tolerance = NEWTON_VNTOL + NEWTON_RELTOL * out.abs();
        assert!((feedback - out / 12.5).abs() <= feedback_tolerance + output_tolerance / 12.5);
        assert!(((out - feedback) / 115e3 - feedback / 10e3).abs() <= 1e-15);
        assert!(
            ((voltage(&solver, "sw") - voltage(&solver, "winding")) / 0.585 - current).abs()
                <= 1e-11
        );
        assert!((0.0..=1.11).contains(&current));
        // Even an immediate current-limit plateau cannot charge 22 uF faster
        // than Q = I*t. The actual startup ramp and divider consume less charge.
        assert!(out >= 0.0 && out <= 1.11 * f64::from(step) * 1e-8 / 22e-6);
    }
    assert!(voltage(&solver, "out") > 0.03);
}
