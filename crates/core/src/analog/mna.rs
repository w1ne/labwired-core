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

use super::device;
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

    /// Junction voltage of each diode, the point its stamp is linearised
    /// about. Carried across steps: the previous step's answer is the best
    /// first guess there is.
    diode_vj: Vec<f64>,
    /// Each BJT's base-emitter voltage in device coordinates.
    bjt_vbe: Vec<f64>,
    /// Each BJT's base-collector voltage in device coordinates.
    bjt_vbc: Vec<f64>,
    /// Each MOSFET's gate-source voltage in device coordinates, as the netlist
    /// names the terminals — not swapped for reverse mode.
    mos_vgs: Vec<f64>,
    /// Each MOSFET's drain-source voltage in device coordinates, as the
    /// netlist names the terminals. Negative means the device is reversed.
    mos_vds: Vec<f64>,
    /// True when the last [`Solver::relinearise`] damped a step, so the next
    /// solution is not a Newton step and must not be accepted as converged.
    limiter_clamped: bool,
    /// True when the circuit holds a device whose stamp depends on the
    /// solution. False takes the pre-Newton code path, untouched.
    nonlinear: bool,
    /// True when a source carries a transient function, so its value has to be
    /// re-evaluated against the clock each step.
    has_waveforms: bool,
    /// Simulated time at the end of the last completed step, seconds.
    time: f64,
    /// Internal steps completed since the solver was built.
    step_index: u64,

    matrix: Vec<f64>,
    rhs: Vec<f64>,
    solution: Vec<f64>,
    /// The previous Newton iterate, for the convergence test.
    previous: Vec<f64>,
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
        let nonlinear = circuit.is_nonlinear();
        let has_waveforms = circuit.has_waveforms();
        let diodes = circuit.diodes.len();
        let bjts = circuit.bjts.len();
        let mosfets = circuit.mosfets.len();

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
            // Every junction starts at zero volts, which is the state of a
            // circuit that has not been powered: the operating-point Newton
            // walks up from there.
            diode_vj: vec![0.0; diodes],
            bjt_vbe: vec![0.0; bjts],
            bjt_vbc: vec![0.0; bjts],
            mos_vgs: vec![0.0; mosfets],
            mos_vds: vec![0.0; mosfets],
            limiter_clamped: false,
            nonlinear,
            has_waveforms,
            time: 0.0,
            step_index: 0,
            matrix: vec![0.0; unknowns * unknowns],
            rhs: vec![0.0; unknowns],
            solution: vec![0.0; unknowns],
            previous: vec![0.0; unknowns],
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
        if self.nonlinear {
            self.newton(Stamp::OperatingPoint, None, self.time)?;
        } else {
            self.build(Stamp::OperatingPoint);
            self.factorize()?;
            self.solve();
        }
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
        let time = self.time + h;
        self.apply_waveforms(time);
        // Backward Euler for the first step after a discontinuity; see the
        // module docs. A no-op distinction when the rule already is BE.
        let rule = if self.restart {
            Integration::BackwardEuler
        } else {
            self.integration
        };
        let stamp = Stamp::Transient(h, rule);
        if self.nonlinear {
            // Every device stamp moves with the solution, so there is nothing
            // to cache between iterations, let alone between steps.
            self.newton(stamp, Some(self.step_index), time)?;
        } else if self.cached_stamp == Some(stamp) {
            if self.settled && !self.restart {
                self.time = time;
                self.step_index += 1;
                return Ok(());
            }
            self.build_rhs(stamp);
            self.solve();
        } else {
            self.cached_stamp = None;
            self.build(stamp);
            self.factorize()?;
            self.cached_stamp = Some(stamp);
            self.solve();
        }
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
        self.time = time;
        self.step_index += 1;
        Ok(())
    }

    /// Simulated time at the end of the last completed step, seconds.
    pub fn time(&self) -> f64 {
        self.time
    }

    /// Internal steps completed since the solver was built.
    pub fn step_index(&self) -> u64 {
        self.step_index
    }

    /// True when the circuit needs Newton iteration.
    pub fn is_nonlinear(&self) -> bool {
        self.nonlinear
    }

    /// Re-evaluate every source that carries a transient function at `time`.
    ///
    /// Sources are evaluated at the END of the step, which is the point the
    /// backward-Euler and trapezoidal companion models are written about, and
    /// is what SPICE does. A value that actually moved counts as a
    /// discontinuity for the trapezoidal restart rule, exactly as a routed
    /// input does.
    fn apply_waveforms(&mut self, time: f64) {
        if !self.has_waveforms {
            return;
        }
        for index in 0..self.circuit.voltage_sources.len() {
            let wave = self.circuit.voltage_sources[index].wave;
            if wave.is_constant() {
                continue;
            }
            self.set_voltage_source(index, wave.at(time));
        }
        for index in 0..self.circuit.current_sources.len() {
            let wave = self.circuit.current_sources[index].wave;
            if wave.is_constant() {
                continue;
            }
            self.set_current_source(index, wave.at(time));
        }
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
        self.stamp_devices();
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

// ---------------------------------------------------------------------------
// Newton–Raphson for the nonlinear devices
// ---------------------------------------------------------------------------

/// Iterations one step is allowed before it is reported as a failure.
///
/// SPICE's default is 100 for a transient step. Reaching anything near it means
/// the circuit is hard, not that the budget is tight: with voltage limiting a
/// diode or transistor stage converges in single digits, and a step that wants
/// 50 iterations is usually one that wants a smaller `substeps` interval.
pub const MAX_NEWTON_ITERATIONS: u32 = 100;

/// Relative tolerance on every unknown between two Newton iterations.
///
/// Tighter than SPICE's `RELTOL` (1e-3) on purpose. The engine's job here is to
/// be reproducible rather than fast: at 1e-3 the converged answer depends on
/// the path the limiter took, which makes a golden worth nothing, and the extra
/// iterations cost microseconds on a 64-unknown dense solve.
pub const NEWTON_RELTOL: f64 = 1e-9;

/// Absolute floor for a node voltage, volts. SPICE's `VNTOL` is 1e-6.
pub const NEWTON_VNTOL: f64 = 1e-12;

/// Absolute floor for a branch current, amps. SPICE's `ABSTOL` is 1e-12.
pub const NEWTON_ABSTOL: f64 = 1e-15;

impl Solver {
    /// Iterate the device stamps until the solution stops moving.
    ///
    /// Each pass re-linearises every nonlinear device about the voltages the
    /// last pass produced, re-stamps, re-factorises and solves. Convergence is
    /// declared when every unknown moved less than its tolerance **and** the
    /// voltage limiter did nothing on the previous pass — a limited step is a
    /// damped one, not a Newton step, so accepting one would report an answer
    /// the equations do not satisfy.
    fn newton(&mut self, stamp: Stamp, step: Option<u64>, time: f64) -> Result<(), AnalogError> {
        let dim = self.dim();
        self.cached_stamp = None;
        self.settled = false;
        // NaN so the first pass can never compare as converged, whatever the
        // previous step left in the buffer.
        self.previous[..dim].fill(f64::NAN);
        self.limiter_clamped = true;

        for _ in 1..=MAX_NEWTON_ITERATIONS {
            self.build(stamp);
            self.factorize()?;
            self.solve();
            if self.newton_converged() {
                return Ok(());
            }
            self.previous[..dim].copy_from_slice(&self.solution[..dim]);
            self.relinearise();
        }

        let (unknown, delta) = self.worst_residual();
        Err(AnalogError::NoConvergence {
            step,
            time,
            iterations: MAX_NEWTON_ITERATIONS,
            unknown,
            delta,
        })
    }

    /// Per-unknown tolerance: volts for a node row, amps for a branch row.
    fn newton_tolerance(&self, index: usize, new: f64, old: f64) -> f64 {
        let floor = if index < self.nodes {
            NEWTON_VNTOL
        } else {
            NEWTON_ABSTOL
        };
        floor + NEWTON_RELTOL * new.abs().max(old.abs())
    }

    fn newton_converged(&self) -> bool {
        if self.limiter_clamped {
            return false;
        }
        for index in 0..self.dim() {
            let new = self.solution[index];
            let old = self.previous[index];
            let delta = (new - old).abs();
            // The NaN test is not decoration. Every comparison against a NaN is
            // false, so a bare `delta > tol` reads a NaN solution as CONVERGED
            // and writes it into the trace as an answer. Naming it here is what
            // turns that into the step's coded error instead.
            if delta.is_nan() || delta > self.newton_tolerance(index, new, old) {
                return false;
            }
        }
        true
    }

    /// The unknown that was still moving most, for the failure message.
    fn worst_residual(&self) -> (String, f64) {
        let mut worst = 0usize;
        let mut worst_ratio = f64::NEG_INFINITY;
        for index in 0..self.dim() {
            let new = self.solution[index];
            let old = self.previous[index];
            let delta = (new - old).abs();
            let ratio = delta / self.newton_tolerance(index, new, old);
            // `>` with a NaN ratio is false, so a NaN never wins the report and
            // hides a real residual behind it; the NaN is named only when
            // nothing else moved at all.
            if ratio > worst_ratio || (worst_ratio.is_nan() && !ratio.is_nan()) {
                worst_ratio = ratio;
                worst = index;
            }
        }
        let name = if worst < self.nodes {
            format!("v({})", self.circuit.node_name(worst))
        } else {
            format!("i({})", self.circuit.branch_name(worst - self.nodes))
        };
        (name, (self.solution[worst] - self.previous[worst]).abs())
    }

    /// Move every device's linearisation point to the solution just found,
    /// damped by the SPICE limiters.
    fn relinearise(&mut self) {
        self.limiter_clamped = false;

        for index in 0..self.circuit.diodes.len() {
            let diode = &self.circuit.diodes[index];
            let raw = node_diff(
                &self.solution[..self.nodes],
                diode.junction_anode,
                diode.cathode,
            );
            let vte = diode.model.n * device::THERMAL_VOLTAGE;
            let critical = device::pn_critical_voltage(diode.model.is, diode.model.n);
            let limited = device::pn_limit(raw, self.diode_vj[index], vte, critical);
            self.limiter_clamped |= limited != raw;
            self.diode_vj[index] = limited;
        }

        for index in 0..self.circuit.bjts.len() {
            let bjt = &self.circuit.bjts[index];
            let sign = bjt.model.polarity.sign();
            let nodes = &self.solution[..self.nodes];
            let raw_be = sign * node_diff(nodes, bjt.b, bjt.e);
            let raw_bc = sign * node_diff(nodes, bjt.b, bjt.c);
            let vtf = bjt.model.nf * device::THERMAL_VOLTAGE;
            let vtr = bjt.model.nr * device::THERMAL_VOLTAGE;
            let crit_f = device::pn_critical_voltage(bjt.model.is, bjt.model.nf);
            let crit_r = device::pn_critical_voltage(bjt.model.is, bjt.model.nr);
            let be = device::pn_limit(raw_be, self.bjt_vbe[index], vtf, crit_f);
            let bc = device::pn_limit(raw_bc, self.bjt_vbc[index], vtr, crit_r);
            self.limiter_clamped |= be != raw_be || bc != raw_bc;
            self.bjt_vbe[index] = be;
            self.bjt_vbc[index] = bc;
        }

        for index in 0..self.circuit.mosfets.len() {
            let mosfet = &self.circuit.mosfets[index];
            let sign = mosfet.model.polarity.sign();
            let nodes = &self.solution[..self.nodes];
            let raw_gs = sign * node_diff(nodes, mosfet.g, mosfet.s);
            let raw_ds = sign * node_diff(nodes, mosfet.d, mosfet.s);

            // The limiters are SPICE's and assume the channel's own source, so
            // they are applied to the EFFECTIVE voltages — the ones a reversed
            // device measures against the terminal that is actually its source.
            // When the device crosses over between two iterations its previous
            // effective Vds in the new orientation is zero, which is exactly
            // where it crossed.
            let old_gs = self.mos_vgs[index];
            let old_ds = self.mos_vds[index];
            let reversed_now = raw_ds < 0.0;
            let reversed_before = old_ds < 0.0;
            let effective = |gs: f64, ds: f64, reversed: bool| {
                if reversed {
                    (gs - ds, -ds)
                } else {
                    (gs, ds)
                }
            };
            let (eff_gs_new, eff_ds_new) = effective(raw_gs, raw_ds, reversed_now);
            let (eff_gs_old, eff_ds_old) = if reversed_before == reversed_now {
                effective(old_gs, old_ds, reversed_before)
            } else {
                (effective(old_gs, old_ds, reversed_before).0, 0.0)
            };

            let eff_ds = device::vds_limit(eff_ds_new, eff_ds_old);
            let eff_gs = device::fet_limit(eff_gs_new, eff_gs_old, sign * mosfet.model.vto);
            let (gs, ds) = if reversed_now {
                (eff_gs - eff_ds, -eff_ds)
            } else {
                (eff_gs, eff_ds)
            };
            self.limiter_clamped |= gs != raw_gs || ds != raw_ds;
            self.mos_vgs[index] = gs;
            self.mos_vds[index] = ds;
        }
    }

    /// Stamp every nonlinear device's companion model at its current
    /// linearisation point.
    ///
    /// Each terminal contributes one linearised current `I ≈ I0 + Σ gᵢ·(uᵢ −
    /// uᵢ0)`, where `uᵢ` are the device's controlling voltages in **circuit**
    /// coordinates. [`stamp_linearised`] turns that into the matrix and RHS
    /// entries; the polarity of a PNP or a PMOS shows up only in the sign of
    /// the currents, never in the conductances, because negating both the
    /// controlling voltage and the current leaves `∂I/∂u` alone.
    ///
    /// Nothing here runs for a circuit with no such device: all three vectors
    /// are empty, so a linear netlist takes the same floating-point operations
    /// in the same order it took before this existed.
    fn stamp_devices(&mut self) {
        let dim = self.dim();
        let matrix = &mut self.matrix;
        let rhs = &mut self.rhs;

        for (index, diode) in self.circuit.diodes.iter().enumerate() {
            if diode.model.rs > 0.0 {
                conductance(
                    matrix,
                    dim,
                    diode.anode,
                    diode.junction_anode,
                    1.0 / diode.model.rs,
                );
            }
            let vj = self.diode_vj[index];
            let op = device::diode_op(vj, diode.model.is, diode.model.n);
            let control = [(op.gd, diode.junction_anode, diode.cathode, vj)];
            stamp_linearised(matrix, rhs, dim, diode.junction_anode, op.id, &control);
            stamp_linearised(matrix, rhs, dim, diode.cathode, -op.id, &negate(&control));
        }

        for (index, bjt) in self.circuit.bjts.iter().enumerate() {
            let sign = bjt.model.polarity.sign();
            let model = bjt.model;
            let (vbe, vbc) = (self.bjt_vbe[index], self.bjt_vbc[index]);
            let op = device::bjt_op(vbe, vbc, model.is, model.bf, model.br, model.nf, model.nr);

            // GMIN across both junctions. Without it a transistor that is fully
            // off leaves its base and collector tied to the rest of the circuit
            // by nothing at all, and the matrix is singular on a netlist the
            // user can see is connected.
            conductance(matrix, dim, bjt.b, bjt.e, device::GMIN);
            conductance(matrix, dim, bjt.b, bjt.c, device::GMIN);

            // Controlling voltages in circuit coordinates: u1 = v(b) − v(e),
            // u2 = v(b) − v(c). The device sees sign·u1 and sign·u2.
            let u1 = sign * vbe;
            let u2 = sign * vbc;
            let collector = [
                (op.gif, bjt.b, bjt.e, u1),
                (-op.gir - op.gmu, bjt.b, bjt.c, u2),
            ];
            let base = [(op.gpi, bjt.b, bjt.e, u1), (op.gmu, bjt.b, bjt.c, u2)];
            let emitter = [
                (-op.gif - op.gpi, bjt.b, bjt.e, u1),
                (op.gir, bjt.b, bjt.c, u2),
            ];
            stamp_linearised(matrix, rhs, dim, bjt.c, sign * op.ic, &collector);
            stamp_linearised(matrix, rhs, dim, bjt.b, sign * op.ib, &base);
            stamp_linearised(matrix, rhs, dim, bjt.e, -sign * (op.ic + op.ib), &emitter);
        }

        for (index, mosfet) in self.circuit.mosfets.iter().enumerate() {
            let sign = mosfet.model.polarity.sign();
            let model = mosfet.model;
            let (vgs, vds) = (self.mos_vgs[index], self.mos_vds[index]);

            // Below Vto every conductance in the device is zero, so the drain
            // node would float; the bulk terminal is connected by nothing else
            // at all. GMIN is what makes an off MOSFET a very large resistor
            // instead of an open circuit the matrix cannot invert.
            conductance(matrix, dim, mosfet.d, mosfet.s, device::GMIN);
            conductance(matrix, dim, mosfet.bulk, mosfet.d, device::GMIN);
            conductance(matrix, dim, mosfet.bulk, mosfet.s, device::GMIN);

            // A MOSFET is symmetric: when Vds goes negative the terminal the
            // netlist calls the drain IS the source, and the level-1 equations
            // are written from the source out. Swapping the two nodes is the
            // whole of "reverse mode".
            let reversed = vds < 0.0;
            let (drain, source) = if reversed {
                (mosfet.s, mosfet.d)
            } else {
                (mosfet.d, mosfet.s)
            };
            let (vgs_eff, vds_eff) = if reversed {
                (vgs - vds, -vds)
            } else {
                (vgs, vds)
            };
            let op = device::mos_op(
                vgs_eff,
                vds_eff,
                sign * model.vto,
                mosfet.beta,
                model.lambda,
            );

            let u1 = sign * vgs_eff;
            let u2 = sign * vds_eff;
            let drain_terms = [(op.gm, mosfet.g, source, u1), (op.gds, drain, source, u2)];
            stamp_linearised(matrix, rhs, dim, drain, sign * op.id, &drain_terms);
            stamp_linearised(
                matrix,
                rhs,
                dim,
                source,
                -sign * op.id,
                &negate(&drain_terms),
            );
            // The gate and the bulk carry no DC current in this model, so they
            // need no stamp of their own beyond the GMIN ties above.
        }
    }
}

/// One terminal's linearised current, as a matrix row and an RHS entry.
///
/// `current` is the current flowing **out of** `terminal` into the device at
/// the linearisation point, and each entry of `control` is `(∂current/∂u, p, n,
/// u)` for a controlling voltage `u = v(p) − v(n)`. The equivalent current
/// source is `current − Σ g·u`, which is the part of the tangent that does not
/// depend on the unknowns.
fn stamp_linearised(
    matrix: &mut [f64],
    rhs: &mut [f64],
    dim: usize,
    terminal: NodeRef,
    current: f64,
    control: &[(f64, NodeRef, NodeRef, f64)],
) {
    let Some(row) = terminal else {
        // Ground is not an unknown: its KCL row is the one the MNA drops.
        return;
    };
    let mut equivalent = current;
    for (g, p, n, u) in control {
        add(matrix, dim, Some(row), *p, *g);
        add(matrix, dim, Some(row), *n, -*g);
        equivalent -= g * u;
    }
    rhs[row] -= equivalent;
}

/// The same controlling terms with every conductance negated — the other
/// terminal of a two-terminal current path.
fn negate<const N: usize>(
    control: &[(f64, NodeRef, NodeRef, f64); N],
) -> [(f64, NodeRef, NodeRef, f64); N] {
    control.map(|(g, p, n, u)| (-g, p, n, u))
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
