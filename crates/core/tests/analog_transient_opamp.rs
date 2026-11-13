//! Integration of explicit reactive state with the optional OPAMP envelope.
use labwired_core::analog::{
    parse_netlist, AnalogError, AnalogInitialization, Integration, Solver, MAX_UNKNOWNS,
};
use std::collections::BTreeMap;

fn initialized(
    deck: &str,
    stored_voltage: f64,
    compensation_voltage: f64,
) -> Result<Solver, AnalogError> {
    let circuit = parse_netlist(deck)?;
    let initial = AnalogInitialization::TransientState {
        capacitor_voltages_v: circuit
            .capacitors
            .iter()
            .map(|c| {
                (
                    c.name.clone(),
                    if c.name == "Cstored" {
                        stored_voltage
                    } else if c.name == "Xamp#cg" {
                        compensation_voltage
                    } else {
                        0.0
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
        inductor_currents_a: circuit
            .inductors
            .iter()
            .map(|l| (l.name.clone(), 0.0))
            .collect(),
    };
    Solver::new_with_initialization(circuit, Integration::BackwardEuler, MAX_UNKNOWNS, &initial)
}
fn volts(s: &Solver, name: &str) -> f64 {
    s.node_voltage(s.circuit().node(name).unwrap())
}
fn near(a: f64, b: f64) {
    assert!(a.is_finite() && (a - b).abs() < 1e-10, "{a:e} != {b:e}");
}

#[test]
fn off_amplifier_does_not_invalidate_an_open_inductive_state_or_erase_charge() {
    for extra in ["", "IOLIM=.02 SR=150meg"] {
        let deck = format!(".model amp OPAMP(AOL=100k GBW=1meg ROUT=20.2 DROP_HI=.025 DROP_LO=.025 VOS=.01 IQ=.005 IBIAS=1u VMIN=2.5 VMAX=5.5 {extra})\nVrail rail 0 0\nVin input 0 1\nXamp input 0 rail 0 out amp\nLopen out winding 1m\nRwinding winding stored 10\nCstored stored 0 1u\n");
        let mut s = initialized(&deck, 0.2, 0.0).unwrap();
        for _ in 0..2 {
            near(volts(&s, "out"), 0.2);
            near(volts(&s, "stored"), 0.2);
            near(
                s.branch_current(s.circuit().branch_index("Lopen").unwrap()),
                0.0,
            );
            near(
                s.branch_current(s.circuit().branch_index("Vrail").unwrap()),
                0.0,
            );
            for diode in 0..s.circuit().diodes.len() {
                assert_eq!(s.diode_current(diode), 0.0);
            }
            s.advance(1e-7).unwrap();
        }
    }
}

#[test]
fn invalid_initial_supply_cannot_publish_a_disabled_amplifier_solution() {
    for rail in [-0.1, 5.6] {
        let deck = format!(".model amp OPAMP(AOL=100k GBW=1meg ROUT=20.2 DROP_HI=.025 DROP_LO=.025 VMIN=2.5 VMAX=5.5)\nVrail rail 0 {rail}\nVin input 0 1\nXamp input 0 rail 0 out amp\nRload out 0 1k\n");
        let error = match initialized(&deck, 0.0, 0.0) {
            Ok(_) => panic!("invalid supply {rail} V accepted at t=0"),
            Err(error) => error,
        };
        assert!(
            matches!(&error, AnalogError::Config(message) if message.contains("solved supply span")),
            "{error}"
        );
    }
}

#[test]
fn powered_amplifier_uses_the_requested_compensation_state_before_the_first_step() {
    let deck = ".model amp OPAMP(AOL=100k GBW=1meg ROUT=20.2 DROP_HI=.025 DROP_LO=.025 VMIN=2.5 VMAX=5.5)\nVrail rail 0 3.3\nVin input 0 1\nXamp input out rail 0 out amp\nRload out 0 1k\n";
    let mut s = initialized(deck, 0.0, 0.5).unwrap();
    near(s.time(), 0.0);
    let compensation = s
        .circuit()
        .capacitors
        .iter()
        .position(|c| c.name == "Xamp#cg")
        .unwrap();
    near(s.capacitor_voltage(compensation), 0.5);
    near(volts(&s, "out"), 0.5 * 1000.0 / (1000.0 + 20.2));
    let initial = serde_json::to_value(s.initialization_status()).unwrap();
    assert_eq!(initial["mode"], "transient_state");
    s.advance(1e-7).unwrap();
    assert!(volts(&s, "out") > 0.0 && volts(&s, "out") < 1.01);
}
