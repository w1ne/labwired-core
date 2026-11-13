// SPDX-License-Identifier: MIT
//! Bound the initial source-current observations to a supported charge graph.
//!
//! A prescribed charge network touching two distinct nodes connected by ideal
//! voltage sources needs a full differentiated charge solve. A spanning set of
//! voltage constraints alone would otherwise give basis-dependent source
//! currents. Refuse that class, including implicit semiconductor storage.

use super::{AnalogError, Circuit, NodeRef};

pub(super) fn validate_charge_source_cycles(circuit: &Circuit) -> Result<(), AnalogError> {
    let ground = circuit.node_count();
    let mut sources = Connectivity::new(ground + 1);
    for source in &circuit.voltage_sources {
        sources.union(source.p.unwrap_or(ground), source.n.unwrap_or(ground));
    }
    let mut combined = sources.clone();
    // Dependent ideal outputs can carry the same ambiguous reaction current.
    // They belong in the cycle graph, but never in the stationary-source
    // exception: their controls can change after this initial instant.
    for source in &circuit.vcvs {
        combined.union(source.p.unwrap_or(ground), source.n.unwrap_or(ground));
    }
    for source in &circuit.ccvs {
        combined.union(source.p.unwrap_or(ground), source.n.unwrap_or(ground));
    }
    let mut charge = Connectivity::new(ground + 1);
    let mut edge = |name: &str, a: NodeRef, b: NodeRef| -> Result<(), AnalogError> {
        let (a, b) = (a.unwrap_or(ground), b.unwrap_or(ground));
        if sources.connected(a, b) {
            // Its voltage difference is fixed by stationary ideal sources.
            // Do not add this edge to the charge-only basis: doing so could
            // hide a later mixed cycle behind a redundant decoupling capacitor.
            return Ok(());
        }
        if combined.connected(a, b) && !charge.connected(a, b) {
            return Err(AnalogError::Config(format!("transient_state does not support mixed charge-storage/ideal-voltage-source cycles (at '{name}'); initial source current requires a differentiated charge-network solve")));
        }
        combined.union(a, b);
        charge.union(a, b);
        Ok(())
    };
    for c in &circuit.capacitors {
        edge(&c.name, c.a, c.b)?;
    }
    for d in &circuit.diodes {
        if d.model.cjo > 0.0 || d.model.tt > 0.0 {
            edge(&d.name, d.junction_anode, d.cathode)?;
        }
    }
    for b in &circuit.bjts {
        // Transit charge can depend on both junction biases; include both
        // couplings whenever either transit-charge term is active.
        let transit = b.model.tf > 0.0 || b.model.tr > 0.0;
        if b.model.cje > 0.0 || transit {
            edge(&b.name, b.junction_b, b.junction_e)?;
        }
        if b.model.cjc > 0.0 || transit {
            edge(&b.name, b.junction_b, b.junction_c)?;
        }
    }
    for m in &circuit.mosfets {
        for (capacitance, other) in
            m.overlap_capacitances()
                .iter()
                .zip([m.junction_s, m.junction_d, m.bulk])
        {
            if *capacitance > 0.0 {
                edge(&m.name, m.g, other)?;
            }
        }
        for (drain, other) in [(true, m.junction_d), (false, m.junction_s)] {
            if m.bulk_junction_capacitances(drain).iter().any(|c| *c > 0.0) {
                edge(&m.name, m.bulk, other)?;
            }
        }
        if m.model.tox.is_some() {
            // Meyer/Ward-Dutton charge depends on the four terminal biases.
            // Conservative topology admission avoids treating a zero-bias
            // differential capacitance as proof that the coupling is absent.
            let terminals = [m.g, m.junction_d, m.junction_s, m.bulk];
            for a in 0..terminals.len() {
                for b in a + 1..terminals.len() {
                    edge(&m.name, terminals[a], terminals[b])?;
                }
            }
        }
    }
    for j in &circuit.jfets {
        if j.model.cgs > 0.0 {
            edge(&j.name, j.g, j.junction_s)?;
        }
        if j.model.cgd > 0.0 {
            edge(&j.name, j.g, j.junction_d)?;
        }
    }
    Ok(())
}

#[derive(Clone)]
struct Connectivity {
    parent: Vec<usize>,
    rank: Vec<u8>,
}
impl Connectivity {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }
    fn root(&mut self, node: usize) -> usize {
        if self.parent[node] != node {
            self.parent[node] = self.root(self.parent[node]);
        }
        self.parent[node]
    }
    fn connected(&mut self, a: usize, b: usize) -> bool {
        self.root(a) == self.root(b)
    }
    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.root(a), self.root(b));
        if a == b {
            return;
        }
        if self.rank[a] < self.rank[b] {
            self.parent[a] = b;
        } else {
            self.parent[b] = a;
            if self.rank[a] == self.rank[b] {
                self.rank[a] += 1;
            }
        }
    }
}
