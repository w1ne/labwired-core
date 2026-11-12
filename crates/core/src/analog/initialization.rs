// SPDX-License-Identifier: MIT
//! Explicit reactive state at t=0, separate from a DC operating point.

use super::{AnalogError, Circuit, NodeRef, Waveform};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_INITIAL_STATE_ENTRIES: usize = 16_384;

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnalogInitialization {
    #[default]
    OperatingPoint,
    TransientState {
        capacitor_voltages_v: BTreeMap<String, f64>,
        inductor_currents_a: BTreeMap<String, f64>,
    },
}

#[derive(serde::Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum RawAnalogInitialization {
    OperatingPoint {},
    TransientState {
        capacitor_voltages_v: BTreeMap<String, f64>,
        inductor_currents_a: BTreeMap<String, f64>,
    },
}

impl<'de> serde::Deserialize<'de> for AnalogInitialization {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value =
            match <RawAnalogInitialization as serde::Deserialize>::deserialize(deserializer)? {
                RawAnalogInitialization::OperatingPoint {} => Self::OperatingPoint,
                RawAnalogInitialization::TransientState {
                    capacitor_voltages_v,
                    inductor_currents_a,
                } => Self::TransientState {
                    capacitor_voltages_v,
                    inductor_currents_a,
                },
            };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalogInitializationMode {
    OperatingPoint,
    TransientState,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AnalogInitializationStatus {
    pub mode: AnalogInitializationMode,
    /// Original parsed MNA dimension, with no temporary state constraints.
    pub base_unknowns: usize,
    /// Peak dimension including independent t=0 capacitor constraints.
    pub initialization_unknowns: usize,
}

impl AnalogInitialization {
    pub fn validate(&self) -> Result<(), AnalogError> {
        let Self::TransientState {
            capacitor_voltages_v,
            inductor_currents_a,
        } = self
        else {
            return Ok(());
        };
        if capacitor_voltages_v
            .len()
            .saturating_add(inductor_currents_a.len())
            > MAX_INITIAL_STATE_ENTRIES
        {
            return Err(AnalogError::Config(format!("transient_state permits at most {MAX_INITIAL_STATE_ENTRIES} capacitor and inductor entries")));
        }
        let mut names = BTreeSet::new();
        for (kind, values) in [
            ("capacitor", capacitor_voltages_v),
            ("inductor", inductor_currents_a),
        ] {
            for (name, value) in values {
                if name.is_empty()
                    || name.len() > 256
                    || !name.bytes().all(|c| (0x21..=0x7e).contains(&c))
                {
                    return Err(AnalogError::Config(format!(
                        "{kind} state key must be 1..256 printable ASCII bytes without whitespace"
                    )));
                }
                if !names.insert(name.to_ascii_lowercase()) {
                    return Err(AnalogError::Config(format!(
                        "duplicate case-insensitive {kind} state '{name}'"
                    )));
                }
                if !value.is_finite() {
                    return Err(AnalogError::Config(format!(
                        "{kind} state '{name}' must be finite"
                    )));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(super) struct InitialVoltageConstraint {
    pub name: String,
    pub a: NodeRef,
    pub b: NodeRef,
    pub volts: f64,
}

#[derive(Debug, Clone)]
pub(super) struct InitialStatePlan {
    pub capacitor_voltages: Vec<f64>,
    pub inductor_currents: Vec<f64>,
    pub constraints: Vec<InitialVoltageConstraint>,
    pub inverse_inductance: Vec<f64>,
}

impl InitialStatePlan {
    pub fn prepare_inductance(&mut self, circuit: &Circuit) -> Result<(), AnalogError> {
        self.inverse_inductance = inverse_inductance(circuit)?;
        Ok(())
    }

    pub fn build(
        circuit: &Circuit,
        initialization: &AnalogInitialization,
    ) -> Result<Option<Self>, AnalogError> {
        initialization.validate()?;
        let AnalogInitialization::TransientState {
            capacitor_voltages_v,
            inductor_currents_a,
        } = initialization
        else {
            return Ok(None);
        };
        if !circuit.node_ic.is_empty()
            || circuit.capacitors.iter().any(|c| c.ic.is_some())
            || circuit.inductors.iter().any(|l| l.ic.is_some())
        {
            return Err(AnalogError::Config("transient_state cannot be mixed with netlist .ic or element ic=; supply one complete explicit state".into()));
        }
        let capacitor_voltages = ordered_state(
            "capacitor",
            circuit.capacitors.iter().map(|c| c.name.as_str()),
            capacitor_voltages_v,
        )?;
        let inductor_currents = ordered_state(
            "inductor",
            circuit.inductors.iter().map(|l| l.name.as_str()),
            inductor_currents_a,
        )?;
        for (c, v) in circuit.capacitors.iter().zip(&capacitor_voltages) {
            if !(0.5 * c.farads * v * v).is_finite() {
                return Err(AnalogError::Config(format!(
                    "initial capacitor energy overflows for '{}'",
                    c.name
                )));
            }
        }
        for (l, i) in circuit.inductors.iter().zip(&inductor_currents) {
            if !(0.5 * l.henries * i * i).is_finite() {
                return Err(AnalogError::Config(format!(
                    "initial inductor energy overflows for '{}'",
                    l.name
                )));
            }
        }
        for mutual in &circuit.mutual_inductors {
            if !(mutual.mutual_henries
                * inductor_currents[mutual.first]
                * inductor_currents[mutual.second])
                .is_finite()
            {
                return Err(AnalogError::Config(
                    "initial mutual-inductor energy overflows".into(),
                ));
            }
        }

        for source in &circuit.current_sources {
            if !stationary_at_origin(source.wave) {
                return Err(AnalogError::Config(format!("transient_state requires current source '{}' to have a stationary initial plateau", source.name)));
            }
        }
        super::initialization_charge::validate_charge_source_cycles(circuit)?;
        let inverse_inductance = Vec::new();
        let ground = circuit.node_count();
        let mut basis = VoltageBasis::new(ground + 1);
        // Independent ideal sources already impose these equations. A matching
        // capacitor must not add a parallel ideal source with an arbitrary
        // split of t=0 source current. The admitted waveforms are locally
        // constant at t=0, so such a capacitor has zero feedthrough current.
        for source in &circuit.voltage_sources {
            if !stationary_at_origin(source.wave) {
                return Err(AnalogError::Config(format!("transient_state requires voltage source '{}' to have a stationary initial plateau; a nonzero t=0 slope has no supported initial source-current observation", source.name)));
            }
            basis.constrain(
                source.p.unwrap_or(ground),
                source.n.unwrap_or(ground),
                source.dc,
                &source.name,
            )?;
        }
        let mut constraints = Vec::new();
        for (c, volts) in circuit.capacitors.iter().zip(&capacitor_voltages) {
            if basis.constrain(
                c.a.unwrap_or(ground),
                c.b.unwrap_or(ground),
                *volts,
                &c.name,
            )? {
                constraints.push(InitialVoltageConstraint {
                    name: c.name.clone(),
                    a: c.a,
                    b: c.b,
                    volts: *volts,
                });
            }
        }
        Ok(Some(Self {
            capacitor_voltages,
            inductor_currents,
            constraints,
            inverse_inductance,
        }))
    }
}

fn ordered_state<'a>(
    kind: &str,
    names: impl Iterator<Item = &'a str>,
    supplied: &BTreeMap<String, f64>,
) -> Result<Vec<f64>, AnalogError> {
    let mut remaining: BTreeMap<_, _> = supplied
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), *value))
        .collect();
    let mut values = Vec::new();
    for name in names {
        values.push(
            remaining
                .remove(&name.to_ascii_lowercase())
                .ok_or_else(|| {
                    AnalogError::Config(format!("transient_state is missing {kind} '{name}'"))
                })?,
        );
    }
    if let Some(name) = remaining.keys().next() {
        return Err(AnalogError::Config(format!(
            "transient_state names unknown {kind} '{name}'"
        )));
    }
    Ok(values)
}

fn stationary_at_origin(wave: Waveform) -> bool {
    match wave {
        Waveform::Dc(_) => true,
        Waveform::Sin {
            amplitude,
            frequency,
            delay,
            ..
        } => delay > 0.0 || amplitude == 0.0 || frequency == 0.0,
        Waveform::Pulse {
            v1,
            v2,
            delay,
            rise,
            fall,
            width,
            ..
        } => v1 == v2 || delay > 0.0 || (rise == 0.0 && (width > 0.0 || fall == 0.0)),
    }
}

/// Weighted union-find: every edge is an exact prescribed voltage difference.
/// Redundant parallel/cyclic constraints are checked, then omitted from MNA.
struct VoltageBasis {
    parent: Vec<usize>,
    rank: Vec<u8>,
    offset: Vec<f64>,
}

impl VoltageBasis {
    fn new(size: usize) -> Self {
        Self {
            parent: (0..size).collect(),
            rank: vec![0; size],
            offset: vec![0.0; size],
        }
    }
    fn find(&mut self, node: usize) -> (usize, f64) {
        if self.parent[node] == node {
            return (node, 0.0);
        }
        let (root, parent_offset) = self.find(self.parent[node]);
        self.offset[node] += parent_offset;
        self.parent[node] = root;
        (root, self.offset[node])
    }
    fn constrain(
        &mut self,
        a: usize,
        b: usize,
        volts: f64,
        name: &str,
    ) -> Result<bool, AnalogError> {
        let (ra, va) = self.find(a);
        let (rb, vb) = self.find(b);
        let offset = volts - va + vb;
        if !offset.is_finite() {
            return Err(AnalogError::Config(format!(
                "initial voltage constraint overflows at '{name}'"
            )));
        }
        if ra == rb {
            let scale = 1.0_f64.max(volts.abs()).max(va.abs()).max(vb.abs());
            if offset.abs() > 64.0 * f64::EPSILON * scale {
                return Err(AnalogError::Config(format!("conflicting initial voltage constraint at '{name}': prescribed {volts} V, implied {} V", va - vb)));
            }
            return Ok(false);
        }
        if self.rank[ra] < self.rank[rb] {
            self.parent[ra] = rb;
            self.offset[ra] = offset;
        } else {
            self.parent[rb] = ra;
            self.offset[rb] = -offset;
            if self.rank[ra] == self.rank[rb] {
                self.rank[ra] += 1;
            }
        }
        Ok(true)
    }
}

/// Invert the physical inductance matrix for differentiated inductive KCL.
/// Positive definiteness is required; an ideal perfectly coupled degeneracy
/// needs a separate state reduction and is refused by this initial-state mode.
fn inverse_inductance(circuit: &Circuit) -> Result<Vec<f64>, AnalogError> {
    let n = circuit.inductors.len();
    let mut matrix = vec![0.0; n * n];
    for (i, l) in circuit.inductors.iter().enumerate() {
        matrix[i * n + i] = l.henries;
    }
    for m in &circuit.mutual_inductors {
        matrix[m.first * n + m.second] += m.mutual_henries;
        matrix[m.second * n + m.first] += m.mutual_henries;
    }
    let invalid = || {
        AnalogError::Config(
            "transient_state requires a finite positive-definite inductance matrix".into(),
        )
    };
    let mut lower = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..=i {
            let mut value = matrix[i * n + j];
            for k in 0..j {
                value -= lower[i * n + k] * lower[j * n + k];
            }
            if i == j {
                if value <= 0.0 || !value.is_finite() {
                    return Err(invalid());
                }
                lower[i * n + j] = libm::sqrt(value);
            } else {
                lower[i * n + j] = value / lower[j * n + j];
                if !lower[i * n + j].is_finite() {
                    return Err(invalid());
                }
            }
        }
    }
    let mut inverse = vec![0.0; n * n];
    let mut y = vec![0.0; n];
    for col in 0..n {
        for i in 0..n {
            let mut value = if i == col { 1.0 } else { 0.0 };
            for j in 0..i {
                value -= lower[i * n + j] * y[j];
            }
            y[i] = value / lower[i * n + i];
        }
        for i in (0..n).rev() {
            let mut value = y[i];
            for j in i + 1..n {
                value -= lower[j * n + i] * inverse[j * n + col];
            }
            value /= lower[i * n + i];
            if !value.is_finite() {
                return Err(invalid());
            }
            inverse[i * n + col] = value;
        }
    }
    Ok(inverse)
}
