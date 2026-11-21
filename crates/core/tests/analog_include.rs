// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Model libraries in the in-core engine: `.include <file>` and
//! `.lib <file> <section>`.
//!
//! Vendors ship models as files: a corner library with one `.lib` section
//! per process corner, a part file that `.include`s its card from another
//! file. The decks below are the exact text ngspice-42 ran from a directory
//! holding the same files (its `.control` block is skipped here), with
//! `.temp 27` and `reltol=1e-12 vntol=1e-15 abstol=1e-18` so ngspice's own
//! answer is converged past the tolerance asserted. Two unrelated devices
//! come in through the libraries: a diode card through `.lib` and an NPN
//! card through a nested `.include`.

mod analog {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use labwired_core::analog::{
        parse_netlist, parse_netlist_with_files, AnalogError, Integration, Solver,
    };

    /// `lib/diodes.lib`: one diode card per corner.
    const DIODES_LIB: &str = "\
* diode corner library
.lib tt
.model DX D(IS=2.5n N=1.75 RS=10)
.endl tt
.lib ff
.model DX D(IS=10n N=1.6 RS=5)
.endl ff
";

    /// `lib/npn.inc`: a part file that pulls its card from a nested file,
    /// relative to its own directory, and ends with `.end`.
    const NPN_INC: &str = "\
* vendor NPN, card in a nested file
.include models/qv.mod
.end
";

    /// `lib/models/qv.mod`: the card, split over a `+` line.
    const QV_MOD: &str = "\
.model QV NPN(IS=1e-15 BF=150 VAF=80
+ RB=50 RC=2)
";

    fn deck(corner: &str) -> String {
        format!(
            "\
* vendor library corners
.temp 27
.options reltol=1e-12 vntol=1e-15 abstol=1e-18
.lib lib/diodes.lib {corner}
.include lib/npn.inc
V1 in 0 5
R1 in a 10k
D1 a 0 DX
V2 vcc 0 5
R2 vcc c 1k
RB in b 470k
Q1 c b 0 QV
.control
op
set numdgt=12
print v(a) v(b) v(c) i(v1) i(v2)
.endc
.end
"
        )
    }

    /// The deck with both cards written in place, for the bit-identity check.
    const TT_LITERAL: &str = "\
.temp 27
.options reltol=1e-12 vntol=1e-15 abstol=1e-18
.model DX D(IS=2.5n N=1.75 RS=10)
.model QV NPN(IS=1e-15 BF=150 VAF=80 RB=50 RC=2)
V1 in 0 5
R1 in a 10k
D1 a 0 DX
V2 vcc 0 5
R2 vcc c 1k
RB in b 470k
Q1 c b 0 QV
.end
";

    /// In-memory files under `/proj`, and the paths the parser asked for.
    struct Files {
        files: BTreeMap<PathBuf, String>,
        asked: Vec<PathBuf>,
    }

    impl Files {
        fn vendor() -> Self {
            let files = [
                ("/proj/lib/diodes.lib", DIODES_LIB),
                ("/proj/lib/npn.inc", NPN_INC),
                ("/proj/lib/models/qv.mod", QV_MOD),
            ]
            .into_iter()
            .map(|(path, text)| (PathBuf::from(path), text.to_string()))
            .collect();
            Self {
                files,
                asked: Vec::new(),
            }
        }

        fn with(mut self, path: &str, text: &str) -> Self {
            self.files.insert(PathBuf::from(path), text.to_string());
            self
        }

        fn parse(&mut self, text: &str) -> Result<Solver, AnalogError> {
            let mut read = |path: &Path| {
                self.asked.push(path.to_path_buf());
                self.files
                    .get(path)
                    .cloned()
                    .ok_or_else(|| "no such file".to_string())
            };
            let circuit = parse_netlist_with_files(text, Path::new("/proj"), &mut read)?;
            Solver::new(circuit, Integration::BackwardEuler)
        }
    }

    fn build(text: &str) -> Solver {
        Files::vendor()
            .parse(text)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    fn voltage(solver: &Solver, node: &str) -> f64 {
        let node = solver
            .circuit()
            .node(node)
            .unwrap_or_else(|| panic!("node `{node}` exists"));
        solver.node_voltage(node)
    }

    fn branch(solver: &Solver, name: &str) -> f64 {
        solver.branch_current(
            solver
                .circuit()
                .branch_index(name)
                .unwrap_or_else(|| panic!("branch `{name}` exists")),
        )
    }

    fn assert_close(what: &str, got: f64, reference: f64, tolerance: f64) {
        let scale = reference.abs().max(1e-9);
        assert!(
            ((got - reference) / scale).abs() < tolerance,
            "{what}: ngspice-42 {reference:e}, got {got:e}"
        );
    }

    /// The BJT stage reads its card through `npn.inc` -> `models/qv.mod`,
    /// so the nested path must resolve against `lib/`, not the deck. Its
    /// answer does not depend on the diode corner.
    fn assert_npn_stage(solver: &Solver) {
        assert_close("v(b)", voltage(solver, "b"), 7.231_759_746_004e-1, 1e-6);
        assert_close("v(c)", voltage(solver, "c"), 3.586_247_160_712, 1e-6);
        assert_close("i(v2)", branch(solver, "V2"), -1.413_752_839_29e-3, 1e-6);
    }

    /// Each `.lib` section is its own diode card. Matches ngspice-42 `.op`
    /// for both corners; fails if the section is ignored (no card), if the
    /// wrong section wins, or if both sections are read (duplicate card).
    #[test]
    fn lib_sections_select_the_corner_like_ngspice() {
        let tt = build(&deck("tt"));
        assert_close("tt v(a)", voltage(&tt, "a"), 5.516_479_483_387e-1, 1e-6);
        assert_close("tt i(v1)", branch(&tt, "V1"), -4.539_348_307_52e-4, 1e-6);
        assert_npn_stage(&tt);

        let ff = build(&deck("FF"));
        assert_close("ff v(a)", voltage(&ff, "a"), 4.461_739_337_074e-1, 1e-6);
        assert_close("ff i(v1)", branch(&ff, "V1"), -4.644_822_322_15e-4, 1e-6);
        assert_npn_stage(&ff);
    }

    /// Splicing is a pure rewrite: the library deck and its hand-written
    /// twin solve to identical bits.
    #[test]
    fn library_deck_is_bit_identical_to_its_literal_twin() {
        let library = build(&deck("tt"));
        let parsed = parse_netlist(TT_LITERAL).unwrap();
        let literal = Solver::new(parsed, Integration::BackwardEuler).unwrap();
        for node in ["in", "a", "vcc", "b", "c"] {
            assert_eq!(
                voltage(&library, node).to_bits(),
                voltage(&literal, node).to_bits(),
                "v({node})"
            );
        }
    }

    /// Paths resolve against the file that names them, as ngspice resolves
    /// them; quotes are removed.
    #[test]
    fn nested_paths_resolve_against_the_including_file() {
        let mut files = Files::vendor();
        let text = deck("tt").replace(".include lib/npn.inc", ".include \"lib/npn.inc\"");
        files.parse(&text).unwrap();
        assert_eq!(
            files.asked,
            [
                PathBuf::from("/proj/lib/diodes.lib"),
                PathBuf::from("/proj/lib/npn.inc"),
                PathBuf::from("/proj/lib/models/qv.mod"),
            ]
        );
    }

    /// ngspice drops a `.end` inside an included file and keeps reading it:
    /// `R9` after the `.end` loads `a` (checked on ngspice-42).
    #[test]
    fn end_inside_an_included_file_is_dropped_like_ngspice() {
        let mut files = Files::vendor().with("/proj/load.inc", ".end\nR9 a 0 1k\n");
        let solver = files
            .parse("V1 in 0 2\nR1 in a 1k\n.include load.inc\n.end\n")
            .unwrap();
        assert_close("v(a)", voltage(&solver, "a"), 1.0, 1e-12);
    }

    /// A library section can call another library and include a file.
    #[test]
    fn a_section_can_call_another_library() {
        let mut files = Files::vendor().with(
            "/proj/lib/kit.lib",
            ".lib typ\n.lib diodes.lib tt\n.include npn.inc\n.endl\n",
        );
        let text = deck("tt")
            .replace(".lib lib/diodes.lib tt\n", "")
            .replace(".include lib/npn.inc", ".lib lib/kit.lib typ");
        let solver = files.parse(&text).unwrap();
        assert_close("v(a)", voltage(&solver, "a"), 5.516_479_483_387e-1, 1e-6);
        assert_npn_stage(&solver);
    }

    fn parse_error(files: &mut Files, text: &str) -> (Option<usize>, String) {
        let err = files.parse(text).expect_err("deck must be refused");
        (err.line(), err.to_string())
    }

    /// What is not a valid library call is refused by name, at the deck line
    /// that made the call.
    #[test]
    fn bad_library_calls_are_refused_at_the_calling_line() {
        let mut files = Files::vendor()
            .with("/proj/a.inc", ".include b.inc\n")
            .with("/proj/b.inc", ".include a.inc\n")
            .with("/proj/bad.inc", "R1 a 0 1k\nR2 a 0 oops\n");

        let (line, message) = parse_error(&mut files, "V1 a 0 1\n.lib lib/diodes.lib ss\n");
        assert_eq!(line, Some(2));
        assert!(message.contains("has no section `ss`"), "{message}");

        let (line, message) = parse_error(&mut files, "V1 a 0 1\n.include missing.inc\n");
        assert_eq!(line, Some(2));
        assert!(
            message.contains("cannot read `/proj/missing.inc`"),
            "{message}"
        );

        let (line, message) = parse_error(&mut files, "* t\nV1 a 0 1\n.include a.inc\n");
        assert_eq!(line, Some(3));
        assert!(message.contains("includes itself"), "{message}");

        // A bad value inside a file: the deck's line, the file's own line.
        let (line, message) = parse_error(&mut files, "V1 a 0 1\n.include bad.inc\n.end\n");
        assert_eq!(line, Some(2));
        assert!(message.contains("R2 a 0 oops"), "{message}");

        let (_, message) = parse_error(&mut files, ".include\n");
        assert!(message.contains("expected `.include <file>`"), "{message}");
    }

    /// Without a file reader the calls stay refused and point at ngspice.
    #[test]
    fn parse_netlist_without_files_still_refuses_library_calls() {
        let err = parse_netlist(&deck("tt")).expect_err("no reader");
        assert_eq!(err.line(), Some(4));
        assert!(err.to_string().contains("needs ngspice"), "{err}");
    }
}

/// The co-simulation adapter reads the libraries from disk, relative to the
/// netlist file, and its probes report the library's physics.
mod adapter {
    use std::collections::HashMap;
    use std::fs;

    use labwired_config::{CosimAdapter as ManifestCosimAdapter, CosimModelConfig};
    use labwired_core::analog::adapter::AnalogCosimAdapter;
    use labwired_core::cosim::{CosimAdapter, CosimSignalValue, CosimStep};

    #[test]
    fn a_manifest_netlist_reads_its_library_from_its_own_directory() {
        let root =
            std::env::temp_dir().join(format!("labwired-analog-include-{}", std::process::id()));
        let circuit = root.join("circuit");
        fs::create_dir_all(circuit.join("lib")).unwrap();
        fs::write(
            circuit.join("lib/diodes.lib"),
            ".lib ff\n.model DX D(IS=10n N=1.6 RS=5)\n.endl\n",
        )
        .unwrap();
        fs::write(
            circuit.join("diode.cir"),
            "* clamp\n.temp 27\n.lib lib/diodes.lib ff\nV1 in 0 5\nR1 in a 10k\nD1 a 0 DX\n.end\n",
        )
        .unwrap();

        let config = CosimModelConfig {
            id: "clamp".to_string(),
            adapter: ManifestCosimAdapter::Analog,
            model: None,
            step_ns: 1_000,
            inputs: HashMap::new(),
            outputs: HashMap::new(),
            config: HashMap::from([
                (
                    "netlist".to_string(),
                    serde_yaml::Value::from("circuit/diode.cir"),
                ),
                (
                    "probes".to_string(),
                    serde_yaml::from_str("{ v_a: \"v(a)\" }").unwrap(),
                ),
            ]),
        };
        let built = AnalogCosimAdapter::from_manifest_config(&config, &root);
        fs::remove_dir_all(&root).unwrap();
        let mut adapter = built.unwrap_or_else(|error| panic!("{error}"));
        let result = adapter
            .step(CosimStep {
                time_ns: 1_000,
                dt_ns: 1_000,
                inputs: Default::default(),
            })
            .unwrap();
        let CosimSignalValue::F64(v_a) = result.outputs["v_a"] else {
            panic!("v_a is a number: {:?}", result.outputs["v_a"]);
        };
        // ngspice-42 `.op`, ff corner.
        assert!((v_a - 4.461_739_337_074e-1).abs() < 1e-6, "v(a) = {v_a}");
    }
}
