//! Explicit bounded MNA capacity changes admission, never the equations.
mod analog {
    use labwired_config::{ChipDescriptor, CosimModelConfig, EnvironmentManifest, SystemManifest};
    use labwired_core::analog::{
        parse_netlist, AnalogConfig, AnalogCosimAdapter, AnalogError, Integration, Solver,
        HARD_MAX_UNKNOWNS, MAX_UNKNOWNS,
    };
    use labwired_core::cosim::{build_cosim_adapter_with_base, validate_analog_models};
    use labwired_core::system::node::NodeFirmware;
    use labwired_core::world::{ResolvedWorldNode, World};
    use std::collections::HashMap;
    use std::path::Path;

    fn ladder(unknowns: usize) -> String {
        let nodes = unknowns - 1;
        let mut deck = String::from("* bounded ladder\nV1 n0 0 dc 1\n");
        for i in 0..nodes - 1 {
            deck.push_str(&format!("R{i} n{i} n{} 1k\n", i + 1));
        }
        deck.push_str(&format!("Rlast n{} 0 1k\n.end\n", nodes - 1));
        deck
    }

    fn model(unknowns: usize, limit: Option<serde_yaml::Value>) -> CosimModelConfig {
        let mut model: CosimModelConfig =
            serde_yaml::from_str("id: capacity\nadapter: analog\nstep_ns: 100\nconfig: {}\n")
                .unwrap();
        model.config.insert(
            "netlist_text".into(),
            serde_yaml::Value::String(ladder(unknowns)),
        );
        model.config.insert(
            "probes".into(),
            serde_yaml::to_value(HashMap::from([(
                "voltage",
                format!("v(n{})", unknowns - 2),
            )]))
            .unwrap(),
        );
        model
            .outputs
            .insert("voltage".into(), "node.a.adc.adc1.0_volts".into());
        if let Some(limit) = limit {
            model.config.insert("max_unknowns".into(), limit);
        }
        model
    }

    #[test]
    fn analog_capacity_default_still_refuses_129_unknowns() {
        assert_eq!(MAX_UNKNOWNS, 128);
        assert_eq!(AnalogConfig::default().max_unknowns, 128);
        let circuit = parse_netlist(&ladder(129)).unwrap();
        assert_eq!(circuit.unknowns(), 129);
        assert!(matches!(
            Solver::new(circuit, Integration::BackwardEuler),
            Err(AnalogError::TooLarge {
                unknowns: 129,
                max: 128
            })
        ));
        assert!(matches!(
            AnalogCosimAdapter::from_netlist(&ladder(129), &AnalogConfig::default()),
            Err(AnalogError::TooLarge {
                unknowns: 129,
                max: 128
            })
        ));
    }

    #[test]
    fn analog_capacity_explicit_379_and_512_solve_complete_ladders() {
        assert_eq!(HARD_MAX_UNKNOWNS, 512);
        for unknowns in [379, 512] {
            let circuit = parse_netlist(&ladder(unknowns)).unwrap();
            assert_eq!(circuit.unknowns(), unknowns);
            let mut solver =
                Solver::new_with_max_unknowns(circuit, Integration::BackwardEuler, unknowns)
                    .unwrap();
            solver.advance(1e-7).unwrap();
            let nodes = unknowns - 1;
            for i in 0..nodes {
                let node = solver.circuit().node(&format!("n{i}")).unwrap();
                let expected = (nodes - i) as f64 / nodes as f64;
                assert!(
                    (solver.node_voltage(node) - expected).abs() < 1e-9,
                    "unknowns={unknowns}, node={i}"
                );
            }
        }
    }

    #[test]
    fn analog_capacity_rejects_above_selected_and_absolute_bounds() {
        let circuit = parse_netlist(&ladder(513)).unwrap();
        assert!(matches!(
            Solver::new_with_max_unknowns(circuit.clone(), Integration::BackwardEuler, 512),
            Err(AnalogError::TooLarge {
                unknowns: 513,
                max: 512
            })
        ));
        for max in [0, 513, usize::MAX] {
            // Invalid configured bounds fail before any solve/allocation path.
            assert!(matches!(
                Solver::new_with_max_unknowns(circuit.clone(), Integration::BackwardEuler, max),
                Err(AnalogError::Config(_))
            ));
            let config = AnalogConfig {
                max_unknowns: max,
                ..Default::default()
            };
            assert!(matches!(
                AnalogCosimAdapter::from_netlist(&ladder(3), &config),
                Err(AnalogError::Config(_))
            ));
        }
        let circuit = parse_netlist(&ladder(4)).unwrap();
        assert!(matches!(
            Solver::new_with_max_unknowns(circuit, Integration::BackwardEuler, 3),
            Err(AnalogError::TooLarge {
                unknowns: 4,
                max: 3
            })
        ));
    }

    #[test]
    fn analog_capacity_yaml_is_a_strict_bounded_integer() {
        for valid in [1, 128, 379, 512] {
            let map = HashMap::from([("max_unknowns".into(), serde_yaml::Value::from(valid))]);
            assert_eq!(
                AnalogConfig::from_yaml(&map).unwrap().max_unknowns,
                valid as usize
            );
        }
        for raw in [
            "0",
            "-1",
            "513",
            "1.5",
            "512.0",
            ".nan",
            ".inf",
            "-.inf",
            "null",
            "true",
            "'512'",
            "[]",
            "{}",
            "18446744073709551615",
        ] {
            let value = serde_yaml::from_str(raw).unwrap();
            let map = HashMap::from([("max_unknowns".into(), value)]);
            let err = AnalogConfig::from_yaml(&map).expect_err(raw);
            assert!(err.to_string().contains("max_unknowns"), "{raw}: {err}");
        }
    }

    #[test]
    fn analog_capacity_explicit_limit_preserves_old_transient_bits() {
        let deck = "* rc\nV1 in 0 dc 0\nR1 in out 10k\nC1 out 0 100n\n.end\n";
        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let circuit = parse_netlist(deck).unwrap();
            let node = circuit.node("out").unwrap();
            let mut original = Solver::new(circuit.clone(), integration).unwrap();
            let mut opt_in = Solver::new_with_max_unknowns(circuit, integration, 512).unwrap();
            for i in 0..100 {
                let volts = if i < 50 { 3.3 } else { 0.0 };
                original.set_voltage_source(0, volts);
                opt_in.set_voltage_source(0, volts);
                original.advance(1e-5).unwrap();
                opt_in.advance(1e-5).unwrap();
                assert_eq!(
                    original.node_voltage(node).to_bits(),
                    opt_in.node_voltage(node).to_bits()
                );
                assert_eq!(
                    original.branch_current(0).to_bits(),
                    opt_in.branch_current(0).to_bits()
                );
            }
        }
    }

    #[test]
    fn analog_capacity_manifest_validation_and_construction_agree() {
        for (unknowns, max, accept) in [
            (129, None, false),
            (379, Some(512), true),
            (512, Some(512), true),
            (513, Some(512), false),
        ] {
            let model = model(unknowns, max.map(serde_yaml::Value::from));
            assert_eq!(
                validate_analog_models(std::slice::from_ref(&model), Path::new(".")).is_empty(),
                accept
            );
            assert_eq!(
                build_cosim_adapter_with_base(&model, Path::new(".")).is_ok(),
                accept
            );
        }
        for raw in ["0", "513", "1.5", ".nan", ".inf", "null", "'512'"] {
            let model = model(3, Some(serde_yaml::from_str(raw).unwrap()));
            assert!(
                !validate_analog_models(std::slice::from_ref(&model), Path::new(".")).is_empty(),
                "{raw}"
            );
            assert!(
                build_cosim_adapter_with_base(&model, Path::new(".")).is_err(),
                "{raw}"
            );
        }
    }

    fn world(model: CosimModelConfig) -> anyhow::Result<World> {
        let mut manifest: EnvironmentManifest = serde_yaml::from_str(
            "name: capacity\nnodes:\n  - {id: a, system: a.yaml, firmware: a.elf}\n",
        )?;
        manifest.cosim_models = vec![model];
        let chip: ChipDescriptor =
            serde_yaml::from_str(include_str!("../../../configs/chips/stm32f401.yaml"))?;
        let system: SystemManifest =
            serde_yaml::from_str("name: capacity-node\nchip: stm32f401\n")?;
        let node = ResolvedWorldNode {
            id: "a".into(),
            chip,
            system,
            firmware: NodeFirmware::from_bytes(
                include_bytes!("../../../tests/fixtures/stm32f401-blinky.elf").to_vec(),
            ),
            blobs: Default::default(),
        };
        World::from_resolved(manifest, vec![node])
    }

    #[test]
    fn analog_capacity_shared_world_uses_the_same_bound() {
        // Only construction is tested; no new MCU behavior is assumed here.
        assert!(world(model(129, None)).is_err());
        let accepted = world(model(379, Some(serde_yaml::Value::from(512)))).unwrap();
        assert!(accepted.cosim_status().is_some());
        for raw in ["513", "1.5", ".nan", "null"] {
            assert!(
                world(model(3, Some(serde_yaml::from_str(raw).unwrap()))).is_err(),
                "{raw}"
            );
        }
    }
}
