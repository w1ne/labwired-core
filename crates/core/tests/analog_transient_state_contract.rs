//! Independent contracts for explicit reactive-state initialization.
//! Drafted outside the integration tree; no build or pass is claimed here.
use labwired_core::analog::{
    parse_netlist, AnalogError, AnalogInitialization, Integration, Solver, HARD_MAX_UNKNOWNS,
    MAX_UNKNOWNS,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn state_value(caps: &[(&str, f64)], inductors: &[(&str, f64)]) -> Value {
    json!({
        "mode": "transient_state",
        "capacitor_voltages_v": caps.iter().copied().collect::<BTreeMap<_, _>>(),
        "inductor_currents_a": inductors.iter().copied().collect::<BTreeMap<_, _>>(),
    })
}
fn state(caps: &[(&str, f64)], inductors: &[(&str, f64)]) -> AnalogInitialization {
    serde_json::from_value(state_value(caps, inductors)).expect("valid explicit state shape")
}
fn initialize(
    deck: &str,
    initial: &AnalogInitialization,
    limit: usize,
) -> Result<Solver, AnalogError> {
    Solver::new_with_initialization(
        parse_netlist(deck).expect("valid circuit fixture"),
        Integration::BackwardEuler,
        limit,
        initial,
    )
}
fn status(solver: &Solver) -> Value {
    serde_json::to_value(solver.initialization_status())
        .expect("serializable initialization status")
}
fn node(solver: &Solver, name: &str) -> f64 {
    solver.node_voltage(solver.circuit().node(name).expect("declared node"))
}
fn current(solver: &Solver, name: &str) -> f64 {
    solver.branch_current(
        solver
            .circuit()
            .branch_index(name)
            .expect("original branch"),
    )
}
fn near(actual: f64, expected: f64) {
    assert!(actual.is_finite());
    assert!(
        (actual - expected).abs() <= 1e-11 * expected.abs().max(1.0),
        "actual={actual:e}, expected={expected:e}"
    );
}
fn refused(deck: &str, value: Value) {
    if let Ok(initial) = serde_json::from_value::<AnalogInitialization>(value) {
        assert!(initialize(deck, &initial, MAX_UNKNOWNS).is_err());
    }
}
const RC: &str = "V1 vin 0 dc 1\nR1 vin out 1k\nC1 out 0 1u\n";
const OPEN_LC: &str = "C1 out 0 10u\nR1 out winding 0.1\nL1 winding sw 3u\n";

#[test]
fn default_operating_point_keeps_its_floating_node_refusal() {
    assert!(matches!(
        Solver::new(
            parse_netlist("C1 out 0 1u\n").unwrap(),
            Integration::BackwardEuler
        ),
        Err(AnalogError::Singular { .. })
    ));
    let solver = Solver::new(parse_netlist(RC).unwrap(), Integration::BackwardEuler).unwrap();
    near(node(&solver, "out"), 1.0);
    assert_eq!(status(&solver)["mode"], "operating_point");
}

#[test]
fn explicit_rc_state_is_not_a_precharged_operating_point_or_time_advance() {
    let mut solver = initialize(RC, &state(&[("C1", 0.0)], &[]), 8).unwrap();
    near(solver.time(), 0.0);
    near(node(&solver, "out"), 0.0);
    near(solver.capacitor_voltage(0), 0.0);
    near(current(&solver, "V1"), -0.001);
    assert_eq!(solver.circuit().voltage_sources.len(), 1);
    assert_eq!(status(&solver)["mode"], "transient_state");
    assert_eq!(status(&solver)["base_unknowns"], 3);
    assert_eq!(status(&solver)["initialization_unknowns"], 4);
    solver.advance(0.001).unwrap();
    near(node(&solver, "out"), 0.5);
    near(current(&solver, "V1"), -0.0005);
    assert_eq!(solver.circuit().voltage_sources.len(), 1);
}

#[test]
fn capacitor_energy_can_discharge_without_a_permanent_constraint_source() {
    for initial_voltage in [-2.0, 2.0] {
        let mut solver = initialize(
            "R1 out 0 1k\nC1 out 0 1u\n",
            &state(&[("C1", initial_voltage)], &[]),
            2,
        )
        .unwrap();
        near(node(&solver, "out"), initial_voltage);
        assert!(solver.circuit().voltage_sources.is_empty());
        assert_eq!(solver.circuit().branch_count(), 0);
        solver.advance(0.001).unwrap();
        near(node(&solver, "out"), initial_voltage / 2.0);
        let energy = 0.5e-6 * solver.capacitor_voltage(0).powi(2);
        assert!(energy < 0.5e-6 * initial_voltage.powi(2));
        assert!(solver.circuit().voltage_sources.is_empty());
    }
}

#[test]
fn inductor_state_preserves_original_branch_current_and_decay() {
    for initial_current in [-0.1, 0.1] {
        let mut solver = initialize(
            "R1 out 0 10\nL1 out 0 1m\n",
            &state(&[], &[("L1", initial_current)]),
            2,
        )
        .unwrap();
        near(current(&solver, "L1"), initial_current);
        near(solver.inductor_current(0), initial_current);
        near(node(&solver, "out"), -10.0 * initial_current);
        assert_eq!(solver.circuit().branch_count(), 1);
        assert_eq!(status(&solver)["base_unknowns"], 2);
        assert_eq!(status(&solver)["initialization_unknowns"], 2);
        solver.advance(0.0001).unwrap();
        near(current(&solver, "L1"), initial_current / 2.0);
        near(node(&solver, "out"), -5.0 * initial_current);
    }
}

#[test]
fn native_shaped_open_inductor_has_a_consistent_zero_current_start() {
    let mut solver = initialize(OPEN_LC, &state(&[("C1", 0.0)], &[("L1", 0.0)]), 8).unwrap();
    near(solver.time(), 0.0);
    near(node(&solver, "out"), 0.0);
    near(node(&solver, "winding"), 0.0);
    near(current(&solver, "L1"), 0.0);
    // The differentiated open-end KCL gives dI/dt=0, hence V(L)=0.
    near(node(&solver, "sw"), 0.0);
    assert!(solver.circuit().voltage_sources.is_empty());
    solver.advance(100e-9).unwrap();
    near(node(&solver, "sw"), 0.0);
    near(solver.capacitor_voltage(0), 0.0);
    near(current(&solver, "L1"), 0.0);
}

#[test]
fn charged_native_shaped_open_inductor_does_not_erase_stored_charge() {
    let mut solver = initialize(OPEN_LC, &state(&[("C1", 1.1)], &[("L1", 0.0)]), 8).unwrap();
    near(node(&solver, "out"), 1.1);
    near(node(&solver, "sw"), 1.1);
    solver.advance(100e-9).unwrap();
    near(solver.capacitor_voltage(0), 1.1);
    near(current(&solver, "L1"), 0.0);
}

#[test]
fn nonzero_open_inductor_current_is_inconsistent_with_kcl() {
    refused(OPEN_LC, state_value(&[("C1", 0.0)], &[("L1", 0.01)]));
}

#[test]
fn inductive_cutset_voltage_uses_inductance_and_preserves_currents() {
    let deck = "C1 left 0 10u\nC2 right 0 20u\nL1 left mid 1m\nL2 mid right 3m\n";
    let solver = initialize(
        deck,
        &state(&[("C1", 4.0), ("C2", 0.0)], &[("L1", 0.1), ("L2", 0.1)]),
        12,
    )
    .unwrap();
    near(node(&solver, "left"), 4.0);
    near(node(&solver, "right"), 0.0);
    near(node(&solver, "mid"), 3.0);
    near(current(&solver, "L1"), 0.1);
    near(current(&solver, "L2"), 0.1);
    near(
        (node(&solver, "left") - node(&solver, "mid")) / 0.001,
        (node(&solver, "mid") - node(&solver, "right")) / 0.003,
    );
}

#[test]
fn inconsistent_inductive_cutset_current_is_refused() {
    let deck = "C1 left 0 10u\nC2 right 0 20u\nL1 left mid 1m\nL2 mid right 3m\n";
    refused(
        deck,
        state_value(&[("C1", 4.0), ("C2", 0.0)], &[("L1", 0.1), ("L2", 0.2)]),
    );
}

#[test]
fn mutual_inductance_is_retained_in_the_cutset_voltage() {
    let deck = "C1 left 0 10u\nC2 right 0 20u\nL1 left mid 1m\nL2 mid right 3m\nK1 L1 L2 0.5\n";
    let solver = initialize(
        deck,
        &state(&[("C1", 4.0), ("C2", 0.0)], &[("L1", 0.1), ("L2", 0.1)]),
        12,
    )
    .unwrap();
    let mutual = 0.5 * (0.001_f64 * 0.003).sqrt();
    near(
        node(&solver, "mid"),
        4.0 * (0.003 + mutual) / (0.004 + 2.0 * mutual),
    );
    near(current(&solver, "L1"), 0.1);
    near(current(&solver, "L2"), 0.1);
}

#[test]
fn equal_parallel_capacitors_share_one_independent_voltage_constraint() {
    let deck = "R1 out 0 1k\nC1 out 0 1u\nC2 out 0 2u\nC3 out 0 3u\n";
    let initial = state(&[("C1", 2.0), ("C2", 2.0), ("C3", 2.0)], &[]);
    let mut solver = initialize(deck, &initial, 2).unwrap();
    assert_eq!(status(&solver)["initialization_unknowns"], 2);
    for index in 0..3 {
        near(solver.capacitor_voltage(index), 2.0);
    }
    solver.advance(0.006).unwrap();
    near(node(&solver, "out"), 1.0);
    assert_eq!(solver.circuit().branch_count(), 0);
}

#[test]
fn consistent_capacitor_cycle_is_reduced_without_losing_state() {
    let deck = "R1 a 0 1k\nR2 b 0 2k\nC1 a 0 1u\nC2 b a 2u\nC3 b 0 3u\n";
    let solver = initialize(
        deck,
        &state(&[("C1", 1.0), ("C2", 2.0), ("C3", 3.0)], &[]),
        4,
    )
    .unwrap();
    near(node(&solver, "a"), 1.0);
    near(node(&solver, "b"), 3.0);
    assert_eq!(status(&solver)["initialization_unknowns"], 4);
}

#[test]
fn contradictory_parallel_and_cyclic_voltage_states_refuse() {
    refused(
        "C1 out 0 1u\nC2 out 0 2u\n",
        state_value(&[("C1", 1.0), ("C2", 2.0)], &[]),
    );
    let cycle = "R1 a 0 1k\nC1 a 0 1u\nC2 b a 2u\nC3 b 0 3u\n";
    refused(
        cycle,
        state_value(&[("C1", 1.0), ("C2", 2.0), ("C3", 4.0)], &[]),
    );
}

#[test]
fn constant_source_constraint_is_counted_before_parallel_capacitor() {
    let solver = initialize(
        "V1 out 0 dc 2\nR1 out 0 1k\nC1 out 0 1u\n",
        &state(&[("C1", 2.0)], &[]),
        2,
    )
    .unwrap();
    near(current(&solver, "V1"), -0.002);
    near(solver.capacitor_voltage(0), 2.0);
    assert_eq!(status(&solver)["base_unknowns"], 2);
    assert_eq!(status(&solver)["initialization_unknowns"], 2);
}

#[test]
fn conflicting_source_and_capacitor_state_refuse() {
    refused(
        "V1 out 0 dc 2\nC1 out 0 1u\n",
        state_value(&[("C1", 1.0)], &[]),
    );
}

#[test]
fn delayed_battery_ramp_starts_at_exact_zero_state_without_time_skip() {
    let deck = "V1 vin 0 PULSE(0 1 1u 1u 1u 1 3)\nR1 vin mid 0.1\nL1 mid out 3u\nC1 out 0 10u\n";
    let mut solver = initialize(deck, &state(&[("C1", 0.0)], &[("L1", 0.0)]), 10).unwrap();
    near(solver.time(), 0.0);
    near(node(&solver, "vin"), 0.0);
    near(current(&solver, "V1"), 0.0);
    near(current(&solver, "L1"), 0.0);
    solver.advance(0.5e-6).unwrap();
    near(node(&solver, "out"), 0.0);
    solver.advance(2e-6).unwrap();
    assert!(node(&solver, "out") > 0.0);
}

#[test]
fn immediate_voltage_ramp_refuses_unrepresented_t0_feedthrough_current() {
    let deck = "V1 out 0 PULSE(0 1 0 1u 1u 1 3)\nC1 out 0 1u\n";
    refused(deck, state_value(&[("C1", 0.0)], &[]));
}

#[test]
fn floating_resistive_or_unreferenced_capacitive_gauges_still_refuse() {
    refused("R1 a b 1k\n", state_value(&[], &[]));
    refused("C1 a b 1u\n", state_value(&[("C1", 1.0)], &[]));
}

#[test]
fn both_state_maps_are_required_even_when_empty() {
    for value in [
        json!({"mode":"transient_state"}),
        json!({"mode":"transient_state", "capacitor_voltages_v":{}}),
        json!({"mode":"transient_state", "inductor_currents_a":{}}),
    ] {
        refused("R1 out 0 1k\n", value);
    }
    initialize("V1 out 0 dc 1\nR1 out 0 1k\n", &state(&[], &[]), 2).unwrap();
}

#[test]
fn incomplete_and_unknown_capacitor_or_inductor_entries_refuse() {
    refused(OPEN_LC, state_value(&[], &[("L1", 0.0)]));
    refused(OPEN_LC, state_value(&[("C1", 0.0)], &[]));
    refused(
        OPEN_LC,
        state_value(&[("C1", 0.0), ("Cmissing", 0.0)], &[("L1", 0.0)]),
    );
    refused(
        OPEN_LC,
        state_value(&[("C1", 0.0)], &[("L1", 0.0), ("Lmissing", 0.0)]),
    );
}

#[test]
fn generated_opamp_capacitor_is_part_of_the_required_state_inventory() {
    let deck = ".model amp OPAMP(AOL=1000 GBW=1meg ROUT=1)\nV1 vcc 0 dc 3\nR1 inp 0 1k\nR2 inn 0 1k\nR3 out 0 1k\nXamp inp inn vcc 0 out amp\n";
    refused(deck, state_value(&[], &[]));
    let solver = initialize(deck, &state(&[("Xamp#cg", 0.0)], &[]), 12).unwrap();
    assert_eq!(solver.circuit().capacitors.len(), 1);
    assert_eq!(solver.circuit().capacitors[0].name, "Xamp#cg");
    near(solver.capacitor_voltage(0), 0.0);
}

#[test]
fn state_names_resolve_case_insensitively_but_case_fold_duplicates_refuse() {
    initialize(OPEN_LC, &state(&[("c1", 0.0)], &[("l1", 0.0)]), 8).unwrap();
    refused(
        OPEN_LC,
        state_value(&[("C1", 0.0), ("c1", 0.0)], &[("L1", 0.0)]),
    );
    refused(
        OPEN_LC,
        state_value(&[("C1", 0.0)], &[("L1", 0.0), ("l1", 0.0)]),
    );
}

#[test]
fn map_name_grammar_is_bounded_printable_ascii_without_whitespace() {
    for name in ["", " C1", "C1 ", "C\t1", "C\n1", "C\u{7f}1", "Cé"] {
        assert!(
            serde_json::from_value::<AnalogInitialization>(state_value(&[(name, 0.0)], &[]))
                .is_err(),
            "accepted key {name:?}"
        );
    }
    let accepted = format!("C{}", "a".repeat(255));
    let deck = format!("R1 out 0 1k\n{accepted} out 0 1u\n");
    initialize(&deck, &state(&[(&accepted, 0.0)], &[]), 2).unwrap();
    let too_long = format!("C{}", "a".repeat(256));
    assert!(
        serde_json::from_value::<AnalogInitialization>(state_value(&[(&too_long, 0.0)], &[]))
            .is_err()
    );
}

#[test]
fn oversized_state_map_is_refused_at_the_configuration_boundary() {
    let caps: BTreeMap<String, f64> = (0..16_385).map(|i| (format!("C{i}"), 0.0)).collect();
    let value =
        json!({"mode":"transient_state", "capacitor_voltages_v":caps, "inductor_currents_a":{}});
    assert!(serde_json::from_value::<AnalogInitialization>(value).is_err());
}

#[test]
fn nonfinite_values_and_unknown_initialization_fields_refuse() {
    for spelling in [".nan", ".inf", "-.inf"] {
        let yaml = format!("mode: transient_state\ncapacitor_voltages_v: {{C1: {spelling}}}\ninductor_currents_a: {{}}\n");
        if let Ok(initial) = serde_yaml::from_str::<AnalogInitialization>(&yaml) {
            assert!(initialize("C1 out 0 1u\n", &initial, 2).is_err());
        }
    }
    refused(
        "R1 out 0 1k\n",
        json!({"mode":"transient_state","capacitor_voltages_v":{},"inductor_currents_a":{},"guess_missing_states":true}),
    );
    refused(
        "R1 out 0 1k\n",
        json!({"mode":"operating_point","capacitor_voltages_v":{}}),
    );
    refused("R1 out 0 1k\n", json!({"mode":"automatic"}));
}

#[test]
fn explicit_state_never_mixes_with_netlist_initial_conditions() {
    for deck in [
        "R1 out 0 1k\nC1 out 0 1u\n.ic V(out)=0\n",
        "R1 out 0 1k\nC1 out 0 1u ic=0\n",
    ] {
        refused(deck, state_value(&[("C1", 0.0)], &[]));
    }
    refused(
        "R1 out 0 10\nL1 out 0 1m ic=0\n",
        state_value(&[], &[("L1", 0.0)]),
    );
}

fn grounded_nodes(count: usize) -> String {
    let mut deck = String::new();
    for i in 0..count {
        deck.push_str(&format!("R{i} n{i} 0 1k\n"));
    }
    deck.push_str("C1 n0 0 1u\n");
    deck
}

#[test]
fn peak_initialization_capacity_is_checked_without_changing_the_base_default() {
    assert_eq!(MAX_UNKNOWNS, 128);
    let deck = grounded_nodes(128);
    let initial = state(&[("C1", 0.0)], &[]);
    Solver::new(parse_netlist(&deck).unwrap(), Integration::BackwardEuler).unwrap();
    assert!(matches!(
        initialize(&deck, &initial, 128),
        Err(AnalogError::TooLarge {
            unknowns: 129,
            max: 128
        })
    ));
    let solver = initialize(&deck, &initial, 129).unwrap();
    assert_eq!(status(&solver)["base_unknowns"], 128);
    assert_eq!(status(&solver)["initialization_unknowns"], 129);
    assert_eq!(solver.circuit().unknowns(), 128);
}

#[test]
fn peak_above_hard_limit_refuses_even_when_the_base_circuit_fits() {
    assert_eq!(HARD_MAX_UNKNOWNS, 1024);
    let deck = grounded_nodes(1024);
    assert_eq!(parse_netlist(&deck).unwrap().unknowns(), 1024);
    assert!(matches!(
        initialize(&deck, &state(&[("C1", 0.0)], &[]), 1024),
        Err(AnalogError::TooLarge {
            unknowns: 1025,
            max: 1024
        })
    ));
}

// Append to analog_transient_state_contract.rs. This uses that file's public-API
// helpers. It was drafted by the independent reviewer; no local pass is claimed.
#[test]
fn biased_intrinsic_junction_keeps_its_initial_charge_without_a_false_pulse() {
    // All explicit C/L maps are empty. The diode's depletion charge is a model
    // state fixed by the solved -1 V terminal bias. Constant voltage means its
    // charge cannot change at the first time boundary.
    let deck = "Vbias bias 0 dc -1\nD1 bias 0 stored\n.model stored D(IS=1e-14 N=1 CJO=1n VJ=1 M=0.5 FC=0.5)\n";
    for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
        let mut baseline = Solver::new(parse_netlist(deck).unwrap(), integration).unwrap();
        let mut explicit = Solver::new_with_initialization(
            parse_netlist(deck).unwrap(),
            integration,
            8,
            &state(&[], &[]),
        )
        .unwrap();
        near(explicit.time(), 0.0);
        near(node(&explicit, "bias"), -1.0);
        let initial_source_current = current(&explicit, "Vbias");
        assert!(initial_source_current.abs() < 1e-10);
        near(initial_source_current, current(&baseline, "Vbias"));
        for _ in 0..2 {
            baseline.advance(100e-9).unwrap();
            explicit.advance(100e-9).unwrap();
            near(node(&explicit, "bias"), -1.0);
            let observed = current(&explicit, "Vbias");
            assert!(
                (observed - initial_source_current).abs() < 1e-10,
                "constant bias generated a false initial charge current: {observed:e} A"
            );
            near(observed, current(&baseline, "Vbias"));
        }
    }
}
