// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Modified nodal analysis with companion models — the numerical core.
//!
//! One `Solver` owns a parsed [`Circuit`] plus the state that carries across
//! steps (node voltages, branch currents, capacitor voltages and currents,
//! inductor currents and voltages). Each call to [`Solver::advance`] stamps one
//! linear system for one internal step `h` and solves it with dense LU.
//!
//! ## Unknowns
//!
//! `N` node voltages (ground is not an unknown) followed by `M` branch
//! currents: one per voltage source in netlist order, then one per inductor in
//! netlist order. `N + M` is capped at [`MAX_UNKNOWNS`]; the matrix is dense
//! and the browser is the target, so a circuit that wants more wants ngspice.
//!
//! ## Companion models
//!
//! Backward Euler (default):
//!
//! * capacitor → conductance `C/h` in parallel with a current source
//!   `(C/h)·v_prev`;
//! * inductor → an extra row `v_a − v_b − (L/h)·i = −(L/h)·i_prev`.
//!
//! Trapezoidal (`integration: trap`):
//!
//! * capacitor → conductance `2C/h` with source `(2C/h)·v_prev + i_prev`;
//! * inductor → `v_a − v_b − (2L/h)·i = −(2L/h)·i_prev − v_prev`.
//!
//! Trapezoidal is second-order accurate and roughly two orders of magnitude
//! closer to the closed form at the same step; backward Euler is the default
//! because it cannot ring on a step input.
//!
//! ## Restart after a discontinuity
//!
//! Trapezoidal integrates with the element's derivative at the previous point
//! (the capacitor current, the inductor voltage). When a source or switch
//! changes, that stored derivative belongs to the circuit as it was BEFORE the
//! change, and trusting it costs a half-step lag that never goes away — on the
//! 1 ms RC fixture that is 0.29 % at one tau, no better than backward Euler.
//! So the first internal step after any change (and after the operating point)
//! is taken with backward Euler, which needs no stored derivative, and
//! trapezoidal resumes from the consistent state it leaves. This is what SPICE
//! does at a breakpoint, and it brings the same fixture to 0.002 %.
//!
//! ## Determinism
//!
//! `f64` throughout, `Vec` indices in the hot path, `BTreeMap` only for name
//! lookup at build time, no threads and no wall clock. The same netlist and the
//! same input sequence produce bit-identical voltages on every host.

use super::netlist::{AnalogError, Circuit, NodeRef};

/// Largest MNA system the in-core engine solves: `N` nodes + `M` branch
/// currents. Dense LU is `O(n³)`; at 64 unknowns one step is a few microseconds
/// even in the browser, and a bigger circuit is a sign the user wants ngspice.
pub const MAX_UNKNOWNS: usize = 64;

/// Integration rule for the reactive companion models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Integration {
    /// Backward Euler — first order, unconditionally stable, never rings.
    #[default]
    BackwardEuler,
    /// Trapezoidal — second order, far more accurate per step, can ring on a
    /// hard step input.
    Trapezoidal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stamp {
    /// t = 0: capacitors open, inductors short.
    OperatingPoint,
    /// One transient step of length `h` seconds under the given rule.
    Transient(f64, Integration),
}

/// One circuit and the state that carries between steps.
#[derive(Debug, Clone)]
pub struct Solver {
    circuit: Circuit,
    integration: Integration,
    nodes: usize,
    branches: usize,

    voltage_source_values: Vec<f64>,
    current_source_values: Vec<f64>,
    switch_closed: Vec<bool>,

    node_v: Vec<f64>,
    branch_i: Vec<f64>,
    cap_v: Vec<f64>,
    cap_i: Vec<f64>,
    ind_i: Vec<f64>,
    ind_v: Vec<f64>,

    matrix: Vec<f64>,
    rhs: Vec<f64>,
    solution: Vec<f64>,
    // Elimination multipliers are kept in the order they were applied, not
    // permuted as a conventional L matrix. Replaying them preserves the old
    // solver's RHS floating-point operation order exactly.
    factors: Vec<f64>,
    pivots: Vec<usize>,
    cached_stamp: Option<Stamp>,
    settled: bool,

    /// The next internal step must not trust the stored derivatives: a source
    /// or switch changed, or the state was just set by the operating point.
    restart: bool,
}

impl Solver {
    /// Build a solver over `circuit` and solve its operating point.
    ///
    /// The operating point uses the netlist's own DC values with capacitors
    /// open and inductors shorted, then applies `.ic` / `ic=` overrides —
    /// exactly what the ngspice wrapper gets from a transient that pauses just
    /// after t = 0 before any routed input is applied. Switches start open
    /// (`roff`), because their control is a routed input and no input has
    /// arrived yet.
    pub fn new(circuit: Circuit, integration: Integration) -> Result<Self, AnalogError> {
        let unknowns = circuit.unknowns();
        if unknowns > MAX_UNKNOWNS {
            return Err(AnalogError::TooLarge {
                unknowns,
                max: MAX_UNKNOWNS,
            });
        }
        if unknowns == 0 {
            return Err(AnalogError::Config(
                "netlist declares no nodes to solve for".to_string(),
            ));
        }

        let nodes = circuit.node_count();
        let branches = circuit.branch_count();
        let voltage_source_values = circuit.voltage_sources.iter().map(|s| s.dc).collect();
        let current_source_values = circuit.current_sources.iter().map(|s| s.dc).collect();
        let switch_closed = vec![false; circuit.switches.len()];
        let capacitors = circuit.capacitors.len();
        let inductors = circuit.inductors.len();

        let mut solver = Self {
            circuit,
            integration,
            nodes,
            branches,
            voltage_source_values,
            current_source_values,
            switch_closed,
            node_v: vec![0.0; nodes],
            branch_i: vec![0.0; branches],
            cap_v: vec![0.0; capacitors],
            cap_i: vec![0.0; capacitors],
            ind_i: vec![0.0; inductors],
            ind_v: vec![0.0; inductors],
            matrix: vec![0.0; unknowns * unknowns],
            rhs: vec![0.0; unknowns],
            solution: vec![0.0; unknowns],
            factors: vec![0.0; unknowns * unknowns],
            pivots: vec![0; unknowns],
            cached_stamp: None,
            settled: false,
            restart: true,
        };
        solver.solve_operating_point()?;
        Ok(solver)
    }

    /// The parsed circuit this solver integrates.
    pub fn circuit(&self) -> &Circuit {
        &self.circuit
    }

    /// Integration rule in force.
    pub fn integration(&self) -> Integration {
        self.integration
    }

    /// Index of the named voltage source, for the hot path.
    pub fn voltage_source_index(&self, name: &str) -> Option<usize> {
        let key = name.trim().to_ascii_lowercase();
        self.circuit
            .voltage_sources
            .iter()
            .position(|s| s.name.to_ascii_lowercase() == key)
    }

    /// Index of the named current source, for the hot path.
    pub fn current_source_index(&self, name: &str) -> Option<usize> {
        let key = name.trim().to_ascii_lowercase();
        self.circuit
            .current_sources
            .iter()
            .position(|s| s.name.to_ascii_lowercase() == key)
    }

    /// Indices of every switch driven by the routed boolean input `ctrl`.
    /// Several switches may share one control (a break-before-make pair).
    pub fn switches_controlled_by(&self, ctrl: &str) -> Vec<usize> {
        let key = ctrl.trim().to_ascii_lowercase();
        self.circuit
            .switches
            .iter()
            .enumerate()
            .filter(|(_, s)| s.ctrl.to_ascii_lowercase() == key)
            .map(|(index, _)| index)
            .collect()
    }

    /// Index of the switch named `name` (the `S...` element, not its control).
    pub fn switch_index(&self, name: &str) -> Option<usize> {
        let key = name.trim().to_ascii_lowercase();
        self.circuit
            .switches
            .iter()
            .position(|s| s.name.to_ascii_lowercase() == key)
    }

    /// Drive a voltage source. Takes effect on the next [`Self::advance`].
    ///
    /// Setting the value it already has is not a change: a GPIO held high is
    /// re-sent every co-simulation step, and treating that as an edge would
    /// restart trapezoidal integration every step.
    pub fn set_voltage_source(&mut self, index: usize, volts: f64) {
        if self.voltage_source_values[index] != volts {
            self.voltage_source_values[index] = volts;
            self.restart = true;
            self.settled = false;
        }
    }

    /// Drive a current source. Takes effect on the next [`Self::advance`].
    pub fn set_current_source(&mut self, index: usize, amps: f64) {
        if self.current_source_values[index] != amps {
            self.current_source_values[index] = amps;
            self.restart = true;
            self.settled = false;
        }
    }

    /// Open or close a switch. Takes effect on the next [`Self::advance`].
    pub fn set_switch(&mut self, index: usize, closed: bool) {
        if self.switch_closed[index] != closed {
            self.switch_closed[index] = closed;
            self.cached_stamp = None;
            self.restart = true;
            self.settled = false;
        }
    }

    /// Voltage at a node (`None` is ground, which is always 0 V).
    pub fn node_voltage(&self, node: NodeRef) -> f64 {
        match node {
            Some(index) => self.node_v[index],
            None => 0.0,
        }
    }

    /// Current through branch `index` (voltage sources first, then inductors),
    /// flowing from the element's first terminal to its second.
    pub fn branch_current(&self, index: usize) -> f64 {
        self.branch_i[index]
    }

    /// Voltage across capacitor `index`, as carried into the next step.
    pub fn capacitor_voltage(&self, index: usize) -> f64 {
        self.cap_v[index]
    }

    /// Current through inductor `index`, as carried into the next step.
    pub fn inductor_current(&self, index: usize) -> f64 {
        self.ind_i[index]
    }

    fn dim(&self) -> usize {
        self.nodes + self.branches
    }

    /// Solve the DC operating point and reset the reactive state to it.
    pub fn solve_operating_point(&mut self) -> Result<(), AnalogError> {
        self.cached_stamp = None;
        self.settled = false;
        self.build(Stamp::OperatingPoint);
        self.factorize()?;
        self.solve();
        self.node_v.copy_from_slice(&self.solution[..self.nodes]);
        self.branch_i.copy_from_slice(&self.solution[self.nodes..]);
        for index in 0..self.circuit.capacitors.len() {
            let capacitor = &self.circuit.capacitors[index];
            self.cap_v[index] = node_diff(&self.node_v, capacitor.a, capacitor.b);
            self.cap_i[index] = 0.0;
        }
        for index in 0..self.circuit.inductors.len() {
            self.ind_i[index] = self.branch_i[self.circuit.voltage_sources.len() + index];
            self.ind_v[index] = 0.0;
        }
        self.apply_initial_conditions();
        self.restart = true;
        Ok(())
    }

    /// `.ic V(node)=v` and element `ic=` override the operating point.
    ///
    /// The operating point is solved first and then overridden, rather than
    /// solved subject to the constraints: the two agree for every netlist where
    /// `.ic` is used for what it is for (seeding a reactive element), and the
    /// override order — node `.ic` first, then per-element `ic=` — is stated
    /// here so a netlist that sets both is not ambiguous.
    fn apply_initial_conditions(&mut self) {
        for (node, value) in &self.circuit.node_ic {
            self.node_v[*node] = *value;
        }
        if !self.circuit.node_ic.is_empty() {
            for index in 0..self.circuit.capacitors.len() {
                let capacitor = &self.circuit.capacitors[index];
                self.cap_v[index] = node_diff(&self.node_v, capacitor.a, capacitor.b);
            }
        }
        for index in 0..self.circuit.capacitors.len() {
            if let Some(ic) = self.circuit.capacitors[index].ic {
                self.cap_v[index] = ic;
            }
        }
        for index in 0..self.circuit.inductors.len() {
            if let Some(ic) = self.circuit.inductors[index].ic {
                self.ind_i[index] = ic;
                self.branch_i[self.circuit.voltage_sources.len() + index] = ic;
            }
        }
    }

    /// Integrate one internal step of `h` seconds.
    pub fn advance(&mut self, h: f64) -> Result<(), AnalogError> {
        if h <= 0.0 || !h.is_finite() {
            return Err(AnalogError::Config(format!(
                "internal step must be a positive finite number of seconds, got {h}"
            )));
        }
        // Backward Euler for the first step after a discontinuity; see the
        // module docs. A no-op distinction when the rule already is BE.
        let rule = if self.restart {
            Integration::BackwardEuler
        } else {
            self.integration
        };
        let stamp = Stamp::Transient(h, rule);
        if self.cached_stamp == Some(stamp) {
            if self.settled && !self.restart {
                return Ok(());
            }
            self.build_rhs(stamp);
        } else {
            self.cached_stamp = None;
            self.build(stamp);
            self.factorize()?;
            self.cached_stamp = Some(stamp);
        }
        self.solve();
        self.restart = false;
        self.settled = true;

        for index in 0..self.circuit.capacitors.len() {
            let capacitor = &self.circuit.capacitors[index];
            let v_new = node_diff(&self.solution[..self.nodes], capacitor.a, capacitor.b);
            let geq = match rule {
                Integration::BackwardEuler => capacitor.farads / h,
                Integration::Trapezoidal => 2.0 * capacitor.farads / h,
            };
            let i_new = match rule {
                Integration::BackwardEuler => geq * (v_new - self.cap_v[index]),
                Integration::Trapezoidal => geq * (v_new - self.cap_v[index]) - self.cap_i[index],
            };
            self.settled &= self.cap_v[index].to_bits() == v_new.to_bits()
                && self.cap_i[index].to_bits() == i_new.to_bits();
            self.cap_v[index] = v_new;
            self.cap_i[index] = i_new;
        }
        for index in 0..self.circuit.inductors.len() {
            let inductor = &self.circuit.inductors[index];
            let branch = self.nodes + self.circuit.voltage_sources.len() + index;
            self.settled &= self.ind_i[index].to_bits() == self.solution[branch].to_bits()
                && self.ind_v[index].to_bits()
                    == node_diff(&self.solution[..self.nodes], inductor.a, inductor.b).to_bits();
            self.ind_i[index] = self.solution[branch];
            self.ind_v[index] = node_diff(&self.solution[..self.nodes], inductor.a, inductor.b);
        }

        self.node_v.copy_from_slice(&self.solution[..self.nodes]);
        self.branch_i.copy_from_slice(&self.solution[self.nodes..]);
        Ok(())
    }

    fn build(&mut self, stamp: Stamp) {
        let dim = self.dim();
        self.matrix.iter_mut().for_each(|cell| *cell = 0.0);
        self.rhs.iter_mut().for_each(|cell| *cell = 0.0);

        for resistor in &self.circuit.resistors {
            conductance(
                &mut self.matrix,
                dim,
                resistor.a,
                resistor.b,
                1.0 / resistor.ohms,
            );
        }
        for (index, switch) in self.circuit.switches.iter().enumerate() {
            let ohms = if self.switch_closed[index] {
                switch.ron
            } else {
                switch.roff
            };
            conductance(&mut self.matrix, dim, switch.a, switch.b, 1.0 / ohms);
        }
        for (index, source) in self.circuit.current_sources.iter().enumerate() {
            // SPICE: positive current flows from n+ through the source to n-,
            // so n+ is a sink and n- a supply.
            let amps = self.current_source_values[index];
            inject(&mut self.rhs, source.p, -amps);
            inject(&mut self.rhs, source.n, amps);
        }
        for (index, source) in self.circuit.voltage_sources.iter().enumerate() {
            let row = self.nodes + index;
            add(&mut self.matrix, dim, Some(row), source.p, 1.0);
            add(&mut self.matrix, dim, Some(row), source.n, -1.0);
            add(&mut self.matrix, dim, source.p, Some(row), 1.0);
            add(&mut self.matrix, dim, source.n, Some(row), -1.0);
            self.rhs[row] = self.voltage_source_values[index];
        }
        for (index, inductor) in self.circuit.inductors.iter().enumerate() {
            let row = self.nodes + self.circuit.voltage_sources.len() + index;
            add(&mut self.matrix, dim, Some(row), inductor.a, 1.0);
            add(&mut self.matrix, dim, Some(row), inductor.b, -1.0);
            add(&mut self.matrix, dim, inductor.a, Some(row), 1.0);
            add(&mut self.matrix, dim, inductor.b, Some(row), -1.0);
            match stamp {
                Stamp::OperatingPoint => {
                    // A shorted inductor: v_a - v_b = 0.
                    self.rhs[row] = 0.0;
                }
                Stamp::Transient(h, rule) => match rule {
                    Integration::BackwardEuler => {
                        let req = inductor.henries / h;
                        self.matrix[row * dim + row] -= req;
                        self.rhs[row] = -req * self.ind_i[index];
                    }
                    Integration::Trapezoidal => {
                        let req = 2.0 * inductor.henries / h;
                        self.matrix[row * dim + row] -= req;
                        self.rhs[row] = -req * self.ind_i[index] - self.ind_v[index];
                    }
                },
            }
        }
        if let Stamp::Transient(h, rule) = stamp {
            for (index, capacitor) in self.circuit.capacitors.iter().enumerate() {
                let (geq, ieq) = match rule {
                    Integration::BackwardEuler => {
                        let geq = capacitor.farads / h;
                        (geq, geq * self.cap_v[index])
                    }
                    Integration::Trapezoidal => {
                        let geq = 2.0 * capacitor.farads / h;
                        (geq, geq * self.cap_v[index] + self.cap_i[index])
                    }
                };
                conductance(&mut self.matrix, dim, capacitor.a, capacitor.b, geq);
                inject(&mut self.rhs, capacitor.a, ieq);
                inject(&mut self.rhs, capacitor.b, -ieq);
            }
        }
    }

    /// Refresh only history and source terms when the conductances are unchanged.
    fn build_rhs(&mut self, stamp: Stamp) {
        self.rhs.fill(0.0);
        for (index, source) in self.circuit.current_sources.iter().enumerate() {
            let amps = self.current_source_values[index];
            inject(&mut self.rhs, source.p, -amps);
            inject(&mut self.rhs, source.n, amps);
        }
        for (index, value) in self.voltage_source_values.iter().enumerate() {
            self.rhs[self.nodes + index] = *value;
        }
        if let Stamp::Transient(h, rule) = stamp {
            for (index, inductor) in self.circuit.inductors.iter().enumerate() {
                let row = self.nodes + self.circuit.voltage_sources.len() + index;
                self.rhs[row] = match rule {
                    Integration::BackwardEuler => -(inductor.henries / h) * self.ind_i[index],
                    Integration::Trapezoidal => {
                        -(2.0 * inductor.henries / h) * self.ind_i[index] - self.ind_v[index]
                    }
                };
            }
            for (index, capacitor) in self.circuit.capacitors.iter().enumerate() {
                let ieq = match rule {
                    Integration::BackwardEuler => (capacitor.farads / h) * self.cap_v[index],
                    Integration::Trapezoidal => {
                        (2.0 * capacitor.farads / h) * self.cap_v[index] + self.cap_i[index]
                    }
                };
                inject(&mut self.rhs, capacitor.a, ieq);
                inject(&mut self.rhs, capacitor.b, -ieq);
            }
        }
    }

    /// Dense LU with partial pivoting, in place over the scratch matrix. Own
    /// implementation on purpose: the systems are at most 64×64, and a linear
    /// algebra dependency would have to compile to WASM and stay deterministic
    /// across hosts to buy nothing at this size.
    fn factorize(&mut self) -> Result<(), AnalogError> {
        let dim = self.dim();
        let matrix = &mut self.matrix;
        self.factors.fill(0.0);

        for column in 0..dim {
            let mut pivot_row = column;
            let mut pivot_magnitude = matrix[column * dim + column].abs();
            for row in (column + 1)..dim {
                let magnitude = matrix[row * dim + column].abs();
                if magnitude > pivot_magnitude {
                    pivot_magnitude = magnitude;
                    pivot_row = row;
                }
            }
            if pivot_magnitude < 1e-30 {
                return Err(AnalogError::Singular { row: column });
            }
            self.pivots[column] = pivot_row;
            if pivot_row != column {
                for index in 0..dim {
                    matrix.swap(pivot_row * dim + index, column * dim + index);
                }
            }
            let pivot = matrix[column * dim + column];
            for row in (column + 1)..dim {
                let factor = matrix[row * dim + column] / pivot;
                if factor == 0.0 {
                    continue;
                }
                self.factors[column * dim + row] = factor;
                matrix[row * dim + column] = 0.0;
                for index in (column + 1)..dim {
                    matrix[row * dim + index] -= factor * matrix[column * dim + index];
                }
            }
        }

        Ok(())
    }

    fn solve(&mut self) {
        let dim = self.dim();
        let matrix = &self.matrix;
        let rhs = &mut self.rhs;
        for column in 0..dim {
            rhs.swap(self.pivots[column], column);
            for row in (column + 1)..dim {
                let factor = self.factors[column * dim + row];
                if factor != 0.0 {
                    rhs[row] -= factor * rhs[column];
                }
            }
        }
        for row in (0..dim).rev() {
            let mut accumulator = rhs[row];
            for index in (row + 1)..dim {
                accumulator -= matrix[row * dim + index] * self.solution[index];
            }
            self.solution[row] = accumulator / matrix[row * dim + row];
        }
    }
}

fn node_diff(node_v: &[f64], a: NodeRef, b: NodeRef) -> f64 {
    let va = a.map(|index| node_v[index]).unwrap_or(0.0);
    let vb = b.map(|index| node_v[index]).unwrap_or(0.0);
    va - vb
}

fn add(matrix: &mut [f64], dim: usize, row: NodeRef, column: NodeRef, value: f64) {
    if let (Some(row), Some(column)) = (row, column) {
        matrix[row * dim + column] += value;
    }
}

fn conductance(matrix: &mut [f64], dim: usize, a: NodeRef, b: NodeRef, g: f64) {
    add(matrix, dim, a, a, g);
    add(matrix, dim, b, b, g);
    add(matrix, dim, a, b, -g);
    add(matrix, dim, b, a, -g);
}

fn inject(rhs: &mut [f64], node: NodeRef, amps: f64) {
    if let Some(index) = node {
        rhs[index] += amps;
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use crate::analog::parse_netlist;

    // Independent pre-cache Gaussian elimination oracle: RHS row operations
    // are interleaved with matrix elimination, including multiple row pivots.
    fn original_solution(solver: &mut Solver, h: f64) -> Vec<u64> {
        let rule = if solver.restart {
            Integration::BackwardEuler
        } else {
            solver.integration
        };
        solver.build(Stamp::Transient(h, rule));
        let dim = solver.dim();
        let matrix = &mut solver.matrix;
        let rhs = &mut solver.rhs;
        for column in 0..dim {
            let mut pivot_row = column;
            let mut magnitude = matrix[column * dim + column].abs();
            for row in column + 1..dim {
                let candidate = matrix[row * dim + column].abs();
                if candidate > magnitude {
                    magnitude = candidate;
                    pivot_row = row;
                }
            }
            assert!(magnitude >= 1e-30);
            if pivot_row != column {
                for index in 0..dim {
                    matrix.swap(pivot_row * dim + index, column * dim + index);
                }
                rhs.swap(pivot_row, column);
            }
            let pivot = matrix[column * dim + column];
            for row in column + 1..dim {
                let factor = matrix[row * dim + column] / pivot;
                if factor == 0.0 {
                    continue;
                }
                matrix[row * dim + column] = 0.0;
                for index in column + 1..dim {
                    matrix[row * dim + index] -= factor * matrix[column * dim + index];
                }
                rhs[row] -= factor * rhs[column];
            }
        }
        let mut solution = vec![0.0; dim];
        for row in (0..dim).rev() {
            let mut accumulator = rhs[row];
            for index in row + 1..dim {
                accumulator -= matrix[row * dim + index] * solution[index];
            }
            solution[row] = accumulator / matrix[row * dim + row];
        }
        bits(&solution)
    }

    fn bits(values: &[f64]) -> Vec<u64> {
        values.iter().map(|value| value.to_bits()).collect()
    }

    #[test]
    fn cached_steps_match_refactorization_across_discontinuities() {
        for rule in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let circuit = parse_netlist("V1 in 0 DC 5\nR1 in out 1000\nC1 out 0 1u ic=0\nL1 out tail 1m\nR2 tail 0 100\nI1 out 0 DC 0\nS1 out 0 touch ron=10 roff=1e9\n").unwrap();
            let mut cached = Solver::new(circuit, rule).unwrap();
            let mut reference = cached.clone();
            for step in 0..400 {
                let h = if step % 70 < 35 { 1e-6 } else { 2e-6 };
                for solver in [&mut cached, &mut reference] {
                    solver.set_voltage_source(0, if step % 90 < 45 { 5.0 } else { 0.0 });
                    solver.set_current_source(0, if step % 60 < 30 { 0.001 } else { 0.0 });
                    solver.set_switch(0, step % 100 >= 50);
                    if step == 250 {
                        solver.solve_operating_point().unwrap();
                    }
                }
                let expected = original_solution(&mut reference, h);
                reference.cached_stamp = None;
                reference.settled = false;
                cached.advance(h).unwrap();
                reference.advance(h).unwrap();
                assert_eq!(
                    bits(&cached.solution),
                    expected,
                    "original solve, step {step} {rule:?}"
                );
                assert_eq!(
                    bits(&cached.node_v),
                    bits(&reference.node_v),
                    "step {step} {rule:?}"
                );
                assert_eq!(bits(&cached.branch_i), bits(&reference.branch_i));
                assert_eq!(bits(&cached.cap_i), bits(&reference.cap_i));
                assert_eq!(bits(&cached.ind_v), bits(&reference.ind_v));
            }
        }
    }

    #[test]
    fn settled_circuit_resumes_after_input_or_step_change() {
        for rule in [Integration::BackwardEuler, Integration::Trapezoidal] {
            for netlist in [
                "V1 in 0 DC 5\nR1 in out 1000\nC1 out 0 1u\n",
                "V1 in 0 DC 5\nR1 in out 1000\nR2 out 0 1000\n",
            ] {
                let circuit = parse_netlist(netlist).unwrap();
                let mut solver = Solver::new(circuit, rule).unwrap();
                for _ in 0..10000 {
                    solver.advance(1e-3).unwrap();
                }
                assert!(solver.settled);
                let mut reference = solver.clone();
                for step in 0..50 {
                    if step == 10 {
                        solver.set_voltage_source(0, 0.0);
                        reference.set_voltage_source(0, 0.0);
                    }
                    reference.cached_stamp = None;
                    reference.settled = false;
                    let h = if step < 5 { 1e-3 } else { 1e-4 };
                    solver.advance(h).unwrap();
                    reference.advance(h).unwrap();
                    assert_eq!(bits(&solver.node_v), bits(&reference.node_v));
                    assert_eq!(bits(&solver.cap_i), bits(&reference.cap_i));
                }
            }
        }
    }
}
