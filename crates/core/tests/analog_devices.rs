// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The nonlinear half of the in-core analog engine: the diode, the BJT, the
//! JFET, the MOSFET, the Newton iteration that reaches them and the model
//! cards that parameterise them.
//!
//! Three kinds of assertion live here and they are deliberately different:
//!
//! * **Hand calculations.** Every DC operating point below is checked against
//!   the closed form of the device equation, arranged so the answer is derived
//!   rather than read back from the solver — the diode fixture picks the
//!   current first and works out the source voltage that produces it. These
//!   catch a wrong equation.
//! * **Goldens.** A few operating points are pinned as raw `f64` bit patterns.
//!   These catch the drift `analog::device` exists to prevent: a `libm` that
//!   stopped being used, an `exp` that became the platform's, a reordered sum.
//!   A golden that fails is not automatically wrong — but it has to be
//!   explained before it is re-pinned.
//! * **The linear path.** Pinned bit patterns taken from the engine *before*
//!   Newton iteration existed. Nothing in this branch may move them.
//!
//! What remains unsupported is documented in `cosimulation_plugins.md`. Diode
//! forward/reverse high-injection knees (`IK`/`IKF`/`IKR`) and the BJT
//! high-current beta exponent (`NKF`), recombination
//! current (`ISR`/`NR`), diffusion charge (`TT`) and soft reverse recovery
//! (`TT`/`VP`), plus BJT forward bias-dependent and reverse
//! transit-time charge (`TF`/`TR`/`XTF`/`VTF`/`ITF`) are exercised below. Diode
//! depletion charge through `CJO`/`VJ`/`M`/`FC` and reverse breakdown emission
//! through `NBV`, BJT base-junction depletion charge through `CJE`/`CJC`,
//! JFET gate-junction depletion charge through
//! `CGS`/`CGD` and the Parker–Skellern channel fit `B`, MOSFET linear overlap
//! charge through `CGSO`/`CGDO`/`CGBO`,
//! MOSFET bulk-junction depletion charge through `CBD`/`CBS` and `CJ`/`CJSW`
//! geometry, MOSFET intrinsic channel capacitance from `TOX`, and JFET terminal
//! resistances `RD`/`RS`, and MOSFET terminal resistances from `RD`/`RS` or
//! `RSH` times per-instance `NRD`/`NRS`, and BJT current-dependent base
//! resistance through `RBM`/`IRB` are exercised below. Circuits depending
//! on unsupported stored charge still belong to `adapter: external_process`.

// The `analog` module puts every test under `analog::`, so the design's
// verification command `cargo test -p labwired-core analog` runs all of them.
mod analog {
    use labwired_core::analog::{
        device, parse_netlist, AnalogError, Integration, Polarity, Solver,
    };

    /// `kT/q` at the engine's fixed 300 K, repeated here rather than imported
    /// so a hand calculation and the thing it checks cannot drift together.
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    fn build(netlist: &str) -> Solver {
        let circuit = parse_netlist(netlist).unwrap_or_else(|error| panic!("{error}"));
        Solver::new(circuit, Integration::BackwardEuler).unwrap_or_else(|error| panic!("{error}"))
    }

    /// Current flowing out of a voltage source's `+` terminal into the rest of
    /// the circuit. The MNA branch unknown is the other sign.
    fn source_current(solver: &Solver, name: &str) -> f64 {
        -solver.branch_current(solver.circuit().branch_index(name).expect("source exists"))
    }

    fn relative(actual: f64, expected: f64) -> f64 {
        ((actual - expected) / expected).abs()
    }

    /// ngspice's level-1 diode `VJ(T)`/`CJO(T)` equations, calculated here
    /// independently of the solver's operating-model adjustment.
    fn spice_diode_depletion_parameters(
        cjo: f64,
        vj: f64,
        m: f64,
        nominal_celsius: f64,
        temperature_celsius: f64,
    ) -> (f64, f64) {
        let nominal_kelvin = nominal_celsius + 273.15;
        let temperature_kelvin = temperature_celsius + 273.15;
        if nominal_kelvin.to_bits() == temperature_kelvin.to_bits() {
            return (cjo, vj);
        }
        let nominal_vt = 1.380_649e-23 * nominal_kelvin / 1.602_176_634e-19;
        let vt = 1.380_649e-23 * temperature_kelvin / 1.602_176_634e-19;
        let eg_nominal =
            1.16 - 7.02e-4 * nominal_kelvin * nominal_kelvin / (nominal_kelvin + 1108.0);
        let eg = 1.16
            - 7.02e-4 * temperature_kelvin * temperature_kelvin / (temperature_kelvin + 1108.0);
        let ratio = temperature_kelvin / nominal_kelvin;
        let temperature_vj =
            vj * ratio - vt * (3.0 * ratio.ln() + eg_nominal / nominal_vt - eg / vt);
        let temperature_cjo = cjo
            * (1.0
                + m * (4.0e-4 * (temperature_kelvin - nominal_kelvin) - temperature_vj / vj + 1.0));
        (temperature_cjo, temperature_vj)
    }

    // -----------------------------------------------------------------------
    // Diode
    // -----------------------------------------------------------------------

    /// The operating point of `V — R — D — gnd`, derived rather than measured.
    ///
    /// The hand calculation runs backwards: choose the junction current, invert
    /// the Shockley equation for the junction voltage it needs, and set the
    /// source to that plus the drop across the resistor. The solver is then
    /// asked for a number the test already knows, and neither side of the
    /// comparison came from the other.
    #[test]
    fn diode_operating_point_matches_the_inverted_shockley_equation() {
        let is = 1e-14;
        let n = 1.0;
        let r = 1000.0;
        for target_current in [1e-6, 1e-4, 1e-3, 1e-2] {
            let vd = n * VT * f64::ln(target_current / is + 1.0);
            let vsource = vd + target_current * r;
            let netlist = format!(
                "* diode operating point\n\
                 Vs s 0 dc {vsource}\n\
                 R1 s a {r}\n\
                 D1 a 0 DMOD\n\
                 .model DMOD D(IS={is} N={n})\n\
                 .end\n"
            );
            let solver = build(&netlist);
            let node = solver
                .circuit()
                .node("a")
                .expect("node a")
                .expect("not gnd");
            let actual = solver.node_voltage(Some(node));
            assert!(
                relative(actual, vd) < 1e-8,
                "at {target_current} A the junction should sit at {vd} V, solver says {actual} V"
            );
            let current = source_current(&solver, "Vs");
            assert!(
                relative(current, target_current) < 1e-6,
                "expected {target_current} A through the loop, got {current} A"
            );
        }
    }

    /// A diode's instance `TEMP` changes both `kT/q` and the SPICE saturation
    /// current law. The expected load line is solved independently from those
    /// equations, including the model's series resistance.
    #[test]
    fn diode_instance_temperature_matches_the_spice_saturation_current_law() {
        let (is, n, eg, xti, tnom_c, rs, r, isr, nr, vj, m, target_current) = (
            2.52e-14, 1.15, 1.11, 3.0, 27.0, 50.0, 2200.0, 4e-11, 1.8, 0.9, 0.4, 200e-6,
        );
        let source_voltage = {
            let vt = 8.617_333_262_145e-5 * (tnom_c + 273.15);
            let junction = n * vt * f64::ln(target_current / is + 1.0);
            junction + target_current * (r + rs)
        };
        let adjusted_saturation = |nominal_is: f64, emission: f64, temperature_c: f64| {
            let nominal = tnom_c + 273.15;
            let temperature = temperature_c + 273.15;
            let vt_nominal = 8.617_333_262_145e-5 * nominal;
            let vt = 8.617_333_262_145e-5 * temperature;
            let log_factor = eg / vt_nominal - eg / vt + xti * f64::ln(temperature / nominal);
            nominal_is * f64::exp(log_factor / emission)
        };
        let expected_junction = |temperature_c: f64| {
            let temperature = temperature_c + 273.15;
            let vt = 8.617_333_262_145e-5 * temperature;
            let is_temperature = adjusted_saturation(is, n, temperature_c);
            let isr_temperature = adjusted_saturation(isr, nr, temperature_c);
            let residual = |junction: f64| {
                let diode = is_temperature * (f64::exp(junction / (n * vt)) - 1.0);
                let recombination = isr_temperature
                    * (f64::exp(junction / (nr * vt)) - 1.0)
                    * ((1.0 - junction / vj).powi(2) + 0.005).powf(m / 2.0);
                (source_voltage - junction) / (r + rs) - diode - recombination
            };
            let (mut low, mut high) = (0.0, source_voltage);
            for _ in 0..100 {
                let mid = 0.5 * (low + high);
                if residual(mid) > 0.0 {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            let junction = 0.5 * (low + high);
            let current = (source_voltage - junction) / (r + rs);
            (junction, current)
        };

        let mut currents = Vec::new();
        for temperature_c in [27.0, 125.0] {
            let netlist = format!(
                "V1 supply 0 dc {source_voltage}\n\
                 R1 supply a {r}\n\
                 D1 a 0 DMOD TEMP={temperature_c}\n\
                 .model DMOD D(IS={is} N={n} RS={rs} EG={eg} XTI={xti} TNOM={tnom_c} \
                               ISR={isr} NR={nr} VJ={vj} M={m})\n"
            );
            let solver = build(&netlist);
            let (expected_vj, expected_current) = expected_junction(temperature_c);
            let actual_current = source_current(&solver, "V1");
            let external = solver.circuit().node("a").expect("external diode node");
            let junction = solver
                .circuit()
                .node("D1#internal")
                .expect("series resistance creates the junction node");
            let actual_external_voltage = solver.node_voltage(external);
            let actual_junction_voltage = solver.node_voltage(junction);

            assert!(
                relative(actual_current, expected_current) < 2e-7,
                "TEMP={temperature_c} °C: SPICE load line predicts {expected_current} A, got {actual_current} A"
            );
            assert!(
                (actual_junction_voltage - expected_vj).abs() < 2e-8,
                "TEMP={temperature_c} °C: junction should be at {expected_vj} V, got {actual_junction_voltage} V"
            );
            assert!(
                (actual_external_voltage - (source_voltage - r * expected_current)).abs() < 2e-8,
                "TEMP={temperature_c} °C: external node must include the load resistor drop"
            );
            currents.push(actual_current);
        }
        assert!(
            currents[1] > currents[0] * 1.3,
            "heating the silicon junction must raise the current at this fixed bias: {currents:?}"
        );
    }

    /// `TRS`/`TRS1` and `TRS2` apply a quadratic temperature law to the diode's
    /// internal series resistance. Check the resulting load-line current and
    /// both sides of the internal resistor against a separately solved diode
    /// equation, with `IS` held temperature-independent to isolate this law.
    #[test]
    fn diode_series_resistance_temperature_coefficients_change_the_load_line() {
        let (is, n, rs, trs1, trs2, tnom, load, supply) =
            (1e-14, 1.2, 300.0, 1e-3, 2e-6, 27.0, 1_000.0, 2.0);
        let expected = |temperature_celsius: f64| {
            let delta = temperature_celsius - tnom;
            let rs_at_temperature = rs * (1.0 + trs1 * delta + trs2 * delta * delta);
            let vt = 8.617_333_262_145e-5 * (temperature_celsius + 273.15);
            let residual = |junction: f64| {
                (supply - junction) / (load + rs_at_temperature)
                    - is * (f64::exp(junction / (n * vt)) - 1.0)
            };
            let (mut low, mut high) = (0.0, supply);
            for _ in 0..100 {
                let mid = 0.5 * (low + high);
                if residual(mid) > 0.0 {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            let junction = 0.5 * (low + high);
            let current = (supply - junction) / (load + rs_at_temperature);
            (rs_at_temperature, junction, current)
        };

        let mut currents = Vec::new();
        for temperature_celsius in [27.0, 127.0] {
            let netlist = format!(
                "V1 supply 0 dc {supply}\n\
                 R1 supply a {load}\n\
                 D1 a 0 DMOD TEMP={temperature_celsius}\n\
                 .model DMOD D(IS={is} N={n} RS={rs} EG=0 XTI=0 TNOM={tnom} TRS={trs1} TRS2={trs2})\n"
            );
            let solver = build(&netlist);
            let diode = &solver.circuit().diodes[0];
            let (expected_rs, expected_junction, expected_current) = expected(temperature_celsius);
            let external = solver.circuit().node("a").expect("external diode node");
            let junction = solver
                .circuit()
                .node("D1#internal")
                .expect("series resistance creates the junction node");
            let actual_current = source_current(&solver, "V1");
            let actual_external = solver.node_voltage(external);
            let actual_junction = solver.node_voltage(junction);

            assert_eq!(diode.model.rs, rs, "RS remains the nominal model value");
            assert_eq!(diode.model.trs1, trs1, "TRS is the TRS1 alias");
            assert_eq!(diode.model.trs2, trs2);
            assert!(
                (diode.series_resistance_ohms - expected_rs).abs() < 1e-12,
                "TEMP={temperature_celsius} °C: expected RS(T)={expected_rs} Ω, got {} Ω",
                diode.series_resistance_ohms
            );
            assert!(
                relative(actual_current, expected_current) < 2e-7,
                "TEMP={temperature_celsius} °C: expected load current {expected_current} A, got {actual_current} A"
            );
            assert!(
                (actual_junction - expected_junction).abs() < 2e-8,
                "TEMP={temperature_celsius} °C: expected junction {expected_junction} V, got {actual_junction} V"
            );
            assert!(
                (actual_external - actual_junction - actual_current * expected_rs).abs() < 2e-8,
                "the internal diode resistor must carry the load current at RS(T)"
            );
            currents.push(actual_current);
        }
        assert_ne!(
            currents[0].to_bits(),
            currents[1].to_bits(),
            "the temperature-adjusted series resistance must change the solved load line"
        );

        let fixed_temperature = build(&format!(
            "V1 supply 0 dc {supply}\nR1 supply a {load}\nD1 a 0 DMOD\n\
             .model DMOD D(IS={is} N={n} RS={rs} EG=0 XTI=0 TNOM={tnom} TRS={trs1} TRS2={trs2})\n"
        ));
        assert_eq!(
            fixed_temperature.circuit().diodes[0].series_resistance_ohms,
            rs,
            "without instance TEMP, preserve the nominal fixed-temperature resistance"
        );
    }

    /// `TCV`/`TBV1` moves a zener's specified breakdown voltage relative to
    /// `TNOM`. Check the shifted reverse knee through model `RS` and an external
    /// load, so ignoring the coefficient or applying it at the external node
    /// produces the wrong operating point.
    #[test]
    fn diode_breakdown_temperature_coefficient_moves_loaded_zener_knee() {
        let (is, n, bv, ibv, nbv, tcv, tnom, rs, load, supply) =
            (1e-14, 1.2, 5.0, 1e-3, 1.4, 2e-3, 27.0, 40.0, 1_000.0, -7.0);
        let mut currents = Vec::new();

        for temperature in [27.0, 127.0] {
            let effective_bv = bv * (1.0 + tcv * (temperature - tnom));
            let temperature_kelvin = temperature + 273.15;
            let vt = 1.380_649e-23 * temperature_kelvin / 1.602_176_634e-19;
            let xbv = device::breakdown_voltage_at_thermal_voltage(effective_bv, ibv, is, nbv, vt);
            let current_at_junction =
                |voltage: f64| -is * (-(xbv + voltage) / (nbv * vt)).exp() + device::GMIN * voltage;
            let load_line = |junction_voltage: f64| {
                let current = current_at_junction(junction_voltage);
                (junction_voltage + current * rs - supply) / load + current
            };
            let (mut low, mut high) = (-12.0, 0.0);
            assert!(load_line(low) < 0.0 && load_line(high) > 0.0);
            for _ in 0..100 {
                let mid = (low + high) / 2.0;
                if load_line(mid) < 0.0 {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            let expected_junction = (low + high) / 2.0;
            let expected_current = current_at_junction(expected_junction);
            assert!(
                expected_junction < -xbv,
                "TEMP={temperature} °C test point must be in breakdown"
            );

            let netlist = format!(
                "Vdrive drive 0 dc {supply}\nRload drive a {load}\nD1 a 0 DBV TEMP={temperature}\n\
                 .model DBV D(IS={is} N={n} EG=0 XTI=0 TNOM={tnom} BV={bv} IBV={ibv} NBV={nbv} TCV={tcv} RS={rs})\n"
            );
            let solver = build(&netlist);
            let diode = &solver.circuit().diodes[0];
            assert_eq!(diode.model.tcv, tcv);
            assert!(
                (diode.breakdown.expect("BV is enabled") - xbv).abs() < 1e-12,
                "TEMP={temperature} °C: effective breakdown fit should use BV(T)={effective_bv} V"
            );
            let external = solver.circuit().node("a").expect("external anode");
            let junction = solver
                .circuit()
                .node("D1#internal")
                .expect("RS creates an internal junction node");
            let actual_external = solver.node_voltage(external);
            let actual_junction = solver.node_voltage(junction);
            let actual_current = source_current(&solver, "Vdrive");

            assert!(
                (actual_junction - expected_junction).abs() < 1e-8,
                "TEMP={temperature} °C: expected internal junction {expected_junction} V, got {actual_junction} V"
            );
            assert!(
                (actual_current - expected_current).abs() < 1e-9,
                "TEMP={temperature} °C: expected reverse current {expected_current} A, got {actual_current} A"
            );
            assert!(
                (actual_external - actual_junction - actual_current * rs).abs() < 1e-8,
                "the diode's series resistance must carry the breakdown current"
            );
            currents.push(actual_current);
        }

        assert!(
            currents[0] < currents[1],
            "the positive zener coefficient raises BV with temperature and reduces current at fixed reverse bias: {currents:?}"
        );

        let legacy = build(&format!(
            "Vdrive a 0 dc -5.5\nD1 a 0 DBV\n\
             .model DBV D(IS={is} N={n} BV={bv} IBV={ibv} NBV={nbv} TCV={tcv})\n"
        ));
        assert_eq!(
            legacy.circuit().diodes[0].breakdown,
            Some(device::breakdown_voltage(bv, ibv, is, nbv)),
            "without instance TEMP, the existing fixed-300 K path is retained"
        );
    }

    /// `IK`/`IKF` and `IKR` reduce forward and reverse junction current at
    /// high injection. The source voltage is derived from the selected
    /// junction voltage and the ngspice current transform, so each case checks
    /// the nonlinear stamp rather than only its parsed model fields.
    #[test]
    fn diode_forward_and_reverse_knee_currents_match_the_spice_equation() {
        let (is, resistance) = (1e-14, 1e3);

        let forward_vd = 0.7;
        let forward_ik = 100e-6;
        let forward_base = is * ((forward_vd / VT).exp() - 1.0);
        let forward_current =
            forward_base / (1.0 + (forward_base / forward_ik).sqrt()) + device::GMIN * forward_vd;
        let forward_netlist = format!(
            "V1 in 0 dc {}\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS={is} N=1 IK={forward_ik})\n",
            forward_vd + forward_current * resistance,
        );
        let forward = build(&forward_netlist);
        let forward_out = forward.circuit().node("out").expect("forward node");
        assert!(
            (forward.node_voltage(forward_out) - forward_vd).abs() < 1e-8,
            "forward junction should be {forward_vd} V, got {} V",
            forward.node_voltage(forward_out)
        );
        assert!(
            relative(source_current(&forward, "V1"), forward_current) < 1e-7,
            "forward knee current should be {forward_current} A, got {} A",
            source_current(&forward, "V1")
        );

        let reverse_vd = -5.0;
        let reverse_ikr = 10e-6;
        let reverse_model = parse_netlist(&format!(
            "V1 in 0 dc 0\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS={is} N=1 BV=5 IBV=1m IKR={reverse_ikr})\n"
        ))
        .expect("reverse knee model parses");
        let diode = &reverse_model.diodes[0];
        let xbv = diode.breakdown.expect("BV enables reverse breakdown");
        let reverse_base = -is * (-(xbv + reverse_vd) / VT).exp();
        let reverse_current =
            reverse_base / (1.0 + (-reverse_base / reverse_ikr).sqrt()) + device::GMIN * reverse_vd;
        let reverse_netlist = format!(
            "V1 in 0 dc {}\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS={is} N=1 BV=5 IBV=1m IKR={reverse_ikr})\n",
            reverse_vd + reverse_current * resistance,
        );
        let reverse = build(&reverse_netlist);
        let reverse_out = reverse.circuit().node("out").expect("reverse node");
        assert!(
            (reverse.node_voltage(reverse_out) - reverse_vd).abs() < 1e-7,
            "reverse junction should be {reverse_vd} V, got {} V",
            reverse.node_voltage(reverse_out)
        );
        assert!(
            relative(source_current(&reverse, "V1"), reverse_current) < 1e-6,
            "reverse knee current should be {reverse_current} A, got {} A",
            source_current(&reverse, "V1")
        );
    }

    /// `NBV` controls the reverse-breakdown exponential independently of the
    /// forward emission coefficient `N`; exercise it through model parsing and
    /// a solved circuit rather than only through the voltage-fit helper.
    #[test]
    fn diode_nondefault_breakdown_emission_matches_the_reverse_current_equation() {
        let (is, n, bv, ibv, nbv, vd) = (1e-14, 1.2, 5.0, 1e-3, 2.0, -5.05);
        let solver = build(&format!(
            "Vd a 0 dc {vd}\nD1 a 0 DBV\n\
             .model DBV D(IS={is} N={n} BV={bv} IBV={ibv} NBV={nbv})\n"
        ));
        let diode = &solver.circuit().diodes[0];
        assert_eq!(diode.model.nbv, Some(nbv));

        // At BV, IBV sets the reverse current; below BV, NBV sets its
        // exponential slope. The tiny correction from ngspice's effective-BV
        // fit is below 2e-9 relative for these values.
        let expected = -ibv * ((-vd - bv) / (nbv * VT)).exp() + 1e-12 * vd;
        let actual = source_current(&solver, "Vd");
        assert!(
            (actual - expected).abs() < 1e-10 + 2e-8 * expected.abs(),
            "NBV={nbv}: expected reverse current {expected} A, got {actual} A"
        );

        let default = build(&format!(
            "Vd a 0 dc {vd}\nD1 a 0 DBV\n\
             .model DBV D(IS={is} N={n} BV={bv} IBV={ibv})\n"
        ));
        let default_current = source_current(&default, "Vd");
        let expected_default = -ibv * ((-vd - bv) / (n * VT)).exp() + 1e-12 * vd;
        assert!(
            (default_current - expected_default).abs()
                < 1e-10 + 2e-8 * expected_default.abs(),
            "default NBV=N={n}: expected reverse current {expected_default} A, got {default_current} A"
        );
        assert!(
            (actual - default_current).abs() > 1e-3,
            "explicit NBV should change current: {actual} A vs default {default_current} A"
        );
    }

    /// The reverse-breakdown exponential is stamped at the junction behind
    /// `RS`, while the external load line sees the anode voltage. Solve that
    /// load line independently so a model that applies `NBV` at the external
    /// terminal, or bypasses `RS` in breakdown, cannot pass on the fixed-bias
    /// equation above.
    #[test]
    fn diode_nondefault_breakdown_emission_flows_through_series_resistance() {
        let (is, n, bv, ibv, nbv, rs, supply, load) =
            (1e-14, 1.2, 5.0, 1e-3, 2.0, 40.0, -6.0, 1_000.0);
        let solver = build(&format!(
            "Vdrive drive 0 dc {supply}\nRload drive a {load}\nD1 a 0 DBV\n\
             .model DBV D(IS={is} N={n} BV={bv} IBV={ibv} NBV={nbv} RS={rs})\n"
        ));
        let diode = &solver.circuit().diodes[0];
        assert_eq!(diode.model.nbv, Some(nbv));
        let xbv = device::breakdown_voltage(bv, ibv, is, nbv);

        // Independent reverse-breakdown law and load-line root. GMIN remains
        // outside the breakdown exponential, as it does in the stamped model.
        let current_at_junction =
            |voltage: f64| -is * (-(xbv + voltage) / (nbv * VT)).exp() + device::GMIN * voltage;
        let load_line = |junction_voltage: f64| {
            let current = current_at_junction(junction_voltage);
            (junction_voltage + current * rs - supply) / load + current
        };
        let (mut low, mut high) = (-10.0, 0.0);
        assert!(load_line(low) < 0.0 && load_line(high) > 0.0);
        for _ in 0..100 {
            let mid = (low + high) / 2.0;
            if load_line(mid) < 0.0 {
                low = mid;
            } else {
                high = mid;
            }
        }
        let expected_junction = (low + high) / 2.0;
        assert!(expected_junction < -xbv, "root must be in breakdown");
        let expected_current = current_at_junction(expected_junction);

        let external_anode = solver
            .circuit()
            .node("a")
            .expect("external anode")
            .expect("anode is not ground");
        let internal_junction = solver
            .circuit()
            .node("D1#internal")
            .expect("RS creates a probeable internal junction")
            .expect("junction is not ground");
        let actual_anode = solver.node_voltage(Some(external_anode));
        let actual_junction = solver.node_voltage(Some(internal_junction));
        let actual_current = source_current(&solver, "Vdrive");

        assert!(
            (actual_junction - expected_junction).abs() < 1e-8,
            "NBV={nbv}: expected junction voltage {expected_junction} V, got {actual_junction} V"
        );
        assert!(
            (actual_current - expected_current).abs() < 1e-10,
            "NBV={nbv}: expected breakdown current {expected_current} A, got {actual_current} A"
        );
        assert!(
            (actual_anode - actual_junction - actual_current * rs).abs() < 1e-8,
            "RS should carry the reverse current: anode {actual_anode} V, junction {actual_junction} V, current {actual_current} A"
        );
        assert!(
            (actual_current - (supply - actual_anode) / load).abs() < 1e-10,
            "the external resistor load line should close at the solved anode voltage"
        );
    }

    /// Reverse-breakdown transit charge uses the same NBV-dependent junction
    /// current as the DC avalanche branch. Check that charge through model-card
    /// RS against a scalar load-line solution and an explicit series resistor,
    /// for both transient companion rules.
    #[test]
    fn diode_breakdown_transit_charge_flows_through_series_resistance() {
        let (is, n, bv, ibv, nbv, rs, rdrive, cjo, vj, m, fc, tt, h) = (
            1e-14, 1.2, 5.0, 1e-3, 2.0, 40.0, 1_000.0, 100e-12, 0.8, 0.5, 0.5, 100e-9, 1e-6,
        );
        let model = format!(
            "IS={is} N={n} BV={bv} IBV={ibv} NBV={nbv} CJO={cjo} VJ={vj} M={m} FC={fc} TT={tt}"
        );
        let modelled = format!(
            "Vdrive drive 0 dc 0\nRdrive drive a {rdrive}\nD1 a 0 DBV\n\
             .model DBV D({model} RS={rs})\n"
        );
        let explicit = format!(
            "Vdrive drive 0 dc 0\nRdrive drive a {rdrive}\nRseries a j {rs}\nD1 j 0 DBV\n\
             .model DBV D({model})\n"
        );
        let uncharged = format!(
            "Vdrive drive 0 dc 0\nRdrive drive a {rdrive}\nD1 a 0 DBV\n\
             .model DBV D(IS={is} N={n} BV={bv} IBV={ibv} NBV={nbv} RS={rs})\n"
        );
        let xbv = device::breakdown_voltage(bv, ibv, is, nbv);
        let current = |voltage: f64| device::diode_op_breakdown(voltage, is, n, xbv, nbv);
        let stored_charge = |voltage: f64| {
            let op = current(voltage);
            let depletion = device::diode_depletion_charge(voltage, cjo, vj, m, fc).0;
            let diffusion = device::diode_diffusion_charge(voltage, op, tt).0;
            depletion + diffusion
        };

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let mut modelled_solver = Solver::new(
                parse_netlist(&modelled).expect("breakdown charge model parses"),
                integration,
            )
            .expect("model-card series-resistance solver builds");
            let mut explicit_solver = Solver::new(
                parse_netlist(&explicit).expect("explicit series resistor parses"),
                integration,
            )
            .expect("explicit-resistor solver builds");
            let mut uncharged_solver = Solver::new(
                parse_netlist(&uncharged).expect("uncharged model parses"),
                integration,
            )
            .expect("uncharged solver builds");

            let modelled_external = modelled_solver.circuit().node("a").expect("external anode");
            let modelled_junction = modelled_solver
                .circuit()
                .node("D1#internal")
                .expect("model-card RS creates an internal junction");
            let explicit_external = explicit_solver.circuit().node("a").expect("external anode");
            let explicit_junction = explicit_solver.circuit().node("j").expect("junction");
            // All three start at zero bias, where the depletion and diffusion
            // charge references are both zero, before the reverse avalanche step.
            for solver in [
                &mut modelled_solver,
                &mut explicit_solver,
                &mut uncharged_solver,
            ] {
                solver.advance(h).expect("zero-bias initial step settles");
            }

            let mut previous_charge = stored_charge(0.0);
            let mut previous_charge_current = 0.0;
            for step in 0..2 {
                let supply = -6.0;
                // A changed source restarts trapezoidal integration with one
                // backward-Euler step, then resumes trapezoidal history.
                let step_integration = if integration == Integration::Trapezoidal && step == 0 {
                    Integration::BackwardEuler
                } else {
                    integration
                };
                let charge_current = |voltage: f64| {
                    let q = stored_charge(voltage);
                    match step_integration {
                        Integration::BackwardEuler => (q - previous_charge) / h,
                        Integration::Trapezoidal => {
                            2.0 * (q - previous_charge) / h - previous_charge_current
                        }
                    }
                };
                let total_current = |voltage: f64| current(voltage).id + charge_current(voltage);
                let residual =
                    |voltage: f64| voltage + (rdrive + rs) * total_current(voltage) - supply;
                let (mut low, mut high) = (-10.0, 0.0);
                assert!(residual(low) < 0.0 && residual(high) > 0.0);
                for _ in 0..120 {
                    let middle = 0.5 * (low + high);
                    if residual(middle) < 0.0 {
                        low = middle;
                    } else {
                        high = middle;
                    }
                }
                let expected_junction = 0.5 * (low + high);
                let expected_current = total_current(expected_junction);
                let expected_external = supply - expected_current * rdrive;

                for solver in [
                    &mut modelled_solver,
                    &mut explicit_solver,
                    &mut uncharged_solver,
                ] {
                    solver.set_voltage_source(
                        solver
                            .voltage_source_index("Vdrive")
                            .expect("drive source exists"),
                        supply,
                    );
                    solver
                        .advance(h)
                        .expect("reverse-breakdown charge step solves");
                }

                let actual_junction = modelled_solver.node_voltage(modelled_junction);
                let actual_external = modelled_solver.node_voltage(modelled_external);
                let actual_current = source_current(&modelled_solver, "Vdrive");
                assert!(
                    expected_junction < -xbv,
                    "step {step}: expected point must be in breakdown, Vj={expected_junction} V"
                );
                assert!(
                    (actual_junction - expected_junction).abs() < 1e-8,
                    "{integration:?} step {step}: expected junction {expected_junction} V, got {actual_junction} V"
                );
                assert!(
                    (actual_external - expected_external).abs() < 1e-8,
                    "{integration:?} step {step}: expected external anode {expected_external} V, got {actual_external} V"
                );
                assert!(
                    (actual_current - expected_current).abs() < 1e-10 + 1e-6 * expected_current.abs(),
                    "{integration:?} step {step}: expected avalanche terminal current {expected_current} A, got {actual_current} A"
                );
                assert!(
                    (actual_junction - modelled_solver.node_voltage(modelled_external)
                        + actual_current * rs)
                        .abs()
                        < 1e-8,
                    "model-card RS must carry avalanche current from the external anode to the junction"
                );
                assert!(
                    (actual_external - explicit_solver.node_voltage(explicit_external)).abs() < 1e-8
                        && (actual_junction - explicit_solver.node_voltage(explicit_junction)).abs()
                            < 1e-8,
                    "{integration:?} step {step}: model-card RS and explicit resistor must agree at both terminals"
                );
                assert!(
                    (actual_current - source_current(&explicit_solver, "Vdrive")).abs() < 1e-10,
                    "{integration:?} step {step}: model-card RS and explicit resistor terminal currents differ"
                );
                if step == 0 {
                    let uncharged_current = source_current(&uncharged_solver, "Vdrive");
                    assert!(
                        (actual_current - uncharged_current).abs() > 1e-5,
                        "reverse-breakdown depletion and transit charge must affect the first-step current, charged {actual_current} A vs uncharged {uncharged_current} A"
                    );
                }

                let actual_charge = stored_charge(actual_junction);
                let actual_charge_current = charge_current(actual_junction);
                previous_charge = actual_charge;
                previous_charge_current = actual_charge_current;
            }
        }
    }

    /// Ngspice's `ISR`/`NR` branch adds recombination current in forward bias
    /// and holds its value at `−3·N·Vt` farther into reverse bias.
    #[test]
    fn diode_recombination_current_matches_the_forward_and_reverse_equations() {
        let (is, n, isr, nr, vj, m) = (1e-14, 1.3, 2e-8, 2.0, 0.75, 0.4);
        let recombination_current = |voltage: f64| {
            let boundary = -3.0 * n * VT;
            let voltage = voltage.max(boundary);
            let base = isr * ((voltage / (nr * VT)).exp() - 1.0);
            let x = 1.0 - voltage / vj;
            base * (x * x + 0.005).powf(m / 2.0)
        };

        for vd in [-0.3, -0.02, 0.15, 0.35] {
            let solver = build(&format!(
                "Vd a 0 dc {vd}\nD1 a 0 DREC\n.model DREC D(IS={is} N={n} ISR={isr} NR={nr} VJ={vj} M={m})\n"
            ));
            let expected =
                is * ((vd / (n * VT)).exp() - 1.0) + device::GMIN * vd + recombination_current(vd);
            let actual = source_current(&solver, "Vd");
            assert!(
                (actual - expected).abs() < 2e-12 + 2e-8 * expected.abs(),
                "at Vd={vd} V: expected {expected} A, got {actual} A"
            );
        }

        let (bv, ibv, vd) = (5.0, 1e-3, -6.0);
        let xbv = device::breakdown_voltage(bv, ibv, is, n);
        let solver = build(&format!(
            "Vd a 0 dc {vd}\nD1 a 0 DREC\n.model DREC D(IS={is} N={n} BV={bv} IBV={ibv} ISR={isr} NR={nr} VJ={vj} M={m})\n"
        ));
        let expected = device::diode_op_breakdown(vd, is, n, xbv, n).id + recombination_current(vd);
        let actual = source_current(&solver, "Vd");
        assert!(
            (actual - expected).abs() < 2e-12 + 2e-8 * expected.abs(),
            "at breakdown Vd={vd} V: expected {expected} A, got {actual} A"
        );

        // The analytic tangent also drives a loaded operating point to the
        // root of the independent diode equation.
        let supply = 0.35;
        let resistance = 10_000.0;
        let diode_current = |voltage: f64| {
            is * ((voltage / (n * VT)).exp() - 1.0)
                + device::GMIN * voltage
                + recombination_current(voltage)
        };
        let mut low = 0.0;
        let mut high = supply;
        for _ in 0..100 {
            let mid = (low + high) / 2.0;
            if supply - mid - resistance * diode_current(mid) > 0.0 {
                low = mid;
            } else {
                high = mid;
            }
        }
        let expected_vd = (low + high) / 2.0;
        let solver = build(&format!(
            "Vs s 0 dc {supply}\nR1 s a {resistance}\nD1 a 0 DREC\n.model DREC D(IS={is} N={n} ISR={isr} NR={nr} VJ={vj} M={m})\n"
        ));
        let anode = solver.circuit().node("a").expect("anode");
        let actual_vd = solver.node_voltage(anode);
        assert!(
            (actual_vd - expected_vd).abs() < 1e-8,
            "expected loaded diode voltage {expected_vd} V, got {actual_vd} V"
        );
    }

    /// `RS` is a real resistor in series with the junction, and it gets the
    /// internal node SPICE also creates — so the anode is no longer the
    /// junction, and `v(d1#internal)` is probeable.
    #[test]
    fn diode_series_resistance_drops_its_own_voltage_on_an_internal_node() {
        let is = 1e-14;
        let rs = 22.0;
        let current = 1e-2;
        let vj = VT * f64::ln(current / is + 1.0);
        let netlist = format!(
            "Vs s 0 dc {}\nR1 s a 1000\nD1 a 0 DMOD\n.model DMOD D(IS={is} N=1 RS={rs})\n",
            vj + current * rs + current * 1000.0
        );
        let solver = build(&netlist);
        let anode = solver.circuit().node("a").unwrap().unwrap();
        let internal = solver
            .circuit()
            .node("D1#internal")
            .expect("RS creates an internal node")
            .expect("which is not ground");
        let drop = solver.node_voltage(Some(anode)) - solver.node_voltage(Some(internal));
        assert!(
            relative(drop, current * rs) < 1e-5,
            "RS should drop {} V at {current} A, got {drop} V",
            current * rs
        );
        assert!(
            relative(solver.node_voltage(Some(internal)), vj) < 1e-6,
            "the junction itself should sit at {vj} V"
        );

        // RS = 0 must NOT create the node: it would cost an unknown for
        // nothing and change every node index behind it.
        let plain = build("Vs s 0 dc 1\nR1 s a 1000\nD1 a 0 DMOD\n.model DMOD D(IS=1e-14)\n");
        assert_eq!(plain.circuit().node("D1#internal"), None);
    }

    #[test]
    fn diode_depletion_charge_and_capacitance_match_the_spice_integral() {
        let cjo: f64 = 4e-12;
        let vj = 0.75;
        let m = 0.333;
        let fc = 0.4;
        let voltage = 0.25;
        let x = 1.0 - voltage / vj;
        let expected_q = cjo * vj / (1.0 - m) * (1.0 - x.powf(1.0 - m));
        let expected_c = cjo * x.powf(-m);
        let (charge, capacitance) = device::diode_depletion_charge(voltage, cjo, vj, m, fc);
        assert!((charge - expected_q).abs() < 1e-26);
        assert!((capacitance - expected_c).abs() < 1e-25);

        let transition = fc * vj;
        let (q_before, c_before) =
            device::diode_depletion_charge(transition - 1e-10, cjo, vj, m, fc);
        let (q_after, c_after) = device::diode_depletion_charge(transition + 1e-10, cjo, vj, m, fc);
        assert!(
            (q_before - q_after).abs() < 3e-21,
            "charge is continuous at FC: {q_before} vs {q_after}"
        );
        assert!(
            (c_before - c_after).abs() < 1e-20,
            "capacitance is continuous at FC: {c_before} vs {c_after}"
        );

        let (q_m1, c_m1) = device::diode_depletion_charge(voltage, cjo, vj, 1.0, fc);
        assert!((q_m1 + cjo * vj * x.ln()).abs() < 1e-26);
        assert!((c_m1 - cjo / x).abs() < 1e-25);
    }

    /// `TT` adds quasi-static diffusion charge `TT·Id` to the junction. The
    /// circuit checks the transient equation against a scalar root under both
    /// integration rules and confirms the operating-point charge is carried
    /// into the first step without a startup impulse.
    #[test]
    fn diode_tt_integrates_diffusion_charge_for_both_integration_rules() {
        let is = 1e-14;
        let tt = 100e-9;
        let resistance = 1e3;
        let h = 20e-9;
        let netlist = format!(
            "V1 in 0 dc 0\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS={is} N=1 TT={tt})\n"
        );
        let circuit = parse_netlist(&netlist).expect("diode TT model parses");
        assert_eq!(circuit.diodes[0].model.tt, tt);

        let charge = |voltage: f64| {
            let op = device::diode_op(voltage, is, 1.0);
            device::diode_diffusion_charge(voltage, op, tt).0
        };

        // A diode already at its operating point must not create a transient
        // current just because its diffusion charge was initialized.
        let biased_netlist = netlist.replacen("V1 in 0 dc 0", "V1 in 0 dc 1", 1);
        let mut biased = Solver::new(
            parse_netlist(&biased_netlist).expect("prebiased diode TT model parses"),
            Integration::BackwardEuler,
        )
        .expect("prebiased diode TT solver builds");
        let biased_out = biased.circuit().node("out").expect("output node");
        let initial_biased_voltage = biased.node_voltage(biased_out);
        biased
            .advance(h)
            .expect("operating-point diffusion charge carries into transient");
        assert!(
            (biased.node_voltage(biased_out) - initial_biased_voltage).abs() < 1e-9,
            "initialized diffusion charge must not create a startup impulse"
        );

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let mut solver =
                Solver::new(circuit.clone(), integration).expect("diode TT solver builds");
            let out = solver.circuit().node("out").expect("output node");
            let source = solver.voltage_source_index("V1").expect("source exists");
            solver.set_voltage_source(source, 1.0);

            let mut previous_charge = charge(solver.node_voltage(out));
            let mut previous_current = 0.0;
            for step in 0..2 {
                let charge_current = |voltage: f64| {
                    let next_charge = charge(voltage);
                    if step == 0 || integration == Integration::BackwardEuler {
                        (next_charge - previous_charge) / h
                    } else {
                        2.0 * (next_charge - previous_charge) / h - previous_current
                    }
                };
                let residual = |voltage: f64| {
                    (1.0 - voltage) / resistance
                        - device::diode_op(voltage, is, 1.0).id
                        - charge_current(voltage)
                };
                let mut lo = 0.0;
                let mut hi = 1.0;
                for _ in 0..100 {
                    let mid = 0.5 * (lo + hi);
                    if residual(mid) > 0.0 {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                let expected = 0.5 * (lo + hi);
                solver
                    .advance(h)
                    .expect("diode diffusion-charge step solves");
                let actual = solver.node_voltage(out);
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "{integration:?} step {step}: expected {expected} V, got {actual} V"
                );
                let next_charge = charge(actual);
                previous_current = (next_charge - previous_charge) / h;
                previous_charge = next_charge;
            }
        }
    }

    /// `TTT1`/`TTT2` scale diode transit time using the instance temperature.
    /// A solved transient circuit must match an otherwise identical model
    /// whose `TT` is set to the independently calculated value. Cover both
    /// quasi-static diffusion charge and the `VP` soft-recovery state.
    #[test]
    fn diode_transit_time_temperature_coefficients_match_explicit_charge_models() {
        let (is, n, tnom, tt) = (1e-14, 1.0, 27.0, 100e-9);
        let (ttt1, ttt2) = (8e-4, 4e-6);
        let (resistance, h) = (1e3, 20e-9);
        let adjusted_tt = |temperature: f64| {
            let delta = temperature - tnom;
            tt * (1.0 + ttt1 * delta + ttt2 * delta * delta)
        };
        let mut hot_effects = Vec::new();

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            for vp in [0.0, 0.25] {
                for temperature in [tnom, 127.0] {
                    let expected_tt = adjusted_tt(temperature);
                    let make_solver = |model: String| {
                        let netlist = format!(
                            "Vdrive drive 0 dc 0\nRdrive drive out {resistance}\n\
                             D1 out 0 DMOD TEMP={temperature}\n.model DMOD D({model})\n"
                        );
                        Solver::new(
                            parse_netlist(&netlist)
                                .expect("diode transient temperature deck parses"),
                            integration,
                        )
                        .expect("diode transient temperature solver builds")
                    };
                    let common = format!("IS={is} N={n} TNOM={tnom} VP={vp}");
                    let adjusted_model = format!("{common} TT={tt} TTT1={ttt1} TTT2={ttt2}");
                    let explicit_model = format!("{common} TT={expected_tt}");
                    let fixed_model = format!("{common} TT={tt}");
                    let mut actual = make_solver(adjusted_model);
                    let mut explicit = make_solver(explicit_model);
                    let mut fixed = make_solver(fixed_model);

                    for solver in [&mut actual, &mut explicit, &mut fixed] {
                        let drive = solver
                            .voltage_source_index("Vdrive")
                            .expect("drive source exists");
                        solver.set_voltage_source(drive, 1.0);
                        solver.advance(h).expect("diode transient step solves");
                    }

                    let out = actual.circuit().node("out").expect("output node exists");
                    let explicit_out = explicit
                        .circuit()
                        .node("out")
                        .expect("explicit output node exists");
                    let fixed_out = fixed
                        .circuit()
                        .node("out")
                        .expect("fixed output node exists");
                    let actual_voltage = actual.node_voltage(out);
                    let explicit_voltage = explicit.node_voltage(explicit_out);
                    assert!(
                        (actual_voltage - explicit_voltage).abs() < 1e-8,
                        "{integration:?} VP={vp} TEMP={temperature}: expected explicit TT={expected_tt} s voltage {explicit_voltage} V, got {actual_voltage} V"
                    );
                    assert!(
                        relative(
                            source_current(&actual, "Vdrive"),
                            source_current(&explicit, "Vdrive")
                        ) < 1e-6,
                        "{integration:?} VP={vp} TEMP={temperature}: explicit-TT drive currents differ"
                    );

                    if temperature > tnom {
                        let effect = (actual_voltage - fixed.node_voltage(fixed_out)).abs();
                        assert!(
                            effect > 1e-7,
                            "{integration:?} VP={vp}: hot TTT1/TTT2 must change the solved transient, difference={effect} V"
                        );
                        hot_effects.push(effect);
                    }
                }
            }
        }
        assert!(hot_effects.iter().all(|effect| *effect > 1e-7));
    }

    /// `TM1`/`TM2` scale the depletion grading coefficient `M` at `TEMP`.
    /// A reverse-biased junction transient must match the same circuit with
    /// the independently calculated `M(T)` entered directly on the model.
    #[test]
    fn diode_grading_temperature_coefficients_change_solved_depletion_charge() {
        let (is, n, tnom, cjo, vj, m) = (1e-30, 1.0, 27.0, 10e-9, 0.75, 0.5);
        let (tm1, tm2) = (8e-4, 4e-6);
        let (resistance, h) = (1e3, 2e-6);
        let adjusted_m = |temperature: f64| {
            let delta = temperature - tnom;
            m * (1.0 + tm1 * delta + tm2 * delta * delta)
        };
        let mut hot_effects = Vec::new();

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            for temperature in [tnom, 127.0] {
                let expected_m = adjusted_m(temperature);
                let (expected_cjo, expected_vj) =
                    spice_diode_depletion_parameters(cjo, vj, expected_m, tnom, temperature);
                let make_solver = |model: String, instance_temperature: bool| {
                    let temperature = if instance_temperature {
                        format!(" TEMP={temperature}")
                    } else {
                        String::new()
                    };
                    let netlist = format!(
                        "Vdrive drive 0 dc 0\nRdrive drive out {resistance}\n\
                         D1 out 0 DMOD{temperature}\n.model DMOD D({model})\n"
                    );
                    Solver::new(
                        parse_netlist(&netlist).expect("diode depletion-temperature deck parses"),
                        integration,
                    )
                    .expect("diode depletion-temperature solver builds")
                };
                let common = format!("IS={is} N={n} TNOM={tnom} CJO={cjo} VJ={vj} FC=0.5");
                let adjusted_model = format!("{common} M={m} TM1={tm1} TM2={tm2}");
                let explicit_model = format!(
                    "IS={is} N={n} TNOM={tnom} CJO={expected_cjo} VJ={expected_vj} FC=0.5 M={expected_m}"
                );
                let fixed_model = format!("{common} M={m}");
                let mut actual = make_solver(adjusted_model, true);
                let mut explicit = make_solver(explicit_model, false);
                let mut fixed = make_solver(fixed_model, true);
                let actual_model = actual.circuit().diodes[0].operating_model;
                assert!(
                    (actual_model.cjo - expected_cjo).abs() < 1e-20
                        && (actual_model.vj - expected_vj).abs() < 1e-14,
                    "TEMP={temperature}: expected operating CJO={expected_cjo}, VJ={expected_vj}; got CJO={}, VJ={}",
                    actual_model.cjo,
                    actual_model.vj
                );

                for solver in [&mut actual, &mut explicit, &mut fixed] {
                    let drive = solver
                        .voltage_source_index("Vdrive")
                        .expect("drive source exists");
                    solver.set_voltage_source(drive, -1.0);
                    solver
                        .advance(h)
                        .expect("depletion-charge transient solves");
                }

                let out = actual.circuit().node("out").expect("output node exists");
                let explicit_out = explicit
                    .circuit()
                    .node("out")
                    .expect("explicit output node exists");
                let fixed_out = fixed
                    .circuit()
                    .node("out")
                    .expect("fixed output node exists");
                let actual_voltage = actual.node_voltage(out);
                let explicit_voltage = explicit.node_voltage(explicit_out);
                assert!(
                    (actual_voltage - explicit_voltage).abs() < 1e-8,
                    "{integration:?} TEMP={temperature}: expected explicit M={expected_m} voltage {explicit_voltage} V, got {actual_voltage} V"
                );
                assert!(
                    relative(
                        source_current(&actual, "Vdrive"),
                        source_current(&explicit, "Vdrive")
                    ) < 1e-6,
                    "{integration:?} TEMP={temperature}: explicit-M drive currents differ"
                );
                if temperature > tnom {
                    let effect = (actual_voltage - fixed.node_voltage(fixed_out)).abs();
                    assert!(
                        effect > 1e-6,
                        "{integration:?}: hot TM1/TM2 must change transient depletion charge, difference={effect} V"
                    );
                    hot_effects.push(effect);
                }
            }
        }
        assert!(hot_effects.iter().all(|effect| *effect > 1e-6));
    }

    /// `TEMP` adjusts the diode junction potential and zero-bias capacitance
    /// used by depletion charge. Compare the transient solution with a
    /// separate nominal-temperature circuit whose CJO/VJ are set from the
    /// ngspice temperature equations, and require a measurable change from the
    /// unadjusted junction at hot and cold temperatures.
    #[test]
    fn diode_depletion_capacitance_and_potential_follow_instance_temperature() {
        let (is, n, tnom_c, cjo, vj, m, fc) = (1e-30, 1.0, 26.85, 10e-9, 0.75, 0.65, 0.5);
        let mut temperature_effects = Vec::new();

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            for temperature in [-40.0, tnom_c, 125.0] {
                let (expected_cjo, expected_vj) =
                    spice_diode_depletion_parameters(cjo, vj, m, tnom_c, temperature);
                let make_solver = |device_temp: Option<f64>, depletion_model: String| {
                    let temperature = device_temp
                        .map(|value| format!(" TEMP={value}"))
                        .unwrap_or_default();
                    let netlist = format!(
                        "Vdrive drive 0 dc 0\nRdrive drive out 1k\n\
                         D1 out 0 DMOD{temperature}\n.model DMOD D({depletion_model})\n"
                    );
                    Solver::new(
                        parse_netlist(&netlist).expect("diode depletion-temperature deck parses"),
                        integration,
                    )
                    .expect("diode depletion-temperature solver builds")
                };
                let common = format!("IS={is} N={n} TNOM={tnom_c} M={m} FC={fc}");
                let adjusted_model = format!("{common} CJO={cjo} VJ={vj}");
                let explicit_model = format!("{common} CJO={expected_cjo} VJ={expected_vj}");
                let mut actual = make_solver(Some(temperature), adjusted_model);
                let mut explicit = make_solver(None, explicit_model);
                let mut nominal = make_solver(None, format!("{common} CJO={cjo} VJ={vj}"));

                for solver in [&mut actual, &mut explicit, &mut nominal] {
                    let drive = solver
                        .voltage_source_index("Vdrive")
                        .expect("drive source exists");
                    solver.set_voltage_source(drive, -1.0);
                    solver
                        .advance(2e-6)
                        .expect("depletion temperature transient solves");
                }

                let out = actual.circuit().node("out").expect("output node exists");
                let explicit_out = explicit
                    .circuit()
                    .node("out")
                    .expect("explicit output node exists");
                let nominal_out = nominal
                    .circuit()
                    .node("out")
                    .expect("nominal output node exists");
                let actual_voltage = actual.node_voltage(out);
                let explicit_voltage = explicit.node_voltage(explicit_out);
                assert!(
                    (actual_voltage - explicit_voltage).abs() < 1e-8,
                    "{integration:?} TEMP={temperature}: expected explicit CJO={expected_cjo} F, VJ={expected_vj} V voltage {explicit_voltage} V, got {actual_voltage} V"
                );
                assert!(
                    relative(
                        source_current(&actual, "Vdrive"),
                        source_current(&explicit, "Vdrive")
                    ) < 1e-6,
                    "{integration:?} TEMP={temperature}: explicit CJO/VJ drive currents differ"
                );
                if (temperature - tnom_c).abs() > 1.0 {
                    let effect = (actual_voltage - nominal.node_voltage(nominal_out)).abs();
                    assert!(
                        effect > 1e-5,
                        "{integration:?} TEMP={temperature}: CJO/VJ temperature law must change solved depletion charge, difference={effect} V"
                    );
                    temperature_effects.push(effect);
                }
            }
        }
        assert!(temperature_effects.iter().all(|effect| *effect > 1e-5));
    }

    /// Depletion charge and transit charge share the diode's transient branch
    /// in MNA. Check their combined current against an independently integrated
    /// charge law while the junction is behind the model's series resistance.
    #[test]
    fn diode_depletion_and_transit_charge_combine_through_series_resistance() {
        let (is, tt, cjo, vj, grading, fc, rs, source_r, h) =
            (1e-14, 100e-9, 50e-12, 0.75, 0.5, 0.4, 47.0, 1_000.0, 1e-6);
        let circuit = parse_netlist(&format!(
            "V1 in 0 dc 0\nR1 in a {source_r}\nD1 a 0 DMOD\n\
             .model DMOD D(IS={is} N=1 RS={rs} CJO={cjo} VJ={vj} M={grading} FC={fc} TT={tt})\n"
        ))
        .expect("diode depletion and transit model parses");

        // This is the SPICE depletion-charge integral, written out here rather
        // than calling the device helper under test. The quadratic continuation
        // above FC·VJ keeps both charge and capacitance continuous.
        let depletion_charge = |voltage: f64| {
            let transition = fc * vj;
            let x_transition: f64 = 1.0 - fc;
            let q_transition =
                cjo * vj / (1.0 - grading) * (1.0 - x_transition.powf(1.0 - grading));
            let c_transition = cjo * x_transition.powf(-grading);
            if voltage < transition {
                let x = 1.0 - voltage / vj;
                cjo * vj / (1.0 - grading) * (1.0 - x.powf(1.0 - grading))
            } else {
                let dv = voltage - transition;
                let slope = grading * cjo * x_transition.powf(-1.0 - grading) / vj;
                q_transition + c_transition * dv + 0.5 * slope * dv * dv
            }
        };
        let junction_current =
            |voltage: f64| is * ((voltage / VT).exp() - 1.0) + device::GMIN * voltage;
        let charge = |voltage: f64| {
            let transport_current = is * ((voltage / VT).exp() - 1.0);
            depletion_charge(voltage) + tt * transport_current
        };

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let mut solver =
                Solver::new(circuit.clone(), integration).expect("diode charge solver builds");
            let external = solver.circuit().node("a").expect("external anode");
            let junction = solver
                .circuit()
                .node("D1#internal")
                .expect("RS creates the internal junction node")
                .expect("junction is not ground");
            let source = solver.voltage_source_index("V1").expect("source exists");
            let mut previous_charge = charge(solver.node_voltage(Some(junction)));
            let mut previous_charge_current = 0.0;

            for (step, supply) in [0.9, 0.9].into_iter().enumerate() {
                solver.set_voltage_source(source, supply);
                // Source changes and operating-point startup use BE even when
                // trapezoidal integration is selected; the next step exercises
                // its stored displacement-current history.
                let rule = if step == 0 || integration == Integration::BackwardEuler {
                    Integration::BackwardEuler
                } else {
                    Integration::Trapezoidal
                };
                let charge_current = |voltage: f64| {
                    let next_charge = charge(voltage);
                    match rule {
                        Integration::BackwardEuler => (next_charge - previous_charge) / h,
                        Integration::Trapezoidal => {
                            2.0 * (next_charge - previous_charge) / h - previous_charge_current
                        }
                    }
                };
                let residual = |voltage: f64| {
                    (supply - voltage) / (source_r + rs)
                        - junction_current(voltage)
                        - charge_current(voltage)
                };
                let mut low = 0.0;
                let mut high = supply;
                for _ in 0..120 {
                    let mid = 0.5 * (low + high);
                    if residual(mid) > 0.0 {
                        low = mid;
                    } else {
                        high = mid;
                    }
                }
                let expected_junction = 0.5 * (low + high);

                solver
                    .advance(h)
                    .expect("combined diode charge step solves");
                let actual_junction = solver.node_voltage(Some(junction));
                let actual_current = source_current(&solver, "V1");
                assert!(
                    (actual_junction - expected_junction).abs() < 1e-8,
                    "{integration:?} step {step}: combined depletion/transit charge predicts {expected_junction} V at the junction, got {actual_junction} V"
                );
                assert!(
                    (solver.node_voltage(external) - actual_junction - actual_current * rs).abs()
                        < 1e-8,
                    "{integration:?} step {step}: diode RS must drop {actual_current}·{rs} V between the external and internal anodes"
                );
                assert!(
                    (actual_current - junction_current(actual_junction)
                        - charge_current(actual_junction))
                    .abs()
                        < 1e-9,
                    "{integration:?} step {step}: source current must supply conduction plus both stored-charge currents"
                );
                assert!(
                    depletion_charge(actual_junction) > 1e-12
                        && tt
                            * (junction_current(actual_junction) - device::GMIN * actual_junction)
                            > 1e-12,
                    "the solved point must carry measurable depletion and transit charge"
                );

                let next_charge = charge(actual_junction);
                previous_charge_current = match rule {
                    Integration::BackwardEuler => (next_charge - previous_charge) / h,
                    Integration::Trapezoidal => {
                        2.0 * (next_charge - previous_charge) / h - previous_charge_current
                    }
                };
                previous_charge = next_charge;
            }
        }
    }

    /// The diffusion charge must use the same high-injection-limited junction
    /// current as the diode's conductive stamp. Exercise the interaction in a
    /// solved transient circuit: an ideal `TT·I` charge would produce a
    /// measurably different node voltage once the base current is well above
    /// `IK`.
    #[test]
    fn diode_high_injection_scales_transit_charge_in_transient_steps() {
        let (is, ik, tt, resistance, h) = (1e-14, 100e-6, 100e-9, 1e3, 1e-6);
        let circuit = parse_netlist(&format!(
            "V1 in 0 dc 0\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS={is} N=1 IK={ik} TT={tt})\n"
        ))
        .expect("high-injection diode with transit charge parses");

        // Independent forward-current transform from the SPICE IK equation.
        // GMIN is outside the high-injection correction and carries no stored
        // charge, so it is included in I(V) but excluded from Q(V).
        let base_current = |voltage: f64| is * ((voltage / VT).exp() - 1.0);
        let current = |voltage: f64| {
            let base = base_current(voltage);
            base / (1.0 + (base / ik).sqrt()) + device::GMIN * voltage
        };
        let charge = |voltage: f64| {
            let base = base_current(voltage);
            tt * base / (1.0 + (base / ik).sqrt())
        };

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let mut solver =
                Solver::new(circuit.clone(), integration).expect("transit solver builds");
            let out = solver.circuit().node("out").expect("output node");
            let source = solver.voltage_source_index("V1").expect("source exists");
            solver.set_voltage_source(source, 2.0);

            let mut previous_charge = charge(solver.node_voltage(out));
            let mut previous_current = 0.0;
            for step in 0..2 {
                let rule = if step == 0 {
                    Integration::BackwardEuler
                } else {
                    integration
                };
                let residual = |voltage: f64| {
                    let next_charge = charge(voltage);
                    let charge_current = match rule {
                        Integration::BackwardEuler => (next_charge - previous_charge) / h,
                        Integration::Trapezoidal => {
                            2.0 * (next_charge - previous_charge) / h - previous_current
                        }
                    };
                    (2.0 - voltage) / resistance - current(voltage) - charge_current
                };
                let mut low = 0.0;
                let mut high = 2.0;
                for _ in 0..100 {
                    let mid = 0.5 * (low + high);
                    if residual(mid) > 0.0 {
                        low = mid;
                    } else {
                        high = mid;
                    }
                }
                let expected = 0.5 * (low + high);
                solver
                    .advance(h)
                    .expect("high-injection transit-charge step solves");
                let actual = solver.node_voltage(out);
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "{integration:?} step {step}: expected {expected} V, got {actual} V"
                );

                let next_charge = charge(actual);
                previous_current = (next_charge - previous_charge) / h;
                previous_charge = next_charge;
                assert!(
                    base_current(actual) / ik > 10.0,
                    "the solved junction must be in high injection; V={actual} V"
                );
            }
        }
    }

    /// `VP` makes the transit charge a first-order state. This step-by-step
    /// scalar solve checks its implicit BE/trapezoidal equation through a
    /// forward-to-reverse transition, where the stored charge produces a
    /// reverse current instead of disappearing in one step.
    #[test]
    fn diode_vp_soft_recovery_matches_the_charge_state_equation() {
        let (is, tt, vp, resistance, h) = (1e-14, 100e-9, 0.2, 1e3, 10e-9);
        let circuit = parse_netlist(&format!(
            "V1 in 0 dc 0\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS={is} N=1 TT={tt} VP={vp})\n"
        ))
        .expect("soft-recovery diode netlist parses");

        let step = |voltage: f64, previous_state: f64, previous_rate: f64, rule: Integration| {
            let diode = device::diode_op(voltage, is, 1.0);
            let tau = tt * vp;
            let (state, rate) = match rule {
                Integration::BackwardEuler => {
                    let ratio = tau / h;
                    let state = (-tt * diode.id + ratio * previous_state) / (1.0 + ratio);
                    (state, (state - previous_state) / h)
                }
                Integration::Trapezoidal => {
                    let ratio = tau / h;
                    let state =
                        (-tt * diode.id + 2.0 * ratio * previous_state + tau * previous_rate)
                            / (1.0 + 2.0 * ratio);
                    (state, 2.0 * (state - previous_state) / h - previous_rate)
                }
            };
            (state, rate, -(1.0 - vp) * rate)
        };

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let mut solver =
                Solver::new(circuit.clone(), integration).expect("soft-recovery solver builds");
            let out = solver.circuit().node("out").expect("output node");
            let source = solver.voltage_source_index("V1").expect("source exists");
            let mut previous_state = -tt * device::diode_op(solver.node_voltage(out), is, 1.0).id;
            let mut previous_rate = 0.0;

            for (step_index, supply) in [1.0, 1.0, 1.0, 1.0, -1.0, -1.0, -1.0]
                .into_iter()
                .enumerate()
            {
                solver.set_voltage_source(source, supply);
                let rule = if step_index == 0 || step_index == 4 {
                    Integration::BackwardEuler
                } else {
                    integration
                };
                let residual = |voltage: f64| {
                    let (_, _, recovery_current) =
                        step(voltage, previous_state, previous_rate, rule);
                    (supply - voltage) / resistance
                        - device::diode_op(voltage, is, 1.0).id
                        - recovery_current
                };
                let mut low = supply - 10.0;
                let mut high = supply + 10.0;
                for _ in 0..100 {
                    let mid = 0.5 * (low + high);
                    if residual(mid) > 0.0 {
                        low = mid;
                    } else {
                        high = mid;
                    }
                }
                let expected = 0.5 * (low + high);

                solver
                    .advance(h)
                    .expect("soft-recovery transient step solves");
                let actual = solver.node_voltage(out);
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "{integration:?} step {step_index}: expected {expected} V, got {actual} V"
                );
                if step_index == 4 {
                    let reverse_current = (supply - actual) / resistance;
                    assert!(
                        reverse_current < -1e-4,
                        "stored forward charge should drive reverse current, got {reverse_current} A"
                    );
                }
                let (next_state, next_rate, _) = step(actual, previous_state, previous_rate, rule);
                previous_state = next_state;
                previous_rate = next_rate;
            }
        }
    }

    #[test]
    fn diode_cjo_integrates_in_backward_euler_and_trapezoidal_steps() {
        let cjo = 10e-9;
        let resistance = 1e3;
        let h = 1e-6;
        let ratio = resistance * cjo / h;
        let netlist = format!(
            "V1 in 0 dc 0\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS=1e-30 CJO={cjo} VJ=1 M=0)\n"
        );

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let circuit = parse_netlist(&netlist).expect("diode capacitor circuit parses");
            let mut solver = Solver::new(circuit, integration).expect("solver builds");
            let out = solver.circuit().node("out").expect("out node");
            let source = solver.voltage_source_index("V1").expect("source exists");
            solver.set_voltage_source(source, 1.0);
            solver.advance(h).expect("first transient step solves");
            let v1 = solver.node_voltage(out);
            let expected_v1 = 1.0 / (1.0 + ratio);
            assert!(
                (v1 - expected_v1).abs() < 1e-8,
                "first BE restart step: {v1}"
            );

            solver.advance(h).expect("second transient step solves");
            let expected_v2 = match integration {
                Integration::BackwardEuler => (1.0 + ratio * expected_v1) / (1.0 + ratio),
                Integration::Trapezoidal => (1.0 + 3.0 * ratio * expected_v1) / (1.0 + 2.0 * ratio),
            };
            let v2 = solver.node_voltage(out);
            assert!(
                (v2 - expected_v2).abs() < 1e-8,
                "{integration:?} second step: expected {expected_v2} V, got {v2} V"
            );
        }
    }

    /// Depletion and diffusion charge are two parts of the same stored
    /// junction charge. Exercise their sum in a solved transient circuit,
    /// including the forward-bias continuation above `FC·VJ`, under both
    /// integration rules.
    #[test]
    fn diode_depletion_and_transit_charge_sum_across_forward_transition() {
        let (is, cjo, vj, m, fc, tt, resistance, h): (f64, f64, f64, f64, f64, f64, f64, f64) =
            (1e-14, 100e-12, 0.8, 0.5, 0.5, 100e-9, 1e3, 100e-9);
        let netlist = format!(
            "V1 in 0 dc 0\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS={is} CJO={cjo} VJ={vj} M={m} FC={fc} TT={tt})\n"
        );
        let circuit = parse_netlist(&netlist).expect("combined diode charge model parses");
        let v_transition = fc * vj;

        // Independent SPICE charge integral with its linear continuation
        // above FC·VJ. Diffusion storage excludes the solver's GMIN shunt.
        let xfc = 1.0 - fc;
        let qfc = cjo * vj / (1.0 - m) * (1.0 - xfc.powf(1.0 - m));
        let cfc = cjo * xfc.powf(-m);
        let slope = m * cjo * xfc.powf(-1.0 - m) / vj;
        let depletion_charge = |voltage: f64| {
            if voltage < v_transition {
                let x = 1.0 - voltage / vj;
                cjo * vj / (1.0 - m) * (1.0 - x.powf(1.0 - m))
            } else {
                let dv = voltage - v_transition;
                qfc + cfc * dv + 0.5 * slope * dv * dv
            }
        };
        let junction_current = |voltage: f64| is * ((voltage / VT).exp() - 1.0);
        let conduction_current = |voltage: f64| junction_current(voltage) + 1e-12 * voltage;
        let stored_charge =
            |voltage: f64| depletion_charge(voltage) + tt * junction_current(voltage);

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let mut solver = Solver::new(circuit.clone(), integration)
                .expect("combined diode charge solver builds");
            let out = solver.circuit().node("out").expect("output node");
            let source = solver.voltage_source_index("V1").expect("source exists");
            solver.set_voltage_source(source, 1.0);

            let mut previous_charge = 0.0;
            let mut previous_current = 0.0;
            for step in 0..2 {
                let charge_current = |voltage: f64| {
                    let next_charge = stored_charge(voltage);
                    if step == 0 || integration == Integration::BackwardEuler {
                        (next_charge - previous_charge) / h
                    } else {
                        2.0 * (next_charge - previous_charge) / h - previous_current
                    }
                };
                let residual = |voltage: f64| {
                    (1.0 - voltage) / resistance
                        - conduction_current(voltage)
                        - charge_current(voltage)
                };
                let mut lo = 0.0;
                let mut hi = 1.0;
                for _ in 0..100 {
                    let mid = 0.5 * (lo + hi);
                    if residual(mid) > 0.0 {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                let expected = 0.5 * (lo + hi);

                solver
                    .advance(h)
                    .expect("combined diode charge step solves");
                let actual = solver.node_voltage(out);
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "{integration:?} step {step}: expected {expected} V, got {actual} V"
                );
                if step == 0 {
                    assert!(
                        actual > v_transition,
                        "the first step must enter the forward continuation above FC·VJ={v_transition} V, got {actual} V"
                    );
                }

                let next_charge = stored_charge(actual);
                previous_current = (next_charge - previous_charge) / h;
                previous_charge = next_charge;
            }
        }
    }

    /// Depletion charge and soft reverse recovery belong to the intrinsic
    /// diode junction, behind model-card `RS`. Check the solved junction
    /// voltage and terminal current against a scalar transient equation, then
    /// compare `RS` with an explicit resistor for both companion rules. The
    /// reverse step also has to retain a recovery current that an uncharged
    /// diode cannot produce.
    #[test]
    fn diode_charge_and_recovery_match_series_resistors_during_transients() {
        let (is, rs, rdrive, cjo, vj, m, fc, tt, vp, h): (
            f64,
            f64,
            f64,
            f64,
            f64,
            f64,
            f64,
            f64,
            f64,
            f64,
        ) = (
            1e-14, 47.0, 100.0, 100e-12, 0.8, 0.5, 0.5, 100e-9, 0.2, 10e-9,
        );
        let charged = format!("CJO={cjo} VJ={vj} M={m} FC={fc} TT={tt} VP={vp}");
        let modelled = format!(
            "Vdrive in 0 dc 0\nRdrive in a {rdrive}\nD1 a 0 DMOD\n\
             .model DMOD D(IS={is} RS={rs} {charged})\n"
        );
        let explicit = format!(
            "Vdrive in 0 dc 0\nRdrive in a {rdrive}\nRseries a j {rs}\nD1 j 0 DMOD\n\
             .model DMOD D(IS={is} {charged})\n"
        );
        let uncharged = format!(
            "Vdrive in 0 dc 0\nRdrive in a {rdrive}\nD1 a 0 DMOD\n\
             .model DMOD D(IS={is} RS={rs})\n"
        );

        // Independent SPICE depletion-charge integral, including its linear
        // continuation above FC·VJ. VP moves diffusion storage into a first-
        // order recovery state; its current is kept separate from depletion
        // current just as the model equations specify.
        let v_transition = fc * vj;
        let xfc = 1.0 - fc;
        let qfc = cjo * vj / (1.0 - m) * (1.0 - xfc.powf(1.0 - m));
        let cfc = cjo * xfc.powf(-m);
        let slope = m * cjo * xfc.powf(-1.0 - m) / vj;
        let depletion_charge = |voltage: f64| {
            if voltage < v_transition {
                let x = 1.0 - voltage / vj;
                cjo * vj / (1.0 - m) * (1.0 - x.powf(1.0 - m))
            } else {
                let dv = voltage - v_transition;
                qfc + cfc * dv + 0.5 * slope * dv * dv
            }
        };
        let junction_current = |voltage: f64| device::diode_op(voltage, is, 1.0).id;

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let mut model = Solver::new(
                parse_netlist(&modelled).expect("model-card diode resistance parses"),
                integration,
            )
            .expect("model-card diode solver builds");
            let mut series = Solver::new(
                parse_netlist(&explicit).expect("explicit diode resistance parses"),
                integration,
            )
            .expect("explicit-resistor diode solver builds");
            let mut no_charge = Solver::new(
                parse_netlist(&uncharged).expect("uncharged diode parses"),
                integration,
            )
            .expect("uncharged diode solver builds");

            let model_external = model.circuit().node("a").expect("model external node");
            let model_junction = model
                .circuit()
                .node("D1#internal")
                .expect("model-card RS creates the junction node");
            let series_external = series.circuit().node("a").expect("series external node");
            let series_junction = series.circuit().node("j").expect("explicit junction node");
            let no_charge_external = no_charge
                .circuit()
                .node("a")
                .expect("uncharged external node");
            // Settle all three at the same zero-bias operating point before
            // applying the waveform, so the comparison covers transient
            // charge rather than initialization order.
            model.advance(h).expect("model diode initial step settles");
            series
                .advance(h)
                .expect("explicit diode initial step settles");
            no_charge
                .advance(h)
                .expect("uncharged diode initial step settles");

            let mut previous_depletion_charge = depletion_charge(0.0);
            let mut previous_depletion_current = 0.0;
            let mut previous_recovery_state = -tt * junction_current(0.0);
            let mut previous_recovery_rate = 0.0;
            for (step, supply) in [2.0, 2.0, 2.0, 2.0, -1.0, -1.0, -1.0]
                .into_iter()
                .enumerate()
            {
                let rule = if step == 0 || step == 4 {
                    Integration::BackwardEuler
                } else {
                    integration
                };
                let old_depletion_charge = previous_depletion_charge;
                let old_depletion_current = previous_depletion_current;
                let old_recovery_state = previous_recovery_state;
                let old_recovery_rate = previous_recovery_rate;
                let depletion_current = |voltage: f64| {
                    let charge = depletion_charge(voltage);
                    match rule {
                        Integration::BackwardEuler => (charge - old_depletion_charge) / h,
                        Integration::Trapezoidal => {
                            2.0 * (charge - old_depletion_charge) / h - old_depletion_current
                        }
                    }
                };
                let recovery = |voltage: f64| {
                    let tau = tt * vp;
                    let current = junction_current(voltage);
                    let (state, rate) = match rule {
                        Integration::BackwardEuler => {
                            let ratio = tau / h;
                            let state =
                                (-tt * current + ratio * old_recovery_state) / (1.0 + ratio);
                            (state, (state - old_recovery_state) / h)
                        }
                        Integration::Trapezoidal => {
                            let ratio = tau / h;
                            let state = (-tt * current
                                + 2.0 * ratio * old_recovery_state
                                + tau * old_recovery_rate)
                                / (1.0 + 2.0 * ratio);
                            (
                                state,
                                2.0 * (state - old_recovery_state) / h - old_recovery_rate,
                            )
                        }
                    };
                    (state, rate, -(1.0 - vp) * rate)
                };
                let residual = |voltage: f64| {
                    supply
                        - voltage
                        - (rdrive + rs)
                            * (junction_current(voltage)
                                + depletion_current(voltage)
                                + recovery(voltage).2)
                };
                let mut low = supply - 10.0;
                let mut high = supply + 10.0;
                for _ in 0..100 {
                    let mid = 0.5 * (low + high);
                    if residual(mid) > 0.0 {
                        low = mid;
                    } else {
                        high = mid;
                    }
                }
                let expected_junction = 0.5 * (low + high);
                let expected_current = (supply - expected_junction) / (rdrive + rs);
                let expected_external = supply - expected_current * rdrive;

                for solver in [&mut model, &mut series, &mut no_charge] {
                    solver.set_voltage_source(
                        solver
                            .voltage_source_index("Vdrive")
                            .expect("source exists"),
                        supply,
                    );
                    solver.advance(h).expect("diode charge step solves");
                }

                let actual_junction = model.node_voltage(model_junction);
                let actual_external = model.node_voltage(model_external);
                let actual_current = source_current(&model, "Vdrive");
                assert!(
                    (actual_junction - expected_junction).abs() < 1e-8,
                    "{integration:?} step {step}: scalar charge equation predicts junction {expected_junction} V, got {actual_junction} V"
                );
                assert!(
                    (actual_external - expected_external).abs() < 1e-8,
                    "{integration:?} step {step}: scalar equation predicts external anode {expected_external} V, got {actual_external} V"
                );
                assert!(
                    (actual_current - expected_current).abs()
                        < 1e-10 + 1e-6 * expected_current.abs(),
                    "{integration:?} step {step}: scalar equation predicts terminal current {expected_current} A, got {actual_current} A"
                );

                for (label, actual, equivalent) in [
                    (
                        "external anode",
                        actual_external,
                        series.node_voltage(series_external),
                    ),
                    (
                        "junction",
                        actual_junction,
                        series.node_voltage(series_junction),
                    ),
                ] {
                    assert!(
                        (actual - equivalent).abs() < 1e-8,
                        "{integration:?} step {step} {label}: model-card RS gives {actual} V, explicit resistor gives {equivalent} V"
                    );
                }
                assert!(
                    (actual_current - source_current(&series, "Vdrive")).abs() < 1e-10,
                    "{integration:?} step {step}: model-card RS and explicit resistor terminal currents differ"
                );

                if step == 0 {
                    let charge_effect =
                        (actual_external - no_charge.node_voltage(no_charge_external)).abs();
                    assert!(
                        charge_effect > 1e-7,
                        "forward depletion/transit charge must affect the external anode, got {charge_effect} V"
                    );
                    assert!(
                        actual_junction > v_transition,
                        "the forward step must enter the depletion continuation above {v_transition} V, got {actual_junction} V"
                    );
                }
                if step == 4 {
                    let uncharged_current = source_current(&no_charge, "Vdrive");
                    assert!(
                        actual_current < uncharged_current - 1e-4,
                        "stored forward charge must drive a measurable reverse-recovery current: charged {actual_current} A, uncharged {uncharged_current} A"
                    );
                }

                previous_depletion_charge = depletion_charge(actual_junction);
                previous_depletion_current = depletion_current(actual_junction);
                let (state, rate, _) = recovery(actual_junction);
                previous_recovery_state = state;
                previous_recovery_rate = rate;
            }
        }
    }

    #[test]
    fn bjt_cje_integrates_as_base_emitter_charge_for_both_polarities() {
        let cje = 10e-9;
        let resistance = 1e3;
        let h = 1e-6;
        let ratio = resistance * cje / h;

        for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
            let netlist = format!(
                "V1 in 0 dc 0\nR1 in b {resistance}\nVc c 0 dc 0\n\
                 Q1 c b 0 QMOD\n.model QMOD {kind}(IS=1e-30 CJE={cje} VJE=1 MJE=0 FC=0.5)\n"
            );
            let circuit = parse_netlist(&netlist).expect("BJT junction-cap circuit parses");
            assert_eq!(circuit.bjts[0].model.cje, cje);
            assert_eq!(circuit.bjts[0].model.vje, 1.0);
            assert_eq!(circuit.bjts[0].model.mje, 0.0);
            assert_eq!(circuit.bjts[0].model.cjc, 0.0);

            for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                let mut solver = Solver::new(circuit.clone(), integration).expect("solver builds");
                let base = solver.circuit().node("b").expect("base node");
                let source = solver.voltage_source_index("V1").expect("source exists");
                solver.set_voltage_source(source, sign);
                solver.advance(h).expect("first transient step solves");
                let expected = sign / (1.0 + ratio);
                let actual = solver.node_voltage(base);
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "{kind} {integration:?} base-emitter step: expected {expected} V, got {actual} V"
                );

                solver.advance(h).expect("second transient step solves");
                let expected_second = match integration {
                    Integration::BackwardEuler => {
                        sign * (1.0 + ratio * expected.abs()) / (1.0 + ratio)
                    }
                    Integration::Trapezoidal => {
                        sign * (1.0 + 3.0 * ratio * expected.abs()) / (1.0 + 2.0 * ratio)
                    }
                };
                let actual_second = solver.node_voltage(base);
                assert!(
                    (actual_second - expected_second).abs() < 1e-8,
                    "{kind} {integration:?} second base-emitter step: expected {expected_second} V, got {actual_second} V"
                );
            }
        }
    }

    #[test]
    fn bjt_mje_changes_the_transient_base_emitter_charge() {
        let cje = 10e-9;
        let vje = 0.75;
        let mje = 0.4;
        let fc = 0.5;
        let resistance = 1e3;
        let h = 1e-6;
        let charge =
            |voltage: f64| cje * vje / (1.0 - mje) * (1.0 - (1.0 - voltage / vje).powf(1.0 - mje));
        let mut lo = 0.0;
        let mut hi = fc * vje;
        for _ in 0..80 {
            let mid = 0.5 * (lo + hi);
            if (1.0 - mid) / resistance > charge(mid) / h {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let expected = 0.5 * (lo + hi);

        let mut solver = build(&format!(
            "V1 in 0 dc 0\nR1 in b {resistance}\nVc c 0 dc 0\nQ1 c b 0 QMOD\n\
             .model QMOD NPN(IS=1e-30 CJE={cje} VJE={vje} MJE={mje} FC={fc})\n"
        ));
        let base = solver.circuit().node("b").expect("base node");
        let source = solver.voltage_source_index("V1").expect("source exists");
        solver.set_voltage_source(source, 1.0);
        solver.advance(h).expect("graded B-E depletion step solves");
        let actual = solver.node_voltage(base);
        assert!(
            (actual - expected).abs() < 1e-8,
            "MJE={mje} charge curve should settle at {expected} V, got {actual} V"
        );
    }

    #[test]
    fn bjt_cjc_couples_a_base_step_into_the_collector() {
        let cjc = 10e-9;
        let resistance = 1e3;
        let h = 1e-6;
        let vjc = 0.75;
        let mjc = 0.4;
        let fc = 0.5;
        let charge =
            |voltage: f64| cjc * vjc / (1.0 - mjc) * (1.0 - (1.0 - voltage / vjc).powf(1.0 - mjc));
        let mut lo = 0.0;
        let mut hi = 1.0;
        for _ in 0..80 {
            let mid = 0.5 * (lo + hi);
            if mid / resistance < charge(1.0 - mid) / h {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let expected_magnitude = 0.5 * (lo + hi);

        for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
            let netlist = format!(
                "V1 b 0 dc 0\nR1 c 0 {resistance}\nQ1 c b 0 QMOD\n\
                 .model QMOD {kind}(IS=1e-30 CJC={cjc} VJC={vjc} MJC={mjc} FC={fc})\n"
            );
            let circuit = parse_netlist(&netlist).expect("BJT junction-cap circuit parses");
            assert_eq!(circuit.bjts[0].model.cje, 0.0);
            assert_eq!(circuit.bjts[0].model.cjc, cjc);
            assert_eq!(circuit.bjts[0].model.vjc, vjc);
            assert_eq!(circuit.bjts[0].model.mjc, mjc);

            let mut solver =
                Solver::new(circuit, Integration::BackwardEuler).expect("solver builds");
            let collector = solver.circuit().node("c").expect("collector node");
            let source = solver.voltage_source_index("V1").expect("source exists");
            solver.set_voltage_source(source, sign);
            solver.advance(h).expect("base step couples through CJC");
            let expected = sign * expected_magnitude;
            let actual = solver.node_voltage(collector);
            assert!(
                (actual - expected).abs() < 1e-8,
                "{kind} collector coupling: expected {expected} V, got {actual} V"
            );
        }
    }

    /// BJT instance temperature adjusts both base-junction depletion charges.
    /// The solved driven-base transient must match an independent ngspice
    /// temperature calculation encoded as explicit nominal parameters, for
    /// the default silicon law and `TLEVC=1`'s linear coefficients.
    #[test]
    fn bjt_junction_depletion_temperature_matches_explicit_charge_models() {
        let (tnom, hot) = (40.0, 120.0);
        let (cje, vje, mje) = (80e-12, 0.72, 0.42);
        let (cjc, vjc, mjc) = (50e-12, 0.82, 0.35);
        let (tmje1, tmje2, tmjc1, tmjc2) = (1e-3, 2e-6, 7e-4, 1e-6);
        let (cte, ctc, tvje, tvjc) = (1.5e-3, 1e-3, 4e-4, 5e-4);
        let h = 1e-6;

        let adjusted_junction = |capacitance: f64,
                                 potential: f64,
                                 grading: f64,
                                 grading_tc1: f64,
                                 grading_tc2: f64,
                                 capacitance_tc: f64,
                                 potential_tc: f64,
                                 temperature: f64,
                                 tlevc: u8| {
            let delta = temperature - tnom;
            let grading_at = grading * (1.0 + grading_tc1 * delta + grading_tc2 * delta * delta);
            if delta == 0.0 {
                return (capacitance, potential, grading_at);
            }
            if tlevc == 1 {
                return (
                    capacitance * (1.0 + capacitance_tc * delta),
                    potential - potential_tc * delta,
                    grading_at,
                );
            }

            const TREF: f64 = 300.15;
            const EG_REF: f64 = 1.115_087_7;
            const KB_EV: f64 = 8.617_333_262_145e-5;
            let nominal_k = tnom + 273.15;
            let temperature_k = temperature + 273.15;
            let ratio_nominal = nominal_k / TREF;
            let ratio_temperature = temperature_k / TREF;
            let eg_temperature =
                1.16 - 7.02e-4 * temperature_k * temperature_k / (temperature_k + 1108.0);
            let vt = KB_EV * temperature_k;
            let pbfact = vt
                * (eg_temperature / (KB_EV * temperature_k)
                    - EG_REF / (KB_EV * TREF)
                    - 3.0 * ratio_temperature.ln());
            // ngspice `bjttemp.c` refers the nominal potential back with
            // `pbfact1`, the same factor evaluated at TNOM.
            let eg_nominal = 1.16 - 7.02e-4 * nominal_k * nominal_k / (nominal_k + 1108.0);
            let pbfact1 = KB_EV
                * nominal_k
                * (eg_nominal / (KB_EV * nominal_k)
                    - EG_REF / (KB_EV * TREF)
                    - 3.0 * ratio_nominal.ln());
            let pbo = (potential - pbfact1) / ratio_nominal;
            let gma_old = (potential - pbo) / pbo;
            let cap_temperature =
                capacitance / (1.0 + grading_at * (4e-4 * (nominal_k - TREF) - gma_old));
            let potential_temperature = ratio_temperature * pbo + pbfact;
            let gma_new = (potential_temperature - pbo) / pbo;
            let cap_temperature =
                cap_temperature * (1.0 + grading_at * (4e-4 * (temperature_k - TREF) - gma_new));
            (cap_temperature, potential_temperature, grading_at)
        };

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            for tlevc in [0u8, 1u8] {
                for temperature in [tnom, hot] {
                    let (cje_at, vje_at, mje_at) = adjusted_junction(
                        cje,
                        vje,
                        mje,
                        tmje1,
                        tmje2,
                        cte,
                        tvje,
                        temperature,
                        tlevc,
                    );
                    let (cjc_at, vjc_at, mjc_at) = adjusted_junction(
                        cjc,
                        vjc,
                        mjc,
                        tmjc1,
                        tmjc2,
                        ctc,
                        tvjc,
                        temperature,
                        tlevc,
                    );
                    let make_solver = |kind: &str, temp: Option<f64>, model: String| {
                        let instance_temperature =
                            temp.map_or_else(String::new, |value| format!(" TEMP={value}"));
                        let netlist = format!(
                            "Vdrive drive 0 dc 0\nRdrive drive b 1k\nVc c 0 dc -0.5\n\
                             Q1 c b 0 QMOD{instance_temperature}\n.model QMOD {kind}({model})\n"
                        );
                        Solver::new(
                            parse_netlist(&netlist).expect("BJT junction-temperature deck parses"),
                            integration,
                        )
                        .expect("BJT junction-temperature solver builds")
                    };
                    let base_model = format!(
                        "IS=1e-40 TNOM={tnom} CJE={cje} VJE={vje} MJE={mje} \
                         CJC={cjc} VJC={vjc} MJC={mjc} FC=0.5"
                    );
                    let temperature_model = format!(
                        "{base_model} TLEVC={tlevc} TMJE1={tmje1} TMJE2={tmje2} \
                         TMJC1={tmjc1} TMJC2={tmjc2} CTE={cte} CTC={ctc} TVJE={tvje} TVJC={tvjc}"
                    );
                    let explicit_model = format!(
                        "IS=1e-40 CJE={cje_at} VJE={vje_at} MJE={mje_at} \
                         CJC={cjc_at} VJC={vjc_at} MJC={mjc_at} FC=0.5"
                    );
                    let fixed_model = format!(
                        "{base_model} TLEVC={tlevc} CTE={cte} CTC={ctc} TVJE={tvje} TVJC={tvjc}"
                    );
                    for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
                        let mut actual =
                            make_solver(kind, Some(temperature), temperature_model.clone());
                        let mut explicit = make_solver(kind, None, explicit_model.clone());
                        let mut fixed = make_solver(kind, Some(temperature), fixed_model.clone());
                        for solver in [&mut actual, &mut explicit, &mut fixed] {
                            let drive = solver
                                .voltage_source_index("Vdrive")
                                .expect("base drive source exists");
                            solver.set_voltage_source(drive, sign * 0.8);
                            solver.advance(h).expect("BJT depletion transient solves");
                        }

                        let node_voltage = |solver: &Solver| {
                            let base = solver.circuit().node("b").expect("base node exists");
                            sign * solver.node_voltage(base)
                        };
                        let actual_voltage = node_voltage(&actual);
                        let explicit_voltage = node_voltage(&explicit);
                        assert!(
                            (actual_voltage - explicit_voltage).abs() < 1e-8,
                            "{kind} {integration:?} TLEVC={tlevc} TEMP={temperature}: explicit adjusted CJE/VJE/MJE and CJC/VJC/MJC predict {explicit_voltage} V, got {actual_voltage} V"
                        );
                        if temperature > tnom {
                            let effect = (actual_voltage - node_voltage(&fixed)).abs();
                            assert!(
                                effect > 1e-7,
                                "{kind} {integration:?} TLEVC={tlevc}: TMJE/TMJC and junction temperature coefficients must change the solved transient, difference={effect} V"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn bjt_bias_dependent_transit_charge_matches_spice_and_partials() {
        let circuit = parse_netlist(
            "Q1 c b e QMOD\n\
             .model QMOD NPN(IS=2e-16 NF=1.2 NR=1.1 VAF=40 VAR=70 IKF=2m IKR=5m TF=12n TR=35n XTF=2.5 VTF=0.6 ITF=80n)\n",
        )
        .expect("bias-dependent transit-time model parses");
        let model = circuit.bjts[0].model;
        let (vbe, vbc) = (0.64, 0.35);
        let charge = device::bjt_transit_charge(vbe, vbc, model);
        let vtf = model.nf * VT;
        let vtr = model.nr * VT;
        let cbe = model.is * ((vbe / vtf).exp() - 1.0);
        let cbc = model.is * ((vbc / vtr).exp() - 1.0);
        let qb = (1.0 - vbc / model.vaf - vbe / model.var).recip()
            * (1.0 + (1.0 + 4.0 * (cbe / model.ikf + cbc / model.ikr)).sqrt())
            / 2.0;
        let tf_adjustment = model.xtf * (vbc / model.vtf).exp() * (cbe / (cbe + model.itf)).powi(2);
        assert!((charge.qbe - model.tf * cbe * (1.0 + tf_adjustment) / qb).abs() < 1e-25);
        assert!((charge.qbc - model.tr * cbc).abs() < 1e-25);

        let delta = 1e-5;
        let plus_be = device::bjt_transit_charge(vbe + delta, vbc, model);
        let minus_be = device::bjt_transit_charge(vbe - delta, vbc, model);
        let plus_bc = device::bjt_transit_charge(vbe, vbc + delta, model);
        let minus_bc = device::bjt_transit_charge(vbe, vbc - delta, model);
        let finite_difference = |plus: f64, minus: f64| (plus - minus) / (2.0 * delta);
        for (actual, expected) in [
            (
                charge.dqbe_dvbe,
                finite_difference(plus_be.qbe, minus_be.qbe),
            ),
            (
                charge.dqbe_dvbc,
                finite_difference(plus_bc.qbe, minus_bc.qbe),
            ),
            (
                charge.dqbc_dvbc,
                finite_difference(plus_bc.qbc, minus_bc.qbc),
            ),
        ] {
            assert!(
                (actual - expected).abs() < expected.abs() * 2e-5 + 1e-24,
                "analytic charge derivative {actual} differs from finite difference {expected}"
            );
        }
        assert_eq!(charge.dqbc_dvbe, 0.0);

        let reverse_be = device::bjt_transit_charge(-0.1, vbc, model);
        let expected_reverse_be = model.tf * model.is * ((-0.1 / vtf).exp() - 1.0);
        assert!((reverse_be.qbe - expected_reverse_be).abs() < 1e-25);
    }

    #[test]
    fn bjt_bias_dependent_tf_and_tr_charge_integrate_for_both_polarities_and_rules() {
        let is = 1e-16;
        let beta_f = 100.0;
        let beta_r = 2.0;
        let resistance = 1e3;
        let h = 1e-9;

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
                for forward in [true, false] {
                    let tf = if forward { 10e-9 } else { 0.0 };
                    let xtf = if forward { 2.0 } else { 0.0 };
                    let vtf = 0.8;
                    let itf = 25e-6;
                    let tr = if forward { 0.0 } else { 10e-9 };
                    let (collector_v, emitter_v) = if forward {
                        (sign * 2.0, 0.0)
                    } else {
                        (0.0, sign * 2.0)
                    };
                    let netlist = format!(
                        "Vdrive drive 0 dc 0\nRdrive drive b {resistance}\n\
                         Vc c 0 dc {collector_v}\nVe e 0 dc {emitter_v}\nQ1 c b e QMOD\n\
                         .model QMOD {kind}(IS={is} BF={beta_f} BR={beta_r} TF={tf} TR={tr} XTF={xtf} VTF={vtf} ITF={itf})\n"
                    );
                    let circuit = parse_netlist(&netlist).expect("BJT transit circuit parses");
                    let mut solver =
                        Solver::new(circuit, integration).expect("BJT transit solver builds");
                    let base = solver.circuit().node("b").expect("base node");
                    let source = solver
                        .voltage_source_index("Vdrive")
                        .expect("base drive exists");
                    solver.set_voltage_source(source, sign);

                    let junctions = |v: f64| {
                        if forward {
                            (v, v - 2.0)
                        } else {
                            (v - 2.0, v)
                        }
                    };
                    let charge = |v: f64| {
                        let (vbe, vbc) = junctions(v);
                        let current = is * ((vbe / VT).exp() - 1.0);
                        if forward {
                            tf * current
                                * (1.0
                                    + xtf * (vbc / vtf).exp() * (current / (current + itf)).powi(2))
                        } else {
                            tr * is * ((vbc / VT).exp() - 1.0)
                        }
                    };
                    let base_current = |vbe: f64, vbc: f64| {
                        is / beta_f * ((vbe / VT).exp() - 1.0)
                            + is / beta_r * ((vbc / VT).exp() - 1.0)
                    };
                    let initial_v = sign * solver.node_voltage(base);
                    let mut previous_charge = charge(initial_v);
                    let mut previous_current = 0.0;
                    for step in 0..2 {
                        let charge_current = |v: f64| {
                            let q = charge(v);
                            if step == 0 || integration == Integration::BackwardEuler {
                                (q - previous_charge) / h
                            } else {
                                2.0 * (q - previous_charge) / h - previous_current
                            }
                        };
                        let residual = |v: f64| {
                            let (vbe, vbc) = junctions(v);
                            (1.0 - v) / resistance
                                - base_current(vbe, vbc)
                                - device::GMIN * (vbe + vbc)
                                - charge_current(v)
                        };
                        let mut lo = 0.0;
                        let mut hi = 1.0;
                        for _ in 0..100 {
                            let mid = 0.5 * (lo + hi);
                            if residual(mid) > 0.0 {
                                lo = mid;
                            } else {
                                hi = mid;
                            }
                        }
                        let expected = 0.5 * (lo + hi);
                        solver.advance(h).expect("transit charge step solves");
                        let actual = sign * solver.node_voltage(base);
                        assert!(
                            (actual - expected).abs() < 1e-8,
                            "{kind} {integration:?} {} step {step}: expected {expected} V, got {actual} V",
                            if forward { "TF" } else { "TR" }
                        );
                        let next_charge = charge(actual);
                        previous_current = (next_charge - previous_charge) / h;
                        previous_charge = next_charge;
                    }
                }
            }
        }
    }

    /// BJT depletion and diffusion charge share each junction's transient
    /// companion. Check the complete base charge in both transport directions:
    /// the result must include CJE/CJC and TF/TR together for NPN/PNP, under
    /// both integration rules.
    #[test]
    fn bjt_depletion_and_transit_charge_integrate_together() {
        let (is, beta_f, beta_r) = (1e-14, 100.0, 2.0);
        let capacitance = 220e-12;
        let transit_time = 1e-6;
        let resistance = 1e3;
        let h = 1e-6;
        let drive_voltage = 1.0;

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
                for forward in [true, false] {
                    let (collector_v, emitter_v) = if forward {
                        (sign * 2.0, 0.0)
                    } else {
                        (0.0, sign * 2.0)
                    };
                    let netlist = format!(
                        "Vdrive drive 0 dc 0\nRdrive drive b {resistance}\n\
                         Vc c 0 dc {collector_v}\nVe e 0 dc {emitter_v}\nQ1 c b e QMOD\n\
                         .model QMOD {kind}(IS={is} BF={beta_f} BR={beta_r} CJE={capacitance} CJC={capacitance} VJE=1 MJE=0 VJC=1 MJC=0 TF={transit_time} TR={transit_time})\n"
                    );
                    let circuit = parse_netlist(&netlist).expect("BJT charge model parses");
                    let mut solver =
                        Solver::new(circuit, integration).expect("BJT charge solver builds");
                    let base = solver.circuit().node("b").expect("base node");
                    let source = solver
                        .voltage_source_index("Vdrive")
                        .expect("base drive exists");
                    solver.set_voltage_source(source, sign * drive_voltage);

                    let junctions = |v: f64| {
                        if forward {
                            (v, v - 2.0)
                        } else {
                            (v - 2.0, v)
                        }
                    };
                    // MJE=MJC=0 makes depletion charge exactly C·V. With
                    // VAF/VAR and IKF/IKR omitted, the transit-charge qb is 1.
                    let depletion_charge = |vbe: f64, vbc: f64| capacitance * (vbe + vbc);
                    let transit_charge = |vbe: f64, vbc: f64| {
                        transit_time * is * (((vbe / VT).exp() - 1.0) + ((vbc / VT).exp() - 1.0))
                    };
                    let charge = |v: f64| {
                        let (vbe, vbc) = junctions(v);
                        depletion_charge(vbe, vbc) + transit_charge(vbe, vbc)
                    };
                    let base_current = |vbe: f64, vbc: f64| {
                        is / beta_f * ((vbe / VT).exp() - 1.0)
                            + is / beta_r * ((vbc / VT).exp() - 1.0)
                    };

                    let previous_v = sign * solver.node_voltage(base);
                    let (previous_vbe, previous_vbc) = junctions(previous_v);
                    let mut previous_charge = charge(previous_v);
                    let mut previous_current = 0.0;
                    for step in 0..2 {
                        let charge_current = |v: f64| {
                            let delta = charge(v) - previous_charge;
                            if step == 0 || integration == Integration::BackwardEuler {
                                delta / h
                            } else {
                                2.0 * delta / h - previous_current
                            }
                        };
                        let residual = |v: f64| {
                            let (vbe, vbc) = junctions(v);
                            (drive_voltage - v) / resistance
                                - base_current(vbe, vbc)
                                - device::GMIN * (vbe + vbc)
                                - charge_current(v)
                        };
                        let mut lo = 0.0;
                        let mut hi = drive_voltage;
                        for _ in 0..100 {
                            let mid = 0.5 * (lo + hi);
                            if residual(mid) > 0.0 {
                                lo = mid;
                            } else {
                                hi = mid;
                            }
                        }
                        let expected = 0.5 * (lo + hi);

                        solver.advance(h).expect("combined BJT charge step solves");
                        let actual = sign * solver.node_voltage(base);
                        assert!(
                            (actual - expected).abs() < 1e-8,
                            "{kind} {integration:?} {} step {step}: expected {expected} V, got {actual} V",
                            if forward { "TF/CJE" } else { "TR/CJC" }
                        );

                        let (next_vbe, next_vbc) = junctions(actual);
                        if step == 0 {
                            let depletion_delta = depletion_charge(next_vbe, next_vbc)
                                - depletion_charge(previous_vbe, previous_vbc);
                            let transit_delta = transit_charge(next_vbe, next_vbc)
                                - transit_charge(previous_vbe, previous_vbc);
                            assert!(
                                depletion_delta.abs() > transit_delta.abs() * 0.1
                                    && transit_delta.abs() > depletion_delta.abs() * 0.1,
                                "both depletion and transit charge must affect the first step: ΔQdep={depletion_delta} C, ΔQtransit={transit_delta} C"
                            );
                        }
                        let next_charge = charge(actual);
                        previous_current = (next_charge - previous_charge) / h;
                        previous_charge = next_charge;
                    }
                }
            }
        }
    }

    /// The transient stamp must use the same Early-effect and high-injection
    /// base charge as the BJT transport model. Solve a driven base node with
    /// both junction depletion charge, forward transit charge, and the two
    /// recombination branches active; compare the MNA result with an
    /// independent scalar charge-balance calculation for NPN and PNP.
    #[test]
    fn bjt_transit_charge_uses_early_and_high_injection_base_charge_in_transient() {
        let (is, bf, br, vaf, var, ikf, ikr) = (1e-16, 80.0, 4.0, 35.0, 50.0, 0.2e-3, 0.5e-3);
        let (ise, ne, isc, nc) = (1e-13, 1.5, 2e-13, 2.0);
        let (tf, tr, cje, cjc) = (20e-9, 30e-9, 60e-12, 40e-12);
        let (eg, xti, xtb, tnom_celsius) = (1.11, 3.0, 1.2, 27.0);
        let (drive, resistance, collector, h) = (0.95, 1e3, 3.0, 1e-6);

        for temperature_celsius in [27.0, 110.0] {
            let temperature_kelvin = temperature_celsius + 273.15;
            let nominal_kelvin = tnom_celsius + 273.15;
            let ratio = temperature_kelvin / nominal_kelvin;
            let vt = 1.380_649e-23 * temperature_kelvin / 1.602_176_634e-19;
            let factlog = (ratio - 1.0) * eg / vt + xti * f64::ln(ratio);
            let beta_factor = ratio.powf(xtb);
            let adjusted_is = is * f64::exp(factlog);
            let adjusted_bf = bf * beta_factor;
            let adjusted_br = br * beta_factor;
            let adjusted_ise = ise * f64::exp(factlog / ne) / beta_factor;
            let adjusted_isc = isc * f64::exp(factlog / nc) / beta_factor;

            for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
                    let netlist = format!(
                    "Vdrive drive 0 dc 0\nRdrive drive b {resistance}\n\
                     Vc c 0 dc {}\nVe e 0 dc 0\n\
                     Q1 c b e QMOD TEMP={temperature_celsius}\n\
                     .model QMOD {kind}(IS={is} BF={bf} BR={br} VAF={vaf} VAR={var} IKF={ikf} IKR={ikr} ISE={ise} NE={ne} ISC={isc} NC={nc} TF={tf} TR={tr} CJE={cje} CJC={cjc} VJE=1 MJE=0 VJC=1 MJC=0 TNOM={tnom_celsius} EG={eg} XTI={xti} XTB={xtb})\n",
                    sign * collector,
                );
                    let circuit = parse_netlist(&netlist).expect("combined BJT model parses");
                    let mut solver =
                        Solver::new(circuit, integration).expect("combined BJT solver builds");
                    let base = solver.circuit().node("b").expect("base node");
                    let drive_source = solver
                        .voltage_source_index("Vdrive")
                        .expect("base drive source");
                    let previous_v = sign * solver.node_voltage(base);
                    solver.set_voltage_source(drive_source, sign * drive);

                    let base_width_and_charge = |vbe: f64, vbc: f64| {
                        let forward = (vbe / vt).exp();
                        let reverse = (vbc / vt).exp();
                        let cbe = adjusted_is * (forward - 1.0);
                        let cbc = adjusted_is * (reverse - 1.0);
                        let base_width = 1.0 - vbc / vaf - vbe / var;
                        let q1 = 1.0 / base_width;
                        let q2 = cbe / ikf + cbc / ikr;
                        let qb = q1 * (1.0 + (1.0 + 4.0 * q2).sqrt()) / 2.0;
                        (qb, cbe, cbc, forward, reverse)
                    };
                    let base_current_and_charge = |vbe: f64, vbc: f64| {
                        let (qb, cbe, cbc, forward, reverse) = base_width_and_charge(vbe, vbc);
                        let ibe = (adjusted_is / adjusted_bf) * (forward - 1.0)
                            + adjusted_ise * ((vbe / (ne * vt)).exp() - 1.0);
                        let ibc = (adjusted_is / adjusted_br) * (reverse - 1.0)
                            + adjusted_isc * ((vbc / (nc * vt)).exp() - 1.0);
                        let qbe = if vbe > 0.0 { tf * cbe / qb } else { tf * cbe };
                        let qbc = tr * cbc;
                        let junction_charge = cje * vbe + cjc * vbc;
                        (
                            ibe + ibc,
                            junction_charge + qbe + qbc,
                            qb,
                            qbe,
                            junction_charge,
                        )
                    };
                    let previous_vbc = previous_v - collector;
                    let (_, previous_charge, _, _, _) =
                        base_current_and_charge(previous_v, previous_vbc);
                    let residual = |vbe: f64| {
                        let vbc = vbe - collector;
                        let (base_current, charge, _, _, _) = base_current_and_charge(vbe, vbc);
                        (drive - vbe) / resistance
                            - base_current
                            - device::GMIN * (vbe + vbc)
                            - (charge - previous_charge) / h
                    };
                    let mut lo = 0.0;
                    let mut hi = drive;
                    for _ in 0..100 {
                        let mid = 0.5 * (lo + hi);
                        if residual(mid) > 0.0 {
                            lo = mid;
                        } else {
                            hi = mid;
                        }
                    }
                    let expected = 0.5 * (lo + hi);
                    let (_, _, qb, transit_charge, junction_charge) =
                        base_current_and_charge(expected, expected - collector);
                    assert!(
                        qb > 1.1,
                        "{kind}: selected bias must activate high injection (qb={qb})"
                    );
                    assert!(
                    transit_charge.abs() > 1e-13,
                    "{kind}: transit charge must affect the solved base (QTF={transit_charge} C)"
                );
                    assert!(
                        (junction_charge - previous_charge).abs() > 1e-13,
                        "{kind}: junction depletion charge must affect the transient"
                    );

                    solver
                        .advance(h)
                        .expect("combined BJT transient step solves");
                    let actual = sign * solver.node_voltage(base);
                    assert!(
                    (actual - expected).abs() < 1e-8,
                    "{kind} TEMP={temperature_celsius} °C {integration:?}: Early/high-injection transit charge and junction charge predict {expected} V, got {actual} V"
                );
                }
            }
        }
    }

    /// `TTF1/2` and `TTR1/2` scale the SPICE forward and reverse transit times
    /// with temperature. Compare each transient step to a separate transistor
    /// whose `TF`/`TR` are explicitly set to the independently calculated
    /// values, and confirm the hot result differs from an unscaled model.
    #[test]
    fn bjt_transit_time_temperature_coefficients_match_explicit_charge_models() {
        let (is, bf, br, tnom, tf, tr) = (1e-14, 100.0, 2.0, 27.0, 1e-6, 2e-6);
        let (ttf1, ttf2, ttr1, ttr2) = (1e-3, 1e-5, 2e-3, 1e-5);
        let h = 1e-6;
        let adjusted_times = |temperature: f64| {
            let delta = temperature - tnom;
            (
                tf * (1.0 + ttf1 * delta + ttf2 * delta * delta),
                tr * (1.0 + ttr1 * delta + ttr2 * delta * delta),
            )
        };
        let mut hot_effects = Vec::new();

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            for temperature in [tnom, 127.0] {
                let (tf_at, tr_at) = adjusted_times(temperature);
                for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
                    for forward in [true, false] {
                        let (collector_v, emitter_v) = if forward {
                            (sign * 2.0, 0.0)
                        } else {
                            (0.0, sign * 2.0)
                        };
                        let make_solver = |transit_model: String| {
                            let netlist = format!(
                                "Vdrive drive 0 dc 0\nRdrive drive b 1k\n\
                                 Vc c 0 dc {collector_v}\nVe e 0 dc {emitter_v}\n\
                                 Q1 c b e QMOD TEMP={temperature}\n.model QMOD {kind}({transit_model})\n"
                            );
                            Solver::new(
                                parse_netlist(&netlist)
                                    .expect("BJT transit-temperature deck parses"),
                                integration,
                            )
                            .expect("BJT transit-temperature solver builds")
                        };
                        let nominal_model =
                            format!("IS={is} BF={bf} BR={br} TNOM={tnom} TF={tf} TR={tr}");
                        let adjusted_model = format!(
                            "IS={is} BF={bf} BR={br} TNOM={tnom} TF={tf} TR={tr} \
                             TTF1={ttf1} TTF2={ttf2} TTR1={ttr1} TTR2={ttr2}"
                        );
                        let explicit_model =
                            format!("IS={is} BF={bf} BR={br} TNOM={tnom} TF={tf_at} TR={tr_at}");
                        let mut actual = make_solver(adjusted_model);
                        let mut explicit = make_solver(explicit_model);
                        let mut fixed = make_solver(nominal_model);
                        for solver in [&mut actual, &mut explicit, &mut fixed] {
                            let drive = solver
                                .voltage_source_index("Vdrive")
                                .expect("base drive source exists");
                            solver.set_voltage_source(drive, sign);
                            solver.advance(h).expect("transit-charge step solves");
                        }

                        let actual_base =
                            actual.circuit().node("b").expect("actual base node exists");
                        let explicit_base = explicit
                            .circuit()
                            .node("b")
                            .expect("explicit base node exists");
                        let fixed_base = fixed.circuit().node("b").expect("fixed base node exists");
                        let actual_voltage = actual.node_voltage(actual_base);
                        let explicit_voltage = explicit.node_voltage(explicit_base);
                        let fixed_voltage = fixed.node_voltage(fixed_base);
                        assert!(
                            (actual_voltage - explicit_voltage).abs() < 1e-8,
                            "{kind} {forward:?} {integration:?} TEMP={temperature}: expected explicit-time base voltage {explicit_voltage} V, got {actual_voltage} V"
                        );
                        assert!(
                            relative(
                                source_current(&actual, "Vdrive"),
                                source_current(&explicit, "Vdrive")
                            ) < 1e-6,
                            "{kind} {forward:?} {integration:?} TEMP={temperature}: transit-time drive currents differ"
                        );
                        if temperature > tnom {
                            let difference = (actual_voltage - fixed_voltage).abs();
                            assert!(
                                difference > 1e-7,
                                "{kind} {forward:?} {integration:?}: TTF/TTR must change the hot transient, difference={difference} V"
                            );
                            hot_effects.push(difference);
                        }
                    }
                }
            }
        }
        assert!(hot_effects.iter().all(|effect| *effect > 1e-7));
    }

    /// A reverse-biased diode with default `CJO=0` and `TT=0` conducts its
    /// saturation current, on SPICE's cubic reverse form, plus `GMIN*V` and
    /// has no stored charge.
    #[test]
    fn a_reverse_biased_diode_carries_only_leakage_with_no_stored_charge() {
        // The built-in `D` card is 1N4148-class.
        let (is, n) = (2.52e-9, 1.752);
        let mut solver = build("Vs s 0 dc -5\nR1 s a 1\nD1 a 0 D\n");
        let current = source_current(&solver, "Vs").abs();
        let arg = 3.0 * n * device::THERMAL_VOLTAGE / (-5.0 * std::f64::consts::E);
        let expected = is * (1.0 + arg * arg * arg) + device::GMIN * 5.0;
        assert!(
            relative(current, expected) < 1e-6,
            "a reverse-biased diode should pass IS·(1 + (3·N·Vt/(e·V))³) + GMIN*V = {expected} A, got {current} A"
        );
        // Switch the polarity and step: a real diode would sweep stored charge
        // out over its reverse-recovery time. This one is at the new operating
        // point on the first step, because there is no charge to sweep.
        let vs = solver.voltage_source_index("Vs").unwrap();
        solver.set_voltage_source(vs, 5.0);
        solver.advance(1e-9).expect("step solves");
        let forward = source_current(&solver, "Vs");
        assert!(
            forward > 1e-3,
            "with no stored charge the junction conducts on the first step, got {forward} A"
        );
    }

    // -----------------------------------------------------------------------
    // Bipolar transistor
    // -----------------------------------------------------------------------

    /// A BJT with both junctions held by voltage sources has an operating point
    /// that is pure arithmetic: no loop to solve, so the transport equations
    /// are checked directly.
    #[test]
    fn bjt_operating_point_matches_ebers_moll_by_hand() {
        let is = 1e-16;
        let bf = 100.0;
        let br = 1.0;
        for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
            let kind = if polarity == Polarity::N {
                "NPN"
            } else {
                "PNP"
            };
            let vbe = 0.65;
            let vbc = -4.35;
            let netlist = format!(
                "Vb b 0 dc {}\nVc c 0 dc {}\nQ1 c b 0 QMOD\n\
                 .model QMOD {kind}(IS={is} BF={bf} BR={br} NF=1 NR=1)\n",
                sign * vbe,
                sign * (vbe - vbc)
            );
            let solver = build(&netlist);

            let forward = (vbe / VT).exp();
            let reverse = (vbc / VT).exp();
            let ic = is * (forward - reverse) - (is / br) * (reverse - 1.0);
            let ib = (is / bf) * (forward - 1.0) + (is / br) * (reverse - 1.0);

            // Current INTO the collector, which is where the source's current
            // goes, plus the GMIN tie across the reverse-biased BC junction.
            let measured_c = sign * source_current(&solver, "Vc");
            let measured_b = sign * source_current(&solver, "Vb");
            assert!(
                relative(measured_c, ic + device::GMIN * vbc.abs()) < 1e-5,
                "{kind}: Ic should be {ic} A, got {measured_c} A"
            );
            assert!(
                relative(measured_b, ib) < 1e-3,
                "{kind}: Ib should be {ib} A, got {measured_b} A"
            );
            let beta = measured_c / measured_b;
            assert!(
                (beta - bf).abs() / bf < 0.02,
                "{kind}: forward beta should be about {bf}, got {beta}"
            );
        }
    }

    /// With the emitter and collector junction roles reversed, the old
    /// emitter carries the reverse transport current and its ratio to base
    /// current is `BR`. Check all three terminal currents against the
    /// Ebers–Moll equations for both polarities; forward-only checks would
    /// miss a swapped BC/BE stamp or a lost reverse gain.
    #[test]
    fn bjt_reverse_active_gain_matches_ebers_moll_by_hand() {
        let is = 1e-16;
        let bf = 80.0;
        let br = 4.0;
        let vbe = -4.35;
        let vbc = 0.65;
        let exp_be = (vbe / VT).exp();
        let exp_bc = (vbc / VT).exp();
        let junction_be = exp_be - 1.0;
        let junction_bc = exp_bc - 1.0;
        let ic = is * (exp_be - exp_bc) - (is / br) * junction_bc - device::GMIN * vbc;
        let ib = (is / bf) * junction_be + (is / br) * junction_bc + device::GMIN * (vbe + vbc);
        // The solver ties both base junctions with GMIN. In this reverse
        // active point, Vb−Ve = Vbe and Vb−Vc = Vbc.
        let ie = -(ic + ib);

        for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
            let kind = if polarity == Polarity::N {
                "NPN"
            } else {
                "PNP"
            };
            let netlist = format!(
                "Vb b 0 dc {}\nVc c 0 dc 0\nVe e 0 dc {}\nQ1 c b e QMOD\n\
                 .model QMOD {kind}(IS={is} BF={bf} BR={br} NF=1 NR=1)\n",
                sign * vbc,
                sign * (vbc - vbe)
            );
            let solver = build(&netlist);
            let actual_ic = sign * source_current(&solver, "Vc");
            let actual_ib = sign * source_current(&solver, "Vb");
            let actual_ie = sign * source_current(&solver, "Ve");

            assert!(
                relative(actual_ic, ic) < 1e-5,
                "{kind}: reverse collector terminal current should be {ic} A, got {actual_ic} A"
            );
            assert!(
                relative(actual_ib, ib) < 1e-5,
                "{kind}: reverse base current should be {ib} A, got {actual_ib} A"
            );
            assert!(
                relative(actual_ie, ie) < 1e-5,
                "{kind}: reverse emitter transport current should be {ie} A, got {actual_ie} A"
            );
            let reverse_gain = actual_ie / actual_ib;
            assert!(
                (reverse_gain - br).abs() / br < 1e-5,
                "{kind}: reverse common-emitter gain should be about {br}, got {reverse_gain}"
            );
        }
    }

    /// When both base junctions are forward biased, the Ebers–Moll transport
    /// currents oppose each other. Check all terminals against the equations
    /// for NPN and PNP; active-region tests do not exercise the BC injection
    /// term that dominates a saturated transistor.
    #[test]
    fn bjt_saturation_matches_ebers_moll_by_hand() {
        let is = 1e-16;
        let bf = 80.0;
        let br = 4.0;
        let vbe: f64 = 0.65;
        let vbc: f64 = 0.55;
        let exp_be = (vbe / VT).exp();
        let exp_bc = (vbc / VT).exp();
        let junction_be = exp_be - 1.0;
        let junction_bc = exp_bc - 1.0;
        let ic = is * (exp_be - exp_bc) - (is / br) * junction_bc - device::GMIN * vbc;
        let ib = (is / bf) * junction_be + (is / br) * junction_bc + device::GMIN * (vbe + vbc);
        let ie = -(ic + ib);

        for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
            let kind = if polarity == Polarity::N {
                "NPN"
            } else {
                "PNP"
            };
            let netlist = format!(
                "Vb b 0 dc {}\nVc c 0 dc {}\nVe e 0 dc 0\nQ1 c b e QMOD\n\
                 .model QMOD {kind}(IS={is} BF={bf} BR={br} NF=1 NR=1)\n",
                sign * vbe,
                sign * (vbe - vbc)
            );
            let solver = build(&netlist);
            let actual_ic = sign * source_current(&solver, "Vc");
            let actual_ib = sign * source_current(&solver, "Vb");
            let actual_ie = sign * source_current(&solver, "Ve");

            assert!(
                relative(actual_ic, ic) < 1e-5,
                "{kind}: saturated collector current should be {ic} A, got {actual_ic} A"
            );
            assert!(
                relative(actual_ib, ib) < 1e-5,
                "{kind}: saturated base current should be {ib} A, got {actual_ib} A"
            );
            assert!(
                relative(actual_ie, ie) < 1e-5,
                "{kind}: saturated emitter current should be {ie} A, got {actual_ie} A"
            );
        }
    }

    #[test]
    fn bjt_terminal_resistances_match_explicit_series_resistors_for_both_polarities() {
        let (rb, rc, re) = (31.0, 18.0, 7.0);

        for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
            let model = format!("IS=1e-15 BF=100 BR=4 RB={rb} RC={rc} RE={re}");
            let modelled = format!(
                "Vc c 0 dc {}\nVdrive drive 0 dc {}\nRdrive drive b 10k\n\
                 Rload e 0 10k\nQ1 c b e QMOD\n.model QMOD {kind}({model})\n",
                sign * 5.0,
                sign * 1.0,
            );
            let explicit = format!(
                "Vc c 0 dc {}\nVdrive drive 0 dc {}\nRdrive drive b 10k\n\
                 Rload e 0 10k\nRbase b qb {rb}\nRcollector c qc {rc}\n\
                 Remitter e qe {re}\nQ1 qc qb qe QMOD\n.model QMOD {kind}(IS=1e-15 BF=100 BR=4)\n",
                sign * 5.0,
                sign * 1.0,
            );
            let actual = build(&modelled);
            let expected = build(&explicit);

            for (actual_name, expected_name) in [
                ("c", "c"),
                ("b", "b"),
                ("e", "e"),
                ("Q1#collector", "qc"),
                ("Q1#base", "qb"),
                ("Q1#emitter", "qe"),
            ] {
                let actual_node = actual.circuit().node(actual_name).expect("node exists");
                let expected_node = expected
                    .circuit()
                    .node(expected_name)
                    .expect("reference node exists");
                let actual_voltage = actual.node_voltage(actual_node);
                let expected_voltage = expected.node_voltage(expected_node);
                assert!(
                    (actual_voltage - expected_voltage).abs() < 1e-8,
                    "{kind} {actual_name}: expected {expected_voltage} V, got {actual_voltage} V"
                );
            }

            for source in ["Vc", "Vdrive"] {
                let actual_current = source_current(&actual, source);
                let expected_current = source_current(&expected, source);
                assert!(
                    (actual_current - expected_current).abs() < 1e-10,
                    "{kind} {source}: expected {expected_current} A, got {actual_current} A"
                );
            }
        }
    }

    /// BJT `TRB*`, `TRC*`, and `TRE*` coefficients scale the three terminal
    /// resistors by `1 + TC1·ΔT + TC2·ΔT²`. Compare a real loaded transistor at
    /// nominal and hot temperatures against explicit resistors with the
    /// independently calculated values, for NPN and PNP models.
    #[test]
    fn bjt_terminal_resistance_temperature_coefficients_match_explicit_resistors() {
        let (tnom, rb, rc, re) = (27.0, 31.0, 18.0, 7.0);
        let (trb1, trb2, trc1, trc2, tre1, tre2) = (1e-3, 1e-5, 2e-3, 1e-5, 5e-4, 1e-5);
        let resistances = |temperature_celsius: f64| {
            let delta = temperature_celsius - tnom;
            (
                rb * (1.0 + trb1 * delta + trb2 * delta * delta),
                rc * (1.0 + trc1 * delta + trc2 * delta * delta),
                re * (1.0 + tre1 * delta + tre2 * delta * delta),
            )
        };

        for temperature in [tnom, 127.0] {
            let (rb_at, rc_at, re_at) = resistances(temperature);
            for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
                let modelled = format!(
                    "Vc c 0 dc {}\nVdrive drive 0 dc {}\nRdrive drive b 10k\n\
                     Rload e 0 10k\nQ1 c b e QMOD TEMP={temperature}\n\
                     .model QMOD {kind}(IS=1e-15 BF=100 BR=4 TNOM={tnom} \
                     RB={rb} TRB1={trb1} TRB2={trb2} RC={rc} TRC1={trc1} TRC2={trc2} \
                     RE={re} TRE1={tre1} TRE2={tre2})\n",
                    sign * 5.0,
                    sign * 1.0,
                );
                let explicit = format!(
                    "Vc c 0 dc {}\nVdrive drive 0 dc {}\nRdrive drive b 10k\n\
                     Rload e 0 10k\nRbase b qb {rb_at}\nRcollector c qc {rc_at}\n\
                     Remitter e qe {re_at}\nQ1 qc qb qe QMOD TEMP={temperature}\n\
                     .model QMOD {kind}(IS=1e-15 BF=100 BR=4 TNOM={tnom})\n",
                    sign * 5.0,
                    sign * 1.0,
                );
                let actual = build(&modelled);
                let expected = build(&explicit);
                let operating = actual.circuit().bjts[0].operating_model;
                assert!((operating.rb - rb_at).abs() < 1e-12);
                assert!((operating.rc - rc_at).abs() < 1e-12);
                assert!((operating.re - re_at).abs() < 1e-12);

                for (actual_name, expected_name) in [
                    ("c", "c"),
                    ("b", "b"),
                    ("e", "e"),
                    ("Q1#collector", "qc"),
                    ("Q1#base", "qb"),
                    ("Q1#emitter", "qe"),
                ] {
                    let actual_node = actual
                        .circuit()
                        .node(actual_name)
                        .expect("modelled node exists");
                    let expected_node = expected
                        .circuit()
                        .node(expected_name)
                        .expect("explicit resistor node exists");
                    let actual_voltage = actual.node_voltage(actual_node);
                    let expected_voltage = expected.node_voltage(expected_node);
                    assert!(
                        (actual_voltage - expected_voltage).abs() < 1e-8,
                        "{kind} TEMP={temperature} °C {actual_name}: expected {expected_voltage} V, got {actual_voltage} V"
                    );
                }
                for source in ["Vc", "Vdrive"] {
                    let actual_current = source_current(&actual, source);
                    let expected_current = source_current(&expected, source);
                    assert!(
                        (actual_current - expected_current).abs() < 1e-10,
                        "{kind} TEMP={temperature} °C {source}: expected {expected_current} A, got {actual_current} A"
                    );
                }
            }
        }
    }

    /// During a base step, Early effect, high-injection rolloff, base
    /// recombination, and junction/transit charge interact while displacement
    /// current flows through all three BJT terminals. Model-card RB/RC/RE must
    /// match explicit resistors around the same transistor, for both channel
    /// polarities and transient companion rules.
    #[test]
    fn bjt_transient_charge_matches_explicit_terminal_resistors_for_both_polarities() {
        let (rb, rc, re) = (31.0, 18.0, 7.0);
        let (h, drive_resistance) = (1e-9, 1e3);
        let bjt_physics = "IS=1e-15 BF=100 BR=4 VAF=50 VAR=75 \
            IKF=150u IKR=80u ISE=1e-12 NE=1.5 ISC=2e-12 NC=1.8";
        let charge = "CJE=60p CJC=40p VJE=1 MJE=0 VJC=1 MJC=0 TF=20n TR=5n";

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            for (kind, sign, collector_v, emitter_v) in
                [("NPN", 1.0, 3.0, 0.0), ("PNP", -1.0, 0.0, 3.0)]
            {
                let model_card = format!("{bjt_physics} RB={rb} RC={rc} RE={re} {charge}");
                let modelled = format!(
                    "Vc c 0 dc {collector_v}\nVe e 0 dc {emitter_v}\n\
                     Vdrive drive 0 dc {emitter_v}\nRdrive drive b {drive_resistance}\n\
                     Q1 c b e QMOD\n.model QMOD {kind}({model_card})\n"
                );
                let explicit = format!(
                    "Vc c 0 dc {collector_v}\nVe e 0 dc {emitter_v}\n\
                     Vdrive drive 0 dc {emitter_v}\nRdrive drive b {drive_resistance}\n\
                     Rbase b qb {rb}\nRcollector c qc {rc}\nRemitter e qe {re}\n\
                     Q1 qc qb qe QMOD\n.model QMOD {kind}({bjt_physics} {charge})\n"
                );
                let no_charge = format!(
                    "Vc c 0 dc {collector_v}\nVe e 0 dc {emitter_v}\n\
                     Vdrive drive 0 dc {emitter_v}\nRdrive drive b {drive_resistance}\n\
                     Q1 c b e QMOD\n\
                     .model QMOD {kind}({bjt_physics} RB={rb} RC={rc} RE={re})\n"
                );

                let mut model = Solver::new(
                    parse_netlist(&modelled).expect("BJT terminal charge model parses"),
                    integration,
                )
                .expect("BJT terminal charge solver builds");
                let mut explicit = Solver::new(
                    parse_netlist(&explicit).expect("explicit BJT terminal resistors parse"),
                    integration,
                )
                .expect("explicit-resistor BJT solver builds");
                let mut uncharged = Solver::new(
                    parse_netlist(&no_charge).expect("uncharged BJT model parses"),
                    integration,
                )
                .expect("uncharged BJT solver builds");

                let model_nodes = ["c", "b", "e", "Q1#collector", "Q1#base", "Q1#emitter"]
                    .map(|name| model.circuit().node(name).expect("model node"));
                let explicit_nodes = ["c", "b", "e", "qc", "qb", "qe"]
                    .map(|name| explicit.circuit().node(name).expect("explicit node"));
                let model_drive = model
                    .voltage_source_index("Vdrive")
                    .expect("model base-drive source");
                let explicit_drive = explicit
                    .voltage_source_index("Vdrive")
                    .expect("explicit base-drive source");
                let uncharged_drive = uncharged
                    .voltage_source_index("Vdrive")
                    .expect("uncharged base-drive source");

                model.advance(h).expect("model operating point settles");
                explicit
                    .advance(h)
                    .expect("explicit-resistor operating point settles");
                uncharged
                    .advance(h)
                    .expect("uncharged operating point settles");
                for (model_node, explicit_node) in model_nodes.into_iter().zip(explicit_nodes) {
                    assert!(
                        (model.node_voltage(model_node) - explicit.node_voltage(explicit_node))
                            .abs()
                            < 1e-9,
                        "{kind} {integration:?}: initial terminal voltages differ"
                    );
                }

                let driven_base = emitter_v + sign * 0.9;
                model.set_voltage_source(model_drive, driven_base);
                explicit.set_voltage_source(explicit_drive, driven_base);
                uncharged.set_voltage_source(uncharged_drive, driven_base);
                model.advance(h).expect("model BJT charge step solves");
                explicit
                    .advance(h)
                    .expect("explicit-resistor BJT charge step solves");
                uncharged.advance(h).expect("uncharged BJT step solves");

                let mut charge_effect = 0.0_f64;
                for (label, model_node, explicit_node) in [
                    ("external collector", model_nodes[0], explicit_nodes[0]),
                    ("external base", model_nodes[1], explicit_nodes[1]),
                    ("external emitter", model_nodes[2], explicit_nodes[2]),
                    ("intrinsic collector", model_nodes[3], explicit_nodes[3]),
                    ("intrinsic base", model_nodes[4], explicit_nodes[4]),
                    ("intrinsic emitter", model_nodes[5], explicit_nodes[5]),
                ] {
                    let actual = model.node_voltage(model_node);
                    let expected = explicit.node_voltage(explicit_node);
                    charge_effect =
                        charge_effect.max((actual - uncharged.node_voltage(model_node)).abs());
                    assert!(
                        (actual - expected).abs() < 1e-8,
                        "{kind} {integration:?} {label}: model-card resistances give {actual} V, explicit resistors give {expected} V"
                    );
                }
                assert!(
                    charge_effect > 1e-7,
                    "{kind} {integration:?}: junction/transit charge must measurably affect terminal voltages, got a largest charged/uncharged difference of {charge_effect} V"
                );
                for source in ["Vc", "Ve", "Vdrive"] {
                    let actual = source_current(&model, source);
                    let expected = source_current(&explicit, source);
                    assert!(
                        (actual - expected).abs() < 1e-10,
                        "{kind} {integration:?} {source}: model-card resistances give {actual} A, explicit resistors give {expected} A"
                    );
                }
            }
        }
    }

    /// The transient base current includes displacement current, but ngspice's
    /// RBM/IRB law is controlled by the transistor's conduction base current.
    /// Step a charged NPN and PNP through the MNA solver and check the external
    /// base drop against that law at the solved intrinsic junction voltages.
    #[test]
    fn bjt_bias_dependent_base_resistance_uses_conduction_current_during_transients() {
        let (rb, rbm, irb) = (50.0, 5.0, 100e-6);
        let (h, drive_resistance) = (1e-6, 1e3);
        let charge = "CJE=60p CJC=40p VJE=1 MJE=0 VJC=1 MJC=0 TF=20n TR=5n";
        let expected_resistance = |base_current: f64| {
            let x = (base_current / irb).max(1e-9);
            let z = ((1.0 + 14.59025 * x).sqrt() - 1.0) / (2.4317 * x.sqrt());
            let tan_z = z.tan();
            rbm + 3.0 * (rb - rbm) * (tan_z - z) / (z * tan_z * tan_z)
        };

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            for (kind, sign, collector_v, emitter_v) in
                [("NPN", 1.0, 3.0, 0.0), ("PNP", -1.0, 0.0, 3.0)]
            {
                let initial_base = emitter_v + sign * 0.7;
                let modelled = format!(
                    "Vc c 0 dc {collector_v}\nVe e 0 dc {emitter_v}\n\
                     Vdrive drive 0 dc {initial_base}\nRdrive drive b {drive_resistance}\n\
                     Q1 c b e QMOD\n\
                     .model QMOD {kind}(IS=1e-15 BF=100 BR=4 VAF=0 VAR=0 \
                     RB={rb} RBM={rbm} IRB={irb} {charge})\n"
                );
                let uncharged = format!(
                    "Vc c 0 dc {collector_v}\nVe e 0 dc {emitter_v}\n\
                     Vdrive drive 0 dc {initial_base}\nRdrive drive b {drive_resistance}\n\
                     Q1 c b e QMOD\n\
                     .model QMOD {kind}(IS=1e-15 BF=100 BR=4 VAF=0 VAR=0 \
                     RB={rb} RBM={rbm} IRB={irb})\n"
                );
                let mut model = Solver::new(
                    parse_netlist(&modelled).expect("charged BJT model parses"),
                    integration,
                )
                .expect("charged BJT solver builds");
                let mut uncharged = Solver::new(
                    parse_netlist(&uncharged).expect("uncharged BJT model parses"),
                    integration,
                )
                .expect("uncharged BJT solver builds");
                let model_drive = model
                    .voltage_source_index("Vdrive")
                    .expect("model base-drive source");
                let uncharged_drive = uncharged
                    .voltage_source_index("Vdrive")
                    .expect("uncharged base-drive source");
                let external_base = model.circuit().node("b").expect("external base");
                let internal_base = model.circuit().node("Q1#base").expect("intrinsic base");
                let internal_collector = model.circuit().bjts[0].junction_c;
                let internal_emitter = model.circuit().bjts[0].junction_e;
                let uncharged_base = uncharged.circuit().node("b").expect("external base");

                model.advance(h).expect("charged operating point settles");
                uncharged
                    .advance(h)
                    .expect("uncharged operating point settles");
                let driven_base = emitter_v + sign * 0.701;
                model.set_voltage_source(model_drive, driven_base);
                uncharged.set_voltage_source(uncharged_drive, driven_base);
                model.advance(h).expect("charged BJT transient solves");
                uncharged
                    .advance(h)
                    .expect("uncharged BJT transient solves");

                let vbe = sign
                    * (model.node_voltage(internal_base) - model.node_voltage(internal_emitter));
                let vbc = sign
                    * (model.node_voltage(internal_base) - model.node_voltage(internal_collector));
                let model_card = model.circuit().bjts[0].model;
                let op = device::bjt_op_with_nkf(
                    vbe,
                    vbc,
                    model_card.is,
                    model_card.bf,
                    model_card.br,
                    model_card.nf,
                    model_card.nr,
                    model_card.vaf,
                    model_card.var,
                    model_card.ikf,
                    model_card.ikr,
                    model_card.ise,
                    model_card.ne,
                    model_card.isc,
                    model_card.nc,
                    model_card.nkf,
                );
                let base_current = sign * source_current(&model, "Vdrive");
                let base_drop =
                    sign * (model.node_voltage(external_base) - model.node_voltage(internal_base));
                let actual_resistance = base_drop / base_current;
                let expected = expected_resistance(op.ib);
                let total_current_resistance = expected_resistance(base_current);
                let uncharged_voltage = uncharged.node_voltage(uncharged_base);
                let charged_voltage = model.node_voltage(external_base);

                assert!(
                    op.ib > 0.0,
                    "{kind} {integration:?}: expected forward conduction current, Vbe={vbe} V, Vbc={vbc} V, Ib={} A",
                    op.ib
                );
                assert!(
                    (base_current - op.ib).abs() > 1e-8,
                    "{kind} {integration:?}: the transient must add measurable base displacement current, source={base_current} A, conduction={}",
                    op.ib
                );
                assert!(
                    relative(expected, total_current_resistance) > 1e-3,
                    "{kind} {integration:?}: displacement current must select a distinct RBM/IRB resistance; conduction predicts {expected} Ω while total current predicts {total_current_resistance} Ω"
                );
                assert!(
                    (charged_voltage - uncharged_voltage).abs() > 1e-7,
                    "{kind} {integration:?}: stored charge must move the solved external base, charged={charged_voltage} V, uncharged={uncharged_voltage} V"
                );
                assert!(
                    relative(actual_resistance, expected) < 1e-5,
                    "{kind} {integration:?}: RBM/IRB should use conduction Ib={} A and give {expected} Ω, got {actual_resistance} Ω from total base current {base_current} A",
                    op.ib
                );
            }
        }
    }

    /// Ngspice's RBM/IRB law makes the base spreading resistance fall from RB
    /// toward RBM as the solved base current rises. This drives both an NPN and
    /// PNP through that transition and checks the voltage drop/current ratio
    /// against the closed-form resistance interpolation.
    #[test]
    fn bjt_base_resistance_rolls_off_with_bias_for_both_polarities() {
        let (rb, rbm, irb) = (50.0, 5.0, 100e-6);
        let (trb1, trb2, trbm1, trbm2, tirb1, tirb2) = (1e-3, 2e-6, 2e-3, 3e-6, 1e-3, 1e-6);
        let expected_resistance = |base_current: f64, temperature: f64| {
            let delta = temperature - 27.0;
            let rb_at = rb * (1.0 + trb1 * delta + trb2 * delta * delta);
            let rbm_at = rbm * (1.0 + trbm1 * delta + trbm2 * delta * delta);
            let irb_at = irb * (1.0 + tirb1 * delta + tirb2 * delta * delta);
            let x = (base_current / irb_at).max(1e-9);
            let z = ((1.0 + 14.59025 * x).sqrt() - 1.0) / (2.4317 * x.sqrt());
            let tan_z = z.tan();
            rbm_at + 3.0 * (rb_at - rbm_at) * (tan_z - z) / (z * tan_z * tan_z)
        };

        for temperature in [27.0, 127.0] {
            for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
                let mut measured = Vec::new();
                for drive in [0.75, 0.82] {
                    let netlist = format!(
                        "Vb b 0 dc {}\nVc c 0 dc {}\nQ1 c b 0 QMOD TEMP={temperature}\n\
                         .model QMOD {kind}(IS=1e-15 BF=100 BR=1 VAF=0 VAR=0 TNOM=27 \
                         RB={rb} RBM={rbm} IRB={irb} TRB1={trb1} TRB2={trb2} \
                         TRBM1={trbm1} TRBM2={trbm2} TIRB1={tirb1} TIRB2={tirb2})\n",
                        sign * drive,
                        sign * 5.0,
                    );
                    let solver = build(&netlist);
                    let base = solver
                        .circuit()
                        .node("b")
                        .expect("external base node")
                        .expect("base is not ground");
                    let internal_base = solver
                        .circuit()
                        .node("Q1#base")
                        .expect("internal base node")
                        .expect("base is not ground");
                    let base_current = sign * source_current(&solver, "Vb");
                    let base_drop = sign
                        * (solver.node_voltage(Some(base))
                            - solver.node_voltage(Some(internal_base)));
                    let actual_resistance = base_drop / base_current;
                    let expected = expected_resistance(base_current, temperature);
                    assert!(base_current > 0.0, "{kind}: expected forward base current");
                    assert!(
                        relative(actual_resistance, expected) < 1e-5,
                        "{kind} TEMP={temperature} °C at {drive} V: expected RB={expected} Ω from Ib={base_current} A, got {actual_resistance} Ω"
                    );
                    measured.push(actual_resistance);
                }
                assert!(
                    measured[1] < measured[0],
                    "{kind} TEMP={temperature} °C: base resistance should roll off with increasing base current, got {measured:?}"
                );
            }
        }
    }

    /// Instance TEMP adjusts BJT saturation current, thermal voltage and beta.
    /// Pin both junction voltages in an MNA circuit, then compare all three
    /// terminal currents with an independent SPICE temperature calculation.
    #[test]
    fn bjt_instance_temperature_matches_gummel_poon_current_laws() {
        let (is, bf, br, nf, nr) = (1e-16, 80.0, 4.0, 1.1, 1.2);
        let (ise, ne, isc, nc) = (1e-12, 1.5, 2e-12, 1.8);
        let (eg, xti, xtb, tnom_celsius) = (1.11, 3.0, 1.4, 27.0);
        let (vbe, vbc) = (0.62, -1.38);
        let expected = |temperature_celsius: f64| {
            let temperature_kelvin = temperature_celsius + 273.15;
            let nominal_kelvin = tnom_celsius + 273.15;
            let ratio = temperature_kelvin / nominal_kelvin;
            let vt = 1.380_649e-23 * temperature_kelvin / 1.602_176_634e-19;
            let factlog = (ratio - 1.0) * eg / vt + xti * f64::ln(ratio);
            let beta_factor = ratio.powf(xtb);
            let adjusted_is = is * f64::exp(factlog);
            let adjusted_bf = bf * beta_factor;
            let adjusted_br = br * beta_factor;
            let adjusted_ise = ise * f64::exp(factlog / ne) / beta_factor;
            let adjusted_isc = isc * f64::exp(factlog / nc) / beta_factor;
            let forward = f64::exp(vbe / (nf * vt));
            let reverse = f64::exp(vbc / (nr * vt));
            let transport = adjusted_is * (forward - reverse);
            let ibe = (adjusted_is / adjusted_bf) * (forward - 1.0)
                + adjusted_ise * (f64::exp(vbe / (ne * vt)) - 1.0);
            let ibc = (adjusted_is / adjusted_br) * (reverse - 1.0)
                + adjusted_isc * (f64::exp(vbc / (nc * vt)) - 1.0);
            let ic = transport - ibc - device::GMIN * vbc;
            let ib = ibe + ibc + device::GMIN * (vbe + vbc);
            [ic, ib, -(ic + ib)]
        };

        for temperature_celsius in [27.0, 125.0] {
            let expected_currents = expected(temperature_celsius);
            for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
                let netlist = format!(
                    "Vb b 0 dc {}\nVc c 0 dc {}\nVe e 0 dc 0\n\
                     Q1 c b e QMOD TEMP={temperature_celsius}\n\
                     .model QMOD {kind}(IS={is} BF={bf} BR={br} NF={nf} NR={nr} \
                     ISE={ise} NE={ne} ISC={isc} NC={nc} TNOM={tnom_celsius} \
                     EG={eg} XTI={xti} XTB={xtb})\n",
                    sign * vbe,
                    sign * (vbe - vbc),
                );
                let solver = build(&netlist);
                let actual = [
                    sign * source_current(&solver, "Vc"),
                    sign * source_current(&solver, "Vb"),
                    sign * source_current(&solver, "Ve"),
                ];
                for (terminal, actual, expected) in [
                    ("collector", actual[0], expected_currents[0]),
                    ("base", actual[1], expected_currents[1]),
                    ("emitter", actual[2], expected_currents[2]),
                ] {
                    assert!(
                        relative(actual, expected) < 2e-5,
                        "{kind} TEMP={temperature_celsius} °C {terminal}: expected {expected} A, got {actual} A"
                    );
                }
            }
        }
    }

    /// With high-injection terms disabled, Gummel–Poon's base-width factor is
    /// `1 - Vbc/VAF - Vbe/VAR`. Check all terminal currents against that
    /// equation in both operating directions and polarities.
    #[test]
    fn bjt_early_effect_matches_gummel_poon_by_hand() {
        let (is, bf, br, vaf, var) = (1e-16, 80.0, 4.0, 35.0, 50.0);
        for (vbe, vbc) in [(0.65, -4.35), (-4.35, 0.65)] {
            let exp_be = (vbe / VT).exp();
            let exp_bc = (vbc / VT).exp();
            let base_width = 1.0 - vbc / vaf - vbe / var;
            let ict = is * (exp_be - exp_bc) * base_width;
            let ibe = (is / bf) * (exp_be - 1.0);
            let ibc = (is / br) * (exp_bc - 1.0);
            let ic = ict - ibc - device::GMIN * vbc;
            let ib = ibe + ibc + device::GMIN * (vbe + vbc);
            let ie = -(ic + ib);

            for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
                let kind = if polarity == Polarity::N {
                    "NPN"
                } else {
                    "PNP"
                };
                let netlist = format!(
                    "Vb b 0 dc {}\nVc c 0 dc 0\nVe e 0 dc {}\nQ1 c b e QMOD\n\
                     .model QMOD {kind}(IS={is} BF={bf} BR={br} VAF={vaf} VAR={var})\n",
                    sign * vbc,
                    sign * (vbc - vbe),
                );
                let solver = build(&netlist);
                let actual_ic = sign * source_current(&solver, "Vc");
                let actual_ib = sign * source_current(&solver, "Vb");
                let actual_ie = sign * source_current(&solver, "Ve");
                let direction = if vbe > 0.0 { "forward" } else { "reverse" };

                assert!(
                    relative(actual_ic, ic) < 1e-5,
                    "{kind} {direction}: collector current should be {ic} A, got {actual_ic} A"
                );
                assert!(
                    relative(actual_ib, ib) < 1e-5,
                    "{kind} {direction}: base current should be {ib} A, got {actual_ib} A"
                );
                assert!(
                    relative(actual_ie, ie) < 1e-5,
                    "{kind} {direction}: emitter current should be {ie} A, got {actual_ie} A"
                );
            }
        }
    }

    #[test]
    fn bjt_early_effect_solves_a_resistively_loaded_collector() {
        let (vcc, rb, vbe, is, br, vaf) = (5.0, 1000.0, 0.65, 1e-16, 4.0, 50.0);
        let current_balance = |vc: f64| {
            let vbc = vbe - vc;
            let forward = (vbe / VT).exp();
            let reverse = (vbc / VT).exp();
            let ict = is * (forward - reverse) * (1.0 - vbc / vaf);
            let ibc = (is / br) * (reverse - 1.0);
            let collector_current = ict - ibc;
            (vcc - vc) / rb - collector_current - device::GMIN * (vc - vbe)
        };
        let mut lo = 0.0;
        let mut hi = vcc;
        for _ in 0..100 {
            let mid = (lo + hi) / 2.0;
            if current_balance(mid) > 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let expected_vc = (lo + hi) / 2.0;
        let solver = build(
            "Vcc supply 0 dc 5\nR1 supply c 1k\nVb b 0 dc 0.65\n\
             Q1 c b 0 QMOD\n.model QMOD NPN(IS=1e-16 BF=80 BR=4 VAF=50)\n",
        );
        let collector = solver.circuit().node("c").unwrap();
        let actual_vc = solver.node_voltage(collector);
        assert!(
            (actual_vc - expected_vc).abs() < 1e-8,
            "loaded collector should settle at {expected_vc} V with Early effect, got {actual_vc} V"
        );
    }

    #[test]
    fn bjt_early_effect_partials_match_transport_and_junction_derivatives() {
        let (is, bf, br, nf, nr, vaf, var) = (2e-15, 75.0, 3.0, 1.1, 1.2, 42.0, 65.0);
        let delta = 1e-7;
        for (vbe, vbc) in [(0.62, -0.1), (-0.1, 0.58)] {
            let currents = |vbe: f64, vbc: f64| {
                let forward = (vbe / (nf * VT)).exp();
                let reverse = (vbc / (nr * VT)).exp();
                let qinv = 1.0 - vbc / vaf - vbe / var;
                (
                    is * (forward - reverse) * qinv,
                    (is / bf) * (forward - 1.0),
                    (is / br) * (reverse - 1.0),
                )
            };
            let op = device::bjt_op(
                vbe,
                vbc,
                is,
                bf,
                br,
                nf,
                nr,
                vaf,
                var,
                f64::INFINITY,
                f64::INFINITY,
                0.0,
                1.5,
                0.0,
                2.0,
            );
            let (ict_plus, ibe_plus, _) = currents(vbe + delta, vbc);
            let (ict_minus, ibe_minus, _) = currents(vbe - delta, vbc);
            let expected_gif = (ict_plus - ict_minus) / (2.0 * delta);
            let expected_gpi = (ibe_plus - ibe_minus) / (2.0 * delta);
            let (ict_plus, _, ibc_plus) = currents(vbe, vbc + delta);
            let (ict_minus, _, ibc_minus) = currents(vbe, vbc - delta);
            let expected_gir = -(ict_plus - ict_minus) / (2.0 * delta);
            let expected_gmu = (ibc_plus - ibc_minus) / (2.0 * delta);

            assert!(
                relative(op.gif, expected_gif) < 1e-6,
                "gif: {} vs {expected_gif}",
                op.gif
            );
            assert!(
                relative(op.gir, expected_gir) < 1e-6,
                "gir: {} vs {expected_gir}",
                op.gir
            );
            assert!(
                relative(op.gpi, expected_gpi) < 1e-6,
                "gpi: {} vs {expected_gpi}",
                op.gpi
            );
            assert!(
                relative(op.gmu, expected_gmu) < 1e-6,
                "gmu: {} vs {expected_gmu}",
                op.gmu
            );
        }
    }

    #[test]
    fn zero_and_infinite_bjt_early_voltages_disable_base_width_modulation() {
        let parameters = (0.65, -4.35, 1e-16, 100.0, 1.0, 1.0, 1.0);
        let disabled_by_zero = device::bjt_op(
            parameters.0,
            parameters.1,
            parameters.2,
            parameters.3,
            parameters.4,
            parameters.5,
            parameters.6,
            0.0,
            0.0,
            f64::INFINITY,
            f64::INFINITY,
            0.0,
            1.5,
            0.0,
            2.0,
        );
        let disabled_by_infinity = device::bjt_op(
            parameters.0,
            parameters.1,
            parameters.2,
            parameters.3,
            parameters.4,
            parameters.5,
            parameters.6,
            f64::INFINITY,
            f64::INFINITY,
            f64::INFINITY,
            f64::INFINITY,
            0.0,
            1.5,
            0.0,
            2.0,
        );
        assert_eq!(disabled_by_zero, disabled_by_infinity);
    }

    /// Forward and reverse Gummel–Poon high-injection rolloff, checked against
    /// the closed-form base charge for both polarities and operation directions.
    #[test]
    fn bjt_high_injection_rolloff_matches_gummel_poon_by_hand() {
        let (is, bf, br, vaf, var, ikf, ikr) = (1e-16, 80.0, 4.0, 35.0, 50.0, 2e-4, 5e-4);
        for (vbe, vbc) in [(0.75, -4.25), (-4.25, 0.75)] {
            let forward = (vbe / VT).exp();
            let reverse = (vbc / VT).exp();
            let q1 = 1.0 / (1.0 - vbc / vaf - vbe / var);
            let q2 = is * (forward - 1.0) / ikf + is * (reverse - 1.0) / ikr;
            let qb = q1 * (1.0 + (1.0 + 4.0 * q2).sqrt()) / 2.0;
            let ict = is * (forward - reverse) / qb;
            let ibe = (is / bf) * (forward - 1.0);
            let ibc = (is / br) * (reverse - 1.0);
            let ic = ict - ibc - device::GMIN * vbc;
            let ib = ibe + ibc + device::GMIN * (vbe + vbc);
            let ie = -(ic + ib);
            let direction = if vbe > 0.0 { "forward" } else { "reverse" };

            for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
                let kind = if polarity == Polarity::N {
                    "NPN"
                } else {
                    "PNP"
                };
                let netlist = format!(
                    "Vb b 0 dc {}\nVc c 0 dc 0\nVe e 0 dc {}\nQ1 c b e QMOD\n\
                     .model QMOD {kind}(IS={is} BF={bf} BR={br} VAF={vaf} VAR={var} IKF={ikf} IKR={ikr})\n",
                    sign * vbc,
                    sign * (vbc - vbe),
                );
                let solver = build(&netlist);
                let actual_ic = sign * source_current(&solver, "Vc");
                let actual_ib = sign * source_current(&solver, "Vb");
                let actual_ie = sign * source_current(&solver, "Ve");

                assert!(
                    relative(actual_ic, ic) < 1e-5,
                    "{kind} {direction}: collector current should be {ic} A, got {actual_ic} A"
                );
                assert!(
                    relative(actual_ib, ib) < 1e-5,
                    "{kind} {direction}: base current should be {ib} A, got {actual_ib} A"
                );
                assert!(
                    relative(actual_ie, ie) < 1e-5,
                    "{kind} {direction}: emitter current should be {ie} A, got {actual_ie} A"
                );
                if vbe > 0.0 {
                    assert!(
                        actual_ic / actual_ib < bf,
                        "{kind}: IKF must reduce forward beta"
                    );
                } else {
                    assert!(
                        actual_ie / actual_ib < br,
                        "{kind}: IKR must reduce reverse beta"
                    );
                }
            }
        }
    }

    /// Explicit `NKF` changes the exponent of the Gummel–Poon high-current
    /// base-charge term. Exercise the resulting terminal currents through the
    /// MNA solver rather than only checking that the model card parsed.
    #[test]
    fn bjt_nkf_controls_high_injection_rolloff_for_both_polarities_and_directions() {
        let (is, bf, br, vaf, var, ikf, ikr, nkf) = (1e-16, 80.0, 4.0, 35.0, 50.0, 2e-4, 5e-4, 0.8);

        for (vbe, vbc) in [(0.75, -4.25), (-4.25, 0.75)] {
            let forward = (vbe / VT).exp();
            let reverse = (vbc / VT).exp();
            let q1 = 1.0 / (1.0 - vbc / vaf - vbe / var);
            let q2 = is * (forward - 1.0) / ikf + is * (reverse - 1.0) / ikr;
            let qb = q1 * (1.0 + (1.0 + 4.0 * q2).powf(nkf)) / 2.0;
            let ict = is * (forward - reverse) / qb;
            let ibe = (is / bf) * (forward - 1.0);
            let ibc = (is / br) * (reverse - 1.0);
            let ic = ict - ibc - device::GMIN * vbc;
            let ib = ibe + ibc + device::GMIN * (vbe + vbc);
            let ie = -(ic + ib);
            let direction = if vbe > 0.0 { "forward" } else { "reverse" };

            for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
                let netlist = format!(
                    "Vb b 0 dc {}\nVc c 0 dc 0\nVe e 0 dc {}\nQ1 c b e QMOD\n\
                     .model QMOD {kind}(IS={is} BF={bf} BR={br} VAF={vaf} VAR={var} IKF={ikf} IKR={ikr} NKF={nkf})\n",
                    sign * vbc,
                    sign * (vbc - vbe),
                );
                let solver = build(&netlist);
                assert_eq!(solver.circuit().bjts[0].model.nkf, Some(nkf));
                let actual_ic = sign * source_current(&solver, "Vc");
                let actual_ib = sign * source_current(&solver, "Vb");
                let actual_ie = sign * source_current(&solver, "Ve");

                assert!(
                    relative(actual_ic, ic) < 1e-5,
                    "{kind} {direction} NKF={nkf}: expected Ic={ic} A, got {actual_ic} A"
                );
                assert!(
                    relative(actual_ib, ib) < 1e-5,
                    "{kind} {direction} NKF={nkf}: expected Ib={ib} A, got {actual_ib} A"
                );
                assert!(
                    relative(actual_ie, ie) < 1e-5,
                    "{kind} {direction} NKF={nkf}: expected Ie={ie} A, got {actual_ie} A"
                );
            }
        }
    }

    /// The same NKF-dependent base charge feeds forward transit charge, so a
    /// non-default exponent must change Qbe as well as the DC transport gain.
    #[test]
    fn bjt_nkf_changes_forward_transit_charge() {
        let (is, nf, nr, vaf, var, ikf, ikr, nkf, tf) =
            (2e-16, 1.2, 1.1, 40.0, 70.0, 2e-3, 5e-3, 0.8, 12e-9);
        let circuit = parse_netlist(&format!(
            "Q1 c b e QMOD\n.model QMOD NPN(IS={is} NF={nf} NR={nr} VAF={vaf} VAR={var} IKF={ikf} IKR={ikr} NKF={nkf} TF={tf})\n"
        ))
        .expect("NKF transit-charge model parses");
        let model = circuit.bjts[0].model;
        let (vbe, vbc) = (0.64, 0.35);
        let forward = (vbe / (nf * VT)).exp();
        let reverse = (vbc / (nr * VT)).exp();
        let cbe = is * (forward - 1.0);
        let cbc = is * (reverse - 1.0);
        let q1 = 1.0 / (1.0 - vbc / vaf - vbe / var);
        let q2 = cbe / ikf + cbc / ikr;
        let qb = q1 * (1.0 + (1.0 + 4.0 * q2).powf(nkf)) / 2.0;
        let expected = tf * cbe / qb;
        let charge = device::bjt_transit_charge(vbe, vbc, model);
        let actual = charge.qbe;
        assert!(
            (actual - expected).abs() < 1e-24 + expected.abs() * 1e-8,
            "NKF={nkf}: expected Qbe={expected} C, got {actual} C"
        );

        let delta = 1e-6;
        let plus_be = device::bjt_transit_charge(vbe + delta, vbc, model).qbe;
        let minus_be = device::bjt_transit_charge(vbe - delta, vbc, model).qbe;
        let plus_bc = device::bjt_transit_charge(vbe, vbc + delta, model).qbe;
        let minus_bc = device::bjt_transit_charge(vbe, vbc - delta, model).qbe;
        let finite_difference = |plus: f64, minus: f64| (plus - minus) / (2.0 * delta);
        for (actual, expected) in [
            (charge.dqbe_dvbe, finite_difference(plus_be, minus_be)),
            (charge.dqbe_dvbc, finite_difference(plus_bc, minus_bc)),
        ] {
            assert!(
                (actual - expected).abs() < expected.abs() * 2e-5 + 1e-24,
                "NKF transit-charge derivative {actual} differs from finite difference {expected}"
            );
        }
    }

    #[test]
    fn bjt_nkf_high_injection_partials_match_finite_difference() {
        let (is, nf, nr, vaf, var, ikf, ikr, nkf) = (2e-15, 1.1, 1.2, 42.0, 65.0, 2e-4, 5e-4, 0.8);
        let delta = 1e-7;
        let transport = |vbe: f64, vbc: f64| {
            let forward = (vbe / (nf * VT)).exp();
            let reverse = (vbc / (nr * VT)).exp();
            let q1 = 1.0 / (1.0 - vbc / vaf - vbe / var);
            let q2 = is * (forward - 1.0) / ikf + is * (reverse - 1.0) / ikr;
            let qb = q1 * (1.0 + (1.0 + 4.0 * q2).powf(nkf)) / 2.0;
            is * (forward - reverse) / qb
        };

        for (vbe, vbc) in [(0.68, -0.1), (-0.1, 0.62)] {
            let op = device::bjt_op_with_nkf(
                vbe,
                vbc,
                is,
                75.0,
                3.0,
                nf,
                nr,
                vaf,
                var,
                ikf,
                ikr,
                0.0,
                1.5,
                0.0,
                2.0,
                Some(nkf),
            );
            let expected_gif =
                (transport(vbe + delta, vbc) - transport(vbe - delta, vbc)) / (2.0 * delta);
            let expected_gir =
                -(transport(vbe, vbc + delta) - transport(vbe, vbc - delta)) / (2.0 * delta);
            assert!(
                relative(op.gif, expected_gif) < 1e-6,
                "NKF={nkf} gif: {} vs {expected_gif}",
                op.gif
            );
            assert!(
                relative(op.gir, expected_gir) < 1e-6,
                "NKF={nkf} gir: {} vs {expected_gir}",
                op.gir
            );
        }
    }

    #[test]
    fn bjt_high_injection_partials_match_finite_difference() {
        let (is, bf, br, nf, nr, vaf, var, ikf, ikr) =
            (2e-15, 75.0, 3.0, 1.1, 1.2, 42.0, 65.0, 2e-4, 5e-4);
        let delta = 1e-7;
        let transport = |vbe: f64, vbc: f64| {
            let forward = (vbe / (nf * VT)).exp();
            let reverse = (vbc / (nr * VT)).exp();
            let q1 = 1.0 / (1.0 - vbc / vaf - vbe / var);
            let q2 = is * (forward - 1.0) / ikf + is * (reverse - 1.0) / ikr;
            let qb = q1 * (1.0 + (1.0 + 4.0 * q2).sqrt()) / 2.0;
            is * (forward - reverse) / qb
        };

        for (vbe, vbc) in [(0.68, -0.1), (-0.1, 0.62)] {
            let op = device::bjt_op(
                vbe, vbc, is, bf, br, nf, nr, vaf, var, ikf, ikr, 0.0, 1.5, 0.0, 2.0,
            );
            let expected_gif =
                (transport(vbe + delta, vbc) - transport(vbe - delta, vbc)) / (2.0 * delta);
            let expected_gir =
                -(transport(vbe, vbc + delta) - transport(vbe, vbc - delta)) / (2.0 * delta);
            assert!(
                relative(op.gif, expected_gif) < 1e-6,
                "gif: {} vs {expected_gif}",
                op.gif
            );
            assert!(
                relative(op.gir, expected_gir) < 1e-6,
                "gir: {} vs {expected_gir}",
                op.gir
            );
        }
    }

    #[test]
    fn bjt_high_injection_solves_a_resistively_loaded_collector() {
        let (vcc, resistance, vbe, is, br, vaf, ikf) = (5.0, 1_000.0, 0.75, 1e-16, 4.0, 50.0, 2e-4);
        let current_balance = |vc: f64| {
            let vbc = vbe - vc;
            let forward = (vbe / VT).exp();
            let reverse = (vbc / VT).exp();
            let q1 = 1.0 / (1.0 - vbc / vaf);
            let q2 = is * (forward - 1.0) / ikf;
            let qb = q1 * (1.0 + (1.0 + 4.0 * q2).sqrt()) / 2.0;
            let ict = is * (forward - reverse) / qb;
            let ibc = (is / br) * (reverse - 1.0);
            let collector_current = ict - ibc;
            (vcc - vc) / resistance - collector_current - device::GMIN * (vc - vbe)
        };
        let mut lo = 0.0;
        let mut hi = vcc;
        for _ in 0..100 {
            let mid = (lo + hi) / 2.0;
            if current_balance(mid) > 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let expected_vc = (lo + hi) / 2.0;
        let solver = build(
            "Vcc supply 0 dc 5\nR1 supply c 1k\nVb b 0 dc 0.75\n\
             Q1 c b 0 QMOD\n.model QMOD NPN(IS=1e-16 BF=80 BR=4 VAF=50 IKF=0.2m)\n",
        );
        let collector = solver.circuit().node("c").unwrap();
        let actual_vc = solver.node_voltage(collector);
        assert!(
            (actual_vc - expected_vc).abs() < 1e-8,
            "loaded collector should settle at {expected_vc} V with IKF rolloff, got {actual_vc} V"
        );
    }

    /// Base-emitter and base-collector recombination currents remain separate
    /// from transport current in forward and reverse operation.
    #[test]
    fn bjt_base_recombination_matches_gummel_poon_by_hand() {
        let (is, bf, br, ise, ne, isc, nc) = (1e-16, 80.0, 4.0, 1e-12, 1.5, 2e-11, 2.0);
        for (vbe, vbc) in [(0.65, -4.35), (-4.35, 0.65)] {
            let forward = (vbe / VT).exp();
            let reverse = (vbc / VT).exp();
            let transport = is * (forward - reverse);
            let ibe = (is / bf) * (forward - 1.0) + ise * ((vbe / (ne * VT)).exp() - 1.0);
            let ibc = (is / br) * (reverse - 1.0) + isc * ((vbc / (nc * VT)).exp() - 1.0);
            let ic = transport - ibc - device::GMIN * vbc;
            let ib = ibe + ibc + device::GMIN * (vbe + vbc);
            let ie = -(ic + ib);
            let direction = if vbe > 0.0 { "forward" } else { "reverse" };

            for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
                let kind = if polarity == Polarity::N {
                    "NPN"
                } else {
                    "PNP"
                };
                let netlist = format!(
                    "Vb b 0 dc {}\nVc c 0 dc 0\nVe e 0 dc {}\nQ1 c b e QMOD\n\
                     .model QMOD {kind}(IS={is} BF={bf} BR={br} ISE={ise} NE={ne} ISC={isc} NC={nc})\n",
                    sign * vbc,
                    sign * (vbc - vbe),
                );
                let solver = build(&netlist);
                let actual_ic = sign * source_current(&solver, "Vc");
                let actual_ib = sign * source_current(&solver, "Vb");
                let actual_ie = sign * source_current(&solver, "Ve");

                assert!(
                    relative(actual_ic, ic) < 1e-5,
                    "{kind} {direction}: collector current should be {ic} A, got {actual_ic} A"
                );
                assert!(
                    relative(actual_ib, ib) < 1e-5,
                    "{kind} {direction}: base current should be {ib} A, got {actual_ib} A"
                );
                assert!(
                    relative(actual_ie, ie) < 1e-5,
                    "{kind} {direction}: emitter current should be {ie} A, got {actual_ie} A"
                );
            }
        }
    }

    /// With the collector-base junction forward biased, both Gummel–Poon base
    /// recombination branches draw current through the external base resistor.
    /// Solve that resistor/device current balance independently, then check the
    /// actual MNA base voltage and drive current for NPN and PNP polarities.
    #[test]
    fn bjt_base_recombination_loads_a_resistively_driven_base() {
        let (is, bf, br, ise, ne, isc, nc, rb, vce, drive) = (
            1e-16, 80.0, 4.0, 1e-11, 1.5, 5e-10, 2.0, 10_000.0, 0.15, 1.0,
        );
        let base_current = |vbe: f64, include_recombination: bool| {
            let vbc = vbe - vce;
            let ise = if include_recombination { ise } else { 0.0 };
            let isc = if include_recombination { isc } else { 0.0 };
            (is / bf) * ((vbe / VT).exp() - 1.0)
                + ise * ((vbe / (ne * VT)).exp() - 1.0)
                + (is / br) * ((vbc / VT).exp() - 1.0)
                + isc * ((vbc / (nc * VT)).exp() - 1.0)
                + device::GMIN * (vbe + vbc)
        };
        let expected_vbe = |include_recombination: bool| {
            let mut lo = -0.1;
            let mut hi = drive;
            for _ in 0..100 {
                let mid = (lo + hi) / 2.0;
                let source_current = (drive - mid) / rb;
                if source_current > base_current(mid, include_recombination) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            (lo + hi) / 2.0
        };
        let expected = expected_vbe(true);
        let without_recombination = expected_vbe(false);
        assert!(expected > vce, "the selected bias must forward-bias B-C");
        assert!(
            without_recombination - expected > 0.1,
            "both recombination branches should materially load the base"
        );

        for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
            let emitter_v = if sign > 0.0 { 0.0 } else { 3.0 };
            let collector_v = emitter_v + sign * vce;
            let drive_v = emitter_v + sign * drive;
            let netlist = format!(
                "Ve e 0 dc {emitter_v}\nVc c 0 dc {collector_v}\n\
                 Vdrive drive 0 dc {drive_v}\nRbase drive b {rb}\n\
                 Q1 c b e QMOD\n\
                 .model QMOD {kind}(IS={is} BF={bf} BR={br} ISE={ise} NE={ne} ISC={isc} NC={nc})\n"
            );
            let solver = build(&netlist);
            let base = solver
                .circuit()
                .node("b")
                .expect("external base node")
                .expect("base is not ground");
            let actual_vbe = sign * (solver.node_voltage(Some(base)) - emitter_v);
            let actual_drive_current = sign * source_current(&solver, "Vdrive");
            let expected_drive_current = (drive - expected) / rb;

            assert!(
                (actual_vbe - expected).abs() < 1e-8,
                "{kind}: both recombination branches predict Vbe={expected} V, got {actual_vbe} V"
            );
            assert!(
                (actual_drive_current - expected_drive_current).abs() < 1e-10,
                "{kind}: the base drive should supply {expected_drive_current} A, got {actual_drive_current} A"
            );
        }
    }

    /// The model-card base resistance sits outside both Gummel–Poon
    /// recombination branches. Solve the current balance through the internal
    /// base node, then compare it with the same resistor written explicitly.
    #[test]
    fn bjt_base_recombination_flows_through_modelled_base_resistance() {
        let (is, bf, br, ise, ne, isc, nc, rb, drive_resistance, vce, drive) = (
            1e-16, 80.0, 4.0, 1e-11, 1.5, 5e-10, 2.0, 250.0, 10_000.0, 0.15, 1.0,
        );
        let base_current = |vbe: f64, include_recombination: bool| {
            let vbc = vbe - vce;
            let ise = if include_recombination { ise } else { 0.0 };
            let isc = if include_recombination { isc } else { 0.0 };
            (is / bf) * ((vbe / VT).exp() - 1.0)
                + ise * ((vbe / (ne * VT)).exp() - 1.0)
                + (is / br) * ((vbc / VT).exp() - 1.0)
                + isc * ((vbc / (nc * VT)).exp() - 1.0)
                + device::GMIN * (vbe + vbc)
        };
        let expected_vbe = |include_recombination: bool| {
            let mut lo = -0.1;
            let mut hi = drive;
            for _ in 0..100 {
                let mid = (lo + hi) / 2.0;
                let source_current = (drive - mid) / (drive_resistance + rb);
                if source_current > base_current(mid, include_recombination) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            (lo + hi) / 2.0
        };
        let expected = expected_vbe(true);
        let without_recombination = expected_vbe(false);
        assert!(expected > vce, "the selected bias must forward-bias B-C");
        assert!(
            without_recombination - expected > 0.1,
            "recombination must materially load the intrinsic base"
        );

        for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
            let emitter_v = if sign > 0.0 { 0.0 } else { 3.0 };
            let collector_v = emitter_v + sign * vce;
            let drive_v = emitter_v + sign * drive;
            let model = format!(
                "Ve e 0 dc {emitter_v}\nVc c 0 dc {collector_v}\n\
                 Vdrive drive 0 dc {drive_v}\nRdrive drive b {drive_resistance}\n\
                 Q1 c b e QMOD\n\
                 .model QMOD {kind}(IS={is} BF={bf} BR={br} RB={rb} \
                 ISE={ise} NE={ne} ISC={isc} NC={nc})\n"
            );
            let explicit = format!(
                "Ve e 0 dc {emitter_v}\nVc c 0 dc {collector_v}\n\
                 Vdrive drive 0 dc {drive_v}\nRdrive drive b {drive_resistance}\n\
                 Rbase b qb {rb}\nQ1 c qb e QMOD\n\
                 .model QMOD {kind}(IS={is} BF={bf} BR={br} \
                 ISE={ise} NE={ne} ISC={isc} NC={nc})\n"
            );
            let actual = build(&model);
            let reference = build(&explicit);
            let model_base = actual
                .circuit()
                .node("b")
                .expect("external base node")
                .expect("base is not ground");
            let model_intrinsic_base = actual
                .circuit()
                .node("Q1#base")
                .expect("internal base node")
                .expect("internal base is not ground");
            let actual_vbe = sign * (actual.node_voltage(Some(model_intrinsic_base)) - emitter_v);
            let actual_base_current = sign * source_current(&actual, "Vdrive");
            let actual_base_drop = sign
                * (actual.node_voltage(Some(model_base))
                    - actual.node_voltage(Some(model_intrinsic_base)));

            assert!(
                (actual_vbe - expected).abs() < 1e-8,
                "{kind}: recombination plus RB predict intrinsic Vbe={expected} V, got {actual_vbe} V"
            );
            assert!(
                (actual_base_current - base_current(expected, true)).abs() < 1e-10,
                "{kind}: recombination predicts Ib={} A, got {actual_base_current} A",
                base_current(expected, true)
            );
            assert!(
                (actual_base_drop - actual_base_current * rb).abs() < 1e-8,
                "{kind}: RB should drop {} V, got {actual_base_drop} V",
                actual_base_current * rb
            );
            for (model_name, reference_name) in
                [("b", "b"), ("Q1#base", "qb"), ("c", "c"), ("e", "e")]
            {
                let model_node = actual.circuit().node(model_name).expect("model node");
                let reference_node = reference
                    .circuit()
                    .node(reference_name)
                    .expect("reference node");
                assert!(
                    (actual.node_voltage(model_node) - reference.node_voltage(reference_node))
                        .abs()
                        < 1e-8,
                    "{kind} {model_name}: model-card RB differs from explicit resistor"
                );
            }
            for source in ["Vdrive", "Vc", "Ve"] {
                assert!(
                    (source_current(&actual, source) - source_current(&reference, source)).abs()
                        < 1e-10,
                    "{kind} {source}: model-card RB current differs from explicit resistor"
                );
            }
        }
    }

    /// Base recombination changes the current that controls RBM/IRB, so both
    /// effects must be solved together at the intrinsic junctions. Compare the
    /// MNA result with an independent root of the base-current and spreading-
    /// resistance equations for NPN and PNP devices.
    #[test]
    fn bjt_recombination_sets_bias_dependent_base_resistance() {
        let (is, bf, br, ise, ne, isc, nc, rb, rbm, irb, drive_resistance, vce, drive) = (
            1e-16, 80.0, 4.0, 1e-11, 1.5, 5e-10, 2.0, 1000.0, 10.0, 100e-6, 4000.0, 0.15, 1.2,
        );
        let base_current = |vbe: f64, include_recombination: bool| {
            let vbc = vbe - vce;
            let ise = if include_recombination { ise } else { 0.0 };
            let isc = if include_recombination { isc } else { 0.0 };
            (is / bf) * ((vbe / VT).exp() - 1.0)
                + ise * ((vbe / (ne * VT)).exp() - 1.0)
                + (is / br) * ((vbc / VT).exp() - 1.0)
                + isc * ((vbc / (nc * VT)).exp() - 1.0)
                + device::GMIN * (vbe + vbc)
        };
        // Independent transcription of the SPICE RBM/IRB interpolation. The
        // input is the total base current, including both recombination legs.
        let base_resistance = |current: f64| {
            let x = (current / irb).max(1e-9);
            let z = ((1.0 + 14.59025 * x).sqrt() - 1.0) / (2.4317 * x.sqrt());
            let tan_z = z.tan();
            rbm + 3.0 * (rb - rbm) * (tan_z - z) / (z * tan_z * tan_z)
        };
        let expected_vbe = |include_recombination: bool| {
            let mut lo = -0.1;
            let mut hi = drive;
            for _ in 0..100 {
                let mid = (lo + hi) / 2.0;
                let current = base_current(mid, include_recombination);
                let drive_current = (drive - mid) / (drive_resistance + base_resistance(current));
                if drive_current > current {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            (lo + hi) / 2.0
        };
        let expected = expected_vbe(true);
        let without_recombination = expected_vbe(false);
        let expected_current = base_current(expected, true);
        let expected_resistance = base_resistance(expected_current);
        assert!(expected > vce, "the selected bias must forward-bias B-C");
        assert!(
            without_recombination - expected > 0.05,
            "recombination must materially change the bias, with/without={without_recombination}/{expected} V"
        );
        assert!(
            expected_resistance < rb * 0.8 && expected_resistance > rbm,
            "the solved current should move RB into its transition: {expected_resistance} Ω"
        );

        for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
            let emitter_v = if sign > 0.0 { 0.0 } else { 3.0 };
            let collector_v = emitter_v + sign * vce;
            let drive_v = emitter_v + sign * drive;
            let netlist = format!(
                "Ve e 0 dc {emitter_v}\nVc c 0 dc {collector_v}\nVdrive drive 0 dc {drive_v}\nRdrive drive b {drive_resistance}\nQ1 c b e QMOD\n.model QMOD {kind}(IS={is} BF={bf} BR={br} RB={rb} RBM={rbm} IRB={irb} ISE={ise} NE={ne} ISC={isc} NC={nc})\n"
            );
            let solver = build(&netlist);
            let base = solver
                .circuit()
                .node("b")
                .expect("external base node")
                .expect("base is not ground");
            let internal_base = solver
                .circuit()
                .node("Q1#base")
                .expect("internal base node")
                .expect("base is not ground");
            let actual_vbe = sign * (solver.node_voltage(Some(internal_base)) - emitter_v);
            let actual_current = sign * source_current(&solver, "Vdrive");
            let actual_base_drop =
                sign * (solver.node_voltage(Some(base)) - solver.node_voltage(Some(internal_base)));

            assert!(
                (actual_vbe - expected).abs() < 1e-8,
                "{kind}: recombination plus RBM/IRB predict intrinsic Vbe={expected} V, got {actual_vbe} V"
            );
            assert!(
                (actual_current - expected_current).abs() < 1e-10,
                "{kind}: recombination predicts Ib={expected_current} A, got {actual_current} A"
            );
            assert!(
                (actual_base_drop - expected_current * expected_resistance).abs() < 1e-8,
                "{kind}: RB(Ib)={expected_resistance} Ω predicts base drop {} V, got {actual_base_drop} V",
                expected_current * expected_resistance
            );
        }
    }

    #[test]
    fn bjt_base_recombination_partials_match_finite_difference() {
        let (is, bf, br, nf, nr, ise, ne, isc, nc) =
            (2e-15, 75.0, 3.0, 1.1, 1.2, 3e-12, 1.5, 5e-12, 2.0);
        let delta = 1e-7;
        let base_emitter_current = |vbe: f64| {
            (is / bf) * ((vbe / (nf * VT)).exp() - 1.0) + ise * ((vbe / (ne * VT)).exp() - 1.0)
        };
        let base_collector_current = |vbc: f64| {
            (is / br) * ((vbc / (nr * VT)).exp() - 1.0) + isc * ((vbc / (nc * VT)).exp() - 1.0)
        };

        for (vbe, vbc) in [(0.62, -0.1), (-0.1, 0.58)] {
            let op = device::bjt_op(
                vbe,
                vbc,
                is,
                bf,
                br,
                nf,
                nr,
                f64::INFINITY,
                f64::INFINITY,
                f64::INFINITY,
                f64::INFINITY,
                ise,
                ne,
                isc,
                nc,
            );
            let expected_gpi = (base_emitter_current(vbe + delta)
                - base_emitter_current(vbe - delta))
                / (2.0 * delta);
            let expected_gmu = (base_collector_current(vbc + delta)
                - base_collector_current(vbc - delta))
                / (2.0 * delta);
            assert!(
                relative(op.gpi, expected_gpi) < 1e-6,
                "gpi: {} vs {expected_gpi}",
                op.gpi
            );
            assert!(
                relative(op.gmu, expected_gmu) < 1e-6,
                "gmu: {} vs {expected_gmu}",
                op.gmu
            );
        }
    }

    // -----------------------------------------------------------------------
    // MOSFET
    // -----------------------------------------------------------------------

    /// Level 1 in all three regions, against the Shichman–Hodges closed form.
    #[test]
    fn mosfet_operating_point_matches_shichman_hodges_by_hand() {
        let kp = 20e-6;
        let (w, l) = (100e-6, 10e-6);
        let beta = kp * w / l;
        let lambda = 0.02;
        for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
            let kind = if polarity == Polarity::N {
                "NMOS"
            } else {
                "PMOS"
            };
            let vto = 1.0;
            for (vgs, vds) in [(0.5, 5.0), (3.0, 5.0), (3.0, 0.5)] {
                let netlist = format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nM1 d g 0 0 MMOD\n\
                     .model MMOD {kind}(VTO={} KP={kp} LAMBDA={lambda} W={w} L={l})\n",
                    sign * vgs,
                    sign * vds,
                    sign * vto
                );
                let solver = build(&netlist);

                let vgst = vgs - vto;
                let expected = if vgst <= 0.0 {
                    0.0
                } else if vds < vgst {
                    beta * (1.0 + lambda * vds) * vds * (vgst - 0.5 * vds)
                } else {
                    0.5 * beta * (1.0 + lambda * vds) * vgst * vgst
                };
                // Two GMIN ties reach the drain: channel and bulk.
                let expected = expected + 2.0 * device::GMIN * vds;
                let measured = sign * source_current(&solver, "Vd");
                let label = format!("{kind} Vgs={vgs} Vds={vds}");
                if expected.abs() < 1e-9 {
                    assert!(
                        measured.abs() < 1e-9,
                        "{label}: cut off, expected ~0 A, got {measured} A"
                    );
                } else {
                    assert!(
                        relative(measured, expected) < 1e-6,
                        "{label}: expected {expected} A, got {measured} A"
                    );
                }
            }
        }
    }

    /// A MOSFET is symmetric. Driving the terminal the netlist calls the drain
    /// BELOW the one it calls the source must give the mirror image of the
    /// forward characteristic, not a device stuck at zero — the failure mode a
    /// model without reverse mode has, and one that a pass transistor or a
    /// low-side switch with a reactive load hits immediately.
    #[test]
    fn a_mosfet_conducts_in_reverse_with_the_mirrored_characteristic() {
        let deck = |vd: f64, vs: f64| {
            format!(
                "Vg g 0 dc 3\nVd d 0 dc {vd}\nVs s 0 dc {vs}\nM1 d g s 0 MMOD\n\
                 .model MMOD NMOS(VTO=1 KP=20u LAMBDA=0 W=100u L=10u)\n"
            )
        };
        // Gate 3 V above whichever terminal is the source in each case.
        let forward = source_current(&build(&deck(0.4, 0.0)), "Vd");
        let reverse = source_current(&build(&deck(0.0, 0.4)), "Vs");
        assert!(
            forward > 1e-6,
            "forward conduction should be micro-amps or more, got {forward} A"
        );
        assert!(
            relative(reverse, forward) < 1e-6,
            "reverse conduction should mirror forward: {forward} A vs {reverse} A"
        );
    }

    /// The fourth terminal is not silently grounded: in cutoff, its two bulk
    /// junctions carry the hand-calculated reverse leakage, including GMIN.
    /// The normalized voltages keep both body diodes reverse biased, and the
    /// same stamp is checked for NMOS and PMOS.
    #[test]
    fn mosfet_bulk_terminal_carries_its_two_reverse_junction_currents() {
        let (vds, vbulk) = (0.4, -0.1);
        let is = 1e-14;
        let vbd = vbulk - vds;
        let vbs = vbulk;
        let bulk_to_drain = is * ((vbd / VT).exp() - 1.0) + device::GMIN * vbd;
        let bulk_to_source = is * ((vbs / VT).exp() - 1.0) + device::GMIN * vbs;
        let expected_drain = device::GMIN * vds - bulk_to_drain;
        let expected_bulk = bulk_to_drain + bulk_to_source;
        let expected_source = -device::GMIN * vds - bulk_to_source;

        for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
            let kind = if polarity == Polarity::N {
                "NMOS"
            } else {
                "PMOS"
            };
            let netlist = format!(
                "Vg g 0 dc 0\nVd d 0 dc {}\nVs s 0 dc 0\nVbulk b 0 dc {}\n\
                 M1 d g s b MMOD\n.model MMOD {kind}(VTO={} KP=20u LAMBDA=0 IS={is})\n",
                sign * vds,
                sign * vbulk,
                sign
            );
            let solver = build(&netlist);
            let actual_drain = sign * source_current(&solver, "Vd");
            let actual_bulk = sign * source_current(&solver, "Vbulk");
            let actual_source = sign * source_current(&solver, "Vs");

            assert!(
                (actual_drain - expected_drain).abs() < device::GMIN * 1e-3,
                "{kind}: drain current should include drain-source and drain-bulk GMIN ties; \
                 expected {expected_drain} A, got {actual_drain} A"
            );
            assert!(
                (actual_bulk - expected_bulk).abs() < device::GMIN * 1e-3,
                "{kind}: bulk current should be {expected_bulk} A, got {actual_bulk} A"
            );
            assert!(
                (actual_source - expected_source).abs() < device::GMIN * 1e-3,
                "{kind}: source current should include its bulk tie; expected {expected_source} A, \
                 got {actual_source} A"
            );
        }
    }

    /// The level-1 bulk junctions conduct from bulk to drain/source for NMOS,
    /// and in the opposite physical direction for PMOS. With the channel off,
    /// the two terminal currents reduce to the Shockley equation.
    #[test]
    fn mosfet_body_diodes_match_the_shockley_equation_for_both_polarities() {
        let is = 2e-14;
        let vbody = 0.55;
        let expected_each = is * ((vbody / VT).exp() - 1.0) + device::GMIN * vbody;

        for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
            let kind = if polarity == Polarity::N {
                "NMOS"
            } else {
                "PMOS"
            };
            let netlist = format!(
                "Vg g 0 dc 0\nVd d 0 dc 0\nVs s 0 dc 0\nVbulk b 0 dc {}\n\
                 M1 d g s b MMOD\n.model MMOD {kind}(VTO={} KP=20u LAMBDA=0 IS={is})\n",
                sign * vbody,
                sign
            );
            let solver = build(&netlist);
            assert_eq!(solver.circuit().mosfets[0].model.is, is);

            let bulk_current = sign * source_current(&solver, "Vbulk");
            let drain_current = sign * source_current(&solver, "Vd");
            let source_current = sign * source_current(&solver, "Vs");
            assert!(
                relative(bulk_current, 2.0 * expected_each) < 1e-6,
                "{kind}: bulk should supply both body diode currents ({}) A, got {bulk_current} A",
                2.0 * expected_each
            );
            assert!(
                relative(drain_current, -expected_each) < 1e-6,
                "{kind}: drain should receive one body diode current (-{expected_each} A), got {drain_current} A"
            );
            assert!(
                relative(source_current, -expected_each) < 1e-6,
                "{kind}: source should receive one body diode current (-{expected_each} A), got {source_current} A"
            );
        }
    }

    /// Level-1 body effect shifts the threshold for reverse body bias. Check
    /// its closed-form current and Newton stamp for both channel polarities,
    /// with either physical drain acting as the effective source.
    #[test]
    fn mosfet_body_effect_matches_level1_for_both_polarities_and_directions() {
        let (kp, w, l, lambda, gamma, phi, vto): (f64, f64, f64, f64, f64, f64, f64) =
            (20e-6, 100e-6, 10e-6, 0.02, 0.4, 0.7, 1.0);
        let beta = kp * w / l;
        let vds = 0.4;
        let vgs = 3.0;
        let vbs = -0.2;
        let is = 1e-14;
        let threshold = vto + gamma * ((phi - vbs).sqrt() - phi.sqrt());
        let overdrive = vgs - threshold;
        let channel = beta * (1.0 + lambda * vds) * vds * (overdrive - 0.5 * vds);
        let vbd = vbs - vds;
        let expected = channel + device::GMIN * vds - device::diode_op(vbd, is, 1.0).id;

        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            for reverse in [false, true] {
                let (drain, source, supply) = if reverse {
                    (0.0, vds, "Vs")
                } else {
                    (vds, 0.0, "Vd")
                };
                let netlist = format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nVs s 0 dc {}\nVbulk b 0 dc {}\nM1 d g s b MMOD\n.model MMOD {kind}(VTO={} KP={kp} LAMBDA={lambda} GAMMA={gamma} PHI={phi} IS={is} W={w} L={l})\n",
                    sign * vgs,
                    sign * drain,
                    sign * source,
                    sign * vbs,
                    sign * vto,
                );
                let solver = build(&netlist);
                let measured = sign * source_current(&solver, supply);
                let direction = if reverse { "reverse" } else { "forward" };
                assert!(
                    relative(measured, expected) < 1e-6,
                    "{kind} {direction}: body-shifted current should be {expected} A, got {measured} A"
                );
            }
        }
    }

    /// ngspice level 1 shortens the channel by lateral diffusion `LD` and,
    /// when `KP` is absent, takes it from the process: `UO·1e-4·εox/TOX`.
    /// Check the solved drain current against the hand Shichman–Hodges law
    /// for both polarities, so an engine that ignored `LD`, `UO` or the
    /// derivation would miss by a factor of two or more.
    #[test]
    fn mosfet_lateral_diffusion_and_process_kp_follow_ngspice_level1() {
        let (w, l, vto, lambda, vgs, vds): (f64, f64, f64, f64, f64, f64) =
            (10e-6, 2e-6, 1.0, 0.02, 2.0, 3.0);
        let saturation = |kp: f64, length: f64| {
            0.5 * kp * w / length * (vgs - vto).powi(2) * (1.0 + lambda * vds) + device::GMIN * vds
        };
        let tox = 20e-9;
        let process_kp = |uo: f64| uo * 1e-4 * device::OXIDE_PERMITTIVITY_F_PER_M / tox;
        let cases = [
            ("drawn length", "KP=50u", 50e-6, l),
            ("LD=0.5u", "KP=50u LD=0.5u", 50e-6, l - 1e-6),
            ("UO default 600", "TOX=20n", process_kp(600.0), l),
            ("UO=400", "TOX=20n UO=400", process_kp(400.0), l),
            ("U0 alias", "TOX=20n U0=400", process_kp(400.0), l),
            (
                "UO with LD",
                "TOX=20n UO=400 LD=0.25u",
                process_kp(400.0),
                l - 0.5e-6,
            ),
            ("explicit KP beats UO", "KP=50u TOX=20n UO=400", 50e-6, l),
        ];
        assert!(
            (process_kp(600.0) / 50e-6 - 1.0).abs() > 0.5,
            "the cases must be far enough apart to tell the laws apart"
        );
        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            for (label, params, kp, length) in cases {
                let netlist = format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nM1 d g 0 0 MMOD W={w} L={l}\n.model MMOD {kind}(VTO={} LAMBDA={lambda} {params})\n",
                    sign * vgs,
                    sign * vds,
                    sign * vto,
                );
                let circuit = parse_netlist(&netlist).expect("level-1 card parses");
                let mosfet = &circuit.mosfets[0];
                assert_eq!(mosfet.l, l, "{kind} {label}: drawn L is kept");
                assert!(
                    relative(mosfet.effective_length(), length) < 1e-12,
                    "{kind} {label}: effective length"
                );
                assert!(
                    relative(mosfet.beta, kp * w / length) < 1e-12,
                    "{kind} {label}: beta should be {} A/V², got {}",
                    kp * w / length,
                    mosfet.beta
                );
                let solver = build(&netlist);
                let measured = sign * source_current(&solver, "Vd");
                let expected = saturation(kp, length);
                assert!(
                    relative(measured, expected) < 1e-6,
                    "{kind} {label}: drain current should be {expected} A, got {measured} A"
                );
            }
        }

        let overlap = parse_netlist(
            "M1 d g 0 0 MMOD W=10u L=2u\n.model MMOD NMOS(KP=50u LD=0.5u CGBO=2n CGSO=3n)\n",
        )
        .expect("overlap card parses")
        .mosfets[0]
            .overlap_capacitances();
        assert!(
            relative(overlap[2], 2e-9 * 1e-6) < 1e-12,
            "CGBO uses L - 2*LD: {overlap:?}"
        );
        assert!(
            relative(overlap[0], 3e-9 * 10e-6) < 1e-12,
            "CGSO still uses W: {overlap:?}"
        );

        for (params, needle) in [
            ("KP=50u LD=1u", "L - 2*LD"),
            ("KP=50u LD=2u", "L - 2*LD"),
            ("TOX=20n UO=0", "UO"),
            ("TOX=20n UO=-5", "UO"),
        ] {
            let error = parse_netlist(&format!(
                "M1 d g 0 0 MMOD W=10u L=2u\n.model MMOD NMOS({params})\n"
            ))
            .expect_err("unphysical LD/UO must be refused")
            .to_string();
            assert!(error.contains(needle), "{params}: unexpected error {error}");
        }
    }

    /// ngspice level 1 derives the threshold from the process when a card has
    /// `TOX` and `NSUB`: `PHI` from the doping, `GAMMA` from the depletion
    /// charge, and `VTO` from the flat-band voltage (gate type `TPG`, surface
    /// states `NSS`). The expected values are ngspice-45 `showmod` output and
    /// `.op` drain voltages for the same deck; an engine that ignored `NSUB`
    /// would leave `VTO=0`, `GAMMA=0`, `PHI=0.6` and miss every one of them.
    #[test]
    fn mosfet_substrate_doping_derives_threshold_like_ngspice_level1() {
        let deck = "V1 vdd 0 3.3\nVG g 0 2.5\nVGP gp 0 0.8\n\
            R1 vdd d1 10k\nR2 vdd d2 10k\nR3 d3 0 10k\nR4 vdd d4 10k\n\
            M1 d1 g 0 0 MN W=10u L=2u\nM2 d2 g 0 0 MN2 W=10u L=2u\n\
            M3 d3 gp vdd vdd MP W=10u L=2u\nM4 d4 g 0 0 MN3 W=10u L=2u\n\
            .model MN NMOS(LEVEL=1 TOX=20n NSUB=1e16 UO=400)\n\
            .model MN2 NMOS(LEVEL=1 TOX=20n NSUB=1e16 NSS=1e11 TPG=-1 UO=400)\n\
            .model MP PMOS(LEVEL=1 TOX=20n NSUB=5e15 UO=200)\n\
            .model MN3 NMOS(LEVEL=1 TOX=20n NSUB=1e16 PHI=0.7 UO=400)\n";
        let circuit = parse_netlist(deck).expect("process cards parse");
        // (device, VTO, GAMMA, PHI, drain node, drain volts) from ngspice.
        let reference = [
            ("M1", 0.068_466_2, 0.333_698, 0.695_453, "d1", 0.377_363_2),
            ("M2", 1.090_76, 0.333_698, 0.695_453, "d2", 0.711_548_3),
            ("M3", 0.036_109_2, 0.235_96, 0.659_597, "d3", 2.610_082),
            ("M4", 0.071_647_8, 0.333_698, 0.7, "d4", 0.377_876_2),
        ];
        let solver = build(deck);
        for (name, vto, gamma, phi, node, volts) in reference {
            let mosfet = circuit
                .mosfets
                .iter()
                .find(|mosfet| mosfet.name == name)
                .expect("device exists");
            let model = mosfet.model;
            // ngspice's own physical constants differ from CODATA 2018 in the
            // seventh digit; 1e-4 still separates every case below.
            assert!(
                (model.vto - vto).abs() < 1e-4,
                "{name}: VTO should be {vto}, got {}",
                model.vto
            );
            assert!(
                relative(model.gamma, gamma) < 1e-4,
                "{name}: GAMMA should be {gamma}, got {}",
                model.gamma
            );
            assert!(
                relative(model.phi, phi) < 1e-4,
                "{name}: PHI should be {phi}, got {}",
                model.phi
            );
            let measured = solver.node_voltage(solver.circuit().node(node).expect("node exists"));
            assert!(
                (measured - volts).abs() < 1e-4,
                "{name}: V({node}) should be {volts} V, got {measured} V"
            );
        }

        // Explicit values beat the derivation; NSUB without TOX is ignored,
        // as in ngspice.
        let explicit =
            parse_netlist("M1 d g 0 0 MN\n.model MN NMOS(TOX=20n NSUB=1e16 VTO=0.8 GAMMA=0.5)\n")
                .expect("explicit card parses")
                .mosfets[0]
                .model;
        assert_eq!((explicit.vto, explicit.gamma), (0.8, 0.5));
        assert!(relative(explicit.phi, 0.695_453) < 1e-4);
        let no_tox = parse_netlist("M1 d g 0 0 MN\n.model MN NMOS(NSUB=1e16)\n")
            .expect("card without TOX parses")
            .mosfets[0]
            .model;
        assert_eq!((no_tox.vto, no_tox.gamma, no_tox.phi), (0.0, 0.0, 0.6));

        for (params, needle) in [
            ("TOX=20n NSUB=1e9", "NSUB"),
            ("TOX=20n NSUB=1e16 TPG=2", "TPG"),
        ] {
            let error = parse_netlist(&format!("M1 d g 0 0 MN\n.model MN NMOS({params})\n"))
                .expect_err("unphysical process card must be refused")
                .to_string();
            assert!(error.contains(needle), "{params}: unexpected error {error}");
        }
    }

    /// ngspice level 1 (`mos1load.c`) replaces the bulk-junction `IS` with
    /// `JS·AD` and `JS·AS` only when `JS`, `AD` and `AS` are all non-zero, and
    /// `mos1temp.c` scales `JS` with the same bandgap law as `IS`. Each bulk
    /// is forward biased through 100 kΩ with drain, gate and source grounded,
    /// so its voltage reads the summed junction saturation current. Expected
    /// voltages are ngspice-45 `.op` output for the same deck (TNOM = TEMP =
    /// 26.85 °C, the engine's fixed 300 K, except the `TEMP=85` row at the
    /// default TNOM of 27 °C). An engine that ignored `JS` would solve every
    /// row except `b_fall` at 0.544 V; one that used `AD` for both junctions
    /// would put `b_asym` 10 mV low; one that did not scale `JS` with `TEMP`
    /// would put `b_hot` above 0.5 V.
    #[test]
    fn mosfet_junction_saturation_current_density_follows_ngspice_level1() {
        let deck = "Vdd supply 0 dc 3.3\n\
            Rarea supply b_area 100k\nRasym supply b_asym 100k\n\
            Rfall supply b_fall 100k\nRhot supply b_hot 100k\n\
            Marea 0 0 0 b_area NJS W=10u L=2u AD=100p AS=100p\n\
            Masym 0 0 0 b_asym NJS W=10u L=2u AD=300p AS=100p\n\
            Mfall 0 0 0 b_fall NJS W=10u L=2u AD=300p\n\
            Mhot 0 0 0 b_hot NJS W=10u L=2u AD=100p AS=100p TEMP=85\n\
            .model NJS NMOS(LEVEL=1 VTO=1 KP=20u IS=1e-14 JS=1e-2)\n";
        let circuit = parse_netlist(deck).expect("JS card parses");
        // (device, drain IS, source IS at the card temperature) from JS·AD/AS.
        for (name, drain_is, source_is) in [
            ("Marea", 1e-12, 1e-12),
            ("Masym", 3e-12, 1e-12),
            ("Mfall", 1e-14, 1e-14),
        ] {
            let mosfet = circuit
                .mosfets
                .iter()
                .find(|mosfet| mosfet.name == name)
                .expect("device exists");
            let (drain, source) = mosfet.junction_saturation_currents();
            assert!(
                relative(drain, drain_is) < 1e-12 && relative(source, source_is) < 1e-12,
                "{name}: junction IS should be ({drain_is}, {source_is}), got ({drain}, {source})"
            );
        }
        let solver = build(deck);
        for (node, volts) in [
            ("b_area", 0.426_057_2),
            ("b_asym", 0.408_297_2),
            ("b_fall", 0.544_026_4),
            ("b_hot", 0.278_212_6),
        ] {
            let measured = solver.node_voltage(solver.circuit().node(node).expect("node exists"));
            assert!(
                (measured - volts).abs() < 1e-4,
                "V({node}) should be {volts} V, got {measured} V"
            );
        }

        for (params, needle) in [("JS=-1", "JS"), ("JS=1e308", "JS")] {
            let error = parse_netlist(&format!(
                "M1 d g 0 0 MN AD=1e10 AS=1e10\n.model MN NMOS({params})\n"
            ))
            .expect_err("unphysical JS must be refused")
            .to_string();
            assert!(error.contains(needle), "{params}: unexpected error {error}");
        }
    }

    /// Forward body bias uses the level-1 linear continuation of the square
    /// root. Check its channel current and `gmb` against the hand equation,
    /// then exercise that same bias through MNA for both polarities and both
    /// choices of effective source.
    #[test]
    fn mosfet_forward_body_effect_uses_the_level1_linear_continuation() {
        let (kp, w, l, lambda, gamma, phi, vto): (f64, f64, f64, f64, f64, f64, f64) =
            (20e-6, 100e-6, 10e-6, 0.02, 0.4, 0.7, 1.0);
        let beta = kp * w / l;
        let vds = 0.4;
        let vgs = 3.0;
        let vbs = 0.2;
        let is = 1e-30;
        let root_phi = phi.sqrt();
        let threshold = vto - gamma * vbs / (2.0 * root_phi);
        let overdrive = vgs - threshold;
        let channel = beta * (1.0 + lambda * vds) * vds * (overdrive - 0.5 * vds);
        let vbd = vbs - vds;
        let expected = channel + device::GMIN * vds - device::diode_op(vbd, is, 1.0).id;

        let op = device::mos_op(vgs, vds, vto, beta, lambda, gamma, phi, vbs);
        let dv = 1e-6;
        let finite_difference_gmb =
            (device::mos_op(vgs, vds, vto, beta, lambda, gamma, phi, vbs + dv).id
                - device::mos_op(vgs, vds, vto, beta, lambda, gamma, phi, vbs - dv).id)
                / (2.0 * dv);
        assert!(
            relative(op.id, channel) < 1e-12,
            "forward body bias should lower Vth by the linear continuation: expected {channel} A, got {} A",
            op.id
        );
        assert!(
            relative(op.gmb, finite_difference_gmb) < 1e-8,
            "forward body-effect gmb should match the derivative: expected {finite_difference_gmb} S, got {} S",
            op.gmb
        );

        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            for reverse in [false, true] {
                // In reverse operation the physical drain becomes the
                // effective source, so keep its body bias at +vbs.
                let (drain, source, supply, bulk) = if reverse {
                    (0.0, vds, "Vs", vbs)
                } else {
                    (vds, 0.0, "Vd", vbs)
                };
                let netlist = format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nVs s 0 dc {}\nVbulk b 0 dc {}\nM1 d g s b MMOD\n.model MMOD {kind}(VTO={} KP={kp} LAMBDA={lambda} GAMMA={gamma} PHI={phi} IS={is} W={w} L={l})\n",
                    sign * vgs,
                    sign * drain,
                    sign * source,
                    sign * bulk,
                    sign * vto,
                );
                let solver = build(&netlist);
                let measured = sign * source_current(&solver, supply);
                let direction = if reverse { "reverse" } else { "forward" };
                assert!(
                    relative(measured, expected) < 1e-6,
                    "{kind} {direction}: forward-body-biased current should be {expected} A, got {measured} A"
                );
            }
        }
    }

    /// The three level-1 gate overlap parameters become linear G-S, G-D and
    /// G-B capacitances after applying the instance width/length. Each route
    /// is solved as an RC transient for NMOS and PMOS under both integration
    /// rules; the first step restarts with BE and the second uses its selected
    /// rule and the prior charge/current history.
    #[test]
    fn mosfet_overlap_charge_integrates_cgso_cgdo_and_cgbo() {
        let width = 2e-3;
        let length = 3e-3;
        let resistance = 1e3;
        let h = 1e-12;
        let input = 0.2;
        let cases = [
            ("CGSO", "2.5p", 2.5e-12 * width, 0),
            ("CGDO", "5p", 5e-12 * width, 1),
            (
                "CGBO",
                "6.666666666666667p",
                6.666666666666667e-12 * length,
                2,
            ),
        ];

        for (parameter, value, capacitance, junction) in cases {
            for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
                let (drain, source, bulk) = match junction {
                    0 => ("0", "x", "0"),
                    1 => ("x", "0", "0"),
                    2 => ("0", "0", "x"),
                    _ => unreachable!(),
                };
                let vto = if kind == "NMOS" { 2.0 } else { -2.0 };
                let netlist = format!(
                    "Vg g 0 dc 0\nR1 x 0 {resistance}\n\
                     M1 {drain} g {source} {bulk} MMOD w={width} l={length}\n\
                     .model MMOD {kind}(VTO={vto} KP=20u IS=1e-30 {parameter}={value})\n"
                );
                let circuit = parse_netlist(&netlist).expect("MOS overlap model parses");
                let mosfet = &circuit.mosfets[0];
                assert!((mosfet.overlap_capacitances()[junction] - capacitance).abs() < 1e-27);
                for (index, other) in mosfet.overlap_capacitances().into_iter().enumerate() {
                    if index != junction {
                        assert_eq!(other, 0.0);
                    }
                }

                let initially_biased =
                    netlist.replacen("Vg g 0 dc 0", &format!("Vg g 0 dc {}", sign * input), 1);
                let mut biased_solver = Solver::new(
                    parse_netlist(&initially_biased).expect("prebiased MOS overlap parses"),
                    Integration::BackwardEuler,
                )
                .expect("prebiased MOS overlap solver builds");
                let biased_x = biased_solver
                    .circuit()
                    .node("x")
                    .expect("overlap terminal node");
                biased_solver
                    .advance(h)
                    .expect("operating-point overlap charge carries into transient");
                assert!(
                    biased_solver.node_voltage(biased_x).abs() < 1e-12,
                    "{kind} {parameter} must not create startup current from its operating-point charge"
                );

                let ratio = resistance * capacitance / h;
                let expected_first = sign * input * ratio / (1.0 + ratio);
                for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                    let mut solver = Solver::new(circuit.clone(), integration)
                        .expect("MOS overlap solver builds");
                    let x = solver.circuit().node("x").expect("overlap terminal node");
                    let source = solver.voltage_source_index("Vg").expect("gate source");
                    solver.set_voltage_source(source, sign * input);
                    solver.advance(h).expect("first MOS overlap step solves");
                    let actual_first = solver.node_voltage(x);
                    assert!(
                        (actual_first - expected_first).abs() < 1e-9,
                        "{kind} {parameter} first step: expected {expected_first} V, got {actual_first} V"
                    );

                    let expected_second = match integration {
                        Integration::BackwardEuler => ratio * expected_first / (1.0 + ratio),
                        Integration::Trapezoidal => {
                            ratio * (3.0 * expected_first - sign * input) / (1.0 + 2.0 * ratio)
                        }
                    };
                    solver.advance(h).expect("second MOS overlap step solves");
                    let actual_second = solver.node_voltage(x);
                    assert!(
                        (actual_second - expected_second).abs() < 1e-9,
                        "{kind} {parameter} {integration:?} second step: expected {expected_second} V, got {actual_second} V"
                    );
                }
            }
        }
    }

    /// Gate-drain overlap charge belongs to the intrinsic drain when model
    /// card `RD` inserts a series resistance. Its transient must therefore
    /// create the same internal-node trajectory as an explicit resistor, and
    /// match the hand-derived RC companion equation for both polarities and
    /// integration rules.
    #[test]
    fn mosfet_gate_drain_overlap_charge_flows_through_drain_resistance() {
        let (resistance, capacitance, width, h, input) = (1e3, 1e-12, 100e-6, 1e-9, 1.0);
        let ratio = resistance * capacitance / h;

        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            let vto = if kind == "NMOS" { 2.0 } else { -2.0 };
            let model_resistance = format!(
                "Vg g 0 dc 0\nVd d 0 dc 0\nVs s 0 dc 0\nVb b 0 dc 0\n\
                 M1 d g s b MMOD w={width}\n\
                 .model MMOD {kind}(VTO={vto} KP=20u LAMBDA=0 IS=1e-30 \
                 CGDO=10n RD={resistance})\n"
            );
            let explicit_resistance = format!(
                "Vg g 0 dc 0\nVd d 0 dc 0\nVs s 0 dc 0\nVb b 0 dc 0\n\
                 RDM d di {resistance}\nM1 di g s b MMOD w={width}\n\
                 .model MMOD {kind}(VTO={vto} KP=20u LAMBDA=0 IS=1e-30 CGDO=10n)\n"
            );

            for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                let mut model = Solver::new(
                    parse_netlist(&model_resistance).expect("MOSFET RD overlap model parses"),
                    integration,
                )
                .expect("MOSFET RD overlap solver builds");
                let mut explicit = Solver::new(
                    parse_netlist(&explicit_resistance)
                        .expect("explicit MOSFET drain resistor netlist parses"),
                    integration,
                )
                .expect("explicit MOSFET resistor solver builds");

                let model_internal = model
                    .circuit()
                    .node("M1#drain")
                    .expect("model-card RD creates the intrinsic drain");
                let explicit_internal = explicit
                    .circuit()
                    .node("di")
                    .expect("explicit resistor creates the intrinsic drain");
                let model_gate = model.voltage_source_index("Vg").expect("model gate source");
                let explicit_gate = explicit
                    .voltage_source_index("Vg")
                    .expect("explicit gate source");

                model.set_voltage_source(model_gate, sign * input);
                explicit.set_voltage_source(explicit_gate, sign * input);

                let expected_first = sign * input * ratio / (1.0 + ratio);
                let expected_second = match integration {
                    Integration::BackwardEuler => ratio * expected_first / (1.0 + ratio),
                    Integration::Trapezoidal => {
                        ratio * (3.0 * expected_first - sign * input) / (1.0 + 2.0 * ratio)
                    }
                };

                for (step, expected) in [expected_first, expected_second].into_iter().enumerate() {
                    model.advance(h).expect("model-card RD overlap step solves");
                    explicit
                        .advance(h)
                        .expect("explicit-resistor overlap step solves");

                    let actual = model.node_voltage(model_internal);
                    let reference = explicit.node_voltage(explicit_internal);
                    assert!(
                        (actual - expected).abs() < 1e-8,
                        "{kind} {integration:?} step {step}: CGDO should put the intrinsic drain at {expected} V, got {actual} V"
                    );
                    assert!(
                        (actual - reference).abs() < 1e-10,
                        "{kind} {integration:?} step {step}: model-card RD gives {actual} V, explicit drain resistance gives {reference} V"
                    );
                    if step == 0 {
                        assert!(
                            sign * actual > 0.1,
                            "{kind} CGDO displacement current must create a measurable RD drop"
                        );
                    }
                }
            }
        }
    }

    /// Gate-source overlap charge belongs to the intrinsic source when model
    /// card `RS` inserts a series resistance. Its transient must therefore
    /// create the same internal-node trajectory as an explicit resistor, and
    /// match the hand-derived RC companion equation for both polarities and
    /// integration rules.
    #[test]
    fn mosfet_gate_source_overlap_charge_flows_through_source_resistance() {
        let (resistance, capacitance, width, h, input) = (1e3, 1e-12, 100e-6, 1e-9, 1.0);
        let ratio = resistance * capacitance / h;

        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            let vto = if kind == "NMOS" { 2.0 } else { -2.0 };
            let model_resistance = format!(
                "Vg g 0 dc 0\nVd d 0 dc 0\nVs s 0 dc 0\nVb b 0 dc 0\n\
                 M1 d g s b MMOD w={width}\n\
                 .model MMOD {kind}(VTO={vto} KP=20u LAMBDA=0 IS=1e-30 \
                 CGSO=10n RS={resistance})\n"
            );
            let explicit_resistance = format!(
                "Vg g 0 dc 0\nVd d 0 dc 0\nVs s 0 dc 0\nVb b 0 dc 0\n\
                 RSM si s {resistance}\nM1 d g si b MMOD w={width}\n\
                 .model MMOD {kind}(VTO={vto} KP=20u LAMBDA=0 IS=1e-30 CGSO=10n)\n"
            );

            for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                let mut model = Solver::new(
                    parse_netlist(&model_resistance).expect("MOSFET RS overlap model parses"),
                    integration,
                )
                .expect("MOSFET RS overlap solver builds");
                let mut explicit = Solver::new(
                    parse_netlist(&explicit_resistance)
                        .expect("explicit MOSFET source resistor netlist parses"),
                    integration,
                )
                .expect("explicit MOSFET source resistor solver builds");

                let model_internal = model
                    .circuit()
                    .node("M1#source")
                    .expect("model-card RS creates the intrinsic source");
                let explicit_internal = explicit
                    .circuit()
                    .node("si")
                    .expect("explicit resistor creates the intrinsic source");
                let model_gate = model.voltage_source_index("Vg").expect("model gate source");
                let explicit_gate = explicit
                    .voltage_source_index("Vg")
                    .expect("explicit gate source");

                model.set_voltage_source(model_gate, sign * input);
                explicit.set_voltage_source(explicit_gate, sign * input);

                let expected_first = sign * input * ratio / (1.0 + ratio);
                let expected_second = match integration {
                    Integration::BackwardEuler => ratio * expected_first / (1.0 + ratio),
                    Integration::Trapezoidal => {
                        ratio * (3.0 * expected_first - sign * input) / (1.0 + 2.0 * ratio)
                    }
                };

                for (step, expected) in [expected_first, expected_second].into_iter().enumerate() {
                    model.advance(h).expect("model-card RS overlap step solves");
                    explicit
                        .advance(h)
                        .expect("explicit-resistor overlap step solves");

                    let actual = model.node_voltage(model_internal);
                    let reference = explicit.node_voltage(explicit_internal);
                    assert!(
                        (actual - expected).abs() < 1e-8,
                        "{kind} {integration:?} step {step}: CGSO should put the intrinsic source at {expected} V, got {actual} V"
                    );
                    assert!(
                        (actual - reference).abs() < 1e-10,
                        "{kind} {integration:?} step {step}: model-card RS gives {actual} V, explicit source resistance gives {reference} V"
                    );
                    if step == 0 {
                        assert!(
                            sign * actual > 0.1,
                            "{kind} CGSO displacement current must create a measurable RS drop"
                        );
                    }
                }
            }
        }
    }

    /// Gate-bulk overlap charge must conserve terminal current: the charge
    /// taken from the gate is returned through the bulk-side load. Checking
    /// only the bulk-node voltage would miss a one-sided G-B charge stamp.
    #[test]
    fn mosfet_gate_bulk_overlap_conserves_terminal_charge() {
        let (resistance, capacitance, h, input) = (1e3, 1e-12, 1e-9, 0.2);
        let ratio = resistance * capacitance / h;

        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            let vto = if kind == "NMOS" { 2.0 } else { -2.0 };
            let netlist = format!(
                "Vg g 0 dc 0\nVd d 0 dc 0\nVs s 0 dc 0\nRbulk b 0 {resistance}\n\
                 M1 d g s b MMOD w=100u l=100u\n\
                 .model MMOD {kind}(VTO={vto} KP=20u LAMBDA=0 IS=1e-30 CGBO=10n)\n"
            );

            for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                let mut solver = Solver::new(
                    parse_netlist(&netlist).expect("MOSFET CGBO model parses"),
                    integration,
                )
                .expect("MOSFET CGBO solver builds");
                let bulk = solver.circuit().node("b").expect("bulk load node");
                let gate = solver.voltage_source_index("Vg").expect("gate source");

                solver.advance(h).expect("initial operating point settles");
                solver.set_voltage_source(gate, sign * input);
                solver.advance(h).expect("CGBO transient step solves");

                // The first step after the gate edge uses backward Euler for
                // both integration rules. The overlap capacitance is
                // CGBO·L = 10 nF/m · 100 µm = 1 pF.
                // The two intrinsic bulk junctions each contribute their
                // documented GMIN conductance to the grounded drain/source.
                let expected_bulk =
                    sign * input * ratio / (1.0 + ratio + 2.0 * device::GMIN * resistance);
                let actual_bulk = solver.node_voltage(bulk);
                assert!(
                    (actual_bulk - expected_bulk).abs() < 1e-10,
                    "{kind} {integration:?}: CGBO should move the bulk node to {expected_bulk} V, got {actual_bulk} V"
                );

                let gate_current = source_current(&solver, "Vg");
                let bulk_return_current =
                    actual_bulk / resistance + 2.0 * device::GMIN * actual_bulk;
                let expected_charge_current = capacitance * (sign * input - actual_bulk) / h;
                assert!(
                    (gate_current - expected_charge_current).abs() < 1e-10,
                    "{kind} {integration:?}: gate current {gate_current} A should match CGBO charge current {expected_charge_current} A"
                );
                assert!(
                    (gate_current - bulk_return_current).abs() < 1e-10,
                    "{kind} {integration:?}: gate charge current {gate_current} A should return through the bulk load at {bulk_return_current} A"
                );
            }
        }
    }

    /// The two intrinsic body junctions carry nonlinear depletion charge.
    /// Each is driven into the forward-bias continuation so CBD/CBS, PB, MJ
    /// and FC participate in the MNA transient for both device polarities and
    /// companion integration rules.
    #[test]
    fn mosfet_bulk_junction_charge_integrates_cbd_and_cbs() {
        let c0 = 10e-9;
        let pb = 0.75;
        let mj = 0.4;
        let fc = 0.5;
        let resistance = 1e3;
        let h = 100e-6;
        let input = 0.6;
        let charge = |voltage: f64| device::junction_depletion_charge(voltage, c0, pb, mj, fc).0;
        let solve_voltage =
            |step_input: f64, previous_charge: f64, previous_current: f64, rule: Integration| {
                let mut lo = 0.0;
                let mut hi = step_input + 0.2;
                for _ in 0..100 {
                    let mid = 0.5 * (lo + hi);
                    let cap_current = match rule {
                        Integration::BackwardEuler => (charge(mid) - previous_charge) / h,
                        Integration::Trapezoidal => {
                            2.0 * (charge(mid) - previous_charge) / h - previous_current
                        }
                    };
                    if (step_input - mid) / resistance > cap_current {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                0.5 * (lo + hi)
            };

        for (parameter, drain, source) in [("CBD", "x", "0"), ("CBS", "0", "x")] {
            for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
                let (cbd, cbs) = if parameter == "CBD" {
                    (c0, 0.0)
                } else {
                    (0.0, c0)
                };
                let vto = if kind == "NMOS" { 1.0 } else { -1.0 };
                let netlist = format!(
                    "V1 in 0 dc {}\nR1 in x {resistance}\nM1 {drain} 0 {source} 0 MMOD\n\
                     .model MMOD {kind}(VTO={vto} KP=20u IS=1e-30 CBD={cbd} CBS={cbs} \
                     PB={pb} MJ={mj} FC={fc})\n",
                    -sign * input
                );
                let circuit = parse_netlist(&netlist).expect("MOSFET bulk capacitance parses");
                let model = circuit.mosfets[0].model;
                assert_eq!(model.cbd, cbd);
                assert_eq!(model.cbs, cbs);
                assert_eq!(model.pb, pb);
                assert_eq!(model.mj, mj);
                assert_eq!(model.fc, fc);

                for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                    let mut solver = Solver::new(circuit.clone(), integration)
                        .expect("MOSFET bulk capacitance solver builds");
                    let x = solver.circuit().node("x").expect("junction node");
                    let source_index = solver
                        .voltage_source_index("V1")
                        .expect("driving source exists");
                    let initial = -sign * solver.node_voltage(x);
                    assert!((initial - input).abs() < 1e-8);
                    solver
                        .advance(h)
                        .expect("initial bulk charge carries into transient");
                    let held = -sign * solver.node_voltage(x);
                    assert!(
                        (held - initial).abs() < 1e-8,
                        "an operating-point junction charge must not create a startup impulse"
                    );

                    let next_input = input + 0.3;
                    solver.set_voltage_source(source_index, -sign * next_input);

                    let mut previous_charge = charge(initial);
                    let mut previous_current = 0.0;
                    for step in 0..2 {
                        let rule = if step == 0 {
                            Integration::BackwardEuler
                        } else {
                            integration
                        };
                        let expected =
                            solve_voltage(next_input, previous_charge, previous_current, rule);
                        if step == 0 {
                            assert!(
                                expected > fc * pb,
                                "first step must exercise the FC continuation"
                            );
                        }
                        solver.advance(h).expect("MOSFET bulk charge step solves");
                        let actual = -sign * solver.node_voltage(x);
                        assert!(
                            (actual - expected).abs() < 1e-8,
                            "{kind} {parameter} {integration:?} step {step}: expected {expected} V, got {actual} V"
                        );

                        let next_charge = charge(expected);
                        previous_current = match rule {
                            Integration::BackwardEuler => (next_charge - previous_charge) / h,
                            Integration::Trapezoidal => {
                                2.0 * (next_charge - previous_charge) / h - previous_current
                            }
                        };
                        previous_charge = next_charge;
                    }
                }
            }
        }
    }

    /// MOS level 1 can derive bottom depletion capacitance from junction area
    /// (`CJ·AD/AS`) and add sidewall charge from perimeter (`CJSW·PD/PS`). The
    /// two grading coefficients are independent and both junctions are
    /// exercised through the transient MNA companion for NMOS/PMOS and both
    /// integration rules.
    #[test]
    fn mosfet_bulk_junction_geometry_integrates_area_and_sidewall_charge() {
        let cj = 2e-3;
        let cjsw = 5e-7;
        let pb = 0.75;
        let mj = 0.4;
        let mjsw = 0.2;
        let fc = 0.5;
        let resistance = 1e3;
        let h = 100e-6;
        let input = 0.6;
        let charge = |voltage: f64, area: f64, perimeter: f64| {
            let bottom = device::junction_depletion_charge(voltage, cj * area, pb, mj, fc).0;
            let sidewall =
                device::junction_depletion_charge(voltage, cjsw * perimeter, pb, mjsw, fc).0;
            bottom + sidewall
        };
        let solve_voltage = |step_input: f64,
                             previous_charge: f64,
                             previous_current: f64,
                             area: f64,
                             perimeter: f64,
                             rule: Integration| {
            let mut lo = 0.0;
            let mut hi = step_input + 0.2;
            for _ in 0..100 {
                let mid = 0.5 * (lo + hi);
                let cap_current = match rule {
                    Integration::BackwardEuler => {
                        (charge(mid, area, perimeter) - previous_charge) / h
                    }
                    Integration::Trapezoidal => {
                        2.0 * (charge(mid, area, perimeter) - previous_charge) / h
                            - previous_current
                    }
                };
                if (step_input - mid) / resistance > cap_current {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            0.5 * (lo + hi)
        };

        for (terminal, drain, source, area, perimeter) in [
            ("drain", "x", "0", 2e-6, 4e-3),
            ("source", "0", "x", 3e-6, 5e-3),
        ] {
            for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
                let vto = if kind == "NMOS" { 1.0 } else { -1.0 };
                let netlist = format!(
                    "V1 in 0 dc {}\nR1 in x {resistance}\n\
                     M1 {drain} 0 {source} 0 MMOD AD=2e-6 AS=3e-6 PD=4e-3 PS=5e-3\n\
                     .model MMOD {kind}(VTO={vto} KP=20u IS=1e-30 CJ={cj} CJSW={cjsw} \
                     PB={pb} MJ={mj} MJSW={mjsw} FC={fc})\n",
                    -sign * input
                );
                let circuit = parse_netlist(&netlist).expect("MOSFET geometry capacitance parses");
                for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                    let mut solver = Solver::new(circuit.clone(), integration)
                        .expect("MOSFET geometry capacitance solver builds");
                    let x = solver.circuit().node("x").expect("junction node");
                    let source_index = solver
                        .voltage_source_index("V1")
                        .expect("driving source exists");
                    let initial = -sign * solver.node_voltage(x);
                    assert!((initial - input).abs() < 1e-8);
                    solver
                        .advance(h)
                        .expect("initial geometry charge carries into transient");
                    let held = -sign * solver.node_voltage(x);
                    assert!(
                        (held - initial).abs() < 1e-8,
                        "operating-point geometry charge must not create a startup impulse"
                    );

                    let next_input = input + 0.3;
                    solver.set_voltage_source(source_index, -sign * next_input);
                    let mut previous_charge = charge(initial, area, perimeter);
                    let mut previous_current = 0.0;
                    for step in 0..2 {
                        let rule = if step == 0 {
                            Integration::BackwardEuler
                        } else {
                            integration
                        };
                        let expected = solve_voltage(
                            next_input,
                            previous_charge,
                            previous_current,
                            area,
                            perimeter,
                            rule,
                        );
                        if step == 0 {
                            assert!(
                                expected > fc * pb,
                                "{terminal} first step must exercise FC continuation"
                            );
                        }
                        solver
                            .advance(h)
                            .expect("MOSFET geometry charge step solves");
                        let actual = -sign * solver.node_voltage(x);
                        assert!(
                            (actual - expected).abs() < 1e-8,
                            "{kind} {terminal} {integration:?} step {step}: \
                             expected {expected} V, got {actual} V"
                        );
                        let next_charge = charge(expected, area, perimeter);
                        previous_current = match rule {
                            Integration::BackwardEuler => (next_charge - previous_charge) / h,
                            Integration::Trapezoidal => {
                                2.0 * (next_charge - previous_charge) / h - previous_current
                            }
                        };
                        previous_charge = next_charge;
                    }
                }
            }
        }
    }

    /// Meyer capacitance halves cover cutoff, depletion, linear and saturated
    /// channel bias. These are the endpoint values averaged by the transient
    /// charge companion, not fixed overlap capacitances.
    #[test]
    fn mosfet_tox_uses_level1_meyer_capacitance_regions() {
        let cox = 12e-12;
        let phi = 0.6;
        let cases = [
            ("cutoff", 0.0, 0.0, [0.0, 0.0, 6e-12]),
            ("depletion", 0.8, 0.0, [1e-12, 1e-12, 2e-12]),
            ("linear", 1.5, 0.2, [3.4375e-12, 2.4375e-12, 0.0]),
            ("saturation", 1.5, 0.6, [4e-12, 0.0, 0.0]),
        ];
        for (region, vgs, vds, expected) in cases {
            let actual = device::mos_meyer_capacitance_halves(vgs, vds, 0.0, 1.0, 0.0, phi, cox);
            for junction in 0..3 {
                assert!(
                    (actual[junction] - expected[junction]).abs() < 1e-26,
                    "{region} Meyer capacitance {junction}: expected {}, got {}",
                    expected[junction],
                    actual[junction]
                );
            }
        }
        assert_eq!(
            device::mos_meyer_capacitance_halves(1.5, 0.6, 0.0, 1.0, 0.0, phi, 0.0),
            [0.0; 3],
            "without TOX, intrinsic channel capacitance is disabled"
        );
        for tox in ["0", "-1n"] {
            let invalid = format!("M1 d g s b MMOD\n.model MMOD NMOS(TOX={tox})\n");
            let error = parse_netlist(&invalid)
                .expect_err("non-positive MOS oxide thickness must be rejected")
                .to_string();
            assert!(error.contains("TOX"), "unexpected TOX error: {error}");
        }
    }

    #[test]
    fn mosfet_ward_dutton_channel_charge_conserves_and_has_analytic_partials() {
        let cases = [
            ("zero-drain-bias", 1.6, 0.0, -0.1),
            ("linear", 1.6, 0.2, -0.1),
            ("saturated", 1.8, 1.0, -0.1),
            ("reverse", 1.6, -0.2, -0.1),
        ];
        for (kind, polarity) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            let vto = polarity;
            let circuit = parse_netlist(&format!(
                "M1 d g s b MOD w=2u l=1u\n.model MOD {kind}(VTO={vto} TOX=10n GAMMA=0.35 PHI=0.7 CHARGEMOD=WARD_DUTTON)\n"
            ))
            .expect("Ward-Dutton model parses");
            let mosfet = &circuit.mosfets[0];
            for (region, vgs, vds, vbs) in cases {
                let evaluate = |vgs, vds, vbs| {
                    device::mos_ward_dutton_channel_charge(mosfet, vgs, vds, vbs - vds, vbs)
                };
                let charge = evaluate(vgs, vds, vbs);
                assert!(
                    charge.q.iter().sum::<f64>().abs() < 1e-25,
                    "{kind} {region}: terminal charges do not sum to zero: {:?}",
                    charge.q
                );
                let reversed = vds < 0.0;
                let (vgs_eff, vds_eff, vbs_eff) = if reversed {
                    (vgs - vds, -vds, vbs - vds)
                } else {
                    (vgs, vds, vbs)
                };
                let sqrt_phi = 0.7_f64.sqrt();
                let body_root = if vbs_eff <= 0.0 {
                    (0.7 - vbs_eff).sqrt()
                } else {
                    (sqrt_phi - vbs_eff / (2.0 * sqrt_phi)).max(0.0)
                };
                let overdrive = vgs_eff - (1.0 + 0.35 * (body_root - sqrt_phi));
                let channel_drop = vds_eff.clamp(0.0, overdrive);
                let denominator = overdrive * channel_drop - 0.5 * channel_drop.powi(2);
                let cox = 3.453_133e-11 * mosfet.w * mosfet.l / 10e-9;
                let intervals = 40_000;
                let mut total_integral = 0.0;
                let mut drain_integral = 0.0;
                for point in 0..=intervals {
                    let x = point as f64 / intervals as f64;
                    let channel_charge_density = (overdrive.powi(2) - 2.0 * x * denominator).sqrt();
                    let weight = if point == 0 || point == intervals {
                        1.0
                    } else if point % 2 == 0 {
                        2.0
                    } else {
                        4.0
                    };
                    total_integral += weight * channel_charge_density;
                    drain_integral += weight * x * channel_charge_density;
                }
                let scale = 1.0 / (3.0 * intervals as f64);
                let expected_total = cox * total_integral * scale;
                let expected_drain = cox * drain_integral * scale;
                let physical_drain_charge = if reversed { charge.q[2] } else { charge.q[1] };
                assert!(
                    (charge.q[0] - polarity * expected_total).abs()
                        < expected_total.abs() * 1e-6,
                    "{kind} {region}: integrated gate charge differs from the charge-sheet quadrature"
                );
                assert!(
                    (physical_drain_charge + polarity * expected_drain).abs()
                        < expected_drain.abs() * 1e-6,
                    "{kind} {region}: drain partition differs from the Ward-Dutton quadrature"
                );
                for control in 0..3 {
                    let jacobian_sum = charge.dq_dv.iter().map(|row| row[control]).sum::<f64>();
                    assert!(
                        jacobian_sum.abs() < 1e-25,
                        "{kind} {region}: Jacobian column {control} does not conserve charge: {jacobian_sum}"
                    );
                }

                let delta = 1e-5;
                for terminal in 0..4 {
                    for control in 0..3 {
                        let mut plus = [vgs, vds, vbs];
                        let mut minus = [vgs, vds, vbs];
                        plus[control] += delta;
                        minus[control] -= delta;
                        let plus = evaluate(plus[0], plus[1], plus[2]).q[terminal];
                        let minus = evaluate(minus[0], minus[1], minus[2]).q[terminal];
                        let finite_difference = if vds == 0.0 && control == 1 {
                            (plus - charge.q[terminal]) / delta * polarity
                        } else {
                            (plus - minus) / (2.0 * delta) * polarity
                        };
                        let analytic = charge.dq_dv[terminal][control];
                        assert!(
                            (analytic - finite_difference).abs()
                                < finite_difference.abs() * 3e-5 + 1e-24,
                            "{kind} {region}, terminal {terminal}, control {control}: analytic {analytic}, finite difference {finite_difference}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn mosfet_ward_dutton_charge_drives_a_loaded_terminal_for_both_polarities_and_directions() {
        let width = 2e-6;
        let length = 1e-6;
        let tox = 10e-9;
        let resistance = 1e3;
        let h = 10e-9;
        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            for reverse in [false, true] {
                for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                    let (drain, source, sensed_terminal) = if reverse {
                        ("0", "x", 2)
                    } else {
                        ("x", "0", 1)
                    };
                    let netlist = format!(
                        "Vg g 0 dc {}\nRload x 0 {resistance}\nM1 {drain} g {source} 0 MOD w={width} l={length}\n.model MOD {kind}(VTO={} KP=1e-30 IS=1e-30 TOX={tox} CHARGEMOD=WARD_DUTTON)\n",
                        sign * 1.5,
                        sign
                    );
                    let circuit = parse_netlist(&netlist).expect("MOS channel charge parses");
                    let mosfet = circuit.mosfets[0].clone();
                    let mut solver = Solver::new(circuit, integration)
                        .expect("MOS channel charge solver builds");
                    let x = solver.circuit().node("x").expect("loaded terminal node");
                    let gate = solver
                        .voltage_source_index("Vg")
                        .expect("gate drive exists");
                    solver
                        .advance(h)
                        .expect("initially biased channel step solves");
                    let initial_x = solver.node_voltage(x);
                    assert!(initial_x.abs() < 1e-10);

                    let old_gate = sign * 1.5;
                    let new_gate = sign * 1.6;
                    let terminal_charge = |node_voltage: f64, gate_voltage: f64| {
                        let (vgs, vds, vbd, vbs) = if reverse {
                            (
                                sign * (gate_voltage - node_voltage),
                                sign * -node_voltage,
                                0.0,
                                sign * -node_voltage,
                            )
                        } else {
                            (
                                sign * gate_voltage,
                                sign * node_voltage,
                                sign * -node_voltage,
                                0.0,
                            )
                        };
                        device::mos_ward_dutton_channel_charge(&mosfet, vgs, vds, vbd, vbs).q
                            [sensed_terminal]
                    };
                    let old_charge = terminal_charge(initial_x, old_gate);
                    let residual = |voltage: f64| {
                        voltage / resistance + (terminal_charge(voltage, new_gate) - old_charge) / h
                    };
                    let mut low = -0.2;
                    let mut high = 0.2;
                    assert!(residual(low) * residual(high) < 0.0);
                    for _ in 0..80 {
                        let middle = 0.5 * (low + high);
                        if residual(middle) < 0.0 {
                            low = middle;
                        } else {
                            high = middle;
                        }
                    }
                    let expected = 0.5 * (low + high);

                    solver.set_voltage_source(gate, new_gate);
                    solver
                        .advance(h)
                        .expect("gate step through Ward-Dutton charge solves");
                    let actual = solver.node_voltage(x);
                    assert!(
                        (actual - expected).abs() < 1e-8,
                        "{kind} reverse={reverse} {integration:?}: expected {expected} V, got {actual} V"
                    );
                }
            }
        }
    }

    /// `TOX` produces an intrinsic gate-bulk capacitance in cutoff. A gate
    /// step charges that branch through the resistor for NMOS and PMOS, and
    /// the held bias exercises the selected trapezoidal history after the
    /// first BE step. The operating-point charge must not create a startup
    /// impulse.
    #[test]
    fn mosfet_tox_integrates_channel_charge_for_both_polarities_and_rules() {
        let width = 2e-6;
        let length = 1e-6;
        let tox = 10e-9;
        let resistance = 1e3;
        let h = 10e-12;
        let gate_step = 0.1;

        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            let vto = sign;
            for (geometry, effective_length) in
                [("", length), (" LD=0.25u", length - 2.0 * 0.25e-6)]
            {
                let cox = 3.453_133e-11 * width * effective_length / tox;
                let ratio = resistance * cox / h;
                let netlist = format!(
                    "Vg g 0 dc {}\nR1 x 0 {resistance}\n\
                     M1 0 g 0 x MMOD w={width} l={length}\n\
                     .model MMOD {kind}(VTO={vto} KP=20u IS=1e-30 PHI=0.6 TOX={tox}{geometry})\n",
                    sign * gate_step
                );
                for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                    let mut solver = Solver::new(
                        parse_netlist(&netlist).expect("MOS TOX model parses"),
                        integration,
                    )
                    .expect("MOS TOX solver builds");
                    let x = solver.circuit().node("x").expect("bulk node");
                    let source = solver.voltage_source_index("Vg").expect("gate source");

                    solver.advance(h).expect("initially biased TOX step solves");
                    assert!(
                        solver.node_voltage(x).abs() < 1e-10,
                        "{kind} {geometry} {integration:?}: operating-point channel charge must not create a startup impulse"
                    );

                    solver.set_voltage_source(source, sign * 2.0 * gate_step);
                    solver.advance(h).expect("gate charge step solves");
                    let first = solver.node_voltage(x);
                    let expected_first = sign * gate_step * ratio / (1.0 + ratio);
                    assert!(
                        (first - expected_first).abs() < 1e-8,
                        "{kind} {geometry} {integration:?} first step: expected {expected_first} V, got {first} V"
                    );

                    solver.advance(h).expect("held gate charge step solves");
                    let actual_second = solver.node_voltage(x);
                    let expected_second = match integration {
                        Integration::BackwardEuler => ratio * expected_first / (1.0 + ratio),
                        Integration::Trapezoidal => {
                            ratio * (3.0 * expected_first - sign * gate_step) / (1.0 + 2.0 * ratio)
                        }
                    };
                    assert!(
                        (actual_second - expected_second).abs() < 1e-8,
                        "{kind} {geometry} {integration:?} second step: expected {expected_second} V, got {actual_second} V"
                    );
                }
            }
        }
    }

    /// With V(d) below V(s), ngspice's Meyer G-S and G-D capacitances swap
    /// their physical terminals. Keep the transistor current negligible and
    /// drive a small gate step so the resistor response identifies which
    /// capacitance the MNA stamp placed on the physical source pin.
    #[test]
    fn mosfet_tox_maps_reversed_channel_capacitance_to_physical_pins() {
        let width = 2e-6;
        let length = 1e-6;
        let tox = 10e-9;
        let cox = 3.453_133e-11 * width * length / tox;
        let resistance = 1e3;
        let h = 10e-12;
        let gate_step = 1e-3;
        let vgs_eff = 1.5;
        let vds_eff = 0.2;
        let vgst = vgs_eff - 1.0;
        let vdsat = vgst;
        let vddif = 2.0 * vdsat - vds_eff;
        let cgd_half = cox * (1.0 - vdsat * vdsat / (vddif * vddif)) / 3.0;
        let capacitance = 2.0 * cgd_half;
        let ratio = resistance * capacitance / h;
        let netlist = format!(
            "Vg g 0 dc {vgs_eff}\nVbias bias 0 dc 0.2\n\
             R1 x bias {resistance}\nM1 0 g x 0 MMOD w={width} l={length}\n\
             .model MMOD NMOS(VTO=1 KP=1e-30 IS=1e-30 TOX={tox})\n"
        );
        let mut solver = Solver::new(
            parse_netlist(&netlist).expect("reversed MOS TOX model parses"),
            Integration::BackwardEuler,
        )
        .expect("reversed MOS TOX solver builds");
        let x = solver.circuit().node("x").expect("physical source node");
        let gate = solver.voltage_source_index("Vg").expect("gate source");
        solver
            .advance(h)
            .expect("reverse-biased channel step solves");
        let initial = solver.node_voltage(x);
        assert!((initial - 0.2).abs() < 1e-8);

        solver.set_voltage_source(gate, vgs_eff + gate_step);
        solver
            .advance(h)
            .expect("reversed channel gate step solves");
        let actual = solver.node_voltage(x) - initial;
        let expected = gate_step * ratio / (1.0 + ratio);
        assert!(
            (actual - expected).abs() < 1e-7,
            "reversed channel should map effective Cgd to physical G-S: expected {expected} V, got {actual} V"
        );
    }

    // -----------------------------------------------------------------------
    // JFET
    // -----------------------------------------------------------------------

    /// The three level-1 channel regions are checked against the model's
    /// closed form, with both channel polarities and reverse gate-junction
    /// leakage included in the measured drain supply current.
    #[test]
    fn jfet_operating_point_matches_level1_channel_equation_by_hand() {
        let beta = 2e-4;
        let lambda = 0.02;
        let is = 1e-14;
        let vto = -2.0;

        for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
            let kind = if polarity == Polarity::N {
                "NJF"
            } else {
                "PJF"
            };
            for (vgs, vds) in [(-2.5, 5.0), (0.0, 0.4), (0.0, 5.0)] {
                let netlist = format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nVs s 0 dc 0\n\
                     J1 d g s JMOD\n.model JMOD {kind}(VTO={} BETA={beta} \
                     LAMBDA={lambda} IS={is})\n",
                    sign * vgs,
                    sign * vds,
                    sign * vto
                );
                let solver = build(&netlist);
                let vgst = vgs - vto;
                let channel = if vgst <= 0.0 {
                    0.0
                } else if vds < vgst {
                    beta * (1.0 + lambda * vds) * vds * (2.0 * vgst - vds)
                } else {
                    beta * (1.0 + lambda * vds) * vgst * vgst
                };
                let gate_drain_voltage = vgs - vds;
                let expected =
                    channel + device::GMIN * vds - device::diode_op(gate_drain_voltage, is, 1.0).id;
                let measured = sign * source_current(&solver, "Vd");
                let label = format!("{kind} Vgs={vgs} Vds={vds}");
                assert!(
                    relative(measured, expected) < 2e-5,
                    "{label}: expected {expected} A including gate leakage, got {measured} A"
                );
            }
        }
    }

    /// SPICE level-1 JFET temperature coefficients adjust the channel law at
    /// the instance's external `TEMP`. Verify both coefficient alternatives,
    /// their precedence, both channel polarities, and the resulting MNA
    /// currents against an independently calculated model card.
    #[test]
    fn jfet_temperature_coefficients_match_level1_laws_in_solved_circuits() {
        let (tnom, temperature, delta) = (40.0, 115.0, 75.0);
        let (beta, lambda, is, vds) = (2e-4, 0.02, 1e-30, 1.0);
        let cases = [
            // VTOTC and BETATCE take precedence over the nonzero TCV and BEX.
            ("NJF", 1.0, -2.0, Some(0.001), 0.02, Some(0.4), 2.5),
            // With alternatives absent, TCV and the absolute-temperature BEX
            // power law set VTO and BETA for a P-channel device.
            ("PJF", -1.0, 2.0, None, -0.001, None, 1.2),
        ];

        for (kind, sign, vto, vtotc, tcv, betatce, bex) in cases {
            let vtotc_parameter = vtotc.map_or_else(String::new, |value| format!(" VTOTC={value}"));
            let betatce_parameter =
                betatce.map_or_else(String::new, |value| format!(" BETATCE={value}"));
            let delta_kelvin: f64 = (temperature + 273.15) / (tnom + 273.15);
            let expected_vto = vtotc.map_or(vto - tcv * delta, |tc| vto + tc * delta);
            let expected_beta = betatce.map_or_else(
                || beta * delta_kelvin.powf(bex),
                |tce| beta * 1.01_f64.powf(tce * delta),
            );
            let model_parameters = format!(
                "TNOM={tnom} VTO={vto} BETA={beta} LAMBDA={lambda} IS={is} \
                 TCV={tcv} BEX={bex}{vtotc_parameter}{betatce_parameter}"
            );
            let netlist = |model: String, instance_temp: Option<f64>| {
                let temp = instance_temp.map_or_else(String::new, |value| format!(" TEMP={value}"));
                format!(
                    "Vg g 0 dc 0\nVd d 0 dc {}\nVs s 0 dc 0\n\
                     J1 d g s JMOD{temp}\n.model JMOD {kind}({model})\n",
                    sign * vds
                )
            };

            let operating = build(&netlist(model_parameters.clone(), Some(temperature)));
            let explicit_model = format!(
                "TNOM={tnom} VTO={expected_vto} BETA={expected_beta} \
                 LAMBDA={lambda} IS={is}"
            );
            let explicit = build(&netlist(explicit_model, None));
            let fixed = build(&netlist(model_parameters, None));
            let adjusted = operating.circuit().jfets[0].operating_model;
            assert_eq!(
                operating.circuit().jfets[0].temperature_celsius,
                Some(temperature)
            );
            assert!(
                (adjusted.vto - expected_vto).abs() < 1e-14,
                "{kind}: expected VTO(T)={expected_vto}, got {}",
                adjusted.vto
            );
            assert!(
                relative(adjusted.beta, expected_beta) < 1e-14,
                "{kind}: expected BETA(T)={expected_beta}, got {}",
                adjusted.beta
            );

            let channel_current = |effective_vto: f64, effective_beta: f64| {
                let vgst = 0.0 - sign * effective_vto;
                let channel = effective_beta * (1.0 + lambda * vds) * vds * (2.0 * vgst - vds);
                channel + device::GMIN * vds - device::diode_op(-vds, is, 1.0).id
            };
            let expected_current = channel_current(expected_vto, expected_beta);
            let actual_current = sign * source_current(&operating, "Vd");
            let explicit_current = sign * source_current(&explicit, "Vd");
            let fixed_current = sign * source_current(&fixed, "Vd");
            assert!(
                relative(actual_current, expected_current) < 2e-5,
                "{kind}: level-1 temperature law predicts {expected_current} A, got {actual_current} A"
            );
            assert!(
                relative(actual_current, explicit_current) < 2e-8,
                "{kind}: TEMP model gives {actual_current} A, equivalent adjusted model gives {explicit_current} A"
            );
            assert!(
                (actual_current - fixed_current).abs() > 1e-5,
                "{kind}: explicit TEMP must change the solved channel current"
            );
        }
    }

    /// The Parker–Skellern B fit changes both JFET channel regions. Its
    /// closed-form current and analytic partials are checked through the
    /// nonlinear MNA solve for both channel polarities.
    #[test]
    fn jfet_parker_skellern_b_matches_both_regions_and_polarities() {
        let beta = 2e-4;
        let lambda = 0.02;
        let is = 1e-30;
        let b = 1.4;
        let pb = 0.75;
        let vto = -2.0;
        let bfac = (1.0 - b) / (pb - vto);
        let cases = [(0.0, 0.4), (0.0, 5.0)];

        let channel_current = |vgs: f64, vds: f64| {
            let vgst = vgs - vto;
            let beta_p = beta * (1.0 + lambda * vds);
            if vgst <= 0.0 {
                0.0
            } else if vgst >= vds {
                let apart = 2.0 * b + 3.0 * bfac * (vgst - vds);
                beta_p * vds * (vds * (bfac * vds - b) + vgst * apart)
            } else {
                beta_p * vgst * vgst * (b + vgst * bfac)
            }
        };

        for (vgs, vds) in cases {
            let op = device::jfet_op_with_b(vgs, vds, vto, beta, lambda, b, pb);
            let dv = 1e-6;
            let gm = (channel_current(vgs + dv, vds) - channel_current(vgs - dv, vds)) / (2.0 * dv);
            let gds =
                (channel_current(vgs, vds + dv) - channel_current(vgs, vds - dv)) / (2.0 * dv);
            assert!(relative(op.gm, gm) < 1e-8, "Vgs={vgs} Vds={vds}: gm");
            assert!(relative(op.gds, gds) < 1e-8, "Vgs={vgs} Vds={vds}: gds");

            for (kind, sign) in [("NJF", 1.0), ("PJF", -1.0)] {
                let netlist = format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nVs s 0 dc 0\nJ1 d g s JMOD\n.model JMOD {kind}(VTO={} BETA={beta} LAMBDA={lambda} IS={is} B={b} PB={pb})\n",
                    sign * vgs,
                    sign * vds,
                    sign * vto
                );
                let measured = sign * source_current(&build(&netlist), "Vd");
                let gate_drain_voltage = vgs - vds;
                let expected = channel_current(vgs, vds) + device::GMIN * vds
                    - device::diode_op(gate_drain_voltage, is, 1.0).id;
                assert!(
                    relative(measured, expected) < 2e-5,
                    "{kind} Vgs={vgs} Vds={vds}: expected {expected} A, got {measured} A"
                );
            }
        }
    }

    /// The JFET channel is symmetric, and each gate junction conducts when
    /// forward biased. Both behaviors are exercised through the MNA solver.
    #[test]
    fn jfet_reverse_channel_and_forward_gate_junction_are_solved() {
        let deck = |kind: &str, sign: f64, drain: f64, source: f64| {
            format!(
                "Vg g 0 dc 0\nVd d 0 dc {}\nVs s 0 dc {}\n\
                 J1 d g s JMOD\n.model JMOD {kind}(VTO={} BETA=200u IS=1e-12)\n",
                sign * drain,
                sign * source,
                -2.0 * sign
            )
        };
        for (kind, sign) in [("NJF", 1.0), ("PJF", -1.0)] {
            let forward = sign * source_current(&build(&deck(kind, sign, 0.4, 0.0)), "Vd");
            let reverse = sign * source_current(&build(&deck(kind, sign, 0.0, 0.4)), "Vs");
            assert!(
                forward > 1e-6,
                "{kind} forward channel current should be on, got {forward} A"
            );
            assert!(
                relative(reverse, forward) < 1e-6,
                "{kind} reverse channel should mirror: {forward} A vs {reverse} A"
            );
        }

        let is = 1e-12;
        let expected = 2.0 * device::diode_op(0.4, is, 1.0).id;
        for (kind, sign) in [("NJF", 1.0), ("PJF", -1.0)] {
            let netlist = format!(
                "Vg g 0 dc {}\nVd d 0 dc 0\nVs s 0 dc 0\n\
                 J1 d g s JMOD\n.model JMOD {kind}(IS={is})\n",
                sign * 0.4
            );
            let measured = sign * source_current(&build(&netlist), "Vg");
            assert!(
                relative(measured, expected) < 1e-5,
                "{kind} gate current should be the two forward junctions: expected \
                 {expected} A, got {measured} A"
            );
        }
    }

    /// MOSFET `RD` and `RS` sit outside the intrinsic channel terminals. The
    /// terminal voltage drops are checked against the solved channel current,
    /// for both MOS polarities, exercising the model parameters in MNA.
    #[test]
    fn mosfet_terminal_resistances_drop_voltage_at_the_intrinsic_nodes() {
        let resistance = 100.0;
        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            for (parameter, terminal) in [("RD", "drain"), ("RS", "source")] {
                let netlist = format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nVs s 0 dc 0\n\
                     M1 d g s 0 MMOD\n.model MMOD {kind}(VTO={} KP=20u LAMBDA=0 \
                     IS=1e-30 W=100u L=10u {parameter}={resistance})\n",
                    sign * 3.0,
                    sign * 0.4,
                    sign
                );
                let solver = build(&netlist);
                let external = solver
                    .circuit()
                    .node(if terminal == "drain" { "d" } else { "s" })
                    .expect("external terminal exists");
                let internal_name = format!("M1#{terminal}");
                let internal = solver
                    .circuit()
                    .node(&internal_name)
                    .expect("nonzero terminal resistance creates an internal node")
                    .expect("internal terminal is not ground");
                let current = sign * source_current(&solver, "Vd");
                assert!(current > 1e-6, "{kind} {parameter} current is {current} A");

                let (external_voltage, internal_voltage) = (
                    solver.node_voltage(external),
                    solver.node_voltage(Some(internal)),
                );
                let actual_drop = if parameter == "RD" {
                    sign * (external_voltage - internal_voltage)
                } else {
                    sign * (internal_voltage - external_voltage)
                };
                assert!(
                    relative(actual_drop, current * resistance) < 1e-6,
                    "{kind} {parameter} should drop {} V, got {actual_drop} V",
                    current * resistance
                );
            }
        }
    }

    /// During a gate step, intrinsic channel charge draws displacement current
    /// through the MOSFET's external drain/source terminals. The model-card
    /// `RD`/`RS` implementation must behave like the same resistors placed
    /// explicitly outside a resistance-free device, including both transient
    /// companion rules and both channel polarities.
    #[test]
    fn mosfet_channel_charge_matches_explicit_terminal_resistors_during_transients() {
        let (rd, rs) = (800.0, 500.0);
        let width = 10e-6;
        let length = 1e-6;
        let h = 10e-12;

        for (kind, sign, vd, vs, vb) in [
            ("NMOS", 1.0, 2.5, 0.0, 0.0),
            ("PMOS", -1.0, 0.0, -2.5, -2.5),
        ] {
            let model_resistances = format!(
                "Vg g 0 dc {}\nVd d 0 dc {vd}\nVs s 0 dc {vs}\nVb b 0 dc {vb}\n\
                 M1 d g s b MMOD w={width} l={length}\n\
                 .model MMOD {kind}(VTO={} KP=20u LAMBDA=0.02 IS=1e-30 \
                 TOX=10n CHARGEMOD=WARD_DUTTON RD={rd} RS={rs})\n",
                sign * 1.2,
                sign
            );
            let explicit_resistances = format!(
                "Vg g 0 dc {}\nVd d 0 dc {vd}\nVs s 0 dc {vs}\nVb b 0 dc {vb}\n\
                 RDM d di {rd}\nRSM si s {rs}\n\
                 M1 di g si b MMOD w={width} l={length}\n\
                 .model MMOD {kind}(VTO={} KP=20u LAMBDA=0.02 IS=1e-30 \
                 TOX=10n CHARGEMOD=WARD_DUTTON)\n",
                sign * 1.2,
                sign
            );
            let no_channel_charge = model_resistances.replace("TOX=10n CHARGEMOD=WARD_DUTTON ", "");

            for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                let mut model = Solver::new(
                    parse_netlist(&model_resistances).expect("MOSFET terminal model parses"),
                    integration,
                )
                .expect("MOSFET terminal solver builds");
                let mut explicit = Solver::new(
                    parse_netlist(&explicit_resistances).expect("explicit resistor netlist parses"),
                    integration,
                )
                .expect("explicit resistor solver builds");
                let mut uncharged = Solver::new(
                    parse_netlist(&no_channel_charge).expect("uncharged MOSFET model parses"),
                    integration,
                )
                .expect("uncharged MOSFET solver builds");

                let model_nodes = ["d", "s", "M1#drain", "M1#source"]
                    .map(|name| model.circuit().node(name).expect("model node"));
                let explicit_nodes = ["d", "s", "di", "si"]
                    .map(|name| explicit.circuit().node(name).expect("explicit node"));
                let model_gate = model.voltage_source_index("Vg").expect("model gate source");
                let explicit_gate = explicit
                    .voltage_source_index("Vg")
                    .expect("explicit gate source");

                model.advance(h).expect("model operating point settles");
                explicit
                    .advance(h)
                    .expect("explicit operating point settles");
                uncharged
                    .advance(h)
                    .expect("uncharged operating point settles");
                for (model_node, explicit_node) in model_nodes.into_iter().zip(explicit_nodes) {
                    assert!(
                        (model.node_voltage(model_node) - explicit.node_voltage(explicit_node))
                            .abs()
                            < 1e-9,
                        "{kind} {integration:?}: initial terminal voltages differ"
                    );
                }

                model.set_voltage_source(model_gate, sign * 3.0);
                explicit.set_voltage_source(explicit_gate, sign * 3.0);
                let uncharged_gate = uncharged
                    .voltage_source_index("Vg")
                    .expect("uncharged gate source");
                uncharged.set_voltage_source(uncharged_gate, sign * 3.0);
                for step in 0..3 {
                    model.advance(h).expect("model channel-charge step solves");
                    explicit
                        .advance(h)
                        .expect("explicit-resistor channel-charge step solves");
                    uncharged.advance(h).expect("uncharged MOSFET step solves");

                    let mut charge_effect = 0.0_f64;
                    for (label, model_node, explicit_node) in [
                        ("external drain", model_nodes[0], explicit_nodes[0]),
                        ("external source", model_nodes[1], explicit_nodes[1]),
                        ("intrinsic drain", model_nodes[2], explicit_nodes[2]),
                        ("intrinsic source", model_nodes[3], explicit_nodes[3]),
                    ] {
                        let actual = model.node_voltage(model_node);
                        let expected = explicit.node_voltage(explicit_node);
                        charge_effect =
                            charge_effect.max((actual - uncharged.node_voltage(model_node)).abs());
                        assert!(
                            (actual - expected).abs() < 1e-8,
                            "{kind} {integration:?} step {step} {label}: model-card resistance gives {actual} V, explicit series resistor gives {expected} V"
                        );
                    }
                    if step == 0 {
                        assert!(
                            charge_effect > 1e-7,
                            "{kind} {integration:?}: the channel-charge step must measurably affect the terminal voltages, got a largest charged/uncharged difference of {charge_effect} V"
                        );
                    }
                }
            }
        }
    }

    /// Channel, overlap and bulk-junction charge must all land on the
    /// intrinsic MOSFET terminals before RD/RS carry their displacement
    /// currents. Compare one combined transient against the same model with
    /// explicit resistors, for both polarities and integration rules; a
    /// charge stamp attached to an external terminal or a replaced charge
    /// history will make the solved trajectories diverge.
    #[test]
    fn mosfet_combined_charge_physics_flows_through_terminal_resistances() {
        let (rd, rs) = (800.0, 500.0);
        let (width, length, h) = (100e-6, 10e-6, 10e-12);
        let model_params = "TOX=10n CHARGEMOD=WARD_DUTTON CGSO=10n CGDO=10n \
            CGBO=10n CBD=100p CBS=100p PB=1 MJ=0";

        for (kind, sign, vd, vs, vb) in [
            ("NMOS", 1.0, 2.5, 0.0, 0.0),
            ("PMOS", -1.0, 0.0, -2.5, -2.5),
        ] {
            let vto = sign;
            let model_resistances = format!(
                "Vg g 0 dc {}\nVd d 0 dc {vd}\nVs s 0 dc {vs}\nVb b 0 dc {vb}\n\
                 M1 d g s b MMOD w={width} l={length}\n\
                 .model MMOD {kind}(VTO={vto} KP=20u LAMBDA=0.02 IS=1e-30 \
                 {model_params} RD={rd} RS={rs})\n",
                sign * 1.2,
            );
            let explicit_resistances = format!(
                "Vg g 0 dc {}\nVd d 0 dc {vd}\nVs s 0 dc {vs}\nVb b 0 dc {vb}\n\
                 RDM d di {rd}\nRSM si s {rs}\n\
                 M1 di g si b MMOD w={width} l={length}\n\
                 .model MMOD {kind}(VTO={vto} KP=20u LAMBDA=0.02 IS=1e-30 \
                 {model_params})\n",
                sign * 1.2,
            );
            let uncharged = model_resistances.replace(model_params, "");

            for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                let mut model = Solver::new(
                    parse_netlist(&model_resistances).expect("combined MOSFET charge model parses"),
                    integration,
                )
                .expect("combined MOSFET charge solver builds");
                let mut explicit = Solver::new(
                    parse_netlist(&explicit_resistances)
                        .expect("explicit-resistor MOSFET model parses"),
                    integration,
                )
                .expect("explicit-resistor MOSFET solver builds");
                let mut without_charge = Solver::new(
                    parse_netlist(&uncharged).expect("uncharged MOSFET model parses"),
                    integration,
                )
                .expect("uncharged MOSFET solver builds");

                let model_nodes = ["d", "s", "b", "M1#drain", "M1#source"]
                    .map(|name| model.circuit().node(name).expect("model terminal node"));
                let explicit_nodes = ["d", "s", "b", "di", "si"].map(|name| {
                    explicit
                        .circuit()
                        .node(name)
                        .expect("explicit terminal node")
                });
                let uncharged_nodes = ["d", "s", "b", "M1#drain", "M1#source"]
                    .map(|name| without_charge.circuit().node(name).expect("uncharged node"));
                let model_gate = model.voltage_source_index("Vg").expect("model gate source");
                let explicit_gate = explicit
                    .voltage_source_index("Vg")
                    .expect("explicit gate source");
                let uncharged_gate = without_charge
                    .voltage_source_index("Vg")
                    .expect("uncharged gate source");

                model.advance(h).expect("model operating point settles");
                explicit
                    .advance(h)
                    .expect("explicit operating point settles");
                without_charge
                    .advance(h)
                    .expect("uncharged operating point settles");
                for (actual, expected) in model_nodes.into_iter().zip(explicit_nodes) {
                    assert!(
                        (model.node_voltage(actual) - explicit.node_voltage(expected)).abs() < 1e-9,
                        "{kind} {integration:?}: initial terminal voltages differ"
                    );
                }

                model.set_voltage_source(model_gate, sign * 3.0);
                explicit.set_voltage_source(explicit_gate, sign * 3.0);
                without_charge.set_voltage_source(uncharged_gate, sign * 3.0);

                let mut largest_charge_effect = 0.0_f64;
                for step in 0..3 {
                    model.advance(h).expect("combined model charge step solves");
                    explicit
                        .advance(h)
                        .expect("combined explicit-resistor charge step solves");
                    without_charge
                        .advance(h)
                        .expect("uncharged comparison step solves");

                    for (index, (actual_node, expected_node)) in model_nodes
                        .iter()
                        .copied()
                        .zip(explicit_nodes.iter().copied())
                        .enumerate()
                    {
                        let actual = model.node_voltage(actual_node);
                        let expected = explicit.node_voltage(expected_node);
                        largest_charge_effect = largest_charge_effect.max(
                            (actual - without_charge.node_voltage(uncharged_nodes[index])).abs(),
                        );
                        assert!(
                            (actual - expected).abs() < 1e-8,
                            "{kind} {integration:?} step {step} terminal {index}: model-card resistances give {actual} V, explicit resistors give {expected} V"
                        );
                    }
                    for source in ["Vg", "Vd", "Vs", "Vb"] {
                        assert!(
                            (source_current(&model, source) - source_current(&explicit, source)).abs()
                                < 1e-10,
                            "{kind} {integration:?} step {step} {source}: model-card resistances and explicit resistors must carry the same combined charge current"
                        );
                    }
                }
                assert!(
                    largest_charge_effect > 1e-7,
                    "{kind} {integration:?}: combined MOSFET charge must measurably affect the terminal voltages"
                );
            }
        }
    }

    /// Drain bulk-junction depletion charge is attached to the intrinsic
    /// drain. With model-card `RD`, its transient current must cross that
    /// resistor exactly as it does when the resistor is explicit in the
    /// netlist. The constant-capacitance case also has a hand-derived RC
    /// trajectory, so a missing junction charge cannot pass by matching two
    /// otherwise identical circuits.
    #[test]
    fn mosfet_bulk_junction_charge_flows_through_drain_resistance() {
        let (resistance, capacitance, step, h) = (1e3, 100e-12, 1.0, 100e-9);
        let beta = resistance * capacitance / h;

        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            let vto = if kind == "NMOS" { 2.0 } else { -2.0 };
            let model_resistance = format!(
                "Vg g 0 dc 0\nVd d 0 dc 0\nVs s 0 dc 0\nVb b 0 dc 0\n\
                 M1 d g s b MMOD\n\
                 .model MMOD {kind}(VTO={vto} KP=20u LAMBDA=0 IS=1e-30 \
                 CBD={capacitance} PB=1 MJ=0 RD={resistance})\n"
            );
            let explicit_resistance = format!(
                "Vg g 0 dc 0\nVd d 0 dc 0\nVs s 0 dc 0\nVb b 0 dc 0\n\
                 RDM d di {resistance}\nM1 di g s b MMOD\n\
                 .model MMOD {kind}(VTO={vto} KP=20u LAMBDA=0 IS=1e-30 \
                 CBD={capacitance} PB=1 MJ=0)\n"
            );

            for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                let mut model = Solver::new(
                    parse_netlist(&model_resistance).expect("MOSFET bulk model parses"),
                    integration,
                )
                .expect("MOSFET bulk solver builds");
                let mut explicit = Solver::new(
                    parse_netlist(&explicit_resistance).expect("explicit resistor netlist parses"),
                    integration,
                )
                .expect("explicit resistor solver builds");

                let model_external = model.circuit().node("d").expect("external drain");
                let model_intrinsic = model
                    .circuit()
                    .node("M1#drain")
                    .expect("model-card RD creates the intrinsic drain");
                let explicit_external = explicit.circuit().node("d").expect("external drain");
                let explicit_intrinsic = explicit.circuit().node("di").expect("intrinsic drain");
                let model_drive = model.voltage_source_index("Vd").expect("drain source");
                let explicit_drive = explicit.voltage_source_index("Vd").expect("drain source");

                model.advance(h).expect("model initial point settles");
                explicit.advance(h).expect("explicit initial point settles");
                model.set_voltage_source(model_drive, sign * step);
                explicit.set_voltage_source(explicit_drive, sign * step);

                let (mut previous_voltage, mut previous_current) = (0.0, 0.0);
                for index in 0..3 {
                    let rule = if index == 0 {
                        // The first transient step restarts the companion with
                        // Backward Euler, regardless of the selected rule.
                        Integration::BackwardEuler
                    } else {
                        integration
                    };
                    let expected = match rule {
                        Integration::BackwardEuler => {
                            (step + beta * previous_voltage) / (1.0 + beta)
                        }
                        Integration::Trapezoidal => {
                            (step + 2.0 * beta * previous_voltage + resistance * previous_current)
                                / (1.0 + 2.0 * beta)
                        }
                    };

                    model.advance(h).expect("model bulk-charge step solves");
                    explicit
                        .advance(h)
                        .expect("explicit-resistor bulk-charge step solves");

                    let actual = sign * model.node_voltage(model_intrinsic);
                    let reference = sign * explicit.node_voltage(explicit_intrinsic);
                    assert!(
                        (actual - expected).abs() < 5e-9,
                        "{kind} {integration:?} step {index}: intrinsic drain should be {expected} V from CBD charge, got {actual} V"
                    );
                    assert!(
                        (actual - reference).abs() < 1e-10,
                        "{kind} {integration:?} step {index}: model-card RD gives {actual} V, explicit series resistance gives {reference} V"
                    );
                    assert!(
                        (model.node_voltage(model_external)
                            - explicit.node_voltage(explicit_external))
                        .abs()
                            < 1e-12,
                        "{kind} {integration:?} step {index}: external drains differ"
                    );
                    if index == 0 {
                        assert!(
                            (step - actual) > 0.1,
                            "{kind} {integration:?}: CBD displacement current must create a measurable RD drop"
                        );
                    }

                    previous_current = match rule {
                        Integration::BackwardEuler => {
                            capacitance * (expected - previous_voltage) / h
                        }
                        Integration::Trapezoidal => {
                            2.0 * capacitance * (expected - previous_voltage) / h - previous_current
                        }
                    };
                    previous_voltage = expected;
                }
            }
        }
    }

    /// Source bulk-junction depletion charge is attached to the intrinsic
    /// source. With model-card `RS`, its transient current must cross that
    /// resistor exactly as it does when the resistor is explicit in the
    /// netlist. The constant-capacitance case also has a hand-derived RC
    /// trajectory, so a missing junction charge cannot pass by matching two
    /// otherwise identical circuits.
    #[test]
    fn mosfet_source_bulk_junction_charge_flows_through_source_resistance() {
        let (resistance, capacitance, step, h) = (1e3, 100e-12, 1.0, 100e-9);
        let beta = resistance * capacitance / h;

        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            let vto = if kind == "NMOS" { 2.0 } else { -2.0 };
            let model_resistance = format!(
                "Vg g 0 dc 0\nVd d 0 dc 0\nVs s 0 dc 0\nVb b 0 dc 0\n\
                 M1 d g s b MMOD\n\
                 .model MMOD {kind}(VTO={vto} KP=20u LAMBDA=0 IS=1e-30 \
                 CBS={capacitance} PB=1 MJ=0 RS={resistance})\n"
            );
            let explicit_resistance = format!(
                "Vg g 0 dc 0\nVd d 0 dc 0\nVs s 0 dc 0\nVb b 0 dc 0\n\
                 RSM si s {resistance}\nM1 d g si b MMOD\n\
                 .model MMOD {kind}(VTO={vto} KP=20u LAMBDA=0 IS=1e-30 \
                 CBS={capacitance} PB=1 MJ=0)\n"
            );

            for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                let mut model = Solver::new(
                    parse_netlist(&model_resistance).expect("MOSFET bulk model parses"),
                    integration,
                )
                .expect("MOSFET bulk solver builds");
                let mut explicit = Solver::new(
                    parse_netlist(&explicit_resistance).expect("explicit resistor netlist parses"),
                    integration,
                )
                .expect("explicit resistor solver builds");

                let model_external = model.circuit().node("s").expect("external source");
                let model_intrinsic = model
                    .circuit()
                    .node("M1#source")
                    .expect("model-card RS creates the intrinsic source");
                let explicit_external = explicit.circuit().node("s").expect("external source");
                let explicit_intrinsic = explicit.circuit().node("si").expect("intrinsic source");
                let model_drive = model.voltage_source_index("Vs").expect("source drive");
                let explicit_drive = explicit.voltage_source_index("Vs").expect("source drive");

                model.advance(h).expect("model initial point settles");
                explicit.advance(h).expect("explicit initial point settles");
                model.set_voltage_source(model_drive, sign * step);
                explicit.set_voltage_source(explicit_drive, sign * step);

                let (mut previous_voltage, mut previous_current) = (0.0, 0.0);
                for index in 0..3 {
                    let rule = if index == 0 {
                        // The first transient step restarts the companion with
                        // Backward Euler, regardless of the selected rule.
                        Integration::BackwardEuler
                    } else {
                        integration
                    };
                    let expected = match rule {
                        Integration::BackwardEuler => {
                            (step + beta * previous_voltage)
                                / (1.0 + beta + 2.0 * device::GMIN * resistance)
                        }
                        Integration::Trapezoidal => {
                            (step + 2.0 * beta * previous_voltage + resistance * previous_current)
                                / (1.0 + 2.0 * beta + 2.0 * device::GMIN * resistance)
                        }
                    };

                    model.advance(h).expect("model bulk-charge step solves");
                    explicit
                        .advance(h)
                        .expect("explicit-resistor bulk-charge step solves");

                    let actual = sign * model.node_voltage(model_intrinsic);
                    let reference = sign * explicit.node_voltage(explicit_intrinsic);
                    assert!(
                        (actual - expected).abs() < 5e-9,
                        "{kind} {integration:?} step {index}: intrinsic source should be {expected} V from CBS charge, got {actual} V"
                    );
                    assert!(
                        (actual - reference).abs() < 1e-10,
                        "{kind} {integration:?} step {index}: model-card RS gives {actual} V, explicit series resistance gives {reference} V"
                    );
                    assert!(
                        (model.node_voltage(model_external)
                            - explicit.node_voltage(explicit_external))
                        .abs()
                            < 1e-12,
                        "{kind} {integration:?} step {index}: external sources differ"
                    );
                    if index == 0 {
                        assert!(
                            (step - actual) > 0.1,
                            "{kind} {integration:?}: CBS displacement current must create a measurable RS drop"
                        );
                    }

                    previous_current = match rule {
                        Integration::BackwardEuler => {
                            capacitance * (expected - previous_voltage) / h
                        }
                        Integration::Trapezoidal => {
                            2.0 * capacitance * (expected - previous_voltage) / h - previous_current
                        }
                    };
                    previous_voltage = expected;
                }
            }
        }
    }

    /// The model's sheet resistance is multiplied by each instance's drain
    /// and source square counts, and those effective resistors participate in
    /// the solved channel path for both MOS polarities.
    #[test]
    fn mosfet_sheet_resistance_drops_voltage_by_instance_square_count() {
        let sheet_resistance = 25.0;
        let squares = [("NRD", "drain", 2.0), ("NRS", "source", 4.0)];
        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            for (parameter, terminal, count) in squares {
                let resistance = sheet_resistance * count;
                let netlist = format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nVs s 0 dc 0\n\
                     M1 d g s 0 MMOD {parameter}={count}\n\
                     .model MMOD {kind}(VTO={} KP=20u LAMBDA=0 IS=1e-30 \
                     W=100u L=10u RSH={sheet_resistance})\n",
                    sign * 3.0,
                    sign * 0.4,
                    sign
                );
                let solver = build(&netlist);
                let external = solver
                    .circuit()
                    .node(if terminal == "drain" { "d" } else { "s" })
                    .expect("external terminal exists");
                let internal_name = format!("M1#{terminal}");
                let internal = solver
                    .circuit()
                    .node(&internal_name)
                    .expect("nonzero sheet resistance creates an internal terminal")
                    .expect("internal terminal is not ground");
                let current = sign * source_current(&solver, "Vd");
                assert!(current > 1e-6, "{kind} {parameter} current is {current} A");

                let (external_voltage, internal_voltage) = (
                    solver.node_voltage(external),
                    solver.node_voltage(Some(internal)),
                );
                let actual_drop = if terminal == "drain" {
                    sign * (external_voltage - internal_voltage)
                } else {
                    sign * (internal_voltage - external_voltage)
                };
                assert!(
                    relative(actual_drop, current * resistance) < 1e-6,
                    "{kind} {parameter} should drop {} V, got {actual_drop} V",
                    current * resistance
                );
            }
        }
    }

    /// JFET `RD` and `RS` sit outside the intrinsic channel terminals. The
    /// terminal voltage drops are checked against the source current, for both
    /// channel polarities, so these model-card parameters are exercised by the
    /// MNA solver rather than only parsed.
    #[test]
    fn jfet_terminal_resistances_drop_voltage_at_the_intrinsic_nodes() {
        let resistance = 100.0;
        for (kind, sign, vto) in [("NJF", 1.0, -2.0), ("PJF", -1.0, 2.0)] {
            for (parameter, terminal, signed_drop) in [
                ("RD", "drain", "external - intrinsic"),
                ("RS", "source", "intrinsic - external"),
            ] {
                let netlist = format!(
                    "Vg g 0 dc 0\nVd d 0 dc {}\nVs s 0 dc 0\n\
                     J1 d g s JMOD\n.model JMOD {kind}(VTO={vto} BETA=200u \
                     LAMBDA=0.02 IS=1e-30 {parameter}={resistance})\n",
                    sign * 0.4
                );
                let solver = build(&netlist);
                let external = solver
                    .circuit()
                    .node(if terminal == "drain" { "d" } else { "s" })
                    .expect("external terminal exists");
                let internal_name = format!("J1#{terminal}");
                let internal = solver
                    .circuit()
                    .node(&internal_name)
                    .expect("nonzero terminal resistance creates an internal node")
                    .expect("internal terminal is not ground");
                let current = sign * source_current(&solver, "Vd");
                assert!(current > 1e-6, "{kind} {parameter} current is {current} A");

                let (external_voltage, internal_voltage) = (
                    solver.node_voltage(external),
                    solver.node_voltage(Some(internal)),
                );
                let actual_drop = if parameter == "RD" {
                    sign * (external_voltage - internal_voltage)
                } else {
                    sign * (internal_voltage - external_voltage)
                };
                assert!(
                    relative(actual_drop, current * resistance) < 1e-6,
                    "{kind} {parameter} {signed_drop} should be {} V, got {actual_drop} V",
                    current * resistance
                );
            }
        }
    }

    /// Gate depletion current flows through the JFET's internal drain/source
    /// resistors during a gate step. Model-card RD/RS must match explicit
    /// series resistors around a resistance-free level-1 JFET for NJF/PJF and
    /// both transient companion rules.
    #[test]
    fn jfet_gate_charge_matches_explicit_terminal_resistors_during_transients() {
        let (rd, rs, h) = (800.0, 500.0, 10e-9);
        let (cgs, cgd, pb, fc) = (200e-12, 100e-12, 0.75, 0.5);

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            for (kind, sign, vto) in [("NJF", 1.0, -2.0), ("PJF", -1.0, 2.0)] {
                let gate_initial = -sign * 3.0;
                let gate_step = -sign * 1.5;
                let modelled = format!(
                    "Vg g 0 dc {gate_initial}\nVd d 0 dc {}\nVs s 0 dc 0\n\
                     J1 d g s JMOD\n.model JMOD {kind}(VTO={vto} BETA=200u \
                     LAMBDA=0.02 IS=1e-30 RD={rd} RS={rs} \
                     CGS={cgs} CGD={cgd} PB={pb} FC={fc})\n",
                    sign * 2.5
                );
                let explicit = format!(
                    "Vg g 0 dc {gate_initial}\nVd d 0 dc {}\nVs s 0 dc 0\n\
                     RDM d di {rd}\nRSM si s {rs}\n\
                     J1 di g si JMOD\n.model JMOD {kind}(VTO={vto} BETA=200u \
                     LAMBDA=0.02 IS=1e-30 CGS={cgs} CGD={cgd} PB={pb} FC={fc})\n",
                    sign * 2.5
                );
                let uncharged = format!(
                    "Vg g 0 dc {gate_initial}\nVd d 0 dc {}\nVs s 0 dc 0\n\
                     J1 d g s JMOD\n.model JMOD {kind}(VTO={vto} BETA=200u \
                     LAMBDA=0.02 IS=1e-30 RD={rd} RS={rs})\n",
                    sign * 2.5
                );

                let mut model = Solver::new(
                    parse_netlist(&modelled).expect("JFET terminal charge model parses"),
                    integration,
                )
                .expect("JFET terminal charge solver builds");
                let mut explicit = Solver::new(
                    parse_netlist(&explicit).expect("explicit JFET resistors parse"),
                    integration,
                )
                .expect("explicit-resistor JFET solver builds");
                let mut uncharged = Solver::new(
                    parse_netlist(&uncharged).expect("uncharged JFET model parses"),
                    integration,
                )
                .expect("uncharged JFET solver builds");

                let model_nodes = ["d", "s", "J1#drain", "J1#source", "g"]
                    .map(|name| model.circuit().node(name).expect("model node"));
                let explicit_nodes = ["d", "s", "di", "si", "g"]
                    .map(|name| explicit.circuit().node(name).expect("explicit node"));
                let model_gate = model.voltage_source_index("Vg").expect("model gate source");
                let explicit_gate = explicit
                    .voltage_source_index("Vg")
                    .expect("explicit gate source");
                let uncharged_gate = uncharged
                    .voltage_source_index("Vg")
                    .expect("uncharged gate source");

                model.advance(h).expect("model operating point settles");
                explicit
                    .advance(h)
                    .expect("explicit-resistor operating point settles");
                uncharged
                    .advance(h)
                    .expect("uncharged operating point settles");
                for (model_node, explicit_node) in model_nodes.into_iter().zip(explicit_nodes) {
                    assert!(
                        (model.node_voltage(model_node) - explicit.node_voltage(explicit_node))
                            .abs()
                            < 1e-9,
                        "{kind} {integration:?}: initial terminal voltages differ"
                    );
                }

                model.set_voltage_source(model_gate, gate_step);
                explicit.set_voltage_source(explicit_gate, gate_step);
                uncharged.set_voltage_source(uncharged_gate, gate_step);
                model.advance(h).expect("model JFET charge step solves");
                explicit
                    .advance(h)
                    .expect("explicit-resistor JFET charge step solves");
                uncharged.advance(h).expect("uncharged JFET step solves");

                let mut charge_effect = 0.0_f64;
                for (label, model_node, explicit_node) in [
                    ("external drain", model_nodes[0], explicit_nodes[0]),
                    ("external source", model_nodes[1], explicit_nodes[1]),
                    ("intrinsic drain", model_nodes[2], explicit_nodes[2]),
                    ("intrinsic source", model_nodes[3], explicit_nodes[3]),
                    ("gate", model_nodes[4], explicit_nodes[4]),
                ] {
                    let actual = model.node_voltage(model_node);
                    let expected = explicit.node_voltage(explicit_node);
                    charge_effect =
                        charge_effect.max((actual - uncharged.node_voltage(model_node)).abs());
                    assert!(
                        (actual - expected).abs() < 1e-8,
                        "{kind} {integration:?} {label}: model-card resistances give {actual} V, explicit resistors give {expected} V"
                    );
                }
                assert!(
                    charge_effect > 1e-7,
                    "{kind} {integration:?}: gate depletion charge must measurably affect terminal voltages, got a largest charged/uncharged difference of {charge_effect} V"
                );
                for source in ["Vd", "Vg"] {
                    let actual = source_current(&model, source);
                    let expected = source_current(&explicit, source);
                    assert!(
                        (actual - expected).abs() < 1e-10,
                        "{kind} {integration:?} {source}: model-card resistances give {actual} A, explicit resistors give {expected} A"
                    );
                }
            }
        }
    }

    /// Each level-1 JFET gate depletion capacitance is parsed and integrated
    /// as a nonlinear junction charge for NJF and PJF, under both companion
    /// rules. The expected node voltage is solved independently from the
    /// level-1 M=0.5 charge equation and the BE/trapezoidal current formulas.
    #[test]
    fn jfet_gate_depletion_charge_integrates_cgs_and_cgd_for_both_polarities() {
        let c0 = 10e-9;
        let pb = 0.75;
        let fc: f64 = 0.5;
        let resistance = 1e3;
        let h = 100e-6;
        let input = 0.6;
        let vfc = fc * pb;
        let qfc = c0 * pb / 0.5 * (1.0 - (1.0 - fc).sqrt());
        let cfc = c0 * (1.0 - fc).powf(-0.5);
        let slope = 0.5 * c0 * (1.0 - fc).powf(-1.5) / pb;
        let charge = |voltage: f64| {
            if voltage < vfc {
                c0 * pb / 0.5 * (1.0 - (1.0 - voltage / pb).sqrt())
            } else {
                let dv = voltage - vfc;
                qfc + cfc * dv + 0.5 * slope * dv * dv
            }
        };
        let solve_voltage = |previous_charge: f64, previous_current: f64, rule: Integration| {
            let mut lo = 0.0;
            // Trapezoidal integration can overshoot a step input because its
            // history term retains the previous positive current.
            let mut hi = input + 0.2;
            for _ in 0..100 {
                let mid = 0.5 * (lo + hi);
                let cap_current = match rule {
                    Integration::BackwardEuler => (charge(mid) - previous_charge) / h,
                    Integration::Trapezoidal => {
                        2.0 * (charge(mid) - previous_charge) / h - previous_current
                    }
                };
                if (input - mid) / resistance > cap_current {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            0.5 * (lo + hi)
        };

        for capacitance in ["CGS", "CGD"] {
            for (kind, sign) in [("NJF", 1.0), ("PJF", -1.0)] {
                let (cgs, cgd) = if capacitance == "CGS" {
                    (c0, 0.0)
                } else {
                    (0.0, c0)
                };
                let netlist = format!(
                    "V1 in 0 dc 0\nR1 in g {resistance}\nVd d 0 dc 0\nVs s 0 dc 0\n\
                     J1 d g s JMOD\n.model JMOD {kind}(VTO={} BETA=100u IS=1e-30 \
                     CGS={cgs} CGD={cgd} PB={pb} FC={fc})\n",
                    if kind == "NJF" { -2.0 } else { 2.0 }
                );
                let circuit = parse_netlist(&netlist).expect("JFET capacitance model parses");
                let model = circuit.jfets[0].model;
                assert_eq!(model.cgs, cgs);
                assert_eq!(model.cgd, cgd);
                assert_eq!(model.pb, pb);
                assert_eq!(model.fc, fc);

                for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                    let mut solver = Solver::new(circuit.clone(), integration)
                        .expect("JFET capacitance solver builds");
                    let gate = solver.circuit().node("g").expect("gate node");
                    let source = solver.voltage_source_index("V1").expect("input source");
                    solver.set_voltage_source(source, sign * input);

                    let first = solve_voltage(0.0, 0.0, Integration::BackwardEuler);
                    assert!(
                        first > vfc,
                        "the first step must exercise the FC continuation"
                    );
                    solver.advance(h).expect("first gate-charge step solves");
                    let actual_first = solver.node_voltage(gate);
                    assert!(
                        (actual_first - sign * first).abs() < 1e-8,
                        "{kind} {capacitance} first {integration:?} step: expected {} V, got {actual_first} V",
                        sign * first
                    );

                    let first_charge = charge(first);
                    let first_current = first_charge / h;
                    let second = solve_voltage(first_charge, first_current, integration);
                    assert!(
                        second > vfc,
                        "the second step must exercise the FC continuation"
                    );
                    solver.advance(h).expect("second gate-charge step solves");
                    let actual_second = solver.node_voltage(gate);
                    assert!(
                        (actual_second - sign * second).abs() < 1e-8,
                        "{kind} {capacitance} second {integration:?} step: expected {} V, got {actual_second} V",
                        sign * second
                    );
                }
            }
        }
    }

    #[test]
    fn jfet_gate_depletion_parameters_reject_invalid_values() {
        for parameter in ["CGS=-1n", "PB=0", "FC=1"] {
            let netlist = format!(
                "Vd d 0 dc 0\nVg g 0 dc 0\nVs s 0 dc 0\nJ1 d g s JMOD\n\
                 .model JMOD NJF({parameter})\n"
            );
            let error = parse_netlist(&netlist).expect_err("invalid JFET capacitance is refused");
            assert!(error.to_string().contains("JFET model"), "{error}");
        }
    }

    /// Thermal voltage written out from CODATA constants, independent of the
    /// engine's helper, for the JFET gate-junction temperature checks.
    fn kelvin_thermal_voltage(kelvin: f64) -> f64 {
        1.380_649e-23 * kelvin / 1.602_176_634e-19
    }

    /// An explicit instance `TEMP` heats both JFET gate junctions: `IS`
    /// follows the `EG`/`XTI` saturation law and the diode exponent uses
    /// `kT/q` at that temperature. The forward gate current is solved for both
    /// polarities, with default and explicit `EG`/`XTI`, and compared against
    /// a hand-written Shockley law. `TEMP == TNOM` must reproduce the card.
    #[test]
    fn jfet_gate_junction_leakage_follows_eg_xti_at_instance_temperature() {
        let (is, tnom, vgs): (f64, f64, f64) = (1e-14, 27.0, 0.45);
        let gate_current = |is_t: f64, kelvin: f64| {
            let vt = kelvin_thermal_voltage(kelvin);
            2.0 * (is_t * ((vgs / vt).exp() - 1.0) + device::GMIN * vgs)
        };
        let saturation = |eg: f64, xti: f64, kelvin: f64| {
            let nominal = tnom + 273.15;
            is * (eg / kelvin_thermal_voltage(nominal) - eg / kelvin_thermal_voltage(kelvin)).exp()
                * (kelvin / nominal).powf(xti)
        };
        // (EG/XTI card text, EG, XTI): ngspice's 1.11 eV / 3 defaults, then a
        // wide-gap card with a weaker power law.
        let cases = [("", 1.11, 3.0), (" EG=1.2 XTI=2", 1.2, 2.0)];

        for (kind, sign) in [("NJF", 1.0), ("PJF", -1.0)] {
            for (parameters, eg, xti) in cases {
                let netlist = |temp: Option<f64>| {
                    let temp = temp.map_or_else(String::new, |value| format!(" TEMP={value}"));
                    format!(
                        "Vg g 0 dc {}\nVd d 0 dc 0\nVs s 0 dc 0\n\
                         J1 d g s JMOD{temp}\n.model JMOD {kind}(VTO={} BETA=100u \
                         IS={is} TNOM={tnom}{parameters})\n",
                        sign * vgs,
                        -2.0 * sign
                    )
                };

                let nominal = build(&netlist(Some(tnom)));
                assert_eq!(nominal.circuit().jfets[0].operating_model.is, is);
                let nominal_current = sign * source_current(&nominal, "Vg");
                let nominal_expected = gate_current(is, tnom + 273.15);
                assert!(
                    relative(nominal_current, nominal_expected) < 1e-6,
                    "{kind}{parameters}: TEMP=TNOM expected {nominal_expected} A, got \
                     {nominal_current} A"
                );

                for temperature in [-20.0, 85.0, 125.0] {
                    let kelvin = temperature + 273.15;
                    let hot = build(&netlist(Some(temperature)));
                    let expected_is = saturation(eg, xti, kelvin);
                    let adjusted_is = hot.circuit().jfets[0].operating_model.is;
                    assert!(
                        relative(adjusted_is, expected_is) < 1e-12,
                        "{kind}{parameters} at {temperature} C: IS(T) {expected_is} A, got \
                         {adjusted_is} A"
                    );
                    let expected = gate_current(expected_is, kelvin);
                    let measured = sign * source_current(&hot, "Vg");
                    assert!(
                        relative(measured, expected) < 1e-6,
                        "{kind}{parameters} at {temperature} C: expected gate current \
                         {expected} A, got {measured} A"
                    );
                    // A wrong law cannot hide in the tolerance: the solved
                    // current moves by well over an order of magnitude.
                    let ratio = measured / nominal_current;
                    assert!(
                        !(0.1..=10.0).contains(&ratio),
                        "{kind}{parameters} at {temperature} C must move gate leakage \
                         decisively, ratio {ratio}"
                    );
                }

                // Without an instance TEMP the historical 300 K junction stays.
                let fixed = sign * source_current(&build(&netlist(None)), "Vg");
                let fixed_expected = gate_current(is, device::TEMPERATURE_K);
                assert!(
                    relative(fixed, fixed_expected) < 1e-6,
                    "{kind}{parameters}: no TEMP expected {fixed_expected} A, got {fixed} A"
                );
            }
        }
    }

    /// The level-1 JFET gate emission coefficient `N` (ngspice `jfetload.c`)
    /// sets `N·kT/q` for both gate diodes. The solved forward gate current is
    /// compared with a hand-written Shockley law for both polarities, at the
    /// fixed 300 K and at an instance `TEMP`. There `IS(T)` divides the `EG`
    /// term by `N` but keeps `XTI` undivided (ngspice `jfettemp.c`), unlike
    /// the diode law. `N <= 0` is refused.
    #[test]
    fn jfet_gate_emission_coefficient_scales_gate_diodes_and_is_temperature_law() {
        let (is, tnom, vgs, eg, xti): (f64, f64, f64, f64, f64) = (1e-14, 27.0, 0.6, 1.11, 3.0);
        let gate_current = |is_t: f64, n: f64, kelvin: f64| {
            let vte = n * kelvin_thermal_voltage(kelvin);
            2.0 * (is_t * ((vgs / vte).exp() - 1.0) + device::GMIN * vgs)
        };
        let netlist = |kind: &str, sign: f64, n: &str, temp: &str| {
            format!(
                "Vg g 0 dc {}\nVd d 0 dc 0\nVs s 0 dc 0\n\
                 J1 d g s JMOD{temp}\n.model JMOD {kind}(VTO={} BETA=100u \
                 IS={is} TNOM={tnom}{n})\n",
                sign * vgs,
                -2.0 * sign
            )
        };

        for (kind, sign) in [("NJF", 1.0), ("PJF", -1.0)] {
            let default_n = sign * source_current(&build(&netlist(kind, sign, "", "")), "Vg");
            let mut previous = None;
            for n in [1.0, 2.0] {
                let card = format!(" N={n}");
                let solver = build(&netlist(kind, sign, &card, ""));
                assert_eq!(solver.circuit().jfets[0].model.n, n);
                let measured = sign * source_current(&solver, "Vg");
                let expected = gate_current(is, n, device::TEMPERATURE_K);
                assert!(
                    relative(measured, expected) < 1e-6,
                    "{kind} N={n}: expected gate current {expected} A, got {measured} A"
                );
                if n == 1.0 {
                    assert!(
                        relative(measured, default_n) < 1e-12,
                        "{kind}: omitted N must equal N=1"
                    );
                }
                previous = Some(previous.map_or(measured, |unit: f64| unit / measured));
            }
            // At 0.6 V, N=2 halves the exponent: about five decades less current.
            let ratio = previous.expect("both N values solved");
            assert!(
                ratio > 1e4,
                "{kind}: N must reduce the gate current, ratio {ratio}"
            );

            let temperature = 85.0;
            let kelvin = temperature + 273.15;
            let nominal = tnom + 273.15;
            let n = 2.0;
            let hot = build(&netlist(
                kind,
                sign,
                " N=2",
                &format!(" TEMP={temperature}"),
            ));
            let expected_is = is
                * ((eg / kelvin_thermal_voltage(nominal) - eg / kelvin_thermal_voltage(kelvin))
                    / n)
                    .exp()
                * (kelvin / nominal).powf(xti);
            let adjusted_is = hot.circuit().jfets[0].operating_model.is;
            assert!(
                relative(adjusted_is, expected_is) < 1e-12,
                "{kind} N=2 at {temperature} C: IS(T) {expected_is} A, got {adjusted_is} A"
            );
            let diode_law_is = is
                * ((eg / kelvin_thermal_voltage(nominal) - eg / kelvin_thermal_voltage(kelvin)
                    + xti * (kelvin / nominal).ln())
                    / n)
                    .exp();
            assert!(
                relative(adjusted_is, diode_law_is) > 0.2,
                "{kind}: XTI must not be divided by N as in the diode law"
            );
            let measured = sign * source_current(&hot, "Vg");
            let expected = gate_current(expected_is, n, kelvin);
            assert!(
                relative(measured, expected) < 1e-6,
                "{kind} N=2 at {temperature} C: expected gate current {expected} A, got \
                 {measured} A"
            );
        }

        for parameter in ["N=0", "N=-1"] {
            let error = parse_netlist(&netlist("NJF", 1.0, &format!(" {parameter}"), ""))
                .expect_err("non-positive JFET N is refused");
            assert!(error.to_string().contains("N > 0"), "{error}");
        }
    }

    /// Instance `TEMP` moves the JFET gate potential `PB` and zero-bias
    /// `CGS`/`CGD` by the silicon depletion law at grading 0.5. A reverse-bias
    /// gate step through a resistor must then follow the same trajectory as a
    /// card that states the hand-computed `PB(T)`/`CGS(T)`/`CGD(T)` directly,
    /// and differ from the unheated card.
    #[test]
    fn jfet_gate_depletion_capacitance_follows_instance_temperature() {
        let (tnom, temperature, pb, cgs, cgd): (f64, f64, f64, f64, f64) =
            (27.0, 125.0, 0.8, 10e-9, 4e-9);
        let (nominal, kelvin) = (tnom + 273.15, temperature + 273.15);
        let bandgap = |t: f64| 1.16 - 7.02e-4 * t * t / (t + 1108.0);
        let (vt_n, vt) = (
            kelvin_thermal_voltage(nominal),
            kelvin_thermal_voltage(kelvin),
        );
        let ratio = kelvin / nominal;
        let pb_t =
            pb * ratio - vt * (3.0 * ratio.ln() + bandgap(nominal) / vt_n - bandgap(kelvin) / vt);
        let scale = 1.0 + 0.5 * (4e-4 * (kelvin - nominal) - pb_t / pb + 1.0);
        let (cgs_t, cgd_t) = (cgs * scale, cgd * scale);
        assert!(
            pb_t < pb - 0.1 && scale > 1.05,
            "a 98 K rise must lower PB and raise the depletion capacitance"
        );

        for (kind, sign) in [("NJF", 1.0), ("PJF", -1.0)] {
            let deck = |card: String, temp: Option<f64>| {
                let temp = temp.map_or_else(String::new, |value| format!(" TEMP={value}"));
                format!(
                    "V1 in 0 dc 0\nR1 in g 1k\nVd d 0 dc 0\nVs s 0 dc 0\n\
                     J1 d g s JMOD{temp}\n.model JMOD {kind}(VTO={} BETA=100u IS=1e-30 \
                     TNOM={tnom} FC=0.5 {card})\n",
                    -2.0 * sign
                )
            };
            let heated = deck(format!("CGS={cgs} CGD={cgd} PB={pb}"), Some(temperature));
            let explicit = deck(format!("CGS={cgs_t} CGD={cgd_t} PB={pb_t}"), None);
            let unheated = deck(format!("CGS={cgs} CGD={cgd} PB={pb}"), None);

            let heated_circuit = parse_netlist(&heated).expect("heated JFET parses");
            let operating = heated_circuit.jfets[0].operating_model;
            assert!((operating.pb - pb_t).abs() < 1e-12, "{kind}: PB(T)");
            assert!(relative(operating.cgs, cgs_t) < 1e-12, "{kind}: CGS(T)");
            assert!(relative(operating.cgd, cgd_t) < 1e-12, "{kind}: CGD(T)");

            for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                let run = |netlist: &str| {
                    let circuit = parse_netlist(netlist).expect("JFET step deck parses");
                    let mut solver =
                        Solver::new(circuit, integration).expect("JFET step solver builds");
                    let gate = solver.circuit().node("g").expect("gate node");
                    let source = solver.voltage_source_index("V1").expect("input source");
                    solver.set_voltage_source(source, -sign * 3.0);
                    (0..6)
                        .map(|_| {
                            solver.advance(2e-6).expect("gate step solves");
                            sign * solver.node_voltage(gate)
                        })
                        .collect::<Vec<_>>()
                };
                let heated_trace = run(&heated);
                let explicit_trace = run(&explicit);
                let unheated_trace = run(&unheated);
                let mut largest_shift: f64 = 0.0;
                for step in 0..heated_trace.len() {
                    assert!(
                        heated_trace[step] < 0.0 && heated_trace[step] > -3.0,
                        "{kind} {integration:?}: the gate must still be charging"
                    );
                    assert!(
                        (heated_trace[step] - explicit_trace[step]).abs() < 1e-9,
                        "{kind} {integration:?} step {step}: TEMP gives {} V, explicit \
                         PB(T)/CGS(T)/CGD(T) give {} V",
                        heated_trace[step],
                        explicit_trace[step]
                    );
                    largest_shift =
                        largest_shift.max((heated_trace[step] - unheated_trace[step]).abs());
                }
                assert!(
                    largest_shift > 0.02,
                    "{kind} {integration:?}: heating must slow the gate charge, shift \
                     {largest_shift} V"
                );
            }
        }
    }

    /// ngspice's level-1 silicon laws, written out independently of the
    /// engine: the Varshni bandgap and the built-in potential moved from
    /// `TNOM` to `T` (used for `PHI` and `PB`).
    fn spice_silicon_potential(potential: f64, nominal_kelvin: f64, kelvin: f64) -> f64 {
        let bandgap = |t: f64| 1.16 - 7.02e-4 * t * t / (t + 1108.0);
        let (vt_n, vt) = (
            kelvin_thermal_voltage(nominal_kelvin),
            kelvin_thermal_voltage(kelvin),
        );
        let ratio = kelvin / nominal_kelvin;
        potential * ratio
            - vt * (3.0 * ratio.ln() + bandgap(nominal_kelvin) / vt_n - bandgap(kelvin) / vt)
    }

    /// An explicit MOSFET instance `TEMP` moves the level-1 channel by
    /// ngspice's `mos1temp` laws: `KP` falls as `(T/TNOM)^-1.5`, `PHI` follows
    /// the silicon potential law and `VTO` moves with the bandgap and the
    /// surface potential, including the `GAMMA·√PHI` body term. A saturated,
    /// back-biased device is solved for NMOS and PMOS and compared against
    /// the hand-written square law. `TEMP == TNOM` and no `TEMP` reproduce
    /// the card.
    #[test]
    fn mosfet_channel_follows_level1_temperature_laws_at_instance_temperature() {
        let (vto, kp, gamma, phi, lambda, tnom): (f64, f64, f64, f64, f64, f64) =
            (0.8, 50e-6, 0.5, 0.65, 0.02, 27.0);
        let (w, l) = (20e-6, 2e-6);
        let (vgs, vds, vbs): (f64, f64, f64) = (2.5, 3.0, -1.0);
        let bandgap = |t: f64| 1.16 - 7.02e-4 * t * t / (t + 1108.0);
        let nominal_kelvin = tnom + 273.15;
        // Card-sign VTO and NMOS-coordinate drain current at `kelvin`.
        let drain_current = |sign: f64, kelvin: f64| {
            let ratio = kelvin / nominal_kelvin;
            let kp_t = kp / ratio.powf(1.5);
            let phi_t = spice_silicon_potential(phi, nominal_kelvin, kelvin);
            let card_vto = sign * vto;
            let card_vto_t = card_vto
                + sign * gamma * (phi_t.sqrt() - phi.sqrt())
                + sign * 0.5 * (phi_t - phi)
                + 0.5 * (bandgap(nominal_kelvin) - bandgap(kelvin));
            let threshold = sign * card_vto_t + gamma * ((phi_t - vbs).sqrt() - phi_t.sqrt());
            let vgst = vgs - threshold;
            assert!(vgst > 0.0 && vgst < vds, "reference must stay saturated");
            let channel = 0.5 * kp_t * w / l * vgst * vgst * (1.0 + lambda * vds);
            // GMIN drain-source and the reverse bulk-drain GMIN term; IS is
            // tiny enough that its leakage is far below the tolerance.
            let vbd = vbs - vds;
            (
                card_vto_t,
                channel + device::GMIN * vds - device::GMIN * vbd,
            )
        };

        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            let netlist = |temp: Option<f64>| {
                let temp = temp.map_or_else(String::new, |value| format!(" TEMP={value}"));
                format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nVb b 0 dc {}\n\
                     M1 d g 0 b MMOD W={w} L={l}{temp}\n.model MMOD {kind}(VTO={} KP={kp} \
                     GAMMA={gamma} PHI={phi} LAMBDA={lambda} IS=1e-30 TNOM={tnom})\n",
                    sign * vgs,
                    sign * vds,
                    sign * vbs,
                    sign * vto
                )
            };

            let nominal = build(&netlist(Some(tnom)));
            assert_eq!(nominal.circuit().mosfets[0].operating_model.vto, sign * vto);
            assert_eq!(nominal.circuit().mosfets[0].operating_model.kp, kp);
            let nominal_current = sign * source_current(&nominal, "Vd");
            let (_, nominal_expected) = drain_current(sign, nominal_kelvin);
            assert!(
                relative(nominal_current, nominal_expected) < 1e-6,
                "{kind}: TEMP=TNOM expected {nominal_expected} A, got {nominal_current} A"
            );
            let fixed = sign * source_current(&build(&netlist(None)), "Vd");
            assert!(
                relative(fixed, nominal_expected) < 1e-6,
                "{kind}: no TEMP keeps the card, expected {nominal_expected} A, got {fixed} A"
            );

            for temperature in [-40.0, 85.0, 125.0] {
                let kelvin = temperature + 273.15;
                let hot = build(&netlist(Some(temperature)));
                let operating = hot.circuit().mosfets[0].operating_model;
                let (expected_vto, expected) = drain_current(sign, kelvin);
                assert!(
                    (operating.vto - expected_vto).abs() < 1e-12,
                    "{kind} at {temperature} C: VTO(T) {expected_vto} V, got {} V",
                    operating.vto
                );
                assert!(
                    relative(operating.kp, kp / (kelvin / nominal_kelvin).powf(1.5)) < 1e-12,
                    "{kind} at {temperature} C: KP(T)"
                );
                let measured = sign * source_current(&hot, "Vd");
                assert!(
                    relative(measured, expected) < 1e-6,
                    "{kind} at {temperature} C: expected drain current {expected} A, got \
                     {measured} A"
                );
                // A missing law cannot hide in the tolerance: the solved
                // current moves by several percent either way.
                assert!(
                    relative(measured, nominal_current) > 0.05,
                    "{kind} at {temperature} C must move the channel decisively: {measured} A \
                     vs {nominal_current} A"
                );
            }
        }
    }

    /// The level-1 bulk junctions heat with the instance: `IS` scales by
    /// `exp(Eg(TNOM)/Vt(TNOM) − Eg(T)/Vt(T))` (level 1 has no `XTI`) and the
    /// diode exponent uses `kT/q` at `TEMP`. Both bulk junctions are forward
    /// biased with the channel off and the bulk current is compared against a
    /// hand-written Shockley law for NMOS and PMOS.
    #[test]
    fn mosfet_bulk_junction_leakage_follows_bandgap_law_at_instance_temperature() {
        let (is, tnom, forward): (f64, f64, f64) = (1e-14, 27.0, 0.5);
        let bandgap = |t: f64| 1.16 - 7.02e-4 * t * t / (t + 1108.0);
        let nominal_kelvin = tnom + 273.15;
        let saturation = |kelvin: f64| {
            is * (bandgap(nominal_kelvin) / kelvin_thermal_voltage(nominal_kelvin)
                - bandgap(kelvin) / kelvin_thermal_voltage(kelvin))
            .exp()
        };
        let bulk_current = |is_t: f64, kelvin: f64| {
            let vt = kelvin_thermal_voltage(kelvin);
            2.0 * (is_t * ((forward / vt).exp() - 1.0) + device::GMIN * forward)
        };

        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            let netlist = |temp: Option<f64>| {
                let temp = temp.map_or_else(String::new, |value| format!(" TEMP={value}"));
                format!(
                    "Vb b 0 dc {}\nVd d 0 dc 0\nVs s 0 dc 0\n\
                     M1 d 0 s b MMOD{temp}\n.model MMOD {kind}(VTO={} KP=20u IS={is} \
                     TNOM={tnom})\n",
                    sign * forward,
                    sign * 1.0
                )
            };

            let nominal = sign * source_current(&build(&netlist(Some(tnom))), "Vb");
            let nominal_expected = bulk_current(is, nominal_kelvin);
            assert!(
                relative(nominal, nominal_expected) < 1e-6,
                "{kind}: TEMP=TNOM expected {nominal_expected} A, got {nominal} A"
            );
            let fixed = sign * source_current(&build(&netlist(None)), "Vb");
            let fixed_expected = bulk_current(is, device::TEMPERATURE_K);
            assert!(
                relative(fixed, fixed_expected) < 1e-6,
                "{kind}: no TEMP expected {fixed_expected} A, got {fixed} A"
            );

            for temperature in [-20.0, 85.0, 125.0] {
                let kelvin = temperature + 273.15;
                let hot = build(&netlist(Some(temperature)));
                let expected_is = saturation(kelvin);
                let adjusted_is = hot.circuit().mosfets[0].operating_model.is;
                assert!(
                    relative(adjusted_is, expected_is) < 1e-12,
                    "{kind} at {temperature} C: IS(T) {expected_is} A, got {adjusted_is} A"
                );
                let expected = bulk_current(expected_is, kelvin);
                let measured = sign * source_current(&hot, "Vb");
                assert!(
                    relative(measured, expected) < 1e-6,
                    "{kind} at {temperature} C: expected bulk current {expected} A, got \
                     {measured} A"
                );
                let ratio = measured / nominal;
                assert!(
                    !(0.1..=10.0).contains(&ratio),
                    "{kind} at {temperature} C must move bulk leakage decisively, ratio {ratio}"
                );
            }
        }
    }

    /// Instance `TEMP` moves the bulk-junction `PB` and every zero-bias
    /// depletion capacitance (`CBD`, or `CJ·AD` with `MJ` and `CJSW·PD` with
    /// `MJSW`) by the silicon depletion law. A reverse-bias drain step through
    /// a resistor must follow a card that states the hand-computed
    /// `PB(T)`/`C(T)` directly, and differ from the unheated card.
    #[test]
    fn mosfet_bulk_depletion_capacitance_follows_instance_temperature() {
        let (tnom, temperature, pb, mj, mjsw): (f64, f64, f64, f64, f64) =
            (27.0, 125.0, 0.8, 0.5, 0.33);
        let (nominal_kelvin, kelvin) = (tnom + 273.15, temperature + 273.15);
        let pb_t = spice_silicon_potential(pb, nominal_kelvin, kelvin);
        let scale =
            |grading: f64| 1.0 + grading * (4e-4 * (kelvin - nominal_kelvin) - pb_t / pb + 1.0);
        assert!(
            pb_t < pb - 0.1 && scale(mj) > 1.05 && scale(mjsw) > 1.03,
            "a 98 K rise must lower PB and raise the depletion capacitance"
        );
        let (cbd, cj, cjsw, ad, pd) = (10e-9, 2e-3, 2e-9, 4e-6, 2.0);

        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            // (label, card parameters at TNOM, the same card at T, instance)
            let variants = [
                (
                    "CBD",
                    format!("CBD={cbd} PB={pb} MJ={mj}"),
                    format!("CBD={} PB={pb_t} MJ={mj}", cbd * scale(mj)),
                    String::new(),
                ),
                (
                    "CJ/CJSW",
                    format!("CJ={cj} CJSW={cjsw} PB={pb} MJ={mj} MJSW={mjsw}"),
                    format!(
                        "CJ={} CJSW={} PB={pb_t} MJ={mj} MJSW={mjsw}",
                        cj * scale(mj),
                        cjsw * scale(mjsw)
                    ),
                    format!(" AD={ad} PD={pd}"),
                ),
            ];
            for (label, card, card_t, instance) in variants {
                let deck = |card: &str, temp: Option<f64>| {
                    let temp = temp.map_or_else(String::new, |value| format!(" TEMP={value}"));
                    format!(
                        "V1 in 0 dc 0\nR1 in x 1k\nM1 x 0 0 0 MMOD{instance}{temp}\n\
                         .model MMOD {kind}(VTO={} KP=20u IS=1e-30 TNOM={tnom} FC=0.5 {card})\n",
                        sign * 1.0
                    )
                };
                let heated = deck(&card, Some(temperature));
                let explicit = deck(&card_t, None);
                let unheated = deck(&card, None);

                let heated_mosfet = parse_netlist(&heated)
                    .expect("heated MOSFET parses")
                    .mosfets[0]
                    .clone();
                let explicit_mosfet = parse_netlist(&explicit)
                    .expect("explicit MOSFET parses")
                    .mosfets[0]
                    .clone();
                assert!(
                    (heated_mosfet.operating_model.pb - pb_t).abs() < 1e-12,
                    "{kind} {label}: PB(T)"
                );
                let heated_caps = heated_mosfet.bulk_junction_capacitances(true);
                let explicit_caps = explicit_mosfet.bulk_junction_capacitances(true);
                for (heated_c, explicit_c) in heated_caps.into_iter().zip(explicit_caps) {
                    assert!(
                        heated_c == explicit_c || relative(heated_c, explicit_c) < 1e-12,
                        "{kind} {label}: zero-bias capacitances {heated_caps:?} vs {explicit_caps:?}"
                    );
                }

                for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                    let run = |netlist: &str| {
                        let circuit = parse_netlist(netlist).expect("MOSFET step deck parses");
                        let mut solver =
                            Solver::new(circuit, integration).expect("MOSFET step solver builds");
                        let drain = solver.circuit().node("x").expect("drain node");
                        let source = solver.voltage_source_index("V1").expect("input source");
                        solver.set_voltage_source(source, sign * 3.0);
                        (0..6)
                            .map(|_| {
                                solver.advance(2e-6).expect("drain step solves");
                                sign * solver.node_voltage(drain)
                            })
                            .collect::<Vec<_>>()
                    };
                    let heated_trace = run(&heated);
                    let explicit_trace = run(&explicit);
                    let unheated_trace = run(&unheated);
                    let mut largest_shift: f64 = 0.0;
                    for step in 0..heated_trace.len() {
                        assert!(
                            heated_trace[step] > 0.0 && heated_trace[step] < 3.0,
                            "{kind} {label} {integration:?}: the drain must still be charging"
                        );
                        assert!(
                            (heated_trace[step] - explicit_trace[step]).abs() < 1e-9,
                            "{kind} {label} {integration:?} step {step}: TEMP gives {} V, \
                             explicit PB(T)/C(T) give {} V",
                            heated_trace[step],
                            explicit_trace[step]
                        );
                        largest_shift =
                            largest_shift.max((heated_trace[step] - unheated_trace[step]).abs());
                    }
                    assert!(
                        largest_shift > 0.02,
                        "{kind} {label} {integration:?}: heating must slow the drain charge, \
                         shift {largest_shift} V"
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Newton iteration itself
    // -----------------------------------------------------------------------

    /// A netlist with no nonlinear device must not take the Newton path at all.
    /// This is the flag every byte-identity claim in this file rests on.
    #[test]
    fn a_linear_netlist_is_not_marked_nonlinear() {
        let linear =
            build("V1 in 0 dc 5\nR1 in out 1k\nC1 out 0 1u\nL1 out tail 1m\nR2 tail 0 100\n");
        assert!(!linear.is_nonlinear());
        assert!(build("V1 in 0 dc 5\nR1 in a 1k\nD1 a 0 D\n").is_nonlinear());
        assert!(build("V1 in 0 dc 5\nR1 in b 100k\nQ1 in b 0 NPN\n").is_nonlinear());
        assert!(build("V1 in 0 dc 5\nR1 in d 1k\nM1 d in 0 0 NMOS\n").is_nonlinear());
        assert!(build("V1 in 0 dc 0\nR1 in d 1k\nJ1 d in 0 NJF\n").is_nonlinear());
    }

    /// A solved load line checks that TC1/TC2 change the resistor stamp, not
    /// only the parsed fields. At ΔT=100 °C, the coefficients multiply 1 kΩ
    /// by 1 + 0.4 + 0.01 = 1.41.
    #[test]
    fn resistor_temperature_coefficients_change_the_operating_point() {
        let solver = build(
            "V1 in 0 dc 1.41\n\
             R1 in 0 1k tc1=4m tc2=1u tnom=25 temp=125\n",
        );

        assert!(
            (source_current(&solver, "V1") - 1e-3).abs() < 1e-12,
            "1.41 V across the adjusted 1.41 kΩ resistor must be 1 mA, got {} A",
            source_current(&solver, "V1")
        );
    }

    /// An ideal voltage source connected straight across a junction with no
    /// series resistance is not a hard circuit, it is an impossible one: the
    /// junction conductance runs away and the matrix loses its pivot. That has
    /// to be the named `Singular` error rather than a runaway current the trace
    /// shows as a plausible number.
    #[test]
    fn an_ideal_source_straight_across_a_junction_is_reported_as_singular() {
        let circuit = parse_netlist("V1 in 0 dc 5\nQ1 in in 0 NPN\n").expect("parses");
        let error = Solver::new(circuit, Integration::BackwardEuler)
            .expect_err("a source hard across a junction has no solution");
        assert!(
            matches!(error, AnalogError::Singular { .. }),
            "expected a Singular error, got {error}"
        );
    }

    /// Failure to converge is a coded error that names the step, not a `NaN`
    /// quietly written into the trace.
    ///
    /// Every circuit this engine is meant for converges, which is the point —
    /// so the *shape* of the failure is what is pinned here, and the test below
    /// pins the other half: that a stiff circuit driven far past anything
    /// reasonable still either solves or returns `Err`, never `NaN`.
    #[test]
    fn a_failure_to_converge_reports_the_step_rather_than_a_nan() {
        let error = AnalogError::NoConvergence {
            step: Some(42),
            time: 4.2e-3,
            iterations: 100,
            unknown: "v(out)".to_string(),
            delta: 1.5,
        };
        let text = error.to_string();
        assert!(text.contains("step 42"), "{text}");
        assert!(text.contains("4.2e-3"), "{text}");
        assert!(text.contains("v(out)"), "{text}");
        assert!(text.contains("100 iterations"), "{text}");
        assert!(
            text.contains("labwired_ngspice.py"),
            "the error has to name the engine that can run it: {text}"
        );
        assert_eq!(error.line(), None);
    }

    /// Whatever a nonlinear circuit does, it does not produce `NaN`: a step
    /// that cannot be solved returns `Err`. Swept over a stiff rectifier driven
    /// hard in both directions with a reactive load.
    #[test]
    fn a_stiff_nonlinear_circuit_never_produces_a_nan() {
        let mut solver = build(
            "Vin in 0 dc 0\nR1 in a 10\nD1 a out DMOD\nC1 out 0 1u\nR2 out 0 1k\n\
             .model DMOD D(IS=1e-15 N=1 RS=0.5)\n",
        );
        let vin = solver.voltage_source_index("Vin").unwrap();
        let out = solver.circuit().node("out").unwrap();
        for step in 0..2000 {
            // A brutal square wave: ±40 V with no ramp at all.
            solver.set_voltage_source(vin, if step % 20 < 10 { 40.0 } else { -40.0 });
            solver
                .advance(1e-6)
                .unwrap_or_else(|error| panic!("step {step}: {error}"));
            let v = solver.node_voltage(out);
            assert!(v.is_finite(), "step {step} produced {v}");
            assert!(
                v.abs() < 60.0,
                "step {step} produced {v} V out of a 40 V drive"
            );
        }
    }

    /// The limiters change the path Newton takes, never the point it reaches.
    /// Starting the same circuit from two different states must land on the
    /// same operating point to the last bit.
    #[test]
    fn the_converged_point_does_not_depend_on_the_path_to_it() {
        let netlist = "Vs s 0 dc 5\nR1 s a 1k\nD1 a b DMOD\nR2 b 0 470\n\
                       .model DMOD D(IS=2.52n N=1.752)\n";
        let cold = build(netlist);

        let mut warmed = build(netlist);
        let vs = warmed.voltage_source_index("Vs").unwrap();
        for value in [-30.0, 30.0, -30.0, 5.0] {
            warmed.set_voltage_source(vs, value);
            warmed.solve_operating_point().expect("op point solves");
        }

        let node = cold.circuit().node("b").unwrap();
        assert_eq!(
            cold.node_voltage(node).to_bits(),
            warmed.node_voltage(node).to_bits(),
            "cold {} V vs warmed {} V",
            cold.node_voltage(node),
            warmed.node_voltage(node)
        );
    }

    /// The SPICE instance multiplier `M` makes one element behave as that many
    /// identical devices in parallel, for every device kind. Each element has
    /// terminal resistances, so a multiplier that scaled only the junction
    /// or channel current would move the bias. DC values are ngspice-45
    /// `.op` (`.temp 26.85`, `tnom=26.85`), where `m=3` and three parallel
    /// copies print the same five node voltages.
    #[test]
    fn device_multiplier_m_matches_parallel_copies_and_ngspice() {
        let models = ".model DR D(IS=1e-14 RS=20 N=1.2 CJO=20p TT=5n)\n\
            .model QR NPN(IS=1e-15 BF=100 RB=50 RC=10 RE=2 VAF=50 IKF=10m CJE=5p CJC=3p TF=1n)\n\
            .model MR NMOS(LEVEL=1 VTO=1 KP=20u RD=200 RS=100 LAMBDA=0.02 CGSO=1n CGDO=1n CBD=2p)\n\
            .model JR NJF(VTO=-2 BETA=1e-4 RD=50 RS=50 LAMBDA=0.01 CGS=4p CGD=2p)\n";
        let devices = [
            ("D1", "a 0 DR"),
            ("Q1", "c b 0 QR"),
            ("M1", "dm supply 0 0 MR W=10u L=2u"),
            ("J1", "dj 0 0 JR"),
        ];
        let deck = |supply: &str, copies: usize, multiplier: Option<&str>| {
            let mut text = format!(
                "Vdd supply 0 {supply}\nRd supply a 1k\nRc supply c 1k\nRb supply b 100k\n\
                 Rm supply dm 10k\nRj supply dj 2k\n"
            );
            for (name, rest) in devices {
                for copy in 0..copies {
                    let suffix = multiplier.map_or(String::new(), |m| format!(" m={m}"));
                    text.push_str(&format!("{name}x{copy} {rest}{suffix}\n"));
                }
            }
            text + models
        };
        let nodes = ["a", "c", "b", "dm", "dj"];
        let volts = |solver: &Solver| {
            nodes.map(|node| solver.node_voltage(solver.circuit().node(node).expect("node exists")))
        };

        let single = volts(&build(&deck("dc 3.3", 1, None)));
        let multiplied = volts(&build(&deck("dc 3.3", 1, Some("3"))));
        let parallel = volts(&build(&deck("dc 3.3", 3, None)));
        let ngspice_single = [0.862_150_2, 1.172_329, 0.744_160_3, 1.249_760, 2.496_725];
        let ngspice_m3 = [
            0.796_819_9,
            0.895_513_0,
            0.712_543_9,
            0.479_434_2,
            1.259_871,
        ];
        for (index, node) in nodes.iter().enumerate() {
            assert!(
                (single[index] - ngspice_single[index]).abs() < 1e-4,
                "V({node}) single device: ngspice {}, got {}",
                ngspice_single[index],
                single[index]
            );
            assert!(
                (multiplied[index] - ngspice_m3[index]).abs() < 1e-4,
                "V({node}) with m=3: ngspice {}, got {}",
                ngspice_m3[index],
                multiplied[index]
            );
            assert!(
                (multiplied[index] - parallel[index]).abs() < 1e-9,
                "V({node}): m=3 gives {}, three parallel copies give {}",
                multiplied[index],
                parallel[index]
            );
        }

        // Stored charge scales too: a supply step charges m=3 the same way
        // as three copies, step for step.
        let pulse = "PULSE(0 3.3 0 20n 20n 1u 2u)";
        let mut multiplied = build(&deck(pulse, 1, Some("3")));
        let mut parallel = build(&deck(pulse, 3, None));
        let mut single = build(&deck(pulse, 1, None));
        let mut largest_difference: f64 = 0.0;
        for _ in 0..40 {
            for solver in [&mut multiplied, &mut parallel, &mut single] {
                solver.advance(5e-9).expect("transient step solves");
            }
            let (m, p, one) = (volts(&multiplied), volts(&parallel), volts(&single));
            for index in 0..nodes.len() {
                assert!(
                    (m[index] - p[index]).abs() < 1e-7,
                    "V({}) at {} s: m=3 gives {}, three copies give {}",
                    nodes[index],
                    multiplied.time(),
                    m[index],
                    p[index]
                );
                largest_difference = largest_difference.max((m[index] - one[index]).abs());
            }
        }
        assert!(
            largest_difference > 0.1,
            "m=3 must load the circuit differently from one device, max diff {largest_difference} V"
        );

        for value in ["0", "-2", "1e999"] {
            let error = parse_netlist(&format!("D1 a 0 DR m={value}\n{models}"))
                .expect_err("a non-positive multiplier must be refused")
                .to_string();
            assert!(
                error.contains("multiplier"),
                "m={value}: unexpected error {error}"
            );
        }
    }

    /// Diode sidewall current `JSW`·`PJ` follows ngspice-42. Without `NS` it
    /// merges into the bottom saturation current (shared `N`, breakdown
    /// region and knee) but temperature-scales with `NS = 1`; with `NS` it is
    /// a separate exponential. The instance `pj=` overrides the model `PJ`,
    /// and `JSW` with no perimeter adds nothing. Reference values are
    /// ngspice-42 `.op` (`.temp 26.85`, `tnom=26.85`). Without the sidewall
    /// the rows move by 39 mV (a), 62 mV (b), 93 mV (c), 145 mV (z) and
    /// 244 mV (e), so an ignored `JSW` fails every row.
    #[test]
    fn diode_sidewall_current_jsw_pj_ns_matches_ngspice() {
        let netlist = "Vdd supply 0 dc 3.3\n\
            R1 supply a 1k\nR2 supply b 1k\nR3 supply c 100k\nR4 supply z 470\n\
            R5 supply e 1k\nR6 supply f 1k\n\
            D1 a 0 DS\nD2 b 0 DS pj=4\nD3 c 0 DN pj=2\nD4 0 z DZ pj=3\n\
            D5 e 0 DT pj=3 temp=75\nD6 f 0 DP\n\
            .model DS D(IS=1e-14 N=1.1 JSW=2e-14 PJ=1.5)\n\
            .model DN D(IS=1e-15 N=1 JSW=5e-10 NS=2)\n\
            .model DZ D(IS=1e-14 BV=2.0 IBV=1m JSW=1e-12)\n\
            .model DT D(IS=1e-14 N=1.5 JSW=1e-13 EG=1.11 XTI=3 TNOM=27)\n\
            .model DP D(IS=1e-14 JSW=1e-12)\n";
        let solver = build(netlist);
        let v = |node: &str| solver.node_voltage(solver.circuit().node(node).expect("node exists"));
        for (node, ngspice) in [
            ("a", 0.707_932_4),
            ("b", 0.685_121_0),
            ("c", 0.527_559_8),
            ("z", 1.881_026),
            ("e", 0.742_548_9),
            ("f", 0.679_693_5),
        ] {
            let got = v(node);
            assert!(
                (got - ngspice).abs() < 1e-4,
                "V({node}): ngspice-42 {ngspice}, got {got}"
            );
        }

        // JSW with no perimeter must be bit-identical to the plain diode.
        let plain = build(&netlist.replace("DP D(IS=1e-14 JSW=1e-12)", "DP D(IS=1e-14)"));
        let plain_f = plain.node_voltage(plain.circuit().node("f").expect("node exists"));
        assert_eq!(v("f"), plain_f, "JSW with PJ=0 must add no current");

        for card in ["JSW=-1e-12", "NS=0", "PJ=-1"] {
            let error = parse_netlist(&format!(
                "V1 a 0 1\nD1 a 0 DX\n.model DX D(IS=1e-14 {card})\n"
            ))
            .expect_err("an invalid sidewall card must be refused")
            .to_string();
            assert!(error.contains("JSW"), "{card}: unexpected error {error}");
        }
        let error = parse_netlist("V1 a 0 1\nD1 a 0 DX pj=-2\n.model DX D(IS=1e-14)\n")
            .expect_err("a negative instance perimeter must be refused")
            .to_string();
        assert!(error.contains("pj"), "unexpected error {error}");
    }

    /// Diode sidewall depletion charge `CJSW`/`CJP`·`PJ` with `PHP`, `MJSW`
    /// and `FCS` follows ngspice-42. Each diode has its own source; one
    /// backward-Euler step to a new voltage and a second step held there give
    /// the stored-charge change as `(I_step − I_hold)·h`. Reference charges are
    /// ngspice-42 `.dc` `@d[charge]` (`.temp 26.85`, `tnom=26.85`). Above
    /// `FCS·PHP` ngspice-42 starts the sidewall's linear region from the
    /// bottom junction's `F1`, so the sidewall diode parts from an otherwise
    /// identical bottom (`CJO`) diode at 0.3 V and 0.6 V. `AREA` leaves the
    /// sidewall unscaled, and `TEMP` moves `CJSW`/`PHP` by the junction law
    /// (that row uses `TNOM=27`, ngspice's `REFTEMP`, in both decks).
    #[test]
    fn diode_sidewall_capacitance_cjsw_php_mjsw_fcs_matches_ngspice() {
        let netlist = "V1 a 0 dc 0\nV2 b 0 dc 0\nV3 c 0 dc 0\nV4 d 0 dc 0\nV5 e 0 dc 0\n\
            D1 a 0 DSW pj=3\nD2 b 0 DSWT pj=3 temp=75\nD3 c 0 DB\n\
            D4 d 0 DSW pj=3 area=2\nD5 e 0 DMIX pj=2\n\
            .model DSW D(IS=1e-14 CJSW=2p PHP=0.5 MJSW=0.4 FCS=0.5)\n\
            .model DSWT D(IS=1e-14 CJSW=2p PHP=0.5 MJSW=0.4 FCS=0.5 TNOM=27)\n\
            .model DB D(IS=1e-14 CJO=6p VJ=0.5 M=0.4 FC=0.5)\n\
            .model DMIX D(IS=1e-14 CJO=4p VJ=0.7 M=0.5 FC=0.6 CJP=1p PHP=0.5 MJSW=0.4 FCS=0.5)\n";
        let sources = ["V1", "V2", "V3", "V4", "V5"];
        // ngspice-42 charge per diode, in the order of `sources`.
        let ngspice: [(f64, [f64; 5]); 7] = [
            (0.0, [0.0; 5]),
            (
                -3.0,
                [
                    -1.107_047_925e-11,
                    -1.136_974_361e-11,
                    -1.107_047_925e-11,
                    -1.107_047_925e-11,
                    -1.096_494_130e-11,
                ],
            ),
            (
                -1.2,
                [
                    -5.419_751_331e-12,
                    -5.672_842_183e-12,
                    -5.419_751_331e-12,
                    -5.419_751_331e-12,
                    -5.432_633_853e-12,
                ],
            ),
            (
                -0.2,
                [
                    -1.118_526_224e-12,
                    -1.214_534_991e-12,
                    -1.118_526_224e-12,
                    -1.118_526_224e-12,
                    -1.122_645_221e-12,
                ],
            ),
            (
                0.1,
                [
                    6.265_517_042e-13,
                    7.054_815_772e-13,
                    6.265_517_042e-13,
                    6.265_517_042e-13,
                    6.242_580_094e-13,
                ],
            ),
            (
                0.3,
                [
                    3.926_405_094e-12,
                    4.824_073_298e-12,
                    2.112_916_691e-12,
                    3.926_405_094e-12,
                    2.533_151_235e-12,
                ],
            ),
            (
                0.6,
                [
                    7.061_555_890e-12,
                    8.930_539_696e-12,
                    5.248_067_487e-12,
                    7.061_555_890e-12,
                    5.591_032_927e-12,
                ],
            ),
        ];
        let h = 1e-6;
        let mut solver = Solver::new(
            parse_netlist(netlist).expect("sidewall deck parses"),
            Integration::BackwardEuler,
        )
        .expect("sidewall solver builds");
        let indices = sources.map(|name| solver.voltage_source_index(name).expect("source exists"));
        for window in ngspice.windows(2) {
            let ((_, before), (voltage, after)) = (window[0], window[1]);
            for index in indices {
                solver.set_voltage_source(index, voltage);
            }
            solver.advance(h).expect("step to the new bias solves");
            let step = sources.map(|name| source_current(&solver, name));
            solver.advance(h).expect("held bias solves");
            let hold = sources.map(|name| source_current(&solver, name));
            for (column, name) in sources.iter().enumerate() {
                let expected = after[column] - before[column];
                let got = (step[column] - hold[column]) * h;
                assert!(
                    (got - expected).abs() < 1e-4 * expected.abs() + 1e-17,
                    "{name} {voltage} V: ngspice-42 charge change {expected} C, got {got} C"
                );
            }
        }

        // Ignoring CJSW, the sidewall diodes would store no charge, and the
        // forward-region F1 quirk is a 1.8 pC offset at 0.3 V.
        let (_, at_0v3) = ngspice[5];
        assert!(at_0v3[0] - at_0v3[2] > 1.5e-12);

        // CJSW with no perimeter must be bit-identical to the plain diode.
        let transient = |card: &str| {
            let mut solver = Solver::new(
                parse_netlist(&format!(
                    "V1 in 0 PULSE(-2 0.5 0 10n 10n 1u 2u)\nR1 in a 1k\nD1 a 0 DX\n.model DX D({card})\n"
                ))
                .expect("transient deck parses"),
                Integration::Trapezoidal,
            )
            .expect("transient solver builds");
            let a = solver.circuit().node("a").expect("node exists");
            (0..40)
                .map(|_| {
                    solver.advance(5e-9).expect("transient step solves");
                    solver.node_voltage(a)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            transient("IS=1e-14 CJSW=2p PHP=0.5"),
            transient("IS=1e-14"),
            "CJSW with PJ=0 must add no charge"
        );
        assert_ne!(
            transient("IS=1e-14 CJSW=2p PHP=0.5 PJ=500"),
            transient("IS=1e-14"),
            "CJSW with a perimeter must load the node"
        );

        for card in ["CJSW=-1p", "PHP=0", "MJSW=-0.1", "FCS=1"] {
            let error = parse_netlist(&format!(
                "V1 a 0 1\nD1 a 0 DX\n.model DX D(IS=1e-14 {card})\n"
            ))
            .expect_err("an invalid sidewall capacitance card must be refused")
            .to_string();
            assert!(error.contains("CJSW"), "{card}: unexpected error {error}");
        }
    }

    /// The BJT substrate junction is a pn junction from the optional fourth
    /// node (ground when omitted) to the internal collector of a vertical
    /// device or the internal base of a lateral one: `CJS`/`VJS`/`MJS`
    /// depletion charge with no `FC`, and `ISS`/`NS` current. References are
    /// ngspice-42 `@q[qsub]` and source currents from one `.op` per bias
    /// (`.temp 26.85`, `tnom=26.85`, `TNOM=27` on the `temp=75` cards). Q1 is
    /// a vertical NPN, Q2 a lateral PNP (ngspice's default), Q3 a
    /// three-terminal NPN at 75 °C with area 2 and `TMJS1`, Q4/Q5 carry
    /// `ISS`, Q5 at 75 °C with area 3 and `TNS1`.
    #[test]
    fn bjt_substrate_junction_cjs_iss_matches_ngspice() {
        let netlist = "V1 s1 0 dc 0\nV2 s2 0 dc 0\nVB2 b2 0 dc 0\nVC2 c2 0 dc 0\n\
            V3 c3 0 dc 0\nV4 s4 0 dc 0\nV5 s5 0 dc 0\n\
            Q1 0 0 0 s1 QN\nQ2 c2 b2 0 s2 QP\nQ3 c3 0 0 QT 2 temp=75\n\
            Q4 0 0 0 s4 QNI\nQ5 0 0 0 s5 QNIT 3 temp=75\n\
            .model QN NPN(IS=1e-16 CJS=2p VJS=0.6 MJS=0.4)\n\
            .model QP PNP(IS=1e-16 CJS=3p VJS=0.7 MJS=0.5)\n\
            .model QT NPN(IS=1e-16 CJS=2p VJS=0.6 MJS=0.4 TNOM=27 TMJS1=1e-3)\n\
            .model QNI NPN(IS=1e-16 ISS=1e-15 NS=1.2)\n\
            .model QNIT NPN(IS=1e-16 ISS=1e-15 NS=1.2 TNS1=2e-3 TNOM=27)\n";
        // (substrate bias, [qsub Q1, Q2, Q3], [I(V4), I(V5)]) from ngspice-42.
        let ngspice: [(f64, [f64; 3], [f64; 2]); 7] = [
            (0.0, [0.0; 3], [0.0; 2]),
            (
                -3.0,
                [-3.860_312_103e-12, -5.456_086_164e-12, -7.788_802_453e-12],
                [3.000_999_998_5e-12, 4.737_947_741_9e-12],
            ),
            (
                -1.2,
                [-1.866_364_090e-12, -2.719_537_557e-12, -3.852_801_257e-12],
                [1.200_999_976_8e-12, 2.937_870_002_1e-12],
            ),
            (
                -0.2,
                [-3.768_032_773e-13, -5.623_523_599e-13, -8.049_207_152e-13],
                [2.009_949_833_2e-13, 1.920_013_108_7e-12],
            ),
            (
                0.1,
                [
                    2.066_666_666_7e-13,
                    3.107_142_857_1e-13,
                    4.522_756_122_4e-13,
                ],
                [-1.241_153_283e-13, -2.027_487_920e-11],
            ),
            (
                0.3,
                [6.6e-13, 9.964_285_714_3e-13, 1.467_979_230_7e-12],
                [-1.614_123_970e-11, -3.482_091_275e-9],
            ),
            (
                0.6,
                [1.44e-12, 2.185_714_285_7e-12, 3.269_415_643_4e-12],
                [-2.509_771_576e-7, -6.982_336_980e-6],
            ),
        ];
        let h = 1e-6;
        let mut solver = Solver::new(
            parse_netlist(netlist).expect("substrate deck parses"),
            Integration::BackwardEuler,
        )
        .expect("substrate solver builds");
        let index = |name: &str| solver.voltage_source_index(name).expect("source exists");
        let (v1, v2, v3, v4, v5) = (
            index("V1"),
            index("V2"),
            index("V3"),
            index("V4"),
            index("V5"),
        );
        // The DC sources carry substrate current; ngspice reports the current
        // into the source's + terminal, the engine the current out of it.
        let charge_sources = ["V1", "V2", "V3"];
        // Q3's substrate is ground and its collector the cathode, so the
        // charge current leaves through V3 with the opposite sign.
        let orientation = [1.0, 1.0, -1.0];
        for window in ngspice.windows(2) {
            let ((_, before, _), (voltage, after, dc)) = (window[0], window[1]);
            for source in [v1, v2, v4, v5] {
                solver.set_voltage_source(source, voltage);
            }
            solver.set_voltage_source(v3, -voltage);
            solver.advance(h).expect("step to the new bias solves");
            let step = charge_sources.map(|name| source_current(&solver, name));
            let step_base = source_current(&solver, "VB2");
            let step_collector = source_current(&solver, "VC2");
            solver.advance(h).expect("held bias solves");
            let hold = charge_sources.map(|name| source_current(&solver, name));
            let hold_base = source_current(&solver, "VB2");
            let hold_collector = source_current(&solver, "VC2");
            for (column, name) in charge_sources.iter().enumerate() {
                let expected = after[column] - before[column];
                let got = orientation[column] * (step[column] - hold[column]) * h;
                assert!(
                    (got - expected).abs() < 1e-6 * expected.abs() + 1e-18,
                    "{name} {voltage} V: ngspice-42 substrate charge change {expected} C, got {got} C"
                );
            }
            // A lateral PNP's substrate junction sits on the base, not the
            // collector.
            let expected = after[1] - before[1];
            let base = -(step_base - hold_base) * h;
            assert!(
                (base - expected).abs() < 1e-6 * expected.abs() + 1e-18,
                "Q2 {voltage} V: lateral substrate charge through the base {expected} C, got {base} C"
            );
            assert!(
                ((step_collector - hold_collector) * h).abs() < 1e-18,
                "Q2 {voltage} V: no substrate charge may reach the lateral collector"
            );
            // Below −3·NS·Vt ngspice-42 switches to its cubic reverse form
            // −ISS·(1 + (3·NS·Vt/(e·V))³); the engine's shared diode path
            // keeps −ISS·(1 − e^(V/(NS·Vt))). The two differ by at most
            // ~1.2 % of ISS (2e-14 A for Q5 at 75 °C, area 3), worst at −0.2 V.
            // ngspice-42 also uses CODATA-2014 k and q, the engine CODATA-2018:
            // 3e-6 relative on the 75 °C exponential.
            let reverse_allowance = if voltage < 0.0 { 1e-13 } else { 1e-17 };
            for (column, name) in ["V4", "V5"].iter().enumerate() {
                let expected = -dc[column];
                let got = source_current(&solver, name);
                assert!(
                    (got - expected).abs() < 1e-5 * expected.abs() + reverse_allowance,
                    "{name} {voltage} V: ngspice-42 substrate current {expected} A, got {got} A"
                );
            }
        }

        // `SUBS=1` moves a PNP's junction onto the collector, and the cathode
        // turns round: the collector is now the anode.
        let vertical_pnp = |subs: &str| {
            let mut solver = Solver::new(
                parse_netlist(&format!(
                    "VS s 0 dc 0.5\nVB b 0 dc 0\nVC c 0 dc 0\nQ1 c b 0 s QP\n\
                     .model QP PNP(IS=1e-16 ISS=1e-15 {subs})\n"
                ))
                .expect("vertical PNP deck parses"),
                Integration::BackwardEuler,
            )
            .expect("vertical PNP solver builds");
            solver.solve_operating_point().expect("vertical PNP solves");
            [source_current(&solver, "VB"), source_current(&solver, "VC")]
        };
        let [lateral_base, lateral_collector] = vertical_pnp("");
        let [vertical_base, vertical_collector] = vertical_pnp("SUBS=1");
        // Lateral: substrate is the anode at +0.5 V, so ISS·(e^(0.5/Vt)−1)
        // leaves through the base. Vertical: the substrate is the cathode,
        // the junction is reverse biased, so VC supplies −(ISS + GMIN·0.5).
        assert!(
            lateral_base < -1e-8,
            "lateral PNP: forward substrate current {lateral_base}"
        );
        assert!(lateral_collector.abs() < 1e-11);
        assert!(vertical_base.abs() < 1e-11);
        assert!(
            (vertical_collector + 1e-15 + 0.5e-12).abs() < 1e-17,
            "vertical PNP: reverse substrate current {vertical_collector}"
        );

        // A model name is never read as the substrate node, and a deck with
        // no substrate parameters is bit-identical with or without a fourth
        // node tied to ground.
        let amplifier = |q: &str| {
            let mut solver = Solver::new(
                parse_netlist(&format!(
                    "V1 vcc 0 5\nVIN in 0 PULSE(0.6 0.7 0 10n 10n 1u 2u)\nRB in b 10k\n\
                     RC vcc c 2k\n{q}\n.model QA NPN(IS=1e-15 BF=100 CJE=2p CJC=1p)\n"
                ))
                .expect("amplifier deck parses"),
                Integration::Trapezoidal,
            )
            .expect("amplifier solver builds");
            let c = solver.circuit().node("c").expect("node exists");
            (0..20)
                .map(|_| {
                    solver.advance(5e-9).expect("amplifier step solves");
                    solver.node_voltage(c)
                })
                .collect::<Vec<_>>()
        };
        let three_terminal = amplifier("Q1 c b 0 QA");
        assert_eq!(three_terminal, amplifier("Q1 c b 0 QA 1"));
        assert_ne!(
            three_terminal,
            amplifier("Q1 c b 0 0 QA"),
            "an explicit substrate adds ngspice's GMIN across the junction"
        );
        assert_ne!(
            amplifier("Q1 c b 0 0 QA"),
            amplifier("Q1 c b 0 0 QS\n.model QS NPN(IS=1e-15 BF=100 CJE=2p CJC=1p CJS=5p)"),
            "CJS must load the collector"
        );

        for card in ["CJS=-1p", "VJS=0", "MJS=-0.1", "ISS=-1e-15", "NS=0"] {
            let error = parse_netlist(&format!(
                "V1 c 0 1\nQ1 c 0 0 s QX\n.model QX NPN(IS=1e-16 {card})\n"
            ))
            .expect_err("an invalid substrate junction must be refused")
            .to_string();
            assert!(
                error.contains("substrate"),
                "{card}: unexpected error {error}"
            );
        }
        let error = parse_netlist("V1 c 0 1\nQ1 c 0 0 QX\n.model QX NPN(SUBS=2)\n")
            .expect_err("SUBS must be 1 or -1")
            .to_string();
        assert!(error.contains("SUBS"), "unexpected error {error}");
    }

    /// SPICE `AREA` on D, Q and J scales the card the instance sees: the
    /// junction and channel currents, knee currents and depletion
    /// capacitances grow with area and the terminal resistances shrink. Every
    /// area-scaled parameter the engine models is set, so one that stayed
    /// unscaled would move a node. DC values are ngspice-45 `.op` with
    /// `area=2.5` (`.temp 26.85`, `tnom=26.85`); ngspice-45 reads a bare
    /// number after a diode's model as a thermal node, so the reference deck
    /// uses the keyword spelling.
    #[test]
    fn device_area_scales_d_q_j_cards_like_ngspice() {
        let models = ".model DR D(IS=1e-14 RS=20 N=1.2 ISR=1e-12 NR=2 IK=5m CJO=20p TT=5n)\n\
            .model DZ D(IS=1e-14 RS=5 BV=2.0 IBV=1m)\n\
            .model QR NPN(IS=1e-15 BF=100 RB=50 RBM=10 IRB=50u RC=10 RE=2 VAF=50 IKF=10m \
            ISE=1e-13 NE=1.5 ISC=1e-13 NC=2 IKR=5m CJE=5p CJC=3p TF=1n XTF=1 ITF=5m)\n\
            .model JR NJF(VTO=-2 BETA=1e-4 RD=50 RS=50 LAMBDA=0.01 IS=1e-14 CGS=4p CGD=2p)\n";
        let deck = |supply: &str, size: &str| {
            format!(
                "Vdd supply 0 {supply}\nRd supply a 1k\nRz supply z 470\nRc supply c 1k\n\
                 Rb supply b 100k\nRj supply dj 2k\n\
                 D1 a 0 DR {size}\nD2 0 z DZ {size}\nQ1 c b 0 QR {size}\nJ1 dj 0 0 JR {size}\n\
                 {models}"
            )
        };
        let nodes = ["a", "z", "c", "b", "dj"];
        let volts = |solver: &Solver| {
            nodes.map(|node| solver.node_voltage(solver.circuit().node(node).expect("node exists")))
        };

        let single = volts(&build(&deck("dc 3.3", "")));
        let keyword = volts(&build(&deck("dc 3.3", "area=2.5")));
        let positional = volts(&build(&deck("dc 3.3", "2.5")));
        let ngspice_single = [0.882_609_7, 2.038_933, 2.061_702, 0.725_526_4, 2.496_725];
        let ngspice_area = [0.819_020_6, 2.031_081, 2.183_537, 0.695_149_0, 1.464_235];
        for (index, node) in nodes.iter().enumerate() {
            assert!(
                (single[index] - ngspice_single[index]).abs() < 1e-4,
                "V({node}) area 1: ngspice {}, got {}",
                ngspice_single[index],
                single[index]
            );
            assert!(
                (keyword[index] - ngspice_area[index]).abs() < 1e-4,
                "V({node}) area=2.5: ngspice {}, got {}",
                ngspice_area[index],
                keyword[index]
            );
            assert_eq!(
                keyword[index], positional[index],
                "V({node}): `area=2.5` and a bare `2.5` must be the same instance"
            );
        }

        // The breakdown knee is where area and m part: ngspice-45 fits BV to
        // the unscaled IBV for one area-3 junction, but m=3 is three
        // junctions that each carry IBV at the knee.
        let z = |size: &str| {
            let solver = build(&deck("dc 3.3", size));
            solver.node_voltage(solver.circuit().node("z").expect("node exists"))
        };
        for (size, ngspice) in [("area=3", 2.030_202), ("m=3", 2.002_458)] {
            let got = z(size);
            assert!(
                (got - ngspice).abs() < 1e-4,
                "V(z) {size}: ngspice {ngspice}, got {got}"
            );
        }

        // Charge scales with area too. Away from breakdown, area 3 and m=3
        // differ only in GMIN (1 pS against 3 pS), so a supply step must
        // track to well under a microvolt, step for step.
        let pulse = "PULSE(0 3.3 0 20n 20n 1u 2u)";
        let mut area = build(&deck(pulse, "area=3"));
        let mut multiplied = build(&deck(pulse, "m=3"));
        let mut one = build(&deck(pulse, ""));
        let mut largest_difference: f64 = 0.0;
        for _ in 0..40 {
            for solver in [&mut area, &mut multiplied, &mut one] {
                solver.advance(5e-9).expect("transient step solves");
            }
            let (a, m, o) = (volts(&area), volts(&multiplied), volts(&one));
            for index in 0..nodes.len() {
                largest_difference = largest_difference.max((a[index] - o[index]).abs());
                if nodes[index] == "z" {
                    continue;
                }
                assert!(
                    (a[index] - m[index]).abs() < 1e-6,
                    "V({}) at {} s: area=3 gives {}, m=3 gives {}",
                    nodes[index],
                    area.time(),
                    a[index],
                    m[index]
                );
            }
        }
        assert!(
            largest_difference > 0.1,
            "area=3 must load the circuit differently from area 1, max diff {largest_difference} V"
        );

        for value in ["area=0", "area=-2", "area=1e999", "-1"] {
            let error = parse_netlist(&format!("D1 a 0 DR {value}\n{models}"))
                .expect_err("a non-positive area must be refused")
                .to_string();
            assert!(error.contains("area"), "{value}: unexpected error {error}");
        }
        let error = parse_netlist("M1 d g 0 0 MR area=2\n.model MR NMOS(LEVEL=1 VTO=1 KP=20u)\n")
            .expect_err("a level-1 MOSFET has no area; W and L size it")
            .to_string();
        assert!(error.contains("unknown parameter `area`"), "{error}");
    }

    /// Reverse-biased pn junctions follow ngspice-42's per-device laws, not
    /// the plain exponential. A diode without `BV` and a JFET gate take the
    /// cubic `−IS·(1 + (3·N·Vt/(e·V))³)` below `−3·N·Vt`; a sidewall with its
    /// own `NS` switches at `−3·NS·Vt`; a level-1 MOSFET bulk junction holds
    /// `−IS` with only `GMIN` conducting at or below `−3·Vt`. Each device
    /// has its own source. References are ngspice-42 `.dc` with `.temp
    /// 26.85` and `tnom=26.85` (decks: `D1 d 0 DM`, `D2 s 0 DSW pj=1`,
    /// `J1 0 jg 0 JM`, `M1 0 0 0 mb MM`, swept together).
    #[test]
    fn reverse_biased_junctions_follow_ngspice_reverse_laws() {
        let models = ".model DM D(IS=1e-9 N=1.5)\n\
            .model DSW D(IS=1e-12 JSW=4e-10 NS=2.0)\n\
            .model JM NJF(VTO=-2 BETA=1e-3 IS=1e-9)\n\
            .model MM NMOS(LEVEL=1 VTO=1 KP=1e-4 IS=1e-9)\n";
        // (V, [diode, sidewall diode, JFET gate, MOSFET bulk]) in amps.
        let ngspice: [(f64, [f64; 4]); 7] = [
            (
                -1.0,
                [
                    -1.000_921_614_4e-9,
                    -4.019_256_556_5e-10,
                    -2.001_953_549_3e-9,
                    -2.002e-9,
                ],
            ),
            (
                -0.5,
                [
                    -9.998_729_154_9e-10,
                    -4.009_052_451_8e-10,
                    -2.000_628_394_4e-9,
                    -2.001e-9,
                ],
            ),
            (
                -0.3,
                [
                    -9.973_968_309_8e-10,
                    -3.985_465_054_8e-10,
                    -1.998_879_603_5e-9,
                    -2.0006e-9,
                ],
            ),
            (
                -0.2,
                [
                    -9.904_018_045_7e-10,
                    -3.919_069_559_8e-10,
                    -1.994_593_662_0e-9,
                    -2.0004e-9,
                ],
            ),
            (
                -0.15,
                [
                    -9.769_246_478_7e-10,
                    -3.791_586_981_6e-10,
                    -1.986_536_828_4e-9,
                    -2.0003e-9,
                ],
            ),
            (
                -0.1,
                [
                    -9.242_336_986_1e-10,
                    -3.432_542_998_7e-10,
                    -1.953_749_295_7e-9,
                    -2.0002e-9,
                ],
            ),
            (
                -0.05,
                [
                    -7.246_116_196_1e-10,
                    -2.488_232_623_8e-10,
                    -1.710_987_626_1e-9,
                    -1.710_987_626_1e-9,
                ],
            ),
        ];
        let sources = ["Vd", "Vs", "Vj", "Vm"];
        let mut exponential_gap = [0.0_f64; 4];
        for (voltage, expected) in ngspice {
            let solver = build(&format!(
                "Vd d 0 dc {voltage}\nVs s 0 dc {voltage}\nVj jg 0 dc {voltage}\nVm mb 0 dc {voltage}\n\
                 D1 d 0 DM\nD2 s 0 DSW pj=1\nJ1 0 jg 0 JM\nM1 0 0 0 mb MM\n{models}"
            ));
            for ((source, reference), gap) in sources.iter().zip(expected).zip(&mut exponential_gap)
            {
                let got = source_current(&solver, source);
                assert!(
                    relative(got, reference) < 2e-5,
                    "{source} at {voltage} V: ngspice-42 {reference} A, got {got} A"
                );
                // The plain exponential every junction used before.
                let exponential = match *source {
                    "Vd" => device::diode_op(voltage, 1e-9, 1.5).id,
                    "Vs" => {
                        device::diode_op(voltage, 1e-12, 1.0).id
                            + device::diode_op(voltage, 4e-10, 2.0).id
                            - device::GMIN * voltage
                    }
                    _ => 2.0 * device::diode_op(voltage, 1e-9, 1.0).id,
                };
                *gap = gap.max(relative(exponential, reference));
            }
        }
        // The exponential misses each reference by 0.4 % or more.
        for (source, gap) in sources.iter().zip(exponential_gap) {
            assert!(
                gap > 2e-3,
                "{source}: the sweep must reach the reverse law, gap {gap}"
            );
        }

        // The cubic meets the exponential in value at −3·N·Vt, and the
        // MOSFET hold conducts only GMIN.
        let boundary = -3.0 * 1.5 * device::THERMAL_VOLTAGE;
        let (forward, reverse) = (
            device::pn_junction_op(boundary, 1e-9, 1.5),
            device::pn_junction_op(boundary - 1e-12, 1e-9, 1.5),
        );
        assert!(relative(reverse.id, forward.id) < 1e-9);
        let bulk = device::mos_bulk_junction_op(-0.2, 1e-9, device::THERMAL_VOLTAGE);
        assert_eq!(bulk.gd, device::GMIN);
        assert_eq!(bulk.id, -1e-9 - device::GMIN * 0.2);
    }
}
