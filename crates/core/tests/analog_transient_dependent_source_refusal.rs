//! Dependent voltage outputs do not receive the stationary independent-source
//! exception. Their controls can change at t=0 with no source waveform edge.
use labwired_core::analog::{
    parse_netlist, AnalogError, AnalogInitialization, Integration, Solver,
};
use serde_json::{json, Value};

fn refuses(deck: &str, capacitors: Value) {
    let initial: AnalogInitialization = serde_json::from_value(json!({
        "mode":"transient_state","capacitor_voltages_v":capacitors,
        "inductor_currents_a":{}
    }))
    .unwrap();
    let error = match Solver::new_with_initialization(
        parse_netlist(deck).unwrap(),
        Integration::BackwardEuler,
        20,
        &initial,
    ) {
        Ok(_) => panic!("unresolved dependent-source initial charge current must fail admission"),
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
        other => panic!("source/charge cycle must be classified before numerical solve: {other}"),
    }
}

#[test]
fn vcvs_mixed_cycles_refuse_for_both_explicit_capacitor_orders() {
    for reverse in [false, true] {
        let cards = if reverse {
            "Cshunt out 0 1u\nCcouple vin out 1u"
        } else {
            "Ccouple vin out 1u\nCshunt out 0 1u"
        };
        refuses(
            &format!("Vctrl ctrl 0 DC 1\nEdrive vin 0 ctrl 0 1\n{cards}\nRload out 0 1k\n"),
            json!({"Ccouple":0.5,"Cshunt":0.5}),
        );
    }
}

#[test]
fn vcvs_mixed_cycle_with_implicit_diode_charge_is_also_refused() {
    refuses(".model rev D(IS=1e-15 N=1 CJO=1u VJ=1 M=0 TT=0 RS=0)\nVctrl ctrl 0 DC 1\nEdrive vin 0 ctrl 0 1\nCcouple vin out 1u\nD1 0 out rev\nRload out 0 1k\n",
        json!({"Ccouple":0.5}));
}

#[test]
fn a_vcvs_direct_output_capacitor_is_not_assumed_to_have_fixed_voltage() {
    refuses("V1 power 0 DC 1\nRcontrol power ctrl 1k\nCcontrol ctrl 0 1u\nEdrive out 0 ctrl 0 1\nCout out 0 1u\n",
        json!({"Ccontrol":0.0,"Cout":0.0}));
}

#[test]
fn a_ccvs_output_does_not_receive_the_independent_source_exception() {
    let prefix =
        "V1 power 0 DC 1\nRsense power sense 1k\nVsense sense 0 DC 0\nHdrive vin 0 Vsense 1000\n";
    refuses(
        &format!("{prefix}Cdirect vin 0 1u\n"),
        json!({"Cdirect":1.0}),
    );
    refuses(
        &format!("{prefix}Ccouple vin out 1u\nCshunt out 0 1u\nRload out 0 1k\n"),
        json!({"Ccouple":0.5,"Cshunt":0.5}),
    );
}
