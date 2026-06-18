// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! IFM-inspired multi-port IO-Link master station — a product/profile model.
//!
//! This is a pure-Rust model of which sensor profile is plugged into each port.
//! It does NOT instantiate one real device stack per port: the `iolinki` device
//! stack is singleton, so real stack-backed behavior is limited to a single
//! port (see docs/engineering/iolink-device-stack-isolation.md). The "without
//! sharing state" guarantee here is a property of this wrapper, not evidence of
//! four coexisting native device stacks.

#[derive(Debug, Clone)]
enum PortModel {
    Empty,
    Proximity { present: bool },
    Pressure { bar: f32 },
    Distance { mm: u16 },
}

#[derive(Debug)]
pub struct IolinkStation {
    ports: Vec<PortModel>,
}

impl IolinkStation {
    pub fn new_4port() -> Self {
        Self {
            ports: vec![PortModel::Empty; 4],
        }
    }

    pub fn connect_proximity(&mut self, port: usize, present: bool) {
        self.ports[port - 1] = PortModel::Proximity { present };
    }

    pub fn connect_pressure(&mut self, port: usize, bar: f32) {
        self.ports[port - 1] = PortModel::Pressure { bar };
    }

    pub fn connect_distance(&mut self, port: usize, mm: u16) {
        self.ports[port - 1] = PortModel::Distance { mm };
    }

    pub fn port_profiles(&self) -> Vec<String> {
        self.ports
            .iter()
            .map(|p| match p {
                PortModel::Empty => "empty".to_string(),
                PortModel::Proximity { present } => {
                    format!("proximity:{}", if *present { "present" } else { "clear" })
                }
                PortModel::Pressure { bar } => format!("pressure:{bar:.2}bar"),
                PortModel::Distance { mm } => format!("distance:{mm}mm"),
            })
            .collect()
    }
}
