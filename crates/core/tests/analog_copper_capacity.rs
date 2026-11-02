// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Routed copper adds real resistor terminals to an otherwise supported circuit.
//! Keep those terminals and verify the resulting circuit instead of idealizing it.

mod analog {
    use labwired_core::analog::{parse_netlist, Integration, Solver};

    const SOURCE: &str = include_str!("fixtures/analog/acid303_source.cir");
    const COPPER: &str = include_str!("fixtures/analog/acid303_copper.cir");
    const DAC_PINS: [&str; 8] = ["pb0", "pb1", "pb3", "pb4", "pb5", "pb6", "pb7", "pb10"];

    fn dac_readings(deck: &str, output_name: &str, expected_unknowns: usize) -> Vec<f64> {
        let circuit = parse_netlist(deck).expect("the compiled diagram parses");
        assert_eq!(circuit.unknowns(), expected_unknowns);
        let output = circuit.node(output_name).expect("ADC sense node exists");
        let drivers: Vec<(usize, usize)> = DAC_PINS
            .iter()
            .map(|pin| {
                let source = circuit
                    .voltage_sources
                    .iter()
                    .position(|source| source.name.eq_ignore_ascii_case(&format!("Vdrv_{pin}")))
                    .unwrap();
                let switch = circuit
                    .switches
                    .iter()
                    .position(|switch| switch.name.eq_ignore_ascii_case(&format!("Sdrv_{pin}")))
                    .unwrap();
                (source, switch)
            })
            .collect();
        let mut solver = Solver::new(circuit, Integration::BackwardEuler)
            .expect("the complete extracted circuit fits the bounded solver");
        let mut readings = Vec::new();
        for code in [32_u8, 128, 224] {
            for (bit, &(source, switch)) in drivers.iter().enumerate() {
                let volts = if code & (1 << bit) != 0 { 3.3 } else { 0.0 };
                solver.set_voltage_source(source, volts);
                solver.set_switch(switch, true);
            }
            for _ in 0..500 {
                solver
                    .advance(1e-5)
                    .expect("the nonlinear copper circuit converges");
            }
            let volts = solver.node_voltage(output);
            let ideal_dac = 3.3 * f64::from(code) / 256.0;
            assert!(
                (volts - ideal_dac).abs() < 0.003,
                "code {code}: {volts} V should follow the R-2R transfer {ideal_dac} V"
            );
            readings.push(volts);
        }
        readings
    }

    #[test]
    fn routed_dac_keeps_all_copper_resistors_and_its_transfer() {
        let copper = parse_netlist(COPPER).unwrap();
        assert_eq!(
            copper
                .resistors
                .iter()
                .filter(|r| r.name.starts_with("Rcu_r"))
                .count(),
            67,
            "do not remove parasitics to fit the old capacity limit"
        );
        let source = dac_readings(SOURCE, "v_net_c_line_1", 48);
        let copper = dac_readings(COPPER, "v_net_cu_r32_2", 101);
        for (source, copper) in source.into_iter().zip(copper) {
            assert!(
                (source - copper).abs() < 25e-6,
                "the extracted copper drop must stay physical: source={source} V, copper={copper} V"
            );
        }
    }

    #[test]
    fn a_large_rc_network_tracks_the_analytic_step_response() {
        // 96 independent RC outputs plus the driven node and source branch:
        // 98 unknowns. Each output still has the closed-form one-pole response.
        let mut deck = String::from("V1 in 0 dc 0\n");
        for index in 0..96 {
            deck.push_str(&format!(
                "R{index} in n{index} 1k\nC{index} n{index} 0 100n\n"
            ));
        }
        let circuit = parse_netlist(&deck).unwrap();
        assert_eq!(circuit.unknowns(), 98);
        let outputs: Vec<_> = (0..96)
            .map(|i| circuit.node(&format!("n{i}")).unwrap())
            .collect();
        let mut solver = Solver::new(circuit, Integration::Trapezoidal).unwrap();
        solver.set_voltage_source(0, 3.3);
        for _ in 0..1000 {
            solver.advance(1e-7).unwrap();
        }
        let expected = 3.3 * (1.0 - (-1.0_f64).exp());
        for output in outputs {
            let measured = solver.node_voltage(output);
            assert!(
                (measured - expected).abs() < 1e-5,
                "one time constant: {measured} V, expected {expected} V"
            );
        }
    }
}
