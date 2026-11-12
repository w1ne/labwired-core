//! Optional functional supply qualification for the existing OPAMP macro.
//!
//! This is an explicit envelope/disable model, not a silicon startup, leakage,
//! protection or supply-current model. Passive input and compensation elements
//! keep their original equations and state. Every legacy card without the pair
//! remains unchanged.

use super::netlist::{AnalogError, Circuit, NodeRef};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub struct OpAmpPowerBinding {
    pub name: String,
    pub vcc: NodeRef,
    pub vee: NodeRef,
    pub min_v: f64,
    pub max_v: f64,
    pub(crate) current_sources: Range<usize>,
    pub(crate) vccs: Range<usize>,
    pub(crate) diodes: Range<usize>,
    pub(crate) output_limits: Range<usize>,
    pub(crate) slew_limits: Range<usize>,
    pub(crate) output_resistor: Option<usize>,
}

pub(crate) struct PrimitiveCounts {
    pub currents: usize,
    pub vccs: usize,
    pub diodes: usize,
    pub output_limits: usize,
    pub slew_limits: usize,
}
impl PrimitiveCounts {
    pub fn of(circuit: &Circuit) -> Self {
        Self {
            currents: circuit.current_sources.len(),
            vccs: circuit.vccs.len(),
            diodes: circuit.diodes.len(),
            output_limits: circuit.output_current_limits.len(),
            slew_limits: circuit.slew_limited_vccs.len(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Primitive {
    Current,
    Vccs,
    Diode,
    OutputLimit,
    SlewLimit,
    Resistor,
}

#[derive(Debug, Clone)]
pub(crate) struct OpAmpPowerState {
    enabled: Vec<bool>,
    saved_enabled: Vec<bool>,
    owners: [Vec<Option<usize>>; 6],
}
impl OpAmpPowerState {
    pub fn new(circuit: &Circuit) -> Self {
        let n = circuit.opamp_power.len();
        let mut state = Self {
            // A Newton seed only. Beginning off from the zero voltage guess
            // would make a powered unloaded follower singular before its
            // independent supply source could be solved.
            enabled: vec![true; n],
            saved_enabled: vec![true; n],
            owners: std::array::from_fn(|_| Vec::new()),
        };
        if n == 0 {
            return state;
        }
        state.owners = [
            vec![None; circuit.current_sources.len()],
            vec![None; circuit.vccs.len()],
            vec![None; circuit.diodes.len()],
            vec![None; circuit.output_current_limits.len()],
            vec![None; circuit.slew_limited_vccs.len()],
            vec![None; circuit.resistors.len()],
        ];
        for (owner, binding) in circuit.opamp_power.iter().enumerate() {
            for (kind, range) in [
                (Primitive::Current, binding.current_sources.clone()),
                (Primitive::Vccs, binding.vccs.clone()),
                (Primitive::Diode, binding.diodes.clone()),
                (Primitive::OutputLimit, binding.output_limits.clone()),
                (Primitive::SlewLimit, binding.slew_limits.clone()),
            ] {
                for index in range {
                    state.owners[kind as usize][index] = Some(owner);
                }
            }
            if let Some(index) = binding.output_resistor {
                state.owners[Primitive::Resistor as usize][index] = Some(owner);
            }
        }
        state
    }

    pub fn active(&self, kind: Primitive, index: usize) -> bool {
        self.owners[kind as usize]
            .get(index)
            .and_then(|x| *x)
            .is_none_or(|owner| self.enabled[owner])
    }
    /// Readback uses the accepted voltage buffer, not a Newton trial gate.
    /// A rejected step may leave the working linearisation at an off or
    /// unsupported supply while all public node voltages retain the last
    /// accepted state.
    pub fn active_at(
        &self,
        kind: Primitive,
        index: usize,
        circuit: &Circuit,
        nodes: &[f64],
    ) -> bool {
        self.owners[kind as usize]
            .get(index)
            .and_then(|x| *x)
            .is_none_or(|owner| Self::desired(&circuit.opamp_power[owner], nodes))
    }

    fn span(binding: &OpAmpPowerBinding, nodes: &[f64]) -> f64 {
        binding.vcc.map_or(0.0, |n| nodes[n]) - binding.vee.map_or(0.0, |n| nodes[n])
    }
    fn desired(binding: &OpAmpPowerBinding, nodes: &[f64]) -> bool {
        let v = Self::span(binding, nodes);
        v >= binding.min_v && v <= binding.max_v
    }
    pub fn consistent(&self, circuit: &Circuit, nodes: &[f64]) -> bool {
        circuit
            .opamp_power
            .iter()
            .zip(&self.enabled)
            .all(|(binding, active)| *active == Self::desired(binding, nodes))
    }
    pub fn update(&mut self, circuit: &Circuit, nodes: &[f64]) -> bool {
        let mut changed = false;
        for (binding, active) in circuit.opamp_power.iter().zip(&mut self.enabled) {
            let desired = Self::desired(binding, nodes);
            changed |= *active != desired;
            *active = desired;
        }
        changed
    }
    pub fn validate(&self, circuit: &Circuit, nodes: &[f64]) -> Result<(), AnalogError> {
        for (index, binding) in circuit.opamp_power.iter().enumerate() {
            let span = Self::span(binding, nodes);
            // MNA can leave roundoff at a nominally zero supply. Admit only
            // one binary epsilon of the declared voltage scale while the
            // active paths are off. Preserve raw voltages and desired(), and
            // keep the upper supply bound strict. This is not reverse-power
            // operation or a change to the Newton convergence tolerances.
            let zero_roundoff_v = f64::EPSILON * binding.max_v;
            let inactive_numerical_zero = !self.enabled[index] && span >= -zero_roundoff_v;
            if !span.is_finite() || (span < 0.0 && !inactive_numerical_zero) || span > binding.max_v
            {
                return Err(AnalogError::Config(format!(
                    "OPAMP '{}' solved supply span {span} V is outside 0..={} V (inactive numerical-zero allowance {zero_roundoff_v} V); reverse or excessive supply is unsupported",
                    binding.name, binding.max_v,
                )));
            }
        }
        Ok(())
    }
    pub fn save(&mut self) {
        self.saved_enabled.copy_from_slice(&self.enabled);
    }
    pub fn restore(&mut self) {
        self.enabled.copy_from_slice(&self.saved_enabled);
    }
}

/// Macro primitives have one owner. Routed source/resistance overrides must
/// not rewrite headroom, bias, gain or compensation behind that contract.
pub(crate) fn owns_generated_primitive(circuit: &Circuit, name: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    circuit
        .opamp_power
        .iter()
        .any(|binding| name.starts_with(&format!("{}#", binding.name.to_ascii_lowercase())))
}
