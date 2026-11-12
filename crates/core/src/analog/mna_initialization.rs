// SPDX-License-Identifier: MIT
//! Constrained t=0 solve. Temporary equations never become circuit elements.

use super::*;

impl Solver {
    pub fn initialization_status(&self) -> &AnalogInitializationStatus {
        &self.initialization_status
    }

    pub(super) fn solve_initial_state(&mut self) -> Result<(), AnalogError> {
        self.solve_static_state().map_err(|error| match error {
            AnalogError::NoConvergence {
                time,
                iterations,
                unknown,
                delta,
                ..
            } => AnalogError::InitialStateNoConvergence {
                time,
                iterations,
                unknown,
                delta,
            },
            other => other,
        })?;
        let state = self.initial_state.as_ref().expect("explicit initial state");
        for (c, prescribed) in self
            .circuit
            .capacitors
            .iter()
            .zip(&state.capacitor_voltages)
        {
            let observed = node_diff(&self.node_v, c.a, c.b);
            if (observed - prescribed).abs() > NEWTON_VNTOL + NEWTON_RELTOL * prescribed.abs() {
                return Err(AnalogError::Config(format!(
                    "initial capacitor state for '{}' is inconsistent with its solved voltage",
                    c.name
                )));
            }
        }
        for (index, (l, prescribed)) in self
            .circuit
            .inductors
            .iter()
            .zip(&state.inductor_currents)
            .enumerate()
        {
            let observed = self.branch_i[self.circuit.voltage_sources.len() + index];
            if (observed - prescribed).abs() > NEWTON_ABSTOL + NEWTON_RELTOL * prescribed.abs() {
                return Err(AnalogError::Config(format!(
                    "initial inductor state for '{}' is inconsistent with its solved current",
                    l.name
                )));
            }
        }
        self.cap_v.copy_from_slice(&state.capacitor_voltages);
        // Individual t=0 capacitor currents are not exposed. Parallel state
        // constraints have only an aggregate reaction current. The mandatory
        // first BE step computes each actual current from its voltage change.
        self.cap_i.fill(0.0);
        self.ind_i.copy_from_slice(&state.inductor_currents);
        for (index, l) in self.circuit.inductors.iter().enumerate() {
            self.ind_v[index] = node_diff(&self.node_v, l.a, l.b);
        }
        self.initialize_device_charge_history();
        self.validate_controlled_buck_direction()?;
        self.initial_state = None;
        let dim = self.nodes + self.branches;
        self.matrix.truncate(dim * dim);
        self.factors.truncate(dim * dim);
        self.rhs.truncate(dim);
        self.solution.truncate(dim);
        self.previous.truncate(dim);
        self.pivots.truncate(dim);
        self.cached_stamp = None;
        self.settled = false;
        self.restart = true;
        Ok(())
    }

    pub(super) fn stamp_initial_capacitor_constraints(&mut self) {
        let Some(state) = &self.initial_state else {
            return;
        };
        let dim = self.nodes + self.branches + state.constraints.len();
        for (index, c) in state.constraints.iter().enumerate() {
            let row = self.nodes + self.branches + index;
            add(&mut self.matrix, dim, Some(row), c.a, 1.0);
            add(&mut self.matrix, dim, Some(row), c.b, -1.0);
            add(&mut self.matrix, dim, c.a, Some(row), 1.0);
            add(&mut self.matrix, dim, c.b, Some(row), -1.0);
            self.rhs[row] = c.volts;
        }
    }

    /// At a purely inductive node, prescribing every I(L) makes its original
    /// KCL row redundant. Check that KCL first, then differentiate it using the
    /// physical inductance matrix: B L^-1 B^T v = 0 for stationary sources.
    /// This determines an open zero-current inductor's voltage without a
    /// leakage resistor, voltage clamp, or an arbitrary voltage assignment.
    pub(super) fn stamp_initial_inductive_kcl(&mut self) -> Result<(), AnalogError> {
        let Some(state) = &self.initial_state else {
            return Ok(());
        };
        let count = self.circuit.inductors.len();
        if count == 0 {
            return Ok(());
        }
        let mut non_inductive = vec![false; self.nodes];
        let mark = |nodes: &mut [bool], refs: &[NodeRef]| {
            for node in refs.iter().flatten() {
                nodes[*node] = true;
            }
        };
        for (index, r) in self.circuit.resistors.iter().enumerate() {
            if self
                .buck_resistor_owner
                .get(index)
                .and_then(|o| *o)
                .is_some_and(|o| !self.buck_enabled[o])
            {
                continue;
            }
            mark(&mut non_inductive, &[r.a, r.b]);
        }
        for c in &self.circuit.capacitors {
            mark(&mut non_inductive, &[c.a, c.b]);
        }
        for s in &self.circuit.switches {
            mark(&mut non_inductive, &[s.a, s.b]);
        }
        for s in &self.circuit.voltage_sources {
            mark(&mut non_inductive, &[s.p, s.n]);
        }
        for s in &self.circuit.vcvs {
            mark(&mut non_inductive, &[s.p, s.n]);
        }
        for s in &self.circuit.ccvs {
            mark(&mut non_inductive, &[s.p, s.n]);
        }
        for s in &self.circuit.cccs {
            mark(&mut non_inductive, &[s.p, s.n]);
        }
        for (index, s) in self.circuit.vccs.iter().enumerate() {
            if self
                .buck_vccs_owner
                .get(index)
                .and_then(|o| *o)
                .is_some_and(|o| !self.buck_enabled[o])
            {
                continue;
            }
            mark(&mut non_inductive, &[s.p, s.n]);
        }
        for d in &self.circuit.diodes {
            mark(&mut non_inductive, &[d.anode, d.cathode, d.junction_anode]);
        }
        for b in &self.circuit.bjts {
            mark(
                &mut non_inductive,
                &[b.c, b.b, b.e, b.junction_c, b.junction_b, b.junction_e],
            );
        }
        for m in &self.circuit.mosfets {
            mark(
                &mut non_inductive,
                &[m.d, m.g, m.s, m.bulk, m.junction_d, m.junction_s],
            );
        }
        for j in &self.circuit.jfets {
            mark(
                &mut non_inductive,
                &[j.d, j.g, j.s, j.junction_d, j.junction_s],
            );
        }
        for (index, s) in self.circuit.power_sinks.iter().enumerate() {
            if self
                .buck_sink_owner
                .get(index)
                .and_then(|o| *o)
                .is_some_and(|o| !self.buck_enabled[o])
            {
                continue;
            }
            mark(&mut non_inductive, &[s.p, s.n]);
        }
        for s in &self.circuit.output_current_limits {
            mark(&mut non_inductive, &[s.output]);
        }
        for s in &self.circuit.slew_limited_vccs {
            mark(&mut non_inductive, &[s.p, s.n]);
        }

        let dim = self.nodes + self.branches + state.constraints.len();
        for (node, has_other_current_path) in non_inductive.iter().enumerate() {
            if *has_other_current_path {
                continue;
            }
            let mut incidence = vec![0.0; count];
            let mut has_inductor = false;
            let mut residual = 0.0;
            let mut scale = 0.0;
            for (index, l) in self.circuit.inductors.iter().enumerate() {
                if l.a == Some(node) {
                    incidence[index] += 1.0;
                    has_inductor = true;
                }
                if l.b == Some(node) {
                    incidence[index] -= 1.0;
                    has_inductor = true;
                }
                residual += incidence[index] * state.inductor_currents[index];
                scale += (incidence[index] * state.inductor_currents[index]).abs();
            }
            if !has_inductor {
                continue;
            } // A floating resistive/current-source net stays singular.
            for (index, source) in self.circuit.current_sources.iter().enumerate() {
                let sign = (if source.p == Some(node) { 1.0 } else { 0.0 })
                    - (if source.n == Some(node) { 1.0 } else { 0.0 });
                residual += sign * self.current_source_values[index];
                scale += (sign * self.current_source_values[index]).abs();
            }
            if !residual.is_finite() || residual.abs() > NEWTON_ABSTOL + 64.0 * f64::EPSILON * scale
            {
                return Err(AnalogError::Config(format!(
                    "initial inductor currents violate KCL at '{}': residual {residual} A",
                    self.circuit.node_name(node)
                )));
            }
            self.matrix[node * dim..(node + 1) * dim].fill(0.0);
            self.rhs[node] = 0.0;
            for (j, sign) in incidence.iter().enumerate() {
                if *sign == 0.0 {
                    continue;
                }
                for (k, l) in self.circuit.inductors.iter().enumerate() {
                    let weight = sign * state.inverse_inductance[j * count + k];
                    add(&mut self.matrix, dim, Some(node), l.a, weight);
                    add(&mut self.matrix, dim, Some(node), l.b, -weight);
                }
            }
            let norm = self.matrix[node * dim..(node + 1) * dim]
                .iter()
                .fold(0.0_f64, |a, b| a.max(b.abs()));
            if !norm.is_finite() {
                return Err(AnalogError::Config(
                    "nonfinite differentiated inductive KCL".into(),
                ));
            }
            if norm > 0.0 {
                for value in &mut self.matrix[node * dim..(node + 1) * dim] {
                    *value /= norm;
                }
            }
        }
        Ok(())
    }
}
