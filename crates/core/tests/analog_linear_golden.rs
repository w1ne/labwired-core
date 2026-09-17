// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The linear engine has not moved by one bit.
//!
//! Newton iteration, the device stamps and the source clock were added to
//! `analog::mna` on top of a solver that a browser already runs. The claim that
//! goes with that change is narrow and testable: **a circuit with no nonlinear
//! element takes the same floating-point operations, in the same order, on the
//! same values as it did before.** Everything new is either behind
//! `Circuit::is_nonlinear`, or a loop over a vector that is empty.
//!
//! The constants below were produced by running this same fixture against the
//! engine as it stood at commit `135a80d7` — the merge base of this branch,
//! before any of it existed — and they are checked here against the engine as
//! it stands now. A failure means the linear path moved, which is a bug in this
//! branch and not a golden to re-pin.
//!
//! To regenerate after a deliberate change to the linear solver:
//!
//! ```text
//! git checkout <old-sha> -- crates/core/src/analog crates/core/Cargo.toml
//! # temporarily make the assertions `eprintln!`s, then
//! cargo test -p labwired-core --test analog_linear_golden -- --nocapture
//! git checkout HEAD -- crates/core/src/analog crates/core/Cargo.toml
//! ```
//!
//! Restore with exactly those two pathspecs. Naming this file as a third
//! makes `git checkout` fail the whole command when it is still untracked,
//! and leave the sources sitting at the old commit.

mod analog {
    use labwired_core::analog::{parse_netlist, Integration, Solver};

    /// Every linear element the engine has, on one circuit: two resistors, a
    /// capacitor with an initial condition, an inductor, a driven voltage
    /// source, a driven current source and a switch. The drive pattern below
    /// walks all of the solver's state machine — the cached-factorisation hit,
    /// the settled early-out, the trapezoidal restart after a discontinuity,
    /// and a mid-run operating-point re-solve.
    const FIXTURE: &str = "V1 in 0 DC 5\n\
                           R1 in out 1000\n\
                           C1 out 0 1u ic=0\n\
                           L1 out tail 1m\n\
                           R2 tail 0 100\n\
                           I1 out 0 DC 0\n\
                           S1 out 0 touch ron=10 roff=1e9\n";

    /// FNV-1a over the raw bits of every unknown at every step. A hash rather
    /// than a list because the point is "nothing moved", and one number that
    /// covers 200 steps of five unknowns says that without 1000 literals.
    fn run_and_hash(integration: Integration) -> (u64, Vec<u64>) {
        let circuit = parse_netlist(FIXTURE).expect("fixture parses");
        let mut solver = Solver::new(circuit, integration).expect("solver builds");
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let absorb = |hash: &mut u64, value: f64| {
            for byte in value.to_bits().to_le_bytes() {
                *hash ^= u64::from(byte);
                *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };

        for step in 0..200u32 {
            let h = if step % 7 < 4 { 1e-6 } else { 2e-6 };
            solver.set_voltage_source(0, if step % 9 < 5 { 5.0 } else { 0.0 });
            solver.set_current_source(0, if step % 6 < 3 { 0.001 } else { 0.0 });
            solver.set_switch(0, step % 10 >= 5);
            if step == 137 {
                solver.solve_operating_point().expect("op point re-solves");
            }
            solver.advance(h).expect("step solves");
            for node in 0..3 {
                absorb(&mut hash, solver.node_voltage(Some(node)));
            }
            for branch in 0..2 {
                absorb(&mut hash, solver.branch_current(branch));
            }
            absorb(&mut hash, solver.capacitor_voltage(0));
            absorb(&mut hash, solver.inductor_current(0));
        }

        let final_state = vec![
            solver.node_voltage(Some(0)).to_bits(),
            solver.node_voltage(Some(1)).to_bits(),
            solver.node_voltage(Some(2)).to_bits(),
            solver.capacitor_voltage(0).to_bits(),
            solver.inductor_current(0).to_bits(),
        ];
        (hash, final_state)
    }

    #[test]
    fn backward_euler_is_bit_identical_to_the_pre_newton_engine() {
        let (hash, final_state) = run_and_hash(Integration::BackwardEuler);
        assert_eq!(hash, BE_HASH, "backward-Euler trajectory moved");
        assert_eq!(final_state, BE_FINAL, "backward-Euler final state moved");
    }

    #[test]
    fn trapezoidal_is_bit_identical_to_the_pre_newton_engine() {
        let (hash, final_state) = run_and_hash(Integration::Trapezoidal);
        assert_eq!(hash, TRAP_HASH, "trapezoidal trajectory moved");
        assert_eq!(final_state, TRAP_FINAL, "trapezoidal final state moved");
    }

    /// Captured from `labwired-core` at `135a80d7`, this branch's merge base.
    const BE_HASH: u64 = 0xbe72_dfd7_6756_62b5;
    const BE_FINAL: [u64; 5] = [
        4_617_315_517_961_601_024,
        4_584_170_094_374_593_437,
        4_585_218_585_826_067_264,
        4_584_170_094_374_593_437,
        4_555_109_562_575_750_186,
    ];
    const TRAP_HASH: u64 = 0xb59b_5612_aca7_e947;
    const TRAP_FINAL: [u64; 5] = [
        4_617_315_517_961_601_024,
        4_583_886_896_028_357_355,
        4_585_057_052_372_911_680,
        4_583_886_896_028_357_355,
        4_554_902_799_755_711_038,
    ];
}
