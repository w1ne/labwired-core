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
        // The accepted constrained point must obey the same signed supply
        // envelope as OP and transient steps, including an off power state.
        self.opamp_power.validate(&self.circuit, &self.node_v)?;
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

    /// A stationary independent-voltage-source tree fixes internal voltage
    /// differences, but its branch currents cancel in the group's summed KCL.
    /// Prescribed inductor currents make exactly that sum redundant. Check it
    /// first, then replace one member row with B_group L^-1 B_node^T v = 0. The remaining
    /// member rows retain every physical source-current observation.
    pub(super) fn stamp_initial_inductive_kcl(&mut self) -> Result<(), AnalogError> {
        let Some(state) = &self.initial_state else {
            return Ok(());
        };
        let count = self.circuit.inductors.len();
        if count == 0 {
            return Ok(());
        }
        let groups = &state.voltage_groups;
        let group_of = |node: NodeRef| groups[node.unwrap_or(self.nodes)];
        let non_inductive = self.initial_non_inductive_groups(groups);
        let mut representatives = vec![None; groups.len()];
        for (node, group) in groups.iter().take(self.nodes).copied().enumerate() {
            representatives[group].get_or_insert(node);
        }
        let dim = self.nodes + self.branches + state.constraints.len();
        for (group, representative) in representatives.iter().enumerate() {
            let Some(node) = *representative else {
                continue;
            };
            if non_inductive[group] {
                continue;
            }
            let mut incidence = vec![0.0; count];
            let mut residual = 0.0;
            let mut scale = 0.0;
            for (index, l) in self.circuit.inductors.iter().enumerate() {
                incidence[index] = (if group_of(l.a) == group { 1.0 } else { 0.0 })
                    - (if group_of(l.b) == group { 1.0 } else { 0.0 });
                residual += incidence[index] * state.inductor_currents[index];
                scale += (incidence[index] * state.inductor_currents[index]).abs();
            }
            if incidence.iter().all(|sign| *sign == 0.0) {
                // Internal L/V loops have no cross-group derivative equation.
                // Their absolute voltage stays undefined; do not invent a gauge.
                continue;
            }
            for (index, source) in self.circuit.current_sources.iter().enumerate() {
                if !self.opamp_power.active(OpAmpPrimitive::Current, index) {
                    continue;
                }
                let sign = (if group_of(source.p) == group {
                    1.0
                } else {
                    0.0
                }) - (if group_of(source.n) == group {
                    1.0
                } else {
                    0.0
                });
                residual += sign * self.current_source_values[index];
                scale += (sign * self.current_source_values[index]).abs();
            }
            if !residual.is_finite() || residual.abs() > NEWTON_ABSTOL + 64.0 * f64::EPSILON * scale
            {
                return Err(AnalogError::Config(format!(
                    "initial inductor currents violate KCL in voltage group at '{}': residual {residual} A",
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

    fn initial_non_inductive_groups(&self, groups: &[usize]) -> Vec<bool> {
        let group_of = |node: NodeRef| groups[node.unwrap_or(self.nodes)];
        let mut non_inductive = vec![false; groups.len()];
        // Ground-connected source groups have a physical current return. Their
        // ordinary KCL/source equations already fix their voltages and currents.
        non_inductive[group_of(None)] = true;
        let mark = |flags: &mut [bool], terminals: &[NodeRef]| {
            let Some(first) = terminals.first() else {
                return;
            };
            let first_group = group_of(*first);
            if terminals.iter().any(|node| group_of(*node) != first_group) {
                for node in terminals {
                    flags[group_of(*node)] = true;
                }
            }
        };
        for (index, r) in self.circuit.resistors.iter().enumerate() {
            if !self.opamp_power.active(OpAmpPrimitive::Resistor, index)
                || self
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
        // Independent sources form the saved groups. Dependent source outputs
        // are deliberately not contracted: their derivatives need their own
        // controlled-state treatment, outside this stationary-source reduction.
        for s in &self.circuit.vcvs {
            non_inductive[group_of(s.p)] = true;
            non_inductive[group_of(s.n)] = true;
        }
        for s in &self.circuit.ccvs {
            non_inductive[group_of(s.p)] = true;
            non_inductive[group_of(s.n)] = true;
        }
        for s in &self.circuit.cccs {
            mark(&mut non_inductive, &[s.p, s.n]);
        }
        for (index, s) in self.circuit.vccs.iter().enumerate() {
            if !self.opamp_power.active(OpAmpPrimitive::Vccs, index)
                || self
                    .buck_vccs_owner
                    .get(index)
                    .and_then(|o| *o)
                    .is_some_and(|o| !self.buck_enabled[o])
            {
                continue;
            }
            mark(&mut non_inductive, &[s.p, s.n]);
        }
        for (index, d) in self.circuit.diodes.iter().enumerate() {
            if self.opamp_power.active(OpAmpPrimitive::Diode, index) {
                mark(&mut non_inductive, &[d.anode, d.cathode, d.junction_anode]);
            }
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
        for (index, s) in self.circuit.output_current_limits.iter().enumerate() {
            if self.opamp_power.active(OpAmpPrimitive::OutputLimit, index) {
                mark(&mut non_inductive, &[s.output, None]);
            }
        }
        for (index, s) in self.circuit.slew_limited_vccs.iter().enumerate() {
            if self.opamp_power.active(OpAmpPrimitive::SlewLimit, index) {
                mark(&mut non_inductive, &[s.p, s.n]);
            }
        }
        non_inductive
    }
}
