// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Current probes on analog LEDs and diodes must observe the solved device,
//! including its internal series resistance and reverse-breakdown region.

mod analog {
    use labwired_core::analog::{AnalogConfig, AnalogCosimAdapter};
    use labwired_core::cosim::{CosimAdapter, CosimSignalValue, CosimStep};
    use std::collections::BTreeMap;

    fn config() -> AnalogConfig {
        AnalogConfig {
            probes: BTreeMap::from([
                ("led_d4".into(), "i(Dd4)".into()),
                ("source".into(), "i(Vdrive)".into()),
            ]),
            sources: BTreeMap::from([("drive".into(), "Vdrive".into())]),
            trace: vec!["I( dD4 )".into()],
            ..AnalogConfig::default()
        }
    }

    fn currents(adapter: &mut AnalogCosimAdapter, time_ns: u64, volts: f64) -> (f64, f64) {
        let result = adapter
            .step(CosimStep {
                time_ns,
                dt_ns: 1_000_000,
                inputs: BTreeMap::from([("drive".into(), CosimSignalValue::F64(volts))]),
            })
            .expect("the analog step solves");
        let amperes = |name: &str| match result.outputs[name] {
            CosimSignalValue::F64(value) => value,
            ref other => panic!("{name} must be a numeric current, got {other:?}"),
        };
        (amperes("led_d4"), amperes("source"))
    }

    fn assert_kcl(diode: f64, source: f64) {
        assert!(
            (diode + source).abs() < 1e-10,
            "anode-to-cathode current {diode} A must balance the source branch {source} A"
        );
    }

    #[test]
    fn emitted_led_current_probe_tracks_the_actual_driven_circuit() {
        // This is the i(Dd4) expression emitted for an LED inside an analog
        // island. The capacitor keeps the fixture a genuinely reactive island.
        let netlist = "Vdrive in 0 dc 0\nR1 in led 330\nDd4 led 0 D\nC1 led 0 10n\n";
        let mut adapter = AnalogCosimAdapter::from_netlist(netlist, &config())
            .expect("the emitted LED current probe must build");
        let (off, source) = currents(&mut adapter, 0, 0.0);
        assert_eq!(off, 0.0);
        assert_kcl(off, source);

        // During charging, the resistor/source current mostly enters C1. A
        // diode probe must not report that total supply current as LED current.
        let (charging, source) = currents(&mut adapter, 100, 3.3);
        assert!(
            (0.0..1e-6).contains(&charging),
            "early LED current {charging} A"
        );
        assert!(
            (charging + source).abs() > 0.005,
            "capacitor charging current is missing"
        );

        let (on, source) = currents(&mut adapter, 1_000_000, 3.3);
        assert!(
            (0.005..0.010).contains(&on),
            "the 330-ohm LED carries {on} A"
        );
        assert_kcl(on, source);
        let led = adapter.solver().circuit().node("led").unwrap();
        let resistor = (3.3 - adapter.solver().node_voltage(led)) / 330.0;
        assert!((on - resistor).abs() < 1e-10);

        let (off_again, source) = currents(&mut adapter, 2_000_000, 0.0);
        assert!(
            off_again.abs() < 1e-10,
            "turning the drive off leaves {off_again} A"
        );
        assert_kcl(off_again, source);

        let trace = adapter.trace_handle();
        let trace = trace.lock().unwrap();
        let series = trace.series();
        let observed = series.channels.iter().find(|c| c.name == "led_d4").unwrap();
        let alias = series
            .channels
            .iter()
            .find(|c| c.name == "I( dD4 )")
            .unwrap();
        assert_eq!(observed.unit, "A");
        assert_eq!(
            observed.points, alias.points,
            "probe names are case-insensitive"
        );
        assert!(observed.points.iter().any(|(_, current)| *current > 0.005));
    }

    #[test]
    fn diode_current_uses_the_solved_junction_behind_series_resistance() {
        // Pick a current, invert Shockley, then add the independently known
        // resistor drops. Using the external anode voltage in Shockley would
        // wildly overstate the current when RS is nonzero.
        let target = 0.01_f64;
        let saturation = 1e-14;
        let thermal_voltage = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;
        let junction = thermal_voltage * (target / saturation + 1.0).ln();
        for series_resistance in [0.0, 22.0] {
            let drive = junction + target * (1000.0 + series_resistance);
            let netlist = format!(
                "Vdrive in 0 dc {drive}\nR1 in led 1000\nDd4 led 0 DMOD\n\
                 .model DMOD D(IS={saturation} N=1 RS={series_resistance})\n"
            );
            let mut adapter = AnalogCosimAdapter::from_netlist(&netlist, &config()).unwrap();
            let (current, source) = currents(&mut adapter, 0, drive);
            assert!(
                (current - target).abs() < target * 1e-6,
                "RS={series_resistance}: {current} A"
            );
            assert_kcl(current, source);
        }
    }

    #[test]
    fn reverse_current_retains_its_sign_and_leakage() {
        let mut adapter = AnalogCosimAdapter::from_netlist(
            "Vdrive in 0 dc -5\nR1 in led 1000\nDd4 led 0 D\n",
            &config(),
        )
        .unwrap();
        let (reverse, source) = currents(&mut adapter, 0, -5.0);
        // Built-in D: IS=2.52nA; solver's 1pS GMIN adds about 5pA at -5V.
        assert!(
            (-2.53e-9..-2.52e-9).contains(&reverse),
            "reverse current {reverse} A"
        );
        assert_kcl(reverse, source);
    }

    #[test]
    fn reverse_breakdown_current_matches_the_solved_source_branch() {
        for (series_resistance, breakdown_emission) in [(0.0, ""), (6.0, "NBV=2.4")] {
            let netlist = format!(
                "Vdrive in 0 dc -10\nR1 in led 1000\nDd4 led 0 DZ\n\
                 .model DZ D(IS=1e-11 N=1.2 RS={series_resistance} BV=4.8 IBV=49m {breakdown_emission})\n"
            );
            let mut adapter = AnalogCosimAdapter::from_netlist(&netlist, &config()).unwrap();
            let (reverse, source) = currents(&mut adapter, 0, -10.0);
            assert!(
                (-0.006..-0.004).contains(&reverse),
                "breakdown current {reverse} A"
            );
            assert_kcl(reverse, source);
        }
    }

    #[test]
    fn unknown_and_unsupported_current_probes_still_fail_loudly() {
        for expression in ["i(Dmissing)", "i(R1)"] {
            let mut cfg = config();
            cfg.probes.insert("bad".into(), expression.into());
            let err = AnalogCosimAdapter::from_netlist(
                "Vdrive in 0 dc 3.3\nR1 in led 330\nDd4 led 0 D\n",
                &cfg,
            )
            .unwrap_err();
            assert!(err.to_string().contains(expression), "{err}");
        }
    }
}
