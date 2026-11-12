//! Independent voltage-connected inductive-group initialization contracts.
//! Drafted outside the integration tree. No build or pass is claimed here.
use labwired_core::analog::{
    parse_netlist, AnalogError, AnalogInitialization, Integration, Solver, MAX_UNKNOWNS,
};
use serde_json::json;
use std::collections::BTreeMap;

const STAR: &str = "Va a 0 9\nVb b 0 -3\nVc c 0 6\nLa a al 1m\nLb b bl 2m\nLc c cl 3m\nVea al star 1\nVeb bl star -2\nVec cl star 3\n";
fn initialize(deck: &str, currents: &[(&str, f64)]) -> Result<Solver, AnalogError> {
    let initial: AnalogInitialization = serde_json::from_value(json!({
        "mode": "transient_state", "capacitor_voltages_v": {},
        "inductor_currents_a": currents.iter().copied().collect::<BTreeMap<_, _>>(),
    }))
    .unwrap();
    Solver::new_with_initialization(
        parse_netlist(deck).unwrap(),
        Integration::BackwardEuler,
        MAX_UNKNOWNS,
        &initial,
    )
}
fn node(s: &Solver, name: &str) -> f64 {
    s.node_voltage(s.circuit().node(name).expect("original named node"))
}
fn current(s: &Solver, name: &str) -> f64 {
    s.branch_current(
        s.circuit()
            .branch_index(name)
            .expect("original named branch"),
    )
}
fn near(actual: f64, expected: f64) {
    assert!(actual.is_finite(), "nonfinite observation");
    assert!(
        (actual - expected).abs() <= 2e-10 * expected.abs().max(1.0),
        "actual={actual:e}, expected={expected:e}"
    );
}
fn ordered_decks(deck: &str) -> [String; 2] {
    [
        deck.to_owned(),
        deck.lines().rev().collect::<Vec<_>>().join("\n") + "\n",
    ]
}
fn star_currents(s: &Solver, expected: [f64; 3]) {
    for (i, (winding, emf, drive)) in [
        ("La", "Vea", "Va"),
        ("Lb", "Veb", "Vb"),
        ("Lc", "Vec", "Vc"),
    ]
    .into_iter()
    .enumerate()
    {
        near(current(s, winding), expected[i]);
        near(current(s, emf), expected[i]);
        near(current(s, drive), -expected[i]);
    }
    near(expected.iter().sum(), 0.0);
    near(current(s, "La") + current(s, "Lb") + current(s, "Lc"), 0.0);
}

#[test]
fn cold_star_with_stationary_back_emf_sources_has_no_artificial_reference() {
    let deck = "Va a 0 0\nVb b 0 0\nVc c 0 0\nLa a al 1m\nLb b bl 2m\nLc c cl 3m\nVea al star 0\nVeb bl star 0\nVec cl star 0\n";
    for ordered in ordered_decks(deck) {
        let mut s = initialize(&ordered, &[("La", 0.0), ("Lb", 0.0), ("Lc", 0.0)]).unwrap();
        for name in ["a", "b", "c", "al", "bl", "cl", "star"] {
            near(node(&s, name), 0.0);
        }
        star_currents(&s, [0.0; 3]);
        near(s.time(), 0.0);
        assert!(s.circuit().resistors.is_empty());
        assert!(s.circuit().capacitors.is_empty());
        assert_eq!(s.circuit().voltage_sources.len(), 6);
        assert_eq!(s.circuit().branch_count(), 9);
        let status = serde_json::to_value(s.initialization_status()).unwrap();
        assert_eq!(status["mode"], "transient_state");
        assert_eq!(status["base_unknowns"], 16);
        assert_eq!(status["initialization_unknowns"], 16);
        s.advance(1e-6).unwrap();
        star_currents(&s, [0.0; 3]);
        near(node(&s, "star"), 0.0);
        assert_eq!(s.circuit().voltage_sources.len(), 6);
    }
}

#[test]
fn nonzero_star_currents_preserve_each_original_source_current_and_offset() {
    for ordered in ordered_decks(STAR) {
        let mut s = initialize(&ordered, &[("La", 0.3), ("Lb", -0.1), ("Lc", -0.2)]).unwrap();
        // d(sum I)/dt=0 gives Vstar=sum((Vphase-E)/L)/sum(1/L)=51/11.
        near(node(&s, "star"), 51.0 / 11.0);
        near(node(&s, "al"), 62.0 / 11.0);
        near(node(&s, "bl"), 29.0 / 11.0);
        near(node(&s, "cl"), 84.0 / 11.0);
        star_currents(&s, [0.3, -0.1, -0.2]);
        let h = 1e-6;
        s.advance(h).unwrap();
        // Constant ideal terminal voltages make this finite BE update exact.
        star_currents(
            &s,
            [
                0.3 + h * 37000.0 / 11.0,
                -0.1 - h * 31000.0 / 11.0,
                -0.2 - h * 6000.0 / 11.0,
            ],
        );
        near(node(&s, "star"), 51.0 / 11.0);
        near(node(&s, "al") - node(&s, "star"), 1.0);
        near(node(&s, "bl") - node(&s, "star"), -2.0);
        near(node(&s, "cl") - node(&s, "star"), 3.0);
        assert_eq!(s.circuit().voltage_sources.len(), 6);
    }
}

#[test]
fn inconsistent_star_sum_is_refused_before_any_state_is_published() {
    for ordered in ordered_decks(STAR) {
        let error = match initialize(&ordered, &[("La", 0.3), ("Lb", -0.1), ("Lc", -0.1)]) {
            Err(error) => error,
            Ok(_) => panic!("sum I is +0.1 A must be refused"),
        };
        assert!(
            matches!(&error, AnalogError::Config(message) if message.contains("KCL")),
            "{error}"
        );
    }
}

#[test]
fn stationary_voltage_offset_between_series_inductors_retains_member_kcl() {
    let deck = "Vdrive drive 0 7\nLleft drive left 1m\nVoffset left right 1\nLright right 0 2m\n";
    for ordered in ordered_decks(deck) {
        let mut s = initialize(&ordered, &[("Lleft", 0.25), ("Lright", 0.25)]).unwrap();
        near(node(&s, "left"), 5.0);
        near(node(&s, "right"), 4.0);
        near(current(&s, "Lleft"), 0.25);
        near(current(&s, "Lright"), 0.25);
        near(current(&s, "Voffset"), 0.25);
        near(current(&s, "Vdrive"), -0.25);
        s.advance(1e-6).unwrap();
        near(current(&s, "Lleft"), 0.252);
        near(current(&s, "Lright"), 0.252);
        near(current(&s, "Voffset"), 0.252);
        near(current(&s, "Vdrive"), -0.252);
        assert!(s.circuit().resistors.is_empty());
        assert_eq!(s.circuit().voltage_sources.len(), 2);
    }
}

#[test]
fn a_voltage_source_cannot_hide_inconsistent_series_inductor_currents() {
    let deck = "Vdrive drive 0 7\nLleft drive left 1m\nVoffset left right 1\nLright right 0 2m\n";
    let error = match initialize(deck, &[("Lleft", 0.25), ("Lright", 0.20)]) {
        Err(error) => error,
        Ok(_) => panic!("group KCL violation must be refused"),
    };
    assert!(
        matches!(&error, AnalogError::Config(message) if message.contains("KCL")),
        "{error}"
    );
}

#[test]
fn mutual_inductance_uses_the_full_matrix_for_voltage_group_derivatives() {
    let deck = "Vdrive drive 0 10\nLleft drive left 1m\nVoffset left right 1\nLright right 0 4m\nKpair Lleft Lright .5\n";
    let mut s = initialize(deck, &[("Lleft", 0.2), ("Lright", 0.2)]).unwrap();
    // M=k*sqrt(L1*L2)=1mH. Series-aiding total L=7mH.
    // dI/dt=(10-1)/.007=9000/7; Vright=(L2+M)*dI/dt=45/7.
    near(node(&s, "right"), 45.0 / 7.0);
    near(node(&s, "left"), 52.0 / 7.0);
    near(current(&s, "Voffset"), 0.2);
    near(current(&s, "Vdrive"), -0.2);
    let h = 1e-6;
    s.advance(h).unwrap();
    let expected = 0.2 + h * 9000.0 / 7.0;
    near(current(&s, "Lleft"), expected);
    near(current(&s, "Lright"), expected);
    near(current(&s, "Voffset"), expected);
    near(current(&s, "Vdrive"), -expected);
}

#[test]
fn ambiguous_parallel_voltage_source_current_sharing_is_not_fabricated() {
    let deck = format!("{STAR}Vparallel al star 1\n");
    assert!(initialize(&deck, &[("La", 0.3), ("Lb", -0.1), ("Lc", -0.2)]).is_err());
}
