//! Independent witnesses for startup-state and OP active-set regressions.
mod controlled_buck_support;
use controlled_buck_support::{config, deck};
use labwired_core::analog::{AnalogCosimAdapter, AnalogError, AnalogInitialization};
use labwired_core::cosim::{CosimAdapter, CosimSignals, CosimStep};
fn unloaded(vin: f64, avdd: f64) -> String {
    deck(vin, avdd, 11.0)
        .lines()
        .filter(|line| !line.starts_with("Rload "))
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn live_unloaded_buck_can_find_a_qualified_dc_operating_point() {
    let adapter = AnalogCosimAdapter::from_netlist(&unloaded(3.3, 3.3), &config()).unwrap();
    let feedback = adapter
        .solver()
        .node_voltage(adapter.solver().circuit().node("out").unwrap());
    assert!((feedback - 1.1).abs() < 0.002, "{feedback}");
    assert_eq!(
        serde_json::to_value(adapter.initialization_status()).unwrap()["mode"],
        "operating_point"
    );
}
#[test]
fn unavailable_supply_keeps_unloaded_dc_state_undefined() {
    for (vin, avdd) in [(0.0, 0.0), (3.3, 0.0), (0.0, 3.3)] {
        assert!(matches!(
            AnalogCosimAdapter::from_netlist(&unloaded(vin, avdd), &config()),
            Err(AnalogError::Singular { .. })
        ));
    }
}
#[test]
fn explicit_unpowered_buck_state_has_zero_charge_and_no_input_energy() {
    let mut cfg = config();
    cfg.initialization = serde_json::from_value::<AnalogInitialization>(serde_json::json!({
        "mode":"transient_state", "capacitor_voltages_v":{"Cexternal":0.0}, "inductor_currents_a":{"Lexternal":0.0}
    })).unwrap();
    let mut adapter = AnalogCosimAdapter::from_netlist(&unloaded(0.0, 0.0), &cfg).unwrap();
    for time in [0, 100] {
        let result = adapter
            .step(CosimStep {
                time_ns: time,
                dt_ns: if time == 0 { 0 } else { 100 },
                inputs: CosimSignals::new(),
            })
            .unwrap();
        assert_eq!(
            result.outputs["reg.output_enabled"],
            labwired_core::cosim::CosimSignalValue::Bool(false)
        );
        assert_eq!(
            result.outputs["reg.input_current_a"],
            labwired_core::cosim::CosimSignalValue::F64(0.0)
        );
        assert!(adapter.solver().capacitor_voltage(0).abs() < 1e-12);
        assert!(adapter.solver().inductor_current(0).abs() < 1e-12);
    }
}

#[test]
fn state_names_cannot_alias_across_capacitor_and_inductor_maps() {
    let error = serde_json::from_value::<AnalogInitialization>(serde_json::json!({
        "mode":"transient_state", "capacitor_voltages_v":{"C1":0.0},
        "inductor_currents_a":{"c1":0.0}
    }))
    .unwrap_err();
    assert!(
        error.to_string().contains("duplicate case-insensitive"),
        "{error}"
    );
}

#[test]
fn reversed_mos_coordinate_roundoff_is_not_a_permanent_voltage_limit() {
    use labwired_core::analog::{parse_netlist, Integration, Solver};
    for (model, sign) in [
        ("NMOS(VTO=.8 KP=1 IS=1e-15)", 1.0),
        ("PMOS(VTO=-.8 KP=1 IS=1e-15)", -1.0),
    ] {
        for drain in [-0.2, -0.3, 0.2] {
            for initial in [AnalogInitialization::OperatingPoint,
                serde_json::from_value(serde_json::json!({"mode":"transient_state", "capacitor_voltages_v":{},"inductor_currents_a":{}})).unwrap()] {
                let text = format!(".model channel {model}\nVg gate 0 DC {}\nVd out 0 DC {}\nM1 out gate 0 0 channel\n", sign*0.1, sign*drain);
                let mut solver = Solver::new_with_initialization(parse_netlist(&text).unwrap(), Integration::BackwardEuler, 8, &initial).unwrap();
                let branch = solver.circuit().branch_index("Vd").unwrap();
                let before = solver.branch_current(branch);
                assert!(before.is_finite());
                solver.advance(1e-6).unwrap();
                assert!((solver.node_voltage(solver.circuit().node("gate").unwrap())-sign*0.1).abs()<1e-14);
                assert!((solver.node_voltage(solver.circuit().node("out").unwrap())-sign*drain).abs()<1e-14);
                assert!((solver.branch_current(branch)-before).abs()<1e-13);
            }
        }
    }
}

fn cascaded_deck(powered: bool) -> String {
    format!("Vbat battery 0 DC {}\n.model upstream BUCK(VREF=5 VDO=0 DMAX=.96 ILIM=5.1 IQ=29u ROUT=.085 EFF=.9 AOL=10000)\nXup battery usw 0 in upstream\nLup usw ulr 3.3u\nRup ulr in .115\nCup in 0 10u\n.model av LDO(VOUT=3.3 VDO=.095 ILIM=.9 IQ=30u ROUT=.0726)\nXav in avdd 0 av\nCav avdd 0 10u\n.model buck BUCK(VREF=1.1 VDO=0 DMAX=1 ILIM=.24 IQ=0 ROUT=.01 EFF=1 AOL=100000)\nXbuck in sw 0 out buck\nLexternal sw lr 3.3u\nRdcr lr out .115\nCexternal out 0 10u\n", if powered {16.8} else {0.0})
}
#[test]
fn cascaded_solved_supplies_can_qualify_an_unloaded_stage_without_a_seed_override() {
    let mut cfg = config();
    cfg.sources.clear();
    let adapter = AnalogCosimAdapter::from_netlist(&cascaded_deck(true), &cfg).unwrap();
    let observation = adapter.solver().buck_observation(0, 0);
    assert!(observation.qualified && observation.output_enabled);
    assert_eq!(observation.applied_revision, 0);
    assert!((observation.feedback_voltage_v - 1.1).abs() < 0.002);
}
#[test]
fn cascaded_unpowered_and_requested_hiz_states_do_not_invent_a_dc_output() {
    let mut cfg = config();
    cfg.sources.clear();
    assert!(matches!(
        AnalogCosimAdapter::from_netlist(&cascaded_deck(false), &cfg),
        Err(AnalogError::Singular { .. })
    ));
    cfg.regulators.get_mut("reg").unwrap().initial.enabled = false;
    assert!(matches!(
        AnalogCosimAdapter::from_netlist(&cascaded_deck(true), &cfg),
        Err(AnalogError::Singular { .. })
    ));
}

#[test]
fn explicit_initial_state_convergence_error_names_its_actual_phase() {
    let error = AnalogError::InitialStateNoConvergence {
        time: 0.0,
        iterations: 100,
        unknown: "initial_constraint(C1)".into(),
        delta: 3.5e-5,
    };
    let text = error.to_string();
    assert!(text.contains("transient_state"));
    assert!(text.contains("100 iterations"));
    assert!(text.contains("initial_constraint(C1)"));
    assert!(text.contains("3.5e-5"));
    assert!(!text.contains("DC operating point"));
    assert!(!text.contains("substeps"));
}
#[test]
fn an_oscillating_voltage_contact_cannot_publish_the_eighth_stale_solution() {
    use labwired_core::analog::{parse_netlist, Integration, Solver};
    let deck = "V1 vin 0 1\nR1 vin out 1k\nS1 out 0 out 0 vt=.5 vh=0 ron=1 roff=1e12\n";
    for initial in [AnalogInitialization::OperatingPoint,serde_json::from_value(serde_json::json!({"mode":"transient_state","capacitor_voltages_v":{},"inductor_currents_a":{}})).unwrap()] {
        let error=Solver::new_with_initialization(parse_netlist(deck).unwrap(),Integration::BackwardEuler,8,&initial).unwrap_err();
        assert!(matches!(&error,AnalogError::Config(message) if message.contains("switch active set") && message.contains("8 passes")),"{error}");
    }
}
#[test]
fn a_stable_voltage_contact_retains_its_solved_output() {
    use labwired_core::analog::{parse_netlist, Integration, Solver};
    for control in [0.0, 1.0] {
        let deck=format!("V1 vin 0 1\nVctrl ctl 0 {control}\nR1 vin out 1k\nS1 out 0 ctl 0 vt=.5 vh=0 ron=1 roff=1e12\n");
        let solver =
            Solver::new(parse_netlist(&deck).unwrap(), Integration::BackwardEuler).unwrap();
        let out = solver.node_voltage(solver.circuit().node("out").unwrap());
        let resistance = if control == 0.0 { 1e12 } else { 1.0 };
        assert!((out - resistance / (1000.0 + resistance)).abs() < 1e-12);
    }
}
