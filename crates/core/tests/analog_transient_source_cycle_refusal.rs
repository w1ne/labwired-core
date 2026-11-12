//! Admission boundaries for t=0 currents that the constrained state solver
//! does not resolve. Analytic current witnesses remain in separate files.
use labwired_core::analog::{
    parse_netlist, AnalogError, AnalogInitialization, Integration, Solver,
};
use serde_json::{json, Value};

fn initial(caps: Value) -> AnalogInitialization {
    serde_json::from_value(json!({"mode":"transient_state",
        "capacitor_voltages_v":caps,"inductor_currents_a":{}}))
    .unwrap()
}
fn refuses_mixed_cycle(deck: &str, caps: Value) {
    let error = match Solver::new_with_initialization(
        parse_netlist(deck).expect("valid physical circuit"),
        Integration::BackwardEuler,
        16,
        &initial(caps),
    ) {
        Ok(_) => panic!("unsupported initial source-current partition must fail admission"),
        Err(error) => error,
    };
    match error {
        AnalogError::Config(message) => {
            let lower = message.to_ascii_lowercase();
            assert!(
                lower.contains("cycle") && lower.contains("source"),
                "{message}"
            );
        }
        other => panic!(
            "must identify the unsupported topology before a singular/numerical solve: {other}"
        ),
    }
}

#[test]
fn explicit_mixed_cycles_refuse_in_both_card_orders_and_capacitance_ratios() {
    for coupling in ["1u", "2u"] {
        for reverse in [false, true] {
            let a = format!("Ccouple vin out {coupling}");
            let b = "Cshunt out 0 1u";
            let cards = if reverse {
                format!("{b}\n{a}")
            } else {
                format!("{a}\n{b}")
            };
            refuses_mixed_cycle(
                &format!("V1 vin 0 DC 1\n{cards}\nR1 out 0 1k\n"),
                json!({"Ccouple":0.5,"Cshunt":0.5}),
            );
        }
    }
}

#[test]
fn implicit_reverse_junction_charge_cannot_evade_the_cycle_guard() {
    for reverse in [false, true] {
        let a = "Ccouple vin out 1u";
        let b = "D1 0 out rev";
        let cards = if reverse {
            format!("{b}\n{a}")
        } else {
            format!("{a}\n{b}")
        };
        refuses_mixed_cycle(&format!(".model rev D(IS=1e-15 N=1 CJO=1u VJ=1 M=0 TT=0 RS=0)\nV1 vin 0 DC 1\n{cards}\nR1 out 0 1k\n"), json!({"Ccouple":0.5}));
    }
}

#[test]
fn a_directly_source_clamped_junction_remains_admitted_without_a_charge_pulse() {
    let deck =
        ".model rev D(IS=1e-15 N=1 CJO=1u VJ=1 M=0 TT=0 RS=0)\nV1 out 0 DC -0.5\nD1 out 0 rev\n";
    let mut solver = Solver::new_with_initialization(
        parse_netlist(deck).unwrap(),
        Integration::BackwardEuler,
        4,
        &initial(json!({})),
    )
    .unwrap();
    let branch = solver.circuit().branch_index("V1").unwrap();
    assert_eq!(solver.time(), 0.0);
    assert!(solver.branch_current(branch).abs() < 1e-11);
    solver.advance(1e-6).unwrap();
    assert!(
        solver.branch_current(branch).abs() < 1e-11,
        "constant junction bias must not create a first-step displacement pulse"
    );
}
